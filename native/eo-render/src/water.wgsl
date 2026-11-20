// the sea surface (eo-render/src/water.rs), a port of the JS game's
// open sea (src/render/materials/ocean-open-sea.js) and its material
// (src/render/materials/water-surface.js with tidewater's optics,
// ocean-water-optics.js):
//
//  vertex:   the camera-following graded grid, displaced by the FFT cascades
//            (band-limited at the vertex spacing, ramped in from 50 m to
//            220 m off the shore as the JS ramps it, the bay's swell reduced);
//  fragment: the shore mask, the wave normal (FFT slopes faded by footprint
//            and range, the near-field capillary taps, the v15 chop octaves,
//            foam relief), Cox-Munk roughness of the unresolved slopes, exact
//            dielectric Fresnel, the sky / scene probe along the raised
//            mirror ray with its horizon occlusion, the screen-space march
//            for the scene standing in the mirror (with the vanishing-point
//            tap where it finds nothing), the GGX sun glint under the
//            cascaded shadow, the refracted water column (Beer-Lambert +
//            single scattering, the regional body water), crest translucency,
//            the shore collar, spindrift and whitecaps, and the look's fog.
//
// Two fragment entry points: `fs_copy` (the frame was copied before the
// the refracted bottom and the mirror are read from the copy) and
// `fs_blend` (no copy this frame: the bottom shows through by blending, no
// screen-space reflection).

@group(0) @binding(0) var<uniform> wu: WaterU;
@group(0) @binding(1) var w_field: texture_2d<f32>;
@group(0) @binding(2) var w_disp: texture_2d_array<f32>;
@group(0) @binding(3) var w_deriv: texture_2d_array<f32>;
@group(0) @binding(4) var w_repeat: sampler;
@group(0) @binding(5) var w_clamp: sampler;
@group(0) @binding(6) var w_depth: texture_depth_2d;
@group(0) @binding(7) var w_scene: texture_2d<f32>;
@group(0) @binding(8) var w_probe: texture_2d_array<f32>;
@group(0) @binding(9) var w_probe_samp: sampler;
@group(0) @binding(10) var w_detail: texture_2d<f32>;
// the sky slice's atmosphere tables (sky view, transmittance)
@group(0) @binding(11) var w_atmo_view: texture_2d<f32>;
@group(0) @binding(12) var w_atmo_trans: texture_2d<f32>;

// the sky as the sky slice now draws it where nothing covers it: the
// look's graded dome blended toward the atmosphere tables (`at_sky`, its
// zenith held to the geometric mean of the graded and the model zenith, as
// sky_volume.wgsl `vol_atmo_gain`) by the slice's own weight (`atmo2.y`, lower
// under a cloud deck and at night).
fn w_atmo_dome(d: vec3<f32>) -> vec3<f32> {
    if (skyu.atmo2.x < 0.5 || skyu.atmo2.y <= 0.0) {
        return lk_sky_dome(d);
    }
    let at = textureSampleLevel(w_atmo_view, w_clamp, at_view_uv(normalize(vec3<f32>(d.x, max(d.y, 0.0), d.z)), skyu.atmo.xyz), 0.0).rgb * w_atmo_g; // (the gain cached)
    // (the graded dome only where the slice still blends it in: under a deck, at dusk)
    if (skyu.atmo2.y >= 0.999) {
        return at;
    }
    return mix(lk_sky_dome(d), at, skyu.atmo2.y);
}

@group(1) @binding(0) var<uniform> look: Look;
@group(1) @binding(1) var lk_shadow_map: texture_depth_2d_array;
@group(1) @binding(2) var lk_shadow_cmp: sampler_comparison;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    // the undisplaced point the field and the spectrum are read at
    @location(1) base: vec2<f32>,
    // the atmosphere table's gain (constant over a frame), made per vertex, not per pixel
    @location(2) @interpolate(flat) atmo_g: f32,
    // cascades 0 and 1's slope read at the vertex at its spacing's mip
    // (xy cascade 0, zw cascade 1): the swell's tilt where the pixel no longer resolves it
    @location(3) wv_far: vec4<f32>,
};

// this fragment's vertex-read far slope (`wv_vertex_slope`); 0 for the surf strip
var<private> wv_far_slope: vec4<f32> = vec4<f32>(0.0);

// cascades 0 and 1's slope at an undisplaced point, times each
// place's amplitude (r2b: level 0; cascade 1 only where the vertex spacing still
// resolves it, under 8 of its texels).
fn wv_vertex_slope(base: vec2<f32>, spacing: f32) -> vec4<f32> {
    let amp = wg_cascade_amp(base.x);
    let s0 = textureSampleLevel(w_deriv, w_repeat, w_casc_uv(base, 0u), 0u, 0.0).xy * amp.x;
    let k1 = 1.0 - smoothstep(1.0, 3.0, wv_lod(spacing, 1u));
    var s1 = vec2<f32>(0.0);
    if (k1 > 0.0) {
        s1 = textureSampleLevel(w_deriv, w_repeat, w_casc_uv(base, 1u), 1u, 0.0).xy * amp.y * k1;
    }
    return vec4<f32>(s0, s1);
}

// the gain this fragment uses (`w_atmo_dome`)
var<private> w_atmo_g: f32 = 0.0;

// the table's gain so its zenith matches the drawn one (sky_volume.wgsl `vol_atmo_gain`)
fn w_atmo_gain() -> f32 {
    if (skyu.atmo2.x < 0.5 || skyu.atmo2.y <= 0.0) {
        return 0.0;
    }
    let zl = sqrt(lk_luma(lk_sky_dome(vec3<f32>(0.0, 1.0, 0.0))) * lk_luma(look.dome_zenith.xyz));
    let z = textureSampleLevel(w_atmo_view, w_clamp, at_view_uv(vec3<f32>(0.0, 1.0, 0.0), skyu.atmo.xyz), 0.0).rgb;
    return zl / max(lk_luma(z), 1e-9);
}

// The shore field at a world point: x signed distance (m), y beach weight.
// Texel centres at origin + step * i, as the bake places them.
fn w_field_at(xz: vec2<f32>) -> vec2<f32> {
    let dims = vec2<f32>(textureDimensions(w_field, 0));
    let t = (xz - wu.field.xy) / wu.field.zw * (dims - 1.0);
    return textureSampleLevel(w_field, w_clamp, (t + 0.5) / dims, 0.0).xy;
}

fn w_casc_uv(xz: vec2<f32>, c: u32) -> vec2<f32> {
    return xz / wu.sizes[c] + vec2<f32>(0.5 / 256.0);
}

// The shoreward direction (down the field's gradient) and the along-shore
// coordinate (the world axis most nearly parallel to the shore), as
// ocean-open-sea.js derives them for the surf.
fn w_shore_frame(xz: vec2<f32>) -> vec3<f32> {
    let step = wu.field.zw / vec2<f32>(textureDimensions(w_field, 0) - vec2<u32>(1u, 1u));
    let gx = w_field_at(xz + vec2<f32>(step.x, 0.0)).x - w_field_at(xz - vec2<f32>(step.x, 0.0)).x;
    let gz = w_field_at(xz + vec2<f32>(0.0, step.y)).x - w_field_at(xz - vec2<f32>(0.0, step.y)).x;
    let shoreward = normalize(-vec2<f32>(gx, gz) + vec2<f32>(1e-5, 0.0));
    let along = select(xz.x, xz.y, abs(shoreward.x) > abs(shoreward.y));
    return vec3<f32>(shoreward, along);
}

// `seaDetailSample` (ocean-sea-detail.js, tidewater SeaDetail.js): x rough
// (short-wave slope multiplier), y gust, z slick, w streak.
fn w_sea_detail(xz: vec2<f32>, t: f32) -> vec4<f32> {
    let w = WS_WIND_DIR;
    let p = xz - w * (t * WS_WIND * 0.7);
    let g1 = textureSampleLevel(w_detail, w_repeat, p / 620.0, 0.0).x;
    let g2 = textureSampleLevel(w_detail, w_repeat, p / 230.0 + vec2<f32>(t * 0.0009, 0.37), 0.0).y;
    let gust = clamp((g1 * 0.62 + g2 * 0.38 - 0.5) * 2.4 + 0.5, 0.0, 1.0);
    let along = dot(xz, w);
    let across = dot(xz, vec2<f32>(-w.y, w.x)) + (g2 - 0.5) * 26.0;
    let sl = textureSampleLevel(w_detail, w_repeat, vec2<f32>(along / 1100.0, across / 70.0), 0.0).z;
    let calm = wg_smooth(13.0, 4.0, WS_WIND);
    let slick = wg_smooth(0.64, 0.76, sl) * (1.0 - gust * 0.8) * calm;
    let st = textureSampleLevel(w_detail, w_repeat, vec2<f32>(along / 380.0, across / 11.0) + vec2<f32>(0.13, 0.71), 0.0).w;
    let brk = textureSampleLevel(w_detail, w_repeat, vec2<f32>(along / 140.0, across / 40.0) + vec2<f32>(0.51, 0.29), 0.0).x;
    let fresh = wg_smooth(6.0, 12.0, WS_WIND);
    let streak = wg_smooth(0.68, 0.82, st) * wg_smooth(0.4, 0.62, brk) * fresh * 0.3;
    let rough = mix(0.5, 1.5, gust) * (1.0 - slick * 0.8) * (1.0 - streak / 0.3 * 0.45);
    return vec4<f32>(rough, gust, slick, streak);
}

// the Pacific's groundswell as three long-crested trains from the
// west-northwest (165-260 m, together about 1.6 m significant height), under
// the FFT's wind sea: the long parallel ridges that set the open ocean apart
// from the bay. Weighted out inside the Golden Gate and inside a beach's surf
// zone (the surf model carries the swell there). Returns (height, d/dx, d/dz).
fn w_swell(p: vec2<f32>, t: f32, signed: f32, beach: f32) -> vec3<f32> {
    let w = wg_ocean(p.x) * (1.0 - beach * (1.0 - wg_smooth(150.0, 450.0, signed))) * wu.wv_misc.y; // 0 when the spectrum carries the swell
    if (w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let dirs = array<vec2<f32>, 3>(vec2<f32>(0.970, 0.243), vec2<f32>(0.940, 0.342), vec2<f32>(0.995, 0.100));
    let lams = vec3<f32>(210.0, 165.0, 260.0);
    let amps = vec3<f32>(0.8, 0.5, 0.6); // (Hs about 2.5 m, the swell a reader sees)
    let phs = vec3<f32>(0.0, 2.1, 4.4);
    var o = vec3<f32>(0.0);
    for (var i = 0; i < 3; i = i + 1) {
        let k = 6.2832 / lams[i];
        let om = sqrt(9.81 * k);
        let a = dot(p, dirs[i]) * k - om * t + phs[i] + 0.6 * sin(dot(p, vec2<f32>(-dirs[i].y, dirs[i].x)) * 0.004 + f32(i));
        o = o + vec3<f32>(amps[i] * sin(a), amps[i] * k * cos(a) * dirs[i].x, amps[i] * k * cos(a) * dirs[i].y);
    }
    return o * w;
}

// a hash of a world cell and a step, 0..1.
fn w_hash(c: vec2<f32>, k: f32) -> f32 {
    let a = bitcast<u32>(i32(c.x)) * 73856093u;
    let b = bitcast<u32>(i32(c.y)) * 19349663u;
    let d = bitcast<u32>(i32(k)) * 83492791u;
    return sky_hash1(a ^ b ^ d);
}

// the swash on a beach. Each bore of the surf model reaches the still
// waterline once a wave period and runs up the sand as a thin sheet: the
// water level there rises quickly to the run-up (0.9 of that wave's offshore
// amplitude, cusped along the shore) and drains back over the rest of the
// period, drawing down a little below the sea. Returns (level above the sea
// in metres, phase 0..1 since the bore arrived).
fn w_swash(along: f32, t: f32, amp: f32) -> vec2<f32> {
    // the bores reach different stretches of the beach at different times
    let stagger = 0.22 * sin(along * 0.043 + 0.8) + 0.13 * sin(along * 0.117 + 2.9) + 0.06 * sin(along * 0.31 + 1.4);
    let phase = t / SF_PERIOD + sf_wobble(along) + stagger;
    let m = floor(phase);
    let u = phase - m;
    let a = min(sf_wave_amp(m, along) * amp, 0.7); // a dissipative beach's run-up stays under about 0.6 m
    // (no shorter than ~10 m: the front is where the grid's level meets the sand)
    let cusp = (1.0 + 0.16 * sin(along * 0.19 + m * 2.3) + 0.1 * sin(along * 0.53 + m * 1.1)) * sf_swash_cusp(along, m); // scalloped (water_surf.wgsl)
    let rise = wg_smooth(0.0, 0.2, u);
    let fall = 1.0 - wg_smooth(0.2, 0.98, u);
    return vec2<f32>(a * (0.9 * cusp * rise * pow(fall, 1.4) - 0.08), u);
}

// the edge distance of a cellular field (F2 - F1, 0 on a cell
// wall), the cells' seeds wandering slowly: foam left by a breaking wave is a
// raft of bubble cells whose walls survive longest.
fn w_cells(q: vec2<f32>, t: f32, k: f32) -> f32 {
    let c = floor(q);
    let f = q - c;
    var d1 = 8.0;
    var d2 = 8.0;
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let o = vec2<f32>(f32(i), f32(j));
            let h1 = w_hash(c + o, k);
            let h2 = w_hash(c + o, k + 7.0);
            let h = vec2<f32>(0.5 + 0.42 * sin(t * 0.21 + 6.2832 * h1), 0.5 + 0.42 * sin(t * 0.17 + 6.2832 * h2));
            let r = o + h - f;
            let d = dot(r, r);
            if (d < d1) {
                d2 = d1;
                d1 = d;
            } else if (d < d2) {
                d2 = d;
            }
        }
    }
    return sqrt(d2) - sqrt(d1);
}

// foam lace from three nested cell scales (1.7 m rafts, 0.5 m
// cells, 0.13 m bubbles near the eye), carried shoreward with the bores and
// drifting along the shore; 1 on a cell wall, 0 in a cell's middle. Where a
// pixel spans a cell the scale is blended toward its mean, so the threshold
// that follows gives a soft, translucent coverage instead of a cut-out edge.
fn w_lace(p: vec2<f32>, shoreward: vec2<f32>, t: f32, footprint: f32) -> f32 {
    let q0 = p - shoreward * (t * 0.8) + vec2<f32>(-shoreward.y, shoreward.x) * (t * 0.15);
    let wv = vec2<f32>(mx_noise(vec3<f32>(q0 * 0.21, 0.5)), mx_noise(vec3<f32>(q0 * 0.21 + vec2<f32>(3.1, 1.7), 0.9)));
    let q = q0 + wv * 1.2;
    let l1 = 1.0 - clamp(w_cells(q / 1.7, t, 11.0) * 1.5, 0.0, 1.0);
    var lace = mix(l1, 0.45, wg_smooth(0.15, 0.8, footprint));
    let k2 = 1.0 - wg_smooth(0.12, 0.4, footprint);
    if (k2 > 0.0) {
        let l2 = 1.0 - clamp(w_cells(q / 0.5 + vec2<f32>(3.7, 9.1), t * 1.5, 23.0) * 2.4, 0.0, 1.0);
        lace = mix(lace, max(lace * 0.7, l2), 0.55 * k2);
    }
    let k3 = 1.0 - wg_smooth(0.02, 0.07, footprint);
    if (k3 > 0.0) {
        let l3 = 1.0 - clamp(w_cells(q / 0.13 + vec2<f32>(1.3, 5.9), t * 2.0, 37.0) * 2.6, 0.0, 1.0);
        lace = mix(lace, lace * (0.65 + 0.5 * l3), k3);
    }
    return clamp(lace, 0.0, 1.0);
}

// how far the weather's wind is a storm's (0 under the reference 6 m/s
// breeze, 1 at 12 m/s and over: the storm preset's 12.2 m/s).
fn w_storm() -> f32 {
    if (skyu.flags.x == 0u) {
        return 0.0;
    }
    return wg_smooth(6.0, 12.0, length(skyu.wind.xy));
}

// rain on the water: rings spreading from drop strikes, two staggered
// layers of 0.6 m cells, each ring a damped ripple running out to 0.2 m and
// fading as it goes. Returns the surface slope (x, z).
fn w_rain_rings(p: vec2<f32>, t: f32) -> vec2<f32> {
    var g = vec2<f32>(0.0);
    let sc = 3.0; // 0.33 m cells (drops of a shower)
    for (var k = 0u; k < 2u; k = k + 1u) {
        let q = p * sc + vec2<f32>(f32(k) * 0.5, f32(k) * 0.37);
        let c = floor(q);
        let h = w_hash(c, f32(k) * 31.0 + 7.0);
        let centre = c + vec2<f32>(0.3 + 0.4 * h, 0.3 + 0.4 * fract(h * 13.7));
        let phase = fract(t * 1.1 + h * 7.3 + f32(k) * 0.5);
        let d = q - centre;
        let r = length(d);
        // (drops of different sizes, some cells empty: no stamped grid)
        let size = 0.55 + 0.9 * fract(h * 41.3);
        let live = select(0.0, 1.0, fract(h * 97.1) < 0.7);
        let x = (r - phase * 0.32 * size) * 30.0 / size;
        let env = exp(-x * x * 0.08) * (1.0 - phase) * (1.0 - phase) * live;
        g = g + d / max(r, 1e-3) * (cos(x) * env * 0.6); // (0.6, the rings were invisible)
    }
    return g;
}

// the sky as the frame finally drew it: the sky slice's resolved
// previous frame (its clouds, rain, haze and fog) where the mirror direction
// falls on sky inside the last frame, the dome elsewhere. Brighter than three
// times the dome is the sun's disc and its glare, which the glint carries.
fn w_drawn_sky(d: vec3<f32>, dome: vec3<f32>) -> vec3<f32> {
    if (skyu.flags.x == 0u || skyu.taa.y < 0.5) {
        return dome;
    }
    let c = skyu.prev_vp * vec4<f32>(d, 0.0);
    let uv = vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
    let e = select(0.0, w_edge(uv), c.w > 1e-4);
    if (e <= 0.0) {
        return w_sky_ratio(d, dome, uv, c.w > 1e-4);
    }
    let fs_far = textureLoad(w_depth, vec2<i32>(clamp(uv, vec2<f32>(0.0), vec2<f32>(0.99999)) * vec2<f32>(textureDimensions(w_depth))), 0) < 1e-12; // the far land (depth 1e-9 / view depth past the far plane, DEPTH_K) mirrors as the sky does, at infinity
    let sky_here = select(0.0, 1.0, w_scene_depth(uv) > 1e8 || fs_far); // + the far land (1e-9 / view depth, DEPTH_K)
    var drawn = textureSampleLevel(sky_prev, sky_rep, uv, 0.0).rgb;
    drawn = drawn * min(1.0, (lk_luma(dome) * 3.0 + 1e-4) / max(lk_luma(drawn), 1e-6));
    return mix(w_sky_ratio(d, dome, uv, c.w > 1e-4), drawn, e * sky_here);
}

// where the mirror direction before the sea's raise for the unresolved slope falls on the far land (depth under 1e-12: 1e-9 / view depth past the far plane, DEPTH_K), most of the reflection is that land as the last frame drew it; the raise alone lifted a far shore's reflection into the sky over it (a bright band under the hills).
fn fs_far_mirror(r: vec3<f32>, dome: vec3<f32>) -> vec3<f32> {
    if (skyu.flags.x == 0u || skyu.taa.y < 0.5) {
        return dome;
    }
    let d = normalize(vec3<f32>(r.x, max(r.y, 0.002), r.z));
    let c = skyu.prev_vp * vec4<f32>(d, 0.0);
    if (c.w <= 1e-4) {
        return dome;
    }
    let uv = vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return dome;
    }
    let dz = textureLoad(w_depth, vec2<i32>(uv * vec2<f32>(textureDimensions(w_depth))), 0);
    if (dz <= 0.0 || dz >= 1e-12) {
        return dome;
    }
    return mix(dome, textureSampleLevel(sky_prev, sky_rep, uv, 0.0).rgb, 0.65);
}

// the dome for a direction off the last frame, carried by what the sky
// slice made of the dome at the nearest sky the frame shows (its haze, cloud
// cover, rain tint: per channel, drawn over dome, at the screen point nearest
// the direction, or the top centre for a direction behind the camera).
fn w_sky_ratio(d: vec3<f32>, dome: vec3<f32>, uv: vec2<f32>, ahead: bool) -> vec3<f32> {
    // under the full atmosphere sky the dome is already the drawn sky
    if (skyu.atmo2.x > 0.5 && skyu.atmo2.y >= 0.999) {
        return dome;
    }
    var uc = vec2<f32>(0.5, 0.03);
    if (ahead) {
        uc = clamp(uv, vec2<f32>(0.03), vec2<f32>(0.97));
    }
    if (w_scene_depth(uc) < 1e8) {
        return dome;
    }
    let ndc = vec4<f32>(uc.x * 2.0 - 1.0, 1.0 - uc.y * 2.0, 1e-6, 1.0);
    let wp = wu.inv_view_proj * ndc;
    let dc = normalize(wp.xyz / wp.w - wu.cam.xyz);
    let dome_c = w_atmo_dome(dc);
    // three taps across the edge (one cloud at the edge must not paint a
    // column down the sea), the sun's glare capped as above
    var drawn_c = vec3<f32>(0.0);
    for (var i = 0; i < 3; i = i + 1) {
        let ut = vec2<f32>(clamp(uc.x + f32(i - 1) * 0.12, 0.03, 0.97), uc.y);
        var s = textureSampleLevel(sky_prev, sky_rep, ut, 0.0).rgb;
        s = s * min(1.0, (lk_luma(dome_c) * 3.0 + 1e-4) / max(lk_luma(s), 1e-6));
        drawn_c = drawn_c + s * (1.0 / 3.0);
    }
    // mostly the brightness the sky slice gave the dome there, half its tint
    let lr = clamp(lk_luma(drawn_c) / max(lk_luma(dome_c), 1e-6), 0.35, 2.0);
    let tint = clamp(drawn_c / max(dome_c, vec3<f32>(1e-5)), vec3<f32>(0.2), vec3<f32>(3.0));
    return dome * mix(vec3<f32>(lr), tint, 0.5);
}

fn w_wave_ramp(f: vec2<f32>) -> f32 {
    return wg_smooth(50.0, 220.0, f.x) * (1.0 - 0.6 * f.y) + 0.25 * f.y * wg_smooth(0.0, 60.0, f.x);
}

// The camera and the mesh sample the same swell, breakers and swash.
fn w_displacement(base: vec2<f32>, spacing: f32, f: vec2<f32>) -> vec3<f32> {
    var disp = vec3<f32>(0.0);
    if (wu.flags.w > 0.5) {
        // ocean-open-sea.js: full spectrum offshore, ramped from zero at the
        // hand-off (50 m) to full at 220 m; at a beach a quarter of it comes ashore.
        // the beach share fades out toward the waterline, where the swash level takes over.
        let ramp = w_wave_ramp(f);
        if (ramp > 0.0) {
            let w = wg_band(spacing) * wg_cascade_amp(base.x);
            for (var c = 0u; c < 4u; c = c + 1u) {
                if (w[c] > 0.0) {
                    disp = disp + textureSampleLevel(w_disp, w_repeat, w_casc_uv(base, c), c, 0.0).xyz * w[c];
                }
            }
            // a storm's sea stands higher (the chop and the swell, 1.6 x at the storm preset)
            disp = disp * ramp; // the sea state's spectrum carries the storm's sea (was x (1 + 0.6 storm))
        }
    }
    // The breaking surf where the nearest shore is a beach (ocean-surf.js):
    // its height and its shoreward throw.
    // Only where the grid is fine enough to carry a breaking face: past ~2 m of
    // vertex spacing the face drew as a row of sawtooth triangles (the JS's
    // grid shows the same), so there the surf is shading and foam only.
    let surf_vw = 1.0 - wg_smooth(1.0, 2.5, spacing);
    if (f.y > 0.01 && f.x > -2.0 && f.x < 420.0 && surf_vw > 0.0) {
        let fr = w_shore_frame(base);
        let s = surf_at(wu.cam.w, f.x, fr.z, f.y, sf_amp_scale(base.x));
        // (sea r4: half the shoreward throw: on the 0.8 m grid the full curl folded the face over itself, and the folded triangles read as dark boxes on the breaker line)
        disp = disp + vec3<f32>(fr.x * s.shoreward * 0.5, s.height, fr.y * s.shoreward * 0.5) * surf_vw;
    }
    // the swash. Near a beach's waterline the water stands at the level
    // the last bore ran up to (`w_swash`): a level, so it meets the sand where
    // the drawn ground lies under it, whatever that ground's height.
    // (no sine swell: the waves simulation carries the swell)
    if (f.y > 0.01 && f.x < 40.0 && f.x > -120.0) {
        let frs = w_shore_frame(base);
        let sw = w_swash(frs.z, wu.cam.w, sf_amp_scale(base.x)).x * f.y;
        disp.y = mix(disp.y, max(disp.y, sw), wg_smooth(40.0, 6.0, f.x));
    }
    return disp;
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @location(0) pid: u32) -> VsOut {
    let side = WG_PATCH_QUADS + 1u;
    let gi = f32((pid % WG_PATCHES) * WG_PATCH_QUADS + vi % side);
    let gj = f32((pid / WG_PATCHES) * WG_PATCH_QUADS + vi / side);
    let r = wu.fwd.w;
    var local = vec2<f32>(wg_axis(gi, r), wg_axis(gj, r));
    // the grid's outermost lines are a skirt thrown out to 12 x the
    // radius (53 km at street level), so the sea reaches the true horizon and
    // past the far-shore water: no rows of seabed under the horizon, no gap
    // between the grid and the far water from the air.
    let last = f32(WG_PATCHES * WG_PATCH_QUADS);
    // (from low eyes only: from the air the grid already reaches 10 x the height,
    // and there the skirt showed the sky volume's shafts through it as radial bands)
    let sk = mix(12.0, 1.0, wg_smooth(60.0, 200.0, wu.cam.y - wu.grid.z));
    if (gi < 0.5 || gi > last - 0.5) {
        local.x = local.x * sk;
    }
    if (gj < 0.5 || gj > last - 0.5) {
        local.y = local.y * sk;
    }
    let base = wu.grid.xy + local;
    let f = w_field_at(base);
    let spacing = max(wg_spacing(local.x, r), wg_spacing(local.y, r));
    let disp = w_displacement(base, spacing, f);
    var wv_far = vec4<f32>(0.0);
    let ramp = w_wave_ramp(f);
    if (wu.flags.w > 0.5 && ramp > 0.0) {
        wv_far = wv_vertex_slope(base, spacing) * ramp;
    }
    let world = vec3<f32>(base.x + disp.x, wu.grid.z + disp.y, base.y + disp.z);
    var o: VsOut;
    o.clip = wu.view_proj * vec4<f32>(world, 1.0);
    // past the far plane (reverse Z below 0) held just inside it, so the
    // sea runs on into the haze from the air; it never writes depth.
    // past it the far layer's depth, 1e-9 / view depth (eo-farshore DEPTH_K), so the land stops the sea.
    o.clip.z = max(o.clip.z, 1e-9); // DEPTH_K
    o.world = world;
    o.base = base;
    o.atmo_g = w_atmo_gain();
    o.wv_far = wv_far;
    return o;
}

// ---- MaterialX Perlin noise (three's mx_noise_float, which the JS samples) ----

fn mx_rotl(x: u32, k: u32) -> u32 {
    return (x << k) | (x >> (32u - k));
}

fn mx_hash3(x: i32, y: i32, z: i32) -> u32 {
    let seed = 0xdeadbeefu + (3u << 2u) + 13u;
    var a = seed + bitcast<u32>(x);
    var b = seed + bitcast<u32>(y);
    var c = seed + bitcast<u32>(z);
    c = c ^ b;
    c = c - mx_rotl(b, 14u);
    a = a ^ c;
    a = a - mx_rotl(c, 11u);
    b = b ^ a;
    b = b - mx_rotl(a, 25u);
    c = c ^ b;
    c = c - mx_rotl(b, 16u);
    a = a ^ c;
    a = a - mx_rotl(c, 4u);
    b = b ^ a;
    b = b - mx_rotl(a, 14u);
    c = c ^ b;
    c = c - mx_rotl(b, 24u);
    return c;
}

fn mx_grad(hash: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = hash & 15u;
    let u = select(y, x, h < 8u);
    let v = select(select(z, x, h == 12u || h == 14u), y, h < 4u);
    return select(u, -u, (h & 1u) != 0u) + select(v, -v, (h & 2u) != 0u);
}

fn mx_fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn mx_noise(p: vec3<f32>) -> f32 {
    let fl = floor(p);
    let i = vec3<i32>(fl);
    let f = p - fl;
    let u = mx_fade(f.x);
    let v = mx_fade(f.y);
    let w = mx_fade(f.z);
    let g000 = mx_grad(mx_hash3(i.x, i.y, i.z), f.x, f.y, f.z);
    let g100 = mx_grad(mx_hash3(i.x + 1, i.y, i.z), f.x - 1.0, f.y, f.z);
    let g010 = mx_grad(mx_hash3(i.x, i.y + 1, i.z), f.x, f.y - 1.0, f.z);
    let g110 = mx_grad(mx_hash3(i.x + 1, i.y + 1, i.z), f.x - 1.0, f.y - 1.0, f.z);
    let g001 = mx_grad(mx_hash3(i.x, i.y, i.z + 1), f.x, f.y, f.z - 1.0);
    let g101 = mx_grad(mx_hash3(i.x + 1, i.y, i.z + 1), f.x - 1.0, f.y, f.z - 1.0);
    let g011 = mx_grad(mx_hash3(i.x, i.y + 1, i.z + 1), f.x, f.y - 1.0, f.z - 1.0);
    let g111 = mx_grad(mx_hash3(i.x + 1, i.y + 1, i.z + 1), f.x - 1.0, f.y - 1.0, f.z - 1.0);
    let s1 = 1.0 - u;
    let t1 = 1.0 - v;
    let r1 = 1.0 - w;
    let res = r1 * (t1 * (g000 * s1 + g100 * u) + v * (g010 * s1 + g110 * u)) + w * (t1 * (g001 * s1 + g101 * u) + v * (g011 * s1 + g111 * u));
    return 0.982 * res;
}

// ---- tidewater's optics (ocean-water-optics.js) ----

fn wo_fresnel(cos_i: f32, eta: f32) -> f32 {
    let c = clamp(cos_i, 0.0, 1.0);
    let g2 = eta * eta - 1.0 + c * c;
    if (g2 < 0.0) {
        return 1.0;
    }
    let g = sqrt(g2);
    let a = (g - c) / (g + c);
    let b = (c * (g + c) - 1.0) / (c * (g - c) + 1.0);
    return a * a * 0.5 * (b * b + 1.0);
}

fn wo_phase(cos_t: f32) -> f32 {
    let g = WO_PHASE_G;
    let g2 = g * g;
    let hg = ((1.0 - g2) / (4.0 * WG_PI)) / pow(max(1.0 + g2 - 2.0 * g * cos_t, 1e-4), 1.5);
    return hg * 0.7 + 0.3 / (4.0 * WG_PI);
}

fn wo_refracted_sun_up(l: vec3<f32>) -> vec3<f32> {
    return -refract(-l, vec3<f32>(0.0, 1.0, 0.0), 1.0 / WO_IOR);
}

// `waterVolume`: transmitted = scene * exp(-sigT L) + single-scattered sun + sky.
struct Volume {
    tview: vec3<f32>,
    inscatter: vec3<f32>,
};

fn wo_volume(path: f32, tv: vec3<f32>, l: vec3<f32>, sun: vec3<f32>, sky_irr: vec3<f32>, sig_a: vec3<f32>, sig_s: vec3<f32>, bb_share: f32) -> Volume {
    let sig_t = sig_a + sig_s;
    let ls = wo_refracted_sun_up(l);
    let mu_s = max(ls.y, 0.1);
    let mu_v = max(-tv.y, 0.15);
    let tview = exp(-sig_t * path);
    let sun_in = sun * (1.0 - wo_fresnel(max(l.y, 0.02), WO_IOR));
    let k_sun = sig_t * (mu_v / mu_s + 1.0);
    let k_amb = sig_t * (mu_v / 0.75 + 1.0);
    let ph = wo_phase(dot(tv, ls));
    let bb = sig_s * bb_share;
    let albedo_ms = bb * (0.33 * 4.0) / (sig_a + bb);
    let in_sun = sun_in * (sig_s * ph + albedo_ms * sig_t * (1.0 / WG_PI)) * (vec3<f32>(1.0) - exp(-k_sun * path)) / k_sun;
    let in_amb = sky_irr * (sig_s * 0.25 + albedo_ms * sig_t) * (vec3<f32>(1.0) - exp(-k_amb * path)) / k_amb;
    var o: Volume;
    o.tview = tview;
    o.inscatter = in_sun + in_amb;
    return o;
}

// `underwaterLightModulation` (sun share 0.6, no caustics), clear constants.
// Optical estimates for a sheltered, silty shoreline. Smoothly return to
// open-bay values beyond the sampled cove. Coordinates use the city's map scale.
fn wo_cove(xz: vec2<f32>) -> f32 {
    if (!W_COVE_DETAIL) { return 0.0; }
    let p = xz / W_COVE_SCALE;
    let lo = smoothstep(vec2<f32>(-725.0, -3750.0), vec2<f32>(-635.0, -3620.0), p);
    let hi = 1.0 - smoothstep(vec2<f32>(-150.0, -3150.0), vec2<f32>(-65.0, -3030.0), p);
    return lo.x * lo.y * hi.x * hi.y;
}
const WO_COVE_A = vec3<f32>(1.12, 0.93, 1.14);
const WO_COVE_S = vec3<f32>(0.16, 0.20, 0.15);

fn wo_underwater(depth: f32, l: vec3<f32>) -> vec3<f32> {
    let sig_t = vec3<f32>(0.432, 0.093, 0.059);
    let ls = refract(-l, vec3<f32>(0.0, 1.0, 0.0), 1.0 / WO_IOR);
    let mu = max(-ls.y, 0.15);
    let d = max(depth, 0.0);
    let direct = mix(vec3<f32>(1.0), exp(-sig_t * d / mu), wg_smooth(0.0, 0.08, d));
    let amb = mix(vec3<f32>(1.0), exp(-sig_t * d * 1.2) * 0.85 + vec3<f32>(0.0, 0.02, 0.04) * exp(-d * 0.1), wg_smooth(0.0, 0.1, d));
    return mix(amb, direct, 0.6);
}

// Focus the direct beam where neighbouring refracted rays converge on a receiver.
// A finite aperture and a bounded gain soften the geometric focus at a fold.
fn w_caustic(receiver: vec3<f32>, depth: f32, sun_up: vec3<f32>) -> f32 {
    let entry = receiver.xz + sun_up.xz * (depth / max(sun_up.y, 0.1));
    let step = wu.sizes.w / 256.0 * 3.0;
    let uv = w_casc_uv(entry, 3u);
    let du = step / wu.sizes.w;
    let slope = textureSampleLevel(w_deriv, w_repeat, uv, 3u, 0.0).xy;
    let dx = textureSampleLevel(w_deriv, w_repeat, uv + vec2<f32>(du, 0.0), 3u, 0.0).xy - slope;
    let dz = textureSampleLevel(w_deriv, w_repeat, uv + vec2<f32>(0.0, du), 3u, 0.0).xy - slope;
    let k = depth * (1.0 - 1.0 / WO_IOR) * wg_cascade_amp(entry.x).w / step;
    let area = (1.0 + k * dx.x) * (1.0 + k * dz.y) - k * k * dx.y * dz.x;
    return clamp(1.0 / max(abs(area), 0.3), 0.45, 2.5);
}

// ---- the look's cascaded sun shadow (look_light.wgsl `lk_shadow`) ----

fn w_shadow(world: vec3<f32>, ng: vec3<f32>) -> f32 {
    let vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    let split = look.cascade_split;
    if (look.key_dir.w <= 0.0) {
        return 1.0;
    }
    // past the cascades the far sun shadow map (far_shadow.wgsl, `--far-shadows`): the city's shadow on the bay.
    let far = lk_far_on();
    if (vd >= split.z) {
        if (!far) {
            return 1.0;
        }
        return mix(1.0, lk_far_shadow(world, ng), look.key_dir.w);
    }
    var c = 0;
    if (vd > split.x) {
        c = 1;
    }
    if (vd > split.y) {
        c = 2;
    }
    let b = look.cascade_bias[c];
    let p = world + ng * b.x;
    let clip = look.cascade_vp[c] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let z = clip.z + b.y;
    let texel = 1.0 / f32(textureDimensions(lk_shadow_map).x);
    let h = 0.5 * texel;
    let vis = 0.25 * (textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(-h, -h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(h, -h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(-h, h), c, z)
        + textureSampleCompareLevel(lk_shadow_map, lk_shadow_cmp, uv + vec2<f32>(h, h), c, z));
    let fade = lk_smooth(split.w, split.z, vd);
    if (far && fade > 0.0) {
        return mix(1.0, mix(vis, lk_far_shadow(world, ng), fade), look.key_dir.w);
    }
    return mix(1.0, mix(vis, 1.0, fade), look.key_dir.w);
}

// The environment's irradiance (look_light.wgsl `lk_sh`).
fn w_sh(n: vec3<f32>) -> vec3<f32> {
    let x = n.x;
    let y = n.y;
    let z = n.z;
    var e = look.sh[0].xyz * 0.282095;
    e = e + look.sh[1].xyz * (0.488603 * y) + look.sh[2].xyz * (0.488603 * z) + look.sh[3].xyz * (0.488603 * x);
    e = e + look.sh[4].xyz * (1.092548 * x * y) + look.sh[5].xyz * (1.092548 * y * z) + look.sh[6].xyz * (0.315392 * (3.0 * z * z - 1.0));
    e = e + look.sh[7].xyz * (1.092548 * x * z) + look.sh[8].xyz * (0.546274 * (x * x - y * y));
    return max(e, vec3<f32>(0.0));
}

// ---- the scene probe (facade_age.wgsl `fa_probe_sample`) ----

fn w_probe_sample(d: vec3<f32>, lod: f32) -> vec3<f32> {
    let a = abs(d);
    var f = 0u;
    var fw = vec3<f32>(1.0, 0.0, 0.0);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (a.x >= a.y && a.x >= a.z) {
        f = select(1u, 0u, d.x > 0.0);
        fw = vec3<f32>(sign(d.x), 0.0, 0.0);
    } else if (a.y >= a.z) {
        f = select(3u, 2u, d.y > 0.0);
        fw = vec3<f32>(0.0, sign(d.y), 0.0);
        up = vec3<f32>(0.0, 0.0, -sign(d.y));
    } else {
        f = select(5u, 4u, d.z > 0.0);
        fw = vec3<f32>(0.0, 0.0, sign(d.z));
    }
    let right = cross(fw, up);
    let k = 1.0 / max(dot(d, fw), 1e-6);
    let uv = vec2<f32>(dot(d, right) * k * 0.5 + 0.5, 0.5 - dot(d, up) * k * 0.5);
    return textureSampleLevel(w_probe, w_probe_samp, uv, f, lod).rgb;
}

// ---- screen helpers ----

fn w_to_screen(p: vec3<f32>) -> vec3<f32> {
    let c = wu.view_proj * vec4<f32>(p, 1.0);
    let ndc = c.xyz / max(c.w, 1e-4);
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, c.w);
}

// View depth (along the camera's forward axis) of what the frame holds at uv;
// 1e9 for the sky.
fn w_scene_depth(uv: vec2<f32>) -> f32 {
    let px = vec2<i32>(clamp(uv, vec2<f32>(0.0), vec2<f32>(0.99999)) * vec2<f32>(textureDimensions(w_depth)));
    let d = textureLoad(w_depth, px, 0);
    if (d <= 0.0) {
        return 1e9;
    }
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let w = wu.inv_view_proj * ndc;
    let p = w.xyz / w.w;
    return max(dot(p - wu.cam.xyz, wu.fwd.xyz), 1e-9 / d); // past the far plane the far layer's depth is 1e-9 / view depth
}

fn w_view_depth(p: vec3<f32>) -> f32 {
    return dot(p - wu.cam.xyz, wu.fwd.xyz);
}

fn w_edge(uv: vec2<f32>) -> f32 {
    return wg_smooth(0.0, WS_UV_EDGE, uv.x) * wg_smooth(0.0, WS_UV_EDGE, 1.0 - uv.x) * wg_smooth(0.0, WS_UV_EDGE, uv.y) * wg_smooth(0.0, WS_UV_EDGE, 1.0 - uv.y);
}

// The reflection march (water-surface.js `buildReflectionMarch`, tidewater's
// `_waterSSR`): growing view-space steps scaled by the fragment's range, a
// thickness test over the last step, bisection, the touch test. Returns
// (uv, weight).
fn w_march(origin: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    let range = w_view_depth(origin);
    let step_scale = max(range / WS_MARCH_REF, 1.0);
    var t = step_scale * WS_MARCH_START;
    var dt = step_scale * WS_MARCH_FIRST;
    var prev_t = 0.0;
    var hit = false;
    for (var i = 0u; i < WS_MARCH_STEPS; i = i + 1u) {
        prev_t = t;
        t = t + dt;
        dt = dt * WS_MARCH_GROWTH;
        let p = origin + dir * t;
        let s = w_to_screen(p);
        let rz = w_view_depth(p);
        if (s.x < 0.0 || s.x > 1.0 || s.y < 0.0 || s.y > 1.0 || rz < 0.1) {
            break;
        }
        let sz = w_scene_depth(s.xy);
        let thickness = max(dt * 1.3, max(t * 0.08, 0.3));
        if (rz > sz && rz - sz < thickness) {
            hit = true;
            break;
        }
    }
    if (!hit) {
        return vec3<f32>(0.5, 0.5, 0.0);
    }
    var a = prev_t;
    var b = t;
    for (var i = 0u; i < WS_MARCH_REFINE; i = i + 1u) {
        let m = (a + b) * 0.5;
        let p = origin + dir * m;
        if (w_view_depth(p) > w_scene_depth(w_to_screen(p).xy)) {
            b = m;
        } else {
            a = m;
        }
    }
    let hp = origin + dir * b;
    let s = w_to_screen(hp);
    let gap = abs(w_scene_depth(s.xy) - w_view_depth(hp));
    let touch = wg_smooth(max(b * 0.04, 0.4), max(b * 0.02, 0.2), gap);
    let above_sea = wg_smooth(wu.grid.z - 0.15, wu.grid.z + 0.35, hp.y);
    let facing = wg_smooth(0.5, 0.1, -dot(dir, wu.fwd.xyz));
    let span = step_scale * WS_MARCH_SPAN;
    let in_reach = wg_smooth(span, span * 0.8, b);
    return vec3<f32>(s.xy, w_edge(s.xy) * facing * touch * above_sea * in_reach);
}

// The frame at the reflected ray's vanishing point, depth-tested
// (`buildVanishingPointTap`).
fn w_far_tap(origin: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    let c = wu.view_proj * vec4<f32>(dir, 0.0);
    let ndc = c.xy / max(c.w, 1e-4);
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let inside = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let range = w_view_depth(origin);
    let scene_range = w_scene_depth(inside);
    let beyond = wg_smooth(range * WS_FAR_BEYOND.x + WS_FAR_BEYOND.y, range * WS_FAR_BEYOND.z + WS_FAR_BEYOND.w, scene_range);
    let ahead = wg_smooth(WS_FAR_AHEAD.x, WS_FAR_AHEAD.y, dot(dir, wu.fwd.xyz));
    let rising = wg_smooth(WS_FAR_RISE.x, WS_FAR_RISE.y, dir.y);
    // geometry only; where it lands on sky the drawn sky (`w_drawn_sky`) stands.
    let valid = select(0.0, 1.0, c.w > 1e-4 && scene_range < 1e8);
    return vec3<f32>(inside, beyond * ahead * rising * w_edge(uv) * valid);
}

// ---- the surface ----

struct Shade {
    // radiance leaving the surface toward the eye, the scene term excluded
    own: vec3<f32>,
    // what multiplies the scene behind (transmittance, Fresnel, foam, fog)
    through: vec3<f32>,
    // where the refracted bottom is read in the copy
    uv: vec2<f32>,
    // the scene's mirror image (copy pass only) weight and uv
    mirror_uv: vec2<f32>,
    mirror_w: f32,
    // the reflection's weight (Fresnel x (1 - foam) x (1 - fog)) and the sky it stands in for
    refl_k: f32,
    sky: vec3<f32>,
    smear: f32,
    // this facet faces the eye enough to show a light above it (a column's dash)
    dash: f32,
    // the facet's tilt across the view (its normal's screen-sideways part)
    side: f32,
    // how near the mirror direction is to the sun (the glare round it)
    sun_near: f32,
};

fn w_capillary(p: vec2<f32>) -> vec2<f32> {
    let a0 = 0.63;
    let a1 = 2.14;
    let r0 = vec2<f32>(p.x * cos(a0) - p.y * sin(a0), p.x * sin(a0) + p.y * cos(a0)) * 7.3;
    let r1 = vec2<f32>(p.x * cos(a1) - p.y * sin(a1), p.x * sin(a1) + p.y * cos(a1)) * 3.1;
    let g0 = textureSampleLevel(w_deriv, w_repeat, w_casc_uv(r0, 3u), 3u, 0.0).xy;
    let g1 = textureSampleLevel(w_deriv, w_repeat, w_casc_uv(r1, 3u), 3u, 0.0).xy;
    let b0 = vec2<f32>(g0.x * cos(a0) + g0.y * sin(a0), -g0.x * sin(a0) + g0.y * cos(a0));
    let b1 = vec2<f32>(g1.x * cos(a1) + g1.y * sin(a1), -g1.x * sin(a1) + g1.y * cos(a1));
    return b0 * 0.55 + b1 * 0.35;
}

fn w_chop(p: vec2<f32>, t: f32, footprint: f32, fine_fade: f32, rough: f32) -> vec2<f32> {
    var sx = 0.0;
    var sz = 0.0;
    // The chop (v15): three octaves of noise slope at non-harmonic spacings,
    // each turned off the wind and drifting at its own phase speed, scaled by
    // a travelling patchiness.
    if (fine_fade > 0.0) {
        let patchy = mx_noise(vec3<f32>((p.x - t * WS_WIND_DIR.x * 1.4) / WS_PATCH_M, (p.y - t * WS_WIND_DIR.y * 1.4) / WS_PATCH_M, t * 0.03));
        let ruffle = rough * mix(WS_PATCH_RANGE.x, WS_PATCH_RANGE.y, wg_smooth(-0.3, 0.35, patchy));
        var octs = array<vec3<f32>, 3>(WS_OCT0, WS_OCT1, WS_OCT2);
        for (var i = 0u; i < 3u; i = i + 1u) {
            let o = octs[i];
            let lam = 1.0 / (max(WS_CHOP_ALONG, WS_CHOP_CROSS) * o.x);
            let fade = fine_fade * (1.0 - wg_smooth(lam * WS_RESOLVE.x, lam * WS_RESOLVE.y, footprint));
            if (fade <= 0.0) {
                continue;
            }
            let ax = WS_WIND_DIR.x * cos(o.y) - WS_WIND_DIR.y * sin(o.y);
            let az = WS_WIND_DIR.x * sin(o.y) + WS_WIND_DIR.y * cos(o.y);
            let drift_v = WS_CHOP_DRIFT / sqrt(o.x);
            let dx0 = p.x - t * ax * drift_v;
            let dz0 = p.y - t * az * drift_v;
            let nt = t * 0.55 * sqrt(o.x) + f32(i) * 17.3;
            let step = WS_CHOP_STEP / o.x;
            let n0 = mx_noise(vec3<f32>((dx0 * ax + dz0 * az) * WS_CHOP_ALONG * o.x, (-dx0 * az + dz0 * ax) * WS_CHOP_CROSS * o.x, nt));
            let xs = dx0 + step;
            let n1 = mx_noise(vec3<f32>((xs * ax + dz0 * az) * WS_CHOP_ALONG * o.x, (-xs * az + dz0 * ax) * WS_CHOP_CROSS * o.x, nt));
            let zs = dz0 + step;
            let n2 = mx_noise(vec3<f32>((dx0 * ax + zs * az) * WS_CHOP_ALONG * o.x, (-dx0 * az + zs * ax) * WS_CHOP_CROSS * o.x, nt));
            let gain = fade * (WS_CHOP_GAIN * sqrt(o.z) * WS_CHOP_UNDER_FFT / o.x) * ruffle;
            sx = sx + (n1 - n0) / step * gain;
            sz = sz + (n2 - n0) / step * gain;
        }
    }
    return vec2<f32>(sx, sz);
}

fn w_shade(world: vec3<f32>, base: vec2<f32>, frag: vec4<f32>, with_copy: bool) -> Shade {
    let t = wu.cam.w;
    let footprint = clamp(max(length(dpdx(world.xz)), length(dpdy(world.xz))), 0.0, 64.0);
    let fs = w_field_at(base);
    let signed = fs.x;
    let beach = fs.y;
    // The mask (ocean-open-sea.js): past the zero line by FAR_SHORE_REACH_M,
    // by 60 m where the nearest shore is a beach. Land above the sea hides
    // the rest by depth.
    let mask = select(wu.flags.z, wu.probe.w, beach > 0.5);
    let reach = wg_reach(); // past the terrain grid over open water
    // (sea r8: past the shore field's region its clamped edge texels hold the coast's
    // cut as long stripes; the skirt there is open sea)
    let in_field = all(world.xz > wu.field.xy) && all(world.xz < wu.field.xy + wu.field.zw);
    if ((signed < mask && in_field) || world.x < reach.x || world.x > reach.z || world.z < reach.y || world.z > reach.w) {
        discard;
    }
    let sf_hand = 1.0 - surf_cover(base, fs); // the surf strip draws here (water_surf.wgsl), the sea stands down
    if (sf_inj.on < 0.5 && sf_hand <= 0.001) {
        discard;
    }
    let to_cam = wu.cam.xyz - world;
    let dist = length(to_cam);
    let v = to_cam / max(dist, 1e-4);
    let view_dist = max(w_view_depth(world), 1e-3);
    let fine_fade = 1.0 - wg_smooth(WS_FINE_FADE.x, WS_FINE_FADE.y, view_dist);
    let swell_fade = 1.0 - wg_smooth(400.0, 2500.0, view_dist);
    let p = world.xz;
    // The water column under this pixel: the frame's depth behind the surface
    // along the view ray, made vertical (water-surface.js `column`).
    let uv_here = frag.xy / vec2<f32>(textureDimensions(w_depth));
    let ray_scale = dist / view_dist;
    let straight = max(w_scene_depth(uv_here) - view_dist, 0.0) * ray_scale;
    let column = min(straight, 400.0) * max(abs(v.y), 0.02);
    // The collar's shore distance. The JS reads the shoreline rings' distance
    // (its ground is pressed to the seabed under the rings' water and a wall
    // stands on the ring line, so ring, ground and wall agree there). Here what
    // stands at the waterline is the surveyed ground and the street world's
    // banks, whose waterline can lie metres off the rings', so the wash runs
    // where the water actually meets something: the column under the pixel,
    // as a distance on a 1:15 face (a wall or a steep bank).
    let shore = column * 15.0;
    let det = w_sea_detail(p, t);
    let storm = w_storm(); // a storm roughens the whole sea
    // the sun-keyed westerly, the swell banding and the storm's noise caps are
    // retired: the waves simulation carries the wind, the swell and the whitecaps
    let rough = det.x * wv_ripple(p.x); // x the wind's ripples (glassy in a calm)

    // --- the shore collar (water-surface.js): a wandering break line, grain
    // scrolling shoreward on the shore distance, dissolve, spindrift.
    let surge_a = sin(p.x * 0.085 - p.y * 0.062 + t * 0.62) * 0.5;
    let surge_b = sin(p.x * 0.031 + p.y * 0.043 - t * 0.37) * 0.5;
    let surge_n = mx_noise(vec3<f32>(p.x * 0.021, p.y * 0.021, t * 0.13)) * 0.5;
    let surge = clamp(surge_a * 0.62 + surge_b * 0.5 + surge_n * 0.55 + 0.5, 0.0, 1.0);
    let break_line = surge * (WS_RUNUP * WS_HS) + WS_RUNUP * WS_HS * 0.5;
    var relief = 0.0;
    var collar = 0.0;
    var drift = 0.0;
    if (shore < break_line + 8.0 * WS_DRIFT_CARRY) {
        relief = mx_noise(vec3<f32>(p.x * WS_GRAIN_FREQ, p.y * WS_GRAIN_FREQ, shore * 0.9 - t * 1.2));
        let grain = (mx_noise(vec3<f32>(p.x * 0.115, p.y * 0.115, shore * 0.42 - t * 1.05)) * 0.5 + 0.5) * 0.62 + (relief * 0.5 + 0.5) * 0.38;
        let wash = clamp(1.0 - shore / break_line, 0.0, 1.0);
        collar = wg_smooth(WS_COLLAR.x, WS_COLLAR.y, wash * (grain * 0.86 + 0.42));
        // slap foam against a wall comes and goes in patches (it read as a
        // glowing ruled line along the seawall and the pier)
        collar = collar * wg_smooth(-0.1, 0.45, mx_noise(vec3<f32>(p * 0.22, t * 0.35))) * 0.7;
        let shore_foam = wg_smooth(WS_COLLAR.x, WS_COLLAR.y, grain * 0.86 + 0.42);
        let torn = wg_smooth(WS_DRIFT_TEAR.x, WS_DRIFT_TEAR.y, grain);
        drift = torn * shore_foam * exp(-max(shore - break_line, 0.0) / WS_DRIFT_CARRY) * WS_DRIFT_TAKE;
    }

    // the swash sheet on a beach: a lacy foam front where the film runs
    // out on the sand, the bore's foam carried up in the uprush and left as
    // lace while it drains; `film` where the water lies thin over sand.
    var fr = vec3<f32>(0.0);
    var lace = 0.0;
    var swash_foam = 0.0;
    var film = 0.0;
    if (beach > 0.01) {
        fr = w_shore_frame(base);
        if (signed < 420.0 && signed > -120.0) {
            lace = w_lace(p, fr.xy, t, footprint);
        }
        if (signed < 40.0 && signed > -120.0) {
            let sw = w_swash(fr.z, t, sf_amp_scale(base.x));
            let front = wg_smooth(0.022, 0.002, column);
            let carried = (1.0 - wg_smooth(0.04, 0.45, sw.y)) * 0.5;
            let cover = clamp(front * 0.7 + carried, 0.0, 1.0);
            let near_shore = wg_smooth(30.0, 4.0, signed) * beach;
            swash_foam = wg_smooth(1.0 - cover, 1.1 - cover, lace) * cover * near_shore;
            film = near_shore * wg_smooth(0.35, 0.0, column);
            // The sheet's edge is ragged: it runs out where the sand stands
            // within a few millimetres to centimetres of its level.
            let rag = mx_noise(vec3<f32>(p * 0.7, 3.3)) * 0.6 + mx_noise(vec3<f32>(p * 2.3, 1.1)) * 0.4;
            if (film > 0.5 && column < 0.012 * (rag + 0.9)) {
                // Past the edge the sand shows as the frame drew it: wholly see-through
                // (fs_copy draws the copy there, fs_blend's alpha is 0), not a discard,
                // which left these pixels black (a black rim along the swash's edge).
                var bare: Shade;
                bare.through = vec3<f32>(1.0);
                bare.uv = uv_here;
                bare.mirror_uv = uv_here;
                return bare;
            }
        }
    }

    // --- slopes: the FFT cascades, the capillary taps, the chop octaves.
    var sx = 0.0;
    var sz = 0.0;
    let amp = wg_cascade_amp(world.x);
    var whitecap = 0.0;
    if (wu.flags.w > 0.5) {
        // The gusts and slicks scale the two finest cascades, the third finest by 40 % of it.
        let rg = rough / wv_ripple(p.x); // the cascades carry the wind themselves
        let fb = wg_band(footprint); // waves
        var wv_raw = 0.0; // the whitecap foam and the fine chop that tears it
        var wv_fine = 0.0;
        let wb = fb * amp * vec4<f32>(1.0, mix(1.0, rg, 0.4), rg, rg);
        for (var c = 0u; c < 4u; c = c + 1u) {
            if (wb[c] > 0.0) {
                let uv = w_casc_uv(p, c);
                let d = textureSampleLevel(w_deriv, w_repeat, uv, c, 0.0);
                sx = sx + d.x * wb[c] * swell_fade;
                sz = sz + d.y * wb[c] * swell_fade;
                wv_raw = max(wv_raw, wv_foam(d, p.x, det.y) * fb[c]); // the place's whitecap foam of this cascade
                wv_fine = wv_fine + select(0.0, (d.x + d.y) * wb[c], c >= 2u);
            }
        }
        // the far slope from the vertex where the pixel's own cascades 0-1 have faded
        sx = sx + wv_far_slope.x * (1.0 - fb.x) + wv_far_slope.z * (1.0 - fb.y);
        sz = sz + wv_far_slope.y * (1.0 - fb.x) + wv_far_slope.w * (1.0 - fb.y);
        // the whitecap: a fresh core, lace as it ages, the mean cover far off
        let wv_farness = wg_smooth(2.0, 3.5, wv_lod(footprint, 1u)); // (r2b: as cascade 1 fades with the footprint)
        var wv_lace = 0.0;
        if (wv_raw > 0.02 && wv_farness < 0.98) {
            wv_lace = mx_noise(vec3<f32>(p * 0.9, t * 0.05)) * 0.65 + mx_noise(vec3<f32>(p * 2.6, t * 0.11 + 3.0)) * 0.35;
        }
        whitecap = wv_edge(wv_raw, wv_fine, wv_lace * 1.6, wv_farness, p.x);
        // Near-field capillary ripples: the finest cascade re-read at ~1 m and
        // ~2.3 m tiles, rotated, where a pixel is under 4 cm (tidewater).
        let near = wg_smooth(0.04, 0.01, footprint) * rough;
        if (near > 0.0) {
            let g = w_capillary(p);
            sx = sx + g.x * near;
            sz = sz + g.y * near;
        }
    }
    // rain on the water: rings where a pixel resolves them.
    let rain = select(0.0, wg_smooth(0.2, 8.0, skyu.wet.z), sky_on(skyu.flags.x, SKY_RAIN));
    if (rain > 0.0) {
        let near_r = wg_smooth(0.15, 0.04, footprint) * rain;
        if (near_r > 0.0) {
            let g = w_rain_rings(p, skyu.wet.w);
            sx = sx + g.x * near_r;
            sz = sz + g.y * near_r;
        }
    }
    let chop = w_chop(p, t, footprint, fine_fade, rough);
    sx += chop.x;
    sz += chop.y;
    // The breaking surf where the nearest shore is a beach (ocean-surf.js via
    // ocean-open-sea.js): its foam (with the whitecaps, over the surf zone),
    // its face's slope one step shoreward; the collar stands down there.
    var surf_cov = 0.0;
    // the breaker under this pixel: its face, its fresh whitewater, its break
    var surf_face = 0.0;
    var surf_ww = 0.0;
    var surf_bored = 0.0;
    if (beach > 0.01 && signed > -4.0 && signed < 420.0) {
        let sa = sf_amp_scale(base.x);
        let s0 = surf_at(t, signed, fr.z, beach, sa);
        let s1 = surf_at(t, signed - 0.6, fr.z, beach, sa);
        let dhds = (s1.height - s0.height) / 0.6;
        sx = sx + fr.x * dhds;
        sz = sz + fr.y * dhds;
        // fresh whitewater nearly solid; the foam a bore leaves behind
        // torn into lace that thins as it ages (most of it gone before the
        // next bore, so the lines of breakers stand apart over darker water).
        // (in the last metres the bore is a thin sheet: its foam a lace, no solid band)
        let inner = mix(0.45, 1.0, wg_smooth(2.0, 18.0, signed));
        // (sea r2: as much foam as the waves have height: a sheltered bay beach
        // keeps a thin swash line, not the Pacific's surf lace)
        let sized = wg_smooth(0.12, 0.7, sa);
        let fresh = sf_fresh_foam(s0.whitewater, lace, p, fr.xy, t, inner) * sized; // broken behind the roller, torn into lace near the sand (water_surf.wgsl)
        // (in a storm the whole surf zone churns: whitewater stays dense between the bores)
        let churn = storm * sized * wg_smooth(-2.0, 25.0, signed) * wg_smooth(520.0, 260.0, signed) * 0.95; // (denser, wider)
        let old = clamp(max(sf_aged_patches((s0.bored * (0.8 * exp(-s0.age / 2.6) + 0.06) * sized + whitecap * (0.3 + 0.5 * storm)) * inner, p, fr.xy, t), churn), 0.0, 1.0); // aged foam in rafts and clearings (water_surf.wgsl)
        let torn = wg_smooth(1.0 - old, 1.14 - old, lace);
        surf_cov = max(fresh, torn) * wg_smooth(-4.0, 0.5, signed) * sf_hand; // x sf_hand
        surf_face = clamp(s0.face * 1.5, 0.0, 1.0) * sized;
        surf_ww = s0.whitewater * sized;
        surf_bored = s0.bored * sized;
        whitecap = mix(whitecap, surf_cov, beach * sf_hand); // x sf_hand
        collar = collar * (1.0 - beach);
        drift = drift * (1.0 - beach);
    }
    sf_inject(&sx, &sz, &whitecap, &surf_cov, &surf_face, &surf_ww, &surf_bored, &swash_foam, base, fs, column); // the strip's own surf (water_surf.wgsl)
    // Foam: the collar under the spray and the whitecaps (`foamCoverage`).
    let spray = clamp(drift + whitecap, 0.0, 1.0);
    // + the swash's foam (opaque, as the surf's).
    let foam = max(clamp(collar + spray * (1.0 - collar), 0.0, 1.0), swash_foam);
    surf_cov = max(surf_cov, swash_foam);
    // a thin sheet over sand is nearly flat (its ripples are millimetres).
    // (0.35, not 0.75: the glassy sheet mirrored the bright sky as a column the eye read as a phantom glint)
    let calm = 1.0 - 0.35 * film;
    sx = sx * calm;
    sz = sz * calm;
    if (foam > 0.001) {
        // Foam relief: its own light and shade (0.30 RMS slope at full foam).
        let rs = 0.2 / WS_GRAIN_FREQ;
        let r1 = mx_noise(vec3<f32>((p.x + rs) * WS_GRAIN_FREQ, p.y * WS_GRAIN_FREQ, shore * 0.9 - t * 1.2));
        let r2 = mx_noise(vec3<f32>(p.x * WS_GRAIN_FREQ, (p.y + rs) * WS_GRAIN_FREQ, shore * 0.9 - t * 1.2));
        let rg = foam * 0.30 / (0.976 * WS_GRAIN_FREQ);
        sx = sx + (r1 - relief) / rs * rg;
        sz = sz + (r2 - relief) / rs * rg;
    }
    // the groundswell's slope (it tilts whole patches of the wind sea)
    let n = normalize(vec3<f32>(-sx, 1.0, -sz));

    // --- the sun lobe's roughness (Cox-Munk unresolved slope variance).
    let kpx = WG_PI / max(footprint, 1e-4);
    let unresolved = clamp(log2(110.0 / kpx) / 9.0, 0.0, 1.0);
    // (+ rain's splashes freckle the surface matte)
    let alpha2 = WO_ROUGHNESS * WO_ROUGHNESS + unresolved * rough * rough * WS_MSS * 2.0 + foam * 0.2 + rain * 0.08 + storm * 0.03; // (rain and storm matte) // (rain's splashes make the far water matte)
    let sigma_unres = sqrt(unresolved * rough * rough * WS_MSS);
    let roughness = clamp(sqrt(sqrt(alpha2)), 0.02, 0.95);

    // --- Fresnel and the reflected sky (raised by the unresolved slope, the
    // below-horizon share faded to the dark sea).
    let dot_nv = clamp(dot(n, v), 0.0, 1.0);
    // a rough sea at grazing angles shadows and masks its own facets,
    // so its reflectance near the horizon stays under about 0.72: the far band
    // reads darker than the sky over it instead of a clipped white strip.
    // (the cap only far out: near the eye a grazing view is a mirror, the bottom unseen)
    let fres = min(wo_fresnel(max(dot_nv, 1e-4), WO_IOR), mix(0.97, 0.72, wg_smooth(150.0, 1500.0, dist)));
    let r = reflect(-v, n);
    let rise = sigma_unres * 1.3 * (1.0 - max(r.y, 0.0));
    let rup = normalize(vec3<f32>(r.x, max(r.y, 0.004) + rise, r.z));
    // The sky as the frame draws it (the graded dome), so the mirror's sky
    // matches the sky pixels the screen-space march reads elsewhere; the scene
    // probe for what stands in it (buildings round the viewer), told apart
    // from its sky as fa_glazing tells them apart.
    var dome = w_drawn_sky(rup, w_atmo_dome(rup)); // the sky as drawn (the atmosphere sky)
    dome = fs_far_mirror(r, dome); // the far land (1e-9 / view depth, DEPTH_K) in the mirror under it
    if (wu.flags.y > 0.5) {
        let lod = clamp(log2(128.0) + 2.0 * log2(max(1.16 * roughness, 1e-4)), 0.0, 4.0);
        let pr = w_probe_sample(rup, lod);
        let an = lk_sky_sample(rup);
        let lp = lk_luma(pr);
        let la = lk_luma(an);
        // what stands in the mirror off the screen stands low over the
        // water; higher up the cube's texels are sky, and the drawn sky is the
        // one to show (a cube from another hour read as "geometry" and painted
        // the sea a stale blue).
        let sky_like = max(wg_smooth(0.55, 0.85, min(lp, la) / max(max(lp, la), 1e-6)), wg_smooth(0.12, 0.35, rup.y));
        dome = mix(pr, dome, sky_like);
    } else if (pg_live()) {
        // the probe grid (`--probe-grid`, probe_grid.wgsl) in the single probe's
        // place: the probes round this point of the water, the same test of
        // its sky, the drawn dome where no probe reaches.
        let ps = pg_select(world);
        if (ps.cover > 0.0) {
            let lod = clamp(lx_probe_table[0].y + 2.0 * log2(max(1.16 * roughness, 1e-4)), 0.0, lx_probe_table[0].z - 1.0);
            let pr = pg_sample(ps, rup, lod);
            let an = lk_sky_sample(rup);
            let lp = lk_luma(pr);
            let la = lk_luma(an);
            let sky_like = max(wg_smooth(0.55, 0.85, min(lp, la) / max(max(lp, la), 1e-6)), wg_smooth(0.12, 0.35, rup.y));
            dome = mix(dome, mix(pr, dome, sky_like), ps.cover);
        }
    }
    // the dark sea a below-horizon ray stands in for, from the sky this mirror shows.
    let sky0 = mix(dome * 0.35, dome, wg_smooth(-0.12, 0.08, r.y));
    // a wind-roughened surface mixes each pixel's mirror with the paler
    // sky around it: the reflected sky a fifth less saturated (the bay read pool-blue)
    // (more at dusk, when the low sky's belt and afterglow turned the bay magenta)
    let dusk_d = wg_smooth(0.25, 0.0, look.sun.y) * (1.0 - wg_smooth(0.6, 0.95, skyu.night.z));
    var sky = mix(sky0, vec3<f32>(lk_luma(sky0)), 0.28 + 0.3 * dusk_d); // (0.28: the Embarcadero read clean cobalt)
    // what the mirror cannot see in the frame but stands over the
    // water (the pier deck over its own shade): the reflected ray a few metres
    // up hidden behind nearer geometry is inside it, so the sky is not there.
    if (with_copy && r.y > 0.0 && sf_inj.rough < 0.5) { // not in the strip's churned water (nothing stands over the surf; its mirror is blurred)
        // a short march up the mirror ray (0.5 m to about 20 m, eight
        // growing steps) for what stands over the water at its foot (a wall face, a
        // deck), then a bisection to its surface, the frame there as the mirror
        // image (not the first step's point, which is off the true image)
        var hid = 0.0;
        var hcol = sky * 0.12;
        var a = 0.0;
        var b = 0.5;
        for (var k = 0; k < 8; k = k + 1) {
            let q = world + r * b;
            let sq = w_to_screen(q);
            if (sq.x <= 0.0 || sq.x >= 1.0 || sq.y <= 0.0 || sq.y >= 1.0) {
                break;
            }
            let sdp = w_scene_depth(sq.xy);
            let qd = w_view_depth(q);
            if (sdp < qd - 0.2 && qd - sdp < max(2.0, b * 0.6)) {
                hid = 1.0 - smoothstep(8.0, 20.0, b);
                break;
            }
            a = b;
            b = b * 1.6;
        }
        if (hid > 0.0) {
            for (var k = 0; k < 5; k = k + 1) {
                let m = 0.5 * (a + b);
                let q = world + r * m;
                if (w_view_depth(q) > w_scene_depth(w_to_screen(q).xy)) {
                    b = m;
                } else {
                    a = m;
                }
            }
            let sq = w_to_screen(world + r * b);
            hcol = textureSampleLevel(w_scene, w_clamp, sq.xy, 0.0).rgb * 0.9;
        }
        sky = mix(sky, min(hcol, sky * 2.0), hid);
    }
    // at night the city's glow on the low sky stands in the water
    // between the lamp columns (never darker than half the horizon's glow)
    if (skyu.night.z > 0.6) {
        // (night only: at blue hour the pink horizon band floored the bay magenta)
        let hg = w_atmo_dome(normalize(vec3<f32>(r.x, 0.03, r.z)));
        sky = max(sky, vec3<f32>(lk_luma(hg)) * 0.5 * wg_smooth(0.6, 0.95, skyu.night.z));
    }

    // --- the sun glint (GGX D V F NdL, clamped as tidewater clamps it) under the cascades.
    let l = look.key_dir.xyz;
    let ndl = max(dot(n, l), 0.0);
    // The cascaded sun shadow at the surface: on the glint, the foam and the
    // sun the water body scatters (buildings and piers shade the water).
    let cloud_t = sky_cloud_transmittance(world); // + the cloud decks' shadow
    let shadow = w_shadow(world, vec3<f32>(0.0, 1.0, 0.0)) * cloud_t;
    var glint = vec3<f32>(0.0);
    // the sun as it reaches the sea through the atmosphere (the sky slice's
    // `sun_glint`), past the rig's horizon envelope: the glitter path lasts until the
    // sun's upper limb sets
    // (r11b: its strength from `sun_glint`, its colour the key's while the sun is above
    // about 5 degrees, turning to `sun_glint`'s own colour only over the last 2-5 degrees:
    // morning and golden paths warm white-gold, only the sunset path red)
    let sg_l = lk_luma(skyu.sun_glint.xyz);
    let k_l = lk_luma(look.key.xyz);
    let sg_warm = select(skyu.sun_glint.xyz, look.key.xyz * (sg_l / max(k_l, 1e-6)), k_l > 1e-6);
    let sg_rgb = mix(skyu.sun_glint.xyz, sg_warm, wg_smooth(0.035, 0.087, skyu.sun_glint.w));
    let key_g = select(look.key.xyz, sg_rgb, skyu.flags.x != 0u);
    // (and toward the sun itself, not the rig's key, which is held up off the horizon)
    let lg = select(l, normalize(look.sun.xyz), skyu.flags.x != 0u && lk_luma(skyu.sun_glint.xyz) > 0.0);
    let ndl_g = max(dot(n, lg), 0.0);
    if (ndl_g > 0.0 && lg.y > -0.05) {
        let h = normalize(lg + v);
        let ndh = max(dot(n, h), 0.0);
        let vdh = max(dot(v, h), 0.0);
        let ndv = max(dot(n, v), 1e-4);
        let dd = ndh * ndh * (alpha2 - 1.0) + 1.0;
        let dggx = alpha2 / (dd * dd * WG_PI);
        let gv = ndl_g * sqrt(ndv * ndv * (1.0 - alpha2) + alpha2);
        let gl = ndv * sqrt(ndl_g * ndl_g * (1.0 - alpha2) + alpha2);
        let vis = 0.5 / max(gv + gl, 1e-5);
        var spec = dggx * vis * wo_fresnel(vdh, WO_IOR) * ndl_g;
        if (wu.screen.z > 0.5) {
            // Resolved crests move the highlight. The GGX lobe already
            // integrates unresolved facets over the pixel footprint.
            spec = min(spec, WO_SPEC_CEILING);
        } else {
            // the share of the glint from facets finer than a pixel lands on a
            // few of them, each a sharp point (same mean): sparkles over dark water,
            // not a smooth sheen.
            let cell = floor(world.xz / max(footprint * 1.2, 0.012));
            let spark = select(0.0, 1.0 / 0.2, w_hash(cell, floor(t * 6.0)) < 0.2);
            // (sea r2: only where a glint is about a pixel; where a pixel holds
            // metres of sea its many glints average to a smooth sheen, not confetti)
            let sparkle = 0.9 * clamp(unresolved * 1.6, 0.0, 1.0) * (1.0 - wg_smooth(0.06, 0.35, footprint));
            // inside the glitter path (where the glint is bright) every
            // pixel is a few facets: about a third of them flash, clipped, and the
            // rest are dark water, at any range (outside the path, where the glint
            // is faint, the smooth sheen stays: no confetti off the path).
            // (by how bright the glint stands against the low sky, not by the raw lobe:
            // under a low sun the sun is so bright that a faint lobe was the whole strip)
            // (r11b: a wide soft envelope; toward its sides the sparkles thin out rather
            // than dim, so the path has no hard edge)
            let in_path = wg_smooth(0.05, 1.5, lk_luma(key_g) * spec / max(lk_luma(look.horizon.xyz), 1e-6));
            // (world cells about three pixels wide, so the temporal resolve keeps them: one-pixel flashes averaged out)
            let gh = w_hash(floor(world.xz / max(footprint * 1.5, 0.02)), floor(t * 8.0) + 3.0);
            let fp = 0.01 + 0.156 * in_path;
            let flash = select(0.0, 1.0 / fp, gh < fp); // (fewer, brighter; r11b: thinning at the envelope's edge)
            // (not under the moon: they read as grey noise on the night water)
            spec = min(spec * mix(mix(1.0, spark, sparkle * (1.0 - skyu.night.z)), flash, wg_smooth(0.0, 0.15, in_path) * (1.0 - skyu.night.z)), WO_SPEC_CEILING);
        }
        // past the shadow cascades, a sun the frame shows hidden behind the
        // skyline (geometry at the sun's own screen point) is hidden from the far water too
        let sc = wu.view_proj * vec4<f32>(lg, 0.0);
        var sun_hidden = 0.0;
        if (sc.w > 1e-4) {
            let su = vec2<f32>(sc.x / sc.w * 0.5 + 0.5, 0.5 - sc.y / sc.w * 0.5);
            if (all(su > vec2<f32>(0.0)) && all(su < vec2<f32>(1.0)) && w_scene_depth(su) < 1e8) {
                sun_hidden = wg_smooth(look.cascade_split.z * 0.8, look.cascade_split.z, view_dist);
            }
        }
        // what stands between this water and the sun in the frame
        // (the skyline at the city's waterline): six steps up the sun's
        // direction (2 m to 400 m), a step hidden behind nearer geometry is in
        // the way (screen-space sun shadow past the cascades).
        if (spec > 0.05 && view_dist > look.cascade_split.y) {
            var occ = 0.0;
            var dd = 2.0;
            for (var k = 0; k < 6; k = k + 1) {
                let q = world + lg * dd;
                let sq = w_to_screen(q);
                if (sq.x > 0.0 && sq.x < 1.0 && sq.y > 0.0 && sq.y < 1.0) {
                    let sdp = w_scene_depth(sq.xy);
                    let qd = w_view_depth(q);
                    if (sdp < qd - 1.0 && sdp > view_dist * 0.7) {
                        occ = 1.0;
                    }
                }
                dd = dd * 2.8;
            }
            sun_hidden = max(sun_hidden, occ);
        }
        // HOOK: the key's envelope is zero at 0 degrees (eo-look rig.rs
        // `key_envelope`), so the glitter path ends at sunset while the clouds are still
        // lit; drive this (and the low-sun body light) from the sky slice's sun radiance
        // through the atmosphere when it lands as a sky uniform.
        glint = key_g * spec * shadow * (1.0 - sun_hidden) * (1.0 - sf_inj.spec_cut) * sf_glint_cut(foam, sf_foam_share(foam, surf_cov, beach, p.x)); // no glint on the surf's foam (water_surf.wgsl); the sea's own foam is a diffuser too, and foam drawn over the strip takes no mirror glint
    }

    // --- the water column: refracted view ray, path to the bottom in the frame.
    let ocean = wg_ocean(world.x);
    let cove = wo_cove(world.xz);
    var sig_a = mix(mix(WO_BAY_A, WO_COAST_A, ocean), WO_COVE_A, cove);
    var sig_s = mix(mix(WO_BAY_S, WO_COAST_S, ocean), WO_COVE_S, cove);
    var bb = mix(mix(WO_BAY_BB, WO_COAST_BB, ocean), 0.045, cove);
    // a beach's surf zone carries churned sand: opaque grey-green to
    // tan, never clear blue (strongest inside the break, gone past 450 m).
    // (sea r3: not in a wave's rising face, which is clearer green-grey water drawn up from outside the break)
    let silt = beach * wg_smooth(450.0, 120.0, signed) * (1.0 - 0.75 * surf_face) * surf_silt(base.x, column); // x surf_silt (water_surf.wgsl)
    sig_a = mix(sig_a, vec3<f32>(0.42, 0.34, 0.55), silt);
    sig_s = mix(sig_s, vec3<f32>(1.5, 1.5, 1.5), silt);
    bb = mix(bb, 0.03, silt);
    let tr = refract(-v, n, 1.0 / WO_IOR);
    let tv = normalize(vec3<f32>(tr.x, min(tr.y, -0.08), tr.z));
    let t_down = max(-tv.y, 0.04);
    let l0 = column / t_down;
    var uv_f = uv_here;
    if (with_copy) {
        let p_end = world + tv * min(l0, 80.0);
        let s_end = w_to_screen(p_end);
        if (s_end.x > 0.0 && s_end.x < 1.0 && s_end.y > 0.0 && s_end.y < 1.0 && w_scene_depth(s_end.xy) - view_dist > 0.05) {
            uv_f = s_end.xy;
        }
    }
    let hit_path = max(w_scene_depth(uv_f) - view_dist, 0.0) * ray_scale;
    let path = clamp(min(l0, hit_path), 0.0, 400.0);
    let p_bottom = world + tv * path;
    let bottom_depth = max(wu.grid.z - p_bottom.y, 0.0);
    // the sun the body scatters entered the surface above the scattering
    // depth, not at this pixel: the shadow averaged over where the refracted
    // sun reaches 1 m and 3 m down the view ray, so a shadow on turbid water
    // has a soft edge, never a ruled one.
    var vol_shadow = shadow;
    let ls_up = wo_refracted_sun_up(l);
    if (look.key_dir.w > 0.0 && ls_up.y > 0.05) {
        let up = vec3<f32>(0.0, 1.0, 0.0);
        let e1 = world + tv * (1.0 / t_down) + ls_up * (1.0 / ls_up.y);
        let e2 = world + tv * (3.0 / t_down) + ls_up * (3.0 / ls_up.y);
        vol_shadow = shadow * 0.34 + (w_shadow(vec3<f32>(e1.x, world.y, e1.z), up) + w_shadow(vec3<f32>(e2.x, world.y, e2.z), up)) * 0.33 * cloud_t;
        // (turbid water carries light sideways under a shadow: the body keeps 40 %
        // of its sunlit glow there, so a bridge's shadow darkens it, not turns it cobalt)
        vol_shadow = max(vol_shadow, 0.4 * cloud_t);
    }
    let sun = look.key.xyz * vol_shadow;
    let sky_irr = look.hemi_sky.xyz / WG_PI;
    var vol = wo_volume(path, tv, l, sun, sky_irr, sig_a, sig_s, bb);
    // the bay is turbid (visibility under a metre): the bottom through
    // it fades within about a metre of water, so Crissy's sand is not seen
    // through the strait.
    vol.tview = vol.tview * exp(-vec3<f32>(2.5 * (1.0 - cove)) * path * (1.0 - wg_ocean(world.x))); // (2.5 / m: a wall under 0.5 m of bay water is barely seen)
    // under an overcast the light the body sends up is a dull grey
    // (the weather's overcast weight desaturates it and takes a third off).
    let ovc = clamp(look.wx.x, 0.0, 1.0);
    vol.inscatter = mix(vol.inscatter, vec3<f32>(lk_luma(vol.inscatter)), 0.6 * ovc) * (1.0 - 0.35 * ovc);
    // a low sun puts little light into the water (it glances off and
    // crosses the turbid bay on a long slant): the body dims toward a third by
    // 3 degrees (the aerial bay read as bronze mud at golden hour)
    vol.inscatter = vol.inscatter * mix(0.35, 1.0, wg_smooth(0.05, 0.45, l.y));
    // Crest translucency: sun through thin wave tips.
    let vh = normalize(v.xz + vec2<f32>(1e-5, 0.0));
    let lh = normalize(l.xz + vec2<f32>(1e-5));
    let back = pow(clamp(dot(vh, -lh) * 0.6 + 0.4, 0.0, 1.0), 2.5);
    let crest = clamp((world.y - wu.grid.z) * 0.9 + 0.1, 0.0, 1.0) * (clamp((1.0 - n.y) * 4.0, 0.0, 1.0) + 0.25);
    // (sea r3: a breaker's thin face near its crest glows bottle-green against the light)
    // (the face's glow held under half the sky's brightness: a single crest pixel flared green)
    let sss = min(sun * vec3<f32>(0.22, 0.42, 0.36) * 0.06 * back * crest * WO_SSS * wg_smooth(0.0, 0.25, l.y) * (1.0 + 3.0 * surf_face) * (1.0 - clamp(look.wx.x, 0.0, 1.0)), vec3<f32>(lk_luma(look.horizon.xyz) * 0.5));

    // --- foam: a diffuse scatterer under the key, the hemisphere, the ambient
    // and the sky's irradiance (the JS lights it with the standard model).
    // The 0.75 ceiling, opaque where the foam is the surf's (tidewater draws it so).
    // thin foam lets the water show through; only thick foam is opaque.
    let foam_w = foam * mix(0.75, 1.0, beach * clamp(surf_cov / max(foam, 1e-3), 0.0, 1.0)) * mix(mix(0.4, 0.75, storm), 1.0, foam);
    // (thin, old foam is greyer than a fresh, thick crest)
    let foam_tone = mix(vec3<f32>(0.80, 0.86, 0.87), vec3<f32>(0.96, 0.98, 0.97), foam) * (relief * 0.2 + 0.9) * mix(0.72, 1.0, foam * foam);
    let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
    // foam is a diffuser: its sky-lit part never brighter than the
    // sky it sees (the sky in this pixel's mirror, a stand-in for the low
    // sky), whatever the ambient terms add up to; the sun's part only where
    // the sun reaches it. Albedo 0.85 fresh, about 0.6 old.
    // (the cap on its brightness only, keeping the ambient's own hue)
    let amb_raw = (hemi + look.ambient.xyz + w_sh(n) * look.ambient.w * look.env_class.w) / WG_PI;
    // (against the brighter of the mirror's sky and the horizon: at a low sun the mirror can be dark and the foam went black)
    // (a white diffuser under the whole sky sits near the sky's own brightness, a little under it)
    // (under an overcast the zenith is brighter than the horizon, so foam lit by the whole dome sits a little over the horizon sky)
    let foam_amb = amb_raw * (mix(0.92, 1.2, clamp(look.wx.x, 0.0, 1.0)) * max(lk_luma(dome), lk_luma(look.horizon.xyz)) / max(lk_luma(amb_raw), 1e-6));
    let foam_rad = sf_foam_light(foam_tone * 0.88 * (look.key.xyz * ndl * shadow * (1.0 - 0.85 * clamp(look.wx.x, 0.0, 1.0)) / WG_PI + foam_amb), foam, sf_foam_share(foam, surf_cov, beach, p.x), n, shadow, relief, v); // the surf's foam lit by the light it receives (water_surf.wgsl)

    // --- the fog (three's range fog on view depth).
    let fog = lk_smooth(look.fog.x, look.fog.y, sky_fog_depth(view_dist, world.y - look.cam_pos.y)); // the city's aerial path density

    var s: Shade;
    if (wu.screen.x > 0.5 && wu.screen.x < 2.5) {
        // EO_WATER_DEBUG=1: r foam, g column (0..2 m), b signed shore distance (0..20 m); =2: r beach, g surf foam, b collar.
        var dbg = vec3<f32>(foam, clamp(column / 2.0, 0.0, 1.0), clamp(signed / 20.0, 0.0, 1.0));
        if (wu.screen.x > 1.5) {
            dbg = vec3<f32>(beach, surf_cov, collar);
        }
        s.own = dbg;
        s.through = vec3<f32>(0.0);
        s.uv = uv_here;
        s.refl_k = 0.0;
        s.sky = vec3<f32>(0.0);
        s.mirror_w = 0.0;
        s.mirror_uv = uv_here;
        s.smear = 0.0;
        s.dash = 0.0;
        s.side = 0.0;
        return s;
    }
    let water = 1.0 - foam_w;
    let k_t = (1.0 - fres) * water;
    s.own = ((vol.inscatter + sss) * k_t + foam_rad * foam_w + glint) * (1.0 - fog) + look.fog_color.xyz * fog;
    // sand under a thin sheet is wet sand (darker: water fills its pores).
    s.through = vol.tview * wo_underwater(bottom_depth, l) * k_t * (1.0 - fog) * (1.0 - 0.4 * film);
    if (wu.screen.z > 0.5 && wu.flags.w > 0.5 && lk_luma(s.through) > 0.015 && l.y > 0.0) {
        let direct = shadow * (1.0 - ovc) * wg_smooth(0.0, 0.2, l.y);
        s.through *= mix(1.0, w_caustic(p_bottom, bottom_depth, ls_up), 0.6 * direct);
    }
    s.uv = uv_f;
    s.refl_k = fres * water * (1.0 - fog);
    // wind patches and slicks read from afar: a slick (its capillaries
    // damped) mirrors more of the sky, a gust patch less and is darker.
    s.refl_k = s.refl_k * (1.0 + 0.3 * det.z - 0.12 * det.y);
    // rain-pocked water scatters the grey sky back from every angle: about
    // twice the mirror's share near the eye (the near water read twice too dark in rain)
    s.refl_k = min(s.refl_k * (1.0 + 0.9 * rain), 0.85);
    // the salt haze over the distant sea: the water fades toward the
    // sky at its own horizon over the kilometres (no hard cut at the sea's
    // edge); kept out where the look's own fog already lies.
    // (thinner with height, as the look's aerial path density: from the air the haze is over the far water only)
    // (the skirt past the grid's radius is horizon colour only: its stretched quads alias)
    let skirt = wg_smooth(wu.fwd.w * 0.98, wu.fwd.w * 1.25, length(world.xz - wu.cam.xz));
    // With the atmosphere tables on, the far sea fades only through the sky volume's aerial
    // perspective (to the sea's own distance) into the same sky drawn above it: the salt haze
    // would fade it a second time toward the raw tables' horizon, drawing a pale seam along the
    // sea horizon. The skirt past the grid stays (its stretched quads alias).
    let salt = select(1.0 - exp(-dist * sky_path_density(world.y - wu.cam.y) / 9000.0), 0.0, skyu.atmo2.x > 0.5);
    let hz_far = max(salt, skirt) * (1.0 - fog);
    if (hz_far > 0.002) {
        let hsky = w_atmo_dome(normalize(vec3<f32>(-v.x, 0.01, -v.z)));
        s.own = mix(s.own, hsky, hz_far);
        s.through = s.through * (1.0 - hz_far);
        s.refl_k = s.refl_k * (1.0 - hz_far);
    }
    s.own = s.own * (1.0 - 0.08 * det.y);
    // (no rim fade into the sky: the skirt carries the sea to
    // the horizon and the salt haze fades it; the fade drew a bright band in
    // front of the far shore from the air)
    // spray and salt haze over a surf zone: thick over a breaking
    // crest and its fresh whitewater, a thin veil over the sea seen past the
    // break (the eye looks through the surf's spray), more in a storm; lit by
    // the sky as foam is (no brighter than the sky in its mirror).
    if (beach > 0.01) {
        let lift = wg_smooth(0.0, 1.6, world.y - wu.grid.z);
        let over = sf_spray_amount(surf_ww, surf_bored * lift, signed, p, wu.cam.w, v.y); // in puffs, none over the last bores, halved at grazing views (water_surf.wgsl)
        let veil = (0.16 + 0.3 * storm) * wg_smooth(60.0, 260.0, signed) * wg_smooth(1200.0 + 2000.0 * storm, 600.0, signed) * surf_veil(signed); // x surf_veil
        let hz = clamp((over + veil) * beach * (1.0 + 1.5 * storm), 0.0, 0.75) * (1.0 - fog);
        let haze_col = sf_spray_colour(min((hemi + look.ambient.xyz) / WG_PI * 0.85, dome * 0.95) + look.key.xyz * shadow * 0.012, over * beach, veil * beach, shadow, v, p.x); // the spray over the crests lit as the foam is (water_surf.wgsl)
        s.own = mix(s.own, haze_col, hz);
        s.through = s.through * (1.0 - hz);
        s.refl_k = s.refl_k * (1.0 - hz);
    }
    s.sky = sky;
    s.mirror_w = 0.0;
    s.mirror_uv = uv_here;
    s.smear = min(sigma_unres * WS_SMEAR_GAIN, WS_SMEAR_MAX);
    s.dash = wg_smooth(-0.02, 0.04, dot(n.xz, normalize(v.xz + vec2<f32>(1e-5, 0.0))));
    s.side = dot(n.xz, normalize(vec2<f32>(-v.z, v.x) + vec2<f32>(1e-5, 0.0)));
    s.sun_near = wg_smooth(0.9, 0.995, dot(rup, normalize(look.sun.xyz))) * wg_smooth(-0.03, 0.02, look.sun.y) * (1.0 - wg_smooth(0.3, 0.6, skyu.night.z));
    if (with_copy && r.y < WS_MARCH_MAX_SIN && fres > WS_MARCH_MIN_F && sf_inj.rough < 0.5) { // no mirror march in the strip's churned water
        let m = w_march(world, r);
        let far = w_far_tap(world, r);
        if (m.z >= far.z) {
            s.mirror_uv = m.xy;
        } else {
            s.mirror_uv = far.xy;
        }
        s.mirror_w = max(m.z, far.z);
    }
    return s;
}

// where this frame's point at `uv` stood in the previous frame.
fn w_prev_uv(uv: vec2<f32>) -> vec2<f32> {
    let px = vec2<i32>(clamp(uv, vec2<f32>(0.0), vec2<f32>(0.99999)) * vec2<f32>(textureDimensions(w_depth)));
    let d = textureLoad(w_depth, px, 0);
    let w = wu.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, max(d, 1e-7), 1.0);
    let c = skyu.prev_vp * vec4<f32>(w.xyz / w.w, 1.0);
    return vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
}

@fragment
fn fs_copy(i: VsOut) -> @location(0) vec4<f32> {
    w_atmo_g = i.atmo_g;
    wv_far_slope = i.wv_far;
    let fpx = max(length(dpdx(i.world.xz)), length(dpdy(i.world.xz)));
    let s = w_shade(i.world, i.base, i.clip, true);
    let bottom = textureSampleLevel(w_scene, w_clamp, s.uv, 0.0).rgb;
    var refl = s.sky;
    if (s.mirror_w > 0.0) {
        // A short vertical smear up the frame (the chop the eye cannot resolve).
        let u = s.mirror_uv;
        let sm = s.smear;
        var tap = textureSampleLevel(w_scene, w_clamp, u, 0.0).rgb * 0.4
            + textureSampleLevel(w_scene, w_clamp, u - vec2<f32>(0.0, sm * 0.5), 0.0).rgb * 0.25
            + textureSampleLevel(w_scene, w_clamp, u - vec2<f32>(0.0, sm), 0.0).rgb * 0.2
            + textureSampleLevel(w_scene, w_clamp, u - vec2<f32>(0.0, sm * 1.5), 0.0).rgb * 0.1
            + textureSampleLevel(w_scene, w_clamp, u - vec2<f32>(0.0, sm * 2.0), 0.0).rgb * 0.05;
        // what the mirror shows, as the frame finally drew it: the
        // same point in the sky slice's resolved previous frame, so the fog and
        // haze that hide a tower hide its reflection too.
        if (skyu.flags.x != 0u && skyu.taa.y > 0.5) {
            let pu = w_prev_uv(u);
            if (all(pu > vec2<f32>(0.0)) && all(pu < vec2<f32>(1.0))) {
                tap = textureSampleLevel(sky_prev, sky_rep, pu, 0.0).rgb * 0.4
                    + textureSampleLevel(sky_prev, sky_rep, pu - vec2<f32>(0.0, sm * 0.5), 0.0).rgb * 0.25
                    + textureSampleLevel(sky_prev, sky_rep, pu - vec2<f32>(0.0, sm), 0.0).rgb * 0.2
                    + textureSampleLevel(sky_prev, sky_rep, pu - vec2<f32>(0.0, sm * 1.5), 0.0).rgb * 0.1
                    + textureSampleLevel(sky_prev, sky_rep, pu - vec2<f32>(0.0, sm * 2.0), 0.0).rgb * 0.05;
            }
        }
        let scene = min(max(tap, vec3<f32>(0.0)), max(s.sky * WS_SCENE_CEIL, vec3<f32>(WS_SCENE_CEIL_FLOOR)));
        let live = wg_smooth(0.0, WS_SCENE_FLOOR, lk_luma(scene));
        let w = s.mirror_w * live * WS_SCENE_TAKE;
        refl = mix(s.sky, scene, w);
    }
    // the sun's glare in the mirror (the drawn sky round the sun, far
    // brighter than the horizon) is seen, like the glint, on a few facets per
    // pixel: flashes over dark water, not a smooth strip.
    // (r11b: gated by the direction to the sun alone, its soft envelope: a brightness
    // threshold cut a hard-edged block where the mirror read the drawn sky's glare)
    let glare = s.sun_near;
    if (glare > 0.0 && wu.screen.z < 0.5) {
        // (fewer, smaller, brighter: one cell in six, 1.5 px, at six times; between
        // them the water keeps the plain low sky, not the sun's glare)
        let gh = w_hash(floor(i.world.xz / max(fpx * 1.5, 0.02)), floor(wu.cam.w * 8.0) + 3.0);
        // (fewer, brighter flashes far off, where a pixel holds more sea: same mean)
        let prob = 0.166 * mix(1.0, 0.4, wg_smooth(300.0, 3000.0, length(i.world - wu.cam.xyz)));
        let base = refl * min(1.0, 0.5 * lk_luma(look.horizon.xyz) / max(lk_luma(refl), 1e-6));
        refl = mix(refl, base + max(refl - base, vec3<f32>(0.0)) * select(0.0, 1.0 / prob, gh < prob), glare);
    }
    // at night a bright light stands in the bay as a long broken
    // column reaching toward the eye. For a pixel of water the flat mirror
    // shows the frame as far above the horizon as the pixel is below it; a
    // facet tilted toward the eye shows what lies between that point and the
    // horizon. Sixteen taps down that span in the last resolved frame (the
    // lamps' heads are drawn after the water), what is brighter than 0.8 at
    // the exposure, weighted by how likely the chop tilts a facet that far
    // (Gaussian, 0.13 rad), on the facets that face the eye.
    if (skyu.night.z > 0.05 && s.dash > 0.0 && skyu.taa.y > 0.5) {
        let px = i.clip.xy / vec2<f32>(textureDimensions(w_depth));
        let hc = wu.view_proj * vec4<f32>(normalize(vec3<f32>(wu.fwd.x, 0.0, wu.fwd.z) + vec3<f32>(1e-5, 0.0, 0.0)), 0.0);
        let yh = 0.5 - hc.y / max(hc.w, 1e-4) * 0.5;
        let ym = max(2.0 * yh - px.y, 0.0);
        var col = vec3<f32>(0.0);
        var twr = vec3<f32>(0.0);
        let ex = look.key.w;
        // (only what stands out of the sky: a lamp is far brighter than the low
        // sky round it; at dusk the bright sky itself drew a phantom column)
        let thr = max(0.8, 3.0 * lk_luma(look.horizon.xyz) * ex);
        // (each facet tilted sideways shifts the column across: wavy, not a ruled bar)
        let xw = px.x + s.side * 0.05;
        for (var k = 0; k < 16; k = k + 1) {
            let fk = (f32(k) + 0.5) / 16.0;
            let yy = mix(ym, yh, fk);
            let tilt = (yy - ym) * 0.45;
            let wk = exp(-tilt * tilt / (2.0 * 0.0169));
            if (wk < 0.02) {
                continue;
            }
            let c = textureSampleLevel(sky_prev, sky_rep, clamp(w_prev_uv(vec2<f32>(xw, yy)), vec2<f32>(0.001), vec2<f32>(0.999)), 0.0).rgb;
            let lum = lk_luma(c) * ex;
            col = col + c * (max(lum - thr, 0.0) / max(lum, 1e-4)) * wk;
            // (lit windows too, dimmer: towers stand in the bay as faint broken columns)
            twr = twr + c * (max(lum - 1.1 * lk_luma(look.horizon.xyz) * ex, 0.0) / max(lum, 1e-4)) * wk;
        }
        // (broken across the column by the chop: a slow along-view wobble)
        // (no per-cell hash: it stamped the columns into a barcode)
        let brk = 0.55 + 0.45 * sin(i.world.z * 0.7 + i.world.x * 0.23 + mx_noise(vec3<f32>(i.world.xz * 0.15, wu.cam.w * 0.3)) * 4.0);
        refl = refl + (col * 0.4 + twr * 0.6) * (s.dash * skyu.night.z * brk);
    }
    // EO_WATER_DEBUG=3: the water's own light only; 4: the reflection only; 5: the bottom only.
    if (wu.screen.x > 2.5) {
        let parts = array<vec3<f32>, 3>(s.own, refl * s.refl_k, bottom * s.through);
        return vec4<f32>(parts[min(u32(wu.screen.x + 0.5) - 3u, 2u)], 1.0);
    }
    return vec4<f32>(s.own + refl * s.refl_k + bottom * s.through, 1.0);
}

@fragment
fn fs_blend(i: VsOut) -> @location(0) vec4<f32> {
    w_atmo_g = i.atmo_g;
    wv_far_slope = i.wv_far;
    let s = w_shade(i.world, i.base, i.clip, false);
    // Premultiplied: own + reflection, the frame behind through one scalar.
    let through = clamp(dot(s.through, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
    return vec4<f32>(s.own + s.sky * s.refl_k, 1.0 - through);
}
