//! the archetype ageing's detail sheet (`facade-detail-v1`,
//! src/render/materials/facade-detail.js; `public/assets/facade-detail-v1.jpg`,
//! CC0, per-cell sources and authors in facade-detail-v1.provenance.json):
//! RG tangent normal xy, B cavity, linear data, six 512 px cells in a 3 x 2
//! grid with a 16 px ring. Read where it is (nothing is copied into
//! `native/`), checked against the SHA-256 its provenance records
//! (`outputs.map`), decoded by this crate's baseline JPEG decoder, and
//! returned as RGBA8 levels 0..FACADE_DETAIL_LEVELS (2 x 2 box mips, linear,
//! alpha 255). The shader never reads past level log2(padPx) = 4
//! (`facadeDetailSampleNode`), so five levels are kept. The decoded levels are
//! cached under `cache_dir/kit/`, keyed by the file's digest, with the
//! payload's own SHA-256 checked on read.

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct FacadeDetail {
    pub width: u32,
    pub height: u32,
    /// Row 0 of each level is the image's TOP row (as decoded).
    pub levels: Vec<Vec<u8>>,
    pub from_cache: bool,
    pub load_ms: f64,
    pub sha256: String,
}

pub const FACADE_DETAIL_LEVELS: usize = 5;
pub const PROVENANCE: &str = "public/assets/facade-detail-v1.provenance.json";

pub fn load_cached(repo_root: &Path, cache_dir: Option<&Path>) -> Result<FacadeDetail> {
    let t = std::time::Instant::now();
    let prov: serde_json::Value =
        serde_json::from_slice(&std::fs::read(repo_root.join(PROVENANCE)).with_context(|| format!("reading {PROVENANCE}"))?)?;
    let map = &prov["outputs"]["map"];
    let path = map["path"].as_str().ok_or_else(|| anyhow!("{PROVENANCE}: no outputs.map.path"))?.to_string();
    let digest = map["sha256"].as_str().ok_or_else(|| anyhow!("{PROVENANCE}: no outputs.map.sha256"))?.to_string();
    let (w, h) = (prov["layout"]["width"].as_u64().unwrap_or(0) as usize, prov["layout"]["height"].as_u64().unwrap_or(0) as usize);
    if w == 0 || h == 0 {
        bail!("{PROVENANCE}: no layout size");
    }
    let sizes: Vec<(usize, usize)> = (0..FACADE_DETAIL_LEVELS).map(|l| ((w >> l).max(1), (h >> l).max(1))).collect();
    let total: usize = sizes.iter().map(|(a, b)| a * b * 4).sum();
    let file = cache_dir.map(|c| c.join("kit").join(format!("facade-detail-v1-{}.bin", &digest[..24.min(digest.len())])));
    if let Some(f) = &file {
        if let Ok(b) = std::fs::read(f) {
            if b.len() == total + 32 && Sha256::digest(&b[..total]).as_slice() == &b[total..] {
                let mut levels = vec![];
                let mut at = 0;
                for (a, bb) in &sizes {
                    levels.push(b[at..at + a * bb * 4].to_vec());
                    at += a * bb * 4;
                }
                return Ok(FacadeDetail { width: w as u32, height: h as u32, levels, from_cache: true, load_ms: t.elapsed().as_secs_f64() * 1e3, sha256: digest });
            }
        }
    }
    let p = repo_root.join(&path);
    let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
    let got = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if got != digest {
        bail!("{} has SHA-256 {got}, its provenance records {digest}", p.display());
    }
    let (dw, dh, rgb) = crate::jpeg::decode_rgb(&bytes)?;
    if dw != w || dh != h {
        bail!("{path} is {dw} x {dh}, its provenance layout says {w} x {h}");
    }
    let mut base = vec![255u8; w * h * 4];
    for i in 0..w * h {
        base[i * 4..i * 4 + 3].copy_from_slice(&rgb[i * 3..i * 3 + 3]);
    }
    let mut levels = vec![base];
    for l in 1..FACADE_DETAIL_LEVELS {
        let (pw, ph) = sizes[l - 1];
        let (nw, nh) = sizes[l];
        let prev = &levels[l - 1];
        let mut next = vec![255u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..3 {
                    let mut acc = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(pw - 1), (y * 2 + dy).min(ph - 1));
                        acc += prev[(sy * pw + sx) * 4 + c] as u32;
                    }
                    next[(y * nw + x) * 4 + c] = ((acc + 2) / 4) as u8;
                }
            }
        }
        levels.push(next);
    }
    if let Some(f) = &file {
        let mut b = Vec::with_capacity(total + 32);
        for l in &levels {
            b.extend_from_slice(l);
        }
        let d = Sha256::digest(&b);
        b.extend_from_slice(&d);
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let tmp = f.with_extension("tmp");
        if std::fs::write(&tmp, &b).is_ok() {
            let _ = std::fs::rename(&tmp, f);
        }
    }
    Ok(FacadeDetail { width: w as u32, height: h as u32, levels, from_cache: false, load_ms: t.elapsed().as_secs_f64() * 1e3, sha256: digest })
}
