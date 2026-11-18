//! Each beach's own foreshore slope (water iteration 2, water-3, water side),
//! measured from the city's elevation grid (`public/data/sf/sf-elevation.bin.gz`:
//! USGS 3DEP lidar above the water, NOAA CUDEM topobathy under it) in place of
//! the surf model's one 1:55 (`field::BEACH_FORESHORE_SLOPE`).
//!
//! For every texel of the shore field on a mapped beach's waterline, a transect
//! along the field's gradient from `TRANSECT_M[0]` inland to `TRANSECT_M[1]`
//! out; the ground's slope fitted (least squares) over the samples between
//! `FIT_BAND_M` below and above the still sea (the foreshore between low water
//! and the berm); each beach takes the median of its transects, held to
//! `SLOPE_RANGE`. Crissy Field's comes out near 1:10 and Ocean Beach's near
//! 1:50 (the grid's 8 m cells smooth a berm's crest, so a steep face reads a
//! little gentler than a 1 m survey would).
//!
//! Heights above the sea follow the approved 1.25 height scale and the plan the
//! 1.25 ground-plan scale, so a slope above the sea is the surveyed one; the
//! drawn foreshore below it (down to `SEABED_Y`) takes the same slope.

use crate::field::{Beach, ShoreField};
use crate::shoreline::{SEA_LEVEL_Y, SEABED_Y};
use std::sync::OnceLock;

/// The transect, game metres along the field's signed distance (negative inland).
pub const TRANSECT_M: [f64; 2] = [-60.0, 40.0];
pub const TRANSECT_STEP_M: f64 = 2.0;
/// The heights fitted: this far below and above the still sea (game m).
pub const FIT_BAND_M: [f64; 2] = [1.0, 1.9];
/// The slopes a beach may take (rise over run).
pub const SLOPE_RANGE: [f64; 2] = [1.0 / 70.0, 1.0 / 6.0];
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
pub fn transect_slope(ground: &dyn Fn(f64, f64) -> Option<f64>, x: f64, z: f64, dir: [f64; 2]) -> Option<f64> {
    let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut s = TRANSECT_M[0];
    while s <= TRANSECT_M[1] {
        if let Some(y) = ground(x + dir[0] * s, z + dir[1] * s) {
            let d = y - SEA_LEVEL_Y;
            if d >= -FIT_BAND_M[0] && d <= FIT_BAND_M[1] {
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
pub fn measure(field: &ShoreField, beaches: &[Beach], ground: &dyn Fn(f64, f64) -> Option<f64>) -> Vec<BeachSlope> {
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
                if let Some(sl) = transect_slope(ground, x - dir[0] * s, z - dir[1] * s, dir) {
                    slopes.push(sl);
                }
            }
        }
        if slopes.len() >= MIN_TRANSECTS {
            slopes.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let med = slopes[slopes.len() / 2].clamp(SLOPE_RANGE[0], SLOPE_RANGE[1]);
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
    let v = measure(field, &beaches, &ground);
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
        let s = transect_slope(&g, 0.0, 0.0, [1.0, 0.0]).unwrap();
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
}
