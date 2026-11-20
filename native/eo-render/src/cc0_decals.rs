//! street wear as world-space decals, placed per 128 m street tile by
//! deterministic rules on the street plan (eo-street), streamed with the
//! tile's near pack through the budgeted upload queue (its records and cells
//! through the upload stager), and applied in the street material's own
//! fragment shading (cc0.wgsl `cc0_street_decals`), so a decal is lit,
//! shadowed, wetted and fogged exactly as the surface it lies on and needs no
//! pass of its own. Every decal draws an ambientCG or Poly Haven scan (or
//! uses one as its mask): see `cc0.wgsl`.
//!
//! Rules (all keyed on the run id and station, so every tile that sees a run
//! places the same decals):
//!
//! * wheel paths: two strips per travel lane at the lane centre +- half a
//!   passenger car's track, merged along straight stretches; lanes are the
//!   gaps between the run's own lane paint (centre, dividers, edge and
//!   parking lines) inside the moving carriageway;
//! * the oil drip line down each lane's centre, and oil drips in the parking
//!   lanes, one chance per stall (`caMetric('parking-stall-desirable-size')`
//!   length), both masked by a stain scan;
//! * tyre tracks: rubber laid down braking in the wheel paths before a run's
//!   intersection approaches (the lanes that travel toward it: right-hand
//!   traffic), and short pull-in tracks in the parking lanes;
//! * sewer manholes on a lane centre, at most one per `MANHOLE_STRETCH_M` of
//!   run (hash); utility vault covers beside the kerb (hash);
//! * storm-drain inlet grates in the gutter at the surveyed inlets (DataSF
//!   `sf-storm-drains.json.gz`, the SFPUC's catch basins and curb inlets: a
//!   survey beats a rule), snapped to the nearest kerbed run's gutter;
//! * utility-cut patches (longitudinal and transverse trenches) and cracked
//!   areas, denser within 1.5 m of the kerb and on busier classes;
//! * on the footway: chewing gum where people wait and walk most (near the
//!   surveyed transit stops and near shopfronts, more on busier classes),
//!   spill stains and cracked slabs.
//!
//! Grime along the gutter, up the kerb face and at the foot of walls is not a
//! decal: it follows the kerb line and the wall base everywhere (cc0.wgsl
//! `cc0_gutter_grime`, `cc0_kerb_grime`, `cc0_wall_base`).
//!
//! A tile's decals cover only its own 32 x 32 grid of 4 m cells, but are
//! gathered from the runs of the tile and its eight neighbours, so a decal
//! crossing a tile edge is drawn on both sides.

use eo_street::StreetWorld;
use std::sync::OnceLock;

/// Decal kinds (cc0.wgsl `CC0_DK_*`).
pub const DK_WHEEL: u32 = 0;
pub const DK_OIL_LINE: u32 = 1;
pub const DK_OIL_SPOT: u32 = 2;
pub const DK_PATCH: u32 = 3;
pub const DK_CRACK: u32 = 4;
pub const DK_MANHOLE: u32 = 5;
pub const DK_VAULT: u32 = 6;
pub const DK_GUM: u32 = 7;
pub const DK_STAIN: u32 = 8;
pub const DK_WALK_CRACK: u32 = 9;
pub const DK_TYRE: u32 = 10;
pub const DK_GRATE: u32 = 11;
/// Weathering only (`--grime`, `set_weathering`): the corner daylighting's red
/// kerb paint (CVC 22500(n), `no-parking-crosswalk-daylighting` on the vehicle
/// approach side), faded and chipped (weathering.wgsl `wx_kerb_paint`).
pub const DK_KERB_PAINT: u32 = 12;
/// Weathering only: dirt along the building line at the walk's back edge.
pub const DK_WALK_EDGE: u32 = 13;

/// Whether street tiles place the weathering decals (`DK_KERB_PAINT`,
/// `DK_WALK_EDGE`): off by default, so every tile's records are as before.
static WEATHERING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Turn the weathering decals on or off (eo-app, with `--grime`, before the street streams).
pub fn set_weathering(on: bool) {
    WEATHERING.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn weathering() -> bool {
    WEATHERING.load(std::sync::atomic::Ordering::Relaxed)
}

/// The kerb paint decal's half-width across the kerb line (m): it covers the
/// kerb face and top (the shader paints only those classes). INFERRED.
pub const KERB_PAINT_HALF_M: f64 = 0.3;
/// The building-line dirt strip's half-width (m), centred on the walk's back edge. INFERRED.
pub const WALK_EDGE_HALF_M: f64 = 0.55;

/// The street tile's grid: 32 x 32 cells of 4 m, at most 8 decals a cell.
pub const CELLS: usize = 32;
pub const CELL_SLOTS: usize = 8;
/// Decal records per tile slot.
pub const MAX_DECALS: usize = 1024;

/// Half a passenger car's track (1.5-1.7 m between tyre centres): the wheel
/// paths' offset from a lane centre. A vehicle dimension, not a street one.
pub const HALF_TRACK_M: f64 = 0.85;
/// A tyre's contact width (the tyre-track strips' width).
pub const TYRE_WIDTH_M: f64 = 0.2;
/// One manhole chance per stretch of run (55 %: about one cover every 80 m, as
/// sewer mains carry one at each junction and change of grade). Utility
/// spacing, not a street standard.
pub const MANHOLE_STRETCH_M: f64 = 45.0;

/// One decal: centre (x, z), unit axis along (x, z); half length, half
/// width, kind (as a float), seed.
#[derive(Clone, Copy, Debug)]
pub struct Decal {
    pub c: [f32; 2],
    pub a: [f32; 2],
    pub half: [f32; 2],
    pub kind: u32,
    pub seed: f32,
}

impl Decal {
    fn priority(&self) -> u32 {
        // street life (`--street-life`; its kinds carry parameters above the low byte):
        // after every other decal, so a full cell drops its pieces first.
        match self.kind & 255 {
            crate::street_life::DK_COLUMN => return 8,
            crate::street_life::DK_LIFE => return 9,
            _ => {}
        }
        match self.kind {
            DK_MANHOLE | DK_VAULT | DK_GRATE => 0,
            DK_PATCH | DK_KERB_PAINT => 1,
            DK_CRACK | DK_WALK_CRACK => 2,
            DK_WHEEL => 3,
            DK_TYRE => 4,
            DK_OIL_SPOT | DK_STAIN => 5,
            DK_OIL_LINE => 6,
            _ => 7,
        }
    }
    /// Plan bounding box.
    fn bounds(&self) -> ([f32; 2], [f32; 2]) {
        let (ax, az) = (self.a[0], self.a[1]);
        let ex = (ax * self.half[0]).abs() + (az * self.half[1]).abs();
        let ez = (az * self.half[0]).abs() + (ax * self.half[1]).abs();
        ([self.c[0] - ex, self.c[1] - ez], [self.c[0] + ex, self.c[1] + ez])
    }
}

/// A tile's decals packed for the GPU: the records (2 vec4 each) and the cell grid
/// (32 x 32 cells of 4 u32: eight u16 local indices, 0xffff empty).
pub struct TileDecals {
    pub records: Vec<[f32; 4]>,
    pub cells: Vec<[u32; 4]>,
    /// The street-life decals' own cells (street_life.rs; `cc0::LIFE_GRID_VEC4`), the
    /// same layout; empty when the tile has none (as without `--street-life`).
    pub life_cells: Vec<[u32; 4]>,
    pub count: usize,
    pub dropped: usize,
}

fn hash(a: u32, b: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9e37_79b1) ^ b.wrapping_mul(0x85eb_ca77).rotate_left(13);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 / 16_777_216.0
}

/// The street dimensions the placement reads, from the California street
/// standards table (`caMetric`). Zero where the table cannot be read (the
/// rules that need it place nothing).
#[derive(Clone, Copy, Debug, Default)]
pub struct Standards {
    /// `parking-stall-desirable-size` length: the parking lanes' stall pitch.
    pub stall_length_m: f64,
    /// `curb-height-sf-standard`: the kerb-face grime's reach (cc0.wgsl).
    pub curb_height_m: f64,
    /// `line-width-local` (4-6 in), its middle: the width the worn-paint scan's own stripe is drawn at (cc0.wgsl).
    pub line_width_m: f64,
    /// `no-parking-crosswalk-daylighting` approach side (20 ft): the red kerb at a corner (weathering).
    pub daylighting_m: f64,
}

pub fn standards() -> Standards {
    static S: OnceLock<Standards> = OnceLock::new();
    *S.get_or_init(|| {
        let Some(t) = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::CaStandards::load(&p).ok()) else {
            return Standards::default();
        };
        Standards {
            stall_length_m: t.metric_key("parking-stall-desirable-size", "length").unwrap_or(0.0),
            curb_height_m: t.metric("curb-height-sf-standard").unwrap_or(0.0),
            line_width_m: match (t.metric_key("line-width-local", "min"), t.metric_key("line-width-local", "max")) {
                (Ok(a), Ok(b)) => (a + b) / 2.0,
                _ => 0.0,
            },
            daylighting_m: t.metric_key("no-parking-crosswalk-daylighting", "approachSide").unwrap_or(0.0),
        }
    })
}

/// Where people wait and walk most: the surveyed transit stops (the parking
/// export's `busStops`, the stop survey the JS game reads) and the shopfronts
/// eo-app hands over (`set_points`), on a 32 m grid; and the surveyed storm
/// drains (eo-app, `set_points`) by street tile.
pub struct Hotspots {
    cell: f64,
    stops: std::collections::HashMap<(i32, i32), Vec<[f64; 2]>>,
    shops: std::collections::HashMap<(i32, i32), Vec<[f64; 2]>>,
    drains: std::collections::HashMap<(i32, i32), Vec<[f64; 2]>>,
    pub stop_count: usize,
    pub shop_count: usize,
    pub drain_count: usize,
}

/// A stop draws gum within this reach of its point; a shopfront within this of its walk.
pub const STOP_REACH_M: f64 = 40.0;
pub const SHOP_REACH_M: f64 = 18.0;

static POINTS: OnceLock<(Vec<[f64; 2]>, Vec<[f64; 2]>)> = OnceLock::new();

/// eo-app, once, before any street tile cooks: the shopfronts (the ring
/// points of buildings whose record names a walk-in use) and the surveyed
/// storm-drain inlets, in world metres.
pub fn set_points(shops: Vec<[f64; 2]>, drains: Vec<[f64; 2]>) {
    let _ = POINTS.set((shops, drains));
}

impl Hotspots {
    fn grid(points: &[[f64; 2]], cell: f64) -> std::collections::HashMap<(i32, i32), Vec<[f64; 2]>> {
        let mut g: std::collections::HashMap<(i32, i32), Vec<[f64; 2]>> = std::collections::HashMap::new();
        for p in points {
            g.entry(((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32)).or_default().push(*p);
        }
        g
    }

    pub fn get() -> &'static Hotspots {
        static H: OnceLock<Hotspots> = OnceLock::new();
        H.get_or_init(|| {
            let stops: Vec<[f64; 2]> = eo_props::parking_rules::ParkingJs::load().map(|p| p.bus_stops).unwrap_or_default();
            let (shops, drains) = POINTS.get().cloned().unwrap_or_default();
            let cell = 32.0;
            Hotspots {
                cell,
                stops: Self::grid(&stops, cell),
                shops: Self::grid(&shops, cell),
                drains: Self::grid(&drains, eo_street::STREET_TILE_M),
                stop_count: stops.len(),
                shop_count: shops.len(),
                drain_count: drains.len(),
            }
        })
    }

    fn nearest(g: &std::collections::HashMap<(i32, i32), Vec<[f64; 2]>>, cell: f64, p: [f64; 2], reach: f64) -> f64 {
        let r = (reach / cell).ceil() as i32;
        let (cx, cz) = ((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32);
        let mut best = f64::INFINITY;
        for dz in -r..=r {
            for dx in -r..=r {
                if let Some(v) = g.get(&(cx + dx, cz + dz)) {
                    for q in v {
                        best = best.min(((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt());
                    }
                }
            }
        }
        best
    }

    /// The surveyed drains of a street tile and its eight neighbours.
    fn drains_near(&self, key: (i32, i32)) -> impl Iterator<Item = &[f64; 2]> {
        (-1..=1).flat_map(move |dz| (-1..=1).filter_map(move |dx| self.drains.get(&(key.0 + dx, key.1 + dz)))).flatten()
    }

    /// 0..1: how much a footway point draws people (1 at a stop or a shop door).
    pub fn draw(&self, p: [f64; 2]) -> f64 {
        let s = Self::nearest(&self.stops, self.cell, p, STOP_REACH_M);
        let h = Self::nearest(&self.shops, self.cell, p, SHOP_REACH_M);
        (1.0 - s / STOP_REACH_M).max(1.0 - h / SHOP_REACH_M).max(0.0)
    }

    /// The surveyed storm-drain inlets within `reach` of the segment a-b: (place
    /// along it, 0 at a .. 1 at b; lateral distance), nearest first. Stage W2c:
    /// the street-life piles' anchors (street_life.rs `place`).
    pub fn drains_along(&self, a: [f64; 2], b: [f64; 2], reach: f64) -> Vec<(f64, f64)> {
        points_along(&self.drains, eo_street::STREET_TILE_M, a, b, reach)
    }

    /// The surveyed transit stops within `reach` of the segment a-b, as `drains_along`.
    pub fn stops_along(&self, a: [f64; 2], b: [f64; 2], reach: f64) -> Vec<(f64, f64)> {
        points_along(&self.stops, self.cell, a, b, reach)
    }
}

/// The points of a grid (`cell` m) within `reach` of the segment a-b, between its
/// ends: (place along it 0..1, lateral distance), nearest first.
fn points_along(grid: &std::collections::HashMap<(i32, i32), Vec<[f64; 2]>>, cell: f64, a: [f64; 2], b: [f64; 2], reach: f64) -> Vec<(f64, f64)> {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = (dx * dx + dz * dz).max(1e-9);
    let (x0, x1) = (a[0].min(b[0]) - reach, a[0].max(b[0]) + reach);
    let (z0, z1) = (a[1].min(b[1]) - reach, a[1].max(b[1]) + reach);
    let mut out = vec![];
    for cz in (z0 / cell).floor() as i32..=(z1 / cell).floor() as i32 {
        for cx in (x0 / cell).floor() as i32..=(x1 / cell).floor() as i32 {
            for p in grid.get(&(cx, cz)).into_iter().flatten() {
                let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let d = ((p[0] - a[0] - dx * t).powi(2) + (p[1] - a[1] - dz * t).powi(2)).sqrt();
                if d <= reach {
                    out.push((t, d));
                }
            }
        }
    }
    out.sort_by(|x, y| x.1.total_cmp(&y.1).then(x.0.total_cmp(&y.0)));
    out
}

/// Travel lanes of a run: (centre u, width) between its lane paint inside the moving carriageway.
fn lanes(world: &StreetWorld, ei: usize) -> (Vec<(f64, f64)>, f64) {
    let e = &world.plan.entries[ei];
    let o = &world.o;
    let half = e.half;
    let lines = &world.lines[ei];
    let parking = lines.iter().filter(|l| l.role == "parking").map(|l| l.u.abs()).fold(f64::NAN, f64::min);
    let moving = if parking.is_finite() { parking } else { (half - o.gutter_width).max(0.5) };
    let mut b: Vec<f64> = lines
        .iter()
        .filter(|l| matches!(l.role, "centre" | "divider" | "edge" | "parking" | "centre-turn-boundary" | "median-edge" | "unpainted-centre" | "cycle-buffer-edge"))
        .map(|l| l.u)
        .filter(|u| u.abs() <= moving + 1e-6)
        .collect();
    b.push(-moving);
    b.push(moving);
    b.sort_by(|x, y| x.partial_cmp(y).unwrap());
    b.dedup_by(|x, y| (*x - *y).abs() < 0.3);
    let mut out = vec![];
    for w in b.windows(2) {
        let width = w[1] - w[0];
        if (2.5..=4.8).contains(&width) {
            out.push(((w[0] + w[1]) / 2.0, width));
        } else if width > 4.8 && lines.iter().all(|l| l.paint == eo_street::paint::Paint::None) {
            // An unpainted carriageway: the planned lane count across it.
            let n = (width / 3.2).round().max(1.0) as usize;
            for k in 0..n {
                out.push((w[0] + width * (k as f64 + 0.5) / n as f64, width / n as f64));
            }
        }
    }
    (out, moving)
}

fn push_strip(out: &mut Vec<Decal>, p0: [f64; 2], p1: [f64; 2], half_w: f32, kind: u32, seed: f32) {
    let (dx, dz) = (p1[0] - p0[0], p1[1] - p0[1]);
    let len = (dx * dx + dz * dz).sqrt();
    if len < 0.2 {
        return;
    }
    out.push(Decal {
        c: [((p0[0] + p1[0]) / 2.0) as f32, ((p0[1] + p1[1]) / 2.0) as f32],
        a: [(dx / len) as f32, (dz / len) as f32],
        half: [(len / 2.0) as f32, half_w],
        kind,
        seed,
    });
}

/// The point at arc length `s` and lateral `u` of a run's stations (between
/// stations linearly), or None outside them.
fn at_s(g: &eo_street::mesh::EntryGeom, s: f64, u: f64) -> Option<[f64; 2]> {
    let st = &g.stations;
    if st.len() < 2 || s < st[0].s || s > st[st.len() - 1].s {
        return None;
    }
    let i = st.partition_point(|f| f.s <= s).clamp(1, st.len() - 1);
    let (a, b) = (&st[i - 1], &st[i]);
    let t = if b.s > a.s { (s - a.s) / (b.s - a.s) } else { 0.0 };
    let (p, q) = (a.at(u), b.at(u));
    Some([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t])
}

/// Every decal of the runs with spans in `key` and its neighbours (with the
/// weathering decals when `set_weathering` turned them on, and the street
/// life's when `street_life::set_level` did).
pub fn place(world: &StreetWorld, key: (i32, i32)) -> Vec<Decal> {
    let mut out = place_with(world, key, weathering());
    let level = crate::street_life::level();
    if level > 0 {
        // street life (`--street-life`): appended after every other decal (none at level 0).
        crate::street_life::place(world, key, &crate::street_life::Context::global(level), &mut out);
    }
    out
}

/// `place`, with the weathering decals (`DK_KERB_PAINT`, `DK_WALK_EDGE`) or
/// without them (then exactly the decals as before; with them the others are
/// the same decals in the same order, the new ones among them).
pub fn place_with(world: &StreetWorld, key: (i32, i32), weather: bool) -> Vec<Decal> {
    let mut spans: Vec<(u32, u32)> = vec![];
    for dz in -1..=1 {
        for dx in -1..=1 {
            if let Some(r) = world.tiles.get(&(key.0 + dx, key.1 + dz)) {
                spans.extend_from_slice(&r.spans);
            }
        }
    }
    spans.sort_unstable();
    spans.dedup();
    let mut out = vec![];
    let o = &world.o;
    let std_ = standards();
    let hot = Hotspots::get();
    // Consecutive spans of one run form a stretch: wheel paths and drip lines merge along it.
    let mut k = 0;
    while k < spans.len() {
        let ei = spans[k].0;
        let mut j = k;
        while j + 1 < spans.len() && spans[j + 1].0 == ei && spans[j + 1].1 == spans[j].1 + 1 {
            j += 1;
        }
        let e = &world.plan.entries[ei as usize];
        let g = &world.geoms[ei as usize];
        let run = eo_street::hash32(&e.id);
        let (lns, moving) = lanes(world, ei as usize);
        let half = e.half;
        // Busier classes wear more (primary .. residential rank).
        let busy = match e.highway.as_str() {
            "primary" | "trunk" | "primary_link" => 1.0f32,
            "secondary" | "secondary_link" => 0.8,
            "tertiary" | "tertiary_link" => 0.6,
            _ => 0.4,
        };
        // Wheel paths and drip lines: one strip per straight stretch (heading change under 2 degrees, at most 36 m).
        let mut s0 = spans[k].1 as usize;
        let last = spans[j].1 as usize + 1;
        while s0 < last {
            let mut s1 = s0 + 1;
            let a0 = &g.stations[s0];
            while s1 < last {
                let b = &g.stations[s1];
                if a0.tx * b.tx + a0.tz * b.tz < 0.9994 || b.s - a0.s > 36.0 {
                    break;
                }
                s1 += 1;
            }
            let (a, b) = (&g.stations[s0], &g.stations[s1.min(g.stations.len() - 1)]);
            if g.keep.get(s0).copied().unwrap_or(true) {
                for (li, &(uc, _w)) in lns.iter().enumerate() {
                    let seed = hash(run, (s0 as u32) << 4 | li as u32);
                    for side in [-HALF_TRACK_M, HALF_TRACK_M] {
                        push_strip(&mut out, a.at(uc + side), b.at(uc + side), 0.34, DK_WHEEL, seed);
                    }
                    push_strip(&mut out, a.at(uc), b.at(uc), 0.28, DK_OIL_LINE, seed);
                }
            }
            s0 = s1;
        }
        // Weathering: the corner daylighting's red kerb paint (CVC 22500(n)) on the
        // vehicle approach side of each end with an intersection approach (right-hand
        // traffic: the -n kerb toward the run's end, the +n kerb toward its start; a
        // one-way run's both kerbs at its end), placed with the stretch that holds the end's span.
        if weather && std_.daylighting_m > 0.0 {
            if let (Some(f0), Some(f1)) = (g.stations.first(), g.stations.last()) {
                let last_span = g.stations.len().saturating_sub(2) as u32;
                for (at_end, toward) in [(true, e.approach_end.is_some()), (false, e.approach_start.is_some())] {
                    let span = if at_end { last_span } else { 0 };
                    if !toward || span < spans[k].1 || span > spans[j].1 {
                        continue;
                    }
                    let len = std_.daylighting_m.min((f1.s - f0.s) * 0.5);
                    if len < 1.0 {
                        continue;
                    }
                    let (end, dir) = if at_end { (f1.s, -1.0) } else { (f0.s, 1.0) };
                    for (wi, side) in [(0usize, 1.0f64), (1, -1.0)] {
                        let approach = if e.oneway { at_end } else if at_end { side < 0.0 } else { side > 0.0 };
                        if !approach || !(e.walks[wi] >= o.min_sidewalk_width) {
                            continue;
                        }
                        let u = side * (half + 0.05);
                        if let (Some(p), Some(q)) = (at_s(g, end, u), at_s(g, end + dir * len, u)) {
                            // On a bend the kerb line is longer than the centreline: never more than the standard's length.
                            let chord = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
                            let k = if chord > len { len / chord } else { 1.0 };
                            let q = [p[0] + (q[0] - p[0]) * k, p[1] + (q[1] - p[1]) * k];
                            push_strip(&mut out, p, q, KERB_PAINT_HALF_M as f32, DK_KERB_PAINT, hash(run ^ 0x6d2b, (at_end as u32) << 1 | wi as u32));
                        }
                    }
                }
            }
        }
        // Tyre tracks braking toward an intersection approach: in the wheel
        // paths of the lanes that travel toward it (right-hand traffic: the -n
        // lanes travel toward the run's end, the +n lanes toward its start; a
        // one-way run's every lane toward its end), a few metres short of it.
        if let (Some(f0), Some(f1)) = (g.stations.first(), g.stations.last()) {
            for (end, toward) in [(f1.s, e.approach_end.is_some()), (f0.s, e.approach_start.is_some())] {
                if !toward {
                    continue;
                }
                let at_end = end > f0.s;
                for (li, &(uc, _)) in lns.iter().enumerate() {
                    let travels = if e.oneway { at_end } else if at_end { uc < 0.0 } else { uc > 0.0 };
                    if !travels {
                        continue;
                    }
                    let salt = (at_end as u32) << 8 | li as u32;
                    if hash(run ^ 0x7ac1, salt) > 0.30 * (0.6 + busy) {
                        continue;
                    }
                    let short = 3.0 + hash(run ^ 0x7ac3, salt) as f64 * 4.0;
                    let len = 5.0 + hash(run ^ 0x7ac5, salt) as f64 * 9.0;
                    let dir = if at_end { -1.0 } else { 1.0 };
                    let (sa, sb) = (end + dir * short, end + dir * (short + len));
                    for (wi, side) in [-HALF_TRACK_M, HALF_TRACK_M].into_iter().enumerate() {
                        if let (Some(p), Some(q)) = (at_s(g, sa, uc + side), at_s(g, sb, uc + side)) {
                            push_strip(&mut out, p, q, (TYRE_WIDTH_M / 2.0) as f32, DK_TYRE, hash(run ^ 0x7ac7, salt << 1 | wi as u32));
                        }
                    }
                }
            }
        }
        // Per span: drips, covers, patches, cracks, footway wear.
        for s in spans[k].1..=spans[j].1 {
            let si = s as usize;
            if si + 1 >= g.stations.len() {
                continue;
            }
            let (a, b) = (&g.stations[si], &g.stations[si + 1]);
            let len = (b.s - a.s).max(0.0);
            if len < 0.5 {
                continue;
            }
            let at = |t: f64, u: f64| -> [f64; 2] {
                let p = a.at(u);
                let q = b.at(u);
                [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]
            };
            let dir = [(b.x - a.x) / len, (b.z - a.z) / len];
            let h = |salt: u32| hash(run ^ salt.wrapping_mul(0x632b_e5ab), s);
            let mut put = |c: [f64; 2], along: f64, across: f64, rot: f64, kind: u32, seed: f32| {
                let (cs, sn) = (rot.cos(), rot.sin());
                let ax = [dir[0] * cs - dir[1] * sn, dir[0] * sn + dir[1] * cs];
                out.push(Decal { c: [c[0] as f32, c[1] as f32], a: [ax[0] as f32, ax[1] as f32], half: [along as f32 / 2.0, across as f32 / 2.0], kind, seed });
            };
            // Parking lanes: per stall, oil drips under the engine and a short pull-in track.
            if moving + 1.6 < half - o.gutter_width && std_.stall_length_m > 1.0 {
                for side in [-1.0f64, 1.0] {
                    let um = side * (moving + (half - o.gutter_width - moving) * 0.5);
                    let n = (len / std_.stall_length_m).floor().max(1.0) as u32;
                    for q in 0..n {
                        let sd = (side > 0.0) as u32;
                        if h(11 + q * 7 + sd) < 0.45 {
                            let t = (q as f64 + 0.5) / n as f64 + (h(19 + q) as f64 - 0.5) * 0.2;
                            put(at(t, um + (h(23 + q) as f64 - 0.5) * 0.8), 0.7 + h(29 + q) as f64 * 0.9, 0.5 + h(31 + q) as f64 * 0.5, h(37 + q) as f64 * 6.28, DK_OIL_SPOT, h(41 + q));
                        }
                        if h(43 + q * 5 + sd) < 0.22 {
                            // Pulled in at a slight angle from the travel lane.
                            let t = (q as f64 + 0.15 + h(47 + q) as f64 * 0.3) / n as f64;
                            let ang = side * (0.08 + h(53 + q) as f64 * 0.12);
                            put(at(t, um - side * 0.4), 1.5 + h(59 + q) as f64 * 1.8, TYRE_WIDTH_M, ang, DK_TYRE, h(61 + q));
                        }
                    }
                }
            }
            // Sewer manholes on a lane centre: one chance per stretch of run.
            let cell = (a.s / MANHOLE_STRETCH_M).floor() as u32;
            let pos = cell as f64 * MANHOLE_STRETCH_M + 8.0 + hash(run, cell ^ 0x5bd1) as f64 * (MANHOLE_STRETCH_M - 16.0);
            if pos >= a.s && pos < b.s && hash(run, cell ^ 0x1b87) < 0.55 && !lns.is_empty() {
                let li = (hash(run, cell ^ 0x2c3d) * lns.len() as f32) as usize % lns.len();
                put(at((pos - a.s) / len, lns[li].0), 0.8, 0.8, hash(run, cell ^ 0x77) as f64 * 6.28, DK_MANHOLE, hash(run, cell ^ 0x99));
            }
            // Utility vault covers beside the kerb.
            if h(63) < 0.06 {
                let side = if h(65) < 0.5 { -1.0 } else { 1.0 };
                put(at(h(67) as f64, side * (half - o.gutter_width - 0.9)), 0.62, 0.62, 0.0, DK_VAULT, h(69));
            }
            // Utility cuts: longitudinal patches and transverse trenches.
            if h(71) < 0.07 * (0.6 + busy) {
                let u = (h(73) as f64 - 0.5) * 2.0 * (moving - 1.0).max(0.5);
                put(at(h(79) as f64, u), 1.4 + h(83) as f64 * 3.2, 0.8 + h(89) as f64 * 1.2, 0.0, DK_PATCH, h(97));
            }
            if h(101) < 0.035 * (0.6 + busy) {
                let u0 = (h(103) as f64 - 0.5) * moving;
                put(at(h(107) as f64, u0), 0.6 + h(109) as f64 * 0.5, (moving * (0.6 + h(113) as f64 * 0.8)).min(2.0 * moving), 0.0, DK_PATCH, h(127));
            }
            // Cracked areas: near the kerbs and in the outer lanes, a few in the middle.
            for q in 0..2u32 {
                if h(131 + q * 17) < 0.22 * (0.5 + busy) {
                    let edge = h(137 + q) < 0.7;
                    let side = if h(139 + q) < 0.5 { -1.0 } else { 1.0 };
                    let u = if edge { side * (moving - 0.6 - h(149 + q) as f64 * 1.2).max(0.0) } else { (h(151 + q) as f64 - 0.5) * moving };
                    let size = 2.0 + h(157 + q) as f64 * 2.4;
                    put(at(h(163 + q) as f64, u), size, size, h(167 + q) as f64 * 6.28, DK_CRACK, h(173 + q));
                }
            }
            // Footways: gum where people wait and walk most, stains, cracked slabs.
            for (wi, side) in [(0usize, 1.0f64), (1, -1.0)] {
                let walk = e.walks[wi];
                if !(walk >= o.min_sidewalk_width) {
                    continue;
                }
                if weather {
                    // Weathering: dirt along the building line, one strip per span on the walk's back
                    // edge (keyed on the span alone, so every tile that sees it places the same strip).
                    put(at(0.5, side * (half + walk)), len, WALK_EDGE_HALF_M * 2.0, 0.0, DK_WALK_EDGE, hash(run ^ 0x3e11, s << 1 | wi as u32));
                }
                let mid = side * (half + walk * 0.5);
                // How much the walk here draws people: a stop or a shop door, else the class alone (a little).
                let draw = hot.draw(at(0.5, mid)) as f32;
                let gum = (draw * (0.55 + 0.45 * busy)).max(0.12 * busy);
                if h(181 + wi as u32) < gum {
                    // The seed carries the field's density (whole part, eighths) and its hash.
                    let dens = (gum * 8.0).round().clamp(1.0, 8.0);
                    put(at(0.5, mid), len, walk * 0.9, 0.0, DK_GUM, dens + h(191 + wi as u32) * 0.999);
                }
                if h(193 + wi as u32) < 0.35 {
                    let u = side * (half + walk * (0.2 + 0.6 * h(197 + wi as u32) as f64));
                    put(at(h(199 + wi as u32) as f64, u), 0.5 + h(211 + wi as u32) as f64 * 1.0, 0.4 + h(223 + wi as u32) as f64 * 0.8, h(227 + wi as u32) as f64 * 6.28, DK_STAIN, h(229 + wi as u32));
                }
                if h(233 + wi as u32) < 0.14 {
                    let u = side * (half + walk * (0.25 + 0.5 * h(239 + wi as u32) as f64));
                    let size = 1.2 + h(241 + wi as u32) as f64 * 1.2;
                    put(at(h(251 + wi as u32) as f64, u), size, size.min(walk), h(257 + wi as u32) as f64 * 6.28, DK_WALK_CRACK, h(263 + wi as u32));
                }
            }
        }
        k = j + 1;
    }
    // Storm-drain inlets at the surveyed points: snapped into the gutter of the
    // nearest kerbed run (its kerb line within 2.5 m of the point), along its heading.
    for p in hot.drains_near(key) {
        let mut best: Option<(f64, [f64; 2], [f64; 2], u32)> = None;
        for &(ei, s) in &spans {
            let e = &world.plan.entries[ei as usize];
            let g = &world.geoms[ei as usize];
            let si = s as usize;
            if si + 1 >= g.stations.len() {
                continue;
            }
            let (a, b) = (&g.stations[si], &g.stations[si + 1]);
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            let l2 = dx * dx + dz * dz;
            if l2 < 1e-6 {
                continue;
            }
            let l = l2.sqrt();
            let t = ((p[0] - a.x) * dx + (p[1] - a.z) * dz) / l2;
            let beyond = if t < 0.0 { -t * l } else if t > 1.0 { (t - 1.0) * l } else { 0.0 };
            let tc = t.clamp(0.0, 1.0);
            let q = [a.x + dx * tc, a.z + dz * tc];
            let u = (p[0] - q[0]) * a.nx + (p[1] - q[1]) * a.nz;
            let wi = if u >= 0.0 { 0 } else { 1 };
            if !(e.walks[wi] >= o.min_sidewalk_width) {
                continue;
            }
            let miss = (u.abs() - e.half).abs() + beyond;
            if miss < 2.5 && best.is_none_or(|bb| miss < bb.0) {
                let uu = u.signum() * (e.half - o.gutter_width * 0.5);
                best = Some((miss, [q[0] + a.nx * uu, q[1] + a.nz * uu], [dx / l, dz / l], eo_street::hash32(&e.id)));
            }
        }
        if let Some((_, c, ax, run)) = best {
            let salt = ((p[0] * 10.0).round() as i64 as u32) ^ ((p[1] * 10.0).round() as i64 as u32).rotate_left(16);
            out.push(Decal { c: [c[0] as f32, c[1] as f32], a: [ax[0] as f32, ax[1] as f32], half: [0.45, 0.45], kind: DK_GRATE, seed: hash(run, salt) });
        }
    }
    out
}

/// Pack the decals that touch `key`'s cells (highest priority first in each cell).
pub fn pack(key: (i32, i32), tile_m: f32, mut decals: Vec<Decal>) -> TileDecals {
    let x0 = key.0 as f32 * tile_m;
    let z0 = key.1 as f32 * tile_m;
    let cell = tile_m / CELLS as f32;
    decals.retain(|d| {
        let (mn, mx) = d.bounds();
        mx[0] >= x0 && mn[0] <= x0 + tile_m && mx[1] >= z0 && mn[1] <= z0 + tile_m
    });
    decals.sort_by_key(|d| d.priority());
    let mut dropped = decals.len().saturating_sub(MAX_DECALS);
    decals.truncate(MAX_DECALS);
    let mut cells = vec![[0xffff_ffffu32; 4]; CELLS * CELLS];
    let mut fill = vec![0usize; CELLS * CELLS];
    // street life (`--street-life`): its decals sort last and take cells of their own.
    let any_life = decals.iter().any(|d| d.priority() >= 8);
    let mut life_cells = if any_life { vec![[0xffff_ffffu32; 4]; CELLS * CELLS] } else { vec![] };
    let mut life_fill = vec![0usize; if any_life { CELLS * CELLS } else { 0 }];
    let mut records = Vec::with_capacity(decals.len() * 2);
    for (i, d) in decals.iter().enumerate() {
        records.push([d.c[0], d.c[1], d.a[0], d.a[1]]);
        records.push([d.half[0], d.half[1], d.kind as f32, d.seed]);
        let life = d.priority() >= 8;
        let (cells, fill) = if life { (&mut life_cells, &mut life_fill) } else { (&mut cells, &mut fill) };
        let (mn, mx) = d.bounds();
        let cx0 = (((mn[0] - x0) / cell).floor().max(0.0) as usize).min(CELLS - 1);
        let cx1 = (((mx[0] - x0) / cell).floor().max(0.0) as usize).min(CELLS - 1);
        let cz0 = (((mn[1] - z0) / cell).floor().max(0.0) as usize).min(CELLS - 1);
        let cz1 = (((mx[1] - z0) / cell).floor().max(0.0) as usize).min(CELLS - 1);
        for cz in cz0..=cz1 {
            for cx in cx0..=cx1 {
                // The cell square against the decal's oriented rectangle (separating axes).
                let (cxm, czm) = (x0 + (cx as f32 + 0.5) * cell, z0 + (cz as f32 + 0.5) * cell);
                let q = [cxm - d.c[0], czm - d.c[1]];
                let n = [-d.a[1], d.a[0]];
                let r = cell * 0.5;
                let along = (q[0] * d.a[0] + q[1] * d.a[1]).abs();
                let across = (q[0] * n[0] + q[1] * n[1]).abs();
                if along > d.half[0] + r * (d.a[0].abs() + d.a[1].abs()) || across > d.half[1] + r * (n[0].abs() + n[1].abs()) {
                    continue;
                }
                let c = cz * CELLS + cx;
                if fill[c] >= CELL_SLOTS {
                    dropped += 1;
                    continue;
                }
                let w = fill[c] / 2;
                let shift = (fill[c] % 2) * 16;
                cells[c][w] = (cells[c][w] & !(0xffff << shift)) | ((i as u32) << shift);
                fill[c] += 1;
            }
        }
    }
    TileDecals { count: decals.len(), records, cells, life_cells, dropped }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_fills_cells_and_orders_by_priority() {
        let d = vec![
            Decal { c: [10.0, 10.0], a: [1.0, 0.0], half: [20.0, 0.3], kind: DK_WHEEL, seed: 0.1 },
            Decal { c: [6.0, 6.0], a: [1.0, 0.0], half: [0.4, 0.4], kind: DK_MANHOLE, seed: 0.2 },
            Decal { c: [500.0, 500.0], a: [1.0, 0.0], half: [1.0, 1.0], kind: DK_CRACK, seed: 0.3 },
        ];
        let t = pack((0, 0), 128.0, d);
        assert_eq!(t.count, 2, "the far decal is not in the tile");
        assert_eq!(t.records.len(), 4);
        // The manhole sorts first (index 0) and owns the cell at (6, 6).
        let c = &t.cells[1 * CELLS + 1];
        assert_eq!(c[0] & 0xffff, 0);
        // The wheel strip crosses cells 0..7 of row 2 (z = 10 m), not row 5.
        assert_ne!(t.cells[2 * CELLS + 7][0], 0xffff_ffff);
        assert_eq!(t.cells[5 * CELLS + 3][0], 0xffff_ffff);
    }

    #[test]
    fn placement_reads_the_standards_table() {
        // caMetric('parking-stall-desirable-size').length: 24 ft; caMetric('curb-height-sf-standard'): 6 in.
        let s = standards();
        assert!((s.stall_length_m - 7.315).abs() < 1e-3, "{s:?}");
        assert!((s.curb_height_m - 0.152).abs() < 1e-3, "{s:?}");
        // caMetric('line-width-local'): 4-6 in, the middle.
        assert!((s.line_width_m - 0.127).abs() < 1e-3, "{s:?}");
    }

    #[test]
    fn hotspots_draw_people_near_stops() {
        let h = Hotspots::get();
        assert!(h.stop_count > 100, "the stop survey has {} points", h.stop_count);
        // At a stop the draw is 1; far from every stop and shop it is 0.
        let p = eo_props::parking_rules::ParkingJs::load().unwrap().bus_stops[0];
        assert!((h.draw(p) - 1.0).abs() < 1e-9);
        assert_eq!(h.draw([1.0e7, 1.0e7]), 0.0);
    }
}

#[cfg(test)]
mod life_pack_tests {
    use super::*;

    /// The street life's decals go to their own cells, never to the wear decals'.
    #[test]
    fn life_decals_take_their_own_cells() {
        let col = crate::street_life::kind_with(crate::street_life::DK_COLUMN, crate::street_life::Params { tags: 6, deck: true, wall: true, ..Default::default() }.pack());
        let d = vec![
            Decal { c: [10.0, 10.0], a: [1.0, 0.0], half: [20.0, 0.3], kind: DK_WHEEL, seed: 0.1 },
            Decal { c: [6.0, 6.0], a: [1.0, 0.0], half: [1.2, 1.2], kind: col, seed: 5.0 },
        ];
        let t = pack((0, 0), 128.0, d);
        assert_eq!(t.count, 2);
        // the column is record 1; listed in its life cell (1, 1), not in the wear cell there
        let c = 1 * CELLS + 1;
        assert_eq!(t.life_cells[c][0] & 0xffff, 1, "{:x?}", t.life_cells[c]);
        assert_eq!(t.cells[c][0], 0xffff_ffff);
        assert_eq!(t.records[3][2] as u32, col);
    }
}
