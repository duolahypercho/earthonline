//! The rear yards' objects (`--yard-detail` with the canopy map, eo-props
//! `yards::items` places them, eo-app yards.rs draws them; `--no-yard-objects`
//! leaves them out and the library as before): what a city backyard holds
//! besides its plants, so a yard is not an empty bed. Render models
//! `YARD_ITEM_BASE + k`, three levels each, built in code like the street
//! furniture (`furniture_models.rs`); colour is the vertex colour on a white
//! material.
//!
//! Sizes (REF where cited, else INFERRED from common products):
//! - The collection carts: Recology's 32-gallon carts, the city's black
//!   (landfill), blue (recycling) and green (compost) bins (Recology San
//!   Francisco's cart sizes: about 0.5 m wide, 0.6 m deep, 0.95 m tall).
//! - A patio set: a 0.9 m round table 0.72 m high with four chairs (seat
//!   0.45 m, back 0.85 m), weathered teak; a black steel cafe set, a 0.6 m
//!   table and two chairs.
//! - A kettle grill: a 0.57 m bowl on three legs, its lid's top 1.0 m up.
//! - Pots: a 0.45 m terracotta pot (a 16 in pot), a 0.55 m glazed pot, a half
//!   wine barrel (0.66 m across, 0.42 m tall), each holding soil a few cm under
//!   its rim (eo-app plants a shrub in it).
//! - A raised bed: 2.4 x 1.2 m of 2x8 boards (two courses, 0.36 m), the
//!   usual kit size; soil 4 cm under the top.
//! - A shed: 1.8 x 1.2 m, walls 1.95 m at the front and 1.75 m at the back
//!   under a shed roof (a lean-to's, its high side the door's).
//!
//! Colours are weathered (the yards are old, matte and sun-faded), the
//! plastics and enamel a little glossier.

use super::*;
use std::f32::consts::TAU;

/// The carts: black, blue, green (`CART + i`).
pub const CART: u32 = 0;
pub const PATIO_WOOD: u32 = 3;
pub const PATIO_METAL: u32 = 4;
pub const GRILL: u32 = 5;
pub const POT_TERRACOTTA: u32 = 6;
pub const POT_GLAZED: u32 = 7;
pub const BARREL: u32 = 8;
pub const RAISED_BED: u32 = 9;
pub const SHED: u32 = 10;
/// How many models (`YARD_ITEM_BASE ..`).
pub const COUNT: u32 = 11;
/// The soil's top in a planted container (m over its foot): a pot, the glazed
/// pot, the half barrel, the raised bed (eo-app plants on it).
pub const SOIL_TOP_M: [f32; 4] = [0.37, 0.50, 0.38, 0.32];

struct Mats {
    plastic: u32,
    enamel: u32,
    wood: u32,
    clay: u32,
    glaze: u32,
    soil: u32,
    metal: u32,
    shingle: u32,
    fabric: u32,
}

/// Weather the mesh being built (every vertex of `out.b`) and close it: the
/// foot darker (splashed soil and algae, the last 0.3 m down to 70 %) and each
/// vertex its own value (+-7 %, a hash of its place on a 12 cm lattice), so no
/// object reads as one flat new colour; then `Out::finish`.
fn done(out: &mut Out) -> u32 {
    for v in &mut out.b.v {
        let [x, y, z] = v.pos;
        let t = (y / 0.3).clamp(0.0, 1.0);
        let foot = 0.7 + 0.3 * t * t * (3.0 - 2.0 * t);
        let q = [(x * 8.0).floor() as i32, (y * 8.0).floor() as i32, (z * 8.0).floor() as i32];
        let mut h = (q[0].wrapping_mul(73_856_093) ^ q[1].wrapping_mul(19_349_663) ^ q[2].wrapping_mul(83_492_791)) as u32;
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        let n = 0.93 + 0.14 * (h & 1023) as f32 / 1023.0;
        for c in 0..3 {
            v.color[c] = (v.color[c] as f32 * foot * n).round().clamp(0.0, 255.0) as u8;
        }
    }
    out.finish()
}

/// Rotate the vertices from `from` about +Y by `yaw` (three.js rotation.y), then move them by `at`.
fn turn(b: &mut Builder, from: usize, yaw: f32, at: [f32; 3]) {
    let (s, c) = yaw.sin_cos();
    for v in &mut b.v[from..] {
        let [x, y, z] = v.pos;
        v.pos = [x * c + z * s + at[0], y + at[1], -x * s + z * c + at[2]];
        let (nx, nz) = (v.normal[0] as f32, v.normal[2] as f32);
        v.normal = [(nx * c + nz * s).round() as i8, v.normal[1], (-nx * s + nz * c).round() as i8, 0];
        // (the grain direction in the UV turns with it, `grain`.)
        let (gx, gz) = (v.uv[0] as f32 / 32767.5 - 1.0, v.uv[1] as f32 / 32767.5 - 1.0);
        let q = |w: f32| ((w * 0.5 + 0.5).clamp(0.0, 1.0) * 65535.0).round() as u16;
        v.uv = [q(gx * c + gz * s), q(-gx * s + gz * c)];
    }
}

/// Mark the vertices from `from` with a piece's grain direction (model space): its
/// x and z in the UV (unorm, -1..1 to 0..1), y the rest, taken upward (a grain has no
/// sense). props_yards.wgsl `pr_yard_item` reads it on the materials marked as wood;
/// the models' materials are untextured, so their UVs are free for it.
pub(super) fn grain(b: &mut Builder, from: usize, d: [f32; 3]) {
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let k = if d[1] < 0.0 { -1.0 / l } else { 1.0 / l };
    let q = |v: f32| ((v * k * 0.5 + 0.5).clamp(0.0, 1.0) * 65535.0).round() as u16;
    for v in &mut b.v[from..] {
        v.uv = [q(d[0]), q(d[2])];
    }
}

/// A box w x h x d centred at `at` (its foot at `at[1] - h / 2`) turned by `yaw`,
/// its grain along its longest side.
pub(super) fn ybox(b: &mut Builder, m: u32, w: f32, h: f32, d: f32, col: [u8; 3], yaw: f32, at: [f32; 3]) {
    let from = b.v.len();
    b.cuboid([-w / 2.0, -h / 2.0, -d / 2.0], [w / 2.0, h / 2.0, d / 2.0], col, m);
    turn(b, from, yaw, at);
    let (sn, cs) = yaw.sin_cos();
    let dir = if h >= w && h >= d { [0.0, 1.0, 0.0] } else if w >= d { [cs, 0.0, -sn] } else { [sn, 0.0, cs] };
    grain(b, from, dir);
}

/// A box whose plan tapers from (w0, d0) at y0 to (w1, d1) at y1, centred on +Y, capped top and bottom.
fn taper(b: &mut Builder, m: u32, y0: f32, y1: f32, w0: f32, d0: f32, w1: f32, d1: f32, col: [u8; 3]) {
    let lo = [[-w0 / 2.0, -d0 / 2.0], [w0 / 2.0, -d0 / 2.0], [w0 / 2.0, d0 / 2.0], [-w0 / 2.0, d0 / 2.0]];
    let hi = [[-w1 / 2.0, -d1 / 2.0], [w1 / 2.0, -d1 / 2.0], [w1 / 2.0, d1 / 2.0], [-w1 / 2.0, d1 / 2.0]];
    for k in 0..4 {
        let (a, c) = (k, (k + 1) % 4);
        let q = [[lo[a][0], y0, lo[a][1]], [lo[c][0], y0, lo[c][1]], [hi[c][0], y1, hi[c][1]], [hi[a][0], y1, hi[a][1]]];
        let e1 = glam::Vec3::from(q[1]) - glam::Vec3::from(q[0]);
        let e2 = glam::Vec3::from(q[3]) - glam::Vec3::from(q[0]);
        let mut n = e1.cross(e2).normalize_or_zero();
        let mid = glam::Vec3::new((q[0][0] + q[1][0]) / 2.0, 0.0, (q[0][2] + q[1][2]) / 2.0);
        if n.dot(mid) < 0.0 {
            n = -n;
        }
        let base = b.v.len() as u32;
        for p in q {
            b.vert(p, n.to_array(), [0.0, 0.0], col, m);
        }
        // (the props draw without face culling: one winding serves.)
        b.tri(false, base, base + 2, base + 1);
        b.tri(false, base, base + 3, base + 2);
    }
    for (y, ring, ny) in [(y0, lo, -1.0f32), (y1, hi, 1.0)] {
        let base = b.v.len() as u32;
        for p in ring {
            b.vert([p[0], y, p[1]], [0.0, ny, 0.0], [0.0, 0.0], col, m);
        }
        b.tri(false, base, base + 1, base + 2);
        b.tri(false, base, base + 2, base + 3);
    }
}

/// A solid disc (both faces) of radius r at height y.
fn disc(b: &mut Builder, m: u32, r: f32, y: f32, sides: u32, col: [u8; 3], at: [f32; 3]) {
    let c = b.vert([at[0], at[1] + y, at[2]], [0.0, 1.0, 0.0], [0.5, 0.5], col, m);
    let ring = b.v.len() as u32;
    for i in 0..=sides {
        let a = i as f32 / sides as f32 * TAU;
        b.vert([at[0] + a.cos() * r, at[1] + y, at[2] + a.sin() * r], [0.0, 1.0, 0.0], [0.5, 0.5], col, m);
    }
    for i in 0..sides {
        b.tri(false, c, ring + i + 1, ring + i);
    }
}

/// One collection cart, its front (the lid's handle side) toward -Z, wheels at the back (+Z).
fn cart(out: &mut Out, m: &Mats, hex: &str) -> [u32; 3] {
    let body = tone(hex, 1.0);
    let lid = tone(hex, 1.08);
    let tyre = tone("#1c1c1d", 1.0);
    let parts = |b: &mut Builder, near: bool| {
        taper(b, m.plastic, 0.06, 0.9, 0.42, 0.5, 0.5, 0.6, body);
        // the rim under the lid, the lid (overhanging, its hinge at the back), its handle lip.
        ybox(b, m.plastic, 0.53, 0.04, 0.63, body, 0.0, [0.0, 0.91, 0.0]);
        ybox(b, m.plastic, 0.55, 0.035, 0.66, lid, 0.0, [0.0, 0.945, -0.01]);
        if near {
            ybox(b, m.plastic, 0.4, 0.05, 0.03, lid, 0.0, [0.0, 0.93, -0.34]);
            // the handle bar across the back, over the axle.
            tube(b, m.plastic, [-0.22, 0.88, 0.33], [0.22, 0.88, 0.33], 0.016, 6, body);
            // the axle housing and its foot.
            ybox(b, m.plastic, 0.46, 0.08, 0.08, body, 0.0, [0.0, 0.11, 0.27]);
            ybox(b, m.plastic, 0.3, 0.06, 0.06, body, 0.0, [0.0, 0.03, -0.18]);
        }
        let sides = if near { 12 } else { 6 };
        for x in [-0.25f32, 0.25] {
            tube(b, m.plastic, [x - 0.03 * x.signum(), 0.1, 0.28], [x + 0.03 * x.signum(), 0.1, 0.28], 0.1, sides, tyre);
        }
    };
    parts(&mut out.b, true);
    let n = done(out);
    parts(&mut out.b, false);
    let md = done(out);
    taper(&mut out.b, m.plastic, 0.0, 0.96, 0.44, 0.52, 0.52, 0.64, body);
    let f = done(out);
    [n, md, f]
}

/// A chair facing -Z at its own origin (seat 0.45 m), wood or steel.
fn chair(b: &mut Builder, mat: u32, col: [u8; 3], slats: bool) {
    let leg = 0.035;
    for (x, z) in [(-0.2f32, -0.2f32), (0.2, -0.2), (-0.2, 0.2), (0.2, 0.2)] {
        ybox(b, mat, leg, 0.45, leg, col, 0.0, [x, 0.225, z]);
    }
    if slats {
        for k in 0..4 {
            ybox(b, mat, 0.46, 0.025, 0.085, col, 0.0, [0.0, 0.46, -0.17 + k as f32 * 0.113]);
        }
        for k in 0..3 {
            ybox(b, mat, 0.46, 0.07, 0.025, col, 0.0, [0.0, 0.58 + k as f32 * 0.11, 0.22]);
        }
    } else {
        ybox(b, mat, 0.46, 0.03, 0.46, col, 0.0, [0.0, 0.46, 0.0]);
        ybox(b, mat, 0.46, 0.3, 0.025, col, 0.0, [0.0, 0.7, 0.22]);
    }
    for x in [-0.2f32, 0.2] {
        ybox(b, mat, leg, 0.42, leg, col, 0.0, [x, 0.66, 0.2]);
    }
}

/// A patio set: a round table and `chairs` chairs round it (the set's own turn), wood or steel.
fn patio(out: &mut Out, m: &Mats, wood: bool) -> [u32; 3] {
    // (teak left out goes silver-grey.)
    let (mat, col, r, chairs) = if wood { (m.wood, tone("#8a8379", 1.0), 0.45f32, 4) } else { (m.enamel, tone("#26272a", 1.0), 0.3, 2) };
    let parts = |b: &mut Builder, near: bool| {
        let sides = if near { 24 } else { 10 };
        let from = b.v.len();
        b.cylinder([0.0, 0.0, 0.0], 0.695, 0.725, r, r, sides, col, mat, true);
        grain(b, from, [1.0, 0.0, 0.0]);
        if near {
            // the apron under the top and the pedestal or four legs.
            let from = b.v.len();
            b.cylinder([0.0, 0.0, 0.0], 0.62, 0.695, r * 0.85, r * 0.85, sides, col, mat, false);
            grain(b, from, [1.0, 0.0, 0.0]);
        }
        if wood {
            for (x, z) in [(-0.25f32, -0.25f32), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                ybox(b, mat, 0.05, 0.69, 0.05, col, 0.0, [x, 0.345, z]);
            }
            // a closed market umbrella through the table: its pole and furled canopy.
            if near {
                tube(b, m.metal, [0.0, 0.0, 0.0], [0.0, 2.3, 0.0], 0.019, 8, tone("#b9b6ae", 1.0));
                lathe(b, m.fabric, &[[0.0, 1.35], [0.07, 1.45], [0.1, 1.9], [0.06, 2.2], [0.0, 2.28]], 10, tone("#cfc6b2", 0.92), [0.0; 3]);
            }
        } else {
            b.cylinder([0.0, 0.0, 0.0], 0.0, 0.69, 0.03, 0.03, 8, col, mat, false);
            b.cylinder([0.0, 0.0, 0.0], 0.0, 0.03, 0.2, 0.18, 12, col, mat, true);
        }
        for k in 0..chairs {
            let yaw = k as f32 / chairs as f32 * TAU + if wood { 0.0 } else { 0.5 };
            let from = b.v.len();
            chair(b, mat, col, wood && near);
            // each chair 0.25 m clear of the table's edge, facing it, a little askew.
            let (s, c) = yaw.sin_cos();
            let d = r + 0.32;
            turn(b, from, yaw + std::f32::consts::PI + 0.12 * (k as f32 - 1.0), [s * d, 0.0, c * d]);
        }
    };
    parts(&mut out.b, true);
    let n = done(out);
    parts(&mut out.b, false);
    let md = done(out);
    let from = out.b.v.len();
    out.b.cylinder([0.0; 3], 0.0, 0.72, r, r, 8, col, mat, true);
    grain(&mut out.b, from, [1.0, 0.0, 0.0]);
    for k in 0..chairs {
        let yaw = k as f32 / chairs as f32 * TAU;
        let (s, c) = yaw.sin_cos();
        ybox(&mut out.b, mat, 0.46, 0.85, 0.46, col, yaw, [s * (r + 0.32), 0.425, c * (r + 0.32)]);
    }
    let f = done(out);
    [n, md, f]
}

/// A kettle grill: a black enamel bowl and lid on three legs, two wheels, a handle.
fn grill(out: &mut Out, m: &Mats) -> [u32; 3] {
    let black = tone("#2a2a2c", 1.0);
    // the legs and the ash pan's ring: aluminium and steel gone dull grey.
    let steel = tone("#55575a", 1.0);
    // a sphere of radius r about y 0.72, bottom up: the lower half the bowl, the upper the lid (a little shallower).
    let bowl = |r: f32, n: u32| -> Vec<[f32; 2]> {
        (0..=n)
            .map(|k| {
                let t = std::f32::consts::PI * (1.0 - k as f32 / n as f32);
                let (s, c) = t.sin_cos();
                [s * r, 0.72 + c * r * if c > 0.0 { 0.72 } else { 1.0 }]
            })
            .collect()
    };
    let parts = |b: &mut Builder, near: bool| {
        let (sides, rows) = if near { (20, 10) } else { (10, 5) };
        lathe(b, m.enamel, &bowl(0.285, rows), sides, black, [0.0; 3]);
        for k in 0..3 {
            let a = k as f32 / 3.0 * TAU + 0.5;
            let (s, c) = a.sin_cos();
            tube(b, m.metal, [s * 0.22, 0.62, c * 0.22], [s * 0.33, 0.0, c * 0.33], 0.012, 6, steel);
        }
        // the lid's rim over the bowl's: a seam of bare steel.
        b.cylinder([0.0, 0.0, 0.0], 0.715, 0.735, 0.29, 0.29, sides, steel, m.metal, false);
        if near {
            // the lid's handle on its two posts, the vent, the bowl's side handle, the ash pan's ring.
            let top = 0.72 + 0.285 * 0.72;
            for x in [-0.06f32, 0.06] {
                tube(b, m.metal, [x, top - 0.01, 0.0], [x, top + 0.045, 0.0], 0.008, 6, steel);
            }
            tube(b, m.plastic, [-0.08, top + 0.05, 0.0], [0.08, top + 0.05, 0.0], 0.016, 8, black);
            b.cylinder([0.0, 0.0, 0.1], top - 0.03, top - 0.005, 0.05, 0.045, 10, steel, m.metal, true);
            tube(b, m.plastic, [-0.07, 0.69, -0.31], [0.07, 0.69, -0.31], 0.014, 6, black);
            b.cylinder([0.0, 0.0, 0.0], 0.25, 0.27, 0.27, 0.27, 16, steel, m.metal, false);
        }
    };
    parts(&mut out.b, true);
    let n = done(out);
    parts(&mut out.b, false);
    let md = done(out);
    out.b.cylinder([0.0; 3], 0.45, 0.98, 0.25, 0.25, 8, black, m.enamel, true);
    out.b.cylinder([0.0; 3], 0.0, 0.45, 0.2, 0.25, 6, steel, m.metal, false);
    let f = done(out);
    [n, md, f]
}

/// A planted container by its profile ([r, y] bottom up, outside) and its soil's
/// radius and top: its lower part (to `stain` of its height) darker with the
/// watering's soil and algae, the rest its own weathered colour.
fn container(out: &mut Out, mat: u32, soil_mat: u32, prof: &[[f32; 2]], soil: (f32, f32), col: [u8; 3], low: [u8; 3], stain: f32) -> [u32; 3] {
    let earth = tone("#4a3d32", 1.0);
    let top = prof.last().map(|p| p[1]).unwrap_or(0.4);
    let cut = stain * top;
    // the profile split at `cut` (the point there interpolated into both halves).
    let mut lower: Vec<[f32; 2]> = vec![];
    let mut upper: Vec<[f32; 2]> = vec![];
    for w in prof.windows(2) {
        let (a, c) = (w[0], w[1]);
        if a[1] <= cut {
            lower.push(a);
        } else {
            upper.push(a);
        }
        if (a[1] - cut) * (c[1] - cut) < 0.0 {
            let t = (cut - a[1]) / (c[1] - a[1]);
            let p = [a[0] + (c[0] - a[0]) * t, cut];
            lower.push(p);
            upper.push(p);
        }
    }
    upper.push(*prof.last().unwrap());
    let parts = |b: &mut Builder, sides: u32| {
        lathe(b, mat, &lower, sides, low, [0.0; 3]);
        lathe(b, mat, &upper, sides, col, [0.0; 3]);
        disc(b, soil_mat, soil.0, soil.1, sides, earth, [0.0; 3]);
    };
    parts(&mut out.b, 20);
    let n = done(out);
    parts(&mut out.b, 10);
    let md = done(out);
    let top = prof.last().map(|p| p[1]).unwrap_or(0.4);
    let r = prof.iter().map(|p| p[0]).fold(0.0, f32::max);
    out.b.cylinder([0.0; 3], 0.0, top, r * 0.85, r, 6, col, mat, true);
    let f = done(out);
    [n, md, f]
}

/// A half wine barrel: oak staves (fluted), two steel hoops, soil.
fn barrel(out: &mut Out, m: &Mats) -> [u32; 3] {
    let oak = tone("#6f5944", 1.0);
    let hoop = tone("#3b3634", 1.0);
    let earth = tone("#4a3d32", 1.0);
    let parts = |b: &mut Builder, near: bool| {
        let sides = if near { 22 } else { 11 };
        let from = b.v.len();
        b.cylinder([0.0; 3], 0.0, 0.42, 0.29, 0.33, sides, oak, m.wood, false);
        grain(b, from, [0.0, 1.0, 0.0]);
        if near {
            for k in 0..sides {
                // the staves' joints: thin dark lines.
                let a = (k as f32 + 0.5) / sides as f32 * TAU;
                let (s, c) = a.sin_cos();
                ybox(b, m.wood, 0.006, 0.42, 0.006, tone("#4a3b2e", 1.0), 0.0, [c * 0.312, 0.21, s * 0.312]);
            }
        }
        for (y, r) in [(0.07f32, 0.297), (0.35, 0.325)] {
            b.cylinder([0.0; 3], y, y + 0.04, r + 0.006, r + 0.006, sides, hoop, m.metal, false);
        }
        // the top rim's thickness and the soil inside.
        let from = b.v.len();
        b.cylinder([0.0; 3], 0.4, 0.42, 0.3, 0.33, sides, oak, m.wood, false);
        grain(b, from, [0.0, 1.0, 0.0]);
        disc(b, m.soil, 0.3, 0.38, sides, earth, [0.0; 3]);
    };
    parts(&mut out.b, true);
    let n = done(out);
    parts(&mut out.b, false);
    let md = done(out);
    out.b.cylinder([0.0; 3], 0.0, 0.42, 0.29, 0.33, 6, oak, m.wood, true);
    grain(&mut out.b, 0, [0.0, 1.0, 0.0]);
    let f = done(out);
    [n, md, f]
}

/// A raised bed of two courses of boards, corner posts, soil.
fn raised_bed(out: &mut Out, m: &Mats) -> [u32; 3] {
    let board = "#7c6452";
    let earth = tone("#3b3129", 1.0);
    let (w, d, h) = (2.4f32, 1.2f32, 0.36f32);
    let parts = |b: &mut Builder, near: bool| {
        let courses = if near { 2 } else { 1 };
        let ch = h / courses as f32;
        for k in 0..courses {
            let y = ch * (k as f32 + 0.5);
            let shade = 1.0 - 0.05 * k as f32;
            for (cx, cz, bw, bd) in [(0.0, -d / 2.0, w, 0.038), (0.0, d / 2.0, w, 0.038), (-w / 2.0, 0.0, 0.038, d), (w / 2.0, 0.0, 0.038, d)] {
                ybox(b, m.wood, bw, ch - if near { 0.006 } else { 0.0 }, bd, tone(board, shade), 0.0, [cx, y, cz]);
            }
        }
        if near {
            for (x, z) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                ybox(b, m.wood, 0.09, h + 0.03, 0.09, tone(board, 0.9), 0.0, [x * (w / 2.0 - 0.06), (h + 0.03) / 2.0, z * (d / 2.0 - 0.06)]);
            }
        }
        // the soil, 4 cm under the top.
        let base = b.v.len() as u32;
        for (x, z) in [(-w / 2.0, -d / 2.0), (w / 2.0, -d / 2.0), (w / 2.0, d / 2.0), (-w / 2.0, d / 2.0)] {
            b.vert([x * 0.98, h - 0.04, z * 0.97], [0.0, 1.0, 0.0], [0.0, 0.0], earth, m.soil);
        }
        b.tri(false, base, base + 2, base + 1);
        b.tri(false, base, base + 3, base + 2);
    };
    parts(&mut out.b, true);
    let n = done(out);
    parts(&mut out.b, false);
    let md = done(out);
    ybox(&mut out.b, m.wood, w, h, d, tone(board, 1.0), 0.0, [0.0, h / 2.0, 0.0]);
    let f = done(out);
    [n, md, f]
}

/// A small shed, its door on the -Z (front, high) side; painted siding, a shingled lean-to roof.
fn shed(out: &mut Out, m: &Mats) -> [u32; 3] {
    let wall = "#7b8270";
    let trim = tone("#cfcbbd", 0.95);
    let roof = tone("#3b3a39", 1.0);
    let (w, d, hf, hb) = (1.8f32, 1.2f32, 1.95f32, 1.75f32);
    let pitch = ((hf - hb) / d).atan();
    let parts = |b: &mut Builder, near: bool| {
        // the walls to the back's height, the front's extra strip and the two gable wedges' boxes.
        ybox(b, m.wood, w, hb, d, tone(wall, 1.0), 0.0, [0.0, hb / 2.0, 0.0]);
        ybox(b, m.wood, w, hf - hb, d * 0.5, tone(wall, 1.0), 0.0, [0.0, hb + (hf - hb) / 2.0, -d * 0.25]);
        // the roof: a slab over the walls, overhanging 8 cm, sloping to the back.
        bx_rot(b, m.shingle, w + 0.16, 0.05, d / pitch.cos() + 0.16, roof, 0.0, -pitch, [0.0, (hf + hb) / 2.0 + 0.03, 0.0]);
        // the door: two leaves, their trim.
        ybox(b, m.wood, 0.9, 1.6, 0.03, tone(wall, 0.85), 0.0, [0.0, 0.82, -d / 2.0 - 0.012]);
        if near {
            ybox(b, m.wood, 0.02, 1.6, 0.035, trim, 0.0, [0.0, 0.82, -d / 2.0 - 0.02]);
            for x in [-0.46f32, 0.46] {
                ybox(b, m.wood, 0.05, 1.66, 0.04, trim, 0.0, [x, 0.83, -d / 2.0 - 0.02]);
            }
            ybox(b, m.wood, 0.97, 0.05, 0.04, trim, 0.0, [0.0, 1.645, -d / 2.0 - 0.02]);
            // corner boards.
            for (x, z) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                ybox(b, m.wood, 0.06, hb, 0.06, trim, 0.0, [x * (w / 2.0 - 0.02), hb / 2.0, z * (d / 2.0 - 0.02)]);
            }
            // the siding's grooves (T1-11 panel, a groove every 8 in): thin dark strips on all four walls.
            let groove = tone(wall, 0.72);
            let mut x = -w / 2.0 + 0.2;
            while x < w / 2.0 - 0.1 {
                for z in [-d / 2.0 - 0.003, d / 2.0 + 0.003] {
                    if z < 0.0 && x.abs() < 0.5 {
                        continue; // (the door)
                    }
                    ybox(b, m.wood, 0.012, hb - 0.02, 0.004, groove, 0.0, [x, hb / 2.0, z]);
                }
                x += 0.2;
            }
            let mut z = -d / 2.0 + 0.2;
            while z < d / 2.0 - 0.1 {
                for x in [-w / 2.0 - 0.003, w / 2.0 + 0.003] {
                    ybox(b, m.wood, 0.004, hb - 0.02, 0.012, groove, 0.0, [x, hb / 2.0, z]);
                }
                z += 0.2;
            }
            // a skid foundation, half sunk.
            for z in [-0.45f32, 0.45] {
                ybox(b, m.wood, w + 0.04, 0.1, 0.09, tone("#5a4c40", 1.0), 0.0, [0.0, 0.0, z]);
            }
        }
    };
    parts(&mut out.b, true);
    let n = done(out);
    parts(&mut out.b, false);
    let md = done(out);
    ybox(&mut out.b, m.wood, w, (hf + hb) / 2.0, d, tone(wall, 1.0), 0.0, [0.0, (hf + hb) / 4.0, 0.0]);
    bx_rot(&mut out.b, m.shingle, w + 0.16, 0.05, d / pitch.cos() + 0.16, roof, 0.0, -pitch, [0.0, (hf + hb) / 2.0 + 0.03, 0.0]);
    let f = done(out);
    [n, md, f]
}

/// Add the yards' object models (`YARD_ITEM_BASE + k`) to the library.
pub(crate) fn cook(out: &mut Out) {
    let white = [255u8; 3];
    let m = Mats {
        plastic: push_mat(out.materials, white, 0.55, 0.0, 0, 0),
        enamel: push_mat(out.materials, white, 0.58, 0.0, 0, 0),
        wood: {
            // bare wood (params.w 2): the grain (props_yards.wgsl `pr_yard_item`).
            let m = push_mat(out.materials, white, 0.88, 0.0, 0, 0);
            out.materials[m as usize].params[3] = 2.0;
            m
        },
        clay: push_mat(out.materials, white, 0.92, 0.0, 0, 0),
        glaze: push_mat(out.materials, white, 0.28, 0.0, 0, 0),
        soil: push_mat(out.materials, white, 1.0, 0.0, 0, 0),
        metal: push_mat(out.materials, white, 0.5, 0.8, 0, 0),
        shingle: push_mat(out.materials, white, 0.95, 0.0, 0, 0),
        // the umbrella's canvas (the wood's roughness, no grain).
        fabric: push_mat(out.materials, white, 0.88, 0.0, 0, 0),
    };
    // (draw distances: a cart or a pot is a few pixels at 120 m; the shed and the bed at 250.)
    let small = [10.0, 35.0, 90.0, 130.0];
    let large = [18.0, 60.0, 160.0, 260.0];
    let base = YARD_ITEM_BASE;
    for (i, hex) in ["#2a2b2d", "#265593", "#2f6a3f"].iter().enumerate() {
        let ids = cart(out, &m, hex);
        out.set(base + CART + i as u32, ids, small, 1, 0.0);
    }
    let ids = patio(out, &m, true);
    out.set(base + PATIO_WOOD, ids, large, 1, 0.0);
    let ids = patio(out, &m, false);
    out.set(base + PATIO_METAL, ids, small, 1, 0.0);
    let ids = grill(out, &m);
    out.set(base + GRILL, ids, small, 1, 0.0);
    // a 16 in terracotta pot, rolled rim; weathered with salts (lighter, pinker than new).
    let ids = container(out, m.clay, m.soil, &[[0.0, 0.0], [0.15, 0.0], [0.2, 0.36], [0.215, 0.365], [0.222, 0.4], [0.2, 0.405], [0.195, 0.39]], (0.195, SOIL_TOP_M[0]), tone("#957a6c", 1.0), tone("#6a5a4f", 1.0), 0.35);
    out.set(base + POT_TERRACOTTA, ids, small, 1, 0.0);
    // a glazed pot, bellied, a deep blue-grey glaze.
    let ids = container(out, m.glaze, m.soil, &[[0.0, 0.0], [0.15, 0.0], [0.22, 0.12], [0.255, 0.3], [0.24, 0.48], [0.2, 0.53], [0.205, 0.55], [0.19, 0.55]], (0.19, SOIL_TOP_M[1]), tone("#3e5466", 1.0), tone("#33404a", 1.0), 0.2);
    out.set(base + POT_GLAZED, ids, small, 1, 0.0);
    let ids = barrel(out, &m);
    out.set(base + BARREL, ids, small, 1, 0.0);
    let ids = raised_bed(out, &m);
    out.set(base + RAISED_BED, ids, large, 1, 0.0);
    let ids = shed(out, &m);
    out.set(base + SHED, ids, large, 2, 0.0);
    out.sources.push("yard objects: carts, patio sets, a grill, pots, a half barrel, a raised bed and a shed built in code (props_lib2/yard_items.rs)".into());
}
