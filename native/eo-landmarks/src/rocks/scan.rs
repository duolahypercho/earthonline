//! The scanned rock library: the seven Poly Haven rock scans of the survey
//! set (group S1, CC0 1.0; provenance and digests in
//! native/assets/survey-manifest.json, read where `EO_SURVEY_ROOT` names),
//! each cut to four levels of detail with meshopt (about 20 000, 5 000,
//! 1 200 and 300 triangles, UVs kept; eo_farshore::survey::scan
//! `lod_chain_cached`, cooked under the native cache by the scans' digests)
//! and kept over the vertices those levels use, with its 2k diffuse,
//! OpenGL normal and ARM (occlusion, roughness, metalness) maps decoded.
//!
//! Nothing here touches the GPU: eo-render's ground cover (`groundcover.rs`)
//! uploads the library once it is loaded and draws the placed pieces
//! (`placed`), instanced. The load runs on its own thread from the start of
//! the landmark build (`start`); `library` waits for it. Without the survey
//! root, with a scan refused by the manifest, or with `--no-scan-rocks` (or
//! the roll-ups), there is no library and every rock draws as round 2 did.
//!
//! The scans are drawn at their scanned size times the ground plan (1.25):
//! they are the ground's rock, drawn with the ground (as rocks.rs scales its
//! stones), and each placement scales them further within its stated law.

use eo_farshore::survey::scan::{Compact, RimPlane, Scan};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

/// The S1 scans, in the order the renderer's texture array holds them.
pub const KINDS: [&str; 7] = ["coast_rocks_05", "sand_rocks_small_01", "coast_land_rocks_02", "rock_face_01", "rock_face_02", "boulder_01", "coastal_cliff_02"];
pub const COAST_ROCKS: u8 = 0;
pub const SAND_ROCKS: u8 = 1;
pub const LAND_ROCKS: u8 = 2;
pub const FACE_1: u8 = 3;
pub const FACE_2: u8 = 4;
pub const BOULDER: u8 = 5;
pub const CLIFF: u8 = 6;

/// The levels of detail, triangles per piece (the plan's 20k, 5k, 1.2k, 300).
pub const LOD_TRIANGLES: [usize; 4] = [20_000, 5_000, 1_200, 300];

/// How a scan stands on the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A closed stone (boulder_01): any turn; bedded a share of its height.
    Boulder,
    /// A scanned patch of ground with its rocks (an open shell over its
    /// footprint): laid on the ground along its rim plane; its own ground,
    /// under `clip_m` over that plane, is cut away where it is drawn so only
    /// the rocks stand out of ours.
    Patch,
    /// A rock face (an open shell scanned from its front): laid with its rim
    /// plane on the slope and its face (+z of the scan) turned downhill.
    Face,
}

pub fn role(kind: u8) -> Role {
    match kind {
        BOULDER => Role::Boulder,
        COAST_ROCKS | SAND_ROCKS | LAND_ROCKS => Role::Patch,
        _ => Role::Face,
    }
}

/// A boulder is bedded this share of its height (INFERRED: a stone resting
/// on sand or soil sinks a little; rocks.rs sinks its boulders a third).
pub const BOULDER_BED_SHARE: f64 = 0.15;
/// A patch's own ground: the scanned surface less than this over the rim
/// plane is cut away when drawn (metres in the scan; measured on the
/// scans: the rim's 95th-percentile residual over its plane is 0.04 m for
/// sand_rocks_small_01, 0.07 m for coast_rocks_05, 0.16 m for
/// coast_land_rocks_02).
pub fn clip_m(kind: u8) -> f32 {
    match kind {
        SAND_ROCKS => 0.04,
        COAST_ROCKS => 0.07,
        LAND_ROCKS => 0.16,
        _ => -1.0e3,
    }
}

/// A face's bed is lifted this share of its rim's upper residual (its
/// uneven edge goes under the slope, its relief stays over it; INFERRED).
pub const FACE_RIM_LIFT: f64 = 0.25;

/// One loaded scan.
pub struct Kind {
    pub name: &'static str,
    pub role: Role,
    /// The four levels over the vertices they use.
    pub mesh: Compact,
    /// meshopt's error per level (relative to the scan's extent).
    pub lod_errors: Vec<f32>,
    pub source_triangles: usize,
    /// The levels' bounds, scan metres (y up).
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// The open rim's plane (None for the closed boulder).
    pub rim: Option<RimPlane>,
    /// The bed the scan stands on, in its own frame: unit normal (up) and a point.
    pub bed_normal: [f64; 3],
    pub bed_point: [f64; 3],
    /// The direction in the scan's frame turned to the world's: a face's
    /// uphill along its bed (seated uphill on the slope, so it stands as it
    /// was scanned); +z for a stone or a patch (turned by a yaw).
    pub face: [f64; 3],
    /// The diffuse map's mean, linear RGB.
    pub mean_albedo: [f32; 3],
    /// RGBA8 maps, `tex_res` square: diffuse (sRGB bytes), normal, ARM.
    pub textures: [Vec<u8>; 3],
    pub tex_res: u32,
}

impl Kind {
    /// The bounding sphere (centre, radius) in the scan's frame.
    pub fn sphere(&self) -> ([f32; 3], f32) {
        let c = [0, 1, 2].map(|k| 0.5 * (self.min[k] + self.max[k]));
        let r = self.mesh.positions.iter().map(|p| ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt()).fold(0.0f32, f32::max);
        (c, r)
    }
}

pub struct Library {
    pub kinds: Vec<Kind>,
    pub load_ms: f64,
}

impl Library {
    pub fn triangles(&self, level: usize) -> usize {
        self.kinds.iter().map(|k| k.mesh.lods[level].len() / 3).sum()
    }
}

/// The switches the scanned rock reads (eo-app accepts them:
/// `hill_rock::SWITCHES`), from the command line once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switches {
    /// The scans at all (`--no-scan-rocks`; the roll-ups `--no-gg3-rock`, `--no-gg3`).
    pub scan: bool,
    /// The cliff band's rock faces (`--no-cliff-dressing`).
    pub cliff: bool,
    /// The intertidal rock field from the imagery (`--no-surf-rocks`).
    pub surf: bool,
    /// The revetment's armour as scanned stone (`--no-scan-riprap`).
    pub riprap: bool,
    /// Tints from the mapped geologic units (`--no-rock-geology`: round 2's colours).
    pub geology: bool,
}

impl Switches {
    pub const OFF: Switches = Switches { scan: false, cliff: false, surf: false, riprap: false, geology: false };

    pub fn from_args(args: &[String]) -> Switches {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-gg3-rock") || has("--no-gg3") || has("--no-scan-rocks") {
            return Switches::OFF;
        }
        Switches { scan: true, cliff: !has("--no-cliff-dressing"), surf: !has("--no-surf-rocks"), riprap: !has("--no-scan-riprap"), geology: !has("--no-rock-geology") }
    }
}

pub fn switches() -> Switches {
    static S: OnceLock<Switches> = OnceLock::new();
    *S.get_or_init(|| Switches::from_args(&std::env::args().collect::<Vec<_>>()))
}

static JOB: Mutex<Option<std::thread::JoinHandle<Option<Arc<Library>>>>> = Mutex::new(None);
static LIB: OnceLock<Option<Arc<Library>>> = OnceLock::new();

/// Start loading the library on its own thread (once; nothing when the
/// switches or the survey root say no).
pub fn start() {
    if LIB.get().is_some() || !switches().scan {
        return;
    }
    let mut job = JOB.lock().unwrap();
    if job.is_none() {
        *job = Some(std::thread::spawn(load_now));
    }
}

/// The library, waiting for its load (started here if it was not); None
/// when there is none (no survey root, a scan refused, switched off).
pub fn library() -> Option<Arc<Library>> {
    LIB.get_or_init(|| {
        if !switches().scan {
            return None;
        }
        start();
        let h = JOB.lock().unwrap().take();
        h.and_then(|h| h.join().ok().flatten())
    })
    .clone()
}

fn load_now() -> Option<Arc<Library>> {
    let t0 = Instant::now();
    let root = eo_farshore::survey::Root::discover()?;
    let cache = crate::features::cache_dir();
    let kinds: Vec<Option<Kind>> = std::thread::scope(|s| {
        let hs: Vec<_> = KINDS
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let (root, cache) = (&root, cache.as_deref());
                s.spawn(move || load_kind(root, i as u8, name, cache).map_err(|e| eprintln!("scanned rock: {name} not loaded ({e:#})")).ok())
            })
            .collect();
        hs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    if kinds.iter().any(|k| k.is_none()) {
        eprintln!("scanned rock: a scan is missing; the rock draws as round 2");
        return None;
    }
    let lib = Library { kinds: kinds.into_iter().flatten().collect(), load_ms: t0.elapsed().as_secs_f64() * 1e3 };
    eprintln!(
        "scanned rock: {} scans ({} source triangles), levels {:?} triangles over all, {:.0} ms",
        lib.kinds.len(),
        lib.kinds.iter().map(|k| k.source_triangles).sum::<usize>(),
        (0..4).map(|l| lib.triangles(l)).collect::<Vec<_>>(),
        lib.load_ms
    );
    Some(Arc::new(lib))
}

fn load_kind(root: &eo_farshore::survey::Root, kind: u8, name: &'static str, cache: Option<&std::path::Path>) -> anyhow::Result<Kind> {
    use eo_farshore::survey::scan;
    let s = Scan::load(root, "S1", name)?;
    anyhow::ensure!(s.pieces.len() == 1, "{name}: {} pieces (one expected)", s.pieces.len());
    let chains = scan::lod_chain_cached(&s, &LOD_TRIANGLES, cache)?;
    let p = &s.pieces[0];
    let mesh = p.compact(&chains[0]);
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for q in &mesh.positions {
        for k in 0..3 {
            min[k] = min[k].min(q[k]);
            max[k] = max[k].max(q[k]);
        }
    }
    let role = role(kind);
    let rim = if role == Role::Boulder { None } else { p.rim_plane() };
    let (bed_normal, bed_point) = match (role, rim) {
        (Role::Boulder, _) | (_, None) => ([0.0, 1.0, 0.0], [0.5 * (min[0] + max[0]) as f64, min[1] as f64 + BOULDER_BED_SHARE * (max[1] - min[1]) as f64, 0.5 * (min[2] + max[2]) as f64]),
        (_, Some(r)) => {
            // the plane under the rim's middle; a face's bed lifted by part of
            // the rim's upper residual, so its uneven edge goes under the slope.
            let (cx, cz) = (0.5 * (min[0] + max[0]) as f64, 0.5 * (min[2] + max[2]) as f64);
            let lift = if role == Role::Face { FACE_RIM_LIFT * r.high } else { 0.0 };
            (r.normal(), [cx, r.a * cx + r.b * cz + r.c + lift, cz])
        }
    };
    // the scan's uphill along its bed: the vertical's part across the bed's
    // normal (a face's rim leans back from its face, +z, so uphill is -z);
    // a stone or a patch, whose bed is level, keeps its +z.
    let (b, up) = (bed_normal, bed_normal[1]);
    let mut face = [-up * b[0], 1.0 - up * b[1], -up * b[2]];
    let fl = (face[0] * face[0] + face[1] * face[1] + face[2] * face[2]).sqrt();
    face = if role == Role::Face && fl > 0.2 { face.map(|v| v / fl) } else { [0.0, 0.0, 1.0] };
    let decode = |path: &std::path::Path| -> anyhow::Result<(u32, Vec<u8>)> {
        let (w, h, rgb) = scan::decode_jpeg(path)?;
        anyhow::ensure!(w == h && w.is_power_of_two(), "{}: {w} x {h}", path.display());
        let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
        for px in rgb.chunks_exact(3) {
            out.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        Ok((w, out))
    };
    let (res, diffuse) = decode(&s.textures.diffuse)?;
    let (r1, normal) = decode(&s.textures.normal)?;
    let (r2, arm) = decode(&s.textures.arm)?;
    anyhow::ensure!(res == r1 && res == r2, "{name}: maps of different sizes");
    let lin = |c: u8| {
        let v = c as f64 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let mut mean = [0.0f64; 3];
    let n = (diffuse.len() / 4) as f64;
    for px in diffuse.chunks_exact(4) {
        for k in 0..3 {
            mean[k] += lin(px[k]);
        }
    }
    Ok(Kind {
        name,
        role,
        lod_errors: chains[0].iter().map(|l| l.error).collect(),
        source_triangles: p.indices.len() / 3,
        mesh,
        min,
        max,
        rim,
        bed_normal,
        bed_point,
        face,
        mean_albedo: mean.map(|v| (v / n) as f32),
        textures: [diffuse, normal, arm],
        tex_res: res,
    })
}

/// One placed piece: which scan, where, how turned and scaled, its tint
/// (a factor on the scan's albedo, linear) and how wet it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub kind: u8,
    /// Where the scan's origin lands (game metres).
    pub pos: [f64; 3],
    /// The turn, a unit quaternion (x, y, z, w), scan frame to world.
    pub rot: [f64; 4],
    /// Scale on the scan's axes.
    pub scale: [f64; 3],
    pub tint: [f32; 3],
    /// 0 dry, 1 wet through (the intertidal: darker, glossier).
    pub wet: f32,
    /// What placed it (for the counts).
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A round-2 boulder (rocks.rs `boulder`) as a scan.
    Boulder,
    /// The revetment's armour (rocks.rs `revetment`) as a scan.
    Armour,
    /// The intertidal field from the imagery.
    Surf,
    /// The cliff band's faces.
    Cliff,
    /// An outcrop on a steep or bare slope.
    Outcrop,
}

/// What the landmark build placed, and where the scans stand in for the
/// props' hill rock.
pub struct Placed {
    pub pieces: Vec<Piece>,
    /// Boxes (x0, z0, x1, z1, game metres) where the scans replace the hill rock.
    pub claims: Vec<[f64; 4]>,
}

static PLACED: RwLock<Option<Arc<Placed>>> = RwLock::new(None);
static REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Publish the placement (the renderer picks it up by its revision).
pub fn publish(p: Placed) {
    *PLACED.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(p));
    REVISION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

pub fn placed() -> Option<Arc<Placed>> {
    PLACED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn revision() -> u64 {
    REVISION.load(std::sync::atomic::Ordering::Acquire)
}

/// Whether the scans stand in for the hill rock at (x, z).
pub fn claims(x: f64, z: f64) -> bool {
    placed().is_some_and(|p| p.claims.iter().any(|b| x >= b[0] && x < b[2] && z >= b[1] && z < b[3]))
}

/// Quaternion helpers (x, y, z, w).
pub fn quat_mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

pub fn quat_axis(axis: [f64; 3], angle: f64) -> [f64; 4] {
    let l = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt().max(1e-12);
    let (s, c) = (0.5 * angle).sin_cos();
    [axis[0] / l * s, axis[1] / l * s, axis[2] / l * s, c]
}

pub fn quat_rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let u = [q[0], q[1], q[2]];
    let s = q[3];
    let cr = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let d = u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let uu = u[0] * u[0] + u[1] * u[1] + u[2] * u[2];
    let c = cr(u, v);
    [0, 1, 2].map(|k| 2.0 * d * u[k] + (s * s - uu) * v[k] + 2.0 * s * c[k])
}

/// The turn taking the orthonormal frame (f, b) of the scan (its face and
/// bed normal) onto (F, B) in the world.
pub fn quat_frames(f: [f64; 3], b: [f64; 3], ff: [f64; 3], bb: [f64; 3]) -> [f64; 4] {
    let cr = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let s = cr(f, b);
    let ss = cr(ff, bb);
    // R = W M^T, M = [f b s] columns, W = [F B S] columns
    let m = [f, b, s];
    let w = [ff, bb, ss];
    let mut r = [[0.0f64; 3]; 3]; // r[row][col]
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| w[k][i] * m[k][j]).sum();
        }
    }
    // rotation matrix to quaternion
    let tr = r[0][0] + r[1][1] + r[2][2];
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [(r[2][1] - r[1][2]) / s, (r[0][2] - r[2][0]) / s, (r[1][0] - r[0][1]) / s, 0.25 * s]
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        [0.25 * s, (r[0][1] + r[1][0]) / s, (r[0][2] + r[2][0]) / s, (r[2][1] - r[1][2]) / s]
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        [(r[0][1] + r[1][0]) / s, 0.25 * s, (r[1][2] + r[2][1]) / s, (r[0][2] - r[2][0]) / s]
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        [(r[0][2] + r[2][0]) / s, (r[1][2] + r[2][1]) / s, 0.25 * s, (r[1][0] - r[0][1]) / s]
    };
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    q.map(|v| v / l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_roll_up() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(Switches::from_args(&a(&["eo-app"])), Switches { scan: true, cliff: true, surf: true, riprap: true, geology: true });
        assert_eq!(Switches::from_args(&a(&["eo-app", "--no-gg3-rock"])), Switches::OFF);
        assert_eq!(Switches::from_args(&a(&["eo-app", "--no-gg3"])), Switches::OFF);
        assert_eq!(Switches::from_args(&a(&["eo-app", "--no-scan-rocks"])), Switches::OFF);
        assert!(!Switches::from_args(&a(&["x", "--no-surf-rocks"])).surf);
        assert!(!Switches::from_args(&a(&["x", "--no-cliff-dressing"])).cliff);
        assert!(!Switches::from_args(&a(&["x", "--no-scan-riprap"])).riprap);
        assert!(!Switches::from_args(&a(&["x", "--no-rock-geology"])).geology);
    }

    #[test]
    fn frames_turn_onto_frames() {
        let n = |v: [f64; 3]| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            v.map(|c| c / l)
        };
        let f = n([0.0, 0.3, 1.0]);
        let b0 = n([0.0, 1.0, -0.3]);
        let ff = n([1.0, -0.5, 0.2]);
        let mut bb = n([0.4, 1.0, 0.1]);
        let d = ff[0] * bb[0] + ff[1] * bb[1] + ff[2] * bb[2];
        bb = n([bb[0] - d * ff[0], bb[1] - d * ff[1], bb[2] - d * ff[2]]);
        let q = quat_frames(f, b0, ff, bb);
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9);
        assert!(close(quat_rotate(q, f), ff), "{:?} {:?}", quat_rotate(q, f), ff);
        assert!(close(quat_rotate(q, b0), bb));
        let qz = quat_axis([0.0, 1.0, 0.0], std::f64::consts::FRAC_PI_2);
        assert!(close(quat_rotate(qz, [0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]));
        assert!(close(quat_rotate(quat_mul(qz, qz), [0.0, 0.0, 1.0]), [0.0, 0.0, -1.0]));
    }

    /// With the survey root: the seven scans load, their levels fall in
    /// triangle count toward the plan's targets, the faces turn +z and the
    /// patches lie flat.
    #[test]
    fn the_seven_scans_load_with_their_levels() {
        if eo_farshore::survey::Root::discover().is_none() || !switches().scan {
            eprintln!("scanned rock library: skipped, no survey root");
            return;
        }
        let lib = library().expect("the library loads");
        assert_eq!(lib.kinds.len(), 7);
        for k in &lib.kinds {
            let tris: Vec<usize> = k.mesh.lods.iter().map(|l| l.len() / 3).collect();
            println!("{}: {} source, levels {tris:?}, errors {:?}, bed {:?} face {:?} mean {:?} rim {:?}", k.name, k.source_triangles, k.lod_errors, k.bed_normal, k.face, k.mean_albedo, k.rim);
            assert!(tris.windows(2).all(|w| w[0] >= w[1]));
            assert!(tris[0] <= LOD_TRIANGLES[0] + 2000 && tris[3] <= 2 * LOD_TRIANGLES[3] + 200, "{tris:?}");
            assert_eq!(k.tex_res, 2048);
            match k.role {
                Role::Face => assert!(k.face[2] < -0.5 && k.face[1] > 0.4, "{} uphill {:?}", k.name, k.face),
                Role::Patch => assert!(k.bed_normal[1] > 0.99, "{} bed {:?}", k.name, k.bed_normal),
                Role::Boulder => assert!(k.rim.is_none()),
            }
        }
    }
}
