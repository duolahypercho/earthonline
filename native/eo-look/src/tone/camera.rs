//! The camera look (`--no-camera-look` keeps the filmic transform): the
//! rendering a stills camera applies between the scene and the picture,
//! built from cited parts and baked once into a 3D table the post pass taps.
//!
//! 1. **White balance**: daylight, D55 to D65 by Bradford (`filmic_white_balance`,
//!    unchanged), so sunlit white reads white and shade reads cool.
//! 2. **Exposure placement** (ISO 12232:2019, Standard Output Sensitivity): a
//!    camera rated at S renders the focal-plane exposure H = 10 / S lx s at
//!    sRGB 118 (display Y 0.181). A meter (ISO 2720) gives a metered
//!    luminance the exposure H = q K / S, q = 0.65, K = 12.5, i.e. 0.8125 x
//!    the SOS exposure. The look's exposure puts the metered grey at scene
//!    value 0.18 (`phys::exposure_at_ev100`), so the camera renders scene
//!    `SOS_SCENE_VALUE` = 0.18 / 0.8125 = 0.2215 at display Y 0.181. The
//!    ACES tone scale alone puts 0.18 at 10 nits (Y 0.10, a cinema grey in a
//!    dim surround); `placement()` is the scene gain that moves the SOS
//!    point onto sRGB 118, solved on the neutral axis.
//! 3. **Tone and gamut** (the ACES 2.0 output transform, `aces2`): the
//!    Michaelis-Menten tone scale with its flare toe, the Hellwig 2022 JMh
//!    chroma compression (highlights desaturate toward white along constant
//!    hue; shadows and mid-tones gain colourfulness) and the gamut
//!    compression into Rec.709. Scene Rec.709 enters as ACES2065-1 through
//!    a Bradford D65-to-D60 conversion (Lib.Academy.ColorSpaces).
//! 4. **Encoding**: sRGB piecewise (IEC 61966-2-1).
//!
//!    Opt-in (`--camera-aces13`): the ACES 1.3 RRT and sRGB 100-nit dim ODT
//!    (`aces13`) in its place, at the same SOS placement: a per-channel tone
//!    scale in AP1 that keeps more chroma (0.98 of colorimetric on the
//!    virtual chart against ACES 2.0's 0.75) at the cost of hue skews (up to
//!    ~11 deg on the chart's blues; ACES 2.0 1.6 deg).
//!
//! No picture-style matrix and no hue skew are added: no published camera
//! picture style or JPEG tone curve under a licence this repository can
//! take was found (the measured camera base curves that exist are GPL or
//! under a vendor SDK licence), and adding either from the reference
//! photographs would be a fit to screenshots (stop-and-ask).
//!
//! The table: `LUT_SIZE`^3 RGBA texels over a per-channel log2 shaper of the
//! post pass's input (radiance x white balance x exposure / 0.6), from
//! `SHAPER_LO_EV` to `SHAPER_HI_EV` stops about 0.18, holding sRGB-encoded
//! display values; the GPU interpolates it trilinearly.

use super::aces2::{self, OdtParams, REC709};
use crate::math::*;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The scene value a meter's grey (0.18) sits under the SOS point: 0.18 / (q K / 10).
pub const ISO_Q: f64 = 0.65;
pub const ISO_K: f64 = 12.5;
pub const SOS_SCENE_VALUE: f64 = 0.18 / (ISO_Q * ISO_K / 10.0);
/// sRGB 118 of 255, display linear (ISO 12232 SOS reference output).
pub fn sos_display_y() -> f64 {
    srgb_to_linear(118.0 / 255.0)
}

/// 65 (not 33): over the 24-stop shaper a 33^3 table misses the exact
/// transform by 0.47 8-bit levels on average and 8.9 at worst on a random
/// sweep of scene colours, 65^3 by 0.13 and 2.8 (`the_table_tracks_the_transform`);
/// 2.2 MB of RGBA16F.
pub const LUT_SIZE: usize = 65;
pub const SHAPER_LO_EV: f64 = -14.0;
pub const SHAPER_HI_EV: f64 = 10.0;
const SHAPER_MID: f64 = 0.18;

/// The tone and gamut rendering the camera look bakes (step 3 above).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rendering {
    /// The ACES 2.0 output transform, Rec.709 100 nits, sRGB (`aces2`; the default).
    Aces2,
    /// The ACES 1.3 RRT and the sRGB 100-nit dim-surround ODT (`aces13`;
    /// opt-in, `--camera-aces13`): the per-channel AP1 tone scale.
    Aces13,
}

pub struct Camera {
    odt: Option<OdtParams>,
    to_ap0: aces2::M33,
    pub rendering: Rendering,
    /// the scene gain that puts `SOS_SCENE_VALUE` at sRGB 118.
    pub gain: f64,
}

impl Camera {
    fn build() -> Self {
        Self::with(if crate::features::colour().aces13 { Rendering::Aces13 } else { Rendering::Aces2 })
    }

    /// The camera with a given rendering (the process's is `camera()`).
    pub fn with(rendering: Rendering) -> Self {
        let odt = match rendering {
            Rendering::Aces2 => Some(OdtParams::new(100.0, &REC709)),
            Rendering::Aces13 => None,
        };
        let to_ap0 = aces2::rgb_to_rgb(&REC709, &aces2::AP0);
        let mut c = Self { odt, to_ap0, rendering, gain: 1.0 };
        // bisect the neutral axis for display Y = sRGB 118
        let target = sos_display_y();
        let (mut lo, mut hi) = (0.0f64, 8.0f64.ln());
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            c.gain = mid.exp();
            if c.display_linear([SOS_SCENE_VALUE; 3])[1] < target { lo = mid } else { hi = mid }
        }
        c.gain = (0.5 * (lo + hi)).exp();
        c
    }

    /// Scene-linear Rec.709 (after white balance and exposure, metered grey at
    /// 0.18) to display-linear Rec.709, clamped to 0..1.
    pub fn display_linear(&self, x: Rgb) -> Rgb {
        let a = aces2::mul_v(x.map(|v| v.max(0.0) * self.gain), &self.to_ap0);
        match &self.odt {
            Some(odt) => odt.forward(a).map(|v| v.clamp(0.0, 1.0)),
            None => super::aces13::forward(a),
        }
    }

    /// As `display_linear`, sRGB encoded.
    pub fn display_srgb(&self, x: Rgb) -> Rgb {
        self.display_linear(x).map(aces2::encode_srgb)
    }
}

pub fn camera() -> &'static Camera {
    static C: OnceLock<Camera> = OnceLock::new();
    C.get_or_init(Camera::build)
}

/// The shaper: scene value -> table coordinate 0..1.
pub fn shaper(v: f64) -> f64 {
    let e = (v.max(1e-30) / SHAPER_MID).log2();
    ((e - SHAPER_LO_EV) / (SHAPER_HI_EV - SHAPER_LO_EV)).clamp(0.0, 1.0)
}

pub fn shaper_inv(u: f64) -> f64 {
    SHAPER_MID * (SHAPER_LO_EV + u * (SHAPER_HI_EV - SHAPER_LO_EV)).exp2()
}

/// The baked table, r fastest then g then b (a 3D texture's row order),
/// sRGB-encoded RGB per texel. Built once, on the threads available.
pub fn lut() -> &'static [[f32; 3]] {
    static L: OnceLock<Vec<[f32; 3]>> = OnceLock::new();
    L.get_or_init(|| bake(LUT_SIZE))
}

pub fn bake(n: usize) -> Vec<[f32; 3]> {
    let cam = camera();
    let coord = |i: usize| shaper_inv(i as f64 / (n - 1) as f64);
    // the black end of the shaper stands for zero
    let val = |i: usize| if i == 0 { 0.0 } else { coord(i) };
    let mut out = vec![[0.0f32; 3]; n * n * n];
    let threads = std::thread::available_parallelism().map(|t| t.get()).unwrap_or(4).clamp(1, 16);
    let per = n.div_ceil(threads);
    std::thread::scope(|s| {
        for (k, chunk) in out.chunks_mut(per * n * n).enumerate() {
            s.spawn(move || {
                for (o, px) in chunk.iter_mut().enumerate() {
                    let idx = k * per * n * n + o;
                    let (r, g, b) = (idx % n, (idx / n) % n, idx / (n * n));
                    let d = cam.display_srgb([val(r), val(g), val(b)]);
                    *px = [d[0] as f32, d[1] as f32, d[2] as f32];
                }
            });
        }
    });
    out
}

/// The table as the GPU samples it (trilinear between texel centres), for tests.
pub fn lut_sample(t: &[[f32; 3]], n: usize, x: Rgb) -> Rgb {
    let u = x.map(|v| if v <= 0.0 { 0.0 } else { shaper(v) * (n - 1) as f64 });
    let i0 = u.map(|v| (v.floor() as usize).min(n - 2));
    let f = [0, 1, 2].map(|k| u[k] - i0[k] as f64);
    let at = |r: usize, g: usize, b: usize| t[r + g * n + b * n * n];
    let mut o = [0.0; 3];
    for (dr, wr) in [(0, 1.0 - f[0]), (1, f[0])] {
        for (dg, wg) in [(0, 1.0 - f[1]), (1, f[1])] {
            for (db, wb) in [(0, 1.0 - f[2]), (1, f[2])] {
                let v = at(i0[0] + dr, i0[1] + dg, i0[2] + db);
                for c in 0..3 {
                    o[c] += wr * wg * wb * v[c] as f64;
                }
            }
        }
    }
    o
}

static CAMERA_ON: AtomicBool = AtomicBool::new(false);

/// Which display transform the frame is drawn with (the renderer sets it
/// from `--no-camera-look`), for the night exposure solve. Off by default.
pub fn set_camera_look(on: bool) {
    CAMERA_ON.store(on, Ordering::Relaxed);
}

pub fn camera_look_on() -> bool {
    CAMERA_ON.load(Ordering::Relaxed)
}

/// The camera look on one pixel (CPU reference of look_camera.wgsl, exact,
/// no table, no dither): sRGB-encoded display values.
pub fn display_camera(c: Rgb, exposure: f64) -> Rgb {
    let wb = super::filmic_white_balance();
    let w = [0, 1, 2].map(|i| wb[i][0] * c[0] + wb[i][1] * c[1] + wb[i][2] * c[2]);
    camera().display_srgb(w.map(|v| v * exposure / 0.6))
}

/// The virtual test chart's neutral ramp: six reflectances from a near-white
/// to a near-black card, halving about every stop and a quarter (0.90 to
/// 0.03, the span of a printed grey scale; INFERRED, analytic, no measured
/// chart data).
pub const NEUTRAL_RAMP: [f64; 6] = [0.90, 0.59, 0.36, 0.19, 0.09, 0.031];

/// The virtual test chart's chromatic patches, defined analytically in
/// CIELAB (D65 white): twelve hues 30 deg apart at L* 50 and at L* 70, each
/// at chroma C* 25 (inside the Rec.709 gamut as a reflectance for every hue;
/// INFERRED, analytic, no measured chart data). Linear Rec.709 reflectance.
pub fn chart_patches() -> Vec<(f64, f64, Rgb)> {
    let mut v = Vec::new();
    for l in [50.0, 70.0] {
        for k in 0..12 {
            let h = k as f64 * 30.0;
            v.push((l, h, lab_to_rec709([l, 25.0 * h.to_radians().cos(), 25.0 * h.to_radians().sin()])));
        }
    }
    v
}

/// CIELAB (D65 white) to linear Rec.709 (IEC 61966-2-1 primaries).
pub fn lab_to_rec709(lab: Rgb) -> Rgb {
    let w = [0.3127 / 0.3290, 1.0, (1.0 - 0.3127 - 0.3290) / 0.3290];
    let fy = (lab[0] + 16.0) / 116.0;
    let (fx, fz) = (fy + lab[1] / 500.0, fy - lab[2] / 200.0);
    let inv = |t: f64| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let xyz = [w[0] * inv(fx), w[1] * inv(fy), w[2] * inv(fz)];
    aces2::mul_v(xyz, &aces2::xyz_to_rgb(&REC709, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab(rgb_lin: Rgb) -> Rgb {
        // linear Rec.709 -> XYZ (D65) -> CIELAB (D65 white)
        let m = aces2::rgb_to_xyz(&REC709, 1.0);
        let xyz = aces2::mul_v(rgb_lin, &m);
        let w = [0.3127 / 0.3290, 1.0, (1.0 - 0.3127 - 0.3290) / 0.3290];
        let f = |t: f64| if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 };
        let (fx, fy, fz) = (f(xyz[0] / w[0]), f(xyz[1] / w[1]), f(xyz[2] / w[2]));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    #[test]
    fn the_sos_point_lands_at_srgb_118() {
        let d = camera().display_srgb([SOS_SCENE_VALUE; 3]);
        assert!((d[1] * 255.0 - 118.0).abs() < 0.2, "{}", d[1] * 255.0);
        // the gain is about half a stop (ACES's 10-nit cinema grey at 0.18 against a
        // stills camera's 118 at 0.2215)
        let g = camera().gain;
        assert!((1.2..2.0).contains(&g), "{g}");
    }

    /// The virtual chart (analytic patches: `NEUTRAL_RAMP`, `chart_patches`)
    /// lit by an even daylight that an incident meter reads (a card of
    /// reflectance rho at scene 0.18 x rho / 0.157, the meter's grey at 0.18),
    /// white-balanced. The camera must render the neutral ramp neutral,
    /// ordered, unclipped and with the SOS grey near L* 50, and keep each
    /// chromatic patch's hue.
    #[test]
    fn the_virtual_chart_renders_neutral_greys_and_keeps_hues() {
        for r in [Rendering::Aces2, Rendering::Aces13] {
            let cam = Camera::with(r);
            // incident meter (C = 250) and reflected meter (K = 12.5): the metered grey of an
            // illuminance E is K E / C, so a card of reflectance rho has scene value 0.18 rho / (pi K / C)
            let k = 0.18 / (std::f64::consts::PI * ISO_K / 250.0);
            let mut last_l = 200.0;
            for (i, &rho) in NEUTRAL_RAMP.iter().enumerate() {
                let d = cam.display_linear([rho * k; 3]);
                let l = lab(d);
                assert!(l[1].abs() < 1.5 && l[2].abs() < 2.5, "{r:?} grey {}: {l:?}", i + 1);
                assert!(l[0] < last_l, "{r:?} ramp order at {}", i + 1);
                last_l = l[0];
                assert!(d.iter().all(|&v| v < 0.98), "{r:?} grey {} clips: {d:?}", i + 1);
            }
            // the 0.19 card (the ramp's fourth) at about the SOS grey
            let n5 = lab(cam.display_linear([NEUTRAL_RAMP[3] * k; 3]));
            assert!((45.0..56.0).contains(&n5[0]), "{r:?} 0.19 card L* {}", n5[0]);
            // the 0.031 card keeps tone above the flare toe
            let blk = lab(cam.display_linear([NEUTRAL_RAMP[5] * k; 3]));
            assert!(blk[0] > 8.0, "{r:?} black L* {}", blk[0]);
            // chromatic patches keep their CIELAB hue: within 8 deg under ACES 2.0 (hue-preserving
            // by design); ACES 1.3's per-channel tone scale skews hue (its known skews: blues
            // toward purple, ~11 deg here), bounded at 15. The mean chroma ratio over a
            // colorimetric rendering is reported (a camera's look).
            let hue_tol = if r == Rendering::Aces2 { 8.0 } else { 15.0 };
            let mut worst_dh: f64 = 0.0;
            let mut chroma_gain = 0.0;
            let patches = chart_patches();
            for (lp, hp, rho) in &patches {
                assert!(rho.iter().all(|&v| (0.0..=1.0).contains(&v)), "{lp} {hp}: {rho:?}");
                let rr = lab(*rho);
                let d = lab(cam.display_linear(rho.map(|v| v * k)));
                let hr = rr[2].atan2(rr[1]).to_degrees();
                let hd = d[2].atan2(d[1]).to_degrees();
                let dh = ((hd - hr + 540.0) % 360.0 - 180.0).abs();
                assert!(dh < hue_tol, "{r:?} L* {lp} h {hp}: hue {hr:.1} -> {hd:.1}");
                worst_dh = worst_dh.max(dh);
                chroma_gain += (d[1].hypot(d[2]) / rr[1].hypot(rr[2])) / patches.len() as f64;
            }
            // the old chart's bound was 0.85 (the measured chart's saturated patches, 0.89 under
            // ACES 2.0); these low-chroma patches (C* 25) render at 0.75 under ACES 2.0: its
            // chroma compression takes more from weak colours. Reported, and bounded only
            // against a rendering that greys colour out or overshoots.
            assert!((0.7..1.6).contains(&chroma_gain), "{r:?} mean chroma ratio {chroma_gain}");
            eprintln!("virtual chart, {r:?}: 0.19 card L* {:.1}, 0.031 card L* {:.1}, mean chroma ratio {chroma_gain:.2}, worst hue shift {worst_dh:.1} deg", n5[0], blk[0]);
        }
    }

    #[test]
    fn the_aces13_rendering_meets_the_same_placement_and_keeps_greys_neutral() {
        let cam = Camera::with(Rendering::Aces13);
        let d = cam.display_srgb([SOS_SCENE_VALUE; 3]);
        assert!((d[1] * 255.0 - 118.0).abs() < 0.2, "{}", d[1] * 255.0);
        for ev in [-6.0, -3.0, 0.0, 2.0, 4.0] {
            let g = cam.display_linear([0.18 * 2f64.powf(ev); 3]);
            assert!((g[0] - g[1]).abs() < 3e-3 && (g[2] - g[1]).abs() < 3e-3, "{ev}: {g:?}");
        }
        // the per-channel AP1 tone scale keeps more chroma through the mid-tones than
        // ACES 2.0: a clear-sky blue (Rec.709 (0.33, 0.6, 1.0) x 0.1) renders more saturated
        let a2 = Camera::with(Rendering::Aces2);
        let sky = [0.33, 0.6, 1.0].map(|v| v * 0.1);
        let sat = |c: Rgb| {
            let mx = c[0].max(c[1]).max(c[2]);
            (mx - c[0].min(c[1]).min(c[2])) / mx
        };
        let (s13, s2) = (sat(cam.display_srgb(sky)), sat(a2.display_srgb(sky)));
        eprintln!("clear-sky blue: HSV S {s13:.2} (ACES 1.3) against {s2:.2} (ACES 2.0)");
        assert!(s13 > s2, "{s13} {s2}");
    }

    #[test]
    fn the_table_tracks_the_transform() {
        let n = LUT_SIZE;
        let t = lut();
        assert_eq!(t.len(), n * n * n);
        let cam = camera();
        // a sweep of scene colours over the shaper's useful range
        let mut worst: f64 = 0.0;
        let mut sum = 0.0;
        let mut count = 0.0;
        let mut s = 12345u64;
        let mut rnd = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..4000 {
            let ev = -8.0 + 12.0 * rnd();
            let c = [rnd(), rnd(), rnd()].map(|v| 0.18 * ev.exp2() * (0.02 + v));
            let a = cam.display_srgb(c);
            let b = lut_sample(t, n, c);
            let e = (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f64::max) * 255.0;
            worst = worst.max(e);
            sum += e;
            count += 1.0;
        }
        let mean = sum / count;
        assert!(mean < 0.25 && worst < 5.0, "8-bit error mean {mean:.2} worst {worst:.2}");
    }

    #[test]
    fn the_shaper_round_trips() {
        for v in [1e-4, 0.01, 0.18, 3.0, 100.0] {
            assert!((shaper_inv(shaper(v)) / v - 1.0).abs() < 1e-9);
        }
    }
}
