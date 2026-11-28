//! Street tree basins (`--tree-basins`; `set_tree_basins`): the square of
//! soil, mulch and edging every San Francisco street tree stands in, flush
//! with the sidewalk. One per tree (and palm) the furniture walk places on a
//! footway, centred on its trunk and squared to the kerb, sized by the
//! footway's surveyed width from SF Public Works Director's Order 187,246's
//! minimum basins (`street-tree-basin-sf`: 3 x 4 ft from a 7 ft 6 in
//! sidewalk, 4 x 4 ft from 12 ft, 5 x 5 ft from 13 ft; across the walk x
//! along the kerb). A tree on a footway narrower than the Order's minimum
//! (older plantings the Order no longer allows) gets the smallest basin.
//! Trees off a footway (medians, parks) get none: their ground is not paving.
//!
//! The basin is a flat panel like the detectable-warning pad (`pads.rs`):
//! laid at the highest of its four corners' drawn ground plus the street's
//! marking lift, so a cross slope never buries an edge, and tilted along the
//! kerb by the street's grade (the instance's `slope`, its local +Z along the
//! kerb). It is clear of the kerb: a basin that would reach past the curb
//! top is moved out along the walk to start at the curb's back, and its
//! `variant` keeps where the trunk stands in it (`encode`).

use crate::{Live, Model, Prop, PropsWorld, project, station_at};
use eo_street::plan::Entry;

static TREE_BASINS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set before the props world is built (eo-app, from `--tree-basins`).
pub fn set_tree_basins(on: bool) {
    TREE_BASINS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn tree_basins() -> bool {
    TREE_BASINS.load(std::sync::atomic::Ordering::Relaxed)
}

/// A basin's `variant`: bits 0-10 its grade along the kerb (`slope` x 1000 + 1000), bits
/// 11-14 the trunk's offset across the basin from its centre in steps of
/// `TRUNK_STEP_M`, bit 15 that offset's sign (local -X). The shader
/// (props_draw.wgsl `pr_basin`) draws the root flare and the watered soil round the trunk.
pub const TRUNK_STEP_M: f64 = 0.05;
const SLOPE_BITS: u16 = 0x7ff;

pub fn encode(slope: f64, trunk_x: f64) -> u16 {
    let g = ((slope * 1000.0).round() as i32 + 1000).clamp(0, SLOPE_BITS as i32) as u16;
    let k = ((trunk_x.abs() / TRUNK_STEP_M).round() as u16).min(15);
    g | (k << 11) | if trunk_x < 0.0 && k > 0 { 0x8000 } else { 0 }
}

/// The grade along the kerb a basin's `variant` carries.
pub fn slope_of(variant: u16) -> f64 {
    ((variant & SLOPE_BITS) as f64 - 1000.0) / 1000.0
}

/// The trunk's offset across the basin (its local X, m) a basin's `variant` carries.
pub fn trunk_of(variant: u16) -> f64 {
    let k = ((variant >> 11) & 15) as f64 * TRUNK_STEP_M;
    if variant & 0x8000 != 0 { -k } else { k }
}

/// The Order's minimum basin (across the walk, along the kerb; m) for a footway of
/// surveyed width `walk` (m): the largest class it reaches, else the smallest.
pub fn basin_size(w: &PropsWorld, walk: f64) -> [f64; 2] {
    let t = &w.std.tree_basin;
    let mut size = [t[0][1], t[0][2]];
    for c in t {
        if walk >= c[0] {
            size = [c[1], c[2]];
        }
    }
    size
}

/// The basins of the trees and palms in `props` that stand on a footway of ribbon `e`.
pub fn entry_basins(w: &PropsWorld, e: &Entry, props: &[Prop], out: &mut Vec<Prop>) {
    let o = &w.street.o;
    let scale = eo_data::groundscale::scale();
    for t in props.iter().filter(|p| matches!(p.model, Model::Tree | Model::Palm)) {
        let Some((s, _, lat)) = project(e, t.pos[0], t.pos[2]) else { continue };
        let k = if lat >= 0.0 { 0 } else { 1 };
        let walk = e.walks[k];
        let off = lat.abs();
        // on this ribbon's footway (the drawn one: the plan is the survey times `scale`).
        if !(walk > 0.0) || off < e.half || off > e.half + walk {
            continue;
        }
        let [across, along] = basin_size(w, walk / scale);
        let f = station_at(e, s, false);
        let side = if k == 0 { 1.0 } else { -1.0 };
        // clear of the kerb: the basin starts no nearer the street than the curb's back.
        let inner = e.half + o.curb_top_width;
        let centre = off.max(inner + across / 2.0).min(e.half + walk - across / 2.0).max(inner + across / 2.0);
        let dl = (centre - off) * side;
        let (cx, cz) = (t.pos[0] + f.nx * dl, t.pos[2] + f.nz * dl);
        // highest corner, and the grade along the kerb (+t) between the ends.
        let (ha, hc) = (along / 2.0, across / 2.0);
        let at = |a: f64, c: f64| w.ground(cx + f.tx * a + f.nx * c, cz + f.tz * a + f.nz * c);
        let corners = [(-ha, at(-ha, -hc)), (-ha, at(-ha, hc)), (ha, at(ha, -hc)), (ha, at(ha, hc))];
        let (back, front) = (at(-ha, 0.0), at(ha, 0.0));
        let slope = if along > 0.1 { ((front - back) / along).clamp(-0.35, 0.35) } else { 0.0 };
        // the panel's centre at its own mid-height over the tilt, no corner's
        // ground over it (the panel at `a` along the kerb is `y + a * slope`;
        // where the corners stand on two surfaces, the higher one's).
        let y = corners.iter().map(|&(a, g)| g - a * slope).fold(f64::MIN, f64::max) + o.marking_lift;
        // local +Z along the kerb (the grade tilts it), local X across the walk.
        let yaw = f.tx.atan2(f.tz);
        let (sy, cy) = yaw.sin_cos();
        let trunk_x = (t.pos[0] - cx) * cy - (t.pos[2] - cz) * sy;
        out.push(Prop {
            model: Model::TreeBasin,
            pos: [cx, y, cz],
            yaw,
            scale: [across as f32, 1.0, along as f32],
            variant: encode(slope, trunk_x),
            live: Live::None,
        });
    }
}
