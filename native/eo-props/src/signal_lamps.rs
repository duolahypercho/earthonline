//! Combination poles (eo-app `--no-signal-luminaires` turns them off; off by default outside
//! eo-app): a district street light the walk stood beside a signal mast pole stands on the
//! mast pole instead, so one pole carries the signals and the light. SF Better Streets Plan,
//! Utilities, Consolidation: "Street lighting, traffic signal, and MUNI catenary poles should
//! share poles wherever possible" (https://sfbetterstreets.org/design-guidelines/utilities/);
//! the SFPUC Streetlight Guidelines allow an SFMTA signal on a streetlight pole.
//!
//! The light keeps its fixture, its mounting height and its orientation and moves no further
//! than the guidelines' like-for-like change (`streetlight-sf-like-for-like-move`: within 10 ft;
//! [`crate::Standards::lamp_share_move_max`]), so its photometry is the walk's light's, from
//! the pole's spot: [`crate::furniture::Furniture::moved_lamp`]. The lamp's own post stands
//! inside the mast pole (drawn without its base, eo-app), the pole reaching the post's top
//! ([`LAMP_POST_TOP_M`]) so the lamp's arm and head sit on it.
//!
//! Which light: the cobra and banner lights (a historic column carries its lantern on its own
//! top and stays); one on the mast pole's side of its own ribbon, at the end of the ribbon it
//! stands nearer (each light has one junction it may move to); the nearest pairs first, each
//! pole and each light taken once. A move that would leave another street light within the
//! lamp kind's gap of the pole (`STREET_FURNITURE_KINDS[].gap`, the walk's least spacing of two
//! of a kind: a corner with two lights) is not made. A light already standing on a trolley
//! support pole (`overhead::Overhead::lamp_moves`) stays there.

use crate::furniture::LampLight;
use crate::{Model, Prop, PropsWorld};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static SIGNAL_LUMINAIRES: AtomicBool = AtomicBool::new(false);

/// Turn the combination poles on or off for the plans that follow (eo-app, at each world load).
pub fn set_signal_luminaires(on: bool) {
    SIGNAL_LUMINAIRES.store(on, Ordering::Relaxed);
}

pub fn signal_luminaires() -> bool {
    SIGNAL_LUMINAIRES.load(Ordering::Relaxed)
}

/// Where the district lights' arm fitting starts over their foot: the sleeve clamped round the
/// post's top that carries the arm (`lampCobra-v2`, `lampBanner-v2`, built by
/// scripts/assets/build-street-props-v2.py; eo-render's signal-pole test reads it from the
/// GLBs). A mast pole carrying a light reaches at least this high (times the lamp's height
/// scale), so the post stays inside it and the sleeve, the arm and the post's cap stand on it.
pub const LAMP_POST_TOP_M: f64 = 6.8;

/// A street light standing on a signal mast pole.
#[derive(Clone, Debug)]
pub struct PoleLamp {
    /// The ribbon whose walk placed it and its index in that walk's items.
    pub entry: u32,
    pub item: usize,
    /// Where the walk stood it (its post's foot).
    pub from: [f64; 3],
    /// The mast pole's foot.
    pub pole: [f64; 3],
    /// The light on the pole's foot: its own fixture, yaw and scale.
    pub prop: Prop,
    pub light: Option<LampLight>,
}

/// The street lights that stand on a signalised junction's mast poles (none when off).
pub fn node_lamps(w: &PropsWorld, ni: usize) -> Arc<Vec<PoleLamp>> {
    if !signal_luminaires() {
        return Arc::new(vec![]);
    }
    static CACHE: OnceLock<Mutex<HashMap<(usize, u32), Arc<Vec<PoleLamp>>>>> = OnceLock::new();
    let key = (Arc::as_ptr(&w.street) as usize, ni as u32);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap().get(&key) {
        return v.clone();
    }
    let v = Arc::new(plan(w, ni));
    let mut c = cache.lock().unwrap();
    if c.len() > 8192 {
        c.clear();
    }
    c.insert(key, v.clone());
    v
}

/// The lights the walk of ribbon `ei` placed that stand on a mast pole instead (by item index).
pub fn entry_moves(w: &PropsWorld, ei: usize) -> Vec<PoleLamp> {
    if !signal_luminaires() {
        return vec![];
    }
    let e = &w.street.plan.entries[ei];
    let mut out = vec![];
    for (ni, _) in [e.approach_start, e.approach_end].into_iter().flatten() {
        if !w.street.plan.nodes[ni].signalised() {
            continue;
        }
        out.extend(node_lamps(w, ni).iter().filter(|l| l.entry as usize == ei).cloned());
    }
    out
}

fn plan(w: &PropsWorld, ni: usize) -> Vec<PoleLamp> {
    let node = &w.street.plan.nodes[ni];
    if !node.signalised() {
        return vec![];
    }
    let reach = w.std.lamp_share_move_max;
    let f = w.furniture();
    let poles: Vec<[f64; 3]> = w.signal_node_props(ni).iter().filter(|p| p.model == Model::SignalPole && p.variant == 1).map(|p| p.pos).collect();
    if poles.is_empty() {
        return vec![];
    }
    // the lights already on a trolley support pole, by (ribbon, item): where they stand
    let trolley = |o: usize| -> Vec<(usize, [f64; 3])> {
        if !crate::overhead::lamp_standards() {
            return vec![];
        }
        crate::overhead::shared(w).lamp_moves(w, o as u32).iter().map(|m| (m.item, m.to)).collect()
    };
    // the candidates: a cobra or banner light of an approach's ribbon, at this junction's end of
    // it, on a pole's side of it, within the reach
    let mut entries: Vec<usize> = node.approaches.iter().map(|a| a.entry).collect();
    entries.sort_unstable();
    entries.dedup();
    let mut cands: Vec<(f64, usize, usize, usize)> = vec![]; // (distance, pole, ribbon, item)
    for &ei in &entries {
        let e = &w.street.plan.entries[ei];
        let total = e.cum.last().copied().unwrap_or(0.0);
        let near_end = |station: f64| if station <= total / 2.0 { e.approach_start } else { e.approach_end };
        let moved = trolley(ei);
        let fe = f.entry(w, ei);
        for (ii, it) in fe.items.iter().enumerate() {
            if it.entry as usize != ei || !matches!(f.kind_id(it.kind), "lampCobra" | "lampBanner") || moved.iter().any(|m| m.0 == ii) {
                continue;
            }
            if near_end(it.station).map(|(n, _)| n) != Some(ni) {
                continue;
            }
            let side = crate::project(e, it.x, it.z).map(|r| r.2.signum());
            for (pi, p) in poles.iter().enumerate() {
                let d = (p[0] - it.x).hypot(p[2] - it.z);
                if d <= reach && side.is_some() && crate::project(e, p[0], p[2]).map(|r| r.2.signum()) == side {
                    cands.push((d, pi, ei, ii));
                }
            }
        }
    }
    if cands.is_empty() {
        return vec![];
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
    // every other street light near the poles, where it stands (a trolley pole's on the pole)
    let mut near: Vec<u32> = vec![];
    for p in &poles {
        let key = eo_street::tile_key(p[0], p[2]);
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(t) = w.street.tiles.get(&(key.0 + dx, key.1 + dz)) {
                    near.extend(t.spans.iter().map(|s| s.0));
                }
            }
        }
    }
    near.sort_unstable();
    near.dedup();
    let mut lights: Vec<(usize, usize, [f64; 2])> = vec![];
    for &o in &near {
        let o = o as usize;
        let moved = trolley(o);
        for (ii, it) in f.entry(w, o).items.iter().enumerate() {
            if it.entry as usize != o || !f.is_street_light(it.kind) {
                continue;
            }
            let at = moved.iter().find(|m| m.0 == ii).map_or([it.x, it.z], |m| [m.1[0], m.1[2]]);
            lights.push((o, ii, at));
        }
    }
    let mut pole_taken = vec![false; poles.len()];
    let mut taken: Vec<(usize, usize)> = vec![];
    let mut out = vec![];
    for &(_, pi, ei, ii) in &cands {
        if pole_taken[pi] || taken.contains(&(ei, ii)) {
            continue;
        }
        let it = &f.entry(w, ei).items[ii];
        let p = poles[pi];
        let gap = f.kinds[it.kind as usize].gap;
        // no second light at the pole's corner (the lights taken here stand on their poles)
        let crowded = lights.iter().any(|&(o, oi, q)| (o, oi) != (ei, ii) && !taken.contains(&(o, oi)) && (q[0] - p[0]).hypot(q[1] - p[2]) < gap)
            || out.iter().any(|l: &PoleLamp| (l.pole[0] - p[0]).hypot(l.pole[2] - p[2]) < gap);
        if crowded {
            continue;
        }
        let Some((prop, light)) = f.moved_lamp(it, p) else { continue };
        pole_taken[pi] = true;
        taken.push((ei, ii));
        out.push(PoleLamp { entry: ei as u32, item: ii, from: [it.x, it.y, it.z], pole: p, prop, light });
    }
    out
}
