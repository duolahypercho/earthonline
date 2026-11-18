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
    if !(area >= 40.0) || !(ceiling > roof_y + 0.5) {
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
}
