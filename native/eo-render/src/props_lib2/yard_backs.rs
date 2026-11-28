//! The backs of the houses round the rear yards (`--yard-detail` with the
//! canopy map; eo-app `yards/backs.rs` places them from the kit's plans;
//! `--no-yard-stairs` leaves them out and the library as before): the wooden
//! back stairs San Francisco's flats climb from the yard, with a landing and a
//! back door at each flat's floor, and the downspouts at the back corners.
//! Render models `YARD_BACK_BASE + k`, three levels each, built in code like
//! the yards' objects (`yard_items.rs`).
//!
//! A stair is assembled from parts so it fits any floor height: flights of a
//! few riser counts that an instance stretches a little in height, landings,
//! straight railings, posts an instance stretches to the ground, and a door.
//! Every part's paint is the instance's (props_yards.wgsl `pr_yard_item`:
//! the lamp word's colour on the materials marked as paint), so one set of
//! models draws grey, white, stained and bare weathered stairs.
//!
//! Sizes (INFERRED from photographs of the city's back stairs and the
//! residential code's stair limits, CRC R311.7: risers at most 7-3/4 in,
//! treads at least 10 in, guards 36 in, handrails 34-38 in):
//! - flights 0.92 m wide over their stringers, risers 0.1-0.197 m, treads 0.255 m,
//!   open risers, 2x10 stringers, 2x2 balusters at 0.11 m under a 2x4 rail;
//! - landings 1.0 m along the wall and 2.04 m out (two flights side by side:
//!   the switchback's lanes), 2x6 decking on 2x8 joists, 4x4 posts;
//! - a back door 0.86 x 2.08 m, its upper half nine lites, in a 1x4 casing;
//! - a 2 x 3 in rectangular downspout, a conductor head under the roof's
//!   edge and a shoe onto a concrete splash block.

use super::yard_items::{grain, ybox};
use super::*;

/// The flights, by riser count (`FLIGHT + i` has `FLIGHT_RISERS[i]` risers; the
/// first, a few steps down from a low landing, has no guards).
pub const FLIGHT: u32 = 0;
pub const FLIGHT_RISERS: [u32; 7] = [3, 6, 7, 8, 9, 10, 11];
pub const LANDING: u32 = 7;
/// Straight railings 1 m and 2 m long (local +Z), an instance stretches them a little.
pub const RAIL_SHORT: u32 = 8;
pub const RAIL_LONG: u32 = 9;
/// A 4x4 post 1 m tall, stretched to its height.
pub const POST: u32 = 10;
pub const DOOR: u32 = 11;
/// A downspout 1 m long, stretched; its shoe and splash block; its conductor head.
pub const DOWNSPOUT: u32 = 12;
pub const SPOUT_SHOE: u32 = 13;
pub const SPOUT_HEAD: u32 = 14;
/// How many models (`YARD_BACK_BASE ..`).
pub const COUNT: u32 = 15;

/// A flight's riser and tread (m) at scale 1; its run is (risers - 1) treads. An
/// instance stretches it in height to its floors' rise, its risers kept between
/// `RISER_MIN_M` (a gentle last flight) and the code's most, `RISER_MAX_M`
/// (7-3/4 in), the tread the code's least (10 in).
pub const RISER_M: f32 = 0.185;
pub const RISER_MIN_M: f32 = 0.1;
pub const RISER_MAX_M: f32 = 0.197;
pub const TREAD_M: f32 = 0.255;
/// A flight's width over its stringers.
pub const FLIGHT_W_M: f32 = 0.92;
/// A landing's width along the wall and depth out from it (two lanes of flights).
pub const LANDING_W_M: f32 = 1.0;
pub const LANDING_D_M: f32 = 2.04;
/// The lanes' centres out from the wall (clear of the sills and frames).
pub const LANE_W_M: [f32; 2] = [0.58, 1.54];
/// The guard's height over a landing or the nosing line.
pub const RAIL_H_M: f32 = 0.92;
/// The downspout's shoe and head heights (the pipe runs between them).
pub const SHOE_H_M: f32 = 0.25;
pub const HEAD_H_M: f32 = 0.28;

/// The materials' marker (props_yards.wgsl `pr_yard_item`): params.w 1 is painted wood
/// (the instance's paint, worn through to grey wood, the grain's relief under it),
/// 3 painted metal (the paint only), 0 keeps its own colour.
const PAINT: f32 = 1.0;
const PAINT_METAL: f32 = 3.0;

struct Mats {
    wood: u32,
    paint: u32,
    tin: u32,
    glass: u32,
    metal: u32,
    concrete: u32,
}

/// Close a mesh with the yards' per-vertex value noise (`yard_items::done` without
/// its darker foot: these parts stand in the air or stretch).
fn close(out: &mut Out) -> u32 {
    for v in &mut out.b.v {
        let [x, y, z] = v.pos;
        let q = [(x * 8.0).floor() as i32, (y * 8.0).floor() as i32, (z * 8.0).floor() as i32];
        let mut h = (q[0].wrapping_mul(73_856_093) ^ q[1].wrapping_mul(19_349_663) ^ q[2].wrapping_mul(83_492_791)) as u32;
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        let n = 0.92 + 0.16 * (h & 1023) as f32 / 1023.0;
        for c in 0..3 {
            v.color[c] = (v.color[c] as f32 * n).round().clamp(0.0, 255.0) as u8;
        }
    }
    out.finish()
}

/// A box from `a` to `b` (each [x, y, z]) of cross-section w (across, local x) by h
/// (perpendicular in the y-z plane): a stringer or a rail along a slope in the y-z plane.
fn slope_bar(b: &mut Builder, m: u32, a: [f32; 3], c: [f32; 3], w: f32, h: f32, col: [u8; 3]) {
    let (dy, dz) = (c[1] - a[1], c[2] - a[2]);
    let len = (dy * dy + dz * dz).sqrt().max(1e-4);
    // bx_rot's rx: a slab descending toward +z by angle t takes rx = t.
    let t = (-dy).atan2(dz);
    let from = b.v.len();
    bx_rot(b, m, w, h, len, col, 0.0, t, [(a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0, (a[2] + c[2]) / 2.0]);
    grain(b, from, [0.0, dy, dz]);
}

/// A flight of `n` risers, its top nosing line at the origin (y 0 the upper floor),
/// descending toward +Z to its foot at z = (n - 1) treads, y = -n risers; x across.
fn flight(out: &mut Out, m: &Mats, n: u32) -> [u32; 3] {
    let wood = [255u8; 3];
    let (r, t, w) = (RISER_M, TREAD_M, FLIGHT_W_M);
    let run = (n - 1) as f32 * t;
    let drop = n as f32 * r;
    let slope = r / t;
    // the nosing line y(z) = -z * slope (through each tread's leading edge), the foot at -drop.
    let parts = |b: &mut Builder, level: u32| {
        let sx = w / 2.0 - 0.02;
        // the stringers: 2x10s along the nosing line (their middle 12 cm under it), from the
        // upper floor's edge down to where their foot meets the ground.
        let z0 = -0.02f32;
        let z1 = (drop - 0.22) / slope;
        for x in [-sx, sx] {
            slope_bar(b, m.paint, [x, -z0 * slope - 0.12, z0], [x, -z1 * slope - 0.12, z1], 0.04, 0.235, wood);
        }
        // the treads (2x12s, a nosing over the next), open risers.
        if level < 2 {
            for k in 1..n {
                let y = -(k as f32) * r;
                let (za, zb) = ((k - 1) as f32 * t - 0.02, k as f32 * t);
                ybox(b, m.paint, w - 0.01, 0.035, zb - za, wood, 0.0, [0.0, y - 0.0175, (za + zb) / 2.0]);
            }
        } else {
            slope_bar(b, m.paint, [0.0, -0.05, 0.0], [0.0, -drop + 0.05, run], w, 0.06, wood);
        }
        // a few steps down from a low landing: closed risers, no guards (the code asks none under four risers).
        if n <= 3 {
            for k in 0..n {
                let (ya, yb) = (-(k as f32 + 1.0) * r, -(k as f32) * r - 0.035);
                ybox(b, m.paint, w - 0.06, yb - ya, 0.025, wood, 0.0, [0.0, (ya + yb) / 2.0, (k as f32) * t - 0.01]);
            }
            return;
        }
        // the guards on both sides: the rail RAIL_H_M over the nosing line, newels at each end, balusters.
        let gx = w / 2.0 + 0.02;
        for x in [-gx, gx] {
            let top = |z: f32| -z * slope + RAIL_H_M;
            slope_bar(b, m.paint, [x, top(-0.02), -0.02], [x, top(run + 0.05), run + 0.05], 0.04, 0.085, wood);
            if level == 2 {
                continue;
            }
            // the newels: 4x4s from the stringer's foot to the rail at the bottom, at the top a short one.
            ybox(b, m.paint, 0.085, RAIL_H_M + 0.08 + drop - (run + 0.04) * slope, 0.085, wood, 0.0, [x, (-drop + top(run + 0.04) + 0.08) / 2.0, run + 0.04]);
            if level == 0 {
                // a sub-rail along the stringer's top and the balusters between it and the rail.
                slope_bar(b, m.paint, [x, 0.04, -0.02], [x, -run * slope + 0.04, run], 0.04, 0.04, wood);
                let mut z = 0.08f32;
                while z < run - 0.04 {
                    let (ya, yb) = (-z * slope + 0.05, top(z) - 0.04);
                    ybox(b, m.paint, 0.036, yb - ya, 0.036, wood, 0.0, [x, (ya + yb) / 2.0, z]);
                    z += 0.11;
                }
            } else {
                // every third baluster.
                let mut z = 0.3f32;
                while z < run - 0.2 {
                    let (ya, yb) = (-z * slope, top(z));
                    ybox(b, m.paint, 0.04, yb - ya, 0.04, wood, 0.0, [x, (ya + yb) / 2.0, z]);
                    z += 0.45;
                }
            }
        }
    };
    let mut ids = [0; 3];
    for (level, id) in ids.iter_mut().enumerate() {
        parts(&mut out.b, level as u32);
        *id = close(out);
    }
    ids
}

/// A landing: its wall side's middle at the origin, y 0 its deck's top, x along
/// the wall (LANDING_W_M), z out from it (LANDING_D_M); 2x6 decking along the
/// wall on 2x8 joists, a ledger on the wall and a rim joist outside.
fn landing(out: &mut Out, m: &Mats) -> [u32; 3] {
    let wood = [255u8; 3];
    let (w, d) = (LANDING_W_M, LANDING_D_M);
    let parts = |b: &mut Builder, level: u32| {
        if level == 0 {
            let mut z = 0.0f32;
            while z < d - 0.01 {
                let bd = (d - z).min(0.14);
                ybox(b, m.paint, w, 0.03, bd - 0.006, wood, 0.0, [0.0, -0.015, z + bd / 2.0]);
                z += 0.146;
            }
        } else {
            ybox(b, m.paint, w, 0.03, d, wood, 0.0, [0.0, -0.015, d / 2.0]);
        }
        if level < 2 {
            for x in [-w / 2.0 + 0.03, 0.0, w / 2.0 - 0.03] {
                ybox(b, m.paint, 0.045, 0.185, d - 0.1, wood, 0.0, [x, -0.03 - 0.0925, d / 2.0]);
            }
            ybox(b, m.paint, w, 0.185, 0.045, wood, 0.0, [0.0, -0.03 - 0.0925, 0.03]);
        }
        ybox(b, m.paint, w + 0.02, 0.185, 0.045, wood, 0.0, [0.0, -0.03 - 0.0925, d - 0.0225]);
    };
    let mut ids = [0; 3];
    for (level, id) in ids.iter_mut().enumerate() {
        parts(&mut out.b, level as u32);
        *id = close(out);
    }
    ids
}

/// A straight guard `len` long along +Z from the origin (y 0 its floor): a 2x4
/// cap rail, a sub-rail, 2x2 balusters at 0.11 m.
fn rail(out: &mut Out, m: &Mats, len: f32) -> [u32; 3] {
    let wood = [255u8; 3];
    let parts = |b: &mut Builder, level: u32| {
        ybox(b, m.paint, 0.09, 0.04, len, wood, 0.0, [0.0, RAIL_H_M - 0.02, len / 2.0]);
        if level == 2 {
            return;
        }
        ybox(b, m.paint, 0.04, 0.085, len, wood, 0.0, [0.0, RAIL_H_M - 0.085, len / 2.0]);
        ybox(b, m.paint, 0.04, 0.04, len, wood, 0.0, [0.0, 0.08, len / 2.0]);
        let step = if level == 0 { 0.11 } else { 0.33 };
        let mut z = step / 2.0;
        while z < len {
            ybox(b, m.paint, 0.036, RAIL_H_M - 0.2, 0.036, wood, 0.0, [0.0, 0.1 + (RAIL_H_M - 0.2) / 2.0, z]);
            z += step;
        }
    };
    let mut ids = [0; 3];
    for (level, id) in ids.iter_mut().enumerate() {
        parts(&mut out.b, level as u32);
        *id = close(out);
    }
    ids
}

/// A 4x4 post 1 m tall, its foot at the origin.
fn post(out: &mut Out, m: &Mats) -> [u32; 3] {
    let mut ids = [0; 3];
    for id in ids.iter_mut() {
        ybox(&mut out.b, m.paint, 0.09, 1.0, 0.09, [255; 3], 0.0, [0.0, 0.5, 0.0]);
        *id = close(out);
    }
    ids
}

/// A back door on the wall: x along it, z out of it, y 0 the floor; its casing
/// 5 cm proud, the leaf inside it, nine lites over two panels, a knob, a sill.
fn door(out: &mut Out, m: &Mats) -> [u32; 3] {
    let paint = [255u8; 3];
    let glass = tone("#1b1f22", 1.0);
    let brass = tone("#7d6a48", 1.0);
    let (dw, dh) = (0.86f32, 2.08f32);
    let parts = |b: &mut Builder, level: u32| {
        // the casing: two jambs and a head.
        for x in [-(dw / 2.0 + 0.045), dw / 2.0 + 0.045] {
            ybox(b, m.paint, 0.09, dh + 0.1, 0.05, paint, 0.0, [x, (dh + 0.1) / 2.0, 0.025]);
        }
        ybox(b, m.paint, dw + 0.22, 0.1, 0.055, paint, 0.0, [0.0, dh + 0.1, 0.0275]);
        // the leaf, a little in from the casing, its glazing and panels.
        ybox(b, m.paint, dw, dh, 0.03, tone("#ffffff", 0.94), 0.0, [0.0, dh / 2.0, 0.018]);
        ybox(b, m.glass, 0.56, 0.78, 0.01, glass, 0.0, [0.0, 1.52, 0.036]);
        if level == 0 {
            for k in [-1.0f32, 1.0] {
                ybox(b, m.paint, 0.022, 0.78, 0.014, paint, 0.0, [k * 0.093, 1.52, 0.041]);
                ybox(b, m.paint, 0.56, 0.022, 0.014, paint, 0.0, [0.0, 1.52 + k * 0.13, 0.041]);
            }
            for x in [-0.17f32, 0.17] {
                ybox(b, m.paint, 0.26, 0.72, 0.012, tone("#ffffff", 1.04), 0.0, [x, 0.6, 0.039]);
            }
            ybox(b, m.metal, 0.03, 0.05, 0.06, brass, 0.0, [0.33, 0.96, 0.06]);
            // the sill, worn.
            ybox(b, m.wood, dw + 0.2, 0.03, 0.1, tone("#7e7468", 1.0), 0.0, [0.0, 0.015, 0.05]);
        }
    };
    let mut ids = [0; 3];
    for (level, id) in ids.iter_mut().enumerate() {
        parts(&mut out.b, level as u32);
        *id = close(out);
    }
    ids
}

/// A rectangular downspout 1 m long (x along the wall, z out of it, 2 cm off it), stretched.
fn downspout(out: &mut Out, m: &Mats) -> [u32; 3] {
    let mut ids = [0; 3];
    for id in ids.iter_mut() {
        ybox(&mut out.b, m.tin, 0.076, 1.0, 0.057, [255; 3], 0.0, [0.0, 0.5, 0.0485]);
        *id = close(out);
    }
    ids
}

/// The downspout's foot: its last 0.25 m down, the shoe kicking out, a concrete splash block.
fn spout_shoe(out: &mut Out, m: &Mats) -> [u32; 3] {
    let parts = |b: &mut Builder, near: bool| {
        ybox(b, m.tin, 0.076, SHOE_H_M - 0.1, 0.057, [255; 3], 0.0, [0.0, 0.1 + (SHOE_H_M - 0.1) / 2.0, 0.0485]);
        slope_bar(b, m.tin, [0.0, 0.13, 0.06], [0.0, 0.06, 0.3], 0.076, 0.057, [255; 3]);
        if near {
            ybox(b, m.concrete, 0.3, 0.05, 0.6, tone("#8d8a84", 1.0), 0.0, [0.0, 0.01, 0.38]);
        }
    };
    parts(&mut out.b, true);
    let n = close(out);
    parts(&mut out.b, true);
    let md = close(out);
    parts(&mut out.b, false);
    let f = close(out);
    [n, md, f]
}

/// The conductor head at the top of the downspout (its foot at the origin): a
/// box under the roof's edge narrowing to the pipe, the scupper's stub from the wall.
fn spout_head(out: &mut Out, m: &Mats) -> [u32; 3] {
    let parts = |b: &mut Builder| {
        ybox(b, m.tin, 0.076, 0.06, 0.057, [255; 3], 0.0, [0.0, 0.03, 0.0485]);
        ybox(b, m.tin, 0.2, HEAD_H_M - 0.08, 0.16, [255; 3], 0.0, [0.0, 0.06 + (HEAD_H_M - 0.08) / 2.0, 0.1]);
        ybox(b, m.tin, 0.22, 0.02, 0.18, [255; 3], 0.0, [0.0, HEAD_H_M - 0.01, 0.1]);
        ybox(b, m.metal, 0.12, 0.08, 0.06, tone("#5d5f60", 1.0), 0.0, [0.0, HEAD_H_M + 0.02, 0.03]);
    };
    let mut ids = [0; 3];
    for id in ids.iter_mut() {
        parts(&mut out.b);
        *id = close(out);
    }
    ids
}

/// Add the backs' models (`YARD_BACK_BASE + k`) to the library.
pub(crate) fn cook(out: &mut Out) {
    let white = [255u8; 3];
    let paint = |out: &mut Out, rough: f32| {
        let m = push_mat(out.materials, white, rough, 0.0, 0, 0);
        out.materials[m as usize].params[3] = PAINT;
        m
    };
    let tin = {
        let m = push_mat(out.materials, white, 0.62, 0.0, 0, 0);
        out.materials[m as usize].params[3] = PAINT_METAL;
        m
    };
    let m = Mats {
        wood: push_mat(out.materials, white, 0.9, 0.0, 0, 0),
        paint: paint(out, 0.82),
        tin,
        glass: push_mat(out.materials, white, 0.12, 0.0, 0, 0),
        metal: push_mat(out.materials, white, 0.45, 0.8, 0, 0),
        concrete: push_mat(out.materials, white, 0.95, 0.0, 0, 0),
    };
    // (a stair is a few pixels wide past 400 m; its parts' near levels within 25 m.)
    let lod = [25.0, 80.0, 240.0, 400.0];
    let small = [12.0, 40.0, 120.0, 180.0];
    let base = YARD_BACK_BASE;
    for (i, n) in FLIGHT_RISERS.iter().enumerate() {
        let ids = flight(out, &m, *n);
        out.set(base + FLIGHT + i as u32, ids, lod, 2, 0.0);
    }
    let ids = landing(out, &m);
    out.set(base + LANDING, ids, lod, 2, 0.0);
    let ids = rail(out, &m, 1.0);
    out.set(base + RAIL_SHORT, ids, lod, 2, 0.0);
    let ids = rail(out, &m, 2.0);
    out.set(base + RAIL_LONG, ids, lod, 2, 0.0);
    // (a post stretched to 12 m: its sphere is the instance's, scaled.)
    let ids = post(out, &m);
    out.set(base + POST, ids, lod, 2, 0.0);
    let ids = door(out, &m);
    out.set(base + DOOR, ids, lod, 1, 0.0);
    let ids = downspout(out, &m);
    out.set(base + DOWNSPOUT, ids, lod, 1, 0.0);
    let ids = spout_shoe(out, &m);
    out.set(base + SPOUT_SHOE, ids, small, 1, 0.0);
    let ids = spout_head(out, &m);
    out.set(base + SPOUT_HEAD, ids, small, 1, 0.0);
    out.sources.push("yard backs: back stairs (flights, landings, guards, posts), back doors and downspouts built in code (props_lib2/yard_backs.rs)".into());
}
