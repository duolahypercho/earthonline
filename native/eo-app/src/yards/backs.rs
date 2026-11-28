//! The backs of the houses round the rear yards (`--yard-detail` with the
//! canopy map; `--no-yard-stairs` leaves them out): what a yard looks up at.
//! Each kit building beside a surveyed yard is planned again as its tiles are
//! (the shared kit city, eo-render `kit::shared_kit_city`, at the floor its
//! tiles take, `kit_floor`), and on each back wall that opens onto a yard:
//!
//! * wooden back stairs: a landing and a back door at each flat's floor (one
//!   door for a single house), in a column of the wall no window of those
//!   floors crosses; switchback flights between the floors, two a floor, side
//!   by side in two lanes; then down to the yard in more of them, or a few
//!   steps out where the floor is near the ground; posts from the ground to
//!   each column's top landing, guards on the landings' open sides. A stair
//!   stands only where the yard is (nothing of it in a building, a street, the
//!   sea or across a fence), clear of what stands out of the wall and of the
//!   yard's trees' trunks;
//! * a downspout at a back corner (both on a wide wall), from a conductor head
//!   under the roof's edge to a shoe and splash block on the ground.
//!
//! Each building's stairs take one paint (bare weathered wood, grey, white, a
//! brown stain, a sage), each door its own; the downspouts galvanised or
//! painted. The stairs' footprints are returned so the yards' plants and
//! objects keep off them.

use eo_props::PropsWorld;
use eo_render::props::GpuPropInstance;
use eo_render::props_lib2::{YARD_BACK_BASE, yard_backs as yb};

/// A rectangle on a wall's basis kept clear of the yards' plants and objects.
#[derive(Clone, Copy, Debug)]
pub struct Clear {
    o: [f64; 2],
    u: [f64; 2],
    n: [f64; 2],
    s0: f64,
    s1: f64,
    w1: f64,
}

impl Clear {
    /// Whether (x, z) is within `pad` of the rectangle.
    pub fn holds(&self, x: f64, z: f64, pad: f64) -> bool {
        let (dx, dz) = (x - self.o[0], z - self.o[1]);
        let s = dx * self.u[0] + dz * self.u[1];
        let w = dx * self.n[0] + dz * self.n[1];
        s > self.s0 - pad && s < self.s1 + pad && w > -pad && w < self.w1 + pad
    }
}

/// A wall's basis: s along it from its start, w out of it.
#[derive(Clone, Copy)]
struct Wall {
    o: [f64; 2],
    u: [f64; 2],
    n: [f64; 2],
    len: f64,
}

impl Wall {
    fn at(&self, s: f64, w: f64) -> (f64, f64) {
        (self.o[0] + self.u[0] * s + self.n[0] * w, self.o[1] + self.u[1] * s + self.n[1] * w)
    }
    /// The yaw (three.js rotation.y) that turns local +Z onto world direction (dx, dz).
    fn yaw_of(dx: f64, dz: f64) -> f32 {
        dx.atan2(dz) as f32
    }
}

/// The paints (sRGB): a building's stairs, its doors, its downspouts.
const STAIR_PAINTS: [(&str, f64); 6] = [("#8b8378", 0.3), ("#8e9191", 0.2), ("#c4c0b6", 0.2), ("#6b5747", 0.12), ("#7f877b", 0.1), ("#a49a88", 0.08)];
const DOOR_PAINTS: [(&str, f64); 7] = [("#c9c5bb", 0.3), ("#7b2f2a", 0.12), ("#2f4a3c", 0.12), ("#33445a", 0.12), ("#5e5a55", 0.14), ("#8a6a4a", 0.1), ("#b9ae94", 0.1)];
const SPOUT_PAINTS: [(&str, f64); 4] = [("#8e9190", 0.45), ("#c2bcae", 0.25), ("#6d6a63", 0.15), ("#a89f8e", 0.15)];

fn pick(list: &[(&'static str, f64)], r: f64) -> &'static str {
    let mut acc = 0.0;
    for (c, w) in list {
        acc += w;
        if r < acc {
            return c;
        }
    }
    list[list.len() - 1].0
}

/// A paint as the instance's lamp word (props_yards.wgsl `pr_yard_item`).
fn paint(hex: &str) -> u32 {
    let h = hex.trim_start_matches('#');
    let c = |k: usize| {
        let v = u8::from_str_radix(&h[k..k + 2], 16).unwrap_or(128) as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    crate::props::vegetation::pack_tint([c(0), c(2), c(4)])
}

fn hash(a: u32, b: u32) -> f64 {
    let mut h = a.wrapping_mul(0x9e37_79b9) ^ b.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h >> 8) as f64 / (1u32 << 24) as f64
}

/// One flight of a stair: from its top (a column's edge, y) along the wall toward the other
/// column, in its lane; its riser count's model (`FLIGHT_RISERS` index: the stair's, or a
/// shorter last one that lands on the ground short of the other column).
struct Flight {
    s_top: f64,
    dir: f64,
    lane: usize,
    y_top: f64,
    drop: f64,
    model: usize,
}

/// A placed stair on a wall.
struct Stair {
    /// Columns' centres (A the doors', B the other) and the side B lies (+1 / -1).
    sa: f64,
    sb: f64,
    dir: f64,
    /// Landings: (column centre, y), top down.
    landings: Vec<(f64, f64)>,
    doors: Vec<f64>,
    flights: Vec<Flight>,
    /// The riser count's model (`FLIGHT_RISERS` index).
    model: usize,
    /// A flight straight out from the lowest landing (its model, its rise) where the
    /// floor is too near the ground for the switchbacks.
    steps: Option<(usize, f64)>,
    s0: f64,
    s1: f64,
}

/// The backs' instances, appended to `out`, and the stairs' footprints.
pub fn back_instances(world: &PropsWorld, out: &mut Vec<GpuPropInstance>) -> Vec<Clear> {
    let grounds = eo_props::yards::grounds();
    if !eo_props::yards::backs_enabled() || grounds.iter().all(|g| g.canopy.len() != g.classes.len()) {
        return vec![];
    }
    let t0 = std::time::Instant::now();
    let Some(kc) = eo_render::kit::shared_kit_city(std::time::Duration::from_secs(5), std::time::Duration::from_secs(120)) else {
        eprintln!("yards: no kit city: no back stairs");
        return vec![];
    };
    let sw = &world.street;
    let sea = eo_water::SEA_LEVEL_Y;
    let blocked = |x: f64, z: f64| sw.buildings.contains(x, z) || sw.paved_at(x, z).is_some() || world.ground(x, z) < sea + 0.3;
    let fences = eo_props::yards::fences();
    // whether a segment crosses any fence line (the fences' own lines, a little long at the ends).
    let crosses = |a: (f64, f64), b: (f64, f64)| {
        let cross = |o: [f64; 2], p: [f64; 2], q: (f64, f64)| (p[0] - o[0]) * (q.1 - o[1]) - (p[1] - o[1]) * (q.0 - o[0]);
        fences.iter().any(|f| {
            let (fa, fb) = (f.a, f.b);
            let d1 = cross(fa, fb, a);
            let d2 = cross(fa, fb, b);
            let d3 = cross([a.0, a.1], [b.0, b.1], (fa[0], fa[1]));
            let d4 = cross([a.0, a.1], [b.0, b.1], (fb[0], fb[1]));
            d1 * d2 < 0.0 && d3 * d4 < 0.0
        })
    };
    // the yards' trees (the canopy's tops and the measured trees standing in the yards): a stair keeps off their trunks.
    let trees: Vec<(f64, f64)> = eo_props::yards::plants()
        .iter()
        .filter(|p| p.kind == eo_props::yards::PlantKind::Tree)
        .map(|p| (p.x, p.z))
        .chain(eo_props::lidar_trees::all().iter().filter(|t| eo_props::yards::in_yard(t.x, t.z)).map(|t| (t.x, t.z)))
        .collect();
    let bounds: Vec<[f64; 4]> = grounds.iter().filter(|g| g.canopy.len() == g.classes.len()).map(|g| [g.x0 - 15.0, g.z0 - 15.0, g.x0 + g.nx as f64 * g.cell + 15.0, g.z0 + g.nz as f64 * g.cell + 15.0]).collect();
    let (mut clears, mut planned_n, mut walls, mut stairs_n, mut doors_n, mut spouts_n) = (vec![], 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut why = [0usize; 8];
    let push = |out: &mut Vec<GpuPropInstance>, model: u32, p: (f64, f64), y: f64, yaw: f32, scale: [f32; 3], lamp: u32, seed: u32| {
        out.push(GpuPropInstance { pos: [p.0 as f32, y as f32, p.1 as f32], yaw, scale, model: YARD_BACK_BASE + model, variant: seed & 255, lamp, slope: 0.0, pad: 0 });
    };
    for b in 0..kc.records.len() {
        let rec = &kc.records[b];
        if kc.excluded.contains(&b) || rec.polygon.len() < 3 {
            continue;
        }
        let n = rec.polygon.len() as f64;
        let (cx, cz) = rec.polygon.iter().fold((0.0, 0.0), |a, p| (a.0 + p.x / n, a.1 + p.z / n));
        if !bounds.iter().any(|q| cx > q[0] && cz > q[1] && cx < q[2] && cz < q[3]) {
            continue;
        }
        let (floor, _, _) = eo_render::kit::kit_floor(&kc, sw, b as u32);
        let Some(planned) = eo_kit::pipeline::plan_building(rec, &kc.world, b as u32, floor.y) else { continue };
        planned_n += 1;
        let plan = &planned.plan;
        let base = floor.y;
        let arch = planned.classified.archetype;
        let seed = planned.classified.seed;
        let Some(top) = plan.storeys.last().map(|s| base + s.y1) else { continue };
        // the floors a back door opens from: the flats' and the house's, not a garage's or a shop's.
        let living: Vec<f64> = plan.storeys.iter().filter(|s| !matches!(s.use_.as_str(), "garage" | "retail" | "parking" | "commercial" | "shop" | "industrial" | "office")).map(|s| base + s.y0).collect();
        let multi = matches!(arch, "edwardian-flats" | "bay-window-apartment") || (arch == "victorian-row" && plan.storeys.len() >= 3 && hash(seed, 1) < 0.55);
        let stair_paint = paint(pick(&STAIR_PAINTS, hash(seed, 2)));
        let spout_paint = paint(pick(&SPOUT_PAINTS, hash(seed, 3)));
        for e in &planned.frame.edges {
            if e.role != "back" || e.party || e.length < 4.0 || e.index == planned.front.index {
                continue;
            }
            let wall = Wall { o: e.basis.origin, u: e.basis.u, n: e.basis.n, len: e.length };
            // the wall opens onto a yard: its middle's ground 1.5 m out a yard's, nothing built or paved there.
            let mid = wall.at(wall.len / 2.0, 1.5);
            if !eo_props::yards::in_yard(mid.0, mid.1) || blocked(mid.0, mid.1) {
                continue;
            }
            walls += 1;
            // what stands out of the wall (a bay, a porch, a fire escape): nothing goes through it.
            let mut obstacles: Vec<[f64; 5]> = vec![];
            let mut escape = false;
            for p in plan.parts.iter() {
                if p.edge != Some(e.index) || matches!(p.shape, eo_kit::elevation::Shape::Prism(_)) {
                    continue;
                }
                escape |= p.kind.starts_with("fire-escape");
                if p.w1 > 0.14 {
                    obstacles.push([p.s0, p.s1, p.y0, p.y1, p.w1]);
                }
            }
            if escape {
                continue;
            }
            let mut apertures: Vec<[f64; 4]> = plan.edges.iter().filter(|r| r.index == e.index).flat_map(|r| r.openings.iter()).map(|o| [o.s0, o.s1, o.y0, o.y1]).collect();
            apertures.extend(plan.parts.iter().filter(|p| p.edge == Some(e.index) && eo_kit::pipeline::is_opening_part(p)).map(|p| [p.s0, p.s1, p.y0, p.y1]));
            let ground = |s: f64, w: f64| {
                let (x, z) = wall.at(s, w);
                world.ground(x, z)
            };
            // where a stair's footprint (s0..s1, out to w1) may stand: all of it yard, clear of fences.
            let footprint_ok = |s0: f64, s1: f64, w1: f64| {
                let mut s = s0;
                while s <= s1 + 1e-6 {
                    let mut w = 0.25;
                    while w <= w1 + 1e-6 {
                        // (the survey's cells along a wall are the building's: its yard from 0.75 m out.)
                        let (x, z) = wall.at(s, w);
                        if blocked(x, z) || (w >= 0.75 && !eo_props::yards::in_yard(x, z)) {
                            return false;
                        }
                        w += 0.5;
                    }
                    s += 0.5;
                }
                // (a measured tree's trunk: the stair goes elsewhere, the tree stays.)
                let here = Clear { o: wall.o, u: wall.u, n: wall.n, s0, s1, w1 };
                if trees.iter().any(|t| here.holds(t.0, t.1, 0.6)) {
                    return false;
                }
                let c = [wall.at(s0, 0.05), wall.at(s1, 0.05), wall.at(s1, w1), wall.at(s0, w1)];
                !(0..4).any(|k| crosses(c[k], c[(k + 1) % 4]))
            };
            let free_door = |sa: f64, y: f64| {
                let (a, c) = (sa - yb::LANDING_W_M as f64 / 2.0 + 0.02, sa + yb::LANDING_W_M as f64 / 2.0 - 0.02);
                !apertures.iter().any(|o| o[1] > a && o[0] < c && o[3] > y + 0.05 && o[2] < y + 2.25) && !obstacles.iter().any(|o| o[1] > a && o[0] < c && o[3] > y && o[2] < y + 2.3)
            };
            let lw = yb::LANDING_W_M as f64;
            let depth = yb::LANDING_D_M as f64;
            let mut placed: Vec<Stair> = vec![];
            let want = if multi && wall.len >= 15.0 { 2 } else { 1 };
            // candidate columns every 0.25 m, both ways; the best first (near an end of the wall).
            let mut cands: Vec<(f64, f64, f64)> = vec![];
            let mut sa = 0.35 + lw / 2.0;
            while sa <= wall.len - 0.35 - lw / 2.0 {
                for dir in [1.0, -1.0] {
                    cands.push((sa, dir, hash(seed ^ e.index as u32, (sa * 4.0) as u32 + (dir > 0.0) as u32 * 1000)));
                }
                sa += 0.25;
            }
            let end_dist = |sa: f64, dir: f64, span: f64| {
                let (lo, hi) = if dir > 0.0 { (sa - lw / 2.0, sa - lw / 2.0 + span) } else { (sa + lw / 2.0 - span, sa + lw / 2.0) };
                lo.min(wall.len - hi)
            };
            for _ in 0..want {
                let mut best: Option<(f64, Stair)> = None;
                for &(sa, dir, r) in &cands {
                    let Some(st) = design(sa, dir, &living, multi, &ground, &free_door, lw, depth) else {
                        why[0] += 1;
                        continue;
                    };
                    if st.s0 < 0.3 || st.s1 > wall.len - 0.3 {
                        why[1] += 1;
                        continue;
                    }
                    if placed.iter().any(|p| st.s0 < p.s1 + 1.5 && st.s1 > p.s0 - 1.5) {
                        continue;
                    }
                    let y_lo = st.flights.last().map(|f| f.y_top - f.drop).unwrap_or(st.landings.last().map(|l| l.1).unwrap_or(0.0)) - 0.2;
                    let y_hi = st.landings.first().map(|l| l.1).unwrap_or(0.0) + yb::RAIL_H_M as f64 + 2.3;
                    if obstacles.iter().any(|o| o[1] > st.s0 && o[0] < st.s1 && o[3] > y_lo && o[2] < y_hi) {
                        why[2] += 1;
                        continue;
                    }
                    let w1 = depth + st.steps.map_or(0.0, |(k, _)| 0.4 + (yb::FLIGHT_RISERS[k] - 1) as f64 * yb::TREAD_M as f64);
                    let score = end_dist(sa, dir, st.s1 - st.s0) + r * 0.6;
                    if best.as_ref().is_some_and(|b| b.0 <= score) {
                        continue;
                    }
                    if !footprint_ok(st.s0, st.s1, w1) {
                        why[3] += 1;
                        continue;
                    }
                    best = Some((score, st));
                }
                let Some((_, st)) = best else { break };
                placed.push(st);
            }
            for st in &placed {
                stairs_n += 1;
                if std::env::var("EO_YARD_ITEMS_TRACE").is_ok_and(|v| v == "1") {
                    let (x, z) = wall.at((st.s0 + st.s1) / 2.0, 0.0);
                    let g = ground((st.s0 + st.s1) / 2.0, 1.0);
                    eprintln!("yards: drawn back stair at {x:.1} {z:.1} ground {g:.1} out {:.3} {:.3} doors {} top {:.1} span {:.1} {arch}", wall.n[0], wall.n[1], st.doors.len(), st.landings.first().map(|l| l.1).unwrap_or(0.0) - g, st.s1 - st.s0);
                }
                draw(out, &push, &wall, st, &ground, stair_paint, seed);
                for (k, &y) in st.doors.iter().enumerate() {
                    let p = wall.at(st.sa, 0.0);
                    push(out, yb::DOOR, p, y, Wall::yaw_of(wall.n[0], wall.n[1]), [1.0; 3], paint(pick(&DOOR_PAINTS, hash(seed, 10 + k as u32))), seed ^ k as u32);
                    doors_n += 1;
                }
                let w1 = depth + st.steps.map_or(0.0, |(k, _)| 0.4 + (yb::FLIGHT_RISERS[k] - 1) as f64 * yb::TREAD_M as f64);
                clears.push(Clear { o: wall.o, u: wall.u, n: wall.n, s0: st.s0, s1: st.s1, w1 });
            }
            // the downspouts: at an end of the wall (both on a wide one) clear of the stairs and the windows.
            let ends: Vec<f64> = if wall.len >= 12.0 { vec![0.12, wall.len - 0.12] } else if hash(seed, 20 + e.index as u32) < 0.5 { vec![0.12] } else { vec![wall.len - 0.12] };
            for s in ends {
                if placed.iter().any(|st| s > st.s0 - 0.3 && s < st.s1 + 0.3) || apertures.iter().any(|o| o[0] < s + 0.12 && o[1] > s - 0.12) || obstacles.iter().any(|o| o[0] < s + 0.12 && o[1] > s - 0.12) {
                    why[4] += 1;
                    continue;
                }
                let (x, z) = wall.at(s, 0.8);
                if blocked(x, z) || blocked(wall.at(s, 0.2).0, wall.at(s, 0.2).1) || !eo_props::yards::in_yard(x, z) {
                    why[5 + blocked(x, z) as usize] += 1;
                    continue;
                }
                let g = ground(s, 0.15);
                let head = top - 0.12 - yb::HEAD_H_M as f64;
                let len = head - (g + yb::SHOE_H_M as f64);
                if len < 1.0 {
                    continue;
                }
                let p = wall.at(s, 0.0);
                let yaw = Wall::yaw_of(wall.n[0], wall.n[1]);
                let sd = seed ^ 0x5157_0000 ^ s.to_bits() as u32;
                push(out, yb::SPOUT_SHOE, p, g, yaw, [1.0; 3], spout_paint, sd);
                push(out, yb::DOWNSPOUT, p, g + yb::SHOE_H_M as f64, yaw, [1.0, len as f32, 1.0], spout_paint, sd);
                push(out, yb::SPOUT_HEAD, p, head, yaw, [1.0; 3], spout_paint, sd);
                spouts_n += 1;
            }
        }
    }
    eprintln!(
        "yards: backs: {planned_n} buildings planned, {walls} back walls on a yard, {stairs_n} back stairs with {doors_n} doors, {spouts_n} downspouts ({:.0} ms); left out {why:?}",
        t0.elapsed().as_secs_f64() * 1e3
    );
    clears
}

/// A stair whose door column is centred at `sa` and whose other column lies toward `dir`,
/// from the floors `living` (absolute floor heights, bottom up), or None where it does not fit.
#[allow(clippy::too_many_arguments)]
fn design(sa: f64, dir: f64, living: &[f64], multi: bool, ground: &dyn Fn(f64, f64) -> f64, free_door: &dyn Fn(f64, f64) -> bool, lw: f64, depth: f64) -> Option<Stair> {
    let g_a = ground(sa, 1.0);
    // the doors' floors: above the yard (a buried floor has no back door here), the flats' all
    // (up to four, at most 12 m up), a house's lowest; each with its door's column free.
    let mut doors: Vec<f64> = living.iter().copied().filter(|&y| y - g_a > -0.1 && y - g_a < 12.0).collect();
    if !multi {
        doors.truncate(1);
    }
    doors.truncate(4);
    // (an upper flat whose door would cross a window goes without; the stair stops below it.)
    while doors.last().is_some_and(|&y| !free_door(sa, y)) {
        doors.pop();
    }
    if doors.is_empty() || !doors.iter().all(|&y| free_door(sa, y)) {
        return None;
    }
    // two flights a floor: the fewest risers that keep every half floor's under the code's most.
    let halves: Vec<f64> = doors.windows(2).map(|w| (w[1] - w[0]) / 2.0).collect();
    let tallest = halves.iter().copied().fold(1.45, f64::max);
    let model = (1..yb::FLIGHT_RISERS.len()).find(|&k| yb::FLIGHT_RISERS[k] as f64 * yb::RISER_MAX_M as f64 >= tallest)?;
    let risers = yb::FLIGHT_RISERS[model] as f64;
    if halves.iter().any(|&h| h < risers * yb::RISER_MIN_M as f64) {
        return None;
    }
    let run = (risers - 1.0) * yb::TREAD_M as f64;
    let sb = sa + dir * (lw + run);
    let col = |k: usize| if k % 2 == 0 { sa } else { sb };
    let mut flights = vec![];
    let mut landings = vec![];
    // top down: each floor's landing at A, a half landing at B between floors.
    let tops: Vec<f64> = doors.iter().rev().copied().collect();
    let mut k = 0usize;
    for (i, &y) in tops.iter().enumerate() {
        landings.push((sa, y));
        if let Some(&below) = tops.get(i + 1) {
            let half = (y - below) / 2.0;
            for j in 0..2 {
                let from = col(k);
                let yt = y - j as f64 * half;
                flights.push(Flight { s_top: from + (col(k + 1) - from).signum() * lw / 2.0, dir: (col(k + 1) - from).signum(), lane: k % 2, y_top: yt, drop: half, model });
                if j == 0 {
                    landings.push((sb, y - half));
                }
                k += 1;
            }
        }
    }
    // down to the yard from the lowest door: a few steps straight out where a house's floor is
    // near the ground; else flights between the columns, each dropping what the ground under
    // its foot leaves in as few of the stair's flights as keep the risers under the code's
    // most, and a shorter last one (fewer risers) where less is left than the stair's least.
    let y1 = doors[0];
    let mut steps = None;
    let low_max = yb::FLIGHT_RISERS[0] as f64 * yb::RISER_MAX_M as f64;
    let run_of = |m: usize| (yb::FLIGHT_RISERS[m] as f64 - 1.0) * yb::TREAD_M as f64;
    if tops.len() == 1 && y1 - g_a <= low_max {
        let rise = y1 - ground(sa, depth + run_of(0) + 0.1);
        if rise > 0.1 {
            if rise > low_max {
                return None;
            }
            steps = Some((0, rise));
        }
    } else {
        // the choices: m of the stair's flights, each dropping d, to the ground; or m of them to a
        // landing and a shorter last flight (model q) from it to the ground; the risers within
        // the code's limits, every landing over the ground, the stair's own risers nearest its floors'.
        let nominal = risers * yb::RISER_M as f64;
        let (lo, hi) = (risers * yb::RISER_MIN_M as f64, risers * yb::RISER_MAX_M as f64);
        let foot = |kk: usize, m: usize| {
            let from = col(kk);
            let dir = (col(kk + 1) - from).signum();
            (from + dir * (lw / 2.0 + run_of(m)), yb::LANE_W_M[kk % 2] as f64)
        };
        let landings_clear = |m: usize, d: f64| (1..=m).all(|i| {
            let c = col(k + i);
            y1 - i as f64 * d > ground(c, depth / 2.0) + 0.15
        });
        // (cost, main flights, their drop, the last flight's model and drop)
        let mut best: Option<(f64, usize, f64, Option<(usize, f64)>)> = None;
        for m in 1..=8usize {
            let (fs, fw) = foot(k + m - 1, model);
            let d = (y1 - ground(fs, fw)) / m as f64;
            if d >= lo && d <= hi && landings_clear(m - 1, d) {
                let cost = (d / nominal).ln().abs();
                if best.as_ref().is_none_or(|b| cost < b.0) {
                    best = Some((cost, m, d, None));
                }
            }
        }
        for m in 0..=7usize {
            for q in 0..model {
                let nq = yb::FLIGHT_RISERS[q] as f64;
                let (fs, fw) = foot(k + m, q);
                let g = ground(fs, fw);
                for r in [0.17, 0.185, 0.15, 0.197, 0.13, 0.11] {
                    let ye = g + nq * r;
                    let d = if m == 0 { 0.0 } else { (y1 - ye) / m as f64 };
                    if m == 0 && ((y1 - g) / nq < yb::RISER_MIN_M as f64 || (y1 - g) / nq > yb::RISER_MAX_M as f64) {
                        continue;
                    }
                    if m > 0 && (d < lo || d > hi || !landings_clear(m, d)) {
                        continue;
                    }
                    let drop = if m == 0 { y1 - g } else { ye - g };
                    let cost = 0.25 + if m == 0 { 0.0 } else { (d / nominal).ln().abs() };
                    if best.as_ref().is_none_or(|b| cost < b.0) {
                        best = Some((cost, m, d, Some((q, drop))));
                    }
                }
            }
        }
        let (_, m, d, last) = best?;
        let mut y = y1;
        for i in 0..m {
            let from = col(k);
            let dir = (col(k + 1) - from).signum();
            flights.push(Flight { s_top: from + dir * lw / 2.0, dir, lane: k % 2, y_top: y, drop: d, model });
            k += 1;
            y -= d;
            if i + 1 < m || last.is_some() {
                landings.push((col(k), y));
            }
        }
        if let Some((q, drop)) = last {
            let from = col(k);
            let dir = (col(k + 1) - from).signum();
            flights.push(Flight { s_top: from + dir * lw / 2.0, dir, lane: k % 2, y_top: y, drop, model: q });
        }
    }
    let cols_used = landings.iter().map(|l| l.0).chain(flights.iter().map(|f| f.s_top + f.dir * (run + lw / 2.0)));
    let (mut s0, mut s1) = (f64::MAX, f64::MIN);
    for c in cols_used {
        s0 = s0.min(c - lw / 2.0);
        s1 = s1.max(c + lw / 2.0);
    }
    s0 = s0.min(sa - lw / 2.0);
    s1 = s1.max(sa + lw / 2.0);
    Some(Stair { sa, sb, dir, landings, doors, flights, model, steps, s0, s1 })
}

/// A stair's instances: its flights, landings, guards and posts.
fn draw(out: &mut Vec<GpuPropInstance>, push: &dyn Fn(&mut Vec<GpuPropInstance>, u32, (f64, f64), f64, f32, [f32; 3], u32, u32), wall: &Wall, st: &Stair, ground: &dyn Fn(f64, f64) -> f64, lamp: u32, seed: u32) {
    let lw = yb::LANDING_W_M as f64;
    let depth = yb::LANDING_D_M as f64;
    let along = |d: f64| Wall::yaw_of(wall.u[0] * d, wall.u[1] * d);
    let out_yaw = Wall::yaw_of(wall.n[0], wall.n[1]);
    for (i, f) in st.flights.iter().enumerate() {
        let w = yb::LANE_W_M[f.lane] as f64;
        let p = wall.at(f.s_top, w);
        let nominal = yb::FLIGHT_RISERS[f.model] as f64 * yb::RISER_M as f64;
        push(out, yb::FLIGHT + f.model as u32, p, f.y_top, along(f.dir), [1.0, (f.drop / nominal) as f32, 1.0], lamp, seed ^ (i as u32) << 3);
    }
    // the landings and their guards: the outer edge, the column's far end; the top's lane-1 gap.
    let top_y = st.landings.first().map(|l| l.1).unwrap_or(0.0);
    for (i, &(c, y)) in st.landings.iter().enumerate() {
        push(out, yb::LANDING, wall.at(c, 0.0), y, out_yaw, [1.0; 3], lamp, seed ^ 0x100 ^ i as u32);
        let away = if (c - st.sa).abs() < 1e-6 { -st.dir } else { st.dir };
        let low = i + 1 == st.landings.len() && st.steps.is_some();
        if !low {
            // the outer edge's guard, along the wall.
            push(out, yb::RAIL_SHORT, wall.at(c - lw / 2.0, depth - 0.03), y, along(1.0), [1.0; 3], lamp, seed ^ 0x200 ^ i as u32);
        }
        if y - ground(c, 1.0) > 0.6 {
            // the far end's guard, out from the wall.
            push(out, yb::RAIL_LONG, wall.at(c + away * (lw / 2.0 - 0.03), 0.0), y, out_yaw, [1.0, 1.0, (depth / 2.0) as f32], lamp, seed ^ 0x300 ^ i as u32);
        }
        if i == 0 && y - ground(c, 1.0) > 0.6 {
            // the top landing's open side past its one flight: the outer lane's gap.
            let s = c - away * (lw / 2.0 - 0.03);
            push(out, yb::RAIL_SHORT, wall.at(s, 1.06), y, out_yaw, [1.0, 1.0, (depth - 1.06) as f32], lamp, seed ^ 0x400);
        }
    }
    // posts at each column's two outer corners, from the ground to its top landing's guard.
    for c in [st.sa, st.sb] {
        let Some(y) = st.landings.iter().filter(|l| (l.0 - c).abs() < 1e-6).map(|l| l.1).reduce(f64::max) else { continue };
        for s in [c - lw / 2.0 + 0.045, c + lw / 2.0 - 0.045] {
            let g = ground(s, depth - 0.045);
            let h = y + if y >= top_y - 1e-6 || y - g > 0.6 { yb::RAIL_H_M as f64 } else { 0.0 } - g + 0.1;
            if h > 0.15 {
                push(out, yb::POST, wall.at(s, depth - 0.045), g - 0.1, along(1.0), [1.0, h as f32, 1.0], lamp, seed ^ 0x500);
            }
        }
    }
    // steps straight out from the lowest landing where its floor is near the ground.
    if let (Some((k, rise)), Some(&(c, y))) = (st.steps, st.landings.last()) {
        let p = wall.at(c, depth);
        let n = yb::FLIGHT_RISERS[k] as f64;
        push(out, yb::FLIGHT + k as u32, p, y, out_yaw, [1.0, (rise / (n * yb::RISER_M as f64)) as f32, 1.0], lamp, seed ^ 0x600);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LW: f64 = yb::LANDING_W_M as f64;
    const DEPTH: f64 = yb::LANDING_D_M as f64;

    /// A stair's flights join up: each starts at a landing's edge and ends at the
    /// other column's edge, a flight's foot is the next one's top (or the ground at
    /// the last), lanes alternate, risers within the code's limits.
    fn check(st: &Stair, ground: f64) {
        for (i, f) in st.flights.iter().enumerate() {
            let risers = yb::FLIGHT_RISERS[f.model] as f64;
            let run = (risers - 1.0) * yb::TREAD_M as f64;
            let riser = f.drop / risers;
            assert!((yb::RISER_MIN_M as f64 - 1e-9..=yb::RISER_MAX_M as f64 + 1e-9).contains(&riser), "flight {i} riser {riser}");
            assert_eq!(f.lane, i % 2, "flight {i} lane");
            let foot = f.s_top + f.dir * run;
            let at_col = [st.sa, st.sb].iter().any(|&c| (foot - (c - f.dir * LW / 2.0)).abs() < 1e-6);
            assert!(at_col || (i + 1 == st.flights.len() && f.model < st.model), "flight {i} ends at {foot}, not a column's edge");
            match st.flights.get(i + 1) {
                Some(g) => assert!((f.y_top - f.drop - g.y_top).abs() < 1e-6, "flight {i}'s foot is not the next one's top"),
                None => assert!((f.y_top - f.drop - ground).abs() < 1e-6, "the last flight ends off the ground"),
            }
        }
        for &(c, y) in &st.landings {
            assert!((c - st.sa).abs() < 1e-6 || (c - st.sb).abs() < 1e-6, "landing off a column");
            assert!(y > ground, "landing under the ground");
        }
    }

    #[test]
    fn flats_get_a_door_a_floor_and_switchbacks_to_the_yard() {
        // three flats' floors 2.8, 5.6 and 8.4 m over a flat yard, no windows.
        let st = design(2.0, 1.0, &[2.8, 5.6, 8.4], true, &|_, _| 0.0, &|_, _| true, LW, DEPTH).expect("a stair");
        assert_eq!(st.doors, vec![2.8, 5.6, 8.4]);
        // two flights a floor between the doors, then two more down to the yard.
        assert_eq!(st.flights.len(), 6);
        assert!(st.steps.is_none());
        check(&st, 0.0);
        // the doors' landings on column A, the half landings on B.
        for y in [2.8, 5.6, 8.4] {
            assert!(st.landings.iter().any(|&(c, ly)| (c - st.sa).abs() < 1e-6 && (ly - y).abs() < 1e-6));
        }
        assert!(st.s1 - st.s0 < 5.0, "a switchback is a few metres along the wall: {}", st.s1 - st.s0);
    }

    #[test]
    fn a_house_gets_one_door_and_a_floor_near_the_ground_steps() {
        let st = design(2.0, -1.0, &[2.6, 5.4], false, &|_, _| 0.0, &|_, _| true, LW, DEPTH).expect("a stair");
        assert_eq!(st.doors, vec![2.6]);
        check(&st, 0.0);
        let st = design(2.0, 1.0, &[0.45, 3.2], false, &|_, _| 0.0, &|_, _| true, LW, DEPTH).expect("a stoop");
        assert!(st.flights.is_empty());
        let (k, rise) = st.steps.expect("steps out");
        assert_eq!(k, 0, "a few steps");
        assert!((rise - 0.45).abs() < 1e-9);
    }

    #[test]
    fn a_window_in_the_door_column_or_a_buried_floor_has_no_door() {
        // the column is a window on every floor: no stair.
        assert!(design(2.0, 1.0, &[2.8, 5.6], true, &|_, _| 0.0, &|_, _| false, LW, DEPTH).is_none());
        // the top floor's column is a window: the stair stops at the floor below.
        let st = design(2.0, 1.0, &[2.8, 5.6], true, &|_, _| 0.0, &|_, y| y < 4.0, LW, DEPTH).expect("a stair");
        assert_eq!(st.doors, vec![2.8]);
        // the yard is up at the second floor: the first is buried, the door the second's, at the ground.
        let st = design(2.0, 1.0, &[0.0, 2.8, 5.6], false, &|_, _| 2.8, &|_, _| true, LW, DEPTH).expect("a door");
        assert_eq!(st.doors, vec![2.8]);
        assert!(st.flights.is_empty() && st.steps.is_none());
    }

    #[test]
    fn a_floor_a_metre_up_gets_a_short_flight() {
        // too low for the stair's own flights (2.8 m floors: 8 risers, at least 0.8 m), too high for steps.
        let st = design(2.0, 1.0, &[0.7, 3.5], true, &|_, _| 0.0, &|_, _| true, LW, DEPTH).expect("a stair");
        let last = st.flights.last().unwrap();
        assert!(last.model < st.model, "a shorter last flight");
        check(&st, 0.0);
    }

    #[test]
    fn the_last_flight_lands_on_sloping_ground() {
        // the yard falls 0.4 in 1 along the wall (a steep lot): one way or the other a stair fits,
        // its last flight on the ground under its foot.
        let ground = |s: f64, _w: f64| -0.4 * (s - 2.0);
        let mut fits = 0;
        for dir in [1.0, -1.0] {
            let Some(st) = design(2.0, dir, &[3.0, 6.0], true, &ground, &|_, _| true, LW, DEPTH) else { continue };
            let last = st.flights.last().unwrap();
            let foot = last.s_top + last.dir * (yb::FLIGHT_RISERS[last.model] as f64 - 1.0) * yb::TREAD_M as f64;
            check(&st, ground(foot, 0.0));
            fits += 1;
        }
        assert!(fits > 0, "no stair either way");
    }

}
