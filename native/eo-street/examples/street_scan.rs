//! The street geometry scorecard: every street tile of the city, as the
//! game builds it without `--structures` (`street_scan_joined`: with them,
//! as `--play` does), against the ground drawn around it.
//!
//! ```sh
//! cargo run --release -p eo-street --example street_scan -- [hits.csv]
//! cargo run --release -p eo-structures --example street_scan_joined -- [hits.csv] [--tunnels]   # as --play builds it
//! EO_STREET_GROUND=0 cargo run --release -p eo-street --example street_scan   # the ground before `ground`
//! ```
//!
//! The drawn ground is the 8 m carpet (`StreetWorld::carpet_y`) and, with the
//! street ground on (eo-street `ground`), the street ground's triangle where
//! `ground_overlay` draws one (`StreetWorld::overlay_height`).
//!
//! Counts, over the walkable paving (carriageway, kerb top, walk, ramp):
//!
//!   floating     paving more than 1 m (and 3 m) over the drawn ground under it (with
//!                the street ground on, the ground is not drawn under the paving: this
//!                is the 8 m carpet there; `cliff edge` is what shows)
//!   off survey   paving more than 3 m over (or under) the surveyed ground: a street
//!                standing on a fill (or in a cut) the survey does not hold
//!   poking       drawn ground more than 5 cm over the paving (ground showing through)
//!   steep road   carriageway steeper than 0.40 rise/run (SF's steepest street is 0.315)
//!   steep walk   walk steeper than 1.0 (45 degrees: a stair drawn as a walk)
//!   cliff edge   a paving edge standing more than 2 m over the open ground 0.6 m past it
//!   pits         open ground (no paving, no building within 2 m) drawn more than 2 m
//!                under the surveyed ground (the elevation grid and the carpet's lift)
//!   sunken gaps  open ground enclosed by paving (within 4 m in 7 of 8 directions)
//!                drawn more than 15 cm under the paving round it: a hole in a junction
//!   junctions    3+-leg junctions whose paving stands more than 1 m (2 m) over
//!                the drawn ground within 6 m of the node
//!   junction pads on a 1 m grid over the hull of each 3+-leg junction's approach
//!                mouths: stacked paving (two walkable surfaces more than 20 cm apart
//!                at one point), holes (no paving inside the pad's own ring), steps
//!                (the carriageway's top breaking from its slope: a second difference
//!                over 1 m either side of more than 25 cm)
//!
//! Optional CSV of every hit (kind, x, z, y, magnitude, area or edge length, layer).

#[allow(unused_imports)]
use eo_data::{City, DataPaths, ElevationGrid};
#[allow(unused_imports)]
use eo_street::{BuildingIndex, Layer, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
struct Hit {
    kind: &'static str,
    x: f32,
    z: f32,
    y: f32,
    mag: f32,
    size: f32,
    layer: u8,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Tot {
    pub paving_m2: f64,
    pub floating1_m2: f64,
    pub floating3_m2: f64,
    pub over_survey3_m2: f64,
    pub under_survey3_m2: f64,
    pub poking_m2: f64,
    pub steep_road_m2: f64,
    pub steep_walk_m2: f64,
    pub edges_m: f64,
    pub cliff2_m: f64,
    pub open_m2: f64,
    pub pit2_m2: f64,
    pub stack_m2: f64,
    pub sunk_m2: f64,
}

/// The scorecard of a set of street tiles and the 3+-leg junctions in them
/// (`measure`): the paving's totals, the junctions' counts, every hit.
pub struct Card {
    pub t: Tot,
    pub tiles: usize,
    /// 3+-leg junctions measured; over the drawn ground >1 m, >2 m
    pub junctions: usize,
    pub junction_over1: usize,
    pub junction_over2: usize,
    /// junction pads: junctions with stacked paving, holes, carriageway steps, and their sums (m2, m2, m)
    pub junction_stack: usize,
    pub junction_hole: usize,
    pub junction_step: usize,
    pub junction_stack_m2: f64,
    pub junction_hole_m2: f64,
    pub junction_step_m: f64,
    pairs: Vec<(String, f64)>,
    hits: Vec<Hit>,
}

fn drawn_ground(w: &StreetWorld, x: f64, z: f64) -> f64 {
    let c = w.carpet_y(x, z);
    match w.overlay_height(x, z) {
        Some(o) => c.max(o),
        None => c,
    }
}

#[allow(dead_code)]
fn main() {
    let paths = DataPaths::discover().unwrap();
    let city = City::load(&paths).unwrap();
    let elev = ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let w = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    scan(&w, std::env::args().nth(1));
}

/// The scorecard of street world `w` (printed; every hit to the CSV at `out`).
/// (eo-structures `street_scan_joined` runs it on the street built with the
/// grade-separated roads' joins, as `--play` builds it.)
#[allow(dead_code)]
pub fn scan(w: &StreetWorld, out: Option<String>) {
    let card = measure(w, &w.tile_keys(), false);
    print_card(&card, out);
}

/// The scorecard over the street tiles `keys` and the city's 3+-leg
/// junctions: every one, or with `nodes_in_keys` only those whose node lies
/// in one of the tiles (a sample's own).
pub fn measure(w: &StreetWorld, keys: &[(i32, i32)], nodes_in_keys: bool) -> Card {
    let w = w;
    let keys = keys.to_vec();
    let key_set: std::collections::HashSet<(i32, i32)> = keys.iter().copied().collect();
    let node_here = |p: [f64; 2]| !nodes_in_keys || key_set.contains(&((p[0] / 128.0).floor() as i32, (p[1] / 128.0).floor() as i32));
    let hits: Mutex<Vec<Hit>> = Mutex::new(vec![]);
    let total: Mutex<Tot> = Mutex::new(Tot::default());
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8).min(16);
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                let (mut local, mut t) = (vec![], Tot::default());
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= keys.len() {
                        break;
                    }
                    let key = keys[i];
                    let m = w.tile_mesh(key);
                    let (x0, z0) = (key.0 as f64 * 128.0, key.1 as f64 * 128.0);
                    let mut edges: std::collections::HashMap<(u32, u32), (u32, u32)> = std::collections::HashMap::new();
                    for tri in 0..m.triangles() {
                        let l = m.layers[tri];
                        if !matches!(l, Layer::Carriageway | Layer::Sidewalk | Layer::CurbTop | Layer::Ramp) {
                            continue;
                        }
                        let ix = [m.indices[3 * tri], m.indices[3 * tri + 1], m.indices[3 * tri + 2]];
                        let p: Vec<[f64; 3]> = ix.iter().map(|&k| m.positions[k as usize].map(|v| v as f64)).collect();
                        let (u, v) = ([p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]], [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]]);
                        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                        if len < 1e-9 {
                            continue;
                        }
                        let area = 0.5 * len;
                        let slope = (n[0] * n[0] + n[2] * n[2]).sqrt() / n[1].abs().max(1e-6);
                        let c = [(p[0][0] + p[1][0] + p[2][0]) / 3.0, (p[0][1] + p[1][1] + p[2][1]) / 3.0, (p[0][2] + p[1][2] + p[2][2]) / 3.0];
                        t.paving_m2 += area;
                        let d = c[1] - drawn_ground(&w, c[0], c[2]);
                        let hit = |kind, mag: f64| Hit { kind, x: c[0] as f32, z: c[2] as f32, y: c[1] as f32, mag: mag as f32, size: area as f32, layer: l as u8 };
                        if d > 1.0 {
                            t.floating1_m2 += area;
                        }
                        if d > 3.0 {
                            t.floating3_m2 += area;
                            local.push(hit("floating", d));
                        }
                        let off = c[1] - (w.ground(c[0], c[2]) + w.terrain.carpet_lift);
                        if off > 3.0 {
                            t.over_survey3_m2 += area;
                        }
                        if off < -3.0 {
                            t.under_survey3_m2 += area;
                        }
                        if d < -0.05 {
                            t.poking_m2 += area;
                            local.push(hit("poking", -d));
                        }
                        if l == Layer::Carriageway && slope > 0.40 {
                            t.steep_road_m2 += area;
                            local.push(hit("steep_road", slope));
                        }
                        if l != Layer::Carriageway && l != Layer::Ramp && slope > 1.0 {
                            t.steep_walk_m2 += area;
                            local.push(hit("steep_walk", slope));
                        }
                        for e in 0..3 {
                            let (a, b) = (ix[e], ix[(e + 1) % 3]);
                            let k = if a < b { (a, b) } else { (b, a) };
                            edges.entry(k).or_insert((0, tri as u32)).0 += 1;
                        }
                    }
                    // open ground (no paving, no building within 2 m), on a 4 m grid:
                    // the drawn ground more than 2 m under the surveyed ground is a pit.
                    let mut gz = z0 + 2.0;
                    while gz < z0 + 128.0 {
                        let mut gx = x0 + 2.0;
                        while gx < x0 + 128.0 {
                            let near_building = (0..8).any(|q| {
                                let a = q as f64 * std::f64::consts::FRAC_PI_4;
                                w.buildings.contains(gx + 2.0 * a.cos(), gz + 2.0 * a.sin())
                            }) || w.buildings.contains(gx, gz);
                            if !near_building && w.paved_heights(gx, gz).is_empty() {
                                t.open_m2 += 16.0;
                                let real = w.ground(gx, gz) + w.terrain.carpet_lift;
                                let dug = real - drawn_ground(&w, gx, gz);
                                if dug > 2.0 {
                                    t.pit2_m2 += 16.0;
                                    local.push(Hit { kind: "pit", x: gx as f32, z: gz as f32, y: real as f32, mag: dug as f32, size: 16.0, layer: 255 });
                                }
                            }
                            gx += 4.0;
                        }
                        gz += 4.0;
                    }
                    // stacked paving anywhere, on a 2 m grid: two walkable surfaces 0.2-4 m
                    // apart at one point (a grade separation stands more than 4 m over).
                    // (and each point's top paving, for the sunken gaps below)
                    let mut tops = vec![f64::NAN; 64 * 64];
                    let mut gz = z0 + 1.0;
                    while gz < z0 + 128.0 {
                        let mut gx = x0 + 1.0;
                        while gx < x0 + 128.0 {
                            let ls = w.paved_layers(gx, gz);
                            if let Some(top) = ls.iter().map(|q| q.0).reduce(f64::max) {
                                tops[(((gz - z0) / 2.0) as usize) * 64 + ((gx - x0) / 2.0) as usize] = top;
                            }
                            if ls.len() >= 2 {
                                let top = ls.iter().cloned().fold((f64::MIN, Layer::Carriageway), |a, b| if b.0 > a.0 { b } else { a });
                                let low = ls.iter().filter(|q| top.0 - q.0 > 0.2 && top.0 - q.0 < 4.0).cloned().fold((f64::MAX, Layer::Carriageway), |a, b| if b.0 < a.0 { b } else { a });
                                if low.0 < f64::MAX {
                                    t.stack_m2 += 4.0;
                                    local.push(Hit { kind: "stack", x: gx as f32, z: gz as f32, y: top.0 as f32, mag: (top.0 - low.0) as f32, size: 4.0, layer: top.1 as u8 * 16 + low.1 as u8 });
                                }
                            }
                            gx += 2.0;
                        }
                        gz += 2.0;
                    }
                    // sunken gaps: open ground enclosed by paving (paving within 4 m in 7 of
                    // 8 directions: a sliver between two pavings, a hole in a junction)
                    // drawn more than 15 cm under the paving round it.
                    for j in 0..64usize {
                        for i in 0..64usize {
                            if !tops[j * 64 + i].is_nan() {
                                continue;
                            }
                            let (mut hit, mut sum) = (0, 0.0);
                            for (di, dj) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)] {
                                for k in 1..=2 {
                                    let (ii, jj) = (i as i32 + di * k, j as i32 + dj * k);
                                    if !(0..64).contains(&ii) || !(0..64).contains(&jj) {
                                        break;
                                    }
                                    let y = tops[jj as usize * 64 + ii as usize];
                                    if !y.is_nan() {
                                        hit += 1;
                                        sum += y;
                                        break;
                                    }
                                }
                            }
                            if hit >= 7 {
                                let (gx, gz) = (x0 + 1.0 + 2.0 * i as f64, z0 + 1.0 + 2.0 * j as f64);
                                let depth = sum / hit as f64 - drawn_ground(&w, gx, gz);
                                if depth > 0.15 {
                                    t.sunk_m2 += 4.0;
                                    local.push(Hit { kind: "sunken_gap", x: gx as f32, z: gz as f32, y: (sum / hit as f64) as f32, mag: depth as f32, size: 4.0, layer: 255 });
                                }
                            }
                        }
                    }
                    for (&(a, b), &(n, tri)) in &edges {
                        if n != 1 {
                            continue;
                        }
                        let (pa, pb) = (m.positions[a as usize], m.positions[b as usize]);
                        let (mx, my, mz) = ((pa[0] + pb[0]) as f64 / 2.0, (pa[1] + pb[1]) as f64 / 2.0, (pa[2] + pb[2]) as f64 / 2.0);
                        let el = ((pb[0] - pa[0]) as f64).hypot((pb[2] - pa[2]) as f64);
                        if el < 0.3 || (mx - x0).abs() < 0.2 || (mx - x0 - 128.0).abs() < 0.2 || (mz - z0).abs() < 0.2 || (mz - z0 - 128.0).abs() < 0.2 {
                            continue;
                        }
                        let ix = [m.indices[3 * tri as usize], m.indices[3 * tri as usize + 1], m.indices[3 * tri as usize + 2]];
                        let cx = ix.iter().map(|&k| m.positions[k as usize][0] as f64).sum::<f64>() / 3.0;
                        let cz = ix.iter().map(|&k| m.positions[k as usize][2] as f64).sum::<f64>() / 3.0;
                        let (mut ox, mut oz) = (-((pb[2] - pa[2]) as f64), (pb[0] - pa[0]) as f64);
                        let ol = ox.hypot(oz);
                        (ox, oz) = (ox / ol, oz / ol);
                        if (cx - mx) * ox + (cz - mz) * oz > 0.0 {
                            (ox, oz) = (-ox, -oz);
                        }
                        let (qx, qz) = (mx + ox * 0.6, mz + oz * 0.6);
                        t.edges_m += el;
                        if !w.paved_heights(qx, qz).is_empty() {
                            continue;
                        }
                        let drop = my - drawn_ground(&w, qx, qz);
                        if drop > 2.0 {
                            t.cliff2_m += el;
                            local.push(Hit { kind: "cliff_edge", x: mx as f32, z: mz as f32, y: my as f32, mag: drop as f32, size: el as f32, layer: m.layers[tri as usize] as u8 });
                        }
                    }
                }
                hits.lock().unwrap().extend(local);
                let mut g = total.lock().unwrap();
                g.paving_m2 += t.paving_m2;
                g.floating1_m2 += t.floating1_m2;
                g.floating3_m2 += t.floating3_m2;
                g.over_survey3_m2 += t.over_survey3_m2;
                g.under_survey3_m2 += t.under_survey3_m2;
                g.poking_m2 += t.poking_m2;
                g.steep_road_m2 += t.steep_road_m2;
                g.steep_walk_m2 += t.steep_walk_m2;
                g.edges_m += t.edges_m;
                g.cliff2_m += t.cliff2_m;
                g.open_m2 += t.open_m2;
                g.pit2_m2 += t.pit2_m2;
                g.stack_m2 += t.stack_m2;
                g.sunk_m2 += t.sunk_m2;
            });
        }
    });
    // junctions
    let (mut n_all, mut j1, mut j2) = (0usize, 0usize, 0usize);
    for n in &w.plan.nodes {
        if n.approaches.len() < 3 || !node_here(n.position) {
            continue;
        }
        n_all += 1;
        let (x, z) = (n.position[0], n.position[1]);
        let mut gap = f64::MIN;
        for q in 0..9 {
            let (px, pz) = if q == 8 { (x, z) } else {
                let a = q as f64 * std::f64::consts::FRAC_PI_4;
                (x + a.cos() * 6.0, z + a.sin() * 6.0)
            };
            if let Some(top) = w.paved_heights(px, pz).into_iter().reduce(f64::max) {
                gap = gap.max(top - drawn_ground(&w, px, pz));
            }
        }
        if gap > 1.0 {
            j1 += 1;
            hits.lock().unwrap().push(Hit { kind: "junction", x: x as f32, z: z as f32, y: 0.0, mag: gap as f32, size: n.approaches.len() as f32, layer: 0 });
        }
        if gap > 2.0 {
            j2 += 1;
        }
    }
    // junction surfaces: a 1 m grid over each 3+-leg junction's pad (the hull of
    // its approaches' mouths): stacked paving, holes, steps in the carriageway.
    let (mut j_stack, mut j_hole, mut j_step, mut stack_m2, mut hole_m2, mut step_m) = (0usize, 0usize, 0usize, 0.0f64, 0.0f64, 0.0f64);
    let nodes: Vec<usize> = (0..w.plan.nodes.len()).filter(|&i| w.plan.nodes[i].approaches.len() >= 3 && node_here(w.plan.nodes[i].position)).collect();
    let jn = AtomicUsize::new(0);
    let jres: Mutex<Vec<(usize, f64, f64, f64)>> = Mutex::new(vec![]);
    let pairs: Mutex<std::collections::BTreeMap<String, f64>> = Mutex::new(Default::default());
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let k = jn.fetch_add(1, Ordering::Relaxed);
                if k >= nodes.len() {
                    break;
                }
                let n = &w.plan.nodes[nodes[k]];
                let mut hull: Vec<[f64; 2]> = vec![];
                // the pad's own ring (mesh.rs `emit_node`): each approach's mouth
                // section, then the corner path's kerb-line stations to the next
                let mut ring: Vec<[f64; 2]> = vec![];
                // (`seams`) a slip merging alongside a through leg has no mouth in the pad, no corner either side
                let merge = |i: usize| n.approaches.get(i).is_some_and(|a| a.merge_trim.is_some() && eo_street::seams::road_seams());
                for (i, app) in n.approaches.iter().enumerate() {
                    let f = eo_street::plan::approach_end_frame(&w.plan.entries, app);
                    for u in [-app.half, app.half] {
                        hull.push(f.at(u));
                    }
                    let sgn = if app.at_start { 1.0 } else { -1.0 };
                    if !merge(i) {
                        for lat in w.o.section_offsets(app.half) {
                            ring.push(f.at(sgn * lat));
                        }
                    }
                    if let Some(path) = n.paths.get(i).filter(|p| !merge(p.a) && !merge(p.b)) {
                        let last = path.stations.len().saturating_sub(1);
                        for (q, st) in path.stations.iter().enumerate() {
                            if q != 0 && q != last {
                                ring.push([st.x, st.z]);
                            }
                        }
                    }
                }
                let hull = convex_hull(hull);
                if hull.len() < 3 || ring.len() < 3 {
                    continue;
                }
                let (mut x0, mut x1, mut z0, mut z1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
                for p in &hull {
                    x0 = x0.min(p[0]);
                    x1 = x1.max(p[0]);
                    z0 = z0.min(p[1]);
                    z1 = z1.max(p[1]);
                }
                let nx = ((x1 - x0).ceil() as usize).max(1);
                let nz = ((z1 - z0).ceil() as usize).max(1);
                let mut top = vec![f64::NAN; (nx + 1) * (nz + 1)];
                let (mut st, mut ho, mut sp) = (0.0, 0.0, 0.0);
                for j in 0..=nz {
                    for i in 0..=nx {
                        let (x, z) = (x0 + i as f64, z0 + j as f64);
                        if !in_hull(&hull, x, z) {
                            continue;
                        }
                        let hs = w.paved_heights(x, z);
                        if hs.is_empty() {
                            // a hole only inside the pad's own ring (the hull's
                            // corners past the kerb returns are walk or open ground)
                            if in_ring(&ring, x, z) {
                                ho += 1.0;
                            }
                            continue;
                        }
                        let (lo, hi) = hs.iter().fold((f64::MAX, f64::MIN), |a, &h| (a.0.min(h), a.1.max(h)));
                        if hi - lo > 0.2 {
                            st += 1.0;
                            let ls = w.paved_layers(x, z);
                            let top = ls.iter().cloned().fold((f64::MIN, Layer::Carriageway), |a, b| if b.0 > a.0 { b } else { a });
                            let bot = ls.iter().cloned().fold((f64::MAX, Layer::Carriageway), |a, b| if b.0 < a.0 { b } else { a });
                            *pairs.lock().unwrap().entry(format!("{:?} over {:?}", top.1, bot.1)).or_insert(0.0) += 1.0;
                        }
                        if w.paved_layer_at(x, z) == Some(Layer::Carriageway) {
                            top[j * (nx + 1) + i] = hi;
                        }
                    }
                }
                // a step: the carriageway's top breaking from its own slope (the
                // second difference over 1 m either side more than 25 cm), not a grade
                for j in 0..=nz {
                    for i in 0..=nx {
                        let b = top[j * (nx + 1) + i];
                        if b.is_nan() {
                            continue;
                        }
                        for (di, dj) in [(1usize, 0usize), (0, 1)] {
                            if i >= di && j >= dj && i + di <= nx && j + dj <= nz {
                                let a = top[(j - dj) * (nx + 1) + i - di];
                                let c = top[(j + dj) * (nx + 1) + i + di];
                                if !a.is_nan() && !c.is_nan() && (a - 2.0 * b + c).abs() > 0.25 {
                                    sp += 1.0;
                                    hits.lock().unwrap().push(Hit { kind: "step_at", x: (x0 + i as f64) as f32, z: (z0 + j as f64) as f32, y: b as f32, mag: (a - 2.0 * b + c).abs() as f32, size: 1.0, layer: 0 });
                                }
                            }
                        }
                    }
                }
                jres.lock().unwrap().push((nodes[k], st, ho, sp));
            });
        }
    });
    for &(ni, st, ho, sp) in jres.lock().unwrap().iter() {
        let n = &w.plan.nodes[ni];
        if st > 2.0 {
            j_stack += 1;
            hits.lock().unwrap().push(Hit { kind: "junction_stack", x: n.position[0] as f32, z: n.position[1] as f32, y: 0.0, mag: st as f32, size: st as f32, layer: 0 });
        }
        if ho > 2.0 {
            j_hole += 1;
            hits.lock().unwrap().push(Hit { kind: "junction_hole", x: n.position[0] as f32, z: n.position[1] as f32, y: 0.0, mag: ho as f32, size: ho as f32, layer: 0 });
        }
        if sp > 0.0 {
            j_step += 1;
            hits.lock().unwrap().push(Hit { kind: "junction_step", x: n.position[0] as f32, z: n.position[1] as f32, y: 0.0, mag: sp as f32, size: sp as f32, layer: 0 });
        }
        stack_m2 += st;
        hole_m2 += ho;
        step_m += sp;
    }
    let t = total.into_inner().unwrap();
    let mut pv: Vec<(String, f64)> = pairs.into_inner().unwrap().into_iter().collect();
    pv.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    Card {
        t,
        tiles: keys.len(),
        junctions: n_all,
        junction_over1: j1,
        junction_over2: j2,
        junction_stack: j_stack,
        junction_hole: j_hole,
        junction_step: j_step,
        junction_stack_m2: stack_m2,
        junction_hole_m2: hole_m2,
        junction_step_m: step_m,
        pairs: pv,
        hits: hits.into_inner().unwrap(),
    }
}

/// Print a scorecard (and every hit to the CSV at `out`).
#[allow(dead_code)]
pub fn print_card(c: &Card, out: Option<String>) {
    let t = c.t;
    let (keys, n_all, j1, j2) = (vec![(); c.tiles], c.junctions, c.junction_over1, c.junction_over2);
    let (j_stack, j_hole, j_step, stack_m2, hole_m2, step_m) = (c.junction_stack, c.junction_hole, c.junction_step, c.junction_stack_m2, c.junction_hole_m2, c.junction_step_m);
    println!("street ground: {}", if eo_street::ground::street_ground_on() { "on" } else { "off (the 8 m carpet)" });
    println!("tiles {}, paving {:.2} km2, paving edges {:.0} km", keys.len(), t.paving_m2 / 1e6, t.edges_m / 1e3);
    println!("floating over the drawn ground  >1 m {:>9.0} m2   >3 m {:>9.0} m2", t.floating1_m2, t.floating3_m2);
    println!("off the survey >3 m: over {:>9.0} m2   under {:>9.0} m2", t.over_survey3_m2, t.under_survey3_m2);
    println!("ground poking through (>5 cm)       {:>9.0} m2", t.poking_m2);
    println!("steep road (>0.40)                  {:>9.0} m2", t.steep_road_m2);
    println!("steep walk (>1.0)                   {:>9.0} m2", t.steep_walk_m2);
    println!("cliff edges (>2 m)                  {:>9.0} m", t.cliff2_m);
    println!("pits: open ground dug >2 m under the survey {:>9.0} m2 of {:.2} km2 open ground in street tiles", t.pit2_m2, t.open_m2 / 1e6);
    {
        let mut by: std::collections::BTreeMap<u8, f64> = Default::default();
        for h in c.hits.iter().filter(|h| h.kind == "stack") {
            *by.entry(h.layer).or_insert(0.0) += h.size as f64;
        }
        let mut by: Vec<(u8, f64)> = by.into_iter().collect();
        by.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let name = |l: u8| ["Carriageway", "CurbFace", "CurbTop", "Sidewalk", "Ramp", "Verge", "Marking", "Ground"].get(l as usize).copied().unwrap_or("?");
        println!("stacked paving (0.2-4 m apart, 2 m grid)  {:>9.0} m2: {}", t.stack_m2, by.iter().take(6).map(|(k, v)| format!("{} over {} {v:.0}", name(k / 16), name(k % 16))).collect::<Vec<_>>().join(", "));
    }
    println!("sunken gaps (open ground enclosed by paving, >15 cm under it, 2 m grid) {:>9.0} m2", t.sunk_m2);
    println!("junctions (3+ legs) {n_all}: over the ground >1 m {j1}, >2 m {j2}");
    println!("junction stacks by surfaces (m2): {}", c.pairs.iter().take(8).map(|(k, v)| format!("{k} {v:.0}")).collect::<Vec<_>>().join(", "));
    println!("junction pads: stacked paving (>20 cm apart) at {j_stack} junctions ({stack_m2:.0} m2), holes at {j_hole} ({hole_m2:.0} m2), carriageway steps (second difference >25 cm over 1 m) at {j_step} ({step_m:.0} m)");
    if let Some(path) = out {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "kind,x,z,y,mag,size,layer").unwrap();
        for h in &c.hits {
            writeln!(f, "{},{:.2},{:.2},{:.2},{:.3},{:.3},{}", h.kind, h.x, h.z, h.y, h.mag, h.size, h.layer).unwrap();
        }
        println!("wrote {path}");
    }
}

/// Monotone chain convex hull (x, z).
fn convex_hull(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.sort_by(|a, b| a.partial_cmp(b).unwrap());
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut h: Vec<[f64; 2]> = vec![];
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &[f64; 2]>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for &q in it {
            while h.len() >= start + 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

fn in_hull(h: &[[f64; 2]], x: f64, z: f64) -> bool {
    (0..h.len()).all(|i| {
        let (a, b) = (h[i], h[(i + 1) % h.len()]);
        (b[0] - a[0]) * (z - a[1]) - (b[1] - a[1]) * (x - a[0]) >= 0.0
    })
}

/// Even-odd point in polygon (x, z).
fn in_ring(r: &[[f64; 2]], x: f64, z: f64) -> bool {
    let mut inside = false;
    let mut j = r.len() - 1;
    for i in 0..r.len() {
        let (a, b) = (r[i], r[j]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}
