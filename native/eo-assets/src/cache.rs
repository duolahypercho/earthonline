//! Binary model cache under `native/cache/` (git-ignored).
//!
//! A cooked model is stored content-addressed by the SHA-256 of the GLB it
//! came from: `cache/models/<sha256>.eomc`. A warm launch reads the cooked
//! bytes instead of re-parsing the GLB. The file records the source digest
//! (which must equal the manifest's) and the SHA-256 of its own payload,
//! checked on every read, so a truncated or altered cache file is refused and
//! re-cooked from the verified GLB. A GLB whose digest moved has a different
//! cache name; nothing stale can be served for it.

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::glb::{AlphaMode, CookedMaterial, CookedModel, CookedPart, cook_glb, read_verified};
use crate::manifest::AssetRef;

pub const MAGIC: &[u8; 4] = b"EOMC";
/// Bump when the cooked layout or the cook itself changes.
pub const VERSION: u32 = 5;
const HEADER: usize = 4 + 4 + 32 + 8 + 32;

pub fn model_path(cache_dir: &Path, sha256: &str) -> PathBuf {
    cache_dir.join("models").join(format!("{}.eomc", sha256.to_ascii_lowercase()))
}

fn hex_to_bytes(h: &str) -> Result<[u8; 32]> {
    if h.len() != 64 {
        bail!("digest {h} is not 64 hex digits");
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}

struct W(Vec<u8>);
impl W {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32s(&mut self, v: &[f32]) {
        for x in v {
            self.f32(*x);
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}
impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        if self.at + n > self.b.len() {
            bail!("cache payload truncated");
        }
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32x<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut a = [0f32; N];
        for v in &mut a {
            *v = self.f32()?;
        }
        Ok(a)
    }
}

pub fn encode(model: &CookedModel) -> Vec<u8> {
    let mut w = W(Vec::with_capacity(model.positions.len() * 16 + model.indices.len() * 4 + 1024));
    w.u32(model.positions.len() as u32);
    w.u32(model.indices.len() as u32);
    w.u32(model.parts.len() as u32);
    w.u32(model.materials.len() as u32);
    w.u32(model.primitives);
    w.u32(model.dropped_shadow_nodes);
    w.u32(model.dropped_shadow_triangles);
    w.f32s(&model.min);
    w.f32s(&model.max);
    w.0.extend_from_slice(bytemuck::cast_slice(&model.positions));
    w.0.extend_from_slice(bytemuck::cast_slice(&model.normals));
    w.0.extend_from_slice(bytemuck::cast_slice(&model.indices));
    for p in &model.parts {
        w.u32(p.material);
        w.u32(p.first_index);
        w.u32(p.index_count);
        w.f32s(&p.min);
        w.f32s(&p.max);
    }
    for m in &model.materials {
        w.u32(m.name.len() as u32);
        w.0.extend_from_slice(m.name.as_bytes());
        w.f32s(&m.base_color);
        w.f32(m.metallic);
        w.f32(m.roughness);
        w.f32s(&m.emissive);
        w.f32(m.emissive_strength);
        w.u32(m.alpha_mode as u32);
        w.f32(m.alpha_cutoff);
        w.u32(m.double_sided as u32);
    }
    w.0
}

pub fn decode(b: &[u8]) -> Result<CookedModel> {
    let mut r = R { b, at: 0 };
    let nv = r.u32()? as usize;
    let ni = r.u32()? as usize;
    let np = r.u32()? as usize;
    let nm = r.u32()? as usize;
    let mut m = CookedModel {
        primitives: r.u32()?,
        dropped_shadow_nodes: r.u32()?,
        dropped_shadow_triangles: r.u32()?,
        ..Default::default()
    };
    m.min = r.f32x::<3>()?;
    m.max = r.f32x::<3>()?;
    m.positions = bytemuck::pod_collect_to_vec(r.take(nv * 12)?);
    m.normals = bytemuck::pod_collect_to_vec(r.take(nv * 4)?);
    m.indices = bytemuck::pod_collect_to_vec(r.take(ni * 4)?);
    for _ in 0..np {
        m.parts.push(CookedPart {
            material: r.u32()?,
            first_index: r.u32()?,
            index_count: r.u32()?,
            min: r.f32x::<3>()?,
            max: r.f32x::<3>()?,
        });
    }
    for _ in 0..nm {
        let len = r.u32()? as usize;
        let name = String::from_utf8(r.take(len)?.to_vec()).map_err(|_| anyhow!("cache: material name not utf-8"))?;
        m.materials.push(CookedMaterial {
            name,
            base_color: r.f32x::<4>()?,
            metallic: r.f32()?,
            roughness: r.f32()?,
            emissive: r.f32x::<3>()?,
            emissive_strength: r.f32()?,
            alpha_mode: match r.u32()? {
                0 => AlphaMode::Opaque,
                1 => AlphaMode::Mask,
                2 => AlphaMode::Blend,
                x => bail!("cache: alpha mode {x}"),
            },
            alpha_cutoff: r.f32()?,
            double_sided: r.u32()? != 0,
        });
    }
    if r.at != b.len() {
        bail!("cache payload has {} trailing bytes", b.len() - r.at);
    }
    Ok(m)
}

/// Write a cooked model (atomically: temp file, then rename).
pub fn write(path: &Path, model: &CookedModel, source_sha256: &str) -> Result<()> {
    let payload = encode(model);
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&hex_to_bytes(source_sha256)?);
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&Sha256::digest(&payload));
    out.extend_from_slice(&payload);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, &out)?;
    std::fs::rename(&tmp, path).or_else(|_| {
        // Another loader thread may have won the race to the same name.
        let _ = std::fs::remove_file(&tmp);
        if path.exists() { Ok(()) } else { Err(anyhow!("could not place {}", path.display())) }
    })?;
    Ok(())
}

/// Read a cooked model; refuse it unless the magic, version, source digest
/// and payload digest all check.
pub fn read(path: &Path, source_sha256: &str) -> Result<CookedModel> {
    let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if b.len() < HEADER || &b[0..4] != MAGIC {
        bail!("{}: not a model cache file", path.display());
    }
    let version = u32::from_le_bytes(b[4..8].try_into().unwrap());
    if version != VERSION {
        bail!("{}: cache version {version}, want {VERSION}", path.display());
    }
    if b[8..40] != hex_to_bytes(source_sha256)? {
        bail!("{}: cooked from a different source digest", path.display());
    }
    let len = u64::from_le_bytes(b[40..48].try_into().unwrap()) as usize;
    if b.len() != HEADER + len {
        bail!("{}: payload length {} != {}", path.display(), b.len() - HEADER, len);
    }
    let payload = &b[HEADER..];
    if Sha256::digest(payload).as_slice() != &b[48..80] {
        bail!("{}: payload digest mismatch", path.display());
    }
    decode(payload)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadSource {
    /// Read from the binary cache.
    Cache,
    /// Read, verified and parsed from the GLB, then written to the cache.
    Cooked,
}

/// The model for `asset`: from the cache when a valid entry exists, else
/// verified + cooked from the GLB under `repo_root/public` and cached.
/// A digest or length mismatch of the GLB is an error; nothing is drawn from it.
pub fn load_model(repo_root: &Path, cache_dir: Option<&Path>, asset: &AssetRef) -> Result<(CookedModel, LoadSource)> {
    if let Some(dir) = cache_dir {
        let p = model_path(dir, &asset.sha256);
        if p.exists() {
            match read(&p, &asset.sha256) {
                Ok(m) => return Ok((m, LoadSource::Cache)),
                Err(e) => eprintln!("model cache: {e:#}; re-cooking"),
            }
        }
    }
    let bytes = read_verified(&asset.path(repo_root), asset)?;
    let model = cook_glb(&bytes).with_context(|| format!("cooking {}", asset.url))?;
    if let Some(dir) = cache_dir {
        write(&model_path(dir, &asset.sha256), &model, &asset.sha256)?;
    }
    Ok((model, LoadSource::Cooked))
}
