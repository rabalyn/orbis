struct Frame {
    view_proj: mat4x4<f32>,
    // Camera position in Earth radii; w is the tangent of half the vertical field of view.
    cam: vec4<f32>,
    // Camera axes; right.w is the aspect ratio.
    right: vec4<f32>,
    up: vec4<f32>,
    look: vec4<f32>,
    // Window size in pixels.
    viewport: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;

// Tiles

@group(1) @binding(0) var tile_tex: texture_2d<f32>;
@group(1) @binding(1) var tile_sampler: sampler;
@group(2) @binding(0) var overlay_tex: texture_2d<f32>;
@group(2) @binding(1) var overlay_sampler: sampler;

struct TileOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) st: vec2<f32>,
}

@vertex
fn vs_tile(
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) st: vec2<f32>,
    @location(3) origin: vec3<f32>,
) -> TileOut {
    var out: TileOut;
    out.clip = frame.view_proj * vec4<f32>(pos + origin, 1.0);
    out.uv = uv;
    out.st = st;
    return out;
}

// The flat map: vertices are already in clip space, so tiles share the fragment stage.
@vertex
fn vs_atlas(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) st: vec2<f32>,
) -> TileOut {
    var out: TileOut;
    out.clip = vec4<f32>(pos, 0.0, 1.0);
    out.uv = uv;
    out.st = st;
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_tile(in: TileOut) -> @location(0) vec4<f32> {
    let base = textureSample(tile_tex, tile_sampler, in.uv).rgb;
    // The overlay holds premultiplied sRGB values, undone here to blend in linear light.
    let line = textureSample(overlay_tex, overlay_sampler, in.st);
    var colour = base;
    if (line.a > 0.002) {
        colour = mix(base, srgb_to_linear(clamp(line.rgb / line.a, vec3<f32>(0.0), vec3<f32>(1.0))), line.a);
    }
    return vec4<f32>(colour, 1.0);
}

// Buildings, whose colours already include their shading

struct BuildingOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec3<f32>,
}

@vertex
fn vs_building(
    @location(0) pos: vec3<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) origin: vec3<f32>,
) -> BuildingOut {
    var out: BuildingOut;
    out.clip = frame.view_proj * vec4<f32>(pos + origin, 1.0);
    out.colour = srgb_to_linear(colour.rgb);
    return out;
}

@fragment
fn fs_building(in: BuildingOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.colour, 1.0);
}

// Text labels

@group(1) @binding(0) var label_tex: texture_2d<f32>;
@group(1) @binding(1) var label_sampler: sampler;

struct LabelOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) halo: vec3<f32>,
}

@vertex
fn vs_label(
    @builtin(vertex_index) i: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) uv_rect: vec4<f32>,
    @location(3) fill: vec4<f32>,
    // The halo's colour, with the label's angle in the last component.
    @location(4) halo: vec4<f32>,
) -> LabelOut {
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    let local = (corner * 2.0 - 1.0) * half_size;
    let c = cos(halo.w);
    let s = sin(halo.w);
    let px = center + vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    var out: LabelOut;
    out.clip = vec4<f32>(px.x / frame.viewport.x * 2.0 - 1.0, 1.0 - px.y / frame.viewport.y * 2.0, 0.0, 1.0);
    out.uv = mix(uv_rect.xy, uv_rect.zw, corner);
    out.fill = fill;
    out.halo = halo.xyz;
    return out;
}

@fragment
fn fs_label(in: LabelOut) -> @location(0) vec4<f32> {
    let mask = textureSample(label_tex, label_sampler, in.uv);
    let alpha = max(mask.r, mask.g * 0.75) * in.fill.a;
    return vec4<f32>(mix(in.halo, in.fill.rgb, mask.r), alpha);
}

// Atmosphere, drawn behind everything as a glow around the Earth

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: SkyOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let t = frame.cam.w;
    let dir = normalize(
        frame.look.xyz
        + frame.right.xyz * in.ndc.x * t * frame.right.w
        + frame.up.xyz * in.ndc.y * t
    );
    let o = frame.cam.xyz;
    // The ray's closest approach to the Earth's centre, in front of the camera.
    let along = max(0.0, -dot(o, dir));
    let height = max(length(o + dir * along) - 1.0, 0.0);
    let glow = exp(-height / 0.012);
    let colour = vec3<f32>(0.22, 0.45, 0.95) * glow;
    return vec4<f32>(colour, 1.0);
}

// Text panels

struct Panel {
    // Left, top, right and bottom in clip space.
    rect: vec4<f32>,
}

@group(1) @binding(0) var<uniform> panel: Panel;
@group(1) @binding(1) var panel_tex: texture_2d<f32>;
@group(1) @binding(2) var panel_sampler: sampler;

struct PanelOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_panel(@builtin(vertex_index) i: u32) -> PanelOut {
    let u = f32(i & 1u);
    let v = f32((i >> 1u) & 1u);
    var out: PanelOut;
    out.clip = vec4<f32>(mix(panel.rect.x, panel.rect.z, u), mix(panel.rect.y, panel.rect.w, v), 0.0, 1.0);
    out.uv = vec2<f32>(u, v);
    return out;
}

@fragment
fn fs_panel(in: PanelOut) -> @location(0) vec4<f32> {
    return textureSample(panel_tex, panel_sampler, in.uv);
}
