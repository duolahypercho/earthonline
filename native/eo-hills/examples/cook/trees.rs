//! `trees`: the trees of the hill parks measured from the USGS 3DEP lidar
//! (`eo_hills::trees`, stage H6c) into `<cache>/hills/trees-v1.bin`.
//!
//! Inputs: the point cloud of each area `native/tools/fetch-lidar.py`
//! fetched into `EO_LIDAR_DIR`, else `<repo>/external/lidar` (one folder an
//! area: `ept-data/*.laz` and `PROVENANCE.json`, every file checked against
//! its size and SHA-256 there; any mismatch refuses the cook); the city's
//! footprints at 1:1 (`eo_data::City`); and, to measure the trees against
//! the aerial image, the hill builder's planting map (`cook planting`) when
//! it is in the cache.
//!
//! Method, on a 1 m grid over each area's rings (the city's mapped greens)
//! grown by [`BOX_MARGIN_M`] (the cloud there is fetched to octree depth 8,
//! about 14 points a square metre; the lidar pilot, eo-kit `lidar_survey`,
//! read the full cloud on a 0.5 m grid):
//!
//! * ground: the lowest ground return (ASPRS class 2) in a cell; the holes
//!   (under dense crowns) filled, ring by ring, with the mean of their filled
//!   neighbours; surface: the highest return with two more within
//!   [`TOP_CLUSTER_M`] below it among the cell's four highest (noise,
//!   classes 7 and 18, left out; a lone return over the crown is a stray, a
//!   bird or a wire); height over ground = surface - ground.
//! * vegetation: a cell at least [`MIN_RAISED_M`] up, off the footprints
//!   grown by [`FOOTPRINT_CLEAR_M`], where at least [`VEG_SHARE`] of the
//!   returns are not the last of their pulse (the beam went on through
//!   leaves; the survey's classes here are only ground, unclassified and
//!   noise), with [`VEG_COMPANY`] such cells in its 3 x 3 (wires and poles
//!   drop out): the pilot's rule.
//! * tops: local maxima of the canopy smoothed over 3 x 3 cells at least
//!   [`MIN_TREE_M`] up, the window radius 0.5 m + a quarter of the height,
//!   1.5-6 m (the pilot's); crowns grown downhill from each top (highest
//!   first) to [`CROWN_BASE_FRACTION`] of its height, never climbing more
//!   than [`CROWN_CLIMB_M`].
//! * a tree: a crown of at least [`MIN_CROWN_M2`]; its position the crown's
//!   centroid (where its trunk stands under a lopsided crown); its height
//!   the surface at the top over the ground at the trunk: the lidar's where
//!   it measured ground within a cell, else the 8 m elevation grid's (the
//!   1 m DEM of this survey, resampled; at its measured datum offset) when
//!   the filled ground parts from it by more than [`GROUND_TRUST_M`]; a tree
//!   over [`MAX_TREE_M`] is kept at it; its crown radius sqrt(area / pi).
//!   Only trees inside the rings are kept.
//!
//! `--area name` (repeatable) cooks only those folders (default: every one
//! with a `PROVENANCE.json`); `--out <file>` writes there instead of the
//! cache; `--planting <file>` names the planting map to measure against;
//! `--preview <dir>` writes `trees-<area>.png` per area, 1 m a pixel: the
//! canopy height in grey (black 0 m, white 40 m), the planting map's canopy
//! texels tinted green, the rings in blue, each crown's circle in yellow and
//! its position in red.

use anyhow::{Context, Result, bail};
use eo_data::roof_colours::PlanFrame;
use eo_hills::trees::{self as tr, Area, AreaStats, Header, LidarTrees, Spread, Tree};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BinaryHeap};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The grid's cell (m).
const CELL: f64 = 1.0;
/// The box round an area's rings (m): inside the 30 m the fetch adds.
const BOX_MARGIN_M: f64 = 24.0;
const R_MERC: f64 = 6378137.0;
const MIN_RAISED_M: f64 = 2.0;
const MIN_TREE_M: f64 = 3.0;
const MIN_CROWN_M2: f64 = 4.0;
const CROWN_BASE_FRACTION: f64 = 0.3;
const CROWN_CLIMB_M: f64 = 0.5;
/// A cell's surface is its highest return with two more this far below it (m).
const TOP_CLUSTER_M: f32 = 2.0;
/// Taller than this is not a tree in the city (m; its tallest blue gums are
/// about 60 m, INFERRED): a crown over a drop (a cliff, a canyon wall) read
/// against the drop's foot; such a tree is kept at this height.
const MAX_TREE_M: f64 = 60.0;
/// The filled lidar ground may part this far from the 8 m grid's under a trunk
/// with no ground return round it before the grid's is taken (m).
const GROUND_TRUST_M: f64 = 2.0;
/// The share of a cell's returns that must not be the last of their pulse (the pilot's).
const VEG_SHARE: f64 = 0.25;
/// Vegetation cells a vegetation cell needs in its 3 x 3 (itself included).
const VEG_COMPANY: usize = 5;
/// Cells this near a footprint are not vegetation (eaves, walls).
const FOOTPRINT_CLEAR_M: f64 = 1.0;
/// The ASPRS classes: ground, the two noise classes.
const C_GROUND: u8 = 2;
const C_NOISE: [u8; 2] = [7, 18];

/// The 1 m grid over an area's box.
struct Grid {
    x0: f64,
    z0: f64,
    w: usize,
    h: usize,
}

impl Grid {
    fn idx(&self, x: f64, z: f64) -> Option<usize> {
        let (i, j) = (((x - self.x0) / CELL).floor(), ((z - self.z0) / CELL).floor());
        (i >= 0.0 && j >= 0.0 && (i as usize) < self.w && (j as usize) < self.h).then(|| j as usize * self.w + i as usize)
    }
    fn centre(&self, k: usize) -> (f64, f64) {
        (self.x0 + ((k % self.w) as f64 + 0.5) * CELL, self.z0 + ((k / self.w) as f64 + 0.5) * CELL)
    }
    fn plan_grid(&self) -> eo_hills::raster::PlanGrid {
        eo_hills::raster::PlanGrid { origin_x: self.x0, origin_z: self.z0, texel: CELL, width: self.w, height: self.h }
    }
    /// The 8 neighbours of cell k that are on the grid.
    fn around(&self, k: usize) -> impl Iterator<Item = usize> + '_ {
        let (i, j) = ((k % self.w) as i64, (k / self.w) as i64);
        const N8: [(i64, i64); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)];
        N8.iter().filter_map(move |&(di, dj)| {
            let (a, b) = (i + di, j + dj);
            (a >= 0 && b >= 0 && a < self.w as i64 && b < self.h as i64).then(|| b as usize * self.w + a as usize)
        })
    }
}

/// Per-cell sums of the returns.
struct Cells {
    ground: Vec<f32>,
    /// The four highest returns, highest first.
    surface: Vec<[f32; 4]>,
    /// Returns (noise left out), and those not the last of their pulse (the
    /// beam went on through leaves).
    hits: Vec<u16>,
    through: Vec<u16>,
    ground_hits: Vec<u16>,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}

fn lidar_root() -> Result<PathBuf> {
    if let Some(v) = std::env::var_os("EO_LIDAR_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(v));
    }
    Ok(eo_data::DataPaths::discover()?.repo_root.join(eo_data::external::DIR).join("lidar"))
}

fn in_ring(r: &[[f64; 2]], x: f64, z: f64) -> bool {
    let (mut inside, n) = (false, r.len());
    let mut j = n - 1;
    for i in 0..n {
        let ((xi, zi), (xj, zj)) = ((r[i][0], r[i][1]), (r[j][0], r[j][1]));
        if (zi > z) != (zj > z) && x < (xj - xi) * (z - zi) / (zj - zi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The files PROVENANCE.json lists, each checked (size, SHA-256); with the
/// SHA-256 of the provenance file and of the digests in path order.
fn verified_files(dir: &Path) -> Result<(serde_json::Value, Vec<PathBuf>, u64, String, String)> {
    let p = dir.join("PROVENANCE.json");
    let bytes = std::fs::read(&p).with_context(|| format!("reading {} (fetch the area with native/tools/fetch-lidar.py)", p.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", p.display()))?;
    let mut files: Vec<(String, u64, String)> = v["files"]
        .as_array()
        .context("PROVENANCE.json: no files")?
        .iter()
        .map(|f| (f["path"].as_str().unwrap_or_default().to_string(), f["bytes"].as_u64().unwrap_or(0), f["sha256"].as_str().unwrap_or_default().to_ascii_lowercase()))
        .collect();
    files.sort();
    let mut all = Sha256::new();
    let mut total = 0u64;
    let mut out = vec![];
    for (path, size, sha) in &files {
        let fp = dir.join(path);
        let b = std::fs::read(&fp).with_context(|| format!("reading {}", fp.display()))?;
        if b.len() as u64 != *size || hex(&Sha256::digest(&b)) != *sha {
            bail!("{}: {} bytes, digest {} (PROVENANCE.json: {size} bytes, {sha})", fp.display(), b.len(), hex(&Sha256::digest(&b)));
        }
        all.update(sha.as_bytes());
        total += *size;
        out.push(fp);
    }
    Ok((v, out, total, hex(&Sha256::digest(&bytes)), hex(&all.finalize())))
}

/// Read every file's points into the grid (several files at once).
fn read_points(files: &[PathBuf], grid: &Grid, frame: &PlanFrame, classes: &mut BTreeMap<String, u64>) -> Result<(Cells, u64)> {
    let n = grid.w * grid.h;
    let mut c = Cells { ground: vec![f32::INFINITY; n], surface: vec![[f32::NEG_INFINITY; 4]; n], hits: vec![0; n], through: vec![0; n], ground_hits: vec![0; n] };
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<(u32, f32, u8, bool)>>>(eo_hills::threads() * 2);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut points = 0u64;
    let mut class_count = [0u64; 256];
    std::thread::scope(|s| -> Result<()> {
        for _ in 0..eo_hills::threads().min(files.len().max(1)) {
            let (tx, next) = (tx.clone(), &next);
            s.spawn(move || {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(p) = files.get(i) else { break };
                    let got = (|| -> Result<Vec<(u32, f32, u8, bool)>> {
                        let mut r = las::Reader::from_path(p).with_context(|| format!("opening {}", p.display()))?;
                        let pts = r.read_all().with_context(|| format!("reading {}", p.display()))?;
                        let mut out = Vec::new();
                        for (((((x, y), z), cl), rn), nr) in pts.x().zip(pts.y()).zip(pts.z()).zip(pts.classification()).zip(pts.return_number()).zip(pts.number_of_returns()) {
                            let lon = (x / R_MERC).to_degrees();
                            let lat = (2.0 * (y / R_MERC).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
                            let (px, pz) = frame.to_plan(lon, lat);
                            if let Some(k) = grid.idx(px, pz) {
                                out.push((k as u32, z as f32, cl, rn < nr));
                            }
                        }
                        Ok(out)
                    })();
                    if tx.send(got).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for got in rx {
            for (k, z, cl, thru) in got? {
                let k = k as usize;
                class_count[cl as usize] += 1;
                points += 1;
                if C_NOISE.contains(&cl) {
                    continue;
                }
                if cl == C_GROUND {
                    c.ground[k] = c.ground[k].min(z);
                    c.ground_hits[k] = c.ground_hits[k].saturating_add(1);
                }
                c.hits[k] = c.hits[k].saturating_add(1);
                if thru {
                    c.through[k] = c.through[k].saturating_add(1);
                }
                let t = &mut c.surface[k];
                if z > t[3] {
                    let mut i = 3;
                    while i > 0 && z > t[i - 1] {
                        t[i] = t[i - 1];
                        i -= 1;
                    }
                    t[i] = z;
                }
            }
        }
        Ok(())
    })?;
    for (k, &v) in class_count.iter().enumerate() {
        if v > 0 {
            *classes.entry(k.to_string()).or_insert(0) += v;
        }
    }
    Ok((c, points))
}

/// Fill the ground's holes ring by ring with the mean of their filled
/// neighbours (a hole under a crown on a slope takes its sides' mean, not
/// the nearest side).
fn fill_ground(grid: &Grid, ground: &mut [f32]) {
    let mut front: Vec<usize> = (0..ground.len()).filter(|&k| !ground[k].is_finite() && grid.around(k).any(|q| ground[q].is_finite())).collect();
    let mut queued = vec![false; ground.len()];
    for &k in &front {
        queued[k] = true;
    }
    while !front.is_empty() {
        let vals: Vec<f32> = front
            .iter()
            .map(|&k| {
                let (s, n) = grid.around(k).filter(|&q| ground[q].is_finite()).fold((0.0f32, 0u32), |a, q| (a.0 + ground[q], a.1 + 1));
                s / n.max(1) as f32
            })
            .collect();
        for (&k, v) in front.iter().zip(vals) {
            ground[k] = v;
        }
        let mut next = vec![];
        for &k in &front {
            for q in grid.around(k) {
                if !ground[q].is_finite() && !queued[q] {
                    queued[q] = true;
                    next.push(q);
                }
            }
        }
        front = next;
    }
}

struct Found {
    /// Crown centroid x, z, the top's height (NAVD88, the top cell's surface), the crown radius.
    trees: Vec<(f64, f64, f64, f64)>,
    veg: Vec<bool>,
    chm: Vec<f64>,
}

/// Vegetation, tops, crowns and trees on the grid.
fn find_trees(grid: &Grid, c: &Cells, near_fp: &[bool]) -> Found {
    let n = grid.w * grid.h;
    // the highest return with two more within TOP_CLUSTER_M below it; else the lowest of the four kept
    let top = |k: usize| {
        let t = c.surface[k];
        (0..2).find(|&i| t[i + 2] >= t[i] - TOP_CLUSTER_M).map(|i| t[i]).unwrap_or_else(|| t.iter().copied().filter(|v| v.is_finite()).fold(f32::INFINITY, f32::min))
    };
    let ndsm: Vec<f64> = (0..n).map(|k| if c.surface[k][0].is_finite() && c.ground[k].is_finite() { (top(k) - c.ground[k]).max(0.0) as f64 } else { 0.0 }).collect();
    let leafy: Vec<bool> = (0..n).map(|k| !near_fp[k] && ndsm[k] >= MIN_RAISED_M && c.hits[k] > 0 && c.through[k] as f64 >= VEG_SHARE * c.hits[k] as f64).collect();
    let veg: Vec<bool> = (0..n).map(|k| leafy[k] && 1 + grid.around(k).filter(|&q| leafy[q]).count() >= VEG_COMPANY).collect();
    // the canopy smoothed over 3 x 3 vegetation cells
    let chm: Vec<f64> = (0..n)
        .map(|k| {
            if !veg[k] {
                return 0.0;
            }
            let (s, m) = std::iter::once(k).chain(grid.around(k)).filter(|&q| veg[q]).fold((0.0, 0.0), |a, q| (a.0 + ndsm[q], a.1 + 1.0));
            s / m
        })
        .collect();
    // tops: local maxima, the window growing with the height
    let mut tops: Vec<usize> = vec![];
    for k in 0..n {
        let h = chm[k];
        if h < MIN_TREE_M {
            continue;
        }
        let r = (0.5 + 0.25 * h).clamp(1.5, 6.0);
        let rc = (r / CELL).ceil() as i64;
        let (i, j) = ((k % grid.w) as i64, (k / grid.w) as i64);
        let mut is_max = true;
        'w: for dj in -rc..=rc {
            for di in -rc..=rc {
                if (di == 0 && dj == 0) || (di * di + dj * dj) as f64 * CELL * CELL > r * r {
                    continue;
                }
                let (a, b) = (i + di, j + dj);
                if a < 0 || b < 0 || a >= grid.w as i64 || b >= grid.h as i64 {
                    continue;
                }
                let q = b as usize * grid.w + a as usize;
                if chm[q] > h || (chm[q] == h && q < k) {
                    is_max = false;
                    break 'w;
                }
            }
        }
        if is_max {
            tops.push(k);
        }
    }
    // crowns grown downhill from the tops (priority flood, highest first)
    let mut label = vec![u32::MAX; n];
    let mut heap = BinaryHeap::new();
    for (t, &k) in tops.iter().enumerate() {
        label[k] = t as u32;
        heap.push(((chm[k] * 1000.0) as i64, k));
    }
    while let Some((_, k)) = heap.pop() {
        let top_h = chm[tops[label[k] as usize]];
        for q in grid.around(k).collect::<Vec<_>>() {
            if label[q] != u32::MAX || !veg[q] || chm[q] < CROWN_BASE_FRACTION * top_h || chm[q] > chm[k] + CROWN_CLIMB_M {
                continue;
            }
            label[q] = label[k];
            heap.push(((chm[q] * 1000.0) as i64, q));
        }
    }
    let mut area = vec![0usize; tops.len()];
    let mut sum = vec![(0.0f64, 0.0f64); tops.len()];
    for k in 0..n {
        if label[k] != u32::MAX {
            let t = label[k] as usize;
            let (x, z) = grid.centre(k);
            area[t] += 1;
            sum[t].0 += x;
            sum[t].1 += z;
        }
    }
    let mut trees = vec![];
    for (t, &k) in tops.iter().enumerate() {
        let a = area[t] as f64 * CELL * CELL;
        if a < MIN_CROWN_M2 {
            continue;
        }
        let top_z = top(k) as f64;
        let (x, z) = (sum[t].0 / area[t] as f64, sum[t].1 / area[t] as f64);
        trees.push((x, z, top_z, (a / std::f64::consts::PI).sqrt()));
    }
    Found { trees, veg, chm }
}

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) -> Result<()> {
    let f = std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(rgb)?;
    Ok(())
}

pub fn run(args: &[String]) -> Result<()> {
    let t0 = Instant::now();
    let (mut only, mut preview): (Vec<String>, Option<PathBuf>) = (vec![], None);
    let mut out = tr::file_path(&eo_hills::cache_dir());
    let mut planting_path = eo_hills::planting::file_path(&eo_hills::cache_dir());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--area" => only.push(it.next().context("--area needs a name")?.clone()),
            "--preview" => preview = Some(PathBuf::from(it.next().context("--preview needs a folder")?)),
            "--out" => out = PathBuf::from(it.next().context("--out needs a file")?),
            "--planting" => planting_path = PathBuf::from(it.next().context("--planting needs a file")?),
            _ => bail!("usage: cook trees [--area name ...] [--preview <dir>] [--out <file>] [--planting <file>]   (EO_LIDAR_DIR: the lidar folders, default <repo>/external/lidar; EO_NATIVE_CACHE: where the trees go)"),
        }
    }
    let root = lidar_root()?;
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .with_context(|| format!("reading {} (fetch the areas with native/tools/fetch-lidar.py)", root.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("PROVENANCE.json").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    if !only.is_empty() {
        if let Some(m) = only.iter().find(|a| !names.contains(a)) {
            bail!("no area {m} in {} (has {names:?})", root.display());
        }
        names.retain(|n| only.contains(n));
    }
    if names.is_empty() {
        bail!("no lidar areas in {}", root.display());
    }
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load_at(&paths, 1.0).context("the city at 1:1")?;
    let frame = PlanFrame::from_meta(&city.meta);
    let elev = eo_data::ElevationGrid::decode(&eo_data::read_maybe_gzip(&paths.sf("sf-elevation.bin.gz"))?)?;
    let planting = planting_path.is_file().then(|| eo_hills::planting::Planting::open(&planting_path)).transpose().context("the planting map")?;
    // the park trees' NAIP rule (eo-app park_tree_canopy_ok) in surveyed metres
    let gs = eo_data::groundscale::scale();
    let naip_box = eo_landmarks::vegetation::NAIP_TREE_BOX_M / gs;
    let naip_canopy = eo_landmarks::vegetation::NAIP_TREE_CANOPY;
    eprintln!("{} areas in {}; {} footprints; planting map {}", names.len(), root.display(), city.buildings.len(), if planting.is_some() { "read" } else { "absent (no NAIP comparison)" });

    let mut areas = vec![];
    let mut trees: Vec<Tree> = vec![];
    for (ai, name) in names.iter().enumerate() {
        let ta = Instant::now();
        let dir = root.join(name);
        let (prov, files, bytes, prov_sha, files_sha) = verified_files(&dir)?;
        let a = &prov["area"];
        let rings: Vec<Vec<[f64; 2]>> = a["rings_lonlat"]
            .as_array()
            .context("PROVENANCE.json: area.rings_lonlat")?
            .iter()
            .map(|r| r.as_array().into_iter().flatten().filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?))).map(|(lon, lat)| frame.to_plan(lon, lat)).map(|(x, z)| [x, z]).collect::<Vec<_>>())
            .filter(|r: &Vec<[f64; 2]>| r.len() >= 3)
            .collect();
        if rings.is_empty() {
            bail!("{name}: no rings");
        }
        let (mut x0, mut z0, mut x1, mut z1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in rings.iter().flatten() {
            x0 = x0.min(p[0]);
            z0 = z0.min(p[1]);
            x1 = x1.max(p[0]);
            z1 = z1.max(p[1]);
        }
        let grid = Grid { x0: (x0 - BOX_MARGIN_M).floor(), z0: (z0 - BOX_MARGIN_M).floor(), w: ((x1 - x0 + 2.0 * BOX_MARGIN_M) / CELL).ceil() as usize + 1, h: ((z1 - z0 + 2.0 * BOX_MARGIN_M) / CELL).ceil() as usize + 1 };
        let n = grid.w * grid.h;
        let mut classes = BTreeMap::new();
        let (mut cells, points) = read_points(&files, &grid, &frame, &mut classes)?;
        let ground_returns: u64 = cells.ground_hits.iter().map(|&v| v as u64).sum();
        fill_ground(&grid, &mut cells.ground);
        // the footprints, grown
        let pg = grid.plan_grid();
        let mut fp = vec![false; n];
        for b in &city.buildings {
            let ring: Vec<[f64; 2]> = b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
            let (bx0, bx1) = ring.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[0]), a.1.max(p[0])));
            let (bz0, bz1) = ring.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
            if bx1 < grid.x0 || bx0 > grid.x0 + grid.w as f64 * CELL || bz1 < grid.z0 || bz0 > grid.z0 + grid.h as f64 * CELL {
                continue;
            }
            eo_hills::raster::fill_rings(&pg, &[ring.as_slice()], |i, j| fp[j * grid.w + i] = true);
        }
        let grow = (FOOTPRINT_CLEAR_M / CELL).round() as usize;
        let mut near_fp = fp;
        for _ in 0..grow {
            let prev = near_fp.clone();
            for k in 0..n {
                if !prev[k] && grid.around(k).any(|q| prev[q]) {
                    near_fp[k] = true;
                }
            }
        }
        let found = find_trees(&grid, &cells, &near_fp);
        // inside the rings
        let mut inside = vec![false; n];
        for r in &rings {
            eo_hills::raster::fill_rings(&pg, &[r.as_slice()], |i, j| inside[j * grid.w + i] = true);
        }
        // the ground under each trunk (the crown's centroid): the lidar's where it
        // measured ground within a cell, else the 8 m grid's (the 1 m DEM of this
        // survey, resampled) when the filled lidar ground parts from it by more than
        // GROUND_TRUST_M (a wide hole under a crown on a slope).
        let offset = {
            let mut d: Vec<f32> = (0..n)
                .step_by(5)
                .filter(|&k| cells.ground_hits[k] > 0)
                .filter_map(|k| {
                    let (x, z) = grid.centre(k);
                    elev.sample_surveyed(x, z).map(|g| cells.ground[k] - g as f32)
                })
                .collect();
            Spread::of(&mut d).p50 as f64
        };
        let (mut from_grid, mut clamped, mut gap) = (0u32, 0u32, vec![]);
        let mut kept: Vec<Tree> = vec![];
        for &(x, z, top_z, r) in found.trees.iter().filter(|t| rings.iter().any(|r| in_ring(r, t.0, t.1))) {
            let Some(k) = grid.idx(x, z) else { continue };
            let lidar_g = cells.ground[k] as f64;
            let measured = std::iter::once(k).chain(grid.around(k)).any(|q| cells.ground_hits[q] > 0);
            let grid_g = elev.sample_surveyed(x, z).map(|g| g + offset);
            if let Some(gg) = grid_g {
                gap.push((lidar_g - gg).abs() as f32);
            }
            let (g, flags) = match grid_g {
                Some(gg) if !measured && (lidar_g - gg).abs() > GROUND_TRUST_M => (gg, tr::FLAG_GRID_GROUND),
                _ => (lidar_g, 0),
            };
            let h = top_z - g;
            if h < MIN_TREE_M {
                continue;
            }
            if h > MAX_TREE_M {
                clamped += 1;
            }
            let h = h.min(MAX_TREE_M);
            from_grid += (flags != 0) as u32;
            kept.push(Tree { x: x as f32, z: z as f32, height_m: h as f32, crown_r_m: r as f32, ground_navd88_m: g as f32, area: ai as u16, flags });
        }
        let gap = Spread::of(&mut gap);
        let ring_cells = inside.iter().filter(|v| **v).count();
        let canopy_cells = (0..n).filter(|&k| inside[k] && found.veg[k]).count();
        // against the aerial image
        let (mut agree, mut lidar_in_naip, mut naip_in_lidar) = (None, None, None);
        if let Some(pm) = &planting {
            let ok: Vec<bool> = kept.iter().filter_map(|t| pm.shares(t.x as f64, t.z as f64, naip_box).map(|s| s.canopy >= naip_canopy)).collect();
            if !ok.is_empty() {
                agree = Some(ok.iter().filter(|v| **v).count() as f64 / ok.len() as f64);
            }
            // 2 m blocks inside the rings: lidar canopy in the block, NAIP canopy (half its pixels) in its texel
            let (mut both, mut l, mut nn) = (0u64, 0u64, 0u64);
            for j in (0..grid.h - 1).step_by(2) {
                for i in (0..grid.w - 1).step_by(2) {
                    let ks = [j * grid.w + i, j * grid.w + i + 1, (j + 1) * grid.w + i, (j + 1) * grid.w + i + 1];
                    if !ks.iter().all(|&k| inside[k]) {
                        continue;
                    }
                    let (x, z) = (grid.x0 + i as f64 * CELL + 1.0, grid.z0 + j as f64 * CELL + 1.0);
                    let Some(s) = pm.shares(x, z, 2.0) else { continue };
                    let lv = ks.iter().any(|&k| found.veg[k]);
                    let nv = s.canopy >= 0.5;
                    both += (lv && nv) as u64;
                    l += lv as u64;
                    nn += nv as u64;
                }
            }
            if l > 0 {
                lidar_in_naip = Some(both as f64 / l as f64);
            }
            if nn > 0 {
                naip_in_lidar = Some(both as f64 / nn as f64);
            }
        }
        let box_m2 = (grid.w * grid.h) as f64 * CELL * CELL;
        let stats = AreaStats {
            trees: kept.len() as u32,
            height_m: Spread::of(&mut kept.iter().map(|t| t.height_m).collect::<Vec<_>>()),
            crown_r_m: Spread::of(&mut kept.iter().map(|t| t.crown_r_m).collect::<Vec<_>>()),
            area_m2: ring_cells as f64 * CELL * CELL,
            canopy_share: canopy_cells as f64 / ring_cells.max(1) as f64,
            points,
            points_per_m2: points as f64 / box_m2,
            ground_per_m2: ground_returns as f64 / box_m2,
            classes,
            naip_canopy_agree: agree,
            lidar_canopy_in_naip: lidar_in_naip,
            naip_canopy_in_lidar: naip_in_lidar,
            ground_from_grid: from_grid,
            clamped_tall: clamped,
            ground_gap_m: gap,
            grid_offset_m: offset,
        };
        eprintln!(
            "{name}: {} files, {:.1} MB, {} points ({:.1} a m2, ground {:.2} a m2); ground under the trunks against the 8 m grid (offset {offset:+.2} m) p50 {:.2} p90 {:.2} m, {from_grid} from the grid, {clamped} over {MAX_TREE_M} m kept at it; {} trees in {:.1} ha (canopy {:.0} %); height p10/50/90 {:.1}/{:.1}/{:.1} m (max {:.1}), crown radius {:.1}/{:.1}/{:.1} m; NAIP canopy over the trees {}, lidar canopy in NAIP canopy {}, NAIP canopy in lidar canopy {} ({:.1} s)",
            files.len(),
            bytes as f64 / 1e6,
            points,
            stats.points_per_m2,
            stats.ground_per_m2,
            gap.p50,
            gap.p90,
            kept.len(),
            stats.area_m2 / 1e4,
            100.0 * stats.canopy_share,
            stats.height_m.p10,
            stats.height_m.p50,
            stats.height_m.p90,
            stats.height_m.max,
            stats.crown_r_m.p10,
            stats.crown_r_m.p50,
            stats.crown_r_m.p90,
            agree.map(|v| format!("{:.0} %", 100.0 * v)).unwrap_or("-".into()),
            lidar_in_naip.map(|v| format!("{:.0} %", 100.0 * v)).unwrap_or("-".into()),
            naip_in_lidar.map(|v| format!("{:.0} %", 100.0 * v)).unwrap_or("-".into()),
            ta.elapsed().as_secs_f64()
        );
        if let Some(dir) = &preview {
            std::fs::create_dir_all(dir)?;
            let mut rgb = vec![0u8; n * 3];
            for k in 0..n {
                let g = (found.chm[k] / 40.0 * 255.0).clamp(0.0, 255.0) as u8;
                let mut px = [g / 2 + 20, g / 2 + 20, g / 2 + 20];
                if found.veg[k] {
                    px = [g, g, g];
                }
                if let Some(pm) = &planting {
                    let (x, z) = grid.centre(k);
                    if pm.shares(x, z, 2.0).is_some_and(|s| s.canopy >= 0.5) {
                        px = [px[0] / 2, (px[1] as u16 / 2 + 70).min(255) as u8, px[2] / 2];
                    }
                }
                rgb[3 * k..3 * k + 3].copy_from_slice(&px);
            }
            let mut put = |x: f64, z: f64, c: [u8; 3]| {
                if let Some(k) = grid.idx(x, z) {
                    rgb[3 * k..3 * k + 3].copy_from_slice(&c);
                }
            };
            for r in &rings {
                for q in 0..r.len() {
                    let (a, b) = (r[q], r[(q + 1) % r.len()]);
                    let steps = ((b[0] - a[0]).hypot(b[1] - a[1]) / (0.5 * CELL)).ceil().max(1.0) as usize;
                    for s in 0..=steps {
                        let u = s as f64 / steps as f64;
                        put(a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, [60, 110, 255]);
                    }
                }
            }
            for t in &kept {
                let steps = (t.crown_r_m as f64 * 8.0).ceil().max(8.0) as usize;
                for s in 0..steps {
                    let (sn, cs) = (s as f64 / steps as f64 * std::f64::consts::TAU).sin_cos();
                    put(t.x as f64 + cs * t.crown_r_m as f64, t.z as f64 + sn * t.crown_r_m as f64, [255, 220, 40]);
                }
                put(t.x as f64, t.z as f64, [255, 40, 40]);
            }
            let out = dir.join(format!("trees-{name}.png"));
            write_png(&out, grid.w, grid.h, &rgb)?;
            eprintln!("  preview {}", out.display());
        }
        trees.extend(kept);
        areas.push(Area {
            name: name.clone(),
            label: a["label"].as_str().unwrap_or_default().to_string(),
            greens: a["greens"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect(),
            rings,
            max_depth: a["max_depth"].as_u64().unwrap_or(0) as u32,
            files: files.len() as u32,
            bytes,
            provenance_sha256: prov_sha,
            files_sha256: files_sha,
            stats,
        });
    }
    let mut city_files = BTreeMap::new();
    for name in ["sf-city.json.gz", "sf-lots-parks.json.gz", "sf-footprints.json.gz"] {
        if let Ok(b) = std::fs::read(paths.sf(name)) {
            city_files.insert(name.to_string(), super::sha256_hex(&b));
        }
    }
    if planting.is_some() {
        if let Ok(b) = std::fs::read(&planting_path) {
            city_files.insert(format!("<cache>/hills/{}", eo_hills::planting::FILE_NAME), super::sha256_hex(&b));
        }
    }
    let params: BTreeMap<String, f64> = [
        ("cell_m", CELL),
        ("box_margin_m", BOX_MARGIN_M),
        ("min_raised_m", MIN_RAISED_M),
        ("min_tree_m", MIN_TREE_M),
        ("min_crown_m2", MIN_CROWN_M2),
        ("crown_base_fraction", CROWN_BASE_FRACTION),
        ("crown_climb_m", CROWN_CLIMB_M),
        ("top_cluster_m", TOP_CLUSTER_M as f64),
        ("max_tree_m", MAX_TREE_M),
        ("ground_trust_m", GROUND_TRUST_M),
        ("veg_share", VEG_SHARE),
        ("veg_company", VEG_COMPANY as f64),
        ("footprint_clear_m", FOOTPRINT_CLEAR_M),
        ("naip_box_surveyed_m", naip_box),
        ("naip_tree_canopy", naip_canopy as f64),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let header = Header {
        format: tr::FORMAT.into(),
        record_bytes: tr::RECORD_BYTES as u32,
        generator: "eo-hills cook trees v1 (1 m grid, split returns as leaves, crowns by priority flood)".into(),
        plan: "surveyed 1:1 plan metres (x east, z south), the city's own frame (sf-city.json.gz meta)".into(),
        source: "USGS 3D Elevation Program, CA_SanFrancisco_B23 (work unit CA_SanFrancisco_1_B23), Entwine Point Tiles on the AWS Open Data bucket usgs-lidar-public".into(),
        source_url: "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/CA_SanFrancisco_1_B23".into(),
        licence: "Public domain (U.S. Government work)".into(),
        attribution: "U.S. Geological Survey, 3D Elevation Program".into(),
        params,
        city_files,
        areas,
        trees: 0,
    };
    let lt = LidarTrees::new(header, trees);
    lt.write(&out)?;
    let mut all_h: Vec<f32> = lt.trees().iter().map(|t| t.height_m).collect();
    let mut all_r: Vec<f32> = lt.trees().iter().map(|t| t.crown_r_m).collect();
    let (sh, sr) = (Spread::of(&mut all_h), Spread::of(&mut all_r));
    eprintln!(
        "wrote {} ({} trees; height p10/50/90 {:.1}/{:.1}/{:.1} m, crown radius {:.1}/{:.1}/{:.1} m; {:.1} s)",
        out.display(),
        lt.header().trees,
        sh.p10,
        sh.p50,
        sh.p90,
        sr.p10,
        sr.p50,
        sr.p90,
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}
