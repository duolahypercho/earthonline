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
    fn the_pool_peaks_at_the_reference_and_spreads() {
        let l = street_lamp(0.0, 0.0, ARTERIAL_HEAD_M, 0.0);
        let peak = street_lamp_lux(&l, 0.0);
        assert!((peak - photometry::LAMP_POOL_PEAK_LUX).abs() < 1e-6);
        // Half a spacing away (16 m) the pool still lights the road.
        let mid = street_lamp_lux(&l, 16.0);
        assert!(mid > 1.0 && mid < peak, "{mid}");
    }
}
