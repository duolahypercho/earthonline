//! the best-quality layer loads only for what the camera can see.
//!
//! The streamers keep two layers. The light layer (kit rung 2, the street's far
//! strips, footprint boxes, the props' far forms) stays radial around the camera
//! as before. The best-quality layer (kit rung-0 near packs, full street-surface
//! tiles, the Blender buildings' full models) is requested only for a tile or a
//! building that is within its usual near distance AND in view:
//!
//! - in view: inside the camera's frustum widened by `margin_deg` on every side
//!   (left, right, up, down), extended toward where the camera is turning (the
//!   smoothed yaw / pitch rate times `predict_s`); the test is separable in yaw
//!   and pitch and conservative (a box that touches the widened view passes; a
//!   view that reaches the zenith or the nadir passes every yaw);
//! - hysteresis: a key seen within the last `hold_s` seconds stays wanted, so
//!   looking around does not request and evict the same packs over and over.
//!
//! A tile the camera stands in contains the cone's apex and is always in view.
//! Requests are ordered in-view first by screen importance (distance to the box
//! times one plus its angle off the view centre over the half field of view),
//! then everything else nearest first (`rank`).
//!
//! Off (`--no-view-detail`), `ViewTracker::cone` is None and every streamer
//! behaves exactly as before.

use crate::camera::Camera;
use glam::Vec3;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// The switch and its numbers (`--no-view-detail`, `--view-margin-deg`,
/// `--view-predict-s`, `--view-hold-s`).
#[derive(Clone, Copy, Debug)]
pub struct ViewDetailConfig {
    pub enabled: bool,
    /// Degrees added to each side of the frustum (left, right, up, down).
    pub margin_deg: f32,
    /// Seconds of the camera's turn rate the view is extended by, toward the turn.
    pub predict_s: f32,
    /// Seconds a key stays wanted after it was last in the widened view.
    pub hold_s: f32,
}

impl Default for ViewDetailConfig {
    /// Off: the renderer's tests and every caller that does not opt in keep
    /// the radial streaming they had. eo-app turns it on (`--no-view-detail`
    /// turns it off again).
    fn default() -> Self {
        Self { enabled: false, margin_deg: 35.0, predict_s: 0.3, hold_s: 2.5 }
    }
}

impl ViewDetailConfig {
    pub fn on() -> Self {
        Self { enabled: true, ..Self::default() }
    }
}

/// The widened view of one frame, as the streamers test it.
#[derive(Clone, Copy, Debug)]
pub struct ViewCone {
    pub eye: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Half extents of the widened view (radians from the view centre).
    pub left: f32,
    pub right: f32,
    pub up: f32,
    pub down: f32,
    /// The un-widened horizontal half field of view (screen importance scale).
    pub half_h: f32,
    /// The view clock (seconds) and the hysteresis.
    pub t: f64,
    pub hold_s: f64,
    /// On, but no camera seen yet: nothing is in view (the best layer waits a frame).
    pub blind: bool,
}

impl ViewCone {
    fn blind() -> Self {
        Self { eye: Vec3::ZERO, yaw: 0.0, pitch: 0.0, left: 0.0, right: 0.0, up: 0.0, down: 0.0, half_h: 1.0, t: 0.0, hold_s: 0.0, blind: true }
    }
}

/// The direction's yaw in the camera's convention (0 north = -z, + toward +x).
fn yaw_of(dx: f32, dz: f32) -> f32 {
    dx.atan2(-dz)
}

fn wrap(a: f32) -> f32 {
    let mut a = (a + PI).rem_euclid(TAU) - PI;
    if a <= -PI {
        a += TAU;
    }
    a
}

impl ViewCone {
    /// Yaw interval of the box relative to the view yaw, or None when the eye
    /// stands inside the box's ground rectangle (every yaw).
    fn yaw_span(&self, min: Vec3, max: Vec3) -> Option<(f32, f32)> {
        let e = self.eye;
        if e.x >= min.x && e.x <= max.x && e.z >= min.z && e.z <= max.z {
            return None;
        }
        let c = yaw_of((min.x + max.x) * 0.5 - e.x, (min.z + max.z) * 0.5 - e.z);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for (x, z) in [(min.x, min.z), (max.x, min.z), (min.x, max.z), (max.x, max.z)] {
            let a = wrap(yaw_of(x - e.x, z - e.z) - c);
            lo = lo.min(a);
            hi = hi.max(a);
        }
        let rel = wrap(c - self.yaw);
        Some((rel + lo, rel + hi))
    }

    /// Elevation interval of the box seen from the eye.
    fn pitch_span(&self, min: Vec3, max: Vec3) -> (f32, f32) {
        let e = self.eye;
        let dx = (min.x - e.x).max(0.0).max(e.x - max.x);
        let dz = (min.z - e.z).max(0.0).max(e.z - max.z);
        let near = (dx * dx + dz * dz).sqrt();
        let fx = (e.x - min.x).abs().max((e.x - max.x).abs());
        let fz = (e.z - min.z).abs().max((e.z - max.z).abs());
        let far = (fx * fx + fz * fz).sqrt();
        let top = if max.y >= e.y { (max.y - e.y).atan2(near) } else { (max.y - e.y).atan2(far) };
        let bottom = if min.y <= e.y { (min.y - e.y).atan2(near) } else { (min.y - e.y).atan2(far) };
        (bottom, top)
    }

    /// Whether any part of the box falls inside the widened view.
    pub fn sees_box(&self, min: Vec3, max: Vec3) -> bool {
        if self.blind {
            return false;
        }
        let (b0, b1) = self.pitch_span(min, max);
        let (v0, v1) = (self.pitch - self.down, self.pitch + self.up);
        if b1 < v0 || b0 > v1 {
            return false;
        }
        // A view over the zenith or the nadir sees every yaw.
        if v1 >= FRAC_PI_2 || v0 <= -FRAC_PI_2 || self.left + self.right >= TAU {
            return true;
        }
        let Some((a0, a1)) = self.yaw_span(min, max) else { return true };
        [-TAU, 0.0, TAU].iter().any(|k| a1 + k >= -self.left && a0 + k <= self.right)
    }

    /// Angle from the view centre to the nearest part of the box (0 when the
    /// box covers the centre's yaw or the eye stands inside it).
    pub fn off_centre(&self, min: Vec3, max: Vec3) -> f32 {
        let Some((a0, a1)) = self.yaw_span(min, max) else { return 0.0 };
        [-TAU, 0.0, TAU]
            .iter()
            .map(|k| {
                let (l, h) = (a0 + k, a1 + k);
                if l <= 0.0 && h >= 0.0 { 0.0 } else { l.abs().min(h.abs()) }
            })
            .fold(PI, f32::min)
    }

    /// Screen importance of an in-view box (lower first): its distance, longer
    /// the farther it lies off the view centre.
    pub fn importance(&self, min: Vec3, max: Vec3, distance: f32) -> f32 {
        distance.max(1.0) * (1.0 + self.off_centre(min, max) / self.half_h.max(0.1))
    }
}

/// Request order of one streamed key (lower first). Without a cone: the
/// distance (as every streamer ordered before). With one: in view first by
/// screen importance, then the rest nearest first.
pub fn rank(cone: Option<&ViewCone>, min: Vec3, max: Vec3, distance: f32) -> f32 {
    match cone {
        None => distance,
        Some(c) if c.sees_box(min, max) => c.importance(min, max, distance),
        Some(_) => 1.0e6 + distance,
    }
}

/// The camera's turn, smoothed, and the cone of each frame.
pub struct ViewTracker {
    pub config: ViewDetailConfig,
    last: Option<(f64, f32, f32)>,
    yaw_rate: f32,
    pitch_rate: f32,
    cone: Option<ViewCone>,
    started: std::time::Instant,
    /// The last time a caller gave (a bench's camera clock); from then on a
    /// call without a time reuses it instead of the wall clock.
    explicit: Option<f64>,
}

/// Seconds over which the turn rate is smoothed.
const RATE_TAU_S: f32 = 0.06;

impl ViewTracker {
    pub fn new(config: ViewDetailConfig) -> Self {
        Self { config, last: None, yaw_rate: 0.0, pitch_rate: 0.0, cone: None, started: std::time::Instant::now(), explicit: None }
    }

    /// Seconds since the tracker was made.
    pub fn wall(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// The clock a call without a time uses: the last time given, else the wall.
    pub fn clock(&self) -> f64 {
        self.explicit.unwrap_or_else(|| self.wall())
    }

    /// `observe` at a caller's clock (it then owns the clock).
    pub fn observe_at(&mut self, cam: &Camera, t: f64) {
        self.explicit = Some(t);
        self.observe(cam, t);
    }

    /// Take this frame's camera at view-clock time `t` (seconds). A second call
    /// at the same or an earlier time only moves the cone, not the turn rate.
    pub fn observe(&mut self, cam: &Camera, t: f64) {
        if !self.config.enabled {
            self.cone = None;
            return;
        }
        match self.last {
            Some((t0, y0, p0)) if t > t0 => {
                let dt = (t - t0) as f32;
                let (iy, ip) = (wrap(cam.yaw - y0) / dt, (cam.pitch - p0) / dt);
                // A jump (teleport, path change) is not a turn.
                let (iy, ip) = if dt > 0.5 || iy.abs() > 60.0 { (0.0, 0.0) } else { (iy, ip) };
                let a = 1.0 - (-dt / RATE_TAU_S).exp();
                self.yaw_rate += (iy - self.yaw_rate) * a;
                self.pitch_rate += (ip - self.pitch_rate) * a;
                self.last = Some((t, cam.yaw, cam.pitch));
            }
            Some(_) => {}
            None => self.last = Some((t, cam.yaw, cam.pitch)),
        }
        let c = &self.config;
        let m = c.margin_deg.to_radians();
        let half_v = (cam.fov_y_deg.to_radians() * 0.5).min(FRAC_PI_2);
        let half_h = (half_v.tan() * cam.aspect.max(0.1)).atan();
        let (ty, tp) = (self.yaw_rate * c.predict_s, self.pitch_rate * c.predict_s);
        self.cone = Some(ViewCone {
            eye: cam.eye,
            yaw: cam.yaw,
            pitch: cam.pitch,
            left: (half_h + m + (-ty).max(0.0)).min(PI),
            right: (half_h + m + ty.max(0.0)).min(PI),
            up: (half_v + m + tp.max(0.0)).min(PI),
            down: (half_v + m + (-tp).max(0.0)).min(PI),
            half_h,
            t: self.last.map(|l| l.0).unwrap_or(t).max(t),
            hold_s: c.hold_s as f64,
            blind: false,
        });
    }

    /// The last frame's cone (None: view detail off; a blind cone, which sees
    /// nothing, before the first camera).
    pub fn cone(&self) -> Option<ViewCone> {
        if self.config.enabled { Some(self.cone.unwrap_or_else(ViewCone::blind)) } else { None }
    }

    pub fn rates(&self) -> (f32, f32) {
        (self.yaw_rate, self.pitch_rate)
    }
}

/// The hysteresis of one streamer: when each key was last in the widened view.
#[derive(Default)]
pub struct ViewHold<K> {
    seen: HashMap<K, f64>,
}

impl<K: Eq + std::hash::Hash + Copy> ViewHold<K> {
    /// Whether `key` (its box `min`..`max`) is in view now or was within the hold.
    pub fn wanted(&mut self, cone: &ViewCone, key: K, min: Vec3, max: Vec3) -> bool {
        if cone.sees_box(min, max) {
            self.seen.insert(key, cone.t);
            return true;
        }
        self.seen.get(&key).is_some_and(|s| cone.t - s <= cone.hold_s)
    }

    /// Forget keys past the hold (once a frame; the map stays small).
    pub fn prune(&mut self, cone: &ViewCone) {
        if self.seen.len() > 64 {
            self.seen.retain(|_, s| cone.t - *s <= cone.hold_s);
        }
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(yaw_deg: f32) -> Camera {
        Camera::new(Vec3::new(0.0, 1.7, 0.0), yaw_deg.to_radians(), 0.0, 16.0 / 9.0)
    }

    fn unit(x: f32, z: f32) -> (Vec3, Vec3) {
        (Vec3::new(x - 5.0, 0.0, z - 5.0), Vec3::new(x + 5.0, 10.0, z + 5.0))
    }

    #[test]
    fn widened_view_sees_ahead_and_the_sides_not_behind() {
        let mut t = ViewTracker::new(ViewDetailConfig::on());
        t.observe(&cam(0.0), 0.0);
        let c = t.cone().unwrap();
        // 52 degree vertical fov at 16:9: 40.9 degree half horizontal, + 35 = 75.9.
        assert!((c.right.to_degrees() - 75.9).abs() < 0.2, "{}", c.right.to_degrees());
        let (a, b) = unit(0.0, -50.0); // ahead (north)
        assert!(c.sees_box(a, b));
        let (a, b) = unit(50.0, -20.0); // 68 degrees right
        assert!(c.sees_box(a, b));
        let (a, b) = unit(0.0, 50.0); // behind
        assert!(!c.sees_box(a, b));
        let (a, b) = unit(50.0, 30.0); // 121 degrees right
        assert!(!c.sees_box(a, b));
        // Standing inside a box: always in view.
        assert!(c.sees_box(Vec3::new(-64.0, 0.0, -10.0), Vec3::new(64.0, 5.0, 118.0)));
    }

    #[test]
    fn a_turn_widens_the_view_toward_it_and_the_hold_keeps_what_was_seen() {
        let mut t = ViewTracker::new(ViewDetailConfig::on());
        let mut hold = ViewHold::default();
        let (ba, bb) = unit(0.0, 50.0); // behind at the start
        // Turning right at 720 degrees a second for a few frames.
        for i in 0..6 {
            t.observe(&cam(i as f32 * 12.0), i as f64 / 60.0);
        }
        let c = t.cone().unwrap();
        assert!(c.right > c.left + 1.0, "right {} left {}", c.right, c.left);
        assert!(c.sees_box(ba, bb), "the turn's prediction reaches behind");
        assert!(hold.wanted(&c, 1u32, ba, bb));
        // Turned back and steady: still held for hold_s, then not.
        let mut tt = 6.0 / 60.0;
        for _ in 0..30 {
            tt += 1.0 / 60.0;
            t.observe(&cam(0.0), tt);
        }
        let c = t.cone().unwrap();
        assert!(!c.sees_box(ba, bb));
        assert!(hold.wanted(&c, 1u32, ba, bb), "within the hold");
        t.observe(&cam(0.0), tt + 3.0);
        let c = t.cone().unwrap();
        assert!(!hold.wanted(&c, 1u32, ba, bb), "past the hold");
    }

    #[test]
    fn looking_down_sees_every_yaw_and_rank_puts_the_view_first() {
        let mut t = ViewTracker::new(ViewDetailConfig::on());
        let mut c0 = cam(0.0);
        c0.eye.y = 300.0;
        c0.pitch = -1.2;
        t.observe(&c0, 0.0);
        let c = t.cone().unwrap();
        let (a, b) = unit(0.0, 80.0);
        assert!(c.sees_box(a, b), "behind, under a downward view");
        let mut t = ViewTracker::new(ViewDetailConfig::on());
        t.observe(&cam(0.0), 0.0);
        let c = t.cone().unwrap();
        let (fa, fb) = unit(0.0, -200.0);
        let (ba, bb) = unit(0.0, 30.0);
        assert!(rank(Some(&c), fa, fb, 195.0) < rank(Some(&c), ba, bb, 25.0), "in view before nearer out of view");
        let (sa, sb) = unit(140.0, -60.0); // in view, off centre
        let (ca, cb) = unit(0.0, -150.0); // in view, centred
        assert!(rank(Some(&c), ca, cb, 145.0) < rank(Some(&c), sa, sb, 145.0));
        assert_eq!(rank(None, ba, bb, 25.0), 25.0);
        // Off: no cone at all. On before the first camera: a cone that sees nothing.
        let mut off = ViewTracker::new(ViewDetailConfig::default());
        off.observe(&cam(0.0), 0.0);
        assert!(off.cone().is_none());
        let blind = ViewTracker::new(ViewDetailConfig::on()).cone().unwrap();
        assert!(blind.blind && !blind.sees_box(Vec3::splat(-1e4), Vec3::splat(1e4)));
    }
}
