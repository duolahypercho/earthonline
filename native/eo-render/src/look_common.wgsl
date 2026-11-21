// The look's per-frame uniform and the analytic sky, shared by the city
// shading (look_light.wgsl), the sky pass (look_sky.wgsl) and the post pass
// (look_post.wgsl). The sky functions are a line-for-line WGSL copy of
// eo-look's `sky.rs` (itself a port of src/render/environment-ibl.js
// `makeSkyField` / `sampleSkyRadiance` / `skyDomeRadiance` with the
// coastal-heat dome grade); every sun-only term is computed on the CPU
// (eo-look) and arrives in `look`.

struct Look {
    // xyz toward the key light, w shadow strength (0..1)
    key_dir: vec4<f32>,
    // rgb key radiance (colour x intensity), w exposure
    key: vec4<f32>,
    rim_dir: vec4<f32>,
    rim: vec4<f32>,
    hemi_sky: vec4<f32>,
    hemi_ground: vec4<f32>,
    // rgb ambient irradiance, w scene.environmentIntensity
    ambient: vec4<f32>,
    // envMapIntensity: facade-painted, facade-glass, asphalt, sidewalk/ground
    env_class: vec4<f32>,
    // cosine-convolved SH9 of the environment (irradiance)
    sh: array<vec4<f32>, 9>,
    // x near, y far (three's range fog), z,w unused
    fog: vec4<f32>,
    fog_color: vec4<f32>,
    cam_pos: vec4<f32>,
    // xyz camera forward (view depth axis)
    cam_fwd: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
    cascade_vp: array<mat4x4<f32>, 3>,
    // xyz split far (view depth), w shadow fade start
    cascade_split: vec4<f32>,
    // per cascade: x normal bias (m), y depth bias (depth units), z texel (m), w max distance
    cascade_bias: array<vec4<f32>, 3>,
    // --- sky: xyz toward the sun, w sunE
    sun: vec4<f32>,
    beta_r: vec4<f32>,   // w horizon_up
    beta_m: vec4<f32>,   // w mie g
    beta_sum: vec4<f32>, // w grade
    mean: vec4<f32>,     // w twilight
    tw: vec4<f32>,       // night, ozone, belt, shadow_top_sin
    tw2: vec4<f32>,      // deep twilight, airglow, urban glow, overcast
    horizon: vec4<f32>,  // horizon radiance, w turbidity
    ground: vec4<f32>,   // ground radiance, w dome grade on (1/0)
    dg0: vec4<f32>,      // weight, veil luminance, zenith desat, horizon desat
    dg1: vec4<f32>,      // horizon lift, dome gain, skyline share, low-sun share
    dg2: vec4<f32>,      // sunward horizon desat, gradient power, skyline band width, skyline band peak
    haze_tint: vec4<f32>,
    veil_tint: vec4<f32>,
    // rgb disc radiance, w cos(disc radius)
    disc: vec4<f32>,
    // post: shoulder start, shoulder scale, saturation, contrast
    post0: vec4<f32>,
    // brightness, finish balance, vibrance, vignette
    post1: vec4<f32>,
    // density, dither amplitude (display), output is sRGB-encoded by the target (1/0), frame index
    post2: vec4<f32>,
    shadow_tint: vec4<f32>,
    highlight_tint: vec4<f32>,
    // the camera the occlusion/bounce result was made with
    // (view-projection, eye, forward), and x = result usable (1/0),
    // y = 1 / focal length in full-resolution pixels.
    ao_vp: mat4x4<f32>,
    ao_cam: vec4<f32>,
    ao_fwd: vec4<f32>,
    ao_params: vec4<f32>,
    // the street field's night practicals: x lamp pool level, y shop spill level.
    field: vec4<f32>,
    // the sky model's zenith radiance (with `horizon`: the JS skyDomeRadiance).
    dome_zenith: vec4<f32>,
    // the room drive (rooms.rs): (dusk, occupancy, shop occupancy, cool share), (office share, pixel angle).
    rooms_env: vec4<f32>,
    rooms_work: vec4<f32>,
    // the weather's dome (eo-look `upperFromRow`): overcast, isotropy, desaturation, directional horizon band (1/0); tint.
    wx: vec4<f32>,
    wx_tint: vec4<f32>,
    enclosure_frame: mat4x4<f32>,
};

// wet-surface-reflection.js `skyDomeRadiance`: the sky model's horizon
// and zenith radiance mixed on the direction's elevation, clamped at the
// horizon (a downward ray reads the horizon colour), exponent
// SKY_DOME_ELEVATION_EXPONENT 0.62. The glazing's open-sky test and its
// own-ray sky term read this dome, not the visible one.
fn lk_js_dome(d: vec3<f32>) -> vec3<f32> {
    let e = clamp(d.y, 0.0, 1.0);
    return mix(look.horizon.xyz, look.dome_zenith.xyz, pow(e, 0.62));
}

const LK_PI: f32 = 3.14159265;
const LK_DEG: f32 = 0.0174532925;

fn lk_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn lk_smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Preetham extinction for a view elevation (`preethamRow`).
fn lk_fex(dy: f32) -> vec3<f32> {
    let up = max(dy, 0.0);
    let za = acos(clamp(up, 0.0, 1.0));
    let inv = 1.0 / (cos(za) + 0.15 * pow(93.885 - za / LK_DEG, -1.253));
    let sr = 8.4e3 * inv;
    let sm = 1.25e3 * inv;
    return exp(-(look.beta_r.xyz * sr + look.beta_m.xyz * sm));
}

// The upper hemisphere for one direction (`upperFromRow`), clear weather
// (overcast, isotropy and desaturation are 0 in the clear profile).
fn lk_sky_upper(d: vec3<f32>) -> vec3<f32> {
    let dy = d.y;
    let cos_t = clamp(dot(d, look.sun.xyz), -1.0, 1.0);
    let fex = lk_fex(dy);
    let r_cos = cos_t * 0.5 + 0.5;
    let r_phase = 0.05968310365946075 * (1.0 + r_cos * r_cos);
    let g = look.beta_m.w;
    let g2 = g * g;
    let q = max(1.0 - 2.0 * g * cos_t + g2, 1e-6);
    let m_phase = 0.07957747154594767 * ((1.0 - g2) / (q * sqrt(q)));
    let beta_theta = look.beta_r.xyz * r_phase + look.beta_m.xyz * m_phase;
    let ratio = beta_theta / look.beta_sum.xyz;
    let sun_e = look.sun.w;
    let in_scatter = max(sun_e * ratio * (vec3<f32>(1.0) - fex), vec3<f32>(0.0));
    var lin = in_scatter * sqrt(in_scatter);
    let horizon_term = sqrt(max(sun_e * ratio * fex, vec3<f32>(0.0)));
    lin = lin * mix(vec3<f32>(1.0), horizon_term, look.beta_r.w);
    // the weather's dome, as eo-look `upper_from_row`: toward the CIE
    // overcast gradient (1.5 x the mean, (1 + 2 sin el) / 3), toward the mean
    // by isotropy, desaturated, x the grade and the tint (identity in clear).
    var base = (lin + 0.1 * fex) * 0.04 + vec3<f32>(0.0, 0.0003, 0.00075);
    let cie = (1.0 + 2.0 * max(dy, 0.0)) / 3.0;
    base = mix(base, look.mean.xyz * 1.5 * cie, look.wx.x);
    base = mix(base, look.mean.xyz, look.wx.y);
    base = mix(base, vec3<f32>(lk_luma(base)), clamp(look.wx.z, 0.0, 1.0));
    var out = base * look.beta_sum.w * select(look.wx_tint.xyz, vec3<f32>(1.0), look.wx_tint.x <= 0.0);
    // Afterglow.
    let s = clamp(abs(dy), 0.0, 1.0);
    let hw = (1.0 - s) * (1.0 - s) * (1.0 - s);
    let twilight = look.mean.w;
    if (twilight > 0.0 && cos_t > 0.0) {
        out = out + twilight * hw * cos_t * cos_t * vec3<f32>(0.160, 0.075, 0.055);
    }
    let night = look.tw.x;
    if (night > 0.0) {
        let glow = night * look.tw2.z;
        let hw3 = hw * hw * hw;
        out = out + glow * vec3<f32>(0.0090 + 0.0520 * hw3, 0.0104 + 0.0328 * hw3, 0.0176 + 0.0176 * hw3);
        let k2 = 0.9732383; // (6371 / (6371 + 87))^2
        let air = look.tw2.y * min(4.0, 1.0 / sqrt(max(1.0 - k2 * (1.0 - s * s), 1e-6)));
        out = out + air * vec3<f32>(0.00107, 0.00161, 0.00129);
    }
    let ozone = look.tw.y;
    let belt = look.tw.z;
    let deep = look.tw2.x;
    if (ozone > 0.0 || belt > 0.0 || deep > 0.0) {
        let top = look.tw.w;
        let lit = lk_smooth(top - 0.052, top + 0.052, s);
        let sunward = max(cos_t, 0.0);
        let antisun = max(-cos_t, 0.0);
        let a2 = antisun * antisun;
        let oz = ozone * (0.55 + 0.45 * (1.0 - s) * (1.0 - s));
        if (oz > 0.0) {
            out = out + oz * mix(1.0, lit, a2) * (0.55 + 0.45 * sunward) * vec3<f32>(0.0075, 0.0225, 0.0640);
        }
        let band = lk_smooth(0.0, 0.139173, s) * (1.0 - lk_smooth(0.139173, 0.438371, s));
        let br = belt * band;
        if (br > 0.0) {
            out = out + br * lit * a2 * vec3<f32>(0.0780, 0.0345, 0.0300);
        }
        let dr = deep * mix(0.22, 1.0, hw);
        if (dr > 0.0) {
            out = out + dr * sunward * sunward * vec3<f32>(0.0520, 0.0370, 0.0420);
        }
    }
    return out;
}

// the lower hemisphere, a street (sky.rs `ground_field`, a port of
// environment-ibl.js `makeGroundField`; GF_* constants generated by look.rs).
fn lk_band(s: f32, a: f32, b: f32, c: f32) -> f32 {
    return lk_smooth(a, b, s) * (1.0 - lk_smooth(b, c, s));
}

fn lk_ground_field(d: vec3<f32>) -> vec3<f32> {
    let s = clamp(-d.y, 0.0, 1.0);
    let pavement = lk_smooth(GF_S_KERB * 0.8, GF_S_KERB * 1.2, s);
    let far = 1.0 - lk_smooth(GF_S_FAR_WALK * 0.75, GF_S_FAR_KERB * 1.35, s);
    let carriage = max(1.0 - pavement - far, 0.0);
    let grazing = 1.0 - s;
    let clearing = 1.0 - GF_CLUTTER * (1.0 - lk_smooth(0.0, GF_CLUTTER_SIN, s));
    let g2 = grazing * grazing;
    let spec = GF_SPEC_MAX * g2 * g2 * grazing * clearing;
    let r = length(d.xz);
    var axis_dot = 0.0;
    if (r > 1e-6) {
        axis_dot = clamp(dot(d.xz, GF_AXIS) / r, -1.0, 1.0);
    }
    let two = 2.0 * axis_dot * axis_dot - 1.0;
    let four = 2.0 * two * two - 1.0;
    let open = clamp(0.5 + 0.5 * mix(two, four, GF_CROSS_SHARE), 0.0, 1.0);
    let far_open = far * open;
    let far_wall = far - far_open;
    let road = (carriage + far_open) * GF_ROAD_RATIO;
    let wall = far_wall * GF_WALL_RATIO;
    let sheen = spec * (GF_WALLED_SHARE + (1.0 - GF_WALLED_SHARE) * open);
    let mirror = lk_sky_upper(vec3<f32>(d.x, -d.y, d.z));
    let diffuse = look.ground.xyz * (pavement + road * GF_ROAD_TINT + wall * GF_WALL_TINT);
    let reflected = mix(look.horizon.xyz, mirror, GF_SHARPNESS);
    var out = diffuse * (1.0 - sheen) + reflected * sheen;
    if (look.field.x > 0.0) {
        out = out + look.field.x * lk_band(s, 0.030, 0.132, 0.40) * (0.45 + 0.55 * open) * GF_POOL_COLOR;
    }
    if (look.field.y > 0.0) {
        out = out + look.field.y * lk_band(s, 0.16, 0.38, 0.80) * pow(1.0 - open, 1.5) * GF_SPILL_COLOR;
    }
    return out;
}

// `sampleSkyRadiance`: the dome, and below the horizon the street field.
fn lk_sky_sample(d: vec3<f32>) -> vec3<f32> {
    let sky = lk_sky_upper(d);
    if (d.y >= 0.0) {
        return sky;
    }
    let t = lk_smooth(0.0, 0.0157074, -d.y);
    return mix(sky, lk_ground_field(d), t);
}

fn lk_grade_dome(rgb: vec3<f32>, to_horizon: f32, horizon_desat: f32) -> vec3<f32> {
    return lk_grade_dome_veil(rgb, to_horizon, horizon_desat, to_horizon);
}

// `lk_grade_dome` with the veil's own horizon weight (eo-look `grade_dome_veil`).
fn lk_grade_dome_veil(rgb: vec3<f32>, to_horizon: f32, horizon_desat: f32, veil_to_horizon: f32) -> vec3<f32> {
    let w = look.dg0.x;
    let des = mix(look.dg0.z, horizon_desat, to_horizon) * w;
    let lift = 1.0 + look.dg1.x * to_horizon * w;
    let y = lk_luma(rgb);
    let tint = look.haze_tint.xyz / lk_luma(look.haze_tint.xyz);
    let grey = y * mix(vec3<f32>(1.0), tint, to_horizon);
    let veil = look.dg0.y * (1.0 - veil_to_horizon);
    let vt = look.veil_tint.xyz / lk_luma(look.veil_tint.xyz);
    return (mix(rgb, grey, des) * lift + veil * vt) * look.dg1.y;
}

// eo-look `aureole_factor`: x (angle / 30 deg)^2 within 30 deg of the sun, at least 0.06.
fn lk_aureole(d: vec3<f32>) -> f32 {
    let a = acos(clamp(dot(d, look.sun.xyz), -1.0, 1.0)) / (30.0 * LK_DEG);
    let f = select(max(a * a, 0.06), 1.0, a >= 1.0);
    return mix(1.0, f, lk_smooth(-0.0175, 0.035, look.sun.y));
}

// eo-look `afterglow_arch`: the twilight arch toward the set sun.
fn lk_afterglow(d: vec3<f32>, strength: f32) -> vec3<f32> {
    let dh = length(d.xz);
    let sh = length(look.sun.xz);
    if (d.y < -0.02 || dh < 1e-6 || sh < 1e-6) {
        return vec3<f32>(0.0);
    }
    let c = max(dot(d.xz, look.sun.xz) / (dh * sh), 0.0);
    let el = max(d.y, 0.0);
    let w = c * c * c * exp(-el / 0.13);
    let t = lk_smooth(0.0, 0.16, el);
    return strength * w * mix(vec3<f32>(2.05, 0.92, 0.36), vec3<f32>(1.20, 1.02, 0.66), t);
}

// `skyDomeRadiance`: what the sky pass paints.
fn lk_sky_dome(dir: vec3<f32>) -> vec3<f32> {
    let d = normalize(dir);
    var base = lk_sky_sample(d);
    var haze = look.horizon.xyz;
    // eo-look `SkyExt` (`wx.w` on, `wx_tint.w` afterglow): the band toward
    // this azimuth's own low sky 2 deg up, the veil kept to the horizon, the
    // aureole narrowed, and the twilight arch toward the set sun.
    let ext = look.wx.w > 0.5;
    if (ext) {
        base = base * lk_aureole(d);
        let hh = length(d.xz);
        if (hh > 1e-6) {
            let bd = normalize(vec3<f32>(d.x / hh, 0.0349, d.z / hh));
            haze = lk_sky_sample(bd) * lk_aureole(bd);
        }
    }
    var skyline = 0.0;
    if (look.ground.w > 0.5) {
        let to_horizon = pow(1.0 - clamp(d.y, 0.0, 1.0), look.dg2.y);
        var hd = look.dg0.w;
        let sun_h = length(look.sun.xz);
        let view_h = length(d.xz);
        if (look.dg1.w > 0.0 && sun_h > 1e-6 && view_h > 1e-6) {
            let gap = max(dot(d.xz, look.sun.xz) / (sun_h * view_h), 0.0);
            hd = mix(look.dg0.w, look.dg2.x, gap * gap * look.dg1.w);
        }
        base = lk_grade_dome_veil(base, to_horizon, hd, select(to_horizon, 0.0, ext));
        haze = lk_grade_dome_veil(haze, 1.0, hd, select(1.0, 0.0, ext));
        skyline = look.dg1.z * lk_smooth(0.9, 1.1, lk_luma(haze) / max(lk_luma(base), 1e-6));
    }
    if (look.wx_tint.w > 0.0) {
        base = base + lk_afterglow(d, look.wx_tint.w);
        let hh = length(d.xz);
        if (hh > 1e-6) {
            haze = haze + lk_afterglow(vec3<f32>(d.x / hh * sqrt(1.0 - 0.0349 * 0.0349), 0.0349, d.z / hh * sqrt(1.0 - 0.0349 * 0.0349)), look.wx_tint.w);
        }
    }
    var bw = mix(0.06, 0.22, clamp((look.horizon.w - 2.0) / 8.0, 0.0, 1.0));
    var bp = 0.42;
    if (skyline > 0.0) {
        bw = mix(bw, max(bw, look.dg2.z), skyline);
        bp = mix(bp, max(bp, look.dg2.w), skyline);
    }
    let b = 1.0 - clamp(abs(d.y) / bw, 0.0, 1.0);
    let band = b * b * bp;
    let below = lk_smooth(0.035, -0.02, d.y);
    return mix(base, haze, clamp(band + below * 0.94, 0.0, 1.0));
}
