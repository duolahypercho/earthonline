//! ETOPO relief for open water beyond the city's detailed elevation grid.
use anyhow::{Result, ensure};

pub struct Shelf {
    west: f64,
    south: f64,
    step: f64,
    width: usize,
    height: usize,
    metres_x4: Vec<i16>,
}

impl Shelf {
    pub fn load(paths: &crate::DataPaths) -> Result<Self> {
        let b = crate::read_maybe_gzip(&paths.sf("sf-shelf.bin.gz"))?;
        ensure!(b.len() >= 36 && &b[..4] == b"EOS1", "invalid shelf header");
        let width = u32::from_le_bytes(b[4..8].try_into()?) as usize;
        let height = u32::from_le_bytes(b[8..12].try_into()?) as usize;
        let west = f64::from_le_bytes(b[12..20].try_into()?);
        let south = f64::from_le_bytes(b[20..28].try_into()?);
        let step = f64::from_le_bytes(b[28..36].try_into()?);
        ensure!(
            width >= 2
                && height >= 2
                && width.checked_mul(height) == Some((b.len() - 36) / 2)
                && b.len() % 2 == 0
                && step.is_finite()
                && step > 0.0
                && west.is_finite()
                && south.is_finite(),
            "invalid shelf dimensions"
        );
        let metres_x4 = b[36..]
            .chunks_exact(2)
            .map(|v| i16::from_le_bytes([v[0], v[1]]))
            .collect();
        Ok(Self {
            west,
            south,
            step,
            width,
            height,
            metres_x4,
        })
    }

    /// Height relative to mean sea level. Inland basins and unsampled cells are absent.
    pub fn height_msl(&self, lon: f64, lat: f64) -> Option<f32> {
        let x = (lon - self.west) / self.step;
        let y = (lat - self.south) / self.step;
        if !(0.0..(self.width - 1) as f64).contains(&x)
            || !(0.0..(self.height - 1) as f64).contains(&y)
        {
            return None;
        }
        let at = |i: usize, j: usize| self.metres_x4[j * self.width + i];
        if at(x.round() as usize, y.round() as usize) == i16::MAX {
            return None;
        }
        let mut sum = 0.;
        let mut weights = 0.;
        for (j, wy) in [(0, 1. - y.fract()), (1, y.fract())] {
            for (i, wx) in [(0, 1. - x.fract()), (1, x.fract())] {
                let h = at(x as usize + i, y as usize + j);
                if h != i16::MAX {
                    weights += wy * wx;
                    sum += wy * wx * h as f64;
                }
            }
        }
        (weights > 0.).then(|| (sum / (weights * 4.)) as f32)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shelf_and_bay_are_connected_but_inland_cells_are_dry() {
        let s = super::Shelf::load(&crate::DataPaths::discover().unwrap()).unwrap();
        assert!(s.height_msl(-122.97, 37.77).unwrap() < -20.);
        assert!(s.height_msl(-122.43, 37.82).unwrap() < -4.);
        assert!(s.height_msl(-122.46, 37.755).is_none());
        assert!(s.height_msl(-155., 37.).is_none());
    }
}
