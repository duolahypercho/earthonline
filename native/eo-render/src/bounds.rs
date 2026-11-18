//! Compacted indirect lists drawn without multi-draw-indirect-count.
//!
//! On an adapter without `MULTI_DRAW_INDIRECT_COUNT` (Metal through wgpu
//! 30: `wgpu-hal/src/metal/command.rs` implements `draw_indexed_indirect` as
//! a loop of one `drawIndexedPrimitives:...indirectBuffer:` per command and
//! leaves `draw_indexed_indirect_count` empty), `multi_draw_indexed_indirect(
//! buf, offset, n)` costs n separate draws on the CPU and on the GPU. Drawing
//! one slot per object (the old fallback, about 212 k objects in the city
//! table) cost about 26 ms of encode a frame on its own.
//!
//! Here the GPU cull compacts every list as it does with the count, and the
//! CPU draws a fixed number of commands, the list's BOUND: the slots
//! [0, bound) are cleared to zero (an empty draw: no indices, no instances)
//! before the cull appends to them, so every slot past the count the cull
//! wrote draws nothing.
//!
//! A bound must never be below the count the GPU writes. The CPU learns the
//! counts a frame or two late (a read-back ring of the lists' counter words,
//! mapped on submit and never waited for: [`Feedback`]), so each cull also
//! counts a GUARD, a superset of what it could append for any view near the
//! one it was counted for:
//!
//! - `wide`: the camera's frustum widened by [`WIDE_DEG`] on every side, plus
//!   a sphere of [`NEAR_M`] round the eye. Used while the camera has since
//!   turned (plus the angle its move sweeps at [`NEAR_M`]) by less than half
//!   that widening.
//! - `ring`: every entry its view-independent tests pass within the far
//!   distance plus [`RING_MOVE_M`] of the eye. Used for any turn while the eye
//!   moved less than [`RING_MOVE_M`].
//!
//! Without a usable sample (the first frames, a camera jump, a list not drawn
//! lately) the list is drawn at its full capacity: slow, but exact. A
//! read-back count above the bound its frame was drawn with means that frame
//! missed entries: it is counted (`overflows`, reported by the bench) and the
//! list is drawn at capacity for the next [`COOLDOWN`] frames.

use glam::Vec3;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Each side of the camera's frustum is widened by this for the `wide` guard
/// (it holds for turns up to half of it: walking and driving turn well under
/// a degree a frame; a faster turn falls back to the ring guard).
pub const WIDE_DEG: f32 = 20.0;
/// Entries within this of the eye count in the `wide` guard whatever their direction.
pub const NEAR_M: f32 = 40.0;
/// The `ring` guard reaches at least this past the far distance; it stays
/// usable while the eye moves less. A moving camera widens it ([`move_margin`]).
pub const RING_MOVE_M: f32 = 64.0;
/// Frames of motion a ring guard's margin covers: the read-back lag (two
/// frames in flight, then the map) with room to spare.
pub const MARGIN_FRAMES: f32 = 8.0;
/// A step longer than this is a jump (a new shot), not motion.
pub const JUMP_M: f32 = 500.0;

/// The ring margin for a camera moving `speed` metres a frame, at least `base`.
pub fn move_margin(speed: f32, base: f32) -> f32 {
    (speed * MARGIN_FRAMES).max(base)
}
/// Added to every bound (entries that stream in or change level in the frames the sample is late).
pub const SLACK: u32 = 128;
/// Frames a list is drawn at capacity after an overflow.
pub const COOLDOWN: u32 = 60;
/// Read-back slots (frames whose counters can be in flight at once).
pub const FEEDBACK_SLOTS: usize = 6;

/// `EO_BOUNDS_CAPACITY=1`: draw every bounded list at its capacity (the same
/// commands as the indirect count would draw, slowly): a diagnostic, to tell
/// a fault of the bounds from one of what they draw.
pub fn at_capacity_forced() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("EO_BOUNDS_CAPACITY").is_ok_and(|v| v == "1"))
}

/// Which guard of a sample covers the view of a later frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    Wide,
    Ring,
    None,
}

/// The camera a view-dependent list was culled for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewKey {
    pub eye: Vec3,
    pub fwd: Vec3,
    pub fov_y_deg: f32,
    pub aspect: f32,
    pub far: f32,
    /// how far past the far distance the ring guard reached (the eye may move this far).
    pub margin: f32,
}

impl ViewKey {
    pub fn of(cam: &crate::camera::Camera, margin: f32) -> Self {
        Self { eye: cam.eye, fwd: cam.forward(), fov_y_deg: cam.fov_y_deg, aspect: cam.aspect, far: cam.far, margin }
    }

    /// Which of the guards counted for `self` holds for the view `now`.
    pub fn cover(&self, now: &ViewKey) -> Cover {
        let moved = now.eye.distance(self.eye);
        if !moved.is_finite() || now.far > self.far + 0.5 {
            return Cover::None;
        }
        let same_lens = (now.fov_y_deg - self.fov_y_deg).abs() < 1e-3 && (now.aspect - self.aspect).abs() < 1e-4;
        let turn = self.fwd.normalize_or_zero().dot(now.fwd.normalize_or_zero()).clamp(-1.0, 1.0).acos().to_degrees();
        // the farthest an entry outside the near sphere can swing as seen from the moved eye.
        let sweep = (moved / NEAR_M).min(1.0).asin().to_degrees();
        if same_lens && turn + sweep <= WIDE_DEG * 0.5 {
            Cover::Wide
        } else if moved <= self.margin {
            Cover::Ring
        } else {
            Cover::None
        }
    }

    /// The widened camera the `wide` guard is counted with (same eye and
    /// direction; the vertical and horizontal half-angles each WIDE_DEG
    /// larger, capped short of a half turn; the far plane NEAR_M farther, for
    /// the move the wide cover allows).
    pub fn widened(cam: &crate::camera::Camera) -> crate::camera::Camera {
        let v = (cam.fov_y_deg * 0.5).to_radians();
        let h = (v.tan() * cam.aspect).atan();
        let w = WIDE_DEG.to_radians();
        let cap = 85f32.to_radians();
        let (v2, h2) = ((v + w).min(cap), (h + w).min(cap));
        crate::camera::Camera { fov_y_deg: (2.0 * v2).to_degrees(), aspect: h2.tan() / v2.tan(), far: cam.far + NEAR_M, jitter: [0.0; 2], ..*cam }
    }
}

/// What a read-back frame says about one list: the count the cull wrote and its guards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Guard {
    pub count: u32,
    pub wide: u32,
    pub ring: u32,
}

/// Running numbers of one list's bounds (bench reports).
#[derive(Clone, Copy, Debug, Default)]
pub struct BoundStats {
    /// Frames drawn with a bound, and of them at full capacity.
    pub frames: u64,
    pub at_capacity: u64,
    pub wide: u64,
    pub ring: u64,
    /// Sum of the bounds drawn (draw commands issued).
    pub drawn: u64,
    /// Read-back samples, the sum of their counts, and those above the bound they were drawn with.
    pub samples: u64,
    pub counted: u64,
    pub overflows: u64,
    pub max_bound: u32,
}

impl BoundStats {
    pub fn json(&self) -> serde_json::Value {
        let per = |v: u64| if self.frames > 0 { v as f64 / self.frames as f64 } else { 0.0 };
        serde_json::json!({
            "frames": self.frames,
            "at_capacity": self.at_capacity,
            "by_wide_guard": self.wide,
            "by_ring_guard": self.ring,
            "commands_per_frame": per(self.drawn),
            "max_bound": self.max_bound,
            "samples": self.samples,
            "count_per_sample": if self.samples > 0 { self.counted as f64 / self.samples as f64 } else { 0.0 },
            "overflows": self.overflows,
        })
    }

    pub fn since(&self, before: &BoundStats) -> BoundStats {
        BoundStats {
            frames: self.frames - before.frames,
            at_capacity: self.at_capacity - before.at_capacity,
            wide: self.wide - before.wide,
            ring: self.ring - before.ring,
            drawn: self.drawn - before.drawn,
            samples: self.samples - before.samples,
            counted: self.counted - before.counted,
            overflows: self.overflows - before.overflows,
            max_bound: self.max_bound,
        }
    }
}

/// One compacted list's bound, from its newest read-back sample.
pub struct ListBound<K> {
    pub capacity: u32,
    last: Option<(u64, K, Guard)>,
    cooldown: u32,
    pub stats: BoundStats,
}

/// `n` plus a share and the slack, in whole 64s.
pub fn pad(n: u32, share: u32) -> u32 {
    (n.saturating_add(n / share).saturating_add(SLACK)).next_multiple_of(64)
}

impl<K: Copy> ListBound<K> {
    pub fn new(capacity: u32) -> Self {
        Self { capacity, last: None, cooldown: 0, stats: BoundStats::default() }
    }

    /// A read-back sample of `frame`, counted for `key`, drawn with `drawn_with` commands.
    pub fn sample(&mut self, frame: u64, key: K, g: Guard, drawn_with: u32) {
        self.stats.samples += 1;
        self.stats.counted += g.count as u64;
        if g.count > drawn_with {
            self.stats.overflows += 1;
            self.cooldown = COOLDOWN;
        }
        if self.last.is_none_or(|(f, _, _)| frame > f) {
            self.last = Some((frame, key, g));
        }
    }

    /// Forget every sample (the next frames draw at capacity until new ones arrive).
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// This frame's bound: the guard the newest sample's key covers (no older
    /// than `max_age` frames), else the capacity.
    pub fn bound(&mut self, frame: u64, max_age: u64, cover: impl Fn(&K) -> Cover) -> u32 {
        let how = match self.last {
            Some((f, k, _)) if self.cooldown == 0 && frame.saturating_sub(f) <= max_age && !at_capacity_forced() => cover(&k),
            _ => Cover::None,
        };
        self.cooldown = self.cooldown.saturating_sub(1);
        let g = self.last.map(|l| l.2).unwrap_or_default();
        let b = match how {
            Cover::Wide => pad(g.wide, 8),
            Cover::Ring => pad(g.ring, 8),
            Cover::None => self.capacity,
        }
        .min(self.capacity);
        self.stats.frames += 1;
        self.stats.drawn += b as u64;
        self.stats.max_bound = self.stats.max_bound.max(b);
        match how {
            Cover::Wide => self.stats.wide += 1,
            Cover::Ring => self.stats.ring += 1,
            Cover::None => self.stats.at_capacity += 1,
        }
        if b == self.capacity && how != Cover::None {
            self.stats.at_capacity += 1;
        }
        b
    }
}

const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

struct Slot<M> {
    buf: wgpu::Buffer,
    state: Arc<AtomicU8>,
    meta: Option<M>,
    /// the frame the slot was recorded at (a slot pending far longer was never submitted).
    since: u64,
}

/// A ring of small read-back buffers: each frame's counter words copied at
/// the end of its encode and mapped when that encoder is submitted, read
/// (never waited for) at a later frame's update.
pub struct Feedback<M> {
    device: wgpu::Device,
    slots: Vec<Slot<M>>,
    /// frame number (low, high words), written each update and copied first:
    /// a slot whose stamp is not its own frame's is thrown away.
    stamp: wgpu::Buffer,
    size: u64,
}

impl<M> Feedback<M> {
    pub fn new(device: &wgpu::Device, size: u64) -> Self {
        let size = (size + 16).next_multiple_of(wgpu::MAP_ALIGNMENT);
        let slots = (0..FEEDBACK_SLOTS).map(|_| Self::slot(device, size)).collect();
        let stamp = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounds-stamp"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { device: device.clone(), slots, stamp, size }
    }

    fn slot(device: &wgpu::Device, size: u64) -> Slot<M> {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounds-feedback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Slot { buf, state: Arc::new(AtomicU8::new(FREE)), meta: None, since: 0 }
    }

    /// This frame's stamp (call once per frame, before its encode).
    pub fn stamp(&self, queue: &wgpu::Queue, frame: u64) {
        queue.write_buffer(&self.stamp, 0, bytemuck::cast_slice(&[frame as u32, (frame >> 32) as u32, 0, 0]));
    }

    /// Copy `sources` ((buffer, bytes, offset in the slot past the stamp)) into
    /// a free slot and map it on submit. Returns false when no slot is free.
    pub fn record(&mut self, enc: &mut wgpu::CommandEncoder, frame: u64, sources: &[(&wgpu::Buffer, u64, u64)], meta: M) -> bool {
        // a slot pending for long was recorded into an encoder that was never submitted.
        for k in 0..self.slots.len() {
            let s = &self.slots[k];
            if s.state.load(Ordering::Acquire) == PENDING && frame.saturating_sub(s.since) > 240 {
                self.slots[k] = Self::slot(&self.device, self.size);
            }
        }
        let Some(k) = (0..self.slots.len()).find(|&k| self.slots[k].state.load(Ordering::Acquire) == FREE && self.slots[k].meta.is_none()) else {
            return false;
        };
        let s = &mut self.slots[k];
        enc.copy_buffer_to_buffer(&self.stamp, 0, &s.buf, 0, 16);
        for &(src, bytes, at) in sources {
            if 16 + at + bytes <= self.size {
                enc.copy_buffer_to_buffer(src, 0, &s.buf, 16 + at, bytes);
            }
        }
        s.state.store(PENDING, Ordering::Release);
        s.meta = Some(meta);
        s.since = frame;
        let flag = s.state.clone();
        enc.map_buffer_on_submit(&s.buf, wgpu::MapMode::Read, .., move |r| flag.store(if r.is_ok() { READY } else { FAILED }, Ordering::Release));
        true
    }

    /// Hand every slot that has arrived to `f` (its meta and its words past the
    /// stamp), oldest frame first, and free it.
    pub fn collect(&mut self, frame_of: impl Fn(&M) -> u64, mut f: impl FnMut(M, &[u32])) {
        let mut ready: Vec<usize> = (0..self.slots.len()).filter(|&k| matches!(self.slots[k].state.load(Ordering::Acquire), READY | FAILED)).collect();
        ready.sort_by_key(|&k| self.slots[k].meta.as_ref().map(&frame_of).unwrap_or(0));
        for k in ready {
            let s = &mut self.slots[k];
            let ok = s.state.load(Ordering::Acquire) == READY;
            let meta = s.meta.take();
            if ok {
                let words: Option<Vec<u32>> = s.buf.slice(..).get_mapped_range().ok().map(|v| bytemuck::pod_collect_to_vec(&v));
                s.buf.unmap();
                if let (Some(w), Some(m)) = (words, meta) {
                    let fr = frame_of(&m);
                    if w.len() >= 4 && w[0] == fr as u32 && w[1] == (fr >> 32) as u32 {
                        f(m, &w[4..]);
                    }
                }
            }
            s.state.store(FREE, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, aabb_visible, frustum_planes};

    fn key(yaw_deg: f32, eye: Vec3) -> ViewKey {
        ViewKey::of(&Camera::new(eye, yaw_deg.to_radians(), 0.0, 16.0 / 9.0), RING_MOVE_M)
    }

    #[test]
    fn cover_follows_the_turn_and_the_move() {
        let k = key(0.0, Vec3::ZERO);
        assert_eq!(k.cover(&key(5.0, Vec3::ZERO)), Cover::Wide);
        assert_eq!(k.cover(&key(9.0, Vec3::new(0.5, 0.0, 0.0))), Cover::Wide);
        assert_eq!(k.cover(&key(14.0, Vec3::ZERO)), Cover::Ring);
        assert_eq!(k.cover(&key(40.0, Vec3::ZERO)), Cover::Ring);
        assert_eq!(k.cover(&key(180.0, Vec3::new(30.0, 0.0, 0.0))), Cover::Ring);
        assert_eq!(k.cover(&key(0.0, Vec3::new(100.0, 0.0, 0.0))), Cover::None);
        let mut wide_far = key(0.0, Vec3::ZERO);
        wide_far.far += 10.0;
        assert_eq!(k.cover(&wide_far), Cover::None);
    }

    /// Every box the camera turned within the wide allowance sees is inside
    /// the widened frustum of the camera it turned from (or the near sphere).
    #[test]
    fn the_widened_frustum_holds_every_view_it_covers() {
        let eye = Vec3::new(0.0, 2.0, 0.0);
        let from = Camera { far: 4200.0, ..Camera::new(eye, 0.3, 0.05, 16.0 / 9.0) };
        let wide = frustum_planes(ViewKey::widened(&from).view_proj());
        let mut n = 0;
        for yaw in [-9.5f32, -5.0, 0.0, 5.0, 9.5] {
            for pitch in [-9.5f32, 0.0, 9.5] {
                let to = Camera { far: 4200.0, ..Camera::new(eye, 0.3 + yaw.to_radians(), 0.05 + pitch.to_radians(), 16.0 / 9.0) };
                if ViewKey::of(&from, RING_MOVE_M).cover(&ViewKey::of(&to, RING_MOVE_M)) != Cover::Wide {
                    continue;
                }
                let planes = frustum_planes(to.view_proj());
                for i in 0..4000u32 {
                    let a = (i as f32 * 0.6180339).fract() * std::f32::consts::TAU;
                    let r = 1.0 + (i as f32 * 0.7548776).fract() * 3000.0;
                    let y = ((i as f32 * 0.5698403).fract() - 0.5) * 600.0;
                    let c = eye + Vec3::new(a.sin() * r, y, -a.cos() * r);
                    let (mn, mx) = (c - Vec3::splat(2.0), c + Vec3::splat(2.0));
                    if aabb_visible(&planes, mn, mx) {
                        n += 1;
                        let near = mn.max(eye.min(mx)).distance(eye) <= NEAR_M;
                        assert!(near || aabb_visible(&wide, mn, mx), "box at {c} seen after yaw {yaw} pitch {pitch} is outside the widened frustum");
                    }
                }
            }
        }
        assert!(n > 1000);
    }

    #[test]
    fn a_bound_is_capacity_until_a_sample_and_after_an_overflow() {
        let mut b: ListBound<ViewKey> = ListBound::new(100_000);
        let k = key(0.0, Vec3::ZERO);
        assert_eq!(b.bound(1, 8, |s| s.cover(&k)), 100_000);
        b.sample(1, k, Guard { count: 900, wide: 2000, ring: 6000 }, 100_000);
        let wide = b.bound(3, 8, |s| s.cover(&k));
        assert!(wide >= 2000 + 250 + SLACK && wide < 2500 + SLACK + 64, "{wide}");
        let ring = b.bound(4, 8, |s| s.cover(&key(90.0, Vec3::ZERO)));
        assert!(ring >= 6000 + 750 + SLACK, "{ring}");
        assert_eq!(b.bound(40, 8, |s| s.cover(&k)), 100_000, "a stale sample");
        // a frame drawn with fewer commands than its count.
        b.sample(5, k, Guard { count: 3000, wide: 2000, ring: 6000 }, 2500);
        assert_eq!(b.stats.overflows, 1);
        assert_eq!(b.bound(6, 8, |s| s.cover(&k)), 100_000, "capacity after an overflow");
    }
}
