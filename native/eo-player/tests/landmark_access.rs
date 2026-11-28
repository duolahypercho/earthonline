//! Landmark access in the collision world:
//!
//! * a synthetic deck inside a building's prism: the prism opens where the
//!   deck stands (collider and analytic guard), the walker stands on the
//!   floor and its railing stops it;
//! * the real decks (eo-landmarks access.rs on the city's ground): the player
//!   stands on every deck, walks along it, and the railing or glazing stops a
//!   run straight at the view;
//! * every lift: interact at the bottom stop rides up, at the top rides down,
//!   with the fade, and two worlds with different tile threads arrive at the
//!   same body bit for bit;
//! * the Twin Peaks summit is walkable ground already.

use eo_player::lift::{self, LiftRide};
use eo_player::world::{self, BuildingSet, CollisionWorld, DeckPart, Decks, ElevationGround, Footprint, GroundProvider, LiftDef, PrismCut, TileKey};
use eo_player::{Player, StepInput};
use glam::DVec3;
use std::sync::{Arc, OnceLock};

struct Flat;

impl GroundProvider for Flat {
    fn ground_y(&self, _x: f64, _z: f64) -> f64 {
        0.0
    }
    fn surface_tile(&self, _key: TileKey) -> Option<world::SurfaceMesh> {
        None
    }
}

fn quad(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> [[[f64; 3]; 3]; 2] {
    [[a, b, c], [a, c, d]]
}

/// The player's yaw facing a compass heading (forward = (sin yaw, 0, cos yaw)).
fn yaw_of_heading(h: f64) -> f64 {
    let d = eo_landmarks::access::dir_of(h);
    d[0].atan2(d[1])
}

#[test]
fn a_building_floors_two_edges_stay_at_their_surveyed_coordinates() {
    let x = 8191.12518;
    let z = 16.013;
    let edge = 2.;
    let walk = quad(
        [x-edge, 7., z-1.], [x-edge, 7., z+1.],
        [x+edge, 7., z+1.], [x+edge, 7., z-1.],
    ).to_vec();
    let decks = Decks { parts: vec![DeckPart { walk, ..Default::default() }], ..Default::default() };
    let mut cw = CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::empty().with_decks(decks)), 1);
    cw.load_blocking(DVec3::new(x, 7., z), 8.);
    for side in [-1., 1.] {
        for (offset, supported) in [(-0.00005, true), (0.00005, false)] {
            let at = DVec3::new(x + side * (edge + offset), 7.03, z);
            let hit = cw.raycast(at, DVec3::NEG_Y, 0.06, world::layer::STREET, None);
            assert_eq!(hit.is_some(), supported, "floor edge at {at:?}");
        }
    }
}

#[test]
fn a_deck_inside_a_building_opens_its_prism_and_its_railing_stops_the_walker() {
    // A 20 m square tower 60 m tall; a deck at 40 m inside it, 16 m square,
    // with a wall round it (the glazing) and a lift from the ground.
    let tower = Footprint { ring: vec![[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]], height: 60.0 };
    let y = 40.0;
    let mut part = DeckPart { name: "test".into(), ..Default::default() };
    part.walk.extend(quad([-8.0, y, -8.0], [8.0, y, -8.0], [8.0, y, 8.0], [-8.0, y, 8.0]));
    let ring = [[-7.5, -7.5], [7.5, -7.5], [7.5, 7.5], [-7.5, 7.5]];
    for i in 0..4 {
        let (a, b) = (ring[i], ring[(i + 1) % 4]);
        part.barrier.extend(quad([a[0], y - 0.2, a[1]], [b[0], y - 0.2, b[1]], [b[0], y + 3.0, b[1]], [a[0], y + 3.0, a[1]]));
    }
    let decks = Decks {
        parts: vec![part],
        cuts: vec![PrismCut { centre: [0.0, 0.0], radius_m: 5.0, y0: y - 0.8, y1: y + 4.0 }],
        lifts: vec![LiftDef { name: "test lift".into(), bottom: DVec3::new(0.0, 0.0, 12.0), bottom_yaw: 0.0, top: DVec3::new(0.0, y, 0.0), top_yaw: 0.0 }],
    };
    assert_eq!(decks.spans(0.0, 0.0, -0.25, 60.0), vec![(-0.25, y - 0.8), (y + 4.0, 60.0)]);
    assert_eq!(decks.spans(30.0, 0.0, -0.25, 60.0), vec![(-0.25, 60.0)], "a prism the cut does not reach is whole");
    let mut cw = CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::new(vec![tower], 0.0).with_decks(decks)), 2);
    cw.load_blocking(DVec3::new(0.0, 0.0, 12.0), 150.0);
    // Ride up from the bottom stop.
    let mut p = Player::new(DVec3::new(0.0, 0.0, 12.0), 0.0);
    for _ in 0..10 {
        p.fixed_step(&mut cw, &StepInput::default());
    }
    assert!(p.lift_prompt(&cw).is_some_and(|q| q.up), "no prompt at the bottom stop");
    p.fixed_step(&mut cw, &StepInput { interact: true, ..Default::default() });
    assert!(p.lift.is_some());
    let mut darkest: f64 = 0.0;
    for _ in 0..lift::RIDE_STEPS + 30 {
        darkest = darkest.max(p.lift_fade());
        p.fixed_step(&mut cw, &StepInput { move_y: 1.0, ..Default::default() });
    }
    assert!(darkest > 0.99, "the ride never went dark ({darkest})");
    assert_eq!(p.lift_fade(), 0.0);
    let feet = p.character.feet();
    assert!((feet.y - y).abs() < 0.05 && feet.x.abs() < 1.0 && feet.z.abs() < 3.0, "not on the deck after the ride: {feet:?}");
    assert!((cw.surface_below(feet.x, feet.z, feet.y + 0.5) - y).abs() < 0.01);
    assert_eq!(cw.surface_below(feet.x, feet.z, y - 1.0), 0.0);
    // Run at the glazing: the deck holds the walker and the wall stops it.
    p.yaw = 0.0; // +z
    for _ in 0..240 {
        p.fixed_step(&mut cw, &StepInput { move_y: 1.0, sprint: true, ..Default::default() });
        let f = p.character.feet();
        assert!(f.y > y - 0.05, "fell through the deck: {f:?}");
        assert!(f.z < 7.5 - 0.3, "through the glazing: {f:?}");
    }
    assert!(p.character.feet().z > 7.5 - 0.5, "stopped short of the glazing: {:?}", p.character.feet());
}

struct RealCity {
    elevation: Arc<eo_data::ElevationGrid>,
    buildings: Arc<BuildingSet>,
    access: eo_landmarks::access::AccessSet,
}

fn real() -> &'static RealCity {
    static C: OnceLock<RealCity> = OnceLock::new();
    C.get_or_init(|| {
        let paths = eo_data::DataPaths::discover().unwrap();
        let city = eo_data::City::load(&paths).unwrap();
        let elevation = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
        let placement: serde_json::Value = serde_json::from_slice(&std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../eo-assets/data/authored-placement.json")).unwrap()).unwrap();
        let access = eo_landmarks::access::AccessSet::build(&|x, z| elevation.sample(x, z));
        let yaw = |h: f64| yaw_of_heading(h);
        let decks = Decks {
            parts: access.decks.iter().map(|d| DeckPart { name: d.id.clone(), walk: d.walk.clone(), barrier: d.barrier.clone(), ..Default::default() }).collect(),
            cuts: access.decks.iter().flat_map(|d| d.cuts.iter()).map(|c| PrismCut { centre: c.centre, radius_m: c.radius_m, y0: c.y0, y1: c.y1 }).collect(),
            lifts: access.lifts().map(|l| LiftDef { name: l.name.clone(), bottom: DVec3::from_array(l.bottom.feet), bottom_yaw: yaw(l.bottom.heading_deg), top: DVec3::from_array(l.top.feet), top_yaw: yaw(l.top.heading_deg) }).collect(),
        };
        let buildings = Arc::new(BuildingSet::from_city(&city, placement["constructionLiftM"].as_f64().unwrap()).with_decks(decks));
        RealCity { elevation, buildings, access }
    })
}

fn real_world(threads: usize) -> CollisionWorld {
    let c = real();
    CollisionWorld::new(Arc::new(ElevationGround(c.elevation.clone())), c.buildings.clone(), threads)
}

#[test]
fn every_real_deck_is_walked_and_its_railing_or_glazing_stops_the_walker() {
    let c = real();
    assert!(c.access.decks.len() >= 8, "{} decks", c.access.decks.len());
    let mut cw = real_world(3);
    for d in &c.access.decks {
        let start = DVec3::from_array(d.view.feet) + DVec3::Y * 0.05;
        cw.load_blocking(start, world::LOAD_RADIUS_M);
        let mut p = Player::new(start, yaw_of_heading(d.view.heading_deg));
        for _ in 0..40 {
            p.fixed_step(&mut cw, &StepInput::default());
        }
        let stood = p.character.feet();
        assert!((stood.y - d.view.feet[1]).abs() < 0.08, "{}: does not stand on the deck: feet {stood:?}, floor {:.3}", d.id, d.view.feet[1]);
        // Straight at the view, running: stopped within a couple of metres, never below the deck.
        let out = eo_landmarks::access::dir_of(d.view.heading_deg);
        let mut furthest: f64 = 0.0;
        for _ in 0..240 {
            p.fixed_step(&mut cw, &StepInput { move_y: 1.0, sprint: true, ..Default::default() });
            let f = p.character.feet();
            assert!(f.y > d.view.feet[1] - 0.6, "{}: fell off the deck running at the view: {f:?}", d.id);
            furthest = furthest.max((f.x - stood.x) * out[0] + (f.z - stood.z) * out[1]);
        }
        assert!(furthest < 1.5, "{}: ran {furthest:.2} m out past the railing", d.id);
        // Along the deck, both ways: it is walkable (the feet stay on it).
        for turn in [90.0, -90.0] {
            p.yaw = yaw_of_heading(d.view.heading_deg + turn);
            for _ in 0..90 {
                p.fixed_step(&mut cw, &StepInput { move_y: 1.0, ..Default::default() });
                let f = p.character.feet();
                assert!(f.y > d.view.feet[1] - 0.6, "{}: fell walking along the deck: {f:?}", d.id);
            }
        }
        println!("{}: stands at {:.2}, stopped {furthest:.2} m out toward {:.0} deg", d.id, stood.y, d.view.heading_deg);
    }
}

/// Ride a lift up from its bottom stop and down again: the states along the way.
fn round_trip(cw: &mut CollisionWorld, l: &LiftDef) -> Vec<eo_player::PlayerState> {
    // On the bottom stop: the ground, or a deck over it (the Golden Gate's sidewalk).
    let start = DVec3::new(l.bottom.x, l.bottom.y.max(cw.ground.ground_y(l.bottom.x, l.bottom.z)), l.bottom.z);
    cw.load_blocking(start, world::LOAD_RADIUS_M);
    let mut p = Player::new(start + DVec3::Y * 0.3, l.bottom_yaw);
    let mut out = vec![];
    let tap = StepInput { interact: true, ..Default::default() };
    let idle = StepInput::default();
    for k in 0..(40 + lift::RIDE_STEPS as usize + 40 + lift::RIDE_STEPS as usize + 40) {
        let i = if k == 40 || k == 40 + lift::RIDE_STEPS as usize + 40 { &tap } else { &idle };
        p.fixed_step(cw, i);
        let pos = p.character.feet();
        cw.pump(pos);
        out.push(p.state(cw));
    }
    out
}

#[test]
fn every_lift_rides_up_and_down_and_two_worlds_agree_bit_for_bit() {
    let c = real();
    let lifts = c.buildings.decks.lifts.clone();
    assert!(lifts.len() >= 7, "{} lifts", lifts.len());
    let mut a = real_world(2);
    let mut b = real_world(4);
    for l in &lifts {
        let ra = round_trip(&mut a, l);
        let rb = round_trip(&mut b, l);
        assert_eq!(ra, rb, "{}: two worlds disagree", l.name);
        // Up: on the deck after the first ride; down: back at the entrance after the second.
        let top = &ra[40 + lift::RIDE_STEPS as usize + 20];
        let feet_top = DVec3::from_array(top.character.position) - DVec3::Y * (eo_player::tuning::capsule::HALF_HEIGHT_M + eo_player::tuning::capsule::RADIUS_M);
        assert!(top.lift.is_none());
        assert!((feet_top - l.top).length() < 0.5, "{}: after the ride up the feet are at {feet_top:?}, the top stop {:?}", l.name, l.top);
        let end = ra.last().unwrap();
        let feet_end = DVec3::from_array(end.character.position);
        assert!((feet_end.x - l.bottom.x).hypot(feet_end.z - l.bottom.z) < 0.5 && (feet_end.y - l.bottom.y).abs() < 4.0, "{}: after the ride down the body is at {feet_end:?}, the bottom stop {:?}", l.name, l.bottom);
        // The ride was in the state the whole time it lasted.
        assert!(ra.iter().filter(|s| s.lift.is_some_and(|r: LiftRide| r.up)).count() >= lift::RIDE_STEPS as usize - 1, "{}: no ride up in the state", l.name);
        println!("{}: up to {:.1} m, down to {:.1} m", l.name, feet_top.y, feet_end.y);
    }
}

#[test]
fn the_twin_peaks_summit_is_already_walkable() {
    let c = real();
    // The highest drawn ground on the city's Twin Peaks block (surveyed
    // 37.7544 N, 122.4477 W, 922 ft; here found as the grid's highest point near it).
    let centre = eo_data::groundscale::plan([-2400.0, 1700.0]);
    let (mut best, mut at) = (f64::MIN, [0.0, 0.0]);
    let mut x = centre[0] - 1500.0;
    while x < centre[0] + 1500.0 {
        let mut z = centre[1] - 1500.0;
        while z < centre[1] + 1500.0 {
            if let Some(h) = c.elevation.sample(x, z) {
                if h > best {
                    best = h;
                    at = [x, z];
                }
            }
            z += 8.0;
        }
        x += 8.0;
    }
    let mut cw = real_world(2);
    let start = DVec3::new(at[0], cw.ground.ground_y(at[0], at[1]) + 0.2, at[1]);
    cw.load_blocking(start, world::LOAD_RADIUS_M);
    let mut p = Player::new(start, 0.0);
    for k in 0..240 {
        let i = StepInput { move_y: if k > 60 { 1.0 } else { 0.0 }, ..Default::default() };
        p.fixed_step(&mut cw, &i);
        let f = p.character.feet();
        let g = cw.floor_y(f.x, f.z);
        assert!(f.y > g - 0.05, "below the summit's ground: {f:?} ground {g:.2}");
    }
    assert!(p.character.grounded, "the walker is not on the summit's ground");
    println!("Twin Peaks summit {best:.1} m at {at:?}: walked to {:?}", p.character.feet());
}
