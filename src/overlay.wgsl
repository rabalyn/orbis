// Draws a tile's lines into its overlay image, and copies images: one over another, or into
// the next smaller mip level.

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// Positions are in overlay texels, y down; colours are premultiplied.
@vertex
fn vs_line(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>) -> LineOut {
    var out: LineOut;
    let size = 512.0;
    out.pos = vec4<f32>(pos.x / size * 2.0 - 1.0, 1.0 - pos.y / size * 2.0, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return in.color;
}

struct CopyOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

// One triangle that covers the whole target.
@vertex
fn vs_copy(@builtin(vertex_index) i: u32) -> CopyOut {
    var out: CopyOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@fragment
fn fs_copy(in: CopyOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(source, source_sampler, in.uv, 0.0);
}
