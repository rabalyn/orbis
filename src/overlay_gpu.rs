//! Draws a tile's roads and boundaries on the graphics card instead of with `tiny-skia`.
//!
//! The lines are cut into triangles here, on a tile thread, and painted into the tile's overlay
//! image by `Painter` on the main thread. The image is the same kind the processor path makes,
//! so nothing else changes. Edges are smoothed by multisampling, so the picture is close to the
//! processor's but not the same pixel for pixel.

use bytemuck::{Pod, Zeroable};
use std::ops::Range;

/// The side of an overlay image in texels; the shader has the same number.
const SIZE: u32 = crate::overlay::OVERLAY_SIZE;
const SAMPLES: u32 = 4;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
/// Groups that one pass can tell apart with an 8-bit stencil.
const GROUPS_PER_PASS: usize = 255;
/// Points nearer than this to the one before, in texels, are left out.
const THIN: f32 = 0.2;
/// The narrowest line multisampling draws without gaps; thinner ones are drawn this wide and
/// fainter instead.
const MIN_WIDTH: f32 = 1.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LineVertex {
    /// In overlay texels, y down.
    pub pos: [f32; 2],
    /// Premultiplied.
    pub color: [u8; 4],
}

/// A tile's lines as triangles. Each range of `groups` is drawn so that a texel it covers
/// several times is painted once, as one stroked path would be.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineMesh {
    pub vertices: Vec<LineVertex>,
    pub groups: Vec<Range<u32>>,
}

/// What `Painter` makes an overlay image from.
#[derive(Clone, Debug, PartialEq)]
pub struct Parts {
    pub lines: LineMesh,
    /// What lies over the lines, drawn by the processor: premultiplied RGBA of the whole image.
    pub top: Option<Vec<u8>>,
}

fn premultiplied(c: [u8; 4], fade: f32) -> [u8; 4] {
    let a = c[3] as f32 / 255.0 * fade;
    let part = |v: u8| (v as f32 * a).round() as u8;
    [
        part(c[0]),
        part(c[1]),
        part(c[2]),
        (a * 255.0).round() as u8,
    ]
}

struct Stroker<'a> {
    out: &'a mut Vec<LineVertex>,
    half: f32,
    color: [u8; 4],
}

impl Stroker<'_> {
    fn push(&mut self, p: [f32; 2]) {
        self.out.push(LineVertex {
            pos: p,
            color: self.color,
        });
    }

    /// Whether anything within the line's half width of the box can lie on the image.
    fn near_image(&self, lo: [f32; 2], hi: [f32; 2]) -> bool {
        let r = self.half;
        hi[0] >= -r && hi[1] >= -r && lo[0] <= SIZE as f32 + r && lo[1] <= SIZE as f32 + r
    }

    fn segment(&mut self, a: [f32; 2], b: [f32; 2]) {
        let lo = [a[0].min(b[0]), a[1].min(b[1])];
        let hi = [a[0].max(b[0]), a[1].max(b[1])];
        if !self.near_image(lo, hi) {
            return;
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 {
            return;
        }
        let (nx, ny) = (-dy / len * self.half, dx / len * self.half);
        let corners = [
            [a[0] + nx, a[1] + ny],
            [a[0] - nx, a[1] - ny],
            [b[0] + nx, b[1] + ny],
            [b[0] - nx, b[1] - ny],
        ];
        for i in [0, 1, 2, 2, 1, 3] {
            self.push(corners[i]);
        }
    }

    /// A disc, for a round join or cap.
    fn disc(&mut self, c: [f32; 2]) {
        if !self.near_image(c, c) {
            return;
        }
        let sides = match self.half {
            r if r <= 1.0 => 6,
            r if r <= 2.5 => 8,
            _ => 12,
        };
        let half = self.half;
        let at = |i: usize| {
            let t = i as f32 / sides as f32 * std::f32::consts::TAU;
            [c[0] + t.cos() * half, c[1] + t.sin() * half]
        };
        for i in 0..sides {
            self.push(c);
            self.push(at(i));
            self.push(at(i + 1));
        }
    }

    /// Whether the line turns enough at `b` for the gap on the outside of the bend to show.
    fn bends(&self, a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
        let (ux, uy) = (b[0] - a[0], b[1] - a[1]);
        let (vx, vy) = (c[0] - b[0], c[1] - b[1]);
        let dot = ux * vx + uy * vy;
        let cross = (ux * vy - uy * vx).abs();
        // The gap is about the half width times the tangent of half the turn.
        dot <= 0.0 || cross * self.half > 0.2 * dot
    }

    fn solid(&mut self, points: &[[f32; 2]]) {
        for pair in points.windows(2) {
            self.segment(pair[0], pair[1]);
        }
        for three in points.windows(3) {
            if self.bends(three[0], three[1], three[2]) {
                self.disc(three[1]);
            }
        }
        // Round caps.
        self.disc(points[0]);
        self.disc(points[points.len() - 1]);
    }

    /// Dashes of `on` texels with gaps of `off`, with flat ends, starting anew with each line.
    fn dashed(&mut self, points: &[[f32; 2]], on: f32, off: f32) {
        if on <= 0.0 || on + off <= 0.0 {
            return self.solid(points);
        }
        let mut drawing = true;
        let mut left = on;
        for (i, pair) in points.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            if len <= 0.0 {
                continue;
            }
            let at = |d: f32| {
                let t = d / len;
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
            };
            let mut done = 0.0;
            while done < len {
                let step = left.min(len - done);
                if drawing {
                    self.segment(at(done), at(done + step));
                }
                done += step;
                left -= step;
                if left <= 0.0 {
                    drawing = !drawing;
                    left = if drawing { on } else { off };
                }
            }
            // A dash that runs through a corner is rounded there.
            if drawing && i + 2 < points.len() && self.bends(a, b, points[i + 2]) {
                self.disc(b);
            }
        }
    }
}

/// Adds one group to `mesh`: `lines` stroked `width` texels wide, in overlay texels after
/// `place` (scale, then offsets). `dash` gives the lengths of dash and gap in line widths.
pub fn stroke(
    mesh: &mut LineMesh,
    lines: &[&crate::vector::Line],
    place: (f32, f32, f32),
    width: f32,
    color: [u8; 4],
    dash: Option<[f32; 2]>,
) {
    let (scale, dx, dy) = place;
    let from = mesh.vertices.len() as u32;
    // A hairline keeps its weight by fading instead of narrowing below a texel.
    let fade = (width / MIN_WIDTH).min(1.0);
    let mut stroker = Stroker {
        out: &mut mesh.vertices,
        half: width.max(MIN_WIDTH) / 2.0,
        color: premultiplied(color, fade),
    };
    let mut points: Vec<[f32; 2]> = Vec::new();
    for line in lines {
        points.clear();
        for (i, p) in line.iter().enumerate() {
            let p = [p[0] * scale + dx, p[1] * scale + dy];
            let last = i + 1 == line.len();
            match points.last() {
                Some(q) if (p[0] - q[0]).abs() < THIN && (p[1] - q[1]).abs() < THIN => {
                    // The end of the line stays where it is.
                    if last && points.len() > 1 {
                        *points.last_mut().unwrap() = p;
                    }
                }
                _ => points.push(p),
            }
        }
        if points.len() < 2 {
            continue;
        }
        match dash {
            Some([on, off]) => stroker.dashed(&points, on * width, off * width),
            None => stroker.solid(&points),
        }
    }
    let to = mesh.vertices.len() as u32;
    if to > from {
        mesh.groups.push(from..to);
    }
}

/// Paints overlay images. One is kept by the renderer and used for every tile.
pub struct Painter {
    line_pipeline: wgpu::RenderPipeline,
    top_pipeline: wgpu::RenderPipeline,
    mip_pipeline: wgpu::RenderPipeline,
    copy_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The multisampled image the lines are drawn into, and its stencil.
    canvas: wgpu::TextureView,
    stencil: wgpu::TextureView,
}

impl Painter {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("overlay.wgsl"));
        let copy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay copy"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let line_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay lines"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let copy_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay copy"),
            bind_group_layouts: &[Some(&copy_layout)],
            immediate_size: 0,
        });
        let over = [Some(wgpu::ColorTargetState {
            format: FORMAT,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let replace = [Some(wgpu::ColorTargetState {
            format: FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let stencil_state = |face: wgpu::StencilFaceState| {
            Some(wgpu::DepthStencilState {
                format: STENCIL_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState {
                    front: face,
                    back: face,
                    read_mask: 0xff,
                    write_mask: 0xff,
                },
                bias: Default::default(),
            })
        };
        let multisample = wgpu::MultisampleState {
            count: SAMPLES,
            ..Default::default()
        };
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Unorm8x4];
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay lines"),
            layout: Some(&line_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_line"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<LineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &line_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            // A sample is painted only where this group has not painted it yet, and marked.
            depth_stencil: stencil_state(wgpu::StencilFaceState {
                compare: wgpu::CompareFunction::NotEqual,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op: wgpu::StencilOperation::Replace,
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_line"),
                compilation_options: Default::default(),
                targets: &over,
            }),
            multiview_mask: None,
            cache: None,
        });
        let copy_pipeline = |label, targets: &[Option<wgpu::ColorTargetState>], onto_canvas| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&copy_pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_copy"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: if onto_canvas {
                    stencil_state(wgpu::StencilFaceState::IGNORE)
                } else {
                    None
                },
                multisample: if onto_canvas {
                    multisample
                } else {
                    Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_copy"),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let top_pipeline = copy_pipeline("overlay top", &over, true);
        let mip_pipeline = copy_pipeline("overlay mips", &replace, false);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("overlay copy"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let target = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: SAMPLES,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        Self {
            line_pipeline,
            top_pipeline,
            mip_pipeline,
            copy_layout,
            sampler,
            canvas: target("overlay canvas", FORMAT),
            stencil: target("overlay stencil", STENCIL_FORMAT),
        }
    }

    fn copy_bind(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay copy"),
            layout: &self.copy_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Makes the image of one tile with all its mip levels. The texture holds the picture once
    /// the commands returned with it have been submitted.
    pub fn paint(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        parts: &Parts,
    ) -> (wgpu::Texture, wgpu::CommandBuffer) {
        use wgpu::util::DeviceExt;
        let levels = SIZE.ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("overlay"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            // Read back by the tests, which compare the picture with the processor's.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let level = |i: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: i,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let views: Vec<wgpu::TextureView> = (0..levels).map(level).collect();
        let vertices = (!parts.lines.vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("overlay lines"),
                contents: bytemuck::cast_slice(&parts.lines.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let top = parts.top.as_ref().map(|rgba| {
            let view = device
                .create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some("overlay top"),
                        size: wgpu::Extent3d {
                            width: SIZE,
                            height: SIZE,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: FORMAT,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    rgba,
                )
                .create_view(&Default::default());
            self.copy_bind(device, &view)
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("overlay"),
        });
        // An 8-bit stencil tells 255 groups apart, so more of them take further passes.
        let groups = &parts.lines.groups;
        let passes = groups.len().div_ceil(GROUPS_PER_PASS).max(1);
        for n in 0..passes {
            let last = n + 1 == passes;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay lines"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.canvas,
                    depth_slice: None,
                    resolve_target: last.then_some(&views[0]),
                    ops: wgpu::Operations {
                        load: if n == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.stencil,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(vertices) = &vertices {
                pass.set_pipeline(&self.line_pipeline);
                pass.set_vertex_buffer(0, vertices.slice(..));
                let these = groups.iter().skip(n * GROUPS_PER_PASS);
                for (i, group) in these.take(GROUPS_PER_PASS).enumerate() {
                    pass.set_stencil_reference(i as u32 + 1);
                    pass.draw(group.clone(), 0..1);
                }
            }
            if let (true, Some(top)) = (last, &top) {
                pass.set_pipeline(&self.top_pipeline);
                pass.set_bind_group(0, top, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        // Each smaller level is the one above it, halved.
        for i in 1..levels as usize {
            let from = self.copy_bind(device, &views[i - 1]);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay mips"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[i],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.mip_pipeline);
            pass.set_bind_group(0, &from, &[]);
            pass.draw(0..3, 0..1);
        }
        (texture, encoder.finish())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Whether a point lies in one of a group's triangles.
    pub(crate) fn covered(mesh: &LineMesh, group: &Range<u32>, x: f32, y: f32) -> bool {
        let v = &mesh.vertices[group.start as usize..group.end as usize];
        v.chunks_exact(3).any(|t| {
            let side =
                |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
            let (d0, d1, d2) = (
                side(t[0].pos, t[1].pos),
                side(t[1].pos, t[2].pos),
                side(t[2].pos, t[0].pos),
            );
            (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
        })
    }

    const AS_IS: (f32, f32, f32) = (1.0, 0.0, 0.0);
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    fn one(line: Vec<[f32; 2]>, width: f32, color: [u8; 4], dash: Option<[f32; 2]>) -> LineMesh {
        let mut mesh = LineMesh::default();
        stroke(&mut mesh, &[&line], AS_IS, width, color, dash);
        mesh
    }

    #[test]
    fn a_line_is_as_wide_as_asked_with_round_ends() {
        let mesh = one(vec![[100.0, 200.0], [300.0, 200.0]], 8.0, WHITE, None);
        assert_eq!(mesh.groups, vec![0..mesh.vertices.len() as u32]);
        let g = &mesh.groups[0];
        assert!(covered(&mesh, g, 200.0, 203.5) && covered(&mesh, g, 200.0, 196.5));
        assert!(!covered(&mesh, g, 200.0, 204.5) && !covered(&mesh, g, 200.0, 195.5));
        // The caps reach half a width past the ends, and are round.
        assert!(covered(&mesh, g, 96.5, 200.0) && covered(&mesh, g, 303.5, 200.0));
        assert!(!covered(&mesh, g, 95.0, 200.0) && !covered(&mesh, g, 97.0, 203.5));
    }

    #[test]
    fn a_bend_is_filled_on_its_outside() {
        let mesh = one(
            vec![[100.0, 100.0], [200.0, 100.0], [200.0, 200.0]],
            10.0,
            WHITE,
            None,
        );
        // Beyond the corner on the diagonal, inside the round join but outside both segments.
        assert!(covered(&mesh, &mesh.groups[0], 203.0, 97.0));
        assert!(!covered(&mesh, &mesh.groups[0], 204.5, 95.5));
    }

    #[test]
    fn dashes_alternate_with_gaps_and_have_flat_ends() {
        // Dashes of three widths and gaps of two, as tiny-skia measures them.
        let mesh = one(
            vec![[0.0, 50.0], [200.0, 50.0]],
            4.0,
            WHITE,
            Some([3.0, 2.0]),
        );
        let g = &mesh.groups[0];
        for (x, on) in [
            (1.0, true),
            (11.0, true),
            (13.0, false),
            (19.0, false),
            (21.0, true),
        ] {
            assert_eq!(covered(&mesh, g, x, 50.0), on, "at {x}");
        }
        // No cap before the start of the first dash.
        let mesh = one(
            vec![[50.0, 50.0], [200.0, 50.0]],
            4.0,
            WHITE,
            Some([3.0, 2.0]),
        );
        assert!(!covered(&mesh, &mesh.groups[0], 49.0, 50.0));
    }

    #[test]
    fn a_hairline_is_a_texel_wide_and_fainter() {
        let mesh = one(
            vec![[10.0, 10.0], [90.0, 10.0]],
            0.5,
            [200, 100, 50, 255],
            None,
        );
        let g = &mesh.groups[0];
        assert!(covered(&mesh, g, 50.0, 10.4) && !covered(&mesh, g, 50.0, 10.6));
        assert_eq!(mesh.vertices[0].color, [100, 50, 25, 128]);
        // At a texel and above the colour is only premultiplied.
        let mesh = one(
            vec![[10.0, 10.0], [90.0, 10.0]],
            2.0,
            [200, 100, 50, 128],
            None,
        );
        assert_eq!(mesh.vertices[0].color, [100, 50, 25, 128]);
    }

    #[test]
    fn what_lies_off_the_image_makes_no_triangles() {
        let mut mesh = LineMesh::default();
        let off = vec![[-300.0, -40.0], [900.0, -40.0]];
        let point = vec![[20.0, 20.0]];
        stroke(&mut mesh, &[&off, &point], AS_IS, 4.0, WHITE, None);
        assert!(mesh.vertices.is_empty() && mesh.groups.is_empty());
        // A line that crosses the image keeps only the part near it.
        let across = vec![
            [-5000.0, 100.0],
            [-1000.0, 100.0],
            [100.0, 100.0],
            [600.0, 100.0],
        ];
        stroke(&mut mesh, &[&across], AS_IS, 4.0, WHITE, None);
        assert_eq!(mesh.groups.len(), 1);
        assert!(mesh.vertices.iter().all(|v| v.pos[0] > -1100.0));
        assert!(covered(&mesh, &mesh.groups[0], 50.0, 100.0));
    }

    #[test]
    fn crowded_points_are_thinned_but_the_ends_stay() {
        let dense: Vec<[f32; 2]> = (0..=1000)
            .map(|i| [100.0 + i as f32 * 0.01, 300.0])
            .collect();
        let mesh = one(dense, 4.0, WHITE, None);
        // Ten texels of line: far fewer segments than points.
        assert!(
            mesh.vertices.len() < 6 * 60 + 2 * 36,
            "{}",
            mesh.vertices.len()
        );
        let g = &mesh.groups[0];
        assert!(covered(&mesh, g, 100.5, 300.0) && covered(&mesh, g, 109.9, 300.0));
        assert!(covered(&mesh, g, 111.5, 300.0) && !covered(&mesh, g, 112.5, 300.0));
    }

    #[test]
    fn each_stroke_is_a_group_of_its_own() {
        let mut mesh = LineMesh::default();
        let (a, b) = (
            vec![[0.0, 10.0], [100.0, 10.0]],
            vec![[0.0, 30.0], [100.0, 30.0]],
        );
        stroke(&mut mesh, &[&a, &b], AS_IS, 4.0, WHITE, None);
        stroke(&mut mesh, &[&a], AS_IS, 2.0, WHITE, None);
        assert_eq!(mesh.groups.len(), 2);
        assert_eq!(mesh.groups[0].end, mesh.groups[1].start);
        assert_eq!(mesh.groups[1].end as usize, mesh.vertices.len());
        assert!(covered(&mesh, &mesh.groups[0], 50.0, 30.0));
        assert!(!covered(&mesh, &mesh.groups[1], 50.0, 30.0));
    }
}
