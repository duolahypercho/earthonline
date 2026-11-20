//! Join the surveyed Aquatic Park beach to the first offshore NOAA sounding.
//! Some intervening lidar cells contain the same hydro-flattened elevation.
//! Build at the terrain vertices so collision, the drawn bed and its props agree.

use eo_data::{ElevationGrid, groundscale};
use eo_water::{SEA_LEVEL_Y, field::ShoreField};

pub struct CoveBed {
    origin: [f64; 2],
    cell: f64,
    cols: usize,
    rows: usize,
    samples: Vec<Option<(f64, f64)>>,
}

impl CoveBed {
    pub fn new(grid: &ElevationGrid, shore: &ShoreField) -> Self {
        let m = &grid.meta;
        let scale = groundscale::scale();
        let cell = m.cell_size;
        let index = |metres: f64, origin: f64, count| {
            (((metres * scale - origin) / cell - 0.5).round() as usize)
                .min(usize::saturating_sub(count, 1))
        };
        if m.width == 0 || m.height == 0 {
            return Self {
                origin: [0.0; 2],
                cell,
                cols: 0,
                rows: 0,
                samples: Vec::new(),
            };
        }
        // Longitude extent of the mapped beach (OSM way 224941181). Leave
        // the seawall west of the beach and the eastern pier approach alone.
        let (west, east) = (
            index(-464.0, m.origin_x, m.width),
            index(-141.0, m.origin_x, m.width),
        );
        let (north, south) = (
            index(-3490.0, m.origin_z, m.height),
            index(-3090.0, m.origin_z, m.height),
        );
        let cols = east - west + 1;
        let rows = south - north + 1;
        let mut bed = Self {
            origin: [
                m.origin_x + (west as f64 + 0.5) * cell,
                m.origin_z + (north as f64 + 0.5) * cell,
            ],
            cell,
            cols,
            rows,
            samples: vec![None; cols * rows],
        };
        for c in 0..cols {
            let x = bed.origin[0] + c as f64 * cell;
            let mut previous_shore = None;
            let mut anchor = None;
            for r in (0..rows).rev() {
                let z = bed.origin[1] + r as f64 * cell;
                let s = shore.signed_exact(x, z);
                if s <= 0.0 {
                    previous_shore = Some(s);
                    anchor = None;
                    continue;
                }
                if let Some(land_s) = previous_shore.take() {
                    // Locate mean water level between these two grid vertices.
                    anchor = Some((r as f64 + s / (s - land_s), SEA_LEVEL_Y));
                }
                let raw = grid.topobathy[(north + r) * m.width + west + c];
                if raw == m.nodata as i16 {
                    continue;
                }
                let y = groundscale::lift_by(
                    f64::from(raw) * 0.01 + eo_data::NAVD88_TO_GAME_Y,
                    grid.height_scale,
                );
                if y >= SEA_LEVEL_Y {
                    continue;
                }
                if let Some((start, start_y)) = anchor {
                    // Fill only between known anchors, and feather into the original
                    // terrain at the other three sides of our survey window.
                    for k in r..=(start.floor() as usize).min(rows - 1) {
                        let t = (start - k as f64) / (start - r as f64);
                        let edge = c.min(cols - 1 - c).min(k) as f64;
                        bed.samples[k * cols + c] =
                            Some((start_y + (y - start_y) * t, (edge / 3.0).min(1.0)));
                    }
                }
                anchor = Some((r as f64, y));
            }
        }
        bed
    }

    /// The carpet invokes this at cell centres. Blend through the outer cells.
    pub fn sample(&self, x: f64, z: f64) -> Option<(f64, f64)> {
        let c = ((x - self.origin[0]) / self.cell).round() as isize;
        let r = ((z - self.origin[1]) / self.cell).round() as isize;
        if c < 0 || r < 0 || c as usize >= self.cols || r as usize >= self.rows {
            return None;
        }
        self.samples[r as usize * self.cols + c as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cove_joins_the_waterline_to_the_offshore_survey() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let grid = ElevationGrid::load(&paths).unwrap();
        let water = eo_water::load(&paths.repo_root, None).unwrap();
        let bed = CoveBed::new(&grid, &water.field);
        let [x, start] = groundscale::plan([-365.0, -3070.0]);
        let z = (0..500)
            .map(|i| start - f64::from(i))
            .find(|&z| water.field.signed_exact(x, z) > 0.0)
            .unwrap();
        let samples = [16., 48., 96., 144.].map(|d| {
            let (h, weight) = bed.sample(x, z - d * groundscale::scale()).unwrap();
            assert_eq!(weight, 1.0);
            SEA_LEVEL_Y - h
        });
        assert!(samples[0] < 1.1 && samples[3] > 2.7, "{samples:?}");
        assert!(
            samples.windows(2).all(|pair| pair[1] > pair[0]),
            "{samples:?}"
        );
        assert!(bed.sample(0.0, 0.0).is_none());
    }
}
