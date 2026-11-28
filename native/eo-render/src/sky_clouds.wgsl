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
// 0.5, light x 0.25, eccentricity x 0.5 per octave) over a dual-lobe phase
// (0.72 HG(0.8) + 0.28 HG(-0.3)) with the droplets' diffraction peak and a
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
// `--volumetric-clouds` high-definition cauliflower detail (64^3) and meteorological weather/cirrus map (512^2)
@group(0) @binding(23) var vc_detail: texture_3d<f32>;
@group(0) @binding(24) var vc_weather: texture_2d<f32>;
// `--volumetric-clouds` hemispherical off-screen cloud reflection dome (128^2 RGBA16F, 5 mips, hemi-octahedral)
@group(0) @binding(25) var cl_refl_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(26) var cl_refl_src: texture_2d<f32>;

const CL_EARTH_R: f32 = 6371000.0;
/// The base-shape noise's tile (m): divides the low deck's drift wrap (3.2 km), so a wrap is seamless.
const CL_BASE_TILE: f32 = 1600.0;
const CL_DETAIL_TILE: f32 = 266.6667;
const CL_CURL_TILE: f32 = 1600.0;
/// Extinction of a cell's core over the deck's nominal extinction (the remapped density peaks ~0.5-0.8).
const CL_DENSITY: f32 = 2.6;
/// The sun's gain on the clouds (the octaves' sum is a single-scattering phase; a thick
/// cloud's multiple scattering returns ~2.5 x what they carry).
const CL_SUN_GAIN: f32 = 4.0;
/// Light-march segment ends along the sun (m): growing segments, then one long step.
const CL_TAPS: array<f32, 5> = array<f32, 5>(30.0, 80.0, 240.0, 700.0, 2600.0);

fn cl_remap(v: f32, lo: f32, hi: f32) -> f32 {
    return clamp((v - lo) / max(hi - lo, 1e-4), 0.0, 1.0);
}

/// 0 for a cumulus field, 1 for a continuous deck (overcast, fog, rain, storm).
fn cl_stratus() -> f32 {
    return clamp((skyu.cloud0.z - 0.75) / 0.2, 0.0, 1.0);
}

/// The fair-weather cumulus field is drawn only with `EO_CUMULUS=1` (`SKY_CUMULUS`):
/// without it a clear sky has the high sheet alone, and the low deck draws only as it
/// closes into a continuous deck (coverage over 0.75: the overcast, fog, drizzle, rain
/// and storm decks), its cumulus part faded out.
fn cl_cumulus_on() -> f32 {
    return select(0.0, skyu.prev_eye.w, sky_on(skyu.flags.x, SKY_CUMULUS));
}

/// Whether the low deck draws at all (the march, its cloud shadows).
fn cl_low_deck_on() -> bool {
    return skyu.cloud0.z > 0.0 && (sky_vol_clouds_on(skyu.cirrus) || cl_cumulus_on() > 0.0 || cl_stratus() > 0.0);
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
    // over a wide band of the sheet's rank (~0.3 km): a steep envelope contour stands as a
    // cloud's outline (a box seen from below); the 3D shape draws it instead
    let cum = smoothstep(thr - 0.25, thr + 0.45, cu.x);
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
    // (the sheet is value noise on a square lattice: this push bends its contours, which
    // otherwise stand as straight walls under a flat lid, most in the big cells, whose
    // lattice is 2.6 x the small cells')
    let warp = (vec2<f32>(n.g, n.b) - 0.5) * (260.0 + 440.0 * h) * (1.0 - st);
    // (1.5 mips softer than the sample's own, never finer than mip 2.5, ~70 m: a smooth
    // envelope the 3D shape carves; the sheet's 25-50 m octaves would cut the walls' edges)
    let wc = cl_weather(wxz + warp, clamp(max(lod + 1.5, 2.5), 0.0, 6.0));
    if (wc <= 0.0) {
        return vec4<f32>(0.0, 0.0, h, 0.0);
    }
    // A cell's crown: thin cells stay flat humilis, the dense cores tower (Schneider:
    // the coverage shapes the top); a deck fills the slab under a billowing top (the
    // marine layer from above, a storm deck's upper surface: 0.4-0.8 km billows, up to a
    // quarter of the slab).
    let lowfbm = 0.625 * n.g + 0.25 * n.b + 0.125 * n.a;
    // and the crown rises and falls across the cell with the 3D noise: a cumulus top is a
    // cluster of domes, never one flat lid
    // (broad domes rather than 100 m spires: 0.75 of the 400 m channel, 0.1 of the 100 m one)
    let turret = 0.55 + 0.75 * n.g + 0.1 * n.a;
    let crown = mix((0.15 + 0.55 * wc * sqrt(wc)) * turret, 0.75 + 0.25 * smoothstep(0.3, 0.8, 0.6 * n.g + 0.4 * n.b), st);
    let hc = h / crown;
    if (hc >= 1.0) {
        return vec4<f32>(0.0, 1.0, h, wc);
    }
    // Height profile: a flat base at one condensation level (a 5 % ramp), a dome over
    // the top 45 % of the column; a deck with a 10 % base ramp and a rolling top.
    // (the dome from 0.3 of the column under a crown 1.1 x as high: the top spreads over
    // 0.5-0.75 of the crown with the noise, not one lid)
    let prof_cu = smoothstep(0.0, 0.03, h) * (1.0 - smoothstep(0.3, 1.0, hc));
    // (a raining deck's base hangs lower in lumps: up to 0.12 of the slab, 0.06 for a dry one)
    let rainy = smoothstep(2.0, 8.0, skyu.wet.z);
    let base_lo = mix(0.06, 0.12, rainy) * smoothstep(0.35, 0.85, n.b);
    let prof_st = smoothstep(base_lo, base_lo + 0.1, h) * (1.0 - smoothstep(0.7, 1.0, hc));
    let prof = mix(prof_cu, prof_st, st);
    let shape = mix(n.r, lowfbm, 0.3);
    // The coverage remap (not a multiply: a cell's core stays solid, its outline is the
    // 3D shape's): coverage over 1 at a cell's centre, 0 at its envelope's edge, shrunk
    // by the height profile (a flat base, a dome narrowing over its top).
    // A cumulus is a rounded tower: its coverage swells and shrinks with a 3D noise
    // (bulges of 0.2-0.4 km, so no side is a flat wall), narrows over its top (a dome,
    // 1 - 0.7 hc^2) and draws in a little at its base, under a body that bulges out.
    // (half of it from the 400 m channel: a side swells over ~0.4 km, not only in 200 m bumps)
    let bulge = mix(0.5 + 0.6 * n.b + 0.4 * n.g, 1.0, st);
    // the lowest third of a cumulus is filled out (x 1.3 at the base), so its base is one flat
    // sheet at the condensation level with no lobes hung under it
    let dome = mix((1.0 - 0.7 * hc * hc) * (1.3 - 0.3 * smoothstep(0.0, 0.3, hc)), 1.0, st);
    // (x 1.2 for cumulus, 1.4 for a deck)
    let c = wc * mix(1.2, 1.4, st) * prof * dome * bulge;
    // a crisp surface: the density reaches the core within 0.25 of the shape
    let d_cu = clamp((shape - (1.0 - c)) / 0.25, 0.0, 1.0) * min(c, 1.0);
    // A continuous deck is not carved into cells with gaps between: its density
    // undulates (thicker rolls and thinner places, 0.62-1 of its peak) under the profile;
    // a deck short of full cover still opens where the weather map's sheet is low.
    // (under rain the cells stand out more: 0.4-1)
    let d_st = prof * min(wc * 1.25, 1.0) * (mix(0.62, 0.4, rainy) + mix(0.38, 0.6, rainy) * smoothstep(0.2, 0.8, shape));
    let d = mix(d_cu * cl_cumulus_on(), d_st, st);
    return vec4<f32>(d, hc, h, wc);
}

/// The full density: the shape eroded by the detail noise within `near` (0..1, faded
/// with distance: the detail is under a pixel far away).
fn cl_density(p: vec3<f32>, t: f32, lod: f32) -> vec4<f32> {
    var s = cl_shape(p, lod);
    if (s.x <= 0.0) {
        return s;
    }
    let near = 1.0 - smoothstep(5000.0, 15000.0, t);
    if (near > 0.0) {
        let st = cl_stratus();
        let wxz = p.xz + skyu.cloud2.xy;
        let cv = cl_curl_at(wxz * 5.0);
        let dq = vec3<f32>(wxz.x, p.y, wxz.y) + vec3<f32>(cv.x, 0.4 * cv.y, cv.y) * 45.0 * (1.0 - 0.6 * s.z);
        let fp = vol_footprint(t);
        let dn = textureSampleLevel(cl_detail, vol_rep, dq / CL_DETAIL_TILE, clamp(log2(max(fp, 1.0) / 8.3), 0.0, 2.0));
        // (without the finest octave, 33 m cells: it cuts pinholes and a stipple into thin rims)
        let hf = 0.7 * dn.r + 0.3 * dn.g;
        // wispy at the base (the Worley web), billowy on the body and top (its inverse)
        let hfm = mix(hf, 1.0 - hf, clamp(s.y * 5.0, 0.0, 1.0));
        // half as much on the thin rim (density under 0.3), where the erosion cuts pinholes
        // that the march's per-pixel offset turns into a stipple; the body keeps its billows
        let rim = mix(0.5, 1.0, smoothstep(0.0, 0.3, s.x));
        // (little at the base, which stays flat and crisp; full on the upper body)
        s.x = cl_remap(s.x, hfm * mix(mix(0.15, 0.58, smoothstep(0.0, 0.5, s.y)), 0.22, st) * near * rim, 1.0);
        // billows between the lumps and the fine detail: the base noise again at a 0.8 km
        // tile (Worley cells of ~200 m) eats into the upper body along its cells' borders, so a
        // crown is a cauliflower of domes, not one rounded block
        // (within 8 km and for cumulus only: a deck's samples would read it for nothing)
        let near2 = 1.0 - smoothstep(2500.0, 8000.0, t);
        if (near2 > 0.0 && st < 0.5) {
            let n2 = textureSampleLevel(cl_base, vol_rep, dq / 800.0, clamp(log2(max(fp, 1.0) / 6.25), 0.0, 3.0));
            // (all the way down: the outline a cloud shows from below is its base's, and needs the
            // lobes too; more on the upper body)
            let topw = mix(0.35, 1.0, smoothstep(0.12, 0.7, s.y));
            s.x = cl_remap(s.x, (1.0 - (0.7 * n2.g + 0.3 * n2.b)) * 0.45 * topw * near2 * (1.0 - st), 1.0);
        }
    }
    return s;
}

/// Henyey-Greenstein, normalised (1 / 4 pi isotropic).
fn cl_hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * c, 1e-4), 1.5));
}

/// The cloud phase: a strong forward lobe and a back lobe (0.72 / 0.28), eccentricity x `k`.
fn cl_phase(c: f32, k: f32) -> f32 {
    return 0.72 * cl_hg(c, 0.8 * k) + 0.28 * cl_hg(c, -0.3 * k);
}

/// The sun scattered at a point under light optical depth `tau` (toward the sun), seen at
/// `c` = cos(view, sun): four octaves (extinction and eccentricity x 0.5, light x 0.25
/// each); the first with the droplets' diffraction peak and the powder term (`tau_near`,
/// the optical depth of the first 80 m toward the sun: a front-lit face's edges and
/// folds darker than its body; faded toward the sun, where the forward lobe rules).
fn cl_ms(tau: f32, tau_near: f32, c: f32) -> f32 {
    // (0.15: with a stronger powder term, front-lit faces, the sunward side seen with the
    // sun behind the viewer, come out dull grey-beige)
    let powder = mix(1.0, 1.0 - exp(-2.0 * tau_near), 0.15 * (0.5 - 0.5 * c));
    var e = exp(-tau) * (0.88 * cl_phase(c, 1.0) + 0.12 * cl_hg(c, 0.96)) * powder;
    var a = 0.25;
    var b = 0.5;
    var k = 0.5;
    for (var o = 1; o < 4; o = o + 1) {
        e = e + a * exp(-b * tau) * cl_phase(c, k);
        a = a * 0.25;
        b = b * 0.5;
        k = k * 0.5;
    }
    return e;
}

/// The sun's light reaching a cloud at height `h_m`: the atmosphere's transmittance from
/// space to that height (reddened, zero once the planet hides the sun there) x the sun's
/// scale; the moon's key at night. Every cloud layer takes it.
fn cl_sun(h_m: f32) -> vec3<f32> {
    // `--smooth-time`: `sun_rad.w` is the clouds' handover from the sun to the
    // moon (eo-render sky.rs `CLOUD_MOON_HANDOVER_DEG`: the moon's weight in the
    // key, taken once the Earth's shadow has risen over the decks). While it is
    // between 0 and 1 a cloud keeps the sun's light through the atmosphere (gone
    // once the planet hides the sun at its height) and takes the moon's by the
    // weight, instead of switching at a half. At 0 and 1 the lines below, as without the option.
    let mw = skyu.sun_rad.w;
    if (lk_lx_on(LX_BIT_SMOOTH_TIME) && mw > 0.0 && mw < 1.0) {
        if (skyu.atmo2.x > 0.5) {
            return vol_atmo_sun_t(h_m) * skyu.atmo2.w + skyu.sun_rad.xyz * 0.35 * mw;
        }
        return skyu.sun_rad.xyz * mix(1.0, 0.35, mw);
    }
    if (skyu.atmo2.x > 0.5 && skyu.sun_rad.w < 0.5) {
        return vol_atmo_sun_t(h_m) * skyu.atmo2.w;
    }
    // the moon's key x 0.35: at night a cloud is lit from below by the city, not from
    // above as by a dim sun
    return skyu.sun_rad.xyz * select(1.0, 0.35, skyu.sun_rad.w > 0.5);
}

/// `--smooth-time`: the unit direction `t` of the way from `a` to `b` on their
/// great circle (eo-look `blend::slerp_dir`); nearly opposite ones switch at a half.
fn cl_slerp(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    let c = clamp(dot(a, b), -1.0, 1.0);
    if (c > 0.9995) {
        return normalize(mix(a, b, t));
    }
    let th = acos(c);
    let s = sin(th);
    if (s < 1e-3) {
        return select(a, b, t > 0.5);
    }
    return normalize(a * (sin((1.0 - t) * th) / s) + b * (sin(t * th) / s));
}

/// The direction the clouds are lit from (the sun, or the moon when it keys).
fn cl_light_dir() -> vec3<f32> {
    // `--smooth-time`, mid-handover: turning from the sun's direction to the key's (the moon's) by the weight.
    if (lk_lx_on(LX_BIT_SMOOTH_TIME) && skyu.sun_rad.w > 0.0 && skyu.sun_rad.w < 1.0) {
        if (skyu.atmo2.x > 0.5) {
            return cl_slerp(normalize(skyu.atmo.xyz), normalize(look.key_dir.xyz), skyu.sun_rad.w);
        }
        return look.key_dir.xyz;
    }
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
    // five segments (30, 80, 240, 700, 2600 m), only the first with
    // the full density; past 6 km the first 80 m at the point's own density and three taps,
    // past 20 km two (700, 2600 m)
    var j0 = 0;
    if (t > 6000.0) {
        tau = 80.0 * d0;
        tau_n = tau;
        sp = 80.0;
        j0 = select(2, 3, t > 20000.0);
    }
    for (var j = j0; j < 5; j = j + 1) {
        let sn = CL_TAPS[j];
        let q = p + l * (0.5 * (sp + sn));
        if (q.y > skyu.cloud0.y) {
            break;
        }
        var dj = 0.0;
        if (j == 0) {
            dj = cl_density(q, t, lod).x;
        } else {
            dj = cl_shape(q, min(lod + f32(j) * 0.5, 3.0)).x;
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
    // the light from below only near the base (by a third of the column the sky round it
    // rules): a cloud in another's shadow is lit by the sky, not left a dark smudge
    // at night (the city's glow on) the light from below reaches most of the body
    let a = mix(below, mix(side, top, 0.55 + 0.45 * up), smoothstep(0.0, mix(0.35, 0.9, clamp(skyu.amb_top.w, 0.0, 1.0)), hc));
    // light scattered many times inside a body stays near the light round it: only a
    // little dimmer in a dense core (darker, a cloud in another's shadow is a dark smudge)
    return a * mix(1.0, 0.85, clamp(d, 0.0, 1.0));
}

// ---- `--volumetric-clouds`: dimensional-profile volumetric cumulus, congestus & stratus ----

/// Noise tile sizes (m): every tile divides eo-sky `VOL_WRAP_TILE_M` (51.2 km) evenly for seamless drift.
const VC_WEATHER_TILE: f32 = 12800.0;
const VC_HIGH_TILE: f32 = 25600.0;
const VC_MACRO_TILE: f32 = 51200.0;
const VC_TURRET_TILE: f32 = 3200.0;
const VC_BASE_TILE: f32 = 1600.0;
const VC_DETAIL_TILE: f32 = 640.0;
const VC_FINE_TILE: f32 = 200.0;
const VC_MICRO_TILE: f32 = 80.0;
const VC_CURL_TILE: f32 = 1600.0;
const VC_DENSITY: f32 = 1.62;
const VC_SUN_GAIN: f32 = 1.58;
/// Onshore wind shear direction (WNW -> ESE).
const VC_SHEAR_DIR: vec2<f32> = vec2<f32>(0.92, 0.39);
/// Non-axis-aligned 3D cascade rotations (orthogonal to < 0.2%, mutually separated by 27-41 deg,
/// and exact integer-periodic at `VOL_WRAP_TILE_M` = 51.2 km so every cascade breaks Voronoi
/// alignment and mid-body cell repetition with zero drift seam):
///  * `VC_ROT1` (tile 640 m, N = 80):  wrap_x = (50, 51, -36),   wrap_z = (54, -12, 58)
///  * `VC_ROT2` (tile 200 m, N = 256): wrap_x = (162, -40, -194), wrap_z = (102, -198, 126)
///  * `VC_ROT3` (tile 80 m,  N = 640): wrap_x = (330, 190, -515), wrap_z = (530, -270, 240)
const VC_ROT1: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.625, 0.6375, -0.45),
    vec3<f32>(-0.3940419, 0.7556370, 0.5232053),
    vec3<f32>(0.675, -0.15, 0.725),
);
const VC_ROT2: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.6328125, -0.15625, -0.7578125),
    vec3<f32>(0.6635717, 0.6139092, 0.4275373),
    vec3<f32>(0.3984375, -0.7734375, 0.4921875),
);
const VC_ROT3: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.515625, 0.296875, -0.8046875),
    vec3<f32>(0.2274763, 0.8572046, 0.4620117),
    vec3<f32>(0.828125, -0.421875, 0.375),
);
/// Exact integer-periodic 2.52:1 wind-sheared underside fibrous basis at `VC_FINE_TILE` (200 m, N = 256):
/// stretches along `VC_SHEAR_DIR` (~23.5 deg WNW -> ESE) with gentle vertical shear tilt;
/// wrap_x = (92, 32, -100), wrap_z = (40, -16, 232).
const VC_WISP_SHEAR: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.359375, 0.125, -0.390625),
    vec3<f32>(0.0, 1.15, 0.0),
    vec3<f32>(0.15625, -0.0625, 0.90625),
);
/// 6-tap geometric light-march segment ends along the sun (m) and lateral-bypass weights:
/// 16 m & 48 m resolve fine cauliflower florets and lobe edges; 130..2100 m resolve overlapping
/// 300-900 m cloud heads and cast crisp yet soft volumetric lobe-to-lobe self-shadows.
const VC_TAPS: array<f32, 6> = array<f32, 6>(16.0, 48.0, 130.0, 340.0, 880.0, 2100.0);
const VC_TAP_W: array<f32, 6> = array<f32, 6>(1.0, 1.0, 0.96, 0.88, 0.78, 0.66);

fn vc_stratus() -> f32 {
    return clamp((skyu.cloud0.z - 0.72) / 0.22, 0.0, 1.0);
}

/// Slab ceiling for `--volumetric-clouds` (`eo-sky::clouds::volumetric_decks` already spans 1.1..3.95 km
/// for fair-weather cumulus and 0.4..5.4 km for storms).
fn vc_slab_top() -> f32 {
    return skyu.cloud0.y;
}

/// Cohesive altitude-dependent onshore wind shear (base cumulus lean + upper anvil downwind spread).
fn vc_shear_xz(p_xz: vec2<f32>, h: f32, st: f32) -> vec2<f32> {
    let anvil_sh = pow(max(h - 0.55, 0.0), 1.6) * 520.0 * (1.0 - st);
    return p_xz + skyu.cloud2.xy - VC_SHEAR_DIR * (h * h * mix(260.0, 90.0, st) + anvil_sh);
}

/// Zero-transcendental smooth periodic wave in `-1..1` (pure FMA, replaces `sin`/`cos` in inner skip loop).
fn vc_tri_wave(x: f32) -> f32 {
    let f = abs(fract(x) * 2.0 - 1.0);
    return (f * f * (3.0 - 2.0 * f)) * 2.0 - 1.0;
}

/// Non-tiling world-scale mesoscale + synoptic front modulation (irrational-ratio 24 km, 43 km &
/// 109 km rotated coastal corridors anchored to Bay Area geography; zero `sqrt`/`sin`/`cos`):
/// preserves monumental hero massifs over downtown SF & the Bay (< 11 km) while ramping smoothly
/// across 11-21 km to carve bold asymmetric open blue-sky bays through distant East Bay & Pacific
/// horizon ribbons (capped at 1.15x so fair-weather cumulus never bloats into overcast sheets).
fn vc_synoptic_front(world_xz: vec2<f32>) -> f32 {
    let r2 = dot(world_xz, world_xz);
    if (r2 <= 121000000.0) {
        return 1.0;
    }
    let far_w = smoothstep(121000000.0, 441000000.0, r2);
    let u1 = dot(world_xz, vec2<f32>(0.85, 0.52)) * (1.0 / 24000.0) + 0.34;
    let u2 = dot(world_xz, vec2<f32>(-0.44, 0.90)) * (1.0 / 43000.0) + 0.71;
    let u3 = dot(world_xz, vec2<f32>(0.31, -0.95)) * (1.0 / 109000.0) + 0.22;
    let wave = 0.48 * vc_tri_wave(u1) + 0.32 * vc_tri_wave(u2) + 0.20 * vc_tri_wave(u3);
    let front = mix(0.02, 1.15, smoothstep(-0.44, 0.68, wave));
    return mix(1.0, front, far_w);
}

/// Multi-scale synoptic weather map at drifted world xz (`pad` widens the envelope for conservative empty-space skips):
/// x: primary low cumulus / congestus coverage (0..1),
/// y: broad convective vertical development (0 humilis .. 1 towering congestus / anvil cumulonimbus),
/// z: synoptic air-mass humidity / LCL moisture (0..1),
/// w: broad ~1.4 km regional cluster support (0..1).
fn vc_weather_at(wxz: vec2<f32>, world_xz: vec2<f32>, fp: f32, pad: f32) -> vec4<f32> {
    let lod0 = clamp(log2(max(fp, 1.0) / 25.0), 0.0, 4.0);
    let lod1 = clamp(log2(max(fp, 1.0) / 100.0), 0.0, 4.0);
    let uv0 = wxz / VC_WEATHER_TILE;
    let wm = textureSampleLevel(vc_weather, vol_rep, uv0, lod0);
    // Transposed 51.2 km synoptic modulation breaks large-scale repetition across a 135 km horizon
    // while wrapping with exact period 51.2 km (zero drift pop).
    let macro_uv = vec2<f32>(wxz.y, -wxz.x) / VC_MACRO_TILE + vec2<f32>(0.37, 0.19);
    let ms = textureSampleLevel(vc_weather, vol_rep, macro_uv, lod1);
    let st = vc_stratus();
    let ms_r = clamp(ms.r + select(0.0, 0.055, pad > 0.0), 0.0, 1.0);
    // Broad coastal cloud corridors alternating with open blue-sky vistas while keeping individual
    // cumulus clouds realistically scaled overhead and sculpted across the Bay and horizon.
    let front_raw = vc_synoptic_front(world_xz);
    let front_w = select(front_raw, max(front_raw, 1.0), pad > 0.0);
    let macro_mod = mix(0.26, 1.38, smoothstep(0.10, 0.85, ms_r)) * mix(front_w, 1.0, st);
    let eff_cov = clamp(skyu.cloud0.z * macro_mod, 0.0, 0.94);
    let thr = 1.0 - eff_cov;
    let pad_r = select(0.0, pad * 0.95 + 0.09, pad > 0.0);
    let cum = smoothstep(thr - 0.11 - pad_r, min(thr + 0.42, 1.04), wm.r);
    let deck = mix(0.42, 1.0, smoothstep(0.06, 0.88, 0.52 * wm.r + 0.48 * ms.r));
    let cov = mix(cum, deck, st);
    // Vary regional convective height along the mesoscale front so distant surviving cells rise as
    // asymmetric congestus ranges rather than a uniform-height horizontal belt (strictly conservative for pad > 0).
    let front_h = clamp(mix(1.0, 0.78 + 0.36 * clamp(front_w, 0.0, 1.5), 1.0 - st), 0.72, 1.32);
    let ctype = clamp(mix((0.56 * wm.g + 0.44 * ms.g) * front_h, 0.45 + 0.55 * wm.g, st), 0.0, 1.0);
    // Smooth regional cluster support derived directly from `cov` + 51.2 km synoptic `ms_r` (zero extra fetch).
    let broad_cov = mix(0.55 * cov + 0.45 * ms_r, deck, st);
    return vec4<f32>(cov, ctype, wm.b, broad_cov);
}

/// Fast 2D weather-envelope check (zero 3D fetches): conservative empty-space skip covering
/// primary low cumulus, towering congestus & sheared anvil cumulonimbus.
fn vc_envelope(p: vec3<f32>, fp: f32, pad: f32) -> f32 {
    let base = skyu.cloud0.x;
    let top = vc_slab_top();
    let h = (p.y - base) / max(top - base, 1.0);
    if (h <= 0.0 || h >= 1.0) {
        return 0.0;
    }
    let st = vc_stratus();
    let wxz = vc_shear_xz(p.xz, h, st);
    let fp_env = select(fp, max(fp, 240.0), pad > 0.0);
    let wm = vc_weather_at(wxz, p.xz, fp_env, pad);
    if (wm.x <= 0.001) {
        return 0.0;
    }
    if (pad > 0.0) {
        // Strictly conservative upper-slab empty-space reject: since `tur_smooth <= 1` and
        // `broad_cov <= 1`, `ceil_frac <= 0.54 * wm.y + 0.46`, so rays above `cu_ceil_hi`
        // cannot intersect cumulus even without sampling 3D `cl_base` (saves ~40% empty skips).
        if (st < 0.05) {
            let ceil_hi = min(0.54 * wm.y + 0.48, 1.0);
            let ceil_bound = ceil_hi * (0.18 + 0.82 * ceil_hi);
            let cu_top_hi = min(0.06 + mix(0.13, 0.96, ceil_bound) + 0.06, 0.99);
            if (h > cu_top_hi) {
                return 0.0;
            }
        }
        return wm.x;
    }
    // Physically coherent coastal LCL base (~120 m synoptic drift; reuses `wm.z` with zero extra fetch).
    let base_lift = mix(0.015 + smoothstep(0.20, 0.80, 1.0 - wm.z) * 0.035, 0.03, st);
    let ceil_frac = clamp(0.54 * wm.y + 0.30 + 0.16 * smoothstep(0.14, 0.64, wm.w), 0.0, 1.0);
    let cu_max_top = min(base_lift + mix(0.13, 0.96, ceil_frac), 0.96);
    let max_top = mix(cu_max_top, 0.99, st);
    if (h <= base_lift || h >= max_top) {
        return 0.0;
    }
    let hc = (h - base_lift) / max(max_top - base_lift, 0.06);
    return wm.x * smoothstep(0.0, 0.14, hc) * (1.0 - smoothstep(0.76, 1.0, hc));
}

fn vc_curl_at(wxz: vec2<f32>, fp: f32) -> vec2<f32> {
    let lod = clamp(log2(max(fp, 1.0) / 20.0), 0.0, 4.0);
    return textureSampleLevel(cl_curl, vol_rep, wxz / VC_CURL_TILE, lod).rg * 2.0 - 1.0;
}

/// Physical vapor condensation density curve (shared across `vc_density`, far light taps, and
/// `cl_shadow_main`): outer 50-150 m entrainment margins taper convexly into glowing translucent
/// mist while buoyant inner cauliflower cores solidify cleanly (gentle distance floor past 20 km).
fn vc_vapor_curve(d: f32, dp: f32, hc: f32, billow_h: f32, moist: f32, st: f32, t: f32) -> f32 {
    let far_w = smoothstep(20000.0, 56000.0, t);
    let mist_knee = mix(0.42, 0.22, far_w);
    let mist_floor = mix(0.28, 0.70, far_w);
    let soft_mist = d * smoothstep(0.0, mist_knee, d) * mix(mist_floor, 1.0, smoothstep(0.06, 0.46, dp));
    let solid_core = pow(max(d, 0.0), 1.02);
    let core_blend = smoothstep(0.16, 0.46, d) * smoothstep(0.14, 0.44, dp) * (0.55 + 0.45 * billow_h) * (1.0 - 0.50 * st);
    let shaped = mix(soft_mist, solid_core, core_blend);
    let adiabatic = mix(mix(0.48, 0.34, st), 1.38, smoothstep(0.0, 0.46, hc));
    let moisture = mix(0.82, 1.24, moist);
    return shaped * adiabatic * moisture;
}

/// True 3D Perlin-Worley isosurface cloud shape (`shape_3d >= 1 - c(x, y, z)`):
/// Instead of extruding a 2D weather polygon or heightmap cone, the 3D Perlin-Worley billow volume
/// defines every cloud's silhouette, overhanging flanks, and uneven multi-dome crowns across near,
/// mid, and 70 km distant horizon ranges — eliminating vertical walls, flat roofs, cones & pinholes.
/// Returns: x shape density, y local height fraction hc, z dimensional profile depth dp, w moisture.
fn vc_shape(p: vec3<f32>, fp: f32, lod: f32) -> vec4<f32> {
    let base = skyu.cloud0.x;
    let top = vc_slab_top();
    let h = (p.y - base) / max(top - base, 1.0);
    if (h <= 0.0 || h >= 1.0) {
        return vec4<f32>(0.0);
    }
    let st = vc_stratus();
    let wxz_base = p.xz + skyu.cloud2.xy;
    // Cohesive upper-level wind shear tilts towering cumulus & sheared anvil caps gently downwind.
    let wxz = vc_shear_xz(p.xz, h, st);
    let cv = vc_curl_at(wxz, fp);
    let q = vec3<f32>(wxz.x, p.y * mix(1.38, 0.72, st), wxz.y) + vec3<f32>(cv.x, 0.0, cv.y) * mix(36.0, 18.0, h);
    // Preserve full 3D billow contrast out to 45 km (<= mip 1.75) while smoothly Nyquist-filtering
    // distant 45-120 km horizon samples (up to mip 2.35) to eliminate grazing horizon stipple.
    let base_lod = min(lod, mix(1.75, 2.35, smoothstep(130.0, 280.0, fp)));
    let bn = textureSampleLevel(cl_base, vol_rep, q / (VC_BASE_TILE * (1.0 + 0.55 * st)), base_lod);
    // Cohesive 3D lateral overhang displacement bends weather-map contours organically with height,
    // tapering cleanly near upper crown tips (`h > 0.62`) so lobes never rip off as detached specks.
    let b_dx = clamp((bn.r - 0.643) * 2.8, -1.0, 1.0);
    let b_dz = clamp((bn.g - 0.478) * 3.8, -1.0, 1.0);
    let warp_h = smoothstep(0.08, 0.58, h) * (1.0 - 0.52 * smoothstep(0.62, 0.96, h));
    let warp_3d = vec2<f32>(b_dx, b_dz) * mix(24.0, 88.0, warp_h);
    let warp_cv = cv * mix(14.0, 24.0, h);
    let warp_xz = (warp_3d + warp_cv) * (1.0 - st);
    let wm = vc_weather_at(wxz + warp_xz, p.xz, max(fp, 90.0), 0.0);
    if (wm.x <= 0.001) {
        return vec4<f32>(0.0);
    }

    // Broad smooth 6.4 km synoptic ceiling variation (depends ONLY on smooth regional instability &
    // broad 6.4 km turbulence, NEVER on local `wm.x`, so small puffs never sharpen into cones).
    let tur_xz = wxz_base - VC_SHEAR_DIR * mix(85.0, 30.0, st);
    let tur = textureSampleLevel(cl_base, vol_rep, vec3<f32>(tur_xz.x / VC_TURRET_TILE, 0.148, tur_xz.y / VC_TURRET_TILE), clamp(max(lod, 1.8), 1.8, 2.5));
    let tur_r = clamp((tur.r - 0.34) / 0.56, 0.0, 1.0);
    let tur_g = clamp((tur.g - 0.32) / 0.36, 0.0, 1.0);
    let tur_smooth = smoothstep(0.10, 0.86, 0.64 * tur_r + 0.36 * tur_g);

    // Physically coherent coastal LCL base (gentle ~120 m synoptic drift; reuses `wm.z` for zero extra fetch).
    let lcl_b = wm.z;
    let base_lift = mix(0.015 + smoothstep(0.20, 0.80, 1.0 - lcl_b) * 0.035, 0.03, st);

    let ceil_frac = clamp(0.54 * wm.y + 0.30 * tur_smooth + 0.16 * smoothstep(0.14, 0.64, wm.w), 0.0, 1.0);
    // Match vertical growth to horizontal cluster breadth (`wm.w`) so smaller fair-weather puffs
    // stay naturally wide-based cumulus humilis while broad clusters build into sculpted mediocris
    // and congestus domes without vertical pillar/drumstick aspect distortion.
    let breadth = smoothstep(0.10, 0.68, wm.w);
    let ceil_pow = pow(max(ceil_frac, 1e-5), 1.42);
    let cu_top = min(base_lift + mix(0.14, 0.94, ceil_pow) * mix(0.44, 1.0, breadth), 0.96);
    let st_top = clamp(0.56 + 0.42 * (0.50 * wm.y + 0.50 * tur_smooth), 0.56, 0.98);
    let cloud_top = mix(cu_top, st_top, st);
    let hc = (h - base_lift) / max(cloud_top - base_lift, 0.05);
    if (hc <= 0.0 || hc >= 1.0) {
        return vec4<f32>(0.0, clamp(hc, 0.0, 1.0), 0.0, lcl_b);
    }

    // Full-range normalized 3D Perlin-Worley billow field from `cl_base`:
    // near upper crown tips (`hc > 0.52`), blend smoothly toward coherent low-frequency `pw` so
    // isolated high-frequency Worley cells can never pop into mid-air as detached floating specks.
    let bn_r01 = clamp((bn.r - 0.28) / 0.64, 0.0, 1.0);
    let bn_g01 = clamp((bn.g - 0.26) / 0.44, 0.0, 1.0);
    let bn_b01 = clamp((bn.b - 0.28) / 0.42, 0.0, 1.0);
    let bn_a01 = clamp((bn.a - 0.29) / 0.41, 0.0, 1.0);
    let w_fbm = 0.56 * bn_g01 + 0.30 * bn_b01 + 0.14 * bn_a01;
    let pw = cl_remap(bn_r01, (1.0 - w_fbm) * 0.34, 1.0);
    let tip_coh = smoothstep(0.48, 0.90, hc) * (1.0 - st);
    let body_3d = 0.46 * pw + 0.36 * bn_g01 + 0.18 * bn_b01;
    let crown_3d = 0.72 * pw + 0.22 * bn_g01 + 0.06 * bn_b01;
    let shape_3d = mix(pw, mix(body_3d, crown_3d, tip_coh), 1.0 - st);

    // Continuous buoyancy height profile with subtle tropopause anvil flare strictly on dense
    // towering convective cores (`wm.x > 0.50`, `ceil_frac > 0.70`) so outer rims stay clean.
    let bot_ramp = smoothstep(0.0, mix(0.14, 0.14, st), hc);
    let top_exp = mix(0.84, 1.24, wm.y);
    let anvil_w = smoothstep(0.70, 0.90, ceil_frac) * smoothstep(0.50, 0.82, wm.x) * (1.0 - st);
    let anvil_flare = anvil_w * smoothstep(0.68, 0.86, hc) * (1.0 - smoothstep(0.88, 1.0, hc)) * 0.46;
    let top_taper = max(1.0 - smoothstep(0.16, 1.0, pow(max(hc, 1e-6), top_exp)), anvil_flare);
    let base_fill = 1.24 - 0.22 * smoothstep(0.0, 0.30, hc);
    let cu_prof = bot_ramp * top_taper * base_fill;
    let st_prof = smoothstep(0.0, 0.14, hc) * (1.0 - smoothstep(0.68, 1.0, hc));

    // Connectedness-preserving 3D isosurface remap (`shape_3d >= 1 - c_cu`): 3D billows carve
    // overhanging flanks while discarding sub-5% isolated micro-islands, requiring coherent crown
    // support (`tip_gate`), and sealing thick adiabatic cores (`core_fill`) against interior pinholes.
    let cov_w = smoothstep(0.06, 0.84, wm.x);
    let c_cu = cov_w * mix(1.40, 1.56, wm.y) * cu_prof;
    let tip_gate = smoothstep(mix(0.06, 0.28, tip_coh), mix(0.30, 0.56, tip_coh), c_cu);
    let core_fill = smoothstep(0.62, 1.05, c_cu) * 0.28;
    let d_cu = clamp((shape_3d + core_fill - (1.0 - c_cu)) / 0.44, 0.0, 1.0) * tip_gate;

    let dp_st = clamp(cov_w * 1.32 * st_prof, 0.0, 1.0);
    let rainy = smoothstep(2.0, 8.0, skyu.wet.z);
    let storm_3d = smoothstep(14.0, 22.0, skyu.wet.z);
    // 3D volumetric storm shelf/roll undulation along the lower storm deck (`Kind::Storm` only).
    let shelf_bulge = 1.0 + 0.34 * storm_3d * smoothstep(0.45, 0.08, hc) * (bn_g01 * 2.0 - 1.0);
    let st_billow = 0.60 * pw + 0.40 * w_fbm;
    let d_st = clamp(dp_st * shelf_bulge, 0.0, 1.0) * (mix(0.20, 0.32, rainy) + mix(0.80, 0.68, rainy) * smoothstep(0.14, 0.84, st_billow));

    let d = mix(d_cu, d_st, st);
    let dp = mix(clamp(c_cu * 0.72, 0.0, 1.0), dp_st, st);
    return vec4<f32>(d, hc, dp, lcl_b);
}

/// Full volumetric density with hierarchical 3-scale convective fluid turbulence:
///  * Scale 1 (640 m, `VC_ROT1`): rotated asymmetric cauliflower domes & convective plumes
///  * Scale 2 (200 m, `VC_ROT2` + wind-sheared `VC_WISP_SHEAR` on undersides/fractus): curled florets
///    on upper bodies and sweeping 2.5:1 wind-torn fibrous vapor tendrils on undersides (< 32 km)
///  * Scale 3 (80 m, `VC_ROT3`): smooth close-up 20 m turbulent micro-vapor & underside wisps (< 9.5 km)
/// Every cascade is mutually unaligned (>= 27 deg) and exact integer-periodic at 51.2 km, eliminating
/// repetitive Voronoi balls while keeping overhead cloud undersides crisp, fibrous, and sculpted.
fn vc_density(p: vec3<f32>, t: f32, fp: f32, lod: f32, want_fine: bool, allow_micro: bool) -> vec4<f32> {
    var s = vc_shape(p, fp, lod);
    if (s.x <= 0.001) {
        return s;
    }
    let moist = s.w;
    let st = vc_stratus();
    let fp_full = fp * 0.5;
    let small_fractus = (1.0 - smoothstep(0.12, 0.50, s.z)) * (1.0 - st);
    let billow_h = smoothstep(0.08, 0.34, s.y) * (1.0 - 0.88 * small_fractus);
    // Underside & wind-torn fractus blend (thin outer flanks turn fully fibrous & wind-sheared).
    let under_w = (1.0 - smoothstep(0.08, 0.38, s.y)) * (1.0 - 0.55 * st);
    let wisp_w = clamp(max(under_w, small_fractus * 0.96), 0.0, 1.0);
    var cavity = 0.5 + 0.5 * s.z;
    let det_fade = (1.0 - smoothstep(38000.0, 68000.0, t)) * (1.0 - smoothstep(48.0, 145.0, fp_full));
    if (det_fade > 0.001) {
        let base = skyu.cloud0.x;
        let top = vc_slab_top();
        let h = clamp((p.y - base) / max(top - base, 1.0), 0.0, 1.0);
        // Glaciated fibrous wisps on rare towering anvil cumulonimbus caps (`h > 0.68`, `s.y > 0.72`).
        let anvil_ice = smoothstep(0.66, 0.86, h) * smoothstep(0.72, 0.92, s.y) * (1.0 - st);
        let billow_eff = billow_h * (1.0 - 0.62 * anvil_ice);
        let wisp_eff = clamp(max(wisp_w, anvil_ice * 0.72), 0.0, 1.0);
        let wxz = vc_shear_xz(p.xz, h, st);
        // Subtle 1.0 m/s upward 3D convective buoyancy boil (exact integer cycles at `skyu.eye.w = 86400 s`
        // across all three rotated cascades & `VC_WISP_SHEAR`: zero midnight seam, zero still TAA ghosting).
        let boil_p = skyu.eye.w * (1.0 / 86400.0);
        // Exact 1280 m LOD-filtered curl tile (`51200 / 1280 = 40` integer wraps) with brisk helical
        // altitude twist and restrained amplitude so 2D curl gently swirls clouds without extruding
        // vertical flutes across rising towers.
        let cv = vc_curl_at(wxz * 2.0, fp_full * 2.0);
        let hel_a = p.y * 0.0048;
        let hc_s = sin(hel_a);
        let hc_c = cos(hel_a);
        let cv_h = vec2<f32>(cv.x * hc_c - cv.y * hc_s, cv.x * hc_s + cv.y * hc_c);
        let curl_amp = mix(42.0, 20.0, smoothstep(0.10, 0.85, s.y));
        let dq0 = vec3<f32>(wxz.x, p.y, wxz.y)
            + vec3<f32>(cv_h.x, 0.42 * (cv.x - cv.y), cv_h.y) * curl_amp;

        // Cascade 1 (640 m rotated tile, 64^3 -> 10.0 m/texel, L2-resident): 3D curl-advected billowing plumes.
        let det_lod = clamp(log2(max(fp_full, 1.0) / 10.0), 0.0, 3.0);
        let u1 = VC_ROT1 * (dq0 / VC_DETAIL_TILE) - boil_p * vec3<f32>(-53.0, 102.0, 71.0);
        let dn = textureSampleLevel(vc_detail, vol_rep, u1, det_lod);
        let dome1_w = clamp(0.30 + 0.48 * dn.a, 0.0, 1.0);
        let dome1 = mix(dn.r, dn.g, dome1_w);
        let b1 = smoothstep(0.05, 0.95, 0.72 * (1.0 - dome1) + 0.28 * (1.0 - dn.b));
        let w1_val = 0.72 * dn.b + 0.28 * dn.a;
        let w1 = smoothstep(0.08, 0.88, 1.0 - w1_val);
        let e1 = mix(w1, b1, billow_eff);
        // Protect dense cloud cores from pinholes, thin flanks from Swiss-cheese shredding, and
        // upper crown necks from severing into detached specks while outer rims keep full curl detail.
        let crown_guard = mix(1.0, 0.44, smoothstep(0.54, 0.88, s.y) * (1.0 - smoothstep(0.08, 0.36, s.z)));
        let edge_soft = mix(0.20, 1.0, smoothstep(0.09, 0.38, s.z)) * crown_guard;
        let tip1 = mix(0.36, 1.0, smoothstep(0.02, 0.20, s.x));
        let mask1 = mix(1.0, 0.07, smoothstep(0.16, 0.54, s.x)) * edge_soft * tip1;
        let str1 = mix(mix(0.30, 0.38, billow_eff), 0.22, st) + small_fractus * 0.015;
        s.x = cl_remap(s.x, e1 * str1 * det_fade * mask1, 1.0);
        // Surface cavity anchors undersides to sweeping 640 m fluid curl plumes & sculpted ridges.
        let und_cav1 = 0.56 * w1_val + 0.44 * dome1;
        var dom_cav = mix(und_cav1, 0.76 * dome1 + 0.24 * dn.b, billow_eff);

        // Cascade 2 (200 m rotated tile, 64^3 -> 3.125 m/texel): curled florets + wind-sheared tendrils.
        // Skipped on marine fog / overcast stratus decks (`st >= 0.45`) which have no cumulus turrets.
        if (s.x > 0.001 && want_fine && st < 0.45 && t < 30000.0) {
            let fine_fade = (1.0 - smoothstep(14000.0, 30000.0, t)) * (1.0 - smoothstep(9.0, 36.0, fp_full));
            if (fine_fade > 0.001) {
                // Strong 3D convective swirl curls 40 m florets organically across kilometer-wide undersides.
                let swirl2 = vec3<f32>(
                    (dn.a - 0.5) * 1.18 + (dn.b - 0.5) * 0.52,
                    (dn.b - 0.5) * 0.92 - (dn.g - 0.5) * 0.38,
                    (dn.g - 0.5) * 1.12 - (dn.a - 0.5) * 0.48,
                ) * mix(74.0, 38.0, s.y);
                let dq2 = dq0 + swirl2;
                let fine_lod = clamp(log2(max(fp_full, 1.0) / 3.125), 0.0, 3.0);
                let u2 = VC_ROT2 * (dq2 / VC_FINE_TILE) + vec3<f32>(0.37, 0.19, 0.61) - boil_p * vec3<f32>(287.0, 265.0, 185.0);
                let dn2 = textureSampleLevel(vc_detail, vol_rep, u2, fine_lod);
                let dome2_w = clamp(0.32 + 0.48 * dn2.a, 0.0, 1.0);
                let dome2 = mix(dn2.r, dn2.g, dome2_w);
                let b2 = smoothstep(0.07, 0.93, 0.70 * (1.0 - dome2) + 0.30 * (1.0 - dn2.b));
                var w2_val = 0.68 * dn2.b + 0.32 * dn2.a;
                // Near cloud undersides, anvil caps & wind-torn fractus blend 2.5:1 wind-sheared swirling
                // fibrous tendrils (+1 L2-cached fetch only when `wisp_fade > 0.005`; zero cost on upper bodies).
                let wisp_fade = wisp_eff * (1.0 - smoothstep(10000.0, 17000.0, t));
                var streak_val = w2_val;
                var streak_r = dome2;
                if (wisp_fade > 0.005) {
                    let shear_perp = vec2<f32>(-VC_SHEAR_DIR.y, VC_SHEAR_DIR.x);
                    let wisp_along = (dn.a - 0.5) * 115.0 + (dn.b - 0.5) * 72.0;
                    let wisp_cross = (dn.g - 0.5) * 44.0 - (dn.a - 0.5) * 28.0;
                    let dq_w = dq0 + vec3<f32>(
                        VC_SHEAR_DIR.x * wisp_along + shear_perp.x * wisp_cross,
                        (dn.b - 0.5) * 48.0 + (dn.a - 0.5) * 22.0,
                        VC_SHEAR_DIR.y * wisp_along + shear_perp.y * wisp_cross,
                    );
                    let u_wisp = VC_WISP_SHEAR * (dq_w / VC_FINE_TILE) + vec3<f32>(0.13, 0.47, 0.29) - vec3<f32>(0.0, boil_p * 497.0, 0.0);
                    let wisp_lod = clamp(log2(max(fp_full, 1.0) / 2.8), 0.0, 3.0);
                    let dn_w = textureSampleLevel(vc_detail, vol_rep, u_wisp, wisp_lod);
                    streak_val = 0.68 * dn_w.b + 0.32 * dn_w.a;
                    streak_r = dn_w.r;
                    w2_val = mix(w2_val, streak_val, wisp_fade);
                }
                let w2 = smoothstep(0.06, 0.88, 1.0 - w2_val);
                let e2 = mix(w2, b2, billow_eff);
                // Keep crown/core pinhole protection on isotropic billows while allowing wind-sheared
                // tendrils to actively fray thin outer flanks (`small_fractus`) into translucent filaments.
                let flank_fray = small_fractus * wisp_fade * (1.0 - smoothstep(0.05, 0.44, s.x)) * crown_guard;
                let tip2 = mix(0.18, 1.0, smoothstep(0.05, 0.24, s.x)) * max(edge_soft, flank_fray * 0.80);
                let mask2 = mix(1.0, mix(0.05, 0.16, under_w), smoothstep(0.14, 0.52, s.x)) * tip2;
                let str2 = mix(mix(0.24, 0.28, billow_eff), 0.14, st) + flank_fray * 0.22;
                s.x = cl_remap(s.x, e2 * str2 * fine_fade * mask2, 1.0);
                // Anchor close undersides to anisotropic wind-sheared filaments + crisp horizontal
                // condensation-level stratification ribs (eliminates sculpted isotropic foam look).
                let strat_rib = 0.5 + 0.5 * sin(p.y * 0.032 + dn.b * 4.2 + dn2.a * 2.4);
                let und_cav2 = mix(0.62 * w2_val + 0.38 * dome2, 0.78 * streak_val + 0.22 * streak_r, wisp_fade * 0.76)
                    * mix(1.0, 0.78 + 0.42 * strat_rib, under_w * 0.62);
                let cav2 = mix(und_cav2, 0.72 * dome2 + 0.28 * dn2.b, billow_eff);
                dom_cav = mix(dom_cav, mix(dom_cav, cav2, mix(0.56, 0.52, billow_eff)), fine_fade);
                // Smooth daytime underside detail AO (+20 m upward tap along world +Y): softly sculpts
                // hanging vapor pockets without noisy high-frequency blotches across overhead bellies.
                if (allow_micro && under_w > 0.15 && t < 8000.0) {
                    let dn_up = textureSampleLevel(vc_detail, vol_rep, u2 + vec3<f32>(0.066, 0.061, 0.043), fine_lod);
                    let up_occ = 0.62 * dn_up.r + 0.38 * dn_up.b;
                    let under_ao = clamp(0.50 + (w2_val - up_occ) * 0.92 + (0.50 - up_occ) * 0.38, 0.0, 1.0);
                    let uao_w = under_w * fine_fade * (1.0 - smoothstep(4500.0, 8000.0, t));
                    dom_cav = mix(dom_cav, mix(dom_cav, under_ao, 0.34), uao_w);
                }

                // Cascade 3 (80 m rotated tile, 64^3 -> 1.25 m/texel): native-1080p micro-vapor & silky
                // overhead underside filaments (< 7.2 km), modulating both rims and underside cavity.
                let micro_cap = mix(0.56, 0.94, under_w);
                if (allow_micro && s.x > 0.001 && s.x < micro_cap && t < 7200.0) {
                    let close_fade = (1.0 - smoothstep(3400.0, 7200.0, t)) * (1.0 - smoothstep(2.5, 9.0, fp_full));
                    if (close_fade > 0.001) {
                        let dq3 = dq2 + vec3<f32>(dn2.a - 0.5, dn2.b - 0.5, dn2.g - 0.5) * mix(26.0, 18.0, billow_eff);
                        let micro_lod = clamp(max(log2(max(fp_full, 1.0) / 1.4), 0.35), 0.35, 3.0);
                        let u3 = VC_ROT3 * (dq3 / VC_MICRO_TILE) + vec3<f32>(0.53, 0.29, 0.17) - boil_p * vec3<f32>(246.0, 926.0, 499.0);
                        let dn3 = textureSampleLevel(vc_detail, vol_rep, u3, micro_lod);
                        let b3_top = smoothstep(0.10, 0.88, 0.45 * (1.0 - dn3.r) + 0.25 * (1.0 - dn3.g) + 0.30 * (1.0 - dn3.b));
                        let b3_und = smoothstep(0.10, 0.86, 1.0 - (0.62 * dn3.b + 0.38 * dn3.a));
                        let b3 = mix(b3_und, b3_top, billow_eff);
                        let tip3 = mix(0.18, 1.0, smoothstep(0.06, 0.24, s.x)) * edge_soft;
                        let rim_m3 = 1.0 - smoothstep(0.08, 0.54, s.x);
                        let mask3 = mix(rim_m3, max(rim_m3, 0.20), under_w) * tip3;
                        let str3 = mix(mix(0.22, 0.26, billow_eff), 0.11, st);
                        s.x = cl_remap(s.x, b3 * str3 * close_fade * mask3, 1.0);
                        let cav3_und = 0.52 * dn3.b + 0.26 * dn3.a + 0.22 * dn3.r;
                        let cav3 = mix(cav3_und, 0.46 * dn3.r + 0.26 * dn3.g + 0.28 * dn3.b, billow_eff);
                        let cav3_w = close_fade * max(mask3, under_w * 0.58);
                        dom_cav = mix(dom_cav, mix(dom_cav, cav3, mix(0.30, 0.36, billow_eff)), cav3_w);
                    }
                }
            }
        }
        // Rich internal cavity contrast sculpts cauliflower folds & wind-torn striated undersides.
        let cav_curve = mix(smoothstep(0.09, 0.91, dom_cav), smoothstep(0.05, 0.95, dom_cav), billow_eff);
        cavity = mix(0.5 + 0.5 * s.z, cav_curve, det_fade);
    }
    s.x = vc_vapor_curve(s.x, s.z, s.y, billow_h, moist, st, t);
    s.w = cavity;
    return s;
}

/// Physically forward-peaked Mie C1 droplet phase: crisp luminous silver-lining diffraction halo +
/// strong forward Mie scatter through translucent outer vapor margins + authentic back-scatter glory.
/// `spike_w` softens the narrow `g = 0.90` silver spike at extreme horizon grazing distances & night.
fn vc_droplet_phase(c: f32, spike_w: f32) -> f32 {
    let spike = cl_hg(c, 0.91);
    let silver = spike / (1.0 + 0.27 * spike);
    let sw = 0.34 * spike_w;
    return sw * silver + (0.85 - sw) * cl_hg(c, 0.60) + 0.15 * cl_hg(c, -0.25);
}

fn vc_phase(c: f32, k: f32) -> f32 {
    return 0.82 * cl_hg(c, 0.64 * k) + 0.18 * cl_hg(c, -0.18 * k);
}

/// 6-octave energy-conserving physical multi-scattering with crisp floret relief, deep concave
/// crevice trapping, free top-down volumetric subsurface scatter (`top_sun`), incandescent
/// silver-gold backlit diffraction rims, and cinematic "fire vs. indigo" golden-hour separation:
///  * Sunlit billowing heads stay luminous warm-white (day) / blazing molten gold-amber (sunset).
///  * Deep recessed creases & overhangs trap light so shadows plunge into cerulean (day) or
///    bruised twilight indigo-violet (sunset) instead of uniform flat peach-brown.
fn vc_ms(tau: f32, tau_near: f32, sun_facing: f32, cavity: f32, top_sun: f32, spike_w: f32, c: f32, low_sun: f32, strat: f32) -> vec3<f32> {
    let cu_w = 1.0 - 0.82 * strat;
    let front_w = (0.62 - 0.40 * c) * cu_w;
    let sf = clamp((sun_facing - 0.24) * 1.75, -1.0, 1.0);
    let inc0 = max(0.14, 1.0 + 1.02 * sf * front_w);
    let inc1 = max(0.28, 1.0 + 0.68 * sf * front_w);
    let inc2 = max(0.48, 1.0 + 0.38 * sf * front_w);
    let inc3 = max(0.70, 1.0 + 0.16 * sf * front_w);
    // Crisp concave cauliflower V-crevice light trapping (deeper internal folds kill soft "clay" look).
    let cav0 = mix(1.0, 0.22 + 1.02 * cavity, cu_w);
    let cav1 = mix(1.0, 0.38 + 0.78 * cavity, cu_w);
    let cav2 = mix(1.0, 0.58 + 0.50 * cavity, cu_w);
    // Real Beer-powder relief on front/side-lit domes (fades out toward sun so thin backlit edges blaze).
    let powder = mix(1.0, 1.0 - exp(-2.6 * tau_near), 0.30 * (0.58 - 0.42 * c) * cu_w);

    // Single-scattered diffraction lobe + two-tier incandescent molten-gold Mie rim corona at sunset:
    // razor-hot white-gold grazing lip (`exp(-0.82*tau)`) + glowing amber translucent rim (`exp(-0.38*tau)`)
    // strictly on backlit margins facing the low sun while thick shadow cores stay dark steel-indigo.
    let rim_fwd = smoothstep(0.45, 0.98, c) * spike_w * low_sun * cu_w;
    let rim_hot = smoothstep(0.72, 0.995, c) * spike_w * low_sun * cu_w;
    let rim_thin = exp(-0.82 * tau) * (0.42 + 0.58 * cavity);
    let rim_lip = exp(-0.38 * tau) * smoothstep(0.20, 0.85, cavity);
    let rim_blaze = vec3<f32>(1.45, 1.12, 0.46) * (rim_fwd * rim_thin * 0.58)
        + vec3<f32>(1.55, 1.18, 0.52) * (rim_hot * rim_lip * 0.26);
    var e = vec3<f32>(exp(-tau) * vc_droplet_phase(c, spike_w) * powder * inc0 * cav0) + rim_blaze;
    // True golden-hour spectral cascade: high R & G with low B produces molten amber-gold
    // fire across sunlit rims & multi-scattered flanks (strictly zero salmon-pink cast).
    let warm_tint = mix(vec3<f32>(1.00, 1.00, 1.01), vec3<f32>(1.12, 0.96, 0.60), low_sun);
    let gold_boost = 1.0 + 0.62 * low_sun;
    var a = 0.42 * gold_boost;
    var b = 0.45;
    var k = 0.56;
    var tint = vec3<f32>(1.0);
    for (var o = 1; o < 6; o = o + 1) {
        tint = tint * warm_tint;
        let inc_o = select(select(inc3, inc2, o == 2), inc1, o == 1);
        let cav_o = select(cav2, cav1, o == 1);
        e = e + tint * (a * exp(-b * tau) * vc_phase(c, k) * inc_o * cav_o);
        a = a * 0.54;
        b = b * 0.40;
        k = k * 0.50;
    }
    // Deep isotropic multi-bounce + free top-down volumetric subsurface escape (`top_sun`):
    // restrained & cooled to deep twilight steel-indigo inside thick sunset shadow cores while
    // sunlit rims and exposed ridges blaze molten amber-gold.
    let deep_tint = mix(vec3<f32>(0.91, 0.96, 1.07), vec3<f32>(0.72, 0.84, 1.18), low_sun);
    let deep_a = mix(0.118, 0.026, low_sun);
    let deep_b = mix(0.072, 0.012, low_sun);
    let deep_sc = (deep_a * exp(-0.044 * tau) + deep_b * exp(-0.012 * tau)) * gold_boost / 12.566371;
    let sub_tint = mix(vec3<f32>(0.97, 0.99, 1.04), vec3<f32>(1.18, 0.95, 0.54), low_sun);
    let sub_sc = top_sun * mix(0.115, 0.048, low_sun) * (0.24 + 0.76 * cavity) * cu_w * gold_boost / 12.566371;
    e = e + deep_tint * (deep_sc * cav2) + sub_tint * sub_sc;
    return e;
}

/// Artifact-free bilinear read of `cl_shadow` `(tr, tau_20, tau_45, tau_72)` for cloud-to-cloud
/// shadow projection: 99.98% of lookups take a single hardware bilinear fetch; the 12 texel quads
/// touching the 5 reserved `ClSky` metadata texels `(0..4, 0)` fall back to a masked 4-tap
/// bilinear that substitutes adjacent row `y = 1` so `ClSky` radiance can never bleed.
fn vc_cshadow_sample(uv: vec2<f32>) -> vec4<f32> {
    let res_i = i32(SKY_CSHADOW_RES);
    let tc = fract(uv) * SKY_CSHADOW_RES - 0.5;
    let fl = floor(tc);
    let i0 = vec2<i32>(fl);
    let x0 = ((i0.x % res_i) + res_i) % res_i;
    let y0 = ((i0.y % res_i) + res_i) % res_i;
    let x1 = (x0 + 1) % res_i;
    let y1 = (y0 + 1) % res_i;
    if (!((y0 == 0 || y1 == 0) && (x0 < 5 || x1 < 5))) {
        return textureSampleLevel(cl_shadow, vol_rep, uv, 0.0);
    }
    let f = tc - fl;
    let y00 = select(y0, 1, y0 == 0 && x0 < 5);
    let y10 = select(y0, 1, y0 == 0 && x1 < 5);
    let y01 = select(y1, 1, y1 == 0 && x0 < 5);
    let y11 = select(y1, 1, y1 == 0 && x1 < 5);
    let s00 = textureLoad(cl_shadow, vec2<i32>(x0, y00), 0);
    let s10 = textureLoad(cl_shadow, vec2<i32>(x1, y10), 0);
    let s01 = textureLoad(cl_shadow, vec2<i32>(x0, y01), 0);
    let s11 = textureLoad(cl_shadow, vec2<i32>(x1, y11), 0);
    return mix(mix(s00, s10, f.x), mix(s01, s11, f.x), f.y);
}

/// Long-range cloud-to-cloud optical depth along sun ray `l` BEYOND local march endpoint `s_done`
/// (exact partition of `[0, s_done] + [s_done, slab_top]`: zero double-counting):
///  * Inside the 32 km `cl_shadow` window (`l.y > 0.08`, aligned with `look.key_dir`): single-fetch
///    piecewise-linear lookup over the 5 strata `(h = 0.00, 0.20, 0.45, 0.72, 1.00)` from the
///    16-step Cascade-1 detail-carved shadow map.
///  * Outside the 32 km window or at grazing sunset (`l.y <= 0.08`): smooth handoff to a 3-tap
///    wide-cone feathered `vc_envelope` march (`2.6 km, 5.0 km, 8.8 km`).
fn vc_cloud_cast_tau(p: vec3<f32>, l: vec3<f32>, s_done: f32, fp: f32, sig: f32) -> f32 {
    let base = skyu.cloud0.x;
    let top = vc_slab_top();
    let p_far = p + l * s_done;
    let h_far = (p_far.y - base) / max(top - base, 1.0);
    if (h_far >= 0.995) {
        return 0.0;
    }
    let span = SKY_CSHADOW_RES * SKY_CSHADOW_TEXEL;
    let g = p.xz - l.xz * (p.y - SKY_SEA_Y) / max(l.y, 0.02);
    let aligned = select(0.0, 1.0, dot(l, look.key_dir.xyz) > 0.995 && vc_stratus() < 0.999);
    let map_w = aligned * (1.0 - smoothstep(0.40, 0.47, length(g - look.cam_pos.xz) / span)) * smoothstep(0.08, 0.16, l.y);
    var tau_map = 0.0;
    if (map_w > 0.001) {
        let s = vc_cshadow_sample(g / span);
        let t20 = max(s.g, 0.0);
        let t45 = max(s.b, 0.0);
        let t72 = max(s.a, 0.0);
        if (h_far >= 0.72) {
            tau_map = mix(t72, 0.0, clamp((h_far - 0.72) / 0.28, 0.0, 1.0));
        } else if (h_far >= 0.45) {
            tau_map = mix(t45, t72, (h_far - 0.45) / 0.27);
        } else if (h_far >= 0.20) {
            tau_map = mix(t20, t45, (h_far - 0.20) / 0.25);
        } else {
            let t00 = max(min(-log(max(s.r, 1e-4)) / 1.95, 10.0), t20);
            tau_map = mix(t00, t20, clamp(h_far / 0.20, 0.0, 1.0));
        }
    }
    var tau_fb = 0.0;
    if (map_w < 0.999) {
        // Grazing golden-hour cloud-to-cloud shadow cascade (`l.y <= 0.14`: 5 stratified 2D taps out
        // to 11.4 km); high-sun out-of-window distant fallback uses 3 lean taps (zero extra day cost).
        if (l.y <= 0.14) {
            var far_d = array<f32, 5>(
                max(s_done + 700.0, 1300.0),
                max(s_done + 1800.0, 2700.0),
                max(s_done + 3400.0, 4700.0),
                max(s_done + 5800.0, 7500.0),
                max(s_done + 9200.0, 11400.0),
            );
            var far_w = array<f32, 5>(260.0, 320.0, 380.0, 420.0, 440.0);
            for (var k = 0; k < 5; k = k + 1) {
                let qf = p + l * far_d[k];
                if (qf.y >= top) {
                    break;
                }
                let ef = vc_envelope(qf, fp + far_d[k] * 0.065, 0.0);
                tau_fb = tau_fb + far_w[k] * smoothstep(0.12, 0.86, ef);
            }
        } else {
            var far_d3 = array<f32, 3>(max(s_done + 900.0, 2600.0), max(s_done + 3100.0, 5200.0), max(s_done + 6800.0, 9200.0));
            var far_w3 = array<f32, 3>(360.0, 480.0, 560.0);
            for (var k = 0; k < 3; k = k + 1) {
                let qf = p + l * far_d3[k];
                if (qf.y >= top) {
                    break;
                }
                let ef = vc_envelope(qf, fp + far_d3[k] * 0.075, 0.0);
                tau_fb = tau_fb + far_w3[k] * smoothstep(0.14, 0.86, ef);
            }
        }
        tau_fb = min(tau_fb * sig, 12.0);
    }
    return mix(tau_fb, tau_map, map_w);
}

/// True 3D upward/hemispherical sky ambient occlusion + free top-down volumetric subsurface
/// escape (2 cheap `vc_shape` taps, zero 3D detail):
///  * x (`ao_sky`): detects real overhanging cloud turrets and deep recessed pockets above `p`.
///  * y (`top_sun`): indirect solar photons entering the sunlit top and scattering downward/sideways.
fn vc_sky_ao(p: vec3<f32>, l: vec3<f32>, hc: f32, t: f32, fp: f32, lod: f32, sig: f32, st: f32) -> vec2<f32> {
    let top_base = max(l.y, 0.10) * (0.35 + 0.65 * smoothstep(0.0, 0.85, hc));
    if (t > 13000.0 || st > 0.45) {
        return vec2<f32>(1.0, 0.55 * top_base);
    }
    let slab_top = vc_slab_top();
    var tau_up = 0.0;
    let q2_w = 1.0 - smoothstep(4500.0, 6800.0, t);
    let q1 = p + vec3<f32>(0.0, 150.0, 0.0);
    if (q1.y < slab_top && vc_envelope(q1, fp + 60.0, 0.12) > 0.001) {
        let s1 = vc_shape(q1, fp + 60.0, min(lod + 0.6, 2.3));
        tau_up = tau_up + mix(250.0, 140.0, q2_w) * vc_vapor_curve(s1.x, s1.z, s1.y, smoothstep(0.08, 0.34, s1.y), s1.w, st, t);
    }
    if (q2_w > 0.005) {
        let q2 = p + vec3<f32>(-l.x * 130.0, 380.0, -l.z * 130.0);
        if (q2.y < slab_top && vc_envelope(q2, fp + 140.0, 0.14) > 0.001) {
            let s2 = vc_shape(q2, fp + 140.0, min(lod + 1.2, 2.3));
            tau_up = tau_up + (260.0 * q2_w) * vc_vapor_curve(s2.x, s2.z, s2.y, smoothstep(0.08, 0.34, s2.y), s2.w, st, t);
        }
    }
    let fade = 1.0 - smoothstep(7500.0, 13000.0, t);
    let ao_sky = mix(1.0, clamp(exp(-tau_up * sig * 0.58), 0.22, 1.0), fade);
    let top_sun = mix(0.55, exp(-tau_up * sig * 0.24), fade) * top_base;
    return vec2<f32>(ao_sky, top_sun);
}

/// Deterministic 6-tap diffusion-cone light march toward the sun + layered cloud-to-cloud shadow.
/// Scales smoothly out to the 128 km horizon (zero sudden lighting cutoff at grazing sunset).
/// Returns: x optical depth `tau`, y near optical depth `tau_near` (first 48 m), z zero-extra-fetch
/// 2-scale directional sun-incidence slope (`sun_facing` in -1..1, neutral 0.24 past 24 km).
fn vc_light_tau(p: vec3<f32>, l: vec3<f32>, t: f32, fp: f32, lod: f32, d0: f32, jit: f32) -> vec3<f32> {
    let _unused_jit = jit;
    let sig = skyu.cloud1.x * skyu.cloud0.w * VC_DENSITY;
    let slab_top = vc_slab_top();
    let st = vc_stratus();
    var tau = 0.0;
    var tau_n = 0.0;
    var sp = 0.0;
    var d_s0 = d0;
    var d_s1 = d0;
    var j0 = 0;
    var j_end = 6;
    if (t > 54000.0) {
        tau = 130.0 * d0;
        tau_n = 48.0 * d0;
        sp = 130.0;
        j0 = 3;
        j_end = 5;
    } else if (t > 32000.0) {
        tau = 48.0 * d0;
        tau_n = tau;
        sp = 48.0;
        j0 = 2;
        j_end = 5;
    } else if (t > 14000.0) {
        tau = 16.0 * d0;
        tau_n = tau;
        sp = 16.0;
        j0 = 1;
    }
    var exited = false;
    for (var j = j0; j < j_end; j = j + 1) {
        let sn = VC_TAPS[j];
        let sm = 0.5 * (sp + sn);
        let q = p + l * sm;
        if (q.y > slab_top) {
            if (j == 0) {
                d_s0 = 0.0;
            }
            if (j <= 1) {
                d_s1 = 0.0;
            }
            exited = true;
            break;
        }
        var dj = 0.0;
        if (j == 0 && t < 13000.0) {
            // Crisp cauliflower floret self-shadow & micro-slope at 8 m toward the sun (Cascade 1+2 near).
            dj = vc_density(q, t, fp, lod, t < 8000.0, false).x;
        } else if (j == 1 && t < 11000.0) {
            // Secondary 32 m cauliflower dome self-shadow & dome-slope (Cascade 1 only).
            dj = vc_density(q, t, fp * 1.25, min(lod + 0.45, 2.3), false, false).x;
        } else {
            // Progressive diffusion-cone widening on far taps (j >= 2) resolving 130..2100 m overlapping domes.
            let cone_fp = fp + sm * 0.16;
            let cone_lod = min(lod + f32(j) * 0.55, 2.3);
            if (vc_envelope(q, cone_fp, 0.16) > 0.001) {
                let s_q = vc_shape(q, cone_fp, cone_lod);
                if (s_q.x > 0.001) {
                    let bh_q = smoothstep(0.08, 0.34, s_q.y);
                    dj = vc_vapor_curve(s_q.x, s_q.z, s_q.y, bh_q, s_q.w, st, t);
                }
            }
        }
        if (j == 0) {
            d_s0 = dj;
        } else if (j == 1) {
            d_s1 = dj;
        }
        tau = tau + (sn - sp) * dj * VC_TAP_W[j];
        sp = sn;
        if (j == 1) {
            tau_n = tau;
        }
    }
    var tau_c2c = 0.0;
    if (!exited && t < 86000.0 && l.y > 0.02 && st < 0.85) {
        let c2c_fade = 1.0 - smoothstep(56000.0, 86000.0, t);
        tau_c2c = vc_cloud_cast_tau(p, l, sp, fp, sig) * c2c_fade;
    }
    let raw_drop = 0.45 * (d0 - d_s0) + 1.15 * (d0 - d_s1);
    let slope_raw = clamp(raw_drop / (0.58 + 0.42 * max(d0, d_s1)), -1.0, 1.0);
    let slope_fade = 1.0 - smoothstep(12000.0, 24000.0, t);
    let sun_facing = mix(0.24, slope_raw, slope_fade);
    return vec3<f32>(tau * sig + tau_c2c, tau_n * sig, sun_facing);
}

/// Physically grounded 3D sky ambient with real hemispherical occlusion (`ao_3d`), Voronoi crevice
/// shading, directional golden-hour horizon under-bounce (`sun_under`), lateral cloud-to-cloud
/// bounce (`c2c_bounce`) onto exposed shaded cheeks, clean nocturnal city-glow localization (`night_w`),
/// and cerulean (day) / indigo-violet (sunset) shadow color separation.
fn vc_ambient(top_col: vec3<f32>, side_col: vec3<f32>, bot_col: vec3<f32>, sun_under: vec3<f32>, c2c_bounce: vec3<f32>, hc: f32, dp: f32, d: f32, cavity: f32, sun_facing: f32, ao_3d: f32, low_sun: f32, night_w: f32, strat: f32) -> vec3<f32> {
    let cu_w = 1.0 - 0.78 * strat;
    // Deepen optical core self-occlusion under thick daytime & golden-hour overhangs so recessed
    // sunset bellies plunge into near-silhouette twilight steel-indigo beside blazing gold rims.
    let core_dark = mix(1.0, mix(mix(0.66, 0.33, low_sun), 0.84, night_w), clamp(dp, 0.0, 1.0) * (1.0 - hc));
    let h_vis = clamp(mix(mix(0.68, 0.30, low_sun), 1.06, smoothstep(0.0, 0.82, hc)) * core_dark, 0.18, 1.06);
    let ao_floor = mix(0.48, 0.15, low_sun);
    let ao_eff = mix(ao_floor, 0.62, night_w);
    let sky_vis = h_vis * mix(1.0, mix(ao_eff, 1.08, ao_3d), cu_w);
    let cav_lo = mix(mix(0.40, 0.18, low_sun), 0.72, night_w);
    let cav_hi = mix(mix(1.24, 1.42, low_sun), 1.12, night_w);
    let cav_shade = mix(1.0, mix(cav_lo, cav_hi, cavity), cu_w);
    let sf_n = clamp((sun_facing - 0.24) * 1.65, -1.0, 1.0);
    let dir_amb = mix(1.0, 1.0 + 0.18 * sf_n, cu_w);
    let up = smoothstep(0.0, 1.0, hc);
    // Deep recessed pockets shift cooler toward cerulean (day) or clean twilight steel-indigo (sunset; R < G < B).
    let deep_cool = mix(vec3<f32>(0.84, 0.93, 1.13), vec3<f32>(0.58, 0.76, 1.20), low_sun);
    let cool_sh = mix(deep_cool, vec3<f32>(1.0), clamp(sky_vis, 0.0, 1.0));
    let upper = mix(side_col, top_col, 0.30 + 0.70 * up) * sky_vis * cool_sh;
    // Keep city skyglow smoothly anchored to undersides over the lit city while sunset recessed
    // shadow bellies stay genuinely dark indigo-slate rather than uniformly washed by warm `bot_col`.
    let bot_reach = mix(0.46, 0.44, night_w);
    let bot_w = 1.0 - smoothstep(0.0, bot_reach, hc);
    let bot_mix = bot_w * mix(mix(mix(0.70, 0.38, low_sun), 0.20, clamp(sky_vis, 0.0, 1.0)), 0.74, night_w);
    let a = mix(upper, bot_col * mix(0.90, 1.10, 1.0 - 0.45 * dp) * mix(vec3<f32>(1.0), cool_sh, low_sun * 0.75), bot_mix);
    // Directional molten amber-gold sunset under-bounce selectively catches PROTRUDING underside
    // ridges & wispy fractures while leaving deep recessed bellies dark coastal twilight slate.
    let ridge_exp = smoothstep(0.44, 0.92, cavity) * smoothstep(0.24, 0.84, ao_3d);
    let under_lit = sun_under * (1.0 - smoothstep(0.0, 0.56, hc)) * (0.08 + 0.92 * ridge_exp);
    // Warm lateral cloud-to-cloud bounce from adjacent sunlit lobes onto exposed shaded flanks & outer rims
    // (deep recessed overhangs with low `ao_3d` / low `cavity` stay cool cerulean / indigo-violet).
    let side_exp = smoothstep(0.26, 0.86, ao_3d) * smoothstep(0.08, 0.52, hc) * (1.0 - 0.50 * smoothstep(0.74, 1.0, hc)) * (0.24 + 0.76 * cavity);
    let c2c_lit = c2c_bounce * side_exp * cu_w;
    return (a * cav_shade * dir_amb + under_lit + c2c_lit) * mix(1.0, 0.90, clamp(d, 0.0, 1.0));
}

/// the cloud-shadow map (clouds.rs, before the city pass): for the sea-level point
/// each texel stands for (a 32 km window about the eye, wrapped, so it is anchored to the
/// world), the transmittance of the low deck along the key, through the same shape the
/// march draws. The city, the water and the volume read it (`sky_cloud_transmittance`,
/// `vol_cloud_t`) at the point their own key ray reaches sea level.
/// Under `--volumetric-clouds`, `.r` carries Cascade-1 detail-carved ground/water
/// transmittance while `.gba` (on all texels outside `(0..4, 0)`) pack cumulative physical
/// optical depth above slab heights `h = 0.20, 0.45, 0.72` for single-fetch cloud-to-cloud
/// shadow projection (`vc_cloud_cast_tau`).
@compute @workgroup_size(8, 8)
fn cl_shadow_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let res = i32(SKY_CSHADOW_RES);
    let ij = vec2<i32>(gid.xy);
    if (ij.x >= res || ij.y >= res) {
        return;
    }
    let l = look.key_dir.xyz;
    var tr = 1.0;
    var gba = vec3<f32>(0.0);
    let vc_on = sky_vol_clouds_on(skyu.cirrus);
    if (vc_on) {
        let st = vc_stratus();
        if (skyu.cloud0.z > 0.0 && l.y > 0.02 && st < 0.85) {
            let k0 = vec2<i32>(floor((look.cam_pos.xz - 0.5 * SKY_CSHADOW_RES * SKY_CSHADOW_TEXEL) / SKY_CSHADOW_TEXEL));
            let wi = k0 + (((ij - k0) % res) + res) % res;
            let g = (vec2<f32>(wi) + 0.5) * SKY_CSHADOW_TEXEL;
            let base = skyu.cloud0.x;
            let top = vc_slab_top();
            let inv_slab = 1.0 / max(top - base, 1.0);
            let s0 = max(base - SKY_SEA_Y, 0.0) / l.y;
            let s1 = min(max(top - SKY_SEA_Y, 0.0) / l.y, s0 + 14000.0);
            let n = 16;
            let ds = (s1 - s0) / f32(n);
            let dh = max(l.y * ds * inv_slab, 1e-4);
            let sig_base = skyu.cloud1.x * skyu.cloud0.w * VC_DENSITY;
            var tau_00 = 0.0;
            var tau_20 = 0.0;
            var tau_45 = 0.0;
            var tau_72 = 0.0;
            let sun_hi = smoothstep(0.12, 0.55, l.y);
            let lod = mix(2.6, 1.05, sun_hi);
            let det_lod = mix(2.4, 1.3, sun_hi);
            let fp = mix(210.0, 88.0, sun_hi);
            // March TOP-DOWN along the sun ray so cumulative optical depth above each altitude
            // stratum (h = 0.72, 0.45, 0.20, 0.00) is computed in one pass with exact early-out.
            // Per-step 2D weather-envelope early skip keeps empty air at zero 3D fetches; non-empty
            // hits apply rotated Cascade-1 cauliflower carving (`vc_detail`, `VC_ROT1`) matching
            // `vc_density` for organic ragged penumbras on SF streets, hills, Bay water & clouds.
            for (var i = n - 1; i >= 0; i = i - 1) {
                let s = s0 + (f32(i) + 0.5) * ds;
                let p = vec3<f32>(g.x, SKY_SEA_Y, g.y) + l * s;
                if (vc_envelope(p, fp, 0.08) <= 0.001) {
                    continue;
                }
                let sh = vc_shape(p, fp, lod);
                if (sh.x > 0.002) {
                    var d = sh.x;
                    let moist = sh.w;
                    let small_fractus = (1.0 - smoothstep(0.14, 0.48, sh.z)) * (1.0 - st);
                    let billow_h = smoothstep(0.08, 0.34, sh.y) * (1.0 - 0.65 * small_fractus);
                    let h = clamp((p.y - base) * inv_slab, 0.0, 1.0);
                    // Cascade 1 (640 m rotated domes/wisps) carves outer lobes & penumbras; solid
                    // opaque cores (d >= 0.80) have minimal erosion so skipping saves 3D fetches.
                    if (d < 0.80) {
                        let wxz = vc_shear_xz(p.xz, h, st);
                        let boil_p = skyu.eye.w * (1.0 / 86400.0);
                        let cv = vc_curl_at(wxz * 2.0, fp * 2.0);
                        let hel_a = p.y * 0.0048;
                        let hc_s = sin(hel_a);
                        let hc_c = cos(hel_a);
                        let cv_h = vec2<f32>(cv.x * hc_c - cv.y * hc_s, cv.x * hc_s + cv.y * hc_c);
                        let curl_amp = mix(42.0, 20.0, smoothstep(0.10, 0.85, sh.y));
                        let dq0 = vec3<f32>(wxz.x, p.y, wxz.y)
                            + vec3<f32>(cv_h.x, 0.42 * (cv.x - cv.y), cv_h.y) * curl_amp;
                        let dn = textureSampleLevel(vc_detail, vol_rep, VC_ROT1 * (dq0 / VC_DETAIL_TILE) - boil_p * vec3<f32>(-53.0, 102.0, 71.0), det_lod);
                        let dome1 = mix(dn.r, dn.g, clamp(0.30 + 0.48 * dn.a, 0.0, 1.0));
                        let b1 = smoothstep(0.05, 0.95, 0.72 * (1.0 - dome1) + 0.28 * (1.0 - dn.b));
                        let w1 = smoothstep(0.08, 0.88, 1.0 - (0.72 * dn.b + 0.28 * dn.a));
                        let e1 = mix(w1, b1, billow_h);
                        let edge_soft = mix(0.24, 1.0, smoothstep(0.08, 0.36, sh.z));
                        let tip1 = mix(0.38, 1.0, smoothstep(0.02, 0.18, d));
                        let mask1 = mix(1.0, 0.08, smoothstep(0.16, 0.54, d)) * edge_soft * tip1;
                        let str1 = mix(mix(0.30, 0.38, billow_h), 0.22, st) + small_fractus * 0.02;
                        d = cl_remap(d, e1 * str1 * mask1, 1.0);
                    }
                    if (d > 0.001) {
                        d = vc_vapor_curve(d, sh.z, sh.y, billow_h, moist, st, 0.0);
                        let dtau = d * sig_base * ds;
                        let h_hi = h + 0.5 * dh;
                        tau_00 = tau_00 + dtau;
                        tau_20 = tau_20 + dtau * clamp((h_hi - 0.20) / dh, 0.0, 1.0);
                        tau_45 = tau_45 + dtau * clamp((h_hi - 0.45) / dh, 0.0, 1.0);
                        tau_72 = tau_72 + dtau * clamp((h_hi - 0.72) / dh, 0.0, 1.0);
                        if (tau_00 > 10.0) {
                            break;
                        }
                    }
                }
            }
            // Ground/water transmittance (.r) uses 1.95x optical boost (matched to balanced vapor extinction)
            // so fair-weather cumulus cast crisp readable shadows; (.gba) store unboosted physical tau.
            tr = exp(-tau_00 * 1.95);
            gba = vec3<f32>(min(tau_20, 10.0), min(tau_45, 10.0), min(tau_72, 10.0));
        } else if (skyu.cloud0.z > 0.0 && l.y > 0.02 && st >= 0.85) {
            tr = skyu.cloud1.w;
        }
    } else if (cl_low_deck_on() && l.y > 0.02 && cl_stratus() < 0.999) {
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
            // the shape read coarser as the sun drops (at ~9 deg the weather map 4 mips
            // coarser, the noise at its coarsest): a low sun's samples lie ~0.6 km apart along a
            // long path, and read fine they alias into one blotch per small cell; coarse, the
            // long shadows merge into the cloud fields' coherent patches
            tau = tau + cl_shape(p, mix(5.0, 1.0, smoothstep(0.15, 0.5, l.y))).x * sig * ds;
        }
        tr = exp(-tau);
    }
    // the sky's light on the clouds, once a frame: texels (0..4, 0) carry it in
    // their g, b, a (amb, zen, top, sun, anti; sky_volume `cl_sky_light`); the receivers read r
    // alone. The volume pass reads these five texels instead of the sky table per pixel.
    if (ij.y == 0 && ij.x < 5) {
        let sk = cl_sky_light();
        var parts = array<vec3<f32>, 5>(sk.amb, sk.zen, sk.top, sk.sun, sk.anti);
        gba = parts[ij.x];
    }
    textureStore(cl_shadow_out, ij, vec4<f32>(tr, gba));
}

/// Seamless hemi-octahedral decode (`uv` in `[0, 1]^2` -> unit upper-hemisphere direction, `d.y > 0`).
/// Matches `water.wgsl::w_hemi_oct_encode` with zero azimuthal wrap seam.
fn vc_hemi_oct_decode(uv: vec2<f32>) -> vec3<f32> {
    let o = uv * 2.0 - 1.0;
    let px = 0.5 * (o.x + o.y);
    let pz = 0.5 * (o.x - o.y);
    let py = max(1.0 - abs(px) - abs(pz), 0.004);
    return normalize(vec3<f32>(px, py, pz));
}

/// (`--volumetric-clouds`) Fast barrier-free time-sliced `128 x 128` RGBA16F hemispherical cloud
/// reflection dome (dispatched at `32 x 32` = 1,024 threads): reads precomputed `ClSky` from
/// `cl_shadow`, fills 4x4 blocks on cut frames and refines in 4x4 Bayer order (effective 64^2 in
/// 4 frames, full 128^2 in 16 frames) at ~0.03 ms steady-state GPU cost.
@compute @workgroup_size(8, 8)
fn cl_refl_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dim = vec2<i32>(textureDimensions(cl_refl_out));
    let base_ij = vec2<i32>(gid.xy) * 4;
    if (base_ij.x >= dim.x || base_ij.y >= dim.y) {
        return;
    }
    let fill_all = skyu.taa.y < 0.5;
    // 4x4 Bayer traversal: every 4 frames visits all four 2x2 quadrants evenly.
    var offs = array<vec2<i32>, 16>(
        vec2<i32>(0, 0), vec2<i32>(2, 2), vec2<i32>(2, 0), vec2<i32>(0, 2),
        vec2<i32>(1, 1), vec2<i32>(3, 3), vec2<i32>(3, 1), vec2<i32>(1, 3),
        vec2<i32>(1, 0), vec2<i32>(3, 2), vec2<i32>(3, 0), vec2<i32>(1, 2),
        vec2<i32>(0, 1), vec2<i32>(2, 3), vec2<i32>(2, 1), vec2<i32>(0, 3),
    );
    let off = select(offs[skyu.flags.y % 16u], vec2<i32>(0, 0), fill_all);
    let ij = min(base_ij + off, dim - 1);
    let bits = skyu.flags.x;
    if (!sky_vol_clouds_on(skyu.cirrus) || !sky_on(bits, SKY_CLOUDS) || skyu.cloud0.z <= 0.0) {
        if (fill_all) {
            for (var y = 0; y < 4; y = y + 1) {
                for (var x = 0; x < 4; x = x + 1) {
                    textureStore(cl_refl_out, min(base_ij + vec2<i32>(x, y), dim - 1), vec4<f32>(0.0));
                }
            }
        } else {
            textureStore(cl_refl_out, ij, vec4<f32>(0.0));
        }
        return;
    }

    let uv = (vec2<f32>(ij) + select(0.5, 2.0, fill_all)) / vec2<f32>(dim);
    let dir = vc_hemi_oct_decode(uv);
    let base = skyu.cloud0.x;
    let top = vc_slab_top();
    let strat = vc_stratus();
    let atmo_on = skyu.atmo2.x > 0.5;
    let cl_l = cl_light_dir();
    let cl_cs = dot(dir, cl_l);
    let heavy = smoothstep(0.03, 0.18, skyu.cloud1.w);
    let vc_beam = heavy * pow(clamp(skyu.cloud1.w, 0.0, 1.0), 0.35);
    let night_w = smoothstep(0.08, 0.52, skyu.night.z);
    let day_w = smoothstep(0.02, 0.25, skyu.atmo.y) * (1.0 - strat * 0.78);
    let low_sun = smoothstep(0.42, 0.04, skyu.atmo.y) * smoothstep(-0.08, 0.02, skyu.atmo.y);
    let day_or_gold = clamp(day_w + 0.74 * low_sun * (1.0 - strat * 0.78), 0.0, 1.0);
    let day_hi = smoothstep(0.22, 0.58, cl_l.y) * (1.0 - strat * 0.82);
    let back_w = smoothstep(-0.25, 0.85, cl_cs);

    // Anchor reflection origin near sea level so overhead clouds line up accurately across water reflections.
    let eye = vec3<f32>(look.cam_pos.x, select(SKY_SEA_Y + 4.0, look.cam_pos.y, look.cam_pos.y > base), look.cam_pos.z);

    var a_top = skyu.amb_top.xyz;
    var a_side = skyu.amb_top.xyz;
    var a_bot = skyu.amb_base.xyz;
    var sk_amb = a_top;
    var sk_sun = a_top;
    if (atmo_on) {
        let sk_amb0 = textureLoad(cl_shadow, vec2<i32>(0, 0), 0).gba;
        let sk_zen0 = textureLoad(cl_shadow, vec2<i32>(1, 0), 0).gba;
        let sk_top0 = textureLoad(cl_shadow, vec2<i32>(2, 0), 0).gba;
        let sk_sun0 = textureLoad(cl_shadow, vec2<i32>(3, 0), 0).gba;
        let sk_anti0 = textureLoad(cl_shadow, vec2<i32>(4, 0), 0).gba;
        a_top = mix(a_top, sk_top0, skyu.atmo2.y);
        a_top = mix(a_top, sk_zen0, 0.35 * smoothstep(0.02, -0.06, skyu.atmo.y));
        let lh = normalize(vec2<f32>(cl_l.x, cl_l.z) + vec2<f32>(1e-5, 0.0));
        let vh = normalize(dir.xz + vec2<f32>(1e-5, 0.0));
        a_side = mix(a_side, mix(sk_sun0, sk_anti0, 0.5 + 0.5 * dot(vh, lh)), skyu.atmo2.y);
        a_bot = mix(a_bot, skyu.amb_base.xyz * 0.4 + sk_amb0 * 0.5, skyu.atmo2.y);
        sk_amb = mix(sk_amb, sk_amb0, skyu.atmo2.y);
        sk_sun = mix(sk_sun, sk_sun0, skyu.atmo2.y);
    }

    let wrap_w = (0.22 + 0.24 * back_w) * day_or_gold;
    let top_sky = mix(a_top, sk_amb, 0.34 * day_or_gold);
    let side_sky = mix(a_side, mix(sk_amb, sk_sun, 0.36), wrap_w);
    let top_l = dot(top_sky, vec3<f32>(0.2126, 0.7152, 0.0722));
    let side_l = dot(side_sky, vec3<f32>(0.2126, 0.7152, 0.0722));
    let a_bot_eff = mix(a_bot, a_top * vec3<f32>(0.76, 0.84, 1.04), night_w);
    let bot_l = dot(a_bot_eff, vec3<f32>(0.2126, 0.7152, 0.0722));

    let sun_cloud_bounce = cl_sun(0.5 * (base + top)) * ((mix(0.011, 0.022, day_hi) * max(cl_l.y, 0.0) + 0.008 * low_sun) * vc_beam * (1.0 - strat * 0.85) * mix(0.62, 1.15, back_w));
    let sun_ground_bounce = cl_sun(base) * ((mix(0.014, 0.032, day_hi) * max(cl_l.y, 0.0) + 0.007 * low_sun) * vc_beam * (1.0 - strat * 0.82));
    let sun_under = cl_sun(base) * (0.064 * low_sun * vc_beam * (1.0 - strat * 0.85) * (0.28 + 0.72 * max(cl_cs, 0.0)) * vec3<f32>(1.28, 0.94, 0.38));
    let c2c_bounce = cl_sun(0.5 * (base + top)) * ((mix(0.020, 0.034, day_hi) * max(cl_l.y, 0.12) + 0.018 * low_sun) * vc_beam * (1.0 - strat * 0.85)) * mix(vec3<f32>(1.02, 0.99, 0.95), vec3<f32>(1.18, 0.96, 0.56), low_sun);

    let desat_w = mix(0.18, 0.04, low_sun) * day_or_gold;
    let top_gain = mix(mix(1.0, 0.92, night_w), mix(mix(1.02, 1.18, day_hi), 1.08, back_w), day_or_gold);
    let side_gain = mix(mix(1.0, 0.88, night_w), mix(mix(0.94, 1.14, day_hi), 0.96, back_w), day_or_gold);
    let bot_gain = mix(mix(1.0, 0.82, night_w), mix(mix(0.96, 1.24, day_hi), 0.82, back_w), day_or_gold);
    let sunset_cool = mix(vec3<f32>(1.0), vec3<f32>(0.76, 0.85, 1.16), low_sun * (1.0 - strat * 0.8));
    let night_cool = mix(vec3<f32>(1.0), vec3<f32>(0.80, 0.88, 1.16), night_w * (1.0 - strat * 0.5));
    let night_dome_fill = SKY_GLOW_RGB * (skyu.amb_top.w * night_w) * vec3<f32>(0.50, 0.58, 0.84);
    let vc_top = mix(top_sky, vec3<f32>(top_l) * vec3<f32>(0.96, 0.98, 1.04), desat_w) * top_gain * sunset_cool * night_cool + night_dome_fill * 0.85 + sun_cloud_bounce * 0.78 + skyu.twi.xyz * (0.10 * low_sun);
    let vc_side = mix(side_sky, vec3<f32>(side_l) * vec3<f32>(0.96, 0.98, 1.04), desat_w) * side_gain * sunset_cool * night_cool + night_dome_fill * 1.15 + sun_cloud_bounce * 0.88 + skyu.twi.xyz * (0.14 * low_sun);
    let vc_bot = mix(a_bot_eff, vec3<f32>(max(bot_l, side_l * 0.84)) * vec3<f32>(0.96, 0.98, 1.04), desat_w * 1.1) * bot_gain * mix(vec3<f32>(1.0), sunset_cool, 0.65) + night_dome_fill * 0.95 + sun_ground_bounce + skyu.twi.xyz * (0.24 * low_sun);

    var L_out = vec3<f32>(0.0);
    var T_out = 1.0;

    // (1) Primary volumetric low cloud slab: 14-step `vc_shape` + 2-tap sun march with full
    // `vc_ms` physical multi-scattering so reflected clouds match main-sky cumulus luminance & sunset fire.
    var t0 = cl_hit(eye.y, dir, base, false);
    var t1 = cl_hit(eye.y, dir, top, false);
    if (eye.y >= base && eye.y < top) {
        t0 = 0.0;
    }
    let refl_far = mix(92000.0, 38000.0, night_w);
    if (t0 >= 0.0 && t0 < refl_far - 4000.0) {
        if (t1 <= t0) {
            t1 = t0 + 22000.0;
        }
        t1 = min(min(t1, t0 + 30000.0), refl_far);
        let hl = length(dir.xz);
        let sig_max = skyu.cloud1.x * skyu.cloud0.w * VC_DENSITY * mix(1.0, 0.64, night_w * (1.0 - strat * 0.7));
        let moon_key_w = clamp(skyu.sun_rad.w, 0.0, 1.0) * night_w;
        let moon_tint = mix(vec3<f32>(1.0), vec3<f32>(1.95, 2.15, 2.65), moon_key_w);
        let twi_spike_soft = 1.0 - 0.72 * night_w;
        var Tc = 1.0;
        var Lc = vec3<f32>(0.0);
        var tw = 0.0;
        var twt = 0.0;
        let dt_span = (t1 - t0) / 14.0;
        var t = t0 + 0.5 * dt_span;
        for (var i = 0; i < 14; i = i + 1) {
            if (t >= t1 || Tc < 0.05) {
                break;
            }
            let pp = eye + dir * t;
            let ht = t * hl;
            let pc = vec3<f32>(pp.x, pp.y + ht * ht / (2.0 * CL_EARTH_R), pp.z);
            let fp = max(t * 0.040, 32.0);
            if (vc_envelope(pc, fp, 0.14) > 0.001) {
                let lod = clamp(log2(fp / 20.0), 0.4, 2.5);
                let sh = vc_shape(pc, fp, lod);
                if (sh.x > 0.0025) {
                    let bh = smoothstep(0.08, 0.34, sh.y);
                    let d = vc_vapor_curve(sh.x, sh.z, sh.y, bh, sh.w, strat, t);
                    let sig = sig_max * d;
                    let alt_grad = mix(vec3<f32>(1.26, 0.90, 0.38), vec3<f32>(1.12, 1.05, 0.80), smoothstep(0.12, 0.78, sh.y));
                    let sun_alt_tint = mix(vec3<f32>(1.0), alt_grad, low_sun * (1.0 - strat * 0.85));
                    let e_sun = cl_sun(pc.y) * vc_beam * sun_alt_tint * moon_tint;
                    var sun = vec3<f32>(0.0);
                    var sun_slope = 0.28;
                    if (any(e_sun > vec3<f32>(0.0))) {
                        var tau_n = 55.0 * d;
                        var tau_l = tau_n;
                        var d_near = d;
                        let q1 = pc + cl_l * 160.0;
                        if (q1.y < top && vc_envelope(q1, fp + 32.0, 0.12) > 0.001) {
                            let sq1 = vc_shape(q1, fp + 32.0, min(lod + 0.45, 2.3));
                            if (sq1.x > 0.002) {
                                d_near = vc_vapor_curve(sq1.x, sq1.z, sq1.y, smoothstep(0.08, 0.34, sq1.y), sq1.w, strat, t);
                                tau_l = tau_l + 190.0 * d_near;
                                tau_n = tau_n + 65.0 * d_near;
                            } else {
                                d_near = 0.0;
                            }
                        } else {
                            d_near = 0.0;
                        }
                        let q2 = pc + cl_l * 520.0;
                        if (q2.y < top && vc_envelope(q2, fp + 95.0, 0.14) > 0.001) {
                            let sq2 = vc_shape(q2, fp + 95.0, min(lod + 0.95, 2.4));
                            if (sq2.x > 0.002) {
                                tau_l = tau_l + 460.0 * vc_vapor_curve(sq2.x, sq2.z, sq2.y, smoothstep(0.08, 0.34, sq2.y), sq2.w, strat, t);
                            }
                        }
                        sun_slope = mix(0.28, clamp((d - d_near) * 1.35 + 0.18, -0.65, 0.90), 1.0 - strat);
                        let cav_r = clamp(0.48 + 0.40 * sh.y + 0.14 * sh.z, 0.28, 0.96);
                        let top_sun_r = max(cl_l.y, 0.12) * (0.42 + 0.58 * smoothstep(0.0, 0.82, sh.y));
                        let spike_w = twi_spike_soft * (1.0 - 0.58 * smoothstep(32000.0, 74000.0, t));
                        sun = e_sun * vc_ms(tau_l * sig_max, tau_n * sig_max, sun_slope, cav_r, top_sun_r, spike_w, cl_cs, low_sun, strat) * (VC_SUN_GAIN * (1.08 + 0.16 * day_hi));
                    }
                    var extra_a = vol_extra_amb(pc, dir);
                    if (night_w > 0.01) {
                        let r_city = length(pc.xz - SKY_CITY_CENTRE);
                        let city_w = skyu.amb_top.w * (1.0 - smoothstep(2600.0, 8200.0, r_city));
                        extra_a = mix(extra_a, vec3<f32>(0.046, 0.035, 0.029) * city_w, night_w);
                    }
                    let twi_dir = 0.32 + 0.68 * max(cl_cs, 0.0);
                    let alpen = skyu.twi.xyz * mix(vec3<f32>(1.06, 0.94, 0.88), vec3<f32>(1.12, 0.96, 0.72), low_sun) * smoothstep(0.0, 0.65, sh.y) * twi_dir * 0.24;
                    let amb = vc_ambient(vc_top + alpen, vc_side + alpen * 0.72, vc_bot + extra_a, sun_under, c2c_bounce, sh.y, sh.z, d, 0.52 + 0.44 * sh.z, sun_slope, 0.78, low_sun, night_w, strat);
                    let ext = exp(-sig * dt_span);
                    let wgt = Tc * (1.0 - ext);
                    Lc = Lc + wgt * (sun + amb);
                    tw = tw + wgt * t;
                    twt = twt + wgt;
                    Tc = Tc * ext;
                }
            }
            t = t + dt_span;
        }
        let alpha = 1.0 - Tc;
        if (alpha > 0.001) {
            let t_m = select(t0, tw / max(twt, 1e-6), twt > 1e-6);
            let cie = 0.72 * (1.0 + 1.8 * clamp(dir.y, 0.0, 1.0)) / 2.8;
            Lc = Lc * mix(1.0, max(cie, 0.34), strat);
            let dh_c = dir.y * t_m;
            let air_d = sky_path_density(dh_c);
            let ray_c = AT_RAY * 0.001 * exp(-max(eye.y + 0.5 * dh_c, 0.0) / 8000.0);
            let graze_h = mix(0.68, 1.12, smoothstep(0.16, 0.02, abs(dir.y)));
            let aer_c = max(skyu.atmo3.y, 0.000024) * air_d * mix(graze_h, 1.05, strat);
            let marine_hz = exp(-t_m * (1.0 - 0.88 * abs(dir.y)) / mix(64000.0, 28000.0, night_w));
            let far_rgb = exp(-(ray_c * mix(0.52, 0.34, low_sun) + vec3<f32>(aer_c)) * t_m) * marine_hz;
            let hor_fade = 1.0 - smoothstep(mix(48000.0, 18000.0, night_w), mix(92000.0, 48000.0, night_w), t_m);
            let alpha_h = alpha * hor_fade;
            let Lc_h = Lc * hor_fade;
            var behind = lk_sky_dome(dir);
            if (atmo_on) {
                behind = mix(behind, vol_atmo_sky(dir), skyu.atmo2.y);
            }
            L_out = Lc_h * far_rgb + behind * alpha_h * (vec3<f32>(1.0) - far_rgb);
            T_out = 1.0 - alpha_h;
        }
    }

    // (2) High-altitude altocumulus reflection layer (day/golden only; skipped at night).
    if (T_out > 0.05 && dir.y > 0.04 && skyu.cloud1.z > 0.0 && night_w < 0.55) {
        let low_sun_d = (1.0 - smoothstep(-0.02, 0.38, skyu.atmo.y)) * smoothstep(-0.14, -0.02, skyu.atmo.y);
        let fair_a = (1.0 - smoothstep(0.15, 0.62, strat)) * (1.0 - 0.82 * night_w);
        if (fair_a > 0.01) {
            let ta = cl_hit(eye.y, dir, 6750.0, false);
            if (ta > 0.0 && ta < 155000.0) {
                let pa = eye + dir * ta;
                let qa = pa.xz + skyu.cloud2.xy * 0.5 + vec2<f32>(9600.0, -5400.0);
                let am_uv = vec2<f32>(qa.y, -qa.x) / VC_HIGH_TILE + vec2<f32>(0.29, 0.63);
                let am = textureSampleLevel(vc_weather, vol_rep, am_uv, 2.0);
                let env_a = smoothstep(0.55, 0.84, 0.56 * am.r + 0.44 * am.g) * smoothstep(0.28, 0.70, am.b);
                if (env_a > 0.01) {
                    let qn = qa / 3200.0;
                    let flow_a = (vec2<f32>(am.b, am.g) - 0.5) * 0.24;
                    let auv = vec3<f32>(0.625 * qn.x + 0.25 * qn.y + flow_a.x, 0.42 + 0.12 * am.b, -0.375 * qn.x + 0.875 * qn.y + flow_a.y);
                    let ad = textureSampleLevel(vc_detail, vol_rep, auv, 1.5);
                    let alto_d = env_a * smoothstep(0.43, 0.81, 0.44 * ad.b + 0.30 * ad.a + 0.26 * ad.r);
                    let aa = clamp(alto_d * fair_a * smoothstep(0.06, 0.24, dir.y) * 0.48, 0.0, 0.45);
                    if (aa > 0.005) {
                        let e_sa = cl_sun(6750.0) * heavy;
                        let fwd_a = cl_hg(cl_cs, 0.66);
                        let ph_a = 0.56 * (fwd_a / (1.0 + 0.46 * fwd_a)) + 0.44 * cl_hg(cl_cs, -0.22);
                        let tau_a = alto_d * mix(1.45, 2.15, low_sun_d);
                        let ms_a = exp(-tau_a) * ph_a + (0.34 * exp(-0.42 * tau_a) + 0.14 * exp(-0.14 * tau_a)) / 12.566371;
                        let sunset_u = mix(vec3<f32>(1.0), vec3<f32>(1.20, 0.87, 0.82), low_sun_d * 0.72);
                        let twi_a = skyu.twi.xyz * (0.65 + 0.48 * max(cl_cs, 0.0)) * mix(0.75, 1.35, low_sun_d);
                        let ca = e_sa * ms_a * sunset_u * (VC_SUN_GAIN * 0.92) + mix(a_top, a_side, 0.5) * 0.90 + twi_a;
                        let fa = exp(-ta * (1.0 - 0.82 * abs(dir.y)) / 80000.0) * (1.0 - smoothstep(105000.0, 155000.0, ta));
                        let a_vis = aa * fa;
                        L_out = L_out + T_out * a_vis * ca;
                        T_out = T_out * (1.0 - a_vis);
                    }
                }
            }
        }
    }

    var out_v = vec4<f32>(max(L_out, vec3<f32>(0.0)), clamp(1.0 - T_out, 0.0, 1.0));
    if (any(out_v != out_v) || any(abs(out_v) > vec4<f32>(1e4))) {
        out_v = vec4<f32>(0.0);
    }
    if (fill_all) {
        for (var y = 0; y < 4; y = y + 1) {
            for (var x = 0; x < 4; x = x + 1) {
                textureStore(cl_refl_out, min(base_ij + vec2<i32>(x, y), dim - 1), out_v);
            }
        }
    } else {
        textureStore(cl_refl_out, ij, out_v);
    }
}

/// 2x2 box-filter mip downsample for the 128^2 RGBA16F hemispherical cloud reflection dome.
/// Because `cl_refl` stores premultiplied `(radiance, alpha)`, linear averaging preserves energy.
@compute @workgroup_size(8, 8)
fn cl_refl_down_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dim = textureDimensions(cl_refl_out);
    if (gid.x >= dim.x || gid.y >= dim.y) {
        return;
    }
    let p = vec2<i32>(gid.xy) * 2;
    let c = 0.25 * (
        textureLoad(cl_refl_src, p, 0) +
        textureLoad(cl_refl_src, p + vec2<i32>(1, 0), 0) +
        textureLoad(cl_refl_src, p + vec2<i32>(0, 1), 0) +
        textureLoad(cl_refl_src, p + vec2<i32>(1, 1), 0)
    );
    textureStore(cl_refl_out, vec2<i32>(gid.xy), c);
}
