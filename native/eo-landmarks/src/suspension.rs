//! The parts every suspension bridge is made of: a port of
//! src/world/landmarks/suspension-bridge.js (`bridgeFrame`, `frameInward`,
//! `stations`, `cableHeightAt`, `sideSpanSag`, `mainCables`,
//! `suspenderPairs`, `stiffeningTruss`, `roadDeck`). Every size comes in
//! through the caller's spec; markings and barrier sections are read from the
//! California standards table (`caMetric`) here, as the JS reads them.

use crate::geometry::{Faces, MeshArrays, NO_BOTTOM, Rgb, V3};
use std::sync::OnceLock;

/// The bridges' fidelity features (both bridges; each on by default and
/// turned off by its eo-app switch, which restores the earlier output;
/// `--no-bridges2` turns every one off).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BridgeFeatures {
    /// `--no-bridge-scale`: the structures' heights follow the drawn
    /// terrain's height scale (`eo_data::groundscale::height_scale`), as
    /// their spans follow the ground plan, so each bridge keeps the surveyed
    /// proportions (span to tower height, deck clearance to truss depth) on
    /// the 1.25 x plan instead of standing 1 / 1.25 as tall as its own span
    /// and its shores allow.
    pub scale: bool,
    /// `--no-bridge-paint`: the cited paint colours (International Orange;
    /// the west span's aluminium grey) instead of the export's tones.
    pub paint: bool,
    /// `--no-bridge-detail`: the towers' portal bracing, setbacks and caps,
    /// the deck's painted underside and the heavier west-span members.
    pub detail: bool,
    /// `--no-bridge-wires`: the cables and suspenders drawn after the
    /// atmosphere at every distance, anti-aliased at a screen-space floor
    /// width (the far item then carries no cable beams).
    pub wires: bool,
    /// `--no-bridge-night`: the Golden Gate towers' floodlights and the west
    /// span's suspender LEDs.
    pub night: bool,
}

impl BridgeFeatures {
    pub const ALL: Self = Self { scale: true, paint: true, detail: true, wires: true, night: true };
    pub const OFF: Self = Self { scale: false, paint: false, detail: false, wires: false, night: false };

    /// The features a command line leaves on.
    pub fn from_args<S: AsRef<str>>(args: &[S]) -> Self {
        let has = |f: &str| args.iter().any(|a| a.as_ref() == f);
        if has("--no-bridges2") {
            return Self::OFF;
        }
        Self { scale: !has("--no-bridge-scale"), paint: !has("--no-bridge-paint"), detail: !has("--no-bridge-detail"), wires: !has("--no-bridge-wires"), night: !has("--no-bridge-night") }
    }
}

static FEATURES: OnceLock<BridgeFeatures> = OnceLock::new();

/// The process's bridge features: pinned by `pin_features`, else read once from its arguments.
pub fn features() -> BridgeFeatures {
    *FEATURES.get_or_init(|| BridgeFeatures::from_args(&std::env::args().collect::<Vec<_>>()))
}

/// Pin the features before anything reads them (a test, a tool); false when
/// the process already reads others.
pub fn pin_features(f: BridgeFeatures) -> bool {
    *FEATURES.get_or_init(|| f) == f
}

/// The kill switches `features` reads (eo-app accepts them).
pub const SWITCHES: &[&str] = &["--no-bridges2", "--no-bridge-scale", "--no-bridge-paint", "--no-bridge-detail", "--no-bridge-wires", "--no-bridge-night"];

/// A frame along a bridge: chainage c along `axis` from `origin`, offset s to
/// its right; `offset`, when given, adds a lateral offset per chainage (the
/// Golden Gate's deck follows its carriageways' curve at each end).
pub struct Frame<'a> {
    pub origin: [f64; 2],
    pub axis: [f64; 2],
    pub across: [f64; 2],
    pub yaw: f64,
    pub offset: Option<&'a dyn Fn(f64) -> f64>,
}

impl<'a> Frame<'a> {
    pub fn new(origin: [f64; 2], axis: [f64; 2]) -> Self {
        Self { origin, axis, across: [-axis[1], axis[0]], yaw: axis[1].atan2(axis[0]), offset: None }
    }
    pub fn point(&self, c: f64, s: f64) -> [f64; 2] {
        let s = match self.offset {
            Some(f) => s + f(c),
            None => s,
        };
        [self.origin[0] + self.axis[0] * c + self.across[0] * s, self.origin[1] + self.axis[1] * c + self.across[1] * s]
    }
    #[allow(non_snake_case)]
    pub fn P(&self, c: f64, s: f64, y: f64) -> V3 {
        let [x, z] = self.point(c, s);
        [x, y, z]
    }
    pub fn chainage(&self, x: f64, z: f64) -> f64 {
        (x - self.origin[0]) * self.axis[0] + (z - self.origin[1]) * self.axis[1]
    }
    /// `frameInward`: across the axis toward the centreline, from wherever a
    /// face stands more than `min` off it.
    pub fn inward(&self) -> impl Fn(f64, f64, f64) -> Option<[f64; 2]> + use<> {
        let [ax, az] = self.across;
        let [ox, oz] = self.origin;
        move |x: f64, z: f64, min: f64| {
            let s = (x - ox) * ax + (z - oz) * az;
            if s.abs() < min {
                return None;
            }
            Some(if s > 0.0 { [-ax, -az] } else { [ax, az] })
        }
    }
}

/// One stage of a tower leg with its corners notched (`BridgeFeatures::detail`):
/// a riveted cellular leg's re-entrant corners, which draw a shadow line down
/// each corner of the leg and its setbacks. Two boxes, the inner one's top
/// 3 cm under the outer's (no coplanar tops). INFERRED: the notch, 0.18 of
/// the leg's smaller half size.
#[allow(clippy::too_many_arguments)]
pub fn notched_box(m: &mut MeshArrays, x: f64, z: f64, y0: f64, y1: f64, half_along: f64, half_across: f64, yaw: f64, colour: Rgb, faces: Faces) {
    let n = 0.18 * half_along.min(half_across);
    m.box_(x, z, y0, y1, half_along, half_across - n, yaw, colour, faces);
    m.box_(x, z, y0, y1 - 0.03, half_along - n, half_across, yaw, colour, faces);
}

pub fn stations(c0: f64, c1: f64, step: f64) -> Vec<f64> {
    let n = (((c1 - c0) / step).round() as i64).max(1);
    (0..=n).map(|i| c0 + ((c1 - c0) * i as f64) / n as f64).collect()
}

/// A main-cable run: a parabola under the chord (c0, y0)-(c1, y1), `sag` below it at mid-run.
#[derive(Clone, Copy, Debug)]
pub struct Run {
    pub c0: f64,
    pub c1: f64,
    pub y0: f64,
    pub y1: f64,
    pub sag: f64,
}

pub fn cable_height_at(runs: &[Run], c: f64) -> Option<f64> {
    for r in runs {
        if c < r.c0 - 1e-6 || c > r.c1 + 1e-6 {
            continue;
        }
        let t = ((c - r.c0) / (r.c1 - r.c0)).max(0.0).min(1.0);
        return Some(r.y0 + (r.y1 - r.y0) * t - 4.0 * r.sag * t * (1.0 - t));
    }
    None
}

pub fn side_span_sag(main_sag: f64, main_span: f64, side_span: f64) -> f64 {
    main_sag * (side_span / main_span).powi(2)
}

pub struct CableSpec {
    pub half_plane: f64,
    pub diameter: f64,
    pub colour: Rgb,
    /// The far item's beams at twice the diameter (the JS); false when the
    /// wire pass draws the cables at every distance (`BridgeFeatures::wires`).
    pub far_beams: bool,
}

/// `mainCables` (step 12, farStep 60, farScale 2).
pub fn main_cables(near: &mut MeshArrays, far: &mut MeshArrays, frame: &Frame, runs: &[Run], o: &CableSpec) {
    let (step, far_step, far_scale) = (12.0, 60.0, 2.0);
    for s in [-o.half_plane, o.half_plane] {
        for r in runs {
            let one = [*r];
            let ns = stations(r.c0, r.c1, step);
            for i in 0..ns.len() - 1 {
                let a = frame.P(ns[i], s, cable_height_at(&one, ns[i]).unwrap_or(f64::NAN));
                let b = frame.P(ns[i + 1], s, cable_height_at(&one, ns[i + 1]).unwrap_or(f64::NAN));
                if near.wire {
                    near.wire_segment(a, b, o.diameter, o.colour);
                } else {
                    near.beam(a, b, o.diameter, o.diameter, o.colour);
                }
            }
            if !o.far_beams {
                continue;
            }
            let fs = stations(r.c0, r.c1, far_step);
            for i in 0..fs.len() - 1 {
                far.beam(
                    frame.P(fs[i], s, cable_height_at(&one, fs[i]).unwrap_or(f64::NAN)),
                    frame.P(fs[i + 1], s, cable_height_at(&one, fs[i + 1]).unwrap_or(f64::NAN)),
                    o.diameter * far_scale,
                    o.diameter * far_scale,
                    o.colour,
                );
            }
        }
    }
}

pub struct SuspenderSpec {
    pub half_plane: f64,
    pub spacing: f64,
    pub diameter: f64,
    pub pair_gap: f64,
    pub colour: Rgb,
}

/// `suspenderPairs` (endClearM 6): returns the stations per plane.
pub fn suspender_pairs(near: &mut MeshArrays, frame: &Frame, runs: &[Run], attach_y: &dyn Fn(f64) -> f64, o: &SuspenderSpec) -> usize {
    let end_clear = 6.0;
    let mut count = 0;
    for r in runs {
        let n = ((r.c1 - r.c0) / o.spacing).floor() as i64;
        let offset = ((r.c1 - r.c0) - n as f64 * o.spacing) / 2.0;
        for k in 0..=n {
            let c = r.c0 + offset + k as f64 * o.spacing;
            if c - r.c0 < end_clear || r.c1 - c < end_clear {
                continue;
            }
            let Some(top) = cable_height_at(&[*r], c) else { continue };
            let bottom = attach_y(c);
            if top - bottom < 0.5 {
                continue;
            }
            for s in [-o.half_plane, o.half_plane] {
                for d in [-o.pair_gap / 2.0, o.pair_gap / 2.0] {
                    if near.wire {
                        near.wire_segment(frame.P(c + d, s, bottom), frame.P(c + d, s, top), o.diameter, o.colour);
                    } else {
                        near.beam(frame.P(c + d, s, bottom), frame.P(c + d, s, top), o.diameter, o.diameter, o.colour);
                    }
                }
            }
            count += 1;
        }
    }
    count
}

pub struct TrussSpec<'a> {
    pub deck_top: &'a dyn Fn(f64) -> f64,
    pub c_start: f64,
    pub c_end: f64,
    pub half_plane: f64,
    pub depth: f64,
    pub panel: f64,
    pub colour: Rgb,
    pub dark: Rgb,
    pub gap: &'a dyn Fn(f64, f64) -> bool,
    pub far_half_width: f64,
    pub far_above: f64,
    /// The members' section sizes times this (the structure scale; 1: the JS's).
    pub member: f64,
}

/// `stiffeningTruss` (farStep 40).
pub fn stiffening_truss(near: &mut MeshArrays, far: &mut MeshArrays, frame: &Frame, o: &TrussSpec) {
    let h = o.half_plane;
    let depth = o.depth;
    let far_step = 40.0;
    let k = o.member;
    let top = 0.5 * k;
    let cs = stations(o.c_start, o.c_end, o.panel);
    for i in 0..cs.len() - 1 {
        let (a, b) = (cs[i], cs[i + 1]);
        let (ta, tb) = ((o.deck_top)(a), (o.deck_top)(b));
        let cut = (o.gap)(a, b);
        if !cut {
            for s in [-h, h] {
                near.beam(frame.P(a, s, ta - top), frame.P(b, s, tb - top), 0.9 * k, 1.0 * k, o.colour);
                near.beam(frame.P(a, s, ta - depth), frame.P(b, s, tb - depth), 0.9 * k, 1.0 * k, o.colour);
                near.beam(frame.P(b, s, tb - depth), frame.P(b, s, tb - top), 0.5 * k, 0.6 * k, o.colour);
                let up = i % 2 == 0;
                near.beam(
                    frame.P(a, s, if up { ta - depth } else { ta - top }),
                    frame.P(b, s, if up { tb - top } else { tb - depth }),
                    0.45 * k,
                    0.5 * k,
                    o.colour,
                );
            }
        }
        near.beam(frame.P(b, -h, tb - 1.1 * k), frame.P(b, h, tb - 1.1 * k), 0.7 * k, 1.0 * k, o.dark);
        if !cut {
            near.beam(frame.P(a, -h, ta - depth), frame.P(b, h, tb - depth), 0.35 * k, 0.35 * k, o.dark);
            near.beam(frame.P(a, h, ta - depth), frame.P(b, -h, tb - depth), 0.35 * k, 0.35 * k, o.dark);
        }
    }
    let fs = stations(o.c_start, o.c_end, far_step);
    for i in 0..fs.len() - 1 {
        let (a, b) = (fs[i], fs[i + 1]);
        let (ta, tb) = ((o.deck_top)(a), (o.deck_top)(b));
        let [mx, mz] = frame.point((a + b) / 2.0, 0.0);
        far.box_(mx, mz, (ta + tb) / 2.0 - depth, (ta + tb) / 2.0 + o.far_above, (b - a) / 2.0, o.far_half_width, frame.yaw, o.colour, crate::geometry::ALL);
    }
    for c in [o.c_start, o.c_end] {
        let t = (o.deck_top)(c);
        let [x, z] = frame.point(c, 0.0);
        near.box_(x, z, t - depth, t, 0.6, o.far_half_width, frame.yaw, o.dark, crate::geometry::ALL);
    }
}

/// Colours of a road deck.
#[derive(Clone, Copy)]
pub struct DeckColours {
    pub asphalt: Rgb,
    pub sidewalk: Rgb,
    pub soffit: Rgb,
    pub barrier: Rgb,
    pub white: Rgb,
    pub yellow: Rgb,
    pub rail: Rgb,
}

/// The California standards a deck reads (`caMetric` in suspension-bridge.js).
#[derive(Clone, Copy, Debug)]
pub struct DeckStandards {
    /// 'temporary-railing-type-k-height'
    pub k_height: f64,
    /// 'temporary-railing-type-k-width'
    pub k_width: f64,
    /// 'line-width-state-highway'
    pub line_w: f64,
    /// 'broken-line-pattern-ge45mph' line and gap
    pub broken_line: f64,
    pub broken_gap: f64,
}

impl DeckStandards {
    pub fn load(ca: &eo_data::CaStandards) -> anyhow::Result<Self> {
        Ok(Self {
            k_height: ca.metric("temporary-railing-type-k-height")?,
            k_width: ca.metric("temporary-railing-type-k-width")?,
            line_w: ca.metric("line-width-state-highway")?,
            broken_line: ca.metric_key("broken-line-pattern-ge45mph", "line")?,
            broken_gap: ca.metric_key("broken-line-pattern-ge45mph", "gap")?,
        })
    }
}

pub struct DeckSpec<'a> {
    pub y: &'a dyn Fn(f64) -> f64,
    pub c_start: f64,
    pub c_end: f64,
    pub half_width: f64,
    pub roadway_m: f64,
    pub lanes: u32,
    pub median: bool,
    pub sidewalk_m: f64,
    pub curb_m: f64,
    pub railings: bool,
    pub railing_m: f64,
    pub inner_barrier_m: f64,
    pub panel: f64,
    pub gap: &'a dyn Fn(f64, f64) -> bool,
    pub colours: DeckColours,
    pub markings: bool,
}

/// `roadDeck`: the roadway, sidewalks behind curbs, the plate's underside, a
/// median barrier, edge lines, broken lane lines and railings.
pub fn road_deck(deck: &mut MeshArrays, rail: &mut MeshArrays, frame: &Frame, ca: &DeckStandards, o: &DeckSpec) {
    let y = o.y;
    let curb = o.roadway_m / 2.0;
    let line_w = ca.line_w;
    let median_half = if o.median { ca.k_width / 2.0 + 0.3 } else { 0.0 };
    let cs = stations(o.c_start, o.c_end, o.panel);
    let colours = o.colours;
    let stripe = |deck: &mut MeshArrays, c0: f64, c1: f64, s: f64, colour: Rgb| {
        let (y0, y1) = (y(c0) + 0.02, y(c1) + 0.02);
        deck.quad(frame.P(c0, s - line_w / 2.0, y0), frame.P(c0, s + line_w / 2.0, y0), frame.P(c1, s + line_w / 2.0, y1), frame.P(c1, s - line_w / 2.0, y1), colour);
    };
    for i in 0..cs.len() - 1 {
        let (a, b) = (cs[i], cs[i + 1]);
        let (ta, tb) = (y(a), y(b));
        deck.quad(frame.P(a, -curb, ta), frame.P(a, curb, ta), frame.P(b, curb, tb), frame.P(b, -curb, tb), colours.asphalt);
        let cut = (o.gap)(a, b);
        if o.sidewalk_m > 0.0 && !cut {
            for sgn in [-1.0, 1.0] {
                let inner = sgn * curb;
                let outer = sgn * o.half_width;
                let (p, q) = if sgn < 0.0 { (outer, inner) } else { (inner, outer) };
                deck.quad(frame.P(a, p, ta + o.curb_m), frame.P(a, q, ta + o.curb_m), frame.P(b, q, tb + o.curb_m), frame.P(b, p, tb + o.curb_m), colours.sidewalk);
                let (c0, c1, t0, t1) = if sgn > 0.0 { (b, a, tb, ta) } else { (a, b, ta, tb) };
                deck.quad(frame.P(c0, inner, t0), frame.P(c1, inner, t1), frame.P(c1, inner, t1 + o.curb_m), frame.P(c0, inner, t0 + o.curb_m), colours.sidewalk);
            }
        }
        deck.quad(frame.P(b, -o.half_width, tb - 0.3), frame.P(b, o.half_width, tb - 0.3), frame.P(a, o.half_width, ta - 0.3), frame.P(a, -o.half_width, ta - 0.3), colours.soffit);
        if o.median {
            let [mx, mz] = frame.point((a + b) / 2.0, 0.0);
            deck.box_(mx, mz, (ta + tb) / 2.0, (ta + tb) / 2.0 + ca.k_height, (b - a) / 2.0, ca.k_width / 2.0, frame.yaw, colours.barrier, NO_BOTTOM);
        }
        if o.markings {
            if o.median {
                for sgn in [-1.0, 1.0] {
                    stripe(deck, a, b, sgn * (median_half + 0.15), colours.yellow);
                    stripe(deck, a, b, sgn * (curb - 0.3), colours.white);
                }
            } else {
                stripe(deck, a, b, -(curb - 0.3), colours.yellow);
                stripe(deck, a, b, curb - 0.3, colours.white);
            }
        }
    }
    if o.markings {
        let mut lines = Vec::new();
        if o.median {
            let per = o.lanes as f64 / 2.0;
            let lane_w = (curb - median_half) / per;
            for sgn in [-1.0, 1.0] {
                let mut k = 1.0;
                while k < per {
                    lines.push(sgn * (median_half + k * lane_w));
                    k += 1.0;
                }
            }
        } else {
            let lane_w = o.roadway_m / o.lanes as f64;
            for k in 1..o.lanes {
                lines.push(-curb + k as f64 * lane_w);
            }
        }
        let cycle = ca.broken_line + ca.broken_gap;
        let mut c = o.c_start + ca.broken_gap / 2.0;
        while c + ca.broken_line < o.c_end {
            for &s in &lines {
                stripe(deck, c, c + ca.broken_line, s, colours.white);
            }
            c += cycle;
        }
    }
    if !o.railings {
        return;
    }
    let rs = stations(o.c_start, o.c_end, o.panel / 2.0);
    for i in 0..rs.len() - 1 {
        let (a, b) = (rs[i], rs[i + 1]);
        let (ta, tb) = (y(a) + o.curb_m, y(b) + o.curb_m);
        let cut = (o.gap)(a, b);
        for sgn in [-1.0, 1.0] {
            let si = sgn * (curb + 0.15);
            rail.beam(frame.P(a, si, ta - o.curb_m + o.inner_barrier_m), frame.P(b, si, tb - o.curb_m + o.inner_barrier_m), 0.25, 0.25, colours.rail);
            if cut {
                continue;
            }
            rail.beam(frame.P(b, si, tb - o.curb_m), frame.P(b, si, tb - o.curb_m + o.inner_barrier_m), 0.2, 0.2, colours.rail);
            if o.sidewalk_m <= 0.0 {
                continue;
            }
            let s = sgn * (o.half_width - 0.15);
            rail.beam(frame.P(a, s, ta + o.railing_m), frame.P(b, s, tb + o.railing_m), 0.12, 0.12, colours.rail);
            rail.beam(frame.P(a, s, ta + o.railing_m * 0.5), frame.P(b, s, tb + o.railing_m * 0.5), 0.08, 0.08, colours.rail);
            rail.beam(frame.P(b, s, tb), frame.P(b, s, tb + o.railing_m), 0.14, 0.14, colours.rail);
        }
    }
}

#[allow(dead_code)]
pub(crate) fn faces_all() -> Faces {
    crate::geometry::ALL
}
