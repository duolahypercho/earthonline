//! The cables' and the deck edge's detail (`BridgeFeatures::cable_detail`).
//!
//! The suspenders are "double suspenders (4 parts)": two ropes looped over
//! a cast-steel cable band, so four rope legs hang at each band, every 50 ft,
//! their feet in forged sockets on the truss verticals' webs (Strauss 1937;
//! ropes 2 11/16 in: goldengate.org, "Design & Construction Stats"). Each
//! rope's two legs straddle the cable; the two ropes stand side by side along
//! the span (`PAIR_GAP_M`, round 2's INFERRED gap). Drawn: the band with a
//! collar at each end, each rope as its loop over the band's top and its two
//! legs down to a socket, the legs apart across the bridge by the loop's
//! diameter (the cable, the band's wall and the rope: about the published
//! "one cable diameter").
//!
//! The lamp standards: "128 lamp posts", "lean, angled form", 1937 spacing
//! "150 ft, opposite each other on both sides" (goldengate.org, "Color & Art
//! Deco Styling"; Strauss 1937): placed at that pitch on the ground plan
//! (lights.rs `gg_deck_pitch_m`, where the night's luminaires stand too), in
//! round 2's standard form: the 1937 standard's sections, head and arm were
//! not found, so no new form is invented for it. INFERRED: the band's collars.

use super::{Ctx, MeshArrays, PAIR_GAP_M, Rgb, num};
use crate::suspension::{Frame, Run, cable_height_at};

/// INFERRED: a band's end collar (length, how much proud of the band), metres;
/// the loop's segments over the band's half circle.
const COLLAR_M: [f64; 2] = [0.14, 0.035];
const LOOP_SEGMENTS: usize = 4;

/// The legs' half spacing across the bridge: the loop's radius over the
/// cable (its radius, the band's wall, the rope's radius).
pub fn leg_half_across(cable_d: f64, band_wall: f64, rope_d: f64) -> f64 {
    cable_d / 2.0 + band_wall + rope_d / 2.0
}

/// The four-leg suspender groups and their bands at `suspension::suspender_pairs`'
/// own stations (the same end clearance and spacing). Returns the stations per plane.
#[allow(clippy::too_many_arguments)]
pub fn suspender_groups(wires: &mut MeshArrays, frame: &Frame, runs: &[Run], attach_y: &dyn Fn(f64) -> f64, half_plane: f64, spacing: f64, rope_d: f64, cable_d: f64, band_len: f64, band_wall: f64, socket: [f64; 2], colour: Rgb) -> usize {
    let end_clear = 6.0;
    let r = leg_half_across(cable_d, band_wall, rope_d);
    let band_d = cable_d + 2.0 * band_wall;
    let mut count = 0;
    for run in runs {
        let n = ((run.c1 - run.c0) / spacing).floor() as i64;
        let offset = ((run.c1 - run.c0) - n as f64 * spacing) / 2.0;
        for k in 0..=n {
            let c = run.c0 + offset + k as f64 * spacing;
            if c - run.c0 < end_clear || run.c1 - c < end_clear {
                continue;
            }
            let one = [*run];
            let (Some(top), Some(a), Some(b)) = (cable_height_at(&one, c), cable_height_at(&one, c - band_len / 2.0), cable_height_at(&one, c + band_len / 2.0)) else { continue };
            let bottom = attach_y(c);
            if top - bottom < 0.5 {
                continue;
            }
            for s in [-half_plane, half_plane] {
                // the band and its collars, on the cable's axis.
                wires.wire_segment(frame.P(c - band_len / 2.0, s, a), frame.P(c + band_len / 2.0, s, b), band_d, colour);
                for end in [-1.0, 1.0] {
                    let (c0, c1) = (c + end * band_len / 2.0, c + end * (band_len / 2.0 - COLLAR_M[0]));
                    let (y0, y1) = (cable_height_at(&one, c0).unwrap_or(top), cable_height_at(&one, c1).unwrap_or(top));
                    wires.wire_segment(frame.P(c0, s, y0), frame.P(c1, s, y1), band_d + 2.0 * COLLAR_M[1], colour);
                }
                for d in [-PAIR_GAP_M / 2.0, PAIR_GAP_M / 2.0] {
                    let cc = c + d;
                    let yc = cable_height_at(&one, cc).unwrap_or(top);
                    // the loop over the band's top, from one leg to the other.
                    let pt = |k: usize| {
                        let t = std::f64::consts::PI * k as f64 / LOOP_SEGMENTS as f64;
                        frame.P(cc, s - r * t.cos(), yc + r * t.sin())
                    };
                    for k in 0..LOOP_SEGMENTS {
                        wires.wire_segment(pt(k), pt(k + 1), rope_d, colour);
                    }
                    // the two legs and their sockets.
                    for side in [-1.0, 1.0] {
                        wires.wire_segment(frame.P(cc, s + side * r, bottom), frame.P(cc, s + side * r, yc), rope_d, colour);
                        wires.wire_segment(frame.P(cc, s + side * r, bottom), frame.P(cc, s + side * r, bottom + socket[0]), socket[1], colour);
                    }
                }
            }
            count += 1;
        }
    }
    count
}

/// The pole's offset off the curb line and the luminaire's centre off the
/// pole's base (across toward the roadway, up), for a standard of `head`:
/// round 2's standard (golden_gate.rs `lamp_standard`), whose housing runs
/// 0.3-1.2 m over the roadway with its top `head` - 0.1 + 0.13 m up; the
/// luminaire sits at the housing's middle and underside (lights.rs places
/// the night's luminaire there).
pub const LAMP2_CURB_OFFSET_M: f64 = 0.25;
pub fn lamp2_luminaire(head: f64) -> (f64, f64) {
    (0.75, head - 0.1 - 0.13)
}

impl Ctx<'_> {
    /// The deck's lamp standards at the published pitch (`cable_detail`), in
    /// round 2's form: the 1937 standard's sections were not found, so its
    /// form stays the one round 2 drew (golden_gate.rs `lamp_standard`).
    pub(super) fn lamp_standards2(&self, orange: &mut MeshArrays) {
        let (d, p, k) = (self.d, self.plan, &self.k);
        let half = num(d, "roadwayM") / 2.0;
        let head = crate::lights::DECK_HEAD_M;
        for c in super::lamp_stations(self.g, p) {
            let y = p.deck_top(c) + num(d, "curbM");
            for sgn in [-1.0, 1.0] {
                super::lamp_standard(orange, self.frame, c, sgn * (half + LAMP2_CURB_OFFSET_M), -sgn, y, head, k.orange);
            }
        }
    }
}
