//! the sound (eo-audio) in the app. The windowed game plays on the
//! default output device; every bench and test uses the null output (the
//! mixer runs at real time, nothing reaches a speaker); `--no-audio` turns
//! it off; `--audio-null` forces the null output in a window.
//!
//! Once a frame on the render thread: the listener (the camera), the player
//! (`--play`), the simulated cars and walkers near the listener (from the
//! simulation's newest frame), the weather and hour (the renderer's sky), the
//! kind of place (a 64 m grid built once on a thread from the buildings and
//! the land cover) and the interface's cues go to the director, which sends
//! commands to the mixer's thread. The surface under the player's feet is
//! looked up on a worker thread (`eo-audio-surface`).
//!
//!   eo-app --audio-render walk|drive|rain|all [--out DIR] [--seconds 30]
//!
//! renders 30 s mixes through the same mixer offline from scripted runs (no
//! GPU, no window, no device): walking a busy Market Street sidewalk at noon,
//! driving down Mission Street, and a rainy night on Valencia Street.

use crate::World;
use anyhow::{Result, anyhow};
use eo_audio::city::{self, CityInput, Director, Menu, Place, PlayerAudio, Surface, Weather};
use eo_audio::{Engine, Listener, Output};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

pub const PLACE_CELL_M: f32 = 64.0;
/// Cars handed to the director (it voices the nearest `eo_audio::city::CAR_VOICES`).
const CAR_RADIUS_M: f32 = 140.0;
const WALKER_RADIUS_M: f32 = 25.0;

/// The kind of place around a point, gridded once.
pub struct PlaceGrid {
    x0: f32,
    z0: f32,
    w: usize,
    h: usize,
    density: Vec<f32>,
    water: Vec<f32>,
    green: Vec<f32>,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl PlaceGrid {
    /// Density: the cell's built volume over its area (an average building height,
    /// m; 2.5 m reads 0, 16 m reads 1). Water: within 400 m of a water cell
    /// (ESA WorldCover class 80). Green: the share of tree, shrub and grass.
    pub fn build(city: &eo_data::City, land: Option<&eo_data::LandCover>) -> PlaceGrid {
        let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for b in &city.buildings {
            for p in &b.ring {
                x0 = x0.min(p[0]);
                z0 = z0.min(p[1]);
                x1 = x1.max(p[0]);
                z1 = z1.max(p[1]);
            }
        }
        let pad = 1_500.0;
        let (x0, z0) = (x0 - pad, z0 - pad);
        let w = (((x1 + pad) - x0) / PLACE_CELL_M).ceil() as usize + 1;
        let h = (((z1 + pad) - z0) / PLACE_CELL_M).ceil() as usize + 1;
        let mut built = vec![0f32; w * h];
        for b in &city.buildings {
            let n = b.ring.len();
            if n < 3 {
                continue;
            }
            let (mut a, mut cx, mut cz) = (0f32, 0f32, 0f32);
            for i in 0..n {
                let (p, q) = (b.ring[i], b.ring[(i + 1) % n]);
                a += p[0] * q[1] - q[0] * p[1];
                cx += p[0];
                cz += p[1];
            }
            let (cx, cz) = (cx / n as f32, cz / n as f32);
            let (i, j) = (((cx - x0) / PLACE_CELL_M) as usize, ((cz - z0) / PLACE_CELL_M) as usize);
            if i < w && j < h {
                built[j * w + i] += (a * 0.5).abs() * b.height;
            }
        }
        let blur = |v: &[f32]| -> Vec<f32> {
            let mut o = vec![0f32; w * h];
            for j in 0..h {
                for i in 0..w {
                    let (mut s, mut k) = (0f32, 0f32);
                    for dj in -1i64..=1 {
                        for di in -1i64..=1 {
                            let (ii, jj) = (i as i64 + di, j as i64 + dj);
                            if ii >= 0 && jj >= 0 && (ii as usize) < w && (jj as usize) < h {
                                s += v[jj as usize * w + ii as usize];
                                k += 1.0;
                            }
                        }
                    }
                    o[j * w + i] = s / k;
                }
            }
            o
        };
        let area = PLACE_CELL_M * PLACE_CELL_M;
        let density: Vec<f32> = blur(&built).into_iter().map(|v| smooth(2.5, 16.0, v / area)).collect();
        let mut wet = vec![0f32; w * h];
        let mut green = vec![0f32; w * h];
        if let Some(l) = land {
            for j in 0..h {
                for i in 0..w {
                    let (mut nw, mut ng) = (0, 0);
                    for sj in 0..4 {
                        for si in 0..4 {
                            let x = x0 + (i as f32 + (si as f32 + 0.5) / 4.0) * PLACE_CELL_M;
                            let z = z0 + (j as f32 + (sj as f32 + 0.5) / 4.0) * PLACE_CELL_M;
                            match l.class_at(x as f64, z as f64) {
                                80 => nw += 1,
                                10 | 20 | 30 => ng += 1,
                                _ => {}
                            }
                        }
                    }
                    wet[j * w + i] = nw as f32 / 16.0;
                    green[j * w + i] = ng as f32 / 16.0;
                }
            }
        }
        // Distance (cells) to the nearest mostly-water cell: two chamfer passes.
        let inf = 1e9f32;
        let mut d: Vec<f32> = wet.iter().map(|&v| if v >= 0.5 { 0.0 } else { inf }).collect();
        let s2 = std::f32::consts::SQRT_2;
        for j in 0..h {
            for i in 0..w {
                let mut v = d[j * w + i];
                if i > 0 {
                    v = v.min(d[j * w + i - 1] + 1.0);
                }
                if j > 0 {
                    v = v.min(d[(j - 1) * w + i] + 1.0);
                    if i > 0 {
                        v = v.min(d[(j - 1) * w + i - 1] + s2);
                    }
                    if i + 1 < w {
                        v = v.min(d[(j - 1) * w + i + 1] + s2);
                    }
                }
                d[j * w + i] = v;
            }
        }
        for j in (0..h).rev() {
            for i in (0..w).rev() {
                let mut v = d[j * w + i];
                if i + 1 < w {
                    v = v.min(d[j * w + i + 1] + 1.0);
                }
                if j + 1 < h {
                    v = v.min(d[(j + 1) * w + i] + 1.0);
                    if i + 1 < w {
                        v = v.min(d[(j + 1) * w + i + 1] + s2);
                    }
                    if i > 0 {
                        v = v.min(d[(j + 1) * w + i - 1] + s2);
                    }
                }
                d[j * w + i] = v;
            }
        }
        let water = d.iter().map(|&c| 1.0 - smooth(0.0, 400.0, c * PLACE_CELL_M)).collect();
        PlaceGrid { x0, z0, w, h, density, water, green: blur(&green) }
    }

    pub fn at(&self, x: f32, z: f32) -> Place {
        let fx = ((x - self.x0) / PLACE_CELL_M - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fz = ((z - self.z0) / PLACE_CELL_M - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (i, j) = (fx.floor() as usize, fz.floor() as usize);
        let (i1, j1) = ((i + 1).min(self.w - 1), (j + 1).min(self.h - 1));
        let (u, v) = (fx - i as f32, fz - j as f32);
        let s = |g: &[f32]| {
            let a = g[j * self.w + i] * (1.0 - u) + g[j * self.w + i1] * u;
            let b = g[j1 * self.w + i] * (1.0 - u) + g[j1 * self.w + i1] * u;
            a * (1.0 - v) + b * v
        };
        Place { density: s(&self.density), water: s(&self.water), green: s(&self.green) }
    }
}

/// What the ground under (x, z) is to a shoe.
pub fn surface_of(street: Option<&eo_street::StreetWorld>, land: Option<&eo_data::LandCover>, x: f64, z: f64) -> Surface {
    use eo_street::Layer;
    match street.and_then(|s| s.paved_layer_at(x, z)) {
        Some(Layer::Carriageway) => Surface::Asphalt,
        Some(Layer::Verge) => Surface::Gravel,
        Some(_) => Surface::Concrete,
        None => match land.map(|l| l.class_at(x, z)) {
            Some(10 | 20 | 30 | 60 | 100) => Surface::Gravel,
            _ => Surface::Concrete,
        },
    }
}

/// The surface lookup off the render thread (a street tile's surface may need building).
struct SurfaceProbe {
    tx: Sender<[f64; 2]>,
    rx: Receiver<Surface>,
    last: Surface,
    asked: f64,
}

fn spawn_probe(street: Option<Arc<eo_street::StreetWorld>>, paths: eo_data::DataPaths) -> SurfaceProbe {
    let (tx, req) = std::sync::mpsc::channel::<[f64; 2]>();
    let (out, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("eo-audio-surface".into())
        .spawn(move || {
            eo_render::hitch::worker_started();
            let land = eo_data::LandCover::load(&paths).ok();
            while let Ok(mut p) = req.recv() {
                while let Ok(q) = req.try_recv() {
                    p = q;
                }
                if out.send(surface_of(street.as_deref(), land.as_ref(), p[0], p[1])).is_err() {
                    break;
                }
            }
        })
        .expect("spawn audio surface probe");
    SurfaceProbe { tx, rx, last: Surface::Concrete, asked: -1.0 }
}

/// The weather and the hour as the director reads them.
pub fn weather_of(w: &eo_sky::WeatherState) -> Weather {
    Weather {
        rain_mm_h: w.rain_mm_h as f32,
        wind_mps: w.wind[0].hypot(w.wind[1]) as f32,
        wetness: w.wetness as f32,
        flash: w.flash as f32,
        flash_m: w.flash_distance_m as f32,
        fog: if w.kind == eo_sky::Kind::Fog { 1.0 } else { 0.0 },
    }
}

/// The simulated cars within reach of `eye` and the walkers near it.
pub fn cars_near<'a>(bufs: impl Iterator<Item = &'a eo_sim::output::AgentBuffer>, eye: [f32; 3], elevation: &eo_data::ElevationGrid, out: &mut Vec<city::Car>) -> f32 {
    out.clear();
    let mut walkers = 0.0;
    let r2 = CAR_RADIUS_M * CAR_RADIUS_M;
    for b in bufs {
        for v in &b.vehicles {
            let (x, z) = ((b.origin[0] + v.pos[0] as f64) as f32, (b.origin[1] + v.pos[2] as f64) as f32);
            let (dx, dz) = (x - eye[0], z - eye[2]);
            if dx * dx + dz * dz > r2 {
                continue;
            }
            let y = elevation.ground(x as f64, z as f64) as f32 + 0.6;
            let (s, c) = v.heading.sin_cos();
            out.push(city::Car { id: v.id, pos: [x, y, z], vel: [s * v.speed, 0.0, c * v.speed], speed: v.speed, kind: v.kind as u8, stopped: v.flags & eo_sim::output::FLAG_STOPPED != 0 });
        }
        for p in &b.walkers {
            let (dx, dz) = ((b.origin[0] + p.pos[0] as f64) as f32 - eye[0], (b.origin[1] + p.pos[2] as f64) as f32 - eye[2]);
            if dx * dx + dz * dz < WALKER_RADIUS_M * WALKER_RADIUS_M {
                walkers += 1.0;
            }
        }
    }
    walkers
}

/// The player as the director reads it.
pub fn player_audio(p: &eo_player::Player, cw: &eo_player::world::CollisionWorld, surface: Surface) -> PlayerAudio {
    use eo_player::Mode;
    match p.mode {
        Mode::OnFoot => {
            let f = p.character.feet();
            PlayerAudio::OnFoot(city::Foot { feet: [f.x as f32, f.y as f32, f.z as f32], speed: p.ground_speed as f32, grounded: p.character.grounded, crouched: p.character.crouched, surface })
        }
        Mode::Driving(i) => {
            let v = &p.vehicles[i];
            let st = v.state(cw);
            let yaw = v.yaw(cw);
            let right = [-yaw.cos() as f32, 0.0, yaw.sin() as f32];
            let vel = st.linvel;
            let d = v.datum(cw);
            PlayerAudio::Driving(city::Drive {
                pos: [d.x as f32, d.y as f32 + 0.5, d.z as f32],
                vel,
                speed: v.forward_speed(cw) as f32,
                throttle: v.controls.throttle as f32,
                brake: v.controls.brake as f32,
                handbrake: v.controls.handbrake,
                lateral: vel[0] * right[0] + vel[2] * right[2],
            })
        }
    }
}

/// One frame's inputs.
pub struct FrameIn<'a> {
    pub dt: f64,
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    pub player: Option<(&'a eo_player::Player, &'a eo_player::world::CollisionWorld)>,
    pub agents: Option<&'a mut crate::agents::AgentRuntime>,
    pub renderer: Option<&'a eo_render::Renderer>,
    pub elevation: &'a eo_data::ElevationGrid,
    pub ui: u32,
    pub menu: Menu,
}

impl<'a> FrameIn<'a> {
    /// The free camera with the simulation (the fly-through and the bench).
    pub fn camera(cam: &eo_render::camera::Camera, dt: f64, agents: &'a mut crate::agents::AgentRuntime, renderer: &'a eo_render::Renderer, elevation: &'a eo_data::ElevationGrid) -> Self {
        FrameIn { dt, eye: cam.eye.to_array(), forward: cam.forward().to_array(), player: None, agents: Some(agents), renderer: Some(renderer), elevation, ui: 0, menu: Menu::None }
    }

    /// `--play`: the player's camera, the player, the simulation.
    #[allow(clippy::too_many_arguments)]
    pub fn player(view: &eo_player::CameraView, dt: f64, player: &'a eo_player::Player, cw: &'a eo_player::world::CollisionWorld, agents: &'a mut crate::agents::AgentRuntime, renderer: &'a eo_render::Renderer, elevation: &'a eo_data::ElevationGrid) -> Self {
        let f = view.forward.normalize_or(glam::DVec3::Z);
        FrameIn { dt, eye: [view.eye.x as f32, view.eye.y as f32, view.eye.z as f32], forward: [f.x as f32, f.y as f32, f.z as f32], player: Some((player, cw)), agents: Some(agents), renderer: Some(renderer), elevation, ui: 0, menu: Menu::None }
    }

    /// `--ui`: the interface over the fly camera (no simulation there).
    pub fn interface(cam: &eo_render::camera::Camera, dt: f64, renderer: &'a eo_render::Renderer, elevation: &'a eo_data::ElevationGrid, screen: eo_ui::Screen, ui: u32) -> Self {
        let menu = match screen {
            eo_ui::Screen::Hud => Menu::None,
            eo_ui::Screen::Pause => Menu::Paused,
            eo_ui::Screen::Title | eo_ui::Screen::Loading => Menu::Title,
        };
        FrameIn { dt, eye: cam.eye.to_array(), forward: cam.forward().to_array(), player: None, agents: None, renderer: Some(renderer), elevation, ui, menu }
    }
}

pub struct AudioRuntime {
    pub engine: Engine,
    pub director: Director,
    place: Option<PlaceGrid>,
    place_rx: Option<Receiver<PlaceGrid>>,
    probe: Option<SurfaceProbe>,
    cars: Vec<city::Car>,
    pub volume: f32,
    time: f64,
    /// Render-thread time of each frame, ms (the bench reports it per path).
    pub cost_ms: Vec<f64>,
    /// The same, on-CPU only (the thread's cycles: a preemption is not the audio's cost).
    pub cpu_ms: Vec<f64>,
    pub commands: Vec<f64>,
}

impl AudioRuntime {
    /// `default`: `Output::Device` for the windowed game, `Output::Null` for benches
    /// (`--no-audio` and `--audio-null` on the command line override it; a bench is
    /// never given the device).
    pub fn start(world: &World, default: Output) -> AudioRuntime {
        let output = Output::from_args(default);
        // The audio's worker threads lower themselves as the streaming workers do (the device callback is left alone).
        let _ = eo_audio::WORKER_STARTED.set(eo_render::hitch::worker_started);
        let root = eo_audio::bank::Bank::root_from_env();
        if output != Output::Off && root.is_none() {
            eprintln!("audio: no sound bank (EO_AUDIO_ROOT not set or holds no copy; silent)");
        }
        let engine = Engine::start(output, root);
        let (tx, rx) = std::sync::mpsc::channel();
        if output != Output::Off {
            let (city, paths) = (world.city.clone(), world.paths.clone());
            std::thread::Builder::new()
                .name("eo-audio-place".into())
                .spawn(move || {
                    eo_render::hitch::worker_started();
                    let land = eo_data::LandCover::load(&paths).ok();
                    let _ = tx.send(PlaceGrid::build(&city, land.as_ref()));
                })
                .expect("spawn audio place grid");
        }
        let probe = (output != Output::Off).then(|| spawn_probe(world.street.as_ref().map(|s| s.world.clone()), world.paths.clone()));
        let settings = eo_ui::settings::default_path().map(|p| eo_ui::settings::Settings::load(&p)).unwrap_or_default();
        eprintln!("audio: output {}", output.name());
        AudioRuntime {
            engine,
            director: Director::new(0x5eed_a0d1),
            place: None,
            place_rx: Some(rx),
            probe,
            cars: vec![],
            volume: (settings.audio_volume * settings.audio_mix.sfx) as f32,
            time: 0.0,
            cost_ms: vec![],
            cpu_ms: vec![],
            commands: vec![],
        }
    }

    /// The interface's volume (master x the sound-effects mix).
    pub fn set_volume(&mut self, s: &eo_ui::settings::Settings) {
        self.volume = (s.audio_volume * s.audio_mix.sfx) as f32;
    }

    /// Once a frame on the render thread.
    pub fn frame(&mut self, f: FrameIn) {
        if !self.engine.on() {
            return;
        }
        let t0 = Instant::now();
        let c0 = eo_render::hitch::thread_cycles();
        self.time += f.dt;
        if let Some(rx) = &self.place_rx {
            if let Ok(g) = rx.try_recv() {
                self.place = Some(g);
                self.place_rx = None;
            }
        }
        let walkers = match f.agents {
            Some(a) => match a.sim.as_mut() {
                Some(sim) => {
                    let fr = sim.latest();
                    cars_near(fr.regions.iter(), f.eye, f.elevation, &mut self.cars)
                }
                None => {
                    self.cars.clear();
                    0.0
                }
            },
            None => {
                self.cars.clear();
                0.0
            }
        };
        let surface = match (&mut self.probe, f.player) {
            (Some(p), Some((pl, _))) => {
                while let Ok(s) = p.rx.try_recv() {
                    p.last = s;
                }
                if self.time - p.asked > 0.2 {
                    p.asked = self.time;
                    let ft = pl.character.feet();
                    let _ = p.tx.send([ft.x, ft.z]);
                }
                p.last
            }
            _ => Surface::Concrete,
        };
        let (weather, hour) = match f.renderer {
            Some(r) => (weather_of(&r.look.sky.weather()), r.look.time_of_day() as f32),
            None => (Weather::default(), 12.0),
        };
        let place = self.place.as_ref().map(|g| g.at(f.eye[0], f.eye[2])).unwrap_or(Place { density: 0.6, water: 0.0, green: 0.0 });
        let player = f.player.map(|(p, cw)| player_audio(p, cw, surface)).unwrap_or_default();
        let input = CityInput {
            dt: f.dt as f32,
            listener: Listener { pos: f.eye, forward: f.forward, up: [0.0, 1.0, 0.0], vel: [0.0; 3] },
            player,
            cars: &self.cars,
            walkers_near: walkers,
            weather,
            hour,
            place,
            ui: f.ui,
            menu: f.menu,
            volume: self.volume,
        };
        let n = self.director.frame(&mut self.engine, &input);
        self.cost_ms.push(t0.elapsed().as_secs_f64() * 1e3);
        self.cpu_ms.push(eo_render::hitch::on_cpu_ms(c0, eo_render::hitch::thread_cycles()));
        self.commands.push(n as f64);
    }

    /// One bench path's numbers (frames after `warmup`), then start the next path's.
    pub fn segment_report(&mut self, name: &str, warmup: usize) {
        if !self.engine.on() {
            self.cost_ms.clear();
            return;
        }
        let v: Vec<f64> = self.cost_ms.iter().skip(warmup).copied().collect();
        let cpu = on_cpu_summary(self.cpu_ms.iter().skip(warmup).copied());
        let c: Vec<f64> = self.commands.iter().skip(warmup).copied().collect();
        let st = &self.engine.stats;
        println!(
            "    audio ({}): render-thread cost per frame median {:.4} / p95 {:.4} / max {:.4} ms wall, on-CPU {} (budget 0.1 ms); commands/frame median {:.0}; car voices {}, voices mixed now {} (max {}), sources {} (max {}), mixer load {:.2} % of a core, underruns {}, refused {}",
            self.engine.output.name(),
            pct(&v, 50.0),
            pct(&v, 95.0),
            pct(&v, 100.0),
            cpu,
            pct(&c, 50.0),
            self.director.stats.car_voices,
            st.mixed.load(Ordering::Relaxed),
            st.max_mixed.load(Ordering::Relaxed),
            st.active.load(Ordering::Relaxed),
            st.max_active.load(Ordering::Relaxed),
            st.load() * 100.0,
            st.underruns.load(Ordering::Relaxed),
            st.refused_plays.load(Ordering::Relaxed),
        );
        let _ = name;
        self.cost_ms.clear();
        self.cpu_ms.clear();
        self.commands.clear();
    }

    pub fn final_report(&self) {
        if !self.engine.on() {
            println!("audio: off");
            return;
        }
        let (bank, fixed) = self.engine.memory_bytes();
        let b = self.engine.bank();
        println!(
            "audio: {} output at {} Hz; bank {} clips ({} streamed), decoded {:.1} MB + mixer/streams {:.1} MB; bank load {:.0} ms, {} refused clips",
            self.engine.output.name(),
            self.engine.rate.load(Ordering::Relaxed),
            b.map(|b| b.clips.len()).unwrap_or(0),
            b.map(|b| b.streamed).unwrap_or(0),
            bank as f64 / 1e6,
            fixed as f64 / 1e6,
            b.map(|b| b.load_ms).unwrap_or(0.0),
            b.map(|b| b.errors.len()).unwrap_or(0),
        );
    }
}

// The per-thread clock is optional. Keep all wall times and the warmup
// boundary even if this platform cannot supply an on-CPU sample.
fn on_cpu_summary(samples: impl Iterator<Item = f64>) -> String {
    let values: Vec<_> = samples.filter(|v| v.is_finite()).collect();
    if values.is_empty() {
        return "unavailable".to_owned();
    }
    format!("{:.4} / {:.4} / {:.4} ms", pct(&values, 50.0), pct(&values, 95.0), pct(&values, 100.0))
}

#[cfg(test)]
mod timing_tests {
    use super::on_cpu_summary;

    #[test]
    fn optional_cpu_counter_keeps_missing_readings_out_of_percentiles() {
        assert_eq!(on_cpu_summary([f64::NAN; 2].into_iter()), "unavailable");
        assert_eq!(on_cpu_summary([0.0, f64::NAN, 0.5, 1.0].into_iter()), "0.5000 / 1.0000 / 1.0000 ms");
    }
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((p / 100.0) * (s.len() - 1) as f64).round() as usize]
}

// ---------------------------------------------------------------- offline mixes

/// main.rs's hook: None unless `--audio-render` is on the command line.
pub fn intercept(process_start: Instant) -> Option<Result<()>> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let k = raw.iter().position(|a| a == "--audio-render")?;
    Some(render_main(&raw, k, process_start))
}

struct Scene {
    name: &'static str,
    street: &'static str,
    /// Where on the street (lat, lon), and how far before it the run starts, m.
    at: (f64, f64),
    lead_m: f64,
    drive: bool,
    sidewalk: bool,
    hour: f32,
    weather: eo_sky::Kind,
    density: u32,
}

/// The three asked for, then two that exercise the San Francisco set: the Embarcadero in
/// morning fog (foghorns, gulls, the water's edge) and Powell Street (the cable in its slot).
const SCENES: [Scene; 5] = [
    Scene { name: "walk_noon_market_st", street: "Market Street", at: (37.7853, -122.4056), lead_m: 25.0, drive: false, sidewalk: true, hour: 12.0, weather: eo_sky::Kind::Clear, density: 3 },
    Scene { name: "drive_downtown_mission_st", street: "Mission Street", at: (37.7840, -122.4010), lead_m: 220.0, drive: true, sidewalk: false, hour: 13.0, weather: eo_sky::Kind::Clear, density: 3 },
    Scene { name: "rain_night_valencia_st", street: "Valencia Street", at: (37.7650, -122.4219), lead_m: 25.0, drive: false, sidewalk: true, hour: 22.5, weather: eo_sky::Kind::Rain, density: 1 },
    Scene { name: "fog_morning_embarcadero", street: "The Embarcadero", at: (37.7955, -122.3937), lead_m: 25.0, drive: false, sidewalk: true, hour: 8.0, weather: eo_sky::Kind::Fog, density: 1 },
    Scene { name: "cable_powell_st", street: "Powell Street", at: (37.7880, -122.4080), lead_m: 25.0, drive: false, sidewalk: true, hour: 15.0, weather: eo_sky::Kind::Clear, density: 2 },
];

fn project(lat: f64, lon: f64) -> [f64; 2] {
    let m_lon = 111_320.0 * 37.778f64.to_radians().cos();
    [(lon + 122.4194) * m_lon, -(lat - 37.778) * 110_574.0]
}

/// The run's path: `before` m behind the anchor to `after` m past it along the
/// street's centreline, moved onto the right-hand sidewalk when asked.
fn scene_path(city: &eo_data::City, street: &eo_street::StreetWorld, s: &Scene, after: f64) -> Result<Vec<[f64; 2]>> {
    let line = eo_player::routes::street_polyline(city, s.street, 4.0);
    if line.len() < 4 {
        anyhow::bail!("no centreline for {}", s.street);
    }
    let a = project(s.at.0, s.at.1);
    let k = (0..line.len()).min_by(|&i, &j| ((line[i][0] - a[0]).hypot(line[i][1] - a[1])).partial_cmp(&(line[j][0] - a[0]).hypot(line[j][1] - a[1])).unwrap()).unwrap();
    let step = |i: usize, j: usize| (line[i][0] - line[j][0]).hypot(line[i][1] - line[j][1]);
    let (mut i0, mut d) = (k, 0.0);
    while i0 > 0 && d < s.lead_m {
        d += step(i0, i0 - 1);
        i0 -= 1;
    }
    let (mut i1, mut d) = (k, 0.0);
    while i1 + 1 < line.len() && d < after {
        d += step(i1, i1 + 1);
        i1 += 1;
    }
    let mut path: Vec<[f64; 2]> = line[i0..=i1].to_vec();
    if s.sidewalk {
        use eo_street::Layer;
        let n = path.len();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let (p, q) = (path[i.saturating_sub(1)], path[(i + 1).min(n - 1)]);
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let l = dx.hypot(dz).max(1e-6);
            let r = [-dz / l, dx / l];
            // The outermost stretch of sidewalk to the side (the one along the buildings,
            // not a boarding island in the roadway), and its middle.
            let (mut span, mut cur): (Option<(f64, f64)>, Option<(f64, f64)>) = (None, None);
            let mut o = 2.0;
            while o < 45.0 {
                let at = [path[i][0] + r[0] * o, path[i][1] + r[1] * o];
                let is = matches!(street.paved_layer_at(at[0], at[1]), Some(Layer::Sidewalk));
                cur = match (is, cur) {
                    (true, None) => Some((o, o)),
                    (true, Some((a, _))) => Some((a, o)),
                    (false, Some(c)) => {
                        if c.1 - c.0 >= 1.5 {
                            span = Some(c);
                        }
                        None
                    }
                    (false, None) => None,
                };
                if street.buildings.contains(at[0], at[1]) {
                    break;
                }
                o += 0.5;
            }
            if let Some(c) = cur.filter(|c| c.1 - c.0 >= 1.5) {
                span = Some(c);
            }
            // 2 m in from the building side: clear of the tree wells and poles along the kerb.
            let off = span.map(|(a, b)| (b - 2.0).max(0.5 * (a + b))).unwrap_or(12.0);
            if std::env::var("EO_AUDIO_RENDER_TRACE").is_ok() && i % 8 == 0 {
                let probe: String = (0..22).map(|k| match street.paved_layer_at(path[i][0] + r[0] * k as f64 * 2.0, path[i][1] + r[1] * k as f64 * 2.0) { Some(Layer::Sidewalk) => 'S', Some(Layer::Carriageway) => 'c', Some(Layer::CurbTop) => 'k', Some(Layer::Ramp) => 'r', Some(Layer::Verge) => 'v', Some(_) => '?', None => '.' }).collect();
                eprintln!("  offset {i}: {probe} -> {off:.1} m (span {span:?})");
            }
            out.push((off, r));
        }
        // Hold the offset steady (the median of its neighbours) so the walk does not zig-zag.
        let offs: Vec<f64> = (0..n)
            .map(|i| {
                let mut w: Vec<f64> = out[i.saturating_sub(3)..(i + 4).min(n)].iter().map(|o| o.0).collect();
                w.sort_by(f64::total_cmp);
                w[w.len() / 2]
            })
            .collect();
        path = (0..n).map(|i| [path[i][0] + out[i].1[0] * offs[i], path[i][1] + out[i].1[1] * offs[i]]).collect();
        if std::env::var("EO_AUDIO_RENDER_TRACE").is_ok() {
            for (i, p) in path.iter().enumerate().take(12) {
                // Sample the leg to the next point.
                let q = path[(i + 1).min(n - 1)];
                let leg: String = (0..10).map(|k| { let t = k as f64 / 10.0; let (x, z) = (p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t); if street.buildings.contains(x, z) { 'B' } else { match street.paved_layer_at(x, z) { Some(Layer::Sidewalk) => 'S', Some(Layer::Carriageway) => 'c', Some(Layer::CurbTop) => 'k', Some(Layer::Ramp) => 'r', Some(_) => '?', None => '.' } } }).collect();
                eprintln!("  wp {i} ({:.1}, {:.1}) off {:.1}: {leg}", p[0], p[1], offs[i]);
            }
        }
    }
    Ok(path)
}

fn render_main(raw: &[String], k: usize, process_start: Instant) -> Result<()> {
    let which = raw.get(k + 1).cloned().unwrap_or_else(|| "all".into());
    let mut out_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out/audio");
    let mut seconds = 30.0f64;
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out_dir = it.next().ok_or_else(|| anyhow!("--out needs a value"))?.into(),
            "--seconds" => seconds = it.next().ok_or_else(|| anyhow!("--seconds needs a value"))?.parse()?,
            _ => {}
        }
    }
    std::fs::create_dir_all(&out_dir)?;
    let paths = eo_data::DataPaths::discover()?;
    let city = Arc::new(eo_data::City::load(&paths)?);
    let elevation = Arc::new(eo_data::ElevationGrid::load(&paths)?);
    let street: Arc<eo_street::StreetWorld> = crate::streets::spawn(crate::streets::spawn_plan(), &city, &elevation).join().map_err(|_| anyhow!("street thread"))??;
    let land = eo_data::LandCover::load(&paths).ok();
    let grid = PlaceGrid::build(&city, land.as_ref());
    let root = eo_audio::bank::Bank::root_from_env().ok_or_else(|| anyhow!("no sound bank (EO_AUDIO_ROOT not set or holds no copy)"))?;
    let bank = Arc::new(eo_audio::bank::Bank::load(&eo_audio::bank::Manifest::embedded()?, &root));
    if !bank.errors.is_empty() {
        anyhow::bail!("{} clips refused: {:?}", bank.errors.len(), &bank.errors[..bank.errors.len().min(3)]);
    }
    eprintln!("audio-render: data at {:.0} ms; bank {} clips ({} streamed), {:.1} MB decoded, loaded in {:.0} ms", process_start.elapsed().as_secs_f64() * 1e3, bank.clips.len(), bank.streamed, bank.memory_bytes as f64 / 1e6, bank.load_ms);
    let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
    let buildings = Arc::new(eo_player::world::BuildingSet::from_city(&city, lift));
    let source = Arc::new(eo_sim::native::load(&paths)?);
    for s in SCENES.iter().filter(|s| which == "all" || which.split(',').any(|w| s.name.starts_with(w))) {
        let rate = eo_audio::NULL_RATE;
        let (mut engine, mut off) = Engine::offline(bank.clone(), rate);
        let mut director = Director::new(0x5eed_0000 + s.name.len() as u64);
        let pre_s = 3.0;
        let speed = if s.drive { 12.0 } else { 1.4 };
        let path = scene_path(&city, &street, s, s.lead_m + speed * (seconds + pre_s) + 60.0)?;
        // the offline mixes walk and drive the street surface, without grade-separated roads.
        let ground: Arc<dyn eo_player::world::GroundProvider> = Arc::new(eo_player::street_ground::StreetGround::new(street.clone(), None));
        let mut cw = eo_player::world::CollisionWorld::new(ground, buildings.clone(), 2);
        let mut start = glam::DVec3::new(path[0][0], elevation.ground(path[0][0], path[0][1]), path[0][1]);
        cw.load_blocking(start, eo_player::world::LOAD_RADIUS_M);
        start.y = cw.surface_y(start.x, start.z);
        let yaw0 = (path[1][0] - path[0][0]).atan2(path[1][1] - path[0][1]);
        let mut player = eo_player::Player::new(start, yaw0);
        if s.drive {
            player.spawn_vehicle(&mut cw, path[0][0], path[0][1], yaw0);
            let side = start + glam::DVec3::new(yaw0.cos() * 3.0, 0.0, -yaw0.sin() * 3.0);
            player.character.teleport_feet(glam::DVec3::new(side.x, cw.surface_y(side.x, side.z), side.z));
            for _ in 0..30 {
                player.fixed_step(&mut cw, &eo_player::StepInput::default());
            }
            player.fixed_step(&mut cw, &eo_player::StepInput { vehicle_toggle: true, ..Default::default() });
        }
        let mut walk = eo_player::routes::WalkPilot::new(path.clone(), false);
        let mut drive = eo_player::routes::DrivePilot::new(path.clone(), speed);
        // The simulation around the start, as the game's agents runtime builds it.
        let mut world = {
            let tiles = eo_sim::native::tiles_around(start.x, start.z, crate::agents::SIM_RADIUS_M);
            let mut net = eo_sim::Network::new(source.timing.clone());
            net.add(source.build(&tiles));
            let mut w = eo_sim::World::new(net);
            let mut cfg = eo_sim::region::RegionConfig::game_default(crate::agents::SEED_INT);
            cfg.vehicle_pool *= s.density as usize;
            if let eo_sim::region::People::Bubble(b) = &mut cfg.people {
                b.max_bodies *= s.density as usize;
            }
            w.regions.push(eo_sim::region::Region::new(&w.net, 0, cfg, eo_sim::region::Focus { x: start.x, z: start.z, yaw: yaw0 }));
            for _ in 0..600 {
                w.step();
            }
            w
        };
        let weather = weather_of(&eo_sky::weather::preset(s.weather));
        let dt = 1.0 / 60.0;
        let frames_total = ((seconds + pre_s) / dt) as usize;
        let per = (rate as f64 * dt) as usize;
        let mut mix: Vec<f32> = Vec::with_capacity((seconds * rate as f64) as usize * 2 + 4096);
        let mut cars = vec![];
        let mut last_stream = [start.x, start.z];
        let (mut cost, mut cpu, mut cmds, mut mixed_max, mut steps0) = (vec![], vec![], vec![], 0u32, 0u64);
        let t_run = Instant::now();
        let trace = std::env::var("EO_AUDIO_RENDER_TRACE").is_ok();
        let (mut stuck_s, mut nudges) = (0.0f64, 0u32);
        if trace {
            eprintln!("  path: {} points, {:?} .. {:?}", path.len(), path.first(), path.last());
        }
        for i in 0..frames_total {
            let input = if s.drive { drive.input(&player, &cw) } else { walk.input(&mut player, &cw) };
            player.advance(&mut cw, &input, dt);
            let pos = player.position(&cw);
            cw.pump(pos);
            // A scripted walker held by an obstacle (a tree well, a pole base) for a second is
            // moved 1.5 m on toward its next waypoint; the run is for its sound, not its path.
            if !s.drive {
                stuck_s = if player.ground_speed < 0.2 { stuck_s + dt } else { 0.0 };
                if stuck_s > 1.0 {
                    stuck_s = 0.0;
                    nudges += 1;
                    let f = player.character.feet();
                    let done = (f.x - path[0][0]).hypot(f.z - path[0][1]);
                    let tgt = path.iter().map(|q| glam::DVec3::new(q[0], f.y, q[1])).find(|q| (q.x - f.x).hypot(q.z - f.z) > 2.0 && (q.x - path[0][0]).hypot(q.z - path[0][1]) > done).unwrap_or(f);
                    let to = f + (tgt - f).normalize_or_zero() * 1.5;
                    player.character.teleport_feet(glam::DVec3::new(to.x, cw.surface_y(to.x, to.z), to.z));
                }
            }
            if trace && i % 120 == 0 {
                eprintln!("  t {:.1} s: at ({:.1}, {:.1}, {:.1}) speed {:.2} m/s, input {:?}", i as f64 * dt, pos.x, pos.y, pos.z, player.ground_speed, (input.move_x, input.move_y, input.throttle));
            }
            let view = player.camera(&cw, dt, 16.0 / 9.0);
            world.set_focus(0, eo_sim::region::Focus { x: pos.x, z: pos.z, yaw: player.yaw });
            if (pos.x - last_stream[0]).hypot(pos.z - last_stream[1]) > 32.0 {
                last_stream = [pos.x, pos.z];
                eo_sim::native::stream(&mut world, &source, crate::agents::SIM_RADIUS_M);
            }
            world.advance(dt);
            let eye = [view.eye.x as f32, view.eye.y as f32, view.eye.z as f32];
            let fwd = view.forward.normalize_or(glam::DVec3::Z);
            // The game asks for the surface on a worker thread (a new street tile may need its surface built): not timed here either.
            let surface = surface_of(Some(street.as_ref()), land.as_ref(), player.character.feet().x, player.character.feet().z);
            let (t0, c0) = (Instant::now(), eo_render::hitch::thread_cycles());
            let walkers = cars_near(world.regions.iter().map(|r| &r.out), eye, &elevation, &mut cars);
            let ci = CityInput {
                dt: dt as f32,
                listener: Listener { pos: eye, forward: [fwd.x as f32, fwd.y as f32, fwd.z as f32], up: [0.0, 1.0, 0.0], vel: [0.0; 3] },
                player: player_audio(&player, &cw, surface),
                cars: &cars,
                walkers_near: walkers,
                weather,
                hour: s.hour,
                place: grid.at(eye[0], eye[2]),
                ui: 0,
                menu: Menu::None,
                volume: 1.0,
            };
            let n = director.frame(&mut engine, &ci);
            if let Ok(solo) = std::env::var("EO_AUDIO_SOLO") {
                // Measuring one bus's share of the mix: the others muted.
                for (k, b) in [eo_audio::Bus::Ambience, eo_audio::Bus::Traffic, eo_audio::Bus::Footsteps, eo_audio::Bus::Effects, eo_audio::Bus::Ui].into_iter().enumerate() {
                    engine.send(eo_audio::Cmd::Bus { bus: b, gain: if eo_audio::mixer::BUS_NAMES[k] == solo { 1.0 } else { 0.0 } });
                }
            }
            cost.push(t0.elapsed().as_secs_f64() * 1e3);
            cpu.push(eo_render::hitch::on_cpu_ms(c0, eo_render::hitch::thread_cycles()));
            cmds.push(n as f64);
            let pre = i < (pre_s / dt) as usize;
            if pre {
                let mut scratch = vec![];
                off.render(&mut scratch, per);
                steps0 = director.stats.steps;
            } else {
                off.render(&mut mix, per);
            }
            mixed_max = mixed_max.max(engine.stats.mixed.load(Ordering::Relaxed));
        }
        let wav = out_dir.join(format!("{}.wav", s.name));
        let mut w = hound::WavWriter::create(&wav, hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int })?;
        for v in &mix {
            w.write_sample((v.clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
        }
        w.finalize()?;
        let peak = mix.iter().fold(0f32, |m, v| m.max(v.abs()));
        let rms = (mix.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / mix.len().max(1) as f64).sqrt();
        let end = player.position(&cw);
        let st = &engine.stats;
        println!(
            "[{}] {:.0} s at {} h, {:?}, density {}: travelled {:.0} m; peak {:.1} dBFS, RMS {:.1} dBFS, limiter > 1 dB on {:.2} % of frames; voices mixed max {} (sources max {}); director per frame median {:.4} / max {:.4} ms wall, on-CPU median {:.4} / max {:.4} ms, {:.0} commands; {} steps, {} one-shots, {} nudges past obstacles; rendered in {:.1} s\n  wrote {}",
            s.name,
            seconds,
            s.hour,
            s.weather,
            s.density,
            (end - start).length(),
            20.0 * (peak.max(1e-9) as f64).log10(),
            20.0 * rms.max(1e-12).log10(),
            st.limited_frames.load(Ordering::Relaxed) as f64 * 100.0 / (frames_total * per) as f64,
            mixed_max,
            st.max_active.load(Ordering::Relaxed),
            pct(&cost, 50.0),
            pct(&cost, 100.0),
            pct(&cpu, 50.0),
            pct(&cpu, 100.0),
            pct(&cmds, 50.0),
            director.stats.steps - steps0,
            director.stats.one_shots,
            nudges,
            t_run.elapsed().as_secs_f64(),
            wav.display()
        );
        let d = &director.stats;
        println!(
            "  events (with the 3 s pre-roll): horns {}, birds {}, thunder {}, foghorns {}, ships {}, streetcars {}, BART/Muni under Market {}, sirens {}, cable in its slot {:.0} s; place density {:.2} water {:.2} green {:.2}",
            d.horns, d.birds, d.thunder, d.foghorns, d.ships, d.streetcars, d.subway, d.sirens, d.cable_s,
            grid.at(end.x as f32, end.z as f32).density,
            grid.at(end.x as f32, end.z as f32).water,
            grid.at(end.x as f32, end.z as f32).green
        );
    }
    Ok(())
}
