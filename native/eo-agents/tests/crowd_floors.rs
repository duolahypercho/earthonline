//! Exercise the crowd feed with stacked routes and the poses returned by local traffic physics.
use std::sync::{Mutex, MutexGuard};

use eo_agents::feed::{
    BODY_RADIUS_M, Decks, FeedThread, Feeder, Ground, PERSONAL_RADIUS_M, set_personal_space,
};
use eo_agents::library::Library;
use eo_agents::{CLASS_PERSON, GpuModel};
use eo_sim::output::{AgentBuffer, AgentInstance, CorridorInstance, KIND_WALKER_BASE};
use eo_sim::world::Frame;

const UPPER: f32 = 4.0;
static FEED_SETTINGS: Mutex<()> = Mutex::new(());

fn lock_feed_settings() -> MutexGuard<'static, ()> {
    // The personal-space switch is process-wide, including the feed threads.
    let guard = FEED_SETTINGS.lock().unwrap_or_else(|error| error.into_inner());
    set_personal_space(false);
    guard
}

fn platform(height: f32, rise: f32) -> Feeder {
    let mut lib = Library::default();
    lib.models.push(GpuModel {
        wheel: [
            [0.8, 0.33, 1.4, 0.],
            [-0.8, 0.33, 1.4, 0.],
            [0.8, 0.33, -1.4, 0.],
            [-0.8, 0.33, -1.4, 0.],
        ],
        wheel_radius: 0.33,
        center: [0., 0.75, 0.],
        ..Default::default()
    });
    lib.meta.model_names.push("sedan".into());
    lib.meta.vehicle_models.insert("sedan".into(), 0);
    lib.models.push(GpuModel {
        class: CLASS_PERSON,
        ..Default::default()
    });
    lib.meta.people_models.push(1);
    let mut feed = Feeder::new(lib.into(), Ground::Flat, None, 7).unwrap();
    let [a, b, c, d] = [
        [0., height, -3.],
        [4., height + rise, -3.],
        [4., height + rise, 3.],
        [0., height, 3.],
    ];
    feed.decks = Some(std::sync::Arc::new(Decks::new(vec![[a, b, c], [a, c, d]])));
    feed.separate = true;
    feed
}

fn walker(id: u32, x: f32) -> AgentInstance {
    AgentInstance {
        id,
        pos: [x, 0., 0.],
        kind: KIND_WALKER_BASE,
        ..Default::default()
    }
}

#[test]
fn crowd_separation_respects_the_floor_and_the_walkway_edge() {
    let _settings = lock_feed_settings();
    for (social, radius) in [(false, BODY_RADIUS_M), (true, PERSONAL_RADIUS_M)] {
        set_personal_space(social);
        let mut feed = platform(UPPER, 0.);
        let mut frame = Frame {
            regions: vec![AgentBuffer {
                walkers: vec![walker(1, -0.8), walker(2, 0.04)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut out = vec![];
        // Enter from ground level to establish the lower person's route under the deck.
        feed.build(&frame, &mut out);
        assert_eq!([out[0].pos[1], out[1].pos[1]], [0., UPPER]);
        frame.regions[0].walkers[0] = walker(1, 0.04);
        out.clear();
        feed.build(&frame, &mut out);
        assert_eq!(out[0].pos, [0.04, 0., 0.]);
        assert_eq!(out[1].pos, [0.04, UPPER, 0.]);
        frame.regions[0].walkers = vec![walker(2, 0.04), walker(3, 0.24)];
        out.clear();
        feed.build(&frame, &mut out);
        assert_eq!(
            [out[0].pos[1], out[1].pos[1]],
            [UPPER, UPPER],
            "crowd moved a walker off the deck"
        );
        let gap = (out[0].pos[0] - out[1].pos[0]).hypot(out[0].pos[2] - out[1].pos[2]);
        assert!(
            f64::from(gap) >= 2. * radius - 0.002,
            "spacing {gap} m with social spacing {social}"
        );
    }
    set_personal_space(false);
}

#[test]
fn a_physical_car_occupies_its_current_floor_and_leaves_a_supported_exit() {
    let _settings = lock_feed_settings();
    let feed = FeedThread::start(platform(UPPER, 0.));
    let frame = Frame {
        regions: vec![AgentBuffer {
            walkers: vec![walker(9, 0.04)],
            vehicles: vec![AgentInstance {
                id: 7,
                pos: [0.50, 0., 0.],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    // The lane stays on the lower level. The body can be carried onto the
    // platform or go airborne while the regional route remains underneath.
    for y in [0., UPPER, UPPER + 5., UPPER] {
        let body = ([0.50, y, 0.], glam::Quat::IDENTITY.to_array(), 0.);
        feed.post_with_traffic(frame.clone(), [(7, body)].into_iter().collect());
        let out = feed.wait_output(5.).expect("completed traffic feed");
        assert_eq!(out[0].pos, body.0);
        let feet = out[1].pos;
        if y == UPPER {
            assert_eq!(feet[1], UPPER, "car pushed the person off the platform");
            assert!(
                feet[0] > 1.5 && feet[2].abs() < 0.01,
                "the nearest supported exit is across the car: {feet:?}"
            );
        } else {
            assert_eq!(
                feet,
                [0.04, UPPER, 0.],
                "car on another level displaced the person"
            );
        }
    }
    // The upright car leaves enough shoulder room here. On its side the
    // roof extends toward the person, past the original wheel track.
    for rolled in [false, true] {
        let angle = if rolled {
            std::f32::consts::FRAC_PI_2
        } else {
            0.
        };
        let body = (
            [1.45, UPPER, 0.],
            glam::Quat::from_rotation_z(angle).to_array(),
            0.,
        );
        feed.post_with_traffic(frame.clone(), [(7, body)].into_iter().collect());
        let out = feed.wait_output(5.).expect("completed traffic feed");
        let feet = out[1].pos;
        if rolled {
            assert!(
                feet[0] > 1.70 && feet[1] == UPPER,
                "the overturned roof occupies this space: {feet:?}"
            );
        } else {
            assert_eq!(feet, [0.04, UPPER, 0.]);
        }
    }
}

#[test]
fn a_crowd_correction_distinguishes_a_step_from_a_drop() {
    let _settings = lock_feed_settings();
    for (height, can_step_down) in [(0.18, true), (0.42, false)] {
        let mut feed = platform(height, 0.);
        let frame = Frame {
            regions: vec![AgentBuffer {
                walkers: vec![walker(1, 0.04), walker(2, 0.24)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut out = vec![];
        feed.build(&frame, &mut out);
        let outside = out[0].pos;
        assert_eq!(
            outside[0] < 0.,
            can_step_down,
            "walker at platform edge, height {height}, feet {outside:?}"
        );
        assert_eq!(outside[1], if can_step_down { 0. } else { height });
        assert!(out[1].pos[0] - outside[0] >= 2. * BODY_RADIUS_M as f32 - 0.002);
    }
}

#[test]
fn a_supported_vehicle_exit_can_follow_an_incline() {
    let _settings = lock_feed_settings();
    let feed = FeedThread::start(platform(UPPER, 3.2));
    let frame = Frame {
        regions: vec![AgentBuffer {
            walkers: vec![walker(9, 0.04)],
            vehicles: vec![AgentInstance {
                id: 7,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let car = ([0.5, UPPER, 0.], glam::Quat::IDENTITY.to_array(), 0.);
    feed.post_with_traffic(frame, [(7, car)].into_iter().collect());
    let out = feed.wait_output(5.).unwrap();
    let feet = out[1].pos;
    assert!(
        feet[0] > 1.5 && (feet[1] - UPPER - feet[0] * 0.8).abs() < 0.002,
        "car clearance along the incline: {feet:?}"
    );
}

#[test]
fn walkers_on_both_routes_clear_a_corridor_car_only_on_their_floor() {
    let _settings = lock_feed_settings();
    let feed = FeedThread::start(platform(UPPER, 0.));
    // The car and the regional pedestrian deliberately share a numeric ID.
    // A physical pose belongs to the corridor car even when its lane is below.
    let frame = Frame {
        regions: vec![AgentBuffer {
            walkers: vec![walker(7, 0.04)],
            ..Default::default()
        }],
        corridor: vec![
            CorridorInstance {
                id: 7,
                pos: [0.5, 0., 0.],
                ..Default::default()
            },
            CorridorInstance {
                id: 8,
                pos: [0.04, UPPER, 0.9],
                kind: KIND_WALKER_BASE,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    for car_y in [0., UPPER, UPPER + 5.] {
        let car = ([0.5, car_y, 0.], glam::Quat::IDENTITY.to_array(), 0.);
        feed.post_with_traffic(frame.clone(), [((1 << 63) | 7, car)].into_iter().collect());
        let out = feed.wait_output(10.).unwrap();
        assert_eq!(out[1].pos, car.0);
        for (actor, z) in [(0, 0.), (2, 0.9)] {
            let feet = out[actor].pos;
            if car_y == UPPER {
                let outside = (feet[0] - 0.5).abs() >= 1.26 || feet[2].abs() >= 1.995;
                assert!(
                    outside
                        && (0.0..=4.0).contains(&feet[0])
                        && feet[2].abs() <= 3.
                        && feet[1] == UPPER,
                    "walker {actor} must clear the chassis on the supported side: {feet:?}"
                );
            } else {
                assert_eq!(feet, [0.04, UPPER, z], "vehicle on another level");
            }
        }
    }
}

#[test]
fn a_regional_and_a_corridor_walker_share_room_at_a_deck_edge() {
    let _settings = lock_feed_settings();
    let mut feed = platform(UPPER, 0.);
    let mut frame = Frame {
        regions: vec![AgentBuffer {
            walkers: vec![walker(3, 0.04)],
            ..Default::default()
        }],
        corridor: vec![CorridorInstance {
            id: 3,
            pos: [0.24, 0., 0.],
            kind: KIND_WALKER_BASE,
            ..Default::default()
        }],
        ..Default::default()
    };
    for height in [0., UPPER] {
        frame.corridor[0].pos[1] = height;
        let mut out = vec![];
        feed.build(&frame, &mut out);
        let (a, b) = (out[0].pos, out[1].pos);
        assert_eq!([a[1], b[1]], [UPPER, height]);
        if height == UPPER {
            assert!(
                a[0] >= 0. && f64::from(b[0] - a[0]) >= 2. * BODY_RADIUS_M - 0.002,
                "the two people need separate footholds on the deck: {a:?} and {b:?}"
            );
        } else {
            assert_eq!([a[0], b[0]], [0.04, 0.24]);
        }
    }
}

#[test]
fn two_people_leaving_the_same_side_of_a_car_have_separate_footholds() {
    let _settings = lock_feed_settings();
    let mut feed = platform(UPPER, 0.);
    let frame = Frame {
        regions: vec![AgentBuffer {
            walkers: vec![walker(3, 0.04)],
            ..Default::default()
        }],
        corridor: vec![
            CorridorInstance {
                id: 3,
                pos: [0.30, UPPER, 0.],
                kind: KIND_WALKER_BASE,
                ..Default::default()
            },
            CorridorInstance {
                id: 4,
                pos: [0.5, UPPER, 0.],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut out = vec![];
    feed.build(&frame, &mut out);
    let [a, b] = [out[0].pos, out[1].pos];
    let gap = f64::from((a[0] - b[0]).hypot(a[2] - b[2]));
    for feet in [a, b] {
        assert!(
            feet[0] >= 1.76 && feet[0] <= 4.0 && feet[1] == UPPER,
            "foothold {feet:?}"
        );
    }
    assert!(
        gap >= 2. * BODY_RADIUS_M - 0.002,
        "same vehicle exit: {a:?} and {b:?}, gap {gap:.3}"
    );
}

#[test]
fn a_second_car_can_block_a_side_exit_on_the_same_level() {
    let _settings = lock_feed_settings();
    for height in [0., UPPER] {
        for reverse in [false, true] {
            let mut feed = platform(UPPER, 0.);
            let mut cars = vec![
                CorridorInstance {
                    id: 12,
                    pos: [0.5, UPPER, 0.],
                    ..Default::default()
                },
                CorridorInstance {
                    id: 13,
                    pos: [2.65, height, 0.],
                    ..Default::default()
                },
            ];
            if reverse {
                cars.reverse();
            }
            let frame = Frame {
                regions: vec![AgentBuffer {
                    walkers: vec![walker(3, 0.04)],
                    ..Default::default()
                }],
                corridor: cars,
                ..Default::default()
            };
            let mut out = vec![];
            feed.build(&frame, &mut out);
            let [x, y, z] = out[0].pos;
            assert_eq!(y, UPPER);
            if height == UPPER {
                assert!(
                    (0.0..=4.0).contains(&x) && (1.995..=3.0).contains(&z.abs()),
                    "the side exit is occupied, deck position {:?}",
                    out[0].pos
                );
            } else {
                assert!(x > 1.76 && z.abs() < 0.01, "a lower car blocked the exit");
            }
        }
    }
}
