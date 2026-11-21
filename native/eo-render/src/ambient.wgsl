// The ambient volume (`LightingOptions::ambient_volume`; ambient.rs,
// ambient_bake.rs): large-scale sky visibility, read by `look_light` while
// LX_BIT_AMBIENT_VOLUME is set and the plane is resident (`look.lx[11].z`,
// its cell; 0 until it has landed). A visibility is `(a0, ax, ay, az)`: the
// visible sky's share of a whole hemisphere's irradiance on a surface with
// normal n is a0 + dot((ax, ay, az), n); AMB_OPEN is the open upper
// hemisphere, with which the shading is the frame without the volume. Only
// the sky terms take it (the hemisphere's sky part, the environment's
// diffuse, the specular environment); the ground lobe, the ambient term,
// the screen-space occlusion and its bounce stay as they are.
//
// Bindings (look_ext.wgsl): 13 the volume, AMB_VOLUME_TEXELS a side, a ring
// of 128 m tiles per level read with the repeat sampler (sky_rep) straight
// from world x and z, one slab of AMB_LAYERS terrain-following layers per
// level; 14 the citywide plane, AMB_PLANE_TEXELS a side, (a0, ay, ground,
// reach) at street level, read with the clamping sampler (lk_ao_samp).
// Rows: look.lx[10] the far and near levels' resident rectangles (two tile
// indices packed in each float), look.lx[11] the plane's corner (xy), its
// cell (z, 0: inert) and 1 / the volume's depth in layers (w).
//
// Both sizes are constants, and some pipelines read fewer levels, or none
// (AMB_SHADER_LEVELS, AMB_SHADER_ON; ambient.rs `ShaderLevels`): on Apple's
// GPU compiler some fragment shaders that discard (props' cut-outs, the far
// cells) stand at the edge of what compiles, and a texture size query, a
// mapping from sizes in the uniform or one level of the volume more made
// their pipelines fail with XPC_ERROR_CONNECTION_INTERRUPTED (measured,
// stage B3).

const AMB_OPEN: vec4<f32> = vec4<f32>(0.5, 0.0, 0.5, 0.0);

fn amb_on() -> bool {
    return AMB_SHADER_ON && lk_lx_on(LX_BIT_AMBIENT_VOLUME) && look.lx[11].z > 0.0;
}

// A level's resident rectangle (min x, min z, max x, max z, metres) from its
// two packed tile indices ((x + 512) * 1024 + z + 512, exact in a float;
// none: both 0).
fn amb_rect(a: f32, b: f32) -> vec4<f32> {
    let p = floor(vec2<f32>(a, b) + 0.5);
    let x = floor(p / 1024.0);
    let z = p - x * 1024.0;
    return (vec4<f32>(x.x, z.x, x.y, z.y) - 512.0) * AMB_TILE_M;
}

// 1 inside a rectangle, easing to 0 over AMB_EDGE_FADE_M to its edge (0 outside it, and with none).
fn amb_edge(r: vec4<f32>, p: vec2<f32>) -> f32 {
    let d = min(min(p.x - r.x, r.z - p.x), min(p.y - r.y, r.w - p.y));
    return lk_smooth(0.0, AMB_EDGE_FADE_M, d);
}

// The continuous layer coordinate (0 .. AMB_LAYERS - 1) of a height over the
// ground: AMB_FINE_LAYERS layers of AMB_FINE_M, then layers of AMB_COARSE_M
// (ambient_bake.rs `layer_centre`), linear between the layers' centres.
fn amb_layer(h: f32) -> f32 {
    let c_fine = (f32(AMB_FINE_LAYERS) - 0.5) * AMB_FINE_M;
    let c_coarse = f32(AMB_FINE_LAYERS) * AMB_FINE_M + 0.5 * AMB_COARSE_M;
    if (h <= c_fine) {
        return max((h - 0.5 * AMB_FINE_M) / AMB_FINE_M, 0.0);
    }
    if (h <= c_coarse) {
        return f32(AMB_FINE_LAYERS - 1u) + (h - c_fine) / (c_coarse - c_fine);
    }
    return min(f32(AMB_FINE_LAYERS) + (h - c_coarse) / AMB_COARSE_M, f32(AMB_LAYERS - 1u));
}

// The plane at a point: (a0, ay, ground, reach); off the plane, the open sky over its edge's ground.
fn amb_plane(xz: vec2<f32>) -> vec4<f32> {
    let n = f32(AMB_PLANE_TEXELS);
    let t = (xz - look.lx[11].xy) / AMB_PLANE_CELL_M;
    let uv = clamp(t, vec2<f32>(0.5), vec2<f32>(n - 0.5)) / n;
    let p = textureSampleLevel(lx_ambient_plane, lk_ao_samp, uv, 0.0);
    if (any(t < vec2<f32>(0.0)) || any(t > vec2<f32>(n))) {
        return vec4<f32>(0.5, 0.5, p.z, 1.0);
    }
    return p;
}

// One level's visibility h metres over the ground (over the top layer, the
// top layer eased to the open sky by the plane's reach).
fn amb_level(xz: vec2<f32>, h: f32, slab: u32, cell: f32, reach: f32) -> vec4<f32> {
    let uv = fract(xz / (cell * f32(AMB_VOLUME_TEXELS)));
    let w = (f32(slab * AMB_LAYERS) + amb_layer(h) + 0.5) * look.lx[11].w;
    let s = textureSampleLevel(lx_ambient_volume, sky_rep, vec3<f32>(uv, w), 0.0);
    return mix(s, AMB_OPEN, lk_smooth(AMB_TOP_C, max(reach, AMB_TOP_C + AMB_OPEN_RISE_M), h));
}

// The visibility at a shaded point (geometric normal ng, read AMB_NORMAL_OFFSET_M
// along it): the plane's street level eased to the open sky by its reach,
// then the far level and the near level inside their rectangles.
fn amb_sample(world: vec3<f32>, ng: vec3<f32>) -> vec4<f32> {
    let p = world + ng * AMB_NORMAL_OFFSET_M;
    let pl = amb_plane(p.xz);
    let h = p.y - pl.z;
    let reach = max(pl.w, 1.0);
    var v = mix(vec4<f32>(pl.x, 0.0, pl.y, 0.0), AMB_OPEN, lk_smooth(0.0, reach, h));
    if (AMB_SHADER_LEVELS > 0u) {
        let wf = amb_edge(amb_rect(look.lx[10].x, look.lx[10].y), p.xz);
        if (wf > 0.0) {
            v = mix(v, amb_level(p.xz, h, 0u, AMB_FAR_CELL_M, reach), wf);
        }
    }
    if (AMB_SHADER_LEVELS > 1u) {
        let wn = amb_edge(amb_rect(look.lx[10].z, look.lx[10].w), p.xz);
        if (wn > 0.0) {
            v = mix(v, amb_level(p.xz, h, 1u, AMB_NEAR_CELL_M, reach), wn);
        }
    }
    return v;
}

// The visible share of the sky around a normal or direction d (0..1).
fn amb_vis(c: vec4<f32>, d: vec3<f32>) -> f32 {
    return clamp(c.x + dot(c.yzw, d), 0.0, 1.0);
}

// The ground bounce (`LightingOptions::ground_bounce`, Full shaders only:
// AMB_SHADER_LEVELS > 0). The levels then hold, half the volume's depth
// further on, the open ground's share of the lower hemisphere (g0, gx, gy,
// gz) in the same form (ambient_bake.rs `Spec::ground`): the ground with
// nothing over it, sunlit while the sun is up. look.lx[0].yzw is the sunlit
// ground's exitance from the sun (eo-look `bounce::sun_exitance`; zero while
// the bounce is off or the sun is down).
const AMB_OPEN_GROUND: vec4<f32> = vec4<f32>(0.5, 0.0, -0.5, 0.0);

// One level's open ground h metres over the ground (over the top layer, eased
// to open ground as the sky eases to the open sky).
fn amb_ground_level(xz: vec2<f32>, h: f32, slab: u32, cell: f32, reach: f32) -> vec4<f32> {
    let uv = fract(xz / (cell * f32(AMB_VOLUME_TEXELS)));
    let w = (f32(slab * AMB_LAYERS) + amb_layer(h) + 0.5) * look.lx[11].w + 0.5;
    let s = textureSampleLevel(lx_ambient_volume, sky_rep, vec3<f32>(uv, w), 0.0);
    return mix(s, AMB_OPEN_GROUND, lk_smooth(AMB_TOP_C, max(reach, AMB_TOP_C + AMB_OPEN_RISE_M), h));
}

// The open ground at a shaded point (read as `amb_sample` reads the sky):
// none past the levels (the plane holds no ground), the far and the near
// level inside their rectangles, eased in over their edges. `w` is how much
// of the point the levels cover (0 past them, 1 inside both edge fades); `g`
// is already scaled by it.
struct AmbGround {
    g: vec4<f32>,
    w: f32,
};

fn amb_ground(world: vec3<f32>, ng: vec3<f32>) -> AmbGround {
    var o = AmbGround(vec4<f32>(0.0), 0.0);
    if (AMB_SHADER_LEVELS > 0u) {
        let p = world + ng * AMB_NORMAL_OFFSET_M;
        let pl = amb_plane(p.xz);
        let h = p.y - pl.z;
        let reach = max(pl.w, 1.0);
        // an underside's own deck shades the ground under it, its shadow
        // shifted away from the sun by the deck's height x cot(the sun's
        // altitude): the sunlit ground it sees is the open ground seen from
        // that far toward the sun (exact for a lone flat deck over flat
        // ground). Undersides only (by -ng.y); walls read where they stand.
        let k = look.key_dir.xyz;
        let kh = length(k.xz);
        let shift = min(max(world.y - pl.z, 0.0) * min(kh / max(k.y, 1e-3), AMB_SUN_SHIFT_COT_MAX), AMB_SUN_SHIFT_MAX_M) * max(-ng.y, 0.0);
        let q = p.xz + k.xz * (shift / max(kh, 1e-4));
        let wf = amb_edge(amb_rect(look.lx[10].x, look.lx[10].y), q);
        if (wf > 0.0) {
            o.g = mix(o.g, amb_ground_level(q, h, 0u, AMB_FAR_CELL_M, reach), wf);
            o.w = wf;
        }
        if (AMB_SHADER_LEVELS > 1u) {
            let wn = amb_edge(amb_rect(look.lx[10].z, look.lx[10].w), q);
            if (wn > 0.0) {
                o.g = mix(o.g, amb_ground_level(q, h, 1u, AMB_NEAR_CELL_M, reach), wn);
                o.w = mix(o.w, 1.0, wn);
            }
        }
    }
    return o;
}

// The irradiance the sunlit ground changes round the normal n (`cloud`: the
// cloud transmittance). The rig's ground lobe stands for the street's bounce
// of sun and sky together, as if the whole lower hemisphere were sunlit
// street; where the levels know the ground, the lobe's sun part (the lobe
// less AMB_LOBE_SKY_ALBEDO x the fill on the ground, never more than the
// sunlit ground's own exitance) leaves the whole lower hemisphere (1 - t) and
// the sunlit ground's exitance from the sun comes back over the open
// ground's share. Nothing is counted twice: over open ground a surface takes
// the sunlit street's light, over covered ground (under a deck, a closed
// yard) it keeps only the lobe's sky part. Zero facing up, at night and in a
// cloud's shadow (eo-look bounce.rs `added_irradiance`).
fn amb_bounce(a: AmbGround, n: vec3<f32>, cloud: f32) -> vec3<f32> {
    let lower = 0.5 - 0.5 * n.y;
    let share = min(amb_vis(a.g, n), a.w * lower);
    let sun = look.lx[0].yzw * cloud;
    let lobe_sun = max(look.hemi_ground.xyz - AMB_LOBE_SKY_ALBEDO * (look.hemi_sky.xyz + look.ambient.xyz), vec3<f32>(0.0));
    return share * sun - (a.w * lower) * min(lobe_sun, sun);
}

// look_light's indirect diffuse irradiance with the volume. The hemisphere's
// sky part and the environment's diffuse take the visible sky (never more
// than the open sky's share t); the sky the volume blocks is lit by what
// blocks it (the ground lobe x AMB_OCCLUDER: facades and soffits stand in
// the street's light), so nothing goes black. The ground part and the
// ambient term as they were. With AMB_OPEN this is `hemi + ambient + env`.
fn amb_irradiance(c: vec4<f32>, n: vec3<f32>, env: vec3<f32>) -> vec3<f32> {
    let t = n.y * 0.5 + 0.5;
    let vs = min(amb_vis(c, n), t);
    let fill = look.hemi_sky.xyz * vs + look.hemi_ground.xyz * (1.0 - t + AMB_OCCLUDER * (t - vs));
    return fill + look.ambient.xyz + env * (1.0 - t + vs);
}

// The specular environment's weights along the reflection r: its own share
// (the sky it sees, and all of what lies below the horizon) and the
// occluders' (their radiance is the ground lobe x AMB_OCCLUDER / pi).
fn amb_reflection(c: vec4<f32>, r: vec3<f32>) -> vec2<f32> {
    let t = r.y * 0.5 + 0.5;
    let vs = min(amb_vis(c, r), t);
    return vec2<f32>(1.0 - t + vs, AMB_OCCLUDER * (t - vs));
}
