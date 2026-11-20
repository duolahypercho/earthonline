//! `--smooth-time` (`LightingOptions::smooth_time`): the drawn hour follows a
//! moving clock without rebuilding the look every frame.
//!
//! `LookGpu::set_time_of_day_smooth` keeps keyframes of the look (the frame
//! and, with any sky stage on, the sky terms) at clock times
//! `KEY_INTERVAL_H` apart, and draws the hour between the two that bracket
//! it as their blend (`eo_look::blend::frame`, `eo_sky::frame::blend_terms`):
//! the colours, the SH, the fog, the atmosphere and the tints are linear
//! between them, the key, rim and moon directions turn through a slerp, the
//! sun is placed at the exact hour. What is per frame (the camera, the
//! cascades, the exposure's adaptation) stays per frame in `LookGpu::update`.
//!
//! A keyframe is the full build at its hour (`build_key`: the sky integration
//! and `eo_sky::frame::apply_with`, smooth). One is built on the render thread
//! at most once a frame; the next one ahead is built on a worker
//! (`eo-look-key`) while the clock runs through the pair, so a clock moving at
//! its own pace never builds on the render thread. A keyframe is kept while
//! its inputs hold (`KeyInputs`: the weather's key, the moon's day, the probe
//! transfer); a change drops them all, as the hard path rebuilds its frame.

use eo_look::LookFrame;
use eo_look::rig::Interp;
use eo_sky::SkyTerms;
use eo_sky::frame::FrameOptions;
use std::sync::{Arc, Mutex};

/// Clock time between two keyframes: one clock minute.
pub const KEY_INTERVAL_H: f64 = 1.0 / 60.0;
/// Keyframes in a day (the index wraps with the hour).
pub const KEYS_PER_DAY: i64 = 1440;
/// All three sun cascades are refitted at once when the key light has turned
/// this far from the direction one of them was last fitted with (the
/// staggered refresh fits one a frame with the light of that frame, so at
/// the clock's own pace this never fires; a fast clock or the sun-to-moon
/// handover does).
pub const REFIT_DEG: f64 = 0.25;
/// At most this many frames in a row may move the scene probe's face off
/// (`ProbeGpu::defer_this_frame_within`) while the clock moves, so a capture
/// finishes within 6 x (this + 1) frames whatever the cascades do.
pub const PROBE_DEFER_MAX: u32 = 2;

/// What a keyframe was built for besides its hour.
#[derive(Clone, Copy, Debug)]
pub struct KeyInputs {
    /// The weather in force (None: every sky stage off, the JS clear frame).
    pub weather: Option<eo_sky::WeatherState>,
    /// `sky::weather_key` of that weather (the hard path rebuilds on its change too).
    pub weather_key: (i64, i64, i64),
    pub moon_day: f64,
    /// The probe's measured transfer (None: the fallback yield).
    pub transfer: Option<f64>,
    /// `--look cinematic` (stage L4): the frame's golden hour (eo-sky `FrameOptions::cinematic`).
    pub cinematic: bool,
}

impl KeyInputs {
    fn same(&self, o: &KeyInputs) -> bool {
        self.weather.is_some() == o.weather.is_some() && self.weather_key == o.weather_key && self.moon_day == o.moon_day && self.transfer == o.transfer && self.cinematic == o.cinematic
    }
}

/// One keyframe: the look frame and, with any sky stage on, its terms.
pub type Key = (LookFrame, Option<SkyTerms>);

/// The keyframe at clock time `index x KEY_INTERVAL_H` (pure; the render
/// thread or a worker). The hard path's frame at that hour, built with the
/// smooth options (cubic rig, blended moon key, ramped vehicle lamps).
pub fn build_key(index: i64, inputs: &KeyInputs) -> Key {
    let hour = index as f64 * KEY_INTERVAL_H;
    match inputs.weather {
        None => (LookFrame::with_sky_profile_interp(hour, inputs.transfer, eo_look::sky::Profile::clear_with_look(&eo_look::look::LOOK), Interp::MonotoneCubic), None),
        Some(w) => {
            let c = eo_sky::Conditions { hour, weather: w, moon_day: inputs.moon_day };
            let (f, t) = eo_sky::frame::look_frame_with(&c, inputs.transfer, FrameOptions { cinematic: inputs.cinematic, ..FrameOptions::SMOOTH });
            (f, Some(t))
        }
    }
}

/// Two keyframes `t` (0..1) of the way from `a` to `b`, drawn at `hour`.
pub fn blend_key(a: &Key, b: &Key, t: f64, hour: f64) -> Key {
    let f = eo_look::blend::frame(&a.0, &b.0, t, hour);
    let terms = match (&a.1, &b.1) {
        (Some(x), Some(y)) => Some(eo_sky::frame::blend_terms(x, y, t)),
        (x, _) => *x,
    };
    (f, terms)
}

fn same_index(a: i64, b: i64) -> bool {
    (a - b).rem_euclid(KEYS_PER_DAY) == 0
}

/// The worker building the keyframe after the pair: its index, its inputs, and where it puts the result.
type Pending = (i64, KeyInputs, Arc<Mutex<Option<Key>>>);

/// The soft path's keyframes.
#[derive(Default)]
pub struct Keys {
    inputs: Option<KeyInputs>,
    /// (index, keyframe) for the inputs in force: the bracketing pair, the one before, the one after.
    keys: Vec<(i64, Key)>,
    pending: Option<Pending>,
    /// The frame (`LookGpu`'s frame counter) of the last build on the calling thread.
    built_in: Option<u32>,
    /// Keyframes built on the calling thread, and taken from the worker.
    pub built: u64,
    pub prefetched: u64,
}

impl Keys {
    /// Forget every keyframe (the hard path took over).
    pub fn clear(&mut self) {
        self.keys.clear();
        self.pending = None;
        self.inputs = None;
    }

    fn get(&self, k: i64) -> Option<&Key> {
        self.keys.iter().find(|(i, _)| same_index(*i, k)).map(|(_, v)| v)
    }

    /// Pick up the worker's keyframe if it is done and still wanted.
    fn adopt(&mut self, lo: i64) {
        let done = match &self.pending {
            Some((_, _, slot)) => slot.lock().unwrap().take(),
            None => return,
        };
        let Some(key) = done else { return };
        let (k, inputs, _) = self.pending.take().unwrap();
        if self.inputs.is_some_and(|i| i.same(&inputs)) && (lo - 1..=lo + 2).any(|j| same_index(j, k)) && self.get(k).is_none() {
            self.keys.push((k, key));
            self.prefetched += 1;
        }
    }

    /// Start the worker on keyframe `k` unless it is there or under way.
    fn prefetch(&mut self, k: i64, inputs: KeyInputs) {
        if self.get(k).is_some() || self.pending.as_ref().is_some_and(|(i, inp, _)| same_index(*i, k) && inp.same(&inputs)) {
            return;
        }
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let spawned = std::thread::Builder::new().name("eo-look-key".into()).spawn(move || {
            crate::hitch::worker_started();
            let key = build_key(k, &inputs);
            *out.lock().unwrap() = Some(key);
        });
        self.pending = spawned.ok().map(|_| (k, inputs, slot));
    }

    /// The look at `hour` (0..24) for `inputs`, in frame `frame` (at most one
    /// keyframe is built on this thread a frame, unless there is none to draw
    /// with). `prefetch`: start the worker on the next keyframe.
    pub fn frame(&mut self, hour: f64, inputs: KeyInputs, frame: u32, prefetch: bool) -> Key {
        if !self.inputs.is_some_and(|i| i.same(&inputs)) {
            self.inputs = Some(inputs);
            self.keys.clear();
            self.pending = None;
        }
        let x = hour / KEY_INTERVAL_H;
        let lo = x.floor() as i64;
        let t = (x - lo as f64).clamp(0.0, 1.0);
        // keyframes away from this hour go (the clock jumped).
        self.keys.retain(|(k, _)| (lo - 1..=lo + 2).any(|j| same_index(j, *k)));
        self.adopt(lo);
        // the nearer of the pair first: after a jump, until the other is built,
        // the frame is the nearer keyframe's (at most half an interval off).
        let order = if t < 0.5 { [lo, lo + 1] } else { [lo + 1, lo] };
        for k in order {
            let none = self.get(lo).is_none() && self.get(lo + 1).is_none();
            if self.get(k).is_none() && (self.built_in != Some(frame) || none) {
                let _span = crate::hitch::span(crate::hitch::Phase::LookFrame);
                crate::hitch::mark(crate::hitch::Mark::LookFrameRebuilt, 1);
                let key = build_key(k, &inputs);
                self.keys.push((k, key));
                self.built_in = Some(frame);
                self.built += 1;
            }
        }
        let out = match (self.get(lo), self.get(lo + 1)) {
            (Some(a), Some(b)) => blend_key(a, b, t, hour),
            (Some(a), None) => blend_key(a, a, 0.0, hour),
            (None, Some(b)) => blend_key(b, b, 1.0, hour),
            (None, None) => unreachable!("a keyframe is always built when none brackets the hour"),
        };
        if prefetch && self.get(lo).is_some() && self.get(lo + 1).is_some() {
            self.prefetch(lo + 2, inputs);
        }
        out
    }
}

/// The sun's altitude (degrees) at an hour of the look's day.
pub fn sun_altitude_deg(hour: f64) -> f64 {
    eo_look::sun::sun_direction(hour).altitude_deg
}

/// The hour between `a` and `b` at which the sun crosses the horizon
/// (rising if `rising`), by bisection: the dusk strip's centre.
pub fn horizon_hour(a: f64, b: f64, rising: bool) -> f64 {
    let (mut lo, mut hi) = (a, b);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if (sun_altitude_deg(mid) > 0.0) == rising {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The angle (degrees) between two unit directions.
pub fn angle_deg(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0).acos().to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear(transfer: Option<f64>) -> KeyInputs {
        let w = eo_sky::weather::preset(eo_sky::Kind::Clear);
        KeyInputs { weather: Some(w), weather_key: crate::sky::weather_key(&w), moon_day: eo_sky::moon::DEFAULT_MOON_DAY, transfer, cinematic: false }
    }

    /// Every number the look uniform takes from the frame and the terms.
    fn numbers(k: &Key) -> Vec<f64> {
        let mut v = eo_look::blend::frame_numbers(&k.0);
        if let Some(t) = &k.1 {
            v.extend([t.exposure_target, t.lamps_on, t.dusk, t.nightness, t.vehicle_lamps, t.moon_key, t.moon_disc, t.star_level, t.lamp_units_per_lux, t.fill_scale, t.moon_weight]);
            v.extend(t.moon.dir);
        }
        v
    }

    /// Across sunset, one-second steps of the clock through the soft path: the
    /// uniform's numbers move by about a sixtieth of a keyframe interval's
    /// change each step, and nothing steps at a keyframe.
    #[test]
    fn the_soft_path_has_no_step_at_a_keyframe() {
        for inputs in [clear(None), KeyInputs { weather: None, weather_key: (0, 0, 0), moon_day: 0.0, transfer: Some(0.6), cinematic: false }] {
            let mut keys = Keys::default();
            let (from, to) = (18.9, 19.3);
            let steps = ((to - from) * 3600.0) as usize;
            let mut prev: Option<Vec<f64>> = None;
            // per number: the largest one-second change at a keyframe boundary, and anywhere else.
            let n = numbers(&build_key(0, &inputs)).len();
            let (mut at_key, mut inside) = (vec![0.0f64; n], vec![0.0f64; n]);
            for s in 0..=steps {
                let h = from + s as f64 / 3600.0;
                let k = keys.frame(h, inputs, s as u32, false);
                let v = numbers(&k);
                if let Some(p) = &prev {
                    let crossed = (h / KEY_INTERVAL_H).floor() != ((h - 1.0 / 3600.0) / KEY_INTERVAL_H).floor();
                    for i in 0..n {
                        let d = (v[i] - p[i]).abs();
                        if crossed { at_key[i] = at_key[i].max(d) } else { inside[i] = inside[i].max(d) }
                    }
                }
                prev = Some(v);
            }
            for i in 0..n {
                // at a boundary the step is the next interval's slope: at most a little over the largest elsewhere.
                assert!(at_key[i] <= 1.5 * inside[i] + 1e-9, "{:?}: number {i} steps by {} at a keyframe, at most {} elsewhere", inputs.weather.map(|w| w.kind), at_key[i], inside[i]);
            }
            // the steps are small against the whole move over the window.
            assert!(keys.built <= 2 + ((to - from) / KEY_INTERVAL_H).ceil() as u64, "{} keyframes built", keys.built);
        }
    }

    /// At a keyframe's hour the soft path draws that keyframe; between two it
    /// stays within a small share of the hard path's frame at that hour.
    #[test]
    fn the_soft_path_meets_the_hard_frame_at_its_keyframes() {
        let inputs = clear(None);
        let mut keys = Keys::default();
        for (f, k) in [1140i64, 1142, 1143, 1150].into_iter().enumerate() {
            let h = k as f64 * KEY_INTERVAL_H;
            let soft = numbers(&keys.frame(h, inputs, 2 * f as u32, false));
            let _ = keys.frame(h + 0.5 * KEY_INTERVAL_H, inputs, 2 * f as u32 + 1, false);
            let hard = numbers(&build_key(k, &inputs));
            for (i, (a, b)) in soft.iter().zip(&hard).enumerate() {
                assert!((a - b).abs() <= 1e-9 * b.abs().max(1e-6), "keyframe {k}: number {i}: {a} vs {b}");
            }
        }
    }

    /// One keyframe a frame on the calling thread, none when the worker has
    /// the next one ready; a change of inputs rebuilds.
    #[test]
    fn at_most_one_keyframe_a_frame() {
        let inputs = clear(None);
        let mut keys = Keys::default();
        // a jump: the first frame builds only the nearer keyframe of the pair.
        let _ = keys.frame(19.0 + 0.2 * KEY_INTERVAL_H, inputs, 1, false);
        assert_eq!(keys.built, 1);
        let _ = keys.frame(19.0 + 0.3 * KEY_INTERVAL_H, inputs, 1, false);
        assert_eq!(keys.built, 1, "a second call in the same frame builds nothing");
        let _ = keys.frame(19.0 + 0.4 * KEY_INTERVAL_H, inputs, 2, true);
        assert_eq!(keys.built, 2, "the next frame builds the other");
        // the worker builds the one after; wait for it, then cross into the next pair.
        for _ in 0..2000 {
            if keys.pending.as_ref().is_some_and(|p| p.2.lock().unwrap().is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let _ = keys.frame(19.0 + 1.2 * KEY_INTERVAL_H, inputs, 3, true);
        assert_eq!((keys.built, keys.prefetched), (2, 1), "the worker's keyframe is used");
        // a new transfer: both rebuilt, one a frame.
        let other = clear(Some(0.5));
        let _ = keys.frame(19.0 + 1.3 * KEY_INTERVAL_H, other, 4, false);
        assert_eq!(keys.built, 3);
        let _ = keys.frame(19.0 + 1.4 * KEY_INTERVAL_H, other, 5, false);
        assert_eq!(keys.built, 4);
    }

    /// What one keyframe costs on the calling thread (printed, not checked: a
    /// timing; the soft path builds one on the render thread only after a jump).
    #[test]
    fn a_keyframe_build_is_timed() {
        for (name, inputs) in [("clear weather", clear(None)), ("every sky stage off", KeyInputs { weather: None, weather_key: (0, 0, 0), moon_day: 0.0, transfer: None, cinematic: false })] {
            let mut ms = vec![];
            for k in 0..20 {
                let t = std::time::Instant::now();
                std::hint::black_box(build_key(1130 + k, &inputs));
                ms.push(t.elapsed().as_secs_f64() * 1e3);
            }
            ms.sort_by(f64::total_cmp);
            println!("keyframe build, {name}: median {:.3} ms, max {:.3} ms", ms[ms.len() / 2], ms[ms.len() - 1]);
        }
    }

    #[test]
    fn the_horizon_hours_are_sunset_and_sunrise() {
        let (set, rise) = (horizon_hour(16.0, 23.0, false), horizon_hour(3.0, 11.0, true));
        assert!(sun_altitude_deg(set - 0.01) > 0.0 && sun_altitude_deg(set + 0.01) < 0.0, "sunset {set}");
        assert!(sun_altitude_deg(rise - 0.01) < 0.0 && sun_altitude_deg(rise + 0.01) > 0.0, "sunrise {rise}");
    }

    /// Midnight: keyframe 1440 (24 h) is keyframe 0.
    #[test]
    fn the_keyframes_wrap_at_midnight() {
        let inputs = clear(None);
        let mut keys = Keys::default();
        let _ = keys.frame(24.0 - 0.5 * KEY_INTERVAL_H, inputs, 1, false);
        let _ = keys.frame(24.0 - 0.4 * KEY_INTERVAL_H, inputs, 2, false);
        assert_eq!(keys.built, 2);
        let a = numbers(&keys.frame(24.0 - 1e-9, inputs, 3, false));
        let b = numbers(&keys.frame(0.0, inputs, 4, false));
        assert_eq!(keys.built, 3, "24 h's keyframe serves 0 h (only 0:01's is built)");
        // (numbers 85-87, the sun's direction at the exact hour, are left out: the
        // solar model's declination moves on a day at midnight, the sun 30+ degrees down.)
        let sun = 7 * 3 + 27 + 6 + 3 + eo_look::rig::RIG_VALUES;
        for (i, (x, y)) in a.iter().zip(&b).enumerate().filter(|(i, _)| !(sun..sun + 3).contains(i)) {
            assert!((x - y).abs() <= 1e-6 * x.abs().max(1e-3), "number {i}: {x} vs {y}");
        }
    }
}
