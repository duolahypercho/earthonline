//! Moored boats in the physics world (eo-player `harbour`): where the drawn
//! sea has no waves a boat lies still; in waves it rises and falls with the
//! sea under it; its mooring keeps it in its berth; two worlds step it to
//! the same pose, and a boat spawned later settles to the same motion.

use eo_player::harbour::{self, Fleet, Hull, Moored, Sea};
use eo_player::world::{BuildingSet, CollisionWorld, GroundProvider};
use eo_water::field::{ShoreField, to_half_three};
use eo_water::waves;
use glam::DVec3;
use std::sync::Arc;

struct Seabed;
impl GroundProvider for Seabed {
    fn ground_y(&self, _x: f64, _z: f64) -> f64 {
        -40.0
    }
}

/// A straight shore along x = 0: the water's signed distance is x (m).
fn shore() -> Arc<ShoreField> {
    let (w, h) = (129usize, 9usize);
    let (origin, size) = ([-200.0, -400.0], [1600.0, 800.0]);
    let mut half = vec![0u16; w * h * 2];
    for r in 0..h {
        for c in 0..w {
            let x = origin[0] + size[0] * c as f64 / (w - 1) as f64;
            half[(r * w + c) * 2] = to_half_three(x);
            half[(r * w + c) * 2 + 1] = to_half_three(0.0);
        }
    }
    Arc::new(ShoreField { origin, size, width: w, height: h, half, range: [-200.0, 1400.0], beach_texels: 0, bake_ms: 0.0 })
}

const YACHT: Hull = Hull { loa: 10.52, beam: 3.58, mass_kg: 5117.0 };
const JET: Hull = Hull { loa: 3.37, beam: 1.27, mass_kg: 378.0 };

fn sea() -> Arc<Sea> {
    // the afternoon westerly over the bay: Hs 0.6 m in the bay
    let plan = waves::Plan::new(&waves::sea_state(16.0, [0.0, 0.0], 0.0));
    Sea::new(Arc::new(eo_water::sea::SeaField::new(&plan)), shore() as Arc<dyn harbour::ShoreLookup>)
}

fn world(fleet: &Arc<Fleet>, sea: &Arc<Sea>, t: f64) -> CollisionWorld {
    let mut w = CollisionWorld::new(Arc::new(Seabed), Arc::new(BuildingSet::empty()), 1);
    w.set_harbour(fleet.clone(), sea.clone(), t);
    w
}

fn run(w: &mut CollisionWorld, steps: usize) -> Vec<(DVec3, glam::DQuat)> {
    let mut out = vec![];
    for _ in 0..steps {
        w.step_physics();
        let h = w.harbour.as_ref().unwrap();
        let p = h.poses(&w.physics);
        out.push((p[0].1, p[0].2));
    }
    out
}

fn tilt(q: glam::DQuat) -> f64 {
    // the angle between the boat's up and the world's
    (q * DVec3::Y).dot(DVec3::Y).clamp(-1.0, 1.0).acos()
}

#[test]
fn a_walker_lands_on_the_moored_deck_and_can_walk_off_it() {
    let fleet = Arc::new(Fleet { boats: vec![Moored { hull: YACHT, pos: [20.0, 0.0], heading: 0.0 }], sea_y: 0.0 });
    let mut w = world(&fleet, &sea(), 100.0);
    let start = DVec3::new(20.0, YACHT.freeboard() + 0.6, 0.0);
    w.load_blocking(start, 50.0);
    w.stream_harbour(start);
    let mut p = eo_player::Player::new(start, 0.0);
    for _ in 0..120 {
        p.fixed_step(&mut w, &Default::default());
    }
    assert!(p.character.grounded);
    assert!((p.character.feet().y - YACHT.freeboard()).abs() < 0.03, "feet {:?}", p.character.feet());
    for _ in 0..240 {
        p.fixed_step(&mut w, &eo_player::StepInput { move_y: 1.0, ..Default::default() });
    }
    assert!(p.character.feet().z > YACHT.beam / 2.0 && p.character.feet().y < 0.0);
}

#[test]
fn no_wave_no_motion() {
    // 20 m off the shore the drawn sea has no waves: the boat floats at rest.
    let sea = sea();
    let fleet = Arc::new(Fleet { boats: vec![Moored { hull: YACHT, pos: [20.0, 0.0], heading: 0.3 }], sea_y: 0.0 });
    let mut w = world(&fleet, &sea, 100.0);
    w.stream_harbour(DVec3::new(20.0, 0.0, 0.0));
    let poses = run(&mut w, 600);
    let (first, last) = (poses[0], poses[poses.len() - 1]);
    let drift = (last.0 - DVec3::new(20.0, 0.0, 0.0)).length();
    println!("calm: after 10 s the boat is {drift:.4} m off its berth's waterline, tilted {:.5} rad", tilt(last.1));
    assert!(drift < 0.01 && tilt(last.1) < 1e-3, "a boat in still water moves ({drift} m, {} rad)", tilt(last.1));
    assert!((first.0 - last.0).length() < 0.01);
}

#[test]
fn in_waves_a_boat_rides_the_sea_under_it() {
    let sea = sea();
    for (name, hull) in [("yacht", YACHT), ("jet ski", JET)] {
        let berth = [600.0, 0.0];
        let fleet = Arc::new(Fleet { boats: vec![Moored { hull, pos: berth, heading: 1.2 }], sea_y: 0.0 });
        let t0 = 200.0;
        let mut w = world(&fleet, &sea, t0);
        w.stream_harbour(DVec3::new(berth[0], 0.0, berth[1]));
        let poses = run(&mut w, 60 * 40);
        // after 10 s settling: the heave against the sea's height at the waterline's middle
        let (mut sxy, mut sxx, mut syy, mut n) = (0.0, 0.0, 0.0, 0.0);
        let (mut max_tilt, mut max_off) = (0f64, 0f64);
        for (k, (p, q)) in poses.iter().enumerate().skip(600) {
            let t = t0 + (k + 1) as f64 * eo_player::tuning::FIXED_STEP_S;
            let s = sea.sample(p.x, p.z, t).height;
            sxy += p.y * s;
            sxx += s * s;
            syy += p.y * p.y;
            n += 1.0;
            max_tilt = max_tilt.max(tilt(*q));
            max_off = max_off.max((DVec3::new(p.x, 0.0, p.z) - DVec3::new(berth[0], 0.0, berth[1])).length());
        }
        let corr = sxy / (sxx * syy).sqrt();
        let (rms_heave, rms_sea) = ((syy / n).sqrt(), (sxx / n).sqrt());
        println!("{name}: heave rms {rms_heave:.3} m on a {rms_sea:.3} m rms sea, correlation {corr:.2}, largest tilt {:.1} deg, furthest {max_off:.2} m off its berth", max_tilt.to_degrees());
        assert!(rms_sea > 0.05, "{name}: the open water has waves");
        assert!(corr > 0.6, "{name}: the heave does not follow the sea ({corr})");
        assert!(rms_heave > 0.3 * rms_sea && rms_heave < 1.5 * rms_sea, "{name}: heave {rms_heave} m on a {rms_sea} m sea");
        assert!(max_tilt > 0.002 && max_tilt < 0.5, "{name}: tilt {max_tilt} rad");
        assert!(max_off < 2.5, "{name}: the mooring lets it drift {max_off} m");
    }
}

#[test]
fn the_same_boat_in_every_world() {
    let sea = sea();
    let fleet = Arc::new(Fleet { boats: vec![Moored { hull: YACHT, pos: [600.0, 50.0], heading: -0.4 }], sea_y: 0.0 });
    // the same spawn: bit for bit.
    let (mut a, mut b) = (world(&fleet, &sea, 300.0), world(&fleet, &sea, 300.0));
    for w in [&mut a, &mut b] {
        w.stream_harbour(DVec3::new(600.0, 0.0, 50.0));
    }
    let (pa, pb) = (run(&mut a, 900), run(&mut b, 900));
    assert_eq!(pa, pb, "two worlds step one boat apart");
    // spawned 10 s later: within 15 s it rides as the first does.
    let mut c = world(&fleet, &sea, 310.0);
    c.stream_harbour(DVec3::new(600.0, 0.0, 50.0));
    let pc = run(&mut c, 300);
    let (late, early) = (pc[pc.len() - 1], pa[pa.len() - 1]);
    let (dp, dq) = ((late.0 - early.0).length(), late.1.angle_between(early.1));
    println!("spawned 10 s apart, 5 s later: {dp:.4} m and {:.3} deg apart", dq.to_degrees());
    assert!(dp < 0.05 && dq < 0.01, "a later spawn rides differently ({dp} m, {dq} rad)");
}

#[test]
fn float_pose_is_the_still_water_rest() {
    let sea = sea();
    let m = Moored { hull: YACHT, pos: [20.0, 0.0], heading: 0.0 };
    let (p, q) = harbour::float_pose(&sea, &m, 1.5, 42.0);
    assert_eq!(p, DVec3::new(20.0, 1.5, 0.0));
    assert!(tilt(q) < 1e-12);
}

#[test]
fn a_rider_moves_with_a_dynamic_deck_and_keeps_its_speed_when_jumping() {
    use eo_player::{Player, StepInput};
    use rapier3d::prelude::*;
    let mut w = CollisionWorld::new(Arc::new(Seabed), Arc::new(BuildingSet::empty()), 1);
    w.load_blocking(DVec3::ZERO, 100.0);
    let body = w.physics.insert_body(RigidBodyBuilder::dynamic().gravity_scale(0.0).build());
    let deck = ColliderBuilder::cuboid(3.0, 0.25, 6.0).mass(3000.0)
        .collision_groups(InteractionGroups::new(eo_player::world::layer::BOAT, Group::ALL, InteractionTestMode::And));
    w.physics.insert_collider(deck, Some(body));
    let mut p = Player::new(DVec3::new(0.0,0.25,0.0),0.0);
    for _ in 0..30 { p.fixed_step(&mut w,&StepInput::default()); }
    assert!(p.character.grounded);
    w.physics.bodies[body].set_linvel(Vector::new(1.5,0.0,0.0),true);
    for _ in 0..180 { p.fixed_step(&mut w,&StepInput::default()); }
    assert!((p.position(&w).x - 4.5).abs() < 0.08, "rider {:?}",p.position(&w));
    assert!(p.ground_speed < 0.05, "stationary rider animated at {} m/s", p.ground_speed);
    let from = p.position(&w);
    p.fixed_step(&mut w,&StepInput { jump:true,..Default::default() });
    for _ in 0..14 { p.fixed_step(&mut w,&StepInput::default()); }
    let delta = p.position(&w) - from;
    assert!((delta.x - 0.375).abs() < 0.04 && delta.y > 0.25, "airborne travel {delta:?}");
}

#[test]
fn a_lift_ride_keeps_the_harbour_on_the_same_fixed_step() {
    use eo_player::{Decks, LiftDef, Player, StepInput};
    let stop = DVec3::new(580.0, -40.0, 30.0);
    let berth = [600.0, 50.0];
    let sea = sea();
    let fleet = Arc::new(Fleet {
        boats: vec![Moored {
            hull: YACHT,
            pos: berth,
            heading: 0.3,
        }],
        sea_y: 0.0,
    });
    let spawn = || {
        let maps = BuildingSet::empty().with_decks(Decks {
            lifts: vec![LiftDef {
                name: "test lift".into(),
                bottom: stop,
                bottom_yaw: 0.0,
                top: stop + DVec3::Y * 5.0,
                top_yaw: 0.0,
            }],
            ..Default::default()
        });
        let mut world = CollisionWorld::new(Arc::new(Seabed), Arc::new(maps), 1);
        world.load_blocking(stop, 80.0);
        world.set_harbour(fleet.clone(), sea.clone(), 100.0);
        world.stream_harbour(stop);
        (Player::new(stop, 0.0), world)
    };
    let (mut rider, mut a) = spawn();
    let (mut watcher, mut b) = spawn();
    for tick in 0..eo_player::lift::RIDE_STEPS {
        rider.fixed_step(
            &mut a,
            &StepInput {
                interact: tick == 0,
                ..Default::default()
            },
        );
        watcher.fixed_step(&mut b, &StepInput::default());
        assert_eq!(rider.lift.is_some(), tick + 1 < eo_player::lift::RIDE_STEPS);
        let riding = a.harbour.as_ref().unwrap();
        let watching = b.harbour.as_ref().unwrap();
        assert_eq!(
            riding.time, watching.time,
            "sea clock while riding at tick {tick}"
        );
        let boats = watching.poses(&b.physics);
        assert_eq!(boats.len(), 1);
        assert_eq!(
            riding.poses(&a.physics),
            boats,
            "boat pose while riding at tick {tick}"
        );
    }
}
