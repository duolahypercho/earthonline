//! The hill builder's cooks (Docs/HILLS-PLAN.md).
//!
//! ```text
//! EO_AERIAL_DIR=<NAIP folder> cargo run --release -p eo-hills --example cook -- macro \
//!     [--preview out.png] [--probe name,x,z ...] [--colour classes|matrix] [--ridge r]
//! ```
//!
//! `relief`: the ground's relief at 2 m (see `cook/relief.rs`).
//!
//! `decks`: the roof decks the bare-earth DEM keeps as ground, brought down to
//! the streets' grade (see `cook/decks.rs`).
//!
//! `planting`: the vegetation planting's NAIP rules counted per 2 m texel
//! citywide (see `cook/planting.rs`).
//!
//! `trees`: the hill parks' trees measured from the USGS 3DEP lidar (see
//! `cook/trees.rs`).
//!
//! `macro`: the aerial macro colour map (`eo_hills::macro_map`, stage H2)
//! into `<cache>/hills/aerial-macro-v1.bin`, `<cache>` being
//! `EO_NATIVE_CACHE`, else `native/cache`. Inputs: the USDA NAIP 0.3 m RGB
//! and 0.6 m colour-infrared tiles in `EO_AERIAL_DIR` (public domain;
//! fetched by `native/tools/fetch-aerial.mjs` on the grid of
//! `eo-kit/examples/roof_survey.rs`, each checked against its SHA-256 in the
//! folder's `manifest.json`), and from `public/data/sf/` the footprints
//! (the closed texels), the roads and paths, the mapped greens, WorldCover
//! and the elevation grid.
//!
//! The photo's colours are carried to the eye's by `--colour classes` (the
//! default; `eo_hills::transfer`) or `--colour matrix` (a fitted 3 x 3
//! matrix, `--ridge r`). Both aim at the renderer's own ground colours, read
//! from its sources at cook time: the lawn hue (`eo-landmarks/src/ground_cover.rs`
//! "grass") at the measured turf luminance (`NT_TURF_LUM`,
//! `eo-render/src/landmarks_surface.wgsl`), dry grass, scrub and canopy
//! (`eo-render/src/ground2.rs` `DRY_A`, `DRY_B`, `SCRUB`, `GROVE`; the
//! matrix's canopy `CANOPY`), soil (`eo-render/src/terrain_detail.rs`
//! `SOIL_SURFACE_SRGB`, `SOIL_ERODED_SRGB`).
//!
//! `--preview` writes level 2 (8 m a pixel) as a PNG, closed texels black;
//! `--probe name,x,z` prints the preview's 3 x 3 pixel mean (sRGB) at a
//! surveyed plan point and what the texel there was read as.

use anyhow::{Context, Result, bail};
use eo_data::roof_colours::{PlanFrame, linear_to_srgb, srgb_to_linear};
use eo_hills::macro_map::{self as mm, ClassFit, ColourTransfer, Image, TEXEL_M, TransferClass};
use eo_hills::naip::{self, TILE, TileEntry};
use eo_hills::raster::{self, PlanGrid};
use eo_hills::transfer::{self as tr, Features};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[path = "cook/relief.rs"]
mod relief; // `relief`: the 2 m relief from the 1 m DEM (stage H3)
#[path = "cook/decks.rs"]
mod decks; // `decks`: structures the bare-earth DEM keeps as ground (stage H4)
#[path = "cook/planting.rs"]
mod planting; // `planting`: the vegetation planting's NAIP rules counted citywide (stage H6a)
#[path = "cook/trees.rs"]
mod trees; // `trees`: the hill parks' trees from the USGS 3DEP lidar (stage H6c)

struct Probe {
    name: String,
    x: f64,
    z: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Classes,
    Matrix,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || -> ! {
        eprintln!("usage: cook macro [--preview out.png] [--probe name,x,z ...] [--colour classes|matrix] [--ridge r]   (EO_AERIAL_DIR: the NAIP folder; EO_NATIVE_CACHE: where the map goes)");
        eprintln!("       cook relief [--preview <dir>]   (EO_DEM1M_DIR: the 1 m DEM tiles, default <repo>/external/dem/1m)");
        eprintln!("       cook decks [--survey] [--probe x0,z0,x1,z1[,step] [--dump <dir>]]   (EO_DEM1M_DIR: the 1 m DEM tiles)");
        eprintln!("       cook planting [--samples n]   (EO_AERIAL_DIR: the NAIP folder; EO_SURVEY_ROOT: the survey, to validate)");
        eprintln!("       cook trees [--area name ...] [--preview <dir>]   (EO_LIDAR_DIR: the lidar areas, default <repo>/external/lidar)");
        std::process::exit(2)
    };
    let sub: Option<fn(&[String]) -> Result<()>> = match args.first().map(String::as_str) {
        Some("relief") => Some(relief::run),
        Some("planting") => Some(planting::run),
        Some("decks") => Some(decks::run),
        Some("trees") => Some(trees::run),
        _ => None,
    };
    if let Some(run) = sub {
        if let Err(e) = run(&args[1..]) {
            eprintln!("cook: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().map(String::as_str) != Some("macro") {
        usage();
    }
    let (mut preview, mut probes, mut ridge, mut mode) = (None, vec![], mm::COLOUR_RIDGE, Mode::Classes);
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--preview" => preview = Some(PathBuf::from(it.next().unwrap_or_else(|| usage()))),
            "--ridge" => ridge = it.next().and_then(|s| s.parse().ok()).filter(|r: &f32| *r >= 0.0).unwrap_or_else(|| usage()),
            "--colour" => {
                mode = match it.next().map(String::as_str) {
                    Some("classes") => Mode::Classes,
                    Some("matrix") => Mode::Matrix,
                    _ => usage(),
                }
            }
            "--probe" => {
                let v: Vec<&str> = it.next().unwrap_or_else(|| usage()).split(',').collect();
                match (v.len(), v.get(1).and_then(|s| s.parse().ok()), v.get(2).and_then(|s| s.parse().ok())) {
                    (3, Some(x), Some(z)) => probes.push(Probe { name: v[0].to_string(), x, z }),
                    _ => usage(),
                }
            }
            _ => usage(),
        }
    }
    if let Err(e) = cook_macro(preview.as_deref(), &probes, ridge, mode) {
        eprintln!("cook: {e:#}");
        std::process::exit(1);
    }
}

/// The macro map's texel grid: the NAIP grid's box on the plan, rounded out
/// to whole texels, the size to a multiple of 64 texels (exact halves for six mips).
fn macro_grid(g: &naip::Grid, frame: &PlanFrame) -> PlanGrid {
    let bb = g.bbox();
    let (xw, zn) = frame.to_plan(bb[0], bb[3]);
    let (xe, zs) = frame.to_plan(bb[2], bb[1]);
    let origin_x = (xw / TEXEL_M).floor() * TEXEL_M;
    let origin_z = (zn / TEXEL_M).floor() * TEXEL_M;
    let w = (((xe - origin_x) / TEXEL_M).ceil() as usize).div_ceil(64) * 64;
    let h = (((zs - origin_z) / TEXEL_M).ceil() as usize).div_ceil(64) * 64;
    PlanGrid { origin_x, origin_z, texel: TEXEL_M, width: w, height: h }
}

fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn to_srgb8(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (linear_to_srgb(v) * 255.0).round() as u8)
}

fn hex_linear(h: &str) -> Result<[f32; 3]> {
    let n = u32::from_str_radix(h.trim_start_matches('#'), 16).with_context(|| format!("colour {h}"))?;
    Ok([(n >> 16) & 255, (n >> 8) & 255, n & 255].map(|v| srgb_to_linear(v as f32 / 255.0)))
}

fn scale_to_lum(c: [f32; 3], lum: f32) -> [f32; 3] {
    let l = mm::luminance(c);
    c.map(|v| v * lum / l)
}

/// A canopy's visible albedo is about 0.03-0.06 (the matrix's canopy target).
const CANOPY_LUM: f32 = 0.045;
/// Dry grass's luminance in the transfer: the renderer's own (`DRY_A`/`DRY_B`
/// mean, about 0.29) unless that is more than `DRY_BRIGHT` times what the
/// photo implies (the dry class's median luminance at paving's gain), then
/// `DRY_LUM_LOW` (the lower end of a cured annual grass's albedo, 0.2-0.3).
const DRY_BRIGHT: f32 = 1.25;
const DRY_LUM_LOW: f32 = 0.24;
/// Dry grass's palette luminance (linear): a cured annual grass canopy's visible albedo
/// is about 0.2-0.25 (INFERRED from the photo's own dry median at paving's gain, 0.250,
/// and the meadow's blades); the renderer's meadow and DRY tones sit at 0.17 and 0.29.
const DRY_LUM: f32 = 0.22;
/// The roads' median colour the paving gain keeps (sRGB): what the matrix
/// cook gave them, accepted.
const ROAD_SRGB: [u8; 3] = [142, 143, 140];

/// The renderer's own ground colours, read from its sources (one source of truth), all linear.
struct Tones {
    lawn_lum: f32,
    /// The surface car parks' tone (ground_cover.rs "lotAsphalt" pair, the linear mean).
    lot: [f32; 3],
    palette: tr::Palette,
    /// The matrix's targets: dry grass at its own luminance, canopy (`CANOPY`) at `CANOPY_LUM`.
    matrix_dry: [f32; 3],
    matrix_canopy: [f32; 3],
    source: String,
}

fn renderer_tones() -> Result<Tones> {
    let read = |rel: &str| -> Result<String> {
        let p = eo_data::native_dir().join(rel);
        std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))
    };
    let line_of = |src: &str, rel: &str, key: &str| -> Result<String> { src.lines().find(|l| l.contains(key)).map(str::to_string).with_context(|| format!("{rel}: no `{key}`")) };
    // NT_TURF_LUM: vec2<f32>(lo, hi): the lawn's luminance, its midpoint.
    let wgsl = "eo-render/src/landmarks_surface.wgsl";
    let line = line_of(&read(wgsl)?, wgsl, "const NT_TURF_LUM")?;
    let inner = line.rsplit_once('(').and_then(|(_, r)| r.split_once(')')).map(|(a, _)| a).with_context(|| format!("NT_TURF_LUM: {line}"))?;
    let v: Vec<f32> = inner.split(',').map(|s| s.trim().parse::<f32>()).collect::<Result<_, _>>().with_context(|| format!("NT_TURF_LUM: {line}"))?;
    if v.len() != 2 || !(v[0] > 0.0 && v[1] > v[0]) {
        bail!("NT_TURF_LUM: {line}");
    }
    let lawn_lum = (v[0] + v[1]) * 0.5;
    // ground_cover.rs: "grass" => ("#56763f", "#66814b"): the lawn's hue, the pair's linear mean.
    let gc = "eo-landmarks/src/ground_cover.rs";
    let line = line_of(&read(gc)?, gc, "\"grass\" => (\"#")?;
    let hexes: Vec<&str> = line.split('"').filter(|s| s.starts_with('#')).collect();
    if hexes.len() != 2 {
        bail!("{gc}: {line}");
    }
    let (a, b) = (hex_linear(hexes[0])?, hex_linear(hexes[1])?);
    let lawn = scale_to_lum([0, 1, 2].map(|k| (a[k] + b[k]) * 0.5), lawn_lum);
    // ground_cover.rs: "lotAsphalt" => (...): the surface car parks' tone, the pair's linear mean.
    let line = line_of(&read(gc)?, gc, "\"lotAsphalt\" => (\"#")?;
    let lh: Vec<&str> = line.split('"').filter(|s| s.starts_with('#')).collect();
    if lh.len() != 2 {
        bail!("{gc}: {line}");
    }
    let (la, lb) = (hex_linear(lh[0])?, hex_linear(lh[1])?);
    let lot = [0, 1, 2].map(|k| (la[k] + lb[k]) * 0.5);
    // const NAME: [u8; 3] = [r, g, b];
    let srgb_const = |rel: &str, src: &str, name: &str| -> Result<[f32; 3]> {
        let line = line_of(src, rel, &format!("const {name}: [u8; 3] ="))?;
        let n: Vec<f32> = line.split_once('=').map(|(_, r)| r).unwrap_or_default().split(|ch: char| !ch.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        if n.len() != 3 {
            bail!("{rel}: {line}");
        }
        Ok([n[0], n[1], n[2]].map(|v| srgb_to_linear(v / 255.0)))
    };
    let g2 = "eo-render/src/ground2.rs";
    let src = read(g2)?;
    // dry grass: the hue of the meadow's cured blades (eo-landmarks vegetation.rs
    // MEADOW_GOLDEN, linear: 1 : 0.77 : 0.43, as a cured grass canopy's reflectance
    // rises from blue to red) at DRY_LUM, not ground2.rs's DRY_A/DRY_B, whose green
    // equals 0.91 of their red and which read grey-blue under the sky from the air.
    let veg = "eo-landmarks/src/vegetation.rs";
    let vsrc = read(veg)?;
    let golden = {
        let line = line_of(&vsrc, veg, "pub const MEADOW_GOLDEN: [f32; 3] =")?;
        let n: Vec<f32> = line.split_once('=').map(|(_, r)| r).unwrap_or_default().split(|ch: char| !(ch.is_ascii_digit() || ch == '.')).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        if n.len() != 3 {
            bail!("{veg}: {line}");
        }
        [n[0], n[1], n[2]]
    };
    let dry = scale_to_lum(golden, DRY_LUM);
    let td = "eo-render/src/terrain_detail.rs";
    let tsrc = read(td)?;
    let palette = tr::Palette {
        lawn,
        dry,
        soil: srgb_const(td, &tsrc, "SOIL_SURFACE_SRGB")?,
        soil_eroded: srgb_const(td, &tsrc, "SOIL_ERODED_SRGB")?,
        scrub: srgb_const(g2, &src, "SCRUB")?,
        canopy: srgb_const(g2, &src, "GROVE")?,
    };
    let source = format!(
        "lawn: the linear mean of {gc} \"grass\" ({}, {}) at the midpoint of NT_TURF_LUM ({}, {}), native/{wgsl}; dry grass: {veg} MEADOW_GOLDEN's hue at DRY_LUM (luminance {:.3}), or {DRY_LUM_LOW} if that is over {DRY_BRIGHT} x the photo's dry median at paving's gain (classes; `colour_transfer.classes` records which); scrub {g2} SCRUB; canopy {g2} GROVE (classes) or CANOPY at {CANOPY_LUM} (matrix); soil {td} SOIL_SURFACE_SRGB, SOIL_ERODED_SRGB on slopes; paving and roads: the texel's own colour, 85 % grey, the roads' median at sRGB {ROAD_SRGB:?}",
        hexes[0],
        hexes[1],
        v[0],
        v[1],
        mm::luminance(dry)
    );
    Ok(Tones { lawn_lum, lot, palette, matrix_dry: dry, matrix_canopy: scale_to_lum(srgb_const(g2, &src, "CANOPY")?, CANOPY_LUM), source })
}

/// The tiles of `list` that are on disk, each checked against its digest
/// (any mismatch refuses the cook); the count missing.
fn verify_tiles(dir: &Path, list: &[TileEntry], set: &str) -> Result<(Vec<TileEntry>, u32)> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results: Vec<(usize, Result<bool, String>)> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..eo_hills::threads())
            .map(|_| {
                let next = &next;
                s.spawn(move || {
                    let mut v = vec![];
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(t) = list.get(k) else { break };
                        let p = dir.join(&t.file);
                        let r = match std::fs::read(&p) {
                            Err(_) if !p.exists() => Ok(false),
                            Err(e) => Err(format!("{}: {e}", t.file)),
                            Ok(b) => {
                                let got = sha256_hex(&b);
                                if got == t.sha256 { Ok(true) } else { Err(format!("{}: SHA-256 {got}, the manifest records {}", t.file, t.sha256)) }
                            }
                        };
                        v.push((k, r));
                    }
                    v
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    results.sort_by_key(|r| r.0);
    let (mut ok, mut missing, mut bad) = (vec![], 0u32, vec![]);
    for (k, r) in results {
        match r {
            Ok(true) => ok.push(list[k].clone()),
            Ok(false) => {
                eprintln!("{}: listed in the manifest, not on disk: skipped", list[k].file);
                missing += 1;
            }
            Err(e) => bad.push(e),
        }
    }
    if !bad.is_empty() {
        bail!("{} tile(s) refused:\n  {}", bad.len(), bad.join("\n  "));
    }
    // Files the manifest does not list have no recorded digest: never read.
    let listed: std::collections::HashSet<&str> = list.iter().map(|t| t.file.as_str()).collect();
    let unlisted = std::fs::read_dir(dir.join(set)).map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "jpg")).filter(|e| !listed.contains(format!("{set}/{}", e.file_name().to_string_lossy()).as_str())).count()).unwrap_or(0);
    if unlisted > 0 {
        eprintln!("{unlisted} file(s) in {set}/ the manifest does not list: not read");
    }
    Ok((ok, missing))
}

fn decode_jpeg(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut d = jpeg_decoder::Decoder::new(&bytes[..]);
    let px = d.decode().with_context(|| format!("decoding {}", path.display()))?;
    let info = d.info().context("no JPEG info")?;
    if info.pixel_format != jpeg_decoder::PixelFormat::RGB24 || info.width as usize != TILE || info.height as usize != TILE {
        bail!("{}: {:?} {} x {}, expected RGB {TILE} x {TILE}", path.display(), info.pixel_format, info.width, info.height);
    }
    Ok(px)
}

const OFF: u32 = u32::MAX;

/// Box sums of one tile set on the texture grid, a row of tiles at a time
/// (in parallel): `f` maps a pixel (3 bytes) to the values summed into the
/// texel holding its centre (None: left out); `seen` sees each decoded
/// tile. Returns the sums and the number of tiles read.
#[allow(clippy::too_many_arguments)]
fn accumulate<const N: usize>(
    dir: &Path,
    g: &naip::Grid,
    tiles: &[TileEntry],
    frame: &PlanFrame,
    pg: &PlanGrid,
    t0: Instant,
    f: impl Fn(&[u8]) -> Option<[f32; N]> + Sync,
    mut seen: impl FnMut(usize, usize, &[u8]),
) -> (Vec<[f32; N]>, u32) {
    let (w, h) = (pg.width, pg.height);
    // The texel column (row) holding each pixel column's (row's) centre: the
    // plan is linear in longitude and latitude, so x hangs on the column and z on the row.
    let col_i: Vec<u32> = (0..g.width_px())
        .map(|gx| {
            let (x, _) = frame.to_plan(g.lonlat(gx as f64 + 0.5, 0.0).0, g.north);
            let i = ((x - pg.origin_x) / pg.texel).floor();
            if i >= 0.0 && (i as usize) < w { i as u32 } else { OFF }
        })
        .collect();
    let row_j: Vec<u32> = (0..g.height_px())
        .map(|gy| {
            let (_, z) = frame.to_plan(g.west, g.lonlat(0.0, gy as f64 + 0.5).1);
            let j = ((z - pg.origin_z) / pg.texel).floor();
            if j >= 0.0 && (j as usize) < h { j as u32 } else { OFF }
        })
        .collect();
    let mut acc: Vec<[f32; N]> = vec![[0.0; N]; w * h];
    let mut used = 0u32;
    for r in 0..g.rows {
        let row: Vec<&TileEntry> = tiles.iter().filter(|t| t.r == r).collect();
        if row.is_empty() {
            continue;
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let decoded: Vec<(usize, Result<Vec<u8>>)> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..eo_hills::threads().min(row.len()))
                .map(|_| {
                    let (next, row) = (&next, &row);
                    s.spawn(move || {
                        let mut v = vec![];
                        loop {
                            let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(t) = row.get(k) else { break };
                            v.push((t.c, decode_jpeg(&dir.join(&t.file))));
                        }
                        v
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let mut band: Vec<(usize, Vec<u8>)> = vec![];
        for (c, px) in decoded {
            match px {
                Ok(px) => {
                    seen(r, c, &px);
                    band.push((c, px));
                    used += 1;
                }
                Err(e) => eprintln!("tile r{r:03}_c{c:03}: {e:#}: skipped"),
            }
        }
        let (gy0, gy1) = (r * TILE, (r + 1) * TILE);
        let js: Vec<u32> = row_j[gy0..gy1].iter().copied().filter(|&j| j != OFF).collect();
        let (Some(&j_lo), Some(&j_hi)) = (js.iter().min(), js.iter().max()) else { continue };
        let (j_lo, j_hi) = (j_lo as usize, j_hi as usize);
        let (band, col_i, row_j, f) = (&band, &col_i, &row_j, &f);
        eo_hills::par_rows(&mut acc[j_lo * w..(j_hi + 1) * w], w, |jr0, chunk| {
            let (ja, jb) = (j_lo + jr0, j_lo + jr0 + chunk.len() / w);
            for gy in gy0..gy1 {
                let j = row_j[gy];
                if j == OFF || (j as usize) < ja || (j as usize) >= jb {
                    continue;
                }
                let out = &mut chunk[(j as usize - ja) * w..(j as usize - ja + 1) * w];
                let ty = gy - gy0;
                for (c, px) in band {
                    let src = &px[ty * TILE * 3..(ty + 1) * TILE * 3];
                    let cols = &col_i[c * TILE..(c + 1) * TILE];
                    for (p, &i) in src.chunks_exact(3).zip(cols) {
                        if i == OFF {
                            continue;
                        }
                        if let Some(v) = f(p) {
                            let a = &mut out[i as usize];
                            for k in 0..N {
                                a[k] += v[k];
                            }
                        }
                    }
                }
            }
        });
    }
    eprintln!("  {used} tiles summed ({:.1} s)", t0.elapsed().as_secs_f64());
    (acc, used)
}

/// The mapped greens by class (later, more specific classes win where they overlap).
const C_OTHER: u8 = 1; // gardens, playgrounds, paved parks, ...
const C_PARK: u8 = 2; // park, nature reserve (vegetated)
const C_GOLF: u8 = 3;
const C_GRASSLAND: u8 = 4; // grassland, meadow
const C_SCRUB: u8 = 5; // scrub, heath, wetland
const C_WOOD: u8 = 6; // wood, forest
const C_LAWN: u8 = 7; // mown grass: OSM grass, pitches, greens and commons on vegetated ground, or surface=grass
const C_BARE: u8 = 8; // bare or rock ground: construction, brownfield, bare rock

fn class_of(kind: &str, ground: &str, surface: &str) -> u8 {
    if surface == "grass" {
        return C_LAWN;
    }
    if matches!(ground, "bare" | "rock") {
        return C_BARE;
    }
    match (kind, ground) {
        ("grass" | "pitch" | "village_green" | "recreation_ground" | "common", "vegetated") => C_LAWN,
        ("wood" | "forest", _) => C_WOOD,
        ("scrub" | "heath" | "wetland", _) => C_SCRUB,
        ("grassland" | "meadow", _) => C_GRASSLAND,
        ("golf_course", _) => C_GOLF,
        ("park" | "nature_reserve", "vegetated") => C_PARK,
        _ => C_OTHER,
    }
}

const CLASS_NAMES: [&str; 9] = ["outside_greens", "other_green", "park", "golf_course", "grassland", "scrub", "wood", "lawn_any", "bare"];

fn green_classes(paths: &eo_data::DataPaths, pg: &PlanGrid) -> Result<Vec<u8>> {
    let p = paths.sf("sf-lots-parks.json.gz");
    let v: serde_json::Value = serde_json::from_slice(&eo_data::read_maybe_gzip(&p)?).with_context(|| format!("parsing {}", p.display()))?;
    if v["meta"]["projection"]["axes"].as_str() != Some("x east, z south, metres") {
        bail!("{}: not on the surveyed plan", p.display());
    }
    let mut greens: Vec<(u8, Vec<Vec<[f64; 2]>>)> = vec![];
    for g in v["greens"].as_array().context("sf-lots-parks: no greens")? {
        let s = |k: &str| g[k].as_str().unwrap_or_default();
        let flat = |a: &serde_json::Value| a.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect::<Vec<f64>>()).unwrap_or_default();
        let mut rings = vec![raster::ring_of_flat(&flat(&g["polygon"]))];
        for h in g["holes"].as_array().into_iter().flatten() {
            rings.push(raster::ring_of_flat(&flat(h)));
        }
        greens.push((class_of(s("kind"), s("ground"), s("surface")), rings));
    }
    greens.sort_by_key(|g| g.0);
    let mut class = vec![0u8; pg.width * pg.height];
    for (c, rings) in &greens {
        let refs: Vec<&[[f64; 2]]> = rings.iter().filter(|r| r.len() >= 3).map(|r| r.as_slice()).collect();
        raster::fill_rings(pg, &refs, |i, j| class[j * pg.width + i] = *c);
    }
    Ok(class)
}

/// The mapped surface car parks (sf-lots-parks `lots` that are not buildings), with their holes.
fn surface_lots(paths: &eo_data::DataPaths, pg: &PlanGrid) -> Result<Vec<bool>> {
    let p = paths.sf("sf-lots-parks.json.gz");
    let v: serde_json::Value = serde_json::from_slice(&eo_data::read_maybe_gzip(&p)?).with_context(|| format!("parsing {}", p.display()))?;
    let mut lot = vec![false; pg.width * pg.height];
    let flat = |a: &serde_json::Value| a.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect::<Vec<f64>>()).unwrap_or_default();
    for l in v["lots"].as_array().context("sf-lots-parks: no lots")? {
        if l["building"].as_bool().unwrap_or(false) {
            continue;
        }
        let mut rings = vec![raster::ring_of_flat(&flat(&l["polygon"]))];
        for h in l["holes"].as_array().into_iter().flatten() {
            rings.push(raster::ring_of_flat(&flat(h)));
        }
        let refs: Vec<&[[f64; 2]]> = rings.iter().filter(|r| r.len() >= 3).map(|r| r.as_slice()).collect();
        raster::fill_rings(pg, &refs, |i, j| lot[j * pg.width + i] = true);
    }
    Ok(lot)
}

/// Mapped trails and bare ground (the soil rule's): a path, footway, track,
/// bridleway or steps at grade within 1.5 m inside a park, grassland, scrub,
/// wood or golf course, or mapped bare ground; and every way at grade within 3 m.
fn trails(city: &eo_data::City, pg: &PlanGrid, class: &[u8]) -> (Vec<bool>, Vec<bool>) {
    let n = pg.width * pg.height;
    let (mut near_way, mut path) = (vec![false; n], vec![false; n]);
    for r in &city.roads {
        let pts: Vec<[f64; 2]> = r.points.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        if pts.is_empty() || r.bridge || r.tunnel || r.layer != 0 {
            continue;
        }
        raster::fill_corridor(pg, &pts, 3.0, |i, j| near_way[j * pg.width + i] = true);
        if matches!(r.highway.as_str(), "path" | "footway" | "track" | "bridleway" | "steps") {
            raster::fill_corridor(pg, &pts, 1.5, |i, j| path[j * pg.width + i] = true);
        }
    }
    let trail = (0..n).map(|k| class[k] == C_BARE || (path[k] && matches!(class[k], C_PARK | C_GRASSLAND | C_SCRUB | C_WOOD | C_GOLF))).collect();
    (trail, near_way)
}

/// Ground slope at (x, z) on the surveyed elevation grid (central differences over 8 m).
fn slope(elev: &eo_data::ElevationGrid, x: f64, z: f64) -> Option<f64> {
    let d = 4.0;
    let s = |x: f64, z: f64| elev.sample_surveyed(x, z).filter(|v| v.is_finite());
    let gx = (s(x + d, z)? - s(x - d, z)?) / (2.0 * d);
    let gz = (s(x, z + d)? - s(x, z - d)?) / (2.0 * d);
    Some(gx.hypot(gz))
}

/// Level `img` as BC7 blocks (sRGB RGBA8 in), a band of block rows per thread.
fn bc7(img: &Image) -> Vec<u8> {
    let (px, pw, ph) = mm::to_srgb8_padded(img);
    let settings = intel_tex_2::bc7::alpha_basic_settings();
    let row_out = (pw / 4) * 16;
    let block_rows = ph / 4;
    let per = block_rows.div_ceil(eo_hills::threads()).max(1);
    let mut out = vec![0u8; row_out * block_rows];
    std::thread::scope(|s| {
        for (k, dst) in out.chunks_mut(per * row_out).enumerate() {
            let rows = dst.len() / row_out;
            let y0 = k * per * 4;
            let src = &px[y0 * pw * 4..(y0 + rows * 4) * pw * 4];
            s.spawn(move || {
                let surf = intel_tex_2::RgbaSurface { data: src, width: pw as u32, height: (rows * 4) as u32, stride: (pw * 4) as u32 };
                intel_tex_2::bc7::compress_blocks_into(&settings, &surf, dst);
            });
        }
    });
    out
}

/// The level as sRGB RGB8, closed texels black.
fn preview_rgb(img: &Image) -> Vec<u8> {
    let mut rgb = vec![0u8; img.w * img.h * 3];
    for (o, p) in rgb.chunks_exact_mut(3).zip(&img.px) {
        if p[3] > 0.0 {
            o.copy_from_slice(&to_srgb8([p[0], p[1], p[2]]));
        }
    }
    rgb
}

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) -> Result<()> {
    if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let f = std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(rgb)?;
    Ok(())
}

fn median_of(v: &mut [f32]) -> f32 {
    mm::median(v).unwrap_or(f32::NAN)
}

fn fmt8(c: [u8; 3]) -> String {
    format!("({:3},{:3},{:3})", c[0], c[1], c[2])
}

fn lum4(p: &[f32; 4]) -> f32 {
    mm::luminance([p[0], p[1], p[2]])
}

/// The per-texel layers the colour modes read.
struct Layers {
    w: usize,
    /// OSM green class (`C_*`, 0 outside), WorldCover class.
    class: Vec<u8>,
    wc: Vec<u8>,
    /// Residential to primary roads at grade, centre within 2 m of the centreline.
    road: Vec<bool>,
    /// Any mapped way at grade within 3 m.
    near_way: Vec<bool>,
    /// A mapped path, footway, track or steps (centre within 1.5 m) inside a
    /// park, grassland, scrub, wood or golf course; or mapped bare ground.
    trail: Vec<bool>,
    slope: Vec<f32>,
    ndvi: Vec<f32>,
    lawn_region: Vec<bool>,
    dry_region: Vec<bool>,
    wood_region: Vec<bool>,
    scrub_region: Vec<bool>,
    /// A mapped surface car park (sf-lots-parks `lots`, not a building).
    lot: Vec<bool>,
}

impl Layers {
    fn features(&self, px: &[f32; 4], k: usize) -> Features {
        let green = matches!(self.class[k], C_PARK | C_GRASSLAND | C_SCRUB | C_WOOD | C_GOLF | C_LAWN) && !self.near_way[k];
        let wild = matches!(self.class[k], C_PARK | C_GRASSLAND | C_SCRUB | C_WOOD);
        Features { c: [px[0], px[1], px[2]], ndvi: self.ndvi[k], wood: self.class[k] == C_WOOD, trail: self.trail[k], green, wild, slope: self.slope[k] }
    }
}

/// p10, p50, p90 of `v` (reordered).
fn percentiles(v: &mut [f32]) -> [f32; 3] {
    if v.is_empty() {
        return [f32::NAN; 3];
    }
    let n = v.len();
    [n / 10, n / 2, (n * 9) / 10].map(|i| *v.select_nth_unstable_by(i.min(n - 1), f32::total_cmp).1)
}

/// The features of reference sets of texels (p10 / p50 / p90): what the
/// class rules' thresholds are tuned on.
fn print_reference_sets(img: &Image, ly: &Layers) {
    let wc_res = |k: usize| ly.class[k] == 0 && ly.wc[k] == 50 && !ly.near_way[k];
    let path_trail = |k: usize| ly.trail[k] && ly.class[k] != C_BARE;
    let dry_slope = |k: usize| ly.dry_region[k] && ly.slope[k] > 0.15 && !ly.near_way[k] && !ly.lawn_region[k];
    let sets: [(&str, &dyn Fn(usize) -> bool); 10] = [
        ("roads", &|k| ly.road[k]),
        ("lawn, flat", &|k| ly.lawn_region[k] && ly.slope[k] <= 0.06),
        ("dry slopes (no NDVI test)", &dry_slope),
        ("dry slopes, NDVI < 0.1", &|k| dry_slope(k) && ly.ndvi[k] < 0.1),
        ("trails in greens, NDVI < 0.1", &|k| path_trail(k) && ly.ndvi[k] < 0.1),
        ("OSM wood", &|k| ly.wood_region[k] && !ly.near_way[k]),
        ("OSM scrub", &|k| ly.scrub_region[k] && !ly.near_way[k]),
        ("trails in greens", &path_trail),
        ("OSM bare ground", &|k| ly.class[k] == C_BARE),
        ("residential (WorldCover built)", &wc_res),
    ];
    eprintln!("reference sets, p10 / p50 / p90 (every 3rd open texel):");
    eprintln!("  {:32} {:>9}  {:^20}  {:^20}  {:^17}  {:^17}  {:^17}", "set", "texels", "NDVI", "luminance", "r/g", "g/b", "max/min");
    for (name, sel) in sets {
        let mut f: [Vec<f32>; 5] = Default::default();
        for k in (0..img.px.len()).step_by(3) {
            let p = img.px[k];
            if p[3] <= 0.0 || !sel(k) {
                continue;
            }
            let [r, g, b] = [p[0], p[1], p[2]].map(|v| v.max(1e-5));
            if !ly.ndvi[k].is_nan() {
                f[0].push(ly.ndvi[k]);
            }
            f[1].push(lum4(&p));
            f[2].push(r / g);
            f[3].push(g / b);
            f[4].push(r.max(g).max(b) / r.min(g).min(b));
        }
        let n = f[1].len();
        let q: Vec<String> = f.iter_mut().enumerate().map(|(i, v)| {
            let p = percentiles(v);
            if i == 1 { format!("{:.3} {:.3} {:.3}", p[0], p[1], p[2]) } else { format!("{:5.2} {:5.2} {:5.2}", p[0], p[1], p[2]) }
        }).collect();
        eprintln!("  {name:32} {n:9}  {:^20}  {:^20}  {:^17}  {:^17}  {:^17}", q[0], q[1], q[2], q[3], q[4]);
    }
}

/// The colour transfer (mode "classes"): weights, references, the transfer.
/// Returns the record and each class's texels (the ones it wins).
fn colour_by_classes(img: &mut Image, ly: &Layers, tones: &Tones, t0: Instant) -> (ColourTransfer, Vec<Vec<usize>>, Vec<u8>) {
    let p = tr::Params::default();
    let w = ly.w;
    // 1. The winning class of every open texel (255: closed).
    let mut win = vec![255u8; img.px.len()];
    {
        let px = &img.px;
        eo_hills::par_rows(&mut win, w, |j0, rows| {
            for (o, v) in rows.iter_mut().enumerate() {
                let k = j0 * w + o;
                if px[k][3] > 0.0 {
                    *v = tr::winner(&tr::weights(&ly.features(&px[k], k), &p)) as u8;
                }
            }
        });
    }
    // Paving's gain: the roads' median luminance to that of ROAD_SRGB.
    let mut road_l: Vec<f32> = (0..img.px.len()).filter(|&k| ly.road[k] && img.px[k][3] > 0.0).map(|k| lum4(&img.px[k])).collect();
    let road_lum = mm::luminance(ROAD_SRGB.map(|v| srgb_to_linear(v as f32 / 255.0)));
    let paving_gain = road_lum / median_of(&mut road_l);
    let mut idx: Vec<Vec<usize>> = vec![vec![]; tr::N];
    for (k, &c) in win.iter().enumerate() {
        if c != 255 {
            idx[c as usize].push(k);
        }
    }
    // 2. Each class's references in the photo.
    let mut refs = tr::ClassRefs { median_lum: [0.1; tr::N], chroma: [[1.0; 3]; tr::N] };
    let mut classes: Vec<TransferClass> = vec![];
    let rules = [
        "NDVI high (smoothstep veg_ndvi) and light (smoothstep lawn_lum on the photo's luminance), fading with slope (wild_lawn_slope) inside a wild mapped green (park, grassland, scrub, wood)",
        "NDVI low (1 - smoothstep bare_ndvi), not neutral, warm (smoothstep warm of min(r/g, g/b)), not very dark (smoothstep dry_lum), not soil",
        "NDVI low, not neutral, red (smoothstep soil_red of r/g), or at least trail_soil on a mapped trail (path, footway, track, steps within 1.5 m, inside a park, grassland, scrub, wood or golf course) or mapped bare ground",
        "NDVI low and neutral (1 - smoothstep neutral of max/min channel), plus the floor (what nothing else claims)",
        "NDVI high and dark (1 - lawn), the lighter share (smoothstep canopy_lum)",
        "NDVI high and dark (1 - lawn), the darker share (1 - smoothstep canopy_lum), at least wood_canopy inside a mapped wood or forest",
    ];
    for k in 0..tr::N {
        let ids = &idx[k];
        let mut ls: Vec<f32> = ids.iter().map(|&i| lum4(&img.px[i])).collect();
        let ml = median_of(&mut ls);
        let mut q: [Vec<f32>; 3] = Default::default();
        for &i in ids {
            let l = lum4(&img.px[i]).max(1e-5);
            for c in 0..3 {
                q[c].push(img.px[i][c] / l);
            }
        }
        let chroma = q.map(|mut v| median_of(&mut v));
        if !ids.is_empty() {
            refs.median_lum[k] = ml;
            refs.chroma[k] = chroma;
        }
        classes.push(TransferClass { name: tr::NAMES[k].into(), rule: rules[k].into(), palette: [0.0; 3], texels: ids.len() as u64, photo_median: mm::channel_medians(&img.px, ids), photo_median_lum: ml, photo_chroma: chroma, ..Default::default() });
    }
    // 3. Dry grass's luminance: the renderer's own unless much brighter than the photo implies.
    let mut pal = tones.palette;
    let implied = refs.median_lum[tr::DRY] * paving_gain;
    let own = mm::luminance(pal.dry);
    let dry_lum = if own > DRY_BRIGHT * implied { DRY_LUM_LOW } else { own };
    pal.dry = scale_to_lum(pal.dry, dry_lum);
    for (k, c) in classes.iter_mut().enumerate() {
        c.palette = if k == tr::PAVING { [0.0; 3] } else { pal.colour(k, 0.0, &p) };
    }
    eprintln!("transfer: paving gain {paving_gain:.3}; the photo's dry grass at that gain: luminance {implied:.3}; dry palette luminance {dry_lum:.3} (own {own:.3}) ({:.1} s)", t0.elapsed().as_secs_f64());
    // 4. The transfer.
    eo_hills::par_rows(&mut img.px, w, |j0, rows| {
        for (o, px) in rows.iter_mut().enumerate() {
            if px[3] <= 0.0 {
                continue;
            }
            let k = j0 * w + o;
            let f = ly.features(px, k);
            // a surface car park in its lots' asphalt tone: the photograph's parked cars
            // are not baked into the ground (the renderer's scans give it its detail)
            let c = if ly.lot[k] { tones.lot } else { tr::transfer(&f, &tr::weights(&f, &p), &pal, &refs, &p, paving_gain) };
            px[0] = c[0];
            px[1] = c[1];
            px[2] = c[2];
        }
    });
    for (k, c) in classes.iter_mut().enumerate() {
        c.after_median = mm::channel_medians(&img.px, &idx[k]);
    }
    let record = ColourTransfer {
        classes,
        params: p,
        soil_eroded: pal.soil_eroded,
        paving_gain,
        paving_road_lum: road_lum,
        ndvi: "per 2 m texel from the 0.6 m colour-infrared tiles (bands NIR, R, G): box sums of the 8-bit NIR and red over the pixels whose centres fall in the texel, each less its haze (the per-band 0.2 % darkest 8-bit level, median over every 9th CIR tile), NDVI = (NIR - R) / (NIR + R) of the means".into(),
        cir_haze: [0.0; 2],
        palette_source: tones.source.clone(),
    };
    (record, idx, win)
}

/// The colour matrix (mode "matrix"): class medians to targets, least squares with a ridge.
fn colour_by_matrix(img: &mut Image, ly: &Layers, tones: &Tones, ridge: f32) -> Result<(mm::Mat3, f32, Vec<ClassFit>, Vec<Vec<usize>>)> {
    let (mut roads, mut lawns, mut dry, mut canopy) = (vec![], vec![], vec![], vec![]);
    for (k, p) in img.px.iter().enumerate() {
        if p[3] <= 0.0 {
            continue;
        }
        if ly.road[k] {
            roads.push(k);
        }
        if ly.lawn_region[k] && ly.slope[k] <= 0.06 && p[1] > 1.03 * p[0] && p[1] > 1.12 * p[2] {
            lawns.push(k);
        }
        if ly.dry_region[k] && !ly.near_way[k] && !ly.road[k] && !ly.lawn_region[k] && ly.slope[k] > 0.15 && ly.ndvi[k] < 0.25 {
            dry.push(k);
        }
        if ly.wood_region[k] && !ly.near_way[k] {
            canopy.push(k);
        }
    }
    let road_lum = median_of(&mut roads.iter().map(|&k| lum4(&img.px[k])).collect::<Vec<_>>());
    let mut fits: Vec<ClassFit> = vec![];
    let groups = vec![roads, lawns, dry, canopy];
    for (k, (name, rule, weight, target)) in [
        ("roads", "centre within 2 m of a residential, tertiary, secondary or primary road's centreline at grade, outside footprints", 1.0, [road_lum; 3]),
        ("lawns", "inside a mapped lawn (8-neighbour erosion), slope <= 0.06, green (G > 1.03 R, G > 1.12 B)", 1.0, tones.palette.lawn),
        ("dry_grass", "inside an OSM park, grassland or meadow or WorldCover grassland (8-neighbour erosion; not lawn, wood or scrub), not within 3 m of a way, slope > 0.15, NDVI < 0.25", 2.0, tones.matrix_dry),
        ("canopy", "inside an OSM wood or forest (8-neighbour erosion), not within 3 m of a way", 1.0, tones.matrix_canopy),
    ]
    .into_iter()
    .enumerate()
    {
        let idx = &groups[k];
        if idx.len() < 100 {
            bail!("class {name}: only {} texels", idx.len());
        }
        fits.push(ClassFit { name: name.into(), rule: rule.into(), weight, texels: idx.len() as u64, naip_median: mm::channel_medians(&img.px, idx), target, ..Default::default() });
    }
    let samples: Vec<([f32; 3], [f32; 3], f32)> = fits.iter().map(|f| (f.naip_median, f.target, f.weight)).collect();
    let m = mm::fit_colour_matrix(&samples, ridge).context("the colour matrix fit is singular")?;
    let residual = mm::fit_residual(&m, &samples);
    for f in &mut fits {
        f.fitted = mm::mat_apply(&m, f.naip_median);
    }
    eprintln!("colour matrix (ridge {ridge}), residual {residual:.3e}:");
    for r in &m {
        eprintln!("  [{:+.4} {:+.4} {:+.4}]", r[0], r[1], r[2]);
    }
    eo_hills::par_rows(&mut img.px, ly.w, |_, rows| {
        for p in rows {
            let c = mm::mat_apply(&m, [p[0], p[1], p[2]]);
            for k in 0..3 {
                p[k] = c[k].max(0.0);
            }
        }
    });
    for (f, idx) in fits.iter_mut().zip(&groups) {
        f.after_median = mm::channel_medians(&img.px, idx);
    }
    Ok((m, residual, fits, groups))
}

fn cook_macro(preview: Option<&Path>, probes: &[Probe], ridge: f32, mode: Mode) -> Result<()> {
    let t0 = Instant::now();
    let dir = PathBuf::from(std::env::var("EO_AERIAL_DIR").context("set EO_AERIAL_DIR to the folder fetch-aerial.mjs wrote")?);
    let mpath = dir.join("manifest.json");
    let manifest_bytes = std::fs::read(&mpath).with_context(|| format!("reading {}", mpath.display()))?;
    let manifest = naip::Manifest::parse(&manifest_bytes)?;
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load_at(&paths, 1.0).context("the city at 1:1")?;
    let frame = PlanFrame::from_meta(&city.meta);
    if let Some(f) = manifest.frame {
        let want = [frame.center_lat, frame.center_lon, frame.m_per_deg_lat, frame.m_per_deg_lon];
        if (0..4).any(|k| (f[k] - want[k]).abs() > 1e-9 * want[k].abs().max(1.0)) {
            bail!("the tiles were planned in frame {f:?}, the city's is {want:?}");
        }
    }
    let tones = renderer_tones()?;
    let g = manifest.rgb;
    let cir_grid = manifest.cir.context("manifest.json: no colour-infrared grid (the classes need NDVI)")?;
    let pm = g.pixel_m(&frame);
    eprintln!("{} footprints, {} roads; NAIP grid {} x {} tiles of {TILE} px ({:.3} x {:.3} m a pixel), {} RGB and {} CIR tiles listed ({:.1} s)", city.buildings.len(), city.roads.len(), g.cols, g.rows, pm[0], pm[1], manifest.tiles.len(), manifest.cir_tiles.len(), t0.elapsed().as_secs_f64());

    // 1. The tiles, each against its digest.
    let (tiles, missing) = verify_tiles(&dir, &manifest.tiles, "rgb")?;
    let (cir_tiles, cir_missing) = verify_tiles(&dir, &manifest.cir_tiles, "cir")?;
    eprintln!("{} RGB tiles verified ({missing} missing), {} CIR ({cir_missing} missing) ({:.1} s)", tiles.len(), cir_tiles.len(), t0.elapsed().as_secs_f64());

    // 2. The texture's grid (`macro_grid`).
    let pg = macro_grid(&g, &frame);
    let (origin_x, origin_z, w, h) = (pg.origin_x, pg.origin_z, pg.width, pg.height);
    let bb = g.bbox();
    let (xw, zn) = frame.to_plan(bb[0], bb[3]);
    let (xe, zs) = frame.to_plan(bb[2], bb[1]);
    eprintln!("texture {w} x {h} texels of {TEXEL_M} m, north-west corner ({origin_x}, {origin_z}) on the surveyed plan; imagery box x {xw:.1}..{xe:.1}, z {zn:.1}..{zs:.1}");

    // 3. RGB box sums in linear light; each tile's haze.
    let lut: Vec<f32> = (0..256).map(|v| srgb_to_linear(v as f32 / 255.0)).collect();
    let mut tile_haze: HashMap<(usize, usize), [f32; 3]> = HashMap::new();
    let mut black_px = 0u64;
    let (acc, used) = accumulate::<4>(
        &dir,
        &g,
        &tiles,
        &frame,
        &pg,
        t0,
        |p| (p != [0, 0, 0]).then(|| [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize], 1.0]),
        |r, c, px| {
            let (hist, n) = naip::histogram(px);
            black_px += (TILE * TILE) as u64 - n as u64;
            if let Some(d) = naip::haze_of(&hist, n) {
                tile_haze.insert((r, c), d);
            }
        },
    );
    // The survey's haze: the median over every 9th tile present (roof_survey's sample).
    let sample = naip::Manifest::haze_sample(tiles.iter());
    let levels: Vec<[f32; 3]> = sample.iter().filter_map(|k| tile_haze.get(k).copied()).collect();
    let haze = naip::median_haze(&levels).context("no tile to take the haze from")?;
    eprintln!("haze (linear, median of {} tiles): {:.4} {:.4} {:.4}; {black_px} no-data pixels", levels.len(), haze[0], haze[1], haze[2]);

    // 4. Level 0: the mean, dehazed; footprints and empty texels closed.
    let mut built = vec![false; w * h];
    for b in &city.buildings {
        let ring: Vec<[f64; 2]> = b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        if ring.len() >= 3 {
            raster::fill_rings(&pg, &[&ring], |i, j| built[j * w + i] = true);
        }
    }
    let mut img = Image { w, h, px: acc };
    let (mut n_built, mut n_empty) = (0u64, 0u64);
    for (p, &b) in img.px.iter_mut().zip(&built) {
        let n = p[3];
        if n > 0.0 {
            for c in 0..3 {
                p[c] = naip::dehaze(p[c] / n, haze[c]);
            }
        }
        p[3] = if n > 0.0 && !b { 1.0 } else { 0.0 };
        if b {
            n_built += 1;
        } else if n <= 0.0 {
            n_empty += 1;
        }
    }
    drop(built);
    eprintln!("level 0: {n_built} built, {n_empty} without imagery ({:.1} s)", t0.elapsed().as_secs_f64());

    // 5. NDVI per texel from the CIR tiles (bands NIR, R, G): box means of the
    //    8-bit NIR and red, each less its haze (dark-object subtraction per band).
    let mut cir_dark: HashMap<(usize, usize), [u8; 3]> = HashMap::new();
    let (cir_acc, cir_used) = accumulate::<3>(&dir, &cir_grid, &cir_tiles, &frame, &pg, t0, |p| (p != [0, 0, 0]).then_some([p[0] as f32, p[1] as f32, 1.0]), |r, c, px| {
        let (hist, n) = naip::histogram(px);
        if let Some(d) = naip::dark_level(&hist, n) {
            cir_dark.insert((r, c), d);
        }
    });
    let cir_sample = naip::Manifest::haze_sample(cir_tiles.iter());
    let cir_levels: Vec<[f32; 3]> = cir_sample.iter().filter_map(|k| cir_dark.get(k).map(|d| d.map(|v| v as f32))).collect();
    let cir_haze = naip::median_haze(&cir_levels).context("no CIR tile to take the haze from")?;
    let ndvi_of = |a: &[f32; 3], hz: [f32; 2]| -> f32 {
        if a[2] <= 0.0 {
            return f32::NAN;
        }
        let (n, r) = ((a[0] / a[2] - hz[0]).max(0.0), (a[1] / a[2] - hz[1]).max(0.0));
        if n + r > 1.0 { (n - r) / (n + r) } else { f32::NAN }
    };
    let ndvi: Vec<f32> = cir_acc.iter().map(|a| ndvi_of(a, [cir_haze[0], cir_haze[1]])).collect();
    let ndvi_raw_med = {
        let mut v: Vec<f32> = cir_acc.iter().step_by(7).map(|a| ndvi_of(a, [0.0, 0.0])).filter(|v| !v.is_nan()).collect();
        median_of(&mut v)
    };
    drop(cir_acc);
    eprintln!("CIR haze (8-bit, median of {} tiles): NIR {:.0}, red {:.0}; NDVI median over all texels {:.3} raw, {:.3} dehazed", cir_levels.len(), cir_haze[0], cir_haze[1], ndvi_raw_med, {
        let mut v: Vec<f32> = ndvi.iter().step_by(7).copied().filter(|v| !v.is_nan()).collect();
        median_of(&mut v)
    });

    // 6. The layers: greens, WorldCover, roads and trails, slope, regions.
    let class = green_classes(&paths, &pg)?;
    let lc = eo_data::LandCover::decode(&eo_data::read_maybe_gzip(&paths.sf("sf-land-cover.bin.gz"))?, eo_data::Frame::sf())?;
    let mut wc = vec![0u8; w * h];
    eo_hills::par_rows(&mut wc, w, |j0, rows| {
        for (k, row) in rows.chunks_mut(w).enumerate() {
            for (i, v) in row.iter_mut().enumerate() {
                let (x, z) = pg.centre(i, j0 + k);
                *v = lc.class_at(x, z);
            }
        }
    });
    let elev = eo_data::ElevationGrid::decode(&eo_data::read_maybe_gzip(&paths.sf("sf-elevation.bin.gz"))?)?;
    let mut slopes = vec![f32::NAN; w * h];
    eo_hills::par_rows(&mut slopes, w, |j0, rows| {
        for (k, row) in rows.chunks_mut(w).enumerate() {
            for (i, v) in row.iter_mut().enumerate() {
                let (x, z) = pg.centre(i, j0 + k);
                *v = slope(&elev, x, z).map(|s| s as f32).unwrap_or(f32::NAN);
            }
        }
    });
    let (mut road, mut near_way, mut path) = (vec![false; w * h], vec![false; w * h], vec![false; w * h]);
    for r in &city.roads {
        let pts: Vec<[f64; 2]> = r.points.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        if pts.is_empty() || r.bridge || r.tunnel || r.layer != 0 {
            continue;
        }
        raster::fill_corridor(&pg, &pts, 3.0, |i, j| near_way[j * w + i] = true);
        match r.highway.as_str() {
            "residential" | "tertiary" | "secondary" | "primary" => raster::fill_corridor(&pg, &pts, 2.0, |i, j| road[j * w + i] = true),
            "path" | "footway" | "track" | "bridleway" | "steps" => raster::fill_corridor(&pg, &pts, 1.5, |i, j| path[j * w + i] = true),
            _ => {}
        }
    }
    let trail: Vec<bool> = (0..w * h).map(|k| class[k] == C_BARE || (path[k] && matches!(class[k], C_PARK | C_GRASSLAND | C_SCRUB | C_WOOD | C_GOLF))).collect();
    drop(path);
    let ly = Layers {
        w,
        lawn_region: raster::erode(&class.iter().map(|&c| c == C_LAWN).collect::<Vec<_>>(), w, h),
        dry_region: raster::erode(&class.iter().zip(&wc).map(|(&c, &v)| matches!(c, C_PARK | C_GRASSLAND) || (v == 30 && !matches!(c, C_LAWN | C_WOOD | C_SCRUB))).collect::<Vec<_>>(), w, h),
        wood_region: raster::erode(&class.iter().map(|&c| c == C_WOOD).collect::<Vec<_>>(), w, h),
        scrub_region: raster::erode(&class.iter().map(|&c| c == C_SCRUB).collect::<Vec<_>>(), w, h),
        lot: surface_lots(&paths, &pg)?,
        class,
        wc,
        road,
        near_way,
        trail,
        slope: slopes,
        ndvi,
    };
    eprintln!("layers ({:.1} s)", t0.elapsed().as_secs_f64());
    print_reference_sets(&img, &ly);

    // OSM classes' luminance medians, for the record.
    let class_medians = |img: &Image, prefix: &str, medians: &mut BTreeMap<String, f32>| {
        let mut by: BTreeMap<&str, Vec<f32>> = BTreeMap::new();
        for (p, &c) in img.px.iter().zip(&ly.class) {
            if p[3] <= 0.0 {
                continue;
            }
            let l = lum4(p);
            by.entry("open_all").or_default().push(l);
            by.entry(CLASS_NAMES[c as usize]).or_default().push(l);
        }
        for (name, mut v) in by {
            medians.insert(format!("{prefix}{name}"), median_of(&mut v));
        }
    };
    let mut medians: BTreeMap<String, f32> = BTreeMap::new();
    class_medians(&img, "raw_", &mut medians);

    // 7. The photo's colours to the eye's.
    let (mut transfer, mut matrix, mut win) = (None, None, vec![]);
    let groups: Vec<Vec<usize>> = match mode {
        Mode::Classes => {
            let (mut rec, idx, wv) = colour_by_classes(&mut img, &ly, &tones, t0);
            rec.cir_haze = [cir_haze[0], cir_haze[1]];
            transfer = Some(rec);
            win = wv;
            idx
        }
        Mode::Matrix => {
            let (m, residual, fits, idx) = colour_by_matrix(&mut img, &ly, &tones, ridge)?;
            matrix = Some((m, residual, fits));
            idx
        }
    };
    class_medians(&img, "", &mut medians);

    // 8. Cast shadows lifted; the albedo held to its range.
    let window = (mm::SHADOW_WINDOW_M / TEXEL_M).round() as usize;
    let lifted = mm::shadow_lift(&mut img, window, mm::SHADOW_RATIO);
    mm::clamp_albedo(&mut img, mm::ALBEDO_MIN, mm::ALBEDO_MAX);
    class_medians(&img, "final_", &mut medians);
    eprintln!("shadow lift: {lifted} texels ({:.1} s)", t0.elapsed().as_secs_f64());
    eprintln!("class medians, sRGB:   texels  photo (dehazed)  target           after colour     final (lift, clamp)");
    if let Some(rec) = &mut transfer {
        for (c, idx) in rec.classes.iter_mut().zip(&groups) {
            c.final_median = mm::channel_medians(&img.px, idx);
            let target = if c.name == "paving" { "(own colour)    ".to_string() } else { fmt8(to_srgb8(c.palette)) };
            eprintln!("  {:8} {:10} {} {} {} {}", c.name, c.texels, fmt8(to_srgb8(c.photo_median)), target, fmt8(to_srgb8(c.after_median)), fmt8(to_srgb8(c.final_median)));
        }
        // The winning class per OSM green class and per WorldCover class.
        let mut hist: BTreeMap<String, [u64; tr::N]> = BTreeMap::new();
        for (k, &c) in win.iter().enumerate() {
            if c == 255 {
                continue;
            }
            hist.entry(format!("OSM {}", CLASS_NAMES[ly.class[k] as usize])).or_default()[c as usize] += 1;
            hist.entry(format!("WorldCover {:3}", ly.wc[k])).or_default()[c as usize] += 1;
        }
        eprintln!("winning class by group (% of the group's open texels):");
        eprintln!("  {:26} {:>9} {}", "group", "texels", tr::NAMES.map(|n| format!("{n:>7}")).join(""));
        for (g, n) in &hist {
            let total: u64 = n.iter().sum();
            if total < 1000 {
                continue;
            }
            eprintln!("  {g:26} {total:9} {}", n.map(|v| format!("{:7.1}", 100.0 * v as f64 / total as f64)).join(""));
        }
    }
    if let Some((_, _, fits)) = &mut matrix {
        for (f, idx) in fits.iter_mut().zip(&groups) {
            f.final_median = mm::channel_medians(&img.px, idx);
            eprintln!("  {:8} {:10} {} {} {} {}", f.name, f.texels, fmt8(to_srgb8(f.naip_median)), fmt8(to_srgb8(f.target)), fmt8(to_srgb8(f.after_median)), fmt8(to_srgb8(f.final_median)));
        }
    }
    for (name, v) in &medians {
        if name.contains("open_all") || name.contains("outside") {
            eprintln!("  median luminance {name}: {v:.4}");
        }
    }

    // 9. Mips, closed texels filled from below; BC7.
    let mut chain = mm::mip_chain(img);
    mm::fill_holes(&mut chain);
    let blocks: Vec<Vec<u8>> = chain.iter().map(bc7).collect();
    eprintln!("{} levels, BC7 ({:.1} s)", chain.len(), t0.elapsed().as_secs_f64());

    // 10. The file.
    let mut city_files = BTreeMap::new();
    for name in ["sf-city.json.gz", "sf-footprints.json.gz", "sf-footprint-supplement.json", "sf-lots-parks.json.gz", "sf-land-cover.bin.gz", "sf-elevation.bin.gz"] {
        if let Ok(b) = std::fs::read(paths.sf(name)) {
            city_files.insert(name.to_string(), sha256_hex(&b));
        }
    }
    let calibration_texels = groups.iter().map(|g| g.len() as u64).sum();
    let (colour_matrix, colour_residual, colour_classes) = match matrix {
        Some((m, r, f)) => (m, r, f),
        None => (mm::IDENTITY, 0.0, vec![]),
    };
    let header = mm::Header {
        format: mm::FORMAT.into(),
        origin_x,
        origin_z,
        texel_m: TEXEL_M,
        width: w as u32,
        height: h as u32,
        levels: mm::level_records(w as u32, h as u32),
        k: 1.0,
        lawn_target: tones.lawn_lum,
        colour_mode: if mode == Mode::Classes { "classes" } else { "matrix" }.into(),
        colour_transfer: transfer,
        colour_matrix,
        colour_ridge: if mode == Mode::Matrix { ridge } else { 0.0 },
        colour_residual,
        colour_classes,
        haze,
        shadow_ratio: mm::SHADOW_RATIO,
        shadow_window_m: mm::SHADOW_WINDOW_M,
        albedo_range: [mm::ALBEDO_MIN, mm::ALBEDO_MAX],
        stats: mm::Stats { texels: (w * h) as u64, lifted_texels: lifted, built_texels: n_built, no_data_texels: n_empty, calibration_texels, medians },
        inputs: mm::Inputs {
            naip_manifest_sha256: sha256_hex(&manifest_bytes),
            naip_tiles_used: used,
            naip_tiles_missing: missing,
            naip_cir_tiles_used: cir_used,
            naip_source: manifest.source.clone(),
            naip_licence: manifest.licence.clone(),
            naip_credit: manifest.credit.clone(),
            naip_downloaded: manifest.downloaded.clone(),
            city_files,
            lawn_target_source: tones.source.clone(),
        },
        generator: mm::GENERATOR.into(),
    };
    let out = mm::file_path(&eo_hills::cache_dir());
    mm::write(&out, &header, &blocks)?;
    let (back, start) = mm::read_header(&out)?;
    assert_eq!(back, header);
    let size = std::fs::metadata(&out)?.len();
    eprintln!("wrote {} ({:.1} MB, data from byte {start}; {:.1} s)", out.display(), size as f64 / 1e6, t0.elapsed().as_secs_f64());

    // 11. The preview (level 2, 8 m a pixel) and the probes on it.
    let l = &chain[2.min(chain.len() - 1)];
    let px_m = TEXEL_M * (w / l.w) as f64;
    let rgb = preview_rgb(l);
    if let Some(p) = preview {
        write_png(p, l.w, l.h, &rgb)?;
        eprintln!("preview {} x {} ({px_m} m a pixel): {}", l.w, l.h, p.display());
    }
    for pr in probes {
        let (cx, cy) = (((pr.x - origin_x) / px_m).floor() as i64, ((pr.z - origin_z) / px_m).floor() as i64);
        let (mut sum, mut n, mut closed) = ([0u32; 3], 0, 0);
        for y in cy - 1..=cy + 1 {
            for x in cx - 1..=cx + 1 {
                if x < 0 || y < 0 || x as usize >= l.w || y as usize >= l.h {
                    continue;
                }
                let k = y as usize * l.w + x as usize;
                for c in 0..3 {
                    sum[c] += rgb[k * 3 + c] as u32;
                }
                n += 1;
                closed += (l.px[k][3] <= 0.0) as u32;
            }
        }
        let mean = sum.map(|s| (s as f64 / n.max(1) as f64).round() as u8);
        let what = pg.texel_of(pr.x, pr.z).map(|(i, j)| {
            let k = j * w + i;
            let won = win.get(k).filter(|&&c| c != 255).map(|&c| tr::NAMES[c as usize]).unwrap_or("-");
            format!("texel: OSM {}, WorldCover {}, slope {:.2}, NDVI {:.2}{}{}, class {won}", CLASS_NAMES[ly.class[k] as usize], ly.wc[k], ly.slope[k], ly.ndvi[k], if ly.road[k] { ", road" } else { "" }, if ly.trail[k] { ", trail" } else { "" })
        });
        eprintln!("probe {:16} ({:6.0}, {:5.0}): 3x3 sRGB {} ({closed}/{n} closed); {}", pr.name, pr.x, pr.z, fmt8(mean), what.unwrap_or_default());
    }
    eprintln!("done in {:.1} s", t0.elapsed().as_secs_f64());
    Ok(())
}
