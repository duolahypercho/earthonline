//! The street surface's scanned PBR sets, loaded from the files the JS game
//! ships (`public/assets/polyhaven-*`, CC0, provenance JSON beside them):
//! the diffuse JPEG and the packed NRA PNG of each set
//! (`STREET_SURFACE_PBR_SETS` in src/world/streets/street-surface-v2-render.js).
//! Nothing is copied into `native/`: the files are read where they are, each
//! checked against the SHA-256 its provenance records before it is decoded.
//!
//! Layout of the result: one layer per set (0 asphalt, 1 concrete), RGBA8,
//! a full mip chain each; albedo stays sRGB-encoded (the GPU array is
//! `Rgba8UnormSrgb`, filtered in linear) and its mips are averaged in linear
//! light; NRA is linear data (R,G tangent normal xy, B roughness,
//! A = 128 + occlusion x 127) averaged as stored.

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct StreetMapSet {
    pub id: String,
    pub diffuse_path: String,
    pub nra_path: String,
    pub license: String,
    pub author: String,
    /// Linear mean RGB of the decoded diffuse (the JS records it as `meanLinear`).
    pub mean_linear: [f64; 3],
    /// Mean occlusion and roughness of the NRA (the JS's `meanAo`, `meanRoughness`).
    pub mean_ao: f64,
    pub mean_roughness: f64,
}

pub struct StreetMaps {
    pub res: u32,
    /// [layer][mip] RGBA8.
    pub albedo: Vec<Vec<Vec<u8>>>,
    pub nra: Vec<Vec<Vec<u8>>>,
    pub sets: Vec<StreetMapSet>,
    pub load_ms: f64,
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn mip_chain(base: Vec<u8>, res: usize, srgb: bool) -> Vec<Vec<u8>> {
    let lut: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
    let mut chain = vec![base];
    let mut r = res;
    while r > 1 {
        let h = r / 2;
        let prev = chain.last().unwrap();
        let mut next = vec![0u8; h * h * 4];
        for y in 0..h {
            for x in 0..h {
                for c in 0..4 {
                    let at = |xx: usize, yy: usize| prev[(yy * r + xx) * 4 + c];
                    let s = [at(2 * x, 2 * y), at(2 * x + 1, 2 * y), at(2 * x, 2 * y + 1), at(2 * x + 1, 2 * y + 1)];
                    let v = if srgb && c < 3 {
                        let m = s.iter().map(|&b| lut[b as usize]).sum::<f32>() / 4.0;
                        (linear_to_srgb(m) * 255.0).round()
                    } else {
                        (s.iter().map(|&b| b as f32).sum::<f32>() / 4.0).round()
                    };
                    next[(y * h + x) * 4 + c] = v.clamp(0.0, 255.0) as u8;
                }
            }
        }
        chain.push(next);
        r = h;
    }
    chain
}

fn read_verified(repo_root: &Path, rel: &str, sha256: &str) -> Result<Vec<u8>> {
    let p = repo_root.join(rel);
    let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
    let got = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if got != sha256 {
        bail!("{} has SHA-256 {got}, its provenance records {sha256}", p.display());
    }
    Ok(bytes)
}

fn decode_png_rgba(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info()?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or_else(|| anyhow!("png: no buffer size"))?];
    let info = reader.next_frame(&mut buf)?;
    let (w, h) = (info.width as usize, info.height as usize);
    if info.bit_depth != png::BitDepth::Eight {
        bail!("png: {:?} bit depth not supported", info.bit_depth);
    }
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        t => bail!("png: colour type {t:?} not supported"),
    };
    Ok((w, h, rgba))
}

/// One set: (`id`, provenance path, diffuse path, nra path), paths relative to
/// the repo root as the JS names them (a leading `/` is `public/`).
pub fn load_set(repo_root: &Path, id: &str, provenance: &str, diffuse: &str, nra: &str) -> Result<(StreetMapSet, usize, Vec<u8>, Vec<u8>)> {
    let prov: serde_json::Value = serde_json::from_slice(&std::fs::read(repo_root.join(provenance)).with_context(|| format!("reading {provenance}"))?)?;
    let pub_rel = |u: &str| format!("public/{}", u.trim_start_matches('/'));
    let (dpath, npath) = (pub_rel(diffuse), pub_rel(nra));
    let out = &prov["outputs"];
    let sha = |k: &str, path: &str| -> Result<String> {
        let o = &out[k];
        if o["path"].as_str() != Some(path) {
            bail!("{provenance}: output {k} is {:?}, the JS loads {path}", o["path"]);
        }
        Ok(o["sha256"].as_str().ok_or_else(|| anyhow!("{provenance}: no sha256 for {k}"))?.to_string())
    };
    let dbytes = read_verified(repo_root, &dpath, &sha("diffuse", &dpath)?)?;
    let nbytes = read_verified(repo_root, &npath, &sha("nra", &npath)?)?;
    let (dw, dh, rgb) = crate::jpeg::decode_rgb(&dbytes)?;
    let (nw, nh, nra) = decode_png_rgba(&nbytes)?;
    if dw != dh || nw != nh || dw != nw || !dw.is_power_of_two() {
        bail!("{id}: maps are {dw}x{dh} and {nw}x{nh}; square power-of-two maps of one size expected");
    }
    let mut albedo = vec![0u8; dw * dh * 4];
    let mut sum = [0f64; 3];
    for i in 0..dw * dh {
        for c in 0..3 {
            albedo[i * 4 + c] = rgb[i * 3 + c];
            sum[c] += srgb_to_linear(rgb[i * 3 + c] as f32 / 255.0) as f64;
        }
        albedo[i * 4 + 3] = 255;
    }
    let n = (dw * dh) as f64;
    let (mut ao, mut rough) = (0f64, 0f64);
    for i in 0..nw * nh {
        rough += nra[i * 4 + 2] as f64 / 255.0;
        ao += ((nra[i * 4 + 3] as f64 - 128.0) / 127.0).clamp(0.0, 1.0);
    }
    let set = StreetMapSet {
        id: id.to_string(),
        diffuse_path: dpath,
        nra_path: npath,
        license: prov["license"].as_str().unwrap_or("").to_string(),
        author: prov["author"].as_str().unwrap_or("").to_string(),
        mean_linear: [sum[0] / n, sum[1] / n, sum[2] / n],
        mean_ao: ao / n,
        mean_roughness: rough / n,
    };
    Ok((set, dw, albedo, nra))
}

const CACHE_VERSION: &str = "street-maps-v1";

/// `load`, through a cache of the decoded mip chains under
/// `cache_dir/street-maps/` keyed by the source files' digests (the files'
/// SHA-256 are still checked against their provenance on every cook; a cache
/// hit skips the decode). A damaged cache file is refused and re-cooked.
pub fn load_cached(repo_root: &Path, sets: &[(&str, &str, &str, &str); 2], cache_dir: Option<&Path>) -> Result<(StreetMaps, bool)> {
    let t = std::time::Instant::now();
    let mut key = Sha256::new();
    key.update(CACHE_VERSION.as_bytes());
    let mut metas = vec![];
    for (id, prov, d, n) in sets {
        let pj: serde_json::Value = serde_json::from_slice(&std::fs::read(repo_root.join(prov)).with_context(|| format!("reading {prov}"))?)?;
        for k in ["diffuse", "nra"] {
            key.update(pj["outputs"][k]["sha256"].as_str().unwrap_or("").as_bytes());
        }
        key.update(id.as_bytes());
        key.update(d.as_bytes());
        key.update(n.as_bytes());
        metas.push(pj);
    }
    let hex: String = key.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect();
    let file = cache_dir.map(|c| c.join("street-maps").join(format!("{hex}.eosm")));
    if let Some(f) = &file {
        if let Ok(bytes) = std::fs::read(f) {
            if let Some(m) = decode_cache(&bytes, sets, &metas) {
                let mut m = m;
                m.load_ms = t.elapsed().as_secs_f64() * 1e3;
                return Ok((m, true));
            }
        }
    }
    let m = load(repo_root, sets)?;
    if let Some(f) = &file {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let mut out: Vec<u8> = vec![];
        out.extend_from_slice(&m.res.to_le_bytes());
        out.extend_from_slice(&(m.sets.len() as u32).to_le_bytes());
        for s in &m.sets {
            for v in s.mean_linear.iter().chain([s.mean_ao, s.mean_roughness].iter()) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for chain in m.albedo.iter().chain(m.nra.iter()) {
            for lvl in chain {
                out.extend_from_slice(lvl);
            }
        }
        let digest = Sha256::digest(&out);
        out.extend_from_slice(&digest);
        let tmp = f.with_extension("tmp");
        if std::fs::write(&tmp, &out).is_ok() {
            let _ = std::fs::rename(&tmp, f);
        }
    }
    Ok((m, false))
}

fn decode_cache(bytes: &[u8], sets: &[(&str, &str, &str, &str); 2], metas: &[serde_json::Value]) -> Option<StreetMaps> {
    if bytes.len() < 40 {
        return None;
    }
    let (body, digest) = bytes.split_at(bytes.len() - 32);
    if Sha256::digest(body).as_slice() != digest {
        return None;
    }
    let res = u32::from_le_bytes(body[0..4].try_into().ok()?);
    let n = u32::from_le_bytes(body[4..8].try_into().ok()?) as usize;
    if n != sets.len() || !res.is_power_of_two() {
        return None;
    }
    let mut at = 8;
    let mut out = StreetMaps { res, albedo: vec![], nra: vec![], sets: vec![], load_ms: 0.0 };
    for (k, (id, _, d, nr)) in sets.iter().enumerate() {
        let mut v = [0f64; 5];
        for x in v.iter_mut() {
            *x = f64::from_le_bytes(body.get(at..at + 8)?.try_into().ok()?);
            at += 8;
        }
        let pub_rel = |u: &str| format!("public/{}", u.trim_start_matches('/'));
        out.sets.push(StreetMapSet {
            id: id.to_string(),
            diffuse_path: pub_rel(d),
            nra_path: pub_rel(nr),
            license: metas[k]["license"].as_str().unwrap_or("").to_string(),
            author: metas[k]["author"].as_str().unwrap_or("").to_string(),
            mean_linear: [v[0], v[1], v[2]],
            mean_ao: v[3],
            mean_roughness: v[4],
        });
    }
    let mips = res.ilog2() + 1;
    for which in 0..2 {
        for _ in 0..n {
            let mut chain = vec![];
            for m in 0..mips {
                let r = (res >> m).max(1) as usize;
                let len = r * r * 4;
                chain.push(body.get(at..at + len)?.to_vec());
                at += len;
            }
            if which == 0 { out.albedo.push(chain) } else { out.nra.push(chain) }
        }
    }
    (at == body.len()).then_some(out)
}

/// one cell of the scanned facade atlas (`FACADE_ATLAS_V2`,
/// src/render/materials/fallback-facade.js; `public/assets/facade-atlas-v2-*`,
/// CC0, layout and per-cell sources in facade-atlas-v2.provenance.json) as a
/// kit map layer of `res` px: the cell's unpadded content (the outer pad ring
/// dropped), box-resampled; albedo sRGB as stored; normal RG = the atlas's GL
/// tangent xy with z rebuilt; ORM = (occlusion 1, the atlas's roughness B,
/// metalness 0) - the atlas has no occlusion channel and the JS kit material
/// sets AO 1 for it. Returns (albedo, normal, orm) RGBA8.
pub fn facade_atlas_cell(repo_root: &Path, cell_id: &str, res: usize) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    facade_atlas_cell_cached(repo_root, cell_id, res, None)
}

/// `facade_atlas_cell` through a cache under `cache_dir/kit/` keyed by the
/// atlas files' recorded digests, the cell and the size (the two 4096 x 2048
/// JPEG decodes cost about a second on every launch otherwise). The cache
/// file ends with the SHA-256 of its payload; a damaged one is re-cooked.
pub fn facade_atlas_cell_cached(repo_root: &Path, cell_id: &str, res: usize, cache_dir: Option<&Path>) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let prov_path = "public/assets/facade-atlas-v2.provenance.json";
    let prov: serde_json::Value = serde_json::from_slice(&std::fs::read(repo_root.join(prov_path)).with_context(|| format!("reading {prov_path}"))?)?;
    let n = res * res * 4;
    let file = cache_dir.map(|c| {
        let mut h = Sha256::new();
        h.update(b"facade-atlas-cell-v1");
        h.update(prov["outputs"]["albedo"]["sha256"].as_str().unwrap_or("").as_bytes());
        h.update(prov["outputs"]["normal"]["sha256"].as_str().unwrap_or("").as_bytes());
        h.update(cell_id.as_bytes());
        h.update(res.to_le_bytes());
        let hex: String = h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect();
        c.join("kit").join(format!("atlas-{cell_id}-{res}-{hex}.bin"))
    });
    if let Some(f) = &file {
        if let Ok(b) = std::fs::read(f) {
            if b.len() == 3 * n + 32 && Sha256::digest(&b[..3 * n]).as_slice() == &b[3 * n..] {
                return Ok((b[..n].to_vec(), b[n..2 * n].to_vec(), b[2 * n..3 * n].to_vec()));
            }
        }
    }
    let out = facade_atlas_cell_decode(repo_root, &prov, prov_path, cell_id, res)?;
    if let Some(f) = &file {
        let mut b = Vec::with_capacity(3 * n + 32);
        b.extend_from_slice(&out.0);
        b.extend_from_slice(&out.1);
        b.extend_from_slice(&out.2);
        let d = Sha256::digest(&b);
        b.extend_from_slice(&d);
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let tmp = f.with_extension("tmp");
        if std::fs::write(&tmp, &b).is_ok() {
            let _ = std::fs::rename(&tmp, f);
        }
    }
    Ok(out)
}

fn facade_atlas_cell_decode(repo_root: &Path, prov: &serde_json::Value, prov_path: &str, cell_id: &str, res: usize) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let cell = prov["cells"].as_array().and_then(|c| c.iter().find(|c| c["id"].as_str() == Some(cell_id))).ok_or_else(|| anyhow!("atlas cell {cell_id} not in {prov_path}"))?;
    let lay = &prov["layout"];
    let (cell_px, pad) = (lay["cellPx"].as_u64().unwrap_or(1024) as usize, lay["padPx"].as_u64().unwrap_or(32) as usize);
    let (x0, y0) = (cell["placement"]["x0"].as_u64().unwrap_or(0) as usize, cell["placement"]["y0"].as_u64().unwrap_or(0) as usize);
    let out = &prov["outputs"];
    let read = |k: &str| -> Result<(usize, usize, Vec<u8>)> {
        let path = out[k]["path"].as_str().ok_or_else(|| anyhow!("{prov_path}: no {k} path"))?;
        let bytes = read_verified(repo_root, path, out[k]["sha256"].as_str().unwrap_or(""))?;
        crate::jpeg::decode_rgb(&bytes)
    };
    let (aw, _, alb) = read("albedo")?;
    let (nw, _, nrm) = read("normal")?;
    let content = cell_px - 2 * pad;
    let resample = |img: &[u8], w: usize, srgb: bool| -> Vec<[f32; 3]> {
        let mut o = vec![[0f32; 3]; res * res];
        for y in 0..res {
            for x in 0..res {
                let (sx0, sx1) = (x * content / res, ((x + 1) * content / res).max(x * content / res + 1));
                let (sy0, sy1) = (y * content / res, ((y + 1) * content / res).max(y * content / res + 1));
                let mut acc = [0f32; 3];
                let mut n = 0f32;
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        let p = ((y0 + pad + sy) * w + x0 + pad + sx) * 3;
                        for c in 0..3 {
                            let v = img[p + c] as f32 / 255.0;
                            acc[c] += if srgb { srgb_to_linear(v) } else { v };
                        }
                        n += 1.0;
                    }
                }
                o[y * res + x] = acc.map(|a| a / n);
            }
        }
        o
    };
    let a = resample(&alb, aw, true);
    let nr = resample(&nrm, nw, false);
    let mut albedo = vec![0u8; res * res * 4];
    let mut normal = vec![0u8; res * res * 4];
    let mut orm = vec![0u8; res * res * 4];
    for i in 0..res * res {
        for c in 0..3 {
            albedo[i * 4 + c] = (linear_to_srgb(a[i][c]) * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        albedo[i * 4 + 3] = 255;
        let (nx, ny) = (nr[i][0] * 2.0 - 1.0, nr[i][1] * 2.0 - 1.0);
        let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
        normal[i * 4..i * 4 + 4].copy_from_slice(&[nr[i][0], nr[i][1], nz * 0.5 + 0.5, 1.0].map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8));
        orm[i * 4..i * 4 + 4].copy_from_slice(&[255, (nr[i][2] * 255.0).round().clamp(0.0, 255.0) as u8, 0, 255]);
    }
    Ok((albedo, normal, orm))
}

/// Both sets as GPU-ready layers (asphalt, concrete), decoded on two threads.
pub fn load(repo_root: &Path, sets: &[(&str, &str, &str, &str); 2]) -> Result<StreetMaps> {
    let t = std::time::Instant::now();
    let results: Vec<Result<(StreetMapSet, usize, Vec<Vec<u8>>, Vec<Vec<u8>>)>> = std::thread::scope(|s| {
        let hs: Vec<_> = sets
            .iter()
            .map(|(id, prov, d, n)| {
                s.spawn(move || {
                    let (set, res, albedo, nra) = load_set(repo_root, id, prov, d, n)?;
                    let (a, b) = std::thread::scope(|s2| {
                        let ha = s2.spawn(move || mip_chain(albedo, res, true));
                        let hb = s2.spawn(move || mip_chain(nra, res, false));
                        (ha.join().unwrap(), hb.join().unwrap())
                    });
                    Ok((set, res, a, b))
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut out = StreetMaps { res: 0, albedo: vec![], nra: vec![], sets: vec![], load_ms: 0.0 };
    for r in results {
        let (set, res, a, b) = r?;
        if out.res != 0 && out.res != res as u32 {
            bail!("street sets differ in size");
        }
        out.res = res as u32;
        out.sets.push(set);
        out.albedo.push(a);
        out.nra.push(b);
    }
    out.load_ms = t.elapsed().as_secs_f64() * 1e3;
    Ok(out)
}
