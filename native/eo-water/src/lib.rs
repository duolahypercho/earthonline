//! Earth Online native: the water (pure logic, no GPU).
//!
//! A port of the JS game's water system as it decides where water is and how
//! it moves and looks, for eo-render's water pass:
//!
//! | JS | Rust |
//! | --- | --- |
//! | src/citygen/shoreline.js `getCitywideShorelineOracle` (`isWater`, `edgeDistance`) | `shoreline` |
//! | src/render/materials/ocean-open-sea.js `bakeCitywideShoreField`, `chamfer`, `beachDistanceGrid`, the grid | `field`, `grid` |
//! | src/render/materials/ocean-fft.js `buildSpectrum`, `cascadeBandWeights`, the transform | `spectrum` |
//! | src/render/materials/ocean-water-optics.js | `optics` |
//! | src/render/materials/water-surface.js `buildWindField`, `buildSwellField`, the collar formulas, the march constants | `surface` |
//!
//! `tests/js_oracle.rs` compares against node-run JS answers
//! (the earlier browser version's exporter -> `data/water-oracle.json.gz`).

pub mod field;
pub mod grid;
pub mod optics;
pub mod shoreline;
pub mod spectrum;
pub mod surface;
pub mod surf_strip; // the surf strip along the beaches
pub mod waves; // the sea state and the spectrum per sea and weather

use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;

pub use shoreline::{SEA_LEVEL_Y, SEABED_Y};

fn read_gz(path: &Path) -> Result<Vec<u8>> {
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut out = Vec::with_capacity(raw.len() * 6);
        flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut out).with_context(|| format!("gunzip {}", path.display()))?;
        return Ok(out);
    }
    Ok(raw)
}

/// The `boundary` rings of sf-city.json.gz (only that field is kept).
pub fn load_boundary(repo_root: &Path) -> Result<Vec<Vec<f64>>> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        boundary: Vec<Vec<f64>>,
    }
    let bytes = read_gz(&repo_root.join("public/data/sf/sf-city.json.gz"))?;
    let raw: Raw = serde_json::from_slice(&bytes).context("sf-city.json boundary")?;
    let mut boundary = raw.boundary;
    let gs = ground_scale();
    if gs != 1.0 {
        for r in &mut boundary {
            for v in r.iter_mut() {
                *v *= gs;
            }
        }
    }
    Ok(boundary)
}

/// the ground-plan scale (eo-data `groundscale::scale`, 1.25 by default).
pub fn ground_scale() -> f64 {
    eo_data::groundscale::scale()
}

/// `sfShoreBeaches(lotsParks)`: the mapped beaches of sf-lots-parks.json.gz.
pub fn load_beaches(repo_root: &Path) -> Result<Vec<field::Beach>> {
    #[derive(Deserialize)]
    struct Green {
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        polygon: Vec<f64>,
    }
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        greens: Vec<Green>,
    }
    let bytes = read_gz(&repo_root.join("public/data/sf/sf-lots-parks.json.gz"))?;
    let raw: Raw = serde_json::from_slice(&bytes).context("sf-lots-parks.json greens")?;
    Ok(raw
        .greens
        .into_iter()
        .filter(|g| g.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("beach")))
        .map(|g| field::Beach { polygon: g.polygon.chunks_exact(2).map(|c| [c[0] * ground_scale(), c[1] * ground_scale()]).collect() }) // * scale
        .filter(|b| b.polygon.len() >= 3)
        .collect())
}

/// SHA-256 (hex) of a file's bytes.
pub fn file_sha256(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Sha256::digest(&b).iter().map(|x| format!("{x:02x}")).collect())
}

/// What the water pass needs from the CPU, built once off the render thread.
pub struct WaterData {
    pub field: field::ShoreField,
    pub field_from_cache: bool,
    pub ms: f64,
}

/// Build (or read from `cache`) the citywide shore field. The cache key is
/// the digest of the two source files and the bake version. (the
/// spectrum is the sea state's, built on the GPU; the JS port's
/// `spectrum::build_spectrum` is kept for the oracle tests only.)
pub fn load(repo_root: &Path, cache: Option<&Path>) -> Result<WaterData> {
    let t = std::time::Instant::now();
    let city = repo_root.join("public/data/sf/sf-city.json.gz");
    let lots = repo_root.join("public/data/sf/sf-lots-parks.json.gz");
    let key = {
        use sha2::{Digest, Sha256};
        let mut k = format!("{}|{}|{}", field::FIELD_VERSION, file_sha256(&city)?, file_sha256(&lots).unwrap_or_default());
        let tag = eo_data::groundscale::cache_tag(); // its own field (scale 1: the key is unchanged)
        if !tag.is_empty() {
            k.push_str(&format!("|{tag}"));
        }
        Sha256::digest(k.as_bytes()).iter().take(12).map(|x| format!("{x:02x}")).collect::<String>()
    };
    let path = cache.map(|c| c.join("water").join(format!("field-{key}.eowf")));
    let mut from_cache = false;
    let mut field = None;
    if let Some(p) = &path {
        if let Ok(b) = std::fs::read(p) {
            match field::ShoreField::from_bytes(&b) {
                Ok(f) => {
                    field = Some(f);
                    from_cache = true;
                }
                Err(e) => eprintln!("water: cached field refused ({e:#}); baking again"),
            }
        }
    }
    let field = match field {
        Some(f) => f,
        None => {
            let boundary = load_boundary(repo_root)?;
            let oracle = shoreline::Oracle::citywide(&boundary).context("sf-city.json has no shoreline rings")?;
            let beaches = load_beaches(repo_root).unwrap_or_default();
            let f = field::bake_citywide(&oracle, field::FAR_FIELD_CELL_M, field::MAX_TEXELS, &beaches);
            if let Some(p) = &path {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let tmp = p.with_extension("tmp");
                if std::fs::write(&tmp, f.to_bytes()).is_ok() {
                    let _ = std::fs::rename(&tmp, p);
                }
            }
            f
        }
    };
    Ok(WaterData { field, field_from_cache: from_cache, ms: t.elapsed().as_secs_f64() * 1e3 })
}
