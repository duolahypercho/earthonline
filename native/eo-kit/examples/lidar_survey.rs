//! Trees and roof shapes measured from the USGS 3DEP lidar point cloud.
//!
//! Source: USGS 3D Elevation Program, CA_SanFrancisco_B23 (the 2023 survey
//! the game's 1 m ground model already comes from), public domain, fetched
//! for one area by `native/tools/fetch-lidar.py` into a folder OUTSIDE the
//! repository named by `EO_LIDAR_DIR` (required). Steps:
//!
//!   python3 -I native/tools/fetch-lidar.py --name liberty-hill --lat 37.7570 --lon -122.4250 --half 150
//!   cargo run --release -p eo-kit --example lidar_survey -- liberty-hill
//!
//! Writes into `$EO_LIDAR_DIR/<name>/`:
//! * `trees.json`  one record per tree crown found off the buildings: its top
//!   (surveyed 1:1 plan metres, x east, z south), height above the lidar
//!   ground and crown radius;
//! * `roofs.json`  one record per footprint wholly inside the area: the roof
//!   form (flat, gable, or complex), the ridge axis, eave and ridge heights
//!   above the ground, pitch, the fit's error, and the footprint's own height;
//! * `evidence-*.png` top-down pictures of the canopy, crowns and roof forms.
//!
//! Method (all on a 0.5 m grid over the area):
//! * ground: the lowest class-2 (ground) return in a cell, holes filled from
//!   the nearest measured ground; surface: the highest return (noise classes
//!   7 and 18 left out); height above ground = surface - ground.
//! * the plan shift between the cloud and the footprints is measured (the
//!   shift, within 3 m, at which the footprints best match the raised cells)
//!   and applied, so crowns and roofs land on the game's footprints.
//! * vegetation: raised cells (>= 2 m) off the footprints (grown by 0.5 m)
//!   where at least 15 % of the returns are not the last of their pulse (the
//!   beam went through leaves), with 5 such cells in its 3 x 3; cells within
//!   1 m of a footprint are left out (eaves and walls split a pulse too).
//! * trees: local maxima of the canopy smoothed over 2.5 m (>= 3 m; window
//!   radius 0.5 m + a quarter of the height, 1.5-6 m, about a broadleaf's crown),
//!   crowns grown downhill from each top to 30 % of its height;
//!   a crown of at least 4 m^2 under a top at least 3 m up is a tree, its
//!   radius sqrt(area / pi).
//! * roofs: the surface inside each footprint (shrunk by 0.75 m); the error of
//!   a fit is its 80th-percentile absolute residual (chimneys, parapets, light
//!   wells and skylights left to the other 20 %), each fit made again over
//!   its best 80 %; cells whose returns leaves split are not roof. One plane:
//!   flat under 8 degrees with an error under 0.5 m (bays and parapet steps),
//!   a shed (one slope) from 8 to 50 degrees with an error under 0.3 m; two planes meeting at a ridge along either of the footprint's main
//!   axes: a gable when both slope 12-60 degrees toward the ridge, the error is
//!   under 0.4 m and at most 0.7 of the one plane's. Stepped: two or three
//!   flat levels at least 1.5 m apart (peaks of the heights in 0.5 m bins,
//!   each holding 12 % of the cells) that hold 75 % of the cells within 0.4 m
//!   (a tall front block and a lower rear, SF's usual row house). Anything
//!   else is complex.

use las::Reader;
use sha2::{Digest, Sha256};
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};

const CELL: f64 = 0.5;
const R_MERC: f64 = 6378137.0;
const NAVD88_TO_GAME_Y: f64 = -(0.969 + 0.55);
const MIN_RAISED_M: f64 = 2.0;
const MIN_TREE_M: f64 = 3.0;
const MIN_CROWN_M2: f64 = 4.0;
const CROWN_BASE_FRACTION: f64 = 0.3;
const VEG_SHARE: f64 = 0.25;
const ROOF_INSET_M: f64 = 0.75;

struct Grid {
    x0: f64,
    z0: f64,
    n: usize,
}

impl Grid {
    fn idx(&self, x: f64, z: f64) -> Option<usize> {
        let i = ((x - self.x0) / CELL).floor();
        let j = ((z - self.z0) / CELL).floor();
        if i < 0.0 || j < 0.0 || i >= self.n as f64 || j >= self.n as f64 {
            return None;
        }
        Some(j as usize * self.n + i as usize)
    }
    fn centre(&self, k: usize) -> (f64, f64) {
        (self.x0 + ((k % self.n) as f64 + 0.5) * CELL, self.z0 + ((k / self.n) as f64 + 0.5) * CELL)
    }
}

fn lidar_dir(name: &str) -> PathBuf {
    PathBuf::from(std::env::var("EO_LIDAR_DIR").expect("set EO_LIDAR_DIR to the folder fetch-lidar.py wrote")).join(name)
}

fn point_in_ring(x: f64, z: f64, ring: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, zi) = (ring[i][0] as f64, ring[i][1] as f64);
        let (xj, zj) = (ring[j][0] as f64, ring[j][1] as f64);
        if (zi > z) != (zj > z) && x < (xj - xi) * (z - zi) / (zj - zi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn dist_to_ring(x: f64, z: f64, ring: &[[f32; 2]]) -> f64 {
    let mut best = f64::MAX;
    let n = ring.len();
    for i in 0..n {
        let a = [ring[i][0] as f64, ring[i][1] as f64];
        let b = [ring[(i + 1) % n][0] as f64, ring[(i + 1) % n][1] as f64];
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dz * dz;
        let t = if l2 > 0.0 { (((x - a[0]) * dx + (z - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let (px, pz) = (a[0] + t * dx - x, a[1] + t * dz - z);
        best = best.min((px * px + pz * pz).sqrt());
    }
    best
}

/// Least squares of y = a + b u over (u, y); returns (a, b, absolute residuals).
fn line_fit(pts: &[(f64, f64)]) -> Option<(f64, f64, Vec<f64>)> {
    let n = pts.len() as f64;
    if n < 6.0 {
        return None;
    }
    let (su, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (mu, my) = (su / n, sy / n);
    let (mut suu, mut suy) = (0.0, 0.0);
    for p in pts {
        suu += (p.0 - mu) * (p.0 - mu);
        suy += (p.0 - mu) * (p.1 - my);
    }
    if suu < 1e-9 {
        return None;
    }
    let b = suy / suu;
    let a = my - b * mu;
    Some((a, b, pts.iter().map(|p| (p.1 - a - b * p.0).abs()).collect()))
}

/// Least squares of y = a + b x + c z; returns (b, c, robust error).
fn plane_fit(pts: &[(f64, f64, f64)]) -> Option<(f64, f64, f64)> {
    let n = pts.len() as f64;
    if n < 8.0 {
        return None;
    }
    let (mx, mz, my) = pts.iter().fold((0.0, 0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n, a.2 + p.2 / n));
    let (mut sxx, mut sxz, mut szz, mut sxy, mut szy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for p in pts {
        let (dx, dz, dy) = (p.0 - mx, p.1 - mz, p.2 - my);
        sxx += dx * dx;
        sxz += dx * dz;
        szz += dz * dz;
        sxy += dx * dy;
        szy += dz * dy;
    }
    let det = sxx * szz - sxz * sxz;
    if det.abs() < 1e-9 {
        return None;
    }
    let b = (sxy * szz - szy * sxz) / det;
    let c = (szy * sxx - sxy * sxz) / det;
    let mut res: Vec<f64> = pts.iter().map(|p| (p.2 - my - b * (p.0 - mx) - c * (p.1 - mz)).abs()).collect();
    Some((b, c, robust(&mut res)))
}

/// `plane_fit` again over the 80 % of the cells it fits best (a chimney, a
/// tree's edge or a light well no longer tilts the plane).
fn plane_fit_trimmed(pts: &[(f64, f64, f64)]) -> Option<(f64, f64, f64)> {
    let (b, c, _) = plane_fit(pts)?;
    let n = pts.len() as f64;
    let (mx, mz, my) = pts.iter().fold((0.0, 0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n, a.2 + p.2 / n));
    let mut res: Vec<f64> = pts.iter().map(|p| (p.2 - my - b * (p.0 - mx) - c * (p.1 - mz)).abs()).collect();
    let cut = percentile(&mut res.clone(), 0.8);
    let kept: Vec<(f64, f64, f64)> = pts.iter().zip(res.iter_mut()).filter(|(_, r)| **r <= cut).map(|(p, _)| *p).collect();
    plane_fit(&kept)
}

/// `line_fit` again over the 80 % of the points it fits best; the residuals
/// returned are every point's against the refit.
fn line_fit_trimmed(pts: &[(f64, f64)]) -> Option<(f64, f64, Vec<f64>)> {
    let (_, _, res) = line_fit(pts)?;
    let cut = percentile(&mut res.clone(), 0.8);
    let kept: Vec<(f64, f64)> = pts.iter().zip(res.iter()).filter(|(_, r)| **r <= cut).map(|(p, _)| *p).collect();
    let (a, b, _) = line_fit(&kept)?;
    Some((a, b, pts.iter().map(|p| (p.1 - a - b * p.0).abs()).collect()))
}

/// A fit's error: its 80th-percentile absolute residual.
fn robust(res: &mut [f64]) -> f64 {
    percentile(res, 0.8)
}

fn percentile(v: &mut [f64], q: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// The footprint's two main axes (unit vectors in plan): its longest edge's
/// direction and the perpendicular.
fn main_axes(ring: &[[f32; 2]]) -> [(f64, f64); 2] {
    let n = ring.len();
    let mut best = (0.0, (1.0, 0.0));
    for i in 0..n {
        let (dx, dz) = ((ring[(i + 1) % n][0] - ring[i][0]) as f64, (ring[(i + 1) % n][1] - ring[i][1]) as f64);
        let l = dx.hypot(dz);
        if l > best.0 {
            best = (l, (dx / l, dz / l));
        }
    }
    let a = best.1;
    [a, (-a.1, a.0)]
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Ord64(i64);

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) {
    let f = std::fs::File::create(path).expect("png file");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().expect("png header").write_image_data(rgb).expect("png data");
}

fn main() {
    let name = std::env::args().nth(1).expect("usage: lidar_survey <area name>");
    let dir = lidar_dir(&name);
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("manifest.json")).expect("manifest json");
    let root = eo_kit::repo_root().expect("repo root (EO_REPO_ROOT)");
    let paths = eo_data::DataPaths { repo_root: root };
    let city = eo_data::City::load_at(&paths, 1.0).expect("city at 1:1");
    let frame = eo_data::roof_colours::PlanFrame::from_meta(&city.meta);

    // the area on the surveyed plan
    let area = &manifest["area"];
    let (lat, lon, half) = (area["lat"].as_f64().unwrap(), area["lon"].as_f64().unwrap(), area["half_m"].as_f64().unwrap());
    let (cx, cz) = frame.to_plan(lon, lat);
    let half = half - 2.0; // the nodes' edge
    let n = (2.0 * half / CELL).floor() as usize;
    let grid = Grid { x0: cx - half, z0: cz - half, n };
    let cells = n * n;

    // ---- read the points, checking every file against the manifest
    let mut ground = vec![f64::INFINITY; cells];
    let mut surface = vec![f64::NEG_INFINITY; cells];
    let mut hits = vec![0u32; cells];
    let mut through = vec![0u32; cells];
    let mut class_count = [0u64; 256];
    let mut digest = Sha256::new();
    let files = manifest["files"].as_array().expect("files");
    let mut raw: Vec<(f64, f64, f64, u8, bool)> = Vec::new();
    for f in files {
        let p = dir.join(f["file"].as_str().unwrap());
        let bytes = std::fs::read(&p).expect("laz file");
        let sum = hex(&Sha256::digest(&bytes));
        assert_eq!(sum, f["sha256"].as_str().unwrap(), "{} does not match its manifest digest", p.display());
        digest.update(sum.as_bytes());
        let mut r = Reader::from_path(&p).expect("laz reader");
        let pts = r.read_all().expect("laz points");
        let (xs, ys, zs) = (pts.x().collect::<Vec<_>>(), pts.y().collect::<Vec<_>>(), pts.z().collect::<Vec<_>>());
        let (cl, rn, nr) = (pts.classification().collect::<Vec<_>>(), pts.return_number().collect::<Vec<_>>(), pts.number_of_returns().collect::<Vec<_>>());
        for i in 0..xs.len() {
            class_count[cl[i] as usize] += 1;
            if cl[i] == 7 || cl[i] == 18 {
                continue;
            }
            let lon = (xs[i] / R_MERC).to_degrees();
            let lat = (2.0 * (ys[i] / R_MERC).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
            let (x, z) = frame.to_plan(lon, lat);
            raw.push((x, z, zs[i], cl[i], rn[i] < nr[i]));
        }
    }
    let cloud_digest = hex(&digest.finalize());
    println!("{} points read from {} files; classes {:?}", raw.len(), files.len(), class_count.iter().enumerate().filter(|c| *c.1 > 0).map(|(k, c)| (k, *c)).collect::<Vec<_>>());

    // ---- the plan shift between the cloud and the footprints
    let near: Vec<&eo_data::Building> = city
        .buildings
        .iter()
        .filter(|b| b.ring.iter().all(|p| (p[0] as f64 - cx).abs() < half - 4.0 && (p[1] as f64 - cz).abs() < half - 4.0))
        .collect();
    let rasterise = |shift: (f64, f64), ground: &mut Vec<f64>, surface: &mut Vec<f64>, hits: &mut Vec<u32>, through: &mut Vec<u32>| {
        ground.iter_mut().for_each(|v| *v = f64::INFINITY);
        surface.iter_mut().for_each(|v| *v = f64::NEG_INFINITY);
        hits.iter_mut().for_each(|v| *v = 0);
        through.iter_mut().for_each(|v| *v = 0);
        for &(x, z, y, c, thru) in &raw {
            let Some(k) = grid.idx(x + shift.0, z + shift.1) else { continue };
            if c == 2 {
                ground[k] = ground[k].min(y);
            }
            surface[k] = surface[k].max(y);
            hits[k] += 1;
            if thru {
                through[k] += 1;
            }
        }
    };
    // the mask: every footprint that meets the area (the roofs: only those wholly inside, `near`)
    let touching: Vec<&eo_data::Building> = city
        .buildings
        .iter()
        .filter(|b| b.ring.iter().any(|p| (p[0] as f64 - cx).abs() < half + 30.0 && (p[1] as f64 - cz).abs() < half + 30.0))
        .collect();
    let mut fp_mask = vec![false; cells];
    for b in &touching {
        let (mut bx0, mut bz0, mut bx1, mut bz1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &b.ring {
            bx0 = bx0.min(p[0] as f64);
            bz0 = bz0.min(p[1] as f64);
            bx1 = bx1.max(p[0] as f64);
            bz1 = bz1.max(p[1] as f64);
        }
        let (i0, j0) = (((bx0 - grid.x0) / CELL).floor().max(0.0) as usize, ((bz0 - grid.z0) / CELL).floor().max(0.0) as usize);
        let (i1, j1) = ((((bx1 - grid.x0) / CELL).ceil() as usize).min(n - 1), (((bz1 - grid.z0) / CELL).ceil() as usize).min(n - 1));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = j * n + i;
                let (x, z) = grid.centre(k);
                if point_in_ring(x, z, &b.ring) {
                    fp_mask[k] = true;
                }
            }
        }
    }
    // coarse pass: raised cells (surface over the area's low ground) against the mask, shifts in 0.5 m steps
    let mut best = (f64::MIN, (0.0, 0.0));
    for sj in -6..=6 {
        for si in -6..=6 {
            let s = (si as f64 * CELL, sj as f64 * CELL);
            rasterise(s, &mut ground, &mut surface, &mut hits, &mut through);
            // local ground: the lowest ground in a 15 m block
            let (mut inter, mut uni) = (0usize, 0usize);
            let b = 30;
            for bj in (0..n).step_by(b) {
                for bi in (0..n).step_by(b) {
                    let mut low = f64::INFINITY;
                    for j in bj..(bj + b).min(n) {
                        for i in bi..(bi + b).min(n) {
                            low = low.min(ground[j * n + i]);
                        }
                    }
                    if !low.is_finite() {
                        continue;
                    }
                    for j in bj..(bj + b).min(n) {
                        for i in bi..(bi + b).min(n) {
                            let k = j * n + i;
                            let raised = surface[k] - low > 4.0 && through[k] * 5 < hits[k].max(1);
                            if raised && fp_mask[k] {
                                inter += 1;
                            }
                            if raised || fp_mask[k] {
                                uni += 1;
                            }
                        }
                    }
                }
            }
            let iou = inter as f64 / uni.max(1) as f64;
            if iou > best.0 {
                best = (iou, s);
            }
        }
    }
    let shift = best.1;
    println!("plan shift cloud -> footprints: ({:+.1}, {:+.1}) m east/south, footprint match (IoU) {:.3}", shift.0, shift.1, best.0);
    rasterise(shift, &mut ground, &mut surface, &mut hits, &mut through);

    // ---- fill the ground's holes from the nearest measured ground (multi-source BFS)
    let mut filled = ground.clone();
    let mut queue: std::collections::VecDeque<usize> = (0..cells).filter(|&k| ground[k].is_finite()).collect();
    while let Some(k) = queue.pop_front() {
        let (i, j) = ((k % n) as i64, (k / n) as i64);
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                continue;
            }
            let q = b as usize * n + a as usize;
            if !filled[q].is_finite() {
                filled[q] = filled[k];
                queue.push_back(q);
            }
        }
    }
    let ground = filled;
    let ndsm: Vec<f64> = (0..cells).map(|k| if surface[k].is_finite() { (surface[k] - ground[k]).max(0.0) } else { 0.0 }).collect();

    // ---- vegetation
    let mut grown = fp_mask.clone();
    for k in 0..cells {
        if fp_mask[k] {
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            for dj in -2..=2i64 {
                for di in -2..=2i64 {
                    let (a, b) = (i + di, j + dj);
                    if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                        grown[b as usize * n + a as usize] = true;
                    }
                }
            }
        }
    }
    let leafy: Vec<bool> = (0..cells).map(|k| !grown[k] && ndsm[k] >= MIN_RAISED_M && hits[k] > 0 && through[k] as f64 >= VEG_SHARE * hits[k] as f64).collect();
    let veg: Vec<bool> = (0..cells)
        .map(|k| {
            if !leafy[k] {
                return false;
            }
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            let mut c = 0;
            for dj in -1..=1i64 {
                for di in -1..=1i64 {
                    let (a, b) = (i + di, j + dj);
                    if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 && leafy[b as usize * n + a as usize] {
                        c += 1;
                    }
                }
            }
            c >= 5
        })
        .collect();
    // smoothed canopy (5 x 5 mean over vegetation cells: one crown, one top)
    let chm: Vec<f64> = (0..cells)
        .map(|k| {
            if !veg[k] {
                return 0.0;
            }
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            let (mut s, mut c) = (0.0, 0.0);
            for dj in -2..=2i64 {
                for di in -2..=2i64 {
                    let (a, b) = (i + di, j + dj);
                    if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                        let q = b as usize * n + a as usize;
                        if veg[q] {
                            s += ndsm[q];
                            c += 1.0;
                        }
                    }
                }
            }
            s / c
        })
        .collect();

    // ---- tree tops: local maxima, window growing with height
    let mut tops: Vec<usize> = Vec::new();
    for k in 0..cells {
        let h = chm[k];
        if h < MIN_TREE_M {
            continue;
        }
        let r = (0.5 + 0.25 * h).clamp(1.5, 6.0);
        let rc = (r / CELL).ceil() as i64;
        let (i, j) = ((k % n) as i64, (k / n) as i64);
        let mut is_max = true;
        'w: for dj in -rc..=rc {
            for di in -rc..=rc {
                if (di * di + dj * dj) as f64 * CELL * CELL > r * r || (di == 0 && dj == 0) {
                    continue;
                }
                let (a, b) = (i + di, j + dj);
                if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                    continue;
                }
                let q = b as usize * n + a as usize;
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
    // ---- crowns grown downhill from the tops (priority flood, highest first)
    let mut label = vec![u32::MAX; cells];
    let mut heap = BinaryHeap::new();
    for (t, &k) in tops.iter().enumerate() {
        label[k] = t as u32;
        heap.push((Ord64((chm[k] * 1000.0) as i64), k));
    }
    while let Some((_, k)) = heap.pop() {
        let t = label[k] as usize;
        let top_h = chm[tops[t]];
        let (i, j) = ((k % n) as i64, (k / n) as i64);
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                continue;
            }
            let q = b as usize * n + a as usize;
            if label[q] != u32::MAX || !veg[q] || chm[q] < CROWN_BASE_FRACTION * top_h || chm[q] > chm[k] + 0.5 {
                continue;
            }
            label[q] = t as u32;
            heap.push((Ord64((chm[q] * 1000.0) as i64), q));
        }
    }
    let mut area = vec![0usize; tops.len()];
    for k in 0..cells {
        if label[k] != u32::MAX {
            area[label[k] as usize] += 1;
        }
    }
    let mut trees = Vec::new();
    for (t, &k) in tops.iter().enumerate() {
        let a = area[t] as f64 * CELL * CELL;
        if a < MIN_CROWN_M2 || ndsm[k] < MIN_TREE_M {
            continue;
        }
        let (x, z) = grid.centre(k);
        trees.push(serde_json::json!({
            "x": (x * 100.0).round() / 100.0, "z": (z * 100.0).round() / 100.0,
            "height_m": (ndsm[k] * 100.0).round() / 100.0,
            "crown_r_m": ((a / std::f64::consts::PI).sqrt() * 100.0).round() / 100.0,
            "ground_y": ((ground[k] + NAVD88_TO_GAME_Y) * 100.0).round() / 100.0,
        }));
    }
    let hs: Vec<f64> = trees.iter().map(|t| t["height_m"].as_f64().unwrap()).collect();
    let rs: Vec<f64> = trees.iter().map(|t| t["crown_r_m"].as_f64().unwrap()).collect();
    let canopy_cells = veg.iter().filter(|v| **v).count();
    println!(
        "{} trees (heights {:.1}-{:.1} m, median {:.1}; crown radius median {:.1} m); canopy {:.1} % of the area",
        trees.len(),
        hs.iter().cloned().fold(f64::MAX, f64::min),
        hs.iter().cloned().fold(f64::MIN, f64::max),
        percentile(&mut hs.clone(), 0.5),
        percentile(&mut rs.clone(), 0.5),
        100.0 * canopy_cells as f64 / cells as f64
    );

    // ---- built area the lidar sees against the footprints: solid raised cells
    // (>= 2.5 m, under 25 % of returns split by leaves, in 3 x 3 company) on and off the footprints
    {
        let solid: Vec<bool> = (0..cells).map(|k| ndsm[k] >= 2.5 && hits[k] > 0 && (through[k] as f64) < VEG_SHARE * hits[k] as f64).collect();
        let (mut on, mut off, mut fp_empty) = (0usize, 0usize, 0usize);
        for k in 0..cells {
            let (i, j) = ((k % n) as i64, (k / n) as i64);
            let near = |m: &Vec<bool>| {
                let mut c = 0;
                for dj in -1..=1i64 {
                    for di in -1..=1i64 {
                        let (a, b) = (i + di, j + dj);
                        if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 && m[b as usize * n + a as usize] {
                            c += 1;
                        }
                    }
                }
                c
            };
            if solid[k] && near(&solid) >= 7 {
                if grown[k] {
                    on += 1;
                } else {
                    off += 1;
                }
            }
            if fp_mask[k] && ndsm[k] < 1.0 {
                fp_empty += 1;
            }
        }
        let a = CELL * CELL;
        println!(
            "built area the lidar sees: {:.0} m^2 on the footprints, {:.0} m^2 off them ({:.1} % of the built area has no footprint); footprints over open ground: {:.0} m^2",
            on as f64 * a,
            off as f64 * a,
            100.0 * off as f64 / (on + off).max(1) as f64,
            fp_empty as f64 * a
        );
    }

    // ---- roofs
    let mut roofs = Vec::new();
    let mut form_of = vec![0u8; cells]; // 0 none, 1 flat, 2 gable, 3 complex, 4 shed, 5 stepped
    let mut counts = [0usize; 6];
    let mut height_err = Vec::new();
    let mut dump: Vec<serde_json::Value> = Vec::new(); // `EO_LIDAR_DUMP=1`: every roof's cells (a diagnostic)
    for b in &near {
        let mut inside: Vec<usize> = Vec::new();
        let mut rim: Vec<f64> = Vec::new();
        let (mut bx0, mut bz0, mut bx1, mut bz1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &b.ring {
            bx0 = bx0.min(p[0] as f64);
            bz0 = bz0.min(p[1] as f64);
            bx1 = bx1.max(p[0] as f64);
            bz1 = bz1.max(p[1] as f64);
        }
        let (i0, j0) = ((((bx0 - 3.0) - grid.x0) / CELL).floor().max(0.0) as usize, (((bz0 - 3.0) - grid.z0) / CELL).floor().max(0.0) as usize);
        let (i1, j1) = (((((bx1 + 3.0) - grid.x0) / CELL).ceil() as usize).min(n - 1), (((((bz1 + 3.0) - grid.z0) / CELL).ceil()) as usize).min(n - 1));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = j * n + i;
                let (x, z) = grid.centre(k);
                let d = dist_to_ring(x, z, &b.ring);
                if point_in_ring(x, z, &b.ring) {
                    // a roof cell: solid (under 25 % of its returns split by leaves)
                    if d >= ROOF_INSET_M && surface[k].is_finite() && (through[k] as f64) < VEG_SHARE * hits[k] as f64 {
                        inside.push(k);
                    }
                } else if d < 3.0 {
                    rim.push(ground[k]);
                }
            }
        }
        if inside.len() < 16 || rim.is_empty() {
            continue;
        }
        let base = percentile(&mut rim, 0.5);
        let mut tops_y: Vec<f64> = inside.iter().map(|&k| surface[k]).collect();
        let roof_top = percentile(&mut tops_y.clone(), 0.95) - base;
        let roof_low = percentile(&mut tops_y, 0.10) - base;
        let pts: Vec<(f64, f64, f64)> = inside.iter().map(|&k| {
            let (x, z) = grid.centre(k);
            (x, z, surface[k])
        }).collect();
        let (pb, pc, prms) = plane_fit_trimmed(&pts).unwrap_or((0.0, 0.0, f64::MAX));
        let slope = pb.hypot(pc).atan().to_degrees();
        let (ccx, ccz) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0 / pts.len() as f64, a.1 + p.1 / pts.len() as f64));
        // two planes meeting at a ridge, along either main axis
        let mut gable: Option<(f64, f64, f64, f64, f64)> = None; // (rms, axis angle deg, pitch deg, ridge offset, eave)
        for (ax, az) in main_axes(&b.ring) {
            // u along the ridge, v across it
            let vs: Vec<(f64, f64)> = pts.iter().map(|p| ((p.0 - ccx) * -az + (p.1 - ccz) * ax, p.2)).collect();
            let (vmin, vmax) = vs.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
            if vmax - vmin < 3.0 {
                continue;
            }
            let mut best_r: Option<(f64, f64, f64, f64)> = None;
            let mut r = vmin + 1.0;
            while r <= vmax - 1.0 {
                let left: Vec<(f64, f64)> = vs.iter().filter(|p| p.0 < r).cloned().collect();
                let right: Vec<(f64, f64)> = vs.iter().filter(|p| p.0 >= r).cloned().collect();
                if let (Some(l), Some(rr)) = (line_fit_trimmed(&left), line_fit_trimmed(&right)) {
                    let mut res: Vec<f64> = l.2.iter().chain(rr.2.iter()).cloned().collect();
                    let rms = robust(&mut res);
                    if best_r.is_none_or(|b| rms < b.0) {
                        best_r = Some((rms, r, l.1, rr.1));
                    }
                }
                r += 0.5;
            }
            if let Some((rms, r, sl, sr)) = best_r {
                let (pl, pr) = (sl.atan().to_degrees(), sr.atan().to_degrees());
                // toward each other: rising to the ridge from both sides
                let ok = pl > 12.0 && pl < 60.0 && pr < -12.0 && pr > -60.0;
                if ok && gable.is_none_or(|g| rms < g.0) {
                    let axis_deg = az.atan2(ax).to_degrees();
                    gable = Some((rms, axis_deg, (pl - pr) / 2.0, r, roof_low));
                }
            }
        }
        // flat levels: peaks of the roof heights in 0.5 m bins
        let levels = {
            let lo = pts.iter().map(|p| p.2).fold(f64::MAX, f64::min);
            let nb = ((pts.iter().map(|p| p.2).fold(f64::MIN, f64::max) - lo) / 0.5).floor() as usize + 1;
            let mut hist = vec![0usize; nb];
            for p in &pts {
                hist[((p.2 - lo) / 0.5).floor() as usize] += 1;
            }
            // smooth over three bins, then take peaks in order of size, 1.5 m apart
            let sm: Vec<usize> = (0..nb).map(|i| hist[i.saturating_sub(1)..(i + 2).min(nb)].iter().sum()).collect();
            let mut order: Vec<usize> = (0..nb).collect();
            order.sort_by(|a, b| sm[*b].cmp(&sm[*a]));
            let mut peaks: Vec<f64> = Vec::new();
            for i in order {
                if sm[i] * 100 < pts.len() * 12 || peaks.len() == 3 {
                    break;
                }
                let y = lo + (i as f64 + 0.5) * 0.5;
                if peaks.iter().all(|q| (q - y).abs() >= 1.5) {
                    peaks.push(y);
                }
            }
            peaks
        };
        let on_level = pts.iter().filter(|p| levels.iter().any(|q| (p.2 - q).abs() <= 0.4)).count();
        let stepped = levels.len() >= 2 && on_level * 100 >= pts.len() * 75;
        let flat = slope < 8.0 && prms < 0.5;
        let shed = (8.0..50.0).contains(&slope) && prms < 0.3;
        let (form, rec) = match gable {
            Some((rms, axis, pitch, ridge, eave)) if rms < 0.4 && rms <= prms * 0.7 => (
                2u8,
                serde_json::json!({ "form": "gable", "axis_deg": (axis * 10.0).round() / 10.0, "pitch_deg": (pitch * 10.0).round() / 10.0,
                    "ridge_offset_m": (ridge * 100.0).round() / 100.0, "eave_m": (eave * 100.0).round() / 100.0, "rms_m": (rms * 100.0).round() / 100.0 }),
            ),
            _ if flat => (1u8, serde_json::json!({ "form": "flat", "rms_m": (prms * 100.0).round() / 100.0 })),
            _ if stepped => {
                let mut ls: Vec<f64> = levels.iter().map(|q| ((q - base) * 100.0).round() / 100.0).collect();
                ls.sort_by(|a, b| b.total_cmp(a));
                (5u8, serde_json::json!({ "form": "stepped", "levels_m": ls, "on_level": (on_level as f64 / pts.len() as f64 * 100.0).round() / 100.0 }))
            }
            _ if shed => (
                4u8,
                serde_json::json!({ "form": "shed", "pitch_deg": (slope * 10.0).round() / 10.0,
                    "down_deg": (-pc).atan2(-pb).to_degrees().round(), "rms_m": (prms * 100.0).round() / 100.0 }),
            ),
            _ => (3u8, serde_json::json!({ "form": "complex", "plane_slope_deg": (slope * 10.0).round() / 10.0, "rms_m": (prms * 100.0).round() / 100.0 })),
        };
        if std::env::var_os("EO_LIDAR_DUMP").is_some() {
            dump.push(serde_json::json!({ "id": b.id, "form": form, "base": base, "cells": pts.iter().map(|p| [((p.0 - ccx) * 10.0).round() / 10.0, ((p.1 - ccz) * 10.0).round() / 10.0, ((p.2 - base) * 100.0).round() / 100.0]).collect::<Vec<_>>() }));
        }
        counts[form as usize] += 1;
        for &k in &inside {
            form_of[k] = form;
        }
        height_err.push(b.height as f64 - roof_top);
        let mut rec = rec;
        rec["id"] = serde_json::json!(b.id);
        rec["top_m"] = serde_json::json!((roof_top * 100.0).round() / 100.0);
        rec["footprint_height_m"] = serde_json::json!((b.height as f64 * 100.0).round() / 100.0);
        roofs.push(rec);
    }
    let abs_err: Vec<f64> = height_err.iter().map(|e| e.abs()).collect();
    println!(
        "{} roofs: {} flat, {} stepped, {} shed, {} gable, {} complex; footprint height minus lidar roof top: median {:+.1} m, median |error| {:.1} m",
        roofs.len(),
        counts[1],
        counts[5],
        counts[4],
        counts[2],
        counts[3],
        percentile(&mut height_err.clone(), 0.5),
        percentile(&mut abs_err.clone(), 0.5)
    );

    // the measured forms against the house type the kit draws for the same record
    // (the game's own classification; only stucco_row and cottage_garage draw pitched roofs)
    {
        let (_c, kc) = eo_kit::city::KitCity::load(&paths.repo_root).expect("kit city");
        let keyed: std::collections::HashMap<&str, usize> = kc.records.iter().enumerate().map(|(i, r)| (r.key.as_str(), i)).collect();
        let mut table: std::collections::BTreeMap<String, [usize; 6]> = Default::default();
        for r in &roofs {
            let id = r["id"].as_str().unwrap_or("");
            let ty = match keyed.get(id) {
                Some(&i) if kc.excluded.contains(&i) => "hand-built".to_string(),
                Some(&i) => eo_kit::pipeline::classify_loaded(&kc.records[i]).map(|c| c.archetype.to_string()).unwrap_or_else(|| "unclassified".into()),
                None => "not in the kit".to_string(),
            };
            let col = match r["form"].as_str() {
                Some("flat") => 0,
                Some("stepped") => 1,
                Some("shed") => 2,
                Some("gable") => 3,
                _ => 4,
            };
            table.entry(ty).or_default()[col] += 1;
        }
        println!("measured roof form by the kit's house type (flat / stepped / shed / gable / complex):");
        for (ty, c) in &table {
            println!("  {ty:<22} {:>3} {:>3} {:>3} {:>3} {:>3}", c[0], c[1], c[2], c[3], c[4]);
        }
    }
    if !dump.is_empty() {
        std::fs::write(dir.join("roof-cells.json"), serde_json::to_string(&dump).unwrap()).unwrap();
    }
    // ---- write
    let provenance = serde_json::json!({
        "source": manifest["source"], "url": manifest["url"], "license": manifest["license"], "attribution": manifest["attribution"],
        "cloud_sha256_of_file_digests": cloud_digest, "generator": "native/eo-kit/examples/lidar_survey.rs",
        "plan": "surveyed 1:1 plan metres (x east, z south), the city's own frame", "plan_shift_m": [shift.0, shift.1], "shift_iou": best.0,
        "cell_m": CELL,
    });
    let out_trees = serde_json::json!({ "provenance": provenance, "area": manifest["area"], "trees": trees });
    std::fs::write(dir.join("trees.json"), serde_json::to_string_pretty(&out_trees).unwrap()).unwrap();
    let out_roofs = serde_json::json!({ "provenance": provenance, "area": manifest["area"], "roofs": roofs });
    std::fs::write(dir.join("roofs.json"), serde_json::to_string_pretty(&out_roofs).unwrap()).unwrap();

    // ---- evidence: hillshaded surface, canopy green, crowns outlined, roof forms tinted, footprints
    let mut img = vec![0u8; cells * 3];
    for k in 0..cells {
        let (i, j) = (k % n, k / n);
        let s = |a: usize, b: usize| if surface[b * n + a].is_finite() { surface[b * n + a] } else { ground[b * n + a] };
        let (ia, ib) = (i.saturating_sub(1), (i + 1).min(n - 1));
        let (ja, jb) = (j.saturating_sub(1), (j + 1).min(n - 1));
        let dx = (s(ib, j) - s(ia, j)) / (2.0 * CELL);
        let dz = (s(i, jb) - s(i, ja)) / (2.0 * CELL);
        let shade = ((-dx * 0.6 - dz * 0.6 + 1.0) / (dx * dx + dz * dz + 1.0).sqrt()).clamp(0.0, 1.0);
        let g = 60.0 + 160.0 * shade;
        let mut c = [g, g, g];
        if veg[k] {
            c = [g * 0.35, g * 0.85, g * 0.35];
        }
        match form_of[k] {
            1 => c = [g * 0.85, g * 0.85, g * 0.95],
            2 => c = [g * 1.0, g * 0.55, g * 0.35],
            3 => c = [g * 0.7, g * 0.45, g * 0.9],
            4 => c = [g * 0.95, g * 0.85, g * 0.3],
            5 => c = [g * 0.45, g * 0.7, g * 1.0],
            _ => {}
        }
        if label[k] != u32::MAX {
            let l = label[k];
            let edge = [(1i64, 0i64), (0, 1)].iter().any(|(di, dj)| {
                let (a, b) = (i as i64 + di, j as i64 + dj);
                a < n as i64 && b < n as i64 && label[b as usize * n + a as usize] != l
            });
            if edge {
                c = [20.0, 70.0, 20.0];
            }
        }
        if fp_mask[k] && [(1i64, 0i64), (0, 1), (-1, 0), (0, -1)].iter().any(|(di, dj)| {
            let (a, b) = (i as i64 + di, j as i64 + dj);
            a >= 0 && b >= 0 && a < n as i64 && b < n as i64 && !fp_mask[b as usize * n + a as usize]
        }) {
            c = [255.0, 255.0, 255.0];
        }
        for q in 0..3 {
            img[k * 3 + q] = c[q].clamp(0.0, 255.0) as u8;
        }
    }
    for t in &trees {
        if let Some(k) = grid.idx(t["x"].as_f64().unwrap(), t["z"].as_f64().unwrap()) {
            for (di, dj) in [(0i64, 0i64), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (a, b) = ((k % n) as i64 + di, (k / n) as i64 + dj);
                if a >= 0 && b >= 0 && a < n as i64 && b < n as i64 {
                    let q = (b as usize * n + a as usize) * 3;
                    img[q..q + 3].copy_from_slice(&[255, 230, 0]);
                }
            }
        }
    }
    write_png(&dir.join("evidence-top-down.png"), n, n, &img);
    println!("wrote {} and {} and {}", dir.join("trees.json").display(), dir.join("roofs.json").display(), dir.join("evidence-top-down.png").display());
}
