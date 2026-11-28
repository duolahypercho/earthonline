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

// ---- (`--rear-lamps`) a vehicle's rear lamps as lights ----
//
// | Lamp | Value | Source |
// | --- | --- | --- |
// | Tail lamp | 10 cd toward the rear | SAE J585 (tail lamps): 2-18 cd at H-V for a single lighted section |
// | Stop lamp | 120 cd toward the rear | SAE J586 (stop lamps): 80-300 cd at H-V; lit while the vehicle brakes (its drawn lens's state, the agents' braking flag) |
// | Colour | SAE red, x 0.69 y 0.30 | SAE J578 red boundary; as linear sRGB at unit luminance |
// | Reach | where it gives `REAR_REACH_LUX` | INFERRED: a tenth of a lux, an eighth of the night's unlit ground (`night_photometry::NIGHT_UNLIT_PHYSICAL_LUX`) |
// | Spread | `REAR_LOBE` about the rear axis (a cone of `REAR_OUTER_DEG` where a shader takes a cone) | SAE J585/J586 photometric test points (the lobes' table below) |
// | Source | just behind the body's rear face (its bounds) | so its own body takes none of it and the road under it none |

/// A tail lamp's intensity toward the rear (cd; SAE J585).
pub const REAR_TAIL_CD: f64 = 10.0;
/// A stop lamp's intensity toward the rear (cd; SAE J586).
pub const REAR_STOP_CD: f64 = 120.0;
/// A rear lamp's light is drawn out to where it gives this illuminance (lux).
pub const REAR_REACH_LUX: f64 = 0.1;
/// A rear lamp's spread as a cone where a shader takes one (the wet street's streaks):
/// full within `REAR_INNER_DEG` of the rear axis, none past `REAR_OUTER_DEG`; the
/// surfaces and the haze take `REAR_LOBE`.
pub const REAR_INNER_DEG: f64 = 20.0;
pub const REAR_OUTER_DEG: f64 = 70.0;
/// A rear lamp's lit lens (m^2; its reflections' source size).
pub const REAR_LENS_AREA_M2: f64 = 0.03;

/// SAE red (x 0.69, y 0.30) in linear sRGB at unit luminance.
pub fn rear_lamp_color() -> [f64; 3] {
    xy_unit_color(0.69, 0.30)
}

/// How far (m) a rear lamp of `candela` reaches (`REAR_REACH_LUX`).
pub fn rear_lamp_reach_m(candela: f64) -> f64 {
    (candela / REAR_REACH_LUX).sqrt()
}

// ---- (`--signal-spill`) a traffic signal's lit lens as a light ----
//
// | Lens (300 mm LED) | Intensity on its axis | Colour | Source |
// | --- | --- | --- | --- |
// | Red | 365 cd | x 0.6997 y 0.3002 (625 nm) | ITE VTCSH Part 2 (LED circular signal supplement), minimum maintained intensity at H-V, 12-inch; the colour inside the ITE red box (standards table `signal-lens-chromaticity`) |
// | Yellow | 910 cd | x 0.5752 y 0.4242 (590 nm) | the same table; inside the ITE yellow box |
// | Green | 475 cd | x 0.0039 y 0.6548 (505 nm) | the same table; inside the ITE green box (the blue-green of the LED lenses) |
// | Beam | `SIGNAL_LOBE` (a cone to `SIGNAL_OUTER_DEG` where a shader takes a cone) | the VTCSH distribution's points (the lobes' table below) | |
// | Lens | 300 mm, its luminance its intensity toward the eye over its area | ITE VTCSH (12-inch) | |
// | Reach | where it gives `SIGNAL_REACH_LUX` | INFERRED: a quarter of a lux |
// | Clear | nothing lit within `SIGNAL_CLEAR_M` of the lens | CA MUTCD 4D.12's clear view of the face along its approach; INFERRED: the distance |

/// A lit lens's intensity on its axis (cd): red, yellow, green (ITE VTCSH, 12-inch LED).
pub const SIGNAL_CD: [f64; 3] = [365.0, 910.0, 475.0];
/// The lenses' chromaticities (x, y): red, yellow, green: the CIE 1931 spectral locus at the
/// LED dies' dominant wavelengths (AlInGaP red 625 nm, AlInGaP amber 590 nm, InGaN green
/// 505 nm), each inside its ITE box (the standards table's `signal-lens-chromaticity`; the
/// colour, `signal_color`, keeps only the hue, the gamut taking the rest).
pub const SIGNAL_XY: [[f64; 2]; 3] = [[0.6997, 0.3002], [0.5752, 0.4242], [0.0039, 0.6548]];
/// The chromaticities before (eo-render `--no-signal-chroma` keeps them): the green a yellow-
/// green outside its ITE box (y over 0.730 - x), the red and yellow just outside theirs.
pub const SIGNAL_XY_BEFORE: [[f64; 2]; 3] = [[0.69, 0.30], [0.57, 0.42], [0.17, 0.70]];
/// The standards table's id for the lenses' chromaticity boxes.
pub const SIGNAL_CHROMATICITY_ID: &str = "signal-lens-chromaticity";
/// The beam as a cone where a shader takes one (the wet street's streaks): full on
/// the axis (`SIGNAL_INNER_DEG`), nothing past `SIGNAL_OUTER_DEG`; the surfaces, the
/// haze and the lens take `SIGNAL_LOBE`.
pub const SIGNAL_INNER_DEG: f64 = 0.0;
pub const SIGNAL_OUTER_DEG: f64 = 25.0;
/// A lens's light is drawn out to where it gives this illuminance (lux).
pub const SIGNAL_REACH_LUX: f64 = 0.25;
/// The beam lights nothing nearer the lens than this (m; from three quarters of
/// it, fully past it): a working head's face is in clear view of its approach
/// for the stopping sight distance (CA MUTCD 4D.12: 250 ft at 35 mph), and near
/// the head the drivers' sight lines fill the beam, so a surface that close in
/// the data (a palm's crown beside a pole carrying back-to-back heads) is a
/// placement's, not the head's, and takes none. INFERRED (the distance).
pub const SIGNAL_CLEAR_M: f64 = 6.0;

// ---- (`--signal-light`) a pedestrian signal's lit symbol ----
//
// | Symbol (16 x 18 in LED module) | Luminance | Colour | Source |
// | --- | --- | --- | --- |
// | Upraised hand | 1400 cd/m2 | Portland orange, x 0.61 y 0.38 | ITE PTCSI Part 2 (LED pedestrian signal modules, 2010): the minimum luminance an ITE-compliant countdown module states (Dialight 430-6479-001X data sheet, WisDOT QPL 658.0635, trust.dot.state.wi.us/ftp/dtsd/bto/qpl/electrical/6580635-ledmodules-pedcountdown-16inch-dialight.pdf); the chromaticity INFERRED inside ITE's Portland orange |
// | Walking person | 2200 cd/m2 | lunar white, x 0.33 y 0.35 | the same sheet; the chromaticity INFERRED inside ITE's lunar white |

/// A lit pedestrian symbol's luminance (cd/m2): the hand, the walking person.
pub const PED_SIGNAL_NIT: [f64; 2] = [1400.0, 2200.0];
/// The pedestrian symbols' chromaticities (x, y): Portland orange, lunar white.
pub const PED_SIGNAL_XY: [[f64; 2]; 2] = [[0.61, 0.38], [0.33, 0.35]];

/// A lit pedestrian symbol's colour at unit luminance (0 the hand, 1 the walking person).
pub fn ped_signal_color(symbol: usize) -> [f64; 3] {
    let [x, y] = PED_SIGNAL_XY[symbol.min(1)];
    xy_unit_color(x, y)
}

/// The sRGB white point (D65) in CIE xy.
pub const D65_XY: [f64; 2] = [0.3127, 0.3290];

/// A chromaticity (x, y) as linear sRGB at unit luminance, unclipped (a
/// colour outside the sRGB gamut has a negative component).
fn xy_linear(x: f64, y: f64) -> [f64; 3] {
    let (cx, cz) = (x / y, (1.0 - x - y) / y);
    [3.2406 * cx - 1.5372 - 0.4986 * cz, -0.9689 * cx + 1.8758 + 0.0415 * cz, 0.0557 * cx - 0.2040 + 1.0570 * cz]
}

/// A chromaticity (x, y) in linear sRGB at unit luminance. One outside the
/// sRGB gamut (the signals' and rear lamps' saturated LEDs) is brought in along
/// its line to the white point (D65), which keeps its dominant wavelength,
/// i.e. its hue: the ITE signal green (x 0.0039 y 0.6548, 505 nm) is the
/// blue-green its LEDs are, where clipping the negative red would turn it to
/// the display's own yellow-green primary (~550 nm) and read as neon.
pub fn xy_unit_color(x: f64, y: f64) -> [f64; 3] {
    let inside = |t: f64| {
        let (px, py) = (D65_XY[0] + t * (x - D65_XY[0]), D65_XY[1] + t * (y - D65_XY[1]));
        xy_linear(px, py).iter().all(|&v| v >= 0.0)
    };
    let t = if inside(1.0) {
        1.0
    } else {
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..48 {
            let m = 0.5 * (lo + hi);
            if inside(m) {
                lo = m;
            } else {
                hi = m;
            }
        }
        lo
    };
    let c = xy_linear(D65_XY[0] + t * (x - D65_XY[0]), D65_XY[1] + t * (y - D65_XY[1])).map(|v| v.max(0.0));
    let l = eo_look::math::luminance(c);
    c.map(|v| v / l)
}

/// A lit lens's colour (aspect 0 red, 1 yellow, 2 green).
pub fn signal_color(aspect: usize) -> [f64; 3] {
    let [x, y] = SIGNAL_XY[aspect.min(2)];
    xy_unit_color(x, y)
}

/// A lit lens's colour as before (`SIGNAL_XY_BEFORE`; eo-render `--no-signal-chroma`).
pub fn signal_color_before(aspect: usize) -> [f64; 3] {
    let [x, y] = SIGNAL_XY_BEFORE[aspect.min(2)];
    xy_unit_color(x, y)
}

// ---- (`--signal-spill`, `--rear-lamps`) the signals' and rear lamps' photometric lobes ----
//
// A signal's or a rear lamp's light is a narrow lobe about its axis, not a
// cone: its intensity falls by angle off the axis as its standard's test
// points do, below and to the sides, and a signal's visor cuts it above.
// The lobe is separable, I(v, h) / I(axis) = f_v(v) f_h(h), each the sum of
// two Gaussians in the angle (a core and a tail), fitted to the points:
//
// | Lamp | Points (share of the axis value) | Source |
// | --- | --- | --- |
// | Signal, below its axis | 2.5 deg down 1.0 (the axis), 7.5 0.8, 12.5 0.5, 17.5 0.25, 22.5 0.12, 27.5 0.06 | ITE VTCSH Part 2 (LED circular signal supplement), 300 mm minimum maintained intensities at 2.5 deg left/right, as shares of the 2.5 deg down value (rounded) |
// | Signal, to the side | 2.5 deg 1.0, 7.5 0.86, 12.5 0.6, 17.5 0.37, 22.5 0.19, 27.5 0.08 | the same table at 2.5 deg down |
// | Signal, above its axis | half by 3 deg up, a few per cent by 8 deg up | INFERRED: the visor (cap) over each lens cuts the light above the axis (ITE VTCSH Part 1: a visor shields the face) |
// | Rear lamp, up or down | 5 deg 0.75, 10 deg 0.33, 15 deg 0.12, 20 deg 0.04 | SAE J585 / J586 test points (H-V, 5 and 10 deg up and down) as shares of H-V; past 10 deg FMVSS 108's visibility minimum |
// | Rear lamp, to the side | 5 deg 0.9, 10 deg 0.7, 20 deg 0.3, 30 deg 0.12, 45 deg 0.04 | SAE J585 / J586 test points (5, 10, 20 deg left and right); 45 deg FMVSS 108's visibility minimum |
//
// The shaders take the vertical angle as the sine of the depression and the
// horizontal one from 2 (1 - cos) (small-angle forms, within ~5 % of the
// angle to 45 deg); `lobe` here is the same arithmetic.

/// A lamp's lobe: `peak_down_deg` the depression of its axis's peak; below
/// the peak `w_v` of a Gaussian of `sigma_v_deg[0]` and the rest of
/// `sigma_v_deg[1]`; above it one of `sigma_up_deg`; to the side `w_h` of
/// `sigma_h_deg[0]` and the rest of `sigma_h_deg[1]`. None behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lobe {
    pub peak_down_deg: f64,
    pub w_v: f64,
    pub sigma_v_deg: [f64; 2],
    pub sigma_up_deg: f64,
    pub w_h: f64,
    pub sigma_h_deg: [f64; 2],
}

/// A 300 mm signal lens (ITE VTCSH; the visor above).
pub const SIGNAL_LOBE: Lobe = Lobe { peak_down_deg: 2.5, w_v: 0.85, sigma_v_deg: [11.0, 24.0], sigma_up_deg: 5.0, w_h: 0.85, sigma_h_deg: [15.5, 30.0] };
/// A tail or stop lamp (SAE J585 / J586; symmetric up and down).
pub const REAR_LOBE: Lobe = Lobe { peak_down_deg: 0.0, w_v: 0.9, sigma_v_deg: [9.0, 20.0], sigma_up_deg: 9.5, w_h: 0.8, sigma_h_deg: [15.0, 40.0] };
/// A 300 mm signal lens's lit area (m^2): its luminance is its intensity over it.
pub const SIGNAL_LENS_AREA_M2: f64 = std::f64::consts::PI * 0.15 * 0.15;

impl Lobe {
    /// The shaders' two vectors: (sin of the peak's depression, w_v, 1 / sigma_v^2 core, tail),
    /// (w_h, 1 / sigma_h^2 core, tail, 1 / sigma_up^2), the sigmas in radians.
    pub fn params(&self) -> [[f64; 4]; 2] {
        let k = |deg: f64| 1.0 / deg.to_radians().powi(2);
        [
            [self.peak_down_deg.to_radians().sin(), self.w_v, k(self.sigma_v_deg[0]), k(self.sigma_v_deg[1])],
            [self.w_h, k(self.sigma_h_deg[0]), k(self.sigma_h_deg[1]), k(self.sigma_up_deg)],
        ]
    }

    /// The share of the axis's intensity toward unit direction `a` (from the
    /// lamp, y up) for a lamp whose axis points along `axis` (its horizontal
    /// part is used): the shaders' `nt_lobe`.
    pub fn share(&self, a: [f64; 3], axis: [f64; 3]) -> f64 {
        let [v, h] = self.params();
        let (hl, ha) = ([a[0], a[2]], [axis[0], axis[2]]);
        let n = ((hl[0] * hl[0] + hl[1] * hl[1]) * (ha[0] * ha[0] + ha[1] * ha[1])).max(1e-10).sqrt();
        let ch = (hl[0] * ha[0] + hl[1] * ha[1]) / n;
        if ch <= 0.0 {
            return 0.0;
        }
        let t2 = 2.0 * (1.0 - ch);
        let x = -a[1] - v[0];
        let x2 = x * x;
        let fv = if x < 0.0 { (-x2 * h[3]).exp() } else { v[1] * (-x2 * v[2]).exp() + (1.0 - v[1]) * (-x2 * v[3]).exp() };
        let fh = h[0] * (-t2 * h[1]).exp() + (1.0 - h[0]) * (-t2 * h[2]).exp();
        fv * fh
    }
}

/// The lobes' WGSL: `nt_lobe(a, axis, v, h)` and each lamp's vectors
/// (`NT_SIGNAL_V`, `NT_SIGNAL_H`, `NT_REAR_V`, `NT_REAR_H`).
pub fn lobe_wgsl() -> String {
    let v4 = |p: [f64; 4]| format!("vec4<f32>({:.8}, {:.8}, {:.8}, {:.8})", p[0], p[1], p[2], p[3]);
    let [sv, sh] = SIGNAL_LOBE.params();
    let [rv, rh] = REAR_LOBE.params();
    format!(
        "\n// generated by eo-sky lights::lobe_wgsl (`--signal-spill`, `--rear-lamps`)\n\
         const NT_SIGNAL_V: vec4<f32> = {};\nconst NT_SIGNAL_H: vec4<f32> = {};\nconst NT_REAR_V: vec4<f32> = {};\nconst NT_REAR_H: vec4<f32> = {};\n\
         const NT_SIGNAL_CLEAR_M: f32 = {:.3};\nconst NT_SIGNAL_LENS_M2: f32 = {:.6};\n\
         /// a signal's or rear lamp's share of its axis intensity toward unit direction `a`\n\
         /// (from the lamp), its axis `axis` (eo-sky lights `Lobe::share`).\n\
         fn nt_lobe(a: vec3<f32>, axis: vec3<f32>, v: vec4<f32>, h: vec4<f32>) -> f32 {{\n\
         \x20   let hl = vec2<f32>(a.x, a.z);\n\
         \x20   let ha = vec2<f32>(axis.x, axis.z);\n\
         \x20   let ch = dot(hl, ha) * inverseSqrt(max(dot(hl, hl) * dot(ha, ha), 1e-10));\n\
         \x20   let x = -a.y - v.x;\n\
         \x20   let x2 = x * x;\n\
         \x20   let t2 = 2.0 * (1.0 - ch);\n\
         \x20   let fv = select(v.y * exp(-x2 * v.z) + (1.0 - v.y) * exp(-x2 * v.w), exp(-x2 * h.w), x < 0.0);\n\
         \x20   let fh = h.x * exp(-t2 * h.y) + (1.0 - h.x) * exp(-t2 * h.z);\n\
         \x20   return select(0.0, fv * fh, ch > 0.0);\n\
         }}\n",
        v4(sv),
        v4(sh),
        v4(rv),
        v4(rh),
        SIGNAL_CLEAR_M,
        SIGNAL_LENS_AREA_M2
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rear_lamps_are_red_and_reach_by_their_light() {
        let c = rear_lamp_color();
        assert!((eo_look::math::luminance(c) - 1.0).abs() < 1e-9);
        assert!(c[0] > 3.0 && c[1] < 0.5 && c[2] < 0.5, "red: {c:?}");
        assert!((rear_lamp_reach_m(REAR_TAIL_CD) - 10.0).abs() < 1e-9);
        assert!(rear_lamp_reach_m(REAR_STOP_CD) > 30.0);
    }

    /// A direction `down` deg below the horizontal and `side` deg off the +z axis.
    fn dir(down: f64, side: f64) -> [f64; 3] {
        let (v, h) = (down.to_radians(), side.to_radians());
        [v.cos() * h.sin(), -v.sin(), v.cos() * h.cos()]
    }

    #[test]
    fn signal_lobe_follows_the_ite_points() {
        let ax = [0.0, 0.0, 1.0];
        let on = SIGNAL_LOBE.share(dir(2.5, 0.0), ax);
        assert!((on - 1.0).abs() < 1e-6, "the axis: {on}");
        // below the axis (the VTCSH points' shares) and to the side (at 2.5 deg down)
        for (d, want) in [(7.5, 0.8), (12.5, 0.5), (17.5, 0.25), (22.5, 0.12), (27.5, 0.06)] {
            let got = SIGNAL_LOBE.share(dir(d, 0.0), ax);
            assert!((got - want).abs() < 0.05 + 0.2 * want, "{d} deg down: {got} vs {want}");
        }
        for (s, want) in [(7.5, 0.86), (12.5, 0.6), (17.5, 0.37), (22.5, 0.19), (27.5, 0.08)] {
            let got = SIGNAL_LOBE.share(dir(2.5, s), ax);
            assert!((got - want).abs() < 0.05 + 0.2 * want, "{s} deg aside: {got} vs {want}");
        }
        // the visor above, nothing behind; the ground 10 m out from a 4.5 m head takes a tenth
        assert!(SIGNAL_LOBE.share(dir(-8.0, 0.0), ax) < 0.05);
        assert_eq!(SIGNAL_LOBE.share(dir(2.5, 120.0), ax), 0.0);
        let ground = SIGNAL_LOBE.share(dir((4.5f64 / 10.0).atan().to_degrees(), 0.0), ax);
        assert!(ground < 0.2, "the walk 10 m out: {ground}");
    }

    #[test]
    fn rear_lobe_follows_the_sae_points_and_spares_the_road_under_it() {
        let ax = [0.0, 0.0, 1.0];
        for (d, want) in [(5.0, 0.75), (10.0, 0.33), (15.0, 0.12), (20.0, 0.04)] {
            for s in [d, -d] {
                let got = REAR_LOBE.share(dir(s, 0.0), ax);
                assert!((got - want).abs() < 0.04 + 0.2 * want, "{s} deg: {got} vs {want}");
            }
        }
        for (s, want) in [(5.0, 0.9), (10.0, 0.7), (20.0, 0.3), (30.0, 0.12), (45.0, 0.04)] {
            let got = REAR_LOBE.share(dir(0.0, s), ax);
            assert!((got - want).abs() < 0.04 + 0.2 * want, "{s} deg aside: {got} vs {want}");
        }
        // the road a metre behind a 0.8 m lamp (39 deg down) takes under a per cent
        assert!(REAR_LOBE.share(dir(38.7, 0.0), ax) < 0.01);
    }

    #[test]
    fn lobe_wgsl_carries_the_lobes() {
        let w = lobe_wgsl();
        assert!(w.contains("fn nt_lobe(") && w.contains("const NT_SIGNAL_V") && w.contains("const NT_REAR_H"));
        assert!((SIGNAL_LENS_AREA_M2 - 0.0707).abs() < 1e-3);
    }

    #[test]
    fn signal_lenses_are_their_colours() {
        let (r, y, g) = (signal_color(0), signal_color(1), signal_color(2));
        assert!(r[0] > r[1] && r[0] > r[2], "red {r:?}");
        assert!(y[0] > y[2] && y[1] > y[2], "yellow {y:?}");
        assert!(g[1] > g[0] && g[2] > g[0], "blue-green {g:?}");
        for c in [r, y, g] {
            assert!((eo_look::math::luminance(c) - 1.0).abs() < 1e-9);
        }
        // the green keeps its blue (its hue), not the display's yellow-green primary
        assert!(g[2] > 0.25 * g[1], "green {g:?}");
    }

    #[test]
    fn pedestrian_symbols_are_their_colours() {
        let (hand, person) = (ped_signal_color(0), ped_signal_color(1));
        assert!(hand[0] > hand[1] && hand[1] > hand[2], "Portland orange {hand:?}");
        assert!(person.iter().all(|&v| (v - 1.0).abs() < 0.25), "lunar white {person:?}");
        for c in [hand, person] {
            assert!((eo_look::math::luminance(c) - 1.0).abs() < 1e-9);
        }
        // the person is the brighter symbol (ITE PTCSI Part 2)
        assert!(PED_SIGNAL_NIT[1] > PED_SIGNAL_NIT[0]);
    }

    #[test]
    fn out_of_gamut_colours_keep_their_hue() {
        for [x, y] in SIGNAL_XY.into_iter().chain(SIGNAL_XY_BEFORE) {
            let c = xy_unit_color(x, y);
            assert!(c.iter().all(|&v| v >= 0.0));
            // the colour's chromaticity on the line from D65 to (x, y), between them
            let xyz = [0.4124 * c[0] + 0.3576 * c[1] + 0.1805 * c[2], 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2], 0.0193 * c[0] + 0.1192 * c[1] + 0.9505 * c[2]];
            let s = xyz[0] + xyz[1] + xyz[2];
            let (px, py) = (xyz[0] / s, xyz[1] / s);
            let (dx, dy) = (x - D65_XY[0], y - D65_XY[1]);
            let cross = (px - D65_XY[0]) * dy - (py - D65_XY[1]) * dx;
            assert!(cross.abs() < 2e-3 * (dx * dx + dy * dy).sqrt(), "({x}, {y}) -> ({px:.4}, {py:.4})");
            assert!((px - D65_XY[0]) * dx + (py - D65_XY[1]) * dy > 0.0);
        }
        // an in-gamut colour is its own
        let w = xy_unit_color(0.3127, 0.3290);
        assert!((w[0] - 1.0).abs() < 2e-3 && (w[1] - 1.0).abs() < 2e-3 && (w[2] - 1.0).abs() < 2e-3, "{w:?}");
    }

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
