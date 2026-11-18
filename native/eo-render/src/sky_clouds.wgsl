// cloud density and cloud light (appended to the volume module after
// sky_volume.wgsl, so it shares its bindings 0-14; clouds.rs owns the noise,
// the cloud-shadow map and their passes).
//
// Density (after Schneider and Vos 2015, Schneider 2017): the weather map (the
// coverage sheet's cells, their two sizes, fields and streets: sky_common
// `sky_cumulus`) gives each place a local coverage; a tiling 128^3 base-shape
// noise (Perlin-Worley over Worley fBm, 3.2 km tile, distorted by a curl field)
// is remapped by that coverage under a per-type height profile, so a cumulus is
// a flat-based dome whose outline comes from the 3D noise (no 2D outline
// extruded); a 32^3 Worley detail noise (267 m tile) erodes its edges, as wisps
// at the base and billows on the top. The layer is a shell over the curved
// Earth (R 6371 km), so a 900 m deck reaches the true horizon ~107 km out.
//
// Light (Hillaire 2016, Wrenninge et al. 2013): Beer's law along the view, a
// light march toward the sun (six segments of growing length, 30-1100 m, and
// one long step to 2.6 km), four multiple-scattering octaves (extinction x
// 0.5, light x 0.3, eccentricity x 0.5 per octave) over a dual-lobe phase
// (0.78 HG(0.8) + 0.22 HG(-0.3)) with the droplets' diffraction peak and a
// powder term on the first octave; the sun's colour and strength are the
// atmosphere's transmittance at the cloud's own height (zero in the Earth's
// shadow at that height), the same for the cumulus, the decks and the high
// sheet; the ambient is the sky itself: a cloud's top takes the upper sky, its
// sides the sky toward or away from the sun, its base the light from below.

@group(0) @binding(15) var cl_base: texture_3d<f32>;
@group(0) @binding(16) var cl_detail: texture_3d<f32>;
@group(0) @binding(17) var cl_curl: texture_2d<f32>;
@group(0) @binding(18) var cl_shadow: texture_2d<f32>;
@group(0) @binding(19) var cl_blue: texture_2d<f32>;
@group(0) @binding(21) var cl_shadow_out: texture_storage_2d<rgba16float, write>;

const CL_EARTH_R: f32 = 6371000.0;
/// The base-shape noise's tile (m): divides the low deck's drift wrap (3.2 km), so a wrap is seamless.
const CL_BASE_TILE: f32 = 1600.0;
const CL_DETAIL_TILE: f32 = 266.6667;
const CL_CURL_TILE: f32 = 1600.0;
/// Extinction of a cell's core over the deck's nominal extinction (the remapped density peaks ~0.5-0.8).
const CL_DENSITY: f32 = 2.6;
/// The sun's gain on the clouds (the octaves' sum is a single-scattering phase; a thick
/// cloud's multiple scattering returns ~2.5 x what they carry).
const CL_SUN_GAIN: f32 = 2.6;
/// Light-march segment ends along the sun (m): growing segments, then one long step.
const CL_TAPS: array<f32, 7> = array<f32, 7>(30.0, 80.0, 170.0, 340.0, 640.0, 1100.0, 2600.0);

fn cl_remap(v: f32, lo: f32, hi: f32) -> f32 {
    return clamp((v - lo) / max(hi - lo, 1e-4), 0.0, 1.0);
}

/// 0 for a cumulus field, 1 for a continuous deck (overcast, fog, rain, storm).
fn cl_stratus() -> f32 {
    return clamp((skyu.cloud0.z - 0.75) / 0.2, 0.0, 1.0);
}

/// The weather map at a drifted xz: the local coverage 0..1 (how far inside a cell's
/// envelope; the 3D shape carves the outline inside it).
fn cl_weather(wxz: vec2<f32>, lod: f32) -> f32 {
    let cov = skyu.cloud0.z;
    let st = cl_stratus();
    let cu = sky_cumulus(vol_sheet, vol_rep, wxz, cov, lod);
    let thr = 1.0 - cu.y;
    // a broad, soft envelope: the coverage falls off across the whole cell, so the 3D
    // shape (not the sheet's outline) decides where its edge is
    let cum = smoothstep(thr - 0.12, thr + 0.32, cu.x);
    // a deck: everywhere covered, thinner where the sheet is low (its rolls and gaps)
    let deck = mix(0.55, 1.0, smoothstep(thr - 0.25, thr + 0.25, cu.x));
    return mix(cum, deck, st);
}

fn cl_curl_at(wxz: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(cl_curl, vol_rep, wxz / CL_CURL_TILE, 0.0).rg * 2.0 - 1.0;
}

/// The cloud's shape without the detail erosion (the light march's far taps and the
/// cloud-shadow map read this): x density 0..~1, y the height in the cell's own
/// column (0 base .. 1 crown), z the height in the slab, w the local coverage.
/// `p.y` is the height above the curved ground; `lod` the base noise's mip.
fn cl_shape(p: vec3<f32>, lod: f32) -> vec4<f32> {
    let base = skyu.cloud0.x;
    let top = skyu.cloud0.y;
    let h = (p.y - base) / (top - base);
    if (h <= 0.0 || h >= 1.0) {
        return vec4<f32>(0.0);
    }
    let st = cl_stratus();
    let wxz = p.xz + skyu.cloud2.xy;
    // The base shape, its lookup swirled by the curl field (most at the base); a deck's
    // cells at twice the tile (0.8 km rolls).
    let cv = cl_curl_at(wxz);
    let q = vec3<f32>(wxz.x, p.y * mix(1.0, 0.6, st), wxz.y) + vec3<f32>(cv.x, 0.0, cv.y) * mix(160.0, 60.0, h);
    let n = textureSampleLevel(cl_base, vol_rep, q / (CL_BASE_TILE * (1.0 + st)), lod);
    // The weather map read at a point the 3D noise pushes sideways, more with height, so a
    // cell's outline changes from one height to the next (no 2D outline extruded upward).
    let warp = (vec2<f32>(n.g, n.b) - 0.5) * (140.0 + 320.0 * h) * (1.0 - st);
    let wc = cl_weather(wxz + warp, clamp(lod + 1.0, 0.0, 6.0));
    if (wc <= 0.0) {
        return vec4<f32>(0.0, 0.0, h, 0.0);
    }
    // A cell's crown: thin cells stay flat humilis, the dense cores tower (Schneider:
    // the coverage shapes the top); a deck fills the slab under a billowing top (the
    // marine layer from above, a storm deck's upper surface: 0.4-0.8 km billows, up to a
    // quarter of the slab).
    let lowfbm = 0.625 * n.g + 0.25 * n.b + 0.125 * n.a;
    let crown = mix(0.14 + 0.5 * wc * sqrt(wc), 0.75 + 0.25 * smoothstep(0.3, 0.8, 0.6 * n.g + 0.4 * n.b), st);
    let hc = h / crown;
    if (hc >= 1.0) {
        return vec4<f32>(0.0, 1.0, h, wc);
    }
    // Height profile: a flat base at one condensation level (a 5 % ramp), a dome over
    // the top 45 % of the column; a deck with a 10 % base ramp and a rolling top.
    let prof_cu = smoothstep(0.0, 0.05, h) * (1.0 - smoothstep(0.55, 1.0, hc));
    // (a raining deck's base hangs lower in lumps: up to 0.12 of the slab, 0.06 for a dry one)
    let rainy = smoothstep(2.0, 8.0, skyu.wet.z);
    let base_lo = mix(0.06, 0.12, rainy) * smoothstep(0.35, 0.85, n.b);
    let prof_st = smoothstep(base_lo, base_lo + 0.1, h) * (1.0 - smoothstep(0.7, 1.0, hc));
    let prof = mix(prof_cu, prof_st, st);
    let shape = mix(n.r, lowfbm, 0.3);
    // The coverage remap (not a multiply: a cell's core stays solid, its outline is the
    // 3D shape's): coverage over 1 at a cell's centre, 0 at its envelope's edge, shrunk
    // by the height profile (a flat base, a dome narrowing over its top).
    let c = wc * mix(1.2, 1.4, st) * prof * mix(mix(1.0, 0.8, hc * hc), 1.0, st);
    let d_cu = clamp((shape - (1.0 - c)) / 0.35, 0.0, 1.0) * min(c, 1.0);
    // A continuous deck is not carved into cells with gaps between: its density
    // undulates (thicker rolls and thinner places, 0.62-1 of its peak) under the profile;
    // a deck short of full cover still opens where the weather map's sheet is low.
    // (under rain the cells stand out more: 0.4-1)
    let d_st = prof * min(wc * 1.25, 1.0) * (mix(0.62, 0.4, rainy) + mix(0.38, 0.6, rainy) * smoothstep(0.2, 0.8, shape));
    let d = mix(d_cu, d_st, st);
    return vec4<f32>(d, hc, h, wc);
}

/// The full density: the shape eroded by the detail noise within `near` (0..1, faded
/// with distance: the detail is under a pixel far away).
fn cl_density(p: vec3<f32>, t: f32, lod: f32) -> vec4<f32> {
    var s = cl_shape(p, lod);
    if (s.x <= 0.0) {
        return s;
    }
    let near = 1.0 - smoothstep(5000.0, 18000.0, t);
    if (near > 0.0) {
        let st = cl_stratus();
        let wxz = p.xz + skyu.cloud2.xy;
        let cv = cl_curl_at(wxz * 5.0);
        let dq = vec3<f32>(wxz.x, p.y, wxz.y) + vec3<f32>(cv.x, 0.4 * cv.y, cv.y) * 45.0 * (1.0 - 0.6 * s.z);
        let fp = vol_footprint(t);
        let dn = textureSampleLevel(cl_detail, vol_rep, dq / CL_DETAIL_TILE, clamp(log2(max(fp, 1.0) / 8.3), 0.0, 2.0));
        let hf = 0.625 * dn.r + 0.25 * dn.g + 0.125 * dn.b;
        // wispy at the base (the Worley web), billowy on the body and top (its inverse)
        let hfm = mix(hf, 1.0 - hf, clamp(s.y * 5.0, 0.0, 1.0));
        s.x = cl_remap(s.x, hfm * mix(mix(0.42, 0.58, clamp(s.y, 0.0, 1.0)), 0.22, st) * near, 1.0);
    }
    return s;
}

/// Henyey-Greenstein, normalised (1 / 4 pi isotropic).
fn cl_hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * c, 1e-4), 1.5));
}

/// The cloud phase: a strong forward lobe and a back lobe, eccentricity x `k`.
fn cl_phase(c: f32, k: f32) -> f32 {
    return 0.78 * cl_hg(c, 0.8 * k) + 0.22 * cl_hg(c, -0.3 * k);
}

/// The sun scattered at a point under light optical depth `tau` (toward the sun), seen at
/// `c` = cos(view, sun): four octaves (extinction and eccentricity x 0.5, light x 0.3
/// each); the first with the droplets' diffraction peak and the powder term (`tau_near`,
/// the optical depth of the first 80 m toward the sun: a front-lit face's edges and
/// folds darker than its body; faded toward the sun, where the forward lobe rules).
fn cl_ms(tau: f32, tau_near: f32, c: f32) -> f32 {
    let powder = mix(1.0, 1.0 - exp(-2.0 * tau_near), 0.5 * (0.5 - 0.5 * c));
    var e = exp(-tau) * (0.88 * cl_phase(c, 1.0) + 0.12 * cl_hg(c, 0.96)) * powder;
    var a = 0.3;
    var b = 0.5;
    var k = 0.5;
    for (var o = 1; o < 4; o = o + 1) {
        e = e + a * exp(-b * tau) * cl_phase(c, k);
        a = a * 0.3;
        b = b * 0.5;
        k = k * 0.5;
    }
    return e;
}

/// The sun's light reaching a cloud at height `h_m`: the atmosphere's transmittance from
/// space to that height (reddened, zero once the planet hides the sun there) x the sun's
/// scale; the moon's key at night. Every cloud layer takes it.
fn cl_sun(h_m: f32) -> vec3<f32> {
    if (skyu.atmo2.x > 0.5 && skyu.sun_rad.w < 0.5) {
        return vol_atmo_sun_t(h_m) * skyu.atmo2.w;
    }
    return skyu.sun_rad.xyz;
}

/// The direction the clouds are lit from (the sun, or the moon when it keys).
fn cl_light_dir() -> vec3<f32> {
    if (skyu.atmo2.x > 0.5 && skyu.sun_rad.w < 0.5) {
        return normalize(skyu.atmo.xyz);
    }
    return look.key_dir.xyz;
}

/// The light march toward the sun from a cloud point of density `d0`: (optical depth, its
/// first 80 m). Past 6 km the first 80 m are taken at the point's own density (the near
/// taps' detail is under a pixel there) and the march starts at the third tap.
fn cl_light_tau(p: vec3<f32>, l: vec3<f32>, t: f32, lod: f32, d0: f32) -> vec2<f32> {
    let sig = skyu.cloud1.x * skyu.cloud0.w * CL_DENSITY;
    var tau = 0.0;
    var tau_n = 0.0;
    var sp = 0.0;
    var j0 = 0;
    if (t > 6000.0) {
        tau = 80.0 * d0;
        tau_n = tau;
        sp = 80.0;
        j0 = 2;
    }
    for (var j = j0; j < 7; j = j + 1) {
        let sn = CL_TAPS[j];
        let q = p + l * (0.5 * (sp + sn));
        if (q.y > skyu.cloud0.y) {
            break;
        }
        var dj = 0.0;
        if (j < 2) {
            dj = cl_density(q, t, lod).x;
        } else {
            dj = cl_shape(q, min(lod + f32(j - 1) * 0.5, 3.0)).x;
        }
        tau = tau + (sn - sp) * dj;
        sp = sn;
        if (j == 1) {
            tau_n = tau;
        }
    }
    return vec2<f32>(tau, tau_n) * sig;
}

/// A view ray's distance to the height `hh` above the curved ground (the stable root of
/// k t^2 + dy t + (eye_y - hh) = 0, k = |dir.xz|^2 / 2R); `far`: the second crossing
/// (a ray from above leaving through the same height). -1 when it never reaches it.
fn cl_hit(eye_y: f32, dir: vec3<f32>, hh: f32, far: bool) -> f32 {
    let k = dot(dir.xz, dir.xz) / (2.0 * CL_EARTH_R);
    let c = eye_y - hh;
    let disc = dir.y * dir.y - 4.0 * k * c;
    if (disc < 0.0) {
        return -1.0;
    }
    let s = sqrt(disc);
    // the two roots, without cancellation
    let qq = -0.5 * (dir.y + select(-s, s, dir.y >= 0.0));
    var r0 = c / select(qq, -1e-9, abs(qq) < 1e-12);
    var r1 = select(qq / k, 1e12, k < 1e-12);
    let lo = min(r0, r1);
    let hi = max(r0, r1);
    if (far) {
        return select(-1.0, hi, hi > 0.0);
    }
    if (lo > 0.0) {
        return lo;
    }
    return select(-1.0, hi, hi > 0.0);
}

/// The blue-noise start offset of a pixel's march, moved along the golden ratio per trace
/// of the pixel (each pixel is traced every fourth frame): even over the step for one
/// pixel over time, and blue (no pattern, no hatching) across pixels in one frame.
fn cl_jitter(px: vec2<i32>, frame: u32) -> f32 {
    let b = textureLoad(cl_blue, px % vec2<i32>(64), 0).r;
    return fract(b + f32((frame / 4u) % 4096u) * 0.6180340);
}

/// The ambient light on a cloud sample (radiance): the light from below at its base, the
/// side sky toward the viewer's side of the cloud, the upper sky at its top, dimmed
/// inside a dense body. `amb`: (top, side, below) per pixel.
fn cl_ambient(top: vec3<f32>, side: vec3<f32>, below: vec3<f32>, hc: f32, d: f32) -> vec3<f32> {
    let up = smoothstep(0.0, 1.0, hc);
    let a = mix(below, mix(side, top, 0.55 + 0.45 * up), sqrt(up));
    return a * mix(1.0, 0.6, clamp(d, 0.0, 1.0));
}

/// the cloud-shadow map (clouds.rs, before the city pass): for the sea-level point
/// each texel stands for (a 32 km window about the eye, wrapped, so it is anchored to the
/// world), the transmittance of the low deck along the key, through the same shape the
/// march draws. The city, the water and the volume read it (`sky_cloud_transmittance`,
/// `vol_cloud_t`) at the point their own key ray reaches sea level.
@compute @workgroup_size(8, 8)
fn cl_shadow_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let res = i32(SKY_CSHADOW_RES);
    let ij = vec2<i32>(gid.xy);
    if (ij.x >= res || ij.y >= res) {
        return;
    }
    let l = look.key_dir.xyz;
    var tr = 1.0;
    if (skyu.cloud0.z > 0.0 && l.y > 0.02 && cl_stratus() < 0.999) {
        // The window's first texel (world index), and this texel's world index in it.
        let k0 = vec2<i32>(floor((look.cam_pos.xz - 0.5 * SKY_CSHADOW_RES * SKY_CSHADOW_TEXEL) / SKY_CSHADOW_TEXEL));
        let wi = k0 + (((ij - k0) % res) + res) % res;
        let g = (vec2<f32>(wi) + 0.5) * SKY_CSHADOW_TEXEL;
        let base = skyu.cloud0.x;
        let top = skyu.cloud0.y;
        let s0 = max(base - SKY_SEA_Y, 0.0) / l.y;
        let s1 = min(max(top - SKY_SEA_Y, 0.0) / l.y, s0 + 6000.0);
        let n = 10;
        let ds = (s1 - s0) / f32(n);
        let sig = skyu.cloud1.x * skyu.cloud0.w * CL_DENSITY;
        var tau = 0.0;
        // a column whose weather map, blurred over ~1 km about its middle, is empty is skipped
        let mid = vec3<f32>(g.x, SKY_SEA_Y, g.y) + l * (0.5 * (s0 + s1));
        let env = sky_cumulus(vol_sheet, vol_rep, mid.xz + skyu.cloud2.xy, skyu.cloud0.z, 4.5);
        let n_eff = select(n, 0, env.x < 1.0 - env.y - 0.25);
        for (var i = 0; i < n_eff; i = i + 1) {
            let s = s0 + (f32(i) + 0.5) * ds;
            let p = vec3<f32>(g.x, SKY_SEA_Y, g.y) + l * s;
            tau = tau + cl_shape(p, 1.0).x * sig * ds;
        }
        tr = exp(-tau);
    }
    textureStore(cl_shadow_out, ij, vec4<f32>(tr, 0.0, 0.0, 1.0));
}
