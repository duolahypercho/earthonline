//! Rooftop plant for flat-roofed archetype buildings
//! (src/world/buildings/archetypes/roof-plant.js, `sf-archetype-roof-plant-v5`).
//!
//! A mechanical port: same streams (`|roof-plant`, `|roof-plant-v4`,
//! `|roof-plant-v5`), same roll order, same placement tests. Nothing is left out.

use crate::classify::Classified;
use crate::elevation::{Part, PartArgs, Plan, Shape};
use crate::frame::{Frame, P2};
use crate::js::*;
use crate::rng::{Rng, hash_seed};

pub const ROOF_PLANT_VERSION: &str = "sf-archetype-roof-plant-v5";
/// Sub-metre dressing on plant (fan shrouds, hoods); dropped at LOD2.
pub const ROOF_DETAIL_KIND: &str = "roof-plant-detail";
/// Solar rails and module seams; dropped from LOD1.
pub const SOLAR_RACK_KIND: &str = "roof-solar-rack";
const SKIP: [&str; 2] = ["downtown-tower", "industrial-shed"];
const RESIDENTIAL: [&str; 6] = ["bay-window-apartment", "victorian-row", "midcentury-stucco", "stucco-row", "edwardian-flats", "cottage-garage"];
const EDGE_MARGIN_M: f64 = 1.2;
const M2_PER_UNIT: f64 = 170.0;
const MAX_UNITS: f64 = 9.0;
const M2_PER_VENT: f64 = 140.0;
const MAX_VENTS: f64 = 8.0;
const BULKHEAD_MIN_STOREYS: f64 = 4.0;
const SLOPED_ROOF_KINDS: [&str; 5] = ["roof-slope", "roof-gable", "gable", "gable-rake", "gable-infill"];

/// `hasPitchedRoof(plan)`: does the plan's template draw a pitched roof?
pub fn has_pitched_roof(plan: &Plan) -> bool {
    let Some(a) = plan.extra.get("archetype").filter(|a| a.is_object()) else { return false };
    if a.at("pitched") == Some(&J::Bool(true)) {
        return true;
    }
    let roof = match a.at("roof") {
        Some(J::String(s)) => Some(s.as_str()),
        other => other.and_then(|r| r.at("form")).and_then(|f| f.as_str()),
    };
    matches!(roof, Some("gable") | Some("hip") | Some("pitched"))
}

/// `partPlanCorners(part, frame)` (kit/elevation.js).
fn part_plan_corners(part: &Part, frame: &Frame) -> Vec<P2> {
    if let Shape::Prism(points) = &part.shape {
        return points.iter().map(|p| frame.to_plan(p.x, p.z)).collect();
    }
    let b = &part.basis;
    let mut out = Vec::with_capacity(4);
    for s in [part.s0, part.s1] {
        for w in [part.w0, part.w1] {
            out.push(frame.to_plan(b.origin[0] + b.u[0] * s + b.n[0] * w, b.origin[1] + b.u[1] * s + b.n[1] * w));
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x0: f64,
    x1: f64,
    z0: f64,
    z1: f64,
    y1: f64,
}

fn sloped_roof_zones(plan: &Plan, frame: &Frame) -> Vec<Rect> {
    let mut zones = vec![];
    for part in &plan.parts {
        if !SLOPED_ROOF_KINDS.contains(&part.kind.as_str()) {
            continue;
        }
        let corners = part_plan_corners(part, frame);
        if corners.is_empty() {
            continue;
        }
        let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for c in &corners {
            x0 = x0.min(c.x);
            x1 = x1.max(c.x);
            z0 = z0.min(c.z);
            z1 = z1.max(c.z);
        }
        if x0.is_finite() && z0.is_finite() {
            zones.push(Rect { x0, x1, z0, z1, y1: 0.0 });
        }
    }
    zones
}

fn rect_inside(frame: &Frame, x0: f64, x1: f64, z0: f64, z1: f64, margin: f64) -> bool {
    for (px, pz) in [(x0, z0), (x1, z0), (x1, z1), (x0, z1), ((x0 + x1) / 2.0, (z0 + z1) / 2.0)] {
        let w = frame.to_world(px, pz);
        if !(frame.signed_distance(w.x, w.z) < -margin) {
            return false;
        }
    }
    true
}

fn overlaps(placed: &[Rect], x0: f64, x1: f64, z0: f64, z1: f64, gap: f64) -> bool {
    placed.iter().any(|r| x0 < r.x1 + gap && x1 > r.x0 - gap && z0 < r.z1 + gap && z1 > r.z0 - gap)
}

/// Which stream a placement draws from.
#[derive(Clone, Copy)]
enum Stream {
    V3,
    V5,
}

struct Ctx<'a> {
    plan: &'a mut Plan,
    frame: &'a Frame,
    archetype: &'static str,
    roof_y: f64,
    ceiling: f64,
    metal_slot: &'static str,
    blocked: Vec<Rect>,
    placed: Vec<Rect>,
    tri_cap: f64,
    tri_used: f64,
    random: Rng,
    v5: Rng,
}

impl Ctx<'_> {
    fn affords(&self, boxes: f64) -> bool {
        self.tri_used + 12.0 * boxes <= self.tri_cap
    }
    fn meta(&self, part: &str) -> J {
        json!({ "archetype": self.archetype, "part": part, "roof": true, "version": ROOF_PLANT_VERSION })
    }
    fn rng(&mut self, s: Stream) -> f64 {
        match s {
            Stream::V3 => self.random.next(),
            Stream::V5 => self.v5.next(),
        }
    }
    fn find_rect(&mut self, sx: f64, sz: f64, s: Stream, gap: f64) -> Option<Rect> {
        let b = self.frame.plan_bounds.clone();
        for _ in 0..18 {
            let (ax, az) = if self.rng(s) < 0.5 { (sx, sz) } else { (sz, sx) };
            let x0 = b.min_x + EDGE_MARGIN_M + self.rng(s) * ((b.max_x - b.min_x) - ax - 2.0 * EDGE_MARGIN_M).max(0.0);
            let z0 = b.min_z + EDGE_MARGIN_M + self.rng(s) * ((b.max_z - b.min_z) - az - 2.0 * EDGE_MARGIN_M).max(0.0);
            let x1 = x0 + ax;
            let z1 = z0 + az;
            if !rect_inside(self.frame, x0, x1, z0, z1, EDGE_MARGIN_M * 0.5) {
                continue;
            }
            if overlaps(&self.placed, x0, x1, z0, z1, gap) {
                continue;
            }
            if overlaps(&self.blocked, x0, x1, z0, z1, 0.3) {
                continue;
            }
            return Some(Rect { x0, x1, z0, z1, y1: 0.0 });
        }
        None
    }
    #[allow(clippy::too_many_arguments)]
    fn place(&mut self, sx: f64, sz: f64, h: f64, kind: &str, slot: &str, part: &str, s: Stream) -> Option<Rect> {
        let found = self.find_rect(sx, sz, s, 0.7)?;
        let y1 = (self.roof_y + h).min(self.ceiling);
        if !(y1 - self.roof_y > 0.2) || !self.affords(1.0) {
            return None;
        }
        let meta = self.meta(part);
        let basis = self.frame.basis;
        self.plan.add_box(
            &basis,
            PartArgs::new(kind, slot, found.x0, found.x1, self.roof_y - 0.02, y1, found.z0, found.z1).shadow(true).receive(true).meta(meta),
        )?;
        self.tri_used += 12.0;
        let rect = Rect { y1, ..found };
        self.placed.push(rect);
        Some(rect)
    }
    #[allow(clippy::too_many_arguments)]
    fn detail(&mut self, x0: f64, x1: f64, z0: f64, z1: f64, y0: f64, y1: f64, kind: &str, slot: &str, part: &str) {
        let top = y1.min(self.ceiling);
        if !(top - y0 > 0.03) || !(x1 - x0 > 0.03) || !(z1 - z0 > 0.03) || !self.affords(1.0) {
            return;
        }
        let meta = self.meta(part);
        let basis = self.frame.basis;
        let cast = kind != ROOF_DETAIL_KIND && kind != SOLAR_RACK_KIND;
        if self.plan.add_box(&basis, PartArgs::new(kind, slot, x0, x1, y0, top, z0, z1).shadow(cast).receive(true).meta(meta)).is_some() {
            self.tri_used += 12.0;
        }
    }
    fn dress_unit(&mut self, rect: Rect, condenser: bool) {
        let w = rect.x1 - rect.x0;
        let d = rect.z1 - rect.z0;
        let (ry, ms) = (self.roof_y, self.metal_slot);
        self.detail(rect.x0 - 0.08, rect.x1 + 0.08, rect.z0 - 0.08, rect.z1 + 0.08, ry - 0.02, ry + 0.16, "roof-curb", ms, "unit-curb");
        let fan = w.min(d) * if condenser { 0.62 } else { 0.42 };
        let cx = rect.x0 + w * if condenser { 0.5 } else { 0.68 };
        let cz = (rect.z0 + rect.z1) / 2.0;
        self.detail(cx - fan / 2.0, cx + fan / 2.0, cz - fan / 2.0, cz + fan / 2.0, rect.y1 - 0.01, rect.y1 + 0.2, ROOF_DETAIL_KIND, ms, "fan-shroud");
        if !condenser && w > 1.6 {
            self.detail(rect.x0 - 0.32, rect.x0 + 0.02, rect.z0 + d * 0.18, rect.z1 - d * 0.18, ry + 0.2, rect.y1 - 0.12, ROOF_DETAIL_KIND, ms, "return-hood");
        }
    }
    /// A ballasted array of module rows inside a free rect; returns modules drawn.
    fn solar_array(&mut self, rect: Rect) -> f64 {
        let w = rect.x1 - rect.x0;
        let d = rect.z1 - rect.z0;
        let along_x = w >= d;
        let run_len = if along_x { w } else { d };
        let across = if along_x { d } else { w };
        const MODULE_L: f64 = 1.7;
        const ROW_D: f64 = 1.05;
        const PITCH: f64 = 1.65;
        let modules = ((run_len + 0.02) / (MODULE_L + 0.02)).floor();
        let rows = ((across - ROW_D) / PITCH).floor() + 1.0;
        if modules < 2.0 || rows < 2.0 || !self.affords(rows * (3.0 + modules)) {
            return 0.0;
        }
        if self.plan.materials.get("solar").map(|s| s.is_empty()).unwrap_or(true) {
            self.plan.materials.insert("solar".into(), "anodized-aluminium".into());
        }
        let used_len = modules * (MODULE_L + 0.02) - 0.02;
        let start = (run_len - used_len) / 2.0;
        let (ry, ms) = (self.roof_y, self.metal_slot);
        let bx = |ctx: &mut Self, a0: f64, a1: f64, c0: f64, c1: f64, y0: f64, y1: f64, kind: &str, slot: &str, part: &str| {
            if along_x {
                ctx.detail(rect.x0 + a0, rect.x0 + a1, rect.z0 + c0, rect.z0 + c1, y0, y1, kind, slot, part)
            } else {
                ctx.detail(rect.x0 + c0, rect.x0 + c1, rect.z0 + a0, rect.z0 + a1, y0, y1, kind, slot, part)
            }
        };
        let mut r = 0.0;
        while r < rows {
            let c0 = r * PITCH;
            let c1 = c0 + ROW_D;
            bx(self, start, start + used_len, c0, c0 + ROW_D * 0.5, ry + 0.18, ry + 0.24, "roof-solar-step", "solar", "solar-module");
            bx(self, start, start + used_len, c0 + ROW_D * 0.5, c1, ry + 0.28, ry + 0.34, "roof-solar", "solar", "solar-module");
            bx(self, start - 0.04, start + used_len + 0.04, c0 + 0.02, c0 + 0.1, ry - 0.02, ry + 0.18, SOLAR_RACK_KIND, ms, "solar-rail");
            bx(self, start - 0.04, start + used_len + 0.04, c1 - 0.1, c1 - 0.02, ry - 0.02, ry + 0.28, SOLAR_RACK_KIND, ms, "solar-rail");
            let mut m = 1.0;
            while m < modules {
                let a = start + m * (MODULE_L + 0.02) - 0.03;
                bx(self, a, a + 0.04, c0, c1, ry + 0.22, ry + 0.36, SOLAR_RACK_KIND, ms, "solar-seam");
                m += 1.0;
            }
            r += 1.0;
        }
        modules * rows
    }
}

/// The v5 rolls, all taken up front.
struct V5Rolls {
    mid_rise: bool,
    storeys: f64,
    area: f64,
    solar_share: f64,
    solar_roll: f64,
    solar_scale: f64,
    solar_aspect: f64,
    overrun: [f64; 3],
    condensers: Vec<[f64; 3]>,
    fans: Vec<[f64; 3]>,
    flues: Vec<[f64; 3]>,
    extras: Vec<(f64, f64, f64, bool)>,
}

fn roofscape_v5(ctx: &mut Ctx, r: &V5Rolls) {
    let ms = ctx.metal_slot;
    if r.mid_rise && r.storeys >= 5.0 {
        if let Some(o) = ctx.place(r.overrun[0], r.overrun[1], r.overrun[2], "roof-bulkhead", "wall", "lift-overrun", Stream::V5) {
            ctx.detail(o.x0 - 0.1, o.x1 + 0.1, o.z0 - 0.1, o.z1 + 0.1, o.y1 - 0.02, o.y1 + 0.1, "roof-curb", ms, "overrun-cap");
        }
    }
    for &[sx, sz, h] in &r.condensers {
        if let Some(rect) = ctx.place(sx, sz, h, "roof-plant", ms, "split-condenser", Stream::V5) {
            ctx.dress_unit(rect, true);
        }
    }
    for &(sx, sz, h, condenser) in &r.extras {
        if let Some(rect) = ctx.place(sx, sz, h, "roof-plant", ms, "packaged-unit", Stream::V5) {
            ctx.dress_unit(rect, condenser);
        }
    }
    for &[sx, sz, h] in &r.fans {
        ctx.place(sx, sz, h, "roof-vent", ms, "exhaust-fan", Stream::V5);
    }
    for &[sx, sz, h] in &r.flues {
        if let Some(f) = ctx.place(sx, sz, h, "roof-vent", ms, "flue", Stream::V5) {
            ctx.detail(f.x0 - 0.08, f.x1 + 0.08, f.z0 - 0.08, f.z1 + 0.08, f.y1 - 0.02, f.y1 + 0.08, ROOF_DETAIL_KIND, ms, "flue-cap");
        }
    }
    if r.solar_roll < r.solar_share && r.area >= 60.0 {
        let side = (r.area * 0.25).sqrt() * r.solar_scale;
        for k in [1.0, 0.75, 0.55, 0.4] {
            let sx = 24f64.min(side * k * r.solar_aspect.sqrt());
            let sz = 16f64.min(side * k / r.solar_aspect.sqrt());
            if sx < 3.6 || sz < 2.8 {
                break;
            }
            let Some(rect) = ctx.find_rect(sx, sz, Stream::V5, 1.0) else { continue };
            if ctx.solar_array(rect) > 0.0 {
                let y1 = ctx.roof_y + 0.36;
                ctx.placed.push(Rect { y1, ..rect });
                break;
            }
        }
    }
}

/// `addArchetypeRoofPlant(plan, frame, classified)`.
pub fn add(plan: &mut Plan, frame: &Frame, c: &Classified) {
    if SKIP.contains(&c.archetype) {
        return;
    }
    if has_pitched_roof(plan) || plan.extra.get("archetype").at("roofPlant") == Some(&J::Bool(false)) {
        return;
    }
    let base_y = if plan.base_y.is_finite() { plan.base_y } else { 0.0 };
    let roof_y = plan.envelope.wall_top + base_y;
    let ceiling = plan.envelope.roof_top + plan.envelope.roof_plant_allowance + base_y - 0.05;
    let area = if frame.area.is_nan() { 0.0 } else { frame.area };
    let detail = crate::typology::detail_enabled();
    let min_area = if detail { 26.0 } else { 40.0 };
    if !(area >= min_area) || !(ceiling > roof_y + 0.5) {
        return;
    }
    let random = Rng::from_num(hash_seed(&format!("{}|roof-plant", c.seed)) as f64);
    // `new Set(parts.map((p) => materials[p.slot])).has(materials.plant)`.
    let plant = plan.materials.get("plant");
    let used = plan.parts.iter().any(|p| plan.materials.get(&p.slot) == plant);
    let metal_slot = if used { "plant" } else { "trim" };
    let residential = RESIDENTIAL.contains(&c.archetype);
    let blocked = sloped_roof_zones(plan, frame);
    let tri_cap = plan.extra.get("archetype").f("roofPlantTriangles").unwrap_or(f64::INFINITY);

    let mut v5 = Rng::from_num(hash_seed(&format!("{}|roof-plant-v5", c.seed)) as f64);
    let storeys = c.storeys;
    let mid_rise = residential && (storeys >= 5.0 || area >= 450.0);
    let solar_share = if residential { if storeys >= 5.0 { 0.1 } else { 0.14 } } else { 0.16 };
    let solar_roll = v5.next();
    let solar_scale = 0.55 + v5.next() * 0.45;
    let solar_aspect = 1.3 + v5.next() * 0.9;
    let overrun = [2.2 + v5.next() * 0.7, 2.5 + v5.next() * 0.9, 2.5 + v5.next() * 0.4];
    let condenser_count = if mid_rise { js_round(area / 220.0).clamp(1.0, 6.0) as usize } else { 0 };
    let mut condensers = vec![];
    for _ in 0..condenser_count {
        condensers.push([0.85 + v5.next() * 0.35, 0.4 + v5.next() * 0.2, 0.75 + v5.next() * 0.25]);
    }
    let fan_count = if mid_rise { js_round(area / 160.0).clamp(1.0, 6.0) as usize } else { 0 };
    let mut fans = vec![];
    for _ in 0..fan_count {
        let f = 0.5 + v5.next() * 0.25;
        fans.push([f, f, 0.4 + v5.next() * 0.25]);
    }
    let low_rise = residential && !mid_rise;
    let flue_count = if low_rise { 1 + usize::from(v5.next() < 0.45) } else { 0 };
    let mut flues = vec![];
    for _ in 0..flue_count {
        let t = 0.26 + v5.next() * 0.1;
        flues.push([t, t, 1.0 + v5.next() * 0.5]);
    }
    let low_condenser = low_rise && area >= 150.0 && v5.next() < 0.4;
    if low_condenser {
        condensers.push([0.85 + v5.next() * 0.3, 0.4 + v5.next() * 0.15, 0.75 + v5.next() * 0.2]);
    }
    let extra_units = if residential { 0 } else { (js_round(area / M2_PER_UNIT) - MAX_UNITS).clamp(0.0, 8.0) as usize };
    let mut extras = vec![];
    for _ in 0..extra_units {
        extras.push((1.3 + v5.next() * 1.4, 1.0 + v5.next() * 0.9, 0.9 + v5.next() * 0.7, v5.next() < 0.33));
    }
    let rolls = V5Rolls { mid_rise, storeys, area, solar_share, solar_roll, solar_scale, solar_aspect, overrun, condensers, fans, flues, extras };

    let mut ctx = Ctx { plan, frame, archetype: c.archetype, roof_y, ceiling, metal_slot, blocked, placed: vec![], tri_cap, tri_used: 0.0, random, v5 };
    let ms = metal_slot;

    let wants_bulkhead = c.storeys >= BULKHEAD_MIN_STOREYS;
    let bulkhead_roll = [2.2 + ctx.random.next() * 0.8, 2.8 + ctx.random.next() * 1.2, 2.3 + ctx.random.next() * 0.5];
    let mut v4 = Rng::from_num(hash_seed(&format!("{}|roof-plant-v4", c.seed)) as f64);
    if wants_bulkhead {
        if let Some(b) = ctx.place(bulkhead_roll[0], bulkhead_roll[1], bulkhead_roll[2], "roof-bulkhead", "wall", "stair-bulkhead", Stream::V3) {
            ctx.detail(b.x0 - 0.1, b.x1 + 0.1, b.z0 - 0.1, b.z1 + 0.1, b.y1 - 0.02, b.y1 + 0.1, "roof-curb", ms, "bulkhead-cap");
        }
    }

    if residential {
        let hatches = if area >= 120.0 { 2 } else { 1 };
        for i in 0..hatches {
            let s = 0.9 + ctx.random.next() * 0.6;
            let sz = s * (1.0 + ctx.random.next() * 0.6);
            let h = 0.45 + ctx.random.next() * 0.25;
            let rect = ctx.place(s, sz, h, "roof-hatch", ms, if i == 0 { "hatch" } else { "skylight" }, Stream::V3);
            if let Some(r) = rect {
                ctx.detail(r.x0 - 0.05, r.x1 + 0.05, r.z0 - 0.05, r.z1 + 0.05, r.y1 - 0.01, r.y1 + 0.06, ROOF_DETAIL_KIND, ms, if i == 0 { "hatch-lid" } else { "skylight-lid" });
            }
        }
        let stacks = js_round(area / 45.0).clamp(2.0, 5.0) as usize;
        for _ in 0..stacks {
            let t = 0.09 + v4.next() * 0.05;
            let h = 0.45 + v4.next() * 0.35;
            ctx.place(t, t, h, "roof-vent", ms, "vent-stack", Stream::V3);
        }
        roofscape_v5(&mut ctx, &rolls);
        if detail {
            roofscape_v6(&mut ctx, c, true, area);
        }
        return;
    }

    let units = js_round(area / M2_PER_UNIT).clamp(1.0, MAX_UNITS) as usize;
    let mut sizes = vec![];
    for _ in 0..units {
        if ctx.random.next() < 0.33 {
            sizes.push([0.8 + ctx.random.next() * 0.4, 0.8 + ctx.random.next() * 0.4, 0.7 + ctx.random.next() * 0.3]);
        } else {
            sizes.push([1.3 + ctx.random.next() * 1.4, 1.0 + ctx.random.next() * 0.9, 0.9 + ctx.random.next() * 0.7]);
        }
    }
    let ducts = if units >= 3 { if units >= 6 { 2 } else { 1 } } else { 0 };
    let mut duct_rolls = vec![];
    for _ in 0..ducts {
        duct_rolls.push([3.0 + ctx.random.next() * 6.0, 0.5 + ctx.random.next() * 0.3, 0.45 + ctx.random.next() * 0.2]);
    }
    let vents = MAX_VENTS.min(js_round(area / M2_PER_VENT)) as usize;
    let mut vent_rolls = vec![];
    for _ in 0..vents {
        let v = 0.45 + ctx.random.next() * 0.3;
        vent_rolls.push([v, v * (0.9 + ctx.random.next() * 0.3), 0.35 + ctx.random.next() * 0.3]);
    }
    for [sx, sz, h] in sizes {
        if let Some(rect) = ctx.place(sx, sz, h, "roof-plant", ms, "packaged-unit", Stream::V3) {
            ctx.dress_unit(rect, sx.max(sz) < 1.25);
        }
    }
    for [len, w, h] in duct_rolls {
        ctx.place(len, w, h, "roof-duct", ms, "duct-run", Stream::V3);
    }
    for [sx, sz, h] in vent_rolls {
        ctx.place(sx, sz, h, "roof-vent", ms, "exhaust-fan", Stream::V3);
    }
    roofscape_v5(&mut ctx, &rolls);
    if detail {
        roofscape_v6(&mut ctx, c, false, area);
    }
}

/// High-detail lived-in SF flat-rooftop pass (`typology::detail_enabled()`):
/// stair/elevator penthouses on bare courtyard roofs, iconic SF wooden/steel
/// rooftop water tanks on steel trestles, multi-unit chiller/HVAC mechanical
/// yards with slatted screen walls & duct runs, glazed lightwell skylights,
/// timber rooftop decks, expanded solar, and hillside condensers.
/// Uses an isolated RNG stream (`|roof-plant-v6`) so legacy v3/v4/v5 streams stay untouched.
fn roofscape_v6(ctx: &mut Ctx, c: &Classified, residential: bool, area: f64) {
    let mut rng = Rng::from_num(hash_seed(&format!("{}|roof-plant-v6", c.seed)) as f64);
    let ms = ctx.metal_slot;
    let ry = ctx.roof_y;
    let ceiling = ctx.ceiling;
    // Allow disciplined headroom for sculpted rooftop assemblies beyond legacy budget caps
    if ctx.tri_cap.is_finite() {
        ctx.tri_cap += 460.0;
    }

    // Precompute wing-aligned interior anchors (inset inward along every roof edge >= 3.5 m) so
    // U-shaped, L-shaped, and courtyard apartment roofs (`05a` foreground U-roof) reliably place
    // stair/elevator bulkheads, water tanks, and chiller yards directly on their solid roof wings.
    let mut wing_anchors: Vec<[f64; 2]> = Vec::new();
    for e in &ctx.frame.edges {
        if e.length < 3.5 {
            continue;
        }
        let s = e.length * 0.5;
        for inset in [2.2, 3.6] {
            let wx = e.basis.origin[0] + e.basis.u[0] * s - e.basis.n[0] * inset;
            let wz = e.basis.origin[1] + e.basis.u[1] * s - e.basis.n[1] * inset;
            let p = ctx.frame.to_plan(wx, wz);
            if rect_inside(ctx.frame, p.x - 0.4, p.x + 0.4, p.z - 0.4, p.z + 0.4, 0.2) {
                wing_anchors.push([p.x, p.z]);
            }
        }
    }

    let find_free = |ctx: &Ctx, rng: &mut Rng, sx: f64, sz: f64, margin: f64, gap: f64| -> Option<Rect> {
        let b = ctx.frame.plan_bounds.clone();
        let inset_margin = (margin * 0.52).max(0.38);
        // Pass 1: probe wing-aligned interior anchors (fast & reliable for U-shaped / L-shaped wings)
        if !wing_anchors.is_empty() {
            let n = wing_anchors.len();
            let start = (rng.next() * n as f64).floor() as usize % n;
            for step in 0..n.min(20) {
                let [cx, cz] = wing_anchors[(start + step) % n];
                let (ax, az) = if (step & 1) == 0 { (sx, sz) } else { (sz, sx) };
                let x0 = cx - ax * 0.5;
                let x1 = cx + ax * 0.5;
                let z0 = cz - az * 0.5;
                let z1 = cz + az * 0.5;
                if rect_inside(ctx.frame, x0, x1, z0, z1, inset_margin)
                    && !overlaps(&ctx.placed, x0, x1, z0, z1, gap)
                    && !overlaps(&ctx.blocked, x0, x1, z0, z1, 0.25)
                {
                    return Some(Rect { x0, x1, z0, z1, y1: ry });
                }
            }
        }
        // Pass 2: random bounding-box search (48 attempts)
        for _ in 0..48 {
            let (ax, az) = if rng.next() < 0.5 { (sx, sz) } else { (sz, sx) };
            let span_x = (b.max_x - b.min_x) - ax - 2.0 * inset_margin;
            let span_z = (b.max_z - b.min_z) - az - 2.0 * inset_margin;
            if span_x < 0.0 || span_z < 0.0 {
                continue;
            }
            let x0 = b.min_x + inset_margin + rng.next() * span_x;
            let z0 = b.min_z + inset_margin + rng.next() * span_z;
            let x1 = x0 + ax;
            let z1 = z0 + az;
            if !rect_inside(ctx.frame, x0, x1, z0, z1, inset_margin) {
                continue;
            }
            if overlaps(&ctx.placed, x0, x1, z0, z1, gap) || overlaps(&ctx.blocked, x0, x1, z0, z1, 0.25) {
                continue;
            }
            return Some(Rect { x0, x1, z0, z1, y1: ry });
        }
        None
    };

    // 0a. Wing-friendly stair / elevator bulkhead penthouse on mid-size & large courtyard roofs
    //     (adds a primary penthouse if missing, plus a 2nd stair bulkhead on large >= 260 m2 U/L wings)
    let bulkhead_count = ctx.plan.parts.iter().filter(|p| matches!(p.kind.as_str(), "roof-bulkhead" | "roof-penthouse")).count();
    let want_bulkheads = if area >= 260.0 { 2usize } else if area >= 95.0 { 1usize } else { 0usize };
    for _ in bulkhead_count..want_bulkheads {
        if !(ceiling > ry + 1.8 && ctx.affords(4.0)) {
            break;
        }
        let pw = 2.2 + rng.next() * 1.1;
        let pd = 2.4 + rng.next() * 1.2;
        let ph = (2.15 + rng.next() * 0.75).min(ceiling - ry - 0.14);
        if ph >= 1.45 {
            if let Some(p) = find_free(ctx, &mut rng, pw, pd, 0.72, 0.65) {
                let y1 = ry + ph;
                ctx.detail(p.x0, p.x1, p.z0, p.z1, ry - 0.02, y1, "roof-penthouse", "wall", "wing-bulkhead");
                // Overhanging coping cap + dark rooftop access door panel
                ctx.detail(p.x0 - 0.10, p.x1 + 0.10, p.z0 - 0.10, p.z1 + 0.10, y1 - 0.02, (y1 + 0.12).min(ceiling), "roof-curb", ms, "bulkhead-cap");
                let dw = 0.88f64.min((p.x1 - p.x0) * 0.45);
                let cx = 0.5 * (p.x0 + p.x1);
                let dh = (ph - 0.25).min(2.05);
                if dh > 1.2 {
                    ctx.detail(cx - dw * 0.5, cx + dw * 0.5, p.z1 - 0.02, p.z1 + 0.05, ry + 0.04, ry + dh, ROOF_DETAIL_KIND, ms, "bulkhead-door");
                }
                ctx.placed.push(Rect { y1: y1 + 0.14, ..p });
            }
        }
    }

    // 0b. Iconic SF rooftop wooden/steel water tank on 4-leg steel trestle (older/larger flat roofs)
    let tank_share = if area >= 220.0 { 0.65 } else { 0.44 };
    if area >= 165.0 && c.storeys >= 3.0 && ceiling > ry + 1.95 && rng.next() < tank_share && ctx.affords(8.0) {
        let rad = 0.92 + rng.next() * 0.32;
        let foot = rad * 2.10;
        let avail_h = (ceiling - ry - 0.08).min(4.2);
        if avail_h >= 1.85 {
            if let Some(t) = find_free(ctx, &mut rng, foot, foot, 0.75, 0.70) {
                let cx = 0.5 * (t.x0 + t.x1);
                let cz = 0.5 * (t.z0 + t.z1);
                let leg_h = (avail_h * 0.34).clamp(0.75, 1.35);
                let tank_y0 = ry + leg_h;
                let tank_y1 = (ry + avail_h - 0.38).max(tank_y0 + 1.0);
                let wood_tank = rng.next() < 0.72;
                let tank_slot = if wood_tank {
                    ctx.plan.materials.entry("waterTankWood".into()).or_insert_with(|| "wood-siding".into());
                    "waterTankWood"
                } else {
                    ms
                };
                ctx.plan.materials.entry("roofRail".into()).or_insert_with(|| "painted-steel".into());
                // 4 dark steel trestle legs + timber dunnage platform under the cylindrical tank
                let off = rad * 0.62;
                for sx in [-off, off] {
                    for sz in [-off, off] {
                        ctx.detail(cx + sx - 0.07, cx + sx + 0.07, cz + sz - 0.07, cz + sz + 0.07, ry - 0.02, tank_y0 + 0.04, "roof-water-tank", "roofRail", "tank-leg");
                    }
                }
                ctx.detail(cx - rad * 0.92, cx + rad * 0.92, cz - rad * 0.92, cz + rad * 0.92, tank_y0 - 0.08, tank_y0 + 0.06, "roof-water-tank", "roofRail", "tank-deck");
                // 8-sided cylindrical tank body + stepped conical cap
                let oct = |r: f64| -> Vec<P2> {
                    (0..8)
                        .map(|i| {
                            let a = std::f64::consts::TAU * (i as f64 + 0.5) / 8.0;
                            ctx.frame.to_world(cx + r * a.cos(), cz + r * a.sin())
                        })
                        .collect()
                };
                ctx.plan.add_prism(PartArgs {
                    kind: "roof-water-tank".into(),
                    slot: tank_slot.into(),
                    points: oct(rad),
                    y0: tank_y0,
                    y1: tank_y1,
                    cast_shadow: Some(true),
                    receive_shadow: Some(true),
                    meta: Some(ctx.meta("water-tank-body")),
                    ..Default::default()
                });
                ctx.plan.add_prism(PartArgs {
                    kind: "roof-water-tank".into(),
                    slot: ms.into(),
                    points: oct(rad * 1.07),
                    y0: tank_y1 - 0.02,
                    y1: (tank_y1 + 0.16).min(ceiling),
                    cast_shadow: Some(true),
                    receive_shadow: Some(true),
                    meta: Some(ctx.meta("water-tank-eave")),
                    ..Default::default()
                });
                let cone_y1 = (tank_y1 + 0.36).min(ceiling);
                if cone_y1 > tank_y1 + 0.18 {
                    ctx.plan.add_prism(PartArgs {
                        kind: "roof-water-tank".into(),
                        slot: ms.into(),
                        points: oct(rad * 0.62),
                        y0: tank_y1 + 0.14,
                        y1: cone_y1,
                        cast_shadow: Some(true),
                        receive_shadow: Some(true),
                        meta: Some(ctx.meta("water-tank-cap")),
                        ..Default::default()
                    });
                }
                ctx.tri_used += 96.0;
                ctx.placed.push(Rect { y1: cone_y1, ..t });
            }
        }
    }

    // 1. Timber rooftop decks on ~30% of mid/larger SF residential flat roofs (lean 5-box silhouette)
    if residential && area >= 72.0 && rng.next() < 0.30 && ceiling > ry + 1.15 && ctx.affords(6.0) {
        let dw = (3.4 + rng.next() * 2.8).min((area * 0.24).sqrt() * 1.15);
        let dd = (2.6 + rng.next() * 2.0).min((area * 0.24).sqrt() * 0.90);
        if let Some(d) = find_free(ctx, &mut rng, dw, dd, 1.0, 0.75) {
            ctx.plan.materials.entry("roofDeck".into()).or_insert_with(|| "wood-siding".into());
            ctx.plan.materials.entry("roofRail".into()).or_insert_with(|| "painted-steel".into());
            ctx.detail(d.x0, d.x1, d.z0, d.z1, ry - 0.02, ry + 0.18, "roof-deck", "roofDeck", "roof-deck");
            let rail_y0 = ry + 0.98;
            let rail_y1 = ry + 1.05;
            let rt = 0.06;
            ctx.detail(d.x0, d.x1, d.z0, d.z0 + rt, rail_y0, rail_y1, "roof-deck-rail", "roofRail", "deck-rail");
            ctx.detail(d.x0, d.x1, d.z1 - rt, d.z1, rail_y0, rail_y1, "roof-deck-rail", "roofRail", "deck-rail");
            ctx.detail(d.x0, d.x0 + rt, d.z0, d.z1, rail_y0, rail_y1, "roof-deck-rail", "roofRail", "deck-rail");
            ctx.detail(d.x1 - rt, d.x1, d.z0, d.z1, rail_y0, rail_y1, "roof-deck-rail", "roofRail", "deck-rail");
            ctx.placed.push(Rect { y1: rail_y1, ..d });
        }
    }

    // 2. Glazed lightwell skylights (lean 2-box curb + lid; wing-friendly margin for courtyard roofs)
    let skylight_count = if area >= 220.0 { 4 } else if area >= 95.0 { 2 } else if area >= 48.0 { 1 } else { 0 };
    let mut has_sky_slot = false;
    for _ in 0..skylight_count {
        if !ctx.affords(2.0) {
            break;
        }
        let sw = 1.15 + rng.next() * 1.05;
        let sd = 0.95 + rng.next() * 0.80;
        let sh = (0.28 + rng.next() * 0.18).min(ceiling - ry - 0.12);
        if sh < 0.16 {
            break;
        }
        if let Some(sk) = find_free(ctx, &mut rng, sw, sd, 0.75, 0.55) {
            if !has_sky_slot {
                ctx.plan.materials.entry("skylightGlass".into()).or_insert_with(|| "glass-sky".into());
                has_sky_slot = true;
            }
            ctx.detail(sk.x0, sk.x1, sk.z0, sk.z1, ry - 0.02, ry + sh, "roof-curb", ms, "skylight-curb");
            ctx.detail(sk.x0 + 0.05, sk.x1 - 0.05, sk.z0 + 0.05, sk.z1 - 0.05, ry + sh - 0.01, ry + sh + 0.09, "roof-skylight", "skylightGlass", "skylight-pane");
            ctx.placed.push(Rect { y1: ry + sh + 0.09, ..sk });
        }
    }

    // 3. Wing-friendly packaged HVAC chiller yard, slatted mechanical screen & duct runs on courtyard/apartment roofs (area >= 130 m2)
    //    so U-shaped / L-shaped residential & mixed-use flat roofs look richly architectural instead of bare slabs.
    if area >= 130.0 && ceiling > ry + 1.1 {
        let extra_hvac = if area >= 340.0 { 4 } else if area >= 200.0 { 3 } else { 2 };
        let mut screened = false;
        for _ in 0..extra_hvac {
            if !ctx.affords(3.0) {
                break;
            }
            let uw = 1.50 + rng.next() * 1.05;
            let ud = 1.10 + rng.next() * 0.70;
            let uh = (0.95 + rng.next() * 0.55).min(ceiling - ry - 0.15);
            if uh > 0.45 {
                if let Some(u) = find_free(ctx, &mut rng, uw, ud, 0.75, 0.60) {
                    let y1 = ry + uh;
                    ctx.detail(u.x0, u.x1, u.z0, u.z1, ry - 0.02, y1, "roof-plant", ms, "wing-hvac");
                    ctx.detail(u.x0 + 0.15, u.x1 - 0.15, u.z0 + 0.15, u.z1 - 0.15, y1 - 0.01, y1 + 0.14, ROOF_DETAIL_KIND, ms, "unit-grille");
                    // Low louvered mechanical screen wall framing the primary rooftop chiller unit
                    if !screened && area >= 200.0 && ctx.affords(4.0) {
                        let sh = (uh + 0.22).min(ceiling - ry - 0.08);
                        let st = 0.09;
                        let pad = 0.22;
                        ctx.detail(u.x0 - pad, u.x1 + pad, u.z0 - pad, u.z0 - pad + st, ry - 0.02, ry + sh, "roof-screen", ms, "hvac-screen");
                        ctx.detail(u.x0 - pad, u.x1 + pad, u.z1 + pad - st, u.z1 + pad, ry - 0.02, ry + sh, "roof-screen", ms, "hvac-screen");
                        ctx.detail(u.x0 - pad, u.x0 - pad + st, u.z0 - pad, u.z1 + pad, ry - 0.02, ry + sh, "roof-screen", ms, "hvac-screen");
                        screened = true;
                    }
                    ctx.placed.push(Rect { y1: y1 + 0.18, ..u });
                }
            }
        }
        let duct_runs = if area >= 280.0 { 2 } else { 1 };
        for _ in 0..duct_runs {
            if !ctx.affords(2.0) {
                break;
            }
            let dl = 3.2 + rng.next() * 3.0;
            let dw = 0.52 + rng.next() * 0.24;
            let dh = (0.44 + rng.next() * 0.18).min(ceiling - ry - 0.15);
            if dh > 0.25 {
                if let Some(dr) = find_free(ctx, &mut rng, dl, dw, 0.72, 0.48) {
                    ctx.detail(dr.x0, dr.x1, dr.z0, dr.z1, ry + 0.10, ry + 0.10 + dh, "roof-duct", ms, "wing-duct");
                    ctx.placed.push(Rect { y1: ry + 0.12 + dh, ..dr });
                }
            }
        }
    }

    // 4. Expanded rooftop solar adoption on flat SF roofs without an existing array
    let has_solar = ctx.plan.parts.iter().any(|p| p.kind == "roof-solar");
    if !has_solar && area >= 68.0 && rng.next() < 0.22 {
        let side = (area * 0.25).sqrt() * (0.60 + rng.next() * 0.35);
        let aspect = 1.2 + rng.next() * 0.8;
        for k in [1.0, 0.75, 0.55] {
            let sx = 18.0f64.min(side * k * aspect.sqrt());
            let sz = 12.0f64.min(side * k / aspect.sqrt());
            if sx < 3.6 || sz < 2.8 {
                break;
            }
            let Some(rect) = find_free(ctx, &mut rng, sx, sz, 0.9, 0.8) else { continue };
            if ctx.solar_array(rect) > 0.0 {
                ctx.placed.push(Rect { y1: ry + 0.36, ..rect });
                break;
            }
        }
    }

    // 5. Compact mini-split AC condenser on mid hillside residential roofs
    if residential && area >= 42.0 && area < 160.0 && rng.next() < 0.48 && ctx.affords(2.0) {
        let cw = 0.85 + rng.next() * 0.30;
        let cd = 0.42 + rng.next() * 0.16;
        let ch = (0.72 + rng.next() * 0.22).min(ceiling - ry - 0.1);
        if ch > 0.35 {
            if let Some(ac) = find_free(ctx, &mut rng, cw, cd, 0.80, 0.50) {
                let y1 = ry + ch;
                ctx.detail(ac.x0, ac.x1, ac.z0, ac.z1, ry - 0.02, y1, "roof-plant", ms, "mini-split");
                ctx.placed.push(Rect { y1: y1 + 0.1, ..ac });
            }
        }
    }
}
