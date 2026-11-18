//! Every body and character in one GPU-ready library: one vertex array, one
//! index array, a mesh table (an opaque and a transparent index range per
//! LOD), a model table (four LOD slots, wheel pivots, bounds, skeleton), one
//! material table, the civilian atlases and the baked joint matrices.
//!
//! Built from the verified GLBs (several threads), or read from
//! `cache/agents/<key>.eoal`, content-addressed by the SHA-256 of every source
//! digest, the JS export and this cook's version, with its own payload digest
//! checked on read (a damaged file is cooked again from the GLBs).

use crate::catalog::{AgentsJs, read_verified};
use crate::motion::MotionRoots;
use crate::people::{Atlas, CookedTier};
use crate::rocketbox;
use crate::vehicles::{CookedBody, LampColours, box_body, cook_body};
use crate::{AgentMaterial, AgentVertex, CLASS_PERSON, CLASS_VEHICLE, CLIPS, GpuMesh, GpuModel, LODS, M_ATLAS, NO_MESH};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Bump when the cooked layout or the cook changes.
pub const COOK_VERSION: u32 = 5; // 4 -> 5 (people moved by the Rocketbox clips and 100STYLE walks only)
const MAGIC: &[u8; 4] = b"EOAL";
/// People tiers in LOD slot order.
pub const PERSON_TIERS: [&str; LODS] = ["hero", "near", "mid", "far"];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub model_names: Vec<String>,
    pub vehicle_models: BTreeMap<String, u32>,
    /// The people of the street pool (model indices).
    pub people_models: Vec<u32>,
    pub player_model: u32,
    pub clip_frames: [u32; CLIPS],
    pub clip_base: [u32; CLIPS],
    /// Seconds a clip slot loops over (the walk: one cycle).
    pub clip_seconds: [f32; CLIPS],
    pub atlas_res: u32,
    /// Opaque + transparent triangles per mesh.
    pub mesh_triangles: Vec<u32>,
    /// Assets not in this checkout (a box stands in), or refused.
    pub missing: Vec<String>,
    pub skeletons: u32,
    pub family_heights: Vec<f32>,
    /// who walks the streets: "rocketbox", or "none" (no people drawn).
    pub people_source: String,
    /// why no people are drawn, when they are not.
    pub people_fallback: Option<String>,
    /// Rocketbox avatars kept out of the random pool, by role (not cooked).
    pub reserved: BTreeMap<String, Vec<String>>,
    /// pool avatars left out by the mesh budget, and tiers replaced.
    pub people_notes: Vec<String>,
    /// first-run cook of the people, ms (0 when read from the cache).
    pub people_cook_ms: f64,
    /// bytes of the people's joint-matrix tables.
    pub people_bone_bytes: u64,
    /// distinct Bip01 skeletons baked (proportion groups).
    pub people_groups: u32,
    /// the slots' layout and each person's walk (empty: today's layout).
    pub motion: crate::motion::MotionMeta,
}

/// where the street people come from.
#[derive(Clone, Debug, PartialEq)]
pub enum PeopleSource {
    /// No people (`--no-rocketbox`, or no copy in the people folder).
    Off,
    /// The Rocketbox avatars under this root (no people if they cannot be cooked).
    Rocketbox(PathBuf),
}

impl PeopleSource {
    /// The people folder (`<repo>/external/people`, or the one `EO_PEOPLE_ROOT`
    /// names) when it holds a copy, else no people.
    pub fn from_env() -> Self {
        match rocketbox::root_from_env() {
            Some(p) => PeopleSource::Rocketbox(p),
            None => PeopleSource::Off,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Library {
    pub vertices: Vec<AgentVertex>,
    pub indices: Vec<u32>,
    pub meshes: Vec<GpuMesh>,
    pub materials: Vec<AgentMaterial>,
    pub models: Vec<GpuModel>,
    pub bones: Vec<[f32; 4]>,
    pub atlases: Vec<Atlas>,
    pub meta: Meta,
    /// How it was loaded (not cached).
    pub from_cache: bool,
    pub load_ms: f64,
    pub cache_path: Option<PathBuf>,
}

struct Builder {
    lib: Library,
    mat_ids: HashMap<Vec<u8>, u32>,
}

impl Builder {
    fn material(&mut self, m: &AgentMaterial) -> u32 {
        let key = bytemuck::bytes_of(m).to_vec();
        if let Some(&i) = self.mat_ids.get(&key) {
            return i;
        }
        let i = self.lib.materials.len() as u32;
        self.lib.materials.push(*m);
        self.mat_ids.insert(key, i);
        i
    }

    /// Append a cooked body as one mesh; returns the mesh index.
    fn body(&mut self, b: &CookedBody) -> u32 {
        let remap: Vec<u32> = b.materials.iter().map(|m| self.material(m)).collect();
        let base = self.lib.vertices.len() as i32;
        for v in &b.vertices {
            let local = v.tag & 0xffff;
            let mut v = *v;
            v.tag = (v.tag & !0xffff) | remap[local as usize];
            self.lib.vertices.push(v);
        }
        let first = self.lib.indices.len() as u32;
        self.lib.indices.extend_from_slice(&b.opaque);
        let glass_first = self.lib.indices.len() as u32;
        self.lib.indices.extend_from_slice(&b.glass);
        self.lib.meshes.push(GpuMesh {
            first_index: first,
            index_count: b.opaque.len() as u32,
            glass_first,
            glass_count: b.glass.len() as u32,
            base_vertex: base,
            pad: [0; 3],
        });
        self.lib.meta.mesh_triangles.push(b.triangles);
        self.lib.meshes.len() as u32 - 1
    }

    fn tier(&mut self, t: &CookedTier, material: u32, skeleton: u32, joints: u32) -> u32 {
        let base = self.lib.vertices.len() as i32;
        for v in &t.vertices {
            let mut v = *v;
            v.tag = (v.tag & !0xffff) | material;
            self.lib.vertices.push(v);
        }
        let first = self.lib.indices.len() as u32;
        self.lib.indices.extend_from_slice(&t.indices);
        self.lib.meshes.push(GpuMesh {
            first_index: first,
            index_count: t.indices.len() as u32,
            glass_first: self.lib.indices.len() as u32,
            glass_count: 0,
            base_vertex: base,
            pad: [skeleton, joints, 0],
        });
        self.lib.meta.mesh_triangles.push(t.indices.len() as u32 / 3);
        self.lib.meshes.len() as u32 - 1
    }
}

/// The cache key: every source digest, the JS export and the cook version
/// (and, for the Rocketbox people, their manifest, motions and cook versions).
fn cache_key(repo_root: &Path, js: &AgentsJs, people: &PeopleSource, motions: &MotionRoots) -> String {
    let mut h = Sha256::new();
    h.update(COOK_VERSION.to_le_bytes());
    h.update(crate::catalog::AGENTS_JS.as_bytes());
    // Which vehicle files this checkout holds (a body left out of it is not drawn).
    for b in js.vehicles.values() {
        for l in &b.lods {
            h.update([l.asset().path(repo_root).is_file() as u8]);
        }
    }
    if let PeopleSource::Rocketbox(_) = people {
        h.update(b"rocketbox");
        h.update(rocketbox::COOK_VERSION.to_le_bytes());
        h.update(rocketbox::Manifest::digest().as_bytes());
        // which motions are cooked in.
        if motions.any() {
            h.update(b"rocketbox-motions");
            h.update(crate::motion::COOK_VERSION.to_le_bytes());
            if motions.clips.is_some() {
                h.update(b"clips");
                h.update(eo_assets::glb::sha256_hex(crate::motion::CLIPS_MANIFEST_JSON.as_bytes()).as_bytes());
            }
            if motions.styles.is_some() {
                h.update(b"styles");
                h.update(eo_assets::glb::sha256_hex(crate::motion::STYLES_MANIFEST_JSON.as_bytes()).as_bytes());
                // The walked styles themselves, so trimming the list re-cooks.
                for (style, phone) in crate::motion::STYLES {
                    h.update(style.as_bytes());
                    h.update([phone as u8]);
                }
            }
        }
    }
    eo_assets::glb::sha256_hex(&h.finalize())
}

pub fn cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join("agents").join(format!("{key}.eoal"))
}

impl Library {
    /// Load from the cache, else cook from the GLBs (and write the cache).
    /// The street people are the Rocketbox people when the people folder
    /// holds a copy (`rocketbox::root_from_env`), else there are none.
    pub fn load(repo_root: &Path, cache_dir: Option<&Path>) -> Result<Self> {
        Self::load_with(repo_root, cache_dir, &PeopleSource::from_env())
    }

    /// `load` with the people source chosen by the caller (`--no-rocketbox`).
    pub fn load_with(repo_root: &Path, cache_dir: Option<&Path>, people: &PeopleSource) -> Result<Self> {
        let t = Instant::now();
        let js = AgentsJs::load()?;
        // the Rocketbox clips and the 100STYLE walks, when present and not switched off.
        let motions = if matches!(people, PeopleSource::Rocketbox(_)) { MotionRoots::from_env() } else { MotionRoots::default() };
        let key = cache_key(repo_root, &js, people, &motions);
        if let Some(dir) = cache_dir {
            let p = cache_path(dir, &key);
            if p.exists() {
                match read_cache(&p) {
                    Ok(mut lib) => {
                        lib.from_cache = true;
                        lib.load_ms = t.elapsed().as_secs_f64() * 1e3;
                        lib.cache_path = Some(p);
                        say_people(&lib, people, &motions);
                        return Ok(lib);
                    }
                    Err(e) => eprintln!("agents: cache {} refused ({e:#}); cooking again", p.display()),
                }
            }
        }
        let mut lib = cook_full(repo_root, &js, people, &motions)?;
        // a library without its people is not cached (the next start tries them again).
        if let Some(dir) = cache_dir.filter(|_| lib.meta.people_fallback.is_none() && lib.meta.motion.fallback.is_empty()) {
            let p = cache_path(dir, &key);
            if let Err(e) = write_cache(&p, &lib) {
                eprintln!("agents: could not write {} ({e:#})", p.display());
            } else {
                lib.cache_path = Some(p);
            }
        }
        lib.load_ms = t.elapsed().as_secs_f64() * 1e3;
        say_people(&lib, people, &motions);
        Ok(lib)
    }

    pub fn model(&self, name: &str) -> Option<u32> {
        self.meta.vehicle_models.get(name).copied()
    }

    pub fn triangles(&self, mesh: u32) -> u32 {
        self.meta.mesh_triangles.get(mesh as usize).copied().unwrap_or(0)
    }
}

/// One line when the people are drawn: which folders they came from.
fn say_people(lib: &Library, people: &PeopleSource, motions: &MotionRoots) {
    if let ("rocketbox", PeopleSource::Rocketbox(root), Some(clips), Some(styles)) = (lib.meta.people_source.as_str(), people, &motions.clips, &motions.styles) {
        eprintln!(
            "agents: people drawn ({}): Rocketbox avatars from {}, clips from {}, walks from {}",
            if lib.from_cache { "from the cache" } else { "cooked" },
            root.display(),
            clips.display(),
            styles.display()
        );
    }
}

/// Cook everything from the verified GLBs (no people).
pub fn cook(repo_root: &Path, js: &AgentsJs) -> Result<Library> {
    cook_with(repo_root, js, &PeopleSource::Off)
}

/// cook everything, the street people from `people`.
pub fn cook_with(repo_root: &Path, js: &AgentsJs, people: &PeopleSource) -> Result<Library> {
    let motions = if matches!(people, PeopleSource::Rocketbox(_)) { MotionRoots::from_env() } else { MotionRoots::default() };
    cook_full(repo_root, js, people, &motions)
}

/// `cook_with`, the Rocketbox people moved by `motions`.
pub fn cook_full(repo_root: &Path, js: &AgentsJs, people: &PeopleSource, motions: &MotionRoots) -> Result<Library> {
    let lamps = LampColours::from_js(js);
    // Vehicles: every LOD of every body, on worker threads.
    let jobs: Vec<(String, usize, crate::catalog::Lod)> =
        js.vehicles.iter().flat_map(|(id, b)| b.lods.iter().enumerate().map(move |(k, l)| (id.clone(), k, l.clone()))).collect();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 12);
    let chunk = jobs.len().div_ceil(threads).max(1);
    let cooked: Vec<(String, usize, Result<CookedBody>)> = std::thread::scope(|s| {
        let hs: Vec<_> = jobs
            .chunks(chunk)
            .map(|c| {
                let lamps = &lamps;
                s.spawn(move || {
                    c.iter()
                        .map(|(id, k, l)| (id.clone(), *k, read_verified(repo_root, &l.asset()).and_then(|b| cook_body(&b, lamps)).with_context(|| l.url.clone())))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().expect("vehicle cook thread")).collect()
    });
    let mut by_body: BTreeMap<String, [Option<CookedBody>; 3]> = BTreeMap::new();
    let mut b = Builder { lib: Library::default(), mat_ids: HashMap::new() };
    for (id, k, r) in cooked {
        match r {
            Ok(c) => by_body.entry(id).or_default()[k.min(2)] = Some(c),
            Err(e) => {
                b.lib.meta.missing.push(format!("{e:#}"));
                by_body.entry(id).or_default();
            }
        }
    }
    // A body with none of its files in this checkout is left out: traffic draws the others.
    by_body.retain(|id, lods| {
        let any = lods.iter().any(|l| l.is_some());
        if !any {
            eprintln!("agents: vehicle body {id} is not drawn (none of its files is in this checkout or verifies); traffic draws the other bodies");
        }
        any
    });
    for (id, lods) in &by_body {
        let body = &js.vehicles[id];
        // Bounds from the finest LOD present, else from the manifest length.
        let (min, max) = lods
            .iter()
            .flatten()
            .next()
            .map(|c| (c.min, c.max))
            .unwrap_or_else(|| {
                let l = body.length_m as f32;
                ([-0.9, 0.0, -l / 2.0], [0.9, 1.5, l / 2.0])
            });
        let boxed = box_body(min, max);
        let mut meshes = [NO_MESH; LODS];
        for k in 0..3 {
            meshes[k] = match &lods[k] {
                Some(c) => b.body(c),
                None => b.body(&boxed),
            };
        }
        meshes[3] = b.body(&boxed);
        let pivots = lods.iter().flatten().next().map(|c| c.pivots).unwrap_or_default();
        let mut wheel = [[0f32; 4]; 4];
        for (w, p) in pivots.iter().enumerate() {
            let p = p.unwrap_or([0.0; 3]);
            wheel[w] = [p[0], p[1], p[2], 0.0];
        }
        let c = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, (min[2] + max[2]) / 2.0];
        let r = ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2) + (max[2] - min[2]).powi(2)).sqrt() / 2.0;
        let m = b.lib.models.len() as u32;
        b.lib.models.push(GpuModel { meshes, wheel, center: c, radius: r, class: CLASS_VEHICLE, skeleton: 0, joints: 0, wheel_radius: body.wheel_radius_m as f32, pad: [0; 4] });
        b.lib.meta.model_names.push(id.clone());
        b.lib.meta.vehicle_models.insert(id.clone(), m);
    }

    // People: the Rocketbox avatars moved by their own clips and the 100STYLE
    // walks; without them no people are drawn, and one line says why.
    let mut atlas_res = 0;
    b.lib.meta.people_source = "none".into();
    match people {
        PeopleSource::Rocketbox(root) => {
            let t = Instant::now();
            match cook_rocketbox(&mut b, root, motions) {
                Ok(res) => {
                    atlas_res = res;
                    b.lib.meta.people_source = "rocketbox".into();
                    b.lib.meta.people_cook_ms = t.elapsed().as_secs_f64() * 1e3;
                }
                Err(e) => {
                    eprintln!("agents: people are not drawn: {e:#}");
                    b.lib.meta.people_fallback = Some(format!("{e:#}"));
                }
            }
        }
        PeopleSource::Off => {
            let why = format!("no Rocketbox people (--no-rocketbox, or no copy at {})", eo_data::external::PEOPLE.looked_in());
            eprintln!("agents: people are not drawn: {why}");
            b.lib.meta.people_fallback = Some(why);
        }
    }
    b.lib.meta.atlas_res = atlas_res;
    if b.lib.indices.is_empty() {
        return Err(anyhow!("agent library is empty"));
    }
    Ok(b.lib)
}

/// the most meshes the agent cull addresses (its record keeps the mesh in 9 bits).
pub const MAX_MESHES: usize = 511;

/// As many Rocketbox pool avatars as the mesh budget leaves room for (four
/// tiers each), with one bake per distinct Bip01 skeleton, moved by the
/// Rocketbox clips and the 100STYLE walks (both are needed: there is no other
/// motion). The player's body is the first pool avatar. Returns the atlas
/// side. Nothing is appended to the library unless at least one avatar cooked.
fn cook_rocketbox(b: &mut Builder, root: &Path, motions: &MotionRoots) -> Result<u32> {
    let manifest = rocketbox::Manifest::load()?;
    // The licence travels with the copy: no LICENSES.md (as the manifest pins it), no people.
    rocketbox::read_verified(root, &manifest, "LICENSES.md").context("the copy's LICENSES.md")?;
    // The motions first: without them nothing could move the avatars.
    let mut motion = crate::motion::MotionMeta::default();
    let mo = motion_sources(motions, &mut motion)?;
    let budget = MAX_MESHES.saturating_sub(b.lib.meshes.len()) / LODS;
    let wanted = manifest.avatars.iter().filter(|a| a.pool).count();
    let pool = rocketbox::pool(&manifest, budget);
    if pool.is_empty() {
        bail!("no room for people in the {MAX_MESHES}-mesh budget");
    }
    if pool.len() < wanted {
        let left: Vec<&str> = manifest.avatars.iter().filter(|a| a.pool).skip(pool.len()).map(|a| a.id.as_str()).collect();
        b.lib.meta.people_notes.push(format!("{} pool avatars over the {MAX_MESHES}-mesh budget, not cooked: {}", left.len(), left.join(", ")));
    }
    // Cook on worker threads.
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut avatars = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = vec![];
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(a) = pool.get(i) else { break };
                        out.push((i, rocketbox::cook_avatar(root, &manifest, a).with_context(|| a.id.clone())));
                    }
                    out
                })
            })
            .collect();
        let mut all: Vec<(usize, Result<rocketbox::RbAvatar>)> = hs.into_iter().flat_map(|h| h.join().expect("people cook thread")).collect();
        all.sort_by_key(|(i, _)| *i);
        all
    });
    let mut errors = vec![];
    avatars.retain(|(_, r)| match r {
        Ok(_) => true,
        Err(e) => {
            errors.push(format!("{e:#}"));
            false
        }
    });
    if avatars.is_empty() {
        bail!("no avatar cooked: {}", errors.join("; "));
    }
    for e in errors {
        b.lib.meta.missing.push(format!("people: {e}"));
    }
    // One bake per distinct Bip01 skeleton, all computed before anything is
    // appended. Skeletons equal within 1 mm and 0.1 degree share one (an
    // exact key would split the men's one skeleton three ways on a
    // quaternion's sign).
    let mut group_first: Vec<usize> = vec![];
    let mut grouped: Vec<(rocketbox::RbAvatar, usize)> = vec![];
    for (_, r) in avatars {
        let av = r.expect("kept");
        let g = match group_first.iter().position(|&first| rocketbox::same_bind(&grouped[first].0.skeleton, &av.skeleton)) {
            Some(g) => g,
            None => {
                group_first.push(grouped.len());
                group_first.len() - 1
            }
        };
        grouped.push((av, g));
    }
    let female: Vec<bool> = (0..group_first.len())
        .map(|g| {
            let ids: Vec<&str> = grouped.iter().filter(|(_, x)| *x == g).map(|(a, _)| a.id.as_str()).collect();
            2 * ids.iter().filter(|id| id.contains("female")).count() > ids.len()
        })
        .collect();
    let skels: Vec<&rocketbox::BipSkeleton> = group_first.iter().map(|&i| &grouped[i].0.skeleton).collect();
    let (segs, tables) = bake_groups(&skels, &female, &mo, &mut motion, &mut b.lib.meta.missing);
    let mut kept = vec![];
    for (av, g) in grouped {
        match &tables[g] {
            Some(_) => kept.push((av, g)),
            None => b.lib.meta.missing.push(format!("people: {}: its proportion group could not be baked", av.id)),
        }
    }
    if kept.is_empty() {
        bail!("no avatar could be baked");
    }
    let bones_before = b.lib.bones.len();
    let mut group_at = vec![0u32; tables.len()];
    for (g, t) in tables.iter().enumerate() {
        if let Some(rows) = t {
            group_at[g] = (b.lib.bones.len() / 3) as u32;
            b.lib.bones.extend_from_slice(rows);
        }
    }
    let mut groups_used: Vec<usize> = vec![];
    for (av, g) in kept {
        let at = group_at[g];
        b.lib.meta.people_notes.extend(av.notes.iter().cloned());
        let layer = b.lib.atlases.len() as i32;
        b.lib.atlases.push(av.atlas);
        let mat = b.material(&AgentMaterial { base: [1.0; 4], params: [1.0, 0.0, 1.0, 0.0], layer, flags: M_ATLAS, ..Default::default() });
        let mut meshes = [NO_MESH; LODS];
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (k, t) in av.tiers.iter().enumerate() {
            for a in 0..3 {
                mn[a] = mn[a].min(t.min[a]);
                mx[a] = mx[a].max(t.max[a]);
            }
            let ct = CookedTier { vertices: t.vertices.clone(), indices: t.indices.clone(), ..Default::default() };
            meshes[k] = b.tier(&ct, mat, at, rocketbox::JOINTS.len() as u32);
        }
        let c = [(mn[0] + mx[0]) / 2.0, (mn[1] + mx[1]) / 2.0, (mn[2] + mx[2]) / 2.0];
        let r = ((mx[0] - mn[0]).powi(2) + (mx[1] - mn[1]).powi(2) + (mx[2] - mn[2]).powi(2)).sqrt() / 2.0 + 0.3;
        let m = b.lib.models.len() as u32;
        b.lib.models.push(GpuModel { meshes, wheel: [[0.0; 4]; 4], center: c, radius: r, class: CLASS_PERSON, skeleton: at, joints: rocketbox::JOINTS.len() as u32, wheel_radius: 0.0, pad: [0; 4] });
        b.lib.meta.model_names.push(av.id.clone());
        b.lib.meta.people_models.push(m);
        b.lib.meta.family_heights.push(av.height);
        motion.person_group.push(g as u32);
        if !groups_used.contains(&g) {
            groups_used.push(g);
        }
    }
    // The player's body: the first pool avatar (its table is the same as the street's).
    b.lib.meta.player_model = b.lib.meta.people_models[0];
    b.lib.meta.people_groups = groups_used.len() as u32;
    b.lib.meta.skeletons = groups_used.len() as u32;
    b.lib.meta.people_bone_bytes = ((b.lib.bones.len() - bones_before) * 16) as u64;
    b.lib.meta.reserved = manifest.reserved();
    // The slots as laid out.
    let mut base = 0;
    for k in 0..CLIPS {
        let frames: u32 = segs.iter().filter(|s| s.slot == k).map(|s| s.frames).sum();
        b.lib.meta.clip_frames[k] = frames;
        b.lib.meta.clip_base[k] = base;
        base += frames;
        b.lib.meta.clip_seconds[k] = if motion.looped(k) { motion.loops[k].iter().map(|l| l.seconds).sum() } else { 1.0 };
        motion.player_seconds[k] = motion.loops[k].first().map(|l| l.seconds).unwrap_or(1.0);
    }
    motion.slot_frames = b.lib.meta.clip_frames;
    b.lib.meta.motion = motion;
    Ok(rocketbox::ATLAS_RES)
}

/// what fills one stretch of the tables.
#[derive(Clone, Copy, Debug)]
enum SegKind {
    /// A 100STYLE walk (index into the loaded cycles).
    Style(usize),
    /// A `motion::ROLES` loop.
    Role(usize),
}

#[derive(Clone, Copy, Debug)]
struct Seg {
    slot: usize,
    kind: SegKind,
    /// Frames stored.
    frames: u32,
}

/// the Rocketbox clips the roles name and the 100STYLE cycles, each file verified.
struct MotionSources {
    clips: HashMap<String, crate::motion::RbClip>,
    styles: Vec<crate::motion::StyleCycle>,
}

/// Both motion sources, or why the people cannot move.
fn motion_sources(roots: &MotionRoots, meta: &mut crate::motion::MotionMeta) -> Result<MotionSources> {
    use crate::motion::{Manifest, ROLES, STYLES, load_rb_clip};
    if !crate::motion::enabled() {
        bail!("the Rocketbox motions are switched off (--no-rocketbox-motions), and nothing else moves the people");
    }
    let clips_root = roots.clips.as_ref().ok_or_else(|| anyhow!("no Rocketbox motion clips: no copy at {}", eo_data::external::PEOPLE_MOTIONS.looked_in()))?;
    let styles_root = roots.styles.as_ref().ok_or_else(|| anyhow!("no 100STYLE walks: no copy at {}", eo_data::external::PEOPLE_STYLES.looked_in()))?;
    let clips = (|| -> Result<HashMap<String, crate::motion::RbClip>> {
        let m = Manifest::clips()?;
        m.read(clips_root, "LICENSES.md").context("the copy's LICENSES.md")?;
        let mut ids: Vec<&str> = ROLES.iter().flat_map(|r| [r.female, r.male]).collect();
        ids.sort_unstable();
        ids.dedup();
        let loaded: Vec<Result<crate::motion::RbClip>> = std::thread::scope(|s| ids.iter().map(|id| s.spawn(|| load_rb_clip(clips_root, &m, id))).collect::<Vec<_>>().into_iter().map(|h| h.join().expect("motion clip thread")).collect());
        let mut map = HashMap::new();
        for c in loaded {
            let c = c?;
            map.insert(c.id.clone(), c);
        }
        Ok(map)
    })()
    .context("the Rocketbox motion clips")?;
    let styles = (|| -> Result<Vec<crate::motion::StyleCycle>> {
        let m = Manifest::styles()?;
        m.read(styles_root, "LICENSES.md").context("the copy's LICENSES.md")?;
        // Every file read and verified first (a damaged copy is refused) ...
        let files: Vec<Vec<u8>> = STYLES.iter().map(|(name, _)| m.read(styles_root, &crate::motion::style_file(name))).collect::<Result<_>>()?;
        let loaded: Vec<Result<crate::motion::StyleCycle>> = std::thread::scope(|s| {
            STYLES.iter().zip(&files).map(|((name, phone), bytes)| s.spawn(move || crate::motion::style_from_bytes(bytes, name, *phone))).collect::<Vec<_>>().into_iter().map(|h| h.join().expect("style thread")).collect()
        });
        // ... then a take with no clean cycle is left out (the data decides it, not the copy).
        let mut v = vec![];
        for (i, c) in loaded.into_iter().enumerate() {
            match c {
                Ok(c) => v.push(c),
                Err(e) => meta.notes.push(format!("100STYLE {} left out: {e:#}", STYLES[i].0)),
            }
        }
        if v.is_empty() {
            bail!("no style gave a gait cycle");
        }
        if let Some(deg) = crate::motion::level_heads(&mut v) {
            meta.notes.push(format!("100STYLE: the Neutral take's mean neck and head rotations ({deg:.1} degrees together) taken out of every style"));
        }
        Ok(v)
    })()
    .context("the 100STYLE walks")?;
    Ok(MotionSources { clips, styles })
}

/// the stretches of every table, in order: one walk loop a style in slot 0,
/// the `ROLES` loops of each standing slot after it.
fn layout(mo: &MotionSources) -> Vec<Seg> {
    let mut segs: Vec<Seg> = (0..mo.styles.len()).map(|i| Seg { slot: 0, kind: SegKind::Style(i), frames: crate::motion::WALK_FRAMES + 1 }).collect();
    for slot in 1..CLIPS {
        for (i, r) in crate::motion::ROLES.iter().enumerate().filter(|(_, r)| r.slot == slot) {
            segs.push(Seg { slot, kind: SegKind::Role(i), frames: (r.seconds * r.fps).round() as u32 + 1 });
        }
    }
    segs
}

/// every group's table over the layout, the motion meta filled for it. None
/// for a group that cannot be baked (its people are left out).
fn bake_groups(skels: &[&rocketbox::BipSkeleton], female: &[bool], mo: &MotionSources, meta: &mut crate::motion::MotionMeta, missing: &mut Vec<String>) -> (Vec<Seg>, Vec<Option<Vec<[f32; 4]>>>) {
    use crate::motion::{LoopMeta, ROLES, StyleMeta};
    use rocketbox::BakeSrc;
    let segs = layout(mo);
    let mut tables = vec![];
    let mut strides: Vec<Vec<Option<(f32, f32)>>> = vec![];
    for (g, sk) in skels.iter().enumerate() {
        let clip_of = |r: usize| mo.clips.get(if female[g] { ROLES[r].female } else { ROLES[r].male });
        let relaxed = clip_of(0).and_then(|c| rocketbox::relaxed_hands(sk, c, &ROLES[0]).ok());
        let srcs: Option<Vec<BakeSrc>> = segs
            .iter()
            .map(|s| match s.kind {
                SegKind::Style(i) => Some(BakeSrc::Style { cycle: &mo.styles[i] }),
                SegKind::Role(r) => clip_of(r).map(|clip| BakeSrc::Clip { clip, role: &ROLES[r], frames: s.frames }),
            })
            .collect();
        match srcs.ok_or_else(|| anyhow!("a role's clip is missing")).and_then(|srcs| rocketbox::bake_list(sk, &srcs, relaxed.as_deref())) {
            Ok(bk) => {
                tables.push(Some(bk.rows));
                strides.push(bk.strides);
            }
            Err(e) => {
                missing.push(format!("people: proportion group {g}: bake: {e:#}"));
                tables.push(None);
                strides.push(vec![]);
            }
        }
    }
    meta.walk = "100style".into();
    meta.standing = "rocketbox".into();
    meta.group_female = female.to_vec();
    meta.walk_frames = crate::motion::WALK_FRAMES;
    meta.styles = mo.styles.iter().map(|c| StyleMeta { name: c.name.clone(), phone: c.phone, seconds: c.seconds, stride: c.stride, speed: c.speed, from_s: c.from_s, stride_scale: c.stride_scale }).collect();
    for (g, sk) in skels.iter().enumerate() {
        let mut row = vec![];
        for (i, c) in mo.styles.iter().enumerate() {
            let seg = segs.iter().position(|s| matches!(s.kind, SegKind::Style(x) if x == i)).unwrap();
            // Scaled by the leg length when the planted foot could not be measured.
            let scaled = c.stride * c.stride_scale * sk.pos[0].y / c.nodes.iter().find(|n| n.name == "Hips").map(|n| n.t.y).unwrap_or(1.0).max(0.1);
            let stride = match strides.get(g).and_then(|v| v.get(seg)).copied().flatten() {
                Some((s, slide)) if (s / scaled - 1.0).abs() < 0.3 => {
                    if g == 0 || slide > 0.15 {
                        meta.notes.push(format!("group {g} {}: stride {s:.3} m measured ({scaled:.3} m by leg length), median planted-foot slide {:.0} %", c.name, slide * 100.0));
                    }
                    s
                }
                other => {
                    meta.notes.push(format!("group {g} {}: stride by leg length {scaled:.3} m (measured {other:?})", c.name));
                    scaled
                }
            };
            row.push([stride, stride / c.seconds.max(0.3)]);
        }
        meta.group_walk.push(row);
    }
    let mut first = [0u32; CLIPS];
    for s in &segs {
        if let SegKind::Role(r) = s.kind {
            let role = &ROLES[r];
            meta.loops[s.slot].push(LoopMeta { role: role.name.into(), family: role.family, first: first[s.slot], frames: s.frames - 1, seconds: role.seconds, female: role.female.into(), male: role.male.into() });
        }
        first[s.slot] += s.frames;
    }
    for c in mo.styles.iter() {
        meta.notes.push(format!("100STYLE {}: a {:.2} s cycle of {:.2} m ({:.2} m/s) from {:.1} s of the take ({} cycles in it, {:.1} degrees of turn over it and the one before), stride x {:.2}", c.name, c.seconds, c.stride, c.speed, c.from_s, c.cycles, c.turn_deg, c.stride_scale));
    }
    (segs, tables)
}

// ---------------------------------------------------------------- cache

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn take<'a>(b: &'a [u8], at: &mut usize) -> Result<&'a [u8]> {
    if *at + 8 > b.len() {
        bail!("cache truncated");
    }
    let n = u64::from_le_bytes(b[*at..*at + 8].try_into().unwrap()) as usize;
    *at += 8;
    if *at + n > b.len() {
        bail!("cache truncated");
    }
    let s = &b[*at..*at + n];
    *at += n;
    Ok(s)
}

pub fn write_cache(path: &Path, lib: &Library) -> Result<()> {
    let mut payload = vec![];
    put(&mut payload, &serde_json::to_vec(&lib.meta)?);
    put(&mut payload, bytemuck::cast_slice(&lib.vertices));
    put(&mut payload, bytemuck::cast_slice(&lib.indices));
    put(&mut payload, bytemuck::cast_slice(&lib.meshes));
    put(&mut payload, bytemuck::cast_slice(&lib.materials));
    put(&mut payload, bytemuck::cast_slice(&lib.models));
    put(&mut payload, bytemuck::cast_slice(&lib.bones));
    put(&mut payload, &(lib.atlases.len() as u32).to_le_bytes());
    for a in &lib.atlases {
        put(&mut payload, &a.res.to_le_bytes());
        for m in &a.maps {
            put(&mut payload, m);
        }
        // the BC chains.
        put(&mut payload, &a.bc_levels.to_le_bytes());
        for m in &a.bc {
            put(&mut payload, m);
        }
    }
    let digest = Sha256::digest(&payload);
    let mut out = Vec::with_capacity(payload.len() + 40);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&COOK_VERSION.to_le_bytes());
    out.extend_from_slice(&digest);
    out.extend_from_slice(&payload);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &out)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read_cache(path: &Path) -> Result<Library> {
    let b = std::fs::read(path)?;
    if b.len() < 40 || &b[..4] != MAGIC {
        bail!("not an agent library cache");
    }
    if u32::from_le_bytes(b[4..8].try_into().unwrap()) != COOK_VERSION {
        bail!("cache version differs");
    }
    let payload = &b[40..];
    if Sha256::digest(payload).as_slice() != &b[8..40] {
        bail!("cache payload digest mismatch");
    }
    let mut at = 0;
    let meta: Meta = serde_json::from_slice(take(payload, &mut at)?)?;
    let vertices = bytemuck::pod_collect_to_vec(take(payload, &mut at)?);
    let indices = bytemuck::pod_collect_to_vec(take(payload, &mut at)?);
    let meshes = bytemuck::pod_collect_to_vec(take(payload, &mut at)?);
    let materials = bytemuck::pod_collect_to_vec(take(payload, &mut at)?);
    let models = bytemuck::pod_collect_to_vec(take(payload, &mut at)?);
    let bones = bytemuck::pod_collect_to_vec(take(payload, &mut at)?);
    let n = u32::from_le_bytes(take(payload, &mut at)?.try_into().map_err(|_| anyhow!("atlas count"))?);
    let mut atlases = vec![];
    for _ in 0..n {
        let res = u32::from_le_bytes(take(payload, &mut at)?.try_into().map_err(|_| anyhow!("atlas res"))?);
        let maps = [take(payload, &mut at)?.to_vec(), take(payload, &mut at)?.to_vec(), take(payload, &mut at)?.to_vec()];
        let bc_levels = u32::from_le_bytes(take(payload, &mut at)?.try_into().map_err(|_| anyhow!("atlas levels"))?);
        let bc = [take(payload, &mut at)?.to_vec(), take(payload, &mut at)?.to_vec(), take(payload, &mut at)?.to_vec()];
        atlases.push(Atlas { res, maps, bc, bc_levels });
    }
    Ok(Library { vertices, indices, meshes, materials, models, bones, atlases, meta, from_cache: true, load_ms: 0.0, cache_path: None })
}
