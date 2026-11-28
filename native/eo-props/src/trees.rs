//! Street trees from the DataSF street tree list (street-furniture.js
//! `placeSurveyedTrees`, `surveyedTreeCandidates`, `lateralFor`, `placeItem`):
//! the list's point gives the station along the ribbon it stands beside; the
//! trunk goes into the ribbon's own furnishing zone (`lateralFor`, curb zone:
//! back of curb + depth + 0.06 m); a station in a kerb ramp's reserved span
//! (`kerbRampClearM` past its flares), a trunk that would block the
//! through-route, stand on carriageway or inside a building is refused; a slot
//! taken by another tree slides along the kerb by `SURVEYED_KERB_SHIFTS`.
//! Size from the list's crown and height (decimetres), else the kind's height
//! with the JS's per-tree scale draw (`<seed>:tree:<listId>`).
//!
//! Differences from the JS, stated: a list tree several ribbons could take is
//! offered only to the ribbon whose centreline is nearest (the JS takes it on
//! the first ribbon its window furnishes); the occupancy test sees the trees of
//! the same ribbon only (the JS also sees the other furniture it placed
//! before); the alley tree fit, the yard trees, observed overlays and the
//! genus crown scaling are not ported (the size is the list's own).

use crate::{Live, Model, Prop, PropsWorld, StreetRandom, project, station_at};

pub struct TreeRecord {
    pub list_id: u32,
    pub x: f64,
    pub z: f64,
    pub form: String,
    pub crown_m: Option<f64>,
    pub height_m: Option<f64>,
    /// The list's genus class (`trees.species`: "Platanus", ..., "other").
    pub genus: String,
}

pub fn record(w: &PropsWorld, i: u32) -> TreeRecord {
    let t = &w.js.trees;
    let o = i as usize * t.stride;
    let d = &t.data;
    TreeRecord {
        list_id: i,
        x: d[o] as f64 / 10.0,
        z: d[o + 1] as f64 / 10.0,
        form: t.forms.get(d[o + 2] as usize).cloned().unwrap_or_else(|| "tall".into()),
        crown_m: (t.stride > 3 && d[o + 3] > 0).then(|| d[o + 3] as f64 / 10.0),
        height_m: (t.stride > 4 && d[o + 4] > 0).then(|| d[o + 4] as f64 / 10.0),
        genus: if t.stride > 5 { t.species.get(d[o + 5] as usize).cloned().unwrap_or_default() } else { String::new() },
    }
}

/// `treeSurveyCovers(segment)`.
fn covers(w: &PropsWorld, ei: usize) -> bool {
    let e = &w.street.plan.entries[ei];
    let seg = w.segment_of(e);
    if !seg.street_id.as_deref().unwrap_or("").starts_with("sf-street-") {
        return false;
    }
    let b = w.js.trees.bounds;
    e.points.iter().all(|q| q[0] >= b.min_x && q[0] <= b.max_x && q[1] >= b.min_z && q[1] <= b.max_z)
}

/// The list trees within `reach` of a ribbon (`surveyedTreesAlong`).
fn along(w: &PropsWorld, ei: usize, reach: f64) -> Vec<u32> {
    let e = &w.street.plan.entries[ei];
    let cell = crate::SURVEYED_CELL_M;
    let span = (reach / cell).ceil() as i32;
    let step = cell / 2.0;
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    let mut s = 0.0;
    while s <= e.length + 1e-6 {
        let st = station_at(e, s.min(e.length), false);
        let (cx, cz) = ((st.x / cell).floor() as i32, (st.z / cell).floor() as i32);
        for a in -span..=span {
            for b in -span..=span {
                if seen.insert((cx + a, cz + b)) {
                    if let Some(v) = w.tree_cells.get(&(cx + a, cz + b)) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        s += step;
    }
    out
}

/// The ribbon nearest a list tree among those near it (its owner).
fn owner(w: &PropsWorld, id: u32, x: f64, z: f64) -> Option<usize> {
    if let Some(&o) = w.tree_owner.lock().unwrap().get(&id) {
        return (o != u32::MAX).then_some(o as usize);
    }
    let o = owner_of(w, x, z);
    let mut c = w.tree_owner.lock().unwrap();
    if c.len() > 1 << 20 {
        c.clear();
    }
    c.insert(id, o.map(|v| v as u32).unwrap_or(u32::MAX));
    o
}

fn owner_of(w: &PropsWorld, x: f64, z: f64) -> Option<usize> {
    let cells = crate::signs::entry_cells(w);
    let key = ((x / 32.0).floor() as i32, (z / 32.0).floor() as i32);
    let mut best: Option<(f64, usize)> = None;
    for &ei in cells.get(&key)? {
        let e = &w.street.plan.entries[ei as usize];
        if let Some((_, d, _)) = project(e, x, z) {
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, ei as usize));
            }
        }
    }
    best.map(|b| b.1)
}

/// The street trees and palms of one ribbon: the furniture walk's (furniture.rs,
/// one occupancy with every other kind). // props2
pub fn plan_entry(w: &PropsWorld, ei: usize) -> Vec<Prop> {
    w.furniture().entry(w, ei).props.iter().filter(|p| matches!(p.model, Model::Tree | Model::Palm)).copied().collect()
}

/// The list trees alone, on a ribbon of their own (the port before the walk; kept for comparison). // props2
pub fn plan_entry_alone(w: &PropsWorld, ei: usize) -> Vec<Prop> {
    let mut out: Vec<Prop> = vec![];
    if !covers(w, ei) {
        return out;
    }
    let e = &w.street.plan.entries[ei];
    let (tree_kind, palm_kind) = (&w.js.consts["furnitureKinds"]["tree"], &w.js.consts["furnitureKinds"]["palm"]);
    let kf = |k: &serde_json::Value, f: &str| k[f].as_f64().unwrap_or(0.0);
    let ramp_clear = w.js.num("furnitureOptions", "kerbRampClearM");
    let shifts = w.js.list("surveyedKerbShifts");
    let s0 = e.trim_start + 1.2;
    let s1 = e.length - e.trim_end - 1.2;
    if !(s1 > s0) {
        return out;
    }
    let resolved = eo_street::plan::segment_kerb_ramps(e, &w.street.o);
    for side in [1.0f64, -1.0] {
        let Some(band) = w.band(e, side, w.std.throughway_min) else { continue };
        let reach = band.outer + 1.5;
        let wins = &resolved[if side > 0.0 { 0 } else { 1 }];
        let ramp_blocked = |s: f64| {
            wins.iter().any(|win| {
                let lo = (if win.flare_lo.is_finite() { win.flare_lo.min(win.lo) } else { win.lo }) - ramp_clear;
                let hi = (if win.flare_hi.is_finite() { win.flare_hi.max(win.hi) } else { win.hi }) + ramp_clear;
                s > lo && s < hi
            })
        };
        let mut cands: Vec<(f64, TreeRecord)> = vec![];
        for id in along(w, ei, reach) {
            let t = record(w, id);
            let Some((station, _, lateral)) = project(e, t.x, t.z) else { continue };
            if station < s0 || station > s1 {
                continue;
            }
            if lateral.abs() > reach || lateral.abs() < e.half - 3.0 {
                continue;
            }
            if (if lateral >= 0.0 { 1.0 } else { -1.0 }) != side {
                continue;
            }
            if owner(w, t.list_id, t.x, t.z) != Some(ei) {
                continue;
            }
            cands.push((station, t));
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut placed: Vec<[f64; 3]> = vec![];
        for (station, t) in cands {
            let palm = t.form == "palm";
            let kind = if palm { palm_kind } else { tree_kind };
            let (depth, radius) = (kf(kind, "depth"), kf(kind, "radius"));
            let through = kind["through"].as_f64().unwrap_or(w.js.num("furnitureOptions", "throughRoute"));
            // lateralFor, curb zone.
            let lateral = band.inner + depth + 0.06;
            let limit = band.outer - through;
            for (k, &shift) in shifts.iter().enumerate() {
                let at = station + shift;
                if at < s0 || at > s1 {
                    continue;
                }
                let mut overlap = false;
                let ok = (|| {
                    if ramp_blocked(at) {
                        return false;
                    }
                    if lateral - depth < band.inner - 0.05 || lateral + depth > band.outer + 0.05 || lateral + depth > limit + 1e-6 {
                        return false;
                    }
                    let st = station_at(e, at, false);
                    let (nx, nz) = (st.nx * st.miter, st.nz * st.miter);
                    let (x, z) = (st.x + nx * side * lateral, st.z + nz * side * lateral);
                    if placed.iter().any(|p| (p[0] - x).hypot(p[1] - z) < radius + p[2]) {
                        overlap = true;
                        return false;
                    }
                    if w.roadway.hit(x, z, depth).is_some() {
                        return false;
                    }
                    let b = &w.street.buildings;
                    if [(0.0, 0.0), (0.15, 0.0), (-0.15, 0.0), (0.0, 0.15), (0.0, -0.15)].iter().any(|(dx, dz)| b.contains(x + dx, z + dz)) {
                        return false;
                    }
                    let mut draw = StreetRandom::new(&format!("{}:tree:{}", w.seed, t.list_id));
                    let spin = (draw.next() - 0.5) * std::f64::consts::PI;
                    let scale = 0.82 + draw.next() * 0.5;
                    let facing = (side * nx).atan2(side * nz);
                    let y = w.ground(x, z);
                    let (model, sc) = if palm {
                        let h = t.height_m.unwrap_or(kf(kind, "height"));
                        (Model::Palm, [(h / 10.0) as f32; 3])
                    } else {
                        let h = t.height_m.unwrap_or(kf(kind, "height") * scale);
                        let c = t.crown_m.unwrap_or(h * 0.6);
                        (Model::Tree, [c as f32, h as f32, c as f32])
                    };
                    let form = w.js.trees.forms.iter().position(|f| *f == t.form).unwrap_or(0) as u16;
                    out.push(Prop { model, pos: [x, y, z], yaw: facing + spin, scale: sc, variant: form, live: Live::None });
                    placed.push([x, z, radius]);
                    true
                })();
                if ok {
                    break;
                }
                // Only a slot taken by another object is worth sliding out of.
                if k == 0 && !overlap {
                    break;
                }
            }
        }
    }
    out
}
