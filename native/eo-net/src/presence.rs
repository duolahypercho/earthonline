//! Being somewhere together: what one player shows the others beyond its
//! body (an emote, a sprint, an arrival by fast travel), the world's shared
//! clock and weather, and the named landmark viewpoints fast travel goes to.
//!
//! EMOTES ride in the player entity's `anim` byte (the low four bits the
//! emote, the high four a count that changes each time one starts, so a
//! repeated wave restarts); a few more `flags` bits say sprinting, in the
//! air and arriving. The client asks with a reliable `msg::EMOTE`; the
//! server holds the emote until the player moves, gets in a car or asks for
//! another (server-authoritative: every client sees the same).
//!
//! THE SKY CLOCK is the server's: the hour, how fast it runs (game hours per
//! real hour) and the weather, in every snapshot header (7 bytes, 11 with a
//! schedule seed). A client extrapolates the hour to its render tick and
//! eases toward it (`SkySmoother`): no jump when a snapshot is late, a cut
//! only when the server's clock is set.
//!
//! VIEWPOINTS are one data table (`native/assets/landmark-viewpoints.json`,
//! coordinates from the repository's OpenStreetMap data): latitude,
//! longitude, the bearing a visitor faces, and a spread radius.

use crate::bits::{BitReader, BitWriter, Truncated};
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

/// What a player is doing for the camera (wire values; 0 none).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Emote {
    #[default]
    None = 0,
    Wave = 1,
    Point = 2,
    Sit = 3,
    Selfie = 4,
}

impl Emote {
    pub const ALL: [Emote; 4] = [Emote::Wave, Emote::Point, Emote::Sit, Emote::Selfie];

    pub fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            1 => Emote::Wave,
            2 => Emote::Point,
            3 => Emote::Sit,
            4 => Emote::Selfie,
            _ => Emote::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Emote::None => "none",
            Emote::Wave => "wave",
            Emote::Point => "point",
            Emote::Sit => "sit",
            Emote::Selfie => "selfie",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" | "" => Emote::None,
            "wave" => Emote::Wave,
            "point" => Emote::Point,
            "sit" => Emote::Sit,
            "selfie" => Emote::Selfie,
            _ => return None,
        })
    }

    /// Held poses stay until the player moves; a wave or a point plays its
    /// loop for this long and ends by itself (seconds; 0 = held).
    pub fn seconds(self) -> f64 {
        match self {
            Emote::Wave => 6.0,
            Emote::Point => 6.0,
            _ => 0.0,
        }
    }
}

/// The player entity's `anim` byte: the emote and its start count.
pub fn pack_anim(e: Emote, count: u8) -> u8 {
    (e as u8 & 0x0F) | (count << 4)
}

pub fn unpack_anim(a: u8) -> (Emote, u8) {
    (Emote::from_u8(a), a >> 4)
}

/// Player entity flags (bits 0..2 are driving, crouched, offline: `eo-server` `player_entities`).
pub mod flag {
    pub const DRIVING: u16 = 1;
    pub const CROUCHED: u16 = 2;
    pub const OFFLINE: u16 = 4;
    /// Running in a sprint (the run tier, past the walk).
    pub const SPRINT: u16 = 8;
    /// Off the ground (a jump, a fall).
    pub const AIRBORNE: u16 = 16;
    /// Just arrived by fast travel: draw it there, do not glide from where it was.
    pub const ARRIVED: u16 = 32;
}

// ---------------------------------------------------------------- the sky clock

/// Weather on the wire: a preset (`eo_sky::Kind` order) or the schedule.
pub const WEATHER_SCHEDULE: u8 = 255;
/// The preset kinds in wire order (index = wire value).
pub const WEATHER_NAMES: [&str; 6] = ["clear", "fog", "overcast", "drizzle", "rain", "storm"];

pub fn weather_code(name: &str) -> Option<u8> {
    if name == "schedule" {
        return Some(WEATHER_SCHEDULE);
    }
    WEATHER_NAMES.iter().position(|n| *n == name).map(|i| i as u8)
}

pub fn weather_name(code: u8) -> &'static str {
    if code == WEATHER_SCHEDULE { "schedule" } else { WEATHER_NAMES.get(code as usize).copied().unwrap_or("clear") }
}

/// The server's sky at one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkyClock {
    /// Hour of the day, 0..24.
    pub hour: f64,
    /// Game hours per real hour (1 = the real day; 0 = held).
    pub rate: f64,
    /// `WEATHER_NAMES` index, or `WEATHER_SCHEDULE`.
    pub weather: u8,
    /// The weather schedule's seed (schedule only).
    pub seed: u32,
    /// Bumped by the server whenever its clock or weather is set (a cut, not an ease).
    pub epoch: u8,
}

/// Wire size: hour (u32, 24 h / 2^32), rate (u16, 1/64), weather (u8), epoch (u8), and the seed (u32) under a schedule.
impl SkyClock {
    pub fn write(&self, w: &mut BitWriter) {
        let h = (self.hour.rem_euclid(24.0) / 24.0 * 4_294_967_296.0).floor().min(4_294_967_295.0) as u32;
        w.write(h, 32);
        w.write((self.rate * 64.0).round().clamp(0.0, 65535.0) as u32, 16);
        w.write(self.weather as u32, 8);
        w.write(self.epoch as u32, 8);
        if self.weather == WEATHER_SCHEDULE {
            w.write(self.seed, 32);
        }
    }

    pub fn read(r: &mut BitReader) -> Result<Self, Truncated> {
        let h = r.read(32)? as f64 / 4_294_967_296.0 * 24.0;
        let rate = r.read(16)? as f64 / 64.0;
        let weather = r.read(8)? as u8;
        let epoch = r.read(8)? as u8;
        let seed = if weather == WEATHER_SCHEDULE { r.read(32)? } else { 0 };
        Ok(Self { hour: h, rate, weather, seed, epoch })
    }

    pub fn bits(&self) -> u32 {
        64 + if self.weather == WEATHER_SCHEDULE { 32 } else { 0 }
    }

    /// The hour `seconds` of real time later.
    pub fn hour_after(&self, seconds: f64) -> f64 {
        (self.hour + self.rate * seconds / 3600.0).rem_euclid(24.0)
    }
}

/// Signed hours from `a` to `b` the short way round the clock.
pub fn hour_delta(a: f64, b: f64) -> f64 {
    (b - a + 12.0).rem_euclid(24.0) - 12.0
}

/// The client's drawn sky: eases toward the server's clock extrapolated to
/// the render tick (time constant `EASE_S`), cuts when the server set its
/// clock (a new epoch) or the two are more than `CUT_H` apart.
#[derive(Clone, Copy, Debug, Default)]
pub struct SkySmoother {
    pub shown: Option<SkyClock>,
    pub cuts: u32,
}

impl SkySmoother {
    pub const EASE_S: f64 = 2.0;
    pub const CUT_H: f64 = 0.5;

    /// `target`: the server's clock at the snapshot, `ahead_s` the render
    /// tick's seconds past it; `dt` this frame. Returns the clock to draw.
    pub fn step(&mut self, target: SkyClock, ahead_s: f64, dt: f64) -> SkyClock {
        let want = SkyClock { hour: target.hour_after(ahead_s), ..target };
        let next = match self.shown {
            Some(s) if s.epoch == target.epoch && s.weather == target.weather && hour_delta(s.hour, want.hour).abs() < Self::CUT_H => {
                // Advance at the server's rate, then ease the residual.
                let own = s.hour_after(dt);
                let k = 1.0 - (-dt.max(0.0) / Self::EASE_S).exp();
                SkyClock { hour: (own + hour_delta(own, want.hour) * k).rem_euclid(24.0), ..want }
            }
            _ => {
                if self.shown.is_some() {
                    self.cuts += 1;
                }
                want
            }
        };
        self.shown = Some(next);
        next
    }
}

// ---------------------------------------------------------------- viewpoints

pub const VIEWPOINTS_JSON: &str = include_str!("../../assets/landmark-viewpoints.json");

#[derive(Clone, Debug, Deserialize)]
pub struct Viewpoint {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub place: String,
    pub lat: f64,
    pub lon: f64,
    pub heading_deg: f64,
    pub spread_m: f64,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ViewpointFile {
    version: String,
    viewpoints: Vec<Viewpoint>,
}

/// The table (checked: unique ids, finite coordinates inside the city's box).
pub fn viewpoints() -> Result<Vec<Viewpoint>> {
    let f: ViewpointFile = serde_json::from_str(VIEWPOINTS_JSON).context("native/assets/landmark-viewpoints.json")?;
    if f.version != "landmark-viewpoints-v1" {
        return Err(anyhow!("landmark-viewpoints.json: unexpected version {}", f.version));
    }
    let mut seen = std::collections::HashSet::new();
    for v in &f.viewpoints {
        if !seen.insert(v.id.clone()) {
            return Err(anyhow!("landmark-viewpoints.json: {} twice", v.id));
        }
        if !(37.6..=37.9).contains(&v.lat) || !(-122.6..=-122.3).contains(&v.lon) || !v.heading_deg.is_finite() || !(0.0..=20.0).contains(&v.spread_m) {
            return Err(anyhow!("landmark-viewpoints.json: {} is outside the city or malformed", v.id));
        }
    }
    Ok(f.viewpoints)
}

impl Viewpoint {
    /// Where it is on the game's ground plan (x east, z south) and the yaw a
    /// visitor faces (the game's convention: forward = (sin yaw, 0, cos yaw)).
    pub fn game(&self, gf: &eo_data::geo::GeoFrame) -> [f64; 3] {
        let p = gf.plan_xz(self.lat, self.lon);
        // Compass bearing (0 north = -z, 90 east = +x) to the game yaw (0 = +z).
        let b = self.heading_deg.to_radians();
        let yaw = b.sin().atan2(-b.cos());
        [p[0], p[1], yaw]
    }

    /// Arrival `k` of a group: on a ring of `spread_m` round the point (the
    /// first at the point), facing the view.
    pub fn arrival(&self, gf: &eo_data::geo::GeoFrame, k: u32) -> [f64; 3] {
        let [x, z, yaw] = self.game(gf);
        if k == 0 {
            return [x, z, yaw];
        }
        // Golden-angle ring: no two of the first dozen within a metre.
        let a = k as f64 * 2.399_963_229_728_653;
        let r = self.spread_m * (k as f64 / 12.0).sqrt().min(1.0);
        [x + a.sin() * r, z + a.cos() * r, yaw]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sky_clock_round_trips_and_eases_without_jumping() {
        let c = SkyClock { hour: 18.4321, rate: 30.0, weather: 2, seed: 0, epoch: 3 };
        let mut w = BitWriter::with_capacity(16);
        c.write(&mut w);
        let b = w.finish();
        assert_eq!(b.len() * 8, c.bits().div_ceil(8) as usize * 8);
        let back = SkyClock::read(&mut BitReader::new(&b)).unwrap();
        assert!((back.hour - c.hour).abs() < 1e-8 && back.rate == 30.0 && back.weather == 2 && back.epoch == 3);
        let s = SkyClock { weather: WEATHER_SCHEDULE, seed: 77, ..c };
        let mut w = BitWriter::with_capacity(16);
        s.write(&mut w);
        assert_eq!(SkyClock::read(&mut BitReader::new(&w.finish())).unwrap().seed, 77);
        // Late snapshots: the target stalls, the shown clock keeps running at the rate and never steps back.
        let mut sm = SkySmoother::default();
        let mut last = sm.step(c, 0.0, 0.0).hour;
        for f in 1..600 {
            let ahead = if f % 7 == 0 { 0.0 } else { (f % 7) as f64 / 60.0 };
            let target = SkyClock { hour: c.hour_after(((f / 7) * 7) as f64 / 60.0), ..c };
            let h = sm.step(target, ahead, 1.0 / 60.0).hour;
            let d = hour_delta(last, h);
            assert!(d >= -1e-9 && d < 30.0 / 3600.0 * 2.0 / 60.0, "frame {f}: the drawn hour moved {d} h");
            last = h;
        }
        assert_eq!(sm.cuts, 0);
        // A new epoch (the server set its clock): a cut.
        sm.step(SkyClock { hour: 6.0, epoch: 4, ..c }, 0.0, 1.0 / 60.0);
        assert_eq!(sm.cuts, 1);
        assert!(hour_delta(23.9, 0.1) > 0.19 && hour_delta(23.9, 0.1) < 0.21);
    }

    #[test]
    fn viewpoints_load_and_emotes_pack() {
        let v = viewpoints().unwrap();
        assert!(v.iter().any(|p| p.id == "coit-tower") && v.iter().any(|p| p.id == "ferry-building"));
        for e in Emote::ALL {
            for n in 0..16 {
                assert_eq!(unpack_anim(pack_anim(e, n)), (e, n));
            }
            assert_eq!(Emote::parse(e.name()), Some(e));
        }
        assert_eq!(weather_code("rain").map(weather_name), Some("rain"));
        assert_eq!(weather_code("schedule"), Some(WEATHER_SCHEDULE));
    }
}
