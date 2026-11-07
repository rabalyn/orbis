// The depth-only passes of the shadow maps: terrain tiles and buildings seen from the Sun.
// Positions are relative to the camera, like in globe.wgsl; `cascade.m` is from src/shadows.rs.

struct Cascade {
    m: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> cascade: Cascade;

@vertex
fn vs_tile(@location(0) pos: vec3<f32>, @location(3) origin: vec3<f32>) -> @builtin(position) vec4<f32> {
    return cascade.m * vec4<f32>(pos + origin, 1.0);
}

@vertex
fn vs_building(@location(0) pos: vec3<f32>, @location(2) origin: vec3<f32>) -> @builtin(position) vec4<f32> {
    return cascade.m * vec4<f32>(pos + origin, 1.0);
}
