// the atmosphere from precomputed tables (Hillaire 2020, "A
// Scalable and Production Ready Sky and Atmosphere Rendering Technique", after
// Bruneton and Neyret 2008): a spherical planet (R 6360 km, top 6460 km) with
// Rayleigh scattering (5.802, 13.558, 33.1) e-3 / km, scale height 8 km; Mie
// (aerosol) scattering 3.996e-3 and absorption 4.4e-3 / km, scale height 1.2
// km, g 0.8 (the medium the tables use is the uniform's, `at_medium_of`); ozone
// absorption (0.650, 1.881, 0.085) e-3 / km in a 30 km tent about 25 km. Three
// tables, all compute: the transmittance to the top of the atmosphere (256 x 64,
// the Bruneton (r, mu) mapping), the multiple-scattering transfer (32 x 32, the
// second order and its geometric series, isotropic), and the sky seen from the
// eye (192 x 108, azimuth about the sun x elevation, dense near the horizon).
// The planet's shadow, the twilight, the Earth's shadow band, the afterglow and
// the reddened sun come out of the integration. Radiance per unit solar
// illuminance; the volume pass scales it to the look's units.
// The sampling helpers here use no uniform: they are appended to every
// module (look.rs `look_common_source`), so any pass that binds the sky-view
// and transmittance tables can read the sky (`at_sky`, `at_sun`). The medium
// and the table passes are in sky_atmo_luts.wgsl.

const AT_R: f32 = 6360.0;
const AT_TOP: f32 = 6460.0;
const AT_RAY: vec3<f32> = vec3<f32>(5.802e-3, 13.558e-3, 33.1e-3);
const AT_MIE_S: f32 = 3.996e-3;
const AT_MIE_A: f32 = 4.40e-3;
// the red channel's ozone is the Chappuis band averaged over the
// sRGB red primary's 580-700 nm (it peaks at 603 nm), not its 680 nm value
// alone (0.650e-3): with it the twilight zenith kept its red, went
// magenta-violet, and was greyed into lavender; ozone is what keeps a blue
// hour blue.
// 1.45e-3 for red (1.70 with grey aerosol it starved the low sky of red;
// the aerosol's own colour, `AT_MIE_ANG`, now reddens the sun and the horizon; at 1.0
// the blue-hour zenith went violet again).
// 1.881e-3, the green's: the blue-hour zenith had red at or above green (lavender)
const AT_OZONE: vec3<f32> = vec3<f32>(1.881e-3, 1.881e-3, 0.085e-3);
const AT_MIE_ANG: vec3<f32> = vec3<f32>(0.76, 1.0, 1.34);
const AT_PI: f32 = 3.14159265;

struct AtMedium {
    scat: vec3<f32>,
    ext: vec3<f32>,
    ray: vec3<f32>,
    mie: vec3<f32>,
};

/// the medium at a height (km) for an aerosol `m` (x scattering, y absorption per km
/// at the ground, 550 nm; z scale height km) and spectral factor `sp.rgb`, Rayleigh
/// scattering `ray.rgb` at sea level (scale height `ray.w` km) and ozone's peak
/// absorption `oz.rgb` (sky.rs `atmo_m`, `atmo_s`, `atmo_r`, `atmo_o`: eo-look
/// `atmosphere`; the constants above with `--no-phys-sky`).
fn at_medium_of(h: f32, m: vec4<f32>, sp: vec4<f32>, ray: vec4<f32>, oz: vec4<f32>) -> AtMedium {
    let hh = max(h, 0.0);
    let dr = exp(-hh / max(ray.w, 0.5));
    let dm = exp(-hh / max(m.z, 0.05));
    let doz = max(0.0, 1.0 - abs(hh - 25.0) / 15.0);
    var o: AtMedium;
    o.ray = ray.rgb * dr;
    o.mie = m.x * dm * sp.rgb;
    o.scat = o.ray + o.mie;
    o.ext = o.ray + (m.x + m.y) * dm * sp.rgb + oz.rgb * doz;
    return o;
}

/// the three-channel model's chroma correction (eo-look `atmosphere::RGB_CHROMA`): the
/// colour `k` of the way from its luminance, by weight `w` (the physical daylight's).
const AT_RGB_CHROMA: f32 = 0.8;

fn at_spectral(c: vec3<f32>, w: f32) -> vec3<f32> {
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(c, vec3<f32>(l) + (c - vec3<f32>(l)) * AT_RGB_CHROMA, w);
}

/// Distance along a ray from radius r (cosine mu to the zenith) to a sphere of radius rs; negative if missed.
fn at_sphere(r: f32, mu: f32, rs: f32) -> f32 {
    let b = r * mu;
    let c = r * r - rs * rs;
    let disc = b * b - c;
    if (disc < 0.0) {
        return -1.0;
    }
    let s = sqrt(disc);
    let t0 = -b - s;
    let t1 = -b + s;
    if (t0 > 0.0) {
        return t0;
    }
    return t1;
}

fn at_ground_hit(r: f32, mu: f32) -> bool {
    return mu < 0.0 && r * r * (mu * mu - 1.0) + AT_R * AT_R >= 0.0;
}

fn at_trans_uv(r: f32, mu: f32) -> vec2<f32> {
    let hh = sqrt(AT_TOP * AT_TOP - AT_R * AT_R);
    let rho = sqrt(max(r * r - AT_R * AT_R, 0.0));
    let disc = r * r * (mu * mu - 1.0) + AT_TOP * AT_TOP;
    let d = max(0.0, -r * mu + sqrt(max(disc, 0.0)));
    let d_min = AT_TOP - r;
    let d_max = rho + hh;
    return vec2<f32>((d - d_min) / max(d_max - d_min, 1e-4), rho / hh);
}

fn at_rayleigh_phase(c: f32) -> f32 {
    return 3.0 / (16.0 * AT_PI) * (1.0 + c * c);
}

fn at_mie_phase(c: f32) -> f32 {
    // 0.75 Cornette-Shanks g 0.7 + 0.25 Henyey-Greenstein g 0.95 (the
    // aerosol's diffraction peak). Cornette-Shanks g 0.8 alone was flat within
    // ~15 deg of the sun (x1.4 from 3 to 10 deg): a clipped glare 20 deg across
    // round a low sun with no compact aureole and no disc standing out of it.
    // 0.55 Cornette-Shanks g 0.6 + 0.45 HG g 0.93 (0.75 CS 0.7 + 0.25 HG 0.95):
    // the broad lobe drew the clipped glare plateau 10-20 deg round a low sun; most of
    // the aerosol's forward light is in its first few degrees, which falls off steeply.
    let g = 0.6;
    let k = 3.0 / (8.0 * AT_PI) * (1.0 - g * g) / (2.0 + g * g);
    let cs = k * (1.0 + c * c) / pow(max(1.0 + g * g - 2.0 * g * c, 1e-4), 1.5);
    let h = 0.93;
    let hg = (1.0 - h * h) / (4.0 * AT_PI * pow(max(1.0 + h * h - 2.0 * h * c, 1e-5), 1.5));
    return 0.55 * cs + 0.45 * hg;
}

/// The sky table's (u, v) for a view direction: u the azimuth about the sun
/// (0.5 toward it), v the elevation, square-root dense at the horizon.
fn at_view_uv(d: vec3<f32>, sun: vec3<f32>) -> vec2<f32> {
    let el = asin(clamp(d.y, -1.0, 1.0));
    let v = 0.5 + 0.5 * sign(el) * sqrt(abs(el) / (0.5 * AT_PI));
    let a = atan2(d.z, d.x) - atan2(sun.z, sun.x);
    var u = a / (2.0 * AT_PI) + 0.5;
    u = u - floor(u);
    return vec2<f32>(u, v);
}

fn at_view_dir(uv: vec2<f32>, sun: vec3<f32>) -> vec3<f32> {
    let c = uv.y * 2.0 - 1.0;
    let el = sign(c) * c * c * 0.5 * AT_PI;
    let a = (uv.x - 0.5) * 2.0 * AT_PI + atan2(sun.z, sun.x);
    return vec3<f32>(cos(el) * cos(a), sin(el), cos(el) * sin(a));
}

/// the sky's radiance along a direction from the sky-view table
/// (`tv`, with a clamped linear sampler `s`), in the look's units: the table
/// scaled so its zenith has the luminance `zenith_luma` (the look's own zenith,
/// `lk_luma(look.dome_zenith.xyz)`, which the exposure was made against).
/// `sun`: toward the sun (`look.sun.xyz`).
fn at_sky(tv: texture_2d<f32>, s: sampler, d: vec3<f32>, sun: vec3<f32>, zenith_luma: f32) -> vec3<f32> {
    let z = textureSampleLevel(tv, s, at_view_uv(vec3<f32>(0.0, 1.0, 0.0), sun), 0.0).rgb;
    let g = zenith_luma / max(dot(z, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-9);
    return textureSampleLevel(tv, s, at_view_uv(d, sun), 0.0).rgb * g;
}

/// the sun's transmittance from space to a height (m) through the
/// transmittance table (`tt`), zero where the planet hides it.
fn at_sun(tt: texture_2d<f32>, s: sampler, h_m: f32, sun_y: f32) -> vec3<f32> {
    let r = AT_R + max(h_m, 1.0) * 0.001;
    if (at_ground_hit(r, sun_y)) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(tt, s, at_trans_uv(r, sun_y), 0.0).rgb;
}
