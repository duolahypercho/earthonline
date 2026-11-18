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
//! No picture-style matrix and no hue skew are added: no published,
//! citable camera picture style was found, and the ACES transform keeps
//! hue by design. Adding either from the reference photographs would be a
//! fit to screenshots (stop-and-ask).
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

pub struct Camera {
    odt: OdtParams,
    to_ap0: aces2::M33,
    /// the scene gain that puts `SOS_SCENE_VALUE` at sRGB 118.
    pub gain: f64,
}

impl Camera {
    fn build() -> Self {
        let odt = OdtParams::new(100.0, &REC709);
        let to_ap0 = aces2::rgb_to_rgb(&REC709, &aces2::AP0);
        let mut c = Self { odt, to_ap0, gain: 1.0 };
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
        self.odt.forward(a).map(|v| v.clamp(0.0, 1.0))
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

/// The BabelColor average of 30 ColorChecker Classic charts (CIE xyY, ICC
/// D50, 2 deg observer; BabelColor 2012, "ColorChecker RGB and spectra",
/// as tabulated in the colour-science package's `CCS_BABELCOLOR_AVERAGE`;
/// see eo-look/data/camera/COLORCHECKER.NOTICE.txt). Patches 1-24 in chart order.
pub const COLORCHECKER_XYY_D50: [[f64; 3]; 24] = [
    [0.4325, 0.3788, 0.1034],
    [0.4191, 0.3748, 0.3525],
    [0.2761, 0.3004, 0.1847],
    [0.3700, 0.4501, 0.1335],
    [0.3020, 0.2877, 0.2324],
    [0.2856, 0.3910, 0.4174],
    [0.5291, 0.4075, 0.3117],
    [0.2339, 0.2155, 0.1140],
    [0.5008, 0.3293, 0.1979],
    [0.3326, 0.2556, 0.0644],
    [0.3989, 0.4998, 0.4435],
    [0.4962, 0.4428, 0.4358],
    [0.2040, 0.1696, 0.0579],
    [0.3270, 0.5033, 0.2307],
    [0.5709, 0.3298, 0.1268],
    [0.4694, 0.4732, 0.6081],
    [0.4177, 0.2704, 0.2007],
    [0.2151, 0.3037, 0.1903],
    [0.3488, 0.3628, 0.9129],
    [0.3451, 0.3596, 0.5885],
    [0.3446, 0.3590, 0.3595],
    [0.3438, 0.3589, 0.1912],
    [0.3423, 0.3576, 0.0893],
    [0.3439, 0.3565, 0.0320],
];

/// ICC D50 white (x, y).
pub const ICC_D50_XY: [f64; 2] = [0.3457, 0.3585];

/// A patch's reflectance as linear Rec.709 under a D65-balanced capture
/// (Bradford D50 to D65, IEC 61966-2-1 primaries).
pub fn colorchecker_rgb(i: usize) -> Rgb {
    let [x, y, yy] = COLORCHECKER_XYY_D50[i];
    let xyz = [x * yy / y, yy, (1.0 - x - y) * yy / y];
    let cat = aces2::cat_matrix(ICC_D50_XY, [0.3127, 0.3290]);
    let xyz65 = aces2::mul_v(xyz, &cat);
    let to_rgb = aces2::xyz_to_rgb(&REC709, 1.0);
    aces2::mul_v(xyz65, &to_rgb)
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

    /// The virtual ColorChecker: the 24 BabelColor patches lit by an even
    /// daylight that an incident meter reads (the card of reflectance rho at
    /// scene 0.18 x rho / 0.157, the meter's grey at 0.18), white-balanced.
    /// The camera must render the neutral ramp neutral, ordered, unclipped and
    /// with the SOS grey near L* 50, and keep each chromatic patch's hue.
    #[test]
    fn the_virtual_colorchecker_renders_neutral_greys_and_keeps_hues() {
        let cam = camera();
        // incident meter (C = 250) and reflected meter (K = 12.5): the metered grey of an
        // illuminance E is K E / C, so a card of reflectance rho has scene value 0.18 rho / (pi K / C)
        let k = 0.18 / (std::f64::consts::PI * ISO_K / 250.0);
        let mut last_l = 200.0;
        for i in 18..24 {
            let rho = colorchecker_rgb(i);
            let d = cam.display_linear(rho.map(|v| v * k));
            let l = lab(d);
            assert!(l[1].abs() < 1.5 && l[2].abs() < 2.5, "patch {}: {l:?}", i + 1);
            assert!(l[0] < last_l, "ramp order at {}", i + 1);
            last_l = l[0];
            assert!(d.iter().all(|&v| v < 0.98), "patch {} clips: {d:?}", i + 1);
        }
        // neutral 5 (patch 22, Y ~0.19) at about the SOS grey
        let n5 = lab(cam.display_linear(colorchecker_rgb(21).map(|v| v * k)));
        assert!((45.0..56.0).contains(&n5[0]), "neutral 5 L* {}", n5[0]);
        // black (patch 24, Y 0.032) keeps tone above the flare toe
        let blk = lab(cam.display_linear(colorchecker_rgb(23).map(|v| v * k)));
        assert!(blk[0] > 8.0, "black L* {}", blk[0]);
        // chromatic patches keep their CIELAB hue (within 8 deg) and gain chroma
        // over a colorimetric rendering of the same lightness (a camera's look)
        let mut chroma_gain = 0.0;
        for i in 0..18 {
            let rho = colorchecker_rgb(i);
            let r = lab(rho);
            let d = lab(cam.display_linear(rho.map(|v| v * k)));
            let hr = r[2].atan2(r[1]).to_degrees();
            let hd = d[2].atan2(d[1]).to_degrees();
            let dh = ((hd - hr + 540.0) % 360.0 - 180.0).abs();
            assert!(dh < 8.0, "patch {}: hue {hr:.1} -> {hd:.1}", i + 1);
            eprintln!("patch {:2}: L* {:5.1} -> {:5.1}, C* {:5.1} -> {:5.1}, h {hr:6.1} -> {hd:6.1}", i + 1, r[0], d[0], r[1].hypot(r[2]), d[1].hypot(d[2]));
            chroma_gain += (d[1].hypot(d[2]) / r[1].hypot(r[2])) / 18.0;
        }
        assert!((0.85..1.6).contains(&chroma_gain), "mean chroma ratio {chroma_gain}");
        eprintln!("virtual ColorChecker: neutral 5 L* {:.1}, black L* {:.1}, mean chroma ratio {chroma_gain:.2}", n5[0], blk[0]);
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
