//! Detectable-warning pads (street-surface-detail.js `emitRampPads`): one
//! rectangular panel per kerb ramp the street cuts, as wide as the ramp's flat
//! run and `detectable-warning-size` deep (CA standard: 3 ft from the back of
//! the curb), laid on the ramp: on a ribbon, one per resolved crossing-ramp
//! window (`segmentKerbRamps`, never a vehicle crossover); on a corner return,
//! one per group of path spans inside one ramp window, across the group's end
//! stations. A footway too narrow for the ramp (`walk <= curbTopWidth +
//! rampRun + 0.05`) carries none. The panel takes the ramp's running slope
//! (`scale` x/z, the slope in `variant` as mm per m).
//!
//! Differences from the JS, stated: the panel is a textured quad on the drawn
//! ramp (the JS lays a yellow field with modelled domes, a bedding joint and a
//! scored pour joint); the building-footprint clip and the forced chord pads
//! (`forcePadPairs`) are not ported.

use crate::{Live, Model, Prop, PropsWorld, station_at};
use eo_street::plan::Entry;

/// Lift over the drawn ramp: the JS paint lift (`markingLift`), so the panel never fights the ramp.
fn lift(w: &PropsWorld) -> f64 {
    w.street.o.marking_lift
}

fn pad(w: &PropsWorld, kerb_a: [f64; 2], kerb_b: [f64; 2], out: [f64; 2], out_props: &mut Vec<Prop>) {
    let depth = w.std.detectable_warning_depth;
    let width = (kerb_b[0] - kerb_a[0]).hypot(kerb_b[1] - kerb_a[1]);
    if !(width > 0.2) {
        return;
    }
    let ol = out[0].hypot(out[1]);
    if !(ol > 1e-9) {
        return;
    }
    let out = [out[0] / ol, out[1] / ol];
    let mid = [(kerb_a[0] + kerb_b[0]) / 2.0, (kerb_a[1] + kerb_b[1]) / 2.0];
    let c = [mid[0] + out[0] * depth / 2.0, mid[1] + out[1] * depth / 2.0];
    // A divided road can cut an obsolete return out of the planned footway.
    // Its panel cannot stand in the travel lane. Keep a panel spanning the
    // edge of a surviving ramp, including a footway below an upper road.
    if w.street.o.pavement_repairs && w.street.only_carriageway_at(c[0], c[1]) {
        let across = [out[1] * width / 2.0, -out[0] * width / 2.0];
        let along = [out[0] * depth / 2.0, out[1] * depth / 2.0];
        let corner_on_road = |side: f64, end: f64| {
            w.street.only_carriageway_at(
                c[0] + side * across[0] + end * along[0],
                c[1] + side * across[1] + end * along[1],
            )
        };
        if [-1.0, 1.0].into_iter().all(|side| [-1.0, 1.0].into_iter().all(|end| corner_on_road(side, end))) {
            return;
        }
    }
    // Running slope of the drawn ramp across the panel's depth.
    let near = w.ground(mid[0] + out[0] * 0.05, mid[1] + out[1] * 0.05);
    let far = w.ground(mid[0] + out[0] * (depth - 0.05), mid[1] + out[1] * (depth - 0.05));
    let slope = ((far - near) / (depth - 0.1)).clamp(-0.2, 0.2);
    let y = w.ground(c[0], c[1]) + lift(w);
    out_props.push(Prop {
        model: Model::TactilePad,
        pos: [c[0], y, c[1]],
        yaw: out[0].atan2(out[1]),
        scale: [width as f32, 1.0, depth as f32],
        variant: ((slope * 1000.0).round() as i32 + 1000) as u16,
        live: Live::None,
    });
}

/// Pads on a ribbon's crossing ramps.
pub fn entry_pads(w: &PropsWorld, e: &Entry, out: &mut Vec<Prop>) {
    let o = &w.street.o;
    let reach = o.curb_top_width + o.ramp_run;
    let wins = eo_street::plan::segment_kerb_ramps(e, o);
    for k in 0..2 {
        let side = if k == 0 { 1.0 } else { -1.0 };
        if !(e.walks[k] > reach + 0.05) {
            continue;
        }
        for win in &wins[k] {
            if !(win.hi > win.lo) {
                continue;
            }
            let (fa, fb) = (station_at(e, win.lo, false), station_at(e, win.hi, false));
            let ka = [fa.x + fa.nx * side * e.half, fa.z + fa.nz * side * e.half];
            let kb = [fb.x + fb.nx * side * e.half, fb.z + fb.nz * side * e.half];
            let fm = station_at(e, (win.lo + win.hi) / 2.0, false);
            pad(w, ka, kb, [fm.nx * side, fm.nz * side], out);
        }
    }
}

/// Pads on a junction's corner-return ramps.
pub fn node_pads(w: &PropsWorld, ni: usize, out: &mut Vec<Prop>) {
    let o = &w.street.o;
    let reach = o.curb_top_width + o.ramp_run;
    let n = &w.street.plan.nodes[ni];
    for path in &n.paths {
        if path.ramps.is_empty() {
            continue;
        }
        let st = &path.stations;
        // Group consecutive spans whose middle angle lies in one window.
        let mut groups: Vec<(usize, usize, usize)> = vec![];
        for i in 0..st.len().saturating_sub(1) {
            let (a, b) = (st[i], st[i + 1]);
            let (Some(aa), Some(bb)) = (a.ang.or(b.ang), b.ang.or(a.ang)) else { continue };
            if !(a.walk > reach + 0.05) && !(b.walk > reach + 0.05) {
                continue;
            }
            let mid = (aa + bb) / 2.0;
            let Some(wi) = path.ramps.iter().position(|w| mid > w.lo - 1e-9 && mid < w.hi + 1e-9) else { continue };
            if (a.x - b.x).hypot(a.z - b.z) < 1e-6 {
                continue;
            }
            match groups.iter_mut().find(|g| g.0 == wi) {
                Some(g) => g.2 = i + 1,
                None => groups.push((wi, i, i + 1)),
            }
        }
        for (_, lo, hi) in groups {
            let (a, b) = (st[lo], st[hi]);
            let outv = [(a.out[0] + b.out[0]) / 2.0, (a.out[1] + b.out[1]) / 2.0];
            pad(w, [a.x, a.z], [b.x, b.z], outv, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lake_merced_crossing_keeps_its_ramp_panels_out_of_the_travel_lane() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let w = crate::load_world(&paths).unwrap();
        let at = |x, z| eo_data::groundscale::plan([x, z]);
        let [x, z] = at(-5926.18919601, 5346.51141757);
        let key = eo_street::tile_key(x, z);
        let mut panels = Vec::new();
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(tile) = w.street.tiles.get(&(key.0 + dx, key.1 + dz)) {
                    for &node in &tile.nodes {
                        node_pads(&w, node as usize, &mut panels);
                    }
                }
            }
        }
        let entries = w.street.plan.entries.iter().filter(|e| e.id == "sf-seg-1420" || e.id == "sf-seg-536");
        for e in entries {
            entry_pads(&w, e, &mut panels);
        }
        let has_panel = |p: [f64; 2]| panels.iter().any(|q| (q.pos[0] - p[0]).hypot(q.pos[2] - p[1]) < 0.02);
        // Two surveyed carriageways and their adjoining corner at Middlefield Drive.
        for p in [at(-5924.4392, 5347.7216), at(-5925.8072, 5345.5152), at(-5922.3536, 5348.008)] {
            assert!(w.street.only_carriageway_at(p[0], p[1]) && !has_panel(p), "panel over asphalt at {p:?}");
        }
        for p in [at(-5921.9224, 5333.760), at(-5931.096, 5359.296)] {
            assert!(has_panel(p) && w.street.paved_layer_at(p[0], p[1]) == Some(eo_street::Layer::Ramp), "missing ramp panel at {p:?}");
        }
    }
}
