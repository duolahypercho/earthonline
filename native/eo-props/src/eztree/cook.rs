//! The ez-tree cooker: every (species, tier) the street pass and the streamed
//! tiles draw (every tier each species declares, variant 0, which is the only
//! variant the JS passes ask for), and the street pass's forms (near, window
//! and far for every street species and every silhouette), built
//! once on worker threads and cached as one file:
//!
//!   `<cache_dir>/props/eztree-<key>.bin`
//!
//! `key` = SHA-256 over a format version, the export's bytes
//! (`data/eztree-js.json.gz`) and this module's source, first 16 bytes in hex.
//! File: `EOEZTRE1` (8 bytes), SHA-256 of the payload (32), payload (little
//! endian): u32 build count, then per build: kind u8 (0 ez build, 1 street
//! form), species / tier-or-form strings (u32 length + UTF-8), u32 variant,
//! the userData numbers (u32 seed, 6 x f64, 6 x u32 counts, 3 x f64), u32
//! vertex count, u32 index count, positions, normals (f32 x 3), uvs (f32 x 2),
//! colours (f32 x 3), part (u8), indices (u32); a street form adds f64
//! bounding radius and f64 facade reach / street lean (NaN when not pruned).
//! The payload digest is checked on read; written via a temporary file and a rename.

use super::street::street_form_of;
use super::{EzData, EzMesh, EzUserData, StreetTreeMesh, StreetTreeMode, build_ez_tree};
use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const FORMAT: &str = "eo-props eztree cook v1";
const MAGIC: &[u8; 8] = b"EOEZTRE1";
const SOURCES: &[&[u8]] = &[
    include_bytes!("mod.rs"),
    include_bytes!("generator.rs"),
    include_bytes!("geometry.rs"),
    include_bytes!("street.rs"),
    include_bytes!("cook.rs"),
    include_bytes!("coastal.rs"),
];

/// Which ring a street form is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StreetForm {
    /// `buildStreetTreeGeometry(v, false, false)`.
    Near,
    /// `buildStreetTreeGeometry(v, true, false)`: the window ring and the near trees' shadow proxies.
    Window,
    /// `buildStreetTreeGeometry(v, true, true)`.
    Far,
}

impl StreetForm {
    fn name(self) -> &'static str {
        match self {
            StreetForm::Near => "near",
            StreetForm::Window => "window",
            StreetForm::Far => "far",
        }
    }
    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "near" => StreetForm::Near,
            "window" => StreetForm::Window,
            "far" => StreetForm::Far,
            _ => return None,
        })
    }
    /// The (coarse, distant) flags of `buildStreetTreeGeometry`.
    pub fn flags(self) -> (bool, bool) {
        match self {
            StreetForm::Near => (false, false),
            StreetForm::Window => (true, false),
            StreetForm::Far => (true, true),
        }
    }
    fn tier(self) -> &'static str {
        match self {
            StreetForm::Near => "near",
            StreetForm::Window => "coarse",
            StreetForm::Far => "distant",
        }
    }
}

/// Every cooked tree.
pub struct EzLibrary {
    /// `buildEzTreeGeometry(species, { tier, variant })`, by (species, tier, variant).
    pub builds: BTreeMap<(String, String, u32), Arc<EzMesh>>,
    /// `buildStreetTreeGeometry` forms, by (species, form).
    pub street: BTreeMap<(String, StreetForm), Arc<StreetTreeMesh>>,
    /// Wall time of the bake (or of the cache read), milliseconds.
    pub millis: f64,
    pub from_cache: bool,
    pub cache_path: Option<PathBuf>,
}

impl EzLibrary {
    pub fn get(&self, species: &str, tier: &str, variant: u32) -> Option<&Arc<EzMesh>> {
        self.builds.get(&(species.to_string(), tier.to_string(), variant))
    }

    pub fn street_form(&self, species: &str, form: StreetForm) -> Option<&Arc<StreetTreeMesh>> {
        self.street.get(&(species.to_string(), form))
    }

    /// The form `buildStreetTreeGeometry(variant, coarse, distant)` returns under `mode`.
    pub fn street_tree(&self, variant: usize, coarse: bool, distant: bool, mode: StreetTreeMode) -> Result<&Arc<StreetTreeMesh>> {
        let id = super::street_tree_species_id(variant, if coarse || distant { 1 } else { 0 }, mode)?;
        let form = if distant { StreetForm::Far } else if coarse { StreetForm::Window } else { StreetForm::Near };
        self.street_form(id, form).ok_or_else(|| anyhow!("{id} {form:?} not cooked"))
    }

    /// `streetTreeTrunkRadius(variant, lod)` from the cooked near builds.
    pub fn street_tree_trunk_radius(&self, variant: usize, lod: u32, mode: StreetTreeMode) -> Result<f64> {
        let id = super::street_tree_species_id(variant, lod, mode)?;
        Ok(self.get(id, "near", 0).ok_or_else(|| anyhow!("{id} near not cooked"))?.user.trunk_radius_m)
    }

    /// One line per cooked mesh: name, vertices, wood / leaf / drawn triangles.
    pub fn report(&self) -> String {
        let mut s = String::new();
        for ((sp, tier, v), m) in &self.builds {
            s += &format!("{sp:<22} {tier:<8} v{v}  {:>6} vertices  {:>5} wood + {:>5} leaf = {:>5} triangles\n", m.positions.len(), m.user.wood_triangles, m.user.leaf_triangles, m.triangles());
        }
        for ((sp, form), m) in &self.street {
            s += &format!("street {sp:<15} {:<8}     {:>6} vertices  {:>5} triangles drawn\n", form.name(), m.mesh.positions.len(), m.mesh.triangles());
        }
        s
    }
}

/// The (species, tier) builds and street forms the cooker makes.
fn plan(d: &EzData) -> (Vec<(String, String)>, Vec<(String, StreetForm)>) {
    let builds = d.all_builds();
    // Every street species at every ring: covers the silhouettes of either
    // mode (legacy draws silhouette 0) and, without modes, any variant index.
    let mut ids: Vec<&String> = d.street.species.iter().chain(d.street.silhouettes4.iter()).chain(d.street.silhouettes2.iter()).collect();
    ids.sort();
    ids.dedup();
    let mut forms = Vec::new();
    for s in ids {
        for f in [StreetForm::Near, StreetForm::Window, StreetForm::Far] {
            forms.push((s.clone(), f));
        }
    }
    (builds, forms)
}

fn cache_key(d_bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(FORMAT.as_bytes());
    h.update((d_bytes.len() as u64).to_le_bytes());
    h.update(d_bytes);
    for s in SOURCES {
        h.update((s.len() as u64).to_le_bytes());
        h.update(s);
    }
    h.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Run `jobs` on `threads` workers; results in job order.
fn parallel<T: Send, J: Sync>(jobs: &[J], threads: usize, f: impl Fn(&J) -> Result<T> + Sync) -> Result<Vec<T>> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<Result<T>>>> = Mutex::new((0..jobs.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..threads.max(1).min(jobs.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= jobs.len() {
                        break;
                    }
                    let r = f(&jobs[i]);
                    out.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|r| r.unwrap_or_else(|| Err(anyhow!("job not run")))).collect()
}

/// Bake (or read from the cache) every tree the street pass and the streamed
/// tiles draw. `cache_dir`: the native cache root (`native/cache`); None bakes
/// without a cache.
pub fn cook_all(cache_dir: Option<&Path>) -> Result<Arc<EzLibrary>> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    cook_all_with(cache_dir, threads)
}

/// `cook_all` with an explicit worker count.
pub fn cook_all_with(cache_dir: Option<&Path>, threads: usize) -> Result<Arc<EzLibrary>> {
    let t0 = Instant::now();
    let d = EzData::get()?;
    let path = cache_dir.map(|c| c.join("props").join(format!("eztree-{}.bin", cache_key(super::EZTREE_JS_GZ))));
    if let Some(p) = &path {
        if let Ok(bytes) = std::fs::read(p) {
            match decode(&bytes) {
                Ok((builds, street)) => {
                    return Ok(Arc::new(EzLibrary { builds, street, millis: t0.elapsed().as_secs_f64() * 1e3, from_cache: true, cache_path: path.clone() }));
                }
                Err(e) => eprintln!("eztree cache {} unreadable ({e:#}); baking", p.display()),
            }
        }
    }
    let (build_jobs, form_jobs) = plan(d);
    let built = parallel(&build_jobs, threads, |(sp, tier)| build_ez_tree(sp, tier, 0).with_context(|| format!("ez-tree {sp} {tier}")))?;
    let mut builds = BTreeMap::new();
    for ((sp, tier), m) in build_jobs.iter().zip(built) {
        builds.insert((sp.clone(), tier.clone(), 0u32), Arc::new(m));
    }
    let formed = parallel(&form_jobs, threads, |(sp, form)| {
        let src = builds.get(&(sp.clone(), form.tier().to_string(), 0)).ok_or_else(|| anyhow!("{sp} {} not built", form.tier()))?;
        let (coarse, distant) = form.flags();
        street_form_of(sp, src, coarse, distant)
    })?;
    let mut street = BTreeMap::new();
    for ((sp, form), m) in form_jobs.iter().zip(formed) {
        street.insert((sp.clone(), *form), Arc::new(m));
    }
    if let Some(p) = &path {
        let payload = encode(&builds, &street);
        let mut file = Vec::with_capacity(payload.len() + 40);
        file.extend_from_slice(MAGIC);
        file.extend_from_slice(&Sha256::digest(&payload));
        file.extend_from_slice(&payload);
        if let Err(e) = write_atomic(p, &file) {
            eprintln!("eztree cache {} not written: {e:#}", p.display());
        }
    }
    Ok(Arc::new(EzLibrary { builds, street, millis: t0.elapsed().as_secs_f64() * 1e3, from_cache: false, cache_path: path }))
}

fn write_atomic(p: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension(format!("bin.tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, p).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// encoding
// ---------------------------------------------------------------------------

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn mesh(&mut self, m: &EzMesh) {
        let u = &m.user;
        self.u32(u.seed);
        for v in [u.height_m, u.half_crown_m, u.trunk_radius_m, u.clearance_m, u.crown_cover, u.crown_leaf_scale] {
            self.f64(v);
        }
        for v in [u.wood_vertex_count, u.wood_triangles, u.leaf_triangles, u.pruned_leaf_triangles, u.lifted_leaf_cards, 0] {
            self.u32(v);
        }
        for v in [u.crown_cover_before_rule, 0.0, 0.0] {
            self.f64(v);
        }
        self.u32(m.positions.len() as u32);
        self.u32(m.indices.len() as u32);
        for a in [&m.positions, &m.normals, &m.colors] {
            for p in a.iter() {
                for &x in p {
                    self.f32(x);
                }
            }
        }
        for p in &m.uvs {
            self.f32(p[0]);
            self.f32(p[1]);
        }
        self.0.extend_from_slice(&m.part);
        for &i in &m.indices {
            self.u32(i);
        }
    }
}

type Decoded = (BTreeMap<(String, String, u32), Arc<EzMesh>>, BTreeMap<(String, StreetForm), Arc<StreetTreeMesh>>);

fn encode(builds: &BTreeMap<(String, String, u32), Arc<EzMesh>>, street: &BTreeMap<(String, StreetForm), Arc<StreetTreeMesh>>) -> Vec<u8> {
    let mut w = W(Vec::new());
    w.u32((builds.len() + street.len()) as u32);
    for ((sp, tier, v), m) in builds {
        w.u8(0);
        w.str(sp);
        w.str(tier);
        w.u32(*v);
        w.mesh(m);
    }
    for ((sp, form), m) in street {
        w.u8(1);
        w.str(sp);
        w.str(form.name());
        w.u32(0);
        w.mesh(&m.mesh);
        w.f64(m.bounding_radius_m);
        let (r, l) = m.street_pruned.unwrap_or((f64::NAN, f64::NAN));
        w.f64(r);
        w.f64(l);
    }
    w.0
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.at + n > self.b.len() {
            bail!("truncated");
        }
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into()?))
    }
    fn f32s(&mut self, n: usize) -> Result<Vec<f32>> {
        Ok(self.take(n * 4)?.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
    }
    fn str(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        Ok(std::str::from_utf8(self.take(n)?)?.to_string())
    }
    fn mesh(&mut self) -> Result<EzMesh> {
        let seed = self.u32()?;
        let f: Vec<f64> = (0..6).map(|_| self.f64()).collect::<Result<_>>()?;
        let c: Vec<u32> = (0..6).map(|_| self.u32()).collect::<Result<_>>()?;
        let g: Vec<f64> = (0..3).map(|_| self.f64()).collect::<Result<_>>()?;
        let user = EzUserData {
            seed,
            height_m: f[0],
            half_crown_m: f[1],
            trunk_radius_m: f[2],
            clearance_m: f[3],
            crown_cover: f[4],
            crown_leaf_scale: f[5],
            wood_vertex_count: c[0],
            wood_triangles: c[1],
            leaf_triangles: c[2],
            pruned_leaf_triangles: c[3],
            lifted_leaf_cards: c[4],
            crown_cover_before_rule: g[0],
        };
        let n = self.u32()? as usize;
        let ni = self.u32()? as usize;
        let three = |v: Vec<f32>| v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect::<Vec<_>>();
        let positions = three(self.f32s(n * 3)?);
        let normals = three(self.f32s(n * 3)?);
        let colors = three(self.f32s(n * 3)?);
        let uvs = self.f32s(n * 2)?.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
        let part = self.take(n)?.to_vec();
        let indices = self.take(ni * 4)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>();
        if indices.iter().any(|&i| i as usize >= n) {
            bail!("index out of range");
        }
        Ok(EzMesh { positions, normals, uvs, colors, part, indices, user })
    }
}

fn decode(bytes: &[u8]) -> Result<Decoded> {
    if bytes.len() < 40 || &bytes[..8] != MAGIC {
        bail!("not an eztree cook");
    }
    let payload = &bytes[40..];
    if Sha256::digest(payload).as_slice() != &bytes[8..40] {
        bail!("payload digest mismatch");
    }
    let mut r = R { b: payload, at: 0 };
    let count = r.u32()?;
    let mut builds = BTreeMap::new();
    let mut street = BTreeMap::new();
    for _ in 0..count {
        let kind = r.u8()?;
        let sp = r.str()?;
        let name = r.str()?;
        let v = r.u32()?;
        let mesh = r.mesh()?;
        match kind {
            0 => {
                builds.insert((sp, name, v), Arc::new(mesh));
            }
            1 => {
                let form = StreetForm::from_name(&name).ok_or_else(|| anyhow!("form {name}"))?;
                let bounding_radius_m = r.f64()?;
                let (reach, lean) = (r.f64()?, r.f64()?);
                let street_pruned = if reach.is_nan() { None } else { Some((reach, lean)) };
                street.insert((sp.clone(), form), Arc::new(StreetTreeMesh { species: sp, mesh, bounding_radius_m, street_pruned }));
            }
            k => bail!("kind {k}"),
        }
    }
    if r.at != payload.len() {
        bail!("trailing bytes");
    }
    Ok((builds, street))
}
