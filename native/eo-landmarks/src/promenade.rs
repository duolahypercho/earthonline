//! The two promenade seawalls of the ocean shore, ported from
//! src/render/ocean-beach-seawall.js (`planOceanBeachSeawall` and the
//! concrete of `installOceanBeachSeawall`: the walk slab panelled by tone and
//! scored, blown sand along the parapet's foot, the planted bed's soil, the
//! kerb, the stair flights at every numbered stair, the panelled parapet and
//! cap and the stepped seaward face) and src/render/sunset-dunes-seawall.js
//! (`buildSunsetDunesSeawall`: cap, battered face, stained foot and the six
//! OSM stair flights). Section constants are the modules' own, exported.
//!
//! Not ported: the ice-plant mats (sunset-dunes-iceplant.js, their own
//! material), the planted edge's shrubs (ez-tree), the benches and bins
//! (street props), the handrails, and the beach sand sheet the JS reads the
//! seaward heights from (the drawn ground stands in for it here).

use crate::geometry::Rgb;
use crate::set::flat_mesh;
use crate::Frozen;
use serde_json::Value;

/// A soup of flat-shaded quads with one colour each (the modules' `Soup`).
#[derive(Default)]
struct Soup {
    p: Vec<[f64; 3]>,
    c: Vec<Rgb>,
}

enum Solid<'a> {
    Floor(&'a mut Surfaces),
    Wall(&'a mut Surfaces),
    Detail,
}

impl Soup {
    fn quad(&mut self, a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3], col: Rgb) {
        for v in [a, b, c, a, c, d] {
            self.p.push(v);
            self.c.push(col);
        }
    }
    /// Emit the visible prism and route its structural faces into the contact mesh.
    #[allow(clippy::too_many_arguments)]
    fn prism(&mut self, s0a: [f64; 2], s0b: [f64; 2], s1a: [f64; 2], s1b: [f64; 2], y0b: f64, y0t: f64, y1b: f64, y1t: f64, col: Rgb, solid: Solid<'_>) {
        let a = [s0a[0], y0t, s0a[1]];
        let b = [s0b[0], y0t, s0b[1]];
        let c = [s1b[0], y1t, s1b[1]];
        let d = [s1a[0], y1t, s1a[1]];
        let ab = [s0a[0], y0b, s0a[1]];
        let bb = [s0b[0], y0b, s0b[1]];
        let cb = [s1b[0], y1b, s1b[1]];
        let db = [s1a[0], y1b, s1a[1]];
        let k = |c: Rgb, f: f64| [c[0] * f, c[1] * f, c[2] * f];
        let top = self.p.len();
        self.quad(a, d, c, b, col);
        let sides = self.p.len();
        self.quad(ab, a, b, bb, k(col, 0.9));
        self.quad(db, cb, c, d, k(col, 0.9));
        self.quad(ab, db, d, a, k(col, 0.94));
        self.quad(bb, b, c, cb, k(col, 0.94));
        match solid {
            Solid::Floor(s) => {
                s.walk.extend(faces(&self.p[top..sides]).map(upward));
                s.barrier.extend(faces(&self.p[sides..]));
            }
            Solid::Wall(s) => s.barrier.extend(faces(&self.p[top..])),
            Solid::Detail => {}
        }
    }
    fn freeze(&self, name: &str) -> Frozen {
        let idx: Vec<u32> = (0..self.p.len() as u32).collect();
        let mut f = flat_mesh(name, &self.p, &idx, [1.0; 3]);
        for (i, c) in self.c.iter().enumerate() {
            f.colors[3 * i..3 * i + 3].copy_from_slice(&[c[0] as f32, c[1] as f32, c[2] as f32]);
        }
        f
    }
}

fn hash(n: f64) -> f64 {
    let s = (n * 127.1 + 311.7).sin() * 43758.5453;
    s - s.floor()
}

fn rgb(v: &Value) -> Rgb {
    [v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0), v[2].as_f64().unwrap_or(0.0)]
}

const WARM: Rgb = [1.05, 1.0, 0.88];

fn mul(a: Rgb, b: Rgb) -> Rgb {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
fn sc(a: Rgb, k: f64) -> Rgb {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// Structural faces shared with player collision and crowd footing, in the
/// same single-precision world coordinates as the drawn mesh.
#[derive(Clone, Debug, Default)]
pub struct Surfaces {
    pub name: &'static str,
    pub walk: Vec<[[f32; 3]; 3]>,
    pub barrier: Vec<[[f32; 3]; 3]>,
}

fn faces(points: &[[f64; 3]]) -> impl Iterator<Item = [[f32; 3]; 3]> + '_ {
    points.chunks_exact(3).map(|f| [f[0], f[1], f[2]].map(|p| p.map(|v| v as f32)))
}

fn upward(mut face: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let [a, b, c] = face;
    if (b[2] - a[2]) * (c[0] - a[0]) < (b[0] - a[0]) * (c[2] - a[2]) {
        face.swap(1, 2);
    }
    face
}

/// A headless world uses the same planner and ground samples as the scene.
pub fn collision(ground: &dyn Fn(f64, f64) -> f64) -> Vec<Surfaces> {
    let p = &crate::LandmarksJs::get().promenades;
    let a = &p["structureAlbedo"];
    [plan_ocean_beach(p, a, ground, ground), plan_sunset_dunes(p, a, ground, ground)]
        .into_iter().flatten().map(|p| p.surfaces).collect()
}

struct Plan {
    concrete: Soup,
    planting: Soup,
    sand: Soup,
    surfaces: Surfaces,
    stations: usize,
    stairs: usize,
}

/// What a promenade builds: its concrete (casts), and its planting and sand skins (receive only).
pub struct Promenade {
    pub surfaces: Surfaces,
    pub concrete: Frozen,
    pub planting: Frozen,
    pub sand: Frozen,
    pub stations: usize,
    pub stairs: usize,
}

/// The Ocean Beach seawall and promenade along the Esplanade (all of it: the JS draws the stations inside its window).
pub fn ocean_beach(p: &Value, albedo: &Value, land_at: &dyn Fn(f64, f64) -> f64, sand_at: &dyn Fn(f64, f64) -> f64) -> Option<Promenade> {
    let plan = plan_ocean_beach(p, albedo, land_at, sand_at)?;
    Some(Promenade { concrete: plan.concrete.freeze("ocean-beach-seawall:concrete"),
        planting: plan.planting.freeze("ocean-beach-seawall:planting"), sand: plan.sand.freeze("ocean-beach-seawall:sand"),
        surfaces: plan.surfaces, stations: plan.stations, stairs: plan.stairs })
}

fn plan_ocean_beach(p: &Value, albedo: &Value, land_at: &dyn Fn(f64, f64) -> f64, sand_at: &dyn Fn(f64, f64) -> f64) -> Option<Plan> {
    let o = &p["oceanBeach"];
    let s = &o["SECTION"];
    let n = |k: &str| s[k].as_f64().unwrap();
    let (a, b) = ((o["ESPLANADE"][0]["x"].as_f64()?, o["ESPLANADE"][0]["z"].as_f64()?), (o["ESPLANADE"][1]["x"].as_f64()?, o["ESPLANADE"][1]["z"].as_f64()?));
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let (dx, dz) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
    let (nx, nz) = (-dz, dx);
    let station_m = o["STATION_M"].as_f64()?;
    let stair_gap = o["STAIR_GAP_M"].as_f64()?;
    let panel_m = o["PANEL_M"].as_f64()?;
    let walk_panel = o["WALK_PANEL_M"].as_f64()?;
    let stairs_z: Vec<f64> = o["STAIRS_Z"].as_array()?.iter().filter_map(|v| v.as_f64()).collect();
    struct St {
        s: f64,
        x: f64,
        z: f64,
        walk_y: f64,
        sand_y: f64,
        stair: bool,
        plant_base: f64,
        kerb_base: f64,
    }
    let mut sts = Vec::new();
    let mut st = 0.0;
    while st <= len + 1e-6 {
        let (x, z) = (a.0 + dx * st, a.1 + dz * st);
        let at = |u: f64| (x - nx * u, z - nz * u);
        let land = |u: f64| {
            let q = at(u);
            land_at(q.0, q.1)
        };
        let walk_y = land(-2.3).max(land(0.0)).max(land(2.3)) + n("walkH");
        let sp = at(n("parapetSea") - 1.2);
        let sand = sand_at(sp.0, sp.1);
        let stair = stairs_z.iter().any(|sz| ((z - sz) / dz).abs() < stair_gap / 2.0);
        sts.push(St { s: st, x, z, walk_y, sand_y: if sand.is_finite() { sand } else { walk_y - 1.0 }, stair, plant_base: land(3.25), kerb_base: land(4.3) });
        st += station_m;
    }
    if sts.is_empty() {
        return None;
    }
    let off = |st: &St, u: f64| [st.x - nx * u, st.z - nz * u];
    let parapet_col = mul(rgb(&albedo["barrier"]), WARM);
    let walk_col = sc(mul(rgb(&albedo["fascia"]), WARM), 0.78);
    let kerb_col = rgb(&albedo["barrier"]);
    let face_col = rgb(&albedo["pier"]);
    let foot_col = rgb(&albedo["pierFoot"]);
    let (mut walkable, mut barrier, mut plants, mut sand, mut decor) = (Soup::default(), Soup::default(), Soup::default(), Soup::default(), Soup::default());
    let mut surfaces = Surfaces { name: "Ocean Beach Esplanade", ..Default::default() };
    let mut stairs = 0;
    for i in 0..sts.len() - 1 {
        let (p, q) = (&sts[i], &sts[i + 1]);
        if q.s - p.s > station_m + 1e-3 {
            continue;
        }
        let tone = |period: f64, k: f64, c: Rgb| sc(c, 0.94 + 0.08 * hash((p.s / period).floor() + k));
        walkable.prism(off(p, n("parapetLand")), off(p, n("walkLand")), off(q, n("parapetLand")), off(q, n("walkLand")), p.walk_y - n("walkH") - 0.25, p.walk_y, q.walk_y - n("walkH") - 0.25, q.walk_y, tone(walk_panel, 7.0, walk_col), Solid::Floor(&mut surfaces));
        if (q.s / walk_panel).floor() != (p.s / walk_panel).floor() {
            let js = (q.s / walk_panel).floor() * walk_panel;
            let t = (js - p.s) / (q.s - p.s);
            let at = |u: f64, dt: f64| {
                let (a, b) = (off(p, u), off(q, u));
                let k = (t + dt / (q.s - p.s)).clamp(0.0, 1.0);
                [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k]
            };
            let y = p.walk_y + (q.walk_y - p.walk_y) * t + 0.004;
            let (j0, j1, j2, j3) = (at(n("parapetLand"), -0.015), at(n("walkLand"), -0.015), at(n("walkLand"), 0.015), at(n("parapetLand"), 0.015));
            decor.quad([j0[0], y, j0[1]], [j3[0], y, j3[1]], [j2[0], y, j2[1]], [j1[0], y, j1[1]], sc(walk_col, 0.55));
        }
        // Blown sand along the parapet's foot, a fan at every stair gap.
        {
            let cell = |k: f64| hash(k.floor() * 1.73 + 4.1);
            let smooth = |s0: f64| {
                let f = s0 / 7.0;
                let i = f.floor();
                let t = f - i;
                let e = t * t * (3.0 - 2.0 * t);
                cell(i) * (1.0 - e) + cell(i + 1.0) * e
            };
            let w_of = |st: &St| if st.stair { (n("walkLand") - n("parapetLand")) * (0.65 + 0.3 * smooth(st.s + 50.0)) } else { 0.15 + 1.25 * smooth(st.s).powf(1.6) };
            let (wp, wq) = (w_of(p), w_of(q));
            let k = 0.95 + 0.1 * hash(p.s * 1.7);
            let sand_col = [0.45 * k, 0.37 * k, 0.25 * k];
            let blend = [(sand_col[0] + walk_col[0]) * 0.5, (sand_col[1] + walk_col[1]) * 0.5, (sand_col[2] + walk_col[2]) * 0.5];
            let pl = n("parapetLand");
            let rails = [(pl, sand_col, 0.006), (pl + 0.55, sand_col, 0.004), (pl + 1.0, blend, 0.0015)];
            for r in 0..rails.len() - 1 {
                let (up0, up1) = (pl + (rails[r].0 - pl) * wp, pl + (rails[r + 1].0 - pl) * wp);
                let (uq0, uq1) = (pl + (rails[r].0 - pl) * wq, pl + (rails[r + 1].0 - pl) * wq);
                let (a, b, c, d) = (off(p, up0), off(p, up1), off(q, uq1), off(q, uq0));
                let (ya, yb, yc, yd) = (p.walk_y + rails[r].2, p.walk_y + rails[r + 1].2, q.walk_y + rails[r + 1].2, q.walk_y + rails[r].2);
                let (ca, cb) = (rails[r].1, rails[r + 1].1);
                for (v, col) in [([a[0], ya, a[1]], ca), ([d[0], yd, d[1]], ca), ([c[0], yc, c[1]], cb), ([a[0], ya, a[1]], ca), ([c[0], yc, c[1]], cb), ([b[0], yb, b[1]], cb)] {
                    sand.p.push(v);
                    sand.c.push(col);
                }
            }
        }
        // The planted edge's soil bed (its ice-plant mats are not ported).
        let bed_p = p.plant_base.max(p.walk_y - n("walkH")) + 0.02;
        let bed_q = q.plant_base.max(q.walk_y - n("walkH")) + 0.02;
        let soil = 0.92 + 0.12 * hash(p.s * 2.3);
        plants.prism(off(p, n("walkLand")), off(p, n("plantLand")), off(q, n("walkLand")), off(q, n("plantLand")), p.plant_base - 0.2, bed_p, q.plant_base - 0.2, bed_q, [0.44 * soil, 0.39 * soil, 0.29 * soil], Solid::Floor(&mut surfaces));
        walkable.prism(off(p, n("plantLand")), off(p, n("kerbLand")), off(q, n("plantLand")), off(q, n("kerbLand")), p.kerb_base - 0.25, p.kerb_base + n("kerbH"), q.kerb_base - 0.25, q.kerb_base + n("kerbH"), kerb_col, Solid::Floor(&mut surfaces));
        if p.stair || q.stair {
            if p.stair && !q.stair {
                stairs += 1;
            }
            let drop = (p.walk_y - p.sand_y).clamp(0.0, n("maxDrop"));
            let steps = (drop / n("stairRiseMax")).ceil() as i64;
            let base = p.sand_y - 0.3;
            for k in 0..steps {
                let top_p = p.walk_y - (k + 1) as f64 * (drop / steps as f64);
                let top_q = q.walk_y - (k + 1) as f64 * (drop / steps as f64);
                let u0 = n("parapetLand") - k as f64 * n("stairTread");
                let u1 = u0 - n("stairTread");
                walkable.prism(off(p, u1), off(p, u0), off(q, u1), off(q, u0), (top_p - 0.25).min(base), top_p, (top_q - 0.25).min(base), top_q, if k == steps - 1 { foot_col } else { walk_col }, Solid::Floor(&mut surfaces));
            }
            continue;
        }
        let joint = if (q.s / panel_m).floor() != (p.s / panel_m).floor() { 0.02 } else { 0.0 };
        let qq = St { s: q.s, x: q.x - dx * joint, z: q.z - dz * joint, walk_y: q.walk_y, sand_y: q.sand_y, stair: q.stair, plant_base: q.plant_base, kerb_base: q.kerb_base };
        let (p_top, q_top) = (p.walk_y + n("parapetH"), q.walk_y + n("parapetH"));
        let p_base = (p.walk_y - n("walkH")).min(p.sand_y) - 0.3;
        let q_base = (q.walk_y - n("walkH")).min(q.sand_y) - 0.3;
        let pc = tone(panel_m, 3.0, parapet_col);
        barrier.prism(off(p, n("parapetSea")), off(p, n("parapetLand")), off(&qq, n("parapetSea")), off(&qq, n("parapetLand")), p_base, p_top, q_base, q_top, pc, Solid::Wall(&mut surfaces));
        let cap = [(pc[0] * 1.05).min(1.0), (pc[1] * 1.05).min(1.0), (pc[2] * 1.05).min(1.0)];
        barrier.prism(off(p, n("parapetSea") - n("capOver")), off(p, n("parapetLand") + n("capOver")), off(&qq, n("parapetSea") - n("capOver")), off(&qq, n("parapetLand") + n("capOver")), p_top, p_top + n("capH"), q_top, q_top + n("capH"), cap, Solid::Wall(&mut surfaces));
        let drop = (p.walk_y - p.sand_y).clamp(0.0, n("maxDrop"));
        let steps = (drop / n("stepRise")).floor() as i64;
        for k in 0..steps {
            let top = p.walk_y - (k + 1) as f64 * n("stepRise");
            let u0 = n("parapetSea") - k as f64 * n("stepRun");
            let u1 = u0 - n("stepRun");
            barrier.prism(off(p, u1), off(p, u0), off(&qq, u1), off(&qq, u0), p_base, top, q_base, top, if k == steps - 1 { foot_col } else { face_col }, Solid::Wall(&mut surfaces));
        }
    }
    let mut concrete = walkable;
    concrete.p.extend(barrier.p);
    concrete.c.extend(barrier.c);
    concrete.p.extend(decor.p);
    concrete.c.extend(decor.c);
    Some(Plan { concrete, planting: plants, sand, surfaces, stations: sts.len(), stairs })
}

/// The Sunset Dunes seawall and its six stair flights.
pub fn sunset_dunes(p: &Value, albedo: &Value, land_at: &dyn Fn(f64, f64) -> f64, sand_at: &dyn Fn(f64, f64) -> f64) -> Option<Promenade> {
    let plan = plan_sunset_dunes(p, albedo, land_at, sand_at)?;
    Some(Promenade { concrete: plan.concrete.freeze("sunset-dunes-seawall"), planting: Frozen::default(), sand: Frozen::default(),
        surfaces: plan.surfaces, stations: plan.stations, stairs: plan.stairs })
}

fn plan_sunset_dunes(p: &Value, albedo: &Value, land_at: &dyn Fn(f64, f64) -> f64, sand_at: &dyn Fn(f64, f64) -> f64) -> Option<Plan> {
    let o = &p["sunsetDunes"];
    let g = |k: &str| o[k].as_f64().unwrap();
    let (a, b) = ([o["AXIS"]["a"][0].as_f64()?, o["AXIS"]["a"][1].as_f64()?], [o["AXIS"]["b"][0].as_f64()?, o["AXIS"]["b"][1].as_f64()?]);
    let l = (b[0] - a[0]).hypot(b[1] - a[1]);
    let (dx, dz) = ((b[0] - a[0]) / l, (b[1] - a[1]) / l);
    let (nx, nz) = (-dz, dx);
    let off_m = g("WALL_OFFSET_M");
    let line_at = |z: f64| {
        let s = (z - a[1]) / dz;
        a[0] + dx * s + nx * off_m
    };
    let (z0, z1) = (o["WALL_Z"][0].as_f64()?, o["WALL_Z"][1].as_f64()?);
    let (min_drop, cap_m, station, riser, tread, flight_w) = (g("WALL_MIN_DROP_M"), g("CAP_M"), g("STATION_M"), g("RISER_MAX"), g("TREAD_MIN"), g("FLIGHT_W"));
    let mut sts: Vec<[f64; 4]> = Vec::new();
    let mut z = z0;
    while z <= z1 + 1e-6 {
        let x = line_at(z);
        let top = land_at(x - nx * 0.8, z - nz * 0.8);
        let mut foot = sand_at(x + nx * 1.5, z + nz * 1.5);
        if !foot.is_finite() || top - foot < min_drop {
            foot = top - min_drop;
        }
        sts.push([x, z, top, foot]);
        z += station;
    }
    if sts.is_empty() {
        return None;
    }
    let face_col = mul(rgb(&albedo["barrier"]), WARM);
    let cap_col = sc(face_col, 1.05);
    let foot_col = mul(rgb(&albedo["pierFoot"]), WARM);
    let off = |x: f64, z: f64, u: f64| [x + nx * u, z + nz * u];
    let mut concrete = Soup::default();
    let mut surfaces = Surfaces { name: "Sunset Dunes seawall", ..Default::default() };
    for i in 0..sts.len() - 1 {
        let ([px, pz, pt, pf], [qx, qz, qt, qf]) = (sts[i], sts[i + 1]);
        if qz - pz > station + 1e-3 {
            continue;
        }
        concrete.prism(off(px, pz, 0.0), off(px, pz, -cap_m), off(qx, qz, 0.0), off(qx, qz, -cap_m), pt - 0.3, pt + 0.05, qt - 0.3, qt + 0.05, cap_col, Solid::Wall(&mut surfaces));
        concrete.prism(off(px, pz, 0.25), off(px, pz, 0.0), off(qx, qz, 0.25), off(qx, qz, 0.0), pf - 0.6, pt - 0.3, qf - 0.6, qt - 0.3, face_col, Solid::Wall(&mut surfaces));
        concrete.prism(off(px, pz, 0.32), off(px, pz, 0.25), off(qx, qz, 0.32), off(qx, qz, 0.25), pf - 0.6, pf + 0.45, qf - 0.6, qf + 0.45, foot_col, Solid::Wall(&mut surfaces));
    }
    let mut flights = 0;
    for f in o["steps"].as_array()? {
        let pts = f["pts"].as_array()?;
        let (zn, zs) = (pts[1][1].as_f64()?, pts[3][1].as_f64()?);
        if zs - zn < 2.0 {
            continue;
        }
        let (xn, xs) = (line_at(zn), line_at(zs));
        let top = land_at(xn - nx * 0.8, zn - nz * 0.8);
        let mut foot = sand_at(xs + nx * (flight_w + 1.5), zs + nz * (flight_w + 1.5));
        if !foot.is_finite() || top - foot < 0.5 {
            foot = top - min_drop;
        }
        let drop = top - foot;
        let steps = ((drop / riser).ceil() as i64).max(2);
        let run = zs - zn;
        let flight = run.min(steps as f64 * tread);
        let z_start = zs - flight;
        let rise = drop / steps as f64;
        if z_start > zn + 0.05 {
            concrete.prism(off(line_at(zn), zn, 0.25 + flight_w), off(line_at(zn), zn, 0.25), off(line_at(z_start), z_start, 0.25 + flight_w), off(line_at(z_start), z_start, 0.25), foot - 0.6, top, foot - 0.6, top, cap_col, Solid::Floor(&mut surfaces));
        }
        for k in 0..steps {
            let za = z_start + k as f64 * tread;
            let zb = za + tread;
            let y = top - (k + 1) as f64 * rise;
            concrete.prism(off(line_at(za), za, 0.25 + flight_w), off(line_at(za), za, 0.25), off(line_at(zb), zb, 0.25 + flight_w), off(line_at(zb), zb, 0.25), foot - 0.6, y, foot - 0.6, y, cap_col, Solid::Floor(&mut surfaces));
            let zna = zb - 0.06;
            concrete.prism(off(line_at(zna), zna, 0.25 + flight_w), off(line_at(zna), zna, 0.25), off(line_at(zb), zb, 0.25 + flight_w), off(line_at(zb), zb, 0.25), y, y + 0.004, y, y + 0.004, foot_col, Solid::Detail);
        }
        flights += 1;
    }
    Some(Plan { concrete, planting: Soup::default(), sand: Soup::default(), surfaces, stations: sts.len(), stairs: flights })
}
