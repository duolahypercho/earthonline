//! Street signs from SFMTA's own inventory (street-signs.js): every listed
//! post with its plates stacked in inventory order, the top plate's bottom at
//! the CA MUTCD 2A.18 mounting height (`sign-mounting-height-urban`: 7 ft, or
//! higher so the lowest plate stays at 6 ft), each plate facing its listed
//! yaw; a post the city geocoded onto carriageway, a junction, a driveway
//! mouth or a kerb ramp moved to the nearest kerb seat
//! (`createStreetSignKerbSnap`, CA MUTCD 2A.16 lateral offset
//! `sign-lateral-offset-urban`), dropped when no kerb is within the snap
//! reach; a post repeating every plate of a post already placed within the
//! merge radius collapsed (F3). With [`set_merge_posts`] (eo-app; its
//! `--no-sign-post-merge` leaves it off), a post seated within the merge
//! radius of one already placed is that pole: the inventory lists the signs
//! of one physical pole under several records, often at the same point, and
//! each stood its own post (one 0-15 cm in front of another's plates). Its
//! plates hang on the pole already standing: a repeated plate (same face,
//! same facing) is dropped; a plate back to back with the pole's plates
//! (facings at least `BACK_TO_BACK_DEG` apart) shares their height; any other
//! plate that would cross one goes under the pole's plates of that facing
//! (keeping the secondary-plate minimum), else over all of them, and the pole
//! reaches its top plate.
//!
//! Differences from the JS, stated: posts are planned per 128 m tile of their
//! listed point in inventory order (the JS plans a 180 m window round the
//! camera in cell order), so a merge across a tile border is not made; the
//! post is grounded on the drawn surface at its seat.

use crate::{Live, Model, Prop, PropsWorld, station_at};
use std::collections::HashMap;

const INCH_M: f64 = 0.0254;
/// The JS merge rule (`SIGN_MERGE`: 0.9 m, 20 deg).
const MERGE_M: f64 = 0.9;
const MERGE_YAW_DEG: f64 = 20.0;
/// Two plates on one pole facing at least this far apart are back to back and may share a
/// height; nearer facings (a corner's two plates at 90 deg) would cross each other's edges.
const BACK_TO_BACK_DEG: f64 = 150.0;

static MERGE_POSTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One pole per spot: a post seated within the merge radius of one already placed hangs its
/// plates on it (eo-app; `--no-sign-post-merge` leaves it off). Off in the library unless set.
pub fn set_merge_posts(on: bool) {
    MERGE_POSTS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn merge_posts() -> bool {
    MERGE_POSTS.load(std::sync::atomic::Ordering::Relaxed)
}

#[allow(non_snake_case)]
pub fn SIGN_CELL_M(js: &crate::JsProps) -> f64 {
    js.num("streetSigns", "cellM")
}

fn plate_yaw(code: i32, fallback: f64) -> f64 {
    if code == 255 { fallback } else { code as f64 / 254.0 * std::f64::consts::TAU }
}

fn yaw_gap(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % std::f64::consts::TAU;
    if d > std::f64::consts::PI { std::f64::consts::TAU - d } else { d }
}

/// `SNAP_CELL_M` of street-signs.js.
const SNAP_CELL_M: f64 = 32.0;

/// `segmentCells`: plan ribbons bucketed by 32 m cells, padded by their own reach.
pub fn entry_cells(w: &PropsWorld) -> &HashMap<(i32, i32), Vec<u32>> {
    let reach = w.js.num("streetSigns", "snapReachM");
    w.snap_cells.get_or_init(|| {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (ei, e) in w.street.plan.entries.iter().enumerate() {
            // (a structure's leg into a junction, eo-street joins, is not drawn.)
            if e.points.len() < 2 || !(e.half > 0.0) || e.structure_leg.is_some() {
                continue;
            }
            let pad = e.half + e.walks[0].max(e.walks[1]) + reach;
            let (mut x0, mut x1, mut z0, mut z1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for p in &e.points {
                x0 = x0.min(p[0]);
                x1 = x1.max(p[0]);
                z0 = z0.min(p[1]);
                z1 = z1.max(p[1]);
            }
            for gx in ((x0 - pad) / SNAP_CELL_M).floor() as i32..=((x1 + pad) / SNAP_CELL_M).floor() as i32 {
                for gz in ((z0 - pad) / SNAP_CELL_M).floor() as i32..=((z1 + pad) / SNAP_CELL_M).floor() as i32 {
                    cells.entry((gx, gz)).or_default().push(ei as u32);
                }
            }
        }
        cells
    })
}

/// `createStreetSignKerbSnap(plan, { postRadiusM })` applied to one listed point:
/// the seat, or None (dropped). `paved` refuses a seat with no drawn paving.
pub fn snap(w: &PropsWorld, x: f64, z: f64, edge_m: f64, post_r: f64, paved: bool) -> Option<(f64, f64, f64)> {
    snap_with(w, x, z, edge_m, post_r, paved, &|_, _| true)
}

/// `snap` refusing every seat `free` does not take too (the listed point
/// and each kerb seat tried): `--structures` keeps a post off the columns
/// and out of the decks (`plan_tile_clear`).
pub fn snap_with(w: &PropsWorld, x: f64, z: f64, edge_m: f64, post_r: f64, paved: bool, free: &dyn Fn(f64, f64) -> bool) -> Option<(f64, f64, f64)> {
    let typical = w.std.sign_lateral_typical;
    let limited = w.std.sign_lateral_limited;
    let reach = w.js.num("streetSigns", "snapReachM");
    let step = w.js.num("streetSigns", "snapStepM");
    let r = post_r.max(0.0);
    let edge = r.max(edge_m);
    let clear = |x: f64, z: f64, clear_m: f64| {
        w.roadway.hit(x, z, clear_m).is_none() && !w.roadway.ramp_hit(x, z, r) && (!paved || w.street.paved_at(x, z).is_some()) && free(x, z)
    };
    let clear_m = edge + limited;
    if clear(x, z, clear_m) {
        return Some((x, z, 0.0));
    }
    let mut best: Option<(f64, f64, f64)> = None;
    // Ribbons whose padded box reaches the point (the JS buckets them by 32 m cells).
    let cells = entry_cells(w);
    let empty = vec![];
    for &ei in cells.get(&((x / SNAP_CELL_M).floor() as i32, (z / SNAP_CELL_M).floor() as i32)).unwrap_or(&empty) {
        let e = &w.street.plan.entries[ei as usize];
        // projectOnto: arc length and signed lateral.
        let mut hit: Option<(f64, f64, f64)> = None;
        for i in 0..e.points.len() - 1 {
            let (a, b) = (e.points[i], e.points[i + 1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dz);
            if !(len > 1e-9) {
                continue;
            }
            let (tx, tz) = (dx / len, dz / len);
            let along = ((x - a[0]) * tx + (z - a[1]) * tz).clamp(0.0, len);
            let (px, pz) = (a[0] + tx * along, a[1] + tz * along);
            let d = (x - px).hypot(z - pz);
            if hit.is_some_and(|h| d >= h.2) {
                continue;
            }
            hit = Some((e.cum[i] + along, (x - px) * -tz + (z - pz) * tx, d));
        }
        let Some((hs, hu, hd)) = hit else { continue };
        if hd - e.half > reach {
            continue;
        }
        let near = if hu >= 0.0 { 1.0 } else { -1.0 };
        for side in [near, -near] {
            let walk = if side > 0.0 { e.walks[0] } else { e.walks[1] };
            if !(walk > 0.0) {
                continue;
            }
            for offset in [typical, limited] {
                if offset + edge + r > walk {
                    continue;
                }
                let lateral = e.half + offset + edge;
                let mut found = false;
                let mut k = 0usize;
                loop {
                    let ds = (if k % 2 == 0 { 1.0 } else { -1.0 }) * (k as f64 / 2.0).ceil() * step;
                    k += 1;
                    if ds.abs() > reach || best.is_some_and(|b| ds.abs() > b.2) {
                        break;
                    }
                    let s = hs + ds;
                    if s < 0.0 || s > e.length {
                        continue;
                    }
                    let f = station_at(e, s, true);
                    let miter = if f.miter != 0.0 { f.miter } else { 1.0 };
                    let px = f.x + f.nx * side * lateral * miter;
                    let pz = f.z + f.nz * side * lateral * miter;
                    let moved = (px - x).hypot(pz - z);
                    if moved > reach || best.is_some_and(|b| moved >= b.2) {
                        continue;
                    }
                    if !clear(px, pz, clear_m - 0.01) {
                        continue;
                    }
                    best = Some((px, pz, moved));
                    found = true;
                }
                if found {
                    break;
                }
            }
        }
    }
    best
}

/// The posts and plates of one tile (posts whose listed point is in it).
pub fn plan_tile(w: &PropsWorld, key: (i32, i32)) -> Vec<Prop> {
    plan_posts(w, key, true).into_iter().flat_map(|p| p.props).collect()
}

/// `plan_tile` with every post kept clear of the grade-separated roads `s`
/// (`--structures`, stage A6): the kerb snap moves a post whose seat is on
/// a column or under a deck too low for it to the nearest clear seat along
/// its kerb, and drops it where none is.
pub fn plan_tile_clear(w: &PropsWorld, s: &eo_structures::StructureWorld, key: (i32, i32)) -> Vec<Prop> {
    plan_posts_with(w, key, true, Some(s)).into_iter().flat_map(|p| p.props).collect()
}

pub struct PlacedPost {
    pub index: u32,
    pub listed: [f64; 2],
    pub seat: [f64; 2],
    pub moved_m: f64,
    /// (face index, yaw, centre y above ground) per plate.
    pub plates: Vec<(u16, f64, f64)>,
    /// (yaw, bottom, top above ground) per plate: what a plate hung on this pole must not cross.
    pub spans: Vec<(f64, f64, f64)>,
    /// The drawn surface at the seat, and the pole's top above it.
    pub ground: f64,
    pub top: f64,
    pub props: Vec<Prop>,
}

/// Hang the plates of a post seated on `host`'s spot (`merge_posts`) on `host`.
fn mount_on(host: &mut PlacedPost, stack: &[(u16, f64, f64, i32)], top: f64, post_yaw: Option<f64>, post_r: f64, secondary_min: f64) {
    let max_yaw = MERGE_YAW_DEG.to_radians();
    let back = BACK_TO_BACK_DEG.to_radians();
    let mut cursor = top;
    for pl in stack {
        let h = pl.2;
        let yaw = plate_yaw(pl.3, post_yaw.unwrap_or(0.0));
        let mut bottom = cursor - h;
        cursor = bottom;
        // the pole already carries this face this way.
        if host.plates.iter().any(|q| q.0 == pl.0 && yaw_gap(q.1, yaw) <= max_yaw) {
            continue;
        }
        let crosses = |b: f64, spans: &[(f64, f64, f64)]| spans.iter().any(|s| yaw_gap(s.0, yaw) < back && b < s.2 - 0.01 && b + h > s.1 + 0.01);
        if crosses(bottom, &host.spans) {
            let low = host.spans.iter().filter(|s| yaw_gap(s.0, yaw) < back).map(|s| s.1).fold(f64::INFINITY, f64::min);
            bottom = low - h;
            if bottom < secondary_min || crosses(bottom, &host.spans) {
                bottom = host.spans.iter().map(|s| s.2).fold(host.top, f64::max);
            }
        }
        host.spans.push((yaw, bottom, bottom + h));
        host.plates.push((pl.0, yaw, bottom + h / 2.0));
        host.top = host.top.max(bottom + h);
        let plate = Prop {
            model: Model::SignPlate,
            pos: [host.seat[0] + yaw.sin() * post_r, host.ground + bottom + h / 2.0, host.seat[1] + yaw.cos() * post_r],
            yaw,
            scale: [pl.1 as f32, h as f32, 1.0],
            variant: pl.0,
            live: Live::None,
        };
        let at = host.props.len().saturating_sub(1); // before the post (the last prop)
        host.props.insert(at, plate);
    }
    if let Some(post) = host.props.last_mut().filter(|p| p.model == Model::SignPost) {
        post.scale[1] = host.top as f32;
    }
}

pub fn plan_posts(w: &PropsWorld, key: (i32, i32), paved: bool) -> Vec<PlacedPost> {
    plan_posts_with(w, key, paved, None)
}

/// `plan_posts`, the posts kept clear of `structures` when given (`plan_tile_clear`).
pub fn plan_posts_with(w: &PropsWorld, key: (i32, i32), paved: bool, structures: Option<&eo_structures::StructureWorld>) -> Vec<PlacedPost> {
    let inv = &w.js.signs;
    let cell = SIGN_CELL_M(&w.js);
    let post_r = w.js.consts["signPostRadiusM"].as_f64().unwrap_or(0.0);
    let tile = eo_street::STREET_TILE_M;
    let (x0, z0) = (key.0 as f64 * tile, key.1 as f64 * tile);
    let mut idx: Vec<u32> = vec![];
    for cz in (z0 / cell).floor() as i32..=((z0 + tile) / cell).floor() as i32 {
        for cx in (x0 / cell).floor() as i32..=((x0 + tile) / cell).floor() as i32 {
            if let Some(v) = w.sign_cells_get(cx, cz) {
                idx.extend(v.iter().filter(|&&i| {
                    let (x, z) = (inv.posts[i as usize * 4] as f64 / 10.0, inv.posts[i as usize * 4 + 1] as f64 / 10.0);
                    eo_street::tile_key(x, z) == key
                }));
            }
        }
    }
    idx.sort_unstable();
    let mut placed: Vec<PlacedPost> = vec![];
    let mut seats: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let max_yaw = MERGE_YAW_DEG.to_radians();
    for i in idx {
        let o = i as usize * 4;
        let (lx, lz) = (inv.posts[o] as f64 / 10.0, inv.posts[o + 1] as f64 / 10.0);
        let n = inv.posts[o + 3] as usize;
        let first = w.sign_plate_start[i as usize] as usize * 4;
        let mut stack = vec![];
        let mut lower = 0.0;
        let mut post_yaw: Option<f64> = None;
        for k in 0..n {
            let q = first + k * 4;
            let (face, wi, hi, code) = (inv.plates[q], inv.plates[q + 1], inv.plates[q + 2], inv.plates[q + 3]);
            if code != 255 && post_yaw.is_none() {
                post_yaw = Some(plate_yaw(code, 0.0));
            }
            let h = hi as f64 * INCH_M;
            stack.push((face as u16, wi as f64 * INCH_M, h, code));
            if k > 0 {
                lower += h;
            }
        }
        if stack.is_empty() {
            continue;
        }
        let edge = stack.iter().fold(0.0f64, |m, p| m.max(p.1 / 2.0));
        let top = w.std.sign_mount_min.max(w.std.sign_mount_secondary_min + lower) + stack[0].2;
        let seat = match structures {
            None => snap(w, lx, lz, edge, post_r, paved),
            Some(s) => {
                // the post and its widest plate, from the ground to its top.
                let free = |x: f64, z: f64| {
                    let g = w.ground(x, z);
                    !crate::clearance::blocked(s, &crate::clearance::Extent { x, z, r: edge.max(post_r), y0: g, y1: g + top }, None)
                };
                snap_with(w, lx, lz, edge, post_r, paved, &free)
            }
        };
        let Some((x, z, moved)) = seat else { continue };
        let (gx, gz) = ((x / MERGE_M).floor() as i64, (z / MERGE_M).floor() as i64);
        // One pole per spot (`merge_posts`): the nearest post placed within the merge radius takes the plates.
        if merge_posts() {
            let mut host: Option<(f64, usize)> = None;
            for di in -1..=1 {
                for dj in -1..=1 {
                    for &p in seats.get(&(gx + di, gz + dj)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let d = (placed[p].seat[0] - x).hypot(placed[p].seat[1] - z);
                        if d <= MERGE_M && host.is_none_or(|h| d < h.0) {
                            host = Some((d, p));
                        }
                    }
                }
            }
            if let Some((_, p)) = host {
                mount_on(&mut placed[p], &stack, top, post_yaw, post_r, w.std.sign_mount_secondary_min);
                continue;
            }
        }
        // F3: a post repeating every plate of a post placed within 0.9 m.
        let repeats = (-1..=1).any(|di| {
            (-1..=1).any(|dj| {
                seats.get(&(gx + di, gz + dj)).is_some_and(|v| {
                    v.iter().any(|&p| {
                        let other = &placed[p];
                        (other.seat[0] - x).hypot(other.seat[1] - z) <= MERGE_M
                            && stack.iter().all(|pl| {
                                let yaw = plate_yaw(pl.3, post_yaw.unwrap_or(0.0));
                                other.plates.iter().any(|q| q.0 == pl.0 && yaw_gap(q.1, yaw) <= max_yaw)
                            })
                    })
                })
            })
        });
        if repeats {
            continue;
        }
        let ground = w.ground(x, z);
        let mut cursor = top;
        let mut plates = vec![];
        let mut spans = vec![];
        let mut props = vec![];
        for pl in &stack {
            let bottom = cursor - pl.2;
            cursor = bottom;
            let yaw = plate_yaw(pl.3, post_yaw.unwrap_or(0.0));
            plates.push((pl.0, yaw, bottom + pl.2 / 2.0));
            spans.push((yaw, bottom, bottom + pl.2));
            props.push(Prop {
                model: Model::SignPlate,
                pos: [x + yaw.sin() * post_r, ground + bottom + pl.2 / 2.0, z + yaw.cos() * post_r],
                yaw,
                scale: [pl.1 as f32, pl.2 as f32, 1.0],
                variant: pl.0,
                live: Live::None,
            });
        }
        props.push(Prop { model: Model::SignPost, pos: [x, ground, z], yaw: post_yaw.unwrap_or(0.0), scale: [1.0, top as f32, 1.0], variant: 0, live: Live::None });
        seats.entry((gx, gz)).or_default().push(placed.len());
        placed.push(PlacedPost { index: i, listed: [lx, lz], seat: [x, z], moved_m: moved, plates, spans, ground, top, props });
    }
    placed
}

impl PropsWorld {
    fn sign_cells_get(&self, cx: i32, cz: i32) -> Option<&Vec<u32>> {
        self.sign_cells.get(&(cx, cz))
    }
}
