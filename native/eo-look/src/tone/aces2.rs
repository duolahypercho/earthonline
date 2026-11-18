//! The ACES 2.0 output transform (Academy Color Encoding System, v2.0,
//! `Output.Academy.Rec709-D65_100nit_in_Rec709-D65_sRGB-Piecewise`), ported
//! from the reference CTL of the aces-core repository (`lib/Lib.Academy.
//! OutputTransform.ctl`, `Lib.Academy.Tonescale.ctl`, `Lib.Academy.
//! Utilities.ctl`, `Lib.Academy.ColorSpaces.ctl`, `Lib.Academy.
//! DisplayEncoding.ctl`; transform IDs `urn:ampas:aces:transformId:v2.0:*.a2.v1`).
//!
//! SPDX-License-Identifier: Apache-2.0
//! Copyright Contributors to the ACES Project. Ported to Rust for this
//! repository; the licence and attribution are in
//! `eo-look/data/camera/ACES.NOTICE.txt` and `LICENSES/Apache-2.0.txt`.
//!
//! The port keeps the CTL's conventions: a colour is a row vector and
//! `mul_v` is the CTL's `mult_f3_f33` (v x M), so the matrices read as they
//! do there. Only the forward direction is ported (the game never inverts
//! the display). The transform takes ACES2065-1 (AP0, D60 white) scene
//! values, with 0.18 the scene's middle grey, and returns display-linear
//! Rec.709 (D65) values where 1.0 is the 100-nit peak; `encode_srgb` is the
//! IEC 61966-2-1 piecewise inverse EOTF the CTL applies (`eotf_enum` 6).

pub type V3 = [f64; 3];
pub type M33 = [[f64; 3]; 3];

/// (red, green, blue, white) chromaticities.
#[derive(Clone, Copy, Debug)]
pub struct Chromaticities {
    pub red: [f64; 2],
    pub green: [f64; 2],
    pub blue: [f64; 2],
    pub white: [f64; 2],
}

/// ACES primaries 0 (SMPTE ST 2065-1), white ~D60.
pub const AP0: Chromaticities = Chromaticities { red: [0.73470, 0.26530], green: [0.0, 1.0], blue: [0.00010, -0.07700], white: [0.32168, 0.33767] };
/// ACES primaries 1 (the "reach" gamut).
pub const AP1: Chromaticities = Chromaticities { red: [0.713, 0.293], green: [0.165, 0.830], blue: [0.128, 0.044], white: [0.32168, 0.33767] };
/// ITU-R BT.709 primaries, D65 white (the limiting and the encoding gamut of the sRGB output).
pub const REC709: Chromaticities = Chromaticities { red: [0.64, 0.33], green: [0.30, 0.60], blue: [0.15, 0.06], white: [0.3127, 0.3290] };

// ---- CTL utilities ----

pub fn mul_v(v: V3, m: &M33) -> V3 {
    [0, 1, 2].map(|j| v[0] * m[0][j] + v[1] * m[1][j] + v[2] * m[2][j])
}

pub fn mul_m(a: &M33, b: &M33) -> M33 {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

pub fn inv_m(m: &M33) -> M33 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let k = 1.0 / det;
    [
        [(e * i - f * h) * k, (c * h - b * i) * k, (b * f - c * e) * k],
        [(f * g - d * i) * k, (a * i - c * g) * k, (c * d - a * f) * k],
        [(d * h - e * g) * k, (b * g - a * h) * k, (a * e - b * d) * k],
    ]
}

fn scale_m(s: f64, m: &M33) -> M33 {
    m.map(|r| r.map(|v| v * s))
}

const IDENTITY: M33 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn diag(v: V3) -> M33 {
    [[v[0], 0.0, 0.0], [0.0, v[1], 0.0], [0.0, 0.0, v[2]]]
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

fn lerp3(a: V3, b: V3, t: f64) -> V3 {
    [0, 1, 2].map(|i| lerp(a[i], b[i], t))
}

/// `RGBtoXYZ_f33`: rows are the primaries' XYZ (XYZ = rgb x M).
pub fn rgb_to_xyz(c: &Chromaticities, y: f64) -> M33 {
    let (r, g, b, w) = (c.red, c.green, c.blue, c.white);
    let x = w[0] * y / w[1];
    let z = (1.0 - w[0] - w[1]) * y / w[1];
    let d = r[0] * (b[1] - g[1]) + b[0] * (g[1] - r[1]) + g[0] * (r[1] - b[1]);
    let sr = (x * (b[1] - g[1]) - g[0] * (y * (b[1] - 1.0) + b[1] * (x + z)) + b[0] * (y * (g[1] - 1.0) + g[1] * (x + z))) / d;
    let sg = (x * (r[1] - b[1]) + r[0] * (y * (b[1] - 1.0) + b[1] * (x + z)) - b[0] * (y * (r[1] - 1.0) + r[1] * (x + z))) / d;
    let sb = (x * (g[1] - r[1]) - r[0] * (y * (g[1] - 1.0) + g[1] * (x + z)) + g[0] * (y * (r[1] - 1.0) + r[1] * (x + z))) / d;
    [
        [sr * r[0], sr * r[1], sr * (1.0 - r[0] - r[1])],
        [sg * g[0], sg * g[1], sg * (1.0 - g[0] - g[1])],
        [sb * b[0], sb * b[1], sb * (1.0 - b[0] - b[1])],
    ]
}

pub fn xyz_to_rgb(c: &Chromaticities, y: f64) -> M33 {
    inv_m(&rgb_to_xyz(c, y))
}

/// `CONE_RESP_MAT_BRADFORD` (Lib.Academy.ColorSpaces), in the CTL's row-vector form.
const BRADFORD: M33 = [[0.89510, -0.75020, 0.03890], [0.26640, 1.71350, -0.06850], [-0.16140, 0.03670, 1.02960]];

/// `calculate_cat_matrix` (von Kries in Bradford cone space).
pub fn cat_matrix(src: [f64; 2], dst: [f64; 2]) -> M33 {
    let xyz = |w: [f64; 2]| [w[0] / w[1], 1.0, (1.0 - w[0] - w[1]) / w[1]];
    let s = mul_v(xyz(src), &BRADFORD);
    let d = mul_v(xyz(dst), &BRADFORD);
    let vk = diag([d[0] / s[0], d[1] / s[1], d[2] / s[2]]);
    mul_m(&BRADFORD, &mul_m(&vk, &inv_m(&BRADFORD)))
}

/// `calculate_rgb_to_rgb_matrix` (Bradford adaptation between the whites).
pub fn rgb_to_rgb(src: &Chromaticities, dst: &Chromaticities) -> M33 {
    mul_m(&rgb_to_xyz(src, 1.0), &mul_m(&cat_matrix(src.white, dst.white), &xyz_to_rgb(dst, 1.0)))
}

// ---- tone scale (Lib.Academy.Tonescale) ----

#[derive(Clone, Copy, Debug)]
pub struct TsParams {
    pub n: f64,
    pub n_r: f64,
    pub g: f64,
    pub t_1: f64,
    pub c_t: f64,
    pub s_2: f64,
    pub u_2: f64,
    pub m_2: f64,
    pub forward_limit: f64,
    pub log_peak: f64,
}

pub fn ts_params(peak: f64) -> TsParams {
    let n = peak;
    let n_r = 100.0;
    let g = 1.15;
    let c = 0.18;
    let c_d = 10.013;
    let w_g = 0.14;
    let t_1 = 0.04;
    let r_hit_min = 128.0;
    let r_hit_max = 896.0;
    let r_hit = r_hit_min + (r_hit_max - r_hit_min) * ((n / n_r).ln() / (10000.0f64 / 100.0).ln());
    let m_0 = n / n_r;
    let m_1 = 0.5 * (m_0 + (m_0 * (m_0 + 4.0 * t_1)).sqrt());
    let u = ((r_hit / m_1) / ((r_hit / m_1) + 1.0)).powf(g);
    let m = m_1 / u;
    let w_i = (n / 100.0).ln() / 2f64.ln();
    let c_t = c_d / n_r * (1.0 + w_i * w_g);
    let g_ip = 0.5 * (c_t + (c_t * (c_t + 4.0 * t_1)).sqrt());
    let g_ipp2 = -(m_1 * (g_ip / m).powf(1.0 / g)) / ((g_ip / m).powf(1.0 / g) - 1.0);
    let w_2 = c / g_ipp2;
    let s_2 = w_2 * m_1;
    let u_2 = ((r_hit / m_1) / ((r_hit / m_1) + w_2)).powf(g);
    let m_2 = m_1 / u_2;
    TsParams { n, n_r, g, t_1, c_t, s_2, u_2, m_2, forward_limit: 8.0 * r_hit, log_peak: (n / n_r).log10() }
}

/// `tonescale_fwd`: scene-linear in, display luminance (cd/m2) out.
pub fn tonescale_fwd(x: f64, p: &TsParams) -> f64 {
    let f = p.m_2 * (x.max(0.0) / (x + p.s_2)).powf(p.g);
    let h = (f * f / (f + p.t_1)).max(0.0);
    h * p.n_r
}

// ---- the output transform (Lib.Academy.OutputTransform) ----

const TABLE_SIZE: usize = 360;
const TOTAL_TABLE: usize = TABLE_SIZE + 2;
const BASE_INDEX: usize = 1;
const HUE_LIMIT: f64 = 360.0;
const CUSP_CORNERS: usize = 6;
const TOTAL_CORNERS: usize = CUSP_CORNERS + 2;
const MAX_SORTED: usize = 2 * CUSP_CORNERS;
const REACH_CUSP_TOL: f64 = 1e-3;
const DISPLAY_CUSP_TOL: f64 = 1e-7;
const GAMMA_MIN: f64 = 0.0;
const GAMMA_MAX: f64 = 5.0;
const GAMMA_STEP: f64 = 0.4;
const GAMMA_ACCURACY: f64 = 1e-5;

const REF_LUMINANCE: f64 = 100.0;
const L_A: f64 = 100.0;
const Y_B: f64 = 20.0;
const SURROUND: V3 = [0.9, 0.59, 0.9];
const J_SCALE: f64 = 100.0;
const CAM_NL_OFFSET: f64 = 0.2713 * 100.0;
const CAM_NL_SCALE: f64 = 4.0 * 100.0;

fn model_gamma() -> f64 {
    SURROUND[1] * (1.48 + (Y_B / REF_LUMINANCE).sqrt())
}

const CHROMA_COMPRESS: f64 = 2.4;
const CHROMA_COMPRESS_FACT: f64 = 3.3;
const CHROMA_EXPAND: f64 = 1.3;
const CHROMA_EXPAND_FACT: f64 = 0.69;
const CHROMA_EXPAND_THR: f64 = 0.5;
const SMOOTH_CUSPS: f64 = 0.12;
const SMOOTH_M: f64 = 0.27;
const CUSP_MID_BLEND: f64 = 1.3;
const FOCUS_GAIN_BLEND: f64 = 0.3;
const FOCUS_DISTANCE: f64 = 1.35;
const FOCUS_DISTANCE_SCALING: f64 = 1.75;
const COMPRESSION_THRESHOLD: f64 = 0.75;

#[derive(Clone, Copy, Debug)]
pub struct JmhParams {
    rgb_to_cam16_c: M33,
    cam16_c_to_rgb: M33,
    cone_to_aab: M33,
    aab_to_cone: M33,
    f_l_n: f64,
    cz: f64,
    inv_cz: f64,
    a_w_j: f64,
    inv_a_w_j: f64,
}

fn wrap_360(h: f64) -> f64 {
    let y = h % 360.0;
    if y < 0.0 { y + 360.0 } else { y }
}

fn hue_position(h: f64, size: usize) -> i64 {
    (wrap_360(h) / HUE_LIMIT * size as f64) as i64
}

fn pacrc_fwd(rc: f64) -> f64 {
    let f = rc.powf(0.42);
    f / (CAM_NL_OFFSET + f)
}

fn pacrc_inv(ra: f64) -> f64 {
    let r = ra.min(0.99);
    let f = (CAM_NL_OFFSET * r) / (1.0 - r);
    f.powf(1.0 / 0.42)
}

fn pacrc_fwd_s(v: f64) -> f64 {
    pacrc_fwd(v.abs()).copysign(v)
}

fn pacrc_inv_s(v: f64) -> f64 {
    pacrc_inv(v.abs()).copysign(v)
}

fn a_to_j(a: f64, cz: f64) -> f64 {
    J_SCALE * a.powf(cz)
}

fn j_to_a(j: f64, inv_cz: f64) -> f64 {
    (j * (1.0 / J_SCALE)).powf(inv_cz)
}

fn j_to_y(j: f64, p: &JmhParams) -> f64 {
    let a = j_to_a(j.abs(), p.inv_cz);
    pacrc_inv(p.a_w_j * a) / p.f_l_n
}

fn y_to_j(y: f64, p: &JmhParams) -> f64 {
    let ra = pacrc_fwd(y.abs() * p.f_l_n);
    a_to_j(ra * p.inv_a_w_j, p.cz).copysign(y)
}

fn rgb_to_aab(rgb: V3, p: &JmhParams) -> V3 {
    let m = mul_v(rgb, &p.rgb_to_cam16_c);
    let a = m.map(pacrc_fwd_s);
    mul_v(a, &p.cone_to_aab)
}

fn aab_to_jmh(aab: V3, p: &JmhParams) -> V3 {
    if aab[0] <= 0.0 {
        return [0.0; 3];
    }
    let j = a_to_j(aab[0], p.cz);
    let m = (aab[1] * aab[1] + aab[2] * aab[2]).sqrt();
    let h = wrap_360(aab[2].atan2(aab[1]).to_degrees());
    [j, m, h]
}

pub fn rgb_to_jmh(rgb: V3, p: &JmhParams) -> V3 {
    aab_to_jmh(rgb_to_aab(rgb, p), p)
}

fn jmh_to_aab(jmh: V3, p: &JmhParams) -> V3 {
    let hr = jmh[2].to_radians();
    [j_to_a(jmh[0], p.inv_cz), jmh[1] * hr.cos(), jmh[1] * hr.sin()]
}

fn aab_to_rgb(aab: V3, p: &JmhParams) -> V3 {
    let a = mul_v(aab, &p.aab_to_cone);
    mul_v(a.map(pacrc_inv_s), &p.cam16_c_to_rgb)
}

pub fn jmh_to_rgb(jmh: V3, p: &JmhParams) -> V3 {
    aab_to_rgb(jmh_to_aab(jmh, p), p)
}

pub fn jmh_params(prims: &Chromaticities) -> JmhParams {
    let cam16 = Chromaticities { red: [0.8336, 0.1735], green: [2.3854, -1.4659], blue: [0.087, -0.125], white: [0.333, 0.333] };
    let m16 = xyz_to_rgb(&cam16, 1.0);
    let base: M33 = [[2.0, 1.0, 1.0 / 9.0], [1.0, -12.0 / 11.0, 1.0 / 9.0], [1.0 / 20.0, 1.0 / 11.0, -2.0 / 9.0]];
    let to_xyz = rgb_to_xyz(prims, 1.0);
    let xyz_w = mul_v([REF_LUMINANCE; 3], &to_xyz);
    let y_w = xyz_w[1];
    let rgb_w = mul_v(xyz_w, &m16);
    let k = 1.0 / (5.0 * L_A + 1.0);
    let k4 = k * k * k * k;
    let f_l = 0.2 * k4 * (5.0 * L_A) + 0.1 * (1.0 - k4).powi(2) * (5.0 * L_A).powf(1.0 / 3.0);
    let f_l_n = f_l / REF_LUMINANCE;
    let cz = model_gamma();
    let d_rgb = [0, 1, 2].map(|i| f_l_n * y_w / rgb_w[i]);
    let rgb_wc = [0, 1, 2].map(|i| d_rgb[i] * rgb_w[i]);
    let rgb_aw = rgb_wc.map(pacrc_fwd_s);
    let cone_to_aab = mul_m(&scale_m(CAM_NL_SCALE, &IDENTITY), &base);
    let a_w = cone_to_aab[0][0] * rgb_aw[0] + cone_to_aab[1][0] * rgb_aw[1] + cone_to_aab[2][0] * rgb_aw[2];
    let a_w_j = pacrc_fwd(f_l);
    let m1 = mul_m(&to_xyz, &m16);
    let m2 = scale_m(REF_LUMINANCE, &IDENTITY);
    let rgb_to_cam16 = mul_m(&m1, &m2);
    let rgb_to_cam16_c = mul_m(&rgb_to_cam16, &diag(d_rgb));
    let s = 43.0 * SURROUND[2];
    let m_aab = [0, 1, 2].map(|i| [cone_to_aab[i][0] / a_w, cone_to_aab[i][1] * s, cone_to_aab[i][2] * s]);
    JmhParams {
        rgb_to_cam16_c,
        cam16_c_to_rgb: inv_m(&rgb_to_cam16_c),
        cone_to_aab: m_aab,
        aab_to_cone: inv_m(&m_aab),
        f_l_n,
        cz,
        inv_cz: 1.0 / cz,
        a_w_j,
        inv_a_w_j: 1.0 / a_w_j,
    }
}

/// The precomputed output-transform state (`ODTParams`).
#[derive(Clone, Debug)]
pub struct OdtParams {
    pub peak: f64,
    input: JmhParams,
    reach: JmhParams,
    limit: JmhParams,
    pub ts: TsParams,
    limit_j_max: f64,
    model_gamma_inv: f64,
    reach_m: [f64; TOTAL_TABLE],
    sat: f64,
    sat_thr: f64,
    compr: f64,
    chroma_compress_scale: f64,
    mid_j: f64,
    focus_dist: f64,
    lower_hull_gamma_inv: f64,
    hues: [f64; TOTAL_TABLE],
    cusps: [V3; TOTAL_TABLE],
    upper_hull_gamma: [f64; TOTAL_TABLE],
    search_range: [i64; 2],
    ap0_to_ap1: M33,
    ap1_to_ap0: M33,
}

fn reach_m_from_table(h: f64, t: &[f64; TOTAL_TABLE]) -> f64 {
    let base = hue_position(h, TABLE_SIZE);
    let f = h - base as f64;
    let lo = base as usize + BASE_INDEX;
    lerp(t[lo], t[lo + 1], f)
}

fn reinhard_remap(scale: f64, nd: f64) -> f64 {
    scale * nd / (1.0 + nd)
}

fn toe(x: f64, limit: f64, k1_in: f64, k2_in: f64) -> f64 {
    if x > limit {
        return x;
    }
    let k2 = k2_in.max(0.001);
    let k1 = (k1_in * k1_in + k2 * k2).sqrt();
    let k3 = (limit + k1) / (limit + k2);
    let minus_b = k3 * x - k1;
    let minus_c = k2 * k3 * x;
    0.5 * (minus_b + (minus_b * minus_b + 4.0 * minus_c).sqrt())
}

fn chroma_compress_norm(h: f64, scale: f64) -> f64 {
    let hr = h.to_radians();
    let (a, b) = (hr.cos(), hr.sin());
    let cos2 = a * a - b * b;
    let sin2 = 2.0 * a * b;
    let cos3 = 4.0 * a * a * a - 3.0 * a;
    let sin3 = 3.0 * b - 4.0 * b * b * b;
    let m = 11.34072 * a + 16.46899 * cos2 + 7.88380 * cos3 + 14.66441 * b + -6.37224 * sin2 + 9.19364 * sin3 + 77.12896;
    m * scale
}

fn chroma_compress_fwd(jmh: V3, tonemapped_j: f64, p: &OdtParams) -> V3 {
    let (j, m, h) = (jmh[0], jmh[1], jmh[2]);
    let mut mc = m;
    if m != 0.0 {
        let nj = tonemapped_j / p.limit_j_max;
        let snj = (1.0 - nj).max(0.0);
        let mnorm = chroma_compress_norm(h, p.chroma_compress_scale);
        let limit = nj.powf(p.model_gamma_inv) * reach_m_from_table(h, &p.reach_m) / mnorm;
        let toe_limit = limit - 0.001;
        let toe_snj_sat = snj * p.sat;
        let toe_sqrt = (nj * nj + p.sat_thr).sqrt();
        let toe_nj_compr = nj * p.compr;
        mc = m * (tonemapped_j / j).powf(p.model_gamma_inv);
        mc /= mnorm;
        mc = limit - toe(limit - mc, toe_limit, toe_snj_sat, toe_sqrt);
        mc = toe(mc, limit, toe_nj_compr, snj);
        mc *= mnorm;
    }
    [tonemapped_j, mc, h]
}

fn tonemap_and_compress_fwd(jmh: V3, p: &OdtParams) -> V3 {
    let linear = j_to_y(jmh[0], &p.input) / REF_LUMINANCE;
    let ty = tonescale_fwd(linear, &p.ts);
    let j_ts = y_to_j(ty, &p.input);
    chroma_compress_fwd(jmh, j_ts, p)
}

fn compression_vector_slope(intersect_j: f64, focus_j: f64, limit_j_max: f64, slope_gain: f64) -> f64 {
    let d = if intersect_j < focus_j { intersect_j } else { limit_j_max - intersect_j };
    d * (intersect_j - focus_j) / (focus_j * slope_gain)
}

fn solve_j_intersect(j: f64, m: f64, focus_j: f64, max_j: f64, slope_gain: f64) -> f64 {
    let ms = m / slope_gain;
    let a = ms / focus_j;
    if j < focus_j {
        let b = 1.0 - ms;
        let c = -j;
        let det = b * b - 4.0 * a * c;
        -2.0 * c / (b + det.sqrt())
    } else {
        let b = -(1.0 + ms + max_j * a);
        let c = max_j * ms + j;
        let det = b * b - 4.0 * a * c;
        -2.0 * c / (b - det.sqrt())
    }
}

fn smin_scaled(a: f64, b: f64, scale_ref: f64) -> f64 {
    let s = SMOOTH_CUSPS * scale_ref;
    let h = (s - (a - b).abs()).max(0.0) / s;
    a.min(b) - h * h * h * s * (1.0 / 6.0)
}

fn line_boundary_m(j_axis: f64, slope: f64, inv_gamma: f64, j_max: f64, m_max: f64, j_ref: f64) -> f64 {
    let nj = j_axis / j_ref;
    let shifted = j_ref * nj.powf(inv_gamma);
    shifted * m_max / (j_max - slope * m_max)
}

fn gamut_boundary_m(cusp: [f64; 2], j_max: f64, gamma_top_inv: f64, gamma_bottom_inv: f64, j_src: f64, slope: f64, j_cusp: f64) -> f64 {
    let lower = line_boundary_m(j_src, slope, gamma_bottom_inv, cusp[0], cusp[1], j_cusp);
    let upper = line_boundary_m(j_max - j_src, -slope, gamma_top_inv, j_max - cusp[0], cusp[1], j_max - j_cusp);
    smin_scaled(lower, upper, cusp[1])
}

fn focus_gain(j: f64, threshold: f64, limit_j_max: f64, focus_dist: f64) -> f64 {
    let mut gain = limit_j_max * focus_dist;
    if j > threshold {
        let adj = ((limit_j_max - threshold) / (limit_j_max - j).max(0.0001)).log10();
        gain *= adj * adj + 1.0;
    }
    gain
}

fn remap_m(m: f64, gamut_m: f64, reach_m: f64) -> f64 {
    let ratio = gamut_m / reach_m;
    let proportion = ratio.max(COMPRESSION_THRESHOLD);
    let threshold = proportion * gamut_m;
    if m <= threshold || proportion >= 1.0 {
        return m;
    }
    let m_off = m - threshold;
    let gamut_off = gamut_m - threshold;
    let reach_off = reach_m - threshold;
    let scale = reach_off / ((reach_off / gamut_off) - 1.0);
    threshold + reinhard_remap(scale, m_off / scale)
}

struct HueParams {
    cusp: [f64; 2],
    gamma_bottom_inv: f64,
    gamma_top_inv: f64,
    focus_j: f64,
    analytical_threshold: f64,
}

fn compress_gamut(jmh: V3, jx: f64, p: &OdtParams, hp: &HueParams) -> V3 {
    let (j, m, h) = (jmh[0], jmh[1], jmh[2]);
    let slope_gain = focus_gain(jx, hp.analytical_threshold, p.limit_j_max, p.focus_dist);
    let j_src = solve_j_intersect(j, m, hp.focus_j, p.limit_j_max, slope_gain);
    let slope = compression_vector_slope(j_src, hp.focus_j, p.limit_j_max, slope_gain);
    let j_cusp = solve_j_intersect(hp.cusp[0], hp.cusp[1], hp.focus_j, p.limit_j_max, slope_gain);
    let gm = gamut_boundary_m(hp.cusp, p.limit_j_max, hp.gamma_top_inv, hp.gamma_bottom_inv, j_src, slope, j_cusp);
    if gm <= 0.0 {
        return [j, 0.0, h];
    }
    let reach_max = reach_m_from_table(h, &p.reach_m);
    let reach_bound = line_boundary_m(j_src, slope, p.model_gamma_inv, p.limit_j_max, reach_max, p.limit_j_max);
    let rm = remap_m(m, gm, reach_bound);
    [j_src + rm * slope, rm, h]
}

fn cusp_from_table(h: f64, t: &[V3; TOTAL_TABLE]) -> [f64; 2] {
    let mut low = 0usize;
    let mut high = BASE_INDEX + TABLE_SIZE;
    let mut i = (hue_position(h, TABLE_SIZE) as usize) + BASE_INDEX;
    while low + 1 < high {
        if h > t[i][2] {
            low = i;
        } else {
            high = i;
        }
        i = (low + high) / 2;
    }
    let (lo, hi) = (t[high - 1], t[high]);
    let f = (h - lo[2]) / (hi[2] - lo[2]);
    [lerp(lo[0], hi[0], f), lerp(lo[1], hi[1], f)]
}

fn lookup_hue_interval(h: f64, hues: &[f64; TOTAL_TABLE], range: [i64; 2]) -> usize {
    let mut i = BASE_INDEX as i64 + hue_position(h, TOTAL_TABLE);
    let mut lo = (BASE_INDEX as i64).max(i + range[0]);
    let mut hi = ((BASE_INDEX + TABLE_SIZE) as i64).min(i + range[1]);
    while lo + 1 < hi {
        if h > hues[i as usize] {
            lo = i;
        } else {
            hi = i;
        }
        i = (lo + hi) / 2;
    }
    hi.max(1) as usize
}

fn focus_j_of(cusp_j: f64, mid_j: f64, limit_j_max: f64) -> f64 {
    lerp(cusp_j, mid_j, (CUSP_MID_BLEND - cusp_j / limit_j_max).min(1.0))
}

fn hue_params(h: f64, p: &OdtParams) -> HueParams {
    let hi = lookup_hue_interval(h, &p.hues, p.search_range);
    let t = h - p.hues[hi - 1];
    let cusp = cusp_from_table(h, &p.cusps);
    HueParams {
        cusp,
        gamma_bottom_inv: p.lower_hull_gamma_inv,
        gamma_top_inv: lerp(p.upper_hull_gamma[hi - 1], p.upper_hull_gamma[hi], t),
        focus_j: focus_j_of(cusp[0], p.mid_j, p.limit_j_max),
        analytical_threshold: lerp(cusp[0], p.limit_j_max, FOCUS_GAIN_BLEND),
    }
}

fn gamut_compress_fwd(jmh: V3, p: &OdtParams) -> V3 {
    let (j, m, h) = (jmh[0], jmh[1], jmh[2]);
    if j <= 0.0 {
        return [0.0, 0.0, h];
    }
    if m < 0.0 || j > p.limit_j_max {
        return [j, 0.0, h];
    }
    let hp = hue_params(h, p);
    compress_gamut(jmh, j, p, &hp)
}

// ---- table building ----

fn cusp_corner(corner: usize) -> V3 {
    let c = |k: usize| if (corner + k) % CUSP_CORNERS < 3 { 1.0 } else { 0.0 };
    [c(1), c(5), c(3)]
}

fn limiting_corners(params: &JmhParams, peak: f64) -> ([V3; TOTAL_CORNERS], [V3; TOTAL_CORNERS]) {
    let mut trgb = [[0.0; 3]; CUSP_CORNERS];
    let mut tjmh = [[0.0; 3]; CUSP_CORNERS];
    let mut min_i = 0;
    for i in 0..CUSP_CORNERS {
        trgb[i] = cusp_corner(i).map(|v| v * peak / REF_LUMINANCE);
        tjmh[i] = rgb_to_jmh(trgb[i], params);
        if tjmh[i][2] < tjmh[min_i][2] {
            min_i = i;
        }
    }
    let mut rgb = [[0.0; 3]; TOTAL_CORNERS];
    let mut jmh = [[0.0; 3]; TOTAL_CORNERS];
    for i in 0..CUSP_CORNERS {
        rgb[i + 1] = trgb[(i + min_i) % CUSP_CORNERS];
        jmh[i + 1] = tjmh[(i + min_i) % CUSP_CORNERS];
    }
    rgb[0] = rgb[CUSP_CORNERS];
    rgb[CUSP_CORNERS + 1] = rgb[1];
    jmh[0] = jmh[CUSP_CORNERS];
    jmh[CUSP_CORNERS + 1] = jmh[1];
    jmh[0][2] -= HUE_LIMIT;
    jmh[CUSP_CORNERS + 1][2] += HUE_LIMIT;
    (rgb, jmh)
}

fn reach_corners(reach: &JmhParams, limit_j_max: f64, forward_limit: f64) -> [V3; TOTAL_CORNERS] {
    let mut t = [[0.0; 3]; CUSP_CORNERS];
    let limit_a = j_to_a(limit_j_max, reach.inv_cz);
    let mut min_i = 0;
    for i in 0..CUSP_CORNERS {
        let v = cusp_corner(i);
        let (mut lower, mut upper) = (0.0, forward_limit);
        while upper - lower > REACH_CUSP_TOL {
            let test = (lower + upper) / 2.0;
            let a = rgb_to_aab(v.map(|c| c * test), reach)[0];
            if a < limit_a { lower = test } else { upper = test }
        }
        t[i] = rgb_to_jmh(v.map(|c| c * upper), reach);
        if t[i][2] < t[min_i][2] {
            min_i = i;
        }
    }
    let mut o = [[0.0; 3]; TOTAL_CORNERS];
    for i in 0..CUSP_CORNERS {
        o[i + 1] = t[(i + min_i) % CUSP_CORNERS];
    }
    o[0] = o[CUSP_CORNERS];
    o[CUSP_CORNERS + 1] = o[1];
    o[0][2] -= HUE_LIMIT;
    o[CUSP_CORNERS + 1][2] += HUE_LIMIT;
    o
}

fn sorted_cube_hues(reach: &[V3; TOTAL_CORNERS], limit: &[V3; TOTAL_CORNERS]) -> [f64; MAX_SORTED] {
    let mut out = [0.0; MAX_SORTED];
    let (mut idx, mut ri, mut li) = (0, 1, 1);
    while ri < CUSP_CORNERS + 1 || li < CUSP_CORNERS + 1 {
        let rh = reach[ri][2];
        let lh = limit[li][2];
        if rh == lh {
            out[idx] = rh;
            ri += 1;
            li += 1;
        } else if rh < lh {
            out[idx] = rh;
            ri += 1;
        } else {
            out[idx] = lh;
            li += 1;
        }
        idx += 1;
    }
    out
}

fn hue_sample_interval(samples: i64, lower: f64, upper: f64, t: &mut [f64; TOTAL_TABLE], base: i64) {
    let delta = (upper - lower) / samples as f64;
    for i in 0..samples {
        t[(base + i) as usize] = lower + i as f64 * delta;
    }
}

fn build_hue_table(sorted: &[f64; MAX_SORTED]) -> [f64; TOTAL_TABLE] {
    let mut t = [0.0; TOTAL_TABLE];
    let ideal = TABLE_SIZE as f64 / HUE_LIMIT;
    let mut count = [0i64; 2 * CUSP_CORNERS + 2];
    let mut last_idx: i64 = 0;
    let mut min_index: i64 = if sorted[0] == 0.0 { 0 } else { 1 };
    for hi in 0..MAX_SORTED {
        let mut nominal = ((sorted[hi] * ideal).round() as i64).max(min_index).min(TABLE_SIZE as i64 - 1);
        if last_idx == nominal {
            if hi > 1 && count[hi - 2] != count[hi - 1] - 1 {
                count[hi - 1] -= 1;
            } else {
                nominal += 1;
            }
        }
        count[hi] = nominal.min(TABLE_SIZE as i64 - 1);
        min_index = nominal;
        last_idx = min_index;
    }
    let mut total: i64 = 0;
    hue_sample_interval(count[0], 0.0, sorted[0], &mut t, total + 1);
    total += count[0];
    let mut i = 1;
    while i != MAX_SORTED {
        let samples = count[i] - count[i - 1];
        hue_sample_interval(samples, sorted[i - 1], sorted[i], &mut t, total + 1);
        total += samples;
        i += 1;
    }
    hue_sample_interval(TABLE_SIZE as i64 - total, sorted[i - 1], HUE_LIMIT, &mut t, total + 1);
    t[0] = t[BASE_INDEX + TABLE_SIZE - 1] - HUE_LIMIT;
    t[BASE_INDEX + TABLE_SIZE] = t[BASE_INDEX] + HUE_LIMIT;
    t
}

fn display_cusp_for_hue(hue: f64, rgb_c: &[V3; TOTAL_CORNERS], jmh_c: &[V3; TOTAL_CORNERS], params: &JmhParams) -> [f64; 2] {
    let mut upper = 1;
    for i in 1..TOTAL_CORNERS {
        if jmh_c[i][2] > hue {
            upper = i;
            break;
        }
    }
    let lower = upper - 1;
    if jmh_c[lower][2] == hue {
        return [jmh_c[lower][0], jmh_c[lower][1]];
    }
    let (cl, cu) = (rgb_c[lower], rgb_c[upper]);
    let (mut lo_t, mut hi_t) = (0.0, 1.0);
    while hi_t - lo_t > DISPLAY_CUSP_TOL {
        let st = (lo_t + hi_t) / 2.0;
        let j = rgb_to_jmh(lerp3(cl, cu, st), params);
        if j[2] < jmh_c[lower][2] {
            hi_t = st;
        } else if j[2] >= jmh_c[upper][2] {
            lo_t = st;
        } else if j[2] > hue {
            hi_t = st;
        } else {
            lo_t = st;
        }
    }
    let st = (lo_t + hi_t) / 2.0;
    let j = rgb_to_jmh(lerp3(cl, cu, st), params);
    [j[0], j[1]]
}

fn build_cusp_table(hues: &[f64; TOTAL_TABLE], rgb_c: &[V3; TOTAL_CORNERS], jmh_c: &[V3; TOTAL_CORNERS], params: &JmhParams) -> [V3; TOTAL_TABLE] {
    let mut o = [[0.0; 3]; TOTAL_TABLE];
    for i in BASE_INDEX..TOTAL_TABLE {
        let jm = display_cusp_for_hue(hues[i], rgb_c, jmh_c, params);
        o[i] = [jm[0], jm[1] * (1.0 + SMOOTH_M * SMOOTH_CUSPS), hues[i]];
    }
    o[0] = [o[TABLE_SIZE][0], o[TABLE_SIZE][1], hues[0]];
    o[BASE_INDEX + TABLE_SIZE] = [o[BASE_INDEX][0], o[BASE_INDEX][1], hues[BASE_INDEX + TABLE_SIZE]];
    o
}

fn reach_m_table(params: &JmhParams, limit_j_max: f64) -> [f64; TOTAL_TABLE] {
    let mut t = [0.0; TOTAL_TABLE];
    let below = |v: V3| v[0] < 0.0 || v[1] < 0.0 || v[2] < 0.0;
    for i in 0..TABLE_SIZE {
        let hue = i as f64 * HUE_LIMIT / TABLE_SIZE as f64;
        let (range, maximum) = (50.0, 1300.0);
        let mut low = 0.0;
        let mut high = low + range;
        let mut outside = false;
        while !outside && high < maximum {
            outside = below(jmh_to_rgb([limit_j_max, high, hue], params));
            if !outside {
                low = high;
                high += range;
            }
        }
        while high - low > 1e-2 {
            let m = (high + low) / 2.0;
            if below(jmh_to_rgb([limit_j_max, m, hue], params)) { high = m } else { low = m }
        }
        t[i + BASE_INDEX] = high;
    }
    t[0] = t[TABLE_SIZE];
    t[BASE_INDEX + TABLE_SIZE] = t[BASE_INDEX];
    t
}

const TEST_POSITIONS: [f64; 5] = [0.01, 0.1, 0.5, 0.8, 0.99];

fn upper_hull_gamma_table(cusps: &[V3; TOTAL_TABLE], p: &OdtParams) -> [f64; TOTAL_TABLE] {
    let mut out = [0.0; TOTAL_TABLE];
    let lum_limit = p.peak / REF_LUMINANCE;
    for i in BASE_INDEX..BASE_INDEX + TABLE_SIZE {
        let hue = cusps[i][2];
        let cusp = [cusps[i][0], cusps[i][1]];
        let threshold = lerp(cusp[0], p.limit_j_max, FOCUS_GAIN_BLEND);
        let focus_j = focus_j_of(cusp[0], p.mid_j, p.limit_j_max);
        let tests: Vec<(f64, f64, f64, f64)> = TEST_POSITIONS
            .iter()
            .map(|&tp| {
                let tj = lerp(cusp[0], p.limit_j_max, tp);
                let sg = focus_gain(tj, threshold, p.limit_j_max, p.focus_dist);
                let ji = solve_j_intersect(tj, cusp[1], focus_j, p.limit_j_max, sg);
                let slope = compression_vector_slope(ji, focus_j, p.limit_j_max, sg);
                let jc = solve_j_intersect(cusp[0], cusp[1], focus_j, p.limit_j_max, sg);
                (tj, ji, slope, jc)
            })
            .collect();
        let fits = |top_inv: f64| {
            tests.iter().all(|&(_, ji, slope, jc)| {
                let m = gamut_boundary_m(cusp, p.limit_j_max, top_inv, p.lower_hull_gamma_inv, ji, slope, jc);
                let j = ji + slope * m;
                let rgb = jmh_to_rgb([j, m, hue], &p.limit);
                rgb[0] > lum_limit || rgb[1] > lum_limit || rgb[2] > lum_limit
            })
        };
        let mut low = GAMMA_MIN;
        let mut high = low + GAMMA_STEP;
        let mut outside = false;
        while !outside && high < GAMMA_MAX {
            if !fits(1.0 / high) {
                low = high;
                high += GAMMA_STEP;
            } else {
                outside = true;
            }
        }
        while high - low > GAMMA_ACCURACY {
            let g = (high + low) / 2.0;
            if fits(1.0 / g) { high = g } else { low = g }
        }
        out[i] = 1.0 / high;
    }
    out[0] = out[TABLE_SIZE];
    out[TABLE_SIZE + BASE_INDEX] = out[BASE_INDEX];
    out
}

fn hue_search_range(hues: &[f64; TOTAL_TABLE]) -> [i64; 2] {
    let mut r = [0i64, 1i64];
    for (i, &h) in hues.iter().enumerate().take(BASE_INDEX + TABLE_SIZE).skip(BASE_INDEX) {
        let pos = hue_position(h, TOTAL_TABLE);
        let delta = i as i64 - pos;
        r[0] = r[0].min(delta);
        r[1] = r[1].max(delta + 1);
    }
    r
}

impl OdtParams {
    /// `init_ODTParams(peakLuminance, limitingPrimaries)`.
    pub fn new(peak: f64, limiting: &Chromaticities) -> Self {
        let input = jmh_params(&AP0);
        let reach = jmh_params(&AP1);
        let limit = jmh_params(limiting);
        let ts = ts_params(peak);
        let limit_j_max = y_to_j(peak, &input);
        let model_gamma_inv = 1.0 / model_gamma();
        let reach_m = reach_m_table(&reach, limit_j_max);
        let sat = (CHROMA_EXPAND - (CHROMA_EXPAND * CHROMA_EXPAND_FACT) * ts.log_peak).max(0.2);
        let sat_thr = CHROMA_EXPAND_THR / peak;
        let compr = CHROMA_COMPRESS + (CHROMA_COMPRESS * CHROMA_COMPRESS_FACT) * ts.log_peak;
        let chroma_compress_scale = (0.03379 * peak).powf(0.30596) - 0.45135;
        let mid_j = y_to_j(ts.c_t * REF_LUMINANCE, &input);
        let focus_dist = FOCUS_DISTANCE + FOCUS_DISTANCE * FOCUS_DISTANCE_SCALING * ts.log_peak;
        let lower_hull_gamma_inv = 1.0 / (1.14 + 0.07 * ts.log_peak);
        let ap0_to_ap1 = mul_m(&rgb_to_xyz(&AP0, 1.0), &xyz_to_rgb(&AP1, 1.0));
        let ap1_to_ap0 = mul_m(&rgb_to_xyz(&AP1, 1.0), &xyz_to_rgb(&AP0, 1.0));
        let mut p = Self {
            peak,
            input,
            reach,
            limit,
            ts,
            limit_j_max,
            model_gamma_inv,
            reach_m,
            sat,
            sat_thr,
            compr,
            chroma_compress_scale,
            mid_j,
            focus_dist,
            lower_hull_gamma_inv,
            hues: [0.0; TOTAL_TABLE],
            cusps: [[0.0; 3]; TOTAL_TABLE],
            upper_hull_gamma: [0.0; TOTAL_TABLE],
            search_range: [0, 1],
            ap0_to_ap1,
            ap1_to_ap0,
        };
        let rc = reach_corners(&p.reach, p.limit_j_max, p.ts.forward_limit);
        let (lrgb, ljmh) = limiting_corners(&p.limit, p.peak);
        let sorted = sorted_cube_hues(&rc, &ljmh);
        let hue_table = build_hue_table(&sorted);
        p.cusps = build_cusp_table(&hue_table, &lrgb, &ljmh, &p.limit);
        for i in 0..TOTAL_TABLE {
            p.hues[i] = p.cusps[i][2];
        }
        p.upper_hull_gamma = upper_hull_gamma_table(&p.cusps, &p);
        p.search_range = hue_search_range(&p.hues);
        p
    }

    /// `outputTransform_fwd`: ACES2065-1 in, display-linear limiting-gamut RGB
    /// out (1.0 = the reference 100 nits), not yet clamped.
    pub fn forward(&self, aces: V3) -> V3 {
        let ap1 = mul_v(aces, &self.ap0_to_ap1).map(|v| v.clamp(0.0, self.ts.forward_limit));
        let ap0 = mul_v(ap1, &self.ap1_to_ap0);
        let jmh = rgb_to_jmh(ap0, &self.input);
        let tc = tonemap_and_compress_fwd(jmh, self);
        let gc = gamut_compress_fwd(tc, self);
        jmh_to_rgb(gc, &self.limit)
    }

    /// The whole sRGB output (the CTL's `main`): forward, clamped to the peak,
    /// encoded with the sRGB piecewise inverse EOTF. Limiting and encoding
    /// gamuts are both Rec.709 D65, so no limit-to-display matrix remains.
    pub fn srgb(&self, aces: V3) -> V3 {
        self.forward(aces).map(|v| encode_srgb(v.clamp(0.0, self.peak / REF_LUMINANCE)))
    }
}

/// `moncurve_inv(y, 2.4, 0.055)`: IEC 61966-2-1 sRGB encoding.
pub fn encode_srgb(y: f64) -> f64 {
    let (gamma, offs) = (2.4f64, 0.055f64);
    let yb = (offs * gamma / ((gamma - 1.0) * (1.0 + offs))).powf(gamma);
    let rs = ((gamma - 1.0) / offs).powf(gamma - 1.0) * ((1.0 + offs) / gamma).powf(gamma);
    if y >= yb { (1.0 + offs) * y.powf(1.0 / gamma) - offs } else { y * rs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tone_scale_puts_scene_grey_at_ten_nits_and_saturates_at_the_peak() {
        let p = ts_params(100.0);
        // c = 0.18 -> 10 cd/m2 (c_d = 10.013 is the anchor before the flare term)
        assert!((tonescale_fwd(0.18, &p) - 10.0).abs() < 0.01, "{}", tonescale_fwd(0.18, &p));
        // r_hit (128) is where the curve reaches the 100-nit roof
        let roof = tonescale_fwd(128.0, &p);
        assert!((roof - 100.0).abs() < 0.5, "{roof}");
        assert!(tonescale_fwd(0.0, &p) == 0.0);
    }

    #[test]
    fn aces_white_and_grey_come_out_neutral_and_on_the_tone_scale() {
        let o = OdtParams::new(100.0, &REC709);
        for v in [0.01, 0.18, 1.0, 4.0] {
            let d = o.forward([v; 3]);
            assert!((d[0] - d[1]).abs() < 1e-4 && (d[1] - d[2]).abs() < 1e-4, "{v}: {d:?}");
            let want = tonescale_fwd(v, &o.ts) / 100.0;
            assert!((d[1] - want).abs() < 1e-3 * want.max(0.01), "{v}: {} vs {want}", d[1]);
        }
    }

    #[test]
    fn saturated_colours_stay_inside_the_display_gamut_and_keep_their_hue() {
        let o = OdtParams::new(100.0, &REC709);
        let to_ap0 = rgb_to_rgb(&REC709, &AP0);
        for c in [[1.0, 0.05, 0.0], [0.9, 0.3, 0.02], [0.02, 0.1, 1.0], [0.1, 1.0, 0.1], [4.0, 0.4, 0.0], [0.02, 0.4, 3.0]] {
            let d = o.forward(mul_v(c, &to_ap0));
            // the gamut compressor is analytic: it may overshoot the peak a little for
            // scene values well over white; the output's clamp takes the rest (the CTL's main)
            assert!(d.iter().all(|&v| v > -2e-3 && v <= 1.1), "{c:?} -> {d:?}");
            // the hue in the display's own JMh stays near the scene colour's (no clipping skew)
            let hs = rgb_to_jmh(c, &o.limit)[2];
            let hd = rgb_to_jmh(d.map(|v| v.max(0.0)), &o.limit)[2];
            let dh = ((hd - hs + 540.0) % 360.0 - 180.0).abs();
            assert!(dh < 6.0, "{c:?}: hue {hs:.1} -> {hd:.1}");
        }
    }
}
