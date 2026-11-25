//! The probe grid (`LightingOptions::probe_grid`, `--probe-grid 1|2`):
//! reflection probes anchored on the streets round the camera in place of
//! the single cube at the viewer (probe.rs), every change of cube faded.
//!
//! * Anchors (`Anchors::from_streets`): each junction's centre and every drawn
//!   street run's centreline every `ANCHOR_SPACING_M`, `PROBE_HEIGHT_M` over
//!   the street's datum (the terrain plus the road lift), none inside a
//!   building footprint. The set is the `Level::probes` anchors nearest a
//!   point `AHEAD_M` ahead of the eye on the ground, a member kept unless
//!   another is `KEEP_MARGIN_M` nearer.
//! * Captures: one face a frame in all (the single probe captures nothing
//!   while the grid is on), drawn by the single probe's face pass
//!   (`ProbeGpu`) at the grid's face size, the sixth face finished (sky fill,
//!   box levels, irradiance SH) into the probe's slot of one 2D array (6
//!   layers a slot, every level) at group 1 binding 15. A probe is captured
//!   once when it enters the set, kept while it stays, and captured again only
//!   when the clock's 0.25 h bucket or the sky's irradiance (10 %) moves, the
//!   single probe's rules; the single probe's deferral rules apply. A camera
//!   jump (`CUT_M`) starts a capture under way again, so a cube's six faces
//!   are lit under one camera's shadow fit (a cube whose faces straddled a
//!   cut drew a straight seam where they meet).
//! * Fades (`FADE_S`): a new probe's presence rises from 0 to 1 once
//!   captured, a dropped one's falls to 0 before its slot is reused, a
//!   recapture cross-fades from the old cube to the new one. Shading never
//!   changes cube within a frame.
//! * Shading (probe_grid.wgsl, the table at binding 16): at each point the two
//!   probes of highest presence / (distance^2 + `WEIGHT_SOFT_M`^2), each less
//!   the third's weight so the pair can change with no step, sampled along
//!   the reflection; mixed with the analytic sky by the presence and the reach
//!   of the probes (`REACH_M`). Glass keeps the canyon lift from the blended
//!   probes' height. `selection` is the CPU mirror of the shader's weights.

use crate::camera::Camera;
use crate::lighting::{self, Ts};
use glam::Vec3;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The lookups (look_light.wgsl and water.wgsl call them).
pub const WGSL: &str = include_str!("probe_grid.wgsl");

/// The cross-fade of a recaptured probe and the fade of a probe entering or
/// leaving the set, seconds (frame time from the sky's clock).
pub const FADE_S: f64 = 0.6;
/// Frame time when the clock gives none (a pinned world clock): fades still finish.
pub const FALLBACK_DT_S: f64 = 1.0 / 60.0;
/// Candidate anchors along a street run's centreline at most this far apart
/// (plus every junction's centre). A shop window on a 20 m street stands at
/// most about 20 m from one.
pub const ANCHOR_SPACING_M: f64 = 32.0;
/// A probe's height over the street's datum: a standing eye (INFERRED: the JS
/// probe is baked "at standing height where the viewer is",
/// environment-ibl.js `bakeSceneCube`).
pub const PROBE_HEIGHT_M: f64 = 1.7;
/// The set is chosen round a point this far ahead of the eye on the ground
/// (what the camera looks at gets probes first).
pub const AHEAD_M: f32 = 24.0;
/// A probe in the set stays unless another anchor is this much nearer (no
/// back-and-forth at the boundary between two sets).
pub const KEEP_MARGIN_M: f32 = 12.0;
/// A camera jump between two updates of more than this (the renderer's own
/// cut, `COUNT_CUT_M`: a teleport, never a moving camera, so a fast flight
/// still finishes its captures) restarts a capture under way.
pub const CUT_M: f32 = crate::COUNT_CUT_M;
/// Softening of the shading's distance weight, metres.
pub const WEIGHT_SOFT_M: f32 = 4.0;
/// The grid's reach: full within the first distance of a probe, none past the
/// second (a surface farther from every probe than that reflects the
/// analytic sky, not the streets of another place).
pub const REACH_M: [f32; 2] = [250.0, 600.0];
/// Slots beyond the set: one for a capture under way, one for a cross-fade's old cube.
pub const SPARE_SLOTS: usize = 2;
/// Frames a caller waiting for the grid to settle at one camera renders at
/// most (bench cards, the occlusion check): nine probes of six faces, every
/// other frame moved off by the cascades' refresh, and the fades.
pub const SETTLE_FRAMES_MAX: usize = 400;
/// The anchor index's cell, metres.
const CELL_M: f64 = 64.0;
/// Rings of cells searched at most (4 km round the point).
const MAX_RINGS: i32 = 64;

/// Table layout (vec4 rows of `lx_probe_table`, probe_grid.wgsl): a header of
/// two rows, two rows an entry, the slots' irradiance SH from `SH_BASE` bytes,
/// `SH_STRIDE` bytes a slot (the probe fill's 40 floats; storage offsets align to 256).
pub const ENTRY_BASE: usize = 2;
pub const MAX_SLOTS: usize = 9 + SPARE_SLOTS;
pub const SH_BASE: u64 = ((16 * (ENTRY_BASE + 2 * MAX_SLOTS)) as u64).next_multiple_of(256);
pub const SH_STRIDE: u64 = 256;

/// What a level of `--probe-grid` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    /// Probes in the set.
    pub probes: usize,
    /// Cubes in the array (the set and `SPARE_SLOTS`).
    pub slots: usize,
    /// Face size, pixels, and its levels (down to 4 px).
    pub face: u32,
    pub mips: u32,
}

/// Level 1 (high): 4 probes of 128 px (the single probe's size); level 2
/// (hyper): 9 of 256 px. None for 0.
pub fn level(l: u8) -> Option<Level> {
    match l {
        0 => None,
        1 => Some(Level { probes: 4, slots: 4 + SPARE_SLOTS, face: 128, mips: 6 }),
        _ => Some(Level { probes: 9, slots: 9 + SPARE_SLOTS, face: 256, mips: 7 }),
    }
}

impl Level {
    /// One cube with its levels (Rgba16Float, 6 faces).
    pub fn cube_bytes(&self) -> u64 {
        (0..self.mips).map(|m| ((self.face >> m).max(1) as u64).pow(2)).sum::<u64>() * 6 * 8
    }

    /// The array, the face target and its depth, the table.
    pub fn bytes(&self) -> u64 {
        let f2 = self.face as u64 * self.face as u64;
        self.slots as u64 * self.cube_bytes() + f2 * 6 * 8 + f2 * 4 + table_bytes(self.slots)
    }
}

/// Bytes of the table for `slots` slots.
pub fn table_bytes(slots: usize) -> u64 {
    SH_BASE + SH_STRIDE * slots as u64
}

fn smooth(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn smooth_between(e0: f32, e1: f32, x: f32) -> f32 {
    smooth((x - e0) / (e1 - e0))
}

/// Candidate probe points, indexed by cell.
#[derive(Clone, Debug, Default)]
pub struct Anchors {
    points: Vec<[f32; 3]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

fn cell_of(x: f64, z: f64) -> (i32, i32) {
    ((x / CELL_M).floor() as i32, (z / CELL_M).floor() as i32)
}

impl Anchors {
    pub fn new(points: Vec<[f32; 3]>) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in points.iter().enumerate() {
            cells.entry(cell_of(p[0] as f64, p[2] as f64)).or_default().push(i as u32);
        }
        Self { points, cells }
    }

    /// The drawn streets' anchors: every junction's centre and each run's
    /// centreline at most `ANCHOR_SPACING_M` apart (between its trimmed ends),
    /// `PROBE_HEIGHT_M` over the street datum (terrain + road lift, the rule the
    /// paving stands on); none inside a building footprint, none on a
    /// grade-separated road's leg (not drawn; its deck is the structure's).
    pub fn from_streets(w: &eo_street::StreetWorld) -> Self {
        let lift = w.o.road_lift;
        let mut points = Vec::new();
        let mut push = |x: f64, z: f64| {
            if x.is_finite() && z.is_finite() && !w.buildings.contains(x, z) {
                points.push([x as f32, (w.ground(x, z) + lift + PROBE_HEIGHT_M) as f32, z as f32]);
            }
        };
        for n in &w.plan.nodes {
            push(n.position[0], n.position[1]);
        }
        for e in &w.plan.entries {
            if e.structure_leg.is_some() {
                continue;
            }
            let (s0, s1) = (e.s0(), e.s1());
            if s1 - s0 < 1.0 {
                continue;
            }
            // interior points evenly spread (the junctions hold the ends).
            let k = ((s1 - s0) / ANCHOR_SPACING_M).floor() as usize;
            for i in 1..=k {
                let f = e.frame(s0 + (s1 - s0) * i as f64 / (k + 1) as f64, false);
                push(f.x, f.z);
            }
        }
        Self::new(points)
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn points(&self) -> &[[f32; 3]] {
        &self.points
    }

    pub fn point(&self, i: u32) -> Vec3 {
        Vec3::from_array(self.points[i as usize])
    }

    /// The `n` anchors nearest `p` (straight-line distance), the ones in `keep`
    /// counted `margin` nearer; nearest first. Searches cell rings round `p`
    /// until no farther ring can beat the n-th (at most `MAX_RINGS`).
    pub fn nearest(&self, p: Vec3, n: usize, keep: &[u32], margin: f32) -> Vec<u32> {
        if n == 0 || self.points.is_empty() {
            return vec![];
        }
        let (cx, cz) = cell_of(p.x as f64, p.z as f64);
        let mut found: Vec<(f32, u32)> = Vec::new();
        for r in 0..=MAX_RINGS {
            for dx in -r..=r {
                for dz in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    if let Some(list) = self.cells.get(&(cx + dx, cz + dz)) {
                        for &i in list {
                            let d = self.point(i).distance(p) - if keep.contains(&i) { margin } else { 0.0 };
                            found.push((d, i));
                        }
                    }
                }
            }
            if found.len() >= n {
                found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                found.truncate(n);
                // anything in ring r + 1 lies at least r cells away on the ground.
                if found[n - 1].0 <= r as f32 * CELL_M as f32 - margin {
                    break;
                }
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        found.truncate(n);
        found.into_iter().map(|(_, i)| i).collect()
    }
}

/// One probe of the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    pub anchor: u32,
    pub pos: Vec3,
    /// The slot shading reads, and the slot it cross-fades from.
    pub cur: Option<u32>,
    pub prev: Option<u32>,
    /// The cross-fade's progress (0: all `prev`, 1: all `cur`), linear in time.
    pub blend: f32,
    /// In the set and captured: rises to 1; else falls to 0 (linear in time).
    pub presence: f32,
    pub wanted: bool,
    /// The clock bucket and sky irradiance `cur` was captured under (None: redo).
    captured: Option<(i64, f64)>,
}

/// A capture under way: the probe, the slot it fills, the next face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capture {
    pub anchor: u32,
    pub slot: u32,
    pub face: u32,
    pub pos: Vec3,
    pub started: u64,
}

/// What the grid did since it was attached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridStats {
    /// Cubes finished, of which recaptures (a cross-fade), faces drawn.
    pub captures: u64,
    pub recaptures: u64,
    pub faces: u64,
    /// Faces moved off a heavy frame, captures started again after a camera
    /// jump, captures dropped (the probe left the set first).
    pub deferred: u64,
    pub restarted: u64,
    pub abandoned: u64,
    /// Frames from a capture's start to its cube in use: the last, the sum (mean = sum / captures).
    pub last_capture_frames: u32,
    pub capture_frames_sum: u64,
}

/// The grid's state, without the GPU (probe_grid.rs `ProbeGridGpu` drives it).
#[derive(Clone, Debug)]
pub struct Grid {
    pub level: Level,
    anchors: Anchors,
    probes: Vec<Probe>,
    capture: Option<Capture>,
    /// The face this frame draws (`update`), unless deferred.
    this_frame: Option<Capture>,
    frame: u64,
    bucket: i64,
    sky_up: f64,
    defer_run: u32,
    pub stats: GridStats,
}

/// The clock's 0.25 h bucket (the single probe's).
fn bucket_of(hour: f64) -> i64 {
    (hour / 0.25).floor() as i64
}

impl Grid {
    pub fn new(level: Level, anchors: Anchors) -> Self {
        Self { level, anchors, probes: vec![], capture: None, this_frame: None, frame: 0, bucket: i64::MIN, sky_up: f64::NAN, defer_run: 0, stats: GridStats::default() }
    }

    pub fn anchors(&self) -> &Anchors {
        &self.anchors
    }

    pub fn probes(&self) -> &[Probe] {
        &self.probes
    }

    pub fn capture(&self) -> Option<Capture> {
        self.capture
    }

    /// The face this frame draws (None: none, or deferred).
    pub fn this_frame(&self) -> Option<Capture> {
        self.this_frame
    }

    fn stale(&self, p: &Probe) -> bool {
        match p.captured {
            None => true,
            Some((b, s)) => b != self.bucket || (self.sky_up - s).abs() > 0.1 * s.abs().max(1e-9),
        }
    }

    /// Hold everything (frozen, or the grid off this frame): no face.
    pub fn hold(&mut self) {
        self.this_frame = None;
    }

    /// The selection's centre: `AHEAD_M` ahead of the eye on the ground.
    pub fn centre(eye: Vec3, fwd: Vec3) -> Vec3 {
        let h = Vec3::new(fwd.x, 0.0, fwd.z);
        if h.length_squared() > 1e-8 { eye + h.normalize() * AHEAD_M } else { eye }
    }

    /// Per frame: advance the fades by `dt` seconds, choose the set round the
    /// camera, keep or start a capture; returns this frame's face (the caller
    /// writes its uniforms; `defer` may still move it off). `cut`: the camera
    /// jumped since the last update (a capture under way starts again).
    pub fn update(&mut self, dt: f64, eye: Vec3, fwd: Vec3, hour: f64, sky_up: f64, cut: bool) -> Option<Capture> {
        self.frame += 1;
        // the last frame drew its face (none was moved off), or no capture runs.
        if self.this_frame.is_some() || self.capture.is_none() {
            self.defer_run = 0;
        }
        self.this_frame = None;
        self.bucket = bucket_of(hour);
        self.sky_up = sky_up;
        if cut {
            if let Some(c) = &mut self.capture {
                if c.face > 0 {
                    c.face = 0;
                    c.started = self.frame;
                    self.stats.restarted += 1;
                }
            }
        }
        // the fades.
        let step = (dt.max(0.0) / FADE_S) as f32;
        for p in &mut self.probes {
            let target = if p.wanted && p.cur.is_some() { 1.0 } else { 0.0 };
            p.presence = if p.presence < target { (p.presence + step).min(target) } else { (p.presence - step).max(target) };
            if p.prev.is_some() {
                p.blend = (p.blend + step).min(1.0);
                if p.blend >= 1.0 {
                    p.prev = None; // its slot is free again
                }
            }
        }
        // the set.
        let centre = Self::centre(eye, fwd);
        let keep: Vec<u32> = self.probes.iter().filter(|p| p.wanted).map(|p| p.anchor).collect();
        let want = self.anchors.nearest(centre, self.level.probes, &keep, KEEP_MARGIN_M);
        for p in &mut self.probes {
            p.wanted = want.contains(&p.anchor);
        }
        for &a in &want {
            if !self.probes.iter().any(|p| p.anchor == a) {
                self.probes.push(Probe { anchor: a, pos: self.anchors.point(a), cur: None, prev: None, blend: 1.0, presence: 0.0, wanted: true, captured: None });
            }
        }
        // a capture for a probe that left the set is dropped (its slot is free again).
        if let Some(c) = self.capture {
            if !self.probes.iter().any(|p| p.anchor == c.anchor && p.wanted) {
                self.capture = None;
                self.stats.abandoned += 1;
            }
        }
        // a probe out of the set and faded out is forgotten.
        self.probes.retain(|p| p.wanted || p.presence > 0.0);
        if self.capture.is_none() {
            self.capture = self.next_capture(centre);
        }
        self.this_frame = self.capture;
        self.this_frame
    }

    /// The next probe to capture: one entering the set first, then one whose
    /// cube is stale (not while its last cross-fade runs), nearest the centre
    /// first; into the lowest free slot (none free: wait).
    fn next_capture(&self, centre: Vec3) -> Option<Capture> {
        let slot = (0..self.level.slots as u32).find(|&s| !self.probes.iter().any(|p| p.cur == Some(s) || p.prev == Some(s)))?;
        let p = self
            .probes
            .iter()
            .filter(|p| p.wanted && p.prev.is_none() && (p.cur.is_none() || self.stale(p)))
            .min_by(|a, b| (a.cur.is_some(), a.pos.distance(centre)).partial_cmp(&(b.cur.is_some(), b.pos.distance(centre))).unwrap_or(std::cmp::Ordering::Equal))?;
        Some(Capture { anchor: p.anchor, slot, face: 0, pos: p.pos, started: self.frame })
    }

    /// Move this frame's face to a later frame (a heavy frame). True if there was one.
    pub fn defer(&mut self) -> bool {
        if self.this_frame.take().is_some() {
            self.stats.deferred += 1;
            self.defer_run += 1;
            true
        } else {
            false
        }
    }

    /// `defer`, unless the last `max` faces were all moved off (`--smooth-time`).
    pub fn defer_within(&mut self, max: u32) -> bool {
        if self.defer_run >= max {
            return false;
        }
        self.defer()
    }

    /// This frame's face was drawn. Returns the slot when it was the sixth (the
    /// caller has finished the cube into it): the probe reads it from the next
    /// frame, fading in, or cross-fading from its old cube.
    pub fn face_drawn(&mut self) -> Option<u32> {
        let job = self.this_frame.take()?;
        self.stats.faces += 1;
        let c = self.capture.as_mut()?;
        if job.face < 5 {
            c.face = job.face + 1;
            return None;
        }
        let c = self.capture.take()?;
        let (bucket, sky_up, frame) = (self.bucket, self.sky_up, self.frame);
        let p = self.probes.iter_mut().find(|p| p.anchor == c.anchor)?;
        if let Some(old) = p.cur {
            p.prev = Some(old);
            p.blend = 0.0;
            self.stats.recaptures += 1;
        }
        p.cur = Some(c.slot);
        p.captured = Some((bucket, sky_up));
        self.stats.captures += 1;
        let frames = (frame - c.started + 1) as u32;
        self.stats.last_capture_frames = frames;
        self.stats.capture_frames_sum += frames as u64;
        Some(c.slot)
    }

    /// The face list overflowed (a face missed objects): the capture under way
    /// starts again and every cube is captured again (each cross-fading).
    pub fn redo(&mut self) {
        if let Some(c) = &mut self.capture {
            c.face = 0;
            c.started = self.frame;
        }
        for p in &mut self.probes {
            p.captured = None;
        }
    }

    /// Every probe of the set captured under the clock and sky of now, faded
    /// in, no cross-fade and no capture under way.
    pub fn settled(&self) -> bool {
        self.capture.is_none() && self.probes.iter().all(|p| !p.wanted || (p.cur.is_some() && !self.stale(p) && p.presence >= 1.0 && p.prev.is_none()))
    }

    /// The table's rows (probe_grid.wgsl): header, then two rows per probe
    /// with a cube (faded weights; the irradiance SH rows are the fill's).
    pub fn table(&self) -> Vec<[f32; 4]> {
        let entries: Vec<&Probe> = self.probes.iter().filter(|p| p.cur.is_some()).collect();
        let mut rows = Vec::with_capacity(ENTRY_BASE + 2 * entries.len());
        rows.push([entries.len() as f32, (self.level.face as f32).log2(), (self.level.mips - 1) as f32, (SH_BASE / 16) as f32]);
        rows.push([WEIGHT_SOFT_M * WEIGHT_SOFT_M, REACH_M[0], REACH_M[1], 0.0]);
        for p in entries {
            rows.push([p.pos.x, p.pos.y, p.pos.z, smooth(p.presence)]);
            let (prev, b) = match p.prev {
                Some(s) => (s as f32, smooth(p.blend)),
                None => (-1.0, 1.0),
            };
            rows.push([p.cur.unwrap_or(0) as f32, prev, b, 0.0]);
        }
        rows
    }
}

/// The shader's choice at a point (probe_grid.wgsl `pg_select`, mirrored):
/// entries a and b, their weights (summing to 1), and how much of the grid
/// stands in for the analytic sky there (`cover`). None: no entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub a: usize,
    pub b: usize,
    pub wa: f32,
    pub wb: f32,
    pub cover: f32,
}

/// `pg_select` on the CPU from a table's rows (`Grid::table`).
pub fn selection(rows: &[[f32; 4]], world: Vec3) -> Option<Selection> {
    let n = rows.first()?[0].round() as usize;
    let k = rows[1];
    let (mut w1, mut w2, mut w3, mut i1, mut i2, mut cover) = (0f32, 0f32, 0f32, 0usize, 0usize, 0f32);
    for e in 0..n {
        let p = rows[ENTRY_BASE + 2 * e];
        let d2 = world.distance_squared(Vec3::new(p[0], p[1], p[2]));
        let w = p[3] / (d2 + k[0]);
        cover = cover.max(p[3] * (1.0 - smooth_between(k[1], k[2], d2.sqrt())));
        if w > w1 {
            w3 = w2;
            w2 = w1;
            i2 = i1;
            w1 = w;
            i1 = e;
        } else if w > w2 {
            w3 = w2;
            w2 = w;
            i2 = e;
        } else if w > w3 {
            w3 = w;
        }
    }
    if n == 0 || w1 <= 0.0 {
        return None;
    }
    let (mut a, mut b) = (w1 - w3, w2 - w3);
    if a + b <= 0.0 {
        a = 1.0;
        b = 0.0;
    }
    Some(Selection { a: i1, b: i2, wa: a / (a + b), wb: b / (a + b), cover })
}

/// The constants a shader variant compiles the lookups with, and the lookups.
/// `on` false compiles them out (the far layer: past the grid's reach).
pub fn wgsl(on: bool) -> String {
    let c = crate::probe::constants();
    format!(
        "// generated by probe_grid.rs wgsl\nconst PG_SHADER: bool = {on};\nconst PG_CANYON_M: f32 = {:.6};\nconst PG_GLAZING_FLOOR: f32 = {:.6};\n{WGSL}",
        c.canyon_m, c.glazing_floor
    )
}

struct Res {
    level: Level,
    _cubes: wgpu::Texture,
    _raw: wgpu::Texture,
    raw_layers: Vec<wgpu::TextureView>,
    depth: wgpu::TextureView,
    table: wgpu::Buffer,
    /// Per slot: the fill, the levels, the lobe (probe_fill.wgsl).
    fill_bgs: Vec<wgpu::BindGroup>,
    mip_bgs: Vec<Vec<wgpu::BindGroup>>,
    lobe_bgs: Vec<wgpu::BindGroup>,
    /// (`--probe-sky`) the frame's sky and the GGX levels, made only with SF_PROBE_SKY compiled in.
    sky: Option<SkyFill>,
    bytes: u64,
}

/// (`--probe-sky`, the reflect track) The probe grid's sky fill and GGX
/// levels (probe_sky.wgsl): per slot, the fill's bind group and one per level 1...
struct SkyFill {
    fill: wgpu::ComputePipeline,
    prefilter: wgpu::ComputePipeline,
    fill_bgs: Vec<wgpu::BindGroup>,
    pre_bgs: Vec<Vec<wgpu::BindGroup>>,
}

/// (`--probe-sky`) The volume's sky along a direction (sky_volume.wgsl
/// `vol_atmo_gain`, `vol_atmo_sky` and what they read: `VOL_CINE_AUREOLE_KEEP`,
/// `vol_cine_on`), cut out of that file so the probes' sky is the frame's.
pub(crate) fn atmo_source() -> String {
    const V: &str = include_str!("sky_volume.wgsl");
    let a = V.find("/// the atmosphere's sky radiance along a direction").expect("sky_volume.wgsl: vol_atmo_gain");
    let b = a + V[a..].find("// (`--look cinematic`, the cinematic look plan's stage L4").expect("sky_volume.wgsl: the end of vol_atmo_sky");
    let k = V.find("const VOL_CINE_AUREOLE_KEEP").expect("sky_volume.wgsl: VOL_CINE_AUREOLE_KEEP");
    let k_end = k + V[k..].find('\n').expect("a line");
    let c = V.find("fn vol_cine_on() -> bool {").expect("sky_volume.wgsl: vol_cine_on");
    let c_end = c + V[c..].find("\n}\n").expect("vol_cine_on's end") + 3;
    format!("// cut from sky_volume.wgsl by probe_grid.rs atmo_source\n{}\n{}\n{}", &V[k..k_end], &V[c..c_end], &V[a..b])
}

/// (`--probe-sky`) The sky fill's and the GGX levels' module: the look's
/// common code, probe_fill.wgsl (its bindings and the faces' basis), the
/// volume's sky (`atmo_source`), probe_sky.wgsl.
pub(crate) fn sky_source() -> String {
    format!("{}\n{}\n{}\n{}", crate::look::look_common_source(), include_str!("probe_fill.wgsl"), atmo_source(), include_str!("probe_sky.wgsl"))
}

impl SkyFill {
    /// The pipelines and every slot's bind groups (`level_view(slot, m)`: level m of a slot's six faces; `below(slot, m)`: its levels 0..m).
    fn new(
        d: &wgpu::Device,
        r: &crate::Renderer,
        lv: Level,
        raw_all: &wgpu::TextureView,
        level_view: &dyn Fn(u32, u32) -> wgpu::TextureView,
        below: &dyn Fn(u32, u32) -> wgpu::TextureView,
    ) -> Self {
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let tex = |dim| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dim, multisampled: false };
        let uni = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("probe-sky-bgl"),
            entries: &[
                entry(0, uni),
                entry(1, tex(wgpu::TextureViewDimension::D2Array)),
                entry(2, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: crate::probe::PROBE_FORMAT, view_dimension: wgpu::TextureViewDimension::D2Array }),
                entry(4, tex(wgpu::TextureViewDimension::D2)),
                entry(5, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(6, uni),
                entry(7, tex(wgpu::TextureViewDimension::D2Array)),
            ],
        });
        let src = sky_source();
        let m = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("probe-sky"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("probe-sky-layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipe = |e: &str| d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(e), layout: Some(&layout), module: &m, entry_point: Some(e), compilation_options: Default::default(), cache: None });
        let lin = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("probe-sky-linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let (view, _) = r.look.sky.atmo_tables();
        let bg = |dst: &wgpu::TextureView, levels: &wgpu::TextureView| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("probe-sky-bg"),
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: r.look.look_buffer().as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(raw_all) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(dst) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&lin) },
                    wgpu::BindGroupEntry { binding: 6, resource: r.look.sky.uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(levels) },
                ],
            })
        };
        let slots = lv.slots as u32;
        let fill_bgs = (0..slots).map(|s| bg(&level_view(s, 0), raw_all)).collect();
        let pre_bgs = (0..slots).map(|s| (1..lv.mips).map(|m| bg(&level_view(s, m), &below(s, m))).collect()).collect();
        crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, (slots * lv.mips) as u64);
        Self { fill: pipe("ps_fill"), prefilter: pipe("ps_prefilter"), fill_bgs, pre_bgs }
    }
}

#[derive(Default)]
struct State {
    grid: Option<Grid>,
    frozen: bool,
    last_eye: Option<Vec3>,
}

/// The probe grid's GPU side (`Renderer::probe_grid`).
pub struct ProbeGridGpu {
    device: wgpu::Device,
    res: OnceLock<Res>,
    state: Mutex<State>,
}

impl ProbeGridGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        Self { device: device.clone(), res: OnceLock::new(), state: Mutex::new(State::default()) }
    }

    /// Whether the cubes exist (`Renderer::attach_probe_grid`).
    pub fn attached(&self) -> bool {
        self.res.get().is_some()
    }

    /// Attached and asked for: the grid captures and shades, the single probe stands down.
    pub fn active(&self, look: &crate::look::LookGpu) -> bool {
        self.attached() && look.options.lighting.probe_grid > 0
    }

    pub fn level(&self) -> Option<Level> {
        self.res.get().map(|r| r.level)
    }

    /// GPU bytes allocated (the cubes, the face target and depth, the table).
    pub fn bytes(&self) -> u64 {
        self.res.get().map(|r| r.bytes).unwrap_or(0)
    }

    pub fn stats(&self) -> GridStats {
        self.state.lock().unwrap().grid.as_ref().map(|g| g.stats).unwrap_or_default()
    }

    /// Candidate anchors (0 unattached).
    pub fn anchors(&self) -> usize {
        self.state.lock().unwrap().grid.as_ref().map(|g| g.anchors().len()).unwrap_or(0)
    }

    /// The probes now (tests and reports).
    pub fn probes(&self) -> Vec<Probe> {
        self.state.lock().unwrap().grid.as_ref().map(|g| g.probes().to_vec()).unwrap_or_default()
    }

    /// Hold the grid as it is: no capture, no fade (the occlusion check's two
    /// renders of one camera must see the same environment).
    pub fn set_frozen(&self, on: bool) {
        self.state.lock().unwrap().frozen = on;
    }

    /// The set of this camera captured and faded in (true unattached).
    pub fn settled(&self) -> bool {
        self.state.lock().unwrap().grid.as_ref().is_none_or(|g| g.settled())
    }

    /// The camera's eye this update; true when it jumped more than `CUT_M` since the last.
    pub(crate) fn observe_eye(&self, eye: Vec3) -> bool {
        let mut st = self.state.lock().unwrap();
        let cut = st.last_eye.is_some_and(|e| e.distance(eye) > CUT_M);
        st.last_eye = Some(eye);
        cut
    }

    /// Make the cubes (`level` 1 or 2), the face target, the table and the
    /// finishing bind groups; bind 15 and 16. Returns the bytes allocated.
    pub(crate) fn attach(&self, r: &crate::Renderer, level: u8, anchors: Anchors) -> u64 {
        let Some(lv) = self::level(level.max(1)) else { return 0 };
        if self.res.get().is_some() {
            return 0;
        }
        let d = &self.device;
        let tex = |label: &str, size: u32, layers: u32, mips: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        use wgpu::TextureUsages as T;
        let fmt = crate::probe::PROBE_FORMAT;
        let cubes = tex("probe-grid-cubes", lv.face, 6 * lv.slots as u32, lv.mips, fmt, T::TEXTURE_BINDING | T::STORAGE_BINDING);
        let raw = tex("probe-grid-raw", lv.face, 6, 1, fmt, T::RENDER_ATTACHMENT | T::TEXTURE_BINDING);
        let depth_tex = tex("probe-grid-depth", lv.face, 1, 1, crate::DEPTH_FORMAT, T::RENDER_ATTACHMENT);
        let arr = |t: &wgpu::Texture, mip: Option<u32>, layer: u32, layers: Option<u32>| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: mip.unwrap_or(0),
                mip_level_count: mip.map(|_| 1),
                base_array_layer: layer,
                array_layer_count: layers,
                ..Default::default()
            })
        };
        let raw_layers: Vec<wgpu::TextureView> = (0..6)
            .map(|l| raw.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: l, array_layer_count: Some(1), ..Default::default() }))
            .collect();
        let raw_all = arr(&raw, None, 0, None);
        let depth = depth_tex.create_view(&Default::default());
        let all = arr(&cubes, None, 0, None);
        let table = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe-grid-table"),
            size: table_bytes(lv.slots),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // the single probe's finishing passes, into each slot's six layers (their layout is the pipelines' group 0).
        let [fill, _, _] = r.probe.finish_pipelines();
        let bgl = fill.get_bind_group_layout(0);
        let fbg = |src: &wgpu::TextureView, dst: &wgpu::TextureView, slot: u32| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("probe-grid-fill-bg"),
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: r.look.look_buffer().as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(dst) },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &table, offset: SH_BASE + SH_STRIDE * slot as u64, size: std::num::NonZeroU64::new(160) }),
                    },
                ],
            })
        };
        let level_view = |slot: u32, m: u32| arr(&cubes, Some(m), 6 * slot, Some(6));
        let (mut fill_bgs, mut mip_bgs, mut lobe_bgs) = (vec![], vec![], vec![]);
        for s in 0..lv.slots as u32 {
            fill_bgs.push(fbg(&raw_all, &level_view(s, 0), s));
            mip_bgs.push((1..lv.mips).map(|m| fbg(&level_view(s, m - 1), &level_view(s, m), s)).collect());
            lobe_bgs.push(fbg(&level_view(s, 3), &level_view(s, lv.mips - 1), s));
        }
        crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, (lv.slots as u32 * (lv.mips + 1)) as u64);
        // (`--probe-sky`) the frame's sky and the GGX levels, with SF_PROBE_SKY compiled in.
        let below = |slot: u32, m: u32| {
            cubes.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: 0,
                mip_level_count: Some(m),
                base_array_layer: 6 * slot,
                array_layer_count: Some(6),
                ..Default::default()
            })
        };
        let sky = crate::look::shader_features().probe_sky.then(|| SkyFill::new(d, r, lv, &raw_all, &level_view, &below));
        let bytes = lv.bytes();
        let res = Res { level: lv, _cubes: cubes, _raw: raw, raw_layers, depth, table, fill_bgs, mip_bgs, lobe_bgs, sky, bytes };
        if self.res.set(res).is_err() {
            return 0;
        }
        let res = self.res.get().unwrap();
        self.state.lock().unwrap().grid = Some(Grid::new(lv, anchors));
        r.look.set_probe_grid(Some(&all), Some(&res.table));
        // the single probe stands down: nothing of an earlier capture lingers in its cube.
        r.probe.reset();
        bytes
    }

    /// The CPU side, before the look's uniform takes its row: the fades, the
    /// set, this frame's face (its uniforms through the single probe's), the table.
    pub(crate) fn update(&self, r: &crate::Renderer, queue: &wgpu::Queue, cam: &Camera, frame_template: &[f32; 40], probe_on: bool, cut: bool) {
        let Some(res) = self.res.get() else { return };
        let on = r.look.options.lighting.probe_grid > 0 && probe_on;
        let mut guard = self.state.lock().unwrap();
        let st = &mut *guard;
        let Some(grid) = st.grid.as_mut() else { return };
        let job = if !on || st.frozen {
            grid.hold();
            None
        } else {
            let dt = r.look.sky.frame_dt();
            let dt = if dt > 0.0 { dt } else { FALLBACK_DT_S };
            grid.update(dt, cam.eye, cam.forward(), r.look.time_of_day(), r.look.frame().sky_irradiance_up, cut)
        };
        let mut rows = grid.table();
        if !on {
            rows.truncate(ENTRY_BASE);
            rows[0][0] = 0.0; // no entry: the shaders read nothing
        }
        drop(guard);
        if let Some(c) = job {
            r.probe.write_face(queue, c.pos, c.face, frame_template);
        }
        queue.write_buffer(&res.table, 0, bytemuck::cast_slice(&rows));
        r.look.set_lighting_params(crate::look::LX_PROBE_GRID, &[[rows[0][0], rows[0][1], rows[0][2], res.level.slots as f32]]);
    }

    /// Move this frame's face off (a heavy frame), `within` the smooth clock's bound.
    pub(crate) fn defer(&self, within: Option<u32>) {
        if let Some(g) = self.state.lock().unwrap().grid.as_mut() {
            match within {
                Some(max) => {
                    g.defer_within(max);
                }
                None => {
                    g.defer();
                }
            }
        }
    }

    /// The face list overflowed (count emulation): capture again.
    pub(crate) fn redo(&self) {
        if let Some(g) = self.state.lock().unwrap().grid.as_mut() {
            g.redo();
        }
    }

    /// This frame's face (cull + draw through the single probe's face pass)
    /// and, on a capture's sixth, the sky fill, levels and irradiance into its
    /// slot. Timed in `lighting::Q_PROBE_GRID` (an empty span without a face).
    pub(crate) fn encode(&self, enc: &mut wgpu::CommandEncoder, r: &crate::Renderer, ts: Ts<'_>) {
        let job = if self.active(&r.look) { self.state.lock().unwrap().grid.as_ref().and_then(|g| g.this_frame()) } else { None };
        let (Some(res), Some(job)) = (self.res.get(), job) else {
            let label = if r.look.options.lighting.probe_grid == 0 { "probe grid (off)" } else { "probe grid (idle)" };
            lighting::empty_span(enc, ts, lighting::Q_PROBE_GRID, label);
            return;
        };
        let begin = ts.map(|(q, b)| (q, b + lighting::Q_PROBE_GRID));
        let last = job.face == 5;
        let pbg = r.probe_draw_bg.lock().unwrap()[r.probe.front().0].clone();
        r.probe.encode_face_into(enc, &res.raw_layers[job.face as usize], &res.depth, &r.vertex_buf, &r.index_buf, &pbg, &r.look, begin, begin.filter(|_| !last).map(|(q, b)| (q, b + 1)));
        if last {
            let tw = begin.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(b + 1) });
            let [fill, mip, lobe] = r.probe.finish_pipelines();
            let s = job.slot as usize;
            let size = res.level.face;
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("probe-grid-fill"), timestamp_writes: tw });
            if let Some(k) = res.sky.as_ref().filter(|_| r.look.options.sky.probe_sky) {
                // (`--probe-sky`) the frame's sky where nothing was drawn, then the GGX levels
                pass.set_pipeline(&k.fill);
                pass.set_bind_group(0, &k.fill_bgs[s], &[]);
                pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 6);
                pass.set_pipeline(&k.prefilter);
                for m in 1..res.level.mips {
                    let ms = (size >> m).max(1);
                    pass.set_bind_group(0, &k.pre_bgs[s][(m - 1) as usize], &[]);
                    pass.dispatch_workgroups(ms.div_ceil(8), ms.div_ceil(8), 6);
                }
            } else {
                pass.set_pipeline(fill);
                pass.set_bind_group(0, &res.fill_bgs[s], &[]);
                pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 6);
                pass.set_pipeline(mip);
                for m in 1..res.level.mips {
                    let ms = (size >> m).max(1);
                    pass.set_bind_group(0, &res.mip_bgs[s][(m - 1) as usize], &[]);
                    pass.dispatch_workgroups(ms.div_ceil(8), ms.div_ceil(8), 6);
                }
            }
            pass.set_pipeline(lobe);
            pass.set_bind_group(0, &res.lobe_bgs[s], &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        crate::hitch::mark(crate::hitch::Mark::ProbeFace, 1);
        if let Some(g) = self.state.lock().unwrap().grid.as_mut() {
            if g.face_drawn().is_some() {
                crate::hitch::mark(crate::hitch::Mark::ProbeCaptureDone, 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: usize, step: f32) -> Anchors {
        Anchors::new((0..n).map(|i| [i as f32 * step, 1.7, 0.0]).collect())
    }

    const DT: f64 = 1.0 / 60.0;

    /// Run frames at a camera until the grid settles (one face a frame, none deferred); returns the frames run.
    fn settle(g: &mut Grid, eye: Vec3, fwd: Vec3, hour: f64, max: usize) -> usize {
        for k in 0..max {
            if g.update(DT, eye, fwd, hour, 1.0, false).is_some() {
                g.face_drawn();
            }
            if g.settled() {
                return k + 1;
            }
        }
        max
    }

    #[test]
    fn the_probe_sky_takes_the_volumes_own_sky_function() {
        // (`--probe-sky`) the cut holds the volume's sky and what it reads, and nothing past it
        let a = atmo_source();
        for name in ["fn vol_atmo_gain(", "fn vol_atmo_sky(", "const VOL_CINE_AUREOLE_KEEP", "fn vol_cine_on("] {
            assert_eq!(a.matches(name).count(), 1, "{name}");
        }
        assert!(!a.contains("VOL_CINE_HAZE_H_M") && !a.contains("@group"), "only the sky function and its two names");
        let s = sky_source();
        assert!(s.contains("fn ps_fill(") && s.contains("fn ps_prefilter(") && s.contains("fn fill("), "the grid's own entries beside the single probe's");
    }

    #[test]
    fn levels_and_table_layout() {
        let (l1, l2) = (level(1).unwrap(), level(2).unwrap());
        assert_eq!((l1.probes, l1.slots, l1.face, l1.mips), (4, 6, 128, 6));
        assert_eq!((l2.probes, l2.slots, l2.face, l2.mips), (9, 11, 256, 7));
        assert!(level(0).is_none());
        assert_eq!(l2.slots, MAX_SLOTS);
        // the entries fit below the SH, which is aligned for storage bindings.
        assert!(16 * (ENTRY_BASE + 2 * MAX_SLOTS) as u64 <= SH_BASE && SH_BASE % 256 == 0 && SH_STRIDE >= 160);
        // one 128 px cube with its six levels: 1.05 MB (the single probe's).
        assert_eq!(l1.cube_bytes(), 6 * 8 * (128 * 128 + 64 * 64 + 32 * 32 + 16 * 16 + 8 * 8 + 4 * 4));
        eprintln!("probe grid bytes: level 1 {:.1} MB, level 2 {:.1} MB", l1.bytes() as f64 / 1e6, l2.bytes() as f64 / 1e6);
    }

    #[test]
    fn nearest_matches_a_brute_force_search_and_keeps_its_members() {
        let mut pts = vec![];
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f64 / (1u64 << 31) as f64) as f32
        };
        for _ in 0..2000 {
            pts.push([rnd() * 1500.0 - 750.0, rnd() * 40.0, rnd() * 1500.0 - 750.0]);
        }
        let a = Anchors::new(pts.clone());
        for k in 0..40 {
            let p = Vec3::new(k as f32 * 37.0 - 700.0, 5.0, 300.0 - k as f32 * 21.0);
            let got = a.nearest(p, 9, &[], 0.0);
            let mut all: Vec<(f32, u32)> = pts.iter().enumerate().map(|(i, q)| (Vec3::from_array(*q).distance(p), i as u32)).collect();
            all.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
            assert_eq!(got, all.iter().take(9).map(|x| x.1).collect::<Vec<_>>(), "nearest at {p}");
        }
        // a member is kept until another is KEEP_MARGIN_M nearer.
        let l = line(3, 30.0);
        assert_eq!(l.nearest(Vec3::new(14.0, 1.7, 0.0), 1, &[], 0.0), vec![0]);
        assert_eq!(l.nearest(Vec3::new(20.0, 1.7, 0.0), 1, &[0], KEEP_MARGIN_M), vec![0], "kept: 20 m vs 10 m is inside the margin");
        assert_eq!(l.nearest(Vec3::new(25.0, 1.7, 0.0), 1, &[0], KEEP_MARGIN_M), vec![1], "25 m vs 5 m: switched");
        assert!(Anchors::default().nearest(Vec3::ZERO, 4, &[], 0.0).is_empty());
    }

    #[test]
    fn the_set_is_captured_one_face_a_frame_and_fades_in() {
        let lv = level(1).unwrap();
        let mut g = Grid::new(lv, line(12, 32.0));
        let (eye, fwd) = (Vec3::new(0.0, 1.7, 0.0), Vec3::X);
        let mut faces = 0;
        for _ in 0..200 {
            let before = g.stats.faces;
            if g.update(DT, eye, fwd, 15.0, 1.0, false).is_some() {
                g.face_drawn();
            }
            assert!(g.stats.faces - before <= 1, "never more than one face a frame");
            faces = g.stats.faces;
            if g.settled() {
                break;
            }
        }
        assert!(g.settled());
        assert_eq!(g.stats.captures, 4);
        assert_eq!(faces, 24, "six faces a probe");
        // the set: the four anchors nearest a point AHEAD_M ahead of the eye.
        let mut set: Vec<u32> = g.probes().iter().filter(|p| p.wanted).map(|p| p.anchor).collect();
        set.sort();
        assert_eq!(set, vec![0, 1, 2, 3]);
        // nothing captured again while nothing changes.
        for _ in 0..120 {
            assert!(g.update(DT, eye, fwd, 15.0, 1.0, false).is_none());
        }
        assert_eq!(g.stats.captures, 4);
    }

    #[test]
    fn a_recapture_cross_fades_over_fade_s_and_the_shading_follows() {
        let lv = level(1).unwrap();
        let mut g = Grid::new(lv, line(4, 32.0));
        let (eye, fwd) = (Vec3::new(0.0, 1.7, 0.0), Vec3::X);
        settle(&mut g, eye, fwd, 15.0, 400);
        let p0 = *g.probes().iter().find(|p| p.anchor == 0).unwrap();
        // the clock's bucket moves: every probe is captured again, each cross-fading.
        let mut done_at = None;
        for k in 0..60 {
            if g.update(DT, eye, fwd, 15.3, 1.0, false).is_some() && g.this_frame().unwrap().face == 5 {
                let anchor = g.this_frame().unwrap().anchor;
                g.face_drawn();
                if anchor == 0 {
                    done_at = Some(k);
                    break;
                }
            } else if g.this_frame().is_some() {
                g.face_drawn();
            }
        }
        assert!(done_at.is_some(), "probe 0 captured again within 60 frames");
        let p = *g.probes().iter().find(|p| p.anchor == 0).unwrap();
        assert_eq!(p.prev, p0.cur, "the old cube is kept to fade from");
        assert_ne!(p.cur, p0.cur);
        assert_eq!(p.blend, 0.0, "the shading reads the old cube alone when the new one is finished");
        // the table's cross-fade weight rises from 0 to 1 over FADE_S, monotonic, and the old slot is freed after.
        let row = |g: &Grid| {
            let t = g.table();
            let i = g.probes().iter().filter(|p| p.cur.is_some()).position(|p| p.anchor == 0).unwrap();
            t[ENTRY_BASE + 2 * i + 1]
        };
        assert_eq!(row(&g)[2], 0.0);
        let frames = (FADE_S / DT).round() as usize;
        let mut last = 0.0;
        let mut steps = vec![];
        for k in 0..frames + 2 {
            if g.update(DT, eye, fwd, 15.3, 1.0, false).is_some() {
                g.face_drawn();
            }
            let r = row(&g);
            let w = if r[1] < 0.0 { 1.0 } else { r[2] };
            assert!(w >= last, "the weight never goes back");
            steps.push(w - last);
            last = w;
            if k + 1 < frames - 1 {
                assert!(r[1] >= 0.0 && w < 1.0, "frame {k}: still fading ({w})");
            }
        }
        assert_eq!(last, 1.0, "done after FADE_S");
        assert_eq!(g.probes().iter().find(|p| p.anchor == 0).unwrap().prev, None, "the old slot is free again");
        // no frame takes more than a smoothstep's steepest share of the fade.
        let max_step = steps.iter().cloned().fold(0.0, f32::max);
        assert!(max_step <= 1.5 / frames as f32 + 1e-3, "largest step {max_step} of the cross-fade in one frame");
    }

    #[test]
    fn a_probe_leaving_the_set_fades_out_before_its_slot_is_reused() {
        let lv = level(1).unwrap();
        let mut g = Grid::new(lv, line(20, 32.0));
        let fwd = Vec3::X;
        settle(&mut g, Vec3::new(0.0, 1.7, 0.0), fwd, 15.0, 400);
        // walk on: anchor 0 drops out of the set.
        let eye = Vec3::new(120.0, 1.7, 0.0);
        let mut seen = vec![];
        for _ in 0..400 {
            if g.update(DT, eye, fwd, 15.0, 1.0, false).is_some() {
                g.face_drawn();
            }
            if let Some(p) = g.probes().iter().find(|p| p.anchor == 0) {
                assert!(!p.wanted);
                seen.push(p.presence);
            }
            // a slot is never shared by two probes.
            let mut used = vec![];
            for p in g.probes() {
                for s in [p.cur, p.prev].into_iter().flatten() {
                    assert!(!used.contains(&s), "slot {s} used twice");
                    used.push(s);
                }
            }
            if let Some(c) = g.capture() {
                assert!(!used.contains(&c.slot), "a capture writes a slot shading reads");
            }
        }
        assert!(seen.windows(2).all(|w| w[1] <= w[0]), "presence only falls");
        assert!(seen.len() >= (FADE_S / DT) as usize - 1, "it faded over FADE_S ({} frames)", seen.len());
        assert!(g.probes().iter().all(|p| p.anchor != 0), "forgotten once faded out");
        assert!(g.settled());
    }

    #[test]
    fn a_camera_jump_restarts_a_capture_under_way() {
        let lv = level(1).unwrap();
        let mut g = Grid::new(lv, line(4, 32.0));
        let (eye, fwd) = (Vec3::new(0.0, 1.7, 0.0), Vec3::X);
        for _ in 0..3 {
            g.update(DT, eye, fwd, 15.0, 1.0, false);
            g.face_drawn();
        }
        assert_eq!(g.capture().unwrap().face, 3);
        g.update(DT, eye + Vec3::new(0.0, 0.0, 2.0 * CUT_M), fwd, 15.0, 1.0, true);
        assert_eq!(g.this_frame().unwrap().face, 0, "the faces of one cube are drawn after the jump");
        assert_eq!(g.stats.restarted, 1);
    }

    #[test]
    fn deferral_moves_the_face_and_keeps_its_place() {
        let lv = level(1).unwrap();
        let mut g = Grid::new(lv, line(4, 32.0));
        let (eye, fwd) = (Vec3::new(0.0, 1.7, 0.0), Vec3::X);
        g.update(DT, eye, fwd, 15.0, 1.0, false);
        assert!(g.defer());
        assert!(g.face_drawn().is_none() && g.stats.faces == 0);
        assert_eq!(g.update(DT, eye, fwd, 15.0, 1.0, false).unwrap().face, 0, "the deferred face comes back");
        // within 2 (`--smooth-time`): the second heavy frame in a row moves it off, the third draws it.
        assert!(g.defer_within(2));
        g.update(DT, eye, fwd, 15.0, 1.0, false);
        assert!(!g.defer_within(2), "the third heavy frame in a row draws its face");
        assert_eq!(g.this_frame().unwrap().face, 0);
    }

    #[test]
    fn the_shading_blends_between_two_probes_with_no_step() {
        let lv = level(1).unwrap();
        let mut g = Grid::new(lv, line(4, 32.0));
        settle(&mut g, Vec3::new(0.0, 1.7, 0.0), Vec3::X, 15.0, 400);
        let rows = g.table();
        let n = rows[0][0] as usize;
        assert_eq!(n, 4);
        let entry_x = |e: usize| rows[ENTRY_BASE + 2 * e][0];
        // along the line of probes, 1.7 m up: at a probe its own cube, halfway a blend, the cover full.
        let mut last: Option<(f32, f32)> = None;
        for k in 0..=960 {
            let x = -20.0 + k as f32 * 0.15;
            let s = selection(&rows, Vec3::new(x, 1.7, 3.0)).unwrap();
            assert!((s.wa + s.wb - 1.0).abs() < 1e-5);
            assert!(s.cover > 0.999, "inside the reach");
            // the share of each probe's position along x: continuous.
            let pos = s.wa * entry_x(s.a) + s.wb * entry_x(s.b);
            if let Some((px, pp)) = last {
                assert!((pos - pp).abs() < 2.0, "x {x}: the blended probe moved {} m in {} m", (pos - pp).abs(), x - px);
            }
            last = Some((x, pos));
        }
        let at = |x: f32| selection(&rows, Vec3::new(x, 1.7, 0.0)).unwrap();
        let s = at(32.0);
        assert_eq!(entry_x(s.a), 32.0);
        assert!(s.wa > 0.95, "at a probe its own cube ({})", s.wa);
        let s = at(48.0);
        assert!(s.wa > 0.3 && s.wb > 0.3, "halfway a blend ({}, {})", s.wa, s.wb);
        assert_eq!([entry_x(s.a), entry_x(s.b)].iter().map(|v| *v as i32).sum::<i32>(), 32 + 64);
        // past the reach the analytic sky.
        let s = selection(&rows, Vec3::new(0.0, 1.7, 2000.0)).unwrap();
        assert!(s.cover < 1e-6);
        // no entry (the grid off): nothing.
        let mut off = rows[..ENTRY_BASE].to_vec();
        off[0][0] = 0.0;
        assert!(selection(&off, Vec3::ZERO).is_none());
    }

    #[test]
    fn a_probe_entering_fades_the_shading_in_from_the_sky() {
        let lv = level(1).unwrap();
        let mut g = Grid::new(lv, line(1, 32.0));
        let (eye, fwd) = (Vec3::new(0.0, 1.7, 0.0), Vec3::X);
        let mut covers = vec![];
        for _ in 0..120 {
            if g.update(DT, eye, fwd, 15.0, 1.0, false).is_some() {
                g.face_drawn();
            }
            covers.push(selection(&g.table(), eye).map(|s| s.cover).unwrap_or(0.0));
        }
        assert_eq!(covers[0], 0.0);
        assert_eq!(*covers.last().unwrap(), 1.0);
        assert!(covers.windows(2).all(|w| w[1] >= w[0] && w[1] - w[0] <= 1.5 / (FADE_S / DT) as f32 + 1e-3), "no step from the sky to the probe");
    }
}
