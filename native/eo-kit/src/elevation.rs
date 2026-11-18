//! `planElevation` and its part primitives (src/world/buildings/authored/kit/elevation.js).
//!
//! The spec is the JS object literal (a `serde_json::Value`); the plan is typed.
//! Part order, part ids and every dimension follow the JS, so a plan can be
//! compared part by part with the one node computes.
//!
//! Not ported: the sunk-entry and raised-drop interiors (`addSunkEntry`,
//! `addRaisedDrop`), which only a footway grade (`grade`, not supplied by this
//! port) can ask for; the containment report (`checkPlanContainment`, a
//! diagnostic: the mesh clamp in `mesh.rs` is what bounds the geometry).

use crate::frame::{Basis, Edge, FITTING_PROJECTION_ALLOWANCE, Frame, P2, rotation_y_for_direction};
use crate::js::*;
use crate::rng::{Rng, hash_seed};
use std::collections::BTreeMap;

pub const FITTING_KINDS: [&str; 9] =
    ["canopy", "canopy-rod", "awning", "blade-sign", "blade-bracket", "sign-lamp", "fire-escape", "fire-escape-rail", "fire-escape-ladder"];
pub const NON_COLLIDING_KINDS: [&str; 5] = ["door-leaf", "glass", "transom-glass", "awning", "sign-text"];

/// Slot -> default library material name.
pub const DEFAULT_MATERIAL_SLOTS: [(&str, &str); 22] = [
    ("wall", "brick-running-bond"),
    ("base", "cast-stone"),
    ("bulkhead", "cast-stone"),
    ("trim", "cast-stone"),
    ("spandrel", "concrete-precast"),
    ("frame", "painted-steel"),
    ("storefront", "anodized-aluminium"),
    ("glass", "glass-clear"),
    ("doorGlass", "glass-clear"),
    ("door", "painted-steel"),
    ("canopy", "painted-steel"),
    ("signage", "signage"),
    ("fascia", "painted-steel"),
    ("awning", "awning-canvas"),
    ("roof", "asphalt-roof"),
    ("parapet", "brick-running-bond"),
    ("coping", "cast-stone"),
    ("plant", "painted-steel"),
    ("pipe", "painted-steel"),
    ("metal", "painted-steel"),
    ("recess", "painted-stucco"),
    ("roller", "painted-steel-light"),
];

pub fn elevation_defaults() -> J {
    json!({
        "wallThickness": 0.3,
        "reveal": 0.2,
        "endPier": 0.6,
        "pierMin": 0.8,
        "bayWidth": 3.6,
        "window": { "type": "punched", "sillHeight": 0.9, "headClear": 0.5, "widthRatio": 0.55, "frameWidth": 0.07, "frameDepth": 0.08, "mullions": 1, "transoms": 0 },
        "ribbon": { "spandrelHeight": 0.9, "headClear": 0.35, "mullionSpacing": 1.5 },
        "sill": { "height": 0.1, "projection": 0.06, "overhang": 0.08 },
        "storefront": { "stallRiser": 0.5, "headClear": 0.6, "pier": 0.15, "mullionSpacing": 1.6, "transom": 0.5, "frameWidth": 0.09, "frameDepth": 0.12 },
        "entrance": { "width": 1.8, "height": 2.6, "recess": 0.6, "transom": 0.5, "leaf": "double-glass", "frameWidth": 0.08 },
        "roller": { "width": 4.9, "height": 4.2, "inset": 0.12, "hood": 0.45, "slat": 0.45 },
        "parapet": { "height": 0.8, "thickness": 0.3 },
        "coping": { "height": 0.1, "overhang": 0.05 },
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Box,
    /// Closed `[w, y]` polygon extruded along s.
    Profile(Vec<[f64; 2]>),
    /// Closed `[s, y]` polygon extruded along w.
    Plate(Vec<[f64; 2]>),
    /// Vertical prism over a world polygon.
    Prism(Vec<P2>),
}

impl Shape {
    pub fn name(&self) -> &'static str {
        match self {
            Shape::Box => "box",
            Shape::Profile(_) => "profile",
            Shape::Plate(_) => "plate",
            Shape::Prism(_) => "prism",
        }
    }
}

/// Painted fenestration UVs (create-archetype-building.js fenestrateBlindWalls).
#[derive(Clone, Debug, PartialEq)]
pub struct Fenestration {
    pub u: [f64; 2],
    pub v: [f64; 2],
    /// Envelope prisms: storeys counted from the base.
    pub storeys: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Part {
    pub id: String,
    pub kind: String,
    pub slot: String,
    pub shape: Shape,
    pub basis: Basis,
    pub s0: f64,
    pub s1: f64,
    pub y0: f64,
    pub y1: f64,
    pub w0: f64,
    pub w1: f64,
    pub edge: Option<usize>,
    pub storey: Option<i64>,
    pub bay: Option<i64>,
    /// `projectionClass === 'fitting'`.
    pub fitting: bool,
    pub cast_shadow: bool,
    pub receive_shadow: bool,
    pub meta: J,
    /// Prisms: draw the side faces / caps.
    pub side_faces: bool,
    pub top_cap: bool,
    pub bottom_cap: bool,
    pub fenestration: Option<Fenestration>,
}

impl Part {
    pub fn meta_flag(&self, key: &str) -> bool {
        self.meta.truthy(key)
    }
    pub fn is_box(&self) -> bool {
        matches!(self.shape, Shape::Box)
    }
}

/// What a module hands `add_*_part`: the JS object literal's fields.
#[derive(Clone, Debug, Default)]
pub struct PartArgs {
    pub id: Option<String>,
    pub kind: String,
    pub slot: String,
    pub s0: f64,
    pub s1: f64,
    pub y0: f64,
    pub y1: f64,
    pub w0: f64,
    pub w1: f64,
    pub edge: Option<usize>,
    pub storey: Option<i64>,
    pub bay: Option<i64>,
    pub cast_shadow: Option<bool>,
    pub receive_shadow: Option<bool>,
    pub meta: Option<J>,
    /// Profile `[w, y]` or plate `[s, y]` polygon.
    pub profile: Vec<[f64; 2]>,
    /// Prism world points.
    pub points: Vec<P2>,
    pub side_faces: Option<bool>,
}

impl PartArgs {
    /// A box (or profile / plate) in an edge basis.
    #[allow(clippy::too_many_arguments)]
    pub fn new(kind: &str, slot: &str, s0: f64, s1: f64, y0: f64, y1: f64, w0: f64, w1: f64) -> Self {
        Self { kind: kind.into(), slot: slot.into(), s0, s1, y0, y1, w0, w1, ..Default::default() }
    }
    /// A profile swept from s0 to s1.
    pub fn profile(kind: &str, slot: &str, s0: f64, s1: f64, profile: Vec<[f64; 2]>) -> Self {
        Self { kind: kind.into(), slot: slot.into(), s0, s1, profile, ..Default::default() }
    }
    pub fn edge(mut self, e: usize) -> Self {
        self.edge = Some(e);
        self
    }
    pub fn edge_opt(mut self, e: Option<usize>) -> Self {
        self.edge = e;
        self
    }
    pub fn storey(mut self, s: i64) -> Self {
        self.storey = Some(s);
        self
    }
    pub fn storey_opt(mut self, s: Option<i64>) -> Self {
        self.storey = s;
        self
    }
    pub fn bay(mut self, b: i64) -> Self {
        self.bay = Some(b);
        self
    }
    pub fn bay_opt(mut self, b: Option<i64>) -> Self {
        self.bay = b;
        self
    }
    pub fn shadow(mut self, cast: bool) -> Self {
        self.cast_shadow = Some(cast);
        self
    }
    pub fn receive(mut self, r: bool) -> Self {
        self.receive_shadow = Some(r);
        self
    }
    pub fn meta(mut self, m: J) -> Self {
        self.meta = Some(m);
        self
    }
    pub fn id(mut self, id: &str) -> Self {
        self.id = Some(id.into());
        self
    }
    /// `{ ...common }`: edge, storey, bay and meta of an opening.
    pub fn common(mut self, c: &Common) -> Self {
        self.edge = c.edge;
        self.storey = c.storey;
        self.bay = c.bay;
        if self.meta.is_none() {
            self.meta = c.meta.clone();
        }
        self
    }
}

/// The `common` object the opening emitters spread into every part.
#[derive(Clone, Debug, Default)]
pub struct Common {
    pub edge: Option<usize>,
    pub storey: Option<i64>,
    pub bay: Option<i64>,
    pub meta: Option<J>,
}

#[derive(Clone, Debug)]
pub struct Storey {
    pub index: usize,
    pub height: f64,
    pub y0: f64,
    pub y1: f64,
    pub use_: String,
    /// The spec entry's other fields.
    pub extra: J,
}

#[derive(Clone, Debug, Default)]
pub struct Envelope {
    pub wall_top: f64,
    pub roof_top: f64,
    pub height: f64,
    pub height_delta: f64,
    pub roof_plant_allowance: f64,
    pub wall_thickness: f64,
    pub floor_level: f64,
}

#[derive(Clone, Debug)]
pub struct BayRecord {
    pub index: usize,
    pub s0: f64,
    pub s1: f64,
    pub width: f64,
}

#[derive(Clone, Debug)]
pub struct OpeningRecord {
    pub id: String,
    pub type_: String,
    pub storey: usize,
    pub bay: usize,
    pub s0: f64,
    pub s1: f64,
    pub y0: f64,
    pub y1: f64,
}

#[derive(Clone, Debug)]
pub struct EdgeRecord {
    pub index: usize,
    pub role: &'static str,
    pub party: bool,
    pub length: f64,
    pub bays: Vec<BayRecord>,
    pub openings: Vec<OpeningRecord>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub parts: Vec<Part>,
    pub stoop_parts: Vec<Part>,
    pub door_notches: Vec<J>,
    pub id: String,
    pub seed: String,
    pub seed_hash: u32,
    pub base_y: f64,
    pub height: f64,
    pub storeys: Vec<Storey>,
    pub floor_level: f64,
    pub envelope: Envelope,
    pub materials: BTreeMap<String, String>,
    pub edges: Vec<EdgeRecord>,
    pub apertures: Vec<J>,
    pub signs: Vec<J>,
    pub warnings: Vec<String>,
    /// Anything a module hangs on the plan (`plan.archetype`, `plan.tints`,
    /// `plan.shopGround`, `plan.lotLineEdges`, ...).
    pub extra: Map<String, J>,
    /// `partCounter` (module-global in the JS, reset by planElevation).
    pub counter: u64,
}

/// Where a part goes: the plan's parts, or its stoop parts.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sink {
    Parts,
    Stoop,
}

impl Plan {
    /// An empty plan (for sites that are not planned by planElevation).
    pub fn empty() -> Self {
        Plan {
            parts: vec![],
            stoop_parts: vec![],
            door_notches: vec![],
            id: String::new(),
            seed: String::new(),
            seed_hash: 0,
            base_y: 0.0,
            height: 0.0,
            storeys: vec![],
            floor_level: 0.0,
            envelope: Envelope::default(),
            materials: BTreeMap::new(),
            edges: vec![],
            apertures: vec![],
            signs: vec![],
            warnings: vec![],
            extra: Map::new(),
            counter: 0,
        }
    }

    pub fn material(&self, slot: &str) -> Option<&str> {
        self.materials.get(slot).map(|s| s.as_str())
    }

    fn sink(&mut self, sink: Sink) -> &mut Vec<Part> {
        match sink {
            Sink::Parts => &mut self.parts,
            Sink::Stoop => &mut self.stoop_parts,
        }
    }

    /// `addBoxPart(plan, basis, box)`; returns the index of the new part.
    pub fn add_box(&mut self, basis: &Basis, a: PartArgs) -> Option<usize> {
        self.add_box_to(Sink::Parts, basis, a)
    }

    pub fn add_box_to(&mut self, sink: Sink, basis: &Basis, a: PartArgs) -> Option<usize> {
        let (s0, s1) = (a.s0.min(a.s1), a.s0.max(a.s1));
        let (y0, y1) = (a.y0.min(a.y1), a.y0.max(a.y1));
        let (w0, w1) = (a.w0.min(a.w1), a.w0.max(a.w1));
        if !(s1 - s0 > 1e-4 && y1 - y0 > 1e-4 && w1 - w0 > 1e-4) {
            return None;
        }
        self.counter += 1;
        let id = a.id.clone().unwrap_or_else(|| format!("{}:{}", a.kind, self.counter));
        let fitting = FITTING_KINDS.contains(&a.kind.as_str());
        let cast = a.cast_shadow.unwrap_or(!NON_COLLIDING_KINDS.contains(&a.kind.as_str()));
        let part = Part {
            id,
            kind: a.kind,
            slot: a.slot,
            shape: Shape::Box,
            basis: *basis,
            s0,
            s1,
            y0,
            y1,
            w0,
            w1,
            edge: a.edge,
            storey: a.storey,
            bay: a.bay,
            fitting,
            cast_shadow: cast,
            receive_shadow: a.receive_shadow.unwrap_or(true),
            meta: a.meta.unwrap_or(J::Null),
            side_faces: true,
            top_cap: true,
            bottom_cap: true,
            fenestration: None,
        };
        let v = self.sink(sink);
        v.push(part);
        Some(v.len() - 1)
    }

    /// `addProfilePart(plan, basis, part)`.
    pub fn add_profile(&mut self, basis: &Basis, a: PartArgs) -> Option<usize> {
        self.add_profile_to(Sink::Parts, basis, a)
    }

    pub fn add_profile_to(&mut self, sink: Sink, basis: &Basis, a: PartArgs) -> Option<usize> {
        if a.profile.len() < 3 {
            return None;
        }
        let (s0, s1) = (a.s0.min(a.s1), a.s0.max(a.s1));
        if !(s1 - s0 > 1e-4) {
            return None;
        }
        let (mut w0, mut w1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for [w, y] in &a.profile {
            w0 = w0.min(*w);
            w1 = w1.max(*w);
            y0 = y0.min(*y);
            y1 = y1.max(*y);
        }
        self.counter += 1;
        let id = a.id.clone().unwrap_or_else(|| format!("{}:{}", a.kind, self.counter));
        let fitting = FITTING_KINDS.contains(&a.kind.as_str());
        let part = Part {
            id,
            kind: a.kind,
            slot: a.slot,
            shape: Shape::Profile(a.profile),
            basis: *basis,
            s0,
            s1,
            y0,
            y1,
            w0,
            w1,
            edge: a.edge,
            storey: None,
            bay: None,
            fitting,
            cast_shadow: a.cast_shadow.unwrap_or(true),
            receive_shadow: a.receive_shadow.unwrap_or(true),
            meta: a.meta.unwrap_or(J::Null),
            side_faces: true,
            top_cap: true,
            bottom_cap: true,
            fenestration: None,
        };
        let v = self.sink(sink);
        v.push(part);
        Some(v.len() - 1)
    }

    /// `addPlatePart(plan, basis, part)`: `a.profile` holds the `[s, y]` points.
    pub fn add_plate(&mut self, basis: &Basis, a: PartArgs) -> Option<usize> {
        if a.profile.len() < 3 {
            return None;
        }
        let (mut s0, mut s1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for [s, y] in &a.profile {
            s0 = s0.min(*s);
            s1 = s1.max(*s);
            y0 = y0.min(*y);
            y1 = y1.max(*y);
        }
        let (w0, w1) = (a.w0.min(a.w1), a.w0.max(a.w1));
        if !(s1 - s0 > 1e-4 && y1 - y0 > 1e-4 && w1 - w0 > 1e-5) {
            return None;
        }
        self.counter += 1;
        let id = a.id.clone().unwrap_or_else(|| format!("{}:{}", a.kind, self.counter));
        let fitting = FITTING_KINDS.contains(&a.kind.as_str());
        let cast = a.cast_shadow.unwrap_or(!NON_COLLIDING_KINDS.contains(&a.kind.as_str()));
        self.parts.push(Part {
            id,
            kind: a.kind,
            slot: a.slot,
            shape: Shape::Plate(a.profile),
            basis: *basis,
            s0,
            s1,
            y0,
            y1,
            w0,
            w1,
            edge: a.edge,
            storey: a.storey,
            bay: a.bay,
            fitting,
            cast_shadow: cast,
            receive_shadow: a.receive_shadow.unwrap_or(true),
            meta: a.meta.unwrap_or(J::Null),
            side_faces: true,
            top_cap: true,
            bottom_cap: true,
            fenestration: None,
        });
        Some(self.parts.len() - 1)
    }

    /// `addPrismPart(plan, part)`: a vertical prism over world points.
    pub fn add_prism(&mut self, a: PartArgs) -> Option<usize> {
        if a.points.len() < 3 {
            return None;
        }
        self.counter += 1;
        let id = a.id.clone().unwrap_or_else(|| format!("{}:{}", a.kind, self.counter));
        self.parts.push(Part {
            id,
            kind: a.kind,
            slot: a.slot,
            shape: Shape::Prism(a.points),
            basis: Basis::default(),
            s0: 0.0,
            s1: 0.0,
            y0: a.y0.min(a.y1),
            y1: a.y0.max(a.y1),
            w0: 0.0,
            w1: 0.0,
            edge: None,
            storey: None,
            bay: None,
            fitting: false,
            cast_shadow: a.cast_shadow.unwrap_or(true),
            receive_shadow: a.receive_shadow.unwrap_or(true),
            meta: a.meta.unwrap_or(J::Null),
            side_faces: a.side_faces != Some(false),
            top_cap: true,
            bottom_cap: true,
            fenestration: None,
        });
        Some(self.parts.len() - 1)
    }

    /// Bump the shared id counter (`partCounter += 1`) and return it.
    pub fn next_id(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }
}

/// `arcPoints(centreS, centreY, radius, a0, a1, segments)`.
pub fn arc_points(cs: f64, cy: f64, r: f64, a0: f64, a1: f64, segments: usize) -> Vec<[f64; 2]> {
    (0..=segments)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f64 / segments as f64;
            [cs + a.cos() * r, cy + a.sin() * r]
        })
        .collect()
}

/// `trimProfile(kind, { projection, height, y0, back })`.
pub fn trim_profile(kind: &str, projection: f64, height: f64, y0: f64, back: f64) -> Vec<[f64; 2]> {
    let p = projection;
    let h = height;
    let y = |t: f64| y0 + h * t;
    match kind {
        "stepped" => vec![
            [back, y(0.0)],
            [p * 0.4, y(0.0)],
            [p * 0.4, y(0.3)],
            [p * 0.7, y(0.3)],
            [p * 0.7, y(0.62)],
            [p, y(0.62)],
            [p, y(1.0)],
            [back, y(1.0)],
        ],
        "ogee" => {
            let mut pts = vec![[back, y(0.0)], [p * 0.25, y(0.0)]];
            for i in 1..=6 {
                let t = i as f64 / 6.0;
                let ease = if t < 0.5 { 2.0 * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(2) / 2.0 };
                pts.push([p * (0.25 + 0.75 * ease), y(0.15 + 0.7 * t)]);
            }
            pts.push([p, y(1.0)]);
            pts.push([back, y(1.0)]);
            pts
        }
        "cove" => vec![[back, y(0.0)], [p * 0.2, y(0.0)], [p * 0.45, y(0.25)], [p * 0.75, y(0.55)], [p, y(0.85)], [p, y(1.0)], [back, y(1.0)]],
        "sloped" => vec![[back, y(0.0)], [p, y(0.0)], [p, y(0.55)], [back, y(1.0)]],
        _ => vec![[back, y(0.0)], [p, y(0.0)], [p, y(1.0)], [back, y(1.0)]],
    }
}

/// `trimProfile` with the JS default `back = -0.02`.
pub fn trim_profile_d(kind: &str, projection: f64, height: f64, y0: f64) -> Vec<[f64; 2]> {
    trim_profile(kind, projection, height, y0, -0.02)
}

/// `arr?.[i]` for an array or an object with numeric keys.
pub fn index_any(v: Option<&J>, i: usize) -> Option<&J> {
    match v {
        Some(J::Array(a)) => a.get(i),
        Some(J::Object(o)) => o.get(&i.to_string()),
        _ => None,
    }
}

fn resolve_storeys(spec: &J, height: f64, roof_above: f64, floor_level: f64) -> Vec<Storey> {
    let wall_top_target = height - roof_above;
    let mut storeys: Vec<Storey> = vec![];
    if let Some(list) = spec.arr("storeys") {
        for entry in list {
            if let Some(n) = entry.as_f64() {
                storeys.push(Storey { index: 0, height: n, y0: 0.0, y1: 0.0, use_: String::new(), extra: J::Null });
            } else {
                let h = to_number(entry.at("height"));
                storeys.push(Storey {
                    index: 0,
                    height: h,
                    y0: 0.0,
                    y1: 0.0,
                    use_: entry.s("use").unwrap_or("").to_string(),
                    extra: entry.clone(),
                });
            }
        }
    } else {
        let st = spec.get_nn("storeys");
        let count = st.f("count").unwrap_or_else(|| js_round(wall_top_target / 3.4).max(1.0)).max(1.0) as usize;
        let ground = st
            .f("groundHeight")
            .unwrap_or(if count > 1 { 4.6f64.min(wall_top_target * 0.4) } else { wall_top_target });
        let upper = if count > 1 { (wall_top_target - ground) / (count - 1) as f64 } else { 0.0 };
        let ground_use = st.s("groundUse").unwrap_or("retail").to_string();
        for i in 0..count {
            storeys.push(Storey {
                index: i,
                height: if i == 0 { ground } else { upper },
                y0: 0.0,
                y1: 0.0,
                use_: if i == 0 { ground_use.clone() } else { "upper".into() },
                extra: J::Null,
            });
        }
    }
    let mut y = floor_level.max(0.0);
    for (i, s) in storeys.iter_mut().enumerate() {
        s.index = i;
        s.height = if s.height.is_nan() || s.height == 0.0 { 3.2f64 } else { s.height }.max(2.2);
        s.y0 = y;
        y += s.height;
        s.y1 = y;
        if s.use_.is_empty() {
            s.use_ = if i == 0 { "retail".into() } else { "upper".into() };
        }
    }
    storeys
}

/// `resolveElevation(spec, edge)`: always an object.
pub fn resolve_elevation(spec: &J, edge: &Edge) -> J {
    let entry = resolve_elevation_raw(spec, edge);
    match entry {
        J::String(s) => json!({ "window": s }),
        other => other,
    }
}

/// The spec's entry for an edge before a string is widened to `{ window }`.
pub fn resolve_elevation_raw(spec: &J, edge: &Edge) -> J {
    let elevations = spec.get_nn("elevations");
    let mut entry = elevations.obj("byEdge").and_then(|m| m.get(&edge.index.to_string())).cloned();
    if entry.is_none() {
        entry = elevations.at(edge.role).cloned();
    }
    if entry.is_none() && edge.role == "front-secondary" {
        entry = elevations.at("front").cloned();
    }
    if entry.is_none() {
        entry = elevations.at("default").cloned();
    }
    entry.unwrap_or_else(|| if edge.role == "front" { json!({ "window": "punched" }) } else { J::String("blind".into()) })
}

#[derive(Clone, Debug)]
pub struct Bay {
    pub index: usize,
    pub s0: f64,
    pub s1: f64,
    pub width: f64,
    pub spec: J,
}

pub struct BayLayout {
    pub bays: Vec<Bay>,
    pub scale: f64,
    pub end_pier: f64,
}

/// `layoutBays(edge, elev, defaults)`.
pub fn layout_bays(edge_length: f64, elev: &J, defaults: &J) -> BayLayout {
    let end_pier = elev.f("endPier").unwrap_or_else(|| defaults.f("endPier").unwrap());
    let available = (edge_length - 2.0 * end_pier).max(0.0);
    let mut widths: Vec<f64>;
    let specs: Vec<J>;
    if let Some(list) = elev.arr("bays") {
        widths = list
            .iter()
            .map(|b| {
                let w = if b.is_number() { to_number(Some(b)) } else { to_number(b.at("width")) };
                if w == 0.0 || w.is_nan() { defaults.f("bayWidth").unwrap() } else { w }
            })
            .collect();
        specs = list.iter().map(|b| if b.is_number() { json!({}) } else { b.clone() }).collect();
    } else {
        let bays_obj = elev.get_nn("bays");
        let bay_width = elev.f("bayWidth").unwrap_or_else(|| defaults.f("bayWidth").unwrap());
        let count = bays_obj.f("count").unwrap_or_else(|| (available / bay_width).floor().max(1.0));
        let count = if count.is_finite() && count > 0.0 { count as usize } else { 0 };
        widths = vec![available / count.max(1) as f64; count];
        let mut base = bays_obj.and_then(|v| v.as_object()).cloned().unwrap_or_default();
        base.shift_remove("count");
        specs = vec![J::Object(base); count];
    }
    let total: f64 = widths.iter().sum();
    let mut scale = 1.0;
    if total > available + 1e-6 && total > 0.0 {
        scale = available / total;
    }
    let slack = (available - total * scale).max(0.0);
    let align = elev.s("align");
    let mut s = end_pier + if align == Some("start") { 0.0 } else if align == Some("end") { slack } else { slack / 2.0 };
    let mut bays = vec![];
    for (i, w) in widths.iter_mut().enumerate() {
        let width = *w * scale;
        bays.push(Bay { index: i, s0: s, s1: s + width, width, spec: specs.get(i).cloned().unwrap_or(json!({})) });
        s += width;
    }
    BayLayout { bays, scale, end_pier }
}

fn storey_overrides(bay: &Bay, storey: &Storey) -> J {
    match index_any(bay.spec.get_nn("storeys"), storey.index) {
        Some(e) if e.is_object() => e.clone(),
        _ => json!({}),
    }
}

/// `windowTypeFor(elev, bay, storey)` (None: undefined).
pub fn window_type_for(elev: &J, bay: &Bay, storey_index: usize) -> Option<String> {
    let as_type = |v: &J| -> Option<String> {
        match v {
            J::String(s) => Some(s.clone()),
            J::Null => None,
            other => other.s("window").map(|s| s.to_string()),
        }
    };
    if let Some(v) = index_any(bay.spec.get_nn("storeys"), storey_index) {
        return as_type(v);
    }
    if let Some(v) = index_any(elev.get_nn("storeys"), storey_index) {
        return as_type(v);
    }
    if let Some(v) = bay.spec.at("window") {
        return as_type(v);
    }
    if storey_index == 0 {
        if let Some(v) = elev.at("ground") {
            return as_type(v);
        }
    }
    Some(elev.s("window").unwrap_or("punched").to_string())
}

#[derive(Clone, Debug)]
pub struct Opening {
    pub type_: String,
    pub id: String,
    pub s0: f64,
    pub s1: f64,
    pub y0: f64,
    pub y1: f64,
    pub door_top: f64,
    pub spec: J,
    pub storey_overrides: J,
    pub below_slot: Option<&'static str>,
    pub storey: usize,
    pub bay: usize,
}

/// Planning context shared by every edge (`ctx` in planElevation).
pub struct Ctx {
    pub seed: String,
    pub base_y: f64,
    pub wall_top: f64,
    pub floor_level: f64,
    pub wall_thickness: f64,
    pub parapet: Option<J>,
    pub coping: Option<J>,
}

impl Ctx {
    pub fn storefront_head(&self, storey: &Storey, elev: &J, defaults: &J) -> f64 {
        let sf = merge(&defaults["storefront"], elev.at("storefrontSpec")).unwrap_or(json!({}));
        if truthy(elev.at("signband")) {
            let sb = spread(Some(&json!({ "height": 0.8, "offset": 0.1 })), elev.at("signband"));
            return storey.y1 - sb.num("offset") - sb.num("height") - 0.1;
        }
        storey.y1 - sf.num("headClear")
    }
}

#[allow(clippy::too_many_arguments)]
fn opening_for(type_: &str, bay: &Bay, storey: &Storey, elev: &J, defaults: &J, ctx: &Ctx) -> Option<Opening> {
    let so = storey_overrides(bay, storey);
    let offset = so.get_nn("offset").or(bay.spec.get_nn("offset"));
    let mid = (bay.s0 + bay.s1) / 2.0 + num_or(offset, 0.0);
    let mk = |s0: f64, s1: f64, y0: f64, y1: f64, spec: J, below: Option<&'static str>| Opening {
        type_: type_.to_string(),
        id: String::new(),
        s0,
        s1,
        y0,
        y1,
        door_top: 0.0,
        spec,
        storey_overrides: json!({}),
        below_slot: below,
        storey: storey.index,
        bay: bay.index,
    };
    match type_ {
        "punched" => {
            let w = merge(&defaults["window"], elev.at("windowSpec")).unwrap_or(json!({}));
            let pier_min = elev.f("pierMin").unwrap_or(defaults.num("pierMin"));
            let want = so
                .f("openingWidth")
                .or(bay.spec.f("openingWidth"))
                .or(w.f("width"))
                .unwrap_or(bay.width * w.num("widthRatio"));
            let width = (bay.width - pier_min).min(want);
            if width < 0.5 {
                return None;
            }
            let y0 = storey.y0 + so.f("sillHeight").or(bay.spec.f("sillHeight")).unwrap_or(w.num("sillHeight"));
            let head = so.get_nn("head").or(bay.spec.get_nn("head")).or(w.get_nn("head"));
            let hn = to_number(head);
            let y1 = if head.is_some() && hn.is_finite() { storey.y0 + hn } else { storey.y1 - w.num("headClear") };
            if y1 - y0 < 0.6 {
                return None;
            }
            let mut op = mk(mid - width / 2.0, mid + width / 2.0, y0, y1, w, None);
            op.storey_overrides = so;
            Some(op)
        }
        "ribbon" => {
            let r = merge(&defaults["ribbon"], elev.at("ribbonSpec")).unwrap_or(json!({}));
            let y0 = storey.y0 + r.num("spandrelHeight");
            let y1 = storey.y1 - r.num("headClear");
            if y1 - y0 < 0.6 {
                return None;
            }
            Some(mk(bay.s0, bay.s1, y0, y1, r, Some("spandrel")))
        }
        "storefront" => {
            let sf = merge(&defaults["storefront"], elev.at("storefrontSpec")).unwrap_or(json!({}));
            let y0 = storey.y0 + bay.spec.f("stallRiser").unwrap_or(sf.num("stallRiser"));
            let y1 = ctx.storefront_head(storey, elev, defaults);
            if y1 - y0 < 1.2 {
                return None;
            }
            let pier = sf.num("pier");
            Some(mk(bay.s0 + pier, bay.s1 - pier, y0, y1, sf, Some("bulkhead")))
        }
        "roller" => {
            let over = spread(elev.get_nn("rollerSpec"), bay.spec.get_nn("roller"));
            let rd = merge(&defaults["roller"], Some(&over)).unwrap_or(json!({}));
            let width = rd.num("width").min(bay.width - 0.4);
            if width < 1.5 {
                return None;
            }
            let y1 = (storey.y0 + rd.num("height")).min(storey.y1 - 0.4);
            Some(mk(mid - width / 2.0, mid + width / 2.0, storey.y0, y1, rd, None))
        }
        "entrance" => {
            let over = spread(elev.get_nn("entranceSpec"), bay.spec.get_nn("entrance"));
            let en = merge(&defaults["entrance"], Some(&over)).unwrap_or(json!({}));
            let width = en.num("width").min(bay.width - 0.4);
            let sink = num_or(en.get_nn("sink").at("sink"), 0.0).max(0.0);
            let y0 = storey.y0 + num_or(en.at("raise"), 0.0).max(0.0) - sink;
            let door_top = y0 + en.num("height");
            let y1 = (door_top + sink + en.num("transom")).min(ctx.storefront_head(storey, elev, defaults));
            let mut op = mk(mid - width / 2.0, mid + width / 2.0, y0, y1, en, None);
            op.door_top = door_top;
            Some(op)
        }
        _ => None,
    }
}

const STOOP_CHEEK_M: f64 = 0.12;

fn add_entry_stoop(plan: &mut Plan, basis: &Basis, common: &Common, op: &Opening, stoop: &J, recess: f64) {
    let drop = stoop.num("drop");
    let reach = stoop.num("reach");
    let risers = stoop.num("risers");
    let riser = stoop.num("riser");
    let tread = stoop.num("tread");
    let back = -recess - 0.05;
    let foot = op.y0 - drop;
    let bottom = op.y0 - reach;
    let meta = json!({ "belowFloor": true, "stoop": { "drop": jn(drop), "risers": jn(risers), "riser": jn(riser), "tread": jn(tread) } });
    let mut profile = vec![[back, bottom], [0.0, bottom], [0.0, foot + riser]];
    let mut i = 1.0;
    while i < risers {
        profile.push([-i * tread, foot + i * riser]);
        profile.push([-i * tread, foot + (i + 1.0) * riser]);
        i += 1.0;
    }
    profile.push([back, op.y0]);
    let mut c = common.clone();
    c.meta = None;
    plan.add_profile_to(Sink::Stoop, basis, PartArgs::profile("entry-stoop", "base", op.s0, op.s1, profile).common(&c).meta(meta.clone()));
    for (s0, s1) in [(op.s0 - STOOP_CHEEK_M, op.s0), (op.s1, op.s1 + STOOP_CHEEK_M)] {
        plan.add_box_to(
            Sink::Stoop,
            basis,
            PartArgs::new("entry-stoop-cheek", "recess", s0, s1, bottom, op.y0, back, 0.0).common(&c).meta(meta.clone()),
        );
    }
    plan.door_notches.push(json!({
        "s0": op.s0 - STOOP_CHEEK_M, "s1": op.s1 + STOOP_CHEEK_M, "depth": recess + 0.1,
        "origin": basis.origin, "u": basis.u, "n": basis.n,
    }));
}

/// Marks a shop window's frame, mullions, transom and riser cap.
pub fn shop_joinery_meta() -> J {
    json!({ "shopJoinery": true })
}

#[allow(clippy::too_many_arguments)]
fn emit_frame_and_glass(
    plan: &mut Plan,
    edge: &Edge,
    op: &Opening,
    storey: usize,
    bay: usize,
    slot: &str,
    frame_width: f64,
    frame_depth: f64,
    w_at: f64,
    mullion_count: i64,
    transom_count: i64,
    transom_at: Option<&J>,
    glass_slot: &str,
    kind_prefix: &str,
    meta: Option<J>,
) {
    let basis = edge.basis;
    let common = Common { edge: Some(edge.index), storey: Some(storey as i64), bay: Some(bay as i64), meta: meta.clone() };
    let wf0 = w_at - frame_depth / 2.0;
    let wf1 = w_at + frame_depth / 2.0;
    let fk = format!("{kind_prefix}frame");
    plan.add_box(&basis, PartArgs::new(&fk, slot, op.s0, op.s0 + frame_width, op.y0, op.y1, wf0, wf1).common(&common));
    plan.add_box(&basis, PartArgs::new(&fk, slot, op.s1 - frame_width, op.s1, op.y0, op.y1, wf0, wf1).common(&common));
    plan.add_box(&basis, PartArgs::new(&fk, slot, op.s0, op.s1, op.y1 - frame_width, op.y1, wf0, wf1).common(&common));
    plan.add_box(&basis, PartArgs::new(&fk, slot, op.s0, op.s1, op.y0, op.y0 + frame_width, wf0, wf1).common(&common));
    let mw = frame_width * 0.7;
    let mut i = 1;
    while i <= mullion_count {
        let s = op.s0 + ((op.s1 - op.s0) * i as f64) / (mullion_count + 1) as f64;
        plan.add_box(&basis, PartArgs::new("mullion", slot, s - mw / 2.0, s + mw / 2.0, op.y0, op.y1, wf0 + 0.01, wf1 - 0.01).common(&common));
        i += 1;
    }
    let explicit = transom_at.filter(|t| match t {
        J::Array(_) => true,
        J::Null => false,
        other => to_number(Some(other)).is_finite(),
    });
    let ys: Vec<f64> = if let Some(t) = explicit {
        let list: Vec<f64> = match t {
            J::Array(a) => a.iter().map(|v| to_number(Some(v))).collect(),
            other => vec![to_number(Some(other))],
        };
        list.into_iter().filter(|t| t.is_finite() && *t > frame_width && *t < op.y1 - op.y0 - frame_width).map(|t| op.y0 + t).collect()
    } else {
        let n = transom_count.max(0);
        (0..n).map(|i| op.y0 + ((op.y1 - op.y0) * (i + 1) as f64) / (n + 1) as f64).collect()
    };
    for y in ys {
        plan.add_box(&basis, PartArgs::new("transom", slot, op.s0, op.s1, y - mw / 2.0, y + mw / 2.0, wf0 + 0.01, wf1 - 0.01).common(&common));
    }
    // `{ ...common, meta: { opening } }`: the glass's own meta replaces common's.
    let glass_meta = json!({ "opening": op.id });
    plan.add_box(
        &basis,
        PartArgs::new(
            "glass",
            glass_slot,
            op.s0 + frame_width * 0.5,
            op.s1 - frame_width * 0.5,
            op.y0 + frame_width * 0.5,
            op.y1 - frame_width * 0.5,
            w_at - 0.012,
            w_at,
        )
        .common(&Common { meta: None, ..common.clone() })
        .shadow(false)
        .meta(glass_meta),
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_opening(plan: &mut Plan, ctx: &Ctx, edge: &Edge, op: &Opening, storey: &Storey, bay: &Bay, elev: &J, defaults: &J) {
    let basis = edge.basis;
    let mut common = Common { edge: Some(edge.index), storey: Some(storey.index as i64), bay: Some(bay.index as i64), meta: None };
    let reveal_pref = bay.spec.f("reveal").or(elev.f("reveal")).unwrap_or(defaults.num("reveal"));
    let reveal = reveal_pref.min(ctx.wall_thickness - 0.02);
    match op.type_.as_str() {
        "punched" | "ribbon" => {
            let w = &op.spec;
            let so = &op.storey_overrides;
            let ribbon = op.type_ == "ribbon";
            let mullions = if ribbon {
                (js_round((op.s1 - op.s0) / w.num("mullionSpacing")) - 1.0).max(0.0) as i64
            } else {
                so.f("mullions").or(bay.spec.f("mullions")).or(w.f("mullions")).unwrap_or(0.0) as i64
            };
            let transoms = if ribbon { 0 } else { so.f("transoms").or(bay.spec.f("transoms")).or(w.f("transoms")).unwrap_or(0.0) as i64 };
            let transom_at = if ribbon { None } else { so.get_nn("transomAt").or(bay.spec.get_nn("transomAt")).or(w.get_nn("transomAt")) };
            let glass_slot = bay.spec.s("glassSlot").filter(|s| !s.is_empty()).unwrap_or("glass").to_string();
            emit_frame_and_glass(
                plan,
                edge,
                op,
                storey.index,
                bay.index,
                "frame",
                w.f("frameWidth").unwrap_or(defaults["window"].num("frameWidth")),
                w.f("frameDepth").unwrap_or(defaults["window"].num("frameDepth")),
                -reveal,
                mullions,
                transoms,
                transom_at,
                &glass_slot,
                "",
                None,
            );
            let sill_over = if ribbon { Some(json!({ "height": 0.06, "projection": 0.03, "overhang": 0.02 })) } else { elev.at("sill").cloned() };
            let sill = merge(&defaults["sill"], sill_over.as_ref());
            if let Some(sill) = &sill {
                let slot = elev.s("sillSlot").filter(|s| !s.is_empty()).unwrap_or("trim");
                let profile = trim_profile(
                    sill.s("profile").filter(|s| !s.is_empty()).unwrap_or("sloped"),
                    sill.num("projection").min(edge.allowance),
                    sill.num("height"),
                    op.y0 - sill.num("height"),
                    -reveal,
                );
                plan.add_profile(&basis, PartArgs::profile("sill", slot, op.s0 - sill.num("overhang"), op.s1 + sill.num("overhang"), profile).common(&common));
            }
            if truthy(elev.at("lintel")) {
                let l = spread(Some(&json!({ "height": 0.25, "projection": 0.05, "overhang": 0.12 })), elev.at("lintel"));
                let slot = l.s("slot").filter(|s| !s.is_empty()).unwrap_or("trim").to_string();
                plan.add_box(
                    &basis,
                    PartArgs::new("lintel", &slot, op.s0 - l.num("overhang"), op.s1 + l.num("overhang"), op.y1, op.y1 + l.num("height"), -0.02, l.num("projection").min(edge.allowance))
                        .common(&common),
                );
            }
            if truthy(elev.at("surround")) {
                let sr = spread(Some(&json!({ "width": 0.18, "projection": 0.05 })), elev.at("surround"));
                let p = sr.num("projection").min(edge.allowance);
                let slot = sr.s("slot").filter(|s| !s.is_empty()).unwrap_or("trim").to_string();
                let sh = sill.as_ref().map(|s| s.num("height")).unwrap_or(0.0);
                let wd = sr.num("width");
                plan.add_box(&basis, PartArgs::new("jamb", &slot, op.s0 - wd, op.s0, op.y0 - sh, op.y1 + wd, -0.02, p).common(&common));
                plan.add_box(&basis, PartArgs::new("jamb", &slot, op.s1, op.s1 + wd, op.y0 - sh, op.y1 + wd, -0.02, p).common(&common));
                plan.add_box(&basis, PartArgs::new("jamb", &slot, op.s0 - wd, op.s1 + wd, op.y1, op.y1 + wd, -0.02, p).common(&common));
            }
        }
        "storefront" => {
            let sf = &op.spec;
            let mullions = (js_round((op.s1 - op.s0) / sf.num("mullionSpacing")) - 1.0).max(0.0) as i64;
            let transom_y = op.y1 - sf.num("transom");
            let shop = Common { meta: Some(shop_joinery_meta()), ..common.clone() };
            let setback = reveal.min(sf.f("setback").unwrap_or(0.12));
            let fw = sf.num("frameWidth");
            let fd = sf.num("frameDepth");
            emit_frame_and_glass(plan, edge, op, storey.index, bay.index, "storefront", fw, fd, -setback, mullions, 0, None, "glass", "", Some(shop_joinery_meta()));
            if truthy(sf.at("mullionCap")) && mullions > 0 {
                let cap = spread(Some(&json!({ "width": fw * 0.9, "depth": 0.05 })), if sf.at("mullionCap").map(|v| v.is_object()).unwrap_or(false) { sf.at("mullionCap") } else { None });
                for i in 1..=mullions {
                    let s = op.s0 + ((op.s1 - op.s0) * i as f64) / (mullions + 1) as f64;
                    plan.add_box(
                        &basis,
                        PartArgs::new(
                            "mullion",
                            "storefront",
                            s - cap.num("width") / 2.0,
                            s + cap.num("width") / 2.0,
                            op.y0 + 0.02,
                            op.y1 - 0.02,
                            -setback + fd / 2.0,
                            -setback + fd / 2.0 + cap.num("depth"),
                        )
                        .common(&shop),
                    );
                }
            }
            if sf.num("transom") > 0.0 && transom_y > op.y0 + 0.5 {
                let cap_extra = if truthy(sf.at("mullionCap")) { 0.05 } else { -0.01 };
                plan.add_box(
                    &basis,
                    PartArgs::new("transom", "storefront", op.s0, op.s1, transom_y - 0.04, transom_y + 0.04, -setback - fd / 2.0 + 0.01, -setback + fd / 2.0 + cap_extra)
                        .common(&shop),
                );
            }
            plan.add_box(
                &basis,
                PartArgs::new("sill", "trim", op.s0 - 0.03, op.s1 + 0.03, op.y0 - 0.06, op.y0, -0.15, sf.f("capProjection").unwrap_or(0.04).min(edge.allowance)).common(&shop),
            );
        }
        "roller" => {
            let rd = &op.spec;
            let w_at = -(rd.num("inset").min(ctx.wall_thickness - 0.05));
            let rail = 0.08;
            plan.add_box(&basis, PartArgs::new("roller-guide", "metal", op.s0, op.s0 + rail, op.y0, op.y1, w_at - 0.06, w_at + 0.06).common(&common));
            plan.add_box(&basis, PartArgs::new("roller-guide", "metal", op.s1 - rail, op.s1, op.y0, op.y1, w_at - 0.06, w_at + 0.06).common(&common));
            plan.add_box(
                &basis,
                PartArgs::new("roller-door", "roller", op.s0 + rail, op.s1 - rail, op.y0 + 0.02, op.y1, w_at - 0.02, w_at + 0.02).common(&common).meta(json!({ "opening": op.id })),
            );
            let slat = rd.num("slat");
            let mut y = op.y0 + slat;
            while y < op.y1 - 0.1 {
                plan.add_box(
                    &basis,
                    PartArgs::new("roller-slat", "roller", op.s0 + rail, op.s1 - rail, y - 0.02, y + 0.02, w_at + 0.02, w_at + 0.05).common(&common).shadow(false),
                );
                y += slat;
            }
            plan.add_box(&basis, PartArgs::new("roller-bar", "metal", op.s0 + rail, op.s1 - rail, op.y0 + 0.02, op.y0 + 0.1, w_at - 0.03, w_at + 0.05).common(&common));
            let hood = rd.num("hood");
            plan.add_box(
                &basis,
                PartArgs::new("roller-hood", "metal", op.s0 - 0.1, op.s1 + 0.1, op.y1, op.y1 + hood, -ctx.wall_thickness - hood, -ctx.wall_thickness + 0.02).common(&common),
            );
        }
        "entrance" => {
            let en = &op.spec;
            let recess = en.num("recess").max(0.05);
            let w_leaf = -recess;
            let fw = en.num("frameWidth");
            let stoop = en.get_nn("stoop").filter(|v| truthy(Some(v))).cloned();
            let sunk = if op.y0 < storey.y0 - 1e-3 && truthy(en.at("sink")) { en.get_nn("sink").cloned() } else { None };
            if let Some(s) = &sunk {
                common.meta = Some(json!({ "belowFloor": true, "sunkEntry": true, "floorY": op.y0 - num_or(s.at("reach"), 0.0).max(0.08) }));
            }
            if recess > ctx.wall_thickness + 0.01 {
                let cheek = 0.12;
                let wt = ctx.wall_thickness;
                plan.add_box(&basis, PartArgs::new("recess", "recess", op.s0 - cheek, op.s0, op.y0 - 0.05, op.y1 + 0.12, -recess - 0.1, -wt).common(&common));
                plan.add_box(&basis, PartArgs::new("recess", "recess", op.s1, op.s1 + cheek, op.y0 - 0.05, op.y1 + 0.12, -recess - 0.1, -wt).common(&common));
                plan.add_box(&basis, PartArgs::new("recess", "recess", op.s0 - cheek, op.s1 + cheek, op.y1, op.y1 + 0.12, -recess - 0.1, -wt).common(&common));
            }
            let sunk_reach = sunk.as_ref().map(|s| num_or(s.at("reach"), 0.0)).unwrap_or(0.0);
            let thr_w1 = match &stoop {
                Some(s) => -s.num("run"),
                None => 0.06f64.min(edge.allowance),
            };
            plan.add_box(
                &basis,
                PartArgs::new("threshold", "base", op.s0 - 0.05, op.s1 + 0.05, op.y0 - 0.08f64.max(sunk_reach), op.y0 + 0.02, -recess - 0.05, thr_w1).common(&common).receive(true),
            );
            if let Some(s) = &stoop {
                add_entry_stoop(plan, &basis, &common, op, s, recess);
            }
            // addSunkEntry / addRaisedDrop: not ported (footway grade only).
            let wf0 = w_leaf - 0.05;
            let wf1 = w_leaf + 0.05;
            plan.add_box(&basis, PartArgs::new("door-frame", "door", op.s0, op.s0 + fw, op.y0, op.y1, wf0, wf1).common(&common));
            plan.add_box(&basis, PartArgs::new("door-frame", "door", op.s1 - fw, op.s1, op.y0, op.y1, wf0, wf1).common(&common));
            plan.add_box(&basis, PartArgs::new("door-frame", "door", op.s0, op.s1, op.y1 - fw, op.y1, wf0, wf1).common(&common));
            let door_top = op.door_top.min(op.y1 - fw);
            if op.y1 - door_top > fw + 0.15 {
                plan.add_box(&basis, PartArgs::new("door-frame", "door", op.s0, op.s1, door_top, door_top + fw, wf0, wf1).common(&common));
                plan.add_box(
                    &basis,
                    PartArgs::new("transom-glass", "glass", op.s0 + fw, op.s1 - fw, door_top + fw, op.y1 - fw, w_leaf - 0.006, w_leaf + 0.006).common(&common).shadow(false),
                );
            }
            let leaf = en.s("leaf").unwrap_or("").to_string();
            let leaves = if leaf.starts_with("double") { 2 } else { 1 };
            let glazed = leaf.ends_with("glass");
            let clear0 = op.s0 + fw;
            let clear1 = op.s1 - fw;
            let leaf_width = (clear1 - clear0) / leaves as f64;
            for i in 0..leaves {
                let ls0 = clear0 + leaf_width * i as f64 + 0.01;
                let ls1 = clear0 + leaf_width * (i + 1) as f64 - 0.01;
                let hinge_s = if i == 0 { clear0 } else { clear1 };
                let leaf_meta = |g: bool| {
                    let mut m = common.meta.clone().unwrap_or(json!({}));
                    set(&mut m, "aperture", J::String(op.id.clone()));
                    set(&mut m, "leaf", json!(i));
                    set(&mut m, "hinge", json!({ "s": hinge_s, "w": w_leaf }));
                    set(&mut m, "glazed", J::Bool(g));
                    m
                };
                plan.add_box(
                    &basis,
                    PartArgs::new("door-leaf", if glazed { "doorGlass" } else { "door" }, ls0, ls1, op.y0 + 0.02, door_top - 0.01, w_leaf - 0.02, w_leaf + 0.02)
                        .common(&common)
                        .shadow(!glazed)
                        .meta(leaf_meta(glazed)),
                );
                if glazed {
                    let st = 0.09;
                    for (a, b, c, d) in [
                        (ls0, ls0 + st, op.y0 + 0.02, door_top - 0.01),
                        (ls1 - st, ls1, op.y0 + 0.02, door_top - 0.01),
                        (ls0, ls1, op.y0 + 0.02, op.y0 + 0.3),
                        (ls0, ls1, door_top - 0.12, door_top - 0.01),
                    ] {
                        plan.add_box(&basis, PartArgs::new("door-leaf", "door", a, b, c, d, w_leaf - 0.025, w_leaf + 0.025).common(&common).meta(leaf_meta(false)));
                    }
                }
            }
            plan.apertures.push(json!({ "id": op.id, "edge": edge.index, "storey": storey.index, "bay": bay.index, "s0": op.s0, "s1": op.s1 }));
            let canopy = if en.has("canopy") {
                merge(&J::Null, en.at("canopy"))
            } else if elev.has("canopy") {
                merge(&J::Null, elev.at("canopy"))
            } else {
                None
            };
            // merge(null, x): undefined/true -> { ...null } = {}; false/null -> null.
            let canopy = canopy.map(|c| if c.is_null() { json!({}) } else { c });
            if let Some(cn) = canopy {
                let c = spread(Some(&json!({ "projection": 1.5, "thickness": 0.12, "overhang": 0.6, "rods": true })), Some(&cn));
                let projection = c.num("projection").min(FITTING_PROJECTION_ALLOWANCE);
                let cy = match c.f("height") {
                    Some(h) => storey.y0 + h,
                    None => op.y1 + 0.15,
                };
                let cs0 = (op.s0 - c.num("overhang")).max(0.1);
                let cs1 = (edge.length - 0.1).min(op.s1 + c.num("overhang"));
                let th = c.num("thickness");
                let slot = c.s("slot").filter(|s| !s.is_empty()).unwrap_or("canopy").to_string();
                plan.add_box(&basis, PartArgs::new("canopy", &slot, cs0, cs1, cy, cy + th, -0.02, projection).common(&common));
                if truthy(c.at("rods")) {
                    for s in [cs0 + 0.15, cs1 - 0.15] {
                        let rise = 0.9f64.min(projection * 0.7);
                        plan.add_box(&basis, PartArgs::new("canopy-rod", "metal", s - 0.02, s + 0.02, cy + th, cy + th + rise, projection - 0.08, projection - 0.04).common(&common));
                        plan.add_box(
                            &basis,
                            PartArgs::new("canopy-rod", "metal", s - 0.02, s + 0.02, cy + th + rise - 0.02, cy + th + rise + 0.02, 0.0, projection - 0.04).common(&common),
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

fn layout_edge(plan: &mut Plan, ctx: &Ctx, edge: &Edge, spec: &J, defaults: &J) {
    let elev = resolve_elevation(spec, edge);
    let basis = edge.basis;
    let BayLayout { bays, scale, end_pier } = layout_bays(edge.length, &elev, defaults);
    if scale < 1.0 - 1e-6 {
        plan.warnings.push(format!("edge {}: bay widths scaled by {:.3} to fit {:.2} m", edge.index, scale, edge.length));
    }
    let storeys = plan.storeys.clone();
    let mut record = EdgeRecord {
        index: edge.index,
        role: edge.role,
        party: edge.party,
        length: edge.length,
        bays: bays.iter().map(|b| BayRecord { index: b.index, s0: b.s0, s1: b.s1, width: b.width }).collect(),
        openings: vec![],
    };
    let mut openings: Vec<Opening> = vec![];
    for storey in &storeys {
        for bay in &bays {
            let mut type_ = window_type_for(&elev, bay, storey.index).filter(|t| !t.is_empty()).unwrap_or_else(|| "blind".into());
            if edge.party {
                let top = edge.party_top.unwrap_or(f64::INFINITY);
                if !(storey.y0 >= top + 0.3) {
                    type_ = "blind".into();
                }
            }
            if storey.index > 0 && (type_ == "storefront" || type_ == "entrance" || type_ == "roller") {
                type_ = "punched".into();
            }
            if type_ == "blind" {
                continue;
            }
            let Some(mut op) = opening_for(&type_, bay, storey, &elev, defaults, ctx) else { continue };
            let n = plan.next_id();
            op.id = format!("opening:{}:{}:{}:{}", edge.index, storey.index, bay.index, n);
            let (os0, os1) = (op.s0, op.s1);
            openings.push(op);
            if type_ == "entrance" {
                let sf = merge(&defaults["storefront"], elev.at("storefrontSpec")).unwrap_or(json!({}));
                let head = ctx.storefront_head(storey, &elev, defaults);
                let pier = sf.num("pier");
                for (a, b) in [(bay.s0 + pier, os0 - 0.12), (os1 + 0.12, bay.s1 - pier)] {
                    if b - a < 0.5 {
                        continue;
                    }
                    let n = plan.next_id();
                    openings.push(Opening {
                        type_: "storefront".into(),
                        id: format!("opening:{}:{}:{}:{}", edge.index, storey.index, bay.index, n),
                        s0: a,
                        s1: b,
                        y0: storey.y0 + bay.spec.f("stallRiser").unwrap_or(sf.num("stallRiser")),
                        y1: head,
                        door_top: 0.0,
                        spec: sf.clone(),
                        storey_overrides: json!({}),
                        below_slot: Some("bulkhead"),
                        storey: storey.index,
                        bay: bay.index,
                    });
                }
            }
        }
    }
    // Stable sort by storey, then s0 (Array.prototype.sort is stable).
    openings.sort_by(|a, b| a.storey.cmp(&b.storey).then(a.s0.partial_cmp(&b.s0).unwrap_or(std::cmp::Ordering::Equal)));
    let mut merged: Vec<Opening> = vec![];
    for op in openings {
        if let Some(prev) = merged.last_mut() {
            if prev.type_ == "ribbon" && op.type_ == "ribbon" && prev.storey == op.storey && (prev.s1 - op.s0).abs() < 1e-6 && (prev.y0 - op.y0).abs() < 1e-6 {
                prev.s1 = op.s1;
                continue;
            }
        }
        merged.push(op);
    }

    let wall_slot = elev.s("wallSlot").filter(|s| !s.is_empty()).unwrap_or("wall").to_string();
    let wt = ctx.wall_thickness;
    if ctx.floor_level > 1e-4 {
        let slot = elev.s("plinthSlot").filter(|s| !s.is_empty()).unwrap_or("base").to_string();
        plan.add_box(&basis, PartArgs::new("plinth", &slot, 0.0, edge.length, 0.0, ctx.floor_level, -wt, 0.0).edge(edge.index));
    }
    for storey in &storeys {
        let slot_for = if storey.index == 0 && elev.s("groundWallSlot").map(|s| !s.is_empty()).unwrap_or(false) {
            elev.s("groundWallSlot").unwrap().to_string()
        } else {
            wall_slot.clone()
        };
        let mut cursor = 0.0f64;
        let band = |plan: &mut Plan, s0: f64, s1: f64, y0: f64, y1: f64, slot: &str, kind: &str, bay: Option<i64>| {
            plan.add_box(&basis, PartArgs::new(kind, slot, s0, s1, y0, y1, -wt, 0.0).edge(edge.index).storey(storey.index as i64).bay_opt(bay));
        };
        for op in merged.iter().filter(|o| o.storey == storey.index) {
            if op.s0 > cursor + 1e-4 {
                band(plan, cursor, op.s0, storey.y0, storey.y1, &slot_for, "wall", Some(op.bay as i64));
            }
            if op.y0 > storey.y0 + 1e-4 {
                let (slot, kind) = match op.below_slot {
                    Some("spandrel") => ("spandrel".to_string(), "spandrel"),
                    Some("bulkhead") => ("bulkhead".to_string(), "bulkhead"),
                    Some(other) => (other.to_string(), "wall"),
                    None => (slot_for.clone(), "wall"),
                };
                band(plan, op.s0, op.s1, storey.y0, op.y0, &slot, kind, Some(op.bay as i64));
            }
            if op.y1 < storey.y1 - 1e-4 {
                band(plan, op.s0, op.s1, op.y1, storey.y1, &slot_for, "wall", Some(op.bay as i64));
            }
            cursor = cursor.max(op.s1);
            record.openings.push(OpeningRecord {
                id: op.id.clone(),
                type_: op.type_.clone(),
                storey: storey.index,
                bay: op.bay,
                s0: op.s0,
                s1: op.s1,
                y0: op.y0 + ctx.base_y,
                y1: op.y1 + ctx.base_y,
            });
        }
        if cursor < edge.length - 1e-4 {
            band(plan, cursor, edge.length, storey.y0, storey.y1, &slot_for, "wall", None);
        }
    }
    for op in &merged {
        let storey = &storeys[op.storey];
        let bay = &bays[op.bay];
        emit_opening(plan, ctx, edge, op, storey, bay, &elev, defaults);
    }

    // Bands and vertical articulation.
    let allowance = edge.allowance;
    let ext_start = if edge.convex_start { 1.0 } else { 0.0 };
    let ext_end = if edge.convex_end { 1.0 } else { 0.0 };
    let ground_ops: Vec<&Opening> = merged.iter().filter(|o| o.storey == 0).collect();
    let wall_top = ctx.wall_top;
    let cornice = if elev.has("cornice") {
        merge(&J::Null, elev.at("cornice")).map(|c| if c.is_null() { json!({}) } else { c })
    } else if truthy(spec.at("cornice")) {
        Some(spread(spec.at("cornice"), None))
    } else {
        None
    };
    let cornice_height = cornice.as_ref().map(|c| c.f("height").unwrap_or(0.5)).unwrap_or(0.0);

    if elev.at("base") != Some(&J::Bool(false)) && (truthy(elev.at("base")) || truthy(spec.at("base"))) {
        let elev_base = if elev.at("base").map(|v| v.is_object()).unwrap_or(false) { elev.at("base") } else { None };
        let base = spread(Some(&spread(Some(&json!({ "height": 0.7, "projection": 0.04 })), spec.get_nn("base"))), elev_base);
        let mut h = base.num("height");
        for op in &ground_ops {
            if matches!(op.type_.as_str(), "entrance" | "roller" | "storefront") {
                continue;
            }
            h = h.min(op.y0 - 0.02);
        }
        if h > 0.08 {
            let p = base.num("projection").min(allowance);
            let slot = base.s("slot").filter(|s| !s.is_empty()).unwrap_or("base").to_string();
            plan.add_box(&basis, PartArgs::new("base", &slot, -p * ext_start, edge.length + p * ext_end, 0.0, h, -0.01, p).edge(edge.index));
        }
    }
    let courses = elev.arr("stringCourses").or(spec.arr("stringCourses")).cloned().unwrap_or_default();
    for course in &courses {
        let c = spread(Some(&json!({ "height": 0.15, "projection": 0.08, "profile": "flat" })), Some(course));
        let at = c.f("atStorey").map(|v| v as usize).unwrap_or(1);
        let Some(storey) = storeys.get(at) else { continue };
        if storey.index == 0 {
            continue;
        }
        let p = c.num("projection").min(allowance);
        let offset = c.f("offset").unwrap_or(-c.num("height") / 2.0);
        let slot = c.s("slot").filter(|s| !s.is_empty()).unwrap_or("trim").to_string();
        let profile = trim_profile_d(c.s("profile").unwrap_or("flat"), p, c.num("height"), storey.y0 + offset);
        plan.add_profile(&basis, PartArgs::profile("string-course", &slot, -p * ext_start, edge.length + p * ext_end, profile).edge(edge.index));
    }
    if let Some(cn) = &cornice {
        let c = spread(Some(&json!({ "height": 0.5, "projection": 0.3, "profile": "stepped" })), Some(cn));
        let p = c.num("projection").min(allowance);
        let slot = c.s("slot").filter(|s| !s.is_empty()).unwrap_or("trim").to_string();
        let profile = trim_profile_d(c.s("profile").unwrap_or("stepped"), p, c.num("height"), wall_top - c.num("height"));
        plan.add_profile(&basis, PartArgs::profile("cornice", &slot, -p * ext_start, edge.length + p * ext_end, profile).edge(edge.index));
    }
    if let (Some(pp), true) = (&ctx.parapet, elev.at("parapet") != Some(&J::Bool(false))) {
        let slot = elev.s("parapetSlot").filter(|s| !s.is_empty()).or(pp.s("slot").filter(|s| !s.is_empty())).unwrap_or("parapet").to_string();
        let ph = pp.num("height");
        let pt = pp.num("thickness");
        plan.add_box(&basis, PartArgs::new("parapet", &slot, 0.0, edge.length, wall_top, wall_top + ph, -pt, 0.0).edge(edge.index));
        if let Some(cp) = &ctx.coping {
            let p = cp.num("overhang").min(allowance);
            let slot = cp.s("slot").filter(|s| !s.is_empty()).unwrap_or("coping").to_string();
            let profile = trim_profile(cp.s("profile").filter(|s| !s.is_empty()).unwrap_or("flat"), p, cp.num("height"), wall_top + ph, -pt - p);
            plan.add_profile(&basis, PartArgs::profile("coping", &slot, -p * ext_start, edge.length + p * ext_end, profile).edge(edge.index));
        }
    }
    if truthy(elev.at("pilasters")) {
        let pl_over = if elev.at("pilasters").map(|v| v.is_object()).unwrap_or(false) { elev.at("pilasters") } else { None };
        let pl = spread(Some(&json!({ "width": 0.5, "projection": 0.12, "ends": true })), pl_over);
        let p = pl.num("projection").min(allowance);
        let mut positions: Vec<f64> = bays.iter().skip(1).map(|b| b.s0).collect();
        if truthy(pl.at("ends")) {
            positions.push(end_pier);
            positions.push(edge.length - end_pier);
        }
        let slot = pl.s("slot").filter(|s| !s.is_empty()).unwrap_or("trim").to_string();
        let from = pl.f("from").unwrap_or(0.0);
        for s in positions {
            plan.add_box(&basis, PartArgs::new("pilaster", &slot, s - pl.num("width") / 2.0, s + pl.num("width") / 2.0, from, wall_top - cornice_height + 0.02, -0.01, p).edge(edge.index));
        }
    }
    if truthy(elev.at("cornerReturns")) || truthy(spec.at("cornerReturns")) {
        let elev_cr = if elev.at("cornerReturns").map(|v| v.is_object()).unwrap_or(false) { elev.at("cornerReturns") } else { None };
        let cr = spread(Some(&spread(Some(&json!({ "width": 0.6, "projection": 0.08 })), spec.get_nn("cornerReturns"))), elev_cr);
        let p = cr.num("projection").min(allowance);
        let slot = cr.s("slot").filter(|s| !s.is_empty()).unwrap_or("trim").to_string();
        if edge.convex_start {
            plan.add_box(&basis, PartArgs::new("corner-return", &slot, -p, cr.num("width"), 0.0, wall_top - cornice_height + 0.02, -0.01, p).edge(edge.index));
        }
        if edge.convex_end {
            plan.add_box(
                &basis,
                PartArgs::new("corner-return", &slot, edge.length - cr.num("width"), edge.length + p, 0.0, wall_top - cornice_height + 0.02, -0.01, p).edge(edge.index),
            );
        }
    }
    if truthy(elev.at("signband")) {
        let sb = spread(Some(&json!({ "height": 0.8, "projection": 0.08, "offset": 0.1, "span": "storefront" })), elev.at("signband"));
        let p = sb.num("projection").min(allowance);
        let ground = &storeys[0];
        let y1 = ground.y1 - sb.num("offset");
        let y0 = y1 - sb.num("height");
        let mut s0 = end_pier;
        let mut s1 = edge.length - end_pier;
        match sb.at("span") {
            Some(J::Array(a)) if a.len() >= 2 => {
                s0 = to_number(Some(&a[0]));
                s1 = to_number(Some(&a[1]));
            }
            Some(J::String(s)) if s == "storefront" && !ground_ops.is_empty() => {
                s0 = ground_ops.iter().map(|o| o.s0).fold(f64::INFINITY, f64::min) - 0.15;
                s1 = ground_ops.iter().map(|o| o.s1).fold(f64::NEG_INFINITY, f64::max) + 0.15;
            }
            _ => {}
        }
        let slot = sb.s("slot").filter(|s| !s.is_empty()).unwrap_or("signage").to_string();
        let text = sb.get_nn("text").cloned().unwrap_or(J::Null);
        let idx = plan.add_box(&basis, PartArgs::new("signband", &slot, s0, s1, y0, y1, -0.01, p).edge(edge.index).meta(json!({ "text": text })));
        if idx.is_some() && truthy(sb.at("text")) {
            plan.signs.push(json!({ "kind": "fascia", "text": text, "edge": edge.index }));
        }
    }
    for blade in elev.arr("bladeSigns").cloned().unwrap_or_default() {
        let b = spread(Some(&json!({ "at": edge.length / 2.0, "height": 1.2, "projection": 0.9, "thickness": 0.08, "y": storeys[0].y1 + 0.6 })), Some(&blade));
        let p = b.num("projection").min(FITTING_PROJECTION_ALLOWANCE);
        let (at, th, y, h) = (b.num("at"), b.num("thickness"), b.num("y"), b.num("height"));
        let slot = b.s("slot").filter(|s| !s.is_empty()).unwrap_or("signage").to_string();
        let text = b.get_nn("text").cloned().unwrap_or(J::Null);
        let idx = plan.add_box(&basis, PartArgs::new("blade-sign", &slot, at - th / 2.0, at + th / 2.0, y, y + h, 0.12, p).edge(edge.index).meta(json!({ "text": text })));
        plan.add_box(&basis, PartArgs::new("blade-bracket", "metal", at - 0.02, at + 0.02, y + h - 0.04, y + h, 0.0, p).edge(edge.index));
        plan.add_box(&basis, PartArgs::new("blade-bracket", "metal", at - 0.02, at + 0.02, y, y + 0.04, 0.0, p).edge(edge.index));
        if idx.is_some() && truthy(b.at("text")) {
            plan.signs.push(json!({ "kind": "blade", "text": text, "edge": edge.index }));
        }
    }
    if truthy(elev.at("awnings")) {
        let aw_over = if elev.at("awnings").map(|v| v.is_object()).unwrap_or(false) { elev.at("awnings") } else { None };
        let aw = spread(Some(&json!({ "projection": 1.4, "drop": 0.5, "valance": 0.25 })), aw_over);
        let p = aw.num("projection").min(FITTING_PROJECTION_ALLOWANCE);
        let targets: Vec<&Opening> = match aw.arr("bays") {
            Some(list) => {
                let ids: Vec<f64> = list.iter().map(|v| to_number(Some(v))).collect();
                ground_ops.iter().copied().filter(|o| ids.contains(&(o.bay as f64))).collect()
            }
            None => ground_ops.iter().copied().filter(|o| o.type_ == "storefront" || o.type_ == "entrance").collect(),
        };
        let slot = aw.s("slot").filter(|s| !s.is_empty()).unwrap_or("awning").to_string();
        for op in targets {
            let y_top = op.y1 + 0.05;
            let profile = vec![[0.0, y_top], [p, y_top - aw.num("drop")], [p, y_top - aw.num("drop") - aw.num("valance")], [0.02, y_top - 0.06]];
            plan.add_profile(&basis, PartArgs::profile("awning", &slot, op.s0 - 0.1, op.s1 + 0.1, profile).edge(edge.index).shadow(true));
        }
    }
    if truthy(elev.at("fireEscape")) {
        let default_storeys: Vec<J> = storeys.iter().skip(1).map(|s| json!(s.index)).collect();
        let fe = spread(Some(&json!({ "storeys": default_storeys, "at": edge.length * 0.6, "width": 2.4, "projection": 1.0 })), elev.at("fireEscape"));
        let p = fe.num("projection").min(FITTING_PROJECTION_ALLOWANCE);
        let s0 = (fe.num("at") - fe.num("width") / 2.0).max(0.2);
        let s1 = (edge.length - 0.2).min(fe.num("at") + fe.num("width") / 2.0);
        let mut previous_y: Option<f64> = None;
        for index in fe.arr("storeys").cloned().unwrap_or_default() {
            let Some(storey) = storeys.get(to_number(Some(&index)) as usize) else { continue };
            let y = storey.y0 + 0.05;
            plan.add_box(&basis, PartArgs::new("fire-escape", "metal", s0, s1, y, y + 0.08, 0.05, p).edge(edge.index));
            for (a, b, c, d) in [(s0, s0 + 0.04, 0.05, p), (s1 - 0.04, s1, 0.05, p), (s0, s1, p - 0.04, p)] {
                plan.add_box(&basis, PartArgs::new("fire-escape-rail", "metal", a, b, y + 0.95, y + 1.0, c, d).edge(edge.index));
                plan.add_box(&basis, PartArgs::new("fire-escape-rail", "metal", a, b, y + 0.5, y + 0.54, c, d).edge(edge.index));
            }
            for (a, c) in [(s0, 0.05), (s1 - 0.04, 0.05), (s0, p - 0.04), (s1 - 0.04, p - 0.04)] {
                plan.add_box(&basis, PartArgs::new("fire-escape-rail", "metal", a, a + 0.04, y + 0.08, y + 1.0, c, c + 0.04).edge(edge.index));
            }
            if let Some(py) = previous_y {
                let ls = s0 + 0.3;
                for off in [0.0, 0.5] {
                    plan.add_box(&basis, PartArgs::new("fire-escape-ladder", "metal", ls + off, ls + off + 0.04, py + 0.1, y, p * 0.5, p * 0.5 + 0.04).edge(edge.index));
                }
                let mut ry = py + 0.4;
                while ry < y - 0.2 {
                    plan.add_box(&basis, PartArgs::new("fire-escape-ladder", "metal", ls, ls + 0.54, ry, ry + 0.03, p * 0.5, p * 0.5 + 0.03).edge(edge.index));
                    ry += 0.3;
                }
            }
            previous_y = Some(y);
        }
    }
    if truthy(elev.at("downpipes")) {
        let positions: Vec<f64> = match elev.at("downpipes") {
            Some(J::Array(a)) => a.iter().map(|d| if d.is_number() { to_number(Some(d)) } else { to_number(d.at("at")) }).collect(),
            Some(J::String(s)) if s == "corners" => vec![0.25, edge.length - 0.25],
            Some(J::Bool(true)) => vec![0.25, edge.length - 0.25],
            _ => vec![],
        };
        let p = 0.16f64.min(allowance);
        for s in positions {
            plan.add_box(&basis, PartArgs::new("downpipe", "pipe", s - 0.05, s + 0.05, 0.25, wall_top - cornice_height - 0.05, (p - 0.1).max(0.0), p).edge(edge.index).shadow(false));
        }
    }
    plan.edges.push(record);
}

fn layout_roof(plan: &mut Plan, ctx: &Ctx, frame: &Frame, spec: &J) {
    let roof = spread(Some(&json!({ "slab": 0.3 })), spec.get_nn("roof"));
    let wall_top = ctx.wall_top;
    let slot = roof.s("slot").filter(|s| !s.is_empty()).unwrap_or("roof").to_string();
    let mut a = PartArgs { kind: "roof-slab".into(), slot, points: frame.polygon.clone(), y0: wall_top - roof.num("slab"), y1: wall_top, ..Default::default() };
    a.side_faces = Some(roof.at("sideFaces") != Some(&J::Bool(false)));
    a.receive_shadow = Some(true);
    plan.add_prism(a);
    let Some(deck) = roof.get_nn("deck").filter(|d| truthy(Some(d))).cloned() else { return };
    let mut random = Rng::new(&format!("{}|roof", ctx.seed));
    let inset = ctx.parapet.as_ref().map(|p| p.num("thickness")).unwrap_or(0.0) + deck.f("inset").unwrap_or(0.8);
    let fits = |px: f64, pz: f64, sx: f64, sz: f64| {
        for (dx, dz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let w = frame.to_world(px + (dx * sx) / 2.0, pz + (dz * sz) / 2.0);
            if frame.signed_distance(w.x, w.z) > -inset {
                return false;
            }
        }
        true
    };
    let mut placed: Vec<(f64, f64, f64, f64)> = vec![];
    let fb = frame.basis;
    let mut place = |plan: &mut Plan, random: &mut Rng, kind: &str, size: [f64; 3], slot: &str, tries: usize, fixed: Option<(f64, f64)>| -> Option<usize> {
        let [sx, sy, sz] = size;
        for _ in 0..tries {
            let px = match fixed {
                Some(f) => f.0,
                None => (random.next() - 0.5) * (frame.width - 2.0 * inset),
            };
            let pz = match fixed {
                Some(f) => f.1,
                None => (random.next() - 0.5) * (frame.depth - 2.0 * inset),
            };
            let overlaps = placed.iter().any(|q| (q.0 - px).abs() < (q.2 + sx) / 2.0 + 0.6 && (q.1 - pz).abs() < (q.3 + sz) / 2.0 + 0.6);
            if !fits(px, pz, sx, sz) || overlaps {
                if fixed.is_some() {
                    return None;
                }
                continue;
            }
            placed.push((px, pz, sx, sz));
            return plan.add_box(&fb, PartArgs::new(kind, slot, px - sx / 2.0, px + sx / 2.0, wall_top, wall_top + sy, pz - sz / 2.0, pz + sz / 2.0).meta(json!({ "roof": true })));
        }
        None
    };
    let explicit = deck.arr("units").cloned().unwrap_or_default();
    for unit in &explicit {
        let size = unit.arr("size").map(|a| [to_number(a.first()), to_number(a.get(1)), to_number(a.get(2))]).unwrap_or([1.8, 1.2, 1.0]);
        let at = unit.arr("at").map(|a| (to_number(a.first()), to_number(a.get(1))));
        let kind = unit.s("kind").filter(|s| !s.is_empty()).unwrap_or("roof-unit").to_string();
        let slot = unit.s("slot").filter(|s| !s.is_empty()).unwrap_or("plant").to_string();
        place(plan, &mut random, &kind, size, &slot, 1, at);
    }
    let unit_count = if let Some(n) = deck.f("units") {
        n
    } else if deck.arr("units").is_some() {
        0.0
    } else {
        deck.f("autoUnits").unwrap_or(0.0)
    };
    let mut i = 0.0;
    while i < unit_count {
        let size = [1.6 + random.next() * 0.8, 1.0 + random.next() * 0.5, 0.9 + random.next() * 0.5];
        if let Some(k) = place(plan, &mut random, "roof-unit", size, "plant", 40, None) {
            let p = plan.parts[k].clone();
            plan.add_box(&fb, PartArgs::new("roof-unit", "plant", p.s0 + 0.1, p.s1 - 0.1, wall_top - 0.01, wall_top + 0.15, p.w0 + 0.1, p.w1 - 0.1));
        }
        i += 1.0;
    }
    if truthy(deck.at("hatch")) {
        let at = deck.get_nn("hatch").arr("at").map(|a| (to_number(a.first()), to_number(a.get(1))));
        place(plan, &mut random, "roof-hatch", [1.1, 1.3, 1.0], "plant", 40, at);
    }
    if truthy(deck.at("ducts")) {
        let count = deck.f("ducts").unwrap_or(1.0);
        let mut i = 0.0;
        while i < count {
            let size = [0.5 + random.next() * 0.2, 0.5, 2.5 + random.next() * 3.0];
            place(plan, &mut random, "roof-duct", size, "plant", 40, None);
            i += 1.0;
        }
    }
    let antennas = deck.f("antennas").unwrap_or(if truthy(deck.at("antennas")) { 1.0 } else { 0.0 });
    let mut i = 0.0;
    while i < antennas {
        let size = [0.06, 2.4 + random.next() * 1.5, 0.06];
        place(plan, &mut random, "antenna", size, "metal", 40, None);
        i += 1.0;
    }
}

/// `planElevation(spec, frame, { baseY })`.
pub fn plan_elevation(spec: &J, frame: &Frame, base_y: f64) -> Plan {
    let defaults = elevation_defaults();
    let height = num_or(spec.at("height"), frame.height.unwrap_or(12.0));
    let height = if height == 0.0 || height.is_nan() { 12.0 } else { height };
    let parapet = merge(&defaults["parapet"], spec.at("parapet"));
    let coping = if parapet.is_some() { merge(&defaults["coping"], spec.at("coping")) } else { None };
    let roof_above = parapet.as_ref().map(|p| p.f("height").unwrap_or(0.0)).unwrap_or(0.0) + coping.as_ref().map(|c| c.f("height").unwrap_or(0.0)).unwrap_or(0.0);
    let floor_level = num_or(spec.at("floorLevel"), 0.0).max(0.0);
    let storeys = resolve_storeys(spec, height, roof_above, floor_level);
    let wall_top = storeys.last().map(|s| s.y1).unwrap_or(0.0);
    let roof_top = wall_top + roof_above;
    let wall_thickness = spec.f("wallThickness").unwrap_or(defaults.num("wallThickness"));
    let mut materials: BTreeMap<String, String> = DEFAULT_MATERIAL_SLOTS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    if let Some(m) = spec.obj("materials") {
        for (k, v) in m {
            match v.as_str() {
                Some(s) => {
                    materials.insert(k.clone(), s.to_string());
                }
                // `{ ...defaults, slot: null }`: the slot names nothing.
                None => {
                    materials.remove(k);
                }
            }
        }
    }
    let seed = match spec.get_nn("seed").or(spec.get_nn("id")) {
        Some(J::String(s)) => s.clone(),
        Some(J::Number(n)) => crate::rng::js_num(n.as_f64().unwrap_or(0.0)),
        _ => "authored".into(),
    };
    let seed_hash = match spec.get_nn("seed").or(spec.get_nn("id")) {
        Some(J::Number(n)) => crate::rng::hash_seed_num(n.as_f64().unwrap_or(0.0)),
        _ => hash_seed(&seed),
    };
    let mut plan = Plan::empty();
    plan.id = spec.s("id").unwrap_or("").to_string();
    plan.seed = seed.clone();
    plan.seed_hash = seed_hash;
    plan.base_y = base_y;
    plan.height = height;
    plan.storeys = storeys;
    plan.floor_level = floor_level;
    plan.envelope = Envelope {
        wall_top,
        roof_top,
        height,
        height_delta: roof_top - height,
        roof_plant_allowance: spec.get_nn("roof").f("plantAllowance").unwrap_or(3.0),
        wall_thickness,
        floor_level,
    };
    plan.materials = materials;
    if (roof_top - height).abs() > 0.05 {
        plan.warnings.push(format!("storeys + parapet total {:.2} m against source height {:.2} m", roof_top, height));
    }
    let ctx = Ctx { seed, base_y, wall_top, floor_level, wall_thickness, parapet, coping };
    for edge in &frame.edges {
        layout_edge(&mut plan, &ctx, edge, spec, &defaults);
    }
    layout_roof(&mut plan, &ctx, frame, spec);
    if base_y != 0.0 {
        for part in plan.parts.iter_mut().chain(plan.stoop_parts.iter_mut()) {
            lift_part(part, base_y);
        }
    }
    plan
}

/// Lift a part by `dy` (planElevation's baseY pass).
pub fn lift_part(part: &mut Part, dy: f64) {
    part.y0 += dy;
    part.y1 += dy;
    if let Some(f) = part.meta.f("floorY") {
        set(&mut part.meta, "floorY", jn(f + dy));
    }
    match &mut part.shape {
        Shape::Profile(p) => p.iter_mut().for_each(|q| q[1] += dy),
        Shape::Plate(p) => p.iter_mut().for_each(|q| q[1] += dy),
        _ => {}
    }
}

/// World rotation of a basis' u direction (for reports).
pub fn basis_rotation(basis: &Basis) -> f64 {
    rotation_y_for_direction(P2::new(basis.u[0], basis.u[1]))
}
