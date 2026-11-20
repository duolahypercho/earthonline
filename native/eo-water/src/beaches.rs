//! Each beach's own foreshore slope (water iteration 2, water-3, water side),
//! measured from the city's elevation grid (`public/data/sf/sf-elevation.bin.gz`:
//! USGS 3DEP lidar above the water, NOAA CUDEM topobathy under it) in place of
//! the surf model's one 1:55 (`field::BEACH_FORESHORE_SLOPE`).
//!
//! For every texel of the shore field on a mapped beach's waterline, a transect
//! along the field's gradient (`transect_m`: far enough inland and out to
//! cross the tidal band on the gentlest slope allowed); the ground's slope
//! fitted (least squares) over the samples between mean lower low water and
//! mean higher high water (`fit_band`: the intertidal foreshore, NOAA
//! 9414290's datums); each beach takes the median of its transects, which
//! must lie within `SLOPE_RANGE` (outside it the transects crossed a seawall
//! or a back-beach flat, and that beach keeps the default 1:55). The log at
//! load lists each beach's slope; with this band Crissy Field's comes out
//! 1:10 to 1:11, Baker Beach's 1:17 to 1:19 and Ocean Beach's 1:42 to 1:46
//! (the grid's 8 m cells smooth a berm's crest, so a steep face reads a
//! little gentler than a finer survey would).
//!
//! **Source, and a deviation.** The plan asked for the 1 m DEM under
//! `EO_DEM_ROOT`; what that folder holds on this machine is USGS 3DEP 1
//! arc-second tiles (`USGS_1_n38w123.tif` and its neighbours, about 30 m
//! cells), coarser than the repository's 8 m grid, so the 8 m grid is the
//! finer survey at hand. A 1 m lidar raster would sharpen the steep beaches;
//! it is owed through the data pipeline, not read here.
//!
//! Heights above the sea follow the approved 1.25 height scale and the plan the
//! 1.25 ground-plan scale, so a slope above the sea is the surveyed one; the
//! drawn foreshore below it (down to `SEABED_Y`) takes the same slope.

use crate::field::{Beach, ShoreField};
use crate::shoreline::{SEA_LEVEL_Y, SEABED_Y};
use std::sync::OnceLock;

/// The transect's step (game m): a quarter of the elevation grid's 8 m cell at the 1.25 plan.
pub const TRANSECT_STEP_M: f64 = 2.0;
/// NOAA CO-OPS station 9414290 (San Francisco) tidal datums, metres over its
/// station datum, epoch 1983-2001 (api.tidesandcurrents.noaa.gov mdapi,
/// read 2026-11-18): MLLW 1.822, MSL 2.773, MHHW 3.602 (NAVD88 1.804, so MSL
/// is 0.969 m over NAVD88, as eo-data `NAVD88_TO_GAME_Y` takes it). The game's
/// still sea is mean sea level.
pub const MLLW_BELOW_MSL_M: f64 = 2.773 - 1.822;
pub const MHHW_ABOVE_MSL_M: f64 = 3.602 - 2.773;
/// The slopes a beach may take (rise over run): from the gentlest fine-sand
/// beaches, about 1:100, to cobble beaches, about 1:4 (Bascom 1951, "The
/// relationship between sand size and beach-face slope", Trans. AGU 32(6),
/// 866; the bounds INFERRED as round numbers).
pub const SLOPE_RANGE: [f64; 2] = [1.0 / 100.0, 1.0 / 4.0];

/// The heights fitted, game metres below and above the still sea: the
/// intertidal band from MLLW to MHHW at the height scale `hs` (1.25 drawn).
pub fn fit_band_at(hs: f64) -> [f64; 2] {
    [MLLW_BELOW_MSL_M * hs, MHHW_ABOVE_MSL_M * hs]
}

/// `fit_band_at` the drawn height scale.
pub fn fit_band() -> [f64; 2] {
    fit_band_at(eo_data::groundscale::height_scale())
}

/// The transect, game metres along the field's signed distance (negative
/// inland): it crosses the band on the gentlest slope allowed, `SLOPE_RANGE[0]`.
pub fn transect_m(band: [f64; 2]) -> [f64; 2] {
    [-band[1] / SLOPE_RANGE[0], band[0] / SLOPE_RANGE[0]]
}
/// Transects a beach needs for its own slope.
pub const MIN_TRANSECTS: usize = 3;

/// A beach's measured slope and where it applies (its polygon's box, widened).
#[derive(Clone, Debug)]
pub struct BeachSlope {
    pub bbox: [f64; 4],
    pub slope: f64,
    pub transects: usize,
}

static INSTALLED: OnceLock<Vec<BeachSlope>> = OnceLock::new();

/// The slopes this process measured (once `crate::load` has run with the beach profiles on).
pub fn installed() -> Option<&'static [BeachSlope]> {
    INSTALLED.get().map(|v| v.as_slice())
}

pub fn install(v: Vec<BeachSlope>) -> &'static [BeachSlope] {
    INSTALLED.get_or_init(|| v).as_slice()
}

/// How far a beach's box is widened for the lookup (m): out over its foreshore.
pub const BOX_MARGIN_M: f64 = 150.0;

/// The foreshore slope at a world point: the smallest box of a measured beach
/// holding it, else `fallback`.
pub fn slope_at(slopes: &[BeachSlope], x: f64, z: f64, fallback: f64) -> f64 {
    let mut best: Option<(f64, f64)> = None;
    for b in slopes {
        if x >= b.bbox[0] && x <= b.bbox[2] && z >= b.bbox[1] && z <= b.bbox[3] {
            let area = (b.bbox[2] - b.bbox[0]) * (b.bbox[3] - b.bbox[1]);
            if best.is_none_or(|(a, _)| area < a) {
                best = Some((area, b.slope));
            }
        }
    }
    best.map_or(fallback, |(_, s)| s)
}

fn inside(poly: &[[f64; 2]], x: f64, z: f64) -> bool {
    let mut c = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[j], poly[i]);
        if (b[1] > z) != (a[1] > z) && x < (a[0] - b[0]) * (z - b[1]) / (a[1] - b[1]) + b[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// Least-squares slope of the ground falling seaward along one transect (None
/// without enough samples in the band).
pub fn transect_slope(ground: &dyn Fn(f64, f64) -> Option<f64>, x: f64, z: f64, dir: [f64; 2], band: [f64; 2]) -> Option<f64> {
    let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let reach = transect_m(band);
    let mut s = reach[0];
    while s <= reach[1] {
        if let Some(y) = ground(x + dir[0] * s, z + dir[1] * s) {
            let d = y - SEA_LEVEL_Y;
            if d >= -band[0] && d <= band[1] {
                n += 1.0;
                sx += s;
                sy += y;
                sxx += s * s;
                sxy += s * y;
            }
        }
        s += TRANSECT_STEP_M;
    }
    if n < 4.0 {
        return None;
    }
    let den = n * sxx - sx * sx;
    if den.abs() < 1e-9 {
        return None;
    }
    let b = (n * sxy - sx * sy) / den; // dy/ds: negative where the ground falls seaward
    (b < 0.0).then_some(-b)
}

/// Measure every beach's slope on the field's waterline texels.
pub fn measure(field: &ShoreField, beaches: &[Beach], ground: &dyn Fn(f64, f64) -> Option<f64>, band: [f64; 2]) -> Vec<BeachSlope> {
    let step = [field.size[0] / (field.width - 1) as f64, field.size[1] / (field.height - 1) as f64];
    let mut out = Vec::new();
    for b in beaches {
        let p = &b.polygon;
        let (mut x0, mut z0, mut x1, mut z1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for v in p {
            x0 = x0.min(v[0]);
            z0 = z0.min(v[1]);
            x1 = x1.max(v[0]);
            z1 = z1.max(v[1]);
        }
        let mut slopes = Vec::new();
        let (c0, c1) = (((x0 - 60.0 - field.origin[0]) / step[0]).floor().max(0.0) as usize, (((x1 + 60.0 - field.origin[0]) / step[0]).ceil() as usize).min(field.width - 1));
        let (r0, r1) = (((z0 - 60.0 - field.origin[1]) / step[1]).floor().max(0.0) as usize, (((z1 + 60.0 - field.origin[1]) / step[1]).ceil() as usize).min(field.height - 1));
        for r in r0..=r1 {
            for c in c0..=c1 {
                let (x, z) = (field.origin[0] + step[0] * c as f64, field.origin[1] + step[1] * r as f64);
                let s = field.signed_exact(x, z);
                if s.abs() > 0.75 * step[0].max(step[1]) || field.beach_at(x, z) < 0.5 {
                    continue;
                }
                let g = [field.signed_exact(x + 2.0, z) - field.signed_exact(x - 2.0, z), field.signed_exact(x, z + 2.0) - field.signed_exact(x, z - 2.0)];
                let gl = g[0].hypot(g[1]);
                if gl < 1e-3 {
                    continue;
                }
                let dir = [g[0] / gl, g[1] / gl]; // seaward
                // (on this beach: a point a little inland inside its polygon)
                if !inside(p, x - dir[0] * 15.0, z - dir[1] * 15.0) && !inside(p, x, z) {
                    continue;
                }
                if let Some(sl) = transect_slope(ground, x - dir[0] * s, z - dir[1] * s, dir, band) {
                    slopes.push(sl);
                }
            }
        }
        if slopes.len() >= MIN_TRANSECTS {
            slopes.sort_by(|a, b| a.total_cmp(b));
            let med = slopes[slopes.len() / 2];
            // (a median outside the range is not a foreshore: the transects crossed a
            // seawall's riprap or a back-beach flat; that beach keeps the default slope)
            if !(SLOPE_RANGE[0]..=SLOPE_RANGE[1]).contains(&med) {
                eprintln!("water: beach at ({:.0}, {:.0}): median slope 1:{:.0} over {} transects is outside 1:{:.0}..1:{:.0}; kept at the default", 0.5 * (x0 + x1), 0.5 * (z0 + z1), 1.0 / med.max(1e-6), slopes.len(), 1.0 / SLOPE_RANGE[1], 1.0 / SLOPE_RANGE[0]);
                continue;
            }
            out.push(BeachSlope { bbox: [x0 - BOX_MARGIN_M, z0 - BOX_MARGIN_M, x1 + BOX_MARGIN_M, z1 + BOX_MARGIN_M], slope: med, transects: slopes.len() });
        }
    }
    out
}

/// Measure the slopes for the field from the repository's elevation grid and install them.
pub fn load(repo_root: &std::path::Path, field: &ShoreField) -> anyhow::Result<&'static [BeachSlope]> {
    let beaches = crate::load_beaches(repo_root)?;
    let mut grid = eo_data::ElevationGrid::decode(&eo_data::read_maybe_gzip(&repo_root.join("public/data/sf/sf-elevation.bin.gz"))?)?;
    let scale = crate::ground_scale();
    grid.meta.origin_x *= scale;
    grid.meta.origin_z *= scale;
    grid.meta.cell_size *= scale;
    grid.height_scale = eo_data::groundscale::height_scale();
    let ground = |x: f64, z: f64| grid.sample(x, z);
    let v = measure(field, &beaches, &ground, fit_band());
    Ok(install(v))
}

/// The drawn foreshore at signed distance `s` (positive seaward) for a
/// surveyed height `y` on a beach of slope `slope`: `field::beach_foreshore`'s
/// profile with that slope.
pub fn foreshore_with(s: f64, y: f64, slope: f64) -> f64 {
    let profile = (SEA_LEVEL_Y - s * slope).max(SEABED_Y);
    let pressed = y.min(profile);
    let b = crate::field::BEACH_BLEND_M;
    let t = ((s + b[0]) / (b[0] - b[1])).clamp(0.0, 1.0);
    let keep = t * t * (3.0 - 2.0 * t);
    pressed + (y - pressed) * keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transect_recovers_its_slope() {
        // a 1:12 foreshore through the sea level at s = 0, a flat berm 1.9 m up
        let g = |x: f64, _z: f64| Some((SEA_LEVEL_Y - x / 12.0).clamp(SEA_LEVEL_Y - 3.0, SEA_LEVEL_Y + 2.5));
        let s = transect_slope(&g, 0.0, 0.0, [1.0, 0.0], fit_band_at(1.25)).unwrap();
        assert!((s - 1.0 / 12.0).abs() < 1e-6, "{s}");
        // with the default slope the profile is the earlier one
        for s in [-80.0, -30.0, 0.0, 12.0, 300.0] {
            assert!((foreshore_with(s, 0.4, crate::field::BEACH_FORESHORE_SLOPE) - crate::field::beach_foreshore(s, 0.4)).abs() < 1e-12);
        }
        // the lookup takes the smallest box that holds the point
        let v = vec![BeachSlope { bbox: [0.0, 0.0, 100.0, 100.0], slope: 0.1, transects: 5 }, BeachSlope { bbox: [-1000.0, -1000.0, 1000.0, 1000.0], slope: 0.02, transects: 5 }];
        assert_eq!(slope_at(&v, 50.0, 50.0, 1.0), 0.1);
        assert_eq!(slope_at(&v, 500.0, 50.0, 1.0), 0.02);
        assert_eq!(slope_at(&v, 5000.0, 50.0, 1.0), 1.0);
    }

    // the band is NOAA's intertidal range at the drawn 1.25 height scale (rule 4: scaled values tested)
    #[test]
    fn the_fit_band_is_the_tidal_range_scaled() {
        let b = fit_band_at(1.25);
        assert!((b[0] - 0.951 * 1.25).abs() < 1e-9 && (b[1] - 0.829 * 1.25).abs() < 1e-9, "{b:?}");
        // the great diurnal range (MLLW to MHHW) NOAA publishes, 1.78 m, drawn 1.25 x
        assert!((b[0] + b[1] - 1.78 * 1.25).abs() < 0.01, "{b:?}");
        let t = transect_m(b);
        assert!((t[0] + 0.829 * 1.25 * 100.0).abs() < 1e-6 && (t[1] - 0.951 * 1.25 * 100.0).abs() < 1e-6, "{t:?}");
    }
}
