//! `relief`: the ground's relief at 2 m from the USGS 3DEP 1 m DEM
//! (`eo_hills::relief`, stage H3) into `<cache>/hills/relief-v1.bin`.
//!
//! Inputs: the six 1 m GeoTIFF tiles in `EO_DEM1M_DIR`, else
//! `<repo>/external/dem/1m`, each checked against its size and SHA-256 in
//! the folder's `PROVENANCE.json` (any mismatch refuses the cook); a point
//! takes the first `primary` tile (`CA_SanFrancisco_B23`) with a height
//! there, else a `gap-fill` one (`CA_CaliforniaGaps_B23`). The tiles are
//! opened with `eo_farshore::survey::TiledRaster`, copied whole into memory
//! (2.4 GB) and sampled with its own bilinear (checked against
//! `TiledRaster::at` on 20 000 points a tile); plan metres to lon/lat by the
//! city's plan frame, to UTM 10N by `survey::dem1m::UTM10N_NAD83`. The 8 m
//! grid is the elevation grid's (`public/data/sf/sf-elevation.bin.gz`), the
//! footprints the city's (`eo_data::City` at 1:1).
//!
//! `--preview <dir>` writes `relief-bernal.png` and `relief-twinpeaks.png`:
//! a hillshade (sun from the north-west at 45 degrees, heights x 2) of the
//! drawn surface at 2 m over 1.5 km: the 8 m low-pass on its triangles plus
//! the kept blocks' relief. Blocks not kept are tinted blue (their 8 m
//! facets show), points inside footprints red-brown, clamped points red.

use anyhow::{Context, Result, bail};
use eo_data::roof_colours::PlanFrame;
use eo_farshore::survey::{TiledRaster, dem1m::UTM10N_NAD83};
use eo_hills::raster::{self, PlanGrid};
use eo_hills::relief::{self as rl, Block, BlockInfo, Lattice};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// One DEM tile, copied whole out of its `TiledRaster` (NaN where it has no data).
struct DenseTile {
    name: String,
    x0: f64,
    y0: f64,
    dx: f64,
    dy: f64,
    w: i64,
    h: i64,
    px: Vec<f32>,
}

impl DenseTile {
    fn load(t: &TiledRaster) -> DenseTile {
        let (w, h) = (t.width as i64, t.height as i64);
        let mut px = vec![f32::NAN; (w * h) as usize];
        for r in 0..h {
            for c in 0..w {
                if let Some(v) = t.pixel(c, r) {
                    px[(r * w + c) as usize] = v;
                }
            }
        }
        DenseTile { name: t.name.clone(), x0: t.x0, y0: t.y0, dx: t.dx, dy: t.dy, w, h, px }
    }
    fn pixel(&self, c: i64, r: i64) -> Option<f32> {
        if c < 0 || r < 0 || c >= self.w || r >= self.h {
            return None;
        }
        let v = self.px[(r * self.w + c) as usize];
        (!v.is_nan()).then_some(v)
    }
    /// `TiledRaster::at`: bilinear between pixel centres, None when any of the four is missing.
    fn at(&self, x: f64, y: f64) -> Option<f32> {
        let fc = (x - self.x0) / self.dx - 0.5;
        let fr = (self.y0 - y) / self.dy - 0.5;
        let (c, r) = (fc.floor() as i64, fr.floor() as i64);
        let (u, v) = ((fc - c as f64) as f32, (fr - r as f64) as f32);
        let p00 = self.pixel(c, r)?;
        let p10 = self.pixel(c + 1, r)?;
        let p01 = self.pixel(c, r + 1)?;
        let p11 = self.pixel(c + 1, r + 1)?;
        Some((p00 * (1.0 - u) + p10 * u) * (1.0 - v) + (p01 * (1.0 - u) + p11 * u) * v)
    }
}

/// The DEM: the tiles, primaries first.
pub(super) struct Dem {
    tiles: Vec<DenseTile>,
    frame: PlanFrame,
}

impl Dem {
    /// The NAVD88 height (m) at surveyed plan (x, z).
    pub(super) fn at_plan(&self, x: f64, z: f64) -> Option<f32> {
        let (lon, lat) = self.frame.to_lonlat(x, z);
        let (e, n) = UTM10N_NAD83.forward(lon, lat);
        self.tiles.iter().find_map(|t| t.at(e, n))
    }
}

fn dem_dir() -> Result<PathBuf> {
    if let Some(v) = std::env::var_os("EO_DEM1M_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(v));
    }
    Ok(eo_data::DataPaths::discover()?.repo_root.join(eo_data::external::DIR).join("dem").join("1m"))
}

/// The tiles PROVENANCE.json lists, each checked (size, SHA-256), primaries first.
fn verified_tiles(dir: &Path) -> Result<(Vec<rl::TileInput>, rl::Inputs)> {
    let p = dir.join("PROVENANCE.json");
    let bytes = std::fs::read(&p).with_context(|| format!("reading {} (set EO_DEM1M_DIR to the folder of the six 1 m tiles)", p.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", p.display()))?;
    let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
    let mut tiles: Vec<rl::TileInput> = v["files"]
        .as_array()
        .context("PROVENANCE.json: no files")?
        .iter()
        .map(|f| rl::TileInput { path: s(f, "path"), role: s(f, "role"), bytes: f["bytes"].as_u64().unwrap_or(0), sha256: s(f, "sha256").to_ascii_lowercase(), url: s(f, "url") })
        .collect();
    if tiles.is_empty() || tiles.iter().any(|t| !matches!(t.role.as_str(), "primary" | "gap-fill") || t.sha256.len() != 64) {
        bail!("{}: every file needs a role (primary, gap-fill) and a SHA-256", p.display());
    }
    let errors: Vec<String> = std::thread::scope(|sc| {
        let hs: Vec<_> = tiles
            .iter()
            .map(|t| {
                sc.spawn(move || -> Option<String> {
                    let fp = dir.join(&t.path);
                    let b = match std::fs::read(&fp) {
                        Ok(b) => b,
                        Err(e) => return Some(format!("{}: {e}", fp.display())),
                    };
                    let got = super::sha256_hex(&b);
                    if b.len() as u64 != t.bytes || got != t.sha256 {
                        return Some(format!("{}: {} bytes, SHA-256 {got}; PROVENANCE.json records {} bytes, {}", t.path, b.len(), t.bytes, t.sha256));
                    }
                    None
                })
            })
            .collect();
        hs.into_iter().filter_map(|h| h.join().unwrap()).collect()
    });
    if !errors.is_empty() {
        bail!("DEM tile(s) refused:\n  {}", errors.join("\n  "));
    }
    tiles.sort_by_key(|t| t.role != "primary");
    let inputs = rl::Inputs { dem_source: s(&v, "source"), dem_licence: s(&v, "licence"), dem_attribution: s(&v, "attribution"), dem_provenance_sha256: super::sha256_hex(&bytes), tiles: tiles.clone(), city_files: BTreeMap::new() };
    Ok((tiles, inputs))
}

/// The 1 m DEM in memory (every tile checked against PROVENANCE.json and its
/// copy against `TiledRaster::at`), the data paths and the city at 1:1.
pub(super) fn load_dem(t0: Instant) -> Result<(Dem, rl::Inputs, eo_data::DataPaths, eo_data::City)> {
    // 1. The tiles, each against PROVENANCE.json, copied into memory.
    let dir = dem_dir()?;
    let (tiles, inputs) = verified_tiles(&dir)?;
    eprintln!("{} DEM tiles verified in {} ({:.1} s)", tiles.len(), dir.display(), t0.elapsed().as_secs_f64());
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load_at(&paths, 1.0).context("the city at 1:1")?;
    let frame = PlanFrame::from_meta(&city.meta);
    let dense: Vec<DenseTile> = std::thread::scope(|sc| {
        let hs: Vec<_> = tiles
            .iter()
            .map(|t| {
                let p = dir.join(&t.path);
                sc.spawn(move || -> Result<DenseTile> {
                    let r = TiledRaster::open(&p)?;
                    if (r.dx - 1.0).abs() > 1e-6 || (r.dy - 1.0).abs() > 1e-6 {
                        bail!("{}: {} x {} m pixels, not 1 m", r.name, r.dx, r.dy);
                    }
                    let d = DenseTile::load(&r);
                    // the copy samples exactly as the raster does
                    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
                    let b = r.bounds();
                    for _ in 0..20_000 {
                        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        let (fx, fy) = ((seed >> 11) as f64 / (1u64 << 53) as f64, ((seed >> 3) & 0xffff_ffff) as f64 / 4294967296.0);
                        let (x, y) = (b[0] + fx * (b[2] - b[0]), b[1] + fy * (b[3] - b[1]));
                        if d.at(x, y).map(f32::to_bits) != r.at(x, y).map(f32::to_bits) {
                            bail!("{}: the in-memory copy samples ({x}, {y}) as {:?}, the raster as {:?}", r.name, d.at(x, y), r.at(x, y));
                        }
                    }
                    Ok(d)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect::<Result<Vec<_>>>()
    })?;
    for (t, d) in tiles.iter().zip(&dense) {
        eprintln!("  {} ({}): {} x {} px, {:.1}% with data", d.name, t.role, d.w, d.h, 100.0 * d.px.iter().filter(|v| !v.is_nan()).count() as f64 / d.px.len() as f64);
    }
    let dem = Dem { tiles: dense, frame };
    eprintln!("DEM in memory, checked against TiledRaster::at ({:.1} s)", t0.elapsed().as_secs_f64());
    Ok((dem, inputs, paths, city))
}

/// One block's outcome.
struct Outcome {
    key: (i64, i64),
    land: bool,
    kept: bool,
    info: BlockInfo,
    cm: Vec<i16>,
    raw_max: f64,
    raw_max_open: f64,
}

pub fn run(args: &[String]) -> Result<()> {
    let t0 = Instant::now();
    let mut preview: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--preview" => preview = Some(PathBuf::from(it.next().context("--preview needs a folder")?)),
            _ => bail!("usage: cook relief [--preview <dir>]   (EO_DEM1M_DIR: the 1 m DEM tiles; EO_NATIVE_CACHE: where the relief goes)"),
        }
    }
    // 1. The tiles, each against PROVENANCE.json, copied into memory.
    let (dem, mut inputs, paths, city) = load_dem(t0)?;
    let frame = PlanFrame::from_meta(&city.meta);
    let sample = |x: f64, z: f64| dem.at_plan(x, z);

    // 2. The 8 m grid (the elevation grid's cell centres) and its low-pass.
    let elev_path = paths.sf("sf-elevation.bin.gz");
    let elev = eo_data::ElevationGrid::decode(&eo_data::read_maybe_gzip(&elev_path)?)?;
    let m = &elev.meta;
    if (m.cell_size - rl::CELL_M).abs() > 1e-9 {
        bail!("sf-elevation: {} m cells, expected {}", m.cell_size, rl::CELL_M);
    }
    let l = Lattice { ox: m.origin_x + 0.5 * m.cell_size, oz: m.origin_z + 0.5 * m.cell_size, cell: m.cell_size, width: m.width as i64, height: m.height as i64 };
    let lp = rl::low_pass_grid(&l, &sample);
    eprintln!("low-pass at {} x {} vertices ({:.1} s)", l.width + 1, l.height + 1, t0.elapsed().as_secs_f64());

    // 3. The footprints on the 2 m points.
    let step = l.step();
    let fg = PlanGrid { origin_x: l.ox - step / 2.0, origin_z: l.oz - step / 2.0, texel: step, width: ((l.width - 1) * 4 + 1) as usize, height: ((l.height - 1) * 4 + 1) as usize };
    let mut built = vec![false; fg.width * fg.height];
    for b in &city.buildings {
        let ring: Vec<[f64; 2]> = b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        if ring.len() >= 3 {
            raster::fill_rings(&fg, &[&ring], |i, j| built[j * fg.width + i] = true);
        }
    }

    // 4. Every block.
    let keys = l.blocks();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let n = (rl::PATCH_CELLS * rl::STEPS) as i64;
    let mut outcomes: Vec<Outcome> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..eo_hills::threads())
            .map(|_| {
                let (next, keys, lp, built, sample) = (&next, &keys, &lp, &built, &sample);
                sc.spawn(move || {
                    let mut out = vec![];
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&key) = keys.get(k) else { break };
                        let r = rl::block_relief(&l, lp, sample, key);
                        let (gx0, gz0) = (key.0 * n, key.1 * n);
                        let (mut land, mut inside, mut clamped) = (false, 0u32, 0u32);
                        let (mut raw_max, mut raw_max_open) = (0f64, 0f64);
                        let mut cm = vec![0i16; rl::POINTS];
                        for (p, v) in r.iter().enumerate() {
                            let Some(v) = *v else { continue };
                            land = true;
                            let (gx, gz) = (gx0 + (p % rl::SIDE) as i64, gz0 + (p / rl::SIDE) as i64);
                            let b = built[gz as usize * fg.width + gx as usize];
                            if v.abs() > rl::MAX_RELIEF_M {
                                clamped += 1;
                            }
                            raw_max = raw_max.max(v.abs());
                            if !b {
                                raw_max_open = raw_max_open.max(v.abs());
                            }
                            cm[p] = rl::to_cm(v);
                        }
                        for p in 0..rl::POINTS {
                            let (gx, gz) = (gx0 + (p % rl::SIDE) as i64, gz0 + (p / rl::SIDE) as i64);
                            inside += built[gz as usize * fg.width + gx as usize] as u32;
                        }
                        let info = BlockInfo { max_abs_m: raw_max.min(rl::MAX_RELIEF_M) as f32, built_share: inside as f32 / rl::POINTS as f32, clamped };
                        let kept = land && raw_max_open > rl::KEEP_M;
                        out.push(Outcome { key, land, kept, info, cm: if kept { cm } else { vec![] }, raw_max, raw_max_open });
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    outcomes.sort_by_key(|o| (o.key.1, o.key.0));
    let land = outcomes.iter().filter(|o| o.land).count();
    let kept: Vec<&Outcome> = outcomes.iter().filter(|o| o.kept).collect();
    let stats = rl::Stats {
        candidate_blocks: outcomes.len() as u64,
        land_blocks: land as u64,
        kept_blocks: kept.len() as u64,
        kept_mostly_built: kept.iter().filter(|o| o.info.built_share > 0.5).count() as u64,
        clamped_points: kept.iter().map(|o| o.info.clamped as u64).sum(),
        max_abs_m: outcomes.iter().map(|o| o.raw_max).fold(0.0, f64::max) as f32,
        max_open_abs_m: outcomes.iter().map(|o| o.raw_max_open).fold(0.0, f64::max) as f32,
        seconds: 0.0,
    };
    eprintln!(
        "blocks: {} in the grid, {} on land, {} kept ({} mostly inside footprints); {} points clamped; max |relief| {:.2} m ({:.2} m outside footprints) ({:.1} s)",
        stats.candidate_blocks,
        stats.land_blocks,
        stats.kept_blocks,
        stats.kept_mostly_built,
        stats.clamped_points,
        stats.max_abs_m,
        stats.max_open_abs_m,
        t0.elapsed().as_secs_f64()
    );
    let centre = |key: (i64, i64)| (l.ox + (key.0 * 8 + 4) as f64 * l.cell, l.oz + (key.1 * 8 + 4) as f64 * l.cell);
    let mut by_clamped: Vec<&&Outcome> = kept.iter().filter(|o| o.info.clamped > 0).collect();
    by_clamped.sort_by_key(|o| std::cmp::Reverse(o.info.clamped));
    eprintln!("the 20 blocks with the most clamped points (walls, building edges, decks in the bare-earth model):");
    for o in by_clamped.iter().take(20) {
        let (x, z) = centre(o.key);
        let (lon, lat) = frame.to_lonlat(x, z);
        eprintln!("  block ({:4}, {:4}) centre ({x:7.0}, {z:7.0}) = {lat:.5}, {lon:.5}: {:4} clamped, max {:.1} m before the clamp, {:3.0}% in footprints", o.key.0, o.key.1, o.info.clamped, o.raw_max, 100.0 * o.info.built_share);
    }

    // 5. The file.
    for name in ["sf-city.json.gz", "sf-footprints.json.gz", "sf-footprint-supplement.json", "sf-elevation.bin.gz"] {
        if let Ok(b) = std::fs::read(paths.sf(name)) {
            inputs.city_files.insert(name.to_string(), super::sha256_hex(&b));
        }
    }
    let header = rl::Header {
        format: rl::FORMAT.into(),
        origin_x: l.ox,
        origin_z: l.oz,
        cell_m: l.cell,
        steps: rl::STEPS,
        patch_cells: rl::PATCH_CELLS,
        grid_width: l.width as u32,
        grid_height: l.height as u32,
        definition: "relief = the DEM's 2 m mean (4 taps 0.5 m off the point) minus the 8 m low-pass (the DEM averaged over the 8 m cell round each grid vertex, 64 taps 1 m apart) drawn on the cell's two triangles (diagonal from (1, 0) to (0, 1)); 0 where the DEM or a corner's low-pass is missing; clamped to +-clamp_m (eo-render terrain_detail.rs)".into(),
        datum: "a difference of NAVD88 heights, metres on the surveyed plan (1:1); the renderer multiplies it by its height scale and adds it to the drawn 8 m surface".into(),
        keep_m: rl::KEEP_M,
        clamp_m: rl::MAX_RELIEF_M,
        blocks: 0,
        stats: rl::Stats { seconds: t0.elapsed().as_secs_f32(), ..stats },
        inputs,
        generator: rl::GENERATOR.into(),
    };
    let blocks: Vec<Block> = kept.iter().map(|o| Block { key: o.key, info: o.info, cm: o.cm.clone() }).collect();
    let out = rl::file_path(&eo_hills::cache_dir());
    rl::write(&out, &header, &blocks)?;
    let back = rl::Relief::open(&out)?;
    if back.keys().count() != blocks.len() || blocks.iter().any(|b| back.block(b.key.0, b.key.1) != Some(&b.cm[..])) {
        bail!("{}: does not read back as written", out.display());
    }
    eprintln!("wrote {} ({:.1} MB, {} blocks; {:.1} s)", out.display(), std::fs::metadata(&out)?.len() as f64 / 1e6, blocks.len(), t0.elapsed().as_secs_f64());

    // 6. The previews.
    if let Some(dir) = preview {
        std::fs::create_dir_all(&dir)?;
        for (name, cx, cz) in [("bernal", 431.0, 3848.0), ("twinpeaks", -2490.0, 2700.0)] {
            let p = dir.join(format!("relief-{name}.png"));
            hillshade(&p, &l, &lp, &back, &built, &fg, cx, cz, 1500.0)?;
            eprintln!("preview {}", p.display());
        }
    }
    eprintln!("done in {:.1} s", t0.elapsed().as_secs_f64());
    Ok(())
}

/// The relief (cm) the file holds at global point (gx, gz), from any kept block holding it.
fn relief_cm(r: &rl::Relief, gx: i64, gz: i64) -> Option<i16> {
    let n = (rl::PATCH_CELLS * rl::STEPS) as i64;
    let (px, pz) = (gx.div_euclid(n), gz.div_euclid(n));
    for (dx, dz) in [(0, 0), (-1, 0), (0, -1), (-1, -1)] {
        let (bx, bz) = (px + dx, pz + dz);
        let (i, j) = (gx - bx * n, gz - bz * n);
        if (0..=n).contains(&i) && (0..=n).contains(&j) {
            if let Some(b) = r.block(bx, bz) {
                return Some(b[j as usize * rl::SIDE + i as usize]);
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn hillshade(path: &Path, l: &Lattice, lp: &[f32], r: &rl::Relief, built: &[bool], fg: &PlanGrid, cx: f64, cz: f64, span: f64) -> Result<()> {
    const EXAGGERATION: f64 = 2.0;
    let step = l.step();
    let size = (span / step) as i64;
    let (g0x, g0z) = (((cx - span / 2.0 - l.ox) / step).round() as i64, ((cz - span / 2.0 - l.oz) / step).round() as i64);
    let s = rl::STEPS as i64;
    let w = l.width + 1;
    let surface = |gx: i64, gz: i64| -> Option<f64> {
        let (qx, qz) = (gx.div_euclid(s), gz.div_euclid(s));
        if qx < 0 || qz < 0 || qx + 1 > l.width || qz + 1 > l.height {
            return None;
        }
        let at = |a: i64, b: i64| lp[(b * w + a) as usize];
        let (u, v) = (gx.rem_euclid(s) as f32 / s as f32, gz.rem_euclid(s) as f32 / s as f32);
        let base = rl::cell_tri(at(qx, qz), at(qx + 1, qz), at(qx, qz + 1), at(qx + 1, qz + 1), u, v) as f64;
        if base.is_nan() {
            return None;
        }
        Some(base + relief_cm(r, gx, gz).unwrap_or(0) as f64 / 100.0)
    };
    let sun = {
        let v = [-1.0f64, 2f64.sqrt(), -1.0];
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        v.map(|c| c / n)
    };
    let mut rgb = vec![0u8; (size * size * 3) as usize];
    for j in 0..size {
        for i in 0..size {
            let (gx, gz) = (g0x + i, g0z + j);
            let (Some(e), Some(wv), Some(sv), Some(nv)) = (surface(gx + 1, gz), surface(gx - 1, gz), surface(gx, gz + 1), surface(gx, gz - 1)) else { continue };
            let (dx, dz) = ((e - wv) * EXAGGERATION / (2.0 * step), (sv - nv) * EXAGGERATION / (2.0 * step));
            let nl = (dx * dx + 1.0 + dz * dz).sqrt();
            let shade = ((-dx * sun[0] + sun[1] - dz * sun[2]) / nl).clamp(0.0, 1.0);
            let grey = 30.0 + 225.0 * shade;
            let mut c = [grey; 3];
            let n = (rl::PATCH_CELLS * rl::STEPS) as i64;
            let kept = r.block(gx.div_euclid(n), gz.div_euclid(n)).is_some();
            let inside = gx >= 0 && gz >= 0 && (gx as usize) < fg.width && (gz as usize) < fg.height && built[gz as usize * fg.width + gx as usize];
            if !kept {
                c = [c[0] * 0.7, c[1] * 0.8, c[2] * 1.0];
            }
            if inside {
                c = [c[0] * 0.9, c[1] * 0.65, c[2] * 0.55];
            }
            if relief_cm(r, gx, gz).is_some_and(|v| v.unsigned_abs() >= (rl::MAX_RELIEF_M * 100.0) as u16) {
                c = [230.0, 30.0, 30.0];
            }
            let k = ((j * size + i) * 3) as usize;
            for q in 0..3 {
                rgb[k + q] = c[q].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    super::write_png(path, size as usize, size as usize, &rgb)
}
