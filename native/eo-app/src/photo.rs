//! Photo mode in `--play` and `--connect`: P toggles it.
//!
//! The local player's input is frozen (in `--play` the local world holds
//! still too: traffic, people and the player stop; in `--connect` the
//! server's simulation runs on and the client keeps sending empty inputs).
//! A free camera flies or orbits within `RADIUS_M` of the player, swept as a
//! ball against the collision world; the lens runs 14-200 mm (35 mm
//! equivalent, `eo_render::photo`), with roll, focus (on the player, on what
//! is at the centre, or a distance), aperture, exposure compensation, a
//! grade, vignetting, grain, letterbox, the rule-of-thirds guides and a
//! local time of day. The HUD is hidden while the panel (eo-ui
//! `photo::PhotoPanel`) is up. Enter saves a PNG, at the window's size or
//! rendered at twice it, into `photos_dir()`, with the settings beside it.
//!
//! Keys in photo mode: mouse look; W A S D fly (orbit: W S nearer and
//! farther), Q E down and up, Shift faster, Ctrl slower; wheel the lens;
//! Z X roll; Up Down choose a setting, Left Right change it; O orbit or
//! fly; H hide the panel; Enter capture; P or Esc leave.
//!
//! `--no-photo-mode` takes all of it away (P does nothing, the frame is the
//! game's); `--no-dof` keeps photo mode without the depth of field.

use anyhow::{Context, Result};
use eo_player::world::{CollisionWorld, layer};
use eo_render::camera::Camera;
use eo_render::photo::{self as rp, Dof, GRADES, Grade, PhotoParams};
use eo_render::{GpuContext, Renderer, Targets, read_mapped, wgpu};
use eo_ui::photo::{PhotoPanel, PhotoRow};
use glam::DVec3;
#[cfg(test)]
use glam::Vec3;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The key that enters and leaves photo mode (unbound in the game's map, eo-player `input.rs`).
pub const TOGGLE: winit::keyboard::KeyCode = winit::keyboard::KeyCode::KeyP;
/// The camera stays within this of the player (metres).
pub const RADIUS_M: f64 = 30.0;
/// The camera's collision ball, metres.
pub const BALL_M: f64 = 0.2;
const SKIN_M: f64 = 0.03;
pub const LENS_MIN_MM: f64 = 14.0;
pub const LENS_MAX_MM: f64 = 200.0;
/// Third stops from f/1.4 to f/22 (the marked values of ISO 517's series).
pub const STOPS: [f64; 25] = [1.4, 1.6, 1.8, 2.0, 2.2, 2.5, 2.8, 3.2, 3.5, 4.0, 4.5, 5.0, 5.6, 6.3, 7.1, 8.0, 9.0, 10.0, 11.0, 13.0, 14.0, 16.0, 18.0, 20.0, 22.0];
/// Letterbox openings: the scope (2.39:1) and flat (1.85:1) cinema frames
/// (SMPTE ST 195 / DCI projection apertures), 4:3 and square.
pub const FRAMES: [(Option<f64>, &str); 5] = [(None, "Full"), (Some(2.39), "2.39:1"), (Some(1.85), "1.85:1"), (Some(4.0 / 3.0), "4:3"), (Some(1.0), "1:1")];
/// The near plane in photo mode (close framing; the game's is 0.5 m).
pub const NEAR_M: f32 = 0.2;
/// The player's eye height for focusing on the player: 0.936 of stature
/// (eye height over stature of the 50th-percentile adult, Pheasant and
/// Haslegrave, *Bodyspace*, 3rd ed., table 10.1: 1630 / 1740 mm), stature 1.75 m.
pub const SUBJECT_EYE_M: f64 = 1.64;
/// Autofocus settles with this time constant (s).
const FOCUS_TAU_S: f64 = 0.12;
/// Frames a 2x capture draws before it is read (temporal resolve and the depth of field converged).
pub const CAPTURE_2X_FRAMES: usize = 48;

/// Whether photo mode exists in this run (`--no-photo-mode`: none of it).
pub fn enabled() -> bool {
    !std::env::args().any(|a| a == "--no-photo-mode")
}

/// Whether photo mode draws its depth of field (`--no-dof`).
pub fn dof_enabled() -> bool {
    !std::env::args().any(|a| a == "--no-dof")
}

/// `--photo-bench`: the bench's paths through photo mode.
pub fn bench_requested() -> bool {
    std::env::args().any(|a| a == "--photo-bench")
}

/// The bench's photo: f/1.4 focused at 0.5 m through the game's 52 degree
/// lens (20.8 mm): nearly every pixel is behind the focus and blurred to
/// about a 3 % radius of the height, the gather's most rings: the depth of
/// field's heaviest frame. Neutral grade, vignetting and grain on (the finish's whole cost).
pub fn bench_params() -> PhotoParams {
    PhotoParams { dof: Some(Dof { f_number: 1.4, focus_m: 0.5 }), exposure_ev: 0.0, grade: Grade::WarmFilm, vignette: 0.5, grain: 0.5, letterbox: None, accumulate: rp::STILL_FRAMES, animate_grain: true }
}

/// `--photo-bench`'s exact cost: the photo passes' GPU time at one still
/// camera, `frames` frames with the depth of field and `frames` without
/// (the finish alone, as `--no-dof`), interleaved in blocks of ten so both
/// see the same load. Medians and p95, milliseconds.
pub fn gpu_cost(ctx: &GpuContext, renderer: &Renderer, attach: &wgpu::TextureView, resolve: Option<&wgpu::TextureView>, depth: &wgpu::TextureView, cam: &Camera, frames: usize) -> serde_json::Value {
    if !ctx.timestamps {
        return json!({ "error": "no timestamp queries on this adapter" });
    }
    renderer.photo.enable_timer(&ctx.device);
    let period = ctx.queue.get_timestamp_period() as f64;
    let base = bench_params();
    let mut with = vec![];
    let mut without = vec![];
    let mut k = 0;
    while with.len() < frames || without.len() < frames {
        let dof = (k / 10) % 2 == 0;
        k += 1;
        // a still view: the accumulation runs as in a held photo; the jitter still turns.
        renderer.photo.set(Some(PhotoParams { dof: if dof { base.dof } else { None }, ..base }));
        renderer.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        renderer.encode(&mut enc, attach, resolve, depth, None);
        ctx.queue.submit([enc.finish()]);
        if let Some(ms) = renderer.photo.timer_ms(&ctx.device, period) {
            if k > 20 {
                if dof { with.push(ms) } else { without.push(ms) }
            }
        }
    }
    renderer.photo.set(Some(base));
    let stat = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        json!({ "median_ms": v[v.len() / 2], "p95_ms": v[(v.len() * 95 / 100).min(v.len() - 1)], "n": v.len() })
    };
    json!({ "photo_passes_with_dof": stat(&mut with), "photo_passes_finish_only": stat(&mut without), "params": format!("{base:?}") })
}

/// Where photos go: `EO_PHOTOS_DIR`, else the application-data folder (eo-ui `settings::photos_dir`).
pub fn photos_dir() -> Option<PathBuf> {
    eo_ui::settings::photos_dir()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The player's eyes.
    Subject,
    /// Whatever the centre of the frame meets.
    Centre,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Camera,
    Lens,
    Roll,
    Focus,
    Distance,
    Aperture,
    Exposure,
    Look,
    Vignette,
    Grain,
    Frame,
    Grid,
    Time,
    Capture,
}

const ROWS: [Row; 14] = [Row::Camera, Row::Lens, Row::Roll, Row::Focus, Row::Distance, Row::Aperture, Row::Exposure, Row::Look, Row::Vignette, Row::Grain, Row::Frame, Row::Grid, Row::Time, Row::Capture];

/// What the window feeds photo mode each frame.
pub struct Feed<'a> {
    pub held: &'a HashSet<String>,
    pub mouse: [f64; 2],
    pub dt: f64,
    pub aspect: f64,
    /// The player's pivot (the 30 m sphere's centre) and eyes.
    pub anchor: DVec3,
    pub cw: Option<&'a CollisionWorld>,
}

pub struct PhotoMode {
    pub on: bool,
    /// `--connect`: the world moves under a still view.
    pub live: bool,
    pub eye: DVec3,
    /// Renderer convention: yaw 0 north (-z), toward east positive; pitch up positive (radians).
    pub yaw: f64,
    pub pitch: f64,
    pub roll_deg: f64,
    pub orbit: bool,
    orbit_m: f64,
    pub anchor: DVec3,
    pub focal_mm: f64,
    /// None: the aperture "off" (everything sharp); else an index into `STOPS`.
    pub stop: Option<usize>,
    pub focus: Focus,
    pub focus_m: f64,
    focus_now: f64,
    pub ev_thirds: i32,
    pub grade: Grade,
    pub vignette: f64,
    pub grain: f64,
    pub frame: usize,
    pub thirds: bool,
    /// The local time of day override (None: the game's hour).
    pub hour: Option<f64>,
    pub game_hour: f64,
    pub capture_2x: bool,
    pub panel: bool,
    row: usize,
    toast: Option<(String, Instant)>,
    /// A capture asked for (the scale), taken after the frame.
    pub request: Option<u32>,
}

impl PhotoMode {
    pub fn new(live: bool, game_hour: f64) -> Self {
        Self {
            on: false,
            live,
            eye: DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            roll_deg: 0.0,
            orbit: false,
            orbit_m: 4.0,
            anchor: DVec3::ZERO,
            focal_mm: 35.0,
            stop: Some(6),
            focus: Focus::Subject,
            focus_m: 5.0,
            focus_now: 5.0,
            ev_thirds: 0,
            grade: Grade::Neutral,
            vignette: 0.35,
            grain: 0.0,
            frame: 0,
            thirds: true,
            hour: None,
            game_hour,
            capture_2x: false,
            panel: true,
            row: 1,
            toast: None,
            request: None,
        }
    }

    /// Enter at the game camera's pose and lens.
    pub fn enter(&mut self, cam: &Camera, anchor: DVec3) {
        self.on = true;
        self.eye = cam.eye.as_dvec3();
        self.yaw = cam.yaw as f64;
        self.pitch = cam.pitch as f64;
        self.roll_deg = 0.0;
        self.anchor = anchor;
        self.orbit_m = (self.eye - anchor).length().clamp(1.0, RADIUS_M);
        self.focal_mm = rp::focal_mm(cam.fov_y_deg as f64, cam.aspect as f64).clamp(LENS_MIN_MM, LENS_MAX_MM);
        self.focus_now = (anchor + DVec3::Y * (SUBJECT_EYE_M - eo_player::camera::on_foot::PIVOT_HEIGHT) - self.eye).length().max(0.3);
        self.request = None;
    }

    pub fn leave(&mut self) {
        self.on = false;
        self.request = None;
    }

    fn forward(&self) -> DVec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        DVec3::new(sy * cp, sp, -cy * cp)
    }

    /// The renderer's camera.
    pub fn camera(&self, aspect: f64, far: f32) -> Camera {
        Camera {
            fov_y_deg: rp::fov_y_deg(self.focal_mm, aspect) as f32,
            near: NEAR_M,
            far,
            roll: (self.roll_deg as f32).to_radians(),
            ..Camera::new(self.eye.as_vec3(), self.yaw as f32, self.pitch as f32, aspect as f32)
        }
    }

    /// The renderer's photo settings at this frame's focus.
    pub fn params(&self) -> PhotoParams {
        PhotoParams {
            dof: self.stop.map(|s| Dof { f_number: STOPS[s], focus_m: self.focus_now }),
            exposure_ev: self.ev_thirds as f64 / 3.0,
            grade: self.grade,
            vignette: self.vignette,
            grain: self.grain,
            letterbox: FRAMES[self.frame].0,
            accumulate: if self.live { rp::LIVE_FRAMES } else { rp::STILL_FRAMES },
            animate_grain: true,
        }
    }

    /// One frame: look, fly or orbit (swept against the world), autofocus.
    pub fn step(&mut self, f: &Feed) {
        self.anchor = f.anchor;
        let k = |c: &str| f.held.contains(c);
        let fov = rp::fov_y_deg(self.focal_mm, f.aspect);
        // finer look at a long lens (the turn per pixel follows the field of view).
        let s = eo_player::input::MOUSE_PITCH_RAD_PER_PX * (fov / 52.0).clamp(0.08, 1.5);
        self.yaw = (self.yaw + f.mouse[0] * s).rem_euclid(std::f64::consts::TAU);
        self.pitch = (self.pitch - f.mouse[1] * s).clamp(-1.5, 1.5);
        let mut speed = 3.0;
        if k("ShiftLeft") || k("ShiftRight") {
            speed *= 4.0;
        }
        if k("ControlLeft") || k("ControlRight") {
            speed *= 0.25;
        }
        let step = speed * f.dt;
        let fwd = self.forward();
        let (sy, cy) = self.yaw.sin_cos();
        let right = DVec3::new(cy, 0.0, sy);
        let axis = |a: &str, b: &str| (k(a) as i32 - k(b) as i32) as f64;
        if self.orbit {
            self.orbit_m = (self.orbit_m - axis("KeyW", "KeyS") * step).clamp(0.8, RADIUS_M);
            let pivot = f.anchor;
            let back = -fwd;
            let d = match f.cw.and_then(|cw| cw.sphere_cast(pivot, back, self.orbit_m, BALL_M, layer::WORLD | layer::STREET, None)) {
                Some((t, _)) => (t - SKIN_M).max(0.3),
                None => self.orbit_m,
            };
            self.eye = pivot + back * d;
        } else {
            let mv = fwd * axis("KeyW", "KeyS") + right * axis("KeyD", "KeyA") + DVec3::Y * axis("KeyE", "KeyQ");
            if mv.length_squared() > 0.0 {
                self.eye = sweep(f.cw, self.eye, mv.normalize() * step);
            }
            // within reach of the player.
            let off = self.eye - f.anchor;
            if off.length() > RADIUS_M {
                self.eye = f.anchor + off.normalize() * RADIUS_M;
            }
        }
        // focus: the target distance along the view axis, approached like a lens motor.
        let target = match self.focus {
            Focus::Manual => self.focus_m,
            Focus::Subject => (f.anchor + DVec3::Y * (SUBJECT_EYE_M - eo_player::camera::on_foot::PIVOT_HEIGHT) - self.eye).dot(fwd).max(0.3),
            Focus::Centre => f.cw.and_then(|cw| cw.raycast(self.eye, fwd, 1000.0, layer::WORLD | layer::STREET | layer::VEHICLE, None)).map(|h| h.distance).unwrap_or(1000.0).max(0.3),
        };
        if self.focus != Focus::Manual {
            self.focus_m = target;
        }
        let a = 1.0 - (-f.dt / FOCUS_TAU_S).exp();
        // in log distance, as a focus ring moves; within 0.1 % it holds (a
        // still view must keep the same focus for its accumulation to run).
        let gap = target.ln() - self.focus_now.ln();
        self.focus_now = if gap.abs() < 1e-3 { target } else { (self.focus_now.ln() + gap * a).exp() };
        if self.toast.as_ref().is_some_and(|(_, t)| t.elapsed().as_secs_f64() > 3.0) {
            self.toast = None;
        }
    }

    /// The wheel: the lens (a 4 % step a notch).
    pub fn wheel(&mut self, notches: f64) {
        self.focal_mm = (self.focal_mm * 1.04f64.powf(notches)).clamp(LENS_MIN_MM, LENS_MAX_MM);
    }

    /// A key pressed in photo mode; true when photo mode used it.
    pub fn key(&mut self, code: winit::keyboard::KeyCode) -> bool {
        use winit::keyboard::KeyCode as K;
        match code {
            K::ArrowUp => self.row = (self.row + ROWS.len() - 1) % ROWS.len(),
            K::ArrowDown => self.row = (self.row + 1) % ROWS.len(),
            K::ArrowLeft => self.change(-1),
            K::ArrowRight => self.change(1),
            K::Enter | K::NumpadEnter => self.request = Some(if self.capture_2x { 2 } else { 1 }),
            K::KeyH => self.panel = !self.panel,
            K::KeyO => self.orbit = !self.orbit,
            K::KeyZ => self.roll_deg = (self.roll_deg - 1.0).clamp(-45.0, 45.0),
            K::KeyX => self.roll_deg = (self.roll_deg + 1.0).clamp(-45.0, 45.0),
            _ => return false,
        }
        true
    }

    fn change(&mut self, d: i32) {
        let df = d as f64;
        match ROWS[self.row] {
            Row::Camera => self.orbit = !self.orbit,
            Row::Lens => self.focal_mm = (self.focal_mm * 1.06f64.powf(df)).clamp(LENS_MIN_MM, LENS_MAX_MM),
            Row::Roll => self.roll_deg = (self.roll_deg + df).clamp(-45.0, 45.0),
            Row::Focus => {
                let modes = [Focus::Subject, Focus::Centre, Focus::Manual];
                let i = modes.iter().position(|m| *m == self.focus).unwrap_or(0) as i32;
                self.focus = modes[(i + d).rem_euclid(3) as usize];
            }
            Row::Distance => {
                self.focus = Focus::Manual;
                self.focus_m = (self.focus_m * 1.1f64.powf(df)).clamp(0.3, 1000.0);
            }
            Row::Aperture => {
                self.stop = match (self.stop, d > 0) {
                    (None, true) => Some(0),
                    (None, false) => None,
                    (Some(0), false) => None,
                    (Some(s), true) => Some((s + 1).min(STOPS.len() - 1)),
                    (Some(s), false) => Some(s - 1),
                }
            }
            Row::Exposure => self.ev_thirds = (self.ev_thirds + d).clamp(-9, 9),
            Row::Look => {
                let i = GRADES.iter().position(|g| *g == self.grade).unwrap_or(0) as i32;
                self.grade = GRADES[(i + d).rem_euclid(GRADES.len() as i32) as usize];
                self.grain = self.grade.look().grain;
            }
            Row::Vignette => self.vignette = ((self.vignette * 10.0).round() / 10.0 + 0.1 * df).clamp(0.0, 1.0),
            Row::Grain => self.grain = ((self.grain * 10.0).round() / 10.0 + 0.1 * df).clamp(0.0, 1.0),
            Row::Frame => self.frame = (self.frame as i32 + d).rem_euclid(FRAMES.len() as i32) as usize,
            Row::Grid => self.thirds = !self.thirds,
            Row::Time => {
                self.hour = match self.hour {
                    None => Some(((self.game_hour * 4.0).round() / 4.0 + 0.25 * df).rem_euclid(24.0)),
                    Some(h) => {
                        let n = h + 0.25 * df;
                        // stepping back past the game's hour returns to it.
                        if (n - self.game_hour).abs() < 0.125 { None } else { Some(n.rem_euclid(24.0)) }
                    }
                }
            }
            Row::Capture => self.capture_2x = !self.capture_2x,
        }
    }

    /// Put the panel's selection on a row (captures pick one).
    pub fn select_row(&mut self, i: usize) {
        self.row = i.min(ROWS.len() - 1);
    }

    /// A line on screen for a few seconds.
    pub fn say(&mut self, s: String) {
        self.toast = Some((s, Instant::now()));
    }

    /// The panel's rows as eo-ui draws them.
    pub fn panel(&self, aspect: f64) -> PhotoPanel {
        let lin = |v: f64, a: f64, b: f64| Some(((v - a) / (b - a)) as f32);
        let log = |v: f64, a: f64, b: f64| Some(((v.ln() - a.ln()) / (b.ln() - a.ln())) as f32);
        let fov = rp::fov_y_deg(self.focal_mm, aspect);
        let stop = self.stop.map(|s| format!("f/{}", fmt_stop(STOPS[s]))).unwrap_or_else(|| "Off".into());
        let ev = self.ev_thirds as f64 / 3.0;
        let rows = vec![
            PhotoRow::new("Camera", if self.orbit { "Orbit".into() } else { "Free".into() }, None),
            PhotoRow::new("Lens", format!("{:.0} mm", self.focal_mm), log(self.focal_mm, LENS_MIN_MM, LENS_MAX_MM)).note(&format!("{fov:.1}\u{b0} vertical, 35 mm equivalent")),
            PhotoRow::new("Roll", format!("{:+.0}\u{b0}", self.roll_deg), lin(self.roll_deg, -45.0, 45.0)),
            PhotoRow::new("Focus", match self.focus { Focus::Subject => "Player", Focus::Centre => "Centre", Focus::Manual => "Manual" }.into(), None),
            PhotoRow::new("Distance", fmt_m(self.focus_m), log(self.focus_m, 0.3, 1000.0)),
            PhotoRow::new("Aperture", stop, self.stop.map(|s| s as f32 / (STOPS.len() - 1) as f32)),
            PhotoRow::new("Exposure", format!("{ev:+.1} EV"), lin(ev, -3.0, 3.0)),
            PhotoRow::new("Look", self.grade.label().into(), None),
            PhotoRow::new("Vignette", format!("{:.0} %", self.vignette * 100.0), Some(self.vignette as f32)),
            PhotoRow::new("Grain", format!("{:.0} %", self.grain * 100.0), Some(self.grain as f32)),
            PhotoRow::new("Frame", FRAMES[self.frame].1.into(), None),
            PhotoRow::new("Thirds", if self.thirds { "On".into() } else { "Off".into() }, None),
            PhotoRow::new("Time of day", match self.hour { None => "Game".into(), Some(h) => fmt_hour(h) }, self.hour.map(|h| (h / 24.0) as f32)).note("Local: only you see this"),
            PhotoRow::new("Capture", if self.capture_2x { "2\u{d7} size".into() } else { "Screen size".into() }, None),
        ];
        PhotoPanel {
            summary: format!("{:.0} mm \u{b7} {} \u{b7} {} \u{b7} {:+.1} EV", self.focal_mm, if self.stop.is_some() { format!("f/{}", fmt_stop(STOPS[self.stop.unwrap()])) } else { "all sharp".into() }, fmt_m(self.focus_now), ev),
            rows,
            selected: self.row,
            hints: [("Enter", "Capture"), ("Arrows", "Settings"), ("Wheel", "Lens"), ("Z X", "Roll"), ("O", "Orbit"), ("H", "Hide"), ("P", "Exit")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            toast: self.toast.as_ref().map(|t| t.0.clone()),
            thirds: self.thirds,
            frame_aspect: FRAMES[self.frame].0.map(|a| a as f32),
            visible: self.panel,
        }
    }

    /// The settings a saved photo records.
    pub fn describe(&self, cam: &Camera) -> serde_json::Value {
        json!({
            "eye": [cam.eye.x, cam.eye.y, cam.eye.z],
            "yaw_deg": (cam.yaw as f64).to_degrees(), "pitch_deg": (cam.pitch as f64).to_degrees(), "roll_deg": self.roll_deg,
            "focal_mm_35mm_equivalent": self.focal_mm, "fov_y_deg": cam.fov_y_deg,
            "f_number": self.stop.map(|s| STOPS[s]), "focus_m": self.focus_now, "focus": format!("{:?}", self.focus),
            "exposure_ev": self.ev_thirds as f64 / 3.0, "grade": self.grade.name(),
            "vignette": self.vignette, "grain": self.grain, "letterbox": FRAMES[self.frame].0,
            "local_hour": self.hour, "game_hour": self.game_hour,
        })
    }
}

fn fmt_stop(n: f64) -> String {
    if n < 10.0 && n.fract() != 0.0 { format!("{n:.1}") } else { format!("{n:.0}") }
}

fn fmt_m(m: f64) -> String {
    if m >= 999.0 {
        "\u{221e}".into()
    } else if m < 10.0 {
        format!("{m:.1} m")
    } else {
        format!("{m:.0} m")
    }
}

fn fmt_hour(h: f64) -> String {
    let m = (h * 60.0).round() as i64;
    format!("{:02}:{:02}", (m / 60) % 24, m % 60)
}

/// Move a ball from `from` by `d`, one axis at a time (it slides along a
/// wall it meets), stopping short of what it hits.
fn sweep(cw: Option<&CollisionWorld>, from: DVec3, d: DVec3) -> DVec3 {
    let Some(cw) = cw else { return from + d };
    let mut p = from;
    for axis in [DVec3::X, DVec3::Z, DVec3::Y] {
        let m = d.dot(axis);
        if m == 0.0 {
            continue;
        }
        let dir = axis * m.signum();
        let len = m.abs();
        match cw.sphere_cast(p, dir, len + SKIN_M, BALL_M, layer::WORLD | layer::STREET | layer::VEHICLE, None) {
            // already touching (toi 0): let it back out, never deeper.
            Some((t, _)) if t > 1e-4 => p += dir * (t - SKIN_M).clamp(0.0, len),
            Some(_) => {}
            None => p += dir * len,
        }
    }
    p
}

/// An offscreen colour target of the window's format (readable).
fn target(ctx: &GpuContext, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("photo capture"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Read a colour texture to tight RGBA8 rows (BGRA formats swapped).
pub fn read_rgba(ctx: &GpuContext, tex: &wgpu::Texture) -> Result<Vec<u8>> {
    let (w, h) = (tex.width(), tex.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("photo readback"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let bytes = read_mapped(ctx, &buf)?;
    let bgra = matches!(tex.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        for px in bytes[y * row as usize..y * row as usize + w as usize * 4].chunks_exact(4) {
            if bgra { out.extend_from_slice(&[px[2], px[1], px[0], 255]) } else { out.extend_from_slice(&[px[0], px[1], px[2], 255]) }
        }
    }
    Ok(out)
}

pub fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?);
    let mut e = png::Encoder::new(file, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// UTC date and time for a file name (no time-zone database: the name sorts and never collides).
fn stamp() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // civil from days (H. Hinnant, "chrono-Compatible Low-Level Date Algorithms").
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year:04}{month:02}{day:02}-{:02}{:02}{:02}-{:03}", rem / 3600, (rem / 60) % 60, rem % 60, d.subsec_millis())
}

/// Render the view offscreen (no interface) at `scale` x the window's size
/// and save it, with its settings beside it. Returns the PNG's path.
pub fn capture(ctx: &GpuContext, renderer: &Renderer, cam: &Camera, size: (u32, u32), format: wgpu::TextureFormat, scale: u32, settings: serde_json::Value) -> Result<PathBuf> {
    let dir = photos_dir().ok_or_else(|| anyhow::anyhow!("no folder for photos (set EO_PHOTOS_DIR)"))?;
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let (w, h) = (size.0 * scale, size.1 * scale);
    let lim = ctx.device.limits().max_texture_dimension_2d;
    if w > lim || h > lim {
        anyhow::bail!("{w} x {h} is over the device's {lim} texture limit");
    }
    let tex = target(ctx, w, h, format);
    let view = tex.create_view(&Default::default());
    // the renderer's own sample count (with MSAA the colour resolves into the capture).
    let targets = Targets::new(&ctx.device, w, h, format, renderer.sample_count);
    let (attach, resolve) = match &targets.msaa {
        Some(m) => (m, Some(&view)),
        None => (&view, None),
    };
    // the window's size is already converged: one frame; another size starts its resolve again.
    let frames = if scale == 1 { 1 } else { CAPTURE_2X_FRAMES };
    for _ in 0..frames {
        renderer.update(&ctx.queue, cam);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        renderer.encode(&mut enc, attach, resolve, &targets.depth, None);
        ctx.queue.submit([enc.finish()]);
        let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
    }
    let rgba = read_rgba(ctx, &tex)?;
    let name = format!("earth-online-{}", stamp());
    let png = dir.join(format!("{name}.png"));
    write_png(&png, w, h, &rgba)?;
    let meta = json!({ "file": png.file_name().map(|f| f.to_string_lossy().to_string()), "width": w, "height": h, "scale": scale, "photo": settings, "build": env!("EO_BUILD_COMMIT") });
    std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string_pretty(&meta)?)?;
    Ok(png)
}

/// The player's pivot for photo mode (the 30 m sphere's centre): the feet plus the camera rig's pivot height.
pub fn anchor_of(feet: DVec3) -> DVec3 {
    feet + DVec3::Y * eo_player::camera::on_foot::PIVOT_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_rows_step_and_clamp() {
        let mut m = PhotoMode::new(false, 12.0);
        m.focal_mm = 50.0;
        m.row = ROWS.iter().position(|r| *r == Row::Lens).unwrap();
        for _ in 0..200 {
            m.change(1);
        }
        assert_eq!(m.focal_mm, LENS_MAX_MM);
        for _ in 0..200 {
            m.change(-1);
        }
        assert_eq!(m.focal_mm, LENS_MIN_MM);
        m.row = ROWS.iter().position(|r| *r == Row::Aperture).unwrap();
        m.stop = Some(0);
        m.change(-1);
        assert_eq!(m.stop, None, "below f/1.4 the aperture turns off");
        m.change(1);
        assert_eq!(m.stop, Some(0));
        m.row = ROWS.iter().position(|r| *r == Row::Time).unwrap();
        m.change(1);
        assert_eq!(m.hour, Some(12.25));
        m.change(-1);
        assert_eq!(m.hour, None, "back to the game's hour");
    }

    #[test]
    fn third_stops_are_a_third_of_a_stop_apart() {
        for w in STOPS.windows(2) {
            let stops = 2.0 * (w[1] / w[0]).log2();
            assert!((stops - 1.0 / 3.0).abs() < 0.16, "{w:?}: {stops}");
        }
    }

    #[test]
    fn the_free_camera_flies_without_a_world() {
        let mut m = PhotoMode::new(false, 12.0);
        let cam = Camera::new(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0, 16.0 / 9.0);
        m.enter(&cam, DVec3::new(0.0, 1.43, 0.0));
        let held: HashSet<String> = ["KeyW".to_string()].into_iter().collect();
        for _ in 0..600 {
            m.step(&Feed { held: &held, mouse: [0.0, 0.0], dt: 1.0 / 60.0, aspect: 16.0 / 9.0, anchor: DVec3::new(0.0, 1.43, 0.0), cw: None });
        }
        // north (-z) at 3 m/s for 10 s, held to the 30 m sphere.
        assert!(m.eye.z < -29.0 && (m.eye - DVec3::new(0.0, 1.43, 0.0)).length() <= RADIUS_M + 1e-9, "{:?}", m.eye);
    }

    #[test]
    fn stamps_are_dates() {
        let s = stamp();
        assert_eq!(s.len(), "20261118-120000-000".len(), "{s}");
        assert!(s.starts_with("20"));
    }
}
