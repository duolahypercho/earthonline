// The sky, drawn inside the city pass after the opaque geometry: a
// full-screen triangle at the far plane (reverse-Z depth 0) that only passes
// where nothing was drawn. Per pixel: the analytic dome (`skyDomeRadiance`)
// and the solar disc (sky-atmosphere.js `placeSkyBodies`: 0.5334 degrees,
// 26 + 46 x daylight times the beam ratio, reddening with the beam). Not
// drawn: clouds, the aureole, stars, the moon.

@group(0) @binding(0) var<uniform> look: Look;
// the moon and the stars (sky.rs uniform).
@group(0) @binding(1) var<uniform> skyu: Sky;

// the star field (environment-ibl.js `starField`, sky-atmosphere.js star
// levels), procedural: one candidate star per cell of a 160 x 160 grid on
// each cube face of the celestial sphere (turned at the sidereal rate about
// San Francisco's pole), brightness 0.12 + 0.88 u^3.4, washed out where the
// sky is brighter than 0.010-0.030 and thinned by the city's glow to the
// brightest (`limit = 1 - urban (1 - 0.7 smoothstep(0.84, 0.95, b))`).
fn sky_stars(d: vec3<f32>) -> vec3<f32> {
    let lat = 37.7749 * LK_DEG;
    let pole = vec3<f32>(0.0, sin(lat), -cos(lat));
    let a = skyu.stars.y;
    let c = cos(a);
    let s = sin(a);
    // Rodrigues: the sky turns about the pole.
    let q = d * c + cross(pole, d) * s + pole * dot(pole, d) * (1.0 - c);
    let aq = abs(q);
    var face = 0u;
    var uv = vec2<f32>(0.0);
    if (aq.x >= aq.y && aq.x >= aq.z) {
        face = select(1u, 0u, q.x > 0.0);
        uv = q.yz / aq.x;
    } else if (aq.y >= aq.z) {
        face = select(3u, 2u, q.y > 0.0);
        uv = q.xz / aq.y;
    } else {
        face = select(5u, 4u, q.z > 0.0);
        uv = q.xy / aq.z;
    }
    let n = 160.0;
    let g = (uv * 0.5 + 0.5) * n;
    let cell = floor(g);
    let id = face * 25600u + u32(cell.y) * 160u + u32(cell.x);
    let h = sky_hash1(id * 4u + 1u);
    if (h > 0.03) {
        return vec3<f32>(0.0);
    }
    let pos = vec2<f32>(sky_hash1(id * 4u + 2u), sky_hash1(id * 4u + 3u)) * 0.7 + 0.15;
    // a power-law spread with more bright stars (pow 3.4 left nearly all of them faint)
    let b = 0.15 + 0.85 * pow(sky_hash1(id * 4u + 4u), 2.2);
    let r = length(g - cell - pos);
    // ~1 px at 1080p: a cell spans ~10 px.
    let core = exp(-r * r / 0.012);
    // how far the city glow washes this part of the sky (directional: the sky over the open sea
    // keeps its stars where the sky over the city loses the faint ones)
    let urban = clamp(look.tw2.z * look.tw.x / 0.5, 0.0, 1.0) * clamp(lk_luma(sky_city_glow(d, look.cam_pos.xyz, skyu.amb_top.w)) / 0.012, 0.0, 1.0);
    // the city glow dims the fainter stars to 0.3, not out (nine stars over the Pacific)
    // over the city the fainter stars go out (r8 kept them at 0.3: too many stars
    // over a lit city); away from it they stay
    let limit = 1.0 - urban * (1.0 - 0.7 * smoothstep(0.84, 0.95, b));
    let t = h / 0.03;
    let tint = mix(vec3<f32>(0.78, 0.86, 1.0), vec3<f32>(1.0, 0.96, 0.84), t);
    // x 0.6 (0.35: about ten faint stars over the open sea)
    return tint * (b * limit * core * 1.3);
}

// (`--urban-sky`; eo-sky `urban_sky`) the stars a city sky leaves: the
// field's candidate in this cell (as `sky_stars`), its magnitude from its
// rank (counts grow as 10^(0.5 m) to the faintest's 6), the sky's surface
// brightness here from the city's glow (its clear zenith US_ZENITH_MAG,
// brighter toward the horizon and under a brighter weather's glow by the
// glow's own shape), the naked eye's limiting magnitude for it (Schaefer
// 1990), and the stars over it drawn at the field's scale by their flux
// relative to a first-magnitude star's (faded in over US_FADE_MAG).
fn sky_stars_urban(d: vec3<f32>) -> vec3<f32> {
    let lat = 37.7749 * LK_DEG;
    let pole = vec3<f32>(0.0, sin(lat), -cos(lat));
    let a = skyu.stars.y;
    let c = cos(a);
    let s = sin(a);
    let q = d * c + cross(pole, d) * s + pole * dot(pole, d) * (1.0 - c);
    let aq = abs(q);
    var face = 0u;
    var uv = vec2<f32>(0.0);
    if (aq.x >= aq.y && aq.x >= aq.z) {
        face = select(1u, 0u, q.x > 0.0);
        uv = q.yz / aq.x;
    } else if (aq.y >= aq.z) {
        face = select(3u, 2u, q.y > 0.0);
        uv = q.xz / aq.y;
    } else {
        face = select(5u, 4u, q.z > 0.0);
        uv = q.xy / aq.z;
    }
    let n = 160.0;
    let g = (uv * 0.5 + 0.5) * n;
    let cell = floor(g);
    let id = face * 25600u + u32(cell.y) * 160u + u32(cell.x);
    let h = sky_hash1(id * 4u + 1u);
    if (h > 0.03) {
        return vec3<f32>(0.0);
    }
    let t = h / 0.03;
    let m = max(US_FAINTEST_MAG + log2(max(t, 1e-9)) * 0.30103 / US_COUNT_SLOPE, US_BRIGHTEST_MAG);
    // the city's glow here over its clear zenith (the weather's glow over clear's: the uniform's
    // `amb_top.w` over the practicals' gain `look.fog.z`, 1 without `--night-photometry`).
    let g_rel = skyu.amb_top.w / max(look.fog.z, 1.0);
    let zen = lk_luma(sky_city_glow(vec3<f32>(0.0, 1.0, 0.0), look.cam_pos.xyz, 1.0));
    let here = lk_luma(sky_city_glow(d, look.cam_pos.xyz, 1.0)) * max(g_rel, 0.02);
    let b = US_ZENITH_MAG - 2.5 * log2(max(here / max(zen, 1e-9), 1e-6)) * 0.30103;
    let nelm = 7.93 - 5.0 * log2(exp2((4.316 - b / 5.0) * 3.321928) + 1.0) * 0.30103;
    let vis = smoothstep(0.0, 1.0, (nelm - m) / US_FADE_MAG);
    if (vis <= 0.0) {
        return vec3<f32>(0.0);
    }
    let bright = clamp(exp2(-0.4 * (m - US_FULL_MAG) * 3.321928), US_FAINTEST_DRAWN, 1.0) * vis;
    let pos = vec2<f32>(sky_hash1(id * 4u + 2u), sky_hash1(id * 4u + 3u)) * 0.7 + 0.15;
    let r = length(g - cell - pos);
    let core = exp(-r * r / 0.012);
    let tint = mix(vec3<f32>(0.78, 0.86, 1.0), vec3<f32>(1.0, 0.96, 0.84), sky_hash1(id * 4u + 4u));
    return tint * (bright * core * 1.3);
}

// the moon's disc (0.5181 deg), lit by the sun across it (the phase),
// earthshine on the dark limb (0.022 x the unlit share), and a faint aureole.
fn sky_moon(d: vec3<f32>) -> vec3<f32> {
    let m = skyu.moon.xyz;
    let level = skyu.moon.w;
    let cd = dot(d, m);
    let rad = 0.5181 * 0.5 * LK_DEG;
    let col = vec3<f32>(0.94, 0.95, 1.0);
    // the aureole held to ~2 % of the disc within half a degree (it
    // was 1/(1 - cos) to 2500 x: with the bloom a clipped lamp several discs wide).
    var out = col * level * 0.00002 / max(1.0 - cd, 4e-3) * smoothstep(0.995, 1.0, cd);
    if (cd > cos(rad)) {
        // The point on the lunar sphere this ray meets (orthographic: the disc is tiny).
        let right = normalize(cross(m, vec3<f32>(0.0, 1.0, 0.0)) + vec3<f32>(1e-5, 0.0, 0.0));
        let up = cross(right, m);
        let x = dot(d - m * cd, right) / sin(rad);
        let y = dot(d - m * cd, up) / sin(rad);
        let z = sqrt(max(1.0 - x * x - y * y, 0.0));
        let n = right * x + up * y - m * z;
        // The sun as seen from the moon is the sun as seen from here.
        let lit = smoothstep(-0.05, 0.08, dot(n, look.sun.xyz));
        let mare = 0.85 + 0.15 * sin(x * 7.0 + 1.3) * sin(y * 5.0 - 0.4);
        let edge = smoothstep(1.0, 0.92, x * x + y * y);
        out = out + col * level * mare * edge * mix(0.022 * (1.0 - skyu.stars.z), 1.0, lit);
    }
    return out * smoothstep(-0.02, 0.01, d.y);
}

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn sky_vs(@builtin(vertex_index) vi: u32) -> SkyOut {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    var out: SkyOut;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

fn sky_color(ndc: vec2<f32>) -> vec4<f32> {
    let p = look.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let d = normalize(p.xyz / p.w);
    var rgb = lk_sky_dome(d);
    // A clear night sky away from the city glow is a deep blue-black, not a neutral grey: the dark
    // dome takes a blue cast with the night (the glow added over it keeps its warm colour).
    let night_dome_w = smoothstep(0.85, 1.0, skyu.night.z) * clamp(1.0 - skyu.cloud0.z, 0.0, 1.0);
    rgb = rgb * mix(vec3<f32>(1.0), vec3<f32>(0.78, 0.9, 1.35), night_dome_w);
    if (SF_AAA_AERIAL) {
        // Photographic deep midnight indigo-navy SF night sky (`06a`, `06b`) with smooth blue-hour
        // transition (`10_neighborhood_dusk_houses`) so twilight clouds stay cohesive and night reads
        // authentic deep navy `#06112d` instead of muddy brown-grey or neon royal blue.
        let deep_n = smoothstep(0.84, 1.0, skyu.night.z) * (1.0 - smoothstep(-0.15, -0.07, look.sun.y));
        let dusk_n = smoothstep(0.45, 0.88, skyu.night.z) * (1.0 - smoothstep(-0.08, 0.01, look.sun.y)) * (1.0 - deep_n);
        let clear_w = 1.0 - clamp(look.wx.x, 0.0, 1.0) * 0.75;
        if (deep_n > 0.001) {
            let nl_d = max(lk_luma(rgb), 0.0010);
            let ind_d = mix(vec3<f32>(0.38, 0.52, 1.18), vec3<f32>(0.22, 0.42, 1.38), smoothstep(0.02, 0.48, max(d.y, 0.0)));
            let ind_n = ind_d / max(lk_luma(ind_d), 1e-4);
            rgb = mix(rgb, ind_n * (nl_d * 0.86), deep_n * clear_w * 0.86);
        } else if (dusk_n > 0.001) {
            // Subtle blue-hour twilight slate-indigo cleanup without over-brightening the sky dome behind clouds.
            let nl_tw = lk_luma(rgb);
            let tw_t = mix(vec3<f32>(0.72, 0.78, 1.08), vec3<f32>(0.42, 0.56, 1.24), smoothstep(0.08, 0.55, max(d.y, 0.0)));
            let tw_n = tw_t / max(lk_luma(tw_t), 1e-4);
            rgb = mix(rgb, tw_n * (nl_tw * 0.84), dusk_n * clear_w * 0.34 * smoothstep(0.06, 0.35, max(d.y, 0.0)));
        }
    }
    if (dot(d, look.sun.xyz) > look.disc.w) {
        rgb = rgb + look.disc.xyz * smoothstep(0.15, 0.6, skyu.cloud1.w); // not through a rain deck
    }
    // the stars where the sky is dark enough, and the moon.
    if (skyu.stars.x > 0.0 && d.y > 0.0) {
        // washed by the city's skyglow too (the volume pass draws it over this).
        let bg = rgb + sky_city_glow(d, look.cam_pos.xyz, skyu.amb_top.w);
        if (SF_URBAN_SKY && sky_on(skyu.flags.x, SKY_URBAN_SKY)) {
            // (`--urban-sky`) the city's glow sets which stars show (its limiting magnitude); the
            // dome's own twilight still washes them out.
            let wash = 1.0 - smoothstep(0.010, 0.030, lk_luma(rgb));
            rgb = rgb + sky_stars_urban(d) * skyu.stars.x * wash * smoothstep(0.0, 0.1045, d.y);
        } else {
            let wash = 1.0 - smoothstep(0.010, 0.030, lk_luma(bg));
            rgb = rgb + sky_stars(d) * skyu.stars.x * wash * smoothstep(0.0, 0.1045, d.y);
        }
    }
    if (skyu.moon.w > 0.0) {
        // a moon in a bright sky is a pale disc (its contrast with
        // the daylight or twilight sky is low), not a lamp: faded as the sky
        // behind it brightens.
        // and not where the atmosphere tables draw the sky over this pass: the
        // hidden moon still fed the bloom (a second sun with a streak at noon).
        rgb = rgb + sky_moon(d) * (1.0 - 0.9 * smoothstep(0.01, 0.12, lk_luma(rgb))) * (1.0 - skyu.atmo2.x * skyu.atmo2.y);
    }
    return vec4<f32>(rgb, 1.0);
}

@fragment
fn sky_fs(in: SkyOut) -> @location(0) vec4<f32> {
    return sky_color(in.ndc);
}

struct SkyIds {
    @location(0) color: vec4<f32>,
    @location(1) ids: vec2<u32>,
};

@fragment
fn sky_fs_ids(in: SkyOut) -> SkyIds {
    var out: SkyIds;
    out.color = sky_color(in.ndc);
    out.ids = vec2<u32>(4294967295u, 4294967295u);
    return out;
}
