//! the USGS 3DEP 1 arc-second GeoTIFF tiles, decoded with the
//! `tiff` crate (nothing downloaded is ever run), and a mosaic sampler over
//! them in NAD83 geographic degrees, NAVD88 metres.

use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

/// One decoded tile: row 0 is the north edge, pixel-is-area (GeoTIFF raster
/// type 1), so pixel (c, r) covers lon `west + c * d_lon ..`, lat `north - r * d_lat ..`.
pub struct DemTile {
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// Outer edge of pixel (0, 0).
    pub west: f64,
    pub north: f64,
    pub d_lon: f64,
    pub d_lat: f64,
    pub nodata: f32,
    pub values: Vec<f32>,
}

impl DemTile {
    pub fn read(path: &Path) -> Result<Self> {
        let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut limits = Limits::default();
        limits.decoding_buffer_size = 128 << 20;
        let mut dec = Decoder::new(std::io::BufReader::new(file)).with_context(|| format!("{name}: not a TIFF"))?.with_limits(limits);
        let (w, h) = dec.dimensions()?;
        let scale = dec.get_tag(Tag::ModelPixelScaleTag)?.into_f64_vec()?;
        let tie = dec.get_tag(Tag::ModelTiepointTag)?.into_f64_vec()?;
        if scale.len() < 2 || tie.len() < 6 {
            bail!("{name}: no GeoTIFF pixel scale / tie point");
        }
        if tie[0] != 0.0 || tie[1] != 0.0 {
            bail!("{name}: tie point not at pixel (0, 0)");
        }
        let nodata = match dec.find_tag(Tag::GdalNodata)? {
            Some(v) => v.into_string()?.trim_matches(char::from(0)).trim().parse::<f64>().unwrap_or(-999999.0) as f32,
            None => -999999.0,
        };
        let values = match dec.read_image().with_context(|| format!("{name}: decoding"))? {
            DecodingResult::F32(v) => v,
            DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::I16(v) => v.into_iter().map(|x| x as f32).collect(),
            _ => bail!("{name}: unexpected sample format"),
        };
        if values.len() != (w * h) as usize {
            bail!("{name}: {} samples for {w} x {h}", values.len());
        }
        Ok(Self { name, width: w as usize, height: h as usize, west: tie[3], north: tie[4], d_lon: scale[0], d_lat: scale[1], nodata, values })
    }

    fn east(&self) -> f64 {
        self.west + self.width as f64 * self.d_lon
    }
    fn south(&self) -> f64 {
        self.north - self.height as f64 * self.d_lat
    }

    /// Value at pixel (c, r), None for no data or out of the tile.
    #[inline]
    fn px(&self, c: i64, r: i64) -> Option<f32> {
        if c < 0 || r < 0 || c >= self.width as i64 || r >= self.height as i64 {
            return None;
        }
        let v = self.values[r as usize * self.width + c as usize];
        if v == self.nodata || !v.is_finite() || v < -1000.0 { None } else { Some(v) }
    }

    /// Pixel coordinates of a point (pixel centres at integer + 0.5 removed).
    #[inline]
    fn frac(&self, lat: f64, lon: f64) -> (f64, f64) {
        ((lon - self.west) / self.d_lon - 0.5, (self.north - lat) / self.d_lat - 0.5)
    }
}

/// A pixel is flat water when its eight neighbours are within this of it (3DEP
/// flattens a water body to one height; the 1 arc-second resampling leaves
/// it within millimetres; real ground over 90 m never is).
pub const FLAT_TOLERANCE_M: f32 = 0.02;

/// What a sample found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DemSample {
    /// NAVD88 metres (bilinear over the valid pixels).
    pub navd88: f32,
    /// The nearest pixel and its eight neighbours carry one value (to
    /// FLAT_TOLERANCE_M): a hydro-flattened water surface (3DEP flattens
    /// lakes, ponds and the bay).
    pub flat: bool,
}

pub struct DemMosaic {
    pub tiles: Vec<DemTile>,
}

impl DemMosaic {
    pub fn new(tiles: Vec<DemTile>) -> Result<Self> {
        if tiles.is_empty() {
            return Err(anyhow!("no DEM tiles"));
        }
        Ok(Self { tiles })
    }

    /// Bilinear NAVD88 height at a NAD83 point from the first tile whose
    /// interior holds it (the tiles overlap by a few pixels). None where no
    /// tile has data (the open ocean in the coastal tiles).
    pub fn sample(&self, lat: f64, lon: f64) -> Option<DemSample> {
        for t in &self.tiles {
            if lon < t.west || lon >= t.east() || lat > t.north || lat <= t.south() {
                continue;
            }
            let (fx, fy) = t.frac(lat, lon);
            let (c0, r0) = (fx.floor() as i64, fy.floor() as i64);
            let (tx, ty) = (fx - c0 as f64, fy - r0 as f64);
            let mut acc = 0.0;
            let mut wsum = 0.0;
            for (dc, dr, w) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
                if let Some(v) = t.px(c0 + dc, r0 + dr) {
                    acc += v as f64 * w;
                    wsum += w;
                }
            }
            if wsum < 0.25 {
                return None;
            }
            let (nc, nr) = (fx.round() as i64, fy.round() as i64);
            let flat = match t.px(nc, nr) {
                Some(v) => {
                    let mut same = 0;
                    for (dc, dr) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                        if matches!(t.px(nc + dc, nr + dr), Some(u) if (u - v).abs() <= FLAT_TOLERANCE_M) {
                            same += 1;
                        }
                    }
                    same == 8
                }
                None => false,
            };
            return Some(DemSample { navd88: (acc / wsum) as f32, flat });
        }
        None
    }
}
