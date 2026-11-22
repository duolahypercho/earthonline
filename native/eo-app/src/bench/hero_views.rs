//! The key views (`--bench --light-shots` with `EO_LIGHT_SHOTS=hero`; the
//! cinematic look plan's stage L0): six fixed frames, each with its camera,
//! hour, weather, world clock and seed pinned, drawn in one run so a look
//! change can be judged by eye on matched sheets (`tmp/plan-run/tools/heroes.sh`
//! and `sheets.py` build them):
//!
//! * `hero-castro-night`: Castro Street 10 m north of 18th on the east
//!   walk, looking up the shops' block toward Market at 21:00, clear (mixed
//!   lights, shop windows, parked cars, lamps and trolley wires);
//! * `hero-mission-wet`: Mission Street 40 m north of 24th, 2.5 m up over
//!   the centre line, looking south across the junction at 22:00, the ground
//!   wet after rain with no rain falling (`--weather damp`, stage L3: the
//!   film and puddles an hour after rain, a clearing deck);
//! * `hero-downtown-golden`: Bush Street at Montgomery, looking west down the
//!   canyon toward the low sun at 18.5 h (on the game's date, 22 September,
//!   the sun sets due west; Market Street's axis runs 37 degrees off it, so
//!   the sun never reaches down Market at this hour, while Bush runs east-west);
//! * `hero-viaduct-golden`: 13th Street under the Central Freeway viaduct at
//!   18.5 h, looking west along the deck toward the sun (needs
//!   `--structures`; without it the card is drawn and says so);
//! * `hero-embarcadero-blue`: the Embarcadero promenade at Howard Street at
//!   19.25 h, looking south-east to the Bay Bridge;
//! * `hero-aerial-golden`: the city from 2 km up over the Sunset, looking
//!   north-east across it toward downtown and the bay, horizon in frame, at
//!   18.5 h.
//!
//! Each card settles with the sky's world clock held at `--world-time`
//! (frame step 0, so the clouds do not drift with the settle's length), the
//! water's clock at `WATER_TIME_S`, the sky seed at `SEED`, the exposure at
//! the meter's target for the frame (no adaptation filter: with the clock held
//! the filter would freeze at the first frame's target, before the view had
//! streamed in; a still frame's converged exposure is the target), and the
//! traffic as the world loads it (the light cards pump no simulation), so two
//! runs of one build give the same frame.
//!
//! `EO_HERO_TUNE="name[~variant][@Street|Cross|Toward]:along,right,up,at_along,at_right,at_up[,fov_y];..."`
//! (a diagnostic for choosing the cameras) replaces a card's numbers, or with
//! `~variant` adds a card of that name beside it at the same hour and
//! weather (`@...`: measured from another junction). Street cards measure along the street from their junction toward
//! the next one, right of that line, and up over the street surface; the
//! aerial card is on the plan (east, south, height over the sea level).

use super::{Segment, yaw_of};
use crate::World;
use eo_render::camera::Camera;
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_sky::Kind;
use glam::Vec3;

/// The sky's seed every key view is drawn with (the clouds' field).
pub const SEED: u32 = 0x5f_c1_7e;
/// The water's clock every key view is drawn at (s; as the far cards' 20 s).
pub const WATER_TIME_S: f64 = 20.0;

/// Where a card's numbers are measured from.
#[derive(Clone, Copy, Debug)]
enum Frame {
    /// The junction of `street` and `cross`, along `street` toward its
    /// junction with `toward`; heights over the street surface.
    Street { street: &'static str, cross: &'static str, toward: &'static str },
    /// The plan's origin, along east (+x) and right toward the south (+z);
    /// heights over the sea level.
    Plan,
}

/// One key view.
#[derive(Clone, Copy, Debug)]
pub struct Hero {
    pub name: &'static str,
    frame: Frame,
    /// (along, right, up) of the eye and of the point it looks at (m).
    eye: [f32; 3],
    at: [f32; 3],
    /// Vertical field of view (degrees; None: the game's).
    fov_y: Option<f32>,
    /// The far plane at least this far (m; the aerial view's horizon).
    far_m: f32,
    pub hour: f64,
    pub weather: Kind,
    /// Whether the weather's rain is drawn falling (the wet card: no).
    pub rain_falling: bool,
    /// Whether the card's subject is the grade-separated roads (`--structures`).
    pub structures: bool,
    pub what: &'static str,
}

/// The six key views, in capture order.
pub const HEROES: [Hero; 6] = [
    Hero {
        name: "hero-castro-night",
        frame: Frame::Street { street: "Castro Street", cross: "18th Street", toward: "Market Street" },
        eye: [10.0, 8.5, 2.2],
        at: [120.0, 2.0, 5.0],
        fov_y: Some(52.0),
        far_m: 0.0,
        hour: 21.0,
        weather: Kind::Clear,
        rain_falling: true,
        structures: false,
        what: "Castro Street 10 m north of 18th on the east walk, 2.2 m up, looking north up the shops' block toward Market at 21:00, clear",
    },
    Hero {
        name: "hero-mission-wet",
        frame: Frame::Street { street: "Mission Street", cross: "24th Street", toward: "23rd Street" },
        eye: [40.0, 0.0, 2.5],
        at: [-60.0, 0.0, 3.0],
        fov_y: Some(52.0),
        far_m: 0.0,
        hour: 22.0,
        // (stage L3) `--weather damp`: the ground film and puddles, no rain, no ripples, no veil.
        weather: Kind::Damp,
        rain_falling: false,
        structures: false,
        what: "Mission Street 40 m north of 24th, 2.5 m over the centre line, looking south across the junction at 22:00, the ground wet an hour after rain (--weather damp)",
    },
    Hero {
        name: "hero-downtown-golden",
        frame: Frame::Street { street: "Bush Street", cross: "Montgomery Street", toward: "Kearny Street" },
        eye: [-15.0, 3.0, 1.7],
        at: [200.0, 0.0, 40.0], // (stage L4) pitched ~10 deg up (22: ~5 deg, the lower half of the frame dark asphalt)
        fov_y: Some(55.0),
        far_m: 0.0,
        hour: 18.5,
        weather: Kind::Clear,
        rain_falling: true,
        structures: false,
        what: "Bush Street at Montgomery, looking west down the canyon toward the low sun at 18.5 h, clear",
    },
    Hero {
        name: "hero-viaduct-golden",
        frame: Frame::Street { street: "13th Street", cross: "South Van Ness Avenue", toward: "Mission Street" },
        eye: [40.0, 0.0, 1.7],
        at: [160.0, 0.0, 6.0],
        fov_y: Some(58.0),
        far_m: 0.0,
        hour: 18.5,
        weather: Kind::Clear,
        rain_falling: true,
        structures: true,
        what: "13th Street under the Central Freeway viaduct, west of South Van Ness, looking west along the deck toward Mission and the low sun at 18.5 h, clear (needs --structures)",
    },
    Hero {
        name: "hero-embarcadero-blue",
        frame: Frame::Street { street: "The Embarcadero", cross: "Howard Street", toward: "Folsom Street" },
        eye: [-80.0, -30.0, 2.0],
        at: [600.0, -220.0, 60.0],
        fov_y: Some(55.0),
        far_m: 0.0,
        hour: 19.25,
        weather: Kind::Clear,
        rain_falling: true,
        structures: false,
        what: "the Embarcadero promenade 80 m north of Howard Street on the bay side, looking south-east along the waterfront to the Bay Bridge at 19.25 h (blue hour), clear",
    },
    Hero {
        name: "hero-aerial-golden",
        frame: Frame::Plan,
        eye: [-3200.0, 3000.0, 2000.0],
        at: [2200.0, -1800.0, 0.0],
        fov_y: Some(50.0),
        far_m: 20_000.0,
        hour: 18.5,
        weather: Kind::Clear,
        rain_falling: true,
        structures: false,
        what: "the city from 2 km up over the Sunset, looking north-east across it toward downtown and the bay, horizon in frame, at 18.5 h, clear",
    },
];

/// Whether the cards are asked for (`EO_LIGHT_SHOTS` naming `hero`).
pub fn requested() -> bool {
    std::env::var("EO_LIGHT_SHOTS").is_ok_and(|v| v.split(',').any(|k| k.trim().contains("hero")))
}

/// The card `name` (or its tuned variant `name~x`).
pub fn find(name: &str) -> Option<&'static Hero> {
    let base = name.split('~').next().unwrap_or(name);
    HEROES.iter().find(|h| h.name == base)
}

/// The sky a card is drawn under: its weather, the rain drawn falling or not,
/// the seed, the exposure at its target (see the module's head).
pub fn sky(h: &Hero, base: SkyOptions) -> SkyOptions {
    SkyOptions { weather: WeatherMode::Preset(h.weather), rain: base.rain && h.rain_falling, seed: SEED, adaptation: false, ..base }
}

fn surface(world: &World, x: f32, z: f32) -> f32 {
    match &world.street {
        Some(s) => s.world.surface_at(x as f64, z as f64, None) as f32,
        None => world.elevation.ground(x as f64, z as f64) as f32,
    }
}

/// The plan junction where streets `a` and `b` meet (the first in plan order).
fn junction(world: &World, a: &str, b: &str) -> Option<Vec3> {
    let sw = &world.street.as_ref()?.world;
    let name = |ap: &eo_street::plan::Approach| sw.city.segments[sw.plan.entries[ap.entry].road].street_name.as_str();
    sw.plan.nodes.iter().find(|n| n.approaches.iter().any(|ap| name(ap) == a) && n.approaches.iter().any(|ap| name(ap) == b)).map(|n| Vec3::new(n.position[0] as f32, 0.0, n.position[1] as f32))
}

/// A card's camera with these numbers, or why it cannot be placed.
fn camera(world: &World, h: &Hero, frame: Frame, eye: [f32; 3], at: [f32; 3], fov_y: Option<f32>, aspect: f32, far: f32) -> Result<Camera, String> {
    let (o, fwd) = match frame {
        Frame::Street { street, cross, toward } => {
            let o = junction(world, street, cross).ok_or(format!("{street} at {cross}"))?;
            let t = junction(world, street, toward).ok_or(format!("{street} at {toward}"))?;
            (o, Vec3::new(t.x - o.x, 0.0, t.z - o.z).normalize_or_zero())
        }
        Frame::Plan => (Vec3::ZERO, Vec3::X),
    };
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    let place = |p: [f32; 3]| {
        let q = o + fwd * p[0] + right * p[1];
        match frame {
            Frame::Street { .. } => Vec3::new(q.x, surface(world, q.x, q.z) + p[2], q.z),
            Frame::Plan => Vec3::new(q.x, p[2], q.z),
        }
    };
    let (e, a) = (place(eye), place(at));
    let to = a - e;
    let mut cam = Camera::new(e, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect);
    cam.far = far.max(h.far_m);
    if let Some(f) = fov_y {
        cam.fov_y_deg = f;
    }
    Ok(cam)
}

type Tuned = (String, [f32; 3], [f32; 3], Option<f32>, Option<Frame>);

/// `EO_HERO_TUNE`: (card name, its numbers, another frame) for each entry.
fn tuned() -> Vec<Tuned> {
    let Ok(v) = std::env::var("EO_HERO_TUNE") else { return vec![] };
    v.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|spec| {
            let (name, nums) = spec.split_once(':')?;
            let (name, frame) = match name.split_once('@') {
                Some((n, f)) => {
                    let f: Vec<&'static str> = f.split('|').map(|x| &*Box::leak(x.trim().to_string().into_boxed_str())).collect();
                    (n, (f.len() == 3).then(|| Frame::Street { street: f[0], cross: f[1], toward: f[2] }))
                }
                None => (name, None),
            };
            let n: Vec<f32> = nums.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if !(6..=7).contains(&n.len()) || find(name.trim()).is_none() {
                eprintln!("hero views: EO_HERO_TUNE entry \"{spec}\" is not name[~variant][@Street|Cross|Toward]:along,right,up,at_along,at_right,at_up[,fov_y] of a key view; left out");
                return None;
            }
            Some((name.trim().to_string(), [n[0], n[1], n[2]], [n[3], n[4], n[5]], n.get(6).copied(), frame))
        })
        .collect()
}

/// The cards: (name, camera, hour, what), in capture order.
pub fn cards(world: &World, _segs: &[Segment], aspect: f32, far: f32) -> Vec<(String, Camera, f64, String)> {
    let tune = tuned();
    let structures = crate::structures::drawn(world).is_some();
    let mut out = vec![];
    for h in &HEROES {
        let mut list: Vec<Tuned> = vec![];
        match tune.iter().find(|t| t.0 == h.name) {
            Some(t) => list.push(t.clone()),
            None => list.push((h.name.to_string(), h.eye, h.at, h.fov_y, None)),
        }
        list.extend(tune.iter().filter(|t| t.0.split('~').next() == Some(h.name) && t.0.contains('~')).cloned());
        for (name, eye, at, fov, frame) in list {
            match camera(world, h, frame.unwrap_or(h.frame), eye, at, fov, aspect, far) {
                Ok(cam) => {
                    let mut what = format!("{} (eye along {}, right {}, up {}; at {}, {}, {}; fov_y {:.0})", h.what, eye[0], eye[1], eye[2], at[0], at[1], at[2], cam.fov_y_deg);
                    if h.structures && !structures {
                        what.push_str("; WITHOUT the viaduct: run with --structures");
                        eprintln!("hero views: {name} shows the Central Freeway viaduct only with --structures");
                    }
                    out.push((name, cam, h.hour, what));
                }
                Err(missing) => eprintln!("hero views: {missing} not found; {name} is left out"),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_views_are_six_named_once_with_their_hours() {
        let mut names: Vec<&str> = HEROES.iter().map(|h| h.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 6);
        assert!(HEROES.iter().all(|h| h.name.starts_with("hero-") && find(h.name).is_some_and(|f| f.name == h.name)));
        let hour = |n: &str| find(n).unwrap().hour;
        assert_eq!((hour("hero-castro-night"), hour("hero-mission-wet"), hour("hero-downtown-golden"), hour("hero-viaduct-golden"), hour("hero-embarcadero-blue"), hour("hero-aerial-golden")), (21.0, 22.0, 18.5, 18.5, 19.25, 18.5));
        // the wet card: the damp preset, wet ground, no rain falling; the others clear.
        let wet = find("hero-mission-wet").unwrap();
        let s = sky(wet, SkyOptions::default());
        assert_eq!((s.weather, s.rain, s.wet, s.seed, s.adaptation), (WeatherMode::Preset(Kind::Damp), false, true, SEED, false));
        assert!(HEROES.iter().filter(|h| h.name != wet.name).all(|h| h.weather == Kind::Clear && sky(h, SkyOptions::default()).rain));
        assert!(find("hero-viaduct-golden").unwrap().structures);
        assert_eq!(find("hero-castro-night~b").map(|h| h.name), Some("hero-castro-night"), "a tuned variant runs as its card");
    }
}
