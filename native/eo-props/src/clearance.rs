//! Keeping the props out of the grade-separated roads (`--structures`,
//! stage A6): nothing placed pierces a deck, its girder, a bent or a
//! barrier, and the columns standing in parking lanes and furniture zones
//! keep the cars and the furniture off their footprint plus a margin.
//!
//! Every placed item is a few vertical cylinders (`extents`: a pole and its
//! head, a mast arm sampled along its length, a tree's trunk and crown, a
//! plate, a box of furniture at its kind's size); an item is clear when none
//! of them, grown by `PROP_CLEAR_M` across and over its top, meets the
//! structures' clearance envelope (`eo_structures::StructureWorld::blocked`,
//! which holds every drawn deck, girder, barrier, fill wall, cap and column,
//! and nothing else). What is not clear:
//!
//! * a street tree that does not fit is replaced by the tallest smaller
//!   street species (`eztree::street_tree_species`, at its own size, so its
//!   crown keeps the species' own lift over the footway) whose crown fits
//!   under the deck and off the columns, else dropped; a palm (no smaller
//!   kind) is dropped;
//! * a signal pole's assembly (its arms, the heads on it and the pedestrian
//!   heads mounted on it) is kept whole, or a mast cut down to a pedestal
//!   (`pedestalPoleHeightM`) keeping only its pole-mounted head at the
//!   pedestal's height (`pedestalHeadCentreM`), or dropped
//!   (`signals::plan_node_clear`);
//! * a sign post is moved along its kerb by the kerb snap
//!   (`signs::snap_with`), or dropped where no seat is clear;
//! * the overhead: a pole is dropped with its arm, the plates strapped to it
//!   and the wires ending on it; a wire meeting a structure anywhere along
//!   it is dropped, and the hangers and contact wire ending on a dropped
//!   span; cabinets, corner signs (with their host pole) and lanterns (with
//!   their string) likewise (`clear_overhead`);
//! * a parked car is refused where its footprint plus the margin reaches a
//!   column, or a structure stands lower than it over it
//!   (`parking_rules::KerbRules::structure_reason`);
//! * any other item (lamps with their luminaire, furniture, tactile pads) is
//!   dropped.
//!
//! And the built freeway decks get their luminaires (`place_deck_lamps`,
//! `PropsWorld::deck_lamps`): the landmark deck rule's pitch and head height
//! (`eo_landmarks::lights`), a district cobra head on a pole standing on the
//! barrier top, its light the district lights' own record.
//!
//! Without structures (or with an empty world) none of this runs: every
//! placed list is the one before (tested on a sample of real tiles).
//! Nothing is cached on disk by placement, so no cache key changes.

use std::collections::BTreeMap;

use eo_structures::StructureWorld;

use crate::furniture::LampLight;
use crate::overhead::{OverheadTile, PoleKind, WireKind};
use crate::{Model, Prop, PropsWorld, TilePlacement};

/// How far a placed item keeps from the structures' envelope, across and
/// over its top: the drawn concrete is already padded by
/// `eo_structures::ENVELOPE_PAD_M`; this is the item's own breathing room
/// (a hand's breadth, so a crown or a lamp head never brushes a soffit or
/// a column). INFERRED.
pub const PROP_CLEAR_M: f64 = 0.3;

/// A street tree's trunk radius, its crown's base (share of its height) and
/// a palm's frond radius (share of its height), for the clearance test.
/// INFERRED (bounds of the ez-tree species and the palm GLB).
const TRUNK_R_M: f64 = 0.35;
const CROWN_BASE: f64 = 0.3;
const PALM_CROWN: f64 = 0.28;

/// A luminaire on a built freeway deck's barrier (`PropsWorld::deck_lamps`):
/// its pole and head (a district cobra head, `Model::LampCobra`), its light,
/// and the deck and station it stands at.
#[derive(Clone, Copy, Debug)]
pub struct DeckLamp {
    pub prop: Prop,
    pub light: LampLight,
    pub chain: u32,
    pub s: f64,
}

/// One vertical cylinder an item fills: plan centre, radius, from `y0` to `y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub x: f64,
    pub z: f64,
    pub r: f64,
    pub y0: f64,
    pub y1: f64,
}

impl Extent {
    fn new(x: f64, z: f64, r: f64, y0: f64, y1: f64) -> Self {
        Extent { x, z, r, y0, y1 }
    }
}

/// Whether a cylinder meets the structures (`except`: one deck left out,
/// what stands on it), with the item's margin.
pub fn blocked(s: &StructureWorld, e: &Extent, except: Option<u32>) -> bool {
    s.blocked_except(e.x, e.z, e.r + PROP_CLEAR_M, e.y0, e.y1 + PROP_CLEAR_M, except)
}

/// Whether every cylinder is clear.
pub fn all_clear(s: &StructureWorld, ex: &[Extent], except: Option<u32>) -> bool {
    ex.iter().all(|e| !blocked(s, e, except))
}

/// Cylinders every few `step` metres along a horizontal run from `a`
/// (plan) in direction `dir` (unit) for `len` metres at height [y0, y1].
fn along(out: &mut Vec<Extent>, a: [f64; 2], dir: [f64; 2], len: f64, r: f64, y0: f64, y1: f64, step: f64) {
    let n = (len / step).ceil().max(1.0) as usize;
    for k in 0..=n {
        let t = len * k as f64 / n as f64;
        out.push(Extent::new(a[0] + dir[0] * t, a[1] + dir[1] * t, r, y0, y1));
    }
}

/// The sizes the extents read, from the props' own tables (built once per world).
pub struct Dims {
    /// Per model: the street-furniture kind's radius and height (`STREET_FURNITURE_KINDS`).
    kinds: Vec<Option<(f64, f64)>>,
    /// Per street tree species (the `Model::Tree` variant): height and largest half crown (m).
    trees: Vec<(f64, f64)>,
    palm_h: f64,
    /// `lampHead.district` and `.column`: head height over the foot and reach toward the street.
    district: (f64, f64),
    column: (f64, f64),
    pole_r: f64,
    head_half: f64,
    post_r: f64,
    /// Per `signals::ArmClass` row, the arm's rise (a `MastArm` of variant 1 + row).
    arm_rise: Vec<f64>,
}

impl Dims {
    pub fn new(w: &PropsWorld) -> Self {
        let f = w.furniture();
        let mut kinds = vec![None; crate::MODEL_COUNT];
        for (k, spec) in f.kinds.iter().enumerate() {
            if let Some(m) = f.model_of(k as u16) {
                kinds[m as usize] = Some((spec.radius, spec.height));
            }
        }
        let trees = crate::eztree::street_tree_species()
            .map(|list| list.iter().map(|id| crate::eztree::ez_species(id).map(|s| (s.height_m, s.max_half_crown_m)).unwrap_or((11.8, 3.0))).collect())
            .unwrap_or_default();
        let palm_h = f.js.consts.palm_height_m.filter(|v| *v > 0.0).or_else(|| kinds[Model::Palm as usize].map(|k| k.1)).unwrap_or(10.0);
        let head = |k: &str| f.js.consts.lamp_head.get(k).map(|h| (h.height_m, h.reach_m)).unwrap_or((7.62, 0.0));
        Dims {
            kinds,
            trees,
            palm_h,
            district: head("district"),
            column: head("column"),
            pole_r: w.js.num("signalHardware", "poleRadiusBaseM"),
            head_half: w.js.num("signalHardware", "headHalfHeightM"),
            post_r: w.js.consts["signPostRadiusM"].as_f64().unwrap_or(0.03),
            arm_rise: w.std.signal_arms.iter().map(|c| c.rise).collect(),
        }
    }

    /// A tree's height and half crown at its scale.
    fn tree(&self, p: &Prop) -> (f64, f64) {
        let (h, c) = if self.trees.is_empty() { (11.8, 3.0) } else { self.trees[p.variant as usize % self.trees.len()] };
        (h * p.scale[1] as f64, c * p.scale[0] as f64)
    }

    /// A lamp's head: (height over the foot, reach toward the street), at its scale.
    pub fn lamp_head(&self, p: &Prop) -> (f64, f64) {
        let (h, r) = if p.model == Model::LampHistoric { (self.column.0, 0.0) } else { self.district };
        (h * p.scale[1] as f64, r * p.scale[0] as f64)
    }

    /// The district lamp's head height and reach at scale 1.
    pub fn district_head(&self) -> (f64, f64) {
        self.district
    }
}

/// The cylinders one placed item fills.
pub fn extents(d: &Dims, p: &Prop, out: &mut Vec<Extent>) {
    let [x, y, z] = p.pos;
    let sc = |i: usize| p.scale[i] as f64;
    match p.model {
        Model::SignalPole => out.push(Extent::new(x, z, d.pole_r, y, y + sc(1))),
        Model::MastArm => {
            // local +X (three.js rotation.y = yaw): (cos yaw, -sin yaw); a rising arm up to its rise.
            let rise = (p.variant as usize).checked_sub(1).and_then(|c| d.arm_rise.get(c)).copied().unwrap_or(0.0);
            along(out, [x, z], [p.yaw.cos(), -p.yaw.sin()], sc(0), 0.15, y - 0.25, y + rise + 0.25, 1.0);
        }
        Model::SignalHead => out.push(Extent::new(x, z, 0.35, y - d.head_half - 0.1, y + d.head_half + 0.1)),
        Model::PedHead => out.push(Extent::new(x, z, 0.3, y - 0.35, y + 0.35)),
        Model::SignPost => out.push(Extent::new(x, z, d.post_r.max(0.03), y, y + sc(1))),
        Model::SignPlate => out.push(Extent::new(x, z, sc(0) / 2.0, y - sc(1) / 2.0, y + sc(1) / 2.0)),
        Model::Tree => {
            let (h, c) = d.tree(p);
            out.push(Extent::new(x, z, TRUNK_R_M, y, y + (CROWN_BASE + 0.1) * h));
            out.push(Extent::new(x, z, c, y + CROWN_BASE * h, y + h));
        }
        Model::Palm => {
            let h = d.palm_h * sc(1);
            out.push(Extent::new(x, z, TRUNK_R_M, y, y + h));
            out.push(Extent::new(x, z, PALM_CROWN * h, y + 0.8 * h, y + h));
        }
        Model::TactilePad | Model::TreeBasin => out.push(Extent::new(x, z, sc(0).max(sc(2)) / 2.0, y, y + 0.05)),
        Model::LampHistoric | Model::LampBanner | Model::LampCobra => {
            let (head, reach) = d.lamp_head(p);
            let kind_h = d.kinds[p.model as usize].map(|k| k.1).unwrap_or(7.62) * sc(1);
            out.push(Extent::new(x, z, 0.25, y, y + kind_h.max(head + 0.4)));
            if reach > 0.0 {
                // the arm and head toward the street: -(sin yaw, cos yaw), as the luminaire's.
                along(out, [x, z], [-p.yaw.sin(), -p.yaw.cos()], reach, 0.35, y + head - 0.5, y + head + 0.4, 0.75);
            }
        }
        m => {
            let (r, h) = d.kinds.get(m as usize).copied().flatten().unwrap_or((0.5, 3.0));
            out.push(Extent::new(x, z, r * sc(0), y, y + h * sc(1)));
        }
    }
}

/// What the clearance did (the census and the tests).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClearStats {
    /// Items dropped, by model name (overhead parts by their own names).
    pub dropped: BTreeMap<String, usize>,
    /// Street trees replaced by a smaller species.
    pub trees_capped: usize,
    /// Signal masts cut down to pedestals.
    pub pedestals: usize,
    /// District luminaires dropped with their lamp.
    pub lights_dropped: usize,
    /// Deck luminaires placed.
    pub deck_lamps: usize,
    /// Tunnel bore lights placed (stage A7, `--tunnels`).
    pub tunnel_lights: usize,
}

impl ClearStats {
    pub fn add(&mut self, o: &ClearStats) {
        for (k, v) in &o.dropped {
            *self.dropped.entry(k.clone()).or_default() += v;
        }
        self.trees_capped += o.trees_capped;
        self.pedestals += o.pedestals;
        self.lights_dropped += o.lights_dropped;
        self.deck_lamps += o.deck_lamps;
        self.tunnel_lights += o.tunnel_lights;
    }

    fn drop_one(&mut self, what: &str) {
        *self.dropped.entry(what.to_string()).or_default() += 1;
    }

    /// Everything dropped.
    pub fn dropped_total(&self) -> usize {
        self.dropped.values().sum()
    }
}

/// A street tree or palm that fits: as it is, or a street tree replaced by
/// the tallest smaller street species that fits, at that species' own size
/// (scale 1: its crown keeps the species' own lift over the footway, which
/// a big tree scaled down would not); None when nothing fits (a palm has no
/// smaller kind).
pub fn fit_tree(s: &StructureWorld, d: &Dims, p: &Prop) -> Option<Prop> {
    let mut ex = vec![];
    extents(d, p, &mut ex);
    if all_clear(s, &ex, None) {
        return Some(*p);
    }
    if p.model != Model::Tree || d.trees.is_empty() {
        return None;
    }
    let (h, _) = d.tree(p);
    let mut smaller: Vec<usize> = (0..d.trees.len()).filter(|&v| d.trees[v].0 < h).collect();
    smaller.sort_by(|&a, &b| d.trees[b].0.total_cmp(&d.trees[a].0).then(a.cmp(&b)));
    for v in smaller {
        let q = Prop { variant: v as u16, scale: [1.0; 3], ..*p };
        ex.clear();
        extents(d, &q, &mut ex);
        if all_clear(s, &ex, None) {
            return Some(q);
        }
    }
    None
}

/// One item kept, made smaller (trees), or dropped (None).
pub fn clear_prop(s: &StructureWorld, d: &Dims, p: &Prop, stats: &mut ClearStats) -> Option<Prop> {
    match p.model {
        Model::Tree | Model::Palm => {
            let kept = fit_tree(s, d, p);
            match &kept {
                Some(q) if q.scale != p.scale || q.variant != p.variant => stats.trees_capped += 1,
                None => stats.drop_one(&format!("{:?}", p.model)),
                _ => {}
            }
            kept
        }
        _ => {
            let mut ex = vec![];
            extents(d, p, &mut ex);
            if all_clear(s, &ex, None) {
                Some(*p)
            } else {
                stats.drop_one(&format!("{:?}", p.model));
                None
            }
        }
    }
}

/// The ribbon and junction props of a tile, cleared: the signal assemblies
/// and the sign posts arrive cleared already (`signals::plan_node_clear`,
/// `signs::plan_tile_clear`); every other item is kept, made smaller or
/// dropped (`clear_prop`), a dropped lamp's luminaire with it; then the
/// tile's deck luminaires.
pub fn clear_tile(w: &PropsWorld, s: &StructureWorld, key: (i32, i32), t: &mut TilePlacement, stats: &mut ClearStats) {
    let d = w.clear_dims();
    let mut dropped_lamps: Vec<[u64; 3]> = vec![];
    let mut kept = Vec::with_capacity(t.props.len());
    for p in &t.props {
        if matches!(p.model, Model::SignalPole | Model::MastArm | Model::SignalHead | Model::PedHead | Model::SignPost | Model::SignPlate) {
            kept.push(*p);
            continue;
        }
        match clear_prop(s, d, p, stats) {
            Some(q) => kept.push(q),
            None if matches!(p.model, Model::LampHistoric | Model::LampBanner | Model::LampCobra) => dropped_lamps.push(p.pos.map(f64::to_bits)),
            None => {}
        }
    }
    t.props = kept;
    if !dropped_lamps.is_empty() {
        let before = t.lights.len();
        t.lights.retain(|l| !dropped_lamps.contains(&l.base.map(f64::to_bits)));
        stats.lights_dropped += before - t.lights.len();
    }
    if let Some(lamps) = w.deck_lamps().get(&key) {
        for l in lamps {
            t.props.push(l.prop);
            t.lights.push(l.light);
            stats.deck_lamps += 1;
        }
    }
    // stage A7 (`--tunnels`): the bores' lights, through the district
    // lights' path (`PropsWorld::tunnel_lights`).
    for l in w.tunnel_lights(key) {
        t.lights.push(l);
        stats.tunnel_lights += 1;
    }
}

/// The overhead of a tile, cleared (see the module comment).
pub fn clear_overhead(w: &PropsWorld, s: &StructureWorld, key: (i32, i32), t: &mut OverheadTile, stats: &mut ClearStats) {
    let js = &crate::overhead::shared(w).js;
    let row = &js.consts["utilityPole"];
    let crossarm = row["crossarmM"].as_f64().unwrap_or(2.438) / 2.0;
    let hardware = row["hardwareFromM"].as_f64().unwrap_or(8.668);
    let arm_len = js.consts["cobraArm"]["armM"].as_f64().unwrap_or(1.829);
    // anchors a dropped or shortened pole leaves: (x, z, reach, the height above which nothing holds).
    let mut anchors: Vec<(f64, f64, f64, f64)> = vec![];
    // the signal poles near the tile the clearance dropped or cut down.
    for ch in w.signal_changes_near(key) {
        anchors.push((ch.x, ch.z, 0.6, ch.top_after));
    }
    let mut ex = vec![];
    t.poles.retain(|p| {
        ex.clear();
        let [x, y, z] = p.pos;
        ex.push(Extent::new(x, z, p.radius_base.max(0.1), y, y + p.height));
        if p.kind == PoleKind::Distribution {
            ex.push(Extent::new(x, z, crossarm, y + hardware, y + p.height + 0.2));
        }
        if let Some(a) = p.arm {
            along(&mut ex, [a.pos[0], a.pos[2]], [a.yaw.cos(), -a.yaw.sin()], arm_len, 0.25, a.pos[1] - 0.4, a.pos[1] + 0.5, 0.6);
        }
        let ok = all_clear(s, &ex, None);
        if !ok {
            let reach = if p.kind == PoleKind::Distribution { crossarm + 0.3 } else { 0.6 };
            anchors.push((x, z, reach, f64::NEG_INFINITY));
            stats.drop_one(match p.kind {
                PoleKind::Distribution => "overhead distribution pole",
                _ => "overhead span/support pole",
            });
        }
        ok
    });
    let held_by_dropped = |q: [f32; 3], anchors: &[(f64, f64, f64, f64)]| anchors.iter().any(|a| (q[0] as f64 - a.0).hypot(q[2] as f64 - a.1) <= a.2 && q[1] as f64 > a.3 - 0.5);
    // wires: crossing a structure anywhere, or ending on a dropped pole.
    let mut gone: Vec<(WireKind, Vec<[f32; 3]>)> = vec![];
    t.wires.retain(|wr| {
        let ends = [wr.points[0], wr.points[wr.points.len() - 1]];
        let mut ok = !ends.iter().any(|&q| held_by_dropped(q, &anchors));
        if ok {
            'samples: for pair in wr.points.windows(2) {
                let (a, b) = (pair[0].map(|v| v as f64), pair[1].map(|v| v as f64));
                let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
                let n = len.ceil().max(1.0) as usize;
                for k in 0..=n {
                    let f = k as f64 / n as f64;
                    let q = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
                    if blocked(s, &Extent::new(q[0], q[2], wr.radius_m as f64, q[1] - PROP_CLEAR_M, q[1]), None) {
                        ok = false;
                        break 'samples;
                    }
                }
            }
        }
        if !ok {
            gone.push((wr.kind, wr.points.clone()));
            stats.drop_one(&format!("overhead wire ({})", wr.kind.js_name()));
        }
        ok
    });
    // what hangs from a dropped span or bracket: its hangers and the contact wire ending there.
    let near_gone = |gone: &[(WireKind, Vec<[f32; 3]>)], q: [f32; 3], kinds: &[WireKind]| {
        gone.iter().filter(|g| kinds.contains(&g.0)).any(|g| {
            g.1.windows(2).any(|pair| {
                let (a, b) = (pair[0].map(|v| v as f64), pair[1].map(|v| v as f64));
                let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let l2 = (ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]).max(1e-9);
                let p = q.map(|v| v as f64);
                let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1] + (p[2] - a[2]) * ab[2]) / l2).clamp(0.0, 1.0);
                ((p[0] - a[0] - ab[0] * t).powi(2) + (p[1] - a[1] - ab[1] * t).powi(2) + (p[2] - a[2] - ab[2] * t).powi(2)).sqrt() < 0.75
            })
        })
    };
    if gone.iter().any(|g| matches!(g.0, WireKind::Span | WireKind::Bracket)) {
        let holders = [WireKind::Span, WireKind::Bracket];
        let mut hung: Vec<(WireKind, Vec<[f32; 3]>)> = vec![];
        t.wires.retain(|wr| {
            if !matches!(wr.kind, WireKind::Hanger) {
                return true;
            }
            let hangs = wr.points.iter().any(|&q| near_gone(&gone, q, &holders));
            if hangs {
                hung.push((wr.kind, wr.points.clone()));
                stats.drop_one("overhead wire (hanger)");
            }
            !hangs
        });
        gone.extend(hung);
        let holders = [WireKind::Span, WireKind::Bracket, WireKind::Hanger];
        t.wires.retain(|wr| {
            if !matches!(wr.kind, WireKind::Contact) {
                return true;
            }
            let ends = [wr.points[0], wr.points[wr.points.len() - 1]];
            let hangs = ends.iter().any(|&q| near_gone(&gone, q, &holders));
            if hangs {
                stats.drop_one("overhead wire (contact)");
            }
            !hangs
        });
    }
    t.cabinets.retain(|c| {
        let ok = !blocked(s, &Extent::new(c.pos[0], c.pos[2], c.size[0].hypot(c.size[1]) / 2.0, c.pos[1], c.pos[1] + c.size[2]), None);
        if !ok {
            stats.drop_one("overhead signal cabinet");
        }
        ok
    });
    t.signs.retain(|a| {
        ex.clear();
        let [x, y, z] = a.pos;
        if !a.on_pole {
            ex.push(Extent::new(x, z, 0.05, y, y + a.post_height * a.scale));
        }
        for pl in &a.plates {
            ex.push(Extent::new(pl.centre[0], pl.centre[2], pl.w / 2.0, pl.centre[1] - pl.h / 2.0, pl.centre[1] + pl.h / 2.0));
        }
        let host_gone = a.on_pole && anchors.iter().any(|q| (x - q.0).hypot(z - q.1) <= q.2 + 0.3);
        let ok = !host_gone && all_clear(s, &ex, None);
        if !ok {
            stats.drop_one("overhead sign assembly");
        }
        ok
    });
    t.corner_signs.retain(|c| {
        let [x, y, z] = c.pos;
        let host_gone = anchors.iter().any(|q| (x - q.0).hypot(z - q.1) <= 1.2 && y + 0.45 > q.3);
        let ok = !host_gone && !blocked(s, &Extent::new(x, z, 0.45, y - 0.45, y + 0.45), None);
        if !ok {
            stats.drop_one("overhead corner sign");
        }
        ok
    });
    let strings: Vec<&Vec<[f32; 3]>> = gone.iter().filter(|g| g.0 == WireKind::LanternString).map(|g| &g.1).collect();
    t.lanterns.retain(|l| {
        let [x, y, z] = l.pos;
        let on_gone = strings.iter().any(|pts| pts.iter().any(|q| (q[0] as f64 - x).hypot(q[2] as f64 - z) < 1.5));
        let ok = !on_gone && !blocked(s, &Extent::new(x, z, 0.35, y - 0.1, y + 1.0), None);
        if !ok {
            stats.drop_one("overhead lantern");
        }
        ok
    });
}

/// The deck luminaires of every built freeway deck (see `PropsWorld::deck_lamps`).
pub(crate) fn place_deck_lamps(w: &PropsWorld, s: &StructureWorld) -> std::collections::HashMap<(i32, i32), Vec<DeckLamp>> {
    use eo_landmarks::lights::{DECK_HEAD_M, DECK_PITCH_M};
    let d = w.clear_dims();
    let f = w.furniture();
    let (district_h, district_reach) = d.district_head();
    let mut out: std::collections::HashMap<(i32, i32), Vec<DeckLamp>> = std::collections::HashMap::new();
    let mut ex = vec![];
    for c in s.chains() {
        if !c.is_built() || c.kind != eo_structures::ChainKind::Deck || c.landmark.is_some() || c.class == eo_structures::RoadClass::Other {
            continue;
        }
        // the landmark deck rule: both edges, every DECK_PITCH_M from half a pitch in.
        let skyway = eo_structures::skyway_lights::coverage(c);
        let mut st = DECK_PITCH_M / 2.0;
        while st < c.length() {
            if skyway.is_some_and(|[start, end]| (start..=end).contains(&c.frame(st).x)) {
                st += DECK_PITCH_M;
                continue;
            }
            for b in s.barrier_at(c.id, st) {
                // the pole stands on the barrier; the head DECK_HEAD_M over the deck beside it.
                let foot = [b.x, b.top_y, b.z];
                let scale = (DECK_HEAD_M - (b.top_y - b.deck_y)) / district_h;
                let yaw = (-b.inward[0]).atan2(-b.inward[1]);
                let reach = district_reach * scale;
                let head = [b.x + b.inward[0] * reach, b.deck_y + DECK_HEAD_M, b.z + b.inward[1] * reach];
                let p = Prop { model: Model::LampCobra, pos: foot, yaw, scale: [scale as f32; 3], variant: 0, live: crate::Live::None };
                // nothing of another deck (an upper one, a neighbour's barrier) in the way.
                ex.clear();
                extents(d, &p, &mut ex);
                if !all_clear(s, &ex, Some(c.id)) {
                    continue;
                }
                let light = f.luminaire(head, foot, DECK_HEAD_M, Model::LampCobra);
                out.entry(eo_street::tile_key(foot[0], foot[2])).or_default().push(DeckLamp { prop: p, light, chain: c.id, s: st });
            }
            st += DECK_PITCH_M;
        }
    }
    out
}
