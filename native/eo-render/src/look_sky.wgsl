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
    let b = 0.12 + 0.88 * pow(sky_hash1(id * 4u + 4u), 3.4);
    let r = length(g - cell - pos);
    // ~1 px at 1080p: a cell spans ~10 px.
    let core = exp(-r * r / 0.012);
    let urban = clamp(look.tw2.z * look.tw.x / 0.5, 0.0, 1.0);
    // the city glow dims the fainter stars to 0.3, not out (nine stars over the Pacific)
    // over the city the fainter stars go out (r8 kept them at 0.3: too many stars
    // over a lit city); away from it they stay
    let limit = 1.0 - urban * (1.0 - 0.7 * smoothstep(0.84, 0.95, b));
    let t = h / 0.03;
    let tint = mix(vec3<f32>(0.78, 0.86, 1.0), vec3<f32>(1.0, 0.96, 0.84), t);
    // x 0.6 (0.35: about ten faint stars over the open sea)
    return tint * (b * limit * core * 0.6);
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
    if (dot(d, look.sun.xyz) > look.disc.w) {
        rgb = rgb + look.disc.xyz * smoothstep(0.15, 0.6, skyu.cloud1.w); // not through a rain deck
    }
    // the stars where the sky is dark enough, and the moon.
    if (skyu.stars.x > 0.0 && d.y > 0.0) {
        // washed by the city's skyglow too (the volume pass draws it over this).
        let wash = 1.0 - smoothstep(0.010, 0.030, lk_luma(rgb + sky_city_glow(d, look.cam_pos.xyz, skyu.amb_top.w)));
        rgb = rgb + sky_stars(d) * skyu.stars.x * wash * smoothstep(0.0, 0.1045, d.y);
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
