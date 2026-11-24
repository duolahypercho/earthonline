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
// (`--occluder-light`, eo-look `bounce::added_irradiance_whole`) the lobe's sun
// part leaves whole: the cap a channel left the neutral lobe's green and blue
// over the orange sunlit ground at a low sun, a navy fill on every underside.
// And the open ground's share reflects the whole sky (it has nothing over it),
// where the lobe kept it at the street's own sky under the point (`amb_vg`).
fn amb_bounce(a: AmbGround, n: vec3<f32>, cloud: f32) -> vec3<f32> {
    let lower = 0.5 - 0.5 * n.y;
    let share = min(amb_vis(a.g, n), a.w * lower);
    let sun = look.lx[0].yzw * cloud;
    let lobe_sun = max(look.hemi_ground.xyz - AMB_LOBE_SKY_ALBEDO * (look.hemi_sky.xyz + look.ambient.xyz), vec3<f32>(0.0));
    if (amb_ol_on()) {
        return share * (sun + amb_lobe_sky() * (1.0 - amb_vg)) - (a.w * lower) * lobe_sun;
    }
    return share * sun - (a.w * lower) * min(lobe_sun, sun);
}

// (`--occluder-light`, SF_OCCLUDER_LIGHT; eo-look `occluder`, its CPU
// reference and the reasoning) the light of what hides the sky. look_light
// reads the volume with `amb_sample_ground`, which also sets `amb_vg` (the
// street's own sky under the point), before `amb_irradiance`, which then lights the blocked sky by the shaded
// walls' exitance (`amb_occluder_exitance`, kept in `amb_occ_m` for the
// specular environment's occluders) and the street's lobe and the ambient
// term's ground share by the street's sky. Run bit: look.ao_params.w bit 2.
var<private> amb_vg: f32 = 1.0;
var<private> amb_occ_m: vec3<f32> = vec3<f32>(0.0);

// Only where the levels are read (AMB_SHADER_LEVELS > 0: the city, the
// structures, the agents, the probe faces): the far layer's and the props'
// pipelines read the plane alone and stand at the edge of what Apple's
// compiler takes (`hlod-draw` failed with this in, r2i1); they keep the
// stand-in, past the levels' reach and on small things.
fn amb_ol_on() -> bool {
    return SF_OCCLUDER_LIGHT && AMB_SHADER_LEVELS > 0u && (u32(look.ao_params.w + 0.5) & 4u) != 0u;
}

// `amb_sample` that also sets `amb_vg`, the street's own sky visibility
// facing up (0..1) under the shaded point: the plane and the levels
// OL_GROUND_SKY_H_M over the ground where the point reads (AMB_NORMAL_OFFSET_M
// along the geometric normal: the street at a wall's foot, the ground under a
// soffit), from the same plane texel and level rectangles. Only where it
// counts (shading normal `n`): with no lower hemisphere and no blocked sky
// past OL_SKIP_SHARE (an open roof or square seen from above) the street's
// sky is not read and counts as open.
fn amb_sample_ground(world: vec3<f32>, ng: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let p = world + ng * AMB_NORMAL_OFFSET_M;
    let pl = amb_plane(p.xz);
    let h = p.y - pl.z;
    let hg = OL_GROUND_SKY_H_M;
    let reach = max(pl.w, 1.0);
    let open_pl = vec4<f32>(pl.x, 0.0, pl.y, 0.0);
    var v = mix(open_pl, AMB_OPEN, lk_smooth(0.0, reach, h));
    var g = mix(open_pl, AMB_OPEN, lk_smooth(0.0, reach, hg));
    let wf = amb_edge(amb_rect(look.lx[10].x, look.lx[10].y), p.xz);
    let wn = amb_edge(amb_rect(look.lx[10].z, look.lx[10].w), p.xz);
    if (AMB_SHADER_LEVELS > 0u && wf > 0.0) {
        v = mix(v, amb_level(p.xz, h, 0u, AMB_FAR_CELL_M, reach), wf);
    }
    if (AMB_SHADER_LEVELS > 1u && wn > 0.0) {
        v = mix(v, amb_level(p.xz, h, 1u, AMB_NEAR_CELL_M, reach), wn);
    }
    let t = n.y * 0.5 + 0.5;
    if (1.0 - t <= OL_SKIP_SHARE && t - min(amb_vis(v, n), t) <= OL_SKIP_SHARE) {
        amb_vg = 1.0;
        return v;
    }
    if (AMB_SHADER_LEVELS > 0u && wf > 0.0) {
        g = mix(g, amb_level(p.xz, hg, 0u, AMB_FAR_CELL_M, reach), wf);
    }
    if (AMB_SHADER_LEVELS > 1u && wn > 0.0) {
        g = mix(g, amb_level(p.xz, hg, 1u, AMB_NEAR_CELL_M, reach), wn);
    }
    amb_vg = clamp(g.x + g.z, 0.0, 1.0);
    return v;
}

// The ground lobe's sky part (the street's reflection of the sky's fill), never more than the lobe.
fn amb_lobe_sky() -> vec3<f32> {
    return min(AMB_LOBE_SKY_ALBEDO * (look.hemi_sky.xyz + look.ambient.xyz), look.hemi_ground.xyz);
}

// The sky a canyon's wall sees, from the sky its street sees (eo-look
// `occluder::wall_sky_view`): a street seeing `vg` of the sky lies between
// walls H/W = (1 - vg^2) / (2 vg) high for its width (vg = sqrt(1 + (H/W)^2)
// - H/W), and such a wall sees 1/2 (1 + W/H - sqrt(1 + (W/H)^2)) of it
// (the two-dimensional canyon's view factors), OL_SIDE_OPEN in the open.
fn amb_wall_sky(vg: f32) -> f32 {
    let s = clamp(vg, OL_MIN_STREET_SKY, OL_MAX_STREET_SKY);
    let x = 2.0 * s / (1.0 - s * s); // W / H
    return OL_SIDE_OPEN * (1.0 + x - sqrt(1.0 + x * x));
}

// The exitance of the shaded walls that hide the sky (eo-look
// `occluder::shaded_exitance`): the canyon's walls round a point whose street
// sees `vg` of the sky see `amb_wall_sky(vg)` of it, the street's lobe below
// them with its sky part at `vg`, the ambient term and the environment's
// diffuse on a horizontal normal (the SH's azimuth mean) at the sky and
// street they see, and over the rest of their upper half walls like
// themselves (M = rho E / (1 - rho (1/2 - v_w))).
fn amb_occluder_exitance(vg: f32) -> vec3<f32> {
    let vw = amb_wall_sky(vg);
    let g = look.hemi_ground.xyz - amb_lobe_sky() * (1.0 - vg);
    let env_side = max(look.sh[0].xyz * 0.282095 + look.sh[6].xyz * 0.157696 + look.sh[8].xyz * 0.273137, vec3<f32>(0.0)) * (look.ambient.w * look.env_class.x);
    let e = look.hemi_sky.xyz * vw + g * 0.5 + (look.ambient.xyz + env_side) * (vw + 0.5 * vg);
    return OL_WALL_ALBEDO * e / (vec3<f32>(1.0) - OL_WALL_ALBEDO * (OL_SIDE_OPEN - vw));
}

// (`--occluder-light`, eo-look `occluder::lid_share`) the decks' undersides'
// share of what hides the sky over a point whose volume visibility is `c`,
// read from where the sky it still sees lies: r = a0 / (a0 + ay), the visible
// sky on a vertical surface over that facing up, is 1/2 under the open sky,
// lower between walls (the sky left overhead), higher under a lid (the sky
// left at the sides).
fn amb_lid_share(c: vec4<f32>) -> f32 {
    return lk_smooth(OL_LID_R0, OL_LID_R1, c.x / max(c.x + c.z, 1e-3));
}

// (`--occluder-light`, eo-look `occluder::soffit_exitance`) a deck's
// underside's exitance over a street that sees `vg` of the sky: beyond its
// edges `vg` of its lower hemisphere is open ground (the street's lobe's sky
// part whole, and the sunlit ground's exitance from the sun: look.lx[0].yzw
// with the ground bounce, the rig's lobe's sun part without it), under it
// covered ground lit by the street's own sky.
fn amb_soffit_exitance(vg: f32) -> vec3<f32> {
    let ls = amb_lobe_sky();
    var sun = max(look.hemi_ground.xyz - ls, vec3<f32>(0.0));
    if (any(look.lx[0].yzw > vec3<f32>(0.0))) {
        sun = look.lx[0].yzw;
    }
    return OL_DECK_ALBEDO * (vg * (ls + sun) + (1.0 - vg) * vg * ls);
}

// (`--occluder-light`) the share of the rig's rim light (a directional stand-in
// for the sky opposite the sun, never shadowed) that what hides the sky
// hides: 1 - the visible share along its direction over the open sky's.
fn amb_rim_hidden(c: vec4<f32>, d: vec3<f32>) -> f32 {
    return 1.0 - min(amb_vis(c, d) / max(0.5 + 0.5 * d.y, 1e-3), 1.0);
}

// (`--occluder-light`) the blocked sky's radiance x pi for the specular
// environment's occluders: the shaded walls' exitance while it runs, else the
// ground lobe (AMB_OCCLUDER's stand-in) as before.
fn amb_occluder_light() -> vec3<f32> {
    if (amb_ol_on()) {
        return amb_occ_m;
    }
    return look.hemi_ground.xyz;
}

// look_light's indirect diffuse irradiance with the volume. The hemisphere's
// sky part and the environment's diffuse take the visible sky (never more
// than the open sky's share t); the sky the volume blocks is lit by what
// blocks it (the ground lobe x AMB_OCCLUDER: facades and soffits stand in
// the street's light), so nothing goes black. The ground part and the
// ambient term as they were. With AMB_OPEN this is `hemi + ambient + env`.
// (`--occluder-light`) the blocked sky at the shaded walls' exitance, the
// street's lobe's sky part, the ambient term's and the environment's ground
// share at the street's own sky (`amb_vg`): with the open sky and open
// ground the same sum.
fn amb_irradiance(c: vec4<f32>, n: vec3<f32>, env: vec3<f32>) -> vec3<f32> {
    let t = n.y * 0.5 + 0.5;
    let vs = min(amb_vis(c, n), t);
    if (amb_ol_on()) {
        let lower = 1.0 - t;
        // what hides the sky: the shaded walls, or under a lid the deck's underside (by the lid's share).
        amb_occ_m = amb_occluder_exitance(amb_vg);
        let lid = amb_lid_share(c);
        if (lid > 0.0) {
            amb_occ_m = mix(amb_occ_m, amb_soffit_exitance(amb_vg), lid);
        }
        let lit = look.hemi_sky.xyz * vs + (look.hemi_ground.xyz - amb_lobe_sky() * (1.0 - amb_vg)) * lower + amb_occ_m * (t - vs);
        // the rig's rim (a directional stand-in for the sky opposite the sun, never
        // shadowed) hidden as the sky along it is, taken from the fill it stands for
        // (so the screen-space occlusion takes its share of it too; look_light's
        // direct sum stays as it was, which keeps the compiled-out frame bit for bit).
        let rim = look.rim.xyz * (max(dot(n, look.rim_dir.xyz), 0.0) * amb_rim_hidden(c, look.rim_dir.xyz));
        return lit + (look.ambient.xyz + env) * (vs + lower * amb_vg) - rim;
    }
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
