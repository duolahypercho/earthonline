//! Street lamps as light records: the interface the renderer lights the city
//! with, and a fallback placement until the props slice delivers its own
//! per-lamp records (position, colour, intensity).
//!
//! The fallback is a port of the JS fallback placement
//! (src/citygen/renderer.js `collectStreetLampRecords` without its drawn
//! roadside query, i.e. `LAMP_STATION_SHIFTS = [0]`): lamps on primary,
//! secondary, tertiary and residential segments, `STREET_LAMP_SPACING` 32 /
//! 34 / 42 / 50 m, `count = max(1, round(length / spacing))`, alternating
//! sides that have at least 0.8 m of sidewalk, an arterial pole 0.25 m inside
//! the kerb offset (`width / 2 + 0.3`), a residential one 58 % across the
//! footway; segments under 18 m get none. Heads (street-lamp-ownership.js
//! `STREET_LAMP_HEAD`): arterial 8.84 m with a 2.74 m arm over the road,
//! column 5.12 m on residential streets. Colour 0xffc46a (the JS street
//! lamp role).

use crate::photometry;

/// A light the renderer shades with: a point or spot in world space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LampLight {
    /// The luminaire (light centre), world metres.
    pub pos: [f64; 3],
    /// Luminous intensity along the beam axis, candela.
    pub candela: f64,
    /// Linear colour at unit luminance.
    pub color: [f64; 3],
    /// Where its light is cut off, metres.
    pub range: f64,
    /// Beam axis (unit), and the cone (cos of the half angles).
    pub dir: [f64; 3],
    pub cos_outer: f64,
    pub cos_inner: f64,
    pub kind: LampKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LampKind {
    /// A full-cutoff street luminaire (light below 75 deg from the nadir).
    #[default]
    Street,
    /// A low-beam headlamp (the JS cut-off 0.6 deg below the horizontal).
    Headlight,
    /// A plain point light.
    Point,
}

/// The street lamp role's colour, 0xffc46a, linear at unit luminance.
pub fn street_lamp_color() -> [f64; 3] {
    let c = [0xff, 0xc4, 0x6a].map(|v| eo_look::math::srgb_to_linear(v as f64 / 255.0));
    let l = eo_look::math::luminance(c);
    c.map(|v| v / l)
}

// ---- San Francisco's LED street lamps (lighting, `--light-shadows`) ----
//
// SFPUC Streetlight Standards Rev. 4 (1 Jul 2023) section 13: "All lights are
// 3000k" (reference sheet 3.3/3.4). 0xffc46a is itself a warm white of about
// 3200 K (McCamy) a little above the Planckian locus, not the high-pressure
// sodium's ~2000-2100 K (sheet 3.4): the orange wash the night captures show
// comes from the frame having no white balance (the renderer's output white
// is D65), while the LED-era photos of SF streets at night (sheet 5.10,
// Castro and 18th, Mar 2026) were taken by cameras that adapt to the
// lamps: near-white warm sources, no orange wash. `led_street_lamp_color`
// is therefore the 3000 K Planckian white as an observer adapted to it sees
// it: the CIECAM02 degree of adaptation at a night street's adapting
// luminance in a dark surround (the street seen in its own night), a von
// Kries step in CAT02 space toward D65; the adaptation is incomplete, so the
// lamps stay warm.

/// SF's street LED colour temperature (K), SFPUC Standards Rev. 4 section 13.
pub const SF_LED_CCT_K: f64 = 3000.0;
/// The adapting luminance of a night street (cd/m2): the scene mean of the
/// LED-era night street photos, 1-6 cd/m2 (reference sheet 3.8, EV100 3-6).
pub const NIGHT_ADAPTING_CD_M2: f64 = 2.0;
/// CIECAM02's surround factor F for a dark surround (CIE 159:2004: average
/// 1.0, dim 0.9, dark 0.8): a night street seen in its own surround.
pub const DARK_SURROUND_F: f64 = 0.8;

/// The CIE 1931 chromaticity of a Planckian radiator at `cct` kelvin (Kim et
/// al. 2002 cubic approximation, 1667-25000 K).
pub fn planckian_xy(cct: f64) -> [f64; 2] {
    let t = cct.clamp(1667.0, 25000.0);
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 { -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910 } else { -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390 };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    [x, y]
}

/// The correlated colour temperature of a chromaticity (McCamy 1992).
pub fn mccamy_cct(xy: [f64; 2]) -> f64 {
    let n = (xy[0] - 0.3320) / (0.1858 - xy[1]);
    449.0 * n.powi(3) + 3525.0 * n * n + 6823.3 * n + 5520.33
}

/// CIE XYZ of linear sRGB (D65).
pub fn linear_srgb_to_xyz(c: [f64; 3]) -> [f64; 3] {
    [
        0.4124 * c[0] + 0.3576 * c[1] + 0.1805 * c[2],
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2],
        0.0193 * c[0] + 0.1192 * c[1] + 0.9505 * c[2],
    ]
}

/// Linear sRGB (D65) of CIE XYZ.
pub fn xyz_to_linear_srgb(c: [f64; 3]) -> [f64; 3] {
    [
        3.2406 * c[0] - 1.5372 * c[1] - 0.4986 * c[2],
        -0.9689 * c[0] + 1.8758 * c[1] + 0.0415 * c[2],
        0.0557 * c[0] - 0.2040 * c[1] + 1.0570 * c[2],
    ]
}

/// CIECAM02's degree of adaptation D at adapting luminance `la` (cd/m2), surround `f`.
pub fn degree_of_adaptation(la: f64, f: f64) -> f64 {
    (f * (1.0 - (1.0 / 3.6) * ((-la - 42.0) / 92.0).exp())).clamp(0.0, 1.0)
}

/// The colour (linear sRGB, unit luminance) of a white source of `cct`
/// kelvin seen by an observer adapted to it with degree `d` (CAT02 von
/// Kries toward D65: 0 shows the source's own chromaticity on the D65
/// output, 1 shows it white).
pub fn adapted_white(cct: f64, d: f64) -> [f64; 3] {
    const M: [[f64; 3]; 3] = [[0.7328, 0.4296, -0.1624], [-0.7036, 1.6975, 0.0061], [0.0030, 0.0136, 0.9834]];
    let mul = |m: &[[f64; 3]; 3], v: [f64; 3]| [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]);
    let [x, y] = planckian_xy(cct);
    let w = [x / y, 1.0, (1.0 - x - y) / y];
    let d65 = [0.95047, 1.0, 1.08883];
    let (lw, lr) = (mul(&M, w), mul(&M, d65));
    // the adopted white itself: D of the way from its own cone response to the reference white's.
    let lc = [0, 1, 2].map(|i| d * lr[i] + (1.0 - d) * lw[i]);
    // M^-1 (CAT02, CIE 159:2004)
    const MI: [[f64; 3]; 3] = [[1.096124, -0.278869, 0.182745], [0.454369, 0.473533, 0.072098], [-0.009628, -0.005698, 1.015326]];
    let c = xyz_to_linear_srgb(mul(&MI, lc)).map(|v| v.max(0.0));
    let l = eo_look::math::luminance(c);
    c.map(|v| v / l)
}

/// SF's 3000 K LED street lamps as a camera or eye adapted to a night street
/// sees them (linear sRGB at unit luminance): near-white warm.
pub fn led_street_lamp_color() -> [f64; 3] {
    adapted_white(SF_LED_CCT_K, degree_of_adaptation(NIGHT_ADAPTING_CD_M2, DARK_SURROUND_F))
}

/// The street lamp role's colour as a chromaticity key (linear, its largest
/// channel 1): a light whose colour has this key is a street lamp of the
/// JS role (the fallback lamps, the props' district lights, the landmark
/// decks' luminaires), whatever its intensity.
pub fn street_lamp_chroma() -> [f64; 3] {
    let c = street_lamp_color();
    let m = c[0].max(c[1]).max(c[2]);
    c.map(|v| v / m)
}

/// One street segment as the street contract carries it.
pub struct SegmentIn<'a> {
    pub highway: &'a str,
    /// Kerb-to-kerb width, m.
    pub width: f64,
    pub sidewalk_left: f64,
    pub sidewalk_right: f64,
    pub points: &'a [[f64; 2]],
}

/// `STREET_LAMP_SPACING`, metres.
pub fn spacing(highway: &str) -> Option<f64> {
    Some(match highway {
        "primary" => 32.0,
        "secondary" => 34.0,
        "tertiary" => 42.0,
        "residential" => 50.0,
        _ => return None,
    })
}

pub const ARTERIAL_HEAD_M: f64 = 8.84;
pub const ARTERIAL_ARM_M: f64 = 2.74;
pub const COLUMN_HEAD_M: f64 = 5.12;
/// Full cutoff: no light above 75 deg from the nadir (IES full-cutoff class
/// limits intensity at 80-90 deg; 75 deg is the type II/III peak's edge).
pub const STREET_CUTOFF_DEG: f64 = 75.0;
/// The intensity peak a type III road distribution throws sideways.
pub const STREET_PEAK_DEG: f64 = 60.0;

fn street_lamp(x: f64, z: f64, head: f64, ground: f64) -> LampLight {
    LampLight {
        pos: [x, ground + head, z],
        candela: photometry::nadir_candela_for_pool(head),
        color: street_lamp_color(),
        // The pool's outer edge (1.75 mounting heights, street-lamp-ownership.js) and a margin.
        range: head * 2.6,
        dir: [0.0, -1.0, 0.0],
        cos_outer: STREET_CUTOFF_DEG.to_radians().cos(),
        cos_inner: STREET_PEAK_DEG.to_radians().cos(),
        kind: LampKind::Street,
    }
}

/// Place the fallback lamps on `segments`; `ground(x, z)` gives the walk level.
pub fn place_street_lamps(segments: &[SegmentIn], ground: impl Fn(f64, f64) -> f64) -> Vec<LampLight> {
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for s in segments {
        let Some(space) = spacing(s.highway) else { continue };
        let pts = s.points;
        if pts.len() < 2 {
            continue;
        }
        let mut cum = vec![0.0];
        for w in pts.windows(2) {
            let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            cum.push(cum.last().unwrap() + l);
        }
        let length = *cum.last().unwrap();
        if !(length >= 18.0) {
            continue;
        }
        let mut sides = vec![];
        if s.sidewalk_left >= 0.8 {
            sides.push((1.0, s.sidewalk_left));
        }
        if s.sidewalk_right >= 0.8 {
            sides.push((-1.0, s.sidewalk_right));
        }
        if sides.is_empty() {
            continue;
        }
        let arterial = s.highway != "residential";
        let count = ((length / space).round() as usize).max(1);
        for i in 0..count {
            let (side, walk) = sides[i % sides.len()];
            let along = (i as f64 + 0.5) / count as f64 * length;
            if along < 1.0 || along > length - 1.0 {
                continue;
            }
            let k = cum.partition_point(|c| *c <= along).clamp(1, pts.len() - 1);
            let (a, b) = (pts[k - 1], pts[k]);
            let seg = cum[k] - cum[k - 1];
            let t = if seg > 1e-9 { (along - cum[k - 1]) / seg } else { 0.0 };
            let (px, pz) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            let (tx, tz) = if seg > 1e-9 { ((b[0] - a[0]) / seg, (b[1] - a[1]) / seg) } else { (1.0, 0.0) };
            // The left normal (three's (-tz, tx) in x/z).
            let (nx, nz) = (-tz, tx);
            let min_off = s.width / 2.0 + 0.3;
            let max_off = s.width / 2.0 + walk - 0.3;
            if max_off < min_off {
                continue;
            }
            let lat = if arterial { max_off.min(min_off + 0.25) } else { min_off + (max_off - min_off) * 0.58 };
            let (x, z) = (px + nx * lat * side, pz + nz * lat * side);
            if !seen.insert(((x * 10.0).round() as i64, (z * 10.0).round() as i64)) {
                continue;
            }
            let g = ground(x, z);
            if !g.is_finite() {
                continue;
            }
            let (hx, hz, head) = if arterial {
                (x - nx * ARTERIAL_ARM_M * side, z - nz * ARTERIAL_ARM_M * side, ARTERIAL_HEAD_M)
            } else {
                (x, z, COLUMN_HEAD_M)
            };
            out.push(street_lamp(hx, hz, head, g));
        }
    }
    out
}

/// Illuminance (lux) a street lamp puts on a horizontal surface at
/// horizontal distance `r` from its foot: the batwing distribution the
/// shader draws (`I(theta) = I0 (1 + 1.5 sin^2 theta)` up to the peak angle,
/// falling to 0 at the cutoff), inverse square, cosine on the ground.
pub fn street_lamp_lux(l: &LampLight, r: f64) -> f64 {
    let h = (l.pos[1]).max(0.1);
    let d2 = h * h + r * r;
    let cos_t = h / d2.sqrt();
    let sin2 = 1.0 - cos_t * cos_t;
    let shape = (1.0 + 1.5 * sin2) * eo_look::math::smoothstep(l.cos_outer, l.cos_inner, cos_t);
    l.candela * shape / d2 * cos_t
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lamps_follow_the_js_spacing_and_sides() {
        let pts = [[0.0, 0.0], [200.0, 0.0]];
        let s = SegmentIn { highway: "primary", width: 12.0, sidewalk_left: 3.0, sidewalk_right: 3.0, points: &pts };
        let lamps = place_street_lamps(&[s], |_, _| 10.0);
        assert_eq!(lamps.len(), 6); // round(200 / 32)
        // Alternating sides, the head over the road (inside the kerb offset 6.3 m).
        assert!(lamps[0].pos[2] > 0.0 && lamps[1].pos[2] < 0.0);
        assert!(lamps.iter().all(|l| l.pos[2].abs() < 6.3 && (l.pos[1] - 18.84).abs() < 1e-9));
    }
    #[test]
    fn the_led_lamps_are_a_near_white_warm_3000_k() {
        // the Planckian locus at 3000 K (CIE tables: x 0.4369, y 0.4041).
        let [x, y] = planckian_xy(3000.0);
        assert!((x - 0.4369).abs() < 0.001 && (y - 0.4041).abs() < 0.001, "{x} {y}");
        assert!((mccamy_cct([x, y]) - 3000.0).abs() < 30.0);
        // 0xffc46a is a ~3200 K warm white, not the sodium's ~2000-2100 K.
        let xyz = linear_srgb_to_xyz(street_lamp_color());
        let s = xyz[0] + xyz[1] + xyz[2];
        let cct = mccamy_cct([xyz[0] / s, xyz[1] / s]);
        assert!(cct > 3000.0 && cct < 3400.0, "0xffc46a at {cct} K");
        // unadapted, 3000 K on a D65 output is redder than 0xffc46a; adapted (D ~0.66 at a night
        // street's 2 cd/m2, dark surround) it is a near-white warm.
        let d = degree_of_adaptation(NIGHT_ADAPTING_CD_M2, DARK_SURROUND_F);
        assert!(d > 0.62 && d < 0.70, "D {d}");
        let raw = adapted_white(3000.0, 0.0);
        let led = led_street_lamp_color();
        let old = street_lamp_color();
        assert!(raw[1] / raw[0] < old[1] / old[0], "3000 K unadapted {raw:?} vs 0xffc46a {old:?}");
        println!("3000 K unadapted {raw:?}, 0xffc46a {old:?}, adapted (D {d:.3}) {led:?}");
        assert!((eo_look::math::luminance(led) - 1.0).abs() < 1e-9);
        assert!(led[0] > led[1] && led[1] > led[2], "warm: {led:?}");
        assert!(led[2] / led[0] > 0.5 && led[2] / led[0] < 0.75 && led[1] / led[0] > 0.7, "near-white: {led:?}");
        // a fully adapted observer sees the source white.
        let w = adapted_white(3000.0, 1.0);
        assert!(w.iter().all(|c| (c - 1.0).abs() < 0.02), "{w:?}");
        let k = street_lamp_chroma();
        assert!((k[0] - 1.0).abs() < 1e-12 && k[1] < 1.0 && k[2] < k[1]);
    }

    #[test]
    fn the_pool_peaks_at_the_reference_and_spreads() {
        let l = street_lamp(0.0, 0.0, ARTERIAL_HEAD_M, 0.0);
        let peak = street_lamp_lux(&l, 0.0);
        assert!((peak - photometry::LAMP_POOL_PEAK_LUX).abs() < 1e-6);
        // Half a spacing away (16 m) the pool still lights the road.
        let mid = street_lamp_lux(&l, 16.0);
        assert!(mid > 1.0 && mid < peak, "{mid}");
    }
}
