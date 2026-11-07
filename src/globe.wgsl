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
    // Direction to the Sun in the Earth-fixed frame; w is 1 with sunlight and 0 for flat light.
    sun: vec4<f32>,
    // From J2000 equatorial directions to the Earth-fixed frame.
    sky_rot: mat4x4<f32>,
    // The faintest magnitude that can be seen from here, the body's radius in metres, and 1
    // when the body has air.
    sky: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;

// Sunlight. `n` is the surface normal, `g` the normal of the smooth Earth below it. Day
// and night follow `g`, with a soft twilight; slopes follow `n`.
fn sun_light(n: vec3<f32>, g: vec3<f32>) -> vec3<f32> {
    if (frame.sun.w < 0.5) {
        return vec3<f32>(1.0);
    }
    let s = frame.sun.xyz;
    let high = dot(g, s);
    let direct = max(dot(n, s), 0.0) * smoothstep(-0.02, 0.08, high);
    // Sky light fades out until the Sun is about 14 degrees below the horizon.
    let sky = smoothstep(-0.25, 0.1, high);
    let ambient = mix(vec3<f32>(0.045, 0.06, 0.11), vec3<f32>(0.22, 0.24, 0.28), sky);
    // Low sunlight is warm.
    let warm = mix(vec3<f32>(1.0, 0.62, 0.38), vec3<f32>(1.0), smoothstep(0.0, 0.35, high));
    return ambient + 0.8 * warm * direct;
}

// Tiles

@group(1) @binding(0) var tile_tex: texture_2d<f32>;
@group(1) @binding(1) var tile_sampler: sampler;
@group(2) @binding(0) var overlay_tex: texture_2d<f32>;
@group(2) @binding(1) var overlay_sampler: sampler;

struct TileOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) st: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) ground: vec3<f32>,
}

@vertex
fn vs_tile(
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) st: vec2<f32>,
    @location(3) origin: vec3<f32>,
    @location(4) normal: vec3<f32>,
) -> TileOut {
    var out: TileOut;
    let rel = pos + origin;
    out.clip = frame.view_proj * vec4<f32>(rel, 1.0);
    out.uv = uv;
    out.st = st;
    out.normal = normal;
    out.ground = normalize(rel + frame.cam.xyz * frame.sky.y);
    return out;
}

// The flat map: vertices are already in clip space, so tiles share the fragment stage.
@vertex
fn vs_atlas(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) st: vec2<f32>,
    @location(3) normal: vec3<f32>,
) -> TileOut {
    var out: TileOut;
    out.clip = vec4<f32>(pos, 0.0, 1.0);
    out.uv = uv;
    out.st = st;
    out.normal = normal;
    out.ground = normal;
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
    colour = colour * sun_light(normalize(in.normal), normalize(in.ground));
    return vec4<f32>(colour, 1.0);
}

// Buildings, flat-shaded from the slope of each face

struct BuildingOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec3<f32>,
    // Position relative to the camera.
    @location(1) rel: vec3<f32>,
}

@vertex
fn vs_building(
    @location(0) pos: vec3<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) origin: vec3<f32>,
) -> BuildingOut {
    var out: BuildingOut;
    out.rel = pos + origin;
    out.clip = frame.view_proj * vec4<f32>(out.rel, 1.0);
    out.colour = srgb_to_linear(colour.rgb);
    return out;
}

@fragment
fn fs_building(in: BuildingOut) -> @location(0) vec4<f32> {
    var n = normalize(cross(dpdx(in.rel), dpdy(in.rel)));
    // Faces are drawn from the outside, so the normal faces the camera.
    if (dot(n, in.rel) > 0.0) {
        n = -n;
    }
    let g = normalize(in.rel + frame.cam.xyz * frame.sky.y);
    return vec4<f32>(in.colour * sun_light(n, g), 1.0);
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

// Air seen from inside the atmosphere: daylight blue, the warm glow of a low Sun, then night.
fn sky_inside(dir: vec3<f32>, up: vec3<f32>) -> vec3<f32> {
    let e = dot(dir, up);
    var s = 1.0;
    var cg = -1.0;
    if (frame.sun.w > 0.5) {
        s = dot(up, frame.sun.xyz);
        cg = dot(dir, frame.sun.xyz);
    }
    // Daylight, from the end of nautical twilight to a Sun about nine degrees up.
    let k = smoothstep(-0.2, 0.15, s);
    let zenith = vec3<f32>(0.03, 0.14, 0.50);
    let horizon = vec3<f32>(0.34, 0.58, 0.90);
    let day = mix(horizon, zenith, pow(clamp(e, 0.0, 1.0), 0.45));
    let band = exp(-max(e, 0.0) / 0.12);
    let low = exp(-s * s / 0.02);
    let toward = clamp(cg * 0.5 + 0.5, 0.0, 1.0);
    let warm = vec3<f32>(1.0, 0.30, 0.07);
    let glow = warm * low * (band * (0.1 + 0.9 * pow(toward, 4.0)) + 0.8 * exp((cg - 1.0) / 0.04));
    let night = vec3<f32>(0.0015, 0.003, 0.009) + vec3<f32>(0.003, 0.005, 0.012) * band;
    return night + day * k * k + glow * 0.8;
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
    var colour = vec3<f32>(0.22, 0.45, 0.95) * glow;
    if (frame.sun.w > 0.5) {
        // The glow is lit where the Sun shines on the air at the ray's closest approach.
        let q = normalize(o + dir * along);
        let high = dot(q, frame.sun.xyz);
        let lit = smoothstep(-0.3, 0.25, high);
        let dusk = exp(-high * high / 0.02);
        colour = colour * mix(0.04, 1.0, lit) + vec3<f32>(0.9, 0.38, 0.15) * glow * dusk * 0.5;
    }
    // Close to the ground the whole sky is air; the weight fades out over 15 km.
    let altitude = (length(o) - 1.0) * frame.sky.y;
    let inside = exp(-max(altitude, 0.0) / 15000.0);
    if (inside > 0.002) {
        colour = mix(colour, sky_inside(dir, normalize(o)), inside);
    }
    if (frame.sun.w > 0.5) {
        // The Sun's disc and the bright halo around it.
        let cg = dot(dir, frame.sun.xyz);
        if (cg > 0.0) {
            let a = length(cross(dir, frame.sun.xyz));
            let low = exp(-pow(max(dot(normalize(o), frame.sun.xyz), -0.1), 2.0) / 0.02);
            let tint = mix(vec3<f32>(1.0, 0.96, 0.88), vec3<f32>(1.0, 0.5, 0.22), low);
            let core = 1.0 - smoothstep(0.0042, 0.0052, a);
            colour = colour + tint * (core * 6.0 + exp(-a / 0.012) * 0.35 * inside);
        }
    }
    return vec4<f32>(colour, 1.0);
}

// Stars, planets and the Moon, drawn additively on the sky

struct StarOut {
    @builtin(position) clip: vec4<f32>,
    // Offset from the centre in units of half the quad's size, y up.
    @location(0) local: vec2<f32>,
    @location(1) colour: vec3<f32>,
    // The point's radius and the quad's half size in pixels, the brightness, and 1 for a disc.
    @location(2) shape: vec4<f32>,
    @location(3) dir: vec3<f32>,
}

@vertex
fn vs_star(
    @builtin(vertex_index) i: u32,
    @location(0) pos_mag: vec4<f32>,
    @location(1) colour_radius: vec4<f32>,
) -> StarOut {
    var out: StarOut;
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    let d = normalize((frame.sky_rot * vec4<f32>(pos_mag.xyz, 0.0)).xyz);
    let depth = dot(d, frame.look.xyz);
    if (depth < 0.02) {
        return out;
    }
    let mag = pos_mag.w;
    let fade = clamp((frame.sky.x - mag) / 1.5, 0.0, 1.0);
    if (fade <= 0.0) {
        return out;
    }
    let t = frame.cam.w;
    let ndc = vec2<f32>(
        dot(d, frame.right.xyz) / (depth * t * frame.right.w),
        dot(d, frame.up.xyz) / (depth * t),
    );
    var radius = clamp(0.9 + 0.55 * max(4.0 - mag, 0.0), 0.9, 5.5);
    var bright = (0.2 + 0.8 * pow(clamp((6.6 - mag) / 6.6, 0.0, 1.0), 1.5)) * fade;
    var disc = 0.0;
    if (colour_radius.w > 0.0) {
        radius = max(colour_radius.w * 0.5 * frame.viewport.y / (t * depth * depth), 5.0);
        bright = fade;
        disc = 1.0;
    }
    let half_px = radius + 2.0;
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u)) * 2.0 - 1.0;
    let off = corner * half_px * 2.0 / frame.viewport.xy;
    out.clip = vec4<f32>(ndc + off, 0.0, 1.0);
    out.local = corner;
    out.colour = colour_radius.xyz;
    out.shape = vec4<f32>(radius, half_px, bright, disc);
    out.dir = d;
    return out;
}

@fragment
fn fs_star(in: StarOut) -> @location(0) vec4<f32> {
    let p = in.local * in.shape.y;
    let r = length(p);
    var colour = in.colour;
    var cover = exp(-r * r / (in.shape.x * in.shape.x * 0.5));
    if (in.shape.w > 0.5) {
        cover = clamp(in.shape.x - r + 0.5, 0.0, 1.0);
        // The Moon is a ball lit by the Sun: its normal here, then the light on it.
        let q = p / in.shape.x;
        let z = sqrt(max(1.0 - dot(q, q), 0.0));
        let n = frame.right.xyz * q.x + frame.up.xyz * q.y - in.dir * z;
        var lit = 1.0;
        if (frame.sun.w > 0.5) {
            lit = smoothstep(-0.03, 0.03, dot(n, frame.sun.xyz));
        }
        colour = colour * (0.04 + 0.96 * lit);
    }
    return vec4<f32>(colour * cover * in.shape.z, 1.0);
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
