@group(0) @binding(20) var reef_blades: texture_2d<f32>;

// Intersect the refracted sunlight with the four depths in the local canopy.
// Coordinates follow the same world-aligned 6 m tile as the habitat instances.
fn reef_transmission(p: vec3f, sun: vec3f) -> vec2f {
    let occupied = u32(wu.reach.y);
    if occupied == 0u { return vec2f(1.); }
    let water_depth = wu.grid.z - p.y;
    let origin = floor(wu.cam.xz/6.)*6. - 12.;
    let depth = vec4f(0.85,2.55,5.1,8.2);
    var direct = 1.;
    var diffuse = 1.;
    let ray = sun.xz/max(sun.y,0.25);
    let sway = vec2f(0.20,0.10)*sin(0.83*wu.cam.w + 0.031*p.x + 0.018*p.z);
    for (var k=0u; k<4u; k++) {
        let dz = water_depth-depth[k];
        if dz <= -0.45 { break; }
        if (occupied & (1u << k)) == 0u { continue; }
        let uv = (p.xz + ray*max(dz,0.) - origin - sway)/30.;
        let inside = min(uv,1.-uv);
        let edge = smoothstep(0.,0.075,min(inside.x,inside.y));
        let cover = textureSampleLevel(reef_blades,w_clamp,uv,0.)[k]
                    * edge * smoothstep(-0.45,0.45,dz);
        direct *= 1.-0.94*cover;
        // Scattered skylight arrives from the surrounding hemisphere as well.
        diffuse *= 1.-0.38*cover;
    }
    return vec2f(direct,diffuse);
}
