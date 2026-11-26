//! `planting`: the vegetation planting's NAIP rules counted per 2 m texel
//! citywide (`eo_hills::planting`, stage H6a) into
//! `<cache>/hills/planting-v1.bin`.
//!
//! The citywide 0.3 m RGB tiles (`EO_AERIAL_DIR`, each checked against its
//! manifest digest) are resampled, as delivered (8-bit sRGB values, area
//! weighted, no haze removal), to the Golden Gate survey's own pixel:
//! square in degrees of `eo_farshore::survey::naip::GRID.dlon` (0.600 m east-
//! west, 0.754 m north-south), the pixel the rules were tuned on. Each pixel
//! is classified by the real rules (`eo_hills::planting::pixel_counts`:
//! `naip_shares` over its 5 x 5 pixels) on the slope of the 8 m elevation
//! grid at its texel (degrees, central differences over one cell each way,
//! as eo-app's `site_slope_deg`), and counted in the macro map's 2 m texel
//! holding its centre.
//!
//! With `EO_SURVEY_ROOT` set (the survey's folder), the cook compares the
//! two NAIP sets where they overlap: the per-channel median ratio of the
//! survey's pixels over the resampled ones (a gain is applied to the citywide
//! set when a channel is off by more than `GAIN_TOLERANCE`), then
//! `Planting::shares` against eo-app's `naip_shares_at` on the survey mosaic
//! (its pixel window copied here) at `--samples` random open-ground points
//! (default 3000) with 1, 2.4, 4.8 and 8 m boxes (2.4 and 4.8: the runtime's
//! plant and tree boxes at the 1.25 ground scale).

use anyhow::{Context, Result, bail};
use eo_data::roof_colours::PlanFrame;
use eo_farshore::survey::naip::GRID;
use eo_hills::naip::{self, TILE};
use eo_hills::planting::{self as pl, Planting};
use eo_hills::raster;
use eo_landmarks::vegetation::{NAIP_BARE_SLOPE_DEG, NAIP_BARE_V, Naip, NaipClass, naip_class};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

/// A channel whose survey / citywide median ratio is off 1 by more than this gets a gain.
const GAIN_TOLERANCE: f64 = 0.03;

/// The source pixels (index, weight) a target pixel covers, along one axis:
/// target pixel o spans source [o k, (o + 1) k).
fn spans(n_out: usize, n_src: usize, k: f64) -> Vec<(u32, u8, [f32; 4])> {
    (0..n_out)
        .map(|o| {
            let (a, b) = (o as f64 * k, (o + 1) as f64 * k);
            let (s0, s1) = (a.floor() as usize, (b.ceil() as usize).min(n_src));
            let mut w = [0f32; 4];
            let n = (s1 - s0).min(4);
            for (q, s) in (s0..s0 + n).enumerate() {
                w[q] = ((s as f64 + 1.0).min(b) - (s as f64).max(a)).max(0.0) as f32;
            }
            (s0 as u32, n as u8, w)
        })
        .collect()
}

/// The ground's slope (degrees) at plan (x, z) across one 8 m cell each way.
fn slope_deg(elev: &eo_data::ElevationGrid, x: f64, z: f64) -> f32 {
    let c = elev.meta.cell_size;
    let s = |x: f64, z: f64| elev.sample_surveyed(x, z).filter(|v| v.is_finite());
    let Some(y) = s(x, z) else { return 0.0 };
    let h = |dx: f64, dz: f64| s(x + dx, z + dz).unwrap_or(y);
    let (gx, gz) = ((h(c, 0.0) - h(-c, 0.0)) / (2.0 * c), (h(0.0, c) - h(0.0, -c)) / (2.0 * c));
    (gx * gx + gz * gz).sqrt().atan().to_degrees() as f32
}

// ---- eo-app props/vegetation.rs's survey window, copied (surveyed plan, scale 1) ----

/// eo-app `naip_pixel`: the survey pixel (column, GRID row) holding (lon, lat).
fn naip_pixel(lon: f64, lat: f64) -> Option<(i64, i64)> {
    let n = GRID.tile as f64;
    let tile_lat = n * GRID.dlat;
    let t = ((GRID.north - lat) / tile_lat).floor();
    let c = ((lon - GRID.west) / GRID.dlon).floor();
    if t < 0.0 || t >= GRID.rows as f64 || c < 0.0 || c >= (GRID.cols * GRID.tile) as f64 {
        return None;
    }
    let centre = GRID.north - (t + 0.5) * tile_lat;
    let r = (n / 2.0 + (centre - lat) / GRID.dlon).floor().clamp(0.0, n - 1.0);
    Some((c as i64, t as i64 * GRID.tile as i64 + r as i64))
}

/// eo-app `naip_rgb` (the registered reader): the survey pixel (c, r)'s colour.
fn naip_rgb(n: &eo_farshore::survey::naip::Naip, c: i64, r: i64) -> Option<[u8; 3]> {
    if c < 0 || r < 0 {
        return None;
    }
    let lon = GRID.west + (c as f64 + 0.5) * GRID.dlon;
    let (t, rr) = ((r / GRID.tile as i64) as f64, (r % GRID.tile as i64) as f64);
    let lat = GRID.north - (t + 0.5) * GRID.tile as f64 * GRID.dlat + (GRID.tile as f64 / 2.0 - rr - 0.5) * GRID.dlon;
    n.sample_rgb(lon, lat)
}

/// eo-app `naip_box` at scale 1 (`side` real metres): odd columns and rows, 1 to 15.
fn naip_box(side: f64) -> (i64, i64) {
    let odd = |v: f64| {
        let k = (v.round() as i64).clamp(1, 15);
        if k % 2 == 0 { k + 1 } else { k }
    };
    (odd(side / 0.6), odd(side / (GRID.dlon * 110_574.0)))
}

/// eo-app `naip_shares_at` on surveyed plan metres.
fn survey_shares(n: &eo_farshore::survey::naip::Naip, frame: &PlanFrame, x: f64, z: f64, side: f64, slope: f64) -> Option<Naip> {
    let (lon, lat) = frame.to_lonlat(x, z);
    let (c0, r0) = naip_pixel(lon, lat)?;
    let (kc, kr) = naip_box(side);
    let (w, h) = (kc + 4, kr + 4);
    let mut px = Vec::with_capacity((w * h) as usize);
    for j in 0..h {
        for i in 0..w {
            px.push(naip_rgb(n, c0 + i - w / 2, r0 + j - h / 2));
        }
    }
    if px.iter().filter(|p| p.is_some()).count() < px.len() / 2 {
        return None;
    }
    eo_landmarks::vegetation::naip_shares(&px, w as usize, h as usize, slope)
}

/// The resampled pixel (pc, pr) (None off the grid or without data).
fn pget(img: &[[u8; 3]], cols: usize, rows: usize, pc: i64, pr: i64) -> Option<[u8; 3]> {
    if pc < 0 || pr < 0 || pc >= cols as i64 || pr >= rows as i64 {
        return None;
    }
    let p = img[pr as usize * cols + pc as usize];
    (p != [0, 0, 0]).then_some(p)
}

const OFF: u32 = u32::MAX;

fn share_vec(n: &Naip) -> [f32; 7] {
    [n.canopy, n.low, n.bare, n.lush, n.soil, n.warm, n.green]
}

const SHARES: [&str; 7] = ["canopy", "low", "bare", "lush", "soil", "warm", "green"];

fn dominant(n: &Naip) -> usize {
    let v = [n.canopy, n.low, n.bare];
    (0..3).max_by(|&a, &b| v[a].total_cmp(&v[b])).unwrap()
}

pub fn run(args: &[String]) -> Result<()> {
    let t0 = Instant::now();
    let mut samples = 3000usize;
    let mut bare_override: Option<f32> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--samples" => samples = it.next().and_then(|s| s.parse().ok()).context("--samples needs a count")?,
            "--bare-v5" => bare_override = Some(it.next().and_then(|s| s.parse().ok()).context("--bare-v5 needs a brightness (0-1)")?),
            _ => bail!("usage: cook planting [--samples n] [--bare-v5 v]   (EO_AERIAL_DIR: the NAIP folder; EO_SURVEY_ROOT: the survey, to validate; EO_NATIVE_CACHE: where the map goes)"),
        }
    }
    // 1. The tiles, the city, the grids.
    let dir = PathBuf::from(std::env::var("EO_AERIAL_DIR").context("set EO_AERIAL_DIR to the folder fetch-aerial.mjs wrote")?);
    let manifest_bytes = std::fs::read(dir.join("manifest.json")).context("reading manifest.json")?;
    let manifest = naip::Manifest::parse(&manifest_bytes)?;
    let (tiles, missing) = super::verify_tiles(&dir, &manifest.tiles, "rgb")?;
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load_at(&paths, 1.0).context("the city at 1:1")?;
    let frame = PlanFrame::from_meta(&city.meta);
    let g = manifest.rgb;
    let pg = super::macro_grid(&g, &frame);
    let (w, h) = (pg.width, pg.height);
    let d = GRID.dlon;
    let k = d / g.dlon;
    let (cols, rows) = ((g.width_px() as f64 / k).floor() as usize, (g.height_px() as f64 / k).floor() as usize);
    let (col_m, row_m) = (d * frame.m_per_deg_lon, d * frame.m_per_deg_lat);
    eprintln!("{} tiles verified ({missing} missing); survey pixels {cols} x {rows} of {col_m:.3} x {row_m:.3} m ({k:.4} citywide pixels a side); texels {w} x {h} ({:.1} s)", tiles.len(), t0.elapsed().as_secs_f64());

    // 2. The citywide set resampled to survey pixels (8-bit sRGB means, area weighted).
    let (xs, ys) = (spans(cols, g.width_px(), k), spans(rows, g.height_px(), k));
    let mut pimg: Vec<[u8; 3]> = vec![[0; 3]; cols * rows];
    let mut decoded: HashMap<(usize, usize), Vec<u8>> = HashMap::new();
    let mut p_next = 0usize;
    for r in 0..g.rows {
        decoded.retain(|key, _| key.0 + 1 >= r);
        let row: Vec<&naip::TileEntry> = tiles.iter().filter(|t| t.r == r).collect();
        let got: Vec<((usize, usize), Result<Vec<u8>>)> = std::thread::scope(|s| {
            let hs: Vec<_> = row.iter().map(|t| s.spawn(|| ((t.r, t.c), super::decode_jpeg(&dir.join(&t.file))))).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (key, px) in got {
            match px {
                Ok(px) => {
                    decoded.insert(key, px);
                }
                Err(e) => eprintln!("tile r{:03}_c{:03}: {e:#}: skipped", key.0, key.1),
            }
        }
        let p_end = (((r + 1) * TILE) as f64 / k).floor().min(rows as f64) as usize;
        if p_end <= p_next {
            continue;
        }
        let (decoded, xs, ys) = (&decoded, &xs, &ys);
        let src = |sx: usize, sy: usize| -> Option<[u8; 3]> {
            let t = decoded.get(&(sy / TILE, sx / TILE))?;
            let i = ((sy % TILE) * TILE + sx % TILE) * 3;
            let p = [t[i], t[i + 1], t[i + 2]];
            (p != [0, 0, 0]).then_some(p)
        };
        eo_hills::par_rows(&mut pimg[p_next * cols..p_end * cols], cols, |r0, chunk| {
            for (o, out) in chunk.iter_mut().enumerate() {
                let (pc, pr) = (o % cols, p_next + r0 + o / cols);
                let (sy0, ny, wy) = ys[pr];
                let (sx0, nx, wx) = xs[pc];
                let (mut acc, mut ws, mut tot) = ([0f32; 3], 0f32, 0f32);
                for a in 0..ny as usize {
                    for b in 0..nx as usize {
                        let wt = wy[a] * wx[b];
                        tot += wt;
                        if let Some(p) = src(sx0 as usize + b, sy0 as usize + a) {
                            for q in 0..3 {
                                acc[q] += wt * p[q] as f32;
                            }
                            ws += wt;
                        }
                    }
                }
                if ws >= 0.5 * tot && ws > 0.0 {
                    let v = acc.map(|a| (a / ws).round().clamp(0.0, 255.0) as u8);
                    *out = if v == [0, 0, 0] { [0, 0, 1] } else { v };
                }
            }
        });
        p_next = p_end;
    }
    drop(decoded);
    eprintln!("resampled ({:.1} s)", t0.elapsed().as_secs_f64());

    // 3. The two NAIP sets where they overlap: per-channel ratio, survey over citywide.
    let survey = std::env::var_os("EO_SURVEY_ROOT").filter(|v| !v.is_empty()).and_then(|v| eo_farshore::survey::Root::at(PathBuf::from(v))).map(|r| eo_farshore::survey::naip::Naip::open(&r));
    let mut validation: BTreeMap<String, f64> = BTreeMap::new();
    let mut gain = [1f32; 3];
    let mut rng = 0x2545_f491_4f6c_dd1du64;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    let sb = GRID.bounds();
    if let Some(n) = &survey {
        let mut ch: [Vec<f32>; 6] = Default::default();
        let mut ratio: [Vec<f32>; 3] = Default::default();
        for _ in 0..400_000 {
            let (lon, lat) = (sb[0] + rand() * (sb[2] - sb[0]), sb[1] + rand() * (sb[3] - sb[1]));
            let (Some(a), Some(b)) = (n.sample_rgb(lon, lat), pget(&pimg, cols, rows, ((lon - g.west) / d).floor() as i64, ((g.north - lat) / d).floor() as i64)) else { continue };
            if a == [0, 0, 0] {
                continue;
            }
            for q in 0..3 {
                ch[q].push(a[q] as f32);
                ch[q + 3].push(b[q] as f32);
                if b[q] >= 20 {
                    ratio[q].push(a[q] as f32 / b[q] as f32);
                }
            }
        }
        let med = |v: &mut Vec<f32>| eo_hills::macro_map::median(v).unwrap_or(f32::NAN);
        let m: Vec<f32> = ch.iter_mut().map(med).collect();
        let pr: Vec<f32> = ratio.iter_mut().map(med).collect();
        eprintln!(
            "overlap ({} pixel pairs): median survey R G B {:.0} {:.0} {:.0}, citywide {:.0} {:.0} {:.0}; ratio of medians {:.3} {:.3} {:.3}; median of pixel ratios {:.3} {:.3} {:.3}",
            ch[0].len(),
            m[0],
            m[1],
            m[2],
            m[3],
            m[4],
            m[5],
            m[0] / m[3],
            m[1] / m[4],
            m[2] / m[5],
            pr[0],
            pr[1],
            pr[2]
        );
        for q in 0..3 {
            let r = m[q] / m[q + 3];
            validation.insert(format!("overlap_ratio_{}", ["r", "g", "b"][q]), r as f64);
        }
        if (0..3).any(|q| ((m[q] / m[q + 3]) as f64 - 1.0).abs() > GAIN_TOLERANCE) {
            gain = [0, 1, 2].map(|q| m[q] / m[q + 3]);
            eprintln!("a channel is off by more than {GAIN_TOLERANCE}: gain {gain:?} on the citywide set");
        }
    }
    if gain != [1.0; 3] {
        eo_hills::par_rows(&mut pimg, cols, |_, rows_| {
            for p in rows_ {
                if *p != [0, 0, 0] {
                    let v = [0, 1, 2].map(|q| (p[q] as f32 * gain[q]).round().clamp(0.0, 255.0) as u8);
                    *p = if v == [0, 0, 0] { [0, 0, 1] } else { v };
                }
            }
        });
    }

    // 4. Every pixel classified, counted in its texel.
    let elev = eo_data::ElevationGrid::decode(&eo_data::read_maybe_gzip(&paths.sf("sf-elevation.bin.gz"))?)?;
    let mut slopes = vec![0f32; w * h];
    eo_hills::par_rows(&mut slopes, w, |j0, rows_| {
        for (o, v) in rows_.iter_mut().enumerate() {
            let (x, z) = pg.centre(o % w, j0 + o / w);
            *v = slope_deg(&elev, x, z);
        }
    });
    let col_i: Vec<u32> = (0..cols)
        .map(|pc| {
            let (x, _) = frame.to_plan(g.west + (pc as f64 + 0.5) * d, g.north);
            let i = ((x - pg.origin_x) / pg.texel).floor();
            if i >= 0.0 && (i as usize) < w { i as u32 } else { OFF }
        })
        .collect();
    let row_j: Vec<u32> = (0..rows)
        .map(|pr| {
            let (_, z) = frame.to_plan(g.west, g.north - (pr as f64 + 0.5) * d);
            let j = ((z - pg.origin_z) / pg.texel).floor();
            if j >= 0.0 && (j as usize) < h { j as u32 } else { OFF }
        })
        .collect();
    // The layers the city calibrations read: footprints, mapped greens, trails and ways, WorldCover.
    let mut built = vec![false; w * h];
    for b in &city.buildings {
        let ring: Vec<[f64; 2]> = b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        if ring.len() >= 3 {
            raster::fill_rings(&pg, &[&ring], |i, j| built[j * w + i] = true);
        }
    }
    let greens = super::green_classes(&paths, &pg)?;
    let (trail, near_way) = super::trails(&city, &pg, &greens);
    let lc = eo_data::LandCover::decode(&eo_data::read_maybe_gzip(&paths.sf("sf-land-cover.bin.gz"))?, eo_data::Frame::sf())?;
    let mut wc = vec![0u8; w * h];
    eo_hills::par_rows(&mut wc, w, |j0, rows_| {
        for (o, v) in rows_.iter_mut().enumerate() {
            let (x, z) = pg.centre(o % w, j0 + o / w);
            *v = lc.class_at(x, z);
        }
    });

    // 4a. The city's bare threshold on the 5 x 5 brightness: cured grass (mapped grassland or
    //     meadow, or a park on a slope over 0.15, away from ways and trails) against mapped bare
    //     ground and trails, each over the pixels whose bare-or-low decision hangs on the
    //     brightness alone (the rules' low or bare, not green, not grey-blue, not steep).
    let park_slope_deg = 0.15f64.atan().to_degrees() as f32;
    let (grass_v, bare_v, mut diag) = {
        let (pimg, col_i, row_j, slopes, greens, trail, near_way, built) = (&pimg, &col_i, &row_j, &slopes, &greens, &trail, &near_way, &built);
        let per = rows.div_ceil(eo_hills::threads());
        let parts: Vec<(Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>)> = std::thread::scope(|sc| {
            let hs: Vec<_> = (0..rows.div_ceil(per))
                .map(|t| {
                    sc.spawn(move || {
                        let (mut gv, mut bv) = (vec![], vec![]);
                        let (mut gr, mut br) = (vec![], vec![]);
                        let pix = |pc: i64, pr: i64| pget(pimg, cols, rows, pc, pr);
                        for pr in t * per..((t + 1) * per).min(rows) {
                            let j = row_j[pr];
                            if j == OFF {
                                continue;
                            }
                            for (pc, &i) in col_i.iter().enumerate().step_by(2) {
                                if i == OFF {
                                    continue;
                                }
                                let k = j as usize * w + i as usize;
                                let c = greens[k];
                                let is_grass = !trail[k] && !near_way[k] && (c == super::C_GRASSLAND || (c == super::C_PARK && slopes[k] > park_slope_deg));
                                if built[k] || !(is_grass || trail[k]) {
                                    continue;
                                }
                                let Some(p) = pix(pc as i64, pr as i64) else { continue };
                                let [r, g, b] = p.map(|v| v as f32);
                                if (2.0 * g - r - b > 0.0 && g > r) || (b >= g && b >= r) || slopes[k] as f64 > NAIP_BARE_SLOPE_DEG {
                                    continue;
                                }
                                let block: [Option<[u8; 3]>; 25] = std::array::from_fn(|q| pix(pc as i64 + (q % 5) as i64 - 2, pr as i64 + (q / 5) as i64 - 2));
                                let Some(v) = pl::v5(&block) else { continue };
                                if !matches!(naip_class(p, v, slopes[k] as f64), NaipClass::Low | NaipClass::Bare) {
                                    continue;
                                }
                                if is_grass { gv.push(v) } else { bv.push(v) }
                                if v > NAIP_BARE_V {
                                    if is_grass { gr.push(r / g.max(1.0)) } else { br.push(r / g.max(1.0)) }
                                }
                            }
                        }
                        (gv, bv, gr, br)
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let (mut gv, mut bv) = (vec![], vec![]);
        let mut diag: [Vec<f32>; 2] = Default::default();
        for (a, b, c, d) in parts {
            gv.extend(a);
            bv.extend(b);
            diag[0].extend(c);
            diag[1].extend(d);
        }
        (gv, bv, diag)
    };
    {
        let q = |v: &mut Vec<f32>| -> String {
            v.sort_by(f32::total_cmp);
            if v.is_empty() {
                return "none".into();
            }
            let at = |f: f64| v[((v.len() - 1) as f64 * f) as usize];
            format!("{} px: p10 {:.3}, median {:.3}, p90 {:.3}", v.len(), at(0.1), at(0.5), at(0.9))
        };
        eprintln!("r/g where v5 > {NAIP_BARE_V} (does chroma separate them where brightness does not?): cured grass {}; trails and bare {}", q(&mut diag[0]), q(&mut diag[1]));
    }
    let (mut gs, mut bs) = (grass_v, bare_v);
    gs.sort_by(f32::total_cmp);
    bs.sort_by(f32::total_cmp);
    if gs.len() < 1000 || bs.len() < 1000 {
        bail!("too few pixels to calibrate the bare threshold: {} cured grass, {} bare", gs.len(), bs.len());
    }
    // the threshold with the best balanced accuracy: cured grass at or under it, bare over it
    let (mut best_t, mut best_acc) = (NAIP_BARE_V, 0f64);
    let mut t = 0.40f32;
    while t <= 0.95 {
        let g_ok = gs.partition_point(|&v| v <= t) as f64 / gs.len() as f64;
        let b_ok = 1.0 - bs.partition_point(|&v| v <= t) as f64 / bs.len() as f64;
        if (g_ok + b_ok) / 2.0 > best_acc {
            (best_t, best_acc) = (t, (g_ok + b_ok) / 2.0);
        }
        t += 0.0025;
    }
    // the city's threshold only relaxes the rules' (pale cured grass): never under NAIP_BARE_V; `--bare-v5` sets it
    let bare_t = bare_override.unwrap_or(best_t).max(NAIP_BARE_V);
    let pstats = |v: &[f32]| -> BTreeMap<String, f64> {
        let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize] as f64;
        [("count", v.len() as f64), ("p10", q(0.1)), ("p25", q(0.25)), ("median", q(0.5)), ("p75", q(0.75)), ("p90", q(0.9))].into_iter().map(|(k, x)| (k.to_string(), x)).collect()
    };
    let (gst, bst) = (pstats(&gs), pstats(&bs));
    let fmt = |m: &BTreeMap<String, f64>| format!("{:.0} px: p10 {:.3}, p25 {:.3}, median {:.3}, p75 {:.3}, p90 {:.3}", m["count"], m["p10"], m["p25"], m["median"], m["p75"], m["p90"]);
    let acc_at = |t: f32| (gs.partition_point(|&v| v <= t) as f64 / gs.len() as f64 + 1.0 - bs.partition_point(|&v| v <= t) as f64 / bs.len() as f64) / 2.0;
    eprintln!("v5, cured grass: {}", fmt(&gst));
    eprintln!("v5, trails and bare ground: {}", fmt(&bst));
    eprintln!(
        "bare threshold: best separation at {best_t:.4} (balanced accuracy {best_acc:.3}; the rules' {NAIP_BARE_V}: {:.3}); the city's {bare_t:.4}{} (balanced accuracy {:.3}) ({:.1} s)",
        acc_at(NAIP_BARE_V),
        if bare_override.is_some() { ", set by --bare-v5" } else { "" },
        acc_at(bare_t),
        t0.elapsed().as_secs_f64()
    );
    let acc_rules = acc_at(NAIP_BARE_V);

    // 4b. Every pixel classified with the city calibrations, counted in its texel.
    let mut counts = vec![0u32; w * h];
    {
        let (pimg, col_i, row_j, slopes, trail) = (&pimg, &col_i, &row_j, &slopes, &trail);
        let pix = |pc: i64, pr: i64| pget(pimg, cols, rows, pc, pr);
        eo_hills::par_rows(&mut counts, w, |ja, chunk| {
            let jb = ja + chunk.len() / w;
            for (pr, &j) in row_j.iter().enumerate() {
                if j == OFF || (j as usize) < ja || (j as usize) >= jb {
                    continue;
                }
                let out = &mut chunk[(j as usize - ja) * w..(j as usize - ja + 1) * w];
                for (pc, &i) in col_i.iter().enumerate() {
                    if i == OFF || pix(pc as i64, pr as i64).is_none() {
                        continue;
                    }
                    let block: [Option<[u8; 3]>; 25] = std::array::from_fn(|q| pix(pc as i64 + (q % 5) as i64 - 2, pr as i64 + (q / 5) as i64 - 2));
                    let k = j as usize * w + i as usize;
                    if let Some(c) = pl::city_counts(&block, slopes[k] as f64, bare_t, trail[k]) {
                        out[i as usize] += pl::pack(c);
                    }
                }
            }
        });
    }
    drop(pimg);
    let mut stats = pl::Stats::default();
    for &v in &counts {
        let c = pl::unpack(v);
        let data = (c[0] + c[1] + c[2] + c[3]) as u32;
        if data > 0 {
            stats.texels_with_data += 1;
            stats.pixels += data as u64;
            stats.max_pixels_per_texel = stats.max_pixels_per_texel.max(data);
        }
        for q in 0..8 {
            stats.totals[q] += c[q] as u64;
        }
    }
    if stats.max_pixels_per_texel > 15 {
        bail!("{} pixels in a texel: past a 4-bit count", stats.max_pixels_per_texel);
    }
    eprintln!(
        "classified {} pixels in {} texels (at most {} a texel): canopy {:.1}%, low {:.1}%, bare {:.1}%, unknown {:.1}% ({:.1} s)",
        stats.pixels,
        stats.texels_with_data,
        stats.max_pixels_per_texel,
        100.0 * stats.totals[0] as f64 / stats.pixels as f64,
        100.0 * stats.totals[1] as f64 / stats.pixels as f64,
        100.0 * stats.totals[2] as f64 / stats.pixels as f64,
        100.0 * stats.totals[3] as f64 / stats.pixels as f64,
        t0.elapsed().as_secs_f64()
    );

    // 5. The file.
    let mut city_files = BTreeMap::new();
    for name in ["sf-city.json.gz", "sf-elevation.bin.gz"] {
        if let Ok(b) = std::fs::read(paths.sf(name)) {
            city_files.insert(name.to_string(), super::sha256_hex(&b));
        }
    }
    let header = pl::Header {
        format: pl::FORMAT.into(),
        origin_x: pg.origin_x,
        origin_z: pg.origin_z,
        texel_m: pg.texel,
        width: w as u32,
        height: h as u32,
        pixel: pl::PixelGrid {
            deg: d,
            col_m,
            row_m,
            west: g.west,
            north: g.north,
            cols: cols as u32,
            rows: rows as u32,
            resampling: "the citywide 0.3 m tiles' 8-bit sRGB values as delivered (no haze removal), area-weighted means over each survey-sized pixel (half its area with data or none)".into(),
            gain,
        },
        counts: pl::COUNTS.map(String::from).to_vec(),
        rules: pl::rule_constants(),
        slope: "the 8 m elevation grid's slope at the pixel's texel centre: central differences over one cell each way, degrees (eo-app site_slope_deg)".into(),
        block_texels: 0,
        blocks_x: 0,
        blocks_y: 0,
        blocks: 0,
        calibration: pl::Calibration {
            bare_v5: bare_t as f64,
            survey_bare_v5: NAIP_BARE_V as f64,
            bare_rule: format!(
                "a pixel the rules call bare only for its 5 x 5 brightness (naip_class at NAIP_BARE_V gives low: not grey-blue, not steeper than NAIP_BARE_SLOPE_DEG) is low up to bare_v5{}: the brightness that best separates cured grass (mapped grassland or meadow, or a park steeper than 0.15, away from ways and trails) from mapped trails and bare ground is {best_t:.4} (balanced accuracy {best_acc:.3}; the rules' NAIP_BARE_V gives {acc_rules:.3}), over pixels the rules call low or bare that are not green, not grey-blue and not steep; never under NAIP_BARE_V",
                if bare_override.is_some() { " (set by --bare-v5)" } else { " (the best separation)" }
            ),
            populations: [("cured_grass".to_string(), gst), ("trails_and_bare".to_string(), bst)].into_iter().collect(),
            warm_soil_rule: "warm and soil (the rules' tests) are counted only on mapped trails (a path, footway, track, bridleway or steps at grade within 1.5 m, inside a park, grassland, scrub, wood or golf course) and mapped bare ground; elsewhere 0 (the city's cured grass is redder than its trails)".into(),
            block_rule: format!("blocks of {} x {} texels (64 m) holding any texel with data, outside the footprints and not WorldCover water, that lies in a mapped green (sf-lots-parks.json greens, any kind) or that WorldCover calls shrub, grass, bare or sparse, or wetland (20, 30, 60, 90, 95, 100): the open ground the planting draws on; elsewhere (streets, yards, built blocks) the map has no data and the runtime keeps its rules without the image", pl::BLOCK, pl::BLOCK),
        },
        stats,
        inputs: pl::Inputs {
            naip_manifest_sha256: super::sha256_hex(&manifest_bytes),
            naip_tiles_used: tiles.len() as u32,
            naip_source: manifest.source.clone(),
            naip_licence: manifest.licence.clone(),
            naip_credit: manifest.credit.clone(),
            city_files,
        },
        generator: pl::GENERATOR.into(),
    };

    // The blocks kept: open land with data.
    let bxn = w.div_ceil(pl::BLOCK);
    let mut keep_block = vec![false; bxn * h.div_ceil(pl::BLOCK)];
    let mut any_open = 0usize;
    let mut open_block = vec![false; keep_block.len()];
    for (k, &v) in counts.iter().enumerate() {
        let c = pl::unpack(v);
        if c[0] + c[1] + c[2] + c[3] > 0 && !built[k] && !matches!(wc[k], 0 | 80) {
            let b = (k / w / pl::BLOCK) * bxn + (k % w) / pl::BLOCK;
            open_block[b] = true;
            if greens[k] != 0 || matches!(wc[k], 20 | 30 | 60 | 90 | 95 | 100) {
                keep_block[b] = true;
            }
        }
    }
    any_open += open_block.iter().filter(|b| **b).count();
    let mut planting = Planting::from_dense(header, &counts, &|px, pz| keep_block[pz * bxn + px])?;
    drop(counts);
    eprintln!(
        "{} of {} blocks kept ({any_open} hold open land with data): {:.1} MB held ({:.1} s)",
        planting.header().blocks,
        keep_block.len(),
        planting.held_bytes() as f64 / 1e6,
        t0.elapsed().as_secs_f64()
    );

    // 6. Against the survey: Planting::shares and naip_shares_at at random open-ground points.
    if let Some(n) = &survey {
        let (x0, z0) = frame.to_plan(sb[0], sb[3]);
        let (x1, z1) = frame.to_plan(sb[2], sb[1]);
        let mut pts = vec![];
        let mut tries = 0;
        while pts.len() < samples && tries < samples * 200 {
            tries += 1;
            let (x, z) = (x0 + rand() * (x1 - x0), z0 + rand() * (z1 - z0));
            let Some((i, j)) = pg.texel_of(x, z) else { continue };
            let (lon, lat) = frame.to_lonlat(x, z);
            if built[j * w + i] || naip_pixel(lon, lat).is_none() || planting.shares(x, z, 8.0).is_none() {
                continue;
            }
            pts.push((x, z, greens[j * w + i] != 0, slopes[j * w + i] as f64));
        }
        // 1 m, the runtime's plant and tree boxes (3 m and 6 m at the 1.25 ground scale: 2.4 and 4.8 m real), 8 m
        for side in [1.0, 2.4, 4.8, 8.0] {
            for (subset, keep) in [("open", None), ("greens", Some(true))] {
                let mut diff = [0f64; 7];
                let (mut both, mut agree, mut only_one) = (0usize, 0usize, 0usize);
                let mut conf = [[0usize; 3]; 3];
                let (mut lc_n, mut lc_agree) = (0usize, 0usize);
                for &(x, z, in_green, slope) in &pts {
                    if keep.is_some_and(|k| k != in_green) {
                        continue;
                    }
                    match (survey_shares(n, &frame, x, z, side, slope), planting.shares(x, z, side)) {
                        (Some(a), Some(b)) => {
                            both += 1;
                            let (va, vb) = (share_vec(&a), share_vec(&b));
                            for q in 0..7 {
                                diff[q] += (va[q] - vb[q]).abs() as f64;
                            }
                            let (da, db) = (dominant(&a), dominant(&b));
                            agree += (da == db) as usize;
                            conf[da][db] += 1;
                            if da < 2 {
                                lc_n += 1;
                                lc_agree += (da == db) as usize;
                            }
                        }
                        (None, None) => {}
                        _ => only_one += 1,
                    }
                }
                let line: Vec<String> = SHARES.iter().zip(diff).map(|(s, v)| format!("{s} {:.3}", v / both.max(1) as f64)).collect();
                eprintln!(
                    "validation {side} m, {subset}: {both} points with both ({only_one} with one); mean |difference|: {}; dominant class agrees {:.1}%, where the survey's is canopy or low {:.1}% of {lc_n} (rows survey, columns cooked, canopy/low/bare: {conf:?})",
                    line.join(", "),
                    100.0 * agree as f64 / both.max(1) as f64,
                    100.0 * lc_agree as f64 / lc_n.max(1) as f64
                );
                validation.insert(format!("dominant_agree_canopy_low_{side}m_{subset}"), lc_agree as f64 / lc_n.max(1) as f64);
                for (q, s) in SHARES.iter().enumerate() {
                    validation.insert(format!("mad_{side}m_{subset}_{s}"), diff[q] / both.max(1) as f64);
                }
                validation.insert(format!("dominant_agree_{side}m_{subset}"), agree as f64 / both.max(1) as f64);
                validation.insert(format!("points_{side}m_{subset}"), both as f64);
            }
        }
    }
    let mut stats = planting.header().stats.clone();
    stats.validation = validation;
    stats.seconds = t0.elapsed().as_secs_f32();
    planting.set_stats(stats);
    let out = pl::file_path(&eo_hills::cache_dir());
    planting.write(&out)?;
    let back = Planting::open(&out)?;
    eprintln!("wrote {} ({:.1} MB; {:.1} MB held when open; {:.1} s)", out.display(), std::fs::metadata(&out)?.len() as f64 / 1e6, back.held_bytes() as f64 / 1e6, t0.elapsed().as_secs_f64());

    // 7. The shares on a few hills: the 8 m box at the point, and the medians of 8 m boxes every 4 m over 128 m round it.
    for (name, x, z) in [("Bernal", 431.0, 3848.0), ("Twin Peaks", -2490.0, 2919.0), ("McLaren grassland", -498.0, 7362.0), ("Presidio forest", -3600.0, -1900.0)] {
        let at = back.shares(x, z, 8.0);
        let mut v: [Vec<f32>; 7] = Default::default();
        for j in -16..16 {
            for i in -16..16 {
                if let Some(s) = back.shares(x + i as f64 * 4.0, z + j as f64 * 4.0, 8.0) {
                    for (q, s) in share_vec(&s).into_iter().enumerate() {
                        v[q].push(s);
                    }
                }
            }
        }
        let med: Vec<String> = SHARES.iter().zip(v.iter_mut()).map(|(s, v)| format!("{s} {:.2}", eo_hills::macro_map::median(v).unwrap_or(f32::NAN))).collect();
        eprintln!("{name} ({x}, {z}): at the point {:?}; medians over 128 m: {}", at.map(|s| share_vec(&s).map(|v| (v * 100.0).round() / 100.0)), med.join(", "));
    }
    eprintln!("done in {:.1} s", t0.elapsed().as_secs_f64());
    Ok(())
}
