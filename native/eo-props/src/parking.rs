//! Parked cars on the kerb (vehicle-presentation.js `planParkedVehicles`):
//! parkable street classes with a carriageway half-width of at least 3.1 m;
//! the kerb from the junction trim plus the daylighting clearance (`KERB`
//! `endClearance`, `splitClearance` at a node-less end); the street contract's
//! declared parking bays decide the kerbs; each stall draws from its own
//! stream `<seed>|stall|<run key>|<side>|<ordinal>` (body class by
//! `PARKING_WEIGHT`, no repeat of the previous class, paint by its set with the
//! tone-bucket rule, the pitch gap or the metered stall pitch of the street
//! type, the street type's occupancy, the kerb gap inside the bay, a small
//! skew); nose to the lane's flow.
//!
//! a stall is refused, in the JS order and with the JS reason, by
//! (`parking_rules.rs`): a vehicle crossover (`planVehicleCrossovers`, ported:
//! drives and surface-lot frontages), the kerb restrictions
//! (`kerbRestrictionsFor`: corner red, observed red, the kerb inside another
//! street, an observed empty kerb, a surveyed hydrant's 15 ft
//! (`caMetric('no-parking-fire-hydrant')`), a surveyed bus stop's zone, a
//! mapped exclusive-turn pocket), a driveway or alley mouth (`planKerbMouths`,
//! `caMetric('no-parking-driveway')`), a drawn hydrant (`hydrants`, the
//! furniture adapter), the churn draw, the running lane, another street's
//! running carriageway, the footprint zones (junction box, crossing band, other
//! carriageway, other footway), a mapped crossing or a way closed to cars with
//! the 20 ft before it, and another parked car (`gridHit`). The observed block
//! overlays' kerb records (parked kerb, excluded bodies, cast cars, block
//! paints) apply as in the JS.
//!
//! Identity: the runtime keys a run's stall streams by its stable run key
//! (`stallIdentityOf`: the pieces' keys joined by `+`), as the streamed JS kerb
//! does. `plan_window` takes the key per run, so a test can deal the JS boot
//! window's own segment ids instead.
//!
//! The JS walks the window's segments nearest the camera first through one
//! shared overlap grid. The runtime has no camera order: each run is planned
//! alone (its own grid), and a car is then dropped when it overlaps a car of a
//! neighbouring run with a lower plan index (`plan_entry`), deterministic and
//! independent of which tiles were asked first.
//!
//! Not ported, stated: the garage doors of the archetype buildings
//! (`garageCurbCutSteps` reads the JS building templates, not in the native
//! data), the seam / crest / buried-stall grounding guards (they read the JS
//! drawn-road mesh index; the oracle runs without one too), street furniture
//! in the stall (`collectStreetFurnitureObstacles`), the other parked layer's
//! claims (street-life is not ported) and the ring caps.

use crate::parking_rules::{KerbRules, observed_cast_at, observed_station, quads_overlap, vehicle_footprint};
use crate::{P2, ParkedCar, PropsWorld, StreetRandom, pick_weighted, station_at};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

const PARKABLE: [&str; 10] = ["residential", "tertiary", "tertiary_link", "secondary", "secondary_link", "primary", "primary_link", "unclassified", "living_street", "service"];
/// `PARKED_SKEW_MAX_RAD`.
const SKEW_MAX: f64 = 0.025;

/// `classBand(classRank)`.
pub fn class_band(rank: i32) -> usize {
    if rank <= 1 {
        0
    } else if rank <= 3 {
        1
    } else if rank <= 4 {
        2
    } else {
        3
    }
}

/// `paintToneBucket(hex)`.
pub fn tone_bucket(hex: u32) -> &'static str {
    let r = ((hex >> 16) & 255) as f64 / 255.0;
    let g = ((hex >> 8) & 255) as f64 / 255.0;
    let b = (hex & 255) as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let light = (max + min) / 2.0;
    if max - min < 0.075 {
        return if light > 0.68 {
            "white"
        } else if light > 0.45 {
            "silver"
        } else if light > 0.24 {
            "grey"
        } else {
            "black"
        };
    }
    let mut hue = if max == r {
        ((g - b) / (max - min)) % 6.0
    } else if max == g {
        (b - r) / (max - min) + 2.0
    } else {
        (r - g) / (max - min) + 4.0
    };
    hue = (hue * 60.0 + 360.0) % 360.0;
    if !(40.0..330.0).contains(&hue) {
        "red"
    } else if hue < 75.0 {
        "tan"
    } else if hue < 170.0 {
        "green"
    } else if hue < 270.0 {
        "blue"
    } else {
        "red"
    }
}

/// `planKerbMouths`: station windows per (entry, side) where a drive (class rank 1)
/// meets or crosses a host street's kerb.
pub fn kerb_mouths(w: &PropsWorld) -> &HashMap<(u32, i8), Vec<[f64; 2]>> {
    w.mouths.get_or_init(|| {
        let plan = &w.street.plan;
        let clear = w.std.no_parking_driveway;
        let along_cos = (20.0f64).to_radians().cos();
        let cell = 32.0;
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (hi, h) in plan.entries.iter().enumerate() {
            // (a structure's leg into a junction, eo-street joins, is not drawn.)
            if h.class_rank <= 1 || h.points.len() < 2 || h.structure_leg.is_some() {
                continue;
            }
            let reach = h.half + h.walks[0].max(h.walks[1]);
            let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for p in &h.points {
                x0 = x0.min(p[0]);
                x1 = x1.max(p[0]);
                z0 = z0.min(p[1]);
                z1 = z1.max(p[1]);
            }
            for gx in ((x0 - reach) / cell).floor() as i32..=((x1 + reach) / cell).floor() as i32 {
                for gz in ((z0 - reach) / cell).floor() as i32..=((z1 + reach) / cell).floor() as i32 {
                    grid.entry((gx, gz)).or_default().push(hi as u32);
                }
            }
        }
        let near = |x: f64, z: f64, r: f64| {
            let mut out: Vec<u32> = vec![];
            for gx in ((x - r) / cell).floor() as i32..=((x + r) / cell).floor() as i32 {
                for gz in ((z - r) / cell).floor() as i32..=((z + r) / cell).floor() as i32 {
                    if let Some(v) = grid.get(&(gx, gz)) {
                        out.extend_from_slice(v);
                    }
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        };
        // (d, s, tx, tz, nx, nz, lat)
        let project = |pts: &[[f64; 2]], x: f64, z: f64| {
            let mut best: Option<[f64; 7]> = None;
            let mut run = 0.0;
            for i in 1..pts.len() {
                let (a, b) = (pts[i - 1], pts[i]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let len = dx.hypot(dz);
                if !(len > 1e-9) {
                    continue;
                }
                let (tx, tz) = (dx / len, dz / len);
                let t = ((x - a[0]) * tx + (z - a[1]) * tz).clamp(0.0, len);
                let (px, pz) = (a[0] + tx * t, a[1] + tz * t);
                let d = (x - px).hypot(z - pz);
                if best.is_none_or(|bb| d < bb[0]) {
                    best = Some([d, run + t, tx, tz, -tz, tx, (x - px) * -tz + (z - pz) * tx]);
                }
                run += len;
            }
            best
        };
        let mut windows: HashMap<(u32, i8), Vec<[f64; 2]>> = HashMap::new();
        let mut add = |host: u32, side: i8, s_kerb: f64, along: f64, drive_half: f64| {
            let half = drive_half / (1.0 - along_cos * along_cos).max(1.0 - along * along).sqrt() + clear;
            windows.entry((host, side)).or_default().push([s_kerb - half, s_kerb + half]);
        };
        for (di, drive) in plan.entries.iter().enumerate() {
            if drive.class_rank != 1 || drive.points.len() < 2 || drive.structure_leg.is_some() {
                continue;
            }
            let pts = &drive.points;
            for at_start in [true, false] {
                let p = if at_start { pts[0] } else { pts[pts.len() - 1] };
                let q = if at_start { pts[1] } else { pts[pts.len() - 2] };
                let len = (q[0] - p[0]).hypot(q[1] - p[1]);
                if !(len > 1e-6) {
                    continue;
                }
                let (vx, vz) = ((q[0] - p[0]) / len, (q[1] - p[1]) / len);
                for hi in near(p[0], p[1], drive.half * 4.0) {
                    if hi as usize == di {
                        continue;
                    }
                    let host = &plan.entries[hi as usize];
                    let Some(f) = project(&host.points, p[0], p[1]) else { continue };
                    let out_n = vx * f[4] + vz * f[5];
                    let side: i8 = if out_n > 0.0 { 1 } else { -1 };
                    let walk = if side > 0 { host.walks[0] } else { host.walks[1] };
                    if !(f[0] <= host.half + walk + drive.half * 2.0) {
                        continue;
                    }
                    if f[1] <= 0.0 || f[1] >= host.length {
                        continue;
                    }
                    let along = vx * f[2] + vz * f[3];
                    if along.abs() > along_cos {
                        continue;
                    }
                    let t = (side as f64 * host.half - f[6]) / out_n;
                    add(hi, side, f[1] + t * along, along, drive.half);
                }
            }
            for i in 1..pts.len() {
                let (a, b) = (pts[i - 1], pts[i]);
                let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                let seg_len = (b[0] - a[0]).hypot(b[1] - a[1]);
                for hi in near(mid[0], mid[1], seg_len / 2.0 + 1.0) {
                    if hi as usize == di {
                        continue;
                    }
                    let host = &plan.entries[hi as usize];
                    let hp = &host.points;
                    for k in 1..hp.len() {
                        let (c, d) = (hp[k - 1], hp[k]);
                        let (rx, rz, sx, sz) = (b[0] - a[0], b[1] - a[1], d[0] - c[0], d[1] - c[1]);
                        let den = rx * sz - rz * sx;
                        if den.abs() < 1e-12 {
                            continue;
                        }
                        let t = ((c[0] - a[0]) * sz - (c[1] - a[1]) * sx) / den;
                        let u = ((c[0] - a[0]) * rz - (c[1] - a[1]) * rx) / den;
                        if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
                            continue;
                        }
                        let x = [a[0] + rx * t, a[1] + rz * t];
                        if (x[0] - pts[0][0]).hypot(x[1] - pts[0][1]) < 1e-3 || (x[0] - pts[pts.len() - 1][0]).hypot(x[1] - pts[pts.len() - 1][1]) < 1e-3 {
                            continue;
                        }
                        let Some(f) = project(hp, x[0], x[1]) else { continue };
                        if f[1] <= 0.0 || f[1] >= host.length {
                            continue;
                        }
                        let l = seg_len.max(1e-9);
                        let along = (rx * f[2] + rz * f[3]) / l;
                        if along.abs() > along_cos {
                            continue;
                        }
                        let out_n = (rx * f[4] + rz * f[5]) / l;
                        for side in [1i8, -1] {
                            add(hi, side, f[1] + ((side as f64 * host.half) / out_n) * along, along, drive.half);
                        }
                    }
                }
            }
        }
        windows
    })
}


/// The kerb rules of this world, built on first use.
pub fn rules(w: &PropsWorld) -> &KerbRules {
    w.kerb_rules.get_or_init(|| KerbRules::build(w).expect("parking-js.json.gz"))
}

/// Drawn hydrants of one run: (station, side, foot position), as the furniture
/// walk places them (`furniture.hydrantsNear` in the JS reads the drawn list).
pub fn hydrants(w: &PropsWorld, ei: usize) -> Vec<(f64, i8, [f64; 3])> {
    crate::furniture::hydrants_of_entry(w, ei)
}

/// A planned stall: the car and its plan footprint (`vehicleFootprint`, 0.05 m margin).
#[derive(Clone, Debug)]
pub struct Stall {
    pub car: ParkedCar,
    pub corners: [P2; 4],
}

/// The JS overlap grid (`makeGrid(8)`, `gridHit`, `gridAdd`).
#[derive(Default)]
pub struct OverlapGrid {
    cells: HashMap<(i32, i32), Vec<[P2; 4]>>,
}

impl OverlapGrid {
    fn keys(c: &[P2; 4]) -> impl Iterator<Item = (i32, i32)> {
        let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for p in c {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            z0 = z0.min(p[1]);
            z1 = z1.max(p[1]);
        }
        let (gx0, gx1, gz0, gz1) = ((x0 / 8.0).floor() as i32, (x1 / 8.0).floor() as i32, (z0 / 8.0).floor() as i32, (z1 / 8.0).floor() as i32);
        (gx0..=gx1).flat_map(move |gx| (gz0..=gz1).map(move |gz| (gx, gz)))
    }
    pub fn hit(&self, c: &[P2; 4]) -> bool {
        Self::keys(c).any(|k| self.cells.get(&k).is_some_and(|l| l.iter().any(|o| quads_overlap(c, o))))
    }
    pub fn add(&mut self, c: [P2; 4]) {
        for k in Self::keys(&c) {
            self.cells.entry(k).or_default().push(c);
        }
    }
}

/// One planning pass: the shared grid, the rejection tally and the caps.
pub struct PlanCtx {
    pub grid: OverlapGrid,
    pub rejections: BTreeMap<String, usize>,
    /// Test the drawn hydrants (`furniture.hydrantsNear`): the JS oracle plans without furniture.
    pub drawn_hydrants: bool,
    /// `maxVehicles`.
    pub cap: usize,
    pub placed: usize,
}

impl PlanCtx {
    pub fn new(drawn_hydrants: bool, cap: usize) -> Self {
        Self { grid: OverlapGrid::default(), rejections: BTreeMap::new(), drawn_hydrants, cap, placed: 0 }
    }
    fn reject(&mut self, why: &str) {
        *self.rejections.entry(why.to_string()).or_default() += 1;
    }
}

/// `stallIdentityOf`: the run's key (its pieces' keys joined by `+`), else its id.
pub fn stall_key(w: &PropsWorld, ei: usize) -> String {
    let e = &w.street.plan.entries[ei];
    let segs = &w.street.city.segments;
    let keys: Option<Vec<&str>> = e.pieces.iter().map(|&p| segs[p].key.as_deref()).collect();
    match keys {
        Some(k) if !k.is_empty() => k.join("+"),
        _ => e.id.clone(),
    }
}

/// One side's observed overlay records, in the run's stations.
#[derive(Default)]
struct Observed {
    on: bool,
    parked: Vec<(f64, f64)>,
    bodies: Vec<(f64, f64, Vec<String>)>,
    /// (lo, hi, only, cars: (axis s, station, body, alt, paint))
    cast: Vec<(f64, f64, bool, Vec<(f64, Option<f64>, String, Option<String>, Option<u32>)>)>,
}

struct Cast {
    d: f64,
    station: f64,
    key: u64,
    body: String,
    alt: Option<String>,
    paint: Option<u32>,
}

struct ObservedStall {
    full: bool,
    exclude: Option<HashSet<String>>,
    cast: Option<Cast>,
    cast_only: bool,
}

impl Observed {
    fn of(w: &PropsWorld, r: &KerbRules, ei: usize, side: i8) -> Self {
        let Some(ov) = r.overlay(ei) else { return Self::default() };
        let e = &w.street.plan.entries[ei];
        let mut o = Observed { on: true, ..Default::default() };
        for (s, lo, hi, _) in r.overlay_windows(w, ei, "kerbParked") {
            if s == side {
                o.parked.push((lo, hi));
            }
        }
        for (s, lo, hi, entry) in r.overlay_windows(w, ei, "kerbBodies") {
            if s == side {
                let ex = entry["exclude"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
                if let Some(ex) = ex {
                    o.bodies.push((lo, hi, ex));
                }
            }
        }
        for (s, lo, hi, entry) in r.overlay_windows(w, ei, "kerbCast") {
            if s != side {
                continue;
            }
            let cars = entry["cars"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|car| {
                            let cs = car["s"].as_f64().unwrap_or(f64::NAN);
                            (
                                cs,
                                observed_station(e, ov, cs),
                                car["body"].as_str().unwrap_or("").to_string(),
                                car["alt"].as_str().map(String::from),
                                car["paint"].as_f64().map(|p| p as u32),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            o.cast.push((lo, hi, entry["only"].as_bool().unwrap_or(false), cars));
        }
        o
    }

    /// `observedKerbStall(segment, side, s, reach, taken)`.
    fn stall(&self, s: f64, reach: f64, taken: Option<&HashSet<u64>>) -> Option<ObservedStall> {
        if !self.on {
            return None;
        }
        let full = self.parked.iter().any(|w| s >= w.0 && s <= w.1);
        let mut exclude: Option<HashSet<String>> = None;
        for (lo, hi, ex) in &self.bodies {
            if s < *lo || s > *hi {
                continue;
            }
            exclude.get_or_insert_with(HashSet::new).extend(ex.iter().cloned());
        }
        let mut cast: Option<Cast> = None;
        let mut cast_only = false;
        for (lo, hi, only, cars) in &self.cast {
            if s >= *lo && s <= *hi && *only {
                cast_only = true;
            }
            for (cs, station, body, alt, paint) in cars {
                let Some(station) = *station else { continue };
                let key = cs.to_bits();
                if taken.is_some_and(|t| t.contains(&key)) {
                    continue;
                }
                let d = (station - s).abs();
                if d > reach || cast.as_ref().is_some_and(|c| d >= c.d) {
                    continue;
                }
                cast = Some(Cast { d, station, key, body: body.clone(), alt: alt.clone(), paint: *paint });
            }
        }
        if full || exclude.is_some() || cast.is_some() || cast_only {
            Some(ObservedStall { full: full || cast.is_some(), exclude, cast, cast_only })
        } else {
            None
        }
    }
}

/// `planParkedVehicles`' body for one run (segment), with `key` naming its stall streams.
pub fn plan_kerbs(w: &PropsWorld, ei: usize, key: &str, ctx: &mut PlanCtx) -> Vec<Stall> {
    let mut out = vec![];
    let r = rules(w);
    let e = &w.street.plan.entries[ei];
    let seg = w.segment_of(e);
    if !PARKABLE.contains(&e.highway.as_str()) {
        ctx.reject("class-not-parkable");
        return out;
    }
    if !(e.half >= 3.1) {
        ctx.reject("carriageway-too-narrow");
        return out;
    }
    let k = |key: &str| w.js.num("kerb", key);
    let band = class_band(e.class_rank);
    let node_start = e.approach_start.is_some();
    let node_end = e.approach_end.is_some();
    let start = e.trim_start + if node_start { k("endClearance") } else { k("splitClearance") };
    let end = e.length - e.trim_end - if node_end { k("endClearance") } else { k("splitClearance") };
    let street_type = seg.street_type.clone().unwrap_or_default();
    let stall_pitch = w.js.consts["kerbStallPitch"][street_type.as_str()].as_f64();
    if !(end - start > 4.2) {
        ctx.reject("no-usable-kerb");
        return out;
    }
    let bays = seg.parking;
    if !(bays[0] > 0.0 || bays[1] > 0.0) {
        ctx.reject("no-declared-parking-bay");
        return out;
    }
    let both = bays[0] > 0.0 && bays[1] > 0.0;
    let sides: Vec<i8> = if both { vec![1, -1] } else { vec![if bays[0] > 0.0 { 1 } else { -1 }] };
    let lanes = if both { 2.0 } else { 1.0 };
    let weight = |id: &str| w.js.parking_weight.get(id).map(|wt| wt[band]).unwrap_or(0.0);
    let candidates: Vec<&crate::data::VehicleSpec> = w
        .js
        .vehicle_specs
        .iter()
        .filter(|s| weight(&s.id) > 0.0 && e.half * 2.0 - lanes * (s.width + k("targetGap")) >= k("minRunningLane"))
        .collect();
    if candidates.is_empty() {
        ctx.reject("no-class-fits-carriageway");
        return out;
    }
    let occupancy = w.js.consts["kerbOccupancy"][street_type.as_str()].as_f64().unwrap_or(1.0 - w.js.consts["parkedEmptyStallRate"].as_f64().unwrap_or(0.36));
    let empty_rate = 1.0 - occupancy;
    let mouths = kerb_mouths(w);
    let along_dir = match seg.oneway.as_str() {
        "yes" | "true" | "1" | "increasing" => Some(1.0),
        "-1" | "decreasing" | "reverse" => Some(-1.0),
        _ => None,
    };
    let overlay = r.overlay(ei);
    let block_paints: Vec<(u32, f64)> = overlay
        .map(|ov| ov.kerb_paints.iter().filter_map(|p| Some((p["hex"].as_f64()? as u32, p["weight"].as_f64().unwrap_or(0.0)))).collect())
        .unwrap_or_default();
    let drawn: Vec<(f64, i8, [f64; 3])> = if ctx.drawn_hydrants { hydrants(w, ei) } else { vec![] };
    for side in sides {
        let sd = side as f64;
        let bay = if side > 0 { bays[0] } else { bays[1] };
        let max_gap = |s: &crate::data::VehicleSpec| bay - s.width - k("bayLaneClearance") - (s.length / 2.0) * SKEW_MAX.sin();
        let side_c: Vec<&crate::data::VehicleSpec> = candidates.iter().copied().filter(|s| max_gap(s) >= k("minGap")).collect();
        if side_c.is_empty() {
            ctx.reject("no-class-fits-parking-bay");
            continue;
        }
        let side_mouths = mouths.get(&(ei as u32, side));
        let red = r.red_windows(w, ei, side);
        let points = r.point_windows(ei, side);
        let obs = Observed::of(w, r, ei, side);
        let mut cast_taken: HashSet<u64> = HashSet::new();
        let mut cursor = start;
        let mut guard = 0;
        let mut last_spec: Option<String> = None;
        let mut last_paint: Option<u32> = None;
        while cursor < end && guard < 400 {
            guard += 1;
            if ctx.placed >= ctx.cap {
                break;
            }
            let mut rand = StreetRandom::new(&format!("{}|stall|{}|{}|{}", w.seed, key, side as i32, guard));
            let ahead = obs.stall(cursor + 2.5, 0.0, None);
            let stall_c: Vec<&crate::data::VehicleSpec> = match ahead.as_ref().and_then(|a| a.exclude.as_ref()) {
                Some(ex) => {
                    let kept: Vec<_> = side_c.iter().copied().filter(|s| !ex.contains(&s.id)).collect();
                    if kept.is_empty() { side_c.clone() } else { kept }
                }
                None => side_c.clone(),
            };
            let mut spec = *pick_weighted(&stall_c, rand.next(), |s| weight(&s.id)).unwrap();
            if last_spec.as_deref() == Some(spec.id.as_str()) && stall_c.len() > 1 {
                let last = last_spec.clone().unwrap();
                spec = *pick_weighted(&stall_c, rand.next(), |s| if s.id == last { 0.0 } else { weight(&s.id) }).unwrap();
            }
            last_spec = Some(spec.id.clone());
            let set = w.js.paint_sets.get(&spec.paint_set).or_else(|| w.js.paint_sets.get("civilian")).unwrap();
            let mut paint = pick_weighted(set, rand.next(), |p| p.weight).unwrap().hex;
            let last_bucket = last_paint.map(tone_bucket);
            for _ in 0..3 {
                if Some(paint) != last_paint && Some(tone_bucket(paint)) != last_bucket {
                    break;
                }
                paint = pick_weighted(set, rand.next(), |p| p.weight).unwrap().hex;
            }
            // An overlay's `kerbPaints` replace the set, from a stream of their own.
            if !block_paints.is_empty() {
                let mut own = StreetRandom::new(&format!("{}|kerb-paint|{}|{}|{}", w.seed, key, side as i32, guard));
                let mut pick = pick_weighted(&block_paints, own.next(), |p| p.1).unwrap().0;
                let mut redraw = 0;
                while redraw < 3 && Some(tone_bucket(pick)) == last_bucket {
                    pick = pick_weighted(&block_paints, own.next(), |p| p.1).unwrap().0;
                    redraw += 1;
                }
                paint = pick;
            }
            last_paint = Some(paint);
            let _rim = rand.next();
            let _settle_y = rand.next();
            let _settle_pitch = rand.next();
            let pitch_gap = k("minPitchGap") + rand.next() * (k("maxPitchGap") - k("minPitchGap"));
            let stalls = stall_pitch.map(|p| ((spec.length + k("minPitchGap") + 0.2) / p).ceil()).unwrap_or(0.0);
            let slot = if stalls > 0.0 { stalls * stall_pitch.unwrap() } else { spec.length + pitch_gap };
            if cursor + slot > end {
                ctx.reject("stall-does-not-fit");
                break;
            }
            let mut station = if stalls > 0.0 {
                cursor + slot / 2.0 + (pitch_gap - (k("minPitchGap") + k("maxPitchGap")) / 2.0) * 0.19
            } else {
                cursor + spec.length / 2.0 + pitch_gap * 0.35
            };
            cursor += slot;
            // Observed cast cars (`kerbCast`, `kerbCastAt`).
            let observed = obs.stall(station, (3.8f64).max(slot / 2.0 + 1.0), Some(&cast_taken));
            if let Some(cast) = observed.as_ref().and_then(|o| o.cast.as_ref()) {
                cast_taken.insert(cast.key);
                if let Some(cs) = side_c.iter().find(|c| c.id == cast.body).or_else(|| cast.alt.as_ref().and_then(|a| side_c.iter().find(|c| &c.id == a))) {
                    spec = cs;
                }
                if let Some(p) = cast.paint {
                    paint = p;
                }
                station = cast.station;
            } else if observed.as_ref().is_some_and(|o| o.cast_only) {
                ctx.reject("stall-observed-cast-empty");
                continue;
            } else if overlay.is_some() || !r.js.overlays.is_empty() {
                let f = station_at(e, station.clamp(0.0, e.length), true);
                let off = (e.half - 1.3) * sd;
                if let Some(car) = observed_cast_at(&r.js, f.x + f.nx * off, f.z + f.nz * off) {
                    let (body, alt) = (car["body"].as_str().unwrap_or(""), car["alt"].as_str());
                    if let Some(cs) = side_c.iter().find(|c| c.id == body).or_else(|| alt.and_then(|a| side_c.iter().find(|c| c.id == a))) {
                        spec = cs;
                    }
                    if let Some(p) = car["paint"].as_f64() {
                        paint = p as u32;
                    }
                }
            }
            let half_len = spec.length / 2.0;
            if r.on_crossover(ei, side, station, half_len) {
                ctx.reject("stall-curb-cut");
                continue;
            }
            {
                let (lo, hi) = (station - half_len, station + half_len);
                let why = red.iter().chain(points.iter()).find(|q| hi > q.lo && lo < q.hi).map(|q| q.reason);
                if let Some(why) = why {
                    ctx.reject(&format!("stall-kerb-{why}"));
                    continue;
                }
            }
            if side_mouths.is_some_and(|v| v.iter().any(|m| station + half_len > m[0] && station - half_len < m[1])) {
                ctx.reject("stall-kerb-mouth");
                continue;
            }
            if !drawn.is_empty() {
                let f = station_at(e, station.clamp(0.0, e.length), true);
                let walk = if side > 0 { e.walks[0] } else { e.walks[1] };
                let hit = drawn.iter().any(|h| {
                    let (dx, dz) = (h.2[0] - f.x, h.2[2] - f.z);
                    let lat = sd * (dx * f.nx + dz * f.nz);
                    !(lat < e.half || lat > e.half + walk + 1.0) && (dx * f.tx + dz * f.tz).abs() - half_len < r.c.hydrant_clear
                });
                if hit {
                    ctx.reject("stall-kerb-hydrant-drawn");
                    continue;
                }
            }
            if rand.next() < empty_rate && !obs.stall(station, 0.0, None).is_some_and(|o| o.full) {
                ctx.reject("stall-empty");
                continue;
            }
            let gap_draw = rand.next();
            let drawn_gap = k("minGap") + gap_draw * (k("maxGap") - k("minGap")) * 0.7 + (k("targetGap") - k("minGap")) * 0.3;
            let kerb_gap = drawn_gap.min(max_gap(spec));
            // `parkingKerbOffsetFor`: a kerbside cycle track with a bay beside it puts the cars outside the track.
            let k_side = if side > 0 { 0 } else { 1 };
            let kerb_offset = if seg.cycle_kerbside[k_side] && bay > 0.0 { seg.cycle[k_side] } else { 0.0 };
            let u = (e.half - kerb_offset - kerb_gap - spec.width / 2.0) * sd;
            let inner = u.abs() - spec.width / 2.0;
            let running = if lanes == 2.0 { inner * 2.0 } else { inner + e.half };
            if !(running >= k("minRunningLane")) {
                ctx.reject("no-running-lane");
                continue;
            }
            if u.abs() + spec.width / 2.0 > e.half - 0.02 {
                ctx.reject("over-the-kerb");
                continue;
            }
            let f = station_at(e, station.clamp(0.0, e.length), true);
            let skew = (rand.next() - 0.5) * 0.05;
            let along = along_dir.unwrap_or(if side > 0 { 1.0 } else { -1.0 });
            let yaw = (f.tx * along).atan2(f.tz * along) + skew;
            let [x, z] = f.at(u);
            let corners = vehicle_footprint(spec.length, spec.width, x, z, yaw, 0.05);
            if r.laps_other_street(&corners, ei) {
                ctx.reject("stall-across-another-street");
                continue;
            }
            if let Some(why) = r.footprint_reason(&corners, ei) {
                ctx.reject(&format!("stall-in-{why}"));
                continue;
            }
            // (`--structures`: off the columns, and under nothing lower than the car.)
            if let Some(why) = r.structure_reason(w, &corners, spec.height) {
                ctx.reject(&format!("stall-{why}"));
                continue;
            }
            if r.on_mapped_crossing(&corners, yaw) {
                ctx.reject("stall-on-mapped-crossing");
                continue;
            }
            if ctx.grid.hit(&corners) {
                ctx.reject("overlaps-another-vehicle");
                continue;
            }
            ctx.grid.add(corners);
            let y = w.street.paved_at(x, z).unwrap_or_else(|| w.ground(x, z));
            ctx.placed += 1;
            out.push(Stall { car: ParkedCar { spec: spec.id.clone(), pos: [x, y, z], yaw, paint, entry: ei as u32, side, station }, corners });
        }
    }
    out
}

/// One run planned alone (its own overlap grid), cached: the runtime's raw kerb.
fn raw_entry(w: &PropsWorld, ei: usize) -> Arc<Vec<Stall>> {
    let r = rules(w);
    if let Some(v) = r.raw.lock().unwrap().get(&(ei as u32)) {
        return v.clone();
    }
    let mut ctx = PlanCtx::new(true, usize::MAX);
    let v = Arc::new(plan_kerbs(w, ei, &stall_key(w, ei), &mut ctx));
    let mut c = r.raw.lock().unwrap();
    if c.len() > 8192 {
        c.clear();
    }
    c.insert(ei as u32, v.clone());
    v
}

/// The parked cars of one run (runtime): its stable-key kerb plan, less every car
/// that overlaps a car of a neighbouring run with a lower plan index.
pub fn plan_entry(w: &PropsWorld, ei: usize) -> Vec<ParkedCar> {
    let r = rules(w);
    let own = raw_entry(w, ei);
    let mut out = Vec::with_capacity(own.len());
    for st in own.iter() {
        let mut clash = false;
        let mut near: Vec<u32> = vec![];
        for c in &st.corners {
            near.extend(r.entries_near(c[0], c[1]).iter().copied().filter(|&o| (o as usize) < ei));
        }
        near.sort_unstable();
        near.dedup();
        for o in near {
            if raw_entry(w, o as usize).iter().any(|s| quads_overlap(&st.corners, &s.corners)) {
                clash = true;
                break;
            }
        }
        if !clash {
            out.push(st.car.clone());
        }
    }
    out
}

/// `planParkedVehicles(plan, { focus, seed })` over a window: the runs in the JS
/// walk order (nearest the focus first) with the key each deals its stalls by,
/// one shared overlap grid, the far-ring reach and the 900-car cap. Drawn
/// hydrants are not tested (the JS window plan runs without furniture).
pub fn plan_window(w: &PropsWorld, runs: &[(usize, String)], focus: [f64; 2], cap: usize) -> (Vec<Stall>, BTreeMap<String, usize>) {
    let r = rules(w);
    let mut ctx = PlanCtx::new(false, cap);
    let mut out = vec![];
    for (ei, key) in runs {
        if ctx.placed >= ctx.cap {
            ctx.reject("global-cap");
            break;
        }
        let e = &w.street.plan.entries[*ei];
        let mut reach = f64::INFINITY;
        for i in 0..e.points.len().saturating_sub(1) {
            let (a, b) = (e.points[i], e.points[i + 1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let l2 = dx * dx + dz * dz;
            let t = if l2 > 1e-9 { (((focus[0] - a[0]) * dx + (focus[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
            reach = reach.min((focus[0] - (a[0] + dx * t)).hypot(focus[1] - (a[1] + dz * t)));
        }
        if !(reach <= r.c.far_radius + e.length * 0.5) {
            ctx.reject("out-of-range");
            continue;
        }
        out.extend(plan_kerbs(w, *ei, key, &mut ctx));
    }
    (out, ctx.rejections)
}
