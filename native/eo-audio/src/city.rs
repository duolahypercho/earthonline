//! The city's sound director: what plays where, from the game state of one
//! frame given as plain data (no game crate types): the listener (the
//! camera), the player on foot or driving, the simulated cars near the
//! listener, the weather and the hour, the kind of place (density, water,
//! green), and the interface's cues. It runs on the render thread once a
//! frame and only sends commands; its cost is measured by the caller.
//!
//! San Francisco's own sounds are placed where the city has them
//! (`data/sf-sound-places.json`, the earlier browser version's exporter): the
//! cable running in its slot along the cable car tracks, heritage streetcars
//! passing on the surveyed streetcar tracks (heard from a street or two
//! away: the simulation has no streetcars to show), BART and Muni Metro
//! under Market Street, the foghorns of the Golden Gate Bridge, Alcatraz and
//! Yerba Buena Island (often in fog, now and then near the bay), ships'
//! horns on the bay, gulls at the waterfront, and US-pattern sirens in the
//! distance (the simulation has no emergency vehicles).

use crate::bank::{Bank, ClipId};
use crate::mixer::{Bus, Play, Source, Spatial};
use crate::spatial::{self, Falloff, Listener};
use crate::{Engine, Handle};
use serde::Deserialize;
use std::collections::HashMap;

pub const PLACES_JSON: &str = include_str!("../data/sf-sound-places.json");

/// Interface cues (bits in [`CityInput::ui`]).
pub mod cue {
    pub const HOVER: u32 = 1;
    pub const MOVE: u32 = 2;
    pub const CONFIRM: u32 = 4;
    pub const BACK: u32 = 8;
    pub const TOGGLE: u32 = 16;
    pub const START: u32 = 32;
    pub const OPEN: u32 = 64;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Surface {
    #[default]
    Concrete,
    Asphalt,
    Wood,
    Metal,
    Gravel,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Foot {
    pub feet: [f32; 3],
    /// Ground speed, m/s.
    pub speed: f32,
    pub grounded: bool,
    pub crouched: bool,
    pub surface: Surface,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Drive {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    /// Signed speed along the car, m/s.
    pub speed: f32,
    pub throttle: f32,
    pub brake: f32,
    pub handbrake: bool,
    /// Sideways speed of the body, m/s (slip).
    pub lateral: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum PlayerAudio {
    #[default]
    None,
    OnFoot(Foot),
    Driving(Drive),
}

/// A simulated car near the listener.
#[derive(Clone, Copy, Debug, Default)]
pub struct Car {
    pub id: u32,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub speed: f32,
    /// eo-sim kinds: 0 sedan, 1 taxi, 2 truck, 3 bus.
    pub kind: u8,
    pub stopped: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Weather {
    pub rain_mm_h: f32,
    pub wind_mps: f32,
    pub wetness: f32,
    /// Lightning flash 0..1 and its distance, m.
    pub flash: f32,
    pub flash_m: f32,
    /// 0 clear .. 1 fog (the fog preset or the schedule's fog rung).
    pub fog: f32,
}

/// What kind of place the listener is in, 0..1 each.
#[derive(Clone, Copy, Debug, Default)]
pub struct Place {
    /// Built-up and busy (downtown 1, the Sunset's houses ~0.2, a park 0).
    pub density: f32,
    /// At the water's edge 1, 400 m inland 0.
    pub water: f32,
    pub green: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Menu {
    #[default]
    None,
    /// The pause menu over the world: the city ducks.
    Paused,
    /// The title: no city.
    Title,
}

pub struct CityInput<'a> {
    pub dt: f32,
    pub listener: Listener,
    pub player: PlayerAudio,
    pub cars: &'a [Car],
    pub walkers_near: f32,
    pub weather: Weather,
    pub hour: f32,
    pub place: Place,
    pub ui: u32,
    pub menu: Menu,
    /// Master volume x the sound-effects mix (settings).
    pub volume: f32,
}

// ---------------------------------------------------------------- places

#[derive(Deserialize)]
struct PlacesJson {
    cable: Vec<Vec<f32>>,
    streetcar: Vec<StreetcarJson>,
    foghorns: Vec<FoghornJson>,
    bay_points: Vec<[f32; 2]>,
    market_subway: [f32; 4],
}
#[derive(Deserialize)]
struct StreetcarJson {
    line: String,
    pts: Vec<f32>,
}
#[derive(Deserialize)]
struct FoghornJson {
    name: String,
    at: [f32; 2],
}

/// Polylines with a grid for the nearest-point query.
pub struct Lines {
    pub lines: Vec<Vec<[f32; 2]>>,
    pub names: Vec<String>,
    cum: Vec<Vec<f32>>,
    grid: HashMap<(i32, i32), Vec<(u32, u32)>>,
}

const CELL: f32 = 64.0;

impl Lines {
    fn new(lines: Vec<Vec<[f32; 2]>>, names: Vec<String>) -> Self {
        let mut grid: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
        let mut cum = vec![];
        for (li, l) in lines.iter().enumerate() {
            let mut c = vec![0.0];
            for (si, w) in l.windows(2).enumerate() {
                c.push(c[si] + ((w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])));
                let (x0, x1) = (w[0][0].min(w[1][0]), w[0][0].max(w[1][0]));
                let (z0, z1) = (w[0][1].min(w[1][1]), w[0][1].max(w[1][1]));
                for gx in (x0 / CELL).floor() as i32..=(x1 / CELL).floor() as i32 {
                    for gz in (z0 / CELL).floor() as i32..=(z1 / CELL).floor() as i32 {
                        grid.entry((gx, gz)).or_default().push((li as u32, si as u32));
                    }
                }
            }
            cum.push(c);
        }
        Lines { lines, names, cum, grid }
    }

    /// Nearest point within `reach` m: (distance, line, arc length along it, point).
    pub fn nearest(&self, x: f32, z: f32, reach: f32) -> Option<(f32, usize, f32, [f32; 2])> {
        let r = (reach / CELL).ceil() as i32;
        let (cx, cz) = ((x / CELL).floor() as i32, (z / CELL).floor() as i32);
        let mut best: Option<(f32, usize, f32, [f32; 2])> = None;
        for gx in cx - r..=cx + r {
            for gz in cz - r..=cz + r {
                let Some(v) = self.grid.get(&(gx, gz)) else { continue };
                for &(li, si) in v {
                    let l = &self.lines[li as usize];
                    let (a, b) = (l[si as usize], l[si as usize + 1]);
                    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                    let ll = dx * dx + dz * dz;
                    let t = if ll > 1e-6 { (((x - a[0]) * dx + (z - a[1]) * dz) / ll).clamp(0.0, 1.0) } else { 0.0 };
                    let p = [a[0] + dx * t, a[1] + dz * t];
                    let d = (x - p[0]).hypot(z - p[1]);
                    if d <= reach && best.is_none_or(|bb| d < bb.0) {
                        best = Some((d, li as usize, self.cum[li as usize][si as usize] + t * ll.sqrt(), p));
                    }
                }
            }
        }
        best
    }

    /// The point at arc length `s` along line `li` (clamped to its ends).
    pub fn at(&self, li: usize, s: f32) -> [f32; 2] {
        let l = &self.lines[li];
        let c = &self.cum[li];
        let s = s.clamp(0.0, *c.last().unwrap_or(&0.0));
        let k = c.partition_point(|&v| v <= s).clamp(1, l.len() - 1);
        let (a, b) = (l[k - 1], l[k]);
        let seg = (c[k] - c[k - 1]).max(1e-6);
        let t = ((s - c[k - 1]) / seg).clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }

    pub fn length(&self, li: usize) -> f32 {
        *self.cum[li].last().unwrap_or(&0.0)
    }
}

pub struct Places {
    pub cable: Lines,
    pub streetcar: Lines,
    pub foghorns: Vec<(String, [f32; 2])>,
    pub bay_points: Vec<[f32; 2]>,
    pub market_subway: [f32; 4],
}

impl Places {
    pub fn embedded() -> Places {
        let mut j: PlacesJson = serde_json::from_str(PLACES_JSON).expect("sf-sound-places.json");
        // the surveyed places (tracks, the subway, the foghorns, the bay) on the scaled ground plan.
        let gs = eo_data::groundscale::scale() as f32;
        if gs != 1.0 {
            j.cable.iter_mut().flatten().for_each(|v| *v *= gs);
            j.streetcar.iter_mut().flat_map(|l| l.pts.iter_mut()).for_each(|v| *v *= gs);
            j.foghorns.iter_mut().for_each(|f| f.at = [f.at[0] * gs, f.at[1] * gs]);
            j.bay_points.iter_mut().for_each(|p| *p = [p[0] * gs, p[1] * gs]);
            j.market_subway.iter_mut().for_each(|v| *v *= gs);
        }
        let pts = |v: &[f32]| v.chunks_exact(2).map(|p| [p[0], p[1]]).collect::<Vec<_>>();
        let cable = Lines::new(j.cable.iter().map(|l| pts(l)).collect(), vec![String::new(); j.cable.len()]);
        let streetcar = Lines::new(j.streetcar.iter().map(|l| pts(&l.pts)).collect(), j.streetcar.iter().map(|l| l.line.clone()).collect());
        Places { cable, streetcar, foghorns: j.foghorns.into_iter().map(|f| (f.name, f.at)).collect(), bay_points: j.bay_points, market_subway: j.market_subway }
    }
}

// ---------------------------------------------------------------- director

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
    fn pick(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    /// Exponential wait with this mean, s.
    fn wait(&mut self, mean: f32) -> f64 {
        (-(1.0 - self.f()).max(1e-6).ln() * mean) as f64
    }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash32(x: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA77);
    h ^ (h >> 13)
}

/// Bank roles the director plays (written by native/tools/write-audio-manifest.mjs).
pub mod role {
    pub const BED_CITY: &str = "bed_city";
    pub const BED_QUIET: &str = "bed_quiet";
    pub const BED_WALLA: &str = "bed_walla";
    pub const BED_RAIN: &str = "bed_rain";
    pub const BED_WIND: &str = "bed_wind";
    pub const BED_WATER: &str = "bed_water";
    pub const ENGINE_CAR: &str = "engine_car";
    pub const ENGINE_HEAVY: &str = "engine_heavy";
    pub const ENGINE_PLAYER_LO: &str = "engine_player_lo";
    pub const ENGINE_PLAYER_HI: &str = "engine_player_hi";
    pub const TYRE_SQUEAL: &str = "tyre_squeal";
    pub const BRAKE: &str = "brake";
    pub const HORN: &str = "horn";
    pub const SIREN: &str = "siren";
    pub const CAR_DOOR: &str = "car_door";
    pub const GULL: &str = "gull";
    pub const PIGEON: &str = "pigeon";
    pub const BIRD: &str = "bird";
    pub const THUNDER: &str = "thunder";
    pub const FOGHORN: &str = "foghorn";
    pub const SHIP_HORN: &str = "ship_horn";
    pub const CABLE_SLOT: &str = "cable_slot";
    pub const STREETCAR: &str = "streetcar";
    pub const SUBWAY: &str = "subway";
    pub const FOOT: [&str; 5] = ["foot_concrete", "foot_asphalt", "foot_wood", "foot_metal", "foot_gravel"];
    pub const FOOT_GENERIC: &str = "foot_generic";
    pub const FOOT_WET: &str = "foot_wet";
    pub const UI: [(u32, &str); 7] = [
        (super::cue::HOVER, "ui_hover"),
        (super::cue::MOVE, "ui_move"),
        (super::cue::CONFIRM, "ui_confirm"),
        (super::cue::BACK, "ui_back"),
        (super::cue::TOGGLE, "ui_toggle"),
        (super::cue::START, "ui_start"),
        (super::cue::OPEN, "ui_open"),
    ];
}

/// Simulated cars given an engine and road voice at once.
pub const CAR_VOICES: usize = 14;
const CAR_REACH_M: f32 = 110.0;
const CAR_KEEP_M: f32 = 130.0;

struct CarVoice {
    id: u32,
    engine: Handle,
    road: Handle,
    pitch0: f32,
    stopped_s: f32,
    seen: bool,
}

struct BedVoice {
    h: Handle,
    side: f32,
    ends_at: f64,
    handed_on: bool,
}

struct Bed {
    role: &'static str,
    level: f32,
    sent: f32,
    quiet_s: f32,
    voices: Vec<BedVoice>,
}

struct Mover {
    h: Handle,
    line: usize,
    s: f32,
    ds: f32,
    end_s: f32,
    y: f32,
    gain: f32,
}

#[derive(Default)]
struct DriveVoices {
    lo: Handle,
    hi: Handle,
    road: Handle,
    squeal: Handle,
    rpm: f32,
    gear: usize,
    brake_latch: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectorStats {
    pub car_voices: usize,
    pub beds: usize,
    pub commands: usize,
    pub steps: u64,
    pub one_shots: u64,
    /// Events by kind: horns, birds, thunder, foghorns, ships, streetcars, subway trains, sirens, and seconds of cable.
    pub horns: u32,
    pub birds: u32,
    pub thunder: u32,
    pub foghorns: u32,
    pub ships: u32,
    pub streetcars: u32,
    pub subway: u32,
    pub sirens: u32,
    pub cable_s: f32,
}

pub struct Director {
    rng: Rng,
    time: f64,
    pub places: Places,
    cars: Vec<CarVoice>,
    beds: Vec<Bed>,
    drive: DriveVoices,
    was_driving: Option<bool>,
    step_dist: f32,
    air_s: f32,
    was_grounded: bool,
    foot_side: f32,
    last_slice: (ClipId, usize),
    horn_cool: f64,
    siren_t: f64,
    siren: Option<(Handle, [f32; 3], [f32; 3], f64)>,
    streetcar_t: f64,
    streetcar: Option<Mover>,
    subway_t: f64,
    ship_t: f64,
    fog_t: Vec<f64>,
    birds_t: [f64; 3],
    thunder: Vec<(f64, f32)>,
    last_flash: f32,
    cable: Handle,
    ui_t: f64,
    last_pos: Option<[f32; 3]>,
    vel: [f32; 3],
    /// The world buses and master last sent.
    sent: (f32, f32),
    pub stats: DirectorStats,
}

impl Director {
    pub fn new(seed: u64) -> Director {
        let mut d = Director {
            rng: Rng(seed | 1),
            time: 0.0,
            places: Places::embedded(),
            cars: vec![],
            beds: [role::BED_CITY, role::BED_QUIET, role::BED_WALLA, role::BED_RAIN, role::BED_WIND, role::BED_WATER].iter().map(|r| Bed { role: r, level: 0.0, sent: -1.0, quiet_s: 0.0, voices: vec![] }).collect(),
            drive: DriveVoices::default(),
            was_driving: None,
            step_dist: 0.0,
            air_s: 0.0,
            was_grounded: true,
            foot_side: 1.0,
            last_slice: (u32::MAX, 0),
            horn_cool: 0.0,
            siren_t: 0.0,
            siren: None,
            streetcar_t: 0.0,
            streetcar: None,
            subway_t: 0.0,
            ship_t: 0.0,
            fog_t: vec![],
            birds_t: [0.0; 3],
            thunder: vec![],
            last_flash: 0.0,
            cable: Handle(0),
            ui_t: -1.0,
            last_pos: None,
            vel: [0.0; 3],
            sent: (-1.0, -1.0),
            stats: DirectorStats::default(),
        };
        d.siren_t = d.rng.wait(90.0);
        d.streetcar_t = d.rng.wait(40.0);
        d.subway_t = d.rng.wait(30.0);
        d.ship_t = d.rng.wait(120.0);
        d.fog_t = (0..d.places.foghorns.len()).map(|i| 3.0 + 11.0 * i as f64).collect();
        d
    }

    fn pick(&mut self, bank: &Bank, role: &str) -> Option<ClipId> {
        let v = bank.role(role);
        if v.is_empty() { None } else { Some(v[self.rng.pick(v.len())]) }
    }

    fn one_shot(&mut self, e: &mut Engine, bank: &Bank, role: &str, bus: Bus, pri: u8, gain: f32, pitch: f32, at: Option<([f32; 3], [f32; 3], Falloff)>, lowpass: f32) -> Handle {
        let Some(c) = self.pick(bank, role) else { return Handle(0) };
        self.stats.one_shots += 1;
        e.play(Play {
            source: Source::Clip { clip: c, start: 0, len: 0, looping: false },
            bus,
            priority: pri,
            gain,
            pitch,
            pan: 0.0,
            spatial: at.map(|(pos, vel, falloff)| Spatial { pos, vel, falloff, doppler: 1.0 }),
            fade_in_s: 0.0,
            lowpass_hz: lowpass,
        })
    }

    /// One frame. Returns the commands sent.
    pub fn frame(&mut self, e: &mut Engine, inp: &CityInput) -> usize {
        let before = e.pending();
        let dt = inp.dt.clamp(0.0, 0.25);
        self.time += dt as f64;
        let Some(bank) = e.bank().cloned() else { return 0 };
        // Listener velocity from its motion (a teleport is not a Doppler).
        let mut l = inp.listener;
        if let Some(p) = self.last_pos {
            let v = spatial::sub(l.pos, p);
            let s = spatial::len(v) / dt.max(1e-3);
            let k = (dt * 8.0).min(1.0);
            let target = if s < 60.0 && dt > 0.0 { [v[0] / dt, v[1] / dt, v[2] / dt] } else { [0.0; 3] };
            for i in 0..3 {
                self.vel[i] += (target[i] - self.vel[i]) * k;
            }
        }
        self.last_pos = Some(l.pos);
        l.vel = self.vel;
        e.send(crate::Cmd::Listener(l));
        // Buses: the world ducks under the pause menu, goes under the title.
        let world = match inp.menu {
            Menu::None => 1.0,
            Menu::Paused => 0.2,
            Menu::Title => 0.0,
        };
        if self.sent != (world, inp.volume) {
            self.sent = (world, inp.volume);
            for b in [Bus::Ambience, Bus::Traffic, Bus::Footsteps, Bus::Effects] {
                e.send(crate::Cmd::Bus { bus: b, gain: world });
            }
            e.send(crate::Cmd::Master(inp.volume.clamp(0.0, 1.0)));
        }
        self.ui(e, &bank, inp.ui);
        if inp.menu != Menu::Title {
            self.traffic(e, &bank, inp, &l);
            self.player(e, &bank, inp);
            self.beds(e, &bank, inp);
            self.events(e, &bank, inp, &l);
        }
        let n = e.pending().saturating_sub(before);
        self.stats.commands = n;
        n
    }

    fn ui(&mut self, e: &mut Engine, bank: &Bank, cues: u32) {
        for (bit, r) in role::UI {
            if cues & bit == 0 {
                continue;
            }
            if bit == cue::HOVER && self.time - self.ui_t < 0.06 {
                continue;
            }
            self.ui_t = self.time;
            self.one_shot(e, bank, r, Bus::Ui, 7, 0.5, 1.0, None, 20_000.0);
        }
    }

    // ---- traffic: an engine loop and a road roar on each near car, horns at jams.
    fn traffic(&mut self, e: &mut Engine, bank: &Bank, inp: &CityInput, l: &Listener) {
        let mut near: Vec<(f32, usize)> = inp.cars.iter().enumerate().map(|(i, c)| (spatial::len(spatial::sub(c.pos, l.pos)), i)).filter(|(d, _)| *d < CAR_KEEP_M).collect();
        near.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for v in &mut self.cars {
            v.seen = false;
        }
        let mut voiced = 0;
        for &(d, i) in &near {
            let c = inp.cars[i];
            let existing = self.cars.iter().position(|v| v.id == c.id);
            let keep = match existing {
                Some(_) => voiced < CAR_VOICES,
                None => voiced < CAR_VOICES && d < CAR_REACH_M,
            };
            if !keep {
                continue;
            }
            voiced += 1;
            let k = match existing {
                Some(k) => k,
                None => {
                    let pool = if c.kind >= 2 { role::ENGINE_HEAVY } else { role::ENGINE_CAR };
                    let h = hash32(c.id);
                    let ids = bank.role(pool);
                    let ids = if ids.is_empty() { bank.role(role::ENGINE_CAR) } else { ids };
                    let engine = match ids.get(h as usize % ids.len().max(1)) {
                        Some(&clip) => e.play(Play {
                            source: Source::Clip { clip, start: 0, len: 0, looping: true },
                            bus: Bus::Traffic,
                            priority: 3,
                            gain: 0.0,
                            pitch: 1.0,
                            pan: 0.0,
                            spatial: Some(Spatial { pos: c.pos, vel: c.vel, falloff: Falloff::new(3.0, CAR_KEEP_M, 1.0), doppler: 1.0 }),
                            fade_in_s: 0.4,
                            lowpass_hz: 8_000.0,
                        }),
                        None => Handle(0),
                    };
                    let road = e.play(Play {
                        source: Source::Noise { seed: h | 1 },
                        bus: Bus::Traffic,
                        priority: 3,
                        gain: 0.0,
                        pitch: 1.0,
                        pan: 0.0,
                        spatial: Some(Spatial { pos: c.pos, vel: c.vel, falloff: Falloff::new(3.0, CAR_KEEP_M + 10.0, 1.0), doppler: 1.0 }),
                        fade_in_s: 0.4,
                        lowpass_hz: 600.0,
                    });
                    let pitch0 = 0.92 + 0.16 * ((h >> 8) & 0xff) as f32 / 255.0;
                    self.cars.push(CarVoice { id: c.id, engine, road, pitch0, stopped_s: 0.0, seen: true });
                    self.cars.len() - 1
                }
            };
            let wet = inp.weather.wetness;
            let v = &mut self.cars[k];
            v.seen = true;
            v.stopped_s = if c.stopped || c.speed < 0.3 { v.stopped_s + inp.dt } else { 0.0 };
            let sp = c.speed.max(0.0);
            let heavy = if c.kind >= 2 { 1.25 } else { 1.0 };
            let eg = (0.9 + 0.9 * smooth(0.0, 12.0, sp)) * heavy;
            let ep = v.pitch0 * (0.85 + 0.55 * (sp / 15.0).min(1.3));
            e.update(v.engine, c.pos, c.vel, eg, ep, 6_000.0 + 400.0 * sp);
            // Tyres on the road: the roar grows with speed; a wet road hisses.
            let rg = ((sp / 14.0).powf(1.6) * 1.4 * (1.0 + 0.6 * wet)).min(2.0) * heavy;
            e.update(v.road, c.pos, c.vel, rg, 1.0, 450.0 + 120.0 * sp + 2_500.0 * wet);
            // Horns at a jam: a car held for a while, now and then, never two at once.
            let stopped_s = v.stopped_s;
            // About one honk a half minute from each held car, at most one every 10-25 s around the listener.
            if stopped_s > 4.0 && d < 70.0 && self.time > self.horn_cool && self.rng.f() < 0.03 * inp.dt {
                self.horn_cool = self.time + 10.0 + self.rng.range(0.0, 15.0) as f64;
                let pitch = self.rng.range(0.97, 1.03);
                self.stats.horns += 1;
                self.one_shot(e, bank, role::HORN, Bus::Traffic, 5, 0.85, pitch, Some((c.pos, [0.0; 3], Falloff::new(4.0, 350.0, 1.0))), 20_000.0);
            }
        }
        let mut i = 0;
        while i < self.cars.len() {
            if !self.cars[i].seen {
                let v = self.cars.swap_remove(i);
                e.stop(v.engine, 0.5);
                e.stop(v.road, 0.5);
            } else {
                i += 1;
            }
        }
        self.stats.car_voices = self.cars.len();
    }

    // ---- the player: steps on foot; engine, road, squeal and brakes driving.
    fn player(&mut self, e: &mut Engine, bank: &Bank, inp: &CityInput) {
        let driving = matches!(inp.player, PlayerAudio::Driving(_));
        if self.was_driving.is_some_and(|w| w != driving) {
            // Into or out of the car: its door.
            let at = match inp.player {
                PlayerAudio::Driving(d) => d.pos,
                PlayerAudio::OnFoot(f) => f.feet,
                PlayerAudio::None => inp.listener.pos,
            };
            self.one_shot(e, bank, role::CAR_DOOR, Bus::Effects, 6, 0.8, 1.0, Some((at, [0.0; 3], Falloff::new(2.0, 60.0, 1.0))), 20_000.0);
        }
        self.was_driving = Some(driving);
        match inp.player {
            PlayerAudio::OnFoot(f) => {
                self.stop_drive(e);
                self.steps(e, bank, inp, f);
            }
            PlayerAudio::Driving(d) => self.drive(e, bank, inp, d),
            PlayerAudio::None => self.stop_drive(e),
        }
    }

    fn stop_drive(&mut self, e: &mut Engine) {
        let d = std::mem::take(&mut self.drive);
        for h in [d.lo, d.hi, d.road, d.squeal] {
            e.stop(h, 0.3);
        }
    }

    fn steps(&mut self, e: &mut Engine, bank: &Bank, inp: &CityInput, f: Foot) {
        let dt = inp.dt;
        if !f.grounded {
            self.air_s += dt;
            self.was_grounded = false;
            return;
        }
        let landed = !self.was_grounded && self.air_s > 0.3;
        self.was_grounded = true;
        self.air_s = 0.0;
        let running = f.speed > 2.6;
        let stride = if running { 1.0 + 0.1 * smooth(4.0, 6.5, f.speed) } else { 0.72 };
        if f.speed < 0.25 {
            self.step_dist = stride * 0.6; // the first step comes soon after setting off
        } else {
            self.step_dist += f.speed * dt;
        }
        let mut n = 0;
        if landed {
            n = 2;
        } else if self.step_dist >= stride {
            self.step_dist -= stride;
            n = 1;
        }
        for k in 0..n {
            let gain = if f.crouched { 0.7 } else if running { 1.9 } else { 1.2 } * if landed { 1.25 } else { 1.0 };
            let surface = role::FOOT[f.surface as usize];
            let pool = [surface, role::FOOT_GENERIC, role::FOOT[0]].into_iter().find(|r| !bank.role(r).is_empty());
            let Some(pool) = pool else { return };
            self.foot_side = -self.foot_side;
            let right = spatial::normalize(spatial::cross(inp.listener.forward, inp.listener.up));
            let off = 0.12 * self.foot_side;
            let pos = [f.feet[0] + right[0] * off, f.feet[1] + 0.05, f.feet[2] + right[2] * off];
            let pitch = self.rng.range(0.95, 1.05) * if running { 1.03 } else { 1.0 };
            self.slice(e, bank, pool, gain, pitch, pos, k as f32 * 0.0);
            if inp.weather.wetness > 0.25 && !bank.role(role::FOOT_WET).is_empty() {
                self.slice(e, bank, role::FOOT_WET, gain * inp.weather.wetness * 0.7, pitch, pos, 0.0);
            }
            self.stats.steps += 1;
        }
    }

    fn slice(&mut self, e: &mut Engine, bank: &Bank, pool: &str, gain: f32, pitch: f32, pos: [f32; 3], _delay: f32) {
        let ids = bank.role(pool);
        if ids.is_empty() {
            return;
        }
        let c = ids[self.rng.pick(ids.len())];
        let Some(clip) = bank.clip(c) else { return };
        let (start, len) = if clip.slices.is_empty() {
            (0, clip.frames.min((clip.rate as f32 * 0.4) as u32))
        } else {
            let mut k = self.rng.pick(clip.slices.len());
            if (c, k) == self.last_slice && clip.slices.len() > 1 {
                k = (k + 1) % clip.slices.len();
            }
            self.last_slice = (c, k);
            (clip.slices[k][0], clip.slices[k][1])
        };
        e.play(Play {
            source: Source::Clip { clip: c, start, len, looping: false },
            bus: Bus::Footsteps,
            priority: 6,
            gain,
            pitch,
            pan: 0.0,
            spatial: Some(Spatial { pos, vel: [0.0; 3], falloff: Falloff::new(2.5, 45.0, 1.0), doppler: 0.0 }),
            fade_in_s: 0.0,
            lowpass_hz: 20_000.0,
        });
    }

    fn drive(&mut self, e: &mut Engine, bank: &Bank, inp: &CityInput, d: Drive) {
        let falloff = Falloff::new(4.0, 300.0, 1.0);
        let looped = |e: &mut Engine, clip: Option<ClipId>, gain: f32, lp: f32, src: Option<Source>| -> Handle {
            let source = match (src, clip) {
                (Some(s), _) => s,
                (None, Some(c)) => Source::Clip { clip: c, start: 0, len: 0, looping: true },
                (None, None) => return Handle(0),
            };
            e.play(Play { source, bus: Bus::Effects, priority: 7, gain, pitch: 1.0, pan: 0.0, spatial: Some(Spatial { pos: d.pos, vel: d.vel, falloff, doppler: 0.0 }), fade_in_s: 0.3, lowpass_hz: lp })
        };
        if !self.drive.lo.is_some() {
            let lo = self.pick(bank, role::ENGINE_PLAYER_LO).or_else(|| self.pick(bank, role::ENGINE_CAR));
            let hi = self.pick(bank, role::ENGINE_PLAYER_HI);
            let sq = self.pick(bank, role::TYRE_SQUEAL);
            self.drive.lo = looped(e, lo, 0.0, 12_000.0, None);
            self.drive.hi = looped(e, hi, 0.0, 12_000.0, None);
            self.drive.road = looped(e, None, 0.0, 800.0, Some(Source::Noise { seed: 0x5eed }));
            self.drive.squeal = looped(e, sq, 0.0, 20_000.0, None);
            self.drive.gear = 1;
            self.drive.rpm = 800.0;
        }
        // A five-speed automatic: in-gear rpm from speed, up at 5,600, down under 1,500.
        const TOP: [f32; 6] = [0.0, 8.0, 15.0, 23.0, 32.0, 50.0];
        let v = d.speed.abs();
        let g = &mut self.drive.gear;
        let rpm_of = |g: usize| 800.0 + v / TOP[g] * 5_000.0;
        while *g < 5 && rpm_of(*g) > 5_600.0 {
            *g += 1;
        }
        while *g > 1 && rpm_of(*g) < 1_500.0 && rpm_of(*g - 1) < 5_200.0 {
            *g -= 1;
        }
        let free_rev = if v < 1.0 { 1_800.0 * d.throttle } else { 0.0 };
        let target = (rpm_of(*g) + free_rev + 400.0 * d.throttle).clamp(780.0, 6_400.0);
        let k = (inp.dt * 6.0).min(1.0);
        self.drive.rpm += (target - self.drive.rpm) * k;
        let rpm = self.drive.rpm;
        let x = smooth(1_800.0, 3_800.0, rpm);
        let load = 0.55 + 0.45 * d.throttle;
        e.update(self.drive.lo, d.pos, d.vel, (1.0 - x) * load * 0.32, (rpm / 900.0).clamp(0.75, 2.2), 12_000.0);
        e.update(self.drive.hi, d.pos, d.vel, x * load * 0.35, (rpm / 3_200.0).clamp(0.6, 1.9), 9_000.0 + 1_500.0 * d.throttle);
        let wet = inp.weather.wetness;
        let road = ((v / 14.0).powf(1.5) * 0.45 * (1.0 + 0.6 * wet)).min(0.8);
        e.update(self.drive.road, d.pos, d.vel, road, 1.0, 500.0 + 110.0 * v + 2_500.0 * wet);
        // Squeal: sliding sideways, the handbrake at speed, or a locked stop.
        let slide = smooth(2.5, 6.0, d.lateral.abs()) * smooth(3.0, 8.0, v);
        let hand = if d.handbrake && v > 5.0 { 0.8 } else { 0.0 };
        let lock = if d.brake > 0.9 && v > 12.0 { 0.5 } else { 0.0 };
        let sq = slide.max(hand).max(lock) * (1.0 - 0.6 * wet);
        e.update(self.drive.squeal, d.pos, d.vel, sq, 0.95 + 0.1 * smooth(5.0, 25.0, v), 20_000.0);
        // Brakes: a squeal as the car comes to rest.
        if d.brake > 0.4 && v > 1.0 && v < 5.0 && !self.drive.brake_latch {
            self.drive.brake_latch = true;
            self.one_shot(e, bank, role::BRAKE, Bus::Effects, 6, 0.45, 1.0, Some((d.pos, d.vel, falloff)), 20_000.0);
        }
        if d.brake < 0.1 || v < 0.3 {
            self.drive.brake_latch = false;
        }
    }

    // ---- beds: two decorrelated clips left and right, crossfaded to the next before the end.
    fn beds(&mut self, e: &mut Engine, bank: &Bank, inp: &CityInput) {
        let h = inp.hour;
        let day = smooth(5.5, 7.5, h) * (1.0 - smooth(19.5, 21.5, h));
        let night = 1.0 - day;
        let w = &inp.weather;
        let p = &inp.place;
        let rain = smooth(0.05, 1.0, w.rain_mm_h) * 0.85 + smooth(2.0, 15.0, w.rain_mm_h) * 0.45;
        let levels = [
            p.density.powf(0.7) * (0.6 + 0.4 * day) * 0.8,
            (1.0 - p.density) * (0.5 + 0.25 * day) * (1.0 - 0.5 * p.water),
            smooth(1.0, 14.0, inp.walkers_near) * (0.3 + 0.7 * day) * (1.0 - 0.7 * smooth(0.5, 4.0, w.rain_mm_h)) * 0.65,
            rain,
            smooth(3.0, 14.0, w.wind_mps) * 0.65 * (0.6 + 0.4 * night.max(1.0 - p.density)),
            smooth(0.55, 0.95, p.water) * 0.7,
        ];
        let now = self.time;
        for (bi, target) in levels.into_iter().enumerate() {
            let k = (inp.dt * 0.8).min(1.0);
            let b = &mut self.beds[bi];
            b.level += (target - b.level) * k;
            let level = b.level;
            let role = b.role;
            if level < 0.003 {
                b.quiet_s += inp.dt;
                if b.quiet_s > 3.0 && !b.voices.is_empty() {
                    for v in b.voices.drain(..) {
                        e.stop(v.h, 1.5);
                    }
                }
                continue;
            }
            b.quiet_s = 0.0;
            let ids = bank.role(role);
            if ids.is_empty() {
                continue;
            }
            // Start the two sides, then hand each on to a fresh clip 3 s before it ends.
            let mut starts: Vec<f32> = vec![];
            if self.beds[bi].voices.is_empty() {
                starts.extend([-1.0, 1.0]);
            }
            for v in &mut self.beds[bi].voices {
                if !v.handed_on && now > v.ends_at - 3.0 {
                    v.handed_on = true;
                    starts.push(v.side);
                    e.stop(v.h, 3.0);
                }
            }
            self.beds[bi].voices.retain(|v| now < v.ends_at + 0.1);
            for side in starts {
                let clip = ids[self.rng.pick(ids.len())];
                let Some(c) = bank.clip(clip) else { continue };
                let dur = c.duration_s();
                let start_s = if dur > 30.0 { self.rng.range(0.0, (dur - 20.0).max(0.0)) } else { self.rng.range(0.0, (dur * 0.4).max(0.0)) };
                let start = (start_s * c.rate as f32) as u32;
                let fade = if self.beds[bi].voices.is_empty() && level > 0.0 { 1.5 } else { 3.0 };
                let h = e.play(Play { source: Source::Clip { clip, start, len: 0, looping: false }, bus: Bus::Ambience, priority: 7, gain: level, pitch: 1.0, pan: side * 0.65, spatial: None, fade_in_s: fade, lowpass_hz: 20_000.0 });
                let rem = dur - start_s;
                self.beds[bi].voices.push(BedVoice { h, side, ends_at: now + rem as f64, handed_on: false });
            }
            let b = &mut self.beds[bi];
            if (b.level - b.sent).abs() > 0.004 * b.sent.max(0.05) {
                b.sent = b.level;
                for v in &b.voices {
                    e.set(v.h, b.level, 1.0, v.side * 0.65);
                }
            }
        }
        self.stats.beds = self.beds.iter().map(|b| b.voices.len()).sum();
    }

    // ---- events: birds, thunder, the SF set (cable, streetcars, BART, foghorns, ships), sirens.
    fn events(&mut self, e: &mut Engine, bank: &Bank, inp: &CityInput, l: &Listener) {
        let now = self.time;
        let dt = inp.dt;
        let h = inp.hour;
        let day = smooth(5.5, 7.5, h) * (1.0 - smooth(19.0, 21.0, h));
        let dry = 1.0 - smooth(0.3, 3.0, inp.weather.rain_mm_h);
        let p = inp.place;
        // Birds: gulls by the water (and over downtown), pigeons in busy streets, songbirds in green and quiet streets.
        let rates = [
            (0.12 * p.water + 0.015 * p.density) * (0.25 + 0.75 * day) * (0.4 + 0.6 * dry),
            0.05 * p.density * day * dry,
            (0.05 * (1.0 - p.density) + 0.07 * p.green) * day * dry * (1.0 + smooth(5.0, 6.5, h) * (1.0 - smooth(7.5, 9.0, h))),
        ];
        let roles = [role::GULL, role::PIGEON, role::BIRD];
        for k in 0..3 {
            if rates[k] <= 1e-4 {
                continue;
            }
            if now >= self.birds_t[k] {
                self.birds_t[k] = now + self.rng.wait(1.0 / rates[k]);
                let (r0, r1, y0, y1) = if k == 0 { (15.0, 70.0, 6.0, 25.0) } else { (6.0, 35.0, 3.0, 14.0) };
                let a = self.rng.range(0.0, std::f32::consts::TAU);
                let r = self.rng.range(r0, r1);
                let pos = [l.pos[0] + a.cos() * r, l.pos[1] + self.rng.range(y0, y1), l.pos[2] + a.sin() * r];
                let vel = if k == 0 { [self.rng.range(-6.0, 6.0), 0.0, self.rng.range(-6.0, 6.0)] } else { [0.0; 3] };
                let pitch = self.rng.range(0.96, 1.04);
                self.stats.birds += 1;
                self.one_shot(e, bank, roles[k], Bus::Ambience, 2, 0.6, pitch, Some((pos, vel, Falloff::new(2.0, 150.0, 1.0))), 20_000.0);
            }
        }
        // Thunder: after the flash, by the time sound takes to come that far.
        let w = inp.weather;
        if w.flash > 0.4 && self.last_flash <= 0.4 && w.flash_m > 0.0 {
            self.thunder.push((now + (w.flash_m / spatial::SPEED_OF_SOUND) as f64, w.flash_m));
        }
        self.last_flash = w.flash;
        let mut i = 0;
        while i < self.thunder.len() {
            if now >= self.thunder[i].0 {
                let d = self.thunder.swap_remove(i).1;
                let g = (1500.0 / d.max(300.0)).powf(0.7).min(1.3);
                let lp = 700.0 + 9_000.0 * (1500.0 / d.max(300.0)).min(1.0);
                let pitch = self.rng.range(0.9, 1.05);
                self.stats.thunder += 1;
                self.one_shot(e, bank, role::THUNDER, Bus::Effects, 6, g * 0.8, pitch, None, lp);
            } else {
                i += 1;
            }
        }
        let (x, z) = (l.pos[0], l.pos[2]);
        let ground = l.pos[1] - 1.7;
        // The cable car cable, running in its slot (cars run 6:30 to 00:30).
        let cable_on = !(0.5..6.5).contains(&h);
        match self.places.cable.nearest(x, z, 45.0).filter(|_| cable_on) {
            Some((d, _, _, q)) => {
                let pos = [q[0], ground, q[1]];
                if !self.cable.is_some() {
                    if let Some(c) = self.pick(bank, role::CABLE_SLOT) {
                        self.cable = e.play(Play { source: Source::Clip { clip: c, start: 0, len: 0, looping: true }, bus: Bus::Ambience, priority: 5, gain: 0.0, pitch: 1.0, pan: 0.0, spatial: Some(Spatial { pos, vel: [0.0; 3], falloff: Falloff::new(2.0, 50.0, 1.0), doppler: 0.0 }), fade_in_s: 1.0, lowpass_hz: 20_000.0 });
                    }
                }
                let _ = d;
                e.update(self.cable, pos, [0.0; 3], 0.75, 1.0, 20_000.0);
                self.stats.cable_s += dt;
            }
            None => {
                if self.cable.is_some() {
                    e.stop(self.cable, 1.0);
                    self.cable = Handle(0);
                }
            }
        }
        // A heritage streetcar passing on a track a street or two away (55-260 m: out of sight).
        if let Some(m) = &mut self.streetcar {
            m.s += m.ds * dt;
            let q = self.places.streetcar.at(m.line, m.s);
            let q2 = self.places.streetcar.at(m.line, m.s + m.ds.signum());
            let vel = [(q2[0] - q[0]) * m.ds.abs(), 0.0, (q2[1] - q[1]) * m.ds.abs()];
            e.update(m.h, [q[0], m.y, q[1]], vel, m.gain, 1.0, 20_000.0);
            if (m.ds > 0.0 && m.s >= m.end_s) || (m.ds < 0.0 && m.s <= m.end_s) {
                e.stop(m.h, 2.5);
                self.streetcar = None;
                self.streetcar_t = now + self.rng.range(120.0, 300.0) as f64;
            }
        } else if now >= self.streetcar_t && (6.0..24.0).contains(&h) {
            self.streetcar_t = now + 20.0;
            if let Some((d, li, s, _)) = self.places.streetcar.nearest(x, z, 260.0) {
                if d >= 55.0 {
                    if let Some(c) = self.pick(bank, role::STREETCAR) {
                        let dir = if self.rng.f() < 0.5 { 1.0 } else { -1.0 };
                        let len = self.places.streetcar.length(li);
                        let s0 = (s - dir * 90.0).clamp(0.0, len);
                        let s1 = (s + dir * 90.0).clamp(0.0, len);
                        let q = self.places.streetcar.at(li, s0);
                        let hnd = e.play(Play { source: Source::Clip { clip: c, start: 0, len: 0, looping: false }, bus: Bus::Traffic, priority: 4, gain: 0.9, pitch: 1.0, pan: 0.0, spatial: Some(Spatial { pos: [q[0], ground + 1.0, q[1]], vel: [0.0; 3], falloff: Falloff::new(6.0, 420.0, 1.0), doppler: 1.0 }), fade_in_s: 1.5, lowpass_hz: 20_000.0 });
                        self.stats.streetcars += 1;
                        self.streetcar = Some(Mover { h: hnd, line: li, s: s0, ds: dir * 8.0, end_s: s1, y: ground + 1.0, gain: 0.9 });
                    }
                }
            }
        }
        // BART and Muni Metro under Market Street: a train rumbles up through the gratings.
        let [ax, az, bx, bz] = self.places.market_subway;
        let (dx, dz) = (bx - ax, bz - az);
        let t = (((x - ax) * dx + (z - az) * dz) / (dx * dx + dz * dz)).clamp(0.0, 1.0);
        let (qx, qz) = (ax + dx * t, az + dz * t);
        if (x - qx).hypot(z - qz) < 40.0 && now >= self.subway_t && !(1.0..5.0).contains(&h) {
            self.subway_t = now + self.rng.range(70.0, 160.0) as f64;
            self.stats.subway += 1;
            self.one_shot(e, bank, role::SUBWAY, Bus::Ambience, 3, 0.55, 1.0, Some(([qx, ground - 10.0, qz], [0.0; 3], Falloff::new(12.0, 120.0, 1.0))), 900.0);
        }
        // Foghorns: every half minute in fog; near the bay, now and then in clear air.
        let fog = w.fog;
        for i in 0..self.places.foghorns.len() {
            if now < self.fog_t[i] {
                continue;
            }
            let active = fog > 0.3 || p.water > 0.35;
            if !active {
                self.fog_t[i] = now + 5.0;
                continue;
            }
            // In fog every half minute (each station its own beat); near the bay, a distant one now and then.
            let period = if fog > 0.3 { 30.0 + 2.0 * i as f32 } else { self.rng.range(100.0, 220.0) };
            self.fog_t[i] = now + period as f64;
            let at = self.places.foghorns[i].1;
            let ids = bank.role(role::FOGHORN);
            if ids.is_empty() {
                continue;
            }
            self.stats.foghorns += 1;
            // Each station keeps its own voice.
            let clip = ids[i % ids.len()];
            let gain = if fog > 0.3 { 1.0 } else { 0.55 * smooth(0.35, 0.9, p.water) };
            e.play(Play { source: Source::Clip { clip, start: 0, len: 0, looping: false }, bus: Bus::Ambience, priority: 5, gain, pitch: 1.0, pan: 0.0, spatial: Some(Spatial { pos: [at[0], 30.0, at[1]], vel: [0.0; 3], falloff: Falloff::new(400.0, 14_000.0, 0.9), doppler: 0.0 }), fade_in_s: 0.0, lowpass_hz: 20_000.0 });
            self.stats.one_shots += 1;
        }
        // Ships' horns on the bay, heard from the waterfront.
        if now >= self.ship_t {
            self.ship_t = now + self.rng.range(240.0, 480.0) as f64;
            if p.water > 0.5 && !self.places.bay_points.is_empty() {
                let q = self.places.bay_points[self.rng.pick(self.places.bay_points.len())];
                let pos = [q[0] + self.rng.range(-400.0, 400.0), 15.0, q[1] + self.rng.range(-400.0, 400.0)];
                self.stats.ships += 1;
                self.one_shot(e, bank, role::SHIP_HORN, Bus::Ambience, 4, 0.8, 1.0, Some((pos, [0.0; 3], Falloff::new(300.0, 12_000.0, 0.9))), 20_000.0);
            }
        }
        // A US siren somewhere in the city, crossing at a distance (more downtown).
        if let Some((hd, pos, vel, until)) = &mut self.siren {
            for i in 0..3 {
                pos[i] += vel[i] * dt;
            }
            e.update(*hd, *pos, *vel, 0.9, 1.0, 20_000.0);
            if now > *until {
                e.stop(*hd, 2.0);
                self.siren = None;
            }
        } else if now >= self.siren_t {
            self.siren_t = now + self.rng.wait(220.0 / (0.25 + p.density)) + 30.0;
            if p.density > 0.15 {
                if let Some(c) = self.pick(bank, role::SIREN) {
                    let a = self.rng.range(0.0, std::f32::consts::TAU);
                    let r = self.rng.range(280.0, 700.0);
                    let pos = [x + a.cos() * r, ground + 1.5, z + a.sin() * r];
                    let s = self.rng.range(12.0, 18.0);
                    let vel = [-a.sin() * s, 0.0, a.cos() * s];
                    let dur = bank.clip(c).map(|k| k.duration_s()).unwrap_or(10.0) as f64;
                    let hd = e.play(Play { source: Source::Clip { clip: c, start: 0, len: 0, looping: false }, bus: Bus::Traffic, priority: 5, gain: 0.9, pitch: 1.0, pan: 0.0, spatial: Some(Spatial { pos, vel, falloff: Falloff::new(25.0, 1_600.0, 1.0), doppler: 1.0 }), fade_in_s: 2.0, lowpass_hz: 20_000.0 });
                    self.stats.sirens += 1;
                    self.siren = Some((hd, pos, vel, now + dur.min(40.0)));
                    self.stats.one_shots += 1;
                }
            }
        }
    }

    /// Stop everything this director started.
    pub fn stop_all(&mut self, e: &mut Engine) {
        e.send(crate::Cmd::StopAll { fade_s: 0.3 });
        self.cars.clear();
        for b in &mut self.beds {
            b.voices.clear();
            b.sent = -1.0;
        }
        self.drive = DriveVoices::default();
        self.cable = Handle(0);
        self.streetcar = None;
        self.siren = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_load_and_the_nearest_point_is_on_a_track() {
        let p = Places::embedded();
        assert!(p.cable.lines.len() > 50 && p.streetcar.lines.len() > 200);
        assert_eq!(p.foghorns.len(), 3);
        // Start from a vertex of the first cable track: distance 0.
        let q = p.cable.lines[0][0];
        let (d, li, s, at) = p.cable.nearest(q[0] + 0.01, q[1], 10.0).unwrap();
        assert!(d < 0.1, "{d}");
        let back = p.cable.at(li, s);
        assert!((back[0] - at[0]).abs() < 0.05 && (back[1] - at[1]).abs() < 0.05);
        // Nothing within reach far from every track.
        assert!(p.cable.nearest(50_000.0, 50_000.0, 45.0).is_none());
    }

    #[test]
    fn foghorns_sit_on_the_bay_north_of_downtown() {
        let p = Places::embedded();
        // The runtime frame: z is south, so the bay's stations are at negative z, kilometres out.
        for (name, at) in &p.foghorns {
            assert!(at[1] < -3_000.0, "{name} {at:?}");
        }
    }
}
