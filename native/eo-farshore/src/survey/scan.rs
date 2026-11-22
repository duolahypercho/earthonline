//! The Poly Haven scans in the survey set (groups S1: rocks and cliff faces;
//! S2: grass and shrubs; CC0 1.0): each a `<name>_2k.gltf`, its geometry
//! buffer (the download kept the buffer under the URL-shaped path
//! `https:/dl.polyhaven.org/file/ph-assets/Models/gltf/<res>/<name>/<name>.bin`,
//! while the glTF names it `<name>.bin`: resolved through the manifest) and
//! the 2k diffuse, OpenGL-convention normal and ARM (occlusion, roughness,
//! metalness) JPEGs, which the glTF names under `textures/` but which lie
//! beside it. Read with the `gltf` crate (no network, nothing executed).
//!
//! A scan is a list of pieces, one per scene node with a mesh: the node's
//! rotation and scale baked into its positions and normals, its translation
//! kept as the piece's `origin` (a shrub scan lays its variants side by side).
//! `Piece::lod_chain` simplifies a piece with `meshopt` keeping its UVs (the
//! simplified indices reference the original vertices; seams are kept as
//! meshopt keeps attribute discontinuities); `lod_chain_cached` stores the
//! chains under the native cache, keyed by the scan files' digests.

use super::{Root, Set};
use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Bump when what the LOD cook writes changes (part of the cache key).
pub const LOD_COOK_VERSION: &str = "survey-scan-lod-5";

/// One mesh of a scan.
#[derive(Clone, Debug)]
pub struct Piece {
    pub name: String,
    /// The node's translation (m, the scan's frame: y up).
    pub origin: [f32; 3],
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// One level of detail: indices into the piece's own vertices.
#[derive(Clone, Debug, PartialEq)]
pub struct Lod {
    /// The triangle count asked for, and the error meshopt reports (relative to the piece's extent).
    pub target: usize,
    pub error: f32,
    pub indices: Vec<u32>,
}

/// A scan's texture files (verified paths).
#[derive(Clone, Debug)]
pub struct Textures {
    pub diffuse: PathBuf,
    pub normal: PathBuf,
    pub arm: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Scan {
    pub group: String,
    pub name: String,
    pub pieces: Vec<Piece>,
    pub textures: Textures,
    /// SHA-256 of the glTF and its buffer (the manifest's), the cache key's input.
    pub digest: [String; 2],
}

/// The scans the manifest lists: (group, name).
pub fn list() -> Vec<(&'static str, &'static str)> {
    super::manifest().sets.iter().filter(|s| s.group == "S1" || s.group == "S2").map(|s| (s.group.as_str(), s.id.as_str())).collect()
}

/// The set's file whose name ends with `suffix`.
fn file_ending<'a>(set: &'a Set, suffix: &str) -> Option<&'a super::FileEntry> {
    set.files.iter().find(|f| f.path.rsplit('/').next().is_some_and(|n| n.ends_with(suffix)))
}

impl Scan {
    /// Scan `name` of `group` (S1 or S2) from the survey root, every file
    /// checked against the manifest.
    pub fn load(root: &Root, group: &str, name: &str) -> Result<Scan> {
        let set = super::set(group, name).ok_or_else(|| anyhow!("{group}/{name}: not in the survey manifest"))?;
        let gltf_e = file_ending(set, ".gltf").ok_or_else(|| anyhow!("{name}: no .gltf"))?;
        let json = std::fs::read(root.verified(&gltf_e.path)?)?;
        let doc = gltf::Gltf::from_slice(&json).with_context(|| format!("{name}: glTF"))?;
        let mut buffers: Vec<Vec<u8>> = vec![];
        let mut bin_sha = String::new();
        for b in doc.buffers() {
            let uri = match b.source() {
                gltf::buffer::Source::Uri(u) => u.to_string(),
                gltf::buffer::Source::Bin => bail!("{name}: an embedded buffer (not a .gltf with a .bin)"),
            };
            let base = uri.rsplit('/').next().unwrap_or(&uri).to_string();
            let e = file_ending(set, &format!("/{base}")).or_else(|| file_ending(set, &base)).ok_or_else(|| anyhow!("{name}: buffer {uri} not in the manifest"))?;
            let data = std::fs::read(root.verified(&e.path)?)?;
            if data.len() < b.length() {
                bail!("{name}: buffer {base} is {} bytes, the glTF says {}", data.len(), b.length());
            }
            bin_sha = e.sha256.clone();
            buffers.push(data);
        }
        let mut pieces = vec![];
        let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or_else(|| anyhow!("{name}: no scene"))?;
        let mut stack: Vec<(gltf::Node, [[f32; 4]; 4])> = scene.nodes().map(|n| (n, IDENTITY)).collect();
        while let Some((node, parent)) = stack.pop() {
            let m = mul(&parent, &node.transform().matrix());
            for c in node.children() {
                stack.push((c, m));
            }
            let Some(mesh) = node.mesh() else { continue };
            let origin = [m[3][0], m[3][1], m[3][2]];
            let mut p = Piece { name: node.name().or(mesh.name()).unwrap_or("piece").to_string(), origin, positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let r = prim.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
                let base = p.positions.len() as u32;
                let pos: Vec<[f32; 3]> = r.read_positions().ok_or_else(|| anyhow!("{name}: no positions"))?.collect();
                let n = pos.len();
                let nor: Vec<[f32; 3]> = r.read_normals().map(|i| i.collect()).unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; n]);
                let uv: Vec<[f32; 2]> = r.read_tex_coords(0).map(|t| t.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; n]);
                if nor.len() != n || uv.len() != n {
                    bail!("{name}: attribute counts differ");
                }
                for (q, nn) in pos.iter().zip(&nor) {
                    let w = apply(&m, *q);
                    p.positions.push([w[0] - origin[0], w[1] - origin[1], w[2] - origin[2]]);
                    p.normals.push(normalize(apply3(&m, *nn)));
                }
                p.uvs.extend(uv);
                match r.read_indices() {
                    Some(ix) => p.indices.extend(ix.into_u32().map(|i| i + base)),
                    None => p.indices.extend(base..base + n as u32),
                }
            }
            if p.indices.iter().any(|&i| i as usize >= p.positions.len()) {
                bail!("{name}: an index past its vertices");
            }
            if !p.indices.is_empty() {
                pieces.push(p);
            }
        }
        if pieces.is_empty() {
            bail!("{name}: no triangles");
        }
        let tex = |suffix: &str| -> Result<PathBuf> {
            let e = file_ending(set, suffix).ok_or_else(|| anyhow!("{name}: no *{suffix}"))?;
            root.verified(&e.path)
        };
        let textures = Textures { diffuse: tex("_diff_2k.jpg")?, normal: tex("_nor_gl_2k.jpg")?, arm: tex("_arm_2k.jpg")? };
        Ok(Scan { group: group.to_string(), name: name.to_string(), pieces, textures, digest: [gltf_e.sha256.clone(), bin_sha] })
    }

    pub fn triangles(&self) -> usize {
        self.pieces.iter().map(|p| p.indices.len() / 3).sum()
    }
}

impl Piece {
    /// Levels of detail at about `targets` triangles each (largest first;
    /// a target at or over the piece's own count keeps it whole), each
    /// simplified from the one before it, UVs and normals weighted so the
    /// texture stays where it was (weights INFERRED: UV 1, normal 0.5).
    /// Level `k` stops at `errors[k]` (meshopt's error relative to the
    /// piece's extent; missing: unbounded) before its target when the
    /// shape would bend further, so it keeps more triangles than asked
    /// rather than a distorted outline.
    pub fn lod_chain(&self, targets: &[usize], errors: &[f32]) -> Vec<Lod> {
        let mut attrs = Vec::with_capacity(self.positions.len() * 5);
        for (uv, n) in self.uvs.iter().zip(&self.normals) {
            attrs.extend_from_slice(&[uv[0], uv[1], n[0], n[1], n[2]]);
        }
        let weights = [1.0, 1.0, 0.5, 0.5, 0.5];
        let locks = vec![false; self.positions.len()];
        let mut sorted: Vec<(usize, f32)> = targets.iter().enumerate().map(|(k, &t)| (t, errors.get(k).copied().unwrap_or(1.0))).collect();
        sorted.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        let mut prev = self.indices.clone();
        // the error so far: each level is simplified from the one before, and meshopt measures
        // a step against its input, so a level's error over the source is at most the sum
        let mut prev_err = 0.0f32;
        let mut out = vec![];
        for (t, max_err) in sorted {
            let indices = if t * 3 >= prev.len() {
                prev.clone()
            } else {
                let max_err = (max_err - prev_err).max(0.25 * max_err);
                let mut err = 0.0f32;
                let mut ix = meshopt::simplify_with_attributes_and_locks_decoder(&prev, &self.positions, &attrs, &weights, 5 * std::mem::size_of::<f32>(), &locks, t * 3, max_err, meshopt::SimplifyOptions::None, Some(&mut err));
                // the error the attribute-weighted collapse reports counts its
                // UVs' and normals' change too (meshopt), which on a scan of many
                // small UV islands (boulder_01) reaches the bound long before the
                // shape bends: past a third over the target, the shape-only
                // collapse within the same bound, whose error is the outline's
                // alone, allowed across the islands' seams (meshopt
                // `Permissive`: the UVs still the original vertices', so a seam
                // crossed may stretch its texture over a triangle).
                if ix.len() > t * 3 * 4 / 3 {
                    let mut eg = 0.0f32;
                    let geo = meshopt::simplify_decoder(&prev, &self.positions, t * 3, max_err, meshopt::SimplifyOptions::Permissive, Some(&mut eg));
                    if geo.len() < ix.len() && !geo.is_empty() {
                        ix = geo;
                        err = eg;
                    }
                }
                // an open scan's rim stops the topology-keeping collapse near
                // its rim's vertex count (about a thousand triangles for the
                // scanned patches) well under the error bound: past a third
                // over the target, the clustering simplifier, which reaches any
                // count within the same bound (the far levels only; their UVs
                // may stretch at a seam). Stopped by the bound itself, the level
                // keeps its extra triangles.
                if ix.len() > t * 3 * 4 / 3 && err < 0.5 * max_err {
                    let mut e2 = 0.0f32;
                    let sloppy = meshopt::simplify_sloppy_decoder(&prev, &self.positions, t, max_err, Some(&mut e2));
                    if sloppy.len() < ix.len() && !sloppy.is_empty() {
                        ix = sloppy;
                        err = e2;
                    }
                }
                let err = prev_err + err;
                prev_err = err;
                out.push(Lod { target: t, error: err, indices: ix.clone() });
                prev = ix;
                continue;
            };
            out.push(Lod { target: t, error: prev_err, indices });
        }
        out
    }
}

/// A piece's levels of detail over only the vertices they use: the
/// simplified levels reference a few thousand of a scan's hundreds of
/// thousands of vertices, so the drawn mesh keeps those alone (in first-use
/// order over the levels, finest first) and each level's indices are
/// renumbered onto them.
#[derive(Clone, Debug, PartialEq)]
pub struct Compact {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub lods: Vec<Vec<u32>>,
}

impl Piece {
    /// `lods` (from `lod_chain`) over the vertices they use.
    pub fn compact(&self, lods: &[Lod]) -> Compact {
        let mut map = vec![u32::MAX; self.positions.len()];
        let mut c = Compact { positions: vec![], normals: vec![], uvs: vec![], lods: vec![] };
        for l in lods {
            let mut ix = Vec::with_capacity(l.indices.len());
            for &i in &l.indices {
                let i = i as usize;
                if map[i] == u32::MAX {
                    map[i] = c.positions.len() as u32;
                    c.positions.push(self.positions[i]);
                    c.normals.push(self.normals[i]);
                    c.uvs.push(self.uvs[i]);
                }
                ix.push(map[i]);
            }
            c.lods.push(ix);
        }
        c
    }

    /// The scan's open rim: the positions of the edges one triangle uses
    /// (positions welded at 0.1 mm so a UV seam is no edge), one per welded
    /// vertex. Empty for a closed mesh.
    pub fn rim_points(&self) -> Vec<[f32; 3]> {
        use std::collections::HashMap;
        let q = |p: [f32; 3]| p.map(|v| (v as f64 * 1e4).round() as i64);
        let mut weld: HashMap<[i64; 3], u32> = HashMap::new();
        let ids: Vec<u32> = self.positions.iter().map(|p| {
            let n = weld.len() as u32;
            *weld.entry(q(*p)).or_insert(n)
        }).collect();
        let mut rep = vec![[0.0f32; 3]; weld.len()];
        for (i, p) in self.positions.iter().enumerate() {
            rep[ids[i] as usize] = *p;
        }
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for t in self.indices.chunks_exact(3) {
            for k in 0..3 {
                let (a, b) = (ids[t[k] as usize], ids[t[(k + 1) % 3] as usize]);
                if a != b {
                    *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
        }
        let mut rim: Vec<u32> = edges.iter().filter(|(_, n)| **n == 1).flat_map(|(e, _)| [e.0, e.1]).collect();
        rim.sort_unstable();
        rim.dedup();
        rim.iter().map(|&v| rep[v as usize]).collect()
    }

    /// The plane the scan's open rim lies on (a scanned patch of ground or a
    /// rock face is an open shell whose edge is where the scan stopped, on
    /// the ground it stood on): `RimPlane::fit` over `rim_points`. None for
    /// a closed mesh (a boulder).
    pub fn rim_plane(&self) -> Option<RimPlane> {
        RimPlane::fit(&self.rim_points())
    }
}

/// How far each of `points` lies from the nearest of `rim` (straight line,
/// the scan's metres), up to `range` (`range` where no rim point is nearer:
/// a closed mesh's every vertex). A grid of `range` cells over the rim.
pub fn rim_distance(rim: &[[f32; 3]], points: &[[f32; 3]], range: f32) -> Vec<f32> {
    use std::collections::HashMap;
    let cell = |p: [f32; 3]| p.map(|v| (v / range).floor() as i32);
    let mut grid: HashMap<[i32; 3], Vec<[f32; 3]>> = HashMap::new();
    for p in rim {
        grid.entry(cell(*p)).or_default().push(*p);
    }
    points
        .iter()
        .map(|p| {
            let c = cell(*p);
            let mut best = range * range;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(v) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                            for q in v {
                                let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
                                best = best.min(d);
                            }
                        }
                    }
                }
            }
            best.sqrt()
        })
        .collect()
}

/// The plane an open scan's rim lies on: `y = a x + b z + c` (the scan's
/// frame), and its rim's residual heights (5th and 95th percentile).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RimPlane {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub low: f64,
    pub high: f64,
    pub rim_vertices: usize,
}

impl RimPlane {
    /// Least squares `y = a x + b z + c` through the rim's points, with the
    /// rim's residual heights at the 5th and 95th percentiles; None under
    /// 16 points (no open rim).
    pub fn fit(rim: &[[f32; 3]]) -> Option<RimPlane> {
        if rim.len() < 16 {
            return None;
        }
        // normal equations of y = a x + b z + c
        let mut m = [[0.0f64; 3]; 3];
        let mut r = [0.0f64; 3];
        for p in rim {
            let row = [p[0] as f64, p[2] as f64, 1.0];
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] += row[i] * row[j];
                }
                r[i] += row[i] * p[1] as f64;
            }
        }
        let coef = solve3(m, r)?;
        let mut res: Vec<f64> = rim.iter().map(|p| p[1] as f64 - (coef[0] * p[0] as f64 + coef[1] * p[2] as f64 + coef[2])).collect();
        res.sort_by(f64::total_cmp);
        let pct = |f: f64| res[((res.len() - 1) as f64 * f).round() as usize];
        Some(RimPlane { a: coef[0], b: coef[1], c: coef[2], low: pct(0.05), high: pct(0.95), rim_vertices: rim.len() })
    }

    /// The plane's unit normal (up side).
    pub fn normal(&self) -> [f64; 3] {
        let l = (self.a * self.a + 1.0 + self.b * self.b).sqrt();
        [-self.a / l, 1.0 / l, -self.b / l]
    }
}

fn solve3(m: [[f64; 3]; 3], r: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: &[[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = det(&m);
    if d.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for k in 0..3 {
        let mut mk = m;
        for i in 0..3 {
            mk[i][k] = r[i];
        }
        out[k] = det(&mk) / d;
    }
    Some(out)
}

/// The native cache folder (`EO_NATIVE_CACHE`), if set.
pub fn cache_from_env() -> Option<PathBuf> {
    std::env::var_os("EO_NATIVE_CACHE").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The LOD chains of every piece of `scan` at `targets` within `errors`, from the cache
/// (`<cache>/survey-scan/<name>-<key>.lod`) or simplified now and stored
/// there. The key is the SHA-256 of the cook version, the scan's two file
/// digests, the targets and the error bounds; a damaged cache file is cooked again.
pub fn lod_chain_cached(scan: &Scan, targets: &[usize], errors: &[f32], cache: Option<&Path>) -> Result<Vec<Vec<Lod>>> {
    let mut h = Sha256::new();
    h.update(LOD_COOK_VERSION.as_bytes());
    h.update(scan.digest[0].as_bytes());
    h.update(scan.digest[1].as_bytes());
    for t in targets {
        h.update((*t as u64).to_le_bytes());
    }
    for e in errors {
        h.update(e.to_le_bytes());
    }
    let key: String = h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect();
    let file = cache.map(|c| c.join("survey-scan").join(format!("{}-{key}.lod", scan.name)));
    if let Some(f) = &file {
        if let Some(c) = std::fs::read(f).ok().and_then(|b| decode_lods(&b)).filter(|c| c.len() == scan.pieces.len()) {
            return Ok(c);
        }
    }
    let chains: Vec<Vec<Lod>> = scan.pieces.iter().map(|p| p.lod_chain(targets, errors)).collect();
    if let Some(f) = &file {
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d).ok();
        }
        let tmp = f.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, encode_lods(&chains)).is_ok() {
            std::fs::rename(&tmp, f).ok();
        }
    }
    Ok(chains)
}

/// "EOSL" | u32 pieces | per piece: u32 levels | per level: u32 target, f32 error, u32 n, n u32 | SHA-256 of all before it.
fn encode_lods(chains: &[Vec<Lod>]) -> Vec<u8> {
    let mut b = b"EOSL".to_vec();
    b.extend((chains.len() as u32).to_le_bytes());
    for c in chains {
        b.extend((c.len() as u32).to_le_bytes());
        for l in c {
            b.extend((l.target as u32).to_le_bytes());
            b.extend(l.error.to_le_bytes());
            b.extend((l.indices.len() as u32).to_le_bytes());
            for i in &l.indices {
                b.extend(i.to_le_bytes());
            }
        }
    }
    let d = Sha256::digest(&b);
    b.extend_from_slice(&d);
    b
}

fn decode_lods(b: &[u8]) -> Option<Vec<Vec<Lod>>> {
    if b.len() < 40 || &b[..4] != b"EOSL" {
        return None;
    }
    let (body, sum) = b.split_at(b.len() - 32);
    if Sha256::digest(body).as_slice() != sum {
        return None;
    }
    let mut k = 4;
    let mut u32_at = || -> Option<u32> {
        let v = u32::from_le_bytes(body.get(k..k + 4)?.try_into().ok()?);
        k += 4;
        Some(v)
    };
    let pieces = u32_at()?;
    let mut out = vec![];
    for _ in 0..pieces {
        let levels = u32_at()?;
        let mut c = vec![];
        for _ in 0..levels {
            let target = u32_at()? as usize;
            let error = f32::from_bits(u32_at()?);
            let n = u32_at()? as usize;
            let mut indices = Vec::with_capacity(n);
            for _ in 0..n {
                indices.push(u32_at()?);
            }
            c.push(Lod { target, error, indices });
        }
        out.push(c);
    }
    Some(out)
}

/// A scan texture decoded: (width, height, RGB8 row-major).
pub fn decode_jpeg(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let f = std::fs::File::open(path).with_context(|| format!("{}", path.display()))?;
    let mut dec = jpeg_decoder::Decoder::new(std::io::BufReader::new(f));
    let px = dec.decode().with_context(|| format!("{}: decoding", path.display()))?;
    let info = dec.info().context("no JPEG info")?;
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => px,
        jpeg_decoder::PixelFormat::L8 => px.iter().flat_map(|&v| [v, v, v]).collect(),
        other => bail!("{}: {other:?}", path.display()),
    };
    Ok((info.width as u32, info.height as u32, rgb))
}

const IDENTITY: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// a b, column-major (glTF's `matrix`: m[col][row]).
fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut o = [[0.0; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            o[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    o
}

fn apply(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

/// The linear part only (a direction; the scans' nodes scale uniformly, so it serves the normals).
fn apply3(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2])
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    v.map(|c| c / l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_lists_seven_rocks_and_five_plants() {
        let l = list();
        assert_eq!(l.iter().filter(|s| s.0 == "S1").count(), 7);
        assert_eq!(l.iter().filter(|s| s.0 == "S2").count(), 5);
        let rot = mul(&IDENTITY, &[[0.0, 1.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [2.0, 0.0, 0.0, 1.0]]);
        assert_eq!(apply(&rot, [1.0, 0.0, 0.0]), [2.0, 1.0, 0.0]);
    }

    /// An open square sheet of 4 x 4 cells: its rim is its outer edge, its
    /// rim plane its own plane, and a vertex's rim distance its distance to
    /// the nearest edge (capped at the range).
    #[test]
    fn an_open_sheet_has_its_edge_for_a_rim() {
        let n = 5usize;
        let mut p = Piece { name: "sheet".into(), origin: [0.0; 3], positions: vec![], normals: vec![], uvs: vec![], indices: vec![] };
        for j in 0..n {
            for i in 0..n {
                p.positions.push([i as f32, 0.5 * i as f32, j as f32]);
                p.normals.push([0.0, 1.0, 0.0]);
                p.uvs.push([0.0, 0.0]);
            }
        }
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let v = (j * n + i) as u32;
                p.indices.extend_from_slice(&[v, v + n as u32, v + 1, v + 1, v + n as u32, v + n as u32 + 1]);
            }
        }
        let rim = p.rim_points();
        assert_eq!(rim.len(), 16);
        let plane = p.rim_plane().unwrap();
        assert!((plane.a - 0.5).abs() < 1e-9 && plane.b.abs() < 1e-9 && plane.c.abs() < 1e-9, "{plane:?}");
        let d = rim_distance(&rim, &p.positions, 1.5);
        assert_eq!(d[0], 0.0);
        let centre = 2 * n + 2;
        // (2, 1, 2): two cells from every edge, 2 x hypot(1, 0.5) along x, 2 along z; capped at 1.5
        assert_eq!(d[centre], 1.5);
        let inner = n + 1; // (1, 0.5, 1): one cell in
        assert!((d[inner] - 1.0).abs() < 1e-6, "{}", d[inner]);
    }

    /// A small rock face (S1 rock_face_01) and a shrub (S2 shrub_04) load
    /// with UVs, their LOD chains fall in triangle count with the UVs kept,
    /// the cache round-trips, and the diffuse map is 2k.
    #[test]
    fn a_rock_and_a_shrub_load_and_simplify() {
        let Some(root) = super::super::tests::root_or_skip("scan") else { return };
        let rock = Scan::load(&root, "S1", "rock_face_01").expect("rock_face_01");
        println!("rock_face_01: {} pieces, {} triangles", rock.pieces.len(), rock.triangles());
        assert!(rock.triangles() > 1000);
        let p = &rock.pieces[0];
        assert!(p.uvs.iter().all(|uv| uv[0].is_finite() && uv[1].is_finite()));
        assert!(p.normals.iter().all(|n| ((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() < 1e-3));
        let targets = [4000, 1000, 250];
        let chain = p.lod_chain(&targets, &[]);
        assert_eq!(chain.len(), 3);
        let tris: Vec<usize> = chain.iter().map(|l| l.indices.len() / 3).collect();
        println!("LOD triangles {tris:?}, errors {:?}", chain.iter().map(|l| l.error).collect::<Vec<_>>());
        assert!(tris.windows(2).all(|w| w[0] >= w[1]) && tris[2] < p.indices.len() / 3);
        assert!(chain.iter().all(|l| l.indices.iter().all(|&i| (i as usize) < p.positions.len())));
        let dir = std::env::temp_dir().join(format!("eo-survey-scan-test-{}", std::process::id()));
        let a = lod_chain_cached(&rock, &targets, &[], Some(&dir)).unwrap();
        let b = lod_chain_cached(&rock, &targets, &[], Some(&dir)).unwrap();
        assert_eq!(a, b);
        assert_eq!(a[0], chain);
        std::fs::remove_dir_all(&dir).ok();
        let (w, h, px) = decode_jpeg(&rock.textures.diffuse).unwrap();
        assert_eq!((w, h, px.len()), (2048, 2048, 2048 * 2048 * 3));
        // the levels over the vertices they use: fewer vertices, the same triangles
        let c = p.compact(&chain);
        assert!(c.positions.len() < p.positions.len());
        for (l, ix) in chain.iter().zip(&c.lods) {
            assert_eq!(l.indices.len(), ix.len());
            for (a, b) in l.indices.iter().zip(ix) {
                assert_eq!(p.positions[*a as usize], c.positions[*b as usize]);
                assert_eq!(p.uvs[*a as usize], c.uvs[*b as usize]);
            }
        }
        // a rock face is an open shell standing on its rim: the rim leans back
        // (its plane's normal tilts toward the face, +z), the boulder is closed.
        let rim = p.rim_plane().expect("rock_face_01 is open");
        println!("rock_face_01 rim {rim:?}");
        assert!(rim.normal()[2] > 0.3 && rim.normal()[1] > 0.3, "{:?}", rim.normal());
        let shrub = Scan::load(&root, "S2", "shrub_04").expect("shrub_04");
        println!("shrub_04: {} pieces, {} triangles", shrub.pieces.len(), shrub.triangles());
        assert!(!shrub.pieces.is_empty() && shrub.triangles() > 100);
    }
}
