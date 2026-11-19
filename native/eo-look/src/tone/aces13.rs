//! The ACES 1.3 Reference Rendering Transform and the sRGB 100-nit
//! dim-surround Output Device Transform, ported from the Academy's
//! reference CTL (aces-dev v1.3: `RRT.ctl`, `ACESlib.RRT_Common.ctl`,
//! `ACESlib.Tonescales.ctl`, `ACESlib.ODT_Common.ctl`,
//! `ODT.Academy.sRGB_100nits_dim.ctl`; A.M.P.A.S. licence, see
//! eo-look/data/camera/ACES13.NOTICE.txt). Forward direction only.
//!
//! The licence's terms are not yet met in this repository: its text
//! (copyright notice, conditions, Disclaimer of Warranty) is not stored here
//! and awaits the owner's approval (see the NOTICE). Do not merge or
//! redistribute this file until it is.
//!
//! Unlike ACES 2.0 (`aces2`), the 1.x rendering applies its tone scale per
//! channel in the AP1 rendering space, so a colour keeps more of its
//! chroma through the mid-tones and gains it in the toe, and highlights
//! drift toward the AP1 primaries before they desaturate: the rendering
//! most cinema and game pipelines shipped from 2014 to 2024.
//!
//! Row-vector matrices, as the CTL (`aces2::mul_v`).

use super::aces2::{self, AP0, AP1, M33, REC709, V3};
use std::sync::OnceLock;

// ---- ACESlib.Tonescales.ctl ----

struct SplineC5 {
    coefs_low: [f64; 6],
    coefs_high: [f64; 6],
    min_point: [f64; 2],
    mid_point: [f64; 2],
    max_point: [f64; 2],
    slope_low: f64,
    slope_high: f64,
}

struct SplineC9 {
    coefs_low: [f64; 10],
    coefs_high: [f64; 10],
    min_point: [f64; 2],
    mid_point: [f64; 2],
    max_point: [f64; 2],
    slope_low: f64,
    slope_high: f64,
}

const RRT_PARAMS: SplineC5 = SplineC5 {
    coefs_low: [-4.0000000000, -4.0000000000, -3.1573765773, -0.4852499958, 1.8477324706, 1.8477324706],
    coefs_high: [-0.7185482425, 2.0810307172, 3.6681241237, 4.0000000000, 4.0000000000, 4.0000000000],
    min_point: [0.18 / 32768.0, 0.0001], // 0.18 * 2^-15
    mid_point: [0.18, 4.8],
    max_point: [0.18 * 262144.0, 10000.0], // 0.18 * 2^18
    slope_low: 0.0,
    slope_high: 0.0,
};

const ODT_48_LOW: [f64; 10] = [-1.6989700043, -1.6989700043, -1.4779000000, -1.2291000000, -0.8648000000, -0.4480000000, 0.0051800000, 0.4511080334, 0.9113744414, 0.9113744414];
const ODT_48_HIGH: [f64; 10] = [0.5154386965, 0.8470437783, 1.1358000000, 1.3802000000, 1.5197000000, 1.5985000000, 1.6467000000, 1.6746091357, 1.6878733390, 1.6878733390];

/// `M` of the quadratic B-spline segments.
const SPLINE_M: M33 = [[0.5, -1.0, 0.5], [-1.0, 1.0, 0.5], [0.5, 0.0, 0.0]];

/// The CTL's HALF_MIN (the smallest normal half float).
const HALF_MIN: f64 = 6.103_515_625e-5;

fn spline_segment(cf: [f64; 3], t: f64) -> f64 {
    let m = aces2::mul_v(cf, &SPLINE_M);
    t * t * m[0] + t * m[1] + m[2]
}

fn spline_fwd(x: f64, low: &[f64], high: &[f64], n_low: usize, n_high: usize, min: [f64; 2], mid: [f64; 2], max: [f64; 2], s_low: f64, s_high: f64) -> f64 {
    let logx = x.max(HALF_MIN).log10();
    let logy = if logx <= min[0].log10() {
        logx * s_low + (min[1].log10() - s_low * min[0].log10())
    } else if logx < mid[0].log10() {
        let kc = (n_low - 1) as f64 * (logx - min[0].log10()) / (mid[0].log10() - min[0].log10());
        let j = kc as usize;
        spline_segment([low[j], low[j + 1], low[j + 2]], kc - j as f64)
    } else if logx < max[0].log10() {
        let kc = (n_high - 1) as f64 * (logx - mid[0].log10()) / (max[0].log10() - mid[0].log10());
        let j = kc as usize;
        spline_segment([high[j], high[j + 1], high[j + 2]], kc - j as f64)
    } else {
        logx * s_high + (max[1].log10() - s_high * max[0].log10())
    };
    10f64.powf(logy)
}

/// `segmented_spline_c5_fwd` with `RRT_PARAMS`.
pub fn segmented_spline_c5_fwd(x: f64) -> f64 {
    let c = &RRT_PARAMS;
    spline_fwd(x, &c.coefs_low, &c.coefs_high, 4, 4, c.min_point, c.mid_point, c.max_point, c.slope_low, c.slope_high)
}

fn odt_48() -> &'static SplineC9 {
    static C: OnceLock<SplineC9> = OnceLock::new();
    C.get_or_init(|| SplineC9 {
        coefs_low: ODT_48_LOW,
        coefs_high: ODT_48_HIGH,
        min_point: [segmented_spline_c5_fwd(0.18 * 2f64.powf(-6.5)), 0.02],
        mid_point: [segmented_spline_c5_fwd(0.18), 4.8],
        max_point: [segmented_spline_c5_fwd(0.18 * 2f64.powf(6.5)), 48.0],
        slope_low: 0.0,
        slope_high: 0.04,
    })
}

/// `segmented_spline_c9_fwd` with `ODT_48nits`.
pub fn segmented_spline_c9_fwd(x: f64) -> f64 {
    let c = odt_48();
    spline_fwd(x, &c.coefs_low, &c.coefs_high, 8, 8, c.min_point, c.mid_point, c.max_point, c.slope_low, c.slope_high)
}

// ---- ACESlib.Utilities_Color.ctl, ACESlib.RRT_Common.ctl ----

const RRT_GLOW_GAIN: f64 = 0.05;
const RRT_GLOW_MID: f64 = 0.08;
const RRT_RED_SCALE: f64 = 0.82;
const RRT_RED_PIVOT: f64 = 0.03;
const RRT_RED_HUE: f64 = 0.0;
const RRT_RED_WIDTH: f64 = 135.0;
const RRT_SAT_FACTOR: f64 = 0.96;
const ODT_SAT_FACTOR: f64 = 0.93;
const DIM_SURROUND_GAMMA: f64 = 0.9811;
const CINEMA_WHITE: f64 = 48.0;
const CINEMA_BLACK: f64 = 0.02;

fn rgb_2_saturation(rgb: V3) -> f64 {
    const TINY: f64 = 1e-10;
    let mx = rgb[0].max(rgb[1]).max(rgb[2]);
    let mn = rgb[0].min(rgb[1]).min(rgb[2]);
    (mx.max(TINY) - mn.max(TINY)) / mx.max(1e-2)
}

fn rgb_2_yc(rgb: V3) -> f64 {
    const YC_RADIUS_WEIGHT: f64 = 1.75;
    let [r, g, b] = rgb;
    let chroma = (b * (b - g) + g * (g - r) + r * (r - b)).max(0.0).sqrt();
    (b + g + r + YC_RADIUS_WEIGHT * chroma) / 3.0
}

fn sigmoid_shaper(x: f64) -> f64 {
    let t = (1.0 - (x / 2.0).abs()).max(0.0);
    let y = 1.0 + x.signum() * (1.0 - t * t);
    y / 2.0
}

fn glow_fwd(yc_in: f64, glow_gain_in: f64, glow_mid: f64) -> f64 {
    if yc_in <= 2.0 / 3.0 * glow_mid {
        glow_gain_in
    } else if yc_in >= 2.0 * glow_mid {
        0.0
    } else {
        glow_gain_in * (glow_mid / yc_in - 0.5)
    }
}

fn rgb_2_hue(rgb: V3) -> f64 {
    let [r, g, b] = rgb;
    if r == g && g == b {
        return 0.0;
    }
    let h = (3f64.sqrt() * (g - b)).atan2(2.0 * r - g - b).to_degrees();
    if h < 0.0 { h + 360.0 } else { h }
}

fn center_hue(hue: f64, center: f64) -> f64 {
    let h = hue - center;
    if h < -180.0 {
        h + 360.0
    } else if h > 180.0 {
        h - 360.0
    } else {
        h
    }
}

fn cubic_basis_shaper(x: f64, w: f64) -> f64 {
    const M: [[f64; 4]; 4] = [
        [-1.0 / 6.0, 3.0 / 6.0, -3.0 / 6.0, 1.0 / 6.0],
        [3.0 / 6.0, -6.0 / 6.0, 3.0 / 6.0, 0.0],
        [-3.0 / 6.0, 0.0, 3.0 / 6.0, 0.0],
        [1.0 / 6.0, 4.0 / 6.0, 1.0 / 6.0, 0.0],
    ];
    let knots = [-w / 2.0, -w / 4.0, 0.0, w / 4.0, w / 2.0];
    let mut y = 0.0;
    if x > knots[0] && x < knots[4] {
        let kc = (x - knots[0]) * 4.0 / w;
        let j = kc as usize;
        let t = kc - j as f64;
        let mono = [t * t * t, t * t, t, 1.0];
        let col = match j {
            3 => 0,
            2 => 1,
            1 => 2,
            _ => 3,
        };
        if j <= 3 {
            y = (0..4).map(|k| mono[k] * M[k][col]).sum();
        }
    }
    y * 1.5
}

/// `calc_sat_adjust_matrix` (row-vector form): out = sat x in + (1 - sat) x Y(in).
fn sat_matrix(sat: f64, rgb2y: V3) -> M33 {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            // row-vector: out_j = sum_i in_i m[i][j]
            m[i][j] = (1.0 - sat) * rgb2y[i] + if i == j { sat } else { 0.0 };
        }
    }
    m
}

struct Mats {
    ap0_to_ap1: M33,
    ap1_to_ap0: M33,
    ap1_to_xyz: M33,
    xyz_to_ap1: M33,
    rrt_sat: M33,
    odt_sat: M33,
    d60_to_d65: M33,
    xyz_to_rec709: M33,
}

fn mats() -> &'static Mats {
    static M: OnceLock<Mats> = OnceLock::new();
    M.get_or_init(|| {
        let ap0_to_xyz = aces2::rgb_to_xyz(&AP0, 1.0);
        let ap1_to_xyz = aces2::rgb_to_xyz(&AP1, 1.0);
        let xyz_to_ap1 = aces2::xyz_to_rgb(&AP1, 1.0);
        let rgb2y = [ap1_to_xyz[0][1], ap1_to_xyz[1][1], ap1_to_xyz[2][1]];
        Mats {
            ap0_to_ap1: aces2::mul_m(&ap0_to_xyz, &xyz_to_ap1),
            ap1_to_ap0: aces2::mul_m(&ap1_to_xyz, &aces2::xyz_to_rgb(&AP0, 1.0)),
            ap1_to_xyz,
            xyz_to_ap1,
            rrt_sat: sat_matrix(RRT_SAT_FACTOR, rgb2y),
            odt_sat: sat_matrix(ODT_SAT_FACTOR, rgb2y),
            d60_to_d65: aces2::cat_matrix(AP0.white, REC709.white),
            xyz_to_rec709: aces2::xyz_to_rgb(&REC709, 1.0),
        }
    })
}

/// `RRT.ctl`: ACES2065-1 to OCES.
pub fn rrt(aces_in: V3) -> V3 {
    let m = mats();
    let mut aces = aces_in;
    // glow
    let saturation = rgb_2_saturation(aces);
    let yc_in = rgb_2_yc(aces);
    let s = sigmoid_shaper((saturation - 0.4) / 0.2);
    let added_glow = 1.0 + glow_fwd(yc_in, RRT_GLOW_GAIN * s, RRT_GLOW_MID);
    aces = aces.map(|v| v * added_glow);
    // red modifier
    let hue = rgb_2_hue(aces);
    let centered = center_hue(hue, RRT_RED_HUE);
    let hue_weight = cubic_basis_shaper(centered, RRT_RED_WIDTH);
    aces[0] += hue_weight * saturation * (RRT_RED_PIVOT - aces[0]) * (1.0 - RRT_RED_SCALE);
    // to the rendering space
    let aces = aces.map(|v| v.clamp(0.0, 65504.0));
    let pre = aces2::mul_v(aces, &m.ap0_to_ap1).map(|v| v.clamp(0.0, 65504.0));
    let pre = aces2::mul_v(pre, &m.rrt_sat);
    let post = pre.map(segmented_spline_c5_fwd);
    aces2::mul_v(post, &m.ap1_to_ap0)
}

/// `ODT.Academy.sRGB_100nits_dim.ctl` up to the encoding: OCES to display-linear Rec.709, 0..1.
pub fn odt_srgb_100_dim_linear(oces: V3) -> V3 {
    let m = mats();
    let pre = aces2::mul_v(oces, &m.ap0_to_ap1);
    let post = pre.map(segmented_spline_c9_fwd);
    let lin = post.map(|y| (y - CINEMA_BLACK) / (CINEMA_WHITE - CINEMA_BLACK));
    // darkSurround_to_dimSurround
    let xyz = aces2::mul_v(lin, &m.ap1_to_xyz);
    let sum = xyz[0] + xyz[1] + xyz[2];
    let (x, y) = if sum.abs() > 1e-12 { (xyz[0] / sum, xyz[1] / sum) } else { (0.0, 0.0) };
    let yy = xyz[1].max(0.0).powf(DIM_SURROUND_GAMMA);
    let xyz = if y.abs() > 1e-12 { [x * yy / y, yy, (1.0 - x - y) * yy / y] } else { [0.0, 0.0, 0.0] };
    let lin = aces2::mul_v(xyz, &m.xyz_to_ap1);
    let lin = aces2::mul_v(lin, &m.odt_sat);
    let xyz = aces2::mul_v(aces2::mul_v(lin, &m.ap1_to_xyz), &m.d60_to_d65);
    aces2::mul_v(xyz, &m.xyz_to_rec709).map(|v| v.clamp(0.0, 1.0))
}

/// RRT then the sRGB ODT: ACES2065-1 to display-linear Rec.709.
pub fn forward(aces: V3) -> V3 {
    odt_srgb_100_dim_linear(rrt(aces))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_splines_meet_their_anchor_points() {
        // RRT: 0.18 -> 4.8 nits (OCES); ODT: 4.8 -> 4.8 nits
        assert!((segmented_spline_c5_fwd(0.18) - 4.8).abs() < 1e-6, "{}", segmented_spline_c5_fwd(0.18));
        assert!((segmented_spline_c9_fwd(4.8) - 4.8).abs() < 1e-6, "{}", segmented_spline_c9_fwd(4.8));
        // monotone over the useful range
        let mut last = 0.0;
        for i in 0..200 {
            let x = 0.18 * 2f64.powf(-12.0 + i as f64 * 0.12);
            let y = segmented_spline_c9_fwd(segmented_spline_c5_fwd(x));
            assert!(y >= last, "{x}");
            last = y;
        }
    }

    #[test]
    fn grey_is_neutral_and_middle_grey_lands_near_ten_percent() {
        // the published ACES 1.x behaviour: scene 0.18 at ~10 % of the SDR display's white
        let a = aces2::mul_v([0.18; 3], &aces2::rgb_to_rgb(&REC709, &AP0));
        let d = forward(a);
        assert!((d[0] - d[2]).abs() < 2e-3 && (d[0] - d[1]).abs() < 2e-3, "{d:?}");
        assert!((0.08..0.12).contains(&d[1]), "{d:?}");
        // a 16-stop-over white clips; black stays black
        let w = forward(aces2::mul_v([0.18 * 65536.0; 3], &aces2::rgb_to_rgb(&REC709, &AP0)));
        assert!(w.iter().all(|&v| v > 0.99), "{w:?}");
        assert!(forward([0.0; 3]).iter().all(|&v| v < 1e-3));
    }
}
