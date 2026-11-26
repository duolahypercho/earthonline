//! `decks`: structures the bare-earth DEM keeps as ground (stage H4), and
//! the 8 m grid's cells over them brought down to the streets' grade, into
//! `native/assets/sf-decks-v1.json` (eo-data `decks`).
//!
//! A deck is a roof the USGS bare-earth model kept as ground: Moscone Center
//! South's roof (Yerba Buena Gardens' children's garden, the carousel, the
//! ice rink) stands 5-9 m over 4th Street and Folsom Street in the DEM, and
//! the 8 m grid draws it as an earth mound with 25 m banks into the streets.
//! The decks are listed by hand ([`DECKS`], a seed point and why); `--survey`
//! lists the raised regions citywide that could be more (most are hills,
//! reservoirs and graded pads, which are real ground).
//!
//! Per deck: the street grade on the 8 m grid (the inverse-distance mean of
//! the DEM along the carriageways at grade within [`GRADE_RADIUS_M`]); the
//! deck, the 2 m points raised [`SEED_RAISED_M`] or more over that grade
//! connected to the seed; the 8 m cells within one cell of those points; and
//! their new heights, the harmonic fill (Laplace) of the cells round them, so
//! the block meets its streets at their own grade with no bank and no step.
//!
//! `--probe x0,z0,x1,z1[,step]` prints the 1 m DEM over a box of the
//! surveyed plan (metres above the box's lowest point, every `step` m,
//! default 4), the 8 m grid's cells there, and which points lie in a
//! footprint or within 6 m of a road centreline.

use anyhow::{Context, Result, bail};
use std::time::Instant;

pub fn run(args: &[String]) -> Result<()> {
    let t0 = Instant::now();
    let mut probe: Option<Vec<f64>> = None;
    let mut dump: Option<std::path::PathBuf> = None;
    let mut survey = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--survey" => survey = true,
            "--dump" => dump = Some(std::path::PathBuf::from(it.next().context("--dump needs a folder")?)),
            "--probe" => probe = Some(it.next().context("--probe needs x0,z0,x1,z1[,step]")?.split(',').map(|s| s.parse::<f64>()).collect::<Result<Vec<_>, _>>()?),
            _ => bail!("usage: cook decks [--survey] [--probe x0,z0,x1,z1[,step] [--dump <dir>]]   (EO_DEM1M_DIR: the 1 m DEM tiles)"),
        }
    }
    let (dem, inputs, paths, city) = super::relief::load_dem(t0)?;
    let elev = eo_data::ElevationGrid::decode(&eo_data::read_maybe_gzip(&paths.sf("sf-elevation.bin.gz"))?)?;
    let Some(p) = probe else { return detect(&dem, &inputs.dem_provenance_sha256, &paths, &city, &elev, t0, survey, dump.as_deref()) };
    if p.len() < 4 {
        bail!("--probe needs x0,z0,x1,z1[,step]");
    }
    let (x0, z0, x1, z1, step) = (p[0], p[1], p[2], p[3], p.get(4).copied().unwrap_or(4.0));
    if let Some(d) = &dump {
        // the box at 1 m (f32 LE, NaN where none, rows of z), its footprints and roads, for a look elsewhere
        std::fs::create_dir_all(d)?;
        let (w, h) = ((x1 - x0) as usize + 1, (z1 - z0) as usize + 1);
        let mut raw = Vec::with_capacity(w * h * 4);
        for j in 0..h {
            for i in 0..w {
                raw.extend_from_slice(&dem.at_plan(x0 + i as f64, z0 + j as f64).unwrap_or(f32::NAN).to_le_bytes());
            }
        }
        std::fs::write(d.join("dem.f32"), raw)?;
        let inb = |q: [f64; 2]| q[0] > x0 - 50.0 && q[0] < x1 + 50.0 && q[1] > z0 - 50.0 && q[1] < z1 + 50.0;
        let fps: Vec<serde_json::Value> = city.buildings.iter().filter(|b| b.ring.iter().any(|q| inb([q[0] as f64, q[1] as f64]))).map(|b| serde_json::json!({"id": b.id, "height": b.height, "ring": b.ring})).collect();
        let rds: Vec<serde_json::Value> = city.roads.iter().filter(|r| r.points64.iter().any(|q| inb(*q))).map(|r| serde_json::json!({"name": r.name, "highway": r.highway, "bridge": r.bridge, "tunnel": r.tunnel, "layer": r.layer, "points": r.points64})).collect();
        let m = &elev.meta;
        let mut cells = vec![];
        let (ci0, ci1) = (((x0 - m.origin_x) / m.cell_size).floor() as i64, ((x1 - m.origin_x) / m.cell_size).ceil() as i64);
        let (cj0, cj1) = (((z0 - m.origin_z) / m.cell_size).floor() as i64, ((z1 - m.origin_z) / m.cell_size).ceil() as i64);
        for cj in cj0..=cj1 {
            for ci in ci0..=ci1 {
                cells.push(serde_json::json!([m.origin_x + (ci as f64 + 0.5) * m.cell_size, m.origin_z + (cj as f64 + 0.5) * m.cell_size, elev.lidar[(cj as usize) * m.width + ci as usize] as f64 - eo_data::NAVD88_TO_GAME_Y]));
            }
        }
        std::fs::write(d.join("box.json"), serde_json::to_vec(&serde_json::json!({"x0": x0, "z0": z0, "w": w, "h": h, "footprints": fps, "roads": rds, "cells_navd88": cells}))?)?;
        eprintln!("dumped {} x {} m to {}", w, h, d.display());
        return Ok(());
    }
    let rings: Vec<Vec<[f64; 2]>> = city
        .buildings
        .iter()
        .filter(|b| b.ring.iter().any(|q| (q[0] as f64) > x0 - 200.0 && (q[0] as f64) < x1 + 200.0 && (q[1] as f64) > z0 - 200.0 && (q[1] as f64) < z1 + 200.0))
        .map(|b| b.ring.iter().map(|q| [q[0] as f64, q[1] as f64]).collect())
        .collect();
    let in_fp = |x: f64, z: f64| rings.iter().any(|r| point_in_ring(x, z, r));
    let seg_d = |a: [f64; 2], b: [f64; 2], x: f64, z: f64| {
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        let t = if l2 > 1e-9 { (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
        (x - a[0] - t * dx).hypot(z - a[1] - t * dz)
    };
    let roads: Vec<&eo_data::Road> = city.roads.iter().filter(|r| r.points64.iter().any(|q| q[0] > x0 - 100.0 && q[0] < x1 + 100.0 && q[1] > z0 - 100.0 && q[1] < z1 + 100.0)).collect();
    let near_road = |x: f64, z: f64| roads.iter().any(|r| !r.bridge && !r.tunnel && r.points64.windows(2).any(|w| seg_d(w[0], w[1], x, z) < 6.0));
    let mut lo = f32::INFINITY;
    let (nx, nz) = (((x1 - x0) / step) as usize + 1, ((z1 - z0) / step) as usize + 1);
    let mut h = vec![None; nx * nz];
    for j in 0..nz {
        for i in 0..nx {
            let v = dem.at_plan(x0 + i as f64 * step, z0 + j as f64 * step);
            if let Some(v) = v {
                lo = lo.min(v);
            }
            h[j * nx + i] = v;
        }
    }
    println!("1 m DEM over ({x0}, {z0})-({x1}, {z1}) every {step} m, metres above {lo:.1} (NAVD88); '#' footprint, '=' road, digits = whole metres (a=10 ...)");
    for j in 0..nz {
        let mut line = String::new();
        for i in 0..nx {
            let (x, z) = (x0 + i as f64 * step, z0 + j as f64 * step);
            let c = match h[j * nx + i] {
                None => '?',
                Some(v) => {
                    let d = ((v - lo).max(0.0)) as u32;
                    std::char::from_digit(d.min(35), 36).unwrap_or('+')
                }
            };
            let mark = if near_road(x, z) { '=' } else if in_fp(x, z) { '#' } else { ' ' };
            line.push(c);
            line.push(mark);
        }
        println!("{:7.0} {line}", z0 + j as f64 * step);
    }
    let m = &elev.meta;
    println!("8 m grid cells (surveyed game y, NAVD88 + {:.3}):", eo_data::NAVD88_TO_GAME_Y);
    let (ci0, ci1) = (((x0 - m.origin_x) / m.cell_size).floor() as i64, ((x1 - m.origin_x) / m.cell_size).ceil() as i64);
    let (cj0, cj1) = (((z0 - m.origin_z) / m.cell_size).floor() as i64, ((z1 - m.origin_z) / m.cell_size).ceil() as i64);
    for cj in (cj0..cj1).step_by(2) {
        let mut line = String::new();
        for ci in (ci0..ci1).step_by(2) {
            let v = elev.lidar[(cj as usize) * m.width + ci as usize];
            line.push_str(&format!("{:3.0}", v - lo + eo_data::NAVD88_TO_GAME_Y as f32 * -1.0));
        }
        println!("{:7.0} {line}", m.origin_z + (cj as f64 + 0.5) * m.cell_size);
    }
    Ok(())
}

/// Even-odd point in a ring (plan metres).
pub(super) fn point_in_ring(x: f64, z: f64, ring: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let n = ring.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The roads whose centreline is the street's grade: carriageways at grade
/// (no bridge, tunnel, layer or ramp; no service way, footway or path, which
/// may run over a deck).
const GRADE_HIGHWAYS: [&str; 8] = ["trunk", "primary", "secondary", "tertiary", "residential", "unclassified", "living_street", "road"];
/// Street grade samples every this many metres along a centreline.
const SAMPLE_STEP_M: f64 = 2.0;
/// The street grade at a point: the inverse-distance mean of the samples within this radius.
const GRADE_RADIUS_M: f64 = 150.0;
/// At least this many samples within the radius, else no grade (far from streets: no deck).
const GRADE_MIN_SAMPLES: usize = 12;
/// (`--survey`) A 2 m point is raised this far above the street grade or more.
pub const RAISED_M: f64 = 3.0;
/// (`--survey`) A raised region this large (m^2) or larger is a candidate.
pub const MIN_AREA_M2: f64 = 1500.0;
/// (`--survey`) A deck's top is flat: this share of its points slopes under `FLAT_SLOPE`.
pub const FLAT_SHARE: f64 = 0.6;
pub const FLAT_SLOPE: f64 = 0.15;
/// (`--survey`) A deck stands on walls: this share of its edge drops `WALL_DROP_M` within `WALL_RUN_M` outward.
pub const WALL_SHARE: f64 = 0.35;
pub const WALL_DROP_M: f64 = 2.0;
pub const WALL_RUN_M: f64 = 4.0;

/// (`--survey`) A raised region and its measures.
struct Region {
    area: f64,
    mean_up: f64,
    p90_up: f64,
    flat: f64,
    wall: f64,
    built: f64,
    cx: f64,
    cz: f64,
}

/// The decks: a name, a point on the deck (lat, lon) and why it is one.
pub const DECKS: [(&str, f64, f64, &str); 1] = [(
    "moscone-south",
    37.78325,
    -122.40121,
    "Moscone Center South's roof deck (Yerba Buena Gardens: the children's garden, the carousel, the ice rink and bowling centre, the Children's Creativity Museum), 4th, Howard, 3rd and Folsom Streets: the DEM's ground stands 5-9 m over the streets on its walls",
)];
/// The deck: the 2 m points this far over the street grade or more, connected to its seed.
pub const SEED_RAISED_M: f64 = 1.5;
/// A deck larger than this (m^2) has run out of its block: refused.
pub const MAX_DECK_M2: f64 = 60_000.0;

#[allow(clippy::too_many_arguments)]
fn detect(dem: &super::relief::Dem, dem_sha: &str, paths: &eo_data::DataPaths, city: &eo_data::City, elev: &eo_data::ElevationGrid, t0: Instant, survey: bool, dump: Option<&std::path::Path>) -> Result<()> {
    let m = &elev.meta;
    let frame = eo_data::roof_colours::PlanFrame::from_meta(&city.meta);
    // 1. Street grade samples, bucketed.
    let bucket = 30.0;
    let key = |x: f64, z: f64| (((x - m.origin_x) / bucket).floor() as i32, ((z - m.origin_z) / bucket).floor() as i32);
    let mut buckets: std::collections::HashMap<(i32, i32), Vec<[f32; 3]>> = std::collections::HashMap::new();
    let mut n_samples = 0usize;
    for r in &city.roads {
        if r.bridge || r.tunnel || r.layer != 0 || !GRADE_HIGHWAYS.contains(&r.highway.as_str()) {
            continue;
        }
        for w in r.points64.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            let n = (len / SAMPLE_STEP_M).ceil().max(1.0) as usize;
            for k in 0..n {
                let t = k as f64 / n as f64;
                let (x, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                if let Some(h) = dem.at_plan(x, z) {
                    buckets.entry(key(x, z)).or_default().push([x as f32, z as f32, h]);
                    n_samples += 1;
                }
            }
        }
    }
    eprintln!("{n_samples} street grade samples in {} buckets ({:.1} s)", buckets.len(), t0.elapsed().as_secs_f64());
    // 2. The grade at every 8 m cell centre.
    let (w8, h8) = (m.width, m.height);
    let reach = (GRADE_RADIUS_M / bucket).ceil() as i32;
    let mut grade = vec![f32::NAN; w8 * h8];
    eo_hills::par_rows(&mut grade, w8, |row0, rows| {
        for (k, g) in rows.iter_mut().enumerate() {
            let (ci, cj) = ((row0 * w8 + k) % w8, (row0 * w8 + k) / w8);
            let (x, z) = (m.origin_x + (ci as f64 + 0.5) * m.cell_size, m.origin_z + (cj as f64 + 0.5) * m.cell_size);
            let (bi, bj) = key(x, z);
            let (mut sw, mut sh, mut n) = (0.0f64, 0.0f64, 0usize);
            for dj in -reach..=reach {
                for di in -reach..=reach {
                    let Some(v) = buckets.get(&(bi + di, bj + dj)) else { continue };
                    for s in v {
                        let d2 = (s[0] as f64 - x).powi(2) + (s[1] as f64 - z).powi(2);
                        if d2 > GRADE_RADIUS_M * GRADE_RADIUS_M {
                            continue;
                        }
                        let wgt = 1.0 / (d2 + 16.0);
                        sw += wgt;
                        sh += wgt * s[2] as f64;
                        n += 1;
                    }
                }
            }
            if n >= GRADE_MIN_SAMPLES {
                *g = (sh / sw) as f32;
            }
        }
    });
    eprintln!("street grade on the 8 m grid ({:.1} s)", t0.elapsed().as_secs_f64());
    let grade_at = |x: f64, z: f64| -> Option<f64> {
        let fx = (x - m.origin_x) / m.cell_size - 0.5;
        let fz = (z - m.origin_z) / m.cell_size - 0.5;
        let (i0, j0) = (fx.floor() as i64, fz.floor() as i64);
        if i0 < 0 || j0 < 0 || i0 + 1 >= w8 as i64 || j0 + 1 >= h8 as i64 {
            return None;
        }
        let (u, v) = (fx - i0 as f64, fz - j0 as f64);
        let g = |i: i64, j: i64| grade[j as usize * w8 + i as usize] as f64;
        let c = [g(i0, j0), g(i0 + 1, j0), g(i0, j0 + 1), g(i0 + 1, j0 + 1)];
        if c.iter().any(|v| v.is_nan()) {
            return None;
        }
        Some((c[0] * (1.0 - u) + c[1] * u) * (1.0 - v) + (c[2] * (1.0 - u) + c[3] * u) * v)
    };
    // 3. The 2 m points raised over it.
    let (w2, h2) = (w8 * 4, h8 * 4);
    let px = |i: usize| m.origin_x + 1.0 + 2.0 * i as f64;
    let pz = |j: usize| m.origin_z + 1.0 + 2.0 * j as f64;
    let mut up = vec![f32::NAN; w2 * h2];
    eo_hills::par_rows(&mut up, w2, |row0, rows| {
        for (k, u) in rows.iter_mut().enumerate() {
            let (i, j) = ((row0 * w2 + k) % w2, (row0 * w2 + k) / w2);
            let (x, z) = (px(i), pz(j));
            let Some(g) = grade_at(x, z) else { continue };
            let Some(h) = dem.at_plan(x, z) else { continue };
            if h as f64 - g >= SEED_RAISED_M.min(RAISED_M) {
                *u = (h as f64 - g) as f32;
            }
        }
    });
    eprintln!("points {SEED_RAISED_M} m or more over the grade: {} ({:.1} s)", up.iter().filter(|v| !v.is_nan()).count(), t0.elapsed().as_secs_f64());
    let to_ij = |x: f64, z: f64| -> Option<usize> {
        let (i, j) = (((x - m.origin_x - 1.0) / 2.0).round() as i64, ((z - m.origin_z - 1.0) / 2.0).round() as i64);
        (i >= 0 && j >= 0 && (i as usize) < w2 && (j as usize) < h2).then(|| j as usize * w2 + i as usize)
    };
    // 3b. The listed decks: their regions, their cells, the fill.
    let mut out_decks = vec![];
    let mut taken = std::collections::HashSet::new();
    for (name, lat, lon, why) in DECKS {
        let (sx, sz) = frame.to_plan(lon, lat);
        let seed = to_ij(sx, sz).with_context(|| format!("deck {name}: its seed ({sx:.0}, {sz:.0}) is off the grid"))?;
        if up[seed].is_nan() {
            bail!("deck {name}: its seed ({sx:.0}, {sz:.0}) is not {SEED_RAISED_M} m over the street grade (DEM {:?}, grade {:?})", dem.at_plan(sx, sz), grade_at(sx, sz));
        }
        let mut seen = vec![false; w2 * h2];
        let (mut stack, mut pts) = (vec![seed], vec![]);
        seen[seed] = true;
        while let Some(p) = stack.pop() {
            pts.push(p);
            if pts.len() as f64 * 4.0 > MAX_DECK_M2 {
                bail!("deck {name}: over {MAX_DECK_M2} m^2, it has run out of its block");
            }
            let (i, j) = (p % w2, p / w2);
            for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (ni, nj) = (i as i64 + di, j as i64 + dj);
                if ni < 0 || nj < 0 || ni >= w2 as i64 || nj >= h2 as i64 {
                    continue;
                }
                let q = nj as usize * w2 + ni as usize;
                if !seen[q] && !up[q].is_nan() {
                    seen[q] = true;
                    stack.push(q);
                }
            }
        }
        // its cells: every 8 m cell within one cell of a deck point
        let mut cells = std::collections::BTreeSet::new();
        for &p in &pts {
            let (ci, cj) = ((p % w2) / 4, (p / w2) / 4);
            for dj in -1i64..=1 {
                for di in -1i64..=1 {
                    let (a, b) = (ci as i64 + di, cj as i64 + dj);
                    if a > 0 && b > 0 && (a as usize) < w8 - 1 && (b as usize) < h8 - 1 {
                        cells.insert((b as usize, a as usize));
                    }
                }
            }
        }
        if cells.iter().any(|c| taken.contains(c)) {
            bail!("deck {name}: shares cells with a deck before it");
        }
        taken.extend(cells.iter().copied());
        // the harmonic fill: every cell the mean of its four neighbours, the cells round the deck held
        let old = |c: (usize, usize)| elev.lidar[c.0 * w8 + c.1] as f64;
        let mut h: std::collections::BTreeMap<(usize, usize), f64> = cells.iter().map(|&c| (c, old(c))).collect();
        if cells.iter().any(|&c| old(c).is_nan()) {
            bail!("deck {name}: a cell with no height in the grid");
        }
        // a cell the fill would raise (a loading dock or ramp cut below the street beside the deck)
        // keeps its height and holds the fill like the cells round the deck; filled again until none would rise
        let mut held: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
        let mut sweeps = 0;
        loop {
            loop {
                let mut moved = 0f64;
                for &c in &cells {
                    if held.contains(&c) {
                        continue;
                    }
                    let nb = [(c.0 - 1, c.1), (c.0 + 1, c.1), (c.0, c.1 - 1), (c.0, c.1 + 1)];
                    let mean = nb.iter().map(|n| h.get(n).copied().unwrap_or_else(|| old(*n))).sum::<f64>() / 4.0;
                    let v = h.get_mut(&c).unwrap();
                    let next = *v + 1.8 * (mean - *v);
                    moved = moved.max((next - *v).abs());
                    *v = next;
                }
                sweeps += 1;
                if moved < 1e-4 || sweeps > 50_000 {
                    break;
                }
            }
            let rising: Vec<(usize, usize)> = cells.iter().copied().filter(|c| !held.contains(c) && h[c] > old(*c) + 0.01).collect();
            if rising.is_empty() {
                break;
            }
            for c in rising {
                held.insert(c);
                h.insert(c, old(c));
            }
        }
        eprintln!("deck {name}: {} cells kept their own height (below the fill)", held.len());
        let drops: Vec<f64> = cells.iter().map(|&c| old(c) - h[&c]).collect();
        let (mut cx, mut cz) = (0.0, 0.0);
        for &p in &pts {
            cx += px(p % w2);
            cz += pz(p / w2);
        }
        let n = pts.len() as f64;
        eprintln!(
            "deck {name}: {:.0} m^2 of 2 m points over the grade, centre ({:.0}, {:.0}); {} cells of 8 m filled in {sweeps} sweeps, lowered by {:.2} m mean, {:.2} m most, raised {:.2} m most",
            n * 4.0,
            cx / n,
            cz / n,
            cells.len(),
            drops.iter().sum::<f64>() / drops.len() as f64,
            drops.iter().copied().fold(f64::MIN, f64::max),
            -drops.iter().copied().fold(f64::MAX, f64::min)
        );
        let to_cm = |y: f64| ((y - eo_data::NAVD88_TO_GAME_Y) * 100.0).round() as i64;
        let cell_rows: Vec<serde_json::Value> = cells.iter().map(|&c| serde_json::json!([c.1, c.0, to_cm(old(c)), to_cm(h[&c])])).collect();
        out_decks.push(serde_json::json!({
            "name": name,
            "seed": {"lat": lat, "lon": lon},
            "why": why,
            "area_m2": (n * 4.0).round(),
            "centre": [(cx / n).round(), (cz / n).round()],
            "cells": cell_rows,
        }));
    }
    if !survey {
        let mut files = serde_json::Map::new();
        for name in ["sf-city.json.gz", "sf-footprints.json.gz", "sf-elevation.bin.gz"] {
            if let Ok(b) = std::fs::read(paths.sf(name)) {
                files.insert(name.to_string(), serde_json::Value::String(super::sha256_hex(&b)));
            }
        }
        let doc = serde_json::json!({
            "meta": {
                "version": "sf-decks-v1",
                "generator": "native/eo-hills/examples/cook/decks.rs (cargo run --release -p eo-hills --example cook -- decks)",
                "definition": "Per deck, cells [i, j, old, new] of public/data/sf/sf-elevation.bin.gz (i east, j south; heights NAVD88 centimetres on the surveyed plan): the cells within one 8 m cell of the deck (the 2 m points of the USGS 1 m DEM 1.5 m or more over the street grade, connected to the deck's seed; the street grade the inverse-distance mean of the DEM along the carriageways at grade within 150 m), each given the harmonic fill (the mean of its four neighbours) of the cells round them. A reader refuses the table if the grid's version, frame or any cell's old height differs.",
                "grid": {"version": m.version, "originX": m.origin_x, "originZ": m.origin_z, "cellSize": m.cell_size, "width": m.width, "height": m.height},
                "sources": {
                    "dem": "USGS 3D Elevation Program 1 m DEM (public domain), the six tiles of external/dem/1m/PROVENANCE.json",
                    "dem_provenance_sha256": dem_sha,
                    "streets_and_footprints": "OpenStreetMap contributors (ODbL), via public/data/sf",
                    "city_files_sha256": files,
                },
            },
            "decks": out_decks,
        });
        let path = eo_data::roof_colours::native_assets().join("sf-decks-v1.json");
        let mut text = serde_json::to_string_pretty(&doc)?;
        text.push('\n');
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("wrote {} ({:.1} s)", path.display(), t0.elapsed().as_secs_f64());
        return Ok(());
    }
    // 4. Footprints on the 2 m points.
    let pg = eo_hills::raster::PlanGrid { origin_x: m.origin_x, origin_z: m.origin_z, texel: 2.0, width: w2, height: h2 };
    let mut built = vec![false; w2 * h2];
    for b in &city.buildings {
        let ring: Vec<[f64; 2]> = b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        if ring.len() >= 3 {
            eo_hills::raster::fill_rings(&pg, &[&ring], |i, j| built[j * w2 + i] = true);
        }
    }
    // 5. The regions (4-connected), each measured.
    let mut label = vec![u32::MAX; w2 * h2];
    let mut regions: Vec<Region> = vec![];
    let slope = |x: f64, z: f64| -> Option<f64> {
        let gx = (dem.at_plan(x + 2.0, z)? - dem.at_plan(x - 2.0, z)?) as f64 / 4.0;
        let gz = (dem.at_plan(x, z + 2.0)? - dem.at_plan(x, z - 2.0)?) as f64 / 4.0;
        Some(gx.hypot(gz))
    };
    for start in 0..w2 * h2 {
        if up[start].is_nan() || label[start] != u32::MAX {
            continue;
        }
        let id = regions.len() as u32;
        let mut stack = vec![start as u32];
        label[start] = id;
        let mut pts = vec![];
        while let Some(p) = stack.pop() {
            pts.push(p);
            let (i, j) = (p as usize % w2, p as usize / w2);
            for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (ni, nj) = (i as i64 + di, j as i64 + dj);
                if ni < 0 || nj < 0 || ni >= w2 as i64 || nj >= h2 as i64 {
                    continue;
                }
                let q = nj as usize * w2 + ni as usize;
                if !up[q].is_nan() && label[q] == u32::MAX {
                    label[q] = id;
                    stack.push(q as u32);
                }
            }
        }
        let area = pts.len() as f64 * 4.0;
        if area < MIN_AREA_M2 {
            regions.push(Region { area, mean_up: 0.0, p90_up: 0.0, flat: 0.0, wall: 0.0, built: 0.0, cx: 0.0, cz: 0.0 });
            continue;
        }
        let mut ups: Vec<f32> = pts.iter().map(|&p| up[p as usize]).collect();
        ups.sort_by(f32::total_cmp);
        let (mut flat, mut nflat, mut edge, mut walls, mut nb, mut cx, mut cz) = (0usize, 0usize, 0usize, 0usize, 0usize, 0.0, 0.0);
        for &p in &pts {
            let (i, j) = (p as usize % w2, p as usize / w2);
            let (x, z) = (px(i), pz(j));
            cx += x;
            cz += z;
            nb += built[p as usize] as usize;
            if let Some(s) = slope(x, z) {
                nflat += 1;
                flat += (s < FLAT_SLOPE) as usize;
            }
            for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (ni, nj) = (i as i64 + di, j as i64 + dj);
                let outside = ni < 0 || nj < 0 || ni >= w2 as i64 || nj >= h2 as i64 || label[nj as usize * w2 + ni as usize] != id;
                if !outside {
                    continue;
                }
                edge += 1;
                let Some(h) = dem.at_plan(x, z) else { continue };
                let lowest = (1..=WALL_RUN_M as i64).filter_map(|k| dem.at_plan(x + (di * k) as f64, z + (dj * k) as f64)).fold(f32::INFINITY, f32::min);
                walls += ((h - lowest) as f64 >= WALL_DROP_M) as usize;
            }
        }
        let n = pts.len() as f64;
        regions.push(Region {
            area,
            mean_up: ups.iter().map(|&v| v as f64).sum::<f64>() / n,
            p90_up: ups[(ups.len() * 9 / 10).min(ups.len() - 1)] as f64,
            flat: flat as f64 / nflat.max(1) as f64,
            wall: walls as f64 / edge.max(1) as f64,
            built: nb as f64 / n,
            cx: cx / n,
            cz: cz / n,
        });
    }
    let mut cand: Vec<(usize, &Region)> = regions.iter().enumerate().filter(|(_, r)| r.area >= MIN_AREA_M2).collect();
    cand.sort_by(|a, b| b.1.area.total_cmp(&a.1.area));
    eprintln!("{} raised regions of {MIN_AREA_M2} m^2 or more ({:.1} s):", cand.len(), t0.elapsed().as_secs_f64());
    let is_deck = |r: &Region| r.flat >= FLAT_SHARE && r.wall >= WALL_SHARE;
    for (k, (id, r)) in cand.iter().enumerate().take(80) {
        let (lon, lat) = frame.to_lonlat(r.cx, r.cz);
        eprintln!(
            "  {k:3} #{id:<7} {} area {:7.0} m2  up mean {:4.1} p90 {:4.1} m  flat {:3.0}%  walls {:3.0}%  built {:3.0}%  at ({:6.0}, {:6.0}) {lat:.5}, {lon:.5}",
            if is_deck(r) { "DECK" } else { "    " },
            r.area,
            r.mean_up,
            r.p90_up,
            100.0 * r.flat,
            100.0 * r.wall,
            100.0 * r.built,
            r.cx,
            r.cz
        );
    }
    if let Some(d) = dump {
        // a mask image of the candidates: (region id), for a look elsewhere
        std::fs::create_dir_all(d)?;
        let mut out = String::new();
        for (id, r) in &cand {
            out.push_str(&format!("{id} {} {} {} {} {} {}\n", r.area, r.cx, r.cz, r.flat, r.wall, is_deck(r) as u8));
        }
        std::fs::write(d.join("candidates.txt"), out)?;
    }
    Ok(())
}
