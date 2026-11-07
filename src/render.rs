//! GPU drawing with wgpu. Everything is positioned relative to the camera, so 32-bit
//! floats stay precise at any distance from the Earth's centre.

use crate::atlas::{self, AtlasVertex, AtlasView, Mesh};
use crate::buildings::{BuildingMesh, BuildingVertex};
use crate::camera::{Camera, FOV_Y};
use crate::geo::{geodetic_to_ecef, radius, TileKey};
use crate::labels::{AtlasRect, Mask, Shelves, TextKey, TextWorker, ATLAS_SIZE};
use crate::sky::SkyPoint;
use crate::loader::{mesh_indices, TileData, Vertex, GRID, TILE_MAX_Z};
use crate::text;
use bytemuck::{Pod, Zeroable};
use glam::{DMat3, DVec3};
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::ops::Range;
use wgpu::util::{BufferInitDescriptor, DeviceExt};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_TILES: usize = 1500;
/// Overlays are four times the size of imagery, so fewer of them stay in memory.
const MAX_OVERLAYS: usize = 500;
/// Building meshes kept on the GPU; city tiles take up to a few megabytes each.
const MAX_BUILDING_TILES: usize = 300;
/// Latitude where Web Mercator imagery ends, just inside the polar caps.
const IMAGERY_LIMIT_DEG: f64 = 85.06;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    cam: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    look: [f32; 4],
    viewport: [f32; 4],
    /// Direction to the Sun in the Earth-fixed frame; w is 1 with sunlight, 0 for flat light.
    sun: [f32; 4],
    /// From J2000 equatorial directions to the Earth-fixed frame, as columns.
    sky_rot: [[f32; 4]; 4],
    /// The faintest magnitude that can be seen from here.
    sky: [f32; 4],
    /// The Moon's shadow: the axis towards the Sun (w is 1 while there is a shadow), where the axis
    /// crosses the plane through the Earth's centre, and the apexes and slopes of the umbra and penumbra.
    eclipse_axis: [f32; 4],
    eclipse_foot: [f32; 4],
    eclipse_cones: [f32; 4],
    /// The Moon seen from the camera when it can cover the Sun: its Earth-fixed direction and
    /// angular radius, and the Sun's angular radius in w of `sky`; radius 0 means none.
    moon: [f32; 4],
}

/// Room for the bright planets and the Moon after the stars.
const SKY_BODIES: usize = 8;

struct GpuTile {
    vertices: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    overlay: Option<wgpu::BindGroup>,
    /// Where the tile's imagery lies in its texture: (u0, v0, size).
    uv_rect: (f64, f64, f64),
    credit: &'static str,
    heights: Vec<f32>,
    center: DVec3,
    last_used: u64,
}

/// The flat map as meshes in clip space: the underlay first, then each tile.
struct AtlasDraw {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    vertex_capacity: u64,
    index_capacity: u64,
    underlay: Range<u32>,
    tiles: Vec<(TileKey, Range<u32>)>,
}

struct GpuBuildings {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    center: DVec3,
    last_used: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct LabelInstance {
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub uv: [f32; 4],
    /// Text colour and overall opacity.
    pub fill: [f32; 4],
    /// Outline colour, and the label's angle in radians.
    pub halo: [f32; 4],
}

struct Panel {
    uniform: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    texture: Option<wgpu::Texture>,
    text: String,
    width: u32,
    height: u32,
    max_cols: usize,
    scale: usize,
    /// Where an image panel sits, instead of in a corner.
    at: Option<[f64; 2]>,
}

/// Panels 0 to 3 are text panels in corners; the interface's panels follow.
pub const UI_PANEL_BASE: usize = 4;
const UI_PANELS: usize = 6;

#[derive(Clone, Copy)]
pub enum Corner {
    TopRight,
    BottomLeft,
    TopLeft,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    tile_pipeline: wgpu::RenderPipeline,
    atlas_pipeline: wgpu::RenderPipeline,
    /// Set while the flat map is shown instead of the globe.
    atlas: Option<AtlasDraw>,
    sky_pipeline: wgpu::RenderPipeline,
    panel_pipeline: wgpu::RenderPipeline,
    label_pipeline: wgpu::RenderPipeline,
    building_pipeline: wgpu::RenderPipeline,
    buildings: HashMap<TileKey, GpuBuildings>,
    /// Which building tiles to draw this frame.
    visible_buildings: Vec<TileKey>,
    format: wgpu::TextureFormat,
    samples: u32,
    /// The multisampled colour image, when anti-aliasing is available.
    msaa: Option<(wgpu::TextureView, u32, u32)>,
    label_texture: wgpu::Texture,
    label_bind: wgpu::BindGroup,
    label_rects: HashMap<TextKey, Option<AtlasRect>>,
    label_shelves: Shelves,
    label_instances: wgpu::Buffer,
    label_capacity: usize,
    label_count: u32,
    no_overlay: wgpu::BindGroup,
    pub roads_visible: bool,
    /// Direction to the Sun in the Earth-fixed frame, and whether it lights the scene.
    pub sun: DVec3,
    pub sun_on: bool,
    /// Whether stars, planets and the Moon are drawn, the rotation that puts them in the
    /// Earth-fixed frame, and the faintest magnitude visible from the camera.
    pub sky_on: bool,
    pub sky_rot: DMat3,
    pub sky_limit: f64,
    /// The Moon's shadow on the Earth, while it touches it.
    pub shadow: Option<crate::eclipse::Elements>,
    /// The Sun and Moon as seen from the camera, for the Moon covering the Sun in the sky.
    pub sky_discs: Option<crate::ephemeris::Topocentric>,
    sky_points: wgpu::Buffer,
    star_count: usize,
    body_count: usize,
    star_pipeline: wgpu::RenderPipeline,
    frame_buffer: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    tile_layout: wgpu::BindGroupLayout,
    panel_layout: wgpu::BindGroupLayout,
    tile_sampler: wgpu::Sampler,
    panel_sampler: wgpu::Sampler,
    indices: wgpu::Buffer,
    index_count: u32,
    underlay_vertices: wgpu::Buffer,
    underlay_indices: wgpu::Buffer,
    underlay_count: u32,
    underlay_bind: wgpu::BindGroup,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    depth: Option<(wgpu::TextureView, u32, u32)>,
    tiles: HashMap<TileKey, GpuTile>,
    panels: Vec<Panel>,
    frame_no: u64,
}

fn tile_texture_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    mips: &[(u32, u32, Vec<u8>)],
) -> wgpu::TextureView {
    let (w, h, _) = mips[0];
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tile"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (mw, mh, data)) in mips.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mw * 4),
                rows_per_image: Some(*mh),
            },
            wgpu::Extent3d { width: *mw, height: *mh, depth_or_array_layers: 1 },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// A plain globe that shows through where no tile is drawn. Web Mercator imagery does not
/// reach the poles, so the caps get a plain colour: Arctic water in the north, ice over
/// Antarctica. Between them it sits below the tiles in ocean colour.
fn underlay_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let limit = IMAGERY_LIMIT_DEG;
    let mut lats: Vec<f64> = vec![90.0, 87.5, limit];
    let bands = 48;
    for i in 1..bands {
        lats.push(limit - 2.0 * limit * i as f64 / bands as f64);
    }
    lats.extend([-limit, -87.5, -90.0]);
    let segments = 96;
    let mut vertices = Vec::new();
    for lat in &lats {
        for s in 0..=segments {
            let lon = (s as f64 / segments as f64) * std::f64::consts::TAU - std::f64::consts::PI;
            let polar = lat.abs() >= limit;
            // The ocean sits below the deepest terrain so that tiles always cover it.
            let h = if polar { 0.0 } else { -12_000.0 };
            let p = geodetic_to_ecef(lat.to_radians(), lon, h);
            vertices.push(Vertex {
                pos: p.as_vec3().to_array(),
                uv: [0.5, if polar && *lat < 0.0 { 0.25 } else { 0.75 }],
                st: [0.0, 0.0],
                normal: p.normalize().as_vec3().to_array(),
            });
        }
    }
    let row = segments + 1;
    let mut indices = Vec::new();
    for j in 0..(lats.len() as u32 - 1) {
        for i in 0..segments {
            let (a, b) = (j * row + i, j * row + i + 1);
            let (c, d) = (a + row, b + row);
            indices.extend([a, c, b, b, c, d]);
        }
    }
    (vertices, indices)
}

fn underlay_mips() -> Vec<(u32, u32, Vec<u8>)> {
    // Top half ice, bottom half ocean, picked by the mesh's v coordinate.
    let (ice, ocean) = match crate::body::current().ground {
        Some(plain) => (plain, plain),
        None => ([254u8, 254, 254, 255], [17u8, 31, 72, 255]),
    };
    let mut data = Vec::new();
    for _ in 0..2 {
        data.extend(ice);
    }
    for _ in 0..2 {
        data.extend(ocean);
    }
    vec![(1, 4, data)]
}

/// The anti-aliasing level to use for a colour format on this GPU.
pub fn sample_count(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> u32 {
    use wgpu::TextureFormatFeatureFlags as Flags;
    let colour = adapter.get_texture_format_features(format).flags;
    let depth = adapter.get_texture_format_features(DEPTH_FORMAT).flags;
    let usable = colour.contains(Flags::MULTISAMPLE_X4 | Flags::MULTISAMPLE_RESOLVE)
        && depth.contains(Flags::MULTISAMPLE_X4);
    if usable { 4 } else { 1 }
}

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("globe.wgsl"));
        let multisample = wgpu::MultisampleState { count: samples, ..Default::default() };

        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tile"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let panel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("panel"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let layout_for = |second: &wgpu::BindGroupLayout, label| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(&frame_layout), Some(second)],
                immediate_size: 0,
            })
        };
        let tile_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile"),
            bind_group_layouts: &[Some(&frame_layout), Some(&tile_layout), Some(&tile_layout)],
            immediate_size: 0,
        });
        let panel_pl = layout_for(&panel_layout, "panel");
        let label_pl = layout_for(&tile_layout, "label");
        let sky_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });

        let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x2, 4 => Float32x3];
        let instance_attrs = wgpu::vertex_attr_array![3 => Float32x3];
        let label_attrs = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4
        ];
        let opaque = [Some(wgpu::ColorTargetState {
            format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let blended = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let no_depth = wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        };

        let tile_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile"),
            layout: Some(&tile_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_tile"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &vertex_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<[f32; 3]>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                // Depth is reversed: larger values are nearer.
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_tile"),
                compilation_options: Default::default(),
                targets: &opaque,
            }),
            multiview_mask: None,
            cache: None,
        });
        let atlas_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x3];
        let atlas_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("atlas"),
            layout: Some(&tile_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_atlas"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<AtlasVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &atlas_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(no_depth.clone()),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_tile"),
                compilation_options: Default::default(),
                targets: &opaque,
            }),
            multiview_mask: None,
            cache: None,
        });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(no_depth.clone()),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &opaque,
            }),
            multiview_mask: None,
            cache: None,
        });
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let added = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState { color: additive, alpha: additive }),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let star_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
        let star_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("stars"),
            layout: Some(&sky_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_star"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<SkyPoint>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &star_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(no_depth.clone()),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_star"),
                compilation_options: Default::default(),
                targets: &added,
            }),
            multiview_mask: None,
            cache: None,
        });
        let sky_points = create_sky_buffer(&device, 0);
        let building_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("building"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let building_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4];
        let building_instance_attrs = wgpu::vertex_attr_array![2 => Float32x3];
        let building_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("building"),
            layout: Some(&building_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_building"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<BuildingVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &building_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<[f32; 3]>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &building_instance_attrs,
                    }),
                ],
            },
            // Only outward faces, so that a camera inside a building sees out rather than
            // a screen full of wall.
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_building"),
                compilation_options: Default::default(),
                targets: &opaque,
            }),
            multiview_mask: None,
            cache: None,
        });
        let label_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("label"),
            layout: Some(&label_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_label"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<LabelInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &label_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(no_depth.clone()),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_label"),
                compilation_options: Default::default(),
                targets: &blended,
            }),
            multiview_mask: None,
            cache: None,
        });
        let panel_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("panel"),
            layout: Some(&panel_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_panel"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(no_depth),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_panel"),
                compilation_options: Default::default(),
                targets: &blended,
            }),
            multiview_mask: None,
            cache: None,
        });

        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: frame_buffer.as_entire_binding() }],
        });
        let tile_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tile"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let panel_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("panel"),
            ..Default::default()
        });

        let indices_cpu = mesh_indices();
        let indices = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("tile indices"),
            contents: bytemuck::cast_slice(&indices_cpu),
            usage: wgpu::BufferUsages::INDEX,
        });
        let (under_v, under_i) = underlay_mesh();
        let underlay_vertices = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("underlay vertices"),
            contents: bytemuck::cast_slice(&under_v),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let underlay_indices = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("underlay indices"),
            contents: bytemuck::cast_slice(&under_i),
            usage: wgpu::BufferUsages::INDEX,
        });
        let texture_bind = |view: &wgpu::TextureView, sampler: &wgpu::Sampler, label| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &tile_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                ],
            })
        };
        let under_view =
            tile_texture_view(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, &underlay_mips());
        let underlay_bind = texture_bind(&under_view, &tile_sampler, "underlay");
        let empty_view = tile_texture_view(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8Unorm,
            &[(1, 1, vec![0, 0, 0, 0])],
        );
        let no_overlay = texture_bind(&empty_view, &tile_sampler, "no overlay");

        let label_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("label atlas"),
            size: wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let label_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("label"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let label_bind = texture_bind(&label_texture.create_view(&Default::default()), &label_sampler, "label");
        let label_capacity = 256;
        let label_instances = create_label_buffer(&device, label_capacity);

        let instance_capacity = 1024;
        let instances = create_instance_buffer(&device, instance_capacity);
        let panel = || Panel {
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("panel"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            bind_group: None,
            texture: None,
            text: String::new(),
            width: 0,
            height: 0,
            max_cols: 0,
            scale: 1,
            at: None,
        };
        let panels = (0..UI_PANEL_BASE + UI_PANELS).map(|_| panel()).collect();

        Self {
            tile_pipeline,
            atlas_pipeline,
            atlas: None,
            sky_pipeline,
            panel_pipeline,
            label_pipeline,
            building_pipeline,
            buildings: HashMap::new(),
            visible_buildings: Vec::new(),
            format,
            samples,
            msaa: None,
            label_texture,
            label_bind,
            label_rects: HashMap::new(),
            label_shelves: Shelves::new(ATLAS_SIZE),
            label_instances,
            label_capacity,
            label_count: 0,
            no_overlay,
            roads_visible: true,
            sun: DVec3::X,
            sun_on: true,
            sky_on: true,
            sky_rot: DMat3::IDENTITY,
            sky_limit: 6.5,
            shadow: None,
            sky_discs: None,
            sky_points,
            star_count: 0,
            body_count: 0,
            star_pipeline,
            frame_buffer,
            frame_bind,
            tile_layout,
            panel_layout,
            tile_sampler,
            panel_sampler,
            index_count: indices_cpu.len() as u32,
            indices,
            underlay_vertices,
            underlay_indices,
            underlay_count: under_i.len() as u32,
            underlay_bind,
            instances,
            instance_capacity,
            depth: None,
            tiles: HashMap::new(),
            panels,
            frame_no: 0,
            device,
            queue,
        }
    }

    /// Switches to the flat map and builds its meshes, or back to the globe with `None`.
    pub fn set_atlas(&mut self, atlas: Option<(&AtlasView, &[TileKey])>) {
        let Some((view, draw)) = atlas else {
            self.atlas = None;
            return;
        };
        let mut mesh = Mesh::default();
        atlas::add_underlay(view, IMAGERY_LIMIT_DEG.to_radians(), &mut mesh);
        let underlay = 0..mesh.indices.len() as u32;
        let mut tiles = Vec::with_capacity(draw.len());
        for k in draw {
            let Some(t) = self.tiles.get(k) else { continue };
            let from = mesh.indices.len() as u32;
            atlas::add_tile(view, *k, t.uv_rect, &mut mesh);
            tiles.push((*k, from..mesh.indices.len() as u32));
        }
        let (vbytes, ibytes) = (
            bytemuck::cast_slice::<_, u8>(&mesh.vertices),
            bytemuck::cast_slice::<_, u8>(&mesh.indices),
        );
        let device = &self.device;
        let (mut vertices, mut indices, mut vertex_capacity, mut index_capacity) = match self.atlas.take() {
            Some(a) => (Some(a.vertices), Some(a.indices), a.vertex_capacity, a.index_capacity),
            None => (None, None, 0, 0),
        };
        let grow = |buffer: &mut Option<wgpu::Buffer>, capacity: &mut u64, needed: u64, usage| {
            if buffer.is_none() || needed > *capacity {
                *capacity = needed.next_power_of_two().max(1 << 16);
                *buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("atlas"),
                    size: *capacity,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
        };
        grow(&mut vertices, &mut vertex_capacity, vbytes.len() as u64, wgpu::BufferUsages::VERTEX);
        grow(&mut indices, &mut index_capacity, ibytes.len() as u64, wgpu::BufferUsages::INDEX);
        let (vertices, indices) = (vertices.expect("grown"), indices.expect("grown"));
        if !vbytes.is_empty() {
            self.queue.write_buffer(&vertices, 0, vbytes);
            self.queue.write_buffer(&indices, 0, ibytes);
        }
        self.atlas = Some(AtlasDraw { vertices, indices, vertex_capacity, index_capacity, underlay, tiles });
    }

    pub fn has_tile(&self, k: TileKey) -> bool {
        self.tiles.contains_key(&k)
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn insert_tile(&mut self, data: &TileData) {
        let vertices = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("tile vertices"),
            contents: bytemuck::cast_slice(&data.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let view = tile_texture_view(&self.device, &self.queue, wgpu::TextureFormat::Rgba8UnormSrgb, &data.mips);
        let bind_group = self.sampled_bind(&view);
        // The first and last grid vertices are the corners of the imagery's part.
        let (first, last) = (data.vertices.first(), data.vertices.get((GRID + 1) * (GRID + 1) - 1));
        let uv_rect = match (first, last) {
            (Some(a), Some(b)) => (a.uv[0] as f64, a.uv[1] as f64, (b.uv[0] - a.uv[0]) as f64),
            _ => (0.0, 0.0, 1.0),
        };
        self.tiles.insert(
            data.key,
            GpuTile {
                vertices,
                bind_group,
                overlay: None,
                uv_rect,
                credit: data.credit,
                heights: data.heights.clone(),
                center: data.center,
                last_used: self.frame_no,
            },
        );
    }

    fn sampled_bind(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tile"),
            layout: &self.tile_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.tile_sampler) },
            ],
        })
    }

    /// What a loaded tile's imagery needs credited, beyond the credits always shown.
    pub fn tile_credit(&self, k: TileKey) -> &'static str {
        self.tiles.get(&k).map_or("", |t| t.credit)
    }

    pub fn has_buildings(&self, k: TileKey) -> bool {
        self.buildings.contains_key(&k)
    }

    pub fn insert_buildings(&mut self, k: TileKey, mesh: &BuildingMesh) {
        let vertices = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("building vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("building indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.buildings.insert(
            k,
            GpuBuildings {
                vertices,
                indices,
                index_count: mesh.indices.len() as u32,
                center: mesh.center,
                last_used: self.frame_no,
            },
        );
    }

    /// Chooses the building tiles to draw. Ones not loaded yet are simply left out.
    pub fn set_visible_buildings(&mut self, keys: Vec<TileKey>) {
        self.visible_buildings = keys;
    }

    pub fn visible_building_count(&self) -> usize {
        self.visible_buildings.iter().filter(|k| self.buildings.contains_key(k)).count()
    }

    /// Sends queued uploads to the GPU. Until this or a frame is submitted, each upload keeps
    /// a copy in main memory, and resources dropped meanwhile are not released.
    pub fn flush_uploads(&self) {
        self.queue.submit([]);
        let _ = self.device.poll(wgpu::PollType::Poll);
    }

    pub fn has_overlay(&self, k: TileKey) -> bool {
        self.tiles.get(&k).is_some_and(|t| t.overlay.is_some())
    }

    /// Forgets every tile, as when they are to be made again from other imagery.
    pub fn clear_tiles(&mut self) {
        self.tiles.clear();
    }

    /// Drops every tile's overlay, as when what they were drawn from has changed.
    pub fn clear_overlays(&mut self) {
        for tile in self.tiles.values_mut() {
            tile.overlay = None;
        }
    }

    /// Attaches an overlay to its tile. If the tile has gone meanwhile, it is asked for again
    /// whenever the tile returns.
    pub fn insert_overlay(&mut self, k: TileKey, mips: &[(u32, u32, Vec<u8>)]) {
        if !self.tiles.contains_key(&k) {
            return;
        }
        // Premultiplied sRGB values, which the shader decodes itself.
        let view = tile_texture_view(&self.device, &self.queue, wgpu::TextureFormat::Rgba8Unorm, mips);
        let bind = self.sampled_bind(&view);
        if let Some(tile) = self.tiles.get_mut(&k) {
            tile.overlay = Some(bind);
        }
    }

    /// Where a text lies in the label atlas. Text not seen before is sent off to be rasterised.
    pub fn label_rect(&mut self, key: &TextKey, worker: &mut TextWorker) -> Option<AtlasRect> {
        if let Some(rect) = self.label_rects.get(key) {
            return *rect;
        }
        self.label_rects.insert(key.clone(), None);
        worker.request(key.clone());
        None
    }

    /// Puts rasterised text into the atlas. A full atlas starts over; what is still on
    /// screen simply gets requested again.
    pub fn receive_mask(&mut self, mask: Mask) {
        let at = self.label_shelves.alloc(mask.width, mask.height).or_else(|| {
            self.label_rects.clear();
            self.label_shelves.clear();
            self.label_shelves.alloc(mask.width, mask.height)
        });
        let Some((x, y)) = at else { return };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.label_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &mask.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mask.width * 2),
                rows_per_image: Some(mask.height),
            },
            wgpu::Extent3d { width: mask.width, height: mask.height, depth_or_array_layers: 1 },
        );
        self.label_rects.insert(mask.key, Some(AtlasRect { x, y, w: mask.width, h: mask.height }));
    }

    /// Loads the stars, once. Planets and the Moon follow them in the same buffer.
    pub fn set_stars(&mut self, stars: &[SkyPoint]) {
        self.sky_points = create_sky_buffer(&self.device, stars.len());
        self.queue.write_buffer(&self.sky_points, 0, bytemuck::cast_slice(stars));
        self.star_count = stars.len();
        self.body_count = 0;
    }

    pub fn has_stars(&self) -> bool {
        self.star_count > 0
    }

    pub fn set_sky_bodies(&mut self, bodies: &[SkyPoint]) {
        let bodies = &bodies[..bodies.len().min(SKY_BODIES)];
        let offset = (self.star_count * size_of::<SkyPoint>()) as u64;
        self.queue.write_buffer(&self.sky_points, offset, bytemuck::cast_slice(bodies));
        self.body_count = bodies.len();
    }

    pub fn set_labels(&mut self, instances: &[LabelInstance]) {
        if instances.len() > self.label_capacity {
            self.label_capacity = instances.len().next_power_of_two();
            self.label_instances = create_label_buffer(&self.device, self.label_capacity);
        }
        self.queue.write_buffer(&self.label_instances, 0, bytemuck::cast_slice(instances));
        self.label_count = instances.len() as u32;
    }

    /// The screen area a text panel covers, as (x0, y0, x1, y1).
    pub fn panel_rect(&self, i: usize, corner: Corner, width: u32, height: u32) -> [f64; 4] {
        let p = &self.panels[i];
        let margin = 8.0;
        let (sw, sh) = (width as f64, height as f64);
        let (pw, ph) = (p.width as f64, p.height as f64);
        if let Some([x, y]) = p.at {
            return [x, y, x + pw, y + ph];
        }
        let (x0, y0) = match corner {
            Corner::TopRight => (sw - pw - margin, margin),
            Corner::BottomLeft => (margin, sh - ph - margin),
            Corner::TopLeft => (margin, margin),
        };
        [x0, y0, x0 + pw, y0 + ph]
    }

    /// The terrain height from the finest loaded tile covering a position.
    pub fn ground(&self, lat: f64, lon: f64) -> Option<f64> {
        (0..=TILE_MAX_Z).rev().find_map(|z| {
            let k = TileKey::containing(lat, lon, z);
            let tile = self.tiles.get(&k)?;
            let (u, v) = (
                crate::geo::lon_to_merc_u(lon) * (1u64 << z) as f64 - k.x as f64,
                crate::geo::lat_to_merc_v(lat) * (1u64 << z) as f64 - k.y as f64,
            );
            Some(height_on_grid(&tile.heights, u, v))
        })
    }

    fn evict(&mut self, in_use: &HashSet<TileKey>) {
        if self.buildings.len() > MAX_BUILDING_TILES {
            let shown: HashSet<TileKey> = self.visible_buildings.iter().copied().collect();
            let mut old: Vec<_> = self
                .buildings
                .iter()
                .filter(|(k, _)| !shown.contains(*k))
                .map(|(k, b)| (b.last_used, *k))
                .collect();
            old.sort_unstable();
            for (_, k) in old.into_iter().take(self.buildings.len() - MAX_BUILDING_TILES) {
                self.buildings.remove(&k);
            }
        }
        let overlays = self.tiles.values().filter(|t| t.overlay.is_some()).count();
        if overlays > MAX_OVERLAYS {
            let mut old: Vec<_> = self
                .tiles
                .iter()
                .filter(|(k, t)| t.overlay.is_some() && !in_use.contains(*k))
                .map(|(k, t)| (t.last_used, *k))
                .collect();
            old.sort_unstable();
            for (_, k) in old.into_iter().take(overlays - MAX_OVERLAYS) {
                if let Some(tile) = self.tiles.get_mut(&k) {
                    tile.overlay = None;
                }
            }
        }
        if self.tiles.len() <= MAX_TILES {
            return;
        }
        let mut old: Vec<_> = self
            .tiles
            .iter()
            .filter(|(k, _)| !in_use.contains(*k))
            .map(|(k, t)| (t.last_used, *k))
            .collect();
        old.sort_unstable();
        let excess = self.tiles.len() - MAX_TILES;
        for (_, k) in old.into_iter().take(excess) {
            self.tiles.remove(&k);
        }
    }

    pub fn set_panel(&mut self, i: usize, content: &str, max_cols: usize, scale: usize) {
        let p = &self.panels[i];
        if p.text == content && p.max_cols == max_cols && p.scale == scale {
            return;
        }
        if content.is_empty() {
            let p = &mut self.panels[i];
            (p.bind_group, p.texture, p.width, p.height, p.max_cols, p.scale) = (None, None, 0, 0, max_cols, scale);
            p.text.clear();
            return;
        }
        let lines = text::wrap(content, max_cols);
        let (w, h, rgba) = text::rasterize(&lines, scale);
        self.upload_panel(i, w, h, &rgba);
        let p = &mut self.panels[i];
        p.text = content.to_string();
        p.max_cols = max_cols;
        p.scale = scale;
    }

    /// Shows an image from the interface at a screen position, or hides the panel for `None`.
    /// `i` counts from the first interface panel.
    pub fn set_ui_panel(&mut self, i: usize, image: Option<([f64; 2], u32, u32, &[u8])>) {
        let i = UI_PANEL_BASE + i;
        let Some((at, w, h, rgba)) = image else {
            let p = &mut self.panels[i];
            (p.bind_group, p.texture, p.width, p.height, p.at) = (None, None, 0, 0, None);
            return;
        };
        if w == 0 || h == 0 {
            return;
        }
        self.upload_panel(i, w, h, rgba);
        self.panels[i].at = Some(at);
    }

    pub fn ui_panel_count() -> usize {
        UI_PANELS
    }

    fn upload_panel(&mut self, i: usize, w: u32, h: u32, rgba: &[u8]) {
        let p = &self.panels[i];
        let same_size = p.width == w && p.height == h && p.texture.is_some();
        if !same_size {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("panel"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("panel"),
                layout: &self.panel_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.panels[i].uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.panel_sampler) },
                ],
            });
            let p = &mut self.panels[i];
            p.texture = Some(texture);
            p.bind_group = Some(bind);
        }
        let p = &mut self.panels[i];
        p.width = w;
        p.height = h;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: p.texture.as_ref().unwrap(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    fn place_panel(&self, i: usize, corner: Corner, width: u32, height: u32) {
        let r = self.panel_rect(i, corner, width, height).map(|v| v as f32);
        let (sw, sh) = (width as f32, height as f32);
        let clip_x = |x: f32| x / sw * 2.0 - 1.0;
        let clip_y = |y: f32| 1.0 - y / sh * 2.0;
        let rect = [clip_x(r[0]), clip_y(r[1]), clip_x(r[2]), clip_y(r[3])];
        self.queue.write_buffer(&self.panels[i].uniform, 0, bytemuck::cast_slice(&rect));
    }

    fn depth_view(&mut self, width: u32, height: u32) -> wgpu::TextureView {
        if let Some((view, w, h)) = &self.depth {
            if *w == width && *h == height {
                return view.clone();
            }
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: self.samples,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.depth = Some((view.clone(), width, height));
        view
    }

    /// The image to draw into when anti-aliasing, which is then resolved into the target.
    fn msaa_view(&mut self, width: u32, height: u32) -> Option<wgpu::TextureView> {
        if self.samples <= 1 {
            return None;
        }
        if let Some((view, w, h)) = &self.msaa {
            if *w == width && *h == height {
                return Some(view.clone());
            }
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("multisampled colour"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: self.samples,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.msaa = Some((view.clone(), width, height));
        Some(view)
    }

    pub fn draw(
        &mut self,
        target: &wgpu::TextureView,
        (width, height): (u32, u32),
        cam: &Camera,
        altitude: f64,
        draw: &[TileKey],
        before_submit: impl FnOnce(&mut wgpu::CommandEncoder),
    ) {
        self.frame_no += 1;
        let aspect = width as f64 / height.max(1) as f64;
        let basis = cam.basis();
        let near = cam.near_plane(altitude);
        let vec4 = |v: DVec3, w: f64| [v.x as f32, v.y as f32, v.z as f32, w as f32];
        let uniform = FrameUniform {
            view_proj: cam.view_proj(aspect, near).to_cols_array_2d(),
            cam: vec4(basis.pos / radius(), (FOV_Y / 2.0).tan()),
            right: vec4(basis.right, aspect),
            up: vec4(basis.up, 0.0),
            look: vec4(basis.look, 0.0),
            viewport: [width as f32, height as f32, 0.0, 0.0],
            sun: [self.sun.x as f32, self.sun.y as f32, self.sun.z as f32, if self.sun_on { 1.0 } else { 0.0 }],
            sky_rot: {
                let r = self.sky_rot;
                let c = |v: DVec3| [v.x as f32, v.y as f32, v.z as f32, 0.0];
                [c(r.x_axis), c(r.y_axis), c(r.z_axis), [0.0, 0.0, 0.0, 1.0]]
            },
            sky: [
                self.sky_limit as f32,
                radius() as f32,
                if crate::body::current().atmosphere { 1.0 } else { 0.0 },
                self.sky_discs.map_or(0.0, |d| d.sun_radius as f32),
            ],
            eclipse_axis: self.shadow.map_or([0.0; 4], |e| vec4(e.axis, 1.0)),
            eclipse_foot: self.shadow.map_or([0.0; 4], |e| vec4(e.foot, 0.0)),
            eclipse_cones: self.shadow.map_or([0.0; 4], |e| {
                [e.umbra_apex as f32, e.penumbra_apex as f32, e.tan_umbra as f32, e.tan_penumbra as f32]
            }),
            moon: self.sky_discs.map_or([0.0; 4], |d| vec4(d.moon, d.moon_radius)),
        };
        self.queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniform));

        let in_use: HashSet<TileKey> = draw.iter().copied().collect();
        let mut origins: Vec<[f32; 3]> = Vec::with_capacity(draw.len() + 1);
        origins.push((-basis.pos).as_vec3().to_array());
        for k in draw {
            let t = self.tiles.get_mut(k).expect("drawn tile is loaded");
            t.last_used = self.frame_no;
            origins.push((t.center - basis.pos).as_vec3().to_array());
        }
        // Building instances follow those of the tiles.
        let first_building = origins.len() as u32;
        let shown: Vec<TileKey> = self
            .visible_buildings
            .iter()
            .copied()
            .filter(|k| self.buildings.contains_key(k))
            .collect();
        for k in &shown {
            if let Some(b) = self.buildings.get_mut(k) {
                b.last_used = self.frame_no;
                origins.push((b.center - basis.pos).as_vec3().to_array());
            }
        }
        if origins.len() > self.instance_capacity {
            self.instance_capacity = origins.len().next_power_of_two();
            self.instances = create_instance_buffer(&self.device, self.instance_capacity);
        }
        self.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&origins));

        self.place_panel(0, Corner::BottomLeft, width, height);
        self.place_panel(1, Corner::TopRight, width, height);
        self.place_panel(2, Corner::TopLeft, width, height);
        self.place_panel(3, Corner::TopLeft, width, height);
        for i in UI_PANEL_BASE..self.panels.len() {
            self.place_panel(i, Corner::TopLeft, width, height);
        }
        let depth = self.depth_view(width, height);
        let msaa = self.msaa_view(width, height);
        // With anti-aliasing the target only receives the resolved image.
        let (view, resolve_target, store) = match &msaa {
            Some(m) => (m, Some(target), wgpu::StoreOp::Discard),
            None => (target, None, wgpu::StoreOp::Store),
        };

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("globe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind, &[]);
            if let Some(a) = &self.atlas {
                pass.set_pipeline(&self.atlas_pipeline);
                pass.set_vertex_buffer(0, a.vertices.slice(..));
                pass.set_index_buffer(a.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_bind_group(1, &self.underlay_bind, &[]);
                pass.set_bind_group(2, &self.no_overlay, &[]);
                pass.draw_indexed(a.underlay.clone(), 0, 0..1);
                for (k, range) in &a.tiles {
                    let Some(t) = self.tiles.get(k) else { continue };
                    let overlay = t.overlay.as_ref().filter(|_| self.roads_visible);
                    pass.set_bind_group(1, &t.bind_group, &[]);
                    pass.set_bind_group(2, overlay.unwrap_or(&self.no_overlay), &[]);
                    pass.draw_indexed(range.clone(), 0, 0..1);
                }
            } else {
                pass.set_pipeline(&self.sky_pipeline);
                pass.draw(0..3, 0..1);

                let points = (self.star_count + self.body_count) as u32;
                if self.sky_on && points > 0 {
                    pass.set_pipeline(&self.star_pipeline);
                    pass.set_vertex_buffer(0, self.sky_points.slice(..));
                    pass.draw(0..4, 0..points);
                }

                pass.set_pipeline(&self.tile_pipeline);
                pass.set_vertex_buffer(1, self.instances.slice(..));

                pass.set_bind_group(1, &self.underlay_bind, &[]);
                pass.set_bind_group(2, &self.no_overlay, &[]);
                pass.set_vertex_buffer(0, self.underlay_vertices.slice(..));
                pass.set_index_buffer(self.underlay_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.underlay_count, 0, 0..1);

                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                for (i, k) in draw.iter().enumerate() {
                    let t = &self.tiles[k];
                    let instance = i as u32 + 1;
                    let overlay = t.overlay.as_ref().filter(|_| self.roads_visible);
                    pass.set_bind_group(1, &t.bind_group, &[]);
                    pass.set_bind_group(2, overlay.unwrap_or(&self.no_overlay), &[]);
                    pass.set_vertex_buffer(0, t.vertices.slice(..));
                    pass.draw_indexed(0..self.index_count, 0, instance..instance + 1);
                }
            }

            if !shown.is_empty() {
                pass.set_pipeline(&self.building_pipeline);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                for (i, k) in shown.iter().enumerate() {
                    let b = &self.buildings[k];
                    let instance = first_building + i as u32;
                    pass.set_vertex_buffer(0, b.vertices.slice(..));
                    pass.set_index_buffer(b.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..b.index_count, 0, instance..instance + 1);
                }
            }

            if self.label_count > 0 {
                pass.set_pipeline(&self.label_pipeline);
                pass.set_bind_group(1, &self.label_bind, &[]);
                pass.set_vertex_buffer(0, self.label_instances.slice(..));
                pass.draw(0..4, 0..self.label_count);
            }

            pass.set_pipeline(&self.panel_pipeline);
            for p in &self.panels {
                if let Some(bind) = &p.bind_group {
                    pass.set_bind_group(1, bind, &[]);
                    pass.draw(0..4, 0..1);
                }
            }
        }
        before_submit(&mut encoder);
        self.queue.submit([encoder.finish()]);
        self.evict(&in_use);
    }
}

fn create_label_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("label instances"),
        size: (capacity * size_of::<LabelInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_sky_buffer(device: &wgpu::Device, stars: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sky points"),
        size: ((stars + SKY_BODIES).max(1) * size_of::<SkyPoint>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (capacity * size_of::<[f32; 3]>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Bilinear lookup on a tile's mesh heights, with `u` and `v` in 0..1.
fn height_on_grid(heights: &[f32], u: f64, v: f64) -> f64 {
    let n = GRID as f64;
    let (x, y) = ((u * n).clamp(0.0, n), (v * n).clamp(0.0, n));
    let (x0, y0) = ((x as usize).min(GRID - 1), (y as usize).min(GRID - 1));
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let at = |i: usize, j: usize| heights[j * (GRID + 1) + i] as f64;
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
    let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// A position as readable text.
pub fn hemisphere_label(lat: f64, lon: f64) -> String {
    format!(
        "{:.4}° {}  {:.4}° {}",
        lat.to_degrees().abs(),
        if lat >= 0.0 { 'N' } else { 'S' },
        lon.to_degrees().abs(),
        if lon >= 0.0 { 'E' } else { 'W' },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_height_lookup_interpolates() {
        let side = GRID + 1;
        let heights: Vec<f32> = (0..side * side).map(|i| (i % side) as f32).collect();
        assert_eq!(height_on_grid(&heights, 0.0, 0.5), 0.0);
        assert_eq!(height_on_grid(&heights, 1.0, 0.5), GRID as f64);
        assert!((height_on_grid(&heights, 0.5, 0.5) - GRID as f64 / 2.0).abs() < 1e-9);
    }

    #[test]
    fn hemisphere_labels() {
        assert_eq!(hemisphere_label(0.5f64.to_radians(), -1.25f64.to_radians()), "0.5000° N  1.2500° W");
        assert_eq!(hemisphere_label(-0.5f64.to_radians(), 1.25f64.to_radians()), "0.5000° S  1.2500° E");
    }

    #[test]
    fn underlay_covers_both_poles() {
        let (v, i) = underlay_mesh();
        let (mut north, mut south) = (false, false);
        for p in &v {
            north |= p.pos[2] > 6_356_000.0;
            south |= p.pos[2] < -6_356_000.0;
        }
        assert!(north && south);
        assert!(i.iter().all(|&n| (n as usize) < v.len()));
        // Antarctica is ice, and the Arctic cap matches the water around it.
        let south_pole = v.iter().min_by(|a, b| a.pos[2].total_cmp(&b.pos[2])).unwrap();
        let north_pole = v.iter().max_by(|a, b| a.pos[2].total_cmp(&b.pos[2])).unwrap();
        assert_eq!(south_pole.uv[1], 0.25);
        assert_eq!(north_pole.uv[1], 0.75);
    }
}
