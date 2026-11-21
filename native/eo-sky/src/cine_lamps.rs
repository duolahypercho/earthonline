//! (`--look cinematic`, the cinematic look plan's stage L2) The night
//! street's lamp palette and luminaire optics under the cinematic look, as
//! pure functions the renderer's gather and shaders mirror (eo-render
//! sky_lights.wgsl `lt_cine_*`, sky_light.wgsl `sky_cine_*`). The default
//! look keeps every lamp at SF's 3000 K (`lights::led_street_lamp_color`)
//! with the batwing/spot pools; none of this runs without the look.
//!
//! **Palette.** Most of San Francisco's ~18 500 cobra heads were converted
//! to 3000 K LED from 2014 (reference sheet 3.4), but the cinematic look
//! takes a mixed city on purpose (owner's decision, plan "Night lamps"):
//! LED at 3000 K on most streets, a little 4000 K on some arterials and the
//! freeway decks, high-pressure sodium (about 2050 K, sheet 3.4: 2000-2100 K)
//! on a share of the arterials and the industrial and port streets, and the
//! historic and banner heads at 2700 K. The choice is made per street (its
//! name, so a street is one colour from end to end), never per lamp: a whole
//! street goes orange, not every fifth lamp. Every colour is seen by an eye
//! adapted to the city's dominant 3000 K (CAT02, D 0.66, as `lights` and
//! `shops` see theirs), so a 3000 K lamp is the default's colour, sodium reads
//! orange and 4000 K a cool white.
//!
//! The rule (`street_tint`): the street's class (OSM `highway` of the street
//! contract's segment) and a hash of its name (`hash01`):
//! - primary, secondary, tertiary (and their links): sodium below
//!   `SODIUM_ARTERIAL_SHARE`, 4000 K above 1 - `COOL_ARTERIAL_SHARE`, else 3000 K;
//! - residential: sodium below `SODIUM_RESIDENTIAL_SHARE`, else 3000 K;
//! - unclassified and service (industrial access roads, port aprons, alleys):
//!   sodium below `SODIUM_INDUSTRIAL_SHARE`, else 3000 K;
//! - motorway and trunk (the freeway decks' luminaires): 4000 K.
//!
//! The renderer reads the rule through a grid (`PaletteGrid`) over the
//! city: every cell within `REACH_M` of a lamp street's carriageway holds
//! that street's tint and its axis, so the fallback lamps, the props
//! slice's district lights and the far lamps take one street's colour
//! wherever they stand on it (the props lamps replace the fallback ones as
//! their tiles stream in without changing colour).
//!
//! **Pool.** A measured-style LED cobra head (IES type II/III, `cobra_shape`):
//! its intensity over the nadir's rises toward ~65 deg along the street and
//! rolls off softly to ~82 deg there, but only to ~45-70 deg across it, so a
//! pool is long along the street and short across it, without the flat top
//! and hard ring of the batwing's smoothstep; the nadir keeps the RP-8 peak
//! (`photometry::LAMP_POOL_PEAK_LUX`). The light's reach is
//! `RANGE_MOUNTING_HEIGHTS` heads (the default's 2.6), its window the default's.
//!
//! **Low beam.** `low_beam_shape`: a sharp cut-off just under the horizontal
//! on the opposing side (49 CFR 571.108 S10.18.9.1.1: 0.4 deg down), stepped
//! up to the horizontal on the kerb side; a hot spot 1.5 deg right of the
//! axis, falling to ~0.2 of it at 20 deg (the FMVSS test points, sheet 3.6);
//! above the cut-off the glare zone's share.
//!
//! **Specular.** A light of radius r at distance d widens the GGX lobe's
//! alpha by r / (2 d) (the representative-point sphere light), and the
//! local lights' specular BRDF is capped at `SPEC_CAP`, so a wall, a car's
//! back or a wet road under a near lamp keeps a bright highlight that does
//! not clip to a white disc.

use crate::lights::{DARK_SURROUND_F, LampLight, NIGHT_ADAPTING_CD_M2, SF_LED_CCT_K, degree_of_adaptation};
use crate::shops::adapted_source;

/// A street's lamp colour under the cinematic look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LampTint {
    /// SF's 3000 K LED (the default's colour).
    Led3000 = 0,
    /// A cooler 4000 K LED (some arterials, the freeway decks).
    Led4000 = 1,
    /// High-pressure sodium, ~2050 K: deep orange.
    Sodium = 2,
    /// The historic and banner heads' warmer 2700 K.
    Historic = 3,
}

impl LampTint {
    pub const ALL: [LampTint; 4] = [LampTint::Led3000, LampTint::Led4000, LampTint::Sodium, LampTint::Historic];

    pub fn from_code(c: u8) -> LampTint {
        Self::ALL[(c & 3) as usize]
    }

    /// Its correlated colour temperature, kelvin.
    pub fn cct(self) -> f64 {
        match self {
            LampTint::Led3000 => SF_LED_CCT_K,
            LampTint::Led4000 => LED_COOL_CCT_K,
            LampTint::Sodium => SODIUM_CCT_K,
            LampTint::Historic => HISTORIC_CCT_K,
        }
    }

    /// Its colour (linear sRGB, unit luminance) to an eye adapted to the
    /// city's 3000 K lamps (as `lights::led_street_lamp_color` and `shops::colour`).
    pub fn colour(self) -> [f64; 3] {
        adapted_source(self.cct(), SF_LED_CCT_K, degree_of_adaptation(NIGHT_ADAPTING_CD_M2, DARK_SURROUND_F))
    }
}

/// The cooler LED some arterials and the freeway decks take (K).
pub const LED_COOL_CCT_K: f64 = 4000.0;
/// High-pressure sodium (K): 2000-2100 K, reference sheet 3.4.
pub const SODIUM_CCT_K: f64 = 2050.0;
/// The historic and banner heads (K): a warm-white retrofit of the acorn globes.
pub const HISTORIC_CCT_K: f64 = 2700.0;

/// The share of arterial streets (by name) left on sodium.
pub const SODIUM_ARTERIAL_SHARE: f64 = 0.26;
/// The share of arterial streets (by name) at 4000 K.
pub const COOL_ARTERIAL_SHARE: f64 = 0.12;
/// The share of residential streets (by name) left on sodium.
pub const SODIUM_RESIDENTIAL_SHARE: f64 = 0.12;
/// The share of industrial access and service roads (by name) left on sodium.
pub const SODIUM_INDUSTRIAL_SHARE: f64 = 0.5;

/// Where a street's tint reaches from its carriageway's edge (m): over the
/// kerb, the walk and a lamp's arm, so every luminaire of the street finds it.
pub const REACH_M: f64 = 7.0;

/// A stable hash of a name to [0, 1) (FNV-1a 64, then a mix): the same
/// street the same draw on every run and machine.
pub fn hash01(name: &str) -> f64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// How a street class ranks when two streets claim one cell (lower wins):
/// the streets people walk and the lamps stand on before the alleys, those
/// before the freeway over them. None: a class that carries no street lamps.
pub fn class_rank(highway: &str) -> Option<u8> {
    Some(match highway {
        "primary" | "secondary" | "tertiary" | "primary_link" | "secondary_link" | "tertiary_link" | "residential" => 0,
        "unclassified" | "service" => 1,
        "motorway" | "trunk" | "motorway_link" | "trunk_link" => 2,
        _ => return None,
    })
}

/// The tint of a street named `name` (its id when unnamed) of class `highway`
/// (the rule in the module doc). None: a class that carries no street lamps.
pub fn street_tint(name: &str, highway: &str) -> Option<LampTint> {
    let h = hash01(name);
    Some(match highway {
        "primary" | "secondary" | "tertiary" | "primary_link" | "secondary_link" | "tertiary_link" => {
            if h < SODIUM_ARTERIAL_SHARE {
                LampTint::Sodium
            } else if h >= 1.0 - COOL_ARTERIAL_SHARE {
                LampTint::Led4000
            } else {
                LampTint::Led3000
            }
        }
        "residential" => {
            if h < SODIUM_RESIDENTIAL_SHARE {
                LampTint::Sodium
            } else {
                LampTint::Led3000
            }
        }
        "unclassified" | "service" => {
            if h < SODIUM_INDUSTRIAL_SHARE {
                LampTint::Sodium
            } else {
                LampTint::Led3000
            }
        }
        "motorway" | "trunk" | "motorway_link" | "trunk_link" => LampTint::Led4000,
        _ => return None,
    })
}

/// One street segment for the palette grid.
pub struct StreetIn<'a> {
    /// Its street's name (or the segment's id when unnamed).
    pub name: &'a str,
    pub highway: &'a str,
    /// Kerb-to-kerb width, m.
    pub width: f64,
    pub points: &'a [[f64; 2]],
}

/// The palette over the city: per cell, the street (if any) whose lamps
/// stand there, its tint and its axis. A cell's byte: bit 7 a street, bits
/// 0-1 the tint (`LampTint` code), bits 2-5 the axis (16 steps over 180 deg
/// from +x toward +z). Four cells a little-endian u32 word, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteGrid {
    pub corner: [f64; 2],
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<u8>,
}

/// Axis steps over 180 deg.
pub const AXIS_STEPS: u32 = 16;

impl PaletteGrid {
    /// The grid of `cols` x `rows` cells of `cell` m from `corner` (x, z)
    /// over `streets`: each cell takes the lowest-ranked class
    /// (`class_rank`) whose carriageway lies within `REACH_M` of it (by its
    /// centre, plus half a cell's diagonal), the nearest centreline among those.
    pub fn build(corner: [f64; 2], cell: f64, cols: usize, rows: usize, streets: &[StreetIn]) -> PaletteGrid {
        let mut cells = vec![0u8; cols * rows];
        // the claim on each cell: (rank, distance)
        let mut best = vec![(u8::MAX, f64::INFINITY); cols * rows];
        let slack = cell * std::f64::consts::FRAC_1_SQRT_2;
        for s in streets {
            let (Some(rank), Some(tint)) = (class_rank(s.highway), street_tint(s.name, s.highway)) else { continue };
            let reach = s.width.max(0.0) * 0.5 + REACH_M;
            for w in s.points.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let len2 = dx * dx + dz * dz;
                if len2 < 1e-6 {
                    continue;
                }
                let mut ang = dz.atan2(dx);
                if ang < 0.0 {
                    ang += std::f64::consts::PI;
                }
                if ang >= std::f64::consts::PI {
                    ang -= std::f64::consts::PI;
                }
                let axis = ((ang / std::f64::consts::PI * AXIS_STEPS as f64).round() as u32 % AXIS_STEPS) as u8;
                let r = reach + slack;
                let c0 = (((a[0].min(b[0]) - r - corner[0]) / cell).floor().max(0.0)) as usize;
                let c1 = (((a[0].max(b[0]) + r - corner[0]) / cell).floor() as isize).min(cols as isize - 1);
                let r0 = (((a[1].min(b[1]) - r - corner[1]) / cell).floor().max(0.0)) as usize;
                let r1 = (((a[1].max(b[1]) + r - corner[1]) / cell).floor() as isize).min(rows as isize - 1);
                if c1 < 0 || r1 < 0 {
                    continue;
                }
                for row in r0..=r1 as usize {
                    for col in c0..=c1 as usize {
                        let p = [corner[0] + (col as f64 + 0.5) * cell, corner[1] + (row as f64 + 0.5) * cell];
                        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len2).clamp(0.0, 1.0);
                        let d = (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dz * t);
                        if d > r {
                            continue;
                        }
                        let i = row * cols + col;
                        if (rank, d) < best[i] {
                            best[i] = (rank, d);
                            cells[i] = 0x80 | (axis << 2) | tint as u8;
                        }
                    }
                }
            }
        }
        PaletteGrid { corner, cell, cols, rows, cells }
    }

    /// The street at (x, z): its tint and its axis (unit, x/z). None off the streets.
    pub fn at(&self, x: f64, z: f64) -> Option<(LampTint, [f64; 2])> {
        let c = ((x - self.corner[0]) / self.cell).floor();
        let r = ((z - self.corner[1]) / self.cell).floor();
        if c < 0.0 || r < 0.0 || c >= self.cols as f64 || r >= self.rows as f64 {
            return None;
        }
        decode(self.cells[r as usize * self.cols + c as usize])
    }

    /// The cells as the GPU reads them (four a word, the first in the low byte).
    pub fn words(&self) -> Vec<u32> {
        let mut out = vec![0u32; self.cells.len().div_ceil(4).max(1)];
        for (i, c) in self.cells.iter().enumerate() {
            out[i / 4] |= (*c as u32) << (8 * (i % 4));
        }
        out
    }
}

/// The lamps as the cinematic look draws their colour: a street lamp of the
/// JS role (its colour's chromaticity 0xffc46a, `lights::street_lamp_chroma`)
/// takes its street's tint (3000 K off the streets) at its own luminance;
/// every other light as given. (The far lamps; the near ones are recoloured
/// by the renderer's gather from the same grid.)
pub fn recolour(lamps: &[crate::lights::LampLight], grid: &PaletteGrid) -> Vec<crate::lights::LampLight> {
    let key = crate::lights::street_lamp_chroma();
    lamps
        .iter()
        .map(|l| {
            let m = l.color[0].max(l.color[1]).max(l.color[2]).max(1e-9);
            if l.kind != crate::lights::LampKind::Street || (0..3).any(|i| (l.color[i] / m - key[i]).abs() > 0.02) {
                return *l;
            }
            let t = grid.at(l.pos[0], l.pos[2]).map(|s| s.0).unwrap_or(LampTint::Led3000);
            let y = eo_look::math::luminance(l.color);
            LampLight { color: t.colour().map(|c| c * y), ..*l }
        })
        .collect()
}

/// A cell's street: its tint and axis. None: no street.
pub fn decode(c: u8) -> Option<(LampTint, [f64; 2])> {
    if c & 0x80 == 0 {
        return None;
    }
    let a = ((c >> 2) & 15) as f64 / AXIS_STEPS as f64 * std::f64::consts::PI;
    Some((LampTint::from_code(c & 3), [a.cos(), a.sin()]))
}

// ---- the luminaire's pool (sky_light.wgsl `sky_cine_cobra`) ----

/// The intensity rise toward the along-street peak over the nadir's: I/I0 =
/// 1 + gain sin^2 theta there (2.8 x at 65 deg; IES type III cobra heads
/// peak at about 2.5-3.5 x their nadir, at 60-70 deg).
pub const COBRA_ALONG_GAIN: f64 = 2.2;
/// The soft roll-off along the street: full to 62 deg, none past 82 deg.
pub const COBRA_ALONG_ROLL_DEG: [f64; 2] = [62.0, 82.0];
/// Across the street (toward the house fronts and the far kerb): full to
/// 42 deg, none past 70 deg (type II/III: short across).
pub const COBRA_ACROSS_ROLL_DEG: [f64; 2] = [42.0, 70.0];
/// A head without an axis (a historic globe on its column): the along and
/// across shapes half and half, all round (type V).
pub const COBRA_ROUND_ALONG2: f64 = 0.5;
/// The light's reach in mounting heights (the default's 2.6 cut the pool
/// while the type III throw still lit the road).
pub const RANGE_MOUNTING_HEIGHTS: f64 = 3.4;
/// The source's radius (m) for the specular lobe: a cobra head's lens.
pub const STREET_SOURCE_RADIUS_M: f64 = 0.3;
/// A low beam's lens radius (m).
pub const HEADLAMP_SOURCE_RADIUS_M: f64 = 0.09;
/// The local lights' specular BRDF cap (1/sr; GGX's peak at alpha ~0.14).
pub const SPEC_CAP: f64 = 16.0;

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The cobra head's intensity over its nadir's toward a direction `cos_t`
/// (cosine from the nadir) whose horizontal part lies `along2` (cos^2 of
/// its angle to the street's axis: 1 along, 0 across) of the way along the
/// street. The roll-off's angles are mixed in cosine (as the shader does).
pub fn cobra_shape(cos_t: f64, along2: f64) -> f64 {
    let s2 = (1.0 - cos_t * cos_t).max(0.0);
    let gain = 1.0 + COBRA_ALONG_GAIN * s2 * along2;
    let c = |d: f64| d.to_radians().cos();
    let lo = c(COBRA_ACROSS_ROLL_DEG[0]) + (c(COBRA_ALONG_ROLL_DEG[0]) - c(COBRA_ACROSS_ROLL_DEG[0])) * along2;
    let hi = c(COBRA_ACROSS_ROLL_DEG[1]) + (c(COBRA_ALONG_ROLL_DEG[1]) - c(COBRA_ACROSS_ROLL_DEG[1])) * along2;
    gain * smoothstep(hi, lo, cos_t)
}

/// The light's window at `d` of its reach `range` (the default's
/// `(1 - (d / range)^4)^2`, flat near the lamp and with no slope at the
/// reach; over the longer reach it takes 1.5 % off the nadir).
pub fn pool_window(d: f64, range: f64) -> f64 {
    let x = (1.0 - (d / range).powi(4)).clamp(0.0, 1.0);
    x * x
}

/// Illuminance (lux) on the ground at (along, across) metres from the foot
/// of a cobra head `h` m up of nadir intensity `candela`.
pub fn cobra_lux(candela: f64, h: f64, along: f64, across: f64) -> f64 {
    let r2 = along * along + across * across;
    let d2 = h * h + r2;
    let d = d2.sqrt();
    let cos_t = h / d;
    let along2 = if r2 > 1e-9 { along * along / r2 } else { 1.0 };
    candela * cobra_shape(cos_t, along2) * pool_window(d, h * RANGE_MOUNTING_HEIGHTS) / d2 * cos_t
}

// ---- the low beam (sky_light.wgsl kind 1 under the look) ----

/// The cut-off's depression on the opposing (left) side, deg (49 CFR 571.108 S10.18.9.1.1).
pub const LOW_BEAM_CUT_LEFT_DEG: f64 = 0.4;
/// ... and on the kerb (right) side, deg below the horizontal (0: at H-H).
pub const LOW_BEAM_CUT_RIGHT_DEG: f64 = 0.0;
/// Over how many degrees to the right the cut-off steps up.
pub const LOW_BEAM_STEP_DEG: f64 = 2.0;
/// The cut-off's sharpness: from glare to full over this many degrees
/// (gradient >= 0.13 a tenth of a degree, S10.18.9.1.2).
pub const LOW_BEAM_EDGE_DEG: f64 = 0.35;
/// The glare zone's share of the peak above the cut-off (B50L ~625 cd of 20 000).
pub const LOW_BEAM_GLARE: f64 = 0.031;
/// The hot spot's horizontal centre (deg right) and half width (deg).
pub const LOW_BEAM_HOT_DEG: [f64; 2] = [1.5, 7.0];
/// The beam's floor away from the hot spot, as a share of the peak
/// (2D-15L/R >= 1 000, 4D-20L/R >= 300 of 20 000: ~0.05-0.2).
pub const LOW_BEAM_SPREAD: f64 = 0.18;

/// The low beam's intensity over its peak toward `dep_deg` below the
/// horizontal and `h_deg` right of the beam's axis (before its cone and
/// near-field limits): the cut-off, the hot spot, the foreground's fall.
pub fn low_beam_shape(dep_deg: f64, h_deg: f64) -> f64 {
    let cut = LOW_BEAM_CUT_LEFT_DEG + (LOW_BEAM_CUT_RIGHT_DEG - LOW_BEAM_CUT_LEFT_DEG) * smoothstep(0.0, LOW_BEAM_STEP_DEG, h_deg);
    let under = smoothstep(cut, cut + LOW_BEAM_EDGE_DEG, dep_deg);
    let hot = LOW_BEAM_SPREAD + (1.0 - LOW_BEAM_SPREAD) * (-((h_deg - LOW_BEAM_HOT_DEG[0]) / LOW_BEAM_HOT_DEG[1]).powi(2)).exp();
    let fall = if dep_deg > 2.5 { (2.5 / dep_deg).powi(2) } else { 1.0 };
    LOW_BEAM_GLARE + (hot * fall - LOW_BEAM_GLARE) * under
}

/// The GGX roughness a light of radius `r` at `d` m gives a surface of
/// roughness `rough` (alpha widened by r / 2d).
pub fn widened_roughness(rough: f64, r: f64, d: f64) -> f64 {
    (rough * rough + r / (2.0 * d.max(1e-3))).min(1.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photometry::{LAMP_POOL_PEAK_LUX, nadir_candela_for_pool};

    #[test]
    fn the_palette_keeps_3000_k_and_spreads_from_sodium_to_cool_white() {
        let led = LampTint::Led3000.colour();
        let base = crate::lights::led_street_lamp_color();
        for c in 0..3 {
            assert!((led[c] - base[c]).abs() < 1e-3, "3000 K is the default's LED colour: {led:?} vs {base:?}");
        }
        let na = LampTint::Sodium.colour();
        let cool = LampTint::Led4000.colour();
        let hist = LampTint::Historic.colour();
        // sodium deep orange (blue well under red), 4000 K cooler than 3000 K, the historic in between.
        assert!(na[2] / na[0] < 0.3 && na[1] / na[0] < 0.7, "sodium {na:?}");
        assert!(cool[2] / cool[0] > led[2] / led[0] + 0.1, "4000 K {cool:?} vs {led:?}");
        assert!(hist[2] / hist[0] < led[2] / led[0] && hist[2] / hist[0] > na[2] / na[0], "2700 K {hist:?}");
        for t in LampTint::ALL {
            let c = t.colour();
            assert!((eo_look::math::luminance(c) - 1.0).abs() < 1e-6, "{t:?} unit luminance");
        }
        println!("3000 {led:?} 4000 {cool:?} sodium {na:?} 2700 {hist:?}");
    }

    #[test]
    fn a_street_is_one_colour_and_the_rule_is_stable() {
        for name in ["Mission Street", "Castro Street", "The Embarcadero", "Market Street", "3rd Street"] {
            let a = street_tint(name, "primary");
            assert_eq!(a, street_tint(name, "primary"));
            println!("{name}: {a:?} (h {:.3})", hash01(name));
        }
        assert_eq!(street_tint("Anything", "footway"), None);
        assert_eq!(street_tint("I-80", "motorway"), Some(LampTint::Led4000));
        // shares over many names land near the rule's.
        let n = 20000;
        let mut na = 0;
        let mut cool = 0;
        for i in 0..n {
            match street_tint(&format!("Street {i}"), "secondary").unwrap() {
                LampTint::Sodium => na += 1,
                LampTint::Led4000 => cool += 1,
                _ => {}
            }
        }
        let (na, cool) = (na as f64 / n as f64, cool as f64 / n as f64);
        assert!((na - SODIUM_ARTERIAL_SHARE).abs() < 0.015 && (cool - COOL_ARTERIAL_SHARE).abs() < 0.015, "{na} {cool}");
    }

    #[test]
    fn the_grid_holds_a_street_along_its_length_and_the_lower_class_wins() {
        // an arterial along x (z = 0) and a freeway crossing it along z (x = 100).
        let a = [[0.0, 0.0], [200.0, 0.0]];
        let f = [[100.0, -100.0], [100.0, 100.0]];
        let streets = [
            StreetIn { name: "Some Arterial", highway: "secondary", width: 16.0, points: &a },
            StreetIn { name: "A Freeway", highway: "motorway", width: 30.0, points: &f },
        ];
        let g = PaletteGrid::build([-50.0, -150.0], 8.0, 50, 40, &streets);
        let own = street_tint("Some Arterial", "secondary").unwrap();
        for x in [5.0, 50.0, 100.0, 150.0, 195.0] {
            for z in [-14.0, 0.0, 14.0] {
                let (t, axis) = g.at(x, z).unwrap_or_else(|| panic!("no street at {x} {z}"));
                assert_eq!(t, own, "{x} {z}");
                assert!(axis[0].abs() > 0.98, "axis along x at {x} {z}: {axis:?}");
            }
        }
        // the freeway away from the arterial.
        let (t, axis) = g.at(100.0, 60.0).unwrap();
        assert_eq!(t, LampTint::Led4000);
        assert!(axis[1].abs() > 0.98, "{axis:?}");
        assert!(g.at(40.0, 60.0).is_none(), "off the streets");
        // the words carry the bytes.
        let w = g.words();
        let i = 7 * 50 + 3;
        assert_eq!(((w[i / 4] >> (8 * (i % 4))) & 255) as u8, g.cells[i]);
    }

    #[test]
    fn the_cobra_pool_keeps_its_nadir_and_is_long_along_the_street() {
        let h = crate::lights::ARTERIAL_HEAD_M;
        let cd = nadir_candela_for_pool(h);
        let nadir = cobra_lux(cd, h, 0.0, 0.0);
        assert!((nadir - LAMP_POOL_PEAK_LUX).abs() < 0.02 * LAMP_POOL_PEAK_LUX, "nadir {nadir}");
        // along the street the pool reaches further than across it.
        let along = cobra_lux(cd, h, 1.5 * h, 0.0);
        let across = cobra_lux(cd, h, 0.0, 1.5 * h);
        assert!(along > 2.0 * across, "along {along} across {across}");
        // no hard ring: the edge falls smoothly to zero (no step over 10 % of the nadir a 0.5 m step).
        let mut last = nadir;
        for i in 1..200 {
            let r = i as f64 * 0.25;
            let e = cobra_lux(cd, h, r, 0.0);
            assert!((last - e).abs() < 0.1 * LAMP_POOL_PEAK_LUX, "step at {r}: {last} -> {e}");
            last = e;
        }
        assert!(cobra_lux(cd, h, h * RANGE_MOUNTING_HEIGHTS, 0.0) < 1e-9);
        // round (no axis) between the two.
        let round = cd * cobra_shape(h / (h * h + 2.25 * h * h).sqrt(), COBRA_ROUND_ALONG2);
        assert!(round < cd * cobra_shape(h / (h * h + 2.25 * h * h).sqrt(), 1.0));
        println!("nadir {nadir:.1} lux, 1.5 h along {along:.2} / across {across:.2}");
    }

    #[test]
    fn the_low_beam_cuts_off_under_the_horizontal_and_steps_up_on_the_kerb_side() {
        // above the cut-off: the glare share only.
        assert!((low_beam_shape(-1.0, -5.0) - LOW_BEAM_GLARE).abs() < 1e-9);
        assert!((low_beam_shape(0.2, -5.0) - LOW_BEAM_GLARE).abs() < 1e-9, "0.2 D on the left is over its 0.4 D cut-off");
        // on the kerb side the same 0.2 D is under the cut-off (stepped up to H-H).
        assert!(low_beam_shape(0.2, 5.0) > 0.3, "{}", low_beam_shape(0.2, 5.0));
        // the hot spot (0.6D-1.3R >= 10 000 of 20 000) and the sides (2D-15L >= 1 000, 4D-20L >= 300).
        assert!(low_beam_shape(0.9, 1.3) > 0.5);
        assert!(low_beam_shape(2.0, -15.0) * 20000.0 >= 1000.0);
        assert!(low_beam_shape(4.0, -20.0) * 20000.0 >= 300.0);
        // 4D-V <= 10 000.
        assert!(low_beam_shape(4.0, 0.0) * 20000.0 <= 10000.0);
        // a wall straight ahead at the lamp's height is in the glare zone, not the beam.
        assert!(low_beam_shape(0.0, 0.0) < 0.05);
    }

    #[test]
    fn a_small_source_widens_a_mirror_lobe_and_the_cap_holds() {
        let r = widened_roughness(0.05, HEADLAMP_SOURCE_RADIUS_M, 5.0);
        assert!(r > 0.1 && r < 0.2, "{r}");
        assert!((widened_roughness(0.8, STREET_SOURCE_RADIUS_M, 8.0) - 0.8).abs() < 0.02);
        // GGX's peak at the cap's roughness.
        let a = 0.14f64;
        let peak = 1.0 / (std::f64::consts::PI * a * a);
        assert!(peak > SPEC_CAP);
    }
}
