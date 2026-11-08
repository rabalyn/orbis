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
    // The faintest magnitude that can be seen from here, the body's radius in metres, 1 when
    // the body has air, and the Sun's angular radius.
    sky: vec4<f32>,
    // The Moon's shadow: its axis towards the Sun (w is 1 when there is one), where the axis
    // crosses the plane through the Earth's centre, and the umbra's and penumbra's apex
    // distances along the axis and their slopes. Lengths are in Earth radii.
    eclipse_axis: vec4<f32>,
    eclipse_foot: vec4<f32>,
    eclipse_cones: vec4<f32>,
    // The Moon seen from the camera: its Earth-fixed direction and angular radius, zero when it
    // is not worked out. The Sun's angular radius is in `sky.w`.
    moon: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;

// The fraction of the area of a disc of radius `s` that a disc of radius `r` covers, their centres
// `d` apart. Light follows the covered area, not the covered width.
fn sun_covered(s: f32, r: f32, d: f32) -> f32 {
    if (s <= 0.0 || d >= s + r) {
        return 0.0;
    }
    if (d <= abs(r - s)) {
        return select(r * r / (s * s), 1.0, r >= s);
    }
    let ca = clamp((d * d + s * s - r * r) / (2.0 * d * s), -1.0, 1.0);
    let cb = clamp((d * d + r * r - s * s) / (2.0 * d * r), -1.0, 1.0);
    let k = (s + r - d) * (d + s - r) * (d - s + r) * (d + s + r);
    let area = s * s * acos(ca) + r * r * acos(cb) - 0.5 * sqrt(max(k, 0.0));
    return clamp(area / (3.14159265 * s * s), 0.0, 1.0);
}

// How much of the sunlight the Moon's shadow lets through at the Earth-fixed direction `g`.
// The radii follow the cones at the ground point's height along the axis (Espenak's Besselian
// L1 and L2), and the light left is the fraction of the Sun's disc that stays uncovered.
fn eclipse_light(g: vec3<f32>) -> f32 {
    if (frame.eclipse_axis.w < 0.5) {
        return 1.0;
    }
    let axis = frame.eclipse_axis.xyz;
    // The point on the flattened Earth below `g`.
    let c = 0.99664719;
    let p = g / sqrt(g.x * g.x + g.y * g.y + g.z * g.z / (c * c));
    let zeta = dot(p, axis);
    // Only the side facing the Sun.
    if (zeta <= 0.0) {
        return 1.0;
    }
    let q = p - frame.eclipse_foot.xyz;
    let m = length(q - axis * dot(q, axis));
    let l1 = max((frame.eclipse_cones.y - zeta) * frame.eclipse_cones.w, 1e-4);
    // Negative where the umbra reaches the ground, positive in the antumbra.
    let l2 = (frame.eclipse_cones.x - zeta) * frame.eclipse_cones.z;
    // With the Sun's radius s and the Moon's r as seen from here (in the cones' units), the
    // penumbra radius is s + r and the umbra's signed radius s - r, and m is their separation.
    let s = 0.5 * (l1 + l2);
    let r = 0.5 * (l1 - l2);
    return 1.0 - sun_covered(s, r, m);
}

// Cast shadows: cascaded sun-direction depth maps, drawn by shadow.wgsl. `mats` take positions
// relative to the camera to x and y in -1..1 and depth in 0..1 growing away from the Sun.
struct Shadow {
    mats: array<mat4x4<f32>, 3>,
    // The view depth where each cascade ends; w is the part of that depth over which it blends into the next.
    ends: vec4<f32>,
    // The size of a texel in metres; w is the strength (0 for no shadows).
    texel: vec4<f32>,
    // The depth of each map in metres; w is the map's width in texels.
    range: vec4<f32>,
}

@group(0) @binding(1) var<uniform> shadow: Shadow;
@group(0) @binding(2) var shadow_map: texture_depth_2d_array;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;

// How much of the Sun reaches `rel` (relative to the camera, facing `n`) through one cascade.
fn cascade_light(c: u32, rel: vec3<f32>, n: vec3<f32>, ndl: f32) -> f32 {
    let texel = shadow.texel[c];
    let sin_t = sqrt(max(1.0 - ndl * ndl, 0.0));
    let tan_t = min(sin_t / max(ndl, 0.05), 6.0);
    // Moving the point out along its normal by the texel's size, more when the light is low, keeps
    // a surface from shading itself; the small depth bias covers the rest.
    let p = rel + n * (texel * (0.5 + 1.5 * sin_t));
    let ls = shadow.mats[c] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(ls.x * 0.5 + 0.5, 0.5 - ls.y * 0.5);
    let reference = ls.z - texel * (0.7 + 0.5 * tan_t) / shadow.range[c];
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || reference < 0.0 || reference > 1.0) {
        return 1.0;
    }
    let step = 1.0 / shadow.range.w;
    var sum = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let at = uv + vec2<f32>(f32(i), f32(j)) * step;
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, at, i32(c), reference);
        }
    }
    return sum / 9.0;
}

// The share of direct sunlight left at `rel` after the shadows cast by terrain and buildings.
fn shadow_light(rel: vec3<f32>, n: vec3<f32>) -> f32 {
    let ndl = dot(n, frame.sun.xyz);
    if (shadow.texel.w <= 0.0 || ndl <= 0.0) {
        return 1.0;
    }
    let d = dot(rel, frame.look.xyz);
    if (d >= shadow.ends.z) {
        return 1.0;
    }
    var c = 0u;
    if (d > shadow.ends.x) {
        c = 1u;
    }
    if (d > shadow.ends.y) {
        c = 2u;
    }
    var light = cascade_light(c, rel, n, ndl);
    if (c < 2u) {
        // Blend into the next cascade over the last part of this one.
        let end = shadow.ends[c];
        let start = end * (1.0 - shadow.ends.w);
        if (d > start) {
            light = mix(light, cascade_light(c + 1u, rel, n, ndl), smoothstep(start, end, d));
        }
    } else {
        // Past the last cascade the shadows fade out.
        light = mix(1.0, light, 1.0 - smoothstep(0.85 * shadow.ends.z, shadow.ends.z, d));
    }
    return mix(1.0, light, shadow.texel.w);
}

// Sunlight. `n` is the surface normal, `g` the normal of the smooth Earth below it. Day
// and night follow `g`, with a soft twilight; slopes follow `n`.
fn sun_light(n: vec3<f32>, g: vec3<f32>, rel: vec3<f32>) -> vec3<f32> {
    if (frame.sun.w < 0.5) {
        return vec3<f32>(1.0);
    }
    let s = frame.sun.xyz;
    if (frame.sky.z < 0.5) {
        // No air: a sharp terminator, white light, and only a trace of light on the night side.
        let day = smoothstep(0.0, 0.01, dot(g, s));
        var lit = 1.0;
        if (day > 0.0) {
            lit = shadow_light(rel, n);
        }
        return vec3<f32>(0.012) + 0.95 * max(dot(n, s), 0.0) * day * lit;
    }
    let high = dot(g, s);
    let shade = eclipse_light(g);
    let day = smoothstep(-0.02, 0.08, high);
    // Shadows take away direct sunlight only; the sky light stays.
    var lit = 1.0;
    if (day > 0.0) {
        lit = shadow_light(rel, n);
    }
    let direct = max(dot(n, s), 0.0) * day * lit;
    // Sky light fades out until the Sun is about 14 degrees below the horizon.
    let sky = smoothstep(-0.25, 0.1, high);
    let ambient = mix(vec3<f32>(0.045, 0.06, 0.11), vec3<f32>(0.22, 0.24, 0.28), sky);
    // Low sunlight is warm.
    let warm = mix(vec3<f32>(1.0, 0.62, 0.38), vec3<f32>(1.0), smoothstep(0.0, 0.35, high));
    // The sky light is sunlight scattered, so it dims with the Sun; a trace stays in totality.
    return ambient * mix(0.03, 1.0, shade) + 0.8 * warm * direct * shade;
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
    // Position relative to the camera.
    @location(4) rel: vec3<f32>,
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
    out.rel = rel;
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
    out.rel = vec3<f32>(0.0);
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
    colour = colour * sun_light(normalize(in.normal), normalize(in.ground), in.rel);
    return vec4<f32>(colour, 1.0);
}

// Buildings, flat-shaded from the slope of each face

struct BuildingOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec3<f32>,
    // Position relative to the camera.
    @location(1) rel: vec3<f32>,
    // Opacity, for the see-through parts of the energy bars.
    @location(2) alpha: f32,
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
    out.alpha = colour.a;
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
    return vec4<f32>(in.colour * sun_light(n, g, in.rel), 1.0);
}

// The see-through parts of the energy bars: lit like buildings, but never darker than half the
// flat colour, so that they stay legible at night, and blended by their opacity.
@fragment
fn fs_building_ghost(in: BuildingOut) -> @location(0) vec4<f32> {
    var n = normalize(cross(dpdx(in.rel), dpdy(in.rel)));
    if (dot(n, in.rel) > 0.0) {
        n = -n;
    }
    let g = normalize(in.rel + frame.cam.xyz * frame.sky.y);
    let lit = max(in.colour * sun_light(n, g, in.rel), in.colour * 0.5);
    return vec4<f32>(lit, in.alpha);
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

// The share of the Sun's light that is left while the Moon covers part of its disc.
fn sun_daylight() -> f32 {
    if (frame.moon.w <= 0.0 || frame.sun.w < 0.5) {
        return 1.0;
    }
    let d = atan2(length(cross(frame.moon.xyz, frame.sun.xyz)), dot(frame.moon.xyz, frame.sun.xyz));
    return 1.0 - sun_covered(frame.sky.w, frame.moon.w, d);
}

// Air seen from inside the atmosphere: daylight blue, the warm glow of a low Sun, then night.
fn sky_inside(dir: vec3<f32>, up: vec3<f32>, light: f32) -> vec3<f32> {
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
    // The Sun's light, and with it the sky's, fails as the Moon covers the Sun; a trace of the
    // horizon's glow stays in totality.
    let dusk = vec3<f32>(0.9, 0.4, 0.2) * band * 0.015 * (1.0 - light);
    return night + (day * k * k + glow * 0.8) * light + dusk * k;
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
    let air = frame.sky.z;
    let light = sun_daylight();
    let glow = exp(-height / 0.012) * air;
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
    let inside = exp(-max(altitude, 0.0) / 15000.0) * air;
    if (inside > 0.002) {
        colour = mix(colour, sky_inside(dir, normalize(o), light), inside);
    }
    if (frame.sun.w > 0.5) {
        // The Sun's disc and the bright halo around it.
        let cg = dot(dir, frame.sun.xyz);
        if (cg > 0.0) {
            let a = length(cross(dir, frame.sun.xyz));
            let low = exp(-pow(max(dot(normalize(o), frame.sun.xyz), -0.1), 2.0) / 0.02);
            let tint = mix(vec3<f32>(1.0, 0.96, 0.88), vec3<f32>(1.0, 0.5, 0.22), low);
            // The disc has the Sun's true size where it is known, so the Moon can cover it exactly.
            let sr = select(0.0047, frame.sky.w, frame.sky.w > 0.0);
            let core = 1.0 - smoothstep(0.9 * sr, sr, a);
            colour = colour + tint * (core * 6.0 + exp(-a / 0.012) * 0.35 * inside * light);
        }
    }
    if (frame.moon.w > 0.0 && dot(dir, frame.moon.xyz) > 0.0) {
        // The Moon is opaque: it hides the sun and the sky behind it, and in totality the
        // corona shows around it.
        let cm = length(cross(dir, frame.moon.xyz));
        let px = 2.0 * frame.cam.w / frame.viewport.y;
        let edge = smoothstep(frame.moon.w - 0.5 * px, frame.moon.w + 0.5 * px, cm);
        let total = smoothstep(0.97, 1.0, 1.0 - light);
        let corona = vec3<f32>(0.85, 0.88, 1.0) * total * 0.8 * exp(-max(cm - frame.moon.w, 0.0) / 0.008);
        colour = colour * edge + corona * edge;
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
