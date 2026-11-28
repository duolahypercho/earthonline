//! The concrete pylons, the side spans' tie-downs, the backstays and the
//! anchorage housings (`BridgeFeatures::pylons`).
//!
//! The pylons S1 and S2 stand at the Fort Point arch's ends, N1 at the
//! Marin side span's shore end and N2 at the Marin anchorage housing
//! (goldengate.org, "Seismic Retrofit": "San Francisco Anchorage Housing and
//! Pylons S1 and S2", "Marin Anchorage Housing and Pylons N1 and N2"). S1 is
//! "two hollow concrete shafts, each about 32 x 36 ft" (Strauss 1937); the
//! export's support extents (122.5 ft across, 36 ft along) put each shaft on
//! its cable's plane, 32 ft across and 36 ft along, and the others are
//! "treated like S1" (Strauss 1937). The south pylons are "approximately
//! 220-ft-tall" (ENR, "Famed Golden Gate span undergoes complex seismic
//! revamp"). The pylons have "a chevron form in both plan and elevation"
//! and end in "staggered vertical forms" (goldengate.org, "Art Deco on a
//! Grand Scale"), with the towers' "vertical fluting ... of intersecting
//! plane facets" (Strauss 1937), in rough-board-formed concrete.
//!
//! At the shore end of each side span "6 short suspenders" loop over an
//! articulated cable band and hold the cable down to S1 / N1, the tie-down
//! steel set at el. 187 ft MLLW in the shafts (Strauss 1937); past them the
//! cables run on as backstays, at 16 deg 31 min (San Francisco) and 17 deg
//! 40 min (Marin) into the anchorage housings (Strauss 1937): the south one
//! with its west wall "235 ft long, 90 ft tall" (Construction Innovation
//! Forum, 2008), the north one "347'" long (CESMD station CE58700), its
//! roof the bridge floor (Strauss 1937).

use super::{ALL, Ctx, MeshArrays, NO_BOTTOM, num};
use crate::Collider;
use crate::suspension::{self, Run};

/// Feet in metres.
const FT: f64 = 0.3048;
/// S1's shafts, across and along the span (Strauss 1937, read with the export's extents).
const PYLON_SHAFT_FT: [f64; 2] = [32.0, 36.0];
/// The south pylons' height (ENR), from S1's lower foot.
const SOUTH_PYLON_FT: f64 = 220.0;
/// The tie-down steel's elevation in S1's shafts, ft over MLLW (Strauss 1937).
const TIE_DOWN_MLLW_FT: f64 = 187.0;
/// MSL over MLLW at NOAA station 9414290 (epoch 1983-2001), metres.
pub(super) const MSL_OVER_MLLW_M: f64 = 0.951;
/// The backstays' angles at the anchorages (Strauss 1937), degrees.
const BACKSTAY_DEG: [f64; 2] = [16.0 + 31.0 / 60.0, 17.0 + 40.0 / 60.0];
/// The housings' lengths along the span: the south west wall's 235 ft and its
/// 90 ft height (CIF 2008), the north housing's 347 ft (CESMD).
const SOUTH_HOUSING_FT: [f64; 2] = [235.0, 90.0];
const NORTH_HOUSING_FT: f64 = 347.0;

/// The centre-to-centre separation of N1 and N2. The measured housing starts
/// at the south face of N1 and ends at the north face of N2.
pub(super) fn north_pylon_spacing() -> f64 {
    (NORTH_HOUSING_FT - PYLON_SHAFT_FT[1]) * FT * eo_data::groundscale::scale()
}

pub(super) fn housing_half_width(site: &super::Site) -> f64 {
    site.half_plane + PYLON_SHAFT_FT[0] * FT * eo_data::groundscale::scale() / 2.
}

pub(super) fn north_roof_top(deck: f64) -> f64 {
    deck - 0.35
}
/// The tie-down's short suspenders (Strauss 1937).
const TIE_DOWN_ROPES: usize = 6;

/// INFERRED (no drawing found): the walkway's opening through a shaft (its
/// height over the deck; it reaches from the shaft's inner face to
/// `WALK_CABLE_COVER_M` short of the cable plane, so the cable, which bends
/// down into the backstay in the shaft, stays in the concrete); the fluting's pitch and depth; the staggered top's
/// step height and inset; where the cable meets the pylon (over the deck by
/// the railing and half a metre: "at about the level of the truss top" in
/// the Fort Point photograph); the tie-down band's length and the ropes'
/// spacing along it; the seat's depth; the spacing clamps' drop under the
/// cable. Lengths times the structure scale where they are the structure's.
const WALK_OPENING_M: f64 = 3.6;
const WALK_CABLE_COVER_M: f64 = 1.2;
const FLUTE_M: [f64; 2] = [1.8, 0.45];
const TOP_STEP_M: [f64; 2] = [1.8, 1.1];
const TIE_OVER_RAILING_M: f64 = 0.5;
const TIE_BAND_M: [f64; 2] = [3.2, 0.45];
const TIE_SEAT_M: f64 = 2.0;
const SPACING_CLAMP_BELOW_M: f64 = 1.6;

/// A pylon: its name, chainage, its shafts' half sizes (across, along),
/// their foot and top.
pub(super) struct Pylon {
    pub name: &'static str,
    pub c: f64,
    pub half_across: f64,
    pub half_along: f64,
    pub foot: f64,
    centres: [f64; 2],
    pub top: f64,
}

/// An anchorage housing: from c0 to c1 along the span (c0 < c1), half its
/// width, and its top at a chainage (flat in the south; under the deck in
/// the north, whose roof is the bridge floor).
pub(super) struct Housing {
    pub c0: f64,
    pub c1: f64,
    pub half: f64,
    pub north: bool,
    pub flat_top: f64,
}

impl Ctx<'_> {
    fn plan_scale(&self) -> f64 {
        eo_data::groundscale::scale()
    }

    /// The four pylons (S2, S1, N1, N2).
    pub(super) fn pylons(&self) -> Vec<Pylon> {
        let p = self.plan;
        let (gs, h) = (self.plan_scale(), self.h);
        let (half_across, half_along) = (PYLON_SHAFT_FT[0] * FT * gs / 2.0, PYLON_SHAFT_FT[1] * FT * gs / 2.0);
        let hp = self.site.half_plane;
        let foot_at = |c: f64, centres: [f64; 2]| {
            let mut lo = f64::INFINITY;
            for side in centres {
                for dc in [-half_along, 0.0, half_along] {
                    for ds in [-half_across, 0.0, half_across] {
                        let [x, z] = self.frame.point(c + dc, side + ds);
                        lo = lo.min((self.ground)(x, z));
                    }
                }
            }
            lo
        };
        let south = |name: &'static str, c: f64| {
            let centres = [-hp, hp];
            let foot = foot_at(c, centres);
            Pylon { name, c, half_across, half_along, centres, foot: foot - 1.0, top: foot + SOUTH_PYLON_FT * FT * h }
        };
        let s1 = south("S1", p.arch_north_c);
        // the others stand as far over their deck as S1 over its own (INFERRED: "treated like S1";
        // S2's foot is on the hillside's slope, so 220 ft from it would not be S1's height over the deck).
        let over = s1.top - p.deck_top(s1.c);
        let like_s1 = |name: &'static str, c: f64| {
            let centres = [-hp, hp];
            let foot = foot_at(c, centres);
            Pylon { name, c, half_across, half_along, centres, foot: foot - 1.0, top: p.deck_top(c) + over }
        };
        let s2 = like_s1("S2", p.arch_south_c);
        let n1 = like_s1("N1", p.anchor_north_c);
        // the housing runs from N1's south face to N2's north face, 347 ft.
        let n2_c = super::expansion_joints(self.g, self.site)[0];
        let n2 = like_s1("N2", n2_c);
        let mut pylons = vec![s2, s1, n1, n2];
        if super::pylon_survey::enabled() {
            for py in &mut pylons {
                let pair = super::pylon_survey::pair(py.name);
                let half = (pair[1].centre() - pair[0].centre()) * gs / 2.;
                py.centres = [-half, half];
                py.foot = foot_at(py.c, py.centres) - 1.;
                py.top = p.deck_top(py.c) + h * pair[0].height().max(pair[1].height());
            }
        }
        pylons
    }

    /// The two anchorage housings: the south one from S2 south, the north one from N1 to N2.
    pub(super) fn housings(&self) -> Vec<Housing> {
        let (gs, h) = (self.plan_scale(), self.h);
        let pyl = self.pylons();
        let half = housing_half_width(self.site);
        let s2 = &pyl[0];
        let (c0, c1) = (s2.c + s2.half_along - SOUTH_HOUSING_FT[0] * FT * gs, s2.c + s2.half_along);
        // its west (bay side) wall stands 90 ft over its own foot.
        let mut west = f64::INFINITY;
        let mut c = c0;
        while c <= c1 {
            let [x, z] = self.frame.point(c, -half);
            west = west.min((self.ground)(x, z));
            c += 4.0;
        }
        let flat_top = (west + SOUTH_HOUSING_FT[1] * FT * h).min(self.plan.deck_top(c0) - num(self.d, "trussDepthM"));
        let (n1, n2) = (&pyl[2], &pyl[3]);
        vec![Housing { c0, c1, half, north: false, flat_top }, Housing { c0: n1.c - n1.half_along, c1: n2.c + n2.half_along, half, north: true, flat_top: 0.0 }]
    }

    fn housing_top(&self, hs: &Housing, c: f64) -> f64 {
        if hs.north { north_roof_top(self.plan.deck_top(c)) } else { hs.flat_top }
    }

    /// Where a bent stands on a housing's roof: its top there (else none).
    pub(super) fn housing_under(&self, c: f64) -> Option<f64> {
        if !self.pylons {
            return None;
        }
        self.housings().iter().find(|hs| c >= hs.c0 && c <= hs.c1).map(|hs| self.housing_top(hs, c))
    }

    /// The cable runs with the pylons: each side span from the tower down to
    /// its pylon's face at about deck level, and the backstays on from there.
    pub(super) fn cable_runs4(&self, saddle: f64, sag_main: f64) -> Vec<Run> {
        let (d, p) = (self.d, self.plan);
        let pyl = self.pylons();
        let (s1, n1) = (&pyl[1], &pyl[2]);
        let over = num(d, "railingM") + TIE_OVER_RAILING_M;
        let (cs, cn) = (s1.c + s1.half_along, n1.c - n1.half_along);
        let (ys, yn) = (p.deck_top(cs) + over, p.deck_top(cn) + over);
        let s2 = &pyl[0];
        let back_s = (cs - s2.c) * BACKSTAY_DEG[0].to_radians().tan();
        // the Marin backstay runs into its housing until 20 m under the deck (the rest is hidden).
        let back_n_len = 20.0 / BACKSTAY_DEG[1].to_radians().tan();
        vec![
            Run { c0: s2.c, c1: cs, y0: ys - back_s, y1: ys, sag: 0.0 },
            Run { c0: cs, c1: 0.0, y0: ys, y1: saddle, sag: suspension::side_span_sag(sag_main, p.l, -cs) },
            Run { c0: 0.0, c1: p.l, y0: saddle, y1: saddle, sag: sag_main },
            Run { c0: p.l, c1: cn, y0: saddle, y1: yn, sag: suspension::side_span_sag(sag_main, p.l, cn - p.l) },
            Run { c0: cn, c1: cn + back_n_len, y0: yn, y1: yn - 20.0, sag: 0.0 },
        ]
    }

    /// A shaft's fluted outline about (c, s), half sizes `ha` along and `hc`
    /// across, the faces along the span and the outer face (toward `out`)
    /// faceted in V flutes; in world x, z.
    fn fluted_ring(&self, c: f64, s: f64, ha: f64, hc: f64, out: f64) -> Vec<[f64; 2]> {
        let (pitch, depth) = (FLUTE_M[0] * self.h, FLUTE_M[1] * self.h);
        let mut pts: Vec<[f64; 2]> = Vec::new();
        // corners counter-clockwise in (c, s): (+ha, -hc) -> (+ha, +hc) -> (-ha, +hc) -> (-ha, -hc).
        let corners = [[ha, -hc], [ha, hc], [-ha, hc], [-ha, -hc]];
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            // the inward normal of this face (toward the centre).
            let face_c = (a[0] + b[0]) / 2.0;
            let face_s = (a[1] + b[1]) / 2.0;
            let along_face = k % 2 == 0; // faces at c = +-ha
            let fluted = along_face || face_s * out > 0.0;
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let n = if fluted { ((len / pitch).round() as usize).max(1) } else { 1 };
            let inward = if along_face { [-face_c.signum(), 0.0] } else { [0.0, -face_s.signum()] };
            for i in 0..2 * n {
                let t = i as f64 / (2 * n) as f64;
                let mut q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                if fluted && i % 2 == 1 {
                    q = [q[0] + inward[0] * depth, q[1] + inward[1] * depth];
                }
                pts.push(self.frame.point(c + q[0], s + q[1]));
            }
        }
        crate::geometry::orient_ring(&pts)
    }

    /// An extruded ring from y0 to y1 (walls, and the top when `top`).
    fn extrude(m: &mut MeshArrays, ring: &[[f64; 2]], y0: f64, y1: f64, top: bool, colour: super::Rgb) {
        let n = ring.len();
        let (mut cx, mut cz) = (0.0, 0.0);
        for p in ring {
            cx += p[0] / n as f64;
            cz += p[1] / n as f64;
        }
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            m.quad([a[0], y0, a[1]], [b[0], y0, b[1]], [b[0], y1, b[1]], [a[0], y1, a[1]], colour);
            if top {
                m.tri([a[0], y1, a[1]], [b[0], y1, b[1]], [cx, y1, cz], colour);
            }
        }
    }

    /// The pylons' shafts: under the deck, the walkway's opening (the outer
    /// part solid), over it to a staggered top of three tiers.
    pub(super) fn draw_pylons(&self, concrete: &mut MeshArrays, far: &mut MeshArrays) {
        let (p, h, k) = (self.plan, self.h, &self.k);
        for py in self.pylons() {
            let deck = p.deck_top(py.c);
            // (`concrete2`) S1 and S2 in the retrofit's renewed concrete.
            let colour = if py.name.starts_with('S') { k.concrete_south } else { k.concrete };
            let (open0, open1) = (deck - 0.35, deck + WALK_OPENING_M * h);
            let (step, inset) = (TOP_STEP_M[0] * h, TOP_STEP_M[1] * h);
            for (index, s) in py.centres.into_iter().enumerate() {
                let sgn = s.signum();
                let ring = self.fluted_ring(py.c, s, py.half_along, py.half_across, sgn);
                Self::extrude(concrete, &ring, py.foot, open0, false, colour);
                if super::pylon_survey::enabled() {
                    let cap = super::pylon_survey::pair(py.name)[index];
                    let draw_crown = |mesh: &mut MeshArrays| cap.draw(mesh, self.frame, py.c, s, deck, open1, open0, WALK_CABLE_COVER_M, h, colour);
                    draw_crown(concrete);
                    let [x, z] = self.frame.point(py.c, s);
                    if super::pylon_survey::visible_at_distance() {
                        // Keep the terraces and walkway opening when the landmark switches detail.
                        far.box_(x, z, py.foot, open0, py.half_along, py.half_across, self.yaw, colour, crate::geometry::NO_TOP);
                        draw_crown(far);
                    } else {
                        far.box_(x, z, py.foot, deck + cap.height() * h, py.half_along, py.half_across, self.yaw, colour, ALL);
                    }
                    continue;
                }
                // the outer part beside the opening, from short of the cable plane out (the cable passes in it).
                let (s0, s1) = (s - sgn * WALK_CABLE_COVER_M, s + sgn * py.half_across);
                let [x, z] = self.frame.point(py.c, (s0 + s1) / 2.0);
                concrete.box_(x, z, open0, open1, py.half_along, (s1 - s0).abs() / 2.0, self.yaw, colour, NO_BOTTOM);
                // over the opening (its soffit), then the three tiers.
                let [x, z] = self.frame.point(py.c, s);
                concrete.box_(x, z, open1 - 0.01, open1 + 0.4 * h, py.half_along, py.half_across, self.yaw, colour, ALL);
                let tiers = [(open1 + 0.4 * h, py.top - 2.0 * step, 0.0), (py.top - 2.0 * step, py.top - step, inset), (py.top - step, py.top, 2.0 * inset)];
                for (y0, y1, i) in tiers {
                    if y1 - y0 < 0.05 {
                        continue;
                    }
                    let ring = self.fluted_ring(py.c, s, py.half_along - i, py.half_across - i, sgn);
                    Self::extrude(concrete, &ring, y0, y1, true, colour);
                }
                far.box_(x, z, py.foot, py.top, py.half_along, py.half_across, self.yaw, colour, ALL);
            }
        }
        for hs in self.housings() {
            let colour = if hs.north { k.concrete } else { k.concrete_south };
            let n = ((hs.c1 - hs.c0) / 12.0).ceil().max(1.0) as usize;
            let mut foot = f64::INFINITY;
            for i in 0..=8 {
                let c = hs.c0 + (hs.c1 - hs.c0) * i as f64 / 8.0;
                for sv in [-hs.half, 0.0, hs.half] {
                    let [x, z] = self.frame.point(c, sv);
                    foot = foot.min((self.ground)(x, z));
                }
            }
            for i in 0..n {
                let (a, b) = (hs.c0 + (hs.c1 - hs.c0) * i as f64 / n as f64, hs.c0 + (hs.c1 - hs.c0) * (i + 1) as f64 / n as f64);
                let top = self.housing_top(&hs, (a + b) / 2.0);
                let [x, z] = self.frame.point((a + b) / 2.0, 0.0);
                concrete.box_(x, z, foot - 1.0, top, (b - a) / 2.0 + 0.01, hs.half, self.yaw, colour, NO_BOTTOM);
                far.box_(x, z, foot - 1.0, top, (b - a) / 2.0, hs.half, self.yaw, colour, ALL);
            }
        }
    }

    /// The tie-downs at S1 and N1: the band on the cable just off the
    /// pylon's face, its six short suspenders down to a seat on the face at
    /// the tie-down steel's level; and the suspender spacing clamps under the
    /// cable at every suspender station (wires).
    pub(super) fn tie_downs(&self, wires: &mut MeshArrays, concrete: &mut MeshArrays, runs: &[Run]) {
        let (d, p, h, k, w) = (self.d, self.plan, self.h, &self.k, self.water);
        let pyl = self.pylons();
        let hp = self.site.half_plane;
        let tie_y_s = w + (TIE_DOWN_MLLW_FT * FT - MSL_OVER_MLLW_M) * h;
        let below_deck = p.deck_top(pyl[1].c) - tie_y_s;
        let rope_d = num(d, "suspenderDiameterM");
        let band_d = num(d, "cableDiameterM") + 0.3;
        for (py, dir) in [(&pyl[1], 1.0), (&pyl[2], -1.0)] {
            let face = py.c + dir * py.half_along;
            let band_c = face + dir * (TIE_BAND_M[0] / 2.0 + 0.6);
            let seat_y = p.deck_top(py.c) - below_deck;
            for sgn in [-1.0, 1.0] {
                let s = sgn * hp;
                let Some(yc) = suspension::cable_height_at(runs, band_c) else { continue };
                let (ya, yb) = (suspension::cable_height_at(runs, band_c - TIE_BAND_M[0] / 2.0).unwrap_or(yc), suspension::cable_height_at(runs, band_c + TIE_BAND_M[0] / 2.0).unwrap_or(yc));
                wires.wire_segment(self.frame.P(band_c - TIE_BAND_M[0] / 2.0, s, ya), self.frame.P(band_c + TIE_BAND_M[0] / 2.0, s, yb), band_d, k.orange);
                for i in 0..TIE_DOWN_ROPES {
                    let dc = (i as f64 - (TIE_DOWN_ROPES as f64 - 1.0) / 2.0) * TIE_BAND_M[1];
                    let c = band_c + dc;
                    let top = suspension::cable_height_at(runs, c).unwrap_or(yc);
                    wires.wire_segment(self.frame.P(c, s, seat_y), self.frame.P(c, s, top), rope_d, k.orange);
                }
                // the seat: a corbel off the shaft's face under the ropes.
                let reach = TIE_BAND_M[0] + 1.2;
                let [x, z] = self.frame.point(face + dir * reach / 2.0, s);
                concrete.box_(x, z, seat_y - TIE_SEAT_M * h, seat_y, reach / 2.0, 1.2, self.yaw, k.concrete, ALL);
            }
        }
        // the spacing clamps: a short bar across each suspender pair under the cable.
        let spacing = num(d, "suspenderSpacingM");
        let gap = super::PAIR_GAP_M;
        for r in runs {
            if r.c1 - r.c0 < 60.0 {
                continue;
            }
            let n = ((r.c1 - r.c0) / spacing).floor() as i64;
            let offset = ((r.c1 - r.c0) - n as f64 * spacing) / 2.0;
            for i in 0..=n {
                let c = r.c0 + offset + i as f64 * spacing;
                if c - r.c0 < 6.0 || r.c1 - c < 6.0 {
                    continue;
                }
                let Some(top) = suspension::cable_height_at(&[*r], c) else { continue };
                let y = top - SPACING_CLAMP_BELOW_M;
                if y - (p.deck_top(c) - 0.5) < 1.0 {
                    continue;
                }
                for s in [-hp, hp] {
                    if self.cable_detail {
                        // (`cable_detail`) the clamp round the four legs: a bar along each pair of legs, one across each rope.
                        let r = super::cables::leg_half_across(num(d, "cableDiameterM"), super::CABLE_BAND_WALL_M, num(d, "suspenderDiameterM"));
                        for side in [-r, r] {
                            wires.wire_segment(self.frame.P(c - gap / 2.0 - 0.12, s + side, y), self.frame.P(c + gap / 2.0 + 0.12, s + side, y), 0.14, k.orange);
                        }
                        for dc in [-gap / 2.0, gap / 2.0] {
                            wires.wire_segment(self.frame.P(c + dc, s - r - 0.1, y), self.frame.P(c + dc, s + r + 0.1, y), 0.1, k.orange);
                        }
                        continue;
                    }
                    wires.wire_segment(self.frame.P(c - gap / 2.0 - 0.12, s, y), self.frame.P(c + gap / 2.0 + 0.12, s, y), 0.14, k.orange);
                }
            }
        }
    }

    /// The pylons' shafts and the housings as colliders (to 30 m over their foot, as the other parts).
    pub(super) fn pylon_colliders(&self) -> Vec<Collider> {
        let mut out = Vec::new();
        for py in self.pylons() {
            for (s, side) in py.centres.into_iter().zip(["west", "east"]) {
                let [x, z] = self.frame.point(py.c, s);
                out.push(Collider { part: format!("{}-{side}", py.name), x, z, y0: py.foot, y1: py.top.min(py.foot + 30.0), hx: py.half_along, hz: py.half_across, yaw: self.yaw });
            }
        }
        for (i, hs) in self.housings().iter().enumerate() {
            let c = (hs.c0 + hs.c1) / 2.0;
            let [x, z] = self.frame.point(c, 0.0);
            let ground = (self.ground)(x, z);
            let top = self.housing_top(hs, c);
            out.push(Collider { part: format!("anchorage-housing-{i}"), x, z, y0: ground - 1.0, y1: top.min(ground + 30.0), hx: (hs.c1 - hs.c0) / 2.0, hz: hs.half, yaw: self.yaw });
        }
        out
    }
}

/// The dimensions this pass takes from published sources, as drawn (times
/// the ground plan's and the structure's scales), for the tests.
pub fn published_dimensions() -> Vec<(&'static str, f64, f64)> {
    let (gs, h) = (eo_data::groundscale::scale(), super::structure_scale());
    vec![
        ("pylon shaft across (32 ft)", PYLON_SHAFT_FT[0] * FT, PYLON_SHAFT_FT[0] * FT * gs),
        ("pylon shaft along (36 ft)", PYLON_SHAFT_FT[1] * FT, PYLON_SHAFT_FT[1] * FT * gs),
        ("south pylon height (220 ft)", SOUTH_PYLON_FT * FT, SOUTH_PYLON_FT * FT * h),
        ("tie-down over MSL (el. 187 ft MLLW)", TIE_DOWN_MLLW_FT * FT - MSL_OVER_MLLW_M, (TIE_DOWN_MLLW_FT * FT - MSL_OVER_MLLW_M) * h),
        ("south housing length (235 ft)", SOUTH_HOUSING_FT[0] * FT, SOUTH_HOUSING_FT[0] * FT * gs),
        ("south housing wall (90 ft)", SOUTH_HOUSING_FT[1] * FT, SOUTH_HOUSING_FT[1] * FT * h),
        ("north housing length (347 ft)", NORTH_HOUSING_FT * FT, NORTH_HOUSING_FT * FT * gs),
    ]
}
