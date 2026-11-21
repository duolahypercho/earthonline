// the atmosphere volume at half resolution (appended to look_common.wgsl,
// which carries sky_common.wgsl): per pixel, along the view ray up to the
// scene depth, (A) sun shafts in the haze inside the cascades' reach (the
// cascaded sun shadow per sample), (B) the marine layer (a fog bank whose top
// rolls with a drifting field, lit by the key through the cloud shadow and by
// the fog colour), (C) the low cloud deck marched through its shell (clouds:
// the density and light of sky_clouds.wgsl, appended to this module), (D) the
// high deck as one sheet, lit by the same sun. Output: in-scattered radiance (rgb) and
// transmittance (a), and the view distance it was made at.

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var<uniform> skyu: Sky;
@group(0) @binding(2) var vol_depth: texture_depth_2d;
@group(0) @binding(3) var vol_sheet: texture_2d<f32>;
@group(0) @binding(4) var vol_noise: texture_3d<f32>;
@group(0) @binding(5) var vol_rep: sampler;
@group(0) @binding(6) var vol_shadow: texture_depth_2d_array;
@group(0) @binding(7) var vol_cmp: sampler_comparison;
@group(0) @binding(8) var vol_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(9) var vol_zout: texture_storage_2d<r32float, write>;
@group(0) @binding(10) var vol_prev: texture_2d<f32>;
@group(0) @binding(11) var vol_prevz: texture_2d<f32>;
@group(0) @binding(12) var vol_lin: sampler;
// the atmosphere's transmittance and sky-view tables (sky_atmo.wgsl).
@group(0) @binding(13) var vol_at_trans: texture_2d<f32>;
@group(0) @binding(14) var vol_at_view: texture_2d<f32>;
// the aerial-perspective volume (sky_atmo_luts.wgsl `at_ap_main`): in-scattered light per
// unit solar illuminance and transmittance, by direction about the sun and distance.
@group(0) @binding(20) var vol_ap_in: texture_3d<f32>;
@group(0) @binding(21) var vol_ap_tr: texture_3d<f32>;
// (`--look cinematic`, stage L4) the far sun shadow map (far_shadow.wgsl, `--far-shadows`) or
// a stand-in: the shade in the golden haze past the cascades.
@group(0) @binding(22) var vol_far_map: texture_depth_2d_array;

/// the atmosphere's sky radiance along a direction (look units).
/// Its zenith is held to the look's own zenith luminance (the JS sky model's,
/// `look.dome_zenith`) at every hour: the exposure, the night solve and the
/// rig were all made against that dome, and a physical twilight sky (100-1000 x
/// darker than noon) under the look's day-for-night exposure went black. The
/// tables keep the shape and colour: the reddened sun, the afterglow toward it,
/// the Earth's shadow and the pink band opposite.
fn vol_atmo_gain() -> f32 {
    let z = textureSampleLevel(vol_at_view, vol_lin, at_view_uv(vec3<f32>(0.0, 1.0, 0.0), skyu.atmo.xyz), 0.0).rgb;
    let lz = dot(z, vec3<f32>(0.2126, 0.7152, 0.0722));
    // the target is the drawn dome's zenith (`lk_sky_dome`, the look's grade
    // and its low-sun veil included), not the bare model zenith: at a low sun the
    // model's zenith is ~1/10 of a measured one and the sky went near black
    // (12-37 sRGB) under a golden-hour exposure; the grade's zenith keeps the
    // evening ladder monotone (golden brighter than blue hour brighter than night).
    // (geometric mean with the model zenith: the graded zenith alone washed a golden sky pale)
    let tgt = sqrt(lk_luma(lk_sky_dome(vec3<f32>(0.0, 1.0, 0.0))) * dot(look.dome_zenith.xyz, vec3<f32>(0.2126, 0.7152, 0.0722)));
    return clamp(tgt / max(lz, 1e-9), 0.2 * skyu.atmo.w, 400.0 * skyu.atmo.w);
}

/// the physical daylight's weight (`atmo4.w`; compiled out with `--no-phys-sky` at
/// start: look.rs `ShaderFeatures::phys_sky`)
fn vol_phys() -> f32 {
    return select(0.0, skyu.atmo4.w, SF_PHYS_SKY);
}

/// the scale the tables are drawn at (look units per unit solar illuminance): the
/// look's dome anchor (`vol_atmo_gain`), or with the physical daylight the sun's
/// own irradiance above the atmosphere in its colour (`atmo4`), by its weight.
fn vol_atmo_scale() -> vec3<f32> {
    let ph = vol_phys();
    if (ph <= 0.0) {
        return vec3<f32>(vol_atmo_gain());
    }
    if (ph >= 1.0) {
        return skyu.atmo4.rgb;
    }
    return mix(vec3<f32>(vol_atmo_gain()), skyu.atmo4.rgb, ph);
}

fn vol_atmo_sky(d: vec3<f32>) -> vec3<f32> {
    let uv = at_view_uv(d, skyu.atmo.xyz);
    let gs = vol_atmo_scale();
    let ph = clamp(vol_phys(), 0.0, 1.0);
    // the gain that holds the tables' zenith to the look's (`vol_atmo_gain`) scaled the
    // whole low-sun sky, its aureole too: with the look's zenith brighter than the tables' at a
    // low sun, the aureole clipped over 10-20 deg and veiled what stood before the sun. From
    // 1.5 to 30 deg round a sun under 15 deg the tables keep their own zenith's scale (the model
    // zenith anchor), so the glare falls off steeply round a small clipped core; the zenith
    // and the sky away from the sun keep the look's anchor (its blue).
    let g_look = vol_atmo_gain();
    let z = textureSampleLevel(vol_at_view, vol_lin, at_view_uv(vec3<f32>(0.0, 1.0, 0.0), skyu.atmo.xyz), 0.0).rgb;
    let g_model = clamp(dot(look.dome_zenith.xyz, vec3<f32>(0.2126, 0.7152, 0.0722)) / max(dot(z, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-9), 0.2 * skyu.atmo.w, 400.0 * skyu.atmo.w);
    let ang = acos(clamp(dot(d, skyu.atmo.xyz), -1.0, 1.0)) * 57.29578;
    let near_sun = smoothstep(0.5, 1.5, ang) * (1.0 - smoothstep(18.0, 30.0, ang)) * (1.0 - smoothstep(10.0, 15.0, asin(clamp(skyu.atmo.y, -1.0, 1.0)) * 57.29578));
    var c = at_spectral(textureSampleLevel(vol_at_view, vol_lin, uv, 0.0).rgb * gs, ph);
    // Only the excess over the sky 40 deg round in azimuth at the same elevation is scaled (a
    // straight gain switch drew a dark ring round the sun): the glare never falls below the
    // sky beside it.
    // (the physical daylight's sky is at the sun's own scale: no anchor to undo)
    if (near_sun > 0.0 && ph < 1.0) {
        let sh = normalize(skyu.atmo.xz + vec2<f32>(1e-5, 0.0));
        let ca = 0.766;
        let sa = 0.643;
        let r2 = vec2<f32>(ca * sh.x - sa * sh.y, sa * sh.x + ca * sh.y);
        let hd = sqrt(max(1.0 - d.y * d.y, 0.0));
        let dref = vec3<f32>(r2.x * hd, d.y, r2.y * hd);
        let lref = dot(textureSampleLevel(vol_at_view, vol_lin, at_view_uv(dref, skyu.atmo.xyz), 0.0).rgb * g_look, vec3<f32>(0.2126, 0.7152, 0.0722));
        let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        if (l > lref) {
            // (stage L4, the cinematic look) VOL_CINE_AUREOLE_KEEP of the cut: the low sun's glow
            let ns = select(near_sun, near_sun * VOL_CINE_AUREOLE_KEEP, vol_cine_on());
            let k = mix(1.0, min(g_model / max(g_look, 1e-9), 1.0), ns * (1.0 - ph));
            c = c * ((lref + (l - lref) * k) / l);
        }
    }
    // the three-channel tables lead with green in the low sky at golden hour
    // (olive-khaki, where the eye sees peach or lavender-grey): green's excess over
    // red and blue goes to them.
    // green-led hues (reddened sun x the Rayleigh blue peaks in green: an olive band
    // between the blue and the warm low sky) go toward a warm neutral of the same luminance,
    // not into red and blue (that shift drew a magenta stripe).
    // The lowest 4 deg of sky rise at most 25 % over the sky at 4 deg in the same azimuth: the long
    // aerosol path near the horizon drew a clipped white band ~1 deg tall running ~30 deg either
    // side of a low sun with no fall-off.
    if (d.y > -0.02 && d.y < 0.0698) {
        let hd4 = sqrt(max(1.0 - d.y * d.y, 1e-6));
        let d4 = vec3<f32>(d.x / hd4 * 0.99756, 0.0698, d.z / hd4 * 0.99756);
        let l4 = dot(textureSampleLevel(vol_at_view, vol_lin, at_view_uv(d4, skyu.atmo.xyz), 0.0).rgb * gs, vec3<f32>(0.2126, 0.7152, 0.0722));
        let lh = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        let cap = 1.25 * l4;
        if (lh > cap) {
            c = c * mix(cap / lh, 1.0, smoothstep(0.035, 0.0698, d.y));
        }
    }
    let lc = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let gex = clamp((c.g - max(c.r, c.b)) / max(c.g, 1e-6) * 12.0, 0.0, 0.9);
    c = mix(c, lc * vec3<f32>(1.08, 0.99, 0.86), gex);
    // no shift of green's excess into red and blue (it drew a magenta stripe over a
    // khaki one); the aerosol's colour in the tables fixes the cause.
    // the Earth's shadow and the pink belt over it, opposite a sun from 3 deg up
    // to 7 deg down: the shadow a blue-grey band from the horizon to about the sun's
    // depression + 1.5 deg, the belt the 8 deg above it, rose; strongest straight
    // away from the sun, gone 90 deg round (the tables carried neither: the
    // anti-solar sky was one indigo from top to horizon).
    let sun = skyu.atmo.xyz;
    let alt = asin(clamp(sun.y, -1.0, 1.0)) * 57.29578;
    let tw = smoothstep(3.0, 0.5, alt) * smoothstep(-7.0, -3.5, alt);
    let sh = length(sun.xz);
    let dh = length(d.xz);
    if (tw > 0.0 && sh > 1e-4 && dh > 1e-4) {
        let anti = max(-dot(d.xz / dh, sun.xz / sh), 0.0);
        let wa = tw * anti * anti;
        let el = asin(clamp(d.y, -1.0, 1.0)) * 57.29578;
        let top = max(-alt, 0.0) + 1.5;
        let shadow = 1.0 - smoothstep(top - 0.8, top + 1.2, el);
        let belt = smoothstep(top, top + 2.0, el) * (1.0 - smoothstep(top + 5.0, top + 12.0, el));
        let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        // peach-rose (1.3, 0.98, 0.82) + (0.10, 0.03, 0) l (1.35 / 0.95 / 1.0 + 0.12 / 0 / 0.03:
        // mixed into the blue sky it read lilac)
        // through the blue hour (x smoothstep(-7, -2) of the altitude; -6 .. 0 before: at -4
        // deg the arch was a quarter strength and the east read flat violet)
        c = mix(c, c * vec3<f32>(1.45, 1.0, 0.8) + vec3<f32>(0.14, 0.04, 0.0) * l, belt * wa * smoothstep(-7.0, -2.0, alt));
        c = mix(c, l * vec3<f32>(0.62, 0.72, 0.95), shadow * wa * smoothstep(1.5, -1.0, alt) * 0.75);
    }
    return c;
}

// (`--look cinematic`, the cinematic look plan's stage L4; SKY_CINEMATIC, compiled out with
// SF_CINEMATIC) the golden haze: the aerosol of the aerial perspective
//  * thins with height from its own floor (a scale height of VOL_CINE_HAZE_H_M over a
//    VOL_CINE_HAZE_DEAD_M dead band: the mean density between the eye's height and the
//    point's, so a view from 1-3 km up looks down through a haze layer over the city that
//    thins upward, where the default's 400 m law takes only the height difference);
//  * is VOL_CINE_GOLDEN_GAIN denser with the sun low (the marine air of a clear evening:
//    IMPROVE Point Reyes 15:00 runs to 156-196 Mm^-1 on its haziest days against a
//    median of 49, reference sheet 4.3);
//  * scatters VOL_CINE_DIRECT_SHARE of its in-scatter from the key through a
//    Henyey-Greenstein phase of VOL_CINE_G (haze ~0.75: Narasimhan & Nayar, CVPR 2003,
//    reference sheet 4.2) and the rest from the sky (its horizon colour x
//    VOL_CINE_SKY_SHARE): distance glows warm toward the sun and turns cooler and darker
//    away from it. The direct part is held under VOL_CINE_SUNWARD_CAP x the sky drawn above
//    it in that azimuth, so the haze toward the sun never stands brighter than its sky.
const VOL_CINE_HAZE_H_M: f32 = 600.0;
const VOL_CINE_HAZE_DEAD_M: f32 = 20.0;
const VOL_CINE_GOLDEN_GAIN: f32 = 0.4;
const VOL_CINE_DIRECT_SHARE: f32 = 0.6;
const VOL_CINE_G: f32 = 0.75;
const VOL_CINE_SKY_SHARE: f32 = 0.55;
const VOL_CINE_SUNWARD_CAP: f32 = 2.5;
// (stage L4) the share of the low sun's aureole cut (`vol_atmo_sky`) the cinematic look keeps:
// half, the glow round the sun brighter and wider (the default holds it to the tables' zenith).
const VOL_CINE_AUREOLE_KEEP: f32 = 0.5;
// how far the shade in the haze is marched (m): the cascades' 160 m, then the far map to here
// (the street-level ring's fade-out begins ~525 m out, B2).
const VOL_CINE_SHADE_FAR_M: f32 = 900.0;

fn vol_cine_on() -> bool {
    return SF_CINEMATIC && sky_on(skyu.flags.x, SKY_CINEMATIC);
}

/// 0..1: a low sun (under 25 deg, full under 8 deg, gone 4 deg below the horizon).
fn vol_cine_low_sun() -> f32 {
    let alt = asin(clamp(skyu.atmo.y, -1.0, 1.0)) * 57.29578;
    return smoothstep(25.0, 8.0, alt) * smoothstep(-4.0, 0.0, alt);
}

/// 0..1: the key is the sun's (the direct share fades out with it).
fn vol_cine_day() -> f32 {
    return smoothstep(-4.0, 1.0, asin(clamp(skyu.atmo.y, -1.0, 1.0)) * 57.29578);
}

/// The key's single scatter off the haze along `dir` (radiance per unit optical depth),
/// held under VOL_CINE_SUNWARD_CAP x `sky`'s luminance (the sky over the horizon in that
/// azimuth, `vol_cine_sky`).
fn vol_cine_direct(dir: vec3<f32>, sky: vec3<f32>) -> vec3<f32> {
    let direct = skyu.sun_rad.xyz * sky_hg(dot(dir, skyu.atmo.xyz), VOL_CINE_G);
    let ld = dot(direct, vec3<f32>(0.2126, 0.7152, 0.0722));
    let cap = VOL_CINE_SUNWARD_CAP * dot(sky, vec3<f32>(0.2126, 0.7152, 0.0722));
    return select(direct, direct * (cap / max(ld, 1e-12)), ld > cap);
}

/// The sky over the horizon in `dir`'s azimuth (`vol_aerial`'s in-scatter colour).
fn vol_cine_sky(dir: vec3<f32>) -> vec3<f32> {
    let dh_dir = normalize(vec3<f32>(dir.x, max(abs(dir.y), 0.03), dir.z));
    var col = lk_sky_dome(dh_dir);
    if (skyu.atmo2.x > 0.5) {
        col = mix(col, vol_atmo_sky(dh_dir), skyu.atmo2.y);
    }
    return col;
}

/// The aerosol's extinction at its floor (1/m).
fn vol_cine_aerosol() -> f32 {
    return skyu.atmo3.y * (1.0 + VOL_CINE_GOLDEN_GAIN * vol_cine_low_sun());
}

/// The aerosol's density at a height over its floor's (0..1).
fn vol_cine_density_at(y: f32) -> f32 {
    return exp(-max(y - VOL_CINE_HAZE_DEAD_M, 0.0) / VOL_CINE_HAZE_H_M);
}

/// The aerosol's mean density between two heights over its floor's.
fn vol_cine_density(y0: f32, y1: f32) -> f32 {
    let a = max(min(y0, y1) - VOL_CINE_HAZE_DEAD_M, 0.0) / VOL_CINE_HAZE_H_M;
    let b = max(max(y0, y1) - VOL_CINE_HAZE_DEAD_M, 0.0) / VOL_CINE_HAZE_H_M;
    if (b - a < 1e-3) {
        return exp(-0.5 * (a + b));
    }
    return (exp(-a) - exp(-b)) / (b - a);
}

/// aerial perspective (transmittance, in-scattered radiance) over a
/// view distance: Rayleigh at the ground (5.8, 13.6, 33.1 e-3 / km) and clear
/// coastal aerosol (`atmo3.y`) plus the weather's own (`atmo3.x`, from its
/// visibility), both thinning with height as the JS aerial path density; the
/// light scattered in is the sky toward the horizon in that azimuth (the
/// tables by day, the weather's or the night's dome as they take over), so a
/// far hill fades to the sky behind it, warm toward a low sun, blue-grey away
/// from it. Replaces the range fog (45 % at 4.2 km on a clear noon).
/// the aerial-perspective volume along a direction over a distance (m): (in-scattered
/// light per unit solar illuminance, transmittance), both rgb. Slice k of n ends at
/// reach x ((k + 1) / n)^2; under the first slice the light grows from none.
struct VolAp {
    ins: vec3<f32>,
    tr: vec3<f32>,
};

fn vol_ap(dir: vec3<f32>, dist: f32) -> VolAp {
    let uv = at_view_uv(dir, skyu.atmo.xyz);
    let n = f32(textureDimensions(vol_ap_in).z);
    let s = sqrt(clamp(dist * 0.001 / skyu.atmo_s.w, 0.0, 1.0)) * n;
    let w = clamp((s - 0.5) / n, 0.5 / n, 1.0 - 0.5 / n);
    var o: VolAp;
    o.ins = textureSampleLevel(vol_ap_in, vol_lin, vec3<f32>(uv, w), 0.0).rgb;
    o.tr = textureSampleLevel(vol_ap_tr, vol_lin, vec3<f32>(uv, w), 0.0).rgb;
    if (s < 1.0) {
        let k = s * s;
        o.ins = o.ins * k;
        o.tr = mix(vec3<f32>(1.0), o.tr, k);
    }
    return o;
}

fn vol_aerial(dir: vec3<f32>, dist: f32, eye_y: f32) -> vec4<f32> {
    let dh = dir.y * dist;
    var dens = sky_path_density(dh);
    // (stage L4) the golden haze: its own height law and the low sun's marine air.
    var aer = skyu.atmo3.y;
    let cine = vol_cine_on();
    if (cine) {
        dens = vol_cine_density(eye_y, eye_y + dh);
        aer = vol_cine_aerosol();
    }
    let ray = AT_RAY * 0.001 * exp(-max(eye_y + 0.5 * dh, 0.0) / 8000.0);
    // the weather's own extinction lives under its deck or marine layer: none for an
    // eye 200 m over the low deck's top (the fog sea seen from above went hazy blue)
    // 1 (the eye's height no longer switches it off): the in-layer share of the path
    // (`inside`, r11) already keeps the veil to the layer, and with the veil off for an eye
    // 200 m over the fog top the houses under it ghosted through the fog sea seen from above.
    let wk = 1.0;
    let above_top = smoothstep(skyu.cloud0.y, skyu.cloud0.y + 100.0, eye_y);
    let tau = (ray + vec3<f32>(aer * dens)) * dist;
    // fog, drizzle, rain, storm: the weather's own veil keeps the look's range-fog
    // law over its visibility (near .. far), on top of the clear air
    // Koschmieder over the path inside the layer (95 % at the weather's
    // `far`), with no height thinning below its top: advection fog and a rain veil are
    // about uniform up to the deck (the 400 m path density let a bridge tower 200 m up
    // take less fog than the trees at its foot, and the range-fog law kept the first
    // `near` metres clear). A point above the top takes the share of its path below it.
    let ptop = skyu.cloud0.y;
    let y1 = eye_y + dh;
    let lo_y = min(eye_y, y1);
    let hi_y = max(eye_y, y1);
    let inside = select(clamp((ptop - lo_y) / max(hi_y - lo_y, 1e-3), 0.0, 1.0), 1.0, hi_y <= ptop);
    let wv = skyu.atmo3.x * wk * (1.0 - exp(-3.0 * dist * inside / max(skyu.atmo3.w, 1.0)));
    let t = exp(-tau);
    let dh_dir = normalize(vec3<f32>(dir.x, max(abs(dir.y), 0.03), dir.z));
    var col = lk_sky_dome(dh_dir);
    if (skyu.atmo2.x > 0.5) {
        col = mix(col, vol_atmo_sky(dh_dir), skyu.atmo2.y);
    }
    if (cine) {
        // (stage L4) the aerosol's share of the in-scatter, lit by the key through its phase.
        let ta = aer * dens * dist;
        let tr = dot(ray, vec3<f32>(0.2126, 0.7152, 0.0722)) * dist;
        let share = ta / max(ta + tr, 1e-12);
        let direct = vol_cine_direct(dir, col);
        // x the path's transmittance: a far point hazed out meets the sky drawn behind it (the
        // sky pixels take no phase term; at full weight the far land stood darker than the sky
        // above it away from the sun, a line along the horizon from the air).
        let tv = dot(t, vec3<f32>(0.2126, 0.7152, 0.0722));
        col = mix(col, direct + col * VOL_CINE_SKY_SHARE, VOL_CINE_DIRECT_SHARE * share * vol_cine_day() * tv);
    }
    let tl = dot(t, vec3<f32>(0.2126, 0.7152, 0.0722));
    // the weather's veil takes the light under its deck (the light the
    // rain shafts and the fog bank are drawn with), not the clear-sky table's
    // horizon: the fogged far hills went pale blue against a grey fog bank.
    // and toward the grey of the low sky the weather draws in that azimuth (0.7),
    // neutral: the light under the deck alone came out lighter and bluer than the fog
    // bank, so the fogged far hills stood out pale blue and the sea ended at them in a
    // hard line.
    let cw0 = mix(skyu.amb_base.xyz, skyu.amb_top.xyz, 0.5) * 0.9;
    let cl = dot(lk_sky_dome(dh_dir), vec3<f32>(0.2126, 0.7152, 0.0722));
    // x 0.7: measured against the fog bank drawn over the strait (golden-gate-fog), the
    // dome's low sky is ~1.4 x as bright as the bank the volume draws there.
    // x 0.6: at 0.7 fogged far land read ~1.4 x (linear) the fog sky drawn just above it, at 0.5
    // it stood dark against it.
    var colw = mix(cw0, vec3<f32>(cl) * vec3<f32>(0.98, 1.0, 1.02), 0.7) * 0.6;
    // seen from above, the fog is its sunlit top: the dome's low-sky brightness, neutral
    if (skyu.atmo2.x > 0.5) {
        // the sky's own horizon colour (its blue-white), so the fog sea meets the sky with no hue step
        let cs = vol_atmo_sky(dh_dir);
        colw = mix(colw, mix(vec3<f32>(cl), cs, skyu.atmo2.y), above_top);
    } else {
        colw = mix(colw, vec3<f32>(cl) * vec3<f32>(0.99, 1.0, 1.01), above_top);
    }
    // under a veiling deck the clear air's own in-scatter is lit by the deck
    // too (grey, not the clear sky's blue: the part of a far object the veil left showed
    // bluer and lighter than the fog round it).
    let colc = mix(col, colw, skyu.atmo3.x * wk);
    // (`--look cinematic` keeps its own golden haze above: stage L4's in-scatter and the
    // volume's would both add the sun's phase)
    if (SF_AP_LUT && skyu.atmo_m.w > 0.5 && skyu.atmo2.x > 0.5 && !cine) {
        // the aerial-perspective volume: the medium's own in-scatter (the sun's phase, the
        // planet's shadow, the multiple scattering) at the sky's scale, its transmittance, in
        // place of the fade toward the horizon's colour; the weather's veil as above.
        let ap = vol_ap(dir, dist);
        let ta = dot(ap.tr, vec3<f32>(0.2126, 0.7152, 0.0722));
        let ins = mix(at_spectral(ap.ins * vol_atmo_scale(), clamp(vol_phys(), 0.0, 1.0)), colw * (1.0 - ta), skyu.atmo3.x * wk);
        return vec4<f32>(ins * (1.0 - wv) + colw * wv, ta * (1.0 - wv));
    }
    return vec4<f32>(colc * (1.0 - tl) * (1.0 - wv) + colw * wv, tl * (1.0 - wv));
}

/// the sun's transmittance from the top of the atmosphere to a
/// height (m), zero where the planet hides it (a cloud 1.5 km up keeps the sun
/// ~1.2 deg after it has set for the ground).
fn vol_atmo_sun_t(h_m: f32) -> vec3<f32> {
    let r = AT_R + max(h_m, 1.0) * 0.001;
    let mu = skyu.atmo.y;
    if (at_ground_hit(r, mu)) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(vol_at_trans, vol_lin, at_trans_uv(r, mu), 0.0).rgb;
}

/// the sun's disc (0.533 deg) through the atmosphere, limb-darkened, in look units.
fn vol_atmo_disc(d: vec3<f32>) -> vec3<f32> {
    let c = dot(d, skyu.atmo.xyz);
    let cr = cos(0.2667 * 0.0174533);
    if (c < cr) {
        return vec3<f32>(0.0);
    }
    let x = clamp((1.0 - c) / (1.0 - cr), 0.0, 1.0);
    let limb = 1.0 - 0.6 * (1.0 - sqrt(1.0 - x));
    return vol_atmo_sun_t(skyu.eye.y) * vol_atmo_gain() * 3000.0 * limb;
}

/// The sky's light on the clouds, from the atmosphere tables: `amb` the whole sky's
/// mean (the light from below takes half of it), `zen` the zenith x 1.2 (a twilight top),
/// `top` the upper sky as a surface facing up sees it (the zenith and a ring 35 deg up: the
/// sky's blue, not the pale horizon), `sun` / `anti` the half of the sky a side faces
/// toward the sun or away from it. Made once a frame by the cloud-shadow pass (sky_clouds
/// `cl_shadow_main`, five table reads) into five texels of its map.
struct ClSky {
    amb: vec3<f32>,
    zen: vec3<f32>,
    top: vec3<f32>,
    sun: vec3<f32>,
    anti: vec3<f32>,
};

fn cl_sky_light() -> ClSky {
    let sun_h = normalize(vec3<f32>(skyu.atmo.x, 0.0, skyu.atmo.z) + vec3<f32>(1e-5, 0.0, 0.0));
    let side = vec3<f32>(-sun_h.z, 0.0, sun_h.x);
    let zz = vol_atmo_sky(vec3<f32>(0.0, 1.0, 0.0));
    let zen = zz * 1.2;
    // the rings 35 deg up: lower rings are pale, near the horizon's white, and the shaded
    // side of a cloud comes out light grey or, at a low sun, brown
    let e2 = 0.7;
    let ss2 = vol_atmo_sky(normalize(sun_h + vec3<f32>(0.0, e2, 0.0)));
    let aa2 = vol_atmo_sky(normalize(-sun_h + vec3<f32>(0.0, e2, 0.0)));
    let s12 = vol_atmo_sky(normalize(side + vec3<f32>(0.0, e2, 0.0)));
    let s22 = vol_atmo_sky(normalize(-side + vec3<f32>(0.0, e2, 0.0)));
    let top = 0.55 * zz + 0.45 * (ss2 + aa2 + s12 + s22) * 0.25;
    // (five table reads: the whole-sky mean from the zenith and the ring)
    let amb = (zz + (ss2 + aa2 + s12 + s22) * 0.25) * 0.5;
    let sun = 0.4 * zz + 0.6 * (0.5 * ss2 + 0.25 * (s12 + s22));
    let anti = 0.4 * zz + 0.6 * (0.5 * aa2 + 0.25 * (s12 + s22));
    return ClSky(amb, zen, top, sun, anti);
}

/// Metres a half-resolution pixel spans at distance `t`.
fn vol_footprint(t: f32) -> f32 {
    return t * 4.0 * skyu.cluster.z * skyu.res.w;
}

fn vol_sun_visibility(p: vec3<f32>) -> f32 {
    let vd = dot(p - look.cam_pos.xyz, look.cam_fwd.xyz);
    let split = look.cascade_split;
    if (vd >= split.z || look.key_dir.w <= 0.0) {
        return 1.0;
    }
    var c = 0;
    if (vd > split.x) {
        c = 1;
    }
    if (vd > split.y) {
        c = 2;
    }
    let clip = look.cascade_vp[c] * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    let v = textureSampleCompareLevel(vol_shadow, vol_cmp, uv, c, clip.z + look.cascade_bias[c].y);
    return mix(1.0, v, look.key_dir.w);
}

/// (stage L4) sun visibility past the cascades from the far rings (far_shadow.wgsl
/// `lk_far_shadow`'s rings and fades, one compare, no normal offset: a point in the air),
/// 1 without them.
fn vol_far_visibility(p: vec3<f32>) -> f32 {
    if (!(lk_lx_on(LX_BIT_FAR_SHADOWS) && look.lx[9].x > 0.0)) {
        return 1.0;
    }
    var vis = 0.0;
    var rest = 1.0;
    let rings = u32(look.lx[9].x + 0.5);
    for (var k = 0u; k < rings; k = k + 1u) {
        let r = 1u + 4u * k;
        let prm = look.lx[r + 3u];
        if (prm.w <= 0.0) {
            continue;
        }
        let q = vec4<f32>(p, 1.0);
        let c = vec3<f32>(dot(look.lx[r], q), dot(look.lx[r + 1u], q), dot(look.lx[r + 2u], q));
        let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
        let e = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
        let radial = 1.0 - smoothstep(prm.w * look.lx[9].z, prm.w, length(p.xz - look.cam_pos.xz));
        let w = smoothstep(0.0, look.lx[9].y, e) * radial * select(0.0, 1.0, c.z >= 0.0 && c.z <= 1.0);
        if (w > 0.0) {
            let v = textureSampleCompareLevel(vol_far_map, vol_cmp, uv, i32(prm.x + 0.5), c.z);
            vis = vis + rest * w * v;
            rest = rest * (1.0 - w);
            if (rest <= 0.0) {
                break;
            }
        }
    }
    return mix(1.0, vis + rest, look.key_dir.w);
}

fn vol_cloud_t(p: vec3<f32>) -> f32 {
    // not under a sun within 3.5 deg of the horizon: the deck's shadow then
    // lands tens of km away and drew a fan of streaks on the low haze.
    if (!sky_on(skyu.flags.x, SKY_CLOUD_SHADOWS) || look.key_dir.y <= 0.06) {
        return 1.0;
    }
    // the cloud-shadow map (the deck's own shape along the key), as the city reads it;
    // under a continuous deck the deck's beam floor (the map is not marched there).
    let tc = sky_cshadow_at(cl_shadow, vol_rep, p, look.key_dir.xyz, look.cam_pos.xyz, SKY_SEA_Y);
    return mix(tc, skyu.cloud1.w, clamp((skyu.cloud0.z - 0.75) / 0.2, 0.0, 1.0));
}

/// the key scattered at a point under optical depth `tau` toward it:
/// three multiple-scattering octaves (x0.6 scatter, x0.35 extinction, x0.6
/// g), the cloud deck's own law.
fn vol_ms(tau: f32, cos_sun: f32) -> f32 {
    // four octaves, each 0.5 the light, 0.3 the extinction and 0.6 the
    // anisotropy of the one before (three at 0.6 / 0.35 before: the octaves
    // kept a backlit core nearly as bright as its rim); the first octave's
    // phase carries the droplets' strong forward peak (the silver lining).
    var e = 0.0;
    var a = 1.0;
    var b = 1.0;
    var k = 1.0;
    for (var o = 0; o < 4; o = o + 1) {
        e = e + a * exp(-b * tau) * select(sky_cloud_phase(cos_sun, k), vol_droplet_phase(cos_sun), o == 0);
        // 0.42 (0.5 before: a thick core toward a low sun stayed lit cream)
        a = a * 0.42;
        b = b * 0.3;
        k = k * 0.5; // 0.5 (0.6): the deeper octaves near isotropic, a backlit core darker
    }
    return e;
}

/// water droplets' phase, a sharp forward peak over the cloud lobe:
/// 0.2 HG(0.97) + 0.8 (0.85 HG(0.76) + 0.15 HG(-0.25)) (a few-degree forward
/// diffraction peak; the silver lining on a cloud's thin edge toward the sun).
fn vol_droplet_phase(c: f32) -> f32 {
    return 0.2 * sky_hg(c, 0.97) + 0.8 * sky_cloud_phase(c, 1.0);
}

/// light on a cloud or the fog from below and around it that is not
/// the key: the city's glow, only over the city (it fell on the bank and on
/// clouds far out over the Pacific too), and the twilight sky toward the set
/// sun (`twi`), strongest on the side of the cloud toward it.
fn vol_extra_amb(p: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    var a = vec3<f32>(0.0);
    let g = skyu.amb_top.w;
    if (g > 0.0) {
        let r = length(p.xz - SKY_CITY_CENTRE);
        a = a + SKY_CITY_BASE_RGB * (g * (1.0 - smoothstep(4500.0, 9500.0, r)));
    }
    if (any(skyu.twi.xyz > vec3<f32>(0.0))) {
        let sh = length(look.sun.xz);
        let dh = length(dir.xz);
        var c = 0.0;
        if (sh > 1e-6 && dh > 1e-6) {
            c = max(dot(dir.xz, look.sun.xz) / (sh * dh), 0.0);
        }
        // only where the sun still reaches the cloud; a cloud in the Earth's
        // shadow is lit by the sky alone (blue-grey), not the warm twilight
        let lit = select(0.5, 1.0, !at_ground_hit(AT_R + max(p.y, 1.0) * 0.001, skyu.atmo.y)); // 0.5 (0.15: soot at blue hour)
        a = a + skyu.twi.xyz * (0.25 + 0.75 * c * c) * lit;
    }
    return a;
}

/// The high deck stands at cirrus height (eo-sky clouds.rs `CIRRUS_ALT_M`, 9.5 km) on a
/// cirrus day; the fair-weather sheet stays at 5.2 km (`FAIR_HIGH_ALT_M`). While the clear
/// sky's state fades with the weather (clouds.rs `Decks::with_sky`), a fair sheet uniform
/// between 5.2 and 6.2 km (`FADE_BAND_M`) is the sheet at 5.2 km at a share of its opacity.
const VOL_CIRRUS_MIN_M: f32 = 7000.0;
const VOL_FAIR_HIGH_M: f32 = 5200.0;
const VOL_FAIR_FADE_M: f32 = 1000.0;

fn vol_cirrus_day() -> bool {
    return skyu.cloud1.y > VOL_CIRRUS_MIN_M && skyu.cloud1.z > 0.0;
}

/// The cirrus's high wind axis (x east, z south): toward a little south of east.
const VOL_CIRRUS_AXIS: vec2<f32> = vec2<f32>(0.95, 0.31);

/// Cirrus fibratus: the streak's shape 0..1 at a point of its plane, `t` metres along the
/// view `dir` (the optical depth is `CIRRUS_TAU` x it). Long, soft, nearly straight bands
/// drawn out along the high wind (the drift `cloud2.zw` moves them; the jet-level
/// westerlies over the coast blow from a little south of west), each band a smooth ridge
/// of the sheet's channels read almost constant along the wind (200 : 1), tapering in and
/// out along its length, with fine parallel fibres inside it; the bands bend slowly and
/// gather in a few patches. (INFERRED shape constants: the stretch, tiles and thresholds
/// are chosen to the reference photographs' streaks, not measured.)
fn vol_cirrus(pxz: vec2<f32>, t: f32, dir: vec3<f32>) -> f32 {
    let w = pxz + skyu.cloud2.zw;
    let ax = VOL_CIRRUS_AXIS;
    let u = dot(w, ax);
    let v = dot(w, vec2<f32>(-ax.y, ax.x));
    // patches: the high channel at 60 km a tile, drawn out 4 : 1 along the wind; most of
    // the sky stays open
    let env = smoothstep(0.6, 0.92, textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / 4.0, v) / 60000.0, 2.0).g);
    if (env <= 0.0) {
        return 0.0;
    }
    // how end-on the view looks along the wind (1: straight along it). A band is a line of
    // constant `vf`; seen end-on, perspective squeezes its length and keeps its width, so
    // its slow bend (kilometres over tens of kilometres) turns into a crisp vertical S
    // curl on screen. End-on, the band term fades to its mean (a soft veil in the patch)
    // and the bend to nothing.
    let dh = dir.xz / max(length(dir.xz), 1e-4);
    let endon = abs(dot(dh, ax));
    let side = 1.0 - smoothstep(0.8, 0.97, endon);
    // a slow bend of the bands (the marine channel, 80 km along the wind; four mips down,
    // so only its broad swing bends them)
    let vf = v + (textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / 80000.0, 0.37), 4.0).b - 0.5) * 6000.0 * side;
    // the footprint across the wind (the band field barely varies along it): the pixel's
    // span across the view, and its span along the view stretched by the grazing angle,
    // each projected on the across-wind axis; its mip on the band tile (12 km across the
    // wind, 256 texels)
    let f0 = max(vol_footprint(t), 1.0);
    let fv = f0 * max(endon, sqrt(max(1.0 - endon * endon, 0.0)) / max(abs(dir.y), 0.05));
    let lod = clamp(log2(fv / 47.0), 0.0, 6.0);
    // bands: a smooth ridge across the wind, a few hundred metres to 2 km wide
    let f1 = textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / 200.0, vf) / 12000.0, lod).g;
    let ridge = smoothstep(0.25, 1.0, 1.0 - abs(2.0 * f1 - 1.0));
    // (0.28: the mean of ridge^2 over the equalised channel)
    let band = mix(0.28, ridge * ridge, side);
    // each band tapers in and out along its length (20-60 km)
    let along = smoothstep(0.2, 0.7, textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / 60000.0, vf / 40000.0), 3.0).g);
    // fine parallel fibres inside a band (the detail channel, 1.5 km across)
    let f2 = textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / 300.0, vf) / 1500.0 + vec2<f32>(0.5, 0.25), clamp(lod + 3.0, 0.0, 6.0)).a;
    let fibre = mix(0.65, 1.0, mix(0.5, smoothstep(0.2, 0.9, 1.0 - abs(2.0 * f2 - 1.0)), side));
    return clamp(env * band * along * fibre, 0.0, 1.0);
}

/// The second high deck (eo-sky clouds.rs `Decks::cirrus`, sky_common.wgsl `cirrus`): x its
/// height (m; 0: off, round 2's one high deck above), y its amount, the share of the sky it
/// covers (the METAR eighths / 8) x its fade. With it on, the 5.2 km sheet's opacity
/// (`cloud1.z`) comes already faded and floored from the CPU (clouds.rs `with_sky_mix`).
fn vol_ci2_on() -> bool {
    return skyu.cirrus.x > 0.0;
}

/// The cirrus deck's core optical depth (clouds.rs `CIRRUS2_TAU`).
const VOL_CI2_TAU: f32 = 0.3;

/// The second deck's sheet reads, each a tile across the wind (m) and a stretch along it: the
/// patches, the sheets, the bend and the fibres (`vol_cirrus2`). Along the wind (x) every tile is
/// a whole part of 160 km and across it (z) of 80 km, and the high deck's drift wraps at 160 km
/// with the second deck on (eo-sky clouds.rs `CIRRUS2_WRAP_M`, whose test reads these), so the
/// wrap moves no texture and the drifting deck never jumps.
const VOL_CI2_ENV_M: f32 = 80000.0;
const VOL_CI2_ENV_X: f32 = 2.0;
const VOL_CI2_BODY_M: f32 = 10000.0;
const VOL_CI2_BODY_X: f32 = 8.0;
const VOL_CI2_BEND_M: f32 = 20000.0;
const VOL_CI2_BEND_X: f32 = 4.0;
const VOL_CI2_FIBRE_M: f32 = 2000.0;
const VOL_CI2_FIBRE_X: f32 = 20.0;

/// The cirrus deck's cover field (`vol_cirrus2`): the high channel plus this much of the detail
/// channel's departure from its middle, read in the same sample, a zero-mean term that breaks a
/// patch's edge at 2.5-10 km (INFERRED: the round-2 0.3 of the edge term, now from one read).
const VOL_CI2_EDGE: f32 = 0.3;
/// The patch's soft edge: the cover rises from 0 to 1 over this much of the field above the cut
/// (INFERRED: the field moves about 0.08 a kilometre or two across a patch's edge, so a patch's
/// edge thins out over a kilometre or two, not a hard line).
const VOL_CI2_RAMP: f32 = 0.08;
/// How deep into a patch the sheets reach their full thickness: over this much of the field
/// above the cut. A small patch (a FEW day's, cut near the field's top) stays a thin wisp; only a
/// large patch thickens to dense cores (INFERRED: the dense cirrus spissatus patches come with
/// more cover; set before the comparison).
const VOL_CI2_DEEP: f32 = 0.3;

/// The cut of the cover field for an amount (eighths 0..8 across a row) at a mip (rows: mips 1 to
/// 4, the read's range): the field's quantile at `1 - eighths / 8` at that mip (eo-sky clouds.rs `cirrus_cut_table`,
/// whose test checks these against the bake and integrates the cover over the tile), so the deck
/// covers its METAR share of the plane near and far. Box-filtered mips squeeze the field toward
/// its middle: one cut for all mips covered more of the plane far away.
const VOL_CI2_CUT: array<f32, 36> = array<f32, 36>(
    1.1251, 0.8864, 0.7542, 0.6241, 0.4989, 0.3650, 0.2366, 0.1203, -0.2111,
    1.1206, 0.8799, 0.7481, 0.6183, 0.4987, 0.3685, 0.2434, 0.1302, -0.2008,
    1.1072, 0.8591, 0.7321, 0.6037, 0.4953, 0.3803, 0.2663, 0.1544, -0.1581,
    1.0577, 0.8249, 0.6902, 0.5829, 0.4921, 0.4027, 0.3074, 0.2032, -0.1028,
);

/// The cut (`VOL_CI2_CUT`) at an amount 0..1 (linear between eighths) and a mip (linear between rows).
fn vol_ci2_cut(amt: f32, lod: f32) -> f32 {
    var cut = VOL_CI2_CUT;
    let a = clamp(amt, 0.0, 1.0) * 8.0;
    let k = min(u32(a), 7u);
    let fa = a - f32(k);
    let l = clamp(lod, 1.0, 4.0) - 1.0;
    let r = min(u32(l), 2u);
    let fl = l - f32(r);
    let i0 = r * 9u + k;
    let i1 = i0 + 9u;
    return mix(mix(cut[i0], cut[i0 + 1u], fa), mix(cut[i1], cut[i1 + 1u], fa), fl);
}

/// Cirrus as the second deck: how much of the core optical depth (`VOL_CI2_TAU`) a point of
/// its plane carries, 0..1, `t` metres along the view `dir`. Four reads of the sheet, each
/// a channel histogram-equalised at bake time (clouds.rs `bake_sheet`), in the frame of the
/// high wind, toward the east (x; the deck's drift, clouds.rs `DRIFT_HIGH_MS`, 18 m/s east and
/// 2 south: the jet-level westerlies):
///
/// * where the deck is: the high channel over 80 km, drawn out 2 : 1 along the wind, its edge
///   broken by the detail channel of the same read (`VOL_CI2_EDGE`) and cut so the deck covers
///   its METAR share of the plane at the mip it is read at (`VOL_CI2_CUT`): patches of 5-25 km
///   with soft, ragged edges;
/// * the sheets inside it: the low-deck channel over 10 km drawn out 8 : 1, read a mip soft,
///   so a patch is broad thick and thin bands 1-3 km across and 10-20 km long (cirrostratus
///   and cirrus spissatus), not lines; they reach their full thickness only deep inside a
///   patch (`VOL_CI2_DEEP`), so a FEW day's small patches stay thin wisps;
/// * the fibres: the detail channel over 2 km drawn out 20 : 1 along the wind (filaments of
///   60-250 m across, 1-5 km long), which perspective draws converging on the wind's
///   vanishing point; their across-wind coordinate is bent by the marine channel over 20 km
///   across and 80 km along the wind (a slow swing of up to 1.2 km) and by the sheet itself
///   (600 m from its thin to its thick parts), so the fibres curl where they leave a thick
///   head, the fallstreak hooks of cirrus uncinus; the fibres carve up to 70 % out of the thin
///   parts and 25 % out of the thick cores, which read as smooth sheets; seen end-on along the
///   wind the bend and most of the fibres' contrast give way (perspective would squeeze the
///   bend's swing into a marbled swirl);
/// * the fibres' contrast fades as they shrink under a pixel (their mip's mean is not a
///   fibre), toward the zenith (from 30 to 64 degrees up: there the one 2 km fibre tile is seen
///   nearest and least foreshortened, and its regular spacing reads as a repeated grain, which
///   real fibres, each its own width and spacing, do not have; INFERRED, a limit of the one
///   tile, not a property of cirrus), and the whole deck toward the horizon as the round-2 deck did.
///
/// The tiles, stretches and the bend are INFERRED: sizes in the range the WMO
/// International Cloud Atlas (2017) describes for cirrus fibratus, uncinus and spissatus
/// (filaments, hooks and dense patches of the high etage), set before the comparison;
/// the photographs are the check, not the source.
fn vol_cirrus2(pxz: vec2<f32>, t: f32, dir: vec3<f32>) -> f32 {
    let amt = clamp(skyu.cirrus.y, 0.0, 1.0);
    let w = pxz + skyu.cloud2.zw;
    let u = w.x;
    let v = w.y;
    // the pixel's footprint across the wind (as `vol_cirrus`)
    let dh = dir.xz / max(length(dir.xz), 1e-4);
    let endon = abs(dh.x);
    let f0 = max(vol_footprint(t), 1.0);
    let fv = f0 * max(endon, sqrt(max(1.0 - endon * endon, 0.0)) / max(abs(dir.y), 0.05));
    let l2 = log2(fv);
    let lt = log2(1.0 / 256.0);
    // where the deck is: the cover field, cut at the amount for the mip it is read at
    let le = clamp(l2 - log2(VOL_CI2_ENV_M) - lt, 1.0, 4.0);
    let e = textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / VOL_CI2_ENV_X, v) / VOL_CI2_ENV_M, le);
    let x = e.g + VOL_CI2_EDGE * (e.a - 0.5);
    let thr = vol_ci2_cut(amt, le);
    let cov = smoothstep(thr, thr + VOL_CI2_RAMP, x) * smoothstep(0.0, 0.12, amt);
    if (cov <= 0.0) {
        return 0.0;
    }
    // how deep inside its patch the point is
    let deep = smoothstep(thr, thr + VOL_CI2_DEEP, x);
    // the sheets (a mip soft)
    let body = textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / VOL_CI2_BODY_X, v) / VOL_CI2_BODY_M + vec2<f32>(0.31, 0.62), clamp(l2 - log2(VOL_CI2_BODY_M) - lt + 1.0, 1.0, 7.0)).r;
    let core = smoothstep(0.25, 0.85, body) * deep;
    let thick = mix(0.25, 1.0, core);
    // the bend
    let bend = textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / VOL_CI2_BEND_X, v) / VOL_CI2_BEND_M + vec2<f32>(0.17, 0.83), clamp(l2 - log2(VOL_CI2_BEND_M) - lt + 1.0, 2.0, 7.0)).b - 0.5;
    // seen end-on (along the wind) perspective squeezes the along-wind swing of the bend into a
    // few pixels: it and the fibres' contrast give way there (as `vol_cirrus`'s `side`)
    let side = 1.0 - smoothstep(0.8, 0.97, endon);
    let vf = v + (bend * 2400.0 + (body - 0.5) * 600.0) * side;
    // the fibres
    let lf = clamp(l2 - log2(VOL_CI2_FIBRE_M) - lt, 0.0, 7.0);
    let fa = textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(u / VOL_CI2_FIBRE_X, vf) / VOL_CI2_FIBRE_M + vec2<f32>(0.5, 0.25), lf).a;
    let ridge = smoothstep(0.3, 0.95, 1.0 - abs(2.0 * fa - 1.0));
    // the fibres show where the sheet thins (its edges and wisps) and fade into the thick cores,
    // which read as smooth sheets; under a pixel, end-on and toward the zenith they give way
    let fc = (1.0 - smoothstep(2.5, 5.0, lf)) * mix(0.35, 1.0, side) * (1.0 - smoothstep(0.5, 0.9, dir.y));
    let fibre = 1.0 - fc * mix(0.7, 0.25, smoothstep(0.45, 0.85, body)) * (1.0 - ridge);
    return clamp(cov * thick * fibre, 0.0, 1.0);
}

fn vol_remap(v: f32, lo: f32, hi: f32) -> f32 {
    return clamp((v - lo) / max(hi - lo, 1e-4), 0.0, 1.0);
}

// the low deck's density is sky_clouds.wgsl `cl_density` (the sheet extruded and eroded here before).

fn vol_dir(ndc: vec2<f32>) -> vec3<f32> {
    let p = look.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(p.xyz / p.w);
}

@compute @workgroup_size(8, 8)
fn vol_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let od = vec2<i32>(textureDimensions(vol_out));
    let hp = vec2<i32>(gid.xy);
    if (hp.x >= od.x || hp.y >= od.y) {
        return;
    }
    let dd = vec2<i32>(textureDimensions(vol_depth));
    // the depth texel this volume texel stands for (full or half resolution)
    let sc = max(dd.x / max(od.x, 1), 1);
    // at half resolution, this frame's pixel of the 2 x 2 block (the four in turn,
    // sky_recon.wgsl `rc_offset`), traced at its own position; the reconstruction keeps
    // each pixel's samples, so the clouds resolve at full resolution over four frames.
    var offs = array<vec2<i32>, 4>(vec2<i32>(0, 0), vec2<i32>(1, 1), vec2<i32>(1, 0), vec2<i32>(0, 1));
    let off = select(vec2<i32>(0), offs[skyu.flags.y % 4u], sc == 2);
    let px = min(hp * sc + off, dd - 1);
    let d = textureLoad(vol_depth, px, 0);
    let uv = (vec2<f32>(px) + 0.5) / vec2<f32>(dd);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let dir = vol_dir(ndc);
    var dist = 1e9;
    if (d > 0.0) {
        let w = look.inv_view_proj * vec4<f32>(ndc, d, 1.0);
        dist = length(w.xyz / w.w);
        dist = max(dist, 1e-9 / d / max(dot(dir, look.cam_fwd.xyz), 1e-3)); // past the far plane the far layer's depth is 1e-9 / view depth (DEPTH_K)
    }
    let eye = look.cam_pos.xyz;
    // The air ends at the sea: the water pass does not write depth, so what
    // the depth holds there is the seabed or nothing.
    dist = sky_air_end(eye.y, dir.y, dist);
    let bits = skyu.flags.x;
    // Per-pixel white noise moved along the golden-ratio (R1) sequence each
    // frame: no fixed pattern for the resolve to lock onto.
    // moved along the R1 sequence per trace of this pixel (one frame in four),
    // not per frame, so a pixel's own successive traces are evenly spread over the step.
    // (Interleaved gradient noise here showed through as a diagonal hatching on the rims.)
    // from a blue-noise tile (clouds.rs) rather than white noise: no clumps of
    // equal offsets across neighbouring pixels (the fringe's salt and pepper), no pattern.
    let jit = cl_jitter(px, skyu.flags.y);
    // `--smooth-time`: the volume's own light direction (`beam.yzw`, eo-render sky.rs:
    // the sun's while its light is the volume's, the moon's after), not the surfaces' key.
    let vol_l = select(look.key_dir.xyz, skyu.beam.yzw, lk_lx_on(LX_BIT_SMOOTH_TIME));
    let cos_sun = dot(dir, vol_l);
    let key = skyu.sun_rad.xyz;
    // with the atmosphere tables the clouds take the sun through the
    // atmosphere at the deck's height (reddened, and kept after the ground's
    // sunset until the planet hides it) and the sky's own light for their ambient.
    let atmo_on = skyu.atmo2.x > 0.5;
    var L = vec3<f32>(0.0);
    var T = 1.0;
    var tdisc = 0.0;

    // (A) Sun shafts in the haze, inside the cascades' reach: in-scatter only
    // (the range fog already attenuates the scene toward the fog colour).
    // (stage L4, the cinematic look) the golden haze (`vol_aerial`) takes the key's in-scatter
    // over the whole path as if every point were lit; inside the cascades' reach the points in
    // the sun's shadow give their share back here (`cine_shade`, taken off the aerial's
    // in-scatter below), the haze's own density and phase: light shafts between the shadows
    // of the buildings with no second haze counted on top of the aerial's. The sky's pixels
    // (no aerial) keep the shafts below as they were: the lit air in front of the sky, the
    // glow round a low sun seen past the buildings.
    var cine_shade = vec3<f32>(0.0);
    let cine = vol_cine_on() && dist < 1e8;
    if (cine) {
        if (sky_on(bits, SKY_SHAFTS) && look.key_dir.w > 0.0 && atmo_on) {
            // inside the cascades' reach 8 steps on the cascades; past it, with the far rings
            // drawn, 8 more out to VOL_CINE_SHADE_FAR_M on the far map (steps growing as the
            // square), where the haze between the towers takes the shafts.
            let split = look.cascade_split.z;
            let reach = min(dist, split);
            let n = 8;
            let dt = reach / f32(n);
            let sig0 = vol_cine_aerosol();
            var acc = 0.0;
            for (var i = 0; i < n; i = i + 1) {
                let t = (f32(i) + jit) * dt;
                let p = eye + dir * t;
                acc = acc + sig0 * vol_cine_density_at(p.y) * (1.0 - vol_sun_visibility(p) * vol_cloud_t(p)) * dt;
            }
            let far_end = min(dist, VOL_CINE_SHADE_FAR_M);
            if (far_end > split && lk_lx_on(LX_BIT_FAR_SHADOWS) && look.lx[9].x > 0.0) {
                let span = far_end - split;
                for (var i = 0; i < n; i = i + 1) {
                    let f0 = f32(i) / f32(n);
                    let f1 = (f32(i) + 1.0) / f32(n);
                    let fm = (f32(i) + jit) / f32(n);
                    let t = split + span * fm * fm;
                    let p = eye + dir * t;
                    acc = acc + sig0 * vol_cine_density_at(p.y) * (1.0 - vol_far_visibility(p) * vol_cloud_t(p)) * span * (f1 * f1 - f0 * f0);
                }
            }
            // (the aerial's own direct term, its sunward cap included: uncapped, toward the sun the
            // shade took more than the aerial had put in and blacked out the haze in shadow)
            if (acc > 0.0) {
                cine_shade = vol_cine_direct(dir, vol_cine_sky(dir)) * (VOL_CINE_DIRECT_SHARE * vol_cine_day() * acc);
            }
        }
    } else if (sky_on(bits, SKY_SHAFTS) && look.key_dir.w > 0.0 && skyu.cloud4.z > 0.0) {
        let reach = min(dist, look.cascade_split.z);
        let n = 8;
        let dt = reach / f32(n);
        let ph = 0.7 * sky_hg(cos_sun, 0.72) + 0.3 / 12.566371;
        var acc = 0.0;
        for (var i = 0; i < n; i = i + 1) {
            let t = (f32(i) + jit) * dt;
            let p = eye + dir * t;
            let sig = skyu.cloud4.z * exp(-max(p.y, 0.0) / skyu.cloud4.w);
            acc = acc + sig * vol_sun_visibility(p) * vol_cloud_t(p) * dt;
        }
        L = L + key * ph * acc;
    }

    let above = eye.y > skyu.cloud0.y;
    // (B) The marine layer: a bank whose top rolls over the hills.
    let top_max = skyu.cloud3.y + skyu.cloud3.z;
    if (sky_on(bits, SKY_FOG) && (skyu.cloud3.x > 0.0 || skyu.cloud3.w > 0.0)) {
        var t0 = 0.0;
        var t1 = 0.0;
        if (eye.y <= top_max) {
            t1 = select(1e9, (top_max - eye.y) / max(dir.y, 1e-4), dir.y > 1e-4);
        } else if (dir.y < -1e-4) {
            t0 = (top_max - eye.y) / dir.y;
            t1 = 1e9;
        }
        t1 = min(min(t1, dist), t0 + 9000.0);
        if (t1 > t0) {
            let n = 10;
            // Denser samples near the start of the segment.
            let span = t1 - t0;
            var tprev = t0;
            for (var i = 0; i < n; i = i + 1) {
                let f = (f32(i) + jit) / f32(n);
                let t = t0 + span * f * f;
                let tn = t0 + span * ((f32(i) + 1.0) / f32(n)) * ((f32(i) + 1.0) / f32(n));
                let dt = tn - tprev;
                tprev = tn;
                let p = eye + dir * t;
                let s = textureSampleLevel(vol_sheet, vol_rep, (p.xz + skyu.cloud4.xy) / SKY_MARINE_TILE, clamp(log2(max(vol_footprint(t), 1.0) / 37.5), 0.0, 6.0));
                var top = skyu.cloud3.y + skyu.cloud3.z * (s.b * 2.0 - 1.0);
                // offshore, the bank stands over the open Pacific from ~1.2 km west of
                // Ocean Beach (waterline x ~ -10,300 m), dense from ~3.2 km out: a fog bank
                // lurking on the horizon. It began at x -6,800 before, which put the whole
                // Richmond and Sunset (and the beach cards' eye) inside fog of ~1 km
                // visibility on a clear day: the milky white sky toward the sun.
                // from ~2.7 km out, dense from ~5.7 km, and its top at 0.55 of
                // the inland bank's: a low grey band on the sea horizon that the
                // setting sun touches, where it stood ~5 deg high and hid it.
                let off = smoothstep(-13000.0, -16000.0, p.x);
                if (skyu.cloud3.x <= 0.0) {
                    top = top * 0.55;
                }
                // the clear-day offshore bank is a third as dense: a grey haze band on
                // the sea horizon, not a 25-30 px wall with a blown top and a dark shelf
                // that hid the setting sun.
                let sig0 = max(skyu.cloud3.x, 0.35 * skyu.cloud3.w * off);
                // banks and thinner gaps drifting through the fog (a 3D field at ~600 m).
                let fq = textureSampleLevel(vol_noise, vol_rep, (p + vec3<f32>(skyu.cloud4.x, 0.0, skyu.cloud4.y)) / 600.0, 1.0);
                let sig = sig0 * smoothstep(top, top - mix(70.0, 180.0, off), p.y) * (0.55 + 0.9 * s.a) * (0.35 + 1.3 * fq.r);
                if (sig > 0.0) {
                    // the key reaches a point in the bank through the
                    // bank above it (optical depth along the key from the
                    // point to the rolling top), scattered as the cloud deck
                    // scatters it (three octaves of the cloud phase): a lit
                    // top over a grey body, where every point used to take
                    // the full key through the forward lobe (the white wall
                    // at a low sun). The ambient is the sky's (the top) and
                    // the ground's (the base), not the horizon-bright fog
                    // colour.
                    let below = max(top - p.y, 0.0);
                    let tau = sig * below / max(vol_l.y, 0.035);
                    // Under a continuous deck (rain, storm) the bank takes the
                    // sky's diffuse light, not the sun through the sheet's gaps.
                    let shade = mix(1.0, skyu.cloud1.w, clamp((skyu.cloud0.z - 0.75) / 0.2, 0.0, 1.0));
                    let sun = key * vol_ms(tau, cos_sun) * 2.5 * vol_cloud_t(p) * shade;
                    let fr = clamp(1.0 - below / 250.0, 0.0, 1.0);
                    let amb = (mix(skyu.amb_base.xyz, skyu.amb_top.xyz, fr) + vol_extra_amb(p, dir)) * (0.45 + 0.55 * fr);
                    let a = 1.0 - exp(-sig * dt);
                    L = L + T * (sun + amb) * a;
                    T = T * (1.0 - a);
                }
            }
        }
    }

    // (E) rain shafts. Under a raining deck, curtains of rain hang
    // from the base to the sea in cells a few km across (the sheet's detail
    // and marine channels at 6 km), denser in the cells and thin between;
    // extinction from the rate (Marshall-Palmer: visibility ~ 2-5 km at 5-20
    // mm/h, sigma = 3.912 / V), lit by the sky under the deck. 8 samples to
    // 20 km. Only with rain on (rate over 0.5 mm/h).
    let rate = skyu.wet.z;
    if (sky_on(bits, SKY_RAIN) && rate > 0.5 && skyu.cloud0.z > 0.75) {
        let base = skyu.cloud0.x;
        var t1 = min(dist, 20000.0);
        if (dir.y > 1e-3) {
            t1 = min(t1, max(base - eye.y, 0.0) / dir.y);
        }
        if (t1 > 0.0 && eye.y < base) {
            let sig_r = 3.912 / mix(6000.0, 1800.0, clamp(rate / 20.0, 0.0, 1.0));
            let amb = mix(skyu.amb_base.xyz, skyu.amb_top.xyz, 0.5) * 0.9;
            var tp = 0.0;
            // 16 samples spaced as f^1.5 (8 as f^2: the far steps were 2-5 km long
            // and averaged the 1.5-6 km shaft cells into one even grey).
            for (var i = 0; i < 16; i = i + 1) {
                let f = (f32(i) + jit) / 16.0;
                let tt = t1 * f * sqrt(f);
                let fn1 = (f32(i) + 1.0) / 16.0;
                let tn = t1 * fn1 * sqrt(fn1);
                let dt = tn - tp;
                tp = tn;
                let q = eye + dir * tt;
                // a curtain leans with the wind (the cell read at a point drifted along the
                // wind by the height under the base, 0.35 m per m) and frays: straight down the 2D
                // cells drew straight-sided vertical bands from the base to the horizon.
                let wdir = normalize(skyu.wind.xy + vec2<f32>(1e-4, 0.0));
                let below = max(base - q.y, 0.0);
                let qs = q.xz - wdir * below * 0.35;
                let c = textureSampleLevel(vol_sheet, vol_rep, (qs + skyu.cloud2.xy * 0.5) / 6000.0, 4.0);
                // with a 1.5 km octave the shafts are curtains, not one soft gradient
                let c2 = textureSampleLevel(vol_sheet, vol_rep, (qs + skyu.cloud2.xy * 0.7) / 1500.0, 2.0);
                // streaks along the fall: the sheet read across the wind x height (300 m x 1.2 km)
                let perp = vec2<f32>(-wdir.y, wdir.x);
                let streak = textureSampleLevel(vol_sheet, vol_rep, vec2<f32>(dot(qs, perp) / 300.0, below / 1200.0 + 0.37), 1.0).a;
                // a soft edge (0.42-0.85) whose threshold rises toward the ground (the curtain
                // tapers and frays as it falls), its density broken into streaks
                let taper = 0.08 * clamp(below / max(base, 1.0), 0.0, 1.0);
                let cell = smoothstep(0.42 + taper, 0.85 + taper, c.b * 0.5 + c.a * 0.2 + c2.g * 0.3) * (0.55 + 0.45 * streak); // 0.45-0.8: curtains with hard walls
                // x 0.25 .. 1.6 with the cell (0.4 .. 1.6: overhead the veil hid the
                // storm's base entirely; between the shafts the base shows through).
                // x 0.12 .. 1.8 with the cell squared, and x 0.6 looking up past 15 deg:
                // curtains standing out of thin rain between them, the base above showing through.
                let up = mix(1.0, 0.6, smoothstep(0.1, 0.35, dir.y));
                // x 0.1 .. 2.2 (0.12 .. 1.8 before)
                let sig = sig_r * (0.1 + 2.1 * cell * cell) * up * smoothstep(base, base - 150.0, q.y); // curtains softer against the veil (0.15 + 2.2 cell: walls)
                // scud: ragged fractus 80-300 m under a storm's base, darker than
                // the deck (lit only by the sky under it), drifting with the low wind.
                let sh = (base - q.y);
                let scud_band = smoothstep(60.0, 120.0, sh) * (1.0 - smoothstep(220.0, 320.0, sh)) * smoothstep(8.0, 15.0, rate);
                var sig_s = 0.0;
                if (scud_band > 0.0) {
                    let sc_n = textureSampleLevel(vol_sheet, vol_rep, (q.xz + skyu.cloud2.xy * 1.3) / 1600.0, 1.0);
                    sig_s = 0.016 * scud_band * smoothstep(0.55, 0.78, sc_n.a * 0.6 + sc_n.r * 0.4); // 0.012 over 0.62-0.8: more of it
                }
                let sig_t = sig + sig_s;
                if (sig_t > 0.0) {
                    let a = 1.0 - exp(-sig_t * dt);
                    // a dense shaft is a darker grey-blue curtain (the light under the deck
                    // reaches its core through itself); scud darker still.
                    let shade = mix(1.0, 0.68, cell) * vec3<f32>(0.96, 1.0, 1.05);
                    L = L + T * amb * shade * mix(1.0, 0.55, sig_s / sig_t) * a;
                    T = T * (1.0 - a);
                }
            }
        }
    }

    // (C) clouds: the low deck, marched through its shell over the curved ground with the
    // density and light of sky_clouds.wgsl.
    // The ambient's three parts (per pixel): the upper sky (a cloud's top), the sky on the
    // side the viewer sees (looking toward the sun, the side away from it), the light from
    // below (the ground's bounce, the sky low round it; the city's glow and the twilight
    // toward the set sun are added per sample, by position).
    let cl_l = cl_light_dir();
    let cl_cs = dot(dir, cl_l);
    // `--cloud-depth-haze` (off by default until reviewed under the visual gate): over the
    // ground (a pixel the aerial
    // perspective below hazes) the low deck's cloud is held here and put in by the aerial
    // step, hazed by its own depth and hiding the haze behind it. Composited before the
    // aerial, a cloud 50 m off took the haze of the ground 20 km behind it and turned to a
    // blue veil, cut along the horizon where the sky's pixels (no aerial) drew it solid.
    // Not under a weather veil (`atmo3.x`: fog, drizzle, rain, storm): there the aerial's
    // Koschmieder over the path under the deck's top is the deck in front of the ground,
    // and with the march put in front of it the far hills seen from inside the fog went
    // black and grainy; those frames stay as they were.
    let cl_defer = sky_on(bits, SKY_CLOUD_DEPTH_HAZE) && atmo_on && dist < 1e8 && skyu.atmo3.x < 0.5;
    var cl_rad = vec3<f32>(0.0);
    var cl_a = 0.0;
    var cl_tm = 0.0;
    // under a heavy deck (rain, storm) no sun reaches the base or the high sheet
    let heavy = smoothstep(0.03, 0.18, skyu.cloud1.w);
    // (a cirrus day has no low deck: eo-sky clouds.rs `SkyState::Cirrus`)
    if (sky_on(bits, SKY_CLOUDS) && (skyu.cloud0.z > 0.0 || vol_cirrus_day() || (vol_ci2_on() && (skyu.cirrus.y > 0.0 || skyu.cloud1.z > 0.0)))) {
        let base = skyu.cloud0.x;
        let top = skyu.cloud0.y;
        let strat = cl_stratus();
        // Where the ray is inside the shell (heights over the curved ground).
        var t0 = -1.0;
        var t1 = -1.0;
        if (eye.y < base) {
            t0 = cl_hit(eye.y, dir, base, false);
            t1 = cl_hit(eye.y, dir, top, false);
        } else if (eye.y > top) {
            t0 = cl_hit(eye.y, dir, top, false);
            if (t0 > 0.0) {
                let tb = cl_hit(eye.y, dir, base, false);
                t1 = select(cl_hit(eye.y, dir, top, true), tb, tb > 0.0);
            }
        } else {
            t0 = 0.0;
            let tt = cl_hit(eye.y, dir, top, false);
            let tb = cl_hit(eye.y, dir, base, false);
            t1 = select(tt, min(tt, tb), tb > 0.0 && tt > 0.0);
            if (tt <= 0.0) {
                t1 = tb;
            }
        }
        if (t1 <= t0) {
            t1 = t0 + 6000.0;
        }
        // (at most 16 km of the shell along a grazing ray, from at most 60 km out: past that the
        // field has sunk into the horizon haze)
        t1 = min(min(min(t1, t0 + 16000.0), 45000.0), dist); // (past 45 km the field is >85 % haze)
        // no cumulus field unless EO_CUMULUS=1 (sky_clouds `cl_low_deck_on`)
        let march = t0 >= 0.0 && t1 > t0 && t0 < 60000.0 && cl_low_deck_on();
        let high = dist > 1e8 && dir.y > 0.0 && (skyu.cloud1.z > 0.0 || (vol_ci2_on() && skyu.cirrus.y > 0.0));
        var a_top = skyu.amb_top.xyz;
        var a_side = skyu.amb_top.xyz;
        var a_bot = skyu.amb_base.xyz;
        if (atmo_on && (march || high)) {
            // made once a frame by the cloud-shadow pass (sky_clouds `cl_shadow_main`)
            let sk = ClSky(textureLoad(cl_shadow, vec2<i32>(0, 0), 0).gba, textureLoad(cl_shadow, vec2<i32>(1, 0), 0).gba, textureLoad(cl_shadow, vec2<i32>(2, 0), 0).gba, textureLoad(cl_shadow, vec2<i32>(3, 0), 0).gba, textureLoad(cl_shadow, vec2<i32>(4, 0), 0).gba);
            a_top = mix(a_top, sk.top, skyu.atmo2.y);
            // after sunset a cloud's top sees the blue zenith, not the whole-sky average
            a_top = mix(a_top, sk.zen, 0.35 * smoothstep(0.02, -0.06, skyu.atmo.y));
            let lh = normalize(vec2<f32>(cl_l.x, cl_l.z) + vec2<f32>(1e-5, 0.0));
            let vh = normalize(dir.xz + vec2<f32>(1e-5, 0.0));
            a_side = mix(a_side, mix(sk.sun, sk.anti, 0.5 + 0.5 * dot(vh, lh)), skyu.atmo2.y);
            a_bot = mix(a_bot, skyu.amb_base.xyz * 0.4 + sk.amb * 0.5, skyu.atmo2.y);
        }
        if (march) {
            let hl = length(dir.xz);
            let sig_max = skyu.cloud1.x * skyu.cloud0.w * CL_DENSITY;
            // Steps of ~20 m near the eye growing with distance (a pixel's footprint), up to
            // 4 x the quality's count; empty space crossed 3 x as fast.
            // (a ray that meets the shell farther out takes fewer: the far field is faded into the
            // haze, and the horizon rows are the sea views' cost)
            // (x 3 from 5 km: the elevated and aerial views' main load)
            // (x 1.5 from 15 km, as far at the 4 x empty steps)
            let max_steps = select(select(i32(skyu.amb_base.w) * 4, i32(skyu.amb_base.w) * 3, t0 > 5000.0), (i32(skyu.amb_base.w) * 3) / 2, t0 > 15000.0);
            var Tc = 1.0;
            var Lc = vec3<f32>(0.0);
            var tw = 0.0;
            var twt = 0.0;
            // Coarse steps (3 x) on the shape alone through empty air; on a hit, one coarse step
            // back and fine steps with the full density until 6 come up empty (Schneider 2017).
            // The jitter shifts the whole lattice of a ray's samples by a fraction of a step.
            // past 12 km the empty air is crossed 4 x as fast (the far field's rays reach their
            // step limit otherwise); past 6 km 3 empty fine steps end a cloud, not 6
            var t = t0 + jit * select(3.0, 4.0, t0 > 12000.0) * clamp(16.0 + 0.005 * t0 + 0.004 * max(t0 - 10000.0, 0.0), 16.0, 800.0);
            var fine = 0;
            // the sun's light of the last sample, held once little of the pixel is left
            var sun_held = vec3<f32>(-1.0);
            for (var i = 0; i < max_steps; i = i + 1) {
                if (t >= t1) {
                    break;
                }
                let dt = clamp(16.0 + 0.005 * t + 0.004 * max(t - 10000.0, 0.0), 16.0, 800.0);
                let pp = eye + dir * t;
                let ht = t * hl;
                let pc = vec3<f32>(pp.x, pp.y + ht * ht / (2.0 * CL_EARTH_R), pp.z);
                let lod = clamp(log2(max(vol_footprint(t), 1.0) / 25.0), 0.0, 3.0);
                let fine_n = select(6, 3, t > 6000.0);
                if (fine == 0) {
                    let cm = select(3.0, 4.0, t > 12000.0);
                    if (cl_shape(pc, lod).x > 0.0) {
                        fine = fine_n;
                        t = max(t - cm * dt, t0);
                    } else {
                        t = t + cm * dt;
                    }
                    continue;
                }
                let s = cl_density(pc, t, lod);
                if (s.x > 0.002) {
                    fine = fine_n;
                    let sig = sig_max * s.x;
                    let e_sun = cl_sun(pc.y) * heavy;
                    var sun = vec3<f32>(0.0);
                    if (Tc < 0.3 && sun_held.x >= 0.0) {
                        // under 30 % of the pixel left: the last sample's sun light
                        sun = sun_held;
                    } else if (any(e_sun > vec3<f32>(0.0))) {
                        // past 20 km no light march: the optical depth of ~400 m of
                        // the sample's own density (the far field is faded into the haze)
                        var tl = vec2<f32>(400.0, 80.0) * s.x * sig_max;
                        if (t <= 20000.0) {
                            tl = cl_light_tau(pc, cl_l, t, lod, s.x);
                        }
                        sun = e_sun * cl_ms(tl.x, tl.y, cl_cs) * CL_SUN_GAIN;
                    }
                    sun_held = sun;
                    let amb = cl_ambient(a_top, a_side, a_bot + vol_extra_amb(pc, dir), s.y, s.x);
                    let ext = exp(-sig * dt);
                    let wgt = Tc * (1.0 - ext);
                    Lc = Lc + wgt * (sun + amb);
                    tw = tw + wgt * t;
                    twt = twt + wgt;
                    Tc = Tc * ext;
                    // past 6 km 3 % left ends the ray (1 % near)
                    if (Tc < select(0.01, 0.03, t > 6000.0)) {
                        break;
                    }
                } else {
                    fine = fine - 1;
                }
                t = t + dt;
            }
            let t_m = select(t0, tw / max(twt, 1e-6), twt > 1e-6);
            let alpha = 1.0 - Tc;
            // A continuous deck seen from under it: the CIE overcast law (brightest overhead,
            // a third of it at the horizon) under the weather's own dome, in every weather
            // (not a storm law darkest overhead: a storm or rain deck is
            // brightest overhead too, and darkens low down where the far base, the rain
            // shafts and the veil stack up).
            let cie = 0.68 * (1.0 + 2.0 * clamp(dir.y, 0.0, 1.0)) / 3.0;
            Lc = Lc * mix(1.0, max(cie, 0.3), select(strat, 0.0, above));
            // a deck seen from below is a neutral to cool grey
            if (!above) {
                let gl = dot(Lc, vec3<f32>(0.2126, 0.7152, 0.0722));
                Lc = mix(Lc, gl * vec3<f32>(0.95, 1.0, 1.07), 0.6 * strat);
            }
            // a deck's base from below: thicker cells and rolls darker, thin parts lighter.
            // the cells are the deck's own (its base shape where the ray meets the base),
            // with the sheet's 600 m detail over them; x 0.8 under rain or a storm.
            if (!above && strat > 0.0) {
                let pb = eye + dir * t0;
                let wb = pb.xz + skyu.cloud2.xy;
                let nb = textureSampleLevel(cl_base, vol_rep, vec3<f32>(wb.x, base * 0.6, wb.y) / (CL_BASE_TILE * 2.0), 0.0);
                let thick = smoothstep(0.2, 0.8, mix(nb.r, 0.625 * nb.g + 0.25 * nb.b + 0.125 * nb.a, 0.3));
                // under rain the base also rolls: bands drawn out 3 : 1 along the onshore
                // drift (x), darker where the deck above is thick; and the sheet's 600 m detail is
                // read at the footprint's mip (at mip 0 it aliases into the deck's speckle).
                let nr = textureSampleLevel(cl_base, vol_rep, vec3<f32>(wb.x, base * 0.6, wb.y * 3.0) / (CL_BASE_TILE * 2.0), 0.0);
                let roll = smoothstep(0.25, 0.75, 0.6 * nr.g + 0.4 * nr.b);
                let rain_w = smoothstep(2.0, 8.0, skyu.wet.z);
                let thick2 = mix(thick, 0.55 * thick + 0.45 * roll, rain_w);
                let b2 = textureSampleLevel(vol_sheet, vol_rep, (pb.xz + skyu.cloud2.xy) / 640.0, clamp(log2(max(vol_footprint(t0), 1.0) / 2.5), 0.0, 6.0)).a;
                let k = strat * mix(0.8, 0.15, heavy) * smoothstep(0.02, 0.12, dir.y);
                let mf = max(1.0 + k * ((1.0 - 2.0 * thick2) * 0.85 + (2.0 * b2 - 1.0) * 0.15) * 1.6, 0.2);
                // a raining deck's base is the source of the light under it (the light the
                // rain veil in front of it is drawn with, section E): 1.35 x that light overhead,
                // falling to 0.95 x it at the horizon, where the veil in front takes over, so the
                // sky darkens steadily from the zenith down. Lit by its own march it came out
                // darker than the veil, and the sky under a storm was darkest overhead.
                let lu = mix(skyu.amb_base.xyz, skyu.amb_top.xyz, 0.5) * 0.9;
                let deck = alpha * lu * (0.95 + 0.4 * clamp(dir.y, 0.0, 1.0)) * vec3<f32>(0.96, 1.0, 1.05);
                let w_rain = strat * (1.0 - heavy) * smoothstep(2.0, 8.0, skyu.wet.z);
                Lc = mix(Lc, deck, w_rain) * mf;
            }
            // scud under a raining deck: ragged dark fractus 60-350 m under its base (a
            // 3D noise at 0.8 km), lit only by the light under the deck, in front of the base.
            let rate_s = skyu.wet.z;
            if (!above && strat > 0.5 && rate_s > 4.0 && dir.y > 0.0 && eye.y < base - 350.0) {
                let sb0 = cl_hit(eye.y, dir, base - 350.0, false);
                let sb1 = min(cl_hit(eye.y, dir, base - 60.0, false), sb0 + 4000.0);
                if (sb0 > 0.0 && sb1 > sb0 && sb0 < 20000.0) {
                    // (a 1.6 km tile read a mip softer, ten samples: at 0.8 km and mip 0 the rags are
                    // thinner than the samples' spacing and stipple the deck)
                    let ds = (sb1 - sb0) / 10.0;
                    var ts = 1.0;
                    var ls = vec3<f32>(0.0);
                    let lit = a_bot * 0.55;
                    for (var i = 0; i < 10; i = i + 1) {
                        let q = eye + dir * (sb0 + (f32(i) + jit) * ds);
                        let w = q.xz + skyu.cloud2.xy;
                        let nq = textureSampleLevel(cl_base, vol_rep, vec3<f32>(w.x, q.y * 2.0, w.y) / 1600.0, 1.0);
                        let hb = clamp((base - q.y) / 350.0, 0.0, 1.0);
                        let dn = smoothstep(0.5, 0.85, 0.65 * nq.r + 0.35 * nq.g) * smoothstep(0.1, 0.3, hb) * (1.0 - smoothstep(0.7, 1.0, hb)) * smoothstep(4.0, 15.0, rate_s);
                        let e = exp(-0.02 * dn * ds);
                        ls = ls + ts * lit * (1.0 - e);
                        ts = ts * e;
                    }
                    L = L + T * ls;
                    T = T * ts;
                }
            }
            // Aerial perspective on the cloud: with the distance to its mean depth (the shallow
            // share of the path) it sinks into the sky drawn behind it, so the far field piles
            // up into the horizon haze in the haze's own colour instead of ending on a line.
            let far = exp(-t_m * (1.0 - 0.85 * abs(dir.y)) / mix(24000.0, 16000.0, strat));
            var behind = lk_sky_dome(dir);
            if (atmo_on) {
                behind = mix(behind, vol_atmo_sky(dir), skyu.atmo2.y);
            }
            if (cl_defer) {
                // held for the aerial step, which hazes it by its depth (`far` is the
                // sky pixels' stand-in for that aerial)
                cl_rad = Lc;
                cl_a = alpha;
                cl_tm = t_m;
            } else {
                Lc = Lc * far + behind * alpha * (1.0 - far);
                if (above) {
                    // Seen from above, the deck is in front of the marine layer.
                    L = Lc + (1.0 - alpha) * L;
                } else {
                    L = L + T * Lc;
                }
                T = T * (1.0 - alpha);
            }
        }
        // (D) The high deck: one sheet at its altitude, sky only; lit by the same
        // sun as the low deck (the atmosphere's transmittance at its own height, the same
        // phase and octaves), so the two layers agree and the high one keeps the sun longest.
        if (high && vol_ci2_on()) {
            // two decks, each at its own height: the 5.2 km sheet (its opacity as the CPU faded
            // and floored it), then the cirrus behind it
            if (skyu.cloud1.z > 0.0) {
                let t = cl_hit(eye.y, dir, skyu.cloud1.y, false);
                if (t > 0.0) {
                    let p = eye + dir * t;
                    // (the sheet read 2 : 1 along the high wind, east: round 2 turned it a little,
                    // which made the drift's wrap at the tile a jump; at 160 km it is a whole number of tiles)
                    let q = (p.xz + skyu.cloud2.zw) / SKY_HIGH_TILE;
                    let s = textureSampleLevel(vol_sheet, vol_rep, q * vec2<f32>(0.5, 1.0), 0.0);
                    let ci = clamp((0.3 - skyu.cloud0.z) / 0.3, 0.0, 1.0);
                    let a = vol_remap(s.g, mix(1.0 - min(skyu.cloud0.z + 0.2, 1.0), 0.72, ci), 1.0) * skyu.cloud1.z * smoothstep(0.0, 0.1, dir.y) * (0.5 + 0.5 * s.a);
                    let e_sun = cl_sun(skyu.cloud1.y) * heavy;
                    let c = e_sun * cl_ms(0.6 * a, 0.6 * a, cl_cs) * CL_SUN_GAIN * 0.7 + a_top * 0.85;
                    let fh = exp(-t * (1.0 - 0.85 * abs(dir.y)) / 60000.0);
                    var behind_h = lk_sky_dome(dir);
                    if (atmo_on) {
                        behind_h = mix(behind_h, vol_atmo_sky(dir), skyu.atmo2.y);
                    }
                    L = L + T * a * (c * fh + behind_h * 0.4 * (1.0 - fh));
                    T = T * (1.0 - 0.4 * a);
                }
            }
            if (skyu.cirrus.y > 0.0) {
                let t = cl_hit(eye.y, dir, skyu.cirrus.x, false);
                if (t > 0.0) {
                    let p = eye + dir * t;
                    let tau = vol_cirrus2(p.xz, t, dir) * VOL_CI2_TAU * smoothstep(0.0, 0.08, dir.y);
                    let a = 1.0 - exp(-tau);
                    // lit as round 2's cirrus (below): the sun through the atmosphere to the
                    // deck's height (warm at a low sun), the forward phase, half the top light
                    let e_sun = cl_sun(skyu.cirrus.x) * heavy;
                    let c = e_sun * cl_ms(0.6 * tau, 0.6 * tau, cl_cs) * CL_SUN_GAIN * 0.7 + a_top * 0.5;
                    let fh = exp(-t * (1.0 - 0.85 * abs(dir.y)) / 60000.0);
                    var behind_h = lk_sky_dome(dir);
                    if (atmo_on) {
                        behind_h = mix(behind_h, vol_atmo_sky(dir), skyu.atmo2.y);
                    }
                    L = L + T * a * (c * fh + behind_h * (1.0 - fh));
                    T = T * (1.0 - a);
                }
            }
        } else if (high) {
            // (the fair sheet stands at 5.2 km; above it, the uniform's height is its fade)
            let fair_fade = 1.0 - clamp((skyu.cloud1.y - VOL_FAIR_HIGH_M) / VOL_FAIR_FADE_M, 0.0, 1.0);
            let hh = select(min(skyu.cloud1.y, VOL_FAIR_HIGH_M), skyu.cloud1.y, vol_cirrus_day());
            let t = cl_hit(eye.y, dir, hh, false);
            if (t > 0.0 && vol_cirrus_day()) {
                // a cirrus day: fibrous ice streaks at cirrus height (`vol_cirrus`); the
                // uniform's opacity is the streak core's optical depth (clouds.rs `CIRRUS_TAU`)
                let p = eye + dir * t;
                let tau = vol_cirrus(p.xz, t, dir) * skyu.cloud1.z * smoothstep(0.0, 0.08, dir.y);
                let a = 1.0 - exp(-tau);
                let e_sun = cl_sun(skyu.cloud1.y) * heavy;
                // thin ice, single scattering: the sun through the forward-peaked phase, and the
                // sky round it. The sky at 9.5 km: about 0.29 of the air column is above it (US
                // Standard Atmosphere 1976: 29 kPa of 101 kPa), so the light from above is about
                // a third of the low deck's top light and the rest comes up from the air and sea
                // below: half `a_top` (INFERRED from that split; 0.85 left the sun's share too
                // small at a low sun, where cirrus is the most strongly coloured cloud).
                let c = e_sun * cl_ms(0.6 * tau, 0.6 * tau, cl_cs) * CL_SUN_GAIN * 0.7 + a_top * 0.5;
                let fh = exp(-t * (1.0 - 0.85 * abs(dir.y)) / 60000.0);
                var behind_h = lk_sky_dome(dir);
                if (atmo_on) {
                    behind_h = mix(behind_h, vol_atmo_sky(dir), skyu.atmo2.y);
                }
                // the same opacity hides the sky behind as the streak adds (far away, the
                // streak fades into the sky it stands in)
                L = L + T * a * (c * fh + behind_h * (1.0 - fh));
                T = T * (1.0 - a);
            } else if (t > 0.0) {
                let p = eye + dir * t;
                // cirrus: the sheet read stretched 2 : 1 along the high wind
                let q = (p.xz + skyu.cloud2.zw) / SKY_HIGH_TILE;
                let qs = vec2<f32>(0.95 * q.x + 0.31 * q.y, -0.31 * q.x + 0.95 * q.y) * vec2<f32>(0.5, 1.0);
                let s = textureSampleLevel(vol_sheet, vol_rep, qs, 0.0);
                let ci = clamp((0.3 - skyu.cloud0.z) / 0.3, 0.0, 1.0);
                let hop = max(skyu.cloud1.z, 0.10 * ci);
                let a = vol_remap(s.g, mix(1.0 - min(skyu.cloud0.z + 0.2, 1.0), 0.72, ci), 1.0) * hop * smoothstep(0.0, 0.1, dir.y) * (0.5 + 0.5 * s.a) * fair_fade;
                // thin ice: optical depth ~a across it, toward the sun about as much
                let e_sun = cl_sun(hh) * heavy;
                let c = e_sun * cl_ms(0.6 * a, 0.6 * a, cl_cs) * CL_SUN_GAIN * 0.7 + a_top * 0.85;
                let fh = exp(-t * (1.0 - 0.85 * abs(dir.y)) / 60000.0);
                var behind_h = lk_sky_dome(dir);
                if (atmo_on) {
                    behind_h = mix(behind_h, vol_atmo_sky(dir), skyu.atmo2.y);
                }
                L = L + T * a * (c * fh + behind_h * 0.4 * (1.0 - fh));
                // thin ice scatters mostly forward and hides little of the sky behind it
                T = T * (1.0 - 0.4 * a);
            }
        }
    }
    // the city's skyglow, in front of the decks (it is scattered in the
    // low air under them) and half-hidden by the marine layer; over distant
    // ground as a veil growing with the distance (~9 km to 1 - 1/e).
    // the aerial perspective on the city, the far land and the sea, in
    // front of everything in the volume (the range fog is pushed out of reach).
    if (atmo_on && dist < 1e8) {
        var ap = vol_aerial(dir, dist, eye.y);
        if (cine) {
            ap = vec4<f32>(max(ap.rgb - cine_shade, vec3<f32>(0.0)), ap.a); // (stage L4) the shade in the haze
        }
        if (cl_a > 0.0) {
            // (cl_defer) the cloud at its mean depth: the haze in front of it (the
            // aerial to that depth) over it, the haze behind it (the rest of the path's
            // in-scatter) hidden by its opacity. The cloud is in front of what the volume
            // drew before it (the marine layer) when the eye is over the deck (as in the
            // sky's pixels) or over the marine layer's top with the cloud nearer than the
            // ray's way into it: from 1 km up a cumulus 300 m off stood behind the
            // marine bank 20 km out, a bright strip across the cloud at the horizon.
            // With no cloud (cl_a 0) the frame is the one below.
            let apn = vol_aerial(dir, min(cl_tm, dist), eye.y);
            let haze = apn.rgb + (1.0 - cl_a) * max(ap.rgb - apn.rgb, vec3<f32>(0.0));
            let marine_t0 = select(0.0, (top_max - eye.y) / min(dir.y, -1e-4), eye.y > top_max && dir.y < -1e-4);
            let front = above || cl_tm < marine_t0;
            L = haze + ap.a * L * select(1.0, 1.0 - cl_a, front) + apn.a * select(T, 1.0, front) * cl_rad;
            T = T * ap.a * (1.0 - cl_a);
        } else {
            L = ap.rgb + ap.a * L;
            T = T * ap.a;
        }
    }
    // the sky itself from the atmosphere tables, behind everything in
    // the volume (sky pixels, not the sea's), sharing the sky with the JS dome
    // at deep night (`atmo2.y`), where the dome carries the airglow and the city.
    // the sun's disc is drawn by the resolve at full resolution (here, at
    // half resolution and blended over frames, it was a soft smudge); the volume
    // passes on the share of it that reaches the eye (`tdisc`, in its depth texture).
    if (atmo_on && d <= 0.0 && dist > 1e8 && skyu.atmo2.y > 0.0) {
        let w = skyu.atmo2.y;
        let a = vol_atmo_sky(dir);
        tdisc = T * w;
        L = L + T * w * a;
        T = T * (1.0 - w);
    }
    if (skyu.amb_top.w > 0.0) {
        let veil = select(1.0 - exp(-dist / 9000.0), 1.0, dist > 1e8);
        // x 0.4 over the ground and the water (the city's glow scattered in over the
        // bay drew it glowing brown, as bright as the horizon sky)
        L = L + sky_city_glow(dir, eye, skyu.amb_top.w) * veil * select(0.4, 1.0, dist > 1e8) * mix(1.0, T, 0.5);
    }
    // Temporal accumulation: this result blended (0.12) onto the last frame's
    // at the same world point (the sky by direction), where the last frame
    // saw the same depth; the march's per-frame jitter averages out.
    var res = vec4<f32>(max(L, vec3<f32>(0.0)), clamp(T, 0.0, 1.0));
    // no NaN or infinity into the history (black specks on cloud rims at a low sun).
    if (any(res != res) || any(abs(res) > vec4<f32>(1e5))) {
        res = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    // the history is the reconstruction's (sky_recon.wgsl); a texel here is a
    // different pixel of its block each frame.
    if (skyu.taa.y > 0.5 && sc == 1) {
        let wp = select(eye + dir * dist, eye + dir * 1e5, d <= 0.0);
        let pc = skyu.prev_vp * vec4<f32>(wp, 1.0);
        if (pc.w > 1e-3) {
            let puv = vec2<f32>(pc.x / pc.w * 0.5 + 0.5, 0.5 - pc.y / pc.w * 0.5);
            if (all(puv >= vec2<f32>(0.0)) && all(puv <= vec2<f32>(1.0))) {
                let pd = vec2<i32>(textureDimensions(vol_prevz));
                let hz = textureLoad(vol_prevz, clamp(vec2<i32>(puv * vec2<f32>(pd)), vec2<i32>(0), pd - 1), 0).r;
                let same = select(hz >= 0.0 && abs(hz - dist) < 0.05 * dist + 1.0, hz < 0.0, d <= 0.0);
                if (same) {
                    let h = textureSampleLevel(vol_prev, vol_lin, puv, 0.0);
                    if (all(h == h)) {
                        // 0.2 -> 0.12 (the march's grain averaged over ~16 frames); an even
                        // average over the frames since a cut until then (`taa.y`).
                        res = mix(h, res, max(0.12, 1.0 / (skyu.taa.y + 1.0)));
                    }
                }
            }
        }
    }
    textureStore(vol_out, hp, res);
    // a sky texel stores -1 - the share of the sun's disc that reaches the eye.
    textureStore(vol_zout, hp, vec4<f32>(select(-1.0 - clamp(tdisc, 0.0, 1.0), dist, dist < 1e8)));
}

// ---- noise module (sky.rs compiles what follows as its own module) ----
// The cloud detail noise (64^3, once): R Perlin-Worley (value fBm remapped by
// Worley), G/B/A inverted Worley at 3, 6 and 12 cells (environment-ibl.js
// cloud noise: 64^3, the same layout).
@group(0) @binding(0) var noise_out: texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(1) var noise_src: texture_3d<f32>;

// One mip of the noise: the mean of the 2 x 2 x 2 texels above it.
@compute @workgroup_size(4, 4, 4)
fn noise_down(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(noise_out);
    if (any(gid >= n)) {
        return;
    }
    var acc = vec4<f32>(0.0);
    for (var k = 0u; k < 8u; k = k + 1u) {
        let o = vec3<u32>(k & 1u, (k >> 1u) & 1u, (k >> 2u) & 1u);
        acc = acc + textureLoad(noise_src, vec3<i32>(gid * 2u + o), 0);
    }
    textureStore(noise_out, vec3<i32>(gid), acc / 8.0);
}

fn nz_hash3(p: vec3<i32>) -> vec3<f32> {
    let q = vec3<u32>(p) * vec3<u32>(1597334673u, 3812015801u, 2798796415u);
    let n = (q.x ^ q.y ^ q.z) * 1597334673u;
    let a = vec3<u32>(n, n * 16807u, n * 48271u);
    return vec3<f32>(a) / 4294967296.0;
}

fn nz_worley(p: vec3<f32>, cells: i32) -> f32 {
    let q = p * f32(cells);
    let i = vec3<i32>(floor(q));
    let f = fract(q);
    var best = 1e9;
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let o = vec3<i32>(x, y, z);
                let c = i + o;
                let w = ((c % cells) + cells) % cells;
                let fp = vec3<f32>(o) + nz_hash3(w) - f;
                best = min(best, dot(fp, fp));
            }
        }
    }
    return clamp(1.0 - sqrt(best), 0.0, 1.0);
}

fn nz_value(p: vec3<f32>, period: i32) -> f32 {
    let q = p * f32(period);
    let i = vec3<i32>(floor(q));
    let f = fract(q);
    let u = f * f * (3.0 - 2.0 * f);
    var acc = 0.0;
    for (var k = 0; k < 8; k = k + 1) {
        let o = vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        let c = (((i + o) % period) + period) % period;
        let w = select(1.0 - u.x, u.x, o.x == 1) * select(1.0 - u.y, u.y, o.y == 1) * select(1.0 - u.z, u.z, o.z == 1);
        acc = acc + w * nz_hash3(c + vec3<i32>(17, 31, 7)).x;
    }
    return acc;
}

@compute @workgroup_size(4, 4, 4)
fn noise_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = textureDimensions(noise_out);
    if (any(gid >= n)) {
        return;
    }
    let p = (vec3<f32>(gid) + 0.5) / vec3<f32>(n);
    var v = 0.0;
    var a = 1.0;
    var norm = 0.0;
    var per = 4;
    for (var o = 0; o < 4; o = o + 1) {
        v = v + a * nz_value(p, per);
        norm = norm + a;
        a = a * 0.52;
        per = per * 2;
    }
    v = v / norm;
    let w3 = nz_worley(p, 3);
    let w6 = nz_worley(p, 6);
    let w12 = nz_worley(p, 12);
    let wf = 0.625 * w3 + 0.25 * w6 + 0.125 * w12;
    // Perlin-Worley: the value fBm remapped by the Worley fBm.
    let pw = clamp((v - (1.0 - wf)) / max(1.0 - (1.0 - wf), 1e-3), 0.0, 1.0) * 0.5 + v * 0.5;
    textureStore(noise_out, vec3<i32>(gid), vec4<f32>(pw, w3, w6, w12));
}
