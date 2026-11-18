//! Offscreen shots: `eo-app --shots FILE.json --out DIR` renders one PNG per
//! shot of FILE.json into DIR, and `eo-app --scene-cards --out DIR` the
//! visual gate's eight scene cards (scene_cards.rs) the same way. Every frame
//! is taken from the bench's offscreen target after
//!
//! 1. the hour and weather of the shot are set, with the cloud and water
//!    clocks pinned (a still does not depend on how long anything ran);
//! 2. the traffic and people are rebuilt round the shot from the
//!    simulation's seeded warm-up and stepped a fixed number of frames;
//! 3. streaming has settled for that view (`settle_p`: every model, kit and
//!    street tile, far cell and prop the view wants resident, the upload
//!    queue empty), with a time cap;
//! 4. the temporal resolve has converged under the static camera: frames are
//!    drawn until two frames one jitter cycle apart differ by less than
//!    `TAA_RMS` (8-bit RMS), or the difference has stopped changing (what
//!    is left changes every frame: the post pass's display dither, the
//!    rooms' and lamps' noise; not the history), with a frame cap.
//!
//! Beside each PNG, `<name>.json` records the input pose, the resolved eye
//! and orientation in game coordinates (and the eye's latitude, longitude and
//! height), the build's commit, the full command line, the size, the hour,
//! the weather, every switch, and how the settle and the convergence went.
//!
//! Shots are given in a JSON array; each has a `name` (it names the files),
//! one pose, and optional `fov` (vertical, degrees), `far` (metres),
//! `time_of_day` (hours), `weather` (`clear`, `overcast`, `fog`, `drizzle`,
//! `rain`, `storm`) and `what` (a description):
//!
//! * `{"geo": {"lat", "lon", "alt", "range", "heading", "tilt", "fov"}}`: the
//!   look-at pose of the common web-globe URL convention (`eo_data::geo`):
//!   the target at (lat, lon, alt metres above mean sea level) seen from
//!   `range` metres, facing compass `heading`, `tilt` degrees from straight
//!   down. The same keys at the top level are read as a geo pose too.
//! * `{"eye": [x, y, z], "yaw", "pitch"}`: game metres (x east, y up, z
//!   south); yaw in degrees from north (-z) toward east (+x), pitch in
//!   degrees, up positive.
//! * `{"eye": [x, y, z], "look_at": [x, y, z]}`: game metres.

use super::{CAPTURE_WALL_WARN_M, blocked_view, draw, read_color, settle_p, write_png};
use crate::agents::AgentRuntime;
use crate::models::ModelRuntime;
use crate::props::PropsRuntime;
use crate::{Args, World};
use anyhow::{Context, Result, anyhow, bail};
use eo_agents::GpuAgent;
use eo_data::geo::{GeoFrame, look_at_camera};
use eo_render::camera::{Camera, GAME_FOV_Y_DEG, GAME_NEAR_M};
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_render::{GpuContext, Renderer, Targets, wgpu};
use eo_sky::Kind;
use glam::{DVec3, Vec3};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The water's clock for every still (as the sky and ocean cards).
pub const WATER_CLOCK_S: f64 = 20.0;
/// Frames of 1/60 s the rebuilt traffic is stepped before a still (it moves, its lights are on).
const AGENT_STEPS: usize = 120;
const FRAME_S: f64 = 1.0 / 60.0;
/// The temporal resolve: at least this many frames, compared one jitter cycle
/// (8 frames, the Halton sequence's period) apart, until the 8-bit RMS
/// difference is under `TAA_RMS`, or is stationary: over the last three
/// comparisons (24 frames) the RMS spread is within `TAA_STATIONARY` of its
/// mean and the share of channels more than 8 levels apart within half of
/// its. A history still converging shrinks by about half every comparison
/// (the resolve keeps 0.9 of it a frame); what is left when it stops is what
/// changes every frame (the post pass's display dither alone is about 0.7
/// RMS). At most `TAA_MAX_FRAMES`.
const TAA_MIN_FRAMES: usize = 48;
const TAA_STEP: usize = 8;
const TAA_MAX_FRAMES: usize = 320;
const TAA_RMS: f64 = 0.5;
const TAA_STATIONARY: f64 = 0.1;
/// A geodetic shot's far plane when neither the shot nor `--far` gives one:
/// the whole city and the bay (the game camera's 4200 m would cut a view
/// across the city short).
const SHOTS_FAR_M: f32 = 20000.0;
/// Pitch is kept this far off straight up and down (the view basis needs a horizontal).
const MAX_PITCH_DEG: f64 = 89.9;

pub enum Mode {
    File(PathBuf),
    SceneCards,
}

impl Mode {
    /// The output folder under `native/bench-out/` when `--out` is not given.
    pub fn default_dir(&self) -> &'static str {
        match self {
            Mode::File(_) => "shots",
            Mode::SceneCards => "scene-cards",
        }
    }
}

/// Pumps at most spent evicting what an earlier shot left resident (each evicts a few packs and models).
const STALE_PUMPS_MAX: usize = 2000;

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

pub fn arg_f64(name: &str) -> Result<Option<f64>> {
    arg(name).map(|v| v.trim().parse::<f64>().with_context(|| format!("{name} {v} is not a number"))).transpose()
}

pub fn requested() -> Option<Mode> {
    if let Some(f) = arg("--shots") {
        return Some(Mode::File(f.into()));
    }
    std::env::args().any(|a| a == "--scene-cards").then_some(Mode::SceneCards)
}

/// `--shots-only a,b`: only the shots whose name contains a or b.
pub fn wanted(name: &str) -> bool {
    arg("--shots-only").is_none_or(|o| o.split(',').map(str::trim).filter(|k| !k.is_empty()).any(|k| name.contains(k)))
}

/// Before the world loads: the shots file parses and something is selected.
pub fn preflight() -> Result<()> {
    if std::env::args().any(|a| a == "--shots") && arg("--shots").is_none() {
        bail!("--shots needs a file");
    }
    if let Some(Mode::File(p)) = requested() {
        let specs = load_specs(&p)?;
        if !specs.iter().any(|s| wanted(&s.name)) {
            bail!("{}: no shot selected ({} in the file, --shots-only {:?})", p.display(), specs.len(), arg("--shots-only"));
        }
        eprintln!("shots: {} shots in {}", specs.len(), p.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The shots file.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub enum Pose {
    /// The look-at pose: degrees, metres above mean sea level, metres.
    Geo { lat: f64, lon: f64, alt: f64, range: f64, heading: f64, tilt: f64 },
    /// Game metres; degrees.
    YawPitch { eye: DVec3, yaw: f64, pitch: f64 },
    LookAt { eye: DVec3, at: DVec3 },
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub name: String,
    pub pose: Pose,
    /// Vertical field of view, degrees.
    pub fov: f64,
    pub far: Option<f32>,
    pub hour: Option<f64>,
    pub weather: Option<Kind>,
    pub what: Option<String>,
    /// The entry as written.
    pub input: Value,
}

const SHOT_KEYS: &[&str] = &["name", "geo", "lat", "lon", "alt", "range", "heading", "tilt", "fov", "eye", "yaw", "pitch", "look_at", "far", "time_of_day", "weather", "what", "note"];
const GEO_KEYS: &[&str] = &["lat", "lon", "alt", "range", "heading", "tilt", "fov"];

fn unknown_keys(o: &Map<String, Value>, known: &[&str]) -> Vec<String> {
    o.keys().filter(|k| !known.contains(&k.as_str()) && !k.starts_with('_')).cloned().collect()
}

pub fn load_specs(path: &Path) -> Result<Vec<Spec>> {
    let bytes = std::fs::read(path).with_context(|| format!("read the shots file {}", path.display()))?;
    let v: Value = serde_json::from_slice(&bytes).with_context(|| format!("{} is not JSON", path.display()))?;
    parse_specs(&v).with_context(|| format!("shots file {}", path.display()))
}

pub fn parse_specs(v: &Value) -> Result<Vec<Spec>> {
    let arr = v.as_array().ok_or_else(|| anyhow!("a shots file is a JSON array of shots"))?;
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for (i, e) in arr.iter().enumerate() {
        let o = e.as_object().ok_or_else(|| anyhow!("shot {i} is not an object"))?;
        let name = o.get("name").and_then(Value::as_str).ok_or_else(|| anyhow!("shot {i} has no \"name\""))?.to_string();
        if name.is_empty() || name.starts_with('.') || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
            bail!("shot {i}: the name \"{name}\" names files: letters, digits, '-', '_' and '.' only");
        }
        if !seen.insert(name.clone()) {
            bail!("the shot name \"{name}\" is used twice");
        }
        let bad = unknown_keys(o, SHOT_KEYS);
        if !bad.is_empty() {
            bail!("shot \"{name}\": unknown keys {bad:?} (known: {SHOT_KEYS:?}; a key starting with '_' is a comment)");
        }
        let num = |m: &Map<String, Value>, k: &str| -> Result<Option<f64>> {
            match m.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(x) => x.as_f64().filter(|v| v.is_finite()).map(Some).ok_or_else(|| anyhow!("shot \"{name}\": \"{k}\" is not a number")),
            }
        };
        let need = |m: &Map<String, Value>, k: &str| -> Result<f64> { num(m, k)?.ok_or_else(|| anyhow!("shot \"{name}\": no \"{k}\"")) };
        let vec3 = |k: &str| -> Result<Option<DVec3>> {
            match o.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Array(a)) if a.len() == 3 && a.iter().all(|x| x.as_f64().is_some_and(f64::is_finite)) => Ok(Some(DVec3::new(a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()))),
                Some(_) => Err(anyhow!("shot \"{name}\": \"{k}\" is not [x, y, z]")),
            }
        };
        let geo = match o.get("geo") {
            Some(Value::Object(g)) => Some(g),
            Some(_) => bail!("shot \"{name}\": \"geo\" is not an object"),
            None if o.contains_key("lat") || o.contains_key("lon") => Some(o),
            None => None,
        };
        let (pose, geo_fov) = if let Some(g) = geo {
            if !std::ptr::eq(g, o) {
                let bad = unknown_keys(g, GEO_KEYS);
                if !bad.is_empty() {
                    bail!("shot \"{name}\": unknown \"geo\" keys {bad:?} (known: {GEO_KEYS:?})");
                }
            }
            let (lat, lon, range, tilt) = (need(g, "lat")?, need(g, "lon")?, need(g, "range")?, need(g, "tilt")?);
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                bail!("shot \"{name}\": lat {lat}, lon {lon} out of range");
            }
            if range < 0.0 {
                bail!("shot \"{name}\": range {range} is negative");
            }
            if !(0.0..=180.0).contains(&tilt) {
                bail!("shot \"{name}\": tilt {tilt} is not 0 (straight down) to 180 (straight up)");
            }
            (Pose::Geo { lat, lon, alt: need(g, "alt")?, range, heading: need(g, "heading")?, tilt }, num(g, "fov")?)
        } else if let Some(eye) = vec3("eye")? {
            if let Some(at) = vec3("look_at")? {
                if at.distance(eye) < 1e-6 {
                    bail!("shot \"{name}\": look_at is the eye");
                }
                (Pose::LookAt { eye, at }, None)
            } else {
                (Pose::YawPitch { eye, yaw: need(o, "yaw")?, pitch: need(o, "pitch")? }, None)
            }
        } else {
            bail!("shot \"{name}\": give \"geo\" (lat, lon, alt, range, heading, tilt), or \"eye\" with \"yaw\" and \"pitch\" or with \"look_at\"");
        };
        let fov = num(o, "fov")?.or(geo_fov).unwrap_or(GAME_FOV_Y_DEG as f64);
        if !(1.0..=170.0).contains(&fov) {
            bail!("shot \"{name}\": fov {fov} is not 1 to 170 degrees");
        }
        let hour = num(o, "time_of_day")?;
        if hour.is_some_and(|h| !(0.0..=24.0).contains(&h)) {
            bail!("shot \"{name}\": time_of_day {hour:?} is not 0 to 24");
        }
        let weather = match o.get("weather") {
            None | Some(Value::Null) => None,
            Some(Value::String(w)) => Some(Kind::parse(w).ok_or_else(|| anyhow!("shot \"{name}\": weather \"{w}\" is not clear, overcast, fog, drizzle, rain or storm"))?),
            Some(_) => bail!("shot \"{name}\": weather is not a string"),
        };
        let far = num(o, "far")?.map(|f| f as f32);
        if far.is_some_and(|f| f <= GAME_NEAR_M * 2.0) {
            bail!("shot \"{name}\": far {far:?} is too near");
        }
        let what = o.get("what").and_then(Value::as_str).map(String::from);
        out.push(Spec { name, pose, fov, far, hour, weather, what, input: e.clone() });
    }
    if out.is_empty() {
        bail!("no shots");
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Poses.
// ---------------------------------------------------------------------------

/// Yaw (0 north (-z), positive toward east (+x)) and pitch (up positive), radians, from `eye` toward `at`.
pub fn aim(eye: DVec3, at: DVec3, fallback_yaw: f64) -> (f64, f64) {
    let d = at - eye;
    let h = d.x.hypot(d.z);
    let yaw = if h > 1e-9 { d.x.atan2(-d.z) } else { fallback_yaw };
    let lim = MAX_PITCH_DEG.to_radians();
    (yaw, d.y.atan2(h).clamp(-lim, lim))
}

pub fn camera(eye: DVec3, yaw: f64, pitch: f64, fov: f64, far: f32, aspect: f32) -> Camera {
    Camera { eye: eye.as_vec3(), yaw: yaw as f32, pitch: pitch as f32, fov_y_deg: fov as f32, near: GAME_NEAR_M, far, aspect, jitter: [0.0; 2] }
}

fn compass(rad: f64) -> f64 {
    rad.to_degrees().rem_euclid(360.0)
}

fn geodetic_json(gf: &GeoFrame, world: &World, p: DVec3) -> Value {
    let (lat, lon, alt) = gf.from_game(p.to_array(), Some(&world.elevation));
    let g = world.elevation.sample(p.x, p.z);
    json!({ "lat": lat, "lon": lon, "alt_msl_m": alt, "drawn_ground_y": g, "above_drawn_ground_m": g.map(|g| p.y - g) })
}

/// A shot ready to take.
pub struct Shot {
    pub name: String,
    pub cam: Camera,
    pub hour: f64,
    pub weather: WeatherMode,
    /// The traffic's focus (x, z, view yaw atan2(dx, dz)): the look-at target, else the eye.
    pub focus: [f64; 3],
    /// The input, the resolved pose and anything else the manifest carries for this shot.
    pub info: Map<String, Value>,
    /// Extra instances drawn with the agents (a scene card's character).
    pub extras: Vec<GpuAgent>,
    pub notes: Vec<String>,
}

/// The resolved pose's manifest entries: eye, orientation (radians and
/// degrees, compass heading and tilt from straight down), lens and the eye's geodetic position.
pub fn pose_json(gf: &GeoFrame, world: &World, cam: &Camera, look_at: Option<DVec3>) -> Value {
    let f = cam.forward();
    let eye = cam.eye.as_dvec3();
    json!({
        "eye": [cam.eye.x, cam.eye.y, cam.eye.z],
        "yaw_rad": cam.yaw, "pitch_rad": cam.pitch,
        "yaw_deg": (cam.yaw as f64).to_degrees(), "pitch_deg": (cam.pitch as f64).to_degrees(),
        "heading_deg": compass(cam.yaw as f64), "tilt_deg": 90.0 + (cam.pitch as f64).to_degrees(),
        "forward": [f.x, f.y, f.z],
        "fov_y_deg": cam.fov_y_deg, "near": cam.near, "far": cam.far, "aspect": cam.aspect,
        "look_at": look_at.map(|p| [p.x, p.y, p.z]),
        "distance_to_look_at_m": look_at.map(|p| p.distance(eye)),
        "eye_geodetic": geodetic_json(gf, world, eye),
        "look_at_geodetic": look_at.map(|p| geodetic_json(gf, world, p)),
        "axes": "x east, y up, z south, metres; yaw 0 looks north (-z), positive toward east (+x); pitch up positive",
    })
}

/// The frame's conversion, for the manifest.
pub fn frame_json(gf: &GeoFrame) -> Value {
    json!({
        "projection": "sf-city.json meta: x = (lon - center.lon) * metersPerDegreeLon * ground_plan_scale, z = (center.lat - lat) * metersPerDegreeLat * ground_plan_scale",
        "center_lat": gf.plan.center_lat, "center_lon": gf.plan.center_lon,
        "meters_per_degree_lat": gf.plan.m_per_deg_lat, "meters_per_degree_lon": gf.plan.m_per_deg_lon,
        "ground_plan_scale": gf.scale, "height_scale": gf.height_scale,
        "vertical": format!("game y = alt_msl + ({:.2}) (mean sea level in game y; the elevation grid is NAVD88, mean sea level {:.3} m above NAVD88) + the drawn ground's lift over the surveyed ground there (heights above mean sea level x {}): a height above the surveyed ground is kept over the drawn ground", eo_data::SEA_LEVEL_Y, eo_data::SEA_LEVEL_Y - eo_data::NAVD88_TO_GAME_Y, gf.height_scale),
        "look_at": "camera = target - range * forward on the WGS84 ellipsoid, forward = (sin tilt sin heading east, sin tilt cos heading north, -cos tilt up); both points converted, the game camera looks at the target",
    })
}

/// A file shot resolved: the camera and what the manifest records of the pose.
pub fn resolve(spec: &Spec, world: &World, gf: &GeoFrame, args: &Args, aspect: f32, default_far: f32) -> Shot {
    let grid = Some(&world.elevation);
    let far = spec.far.unwrap_or(default_far);
    let mut info = Map::new();
    info.insert("input".into(), spec.input.clone());
    let (cam, look_at) = match spec.pose {
        Pose::Geo { lat, lon, alt, range, heading, tilt } => {
            let (clat, clon, calt) = look_at_camera(lat, lon, alt, range, heading, tilt);
            let t = gf.to_game(lat, lon, alt, grid);
            let c = gf.to_game(clat, clon, calt, grid);
            let (eye, at) = (DVec3::from(c.p), DVec3::from(t.p));
            let (yaw, pitch) = aim(eye, at, heading.to_radians());
            let cam = camera(eye, yaw, pitch, spec.fov, far, aspect);
            let point = |lat: f64, lon: f64, alt: f64, g: &eo_data::geo::GamePoint| json!({ "lat": lat, "lon": lon, "alt_msl_m": alt, "game": g.p, "surveyed_ground_msl_m": g.ground_msl, "drawn_ground_y": g.ground_y });
            let h = (at - eye).x.hypot((at - eye).z);
            info.insert(
                "geo".into(),
                json!({
                    "target": point(lat, lon, alt, &t),
                    "camera": point(clat, clon, calt, &c),
                    "input": { "range_m": range, "heading_deg": heading, "tilt_deg": tilt, "fov_deg": spec.fov },
                    "in_game": { "range_m": eye.distance(at), "horizontal_m": h, "heading_deg": compass(yaw), "tilt_deg": 90.0 + pitch.to_degrees() },
                    "frame": frame_json(gf),
                }),
            );
            (cam, Some(at))
        }
        Pose::YawPitch { eye, yaw, pitch } => {
            let lim = MAX_PITCH_DEG;
            (camera(eye, yaw.to_radians(), pitch.clamp(-lim, lim).to_radians(), spec.fov, far, aspect), None)
        }
        Pose::LookAt { eye, at } => {
            let (yaw, pitch) = aim(eye, at, 0.0);
            (camera(eye, yaw, pitch, spec.fov, far, aspect), Some(at))
        }
    };
    info.insert("resolved".into(), pose_json(gf, world, &cam, look_at));
    if let Some(w) = &spec.what {
        info.insert("what".into(), json!(w));
    }
    let f = cam.forward();
    let focus = match look_at {
        Some(p) => [p.x, p.z, (f.x as f64).atan2(f.z as f64)],
        None => AgentRuntime::focus_of(cam.eye, cam.forward()),
    };
    Shot {
        name: spec.name.clone(),
        cam,
        hour: spec.hour.unwrap_or(args.time_of_day),
        weather: spec.weather.map(WeatherMode::Preset).unwrap_or(args.look.sky.weather),
        focus,
        info,
        extras: vec![],
        notes: vec![],
    }
}

/// `--far` on the command line, else the larger of the game's and `SHOTS_FAR_M`.
fn file_far(args: &Args) -> f32 {
    if std::env::args().any(|a| a == "--far") { args.far } else { args.far.max(SHOTS_FAR_M) }
}

/// The camera the bench streams in at before the first shot.
pub fn first_camera(world: &World, args: &Args, aspect: f32) -> Result<Option<Camera>> {
    match requested() {
        Some(Mode::File(p)) => {
            let specs = load_specs(&p)?;
            let gf = GeoFrame::of_city(&world.city.meta);
            Ok(specs.iter().find(|s| wanted(&s.name)).map(|s| resolve(s, world, &gf, args, aspect, file_far(args)).cam))
        }
        Some(Mode::SceneCards) => Ok(super::scene_cards::first_camera(world, args, aspect)),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Taking a shot.
// ---------------------------------------------------------------------------

/// What a shot draws with.
pub struct Io<'a> {
    pub ctx: &'a GpuContext,
    pub renderer: &'a Renderer,
    pub world: &'a World,
    pub models: &'a mut ModelRuntime,
    pub props: &'a mut PropsRuntime,
    pub agents: &'a mut AgentRuntime,
    pub args: &'a Args,
    pub color: &'a wgpu::Texture,
    pub attach: &'a wgpu::TextureView,
    pub resolve: Option<&'a wgpu::TextureView>,
    pub targets: &'a Targets,
}

impl Io<'_> {
    pub fn aspect(&self) -> f32 {
        self.color.width() as f32 / self.color.height() as f32
    }

    /// The shot's hour and weather; the cloud clock pinned and held still.
    pub fn set_look(&self, hour: f64, weather: WeatherMode) {
        let base = self.args.look.sky;
        self.renderer.look.set_time_of_day(hour);
        if base != SkyOptions::off() {
            self.renderer.look.set_sky_options(SkyOptions { weather, ..base });
        }
        self.renderer.look.sky.set_clock(base.world_time_s, 0.0);
    }

    /// The traffic and people rebuilt round `focus` from the simulation's
    /// seeded warm-up (nothing posted while it loads), stepped `AGENT_STEPS`
    /// frames, then held until the instance list stops changing.
    pub fn place_agents(&mut self, focus: [f64; 3]) -> Value {
        if !self.agents.opts.on {
            return json!({ "on": false });
        }
        let t = Instant::now();
        self.agents.restart_sim(self.world, focus, self.agents.opts.density);
        while !self.agents.ready() && t.elapsed().as_secs_f64() < 300.0 {
            self.agents.pump(self.ctx, self.renderer, focus, 0.0);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if !self.agents.ready() {
            return json!({ "on": true, "ready": false, "errors": self.agents.errors });
        }
        for _ in 0..AGENT_STEPS {
            self.agents.pump(self.ctx, self.renderer, focus, FRAME_S);
        }
        self.hold_agents(focus);
        let snap = self.agents.snapshot();
        let lib = self.agents.lib.clone();
        let class = |a: &GpuAgent| lib.as_ref().and_then(|l| l.models.get(a.model as usize)).map(|m| m.class);
        let people = snap.iter().filter(|a| class(a) == Some(eo_agents::CLASS_PERSON)).count();
        let vehicles = snap.iter().filter(|a| class(a) == Some(eo_agents::CLASS_VEHICLE)).count();
        json!({
            "on": true, "ready": true, "density": self.agents.opts.density,
            "focus": focus, "steps": AGENT_STEPS, "sim_time_s": self.agents.sim_time,
            "instances": snap.len(), "vehicles": vehicles, "people": people,
            "people_drawable": self.people_drawable(),
            "people_source": lib.as_ref().map(|l| l.meta.people_source.clone()),
            "people_fallback": lib.as_ref().and_then(|l| l.meta.people_fallback.clone()),
            "rebuild_ms": t.elapsed().as_secs_f64() * 1e3,
        })
    }

    /// Pump with dt 0 until the instance list stops changing (the feed caught up).
    fn hold_agents(&mut self, focus: [f64; 3]) {
        let mut last: Vec<GpuAgent> = vec![];
        let mut same = 0;
        for _ in 0..400 {
            self.agents.pump(self.ctx, self.renderer, focus, 0.0);
            std::thread::sleep(std::time::Duration::from_millis(10));
            let now = self.agents.snapshot();
            if !now.is_empty() && now == last {
                same += 1;
                if same >= 4 {
                    return;
                }
            } else {
                same = 0;
            }
            last = now;
        }
    }

    /// The agent library has people models (the optional people assets are present).
    pub fn people_drawable(&self) -> bool {
        self.agents.lib.as_ref().is_some_and(|l| !l.meta.people_models.is_empty())
    }

    /// Stream until the view is resident (the upload queue empty, every
    /// tile the view wants in), the parked cars in, and again.
    pub fn settle(&mut self, cam: &Camera, timeout_s: f64) -> Result<Value> {
        let mut a = settle_p(self.ctx, self.renderer, self.world, self.models, Some(&mut *self.props), cam, timeout_s)?;
        // what an earlier shot's camera left resident (kit packs and models
        // no longer wanted here, evicted a few a pump) goes before the frame
        // converges, so a shot draws what this camera alone streams in, not
        // what the shot before it happened to leave (the convergence draws do not pump).
        let mut pumps = 0;
        while self.models.stale() > 0 && pumps < STALE_PUMPS_MAX {
            a += settle_p(self.ctx, self.renderer, self.world, self.models, Some(&mut *self.props), cam, timeout_s)?;
            pumps += 1;
        }
        crate::props::sync_parked(self.ctx, self.renderer, self.props, self.agents, cam);
        let b = settle_p(self.ctx, self.renderer, self.world, self.models, Some(&mut *self.props), cam, timeout_s)?;
        let settled = self.models.settled() && self.props.settled();
        Ok(json!({
            "ms": a + b, "timeout_s": timeout_s, "settled": settled, "stale_pumps": pumps, "stale_left": self.models.stale(),
            "unsettled": (!settled).then(|| self.models.unsettled()),
            "uploads_pending": self.models.uploads.pending(),
        }))
    }

    /// Draw until the temporal resolve has converged under the static camera, then write the PNG.
    pub fn converge_and_write(&self, cam: &Camera, png: &Path) -> Result<Value> {
        let mut frames = 0;
        let mut prev: Option<Vec<u8>> = None;
        let mut last = (f64::NAN, f64::NAN);
        let mut trace = vec![];
        let mut converged = None;
        let img = loop {
            for _ in 0..TAA_STEP {
                draw(self.ctx, self.renderer, self.attach, self.resolve, &self.targets.depth, cam);
                frames += 1;
            }
            if frames < TAA_MIN_FRAMES {
                continue;
            }
            let img = read_color(self.ctx, self.color)?;
            if let Some(p) = &prev {
                last = difference(p, &img);
                trace.push((frames, last.0, last.1));
                if last.0 <= TAA_RMS {
                    converged = Some("under the RMS");
                    break img;
                }
                if let [.., a, b, c] = trace.as_slice() {
                    let spread = |x: [f64; 3]| (x.iter().fold(f64::MIN, |m, v| m.max(*v)) - x.iter().fold(f64::MAX, |m, v| m.min(*v)), (x[0] + x[1] + x[2]) / 3.0);
                    let (rs, rm) = spread([a.1, b.1, c.1]);
                    let (ss, sm) = spread([a.2, b.2, c.2]);
                    if rs <= TAA_STATIONARY * rm && ss <= 0.5 * sm.max(1e-5) {
                        converged = Some("stationary: what is left changes every frame");
                        break img;
                    }
                }
            }
            if frames >= TAA_MAX_FRAMES {
                break img;
            }
            prev = Some(img);
        };
        let (w, h) = (self.color.width(), self.color.height());
        write_png(png, w, h, &img)?;
        // `EO_SHOTS_DIFF=1`: the last two compared frames' difference (x 16) beside the PNG, to see what still moves.
        if let (Some(p), true) = (&prev, std::env::var("EO_SHOTS_DIFF").is_ok_and(|v| v != "0")) {
            let d: Vec<u8> = p.chunks_exact(4).zip(img.chunks_exact(4)).flat_map(|(a, b)| [0, 1, 2].map(|k| (a[k].abs_diff(b[k]) as u32 * 16).min(255) as u8).into_iter().chain([255])).collect();
            write_png(&png.with_extension("diff.png"), w, h, &d)?;
        }
        Ok(json!({
            "frames": frames, "converged": converged.is_some(), "by": converged, "rms_8bit": last.0, "share_over_8_levels": last.1,
            "trace_frame_rms_share": trace.iter().map(|t| json!([t.0, (t.1 * 1e3).round() / 1e3, (t.2 * 1e6).round() / 1e6])).collect::<Vec<_>>(),
            "rule": format!("two frames {TAA_STEP} apart (one jitter cycle) under {TAA_RMS} RMS (8-bit), or the last three such differences within {} % of their mean RMS and half of their mean share of channels more than 8 levels apart; at least {TAA_MIN_FRAMES} and at most {TAA_MAX_FRAMES} frames, the camera still", TAA_STATIONARY * 100.0),
        }))
    }
}

/// RMS difference of two RGBA8 images over RGB (8-bit levels), and the share of channels more than 8 levels apart.
fn difference(a: &[u8], b: &[u8]) -> (f64, f64) {
    let (mut sum, mut over, mut n) = (0f64, 0usize, 0usize);
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for k in 0..3 {
            let d = pa[k] as f64 - pb[k] as f64;
            sum += d * d;
            over += (d.abs() > 8.0) as usize;
            n += 1;
        }
    }
    ((sum / n.max(1) as f64).sqrt(), over as f64 / n.max(1) as f64)
}

/// The building whose footprint holds `eye` below its top, if any (a pose
/// that stands inside a building sees its back faces).
pub fn inside_building(world: &World, eye: Vec3) -> Option<Value> {
    for b in &world.city.buildings {
        let Some(first) = b.ring.first() else { continue };
        let (mut mn, mut mx) = (*first, *first);
        for p in &b.ring {
            mn = [mn[0].min(p[0]), mn[1].min(p[1])];
            mx = [mx[0].max(p[0]), mx[1].max(p[1])];
        }
        if eye.x < mn[0] || eye.x > mx[0] || eye.z < mn[1] || eye.z > mx[1] {
            continue;
        }
        let mut inside = false;
        let n = b.ring.len();
        for i in 0..n {
            let (p, q) = (b.ring[i], b.ring[(i + n - 1) % n]);
            if (p[1] > eye.z) != (q[1] > eye.z) && eye.x < (q[0] - p[0]) * (eye.z - p[1]) / (q[1] - p[1]) + p[0] {
                inside = !inside;
            }
        }
        if !inside {
            continue;
        }
        let base = world.elevation.ground(((mn[0] + mx[0]) * 0.5) as f64, ((mn[1] + mx[1]) * 0.5) as f64) as f32;
        if eye.y < base + b.height + 0.5 {
            return Some(json!({ "building": b.id, "height_m": b.height, "top_y": base + b.height }));
        }
    }
    None
}

/// The build, read once: the commit it was built from and the checkout's state now.
pub fn build_json(repo: &Path) -> Value {
    let git = |a: &[&str]| -> Option<String> {
        let o = std::process::Command::new("git").arg("-C").arg(repo).args(a).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    json!({
        "commit": env!("EO_BUILD_COMMIT"),
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "checkout_commit": git(&["rev-parse", "HEAD"]),
        "checkout_changed_tracked_files": git(&["status", "--porcelain", "--untracked-files=no"]).map(|s| s.lines().filter(|l| !l.trim().is_empty()).count()),
    })
}

/// Every switch: the resolved arguments, the ground-plan factors, the far
/// layer's options and the environment switches (`EO_*`).
pub fn switches_json(args: &Args) -> Value {
    use eo_data::groundscale as gs;
    let env: std::collections::BTreeMap<String, String> = std::env::vars().filter(|(k, _)| k.starts_with("EO_")).collect();
    json!({
        "ground_plan_scale": gs::scale(), "ground_plan_scale_source": gs::source(),
        "height_scale": gs::height_scale(), "height_scale_source": gs::height_source(),
        "low_camera": gs::low_camera(), "people_scale": gs::people(), "vehicle_scale": gs::vehicles(),
        "models": args.models, "kit": args.kit, "streets": args.streets, "terrain_lod": args.terrain_lod,
        "props": args.props, "agents": args.agents, "agent_density": args.agent_density, "rocketbox": args.rocketbox,
        "landmarks": args.landmarks, "landmarks2": args.landmarks2, "msaa": args.msaa, "far_m": args.far,
        "look": format!("{:?}", args.look),
        "props2": format!("{:?}", args.props2),
        "perf": format!("{:?}", args.perf),
        "far_layer": format!("{:?}", eo_render::hlod::HlodOptions::from_env()),
        "args": format!("{args:?}"),
        "environment": env,
    })
}

/// What every manifest of a run shares.
pub fn common_json(io: &Io, mode: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("mode".into(), json!(mode));
    m.insert("build".into(), build_json(&io.world.paths.repo_root));
    m.insert("command_line".into(), json!(std::env::args().collect::<Vec<_>>()));
    m.insert("width".into(), json!(io.color.width()));
    m.insert("height".into(), json!(io.color.height()));
    m.insert("switches".into(), switches_json(io.args));
    m
}

fn weather_name(w: WeatherMode) -> String {
    match w {
        WeatherMode::Preset(k) => k.name().to_string(),
        WeatherMode::Schedule => "schedule".to_string(),
    }
}

/// Take one still: look, traffic, settle, converge, PNG and manifest.
pub fn shoot(io: &mut Io, common: &Map<String, Value>, shot: &Shot, dir: &Path, timeout_s: f64) -> Result<Value> {
    let t0 = Instant::now();
    let cam = shot.cam;
    eprintln!("shots: {}: eye ({:.1}, {:.1}, {:.1}), heading {:.1}, pitch {:.1}, {} h, {}", shot.name, cam.eye.x, cam.eye.y, cam.eye.z, compass(cam.yaw as f64), (cam.pitch as f64).to_degrees(), shot.hour, weather_name(shot.weather));
    io.set_look(shot.hour, shot.weather);
    let agents = io.place_agents(shot.focus);
    io.agents.extras = shot.extras.clone();
    let settle = io.settle(&cam, timeout_s)?;
    let mut notes = shot.notes.clone();
    if settle["settled"] != json!(true) {
        notes.push(format!("streaming did not settle within {timeout_s} s: {}", settle["unsettled"]));
    }
    let blocked = blocked_view(&cam, CAPTURE_WALL_WARN_M);
    if let Some(b) = &blocked {
        notes.push(format!("blocked view: {b}"));
    }
    let inside = inside_building(io.world, cam.eye);
    if let Some(b) = &inside {
        notes.push(format!("the eye is inside building {} (below its top, y {})", b["building"], b["top_y"]));
    }
    let ground = io.world.elevation.ground(cam.eye.x as f64, cam.eye.z as f64);
    if (cam.eye.y as f64) < ground {
        notes.push(format!("the eye is {:.1} m under the drawn ground", ground - cam.eye.y as f64));
    }
    let png = dir.join(format!("{}.png", shot.name));
    let taa = io.converge_and_write(&cam, &png)?;
    if taa["converged"] != json!(true) {
        notes.push(format!("the temporal resolve did not converge in {} frames (RMS {})", taa["frames"], taa["rms_8bit"]));
    }
    io.agents.extras.clear();
    let mut m = common.clone();
    m.insert("name".into(), json!(shot.name));
    m.insert("file".into(), json!(png.file_name().map(|f| f.to_string_lossy().to_string())));
    for (k, v) in &shot.info {
        m.insert(k.clone(), v.clone());
    }
    m.insert("time_of_day".into(), json!(shot.hour));
    m.insert("weather".into(), json!(weather_name(shot.weather)));
    m.insert("sky".into(), super::sky_shots::describe(io.renderer));
    m.insert("water_clock_s".into(), json!(WATER_CLOCK_S));
    m.insert("cloud_clock_s".into(), json!(io.args.look.sky.world_time_s));
    m.insert("agents".into(), agents);
    m.insert("settle".into(), settle);
    m.insert("taa".into(), taa);
    m.insert("blocked_view".into(), json!(blocked));
    m.insert("eye_inside_building".into(), json!(inside));
    m.insert("notes".into(), json!(notes));
    m.insert("shot_s".into(), json!(t0.elapsed().as_secs_f64()));
    let mp = dir.join(format!("{}.json", shot.name));
    std::fs::write(&mp, serde_json::to_string_pretty(&Value::Object(m.clone()))?)?;
    println!("wrote {} and {} ({:.1} s){}", png.display(), mp.display(), t0.elapsed().as_secs_f64(), if notes.is_empty() { String::new() } else { format!("; notes: {}", notes.join("; ")) });
    Ok(Value::Object(m))
}

pub fn run(io: &mut Io, mode: Mode, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    // A still must not depend on how long anything ran: the water's clock is fixed, its FFT runs every frame.
    if let Some(w) = io.renderer.water.get() {
        w.set_time(WATER_CLOCK_S);
        w.set_always(true);
    }
    let timeout = arg_f64("--settle-timeout")?.unwrap_or(600.0);
    let r = match mode {
        Mode::File(path) => {
            let specs = load_specs(&path)?;
            let gf = GeoFrame::of_city(&io.world.city.meta);
            let common = common_json(io, "shots");
            let (aspect, far) = (io.aspect(), file_far(io.args));
            let mut n = 0;
            for spec in specs.iter().filter(|s| wanted(&s.name)) {
                let mut shot = resolve(spec, io.world, &gf, io.args, aspect, far);
                shot.info.insert("shots_file".into(), json!(path.display().to_string()));
                shoot(io, &common, &shot, dir, timeout)?;
                n += 1;
            }
            println!("shots: {n} shots in {}", dir.display());
            Ok(())
        }
        Mode::SceneCards => super::scene_cards::run(io, dir, timeout),
    };
    let base = io.args.look.sky;
    io.renderer.look.set_time_of_day(io.args.time_of_day);
    io.renderer.look.set_sky_options(base);
    io.renderer.look.sky.set_clock(io.renderer.look.sky.world_time(), FRAME_S);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use eo_data::groundscale;
    use eo_data::roof_colours::PlanFrame;
    use eo_landmarks::data::LandmarksJs;

    fn frame() -> GeoFrame {
        let f = eo_data::Frame::sf(); // the city's projection (eo-data's geo test checks it against sf-city.json)
        GeoFrame::new(PlanFrame { center_lat: f.center_lat, center_lon: f.center_lon, m_per_deg_lat: f.m_per_deg_lat, m_per_deg_lon: f.m_per_deg_lon }, groundscale::scale(), groundscale::height_scale())
    }

    /// The landmark buildings are built at their surveyed site and moved
    /// whole onto the scaled plan (`eo_landmarks::set::ground_scale_shift`):
    /// the site's latitude and longitude (the existing unprojection of the
    /// surveyed site) must convert to where the game draws it.
    #[test]
    fn landmark_sites_land_where_their_data_puts_them() {
        let js = LandmarksJs::get();
        let gf = frame();
        let s = gf.scale;
        for id in ["transamerica-pyramid", "coit-tower", "salesforce-tower"] {
            let a = &js.registry[id].anchor;
            let anchor = [a["x"].as_f64().unwrap(), a["z"].as_f64().unwrap()];
            let site = &js.buildings[id];
            // The generator's own centre: Transamerica's frame, Coit's centre, Salesforce's record centroid.
            let centre = if let Some(f) = site.get("frame") {
                [f["cx"].as_f64().unwrap(), f["cz"].as_f64().unwrap()]
            } else if let Some(c) = site.get("centre").and_then(Value::as_array) {
                [c[0].as_f64().unwrap(), c[1].as_f64().unwrap()]
            } else {
                let p: Vec<f64> = site["record"]["points"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                let n = (p.len() / 2) as f64;
                [p.iter().step_by(2).sum::<f64>() / n, p.iter().skip(1).step_by(2).sum::<f64>() / n]
            };
            assert!((centre[0] - anchor[0]).hypot(centre[1] - anchor[1]) < 1e-6, "{id}: the generator's centre {centre:?} is the registry anchor {anchor:?}");
            let d = eo_landmarks::set::ground_scale_shift(js, id);
            let drawn = [anchor[0] + d[0], anchor[1] + d[1]];
            let (lon, lat) = gf.plan.to_lonlat(anchor[0], anchor[1]);
            let g = gf.plan_xz(lat, lon);
            let err = (g[0] - drawn[0]).hypot(g[1] - drawn[1]);
            eprintln!("landmark {id}: surveyed site ({:.2}, {:.2}) = lat {lat:.6}, lon {lon:.6}; converted to game ({:.2}, {:.2}); drawn at ({:.2}, {:.2}); error {err:.2e} m (scale {s})", anchor[0], anchor[1], g[0], g[1], drawn[0], drawn[1]);
            assert!(err < 1e-6, "{id}: lat/lon converts {err} m from where the game draws the landmark");
            // The footprint is real size about the anchor, so a corner of the
            // record converts (s - 1) x its distance from the anchor away from where it is drawn.
            let p: Vec<f64> = site["record"]["points"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            let mut worst = 0f64;
            for c in p.chunks_exact(2) {
                let (lon, lat) = gf.plan.to_lonlat(c[0], c[1]);
                let g = gf.plan_xz(lat, lon);
                let drawn = [c[0] + d[0], c[1] + d[1]];
                let off = (g[0] - drawn[0]).hypot(g[1] - drawn[1]);
                let r = (c[0] - anchor[0]).hypot(c[1] - anchor[1]);
                assert!((off - (s - 1.0) * r).abs() < 1e-6, "{id}: corner {c:?} {off} m off, expected {}", (s - 1.0) * r);
                worst = worst.max(off);
            }
            eprintln!("landmark {id}: footprint corners at most {worst:.2} m from their converted position (the building is real size on the {s} x plan)");
        }
    }

    #[test]
    fn shot_files_parse_both_geo_forms_and_local_poses() {
        let v: Value = serde_json::from_str(
            r#"[
                {"name": "a", "geo": {"lat": 37.7952, "lon": -122.4028, "alt": 110, "range": 800, "fov": 35, "heading": 30, "tilt": 75}, "time_of_day": 13, "weather": "clear"},
                {"name": "b", "lat": 37.7952, "lon": -122.4028, "alt": 110, "range": 800, "fov": 35, "heading": 30, "tilt": 75},
                {"name": "c", "eye": [1, 2, 3], "yaw": 90, "pitch": -10, "fov": 50},
                {"name": "d", "eye": [1, 2, 3], "look_at": [11, 2, 3], "_comment": "x"}
            ]"#,
        )
        .unwrap();
        let s = parse_specs(&v).unwrap();
        assert!(matches!(s[0].pose, Pose::Geo { tilt, .. } if tilt == 75.0) && s[0].fov == 35.0 && s[0].hour == Some(13.0) && s[0].weather == Some(Kind::Clear));
        assert!(matches!(s[1].pose, Pose::Geo { heading, .. } if heading == 30.0) && s[1].fov == 35.0);
        assert!(matches!(s[2].pose, Pose::YawPitch { yaw, pitch, .. } if yaw == 90.0 && pitch == -10.0));
        assert!(matches!(s[3].pose, Pose::LookAt { .. }) && s[3].fov == GAME_FOV_Y_DEG as f64);
        // look_at due east: yaw 90 degrees, level.
        let (yaw, pitch) = aim(DVec3::new(1.0, 2.0, 3.0), DVec3::new(11.0, 2.0, 3.0), 0.0);
        assert!((yaw.to_degrees() - 90.0).abs() < 1e-9 && pitch.abs() < 1e-12);
        // north is -z.
        let (yaw, _) = aim(DVec3::ZERO, DVec3::new(0.0, 0.0, -5.0), 1.0);
        assert!(yaw.abs() < 1e-12);
        for bad in [r#"[{"name": "x", "eye": [1, 2, 3]}]"#, r#"[{"name": "x", "geo": {"lat": 1, "lon": 2}}]"#, r#"[{"name": "x/y", "eye": [1, 2, 3], "yaw": 0, "pitch": 0}]"#, r#"[{"name": "x", "eye": [1, 2, 3], "yaw": 0, "pitch": 0, "tlit": 3}]"#, r#"[{"name": "x", "eye": [0, 0, 0], "yaw": 0, "pitch": 0}, {"name": "x", "eye": [0, 0, 0], "yaw": 0, "pitch": 0}]"#] {
            assert!(parse_specs(&serde_json::from_str(bad).unwrap()).is_err(), "{bad}");
        }
    }

    /// A geo pose in the game looks at its target, with the bearing kept.
    #[test]
    fn a_geo_pose_keeps_its_bearing_and_looks_at_its_target() {
        let gf = frame();
        for (heading, tilt) in [(30.0, 75.0), (320.0, 86.0), (160.0, 85.0), (281.0, 85.0)] {
            let (lat, lon, alt) = (37.7952, -122.4028, 110.0);
            let (clat, clon, calt) = look_at_camera(lat, lon, alt, 800.0, heading, tilt);
            let t = DVec3::from(gf.to_game(lat, lon, alt, None).p);
            let c = DVec3::from(gf.to_game(clat, clon, calt, None).p);
            let (yaw, pitch) = aim(c, t, 0.0);
            // The survey frame's metres per degree differ from the ellipsoid's by
            // 0.1 % (east) and 0.4 % (north), so the bearing moves by under 0.15 degrees.
            let dh = (compass(yaw) - heading + 540.0).rem_euclid(360.0) - 180.0;
            assert!(dh.abs() < 0.15, "heading {heading}: {}", compass(yaw));
            assert!(pitch < 0.0 && (90.0 + pitch.to_degrees()) > tilt - 0.5, "tilt {tilt}: {}", 90.0 + pitch.to_degrees());
            let cam = camera(c, yaw, pitch, 35.0, 20000.0, 16.0 / 9.0);
            let to = (t - c).normalize().as_vec3();
            assert!(cam.forward().dot(to) > 0.999_999, "the camera looks at its target");
        }
    }
}
