// The sun bounce (`--sun-bounce`, SF_SUN_BOUNCE; eo-look `sun_bounce`, its
// CPU reference and the reasoning): the sun's first bounce off the sunlit
// walls round a point, gathered by the occlusion pass (surfaces_ao.wgsl;
// this file is appended only with the feature compiled in) from the far sun
// shadow map's ring 1 as a reflective shadow map (Dachsbacher and
// Stamminger, I3D 2005). Every texel of the map is the first surface the sun
// meets in a beam of the texel's cross-section a; a sunlit wall there sends
// a point (rho E_n - hemi_ground / cos_s) a cos_x cos_p / (pi d^2): its own
// radiance less the rig's stand-in for those directions (the ground lobe,
// which with the ambient volume also lights the buildings that hide the sky).
// The ground and roofs are left to the rig's lobe and the ground bounce. The
// result is E / pi (the bounce channel's units: outgoing radiance = it x the
// diffuse albedo), added to the screen-space bounce; every lit surface spends
// it on its indirect diffuse the next frame (look_light.wgsl `ao_fetch`).
//
// Ring 1's rows (far_shadow.wgsl): look.lx[1..3] world -> its clip space
// (orthogonal rows: lx, ly across the beam / the half extents, -lz along it /
// the depth range), lx[4] (front layer, normal offset, tap uv, fade-out
// distance), lx[9] (rings drawn, edge fade, fade start share).
//
// The map holds no visibility between the point and the wall, and no
// albedo: walls take SB_WALL_ALBEDO.

@group(0) @binding(5) var ao_far_map: texture_depth_2d_array;

// A point of ring 1's map at texel `t` (its depth read there): world position.
// The rows are orthogonal, so world = sum of row_i (c_i - w_i) / |row_i|^2.
fn sb_point(t: vec2<i32>, dims: vec2<f32>, layer: i32) -> vec4<f32> {
    let d = textureLoad(ao_far_map, t, layer, 0);
    let uv = (vec2<f32>(t) + 0.5) / dims;
    let c = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d);
    let r0 = look.lx[1];
    let r1 = look.lx[2];
    let r2 = look.lx[3];
    let p = r0.xyz * ((c.x - r0.w) / dot(r0.xyz, r0.xyz)) + r1.xyz * ((c.y - r1.w) / dot(r1.xyz, r1.xyz)) + r2.xyz * ((c.z - r2.w) / dot(r2.xyz, r2.xyz));
    return vec4<f32>(p, d);
}

// How much of a point the gather reaches: ring 1's own fade with the ground
// distance from the eye (far_shadow.wgsl), inside its box.
fn sb_cover_at(world: vec3<f32>) -> f32 {
    let prm = look.lx[4];
    if (look.lx[9].x < 0.5 || prm.w <= 0.0) {
        return 0.0;
    }
    let d = length(world.xz - look.cam_pos.xz);
    return 1.0 - lk_smooth(prm.w * look.lx[9].z, prm.w, d);
}

// The sun's bounce at a world point with normal n, as E / pi. `rot` turns the
// spiral (radians), `jit` the strata (0..1). `near` 1 where the screen-space
// bounce gathers the near field (its share, d^2 / R^2, is left to it; R
// `sb_near_m`, the 4 m search's, or `--screen-gi`'s reach where it gathers).
var<private> sb_near_m: f32 = AO_RADIUS_M;
// The samples a point gathers this frame: SB_SAMPLES, or SB_SAMPLES_WITH_HISTORY
// where the screen-space gather's history (`--screen-gi`) holds the result.
var<private> sb_samples: i32 = SB_SAMPLES;
fn ao_sun_bounce(world: vec3<f32>, n: vec3<f32>, rot: f32, jit: f32, near: bool) -> vec3<f32> {
    if ((u32(look.ao_params.w + 0.5) & 1u) == 0u || look.key_dir.w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let cover = sb_cover_at(world);
    if (cover <= 0.0) {
        return vec3<f32>(0.0);
    }
    let r0 = look.lx[1];
    let r1 = look.lx[2];
    let r2 = look.lx[3];
    let layer = i32(look.lx[4].x + 0.5);
    let dims = vec2<f32>(textureDimensions(ao_far_map));
    let pw = vec4<f32>(world, 1.0);
    let c = vec2<f32>(dot(r0, pw), dot(r1, pw));
    let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec3<f32>(0.0);
    }
    // metres per uv unit (2 x the half extents) and a baseline of about a metre for the normals.
    let l0 = length(r0.xyz);
    let l1 = length(r1.xyz);
    let m_uv = vec2<f32>(2.0 / l0, 2.0 / l1);
    let texel_m = m_uv / dims;
    let k = vec2<i32>(max(vec2<f32>(1.0), round(vec2<f32>(1.0) / texel_m)));
    let to_sun = -normalize(r2.xyz);
    let range_m = 1.0 / length(r2.xyz);
    let maxt = vec2<i32>(dims) - 1 - k;
    var e = vec3<f32>(0.0);
    let wall = SB_WALL_ALBEDO * look.key.xyz;
    let stand_in = look.hemi_ground.xyz;
    let n_s = f32(sb_samples);
    let sq_r = sqrt(SB_RADIUS_M);
    for (var i = 0; i < sb_samples; i = i + 1) {
        let u = (f32(i) + jit) / n_s;
        let r = SB_RADIUS_M * u * u;
        let a = f32(i) * SB_GOLDEN_ANGLE + rot;
        let off = r * vec2<f32>(cos(a), sin(a));
        let suv = uv + vec2<f32>(off.x, -off.y) / m_uv;
        let t = vec2<i32>(floor(suv * dims));
        if (any(t < vec2<i32>(0)) || any(t > maxt)) {
            continue;
        }
        let x0 = sb_point(t, dims, layer);
        if (x0.w >= 1.0) {
            continue; // nothing drawn there
        }
        let x1 = sb_point(t + vec2<i32>(k.x, 0), dims, layer).xyz - x0.xyz;
        let x2 = sb_point(t + vec2<i32>(0, k.y), dims, layer).xyz - x0.xyz;
        // a jump along the beam over the baseline (a roof's edge to the street) is no surface.
        let jx = abs(dot(x1, to_sun));
        let jy = abs(dot(x2, to_sun));
        if (jx > SB_MAX_SLOPE * f32(k.x) * texel_m.x || jy > SB_MAX_SLOPE * f32(k.y) * texel_m.y) {
            continue;
        }
        var nx = normalize(cross(x1, x2));
        if (dot(nx, to_sun) < 0.0) {
            nx = -nx;
        }
        let cs = dot(nx, to_sun);
        if (cs < SB_MIN_SUN_COS || nx.y > SB_GROUND_COS) {
            continue; // turned from the sun; the ground and roofs are the ground bounce's
        }
        let dv = x0.xyz - world;
        let d2 = dot(dv, dv);
        if (d2 < 1e-6) {
            continue;
        }
        let dir = dv * inverseSqrt(d2);
        let cp = dot(n, dir);
        let cx = -dot(nx, dir);
        if (cp <= 0.0 || cx <= 0.0) {
            continue;
        }
        let area = 4.0 * LK_PI * r * sqrt(r) * sq_r / n_s;
        var w = cp * cx * area / max(max(d2, area), SB_MIN_DIST2);
        if (near) {
            w = w * min(d2 / (sb_near_m * sb_near_m), 1.0);
        }
        // the wall's radiance less the stand-in's (x pi), at least 0.
        e = e + max(wall - stand_in / cs, vec3<f32>(0.0)) * w;
    }
    // cos cos a / (pi d^2) x pi L; the channel holds E / pi.
    return e * (cover / (LK_PI * LK_PI));
}
