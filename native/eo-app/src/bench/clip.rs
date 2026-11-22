//! Moving clips along a camera path: `eo-app --clip FILE.json --out DIR`
//! renders every frame at 60 Hz of wall time (streaming, traffic, water and
//! clouds running, the temporal resolve in motion, as card 8 of the scene
//! cards does) and writes every `60 / fps`-th as `DIR/<name>/NNNN.png`, with
//! `DIR/<name>.json` recording each written frame's camera, the late frames
//! and the streaming state. It is how a level-of-detail switch, a thin
//! member's anti-aliasing or a tile boundary is judged in motion, which a
//! still cannot show.
//!
//! FILE.json is a clip or an array of clips:
//! `{"name", "seconds", "fps" (10), "time_of_day" (13), "fov" (degrees),
//! "day_of_year" (1-365), "sky" ("cloudless", "cirrus" or "fair_cumulus"),
//! "keys": [{"t" (seconds), "eye": [x, y, z], "look_at": [x, y, z]}, ...]}`
//! in game metres (x east, y up, z south). `day_of_year` and `sky` are read
//! and applied as a still's (shots.rs `day_and_sky`, `apply_day_and_sky`;
//! `--no-photo-date` ignores the date); without them the clip is drawn on the
//! game's date with the sky that date draws, as before. The eye and the target run
//! through the keys on a Catmull-Rom spline (uniform in key index, timed by
//! `t`), so the camera's speed is continuous through each key.

use super::shots::{self, Io, WATER_CLOCK_S};
use super::{read_color, write_png};
use anyhow::{Context, Result, anyhow, bail};
use eo_render::camera::Camera;
use eo_render::sky::WeatherMode;
use eo_sky::Kind;
use glam::DVec3;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Key {
    t: f64,
    eye: DVec3,
    at: DVec3,
}

struct Clip {
    name: String,
    seconds: f64,
    fps: f64,
    hour: f64,
    fov: f64,
    /// The day of the year the sun is placed on (`day_of_year`).
    day: Option<f64>,
    /// A clear day's sky (`sky`).
    sky: Option<eo_sky::clouds::SkyState>,
    keys: Vec<Key>,
}

fn load(path: &Path) -> Result<Vec<Clip>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let v: Value = serde_json::from_str(&text).with_context(|| format!("{} does not parse", path.display()))?;
    parse(v).with_context(|| format!("clip file {}", path.display()))
}

fn parse(v: Value) -> Result<Vec<Clip>> {
    let list = match v {
        Value::Array(a) => a,
        o => vec![o],
    };
    let v3 = |k: &Value, name: &str| -> Result<DVec3> {
        let a = k.get(name).and_then(Value::as_array).ok_or_else(|| anyhow!("a key has no {name}"))?;
        let n = |i: usize| a.get(i).and_then(Value::as_f64).ok_or_else(|| anyhow!("{name}[{i}] is not a number"));
        Ok(DVec3::new(n(0)?, n(1)?, n(2)?))
    };
    let mut out = vec![];
    for c in list {
        let name = c.get("name").and_then(Value::as_str).ok_or_else(|| anyhow!("a clip has no name"))?.to_string();
        let mut keys = vec![];
        for k in c.get("keys").and_then(Value::as_array).ok_or_else(|| anyhow!("clip {name}: no keys"))? {
            keys.push(Key { t: k.get("t").and_then(Value::as_f64).ok_or_else(|| anyhow!("clip {name}: a key has no t"))?, eye: v3(k, "eye")?, at: v3(k, "look_at")? });
        }
        if keys.len() < 2 || keys.windows(2).any(|w| w[1].t <= w[0].t) {
            bail!("clip {name}: give two or more keys in increasing t");
        }
        let f = |k: &str, d: f64| c.get(k).and_then(Value::as_f64).unwrap_or(d);
        let seconds = f("seconds", keys[keys.len() - 1].t);
        let (day, sky) = shots::day_and_sky(c.as_object().ok_or_else(|| anyhow!("clip {name} is not an object"))?, &format!("clip {name}"))?;
        out.push(Clip { name, seconds: seconds.clamp(0.1, 600.0), fps: f("fps", 10.0).clamp(1.0, 60.0), hour: f("time_of_day", 13.0), fov: f("fov", eo_render::camera::GAME_FOV_Y_DEG as f64), day, sky, keys });
    }
    Ok(out)
}

/// The path at time `t`: Catmull-Rom through the keys (the end keys repeated).
fn sample(keys: &[Key], t: f64, pick: impl Fn(&Key) -> DVec3) -> DVec3 {
    let n = keys.len();
    let t = t.clamp(keys[0].t, keys[n - 1].t);
    let i = keys.partition_point(|k| k.t <= t).clamp(1, n - 1) - 1;
    let u = ((t - keys[i].t) / (keys[i + 1].t - keys[i].t)).clamp(0.0, 1.0);
    let p = |j: isize| pick(&keys[j.clamp(0, n as isize - 1) as usize]);
    let (p0, p1, p2, p3) = (p(i as isize - 1), p(i as isize), p(i as isize + 1), p(i as isize + 2));
    let (u2, u3) = (u * u, u * u * u);
    0.5 * ((2.0 * p1) + (p2 - p0) * u + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * u3)
}

fn camera_at(c: &Clip, t: f64, far: f32, aspect: f32) -> Camera {
    let eye = sample(&c.keys, t, |k| k.eye);
    let at = sample(&c.keys, t, |k| k.at);
    let d = (at - eye).normalize_or(DVec3::NEG_Z);
    let (yaw, pitch) = (d.x.atan2(-d.z), d.y.clamp(-0.999, 0.999).asin());
    shots::camera(eye, yaw, pitch, c.fov, far, aspect)
}

/// The camera the bench streams in at: the first clip's first frame.
pub fn first_camera(path: &Path, far: f32, aspect: f32) -> Result<Option<Camera>> {
    Ok(load(path)?.first().map(|c| camera_at(c, c.keys[0].t, far, aspect)))
}

pub fn run(io: &mut Io, path: &PathBuf, dir: &Path, timeout: f64) -> Result<()> {
    let clips = load(path)?;
    std::fs::create_dir_all(dir)?;
    let far = io.args.far.max(20000.0);
    for c in clips.iter().filter(|c| shots::wanted(&c.name)) {
        one(io, c, dir, timeout, far)?;
    }
    Ok(())
}

fn one(io: &mut Io, c: &Clip, dir: &Path, timeout: f64, far: f32) -> Result<()> {
    let aspect = io.aspect();
    let every = (60.0 / c.fps).round().max(1.0) as usize;
    let frames = (c.seconds * 60.0).round() as usize;
    let t0k = c.keys[0].t;
    let cam_at = |i: usize| camera_at(c, t0k + i as f64 / 60.0, far, aspect);
    let seq = dir.join(&c.name);
    std::fs::create_dir_all(&seq)?;
    let cam0 = cam_at(0);
    eprintln!("clip: {}: {frames} frames at 60 Hz, writing every {every}", c.name);
    // the clip's date and sky (a still's, shots.rs); absent, the state is left as it is (the game's date)
    if (c.day.is_some() || c.sky.is_some()) && shots::apply_day_and_sky(c.day, c.sky, None, &io.renderer.look.sky.weather()) {
        // a new date or sky at the same hour: make the look frame again (`set_look` sets the hour back)
        io.renderer.look.set_time_of_day(c.hour + 1e-9);
    }
    io.set_look(c.hour, WeatherMode::Preset(Kind::Clear));
    let focus = crate::agents::AgentRuntime::focus_of(cam0.eye, cam0.forward());
    let agents = io.place_agents(focus);
    let settle = io.settle(&cam0, timeout)?;
    for _ in 0..48 {
        super::draw(io.ctx, io.renderer, io.attach, io.resolve, &io.targets.depth, &cam0);
    }
    let (w, h) = (io.color.width(), io.color.height());
    let (tx, rx) = std::sync::mpsc::sync_channel::<(PathBuf, Vec<u8>)>(8);
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    let writers: Vec<_> = (0..4)
        .map(|_| {
            let rx = rx.clone();
            std::thread::spawn(move || -> Result<()> {
                loop {
                    let job = rx.lock().map_err(|_| anyhow!("writer lock"))?.recv();
                    let Ok((p, rgba)) = job else { return Ok(()) };
                    write_png(&p, w, h, &rgba)?;
                }
            })
        })
        .collect();
    let base = io.args.look.sky.world_time_s;
    io.renderer.look.sky.set_clock(base, 1.0 / 60.0);
    let view_t0 = super::view_bench::clock();
    let t0 = Instant::now();
    let mut subs: Vec<eo_render::wgpu::SubmissionIndex> = Vec::with_capacity(frames + 1);
    let (mut records, mut late_max) = (vec![], 0f64);
    for i in 0..=frames {
        let due = t0 + Duration::from_secs_f64(i as f64 / 60.0);
        let now = Instant::now();
        if due > now {
            std::thread::sleep(due - now);
        }
        let late = Instant::now().saturating_duration_since(due).as_secs_f64() * 1e3;
        late_max = late_max.max(late);
        if i >= 2 {
            io.ctx.device.poll(eo_render::wgpu::PollType::Wait { submission_index: Some(subs[i - 2].clone()), timeout: None }).map_err(|e| anyhow!("{e}"))?;
        }
        let cam = cam_at(i);
        io.renderer.observe_view(&cam, Some(view_t0 + i as f64 / 60.0));
        io.models.set_camera(cam.eye);
        let pf = io.models.pump(io.ctx, io.renderer, io.world, cam.eye)?;
        if io.props.on {
            io.props.pump(io.ctx, io.renderer, &mut io.models.uploads, cam.eye, Some(io.agents.sim_time), 1.0 / 60.0);
            io.agents.parked = io.props.parked_agents(io.agents.lib.as_deref(), cam.eye).to_vec();
        }
        io.agents.pump(io.ctx, io.renderer, crate::agents::AgentRuntime::focus_of(cam.eye, cam.forward()), 1.0 / 60.0);
        if let Some(wt) = io.renderer.water.get() {
            wt.set_time(WATER_CLOCK_S + i as f64 / 60.0);
        }
        io.renderer.update(&io.ctx.queue, &cam);
        let mut enc = io.ctx.device.create_command_encoder(&Default::default());
        io.renderer.encode(&mut enc, io.attach, io.resolve, &io.targets.depth, None);
        subs.push(eo_render::pace::submit(&io.ctx.queue, [enc.finish()]));
        if i % every == 0 {
            let n = i / every;
            let rgba = read_color(io.ctx, io.color)?;
            tx.send((seq.join(format!("{n:04}.png")), rgba)).map_err(|_| anyhow!("the PNG writers stopped"))?;
            records.push(json!({
                "frame": n, "file": format!("{}/{n:04}.png", c.name), "sim_frame": i, "t_s": i as f64 / 60.0,
                "eye": cam.eye.to_array(), "yaw_rad": cam.yaw, "pitch_rad": cam.pitch,
                "uploads_pending": io.models.uploads.pending(), "streaming_settled": io.models.settled(), "bytes_uploaded": pf.bytes, "late_ms": late,
            }));
        }
    }
    io.ctx.device.poll(eo_render::wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    drop(tx);
    for wr in writers {
        wr.join().map_err(|_| anyhow!("a PNG writer panicked"))??;
    }
    super::view_bench::set_clock(view_t0 + frames as f64 / 60.0);
    io.renderer.look.sky.set_clock(base, 0.0);
    let achieved_fps = frames as f64 / t0.elapsed().as_secs_f64().max(1e-9);
    let mut m = shots::common_json(io, "clip");
    m.insert("name".into(), json!(c.name));
    m.insert(
        "sequence".into(),
        json!({
            "seconds": c.seconds, "sim_fps": 60, "written_fps": 60.0 / every as f64, "written_frames": records.len(), "fov_deg": c.fov,
            "keys": c.keys.iter().map(|k| json!({ "t": k.t, "eye": k.eye.to_array(), "look_at": k.at.to_array() })).collect::<Vec<_>>(),
            "pacing": "every frame waits for its 1/60 s slot of wall time; the camera is placed by the frame index; streaming, traffic, props, water and clouds run as in play",
            "latest_start_ms": late_max, "achieved_fps": achieved_fps,
        }),
    );
    m.insert("time_of_day".into(), json!(c.hour));
    m.insert("day_of_year".into(), json!(eo_look::sun::day_of_year()));
    m.insert("sky_state_input".into(), json!(c.sky.map(|k| k.name())));
    m.insert("weather".into(), json!("clear"));
    m.insert("sky".into(), super::sky_shots::describe(io.renderer));
    m.insert("agents".into(), agents);
    m.insert("settle_at_start".into(), settle);
    m.insert("frames".into(), json!(records));
    let mp = dir.join(format!("{}.json", c.name));
    std::fs::write(&mp, serde_json::to_string_pretty(&Value::Object(m))?)?;
    println!("wrote {} frames to {} and {} ({achieved_fps:.1} Hz achieved)", records.len(), seq.display(), mp.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: &str = r#""keys": [{"t": 0, "eye": [0, 2, 0], "look_at": [0, 2, -1]}, {"t": 1, "eye": [0, 2, 0], "look_at": [1, 2, -1]}]"#;

    #[test]
    fn a_clip_names_its_day_and_its_sky() {
        let v: Value = serde_json::from_str(&format!(r#"[{{"name": "a", "day_of_year": 148, "sky": "cloudless", {KEYS}}}, {{"name": "b", {KEYS}}}]"#)).unwrap();
        let c = parse(v).unwrap();
        assert_eq!((c[0].day, c[0].sky), (Some(148.0), Some(eo_sky::clouds::SkyState::Cloudless)));
        // without the keys nothing is set: the clip is drawn as before
        assert_eq!((c[1].day, c[1].sky), (None, None));
        for bad in [r#""day_of_year": 0"#, r#""day_of_year": "may""#, r#""sky": "storm""#, r#""sky": 3"#] {
            let v: Value = serde_json::from_str(&format!(r#"{{"name": "a", {bad}, {KEYS}}}"#)).unwrap();
            assert!(parse(v).is_err(), "{bad}");
        }
    }
}
