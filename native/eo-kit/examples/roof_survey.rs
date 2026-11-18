//! measure every building's roof from straight-down aerial imagery
//! and write the game's roof colour table (`eo_data::roof_colours`).
//!
//! Imagery: USDA NAIP through the USGS National Map image service (public
//! domain), fetched as ordinary JPEG tiles on a plain longitude/latitude grid
//! by `native/tools/fetch-aerial.mjs` into a folder OUTSIDE the repository
//! named by `EO_AERIAL_DIR` (required). Steps:
//!
//!   cargo run --release -p eo-kit --example roof_survey -- plan     # tile list (tiles.json) for the fetch script
//!   node native/tools/fetch-aerial.mjs <aerial dir>                 # download + manifest.json (sha-256 per file)
//!   cargo run --release -p eo-kit --example roof_survey -- survey   # measure: writes native/assets/roof-colours-v1.bin.gz
//!                                                                   #   and native/assets/ground-green-v1.bin.gz
//!   cargo run --release -p eo-kit --example roof_survey -- overlay <out dir>   # footprints over the imagery, swatches
//!
//! `EO_ROOF_SURVEY_OUT=<dir>` writes the two tables there instead of `native/assets`.
//!
//! The mapping: footprints are loaded on the surveyed 1:1 plan
//! (`City::load_at(paths, 1.0)`) and unprojected with the city's own frame
//! (`sf-city.json` `meta.center` and `meta.projection`, `PlanFrame`). The
//! game's drawn plan is that times `groundscale::scale()`; the table is keyed
//! by record, so the scale never enters it.

use eo_data::roof_colours::{self as rc, PlanFrame, RawEntry, RoofMeasure, RoofTable};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Ground size of one pixel of each tile set, metres, and the tile side in pixels.
const RGB_M: f64 = 0.3;
const CIR_M: f64 = 0.6;
const TILE: usize = 2000;
const SERVICE: &str = "https://imagery.nationalmap.gov/arcgis/rest/services/USGSNAIPImagery/ImageServer/exportImage";

fn aerial_dir() -> PathBuf {
    PathBuf::from(std::env::var("EO_AERIAL_DIR").expect("set EO_AERIAL_DIR to the folder fetch-aerial.mjs wrote"))
}

/// A plain lon/lat pixel grid: pixel (gx, gy) covers lon west + gx*dlon .. +dlon,
/// lat north - gy*dlat .. -dlat.
#[derive(Clone, Copy, Debug)]
struct Grid {
    west: f64,
    north: f64,
    dlon: f64,
    dlat: f64,
    cols: usize,
    rows: usize,
}

impl Grid {
    fn new(frame: &PlanFrame, west: f64, north: f64, east: f64, south: f64, m: f64) -> Grid {
        let dlon = m / frame.m_per_deg_lon;
        let dlat = m / frame.m_per_deg_lat;
        let cols = (((east - west) / dlon) / TILE as f64).ceil() as usize;
        let rows = (((north - south) / dlat) / TILE as f64).ceil() as usize;
        Grid { west, north, dlon, dlat, cols, rows }
    }
    /// Continuous pixel coordinates of (lon, lat) (pixel centres at +0.5).
    fn px(&self, lon: f64, lat: f64) -> (f64, f64) {
        ((lon - self.west) / self.dlon, (self.north - lat) / self.dlat)
    }
    fn lonlat(&self, gx: f64, gy: f64) -> (f64, f64) {
        (self.west + gx * self.dlon, self.north - gy * self.dlat)
    }
    fn tile_bbox(&self, r: usize, c: usize) -> [f64; 4] {
        let w = self.west + (c * TILE) as f64 * self.dlon;
        let e = w + TILE as f64 * self.dlon;
        let n = self.north - (r * TILE) as f64 * self.dlat;
        let s = n - TILE as f64 * self.dlat;
        [w, s, e, n]
    }
}

struct Ctx {
    frame: PlanFrame,
    city: eo_data::City,
    rgb: Grid,
    cir: Grid,
    dir: PathBuf,
}

fn load_ctx() -> Ctx {
    let root = eo_kit::repo_root().expect("repo root (EO_REPO_ROOT)");
    let paths = eo_data::DataPaths { repo_root: root };
    let city = eo_data::City::load_at(&paths, 1.0).expect("city at 1:1");
    let frame = PlanFrame::from_meta(&city.meta);
    let (mut w, mut e, mut s, mut n) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for b in &city.buildings {
        for p in &b.ring {
            let (lon, lat) = frame.to_lonlat(p[0] as f64, p[1] as f64);
            w = w.min(lon);
            e = e.max(lon);
            s = s.min(lat);
            n = n.max(lat);
        }
    }
    // 120 m margin (the lean of tall buildings, the inset), rounded out to 1e-4 degrees.
    let mlon = 120.0 / frame.m_per_deg_lon;
    let mlat = 120.0 / frame.m_per_deg_lat;
    let (w, e, s, n) = (((w - mlon) * 1e4).floor() / 1e4, ((e + mlon) * 1e4).ceil() / 1e4, ((s - mlat) * 1e4).floor() / 1e4, ((n + mlat) * 1e4).ceil() / 1e4);
    let rgb = Grid::new(&frame, w, n, e, s, RGB_M);
    let cir = Grid::new(&frame, w, n, e, s, CIR_M);
    Ctx { frame, city, rgb, cir, dir: aerial_dir() }
}

/// Where the tables go: `EO_ROOF_SURVEY_OUT`, else this workspace's `native/assets`.
fn out_dir() -> PathBuf {
    std::env::var("EO_ROOF_SURVEY_OUT").map(PathBuf::from).unwrap_or_else(|_| rc::native_assets())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("plan") => plan(),
        Some("survey") => survey(),
        Some("overlay") => overlay(Path::new(args.get(1).map(|s| s.as_str()).unwrap_or(".")), &args[2.min(args.len())..]),
        _ => eprintln!("usage: roof_survey plan | survey | overlay <out dir>"),
    }
}

// ---------------------------------------------------------------------------
// plan: the tiles that hold buildings.
// ---------------------------------------------------------------------------

fn needed_tiles(ctx: &Ctx, g: &Grid) -> Vec<(usize, usize)> {
    let mut need = std::collections::BTreeSet::new();
    let margin = 60.0;
    for b in &ctx.city.buildings {
        let (mut x0, mut x1, mut z0, mut z1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in &b.ring {
            x0 = x0.min(p[0] as f64);
            x1 = x1.max(p[0] as f64);
            z0 = z0.min(p[1] as f64);
            z1 = z1.max(p[1] as f64);
        }
        let (lw, ln) = ctx.frame.to_lonlat(x0 - margin, z0 - margin);
        let (le, ls) = ctx.frame.to_lonlat(x1 + margin, z1 + margin);
        let (a, b0) = g.px(lw, ln);
        let (c, d) = g.px(le, ls);
        let (c0, c1) = ((a.max(0.0) as usize) / TILE, ((c.max(0.0) as usize) / TILE).min(g.cols - 1));
        let (r0, r1) = ((b0.max(0.0) as usize) / TILE, ((d.max(0.0) as usize) / TILE).min(g.rows - 1));
        for r in r0..=r1 {
            for c in c0..=c1 {
                need.insert((r, c));
            }
        }
    }
    need.into_iter().collect()
}

fn plan() {
    let ctx = load_ctx();
    std::fs::create_dir_all(&ctx.dir).unwrap();
    let mut tiles = vec![];
    for (set, g, bands) in [("rgb", &ctx.rgb, "0,1,2"), ("cir", &ctx.cir, "3,0,1")] {
        let need = needed_tiles(&ctx, g);
        eprintln!("{set}: grid {} x {} tiles of {TILE} px, {} hold buildings", g.cols, g.rows, need.len());
        for (r, c) in need {
            let bb = g.tile_bbox(r, c);
            let name = format!("{set}/r{r:03}_c{c:03}.jpg");
            let url = format!(
                "{SERVICE}?bbox={:.9},{:.9},{:.9},{:.9}&bboxSR=4326&imageSR=4326&size={TILE},{TILE}&format=jpg&compressionQuality=90&bandIds={bands}&interpolation=RSP_BilinearInterpolation&f=image",
                bb[0], bb[1], bb[2], bb[3]
            );
            tiles.push(serde_json::json!({ "file": name, "url": url, "bbox": bb }));
        }
    }
    let grid = |g: &Grid, m: f64| serde_json::json!({ "west": g.west, "north": g.north, "dlon": g.dlon, "dlat": g.dlat, "cols": g.cols, "rows": g.rows, "metres": m, "tile": TILE });
    let doc = serde_json::json!({
        "frame": { "centerLat": ctx.frame.center_lat, "centerLon": ctx.frame.center_lon, "metersPerDegreeLat": ctx.frame.m_per_deg_lat, "metersPerDegreeLon": ctx.frame.m_per_deg_lon },
        "rgb": grid(&ctx.rgb, RGB_M), "cir": grid(&ctx.cir, CIR_M), "tiles": tiles,
    });
    let p = ctx.dir.join("tiles.json");
    std::fs::write(&p, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    eprintln!("wrote {}", p.display());
}

// ---------------------------------------------------------------------------
// Tiles in memory.
// ---------------------------------------------------------------------------

struct Img {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

/// Dark-object subtraction: the photo's haze (light scattered into every
/// pixel, which lifts dark roofs toward blue-grey) estimated per tile and
/// channel as the 0.2 % darkest level (deep shadow) and taken out in linear
/// light (per channel: with one amount for all three, the measured greys stay
/// blue; per channel, the city's median roof comes out near neutral). `EO_ROOF_DEHAZE=off` keeps the pixels as delivered.
/// The haze level is one for the whole survey (the median over a sample of
/// tiles, `set_haze`), so no tile edge shows as a step in roof colour.
static HAZE: std::sync::OnceLock<[f32; 3]> = std::sync::OnceLock::new();

fn dehaze(img: &mut Img) {
    if matches!(std::env::var("EO_ROOF_DEHAZE").as_deref(), Ok("off") | Ok("0")) {
        return;
    }
    let d = match HAZE.get() {
        Some(d) => *d,
        None => match haze_of(img) {
            Some(d) => d,
            None => return,
        },
    };
    apply_haze(img, d);
}

/// The tile's haze per channel (linear), from its 0.2 % darkest pixels.
fn haze_of(img: &Img) -> Option<[f32; 3]> {
    let mut hist = [[0u32; 256]; 3];
    let mut n = 0u32;
    for p in img.px.chunks_exact(3) {
        if p == [0, 0, 0] {
            continue; // no-data fill
        }
        for k in 0..3 {
            hist[k][p[k] as usize] += 1;
        }
        n += 1;
    }
    if n < 10_000 {
        return None;
    }
    let mut out = [0f32; 3];
    for k in 0..3 {
        let mut acc = 0;
        let mut dark = 0;
        for (v, c) in hist[k].iter().enumerate() {
            acc += c;
            if acc as f32 >= n as f32 * 0.002 {
                dark = v;
                break;
            }
        }
        out[k] = rc::srgb_to_linear(dark as f32 / 255.0).min(0.2);
    }
    Some(out)
}

fn apply_haze(img: &mut Img, haze: [f32; 3]) {
    let mut lut = [[0u8; 256]; 3];
    for k in 0..3 {
        let d = haze[k];
        for v in 0..256 {
            let l = rc::srgb_to_linear(v as f32 / 255.0);
            lut[k][v] = (rc::linear_to_srgb(((l - d) / (1.0 - d)).max(0.0)) * 255.0).round() as u8;
        }
    }
    for p in img.px.chunks_exact_mut(3) {
        if p != [0, 0, 0] {
            for k in 0..3 {
                p[k] = lut[k][p[k] as usize];
            }
        }
    }
}

fn decode_jpeg(path: &Path) -> Option<Img> {
    let f = std::fs::File::open(path).ok()?;
    let mut d = jpeg_decoder::Decoder::new(std::io::BufReader::new(f));
    let px = d.decode().ok()?;
    let info = d.info()?;
    if info.pixel_format != jpeg_decoder::PixelFormat::RGB24 {
        return None;
    }
    Some(Img { w: info.width as usize, h: info.height as usize, px })
}

/// Decoded tiles of one set, loaded a band of rows at a time.
struct Tiles {
    set: &'static str,
    g: Grid,
    dir: PathBuf,
    map: HashMap<(usize, usize), Arc<Img>>,
}

impl Tiles {
    fn new(set: &'static str, g: Grid, dir: &Path) -> Self {
        Tiles { set, g, dir: dir.to_path_buf(), map: HashMap::new() }
    }
    /// Keep exactly rows r0..=r1 decoded (in parallel).
    fn keep_rows(&mut self, r0: usize, r1: usize) {
        self.map.retain(|k, _| k.0 >= r0 && k.0 <= r1);
        let mut want = vec![];
        for r in r0..=r1.min(self.g.rows.saturating_sub(1)) {
            for c in 0..self.g.cols {
                if !self.map.contains_key(&(r, c)) {
                    let p = self.dir.join(self.set).join(format!("r{r:03}_c{c:03}.jpg"));
                    if p.is_file() {
                        want.push(((r, c), p));
                    }
                }
            }
        }
        let got: Vec<((usize, usize), Option<Img>)> = std::thread::scope(|s| {
            let chunks: Vec<_> = want.chunks(want.len().div_ceil(threads()).max(1)).collect();
            let hs: Vec<_> = chunks.into_iter().map(|ch| s.spawn(move || ch.iter().map(|(k, p)| (*k, decode_jpeg(p))).collect::<Vec<_>>())).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        for (k, img) in got {
            match img {
                Some(mut i) if i.w == TILE && i.h == TILE => {
                    if self.set == "rgb" {
                        dehaze(&mut i);
                    }
                    self.map.insert(k, Arc::new(i));
                }
                _ => eprintln!("{}: tile r{:03}_c{:03} unreadable or wrong size", self.set, k.0, k.1),
            }
        }
    }
    /// The pixel at continuous pixel coordinates (nearest).
    fn at(&self, gx: f64, gy: f64) -> Option<[u8; 3]> {
        if gx < 0.0 || gy < 0.0 {
            return None;
        }
        let (x, y) = (gx as usize, gy as usize);
        let t = self.map.get(&(y / TILE, x / TILE))?;
        let i = ((y % TILE) * t.w + (x % TILE)) * 3;
        Some([t.px[i], t.px[i + 1], t.px[i + 2]])
    }
}

/// The survey's one haze level: the median of the tiles' own, over every 9th tile.
fn set_haze(ctx: &Ctx) {
    let files: Vec<PathBuf> = (0..ctx.rgb.rows).flat_map(|r| (0..ctx.rgb.cols).map(move |c| (r, c))).map(|(r, c)| ctx.dir.join("rgb").join(format!("r{r:03}_c{c:03}.jpg"))).filter(|p| p.is_file()).step_by(9).collect();
    let mut levels: Vec<[f32; 3]> = files.iter().filter_map(|p| decode_jpeg(p)).filter_map(|i| haze_of(&i)).collect();
    if levels.is_empty() {
        return;
    }
    let mut med = [0f32; 3];
    for k in 0..3 {
        levels.sort_by(|a, b| a[k].partial_cmp(&b[k]).unwrap());
        med[k] = levels[levels.len() / 2][k];
    }
    eprintln!("haze (linear, median of {} tiles): {:.4} {:.4} {:.4}", levels.len(), med[0], med[1], med[2]);
    let _ = HAZE.set(med);
}

fn threads() -> usize {
    std::thread::available_parallelism().map(|v| v.get()).unwrap_or(8).saturating_sub(2).max(2)
}

// ---------------------------------------------------------------------------
// Geometry.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Foot {
    key: String,
    ring: Vec<[f64; 2]>,
    min: [f64; 2],
    max: [f64; 2],
    area: f64,
    perimeter: f64,
    height: f64,
    centroid: [f64; 2],
}

fn foot(b: &eo_data::Building) -> Foot {
    let ring: Vec<[f64; 2]> = b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
    let (mut min, mut max) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut per = 0.0;
    let (mut cx, mut cz, mut a2) = (0.0, 0.0, 0.0);
    for i in 0..ring.len() {
        let p = ring[i];
        let q = ring[(i + 1) % ring.len()];
        for k in 0..2 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
        per += (q[0] - p[0]).hypot(q[1] - p[1]);
        let cr = p[0] * q[1] - q[0] * p[1];
        a2 += cr;
        cx += (p[0] + q[0]) * cr;
        cz += (p[1] + q[1]) * cr;
    }
    let area = (a2 * 0.5).abs();
    let centroid = if a2.abs() > 1e-9 { [cx / (3.0 * a2), cz / (3.0 * a2)] } else { [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5] };
    Foot { key: b.id.clone(), ring, min, max, area, perimeter: per, height: b.height as f64, centroid }
}

fn inside(ring: &[[f64; 2]], x: f64, z: f64) -> bool {
    let mut c = false;
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

fn edge_dist(ring: &[[f64; 2]], x: f64, z: f64) -> f64 {
    let mut d = f64::MAX;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let l2 = ex * ex + ez * ez;
        let t = if l2 > 0.0 { (((x - a[0]) * ex + (z - a[1]) * ez) / l2).clamp(0.0, 1.0) } else { 0.0 };
        d = d.min((x - a[0] - t * ex).hypot(z - a[1] - t * ez));
    }
    d
}

/// The inset kept clear of the footprint's edge: a quarter of the mean width, 0.3-1.2 m.
fn inset(f: &Foot) -> f64 {
    (0.25 * 2.0 * f.area / f.perimeter.max(1e-6)).clamp(0.3, 1.2)
}

/// Plan points on the RGB pixel lattice inside the inset footprint shifted by `off`
/// (the roof drawn where the photo shows it), at most about `cap` of them.
fn lattice(ctx: &Ctx, f: &Foot, off: [f64; 2], d: f64, cap: usize) -> Vec<(f64, f64)> {
    let g = &ctx.rgb;
    let (lw, ln) = ctx.frame.to_lonlat(f.min[0] + off[0], f.min[1] + off[1]);
    let (le, ls) = ctx.frame.to_lonlat(f.max[0] + off[0], f.max[1] + off[1]);
    let (x0, y0) = g.px(lw, ln);
    let (x1, y1) = g.px(le, ls);
    let est = f.area / (RGB_M * RGB_M);
    let stride = ((est / cap as f64).sqrt().ceil() as usize).max(1);
    let mut out = vec![];
    let mut gy = y0.floor().max(0.0) as usize;
    while (gy as f64) < y1 {
        let mut gx = x0.floor().max(0.0) as usize;
        while (gx as f64) < x1 {
            let (lon, lat) = g.lonlat(gx as f64 + 0.5, gy as f64 + 0.5);
            let (x, z) = ctx.frame.to_plan(lon, lat);
            let (fx, fz) = (x - off[0], z - off[1]);
            if inside(&f.ring, fx, fz) && edge_dist(&f.ring, fx, fz) >= d {
                out.push((x, z));
            }
            gx += stride;
        }
        gy += stride;
    }
    out
}

fn sample_rgb(ctx: &Ctx, t: &Tiles, x: f64, z: f64) -> Option<[u8; 3]> {
    let (lon, lat) = ctx.frame.to_lonlat(x, z);
    let (gx, gy) = t.g.px(lon, lat);
    t.at(gx, gy)
}

fn lab_of(c: [u8; 3]) -> [f32; 3] {
    rc::oklab(rc::rgb8_linear(c))
}

/// NDVI from a CIR pixel (R = near infrared, G = red).
fn ndvi(c: [u8; 3]) -> f32 {
    let (n, r) = (c[0] as f32, c[1] as f32);
    (n - r) / (n + r).max(1.0)
}

const NDVI_VEG: f32 = 0.12;

// ---------------------------------------------------------------------------
// survey
// ---------------------------------------------------------------------------

/// Per 1 km cell of the city: the median lean of a roof per metre of height
/// (the photo's relief displacement) over the tall buildings whose roofs were
/// found. A diagnostic only (written to roof-survey-summary.txt): the mosaic's
/// frames lean differently within a cell, so roofs use their own offsets.
struct LeanField {
    v: Vec<Option<[f64; 2]>>,
}

const LEAN_CELL: f64 = 1000.0;

/// The offset (metres) at which a tall building's roof lies in the photo
/// (relief displacement): the shift of its footprint, within 0.4 x its
/// height, whose outline sits on the strongest lightness edge (inside 0.9 m
/// against outside 0.9 m) while its inside stays uniform.
fn find_lean(ctx: &Ctx, t: &Tiles, f: &Foot) -> Option<([f64; 2], f64)> {
    let r = (0.4 * f.height).min(60.0);
    let step = (r / 16.0).max(0.9);
    let d = inset(f);
    let base = lattice(ctx, f, [0.0, 0.0], d, 300);
    if base.len() < 40 {
        return None;
    }
    // Pairs across the outline, every metre: (inside point, outside point).
    let sign = {
        let mut a = 0.0;
        for i in 0..f.ring.len() {
            let (p, q) = (f.ring[i], f.ring[(i + 1) % f.ring.len()]);
            a += p[0] * q[1] - q[0] * p[1];
        }
        a.signum()
    };
    let mut pairs = vec![];
    for i in 0..f.ring.len() {
        let (a, b) = (f.ring[i], f.ring[(i + 1) % f.ring.len()]);
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let len = ex.hypot(ez);
        if len < 2.0 {
            continue;
        }
        let n = [-ez / len * sign * 0.9, ex / len * sign * 0.9];
        let steps = len.floor() as usize;
        for s in 1..steps {
            let t = s as f64 / steps as f64;
            let p = [a[0] + ex * t, a[1] + ez * t];
            pairs.push(([p[0] + n[0], p[1] + n[1]], [p[0] - n[0], p[1] - n[1]]));
        }
    }
    if pairs.len() < 20 {
        return None;
    }
    let l_at = |x: f64, z: f64| -> Option<f64> { sample_rgb(ctx, t, x, z).map(|c| lab_of(c)[0] as f64) };
    let score = |o: [f64; 2]| -> Option<f64> {
        let (mut s, mut s2, mut n) = (0.0, 0.0, 0.0);
        for &(x, z) in &base {
            let l = l_at(x + o[0], z + o[1])?;
            s += l;
            s2 += l * l;
            n += 1.0;
        }
        let sd = (s2 / n - (s / n).powi(2)).max(0.0).sqrt();
        let mut c = 0.0;
        for (pi, po) in &pairs {
            c += (l_at(pi[0] + o[0], pi[1] + o[1])? - l_at(po[0] + o[0], po[1] + o[1])?).abs();
        }
        Some(sd - 1.5 * c / pairs.len() as f64)
    };
    let s0 = score([0.0, 0.0])?;
    let mut best = ([0.0, 0.0], s0);
    let k = (r / step).ceil() as i32;
    for j in -k..=k {
        for i in -k..=k {
            let o = [i as f64 * step, j as f64 * step];
            if o[0].hypot(o[1]) > r {
                continue;
            }
            if let Some(s) = score(o) {
                if s < best.1 {
                    best = (o, s);
                }
            }
        }
    }
    // Only a clear win over the footprint as drawn counts.
    (best.1 < s0 - 0.04).then_some((best.0, best.1))
}

fn trimmed_median(v: &mut [f32]) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

struct Measured {
    measure: RoofMeasure,
    samples: usize,
}

fn measure(ctx: &Ctx, rgb: &Tiles, cir: &Tiles, f: &Foot, lean: [f64; 2], lean_known: bool) -> Option<Measured> {
    let off = [lean[0] * f.height, lean[1] * f.height];
    let d = inset(f);
    let mut pts = lattice(ctx, f, off, d, 2500);
    if pts.len() < 12 {
        // Small or thin: no inset.
        pts = lattice(ctx, f, off, 0.0, 2500);
    }
    if pts.is_empty() {
        return None;
    }
    let mut labs = Vec::with_capacity(pts.len());
    let mut veg = 0usize;
    let mut total = 0usize;
    for &(x, z) in &pts {
        let Some(c) = sample_rgb(ctx, rgb, x, z) else { continue };
        total += 1;
        let is_veg = match sample_rgb(ctx, cir, x, z) {
            Some(n) => ndvi(n) > NDVI_VEG,
            None => {
                let l = rc::rgb8_linear(c);
                (2.0 * l[1] - l[0] - l[2]) / (l[0] + l[1] + l[2] + 1e-4) > 0.15
            }
        };
        if is_veg {
            veg += 1;
        } else {
            labs.push((lab_of(c), [x - off[0], z - off[1]]));
        }
    }
    if total == 0 {
        return None;
    }
    let veg_share = veg as f32 / total as f32;
    if labs.len() < 6 {
        return Some(Measured {
            measure: RoofMeasure { rgb: [0; 3], confidence: 0.0, second: [0; 3], second_share: 0.0, second_at: [0.0; 2], vegetation: veg_share },
            samples: labs.len(),
        });
    }
    // Shadow and glint trim: drop the darkest 30 % and brightest 8 % by lightness.
    labs.sort_by(|a, b| a.0[0].partial_cmp(&b.0[0]).unwrap());
    let n = labs.len();
    let (lo, hi) = ((n as f32 * 0.30) as usize, n - (n as f32 * 0.08) as usize);
    let kept = &labs[lo..hi.max(lo + 1)];
    let mut ch: [Vec<f32>; 3] = [vec![], vec![], vec![]];
    for (l, _) in kept {
        for k in 0..3 {
            ch[k].push(l[k]);
        }
    }
    let med = [trimmed_median(&mut ch[0]), trimmed_median(&mut ch[1]), trimmed_median(&mut ch[2])];
    let iqr = {
        let l = &ch[0];
        l[(l.len() * 3) / 4] - l[l.len() / 4]
    };
    // Two clusters over every non-vegetation sample but the darkest 15 % (shadow).
    let pool = &labs[(n as f32 * 0.15) as usize..];
    let mut c = [pool[pool.len() / 4].0, pool[(pool.len() * 3) / 4].0];
    let mut at = [[0f64; 2]; 2];
    let mut counts = [0usize; 2];
    for _ in 0..8 {
        let mut acc = [[0f32; 3]; 2];
        counts = [0, 0];
        at = [[0.0; 2]; 2];
        for (l, p) in pool {
            let d0: f32 = (0..3).map(|k| (l[k] - c[0][k]).powi(2)).sum();
            let d1: f32 = (0..3).map(|k| (l[k] - c[1][k]).powi(2)).sum();
            let w = (d1 < d0) as usize;
            counts[w] += 1;
            at[w][0] += p[0];
            at[w][1] += p[1];
            for k in 0..3 {
                acc[w][k] += l[k];
            }
        }
        for w in 0..2 {
            if counts[w] > 0 {
                c[w] = acc[w].map(|v| v / counts[w] as f32);
            }
        }
    }
    let dist = |a: [f32; 3], b: [f32; 3]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt();
    let far = if dist(c[0], med) > dist(c[1], med) { 0 } else { 1 };
    let share = counts[far] as f32 / pool.len() as f32;
    let (second, second_share) = if share >= 0.12 && dist(c[0], c[1]) > 0.07 { (c[far], share) } else { (med, 0.0) };
    let unit = 0.5 * f.area.sqrt().max(1.0);
    let second_at = if second_share > 0.0 && counts[far] > 0 {
        let n = counts[far] as f64;
        [((at[far][0] / n - f.centroid[0]) / unit) as f32, ((at[far][1] / n - f.centroid[1]) / unit) as f32]
    } else {
        [0.0; 2]
    };
    let to8 = |l: [f32; 3]| rc::oklab_to_linear(l).map(|v| (rc::linear_to_srgb(v) * 255.0).round() as u8);
    // Confidence: enough clean samples, a tight spread, few trees, a roof where we looked.
    let c_n = ((kept.len() as f32 - 10.0) / 120.0).clamp(0.0, 1.0);
    let c_s = (1.0 - (iqr - 0.10) / 0.25).clamp(0.25, 1.0);
    let c_v = (1.0 - veg_share * 1.2).clamp(0.0, 1.0);
    let c_l = if f.height <= 18.0 || lean_known { (1.15 - f.height as f32 / 300.0).clamp(0.5, 1.0) } else { (1.0 - (f.height as f32 - 18.0) / 60.0).clamp(0.25, 1.0) };
    let confidence = (c_n * c_s * c_v * c_l).clamp(0.0, 1.0);
    Some(Measured { measure: RoofMeasure { rgb: to8(med), confidence, second: if second_share > 0.0 { to8(second) } else { [0; 3] }, second_share, second_at, vegetation: veg_share }, samples: kept.len() })
}

fn row_of(ctx: &Ctx, g: &Grid, z: f64) -> usize {
    let (_, lat) = ctx.frame.to_lonlat(0.0, z);
    let (_, gy) = g.px(ctx.frame.center_lon, lat);
    (gy.max(0.0) as usize) / TILE
}

/// Run `work` over the buildings grouped by RGB tile row, with that row and its
/// neighbours (and the CIR rows under them) decoded.
fn by_rows<T: Send>(ctx: &Ctx, feet: &[Foot], label: &str, work: &(dyn Fn(&Tiles, &Tiles, usize) -> Option<T> + Sync)) -> Vec<Option<T>> {
    let mut rgb = Tiles::new("rgb", ctx.rgb, &ctx.dir);
    let mut cir = Tiles::new("cir", ctx.cir, &ctx.dir);
    let mut out: Vec<Option<T>> = (0..feet.len()).map(|_| None).collect();
    let mut rows: Vec<Vec<usize>> = vec![vec![]; ctx.rgb.rows];
    for (i, f) in feet.iter().enumerate() {
        rows[row_of(ctx, &ctx.rgb, f.centroid[1]).min(ctx.rgb.rows - 1)].push(i);
    }
    let t0 = std::time::Instant::now();
    for (r, list) in rows.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        rgb.keep_rows(r.saturating_sub(1), r + 1);
        // CIR rows spanning the same latitudes (+-1 RGB tile).
        let z_n = feet[list[0]].centroid[1];
        let cr = row_of(ctx, &ctx.cir, z_n);
        cir.keep_rows(cr.saturating_sub(1), cr + 1);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let res: Vec<(usize, Option<T>)> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..threads())
                .map(|_| {
                    let (rgb, cir, next) = (&rgb, &cir, &next);
                    s.spawn(move || {
                        let mut v = vec![];
                        loop {
                            let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if k >= list.len() {
                                break;
                            }
                            v.push((list[k], work(rgb, cir, list[k])));
                        }
                        v
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        for (i, v) in res {
            out[i] = v;
        }
        eprintln!("{label}: row {r}/{} ({} buildings) {:.0} s", ctx.rgb.rows, list.len(), t0.elapsed().as_secs_f64());
    }
    out
}

fn manifest_digest(dir: &Path) -> u32 {
    use sha2::Digest;
    let b = std::fs::read(dir.join("manifest.json")).unwrap_or_default();
    let d = sha2::Sha256::digest(&b);
    u32::from_le_bytes([d[0], d[1], d[2], d[3]])
}

fn survey() {
    let ctx = load_ctx();
    let feet: Vec<Foot> = ctx.city.buildings.iter().map(foot).collect();
    eprintln!("{} footprints (1:1 plan)", feet.len());
    set_haze(&ctx);
    // 1. The lean field from tall buildings.
    let tall: Vec<usize> = (0..feet.len()).filter(|&i| feet[i].height >= 24.0 && feet[i].area >= 250.0).collect();
    let tall_feet: Vec<Foot> = tall.iter().map(|&i| feet[i].clone()).collect();
    let leans = by_rows(&ctx, &tall_feet, "lean", &|rgb, _cir, i| find_lean(&ctx, rgb, &tall_feet[i]).map(|(o, _)| [o[0] / tall_feet[i].height, o[1] / tall_feet[i].height]));
    let (mut x0, mut z0, mut x1, mut z1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for f in &feet {
        x0 = x0.min(f.min[0]);
        z0 = z0.min(f.min[1]);
        x1 = x1.max(f.max[0]);
        z1 = z1.max(f.max[1]);
    }
    let nx = ((x1 - x0) / LEAN_CELL).ceil() as usize + 1;
    let nz = ((z1 - z0) / LEAN_CELL).ceil() as usize + 1;
    let mut cells: Vec<Vec<[f64; 2]>> = vec![vec![]; nx * nz];
    let mut found = 0;
    for (k, l) in leans.iter().enumerate() {
        if let Some(l) = l {
            let f = &tall_feet[k];
            let i = ((f.centroid[0] - x0) / LEAN_CELL) as usize;
            let j = ((f.centroid[1] - z0) / LEAN_CELL) as usize;
            cells[j * nx + i].push(*l);
            found += 1;
        }
    }
    let med2 = |v: &[[f64; 2]]| -> [f64; 2] {
        let mut a: Vec<f64> = v.iter().map(|p| p[0]).collect();
        let mut b: Vec<f64> = v.iter().map(|p| p[1]).collect();
        a.sort_by(|p, q| p.partial_cmp(q).unwrap());
        b.sort_by(|p, q| p.partial_cmp(q).unwrap());
        [a[a.len() / 2], b[b.len() / 2]]
    };
    let mut field = LeanField { v: vec![None; nx * nz] };
    for j in 0..nz {
        for i in 0..nx {
            // The cell's own tall buildings if it has 6, else the 3 x 3 cells round it if they have 10.
            let own = &cells[j * nx + i];
            let mut pool: Vec<[f64; 2]> = own.clone();
            if pool.len() < 6 {
                pool.clear();
                for dj in -1i64..=1 {
                    for di in -1i64..=1 {
                        let (a, b) = (i as i64 + di, j as i64 + dj);
                        if a >= 0 && b >= 0 && (a as usize) < nx && (b as usize) < nz {
                            pool.extend(&cells[b as usize * nx + a as usize]);
                        }
                    }
                }
                if pool.len() < 10 {
                    continue;
                }
            }
            field.v[j * nx + i] = Some(med2(&pool));
        }
    }
    let known = field.v.iter().filter(|v| v.is_some()).count();
    eprintln!("lean: {} tall buildings, roof found offset on {found}; {known} of {} km cells have a lean", tall.len(), nx * nz);
    let mut lean_log = String::new();
    for j in 0..nz {
        for i in 0..nx {
            if let Some(v) = field.v[j * nx + i] {
                lean_log += &format!("cell x {:.0} z {:.0}: {:.3} {:.3} m per m ({} own samples)\n", x0 + i as f64 * LEAN_CELL, z0 + j as f64 * LEAN_CELL, v[0], v[1], cells[j * nx + i].len());
            }
        }
    }
    // 2. Every roof.
    // The photo is a mosaic of frames whose lean changes within a cell (the
    // per-cell medians above stay near zero downtown, where towers clearly
    // lean), so a tall building uses its own found offset; every other roof
    // is sampled on its footprint (the inset absorbs a low roof's lean).
    let own_lean: HashMap<usize, [f64; 2]> = tall.iter().zip(&leans).filter_map(|(&i, l)| l.map(|l| (i, l))).collect();
    let measured = by_rows(&ctx, &feet, "roofs", &|rgb, cir, i| {
        let f = &feet[i];
        match own_lean.get(&i) {
            Some(l) => measure(&ctx, rgb, cir, f, *l, true),
            None => measure(&ctx, rgb, cir, f, [0.0, 0.0], false),
        }
    });
    let mut entries = vec![];
    let (mut any, mut confident, mut none) = (0, 0, 0);
    for (f, m) in feet.iter().zip(&measured) {
        match m {
            Some(m) if m.samples > 0 => {
                any += 1;
                if m.measure.confidence >= 0.5 {
                    confident += 1;
                }
                entries.push(RawEntry::from_measure(rc::key_hash(&f.key), &m.measure));
            }
            _ => none += 1,
        }
    }
    let digest = manifest_digest(&ctx.dir);
    let table = RoofTable::from_entries(entries, digest);
    let out = out_dir();
    std::fs::create_dir_all(&out).unwrap();
    let gz = table.to_gzip();
    let p = out.join("roof-colours-v1.bin.gz");
    std::fs::write(&p, &gz).unwrap();
    let summary = format!(
        "{} footprints: {any} measured, {confident} with confidence >= 0.5, {none} without imagery or samples; table {} records ({} dropped as hash collisions), {} bytes gzipped ({:.2} bytes/building); imagery digest {digest:08x}\n",
        feet.len(),
        table.len(),
        any - table.len(),
        gz.len(),
        gz.len() as f64 / table.len().max(1) as f64
    );
    eprint!("{summary}");
    let lean_json: serde_json::Map<String, serde_json::Value> = own_lean.iter().map(|(&i, l)| (feet[i].key.clone(), serde_json::json!([l[0] * feet[i].height, l[1] * feet[i].height]))).collect();
    std::fs::write(ctx.dir.join("lean.json"), serde_json::to_vec(&lean_json).unwrap()).ok();
    std::fs::write(ctx.dir.join("roof-survey-summary.txt"), format!("{summary}\nlean field (roof offset per metre of height, x east z south):\n{lean_log}")).ok();
    eprintln!("wrote {}", p.display());
    // 3. The green share of the ground between buildings.
    green_grid(&ctx, &feet, &out);
}

/// The ground between buildings, per 16 m cell: vegetation's share (NDVI on
/// the CIR tiles, every other 0.6 m pixel) and the footprints' share.
fn green_grid(ctx: &Ctx, feet: &[Foot], out: &Path) {
    const CELL: f64 = 16.0;
    let (mut x0, mut z0, mut x1, mut z1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for f in feet {
        x0 = x0.min(f.min[0]);
        z0 = z0.min(f.min[1]);
        x1 = x1.max(f.max[0]);
        z1 = z1.max(f.max[1]);
    }
    let (x0, z0) = ((x0 / CELL).floor() * CELL - CELL * 4.0, (z0 / CELL).floor() * CELL - CELL * 4.0);
    let nx = ((x1 - x0) / CELL).ceil() as usize + 4;
    let nz = ((z1 - z0) / CELL).ceil() as usize + 4;
    // Buildings bucketed by cell.
    let mut bucket: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for (k, f) in feet.iter().enumerate() {
        for j in ((f.min[1] - z0) / CELL).floor() as i64..=((f.max[1] - z0) / CELL).floor() as i64 {
            for i in ((f.min[0] - x0) / CELL).floor() as i64..=((f.max[0] - x0) / CELL).floor() as i64 {
                bucket.entry((i, j)).or_default().push(k as u32);
            }
        }
    }
    let mut cir = Tiles::new("cir", ctx.cir, &ctx.dir);
    let mut acc = vec![[0u32; 3]; nx * nz]; // open ground samples, open green samples, built samples
    let g = ctx.cir;
    for r in 0..g.rows {
        cir.keep_rows(r, r);
        if cir.map.is_empty() {
            continue;
        }
        let ys: Vec<usize> = (r * TILE..(r + 1) * TILE).step_by(2).collect();
        let parts: Vec<Vec<(usize, u8)>> = std::thread::scope(|s| {
            let chunks: Vec<&[usize]> = ys.chunks(ys.len().div_ceil(threads())).collect();
            let hs: Vec<_> = chunks
                .into_iter()
                .map(|ch| {
                    let (cir, bucket) = (&cir, &bucket);
                    s.spawn(move || {
                        let mut v = vec![];
                        for &gy in ch {
                            for gx in (0..g.cols * TILE).step_by(2) {
                                let Some(c) = cir.at(gx as f64 + 0.5, gy as f64 + 0.5) else { continue };
                                if c == [0, 0, 0] {
                                    continue;
                                }
                                let (lon, lat) = g.lonlat(gx as f64 + 0.5, gy as f64 + 0.5);
                                let (x, z) = ctx.frame.to_plan(lon, lat);
                                let (i, j) = (((x - x0) / CELL).floor() as i64, ((z - z0) / CELL).floor() as i64);
                                if i < 0 || j < 0 || i as usize >= nx || j as usize >= nz {
                                    continue;
                                }
                                let built = bucket.get(&(i, j)).is_some_and(|l| {
                                    l.iter().any(|&k| {
                                        let f = &feet[k as usize];
                                        x >= f.min[0] && x <= f.max[0] && z >= f.min[1] && z <= f.max[1] && inside(&f.ring, x, z)
                                    })
                                });
                                let code = if built { 2 } else if ndvi(c) > NDVI_VEG { 1 } else { 0 };
                                v.push((j as usize * nx + i as usize, code));
                            }
                        }
                        v
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for p in parts {
            for (cell, code) in p {
                match code {
                    2 => acc[cell][2] += 1,
                    1 => {
                        acc[cell][0] += 1;
                        acc[cell][1] += 1
                    }
                    _ => acc[cell][0] += 1,
                }
            }
        }
        eprintln!("green: cir row {r}/{}", g.rows);
    }
    let mut grid = rc::GreenGrid { x0: x0 as f32, z0: z0 as f32, cell: CELL as f32, nx: nx as u32, nz: nz as u32, green: vec![255; nx * nz], built: vec![0; nx * nz] };
    let (mut open_total, mut green_total) = (0u64, 0u64);
    for (k, a) in acc.iter().enumerate() {
        let total = a[0] + a[2];
        if a[0] >= 4 {
            grid.green[k] = ((a[1] as f64 / a[0] as f64) * 250.0).round() as u8;
        }
        if total > 0 {
            grid.built[k] = ((a[2] as f64 / total as f64) * 250.0).round() as u8;
        }
        open_total += a[0] as u64;
        green_total += a[1] as u64;
    }
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut e, &grid.encode()).unwrap();
    let gz = e.finish().unwrap();
    let p = out.join("ground-green-v1.bin.gz");
    std::fs::write(&p, &gz).unwrap();
    eprintln!("green grid {nx} x {nz} cells of {CELL} m: open ground {:.1} % green overall; {} bytes; wrote {}", 100.0 * green_total as f64 / open_total.max(1) as f64, gz.len(), p.display());
}

// ---------------------------------------------------------------------------
// overlay: evidence pictures.
// ---------------------------------------------------------------------------

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) {
    let f = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(rgb).unwrap();
}

fn line(img: &mut [u8], w: usize, h: usize, a: (f64, f64), b: (f64, f64), c: [u8; 3]) {
    let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
    for s in 0..=n {
        let t = s as f64 / n as f64;
        let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        if x >= 0.0 && y >= 0.0 && (x as usize) < w && (y as usize) < h {
            let i = (y as usize * w + x as usize) * 3;
            img[i..i + 3].copy_from_slice(&c);
        }
    }
}

/// `extra`: more places as name lon lat triples (the four below are always drawn).
fn overlay(out: &Path, extra: &[String]) {
    // The pictures show the photo as delivered (the survey measures it dehazed).
    // SAFETY: before any thread starts.
    unsafe { std::env::set_var("EO_ROOF_DEHAZE", "off") };
    let ctx = load_ctx();
    std::fs::create_dir_all(out).unwrap();
    let table = RoofTable::read(&out_dir().join("roof-colours-v1.bin.gz")).ok();
    // The lean field the survey used (roof offset per metre of height), to draw tall roofs where the photo shows them.
    let lean: Option<serde_json::Value> = std::fs::read(ctx.dir.join("lean.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let lean_of = |key: &str| -> [f64; 2] {
        let v = lean.as_ref().map(|l| &l[key]);
        match v {
            Some(serde_json::Value::Array(a)) if a.len() == 2 => [a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)],
            _ => [0.0; 2],
        }
    };
    let feet: Vec<Foot> = ctx.city.buildings.iter().map(foot).collect();
    let mut places: Vec<(String, f64, f64)> = extra.chunks_exact(3).map(|c| (c[0].clone(), c[1].parse().unwrap(), c[2].parse().unwrap())).collect();
    for (n, lon, lat) in [
        ("civic-center", -122.4185, 37.7790),
        ("western-addition", -122.4335, 37.7765),
        ("outer-sunset", -122.4950, 37.7530),
        ("downtown", -122.3990, 37.7905),
    ] {
        places.push((n.to_string(), lon, lat));
    }
    let half = 150.0; // metres
    for (name, lon, lat) in places {
        let name = name.as_str();
        let (cx, cz) = ctx.frame.to_plan(lon, lat);
        let n = (2.0 * half / RGB_M) as usize;
        let mut rgb = Tiles::new("rgb", ctx.rgb, &ctx.dir);
        let (_, gy) = ctx.rgb.px(lon, lat);
        let r = gy as usize / TILE;
        rgb.keep_rows(r.saturating_sub(1), r + 1);
        let mut img = vec![0u8; n * n * 3];
        for y in 0..n {
            for x in 0..n {
                let (px, pz) = (cx - half + (x as f64 + 0.5) * RGB_M, cz - half + (y as f64 + 0.5) * RGB_M);
                if let Some(c) = sample_rgb(&ctx, &rgb, px, pz) {
                    let i = (y * n + x) * 3;
                    img[i..i + 3].copy_from_slice(&c);
                }
            }
        }
        let mut swatched = img.clone();
        let to_img = |x: f64, z: f64| ((x - (cx - half)) / RGB_M, (z - (cz - half)) / RGB_M);
        for f in &feet {
            if f.max[0] < cx - half || f.min[0] > cx + half || f.max[1] < cz - half || f.min[1] > cz + half {
                continue;
            }
            for i in 0..f.ring.len() {
                let (a, b) = (f.ring[i], f.ring[(i + 1) % f.ring.len()]);
                line(&mut img, n, n, to_img(a[0], a[1]), to_img(b[0], b[1]), [255, 230, 0]);
                // Tall buildings: the footprint moved by the photo's lean (where the survey sampled the roof).
                let o = lean_of(&f.key);
                if o[0].hypot(o[1]) > 1.5 {
                    line(&mut img, n, n, to_img(a[0] + o[0], a[1] + o[1]), to_img(b[0] + o[0], b[1] + o[1]), [0, 230, 255]);
                }
            }
            // The measured colour as a disc at the centroid (outlined white).
            if let Some(m) = table.as_ref().and_then(|t| t.get(&f.key)) {
                let (px, py) = to_img(f.centroid[0], f.centroid[1]);
                let rad = (f.area.sqrt() * 0.25 / RGB_M).clamp(3.0, 14.0);
                for y in (py - rad - 1.0) as i64..=(py + rad + 1.0) as i64 {
                    for x in (px - rad - 1.0) as i64..=(px + rad + 1.0) as i64 {
                        if x < 0 || y < 0 || x as usize >= n || y as usize >= n {
                            continue;
                        }
                        let d = ((x as f64 - px).powi(2) + (y as f64 - py).powi(2)).sqrt();
                        let i = (y as usize * n + x as usize) * 3;
                        if d <= rad {
                            swatched[i..i + 3].copy_from_slice(&m.rgb);
                        } else if d <= rad + 1.0 {
                            swatched[i..i + 3].copy_from_slice(&if m.confidence >= 0.5 { [255, 255, 255] } else { [255, 0, 0] });
                        }
                    }
                }
            }
        }
        write_png(&out.join(format!("overlay-{name}.png")), n, n, &img);
        write_png(&out.join(format!("measured-{name}.png")), n, n, &swatched);
        eprintln!("overlay {name}: plan ({cx:.0}, {cz:.0})");
    }
    if let Some(t) = &table {
        swatches(&feet, t, out);
    }
}

/// The measured roof colours by building type (the game's own classification
/// on its drawn plan): one row of swatches per type, sorted by lightness, and
/// a text table of counts.
fn swatches(feet: &[Foot], t: &RoofTable, out: &Path) {
    let root = eo_kit::repo_root().unwrap();
    let (_city, kc) = eo_kit::city::KitCity::load(&root).expect("kit city");
    let mut by: std::collections::BTreeMap<String, Vec<RoofMeasure>> = Default::default();
    let mut counts: std::collections::BTreeMap<String, (usize, usize, usize)> = Default::default();
    let keyed: HashMap<&str, usize> = kc.records.iter().enumerate().map(|(i, r)| (r.key.as_str(), i)).collect();
    for f in feet {
        let Some(&i) = keyed.get(f.key.as_str()) else { continue };
        let ty = if kc.excluded.contains(&i) { "hand-built".to_string() } else { eo_kit::pipeline::classify_loaded(&kc.records[i]).map(|c| c.archetype.to_string()).unwrap_or_else(|| "unclassified".into()) };
        let e = counts.entry(ty.clone()).or_default();
        e.0 += 1;
        if let Some(m) = t.get(&f.key) {
            e.1 += 1;
            if m.confidence >= 0.5 {
                e.2 += 1;
                by.entry(ty).or_default().push(m);
            }
        }
    }
    let cols = 240usize;
    let sw = 4usize;
    let rows = by.len();
    let (w, h) = (cols * sw, rows * 24);
    let mut img = vec![32u8; w * h * 3];
    let mut text = String::from("type\tbuildings\tmeasured\tconfident(>=0.5)\tmedian sRGB of confident\tdark (L<0.5)\tmid\tlight (L>0.75)\n");
    for (row, (ty, list)) in by.iter_mut().enumerate() {
        list.sort_by(|a, b| lab_of(a.rgb)[0].partial_cmp(&lab_of(b.rgb)[0]).unwrap());
        for c in 0..cols {
            let m = list[(c * list.len()) / cols];
            for y in row * 24 + 2..row * 24 + 22 {
                for x in c * sw..(c + 1) * sw {
                    let i = (y * w + x) * 3;
                    img[i..i + 3].copy_from_slice(&m.rgb);
                }
            }
        }
        let med = list[list.len() / 2].rgb;
        let ls: Vec<f32> = list.iter().map(|m| lab_of(m.rgb)[0]).collect();
        let dark = ls.iter().filter(|&&l| l < 0.5).count() as f64 / ls.len() as f64;
        let light = ls.iter().filter(|&&l| l > 0.75).count() as f64 / ls.len() as f64;
        let c = counts[ty.as_str()];
        text += &format!("{ty}\t{}\t{}\t{}\t#{:02x}{:02x}{:02x}\t{:.0}%\t{:.0}%\t{:.0}%\n", c.0, c.1, c.2, med[0], med[1], med[2], dark * 100.0, (1.0 - dark - light) * 100.0, light * 100.0);
    }
    for (ty, c) in &counts {
        if !by.contains_key(ty) {
            text += &format!("{ty}\t{}\t{}\t{}\t-\n", c.0, c.1, c.2);
        }
    }
    text += "\nrows of swatches-by-type.png, top to bottom:\n";
    for ty in by.keys() {
        text += &format!("{ty}\n");
    }
    write_png(&out.join("swatches-by-type.png"), w, h, &img);
    std::fs::write(out.join("swatches-by-type.txt"), text).unwrap();
}
