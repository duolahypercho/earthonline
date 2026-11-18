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

fn vol_atmo_sky(d: vec3<f32>) -> vec3<f32> {
    let uv = at_view_uv(d, skyu.atmo.xyz);
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
    var c = textureSampleLevel(vol_at_view, vol_lin, uv, 0.0).rgb * g_look;
    // Only the excess over the sky 40 deg round in azimuth at the same elevation is scaled (a
    // straight gain switch drew a dark ring round the sun): the glare never falls below the
    // sky beside it.
    if (near_sun > 0.0) {
        let sh = normalize(skyu.atmo.xz + vec2<f32>(1e-5, 0.0));
        let ca = 0.766;
        let sa = 0.643;
        let r2 = vec2<f32>(ca * sh.x - sa * sh.y, sa * sh.x + ca * sh.y);
        let hd = sqrt(max(1.0 - d.y * d.y, 0.0));
        let dref = vec3<f32>(r2.x * hd, d.y, r2.y * hd);
        let lref = dot(textureSampleLevel(vol_at_view, vol_lin, at_view_uv(dref, skyu.atmo.xyz), 0.0).rgb * g_look, vec3<f32>(0.2126, 0.7152, 0.0722));
        let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        if (l > lref) {
            let k = mix(1.0, min(g_model / max(g_look, 1e-9), 1.0), near_sun);
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
        let l4 = dot(textureSampleLevel(vol_at_view, vol_lin, at_view_uv(d4, skyu.atmo.xyz), 0.0).rgb * g_look, vec3<f32>(0.2126, 0.7152, 0.0722));
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

/// aerial perspective (transmittance, in-scattered radiance) over a
/// view distance: Rayleigh at the ground (5.8, 13.6, 33.1 e-3 / km) and clear
/// coastal aerosol (`atmo3.y`) plus the weather's own (`atmo3.x`, from its
/// visibility), both thinning with height as the JS aerial path density; the
/// light scattered in is the sky toward the horizon in that azimuth (the
/// tables by day, the weather's or the night's dome as they take over), so a
/// far hill fades to the sky behind it, warm toward a low sun, blue-grey away
/// from it. Replaces the range fog (45 % at 4.2 km on a clear noon).
fn vol_aerial(dir: vec3<f32>, dist: f32, eye_y: f32) -> vec4<f32> {
    let dh = dir.y * dist;
    let dens = sky_path_density(dh);
    let ray = AT_RAY * 0.001 * exp(-max(eye_y + 0.5 * dh, 0.0) / 8000.0);
    // the weather's own extinction lives under its deck or marine layer: none for an
    // eye 200 m over the low deck's top (the fog sea seen from above went hazy blue)
    // 1 (the eye's height no longer switches it off): the in-layer share of the path
    // (`inside`, r11) already keeps the veil to the layer, and with the veil off for an eye
    // 200 m over the fog top the houses under it ghosted through the fog sea seen from above.
    let wk = 1.0;
    let above_top = smoothstep(skyu.cloud0.y, skyu.cloud0.y + 100.0, eye_y);
    let tau = (ray + vec3<f32>(skyu.atmo3.y * dens)) * dist;
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
    let cos_sun = dot(dir, look.key_dir.xyz);
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
    if (sky_on(bits, SKY_SHAFTS) && look.key_dir.w > 0.0 && skyu.cloud4.z > 0.0) {
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
                    let tau = sig * below / max(look.key_dir.y, 0.035);
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
    // under a heavy deck (rain, storm) no sun reaches the base or the high sheet
    let heavy = smoothstep(0.03, 0.18, skyu.cloud1.w);
    if (sky_on(bits, SKY_CLOUDS) && skyu.cloud0.z > 0.0) {
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
        let high = dist > 1e8 && dir.y > 0.0 && skyu.cloud1.z > 0.0;
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
            Lc = Lc * far + behind * alpha * (1.0 - far);
            if (above) {
                // Seen from above, the deck is in front of the marine layer.
                L = Lc + (1.0 - alpha) * L;
            } else {
                L = L + T * Lc;
            }
            T = T * (1.0 - alpha);
        }
        // (D) The high deck: one sheet at its altitude, sky only; lit by the same
        // sun as the low deck (the atmosphere's transmittance at its own height, the same
        // phase and octaves), so the two layers agree and the high one keeps the sun longest.
        if (high) {
            let t = cl_hit(eye.y, dir, skyu.cloud1.y, false);
            if (t > 0.0) {
                let p = eye + dir * t;
                // cirrus: the sheet read stretched 2 : 1 along the high wind
                let q = (p.xz + skyu.cloud2.zw) / SKY_HIGH_TILE;
                let qs = vec2<f32>(0.95 * q.x + 0.31 * q.y, -0.31 * q.x + 0.95 * q.y) * vec2<f32>(0.5, 1.0);
                let s = textureSampleLevel(vol_sheet, vol_rep, qs, 0.0);
                let ci = clamp((0.3 - skyu.cloud0.z) / 0.3, 0.0, 1.0);
                let hop = max(skyu.cloud1.z, 0.10 * ci);
                let a = vol_remap(s.g, mix(1.0 - min(skyu.cloud0.z + 0.2, 1.0), 0.72, ci), 1.0) * hop * smoothstep(0.0, 0.1, dir.y) * (0.5 + 0.5 * s.a);
                // thin ice: optical depth ~a across it, toward the sun about as much
                let e_sun = cl_sun(skyu.cloud1.y) * heavy;
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
        let ap = vol_aerial(dir, dist, eye.y);
        L = ap.rgb + ap.a * L;
        T = T * ap.a;
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
