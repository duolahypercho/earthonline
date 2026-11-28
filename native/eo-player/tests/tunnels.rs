//! The tunnels (eo-structures with `--tunnels`, stage A7) in the collision
//! world, on the real city: the heightfield leaves out the ground the
//! tunnels' approach cuts go through, the floor there is the cut's, and a
//! car driven along the General Douglas MacArthur Tunnel, from the street
//! before one portal to the street past the other, stays on its floor.

use eo_player::routes::DrivePilot;
use eo_player::street_ground::StreetGround;
use eo_player::world::{self, BuildingSet, CollisionWorld, GroundProvider, TileKey, cook_tile};
use eo_player::{Mode, Player, StepInput};
use eo_structures::{ChainKind, StructureOptions};
use glam::DVec3;
use std::sync::Arc;

#[test]
fn a_car_driven_through_the_macarthur_tunnel_stays_on_its_floor() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let elevation = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let lift: serde_json::Value = serde_json::from_slice(&std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../eo-assets/data/authored-placement.json")).unwrap()).unwrap();
    let buildings = Arc::new(BuildingSet::from_city(&city, lift["constructionLiftM"].as_f64().unwrap()));
    // (the street built with the structures' joins: the stubs come down the
    // portals' approach cuts.)
    let ground = Arc::new(StreetGround::build_joined(&city, elevation.clone(), Some(StructureOptions { tunnels: true })).unwrap());
    let sw = ground.decks().expect("structures");
    let t = sw.chains().iter().find(|c| c.kind == ChainKind::Tunnel && c.is_built() && c.name().contains("MacArthur")).expect("the MacArthur tunnel");
    let len = t.length();
    // the cuts by its portals: the heightfield has no cells there, and the
    // floor there is the cut's.
    let cuts: Vec<_> = sw.terrain_cuts().iter().filter(|k| k.chain == t.id).collect();
    assert!(!cuts.is_empty(), "no cut by its portals");
    let probe = t.frame(1.0);
    let key = TileKey::of(probe.x, probe.z);
    let cooked = cook_tile(key, ground.as_ref(), &buildings);
    let plain = cook_tile(key, &StreetGround::new(ground.world.clone(), None), &buildings);
    assert!(cooked.heights.cut.iter().any(|f| f.is_finite()), "cells cut by the portal");
    assert!(plain.heights.cut.is_empty(), "no cut without the tunnels");
    let cut = sw.terrain_cut_at(probe.x, probe.z).expect("the portal's cut");
    assert_eq!(cooked.heights.height_at(probe.x, probe.z), Some(cut.floor as f32 as f64));
    assert_eq!(ground.terrain_cut(probe.x, probe.z), Some(cut.floor));
    // the drive: 40 m of street before the portal, the bore, 40 m after.
    let (f0, f1) = (t.frame(0.0), t.frame(len));
    let mut path: Vec<[f64; 2]> = (0..=8).map(|k| [f0.x - f0.tx * (40.0 - 5.0 * k as f64), f0.z - f0.tz * (40.0 - 5.0 * k as f64)]).collect();
    let n = (len / 4.0).ceil() as usize;
    path.extend((1..n).map(|k| t.frame(len * k as f64 / n as f64)).map(|f| [f.x, f.z]));
    path.extend((0..=8).map(|k| [f1.x + f1.tx * 5.0 * k as f64, f1.z + f1.tz * 5.0 * k as f64]));
    let mut cw = CollisionWorld::new(ground.clone(), buildings.clone(), 4);
    let p0 = path[0];
    cw.load_blocking(DVec3::new(p0[0], 0.0, p0[1]), world::LOAD_RADIUS_M);
    let feet = DVec3::new(p0[0] + 3.0, cw.surface_y(p0[0] + 3.0, p0[1]), p0[1]);
    let mut player = Player::new(feet, 0.0);
    player.spawn_vehicle(&mut cw, p0[0], p0[1], (path[2][0] - p0[0]).atan2(path[2][1] - p0[1]));
    for _ in 0..60 {
        player.fixed_step(&mut cw, &StepInput::default());
    }
    player.fixed_step(&mut cw, &StepInput { vehicle_toggle: true, ..Default::default() });
    assert!(matches!(player.mode, Mode::Driving(0)), "could not enter the car");
    let mut pilot = DrivePilot::new(path, 12.0);
    let (mut steps, mut inside, mut max_dev, mut deepest, mut contact4) = (0usize, 0usize, 0f64, 0f64, 0usize);
    while !pilot.done() && steps < 60 * 120 {
        let input = pilot.input(&player, &cw);
        player.fixed_step(&mut cw, &input);
        let v = &player.vehicles[0];
        let d = v.datum(&cw);
        cw.pump(d);
        steps += 1;
        if v.wheels_in_contact() == 4 {
            contact4 += 1;
        }
        let (s, dist) = t.project(d.x, d.z);
        if s > 2.0 && s < len - 2.0 && dist < t.half_at(s) {
            inside += 1;
            let top = sw.deck_top(t.id, d.x, d.z).expect("the bore's roadway");
            let dev = d.y - top;
            max_dev = max_dev.max(dev.abs());
            deepest = deepest.max(ground.world.ground(d.x, d.z) - d.y);
            assert!(dev.abs() < 0.3, "step {steps} (s {s:.1}): the car's datum {dev:.3} m off the bore's roadway at {d:?}");
        }
    }
    let d = player.vehicles[0].datum(&cw);
    let (s_end, dist_end) = t.project(d.x, d.z);
    println!(
        "MacArthur tunnel (chain {}, {len:.0} m, {} cuts): drove in {:.1} s, {inside} steps in the bore, datum within {max_dev:.3} m of its roadway, up to {deepest:.1} m under the ground; four wheels down {:.1} %",
        t.id,
        cuts.len(),
        steps as f64 / 60.0,
        contact4 as f64 * 100.0 / steps as f64
    );
    assert!(pilot.done(), "did not finish the drive in {steps} steps: the car at {d:?}, s {s_end:.1} of {len:.1} ({dist_end:.1} m off the centreline)");
    assert!(inside > 60 * 20, "only {inside} steps in the bore");
    assert!(deepest > 10.0, "never under the hill ({deepest:.1} m)");
    assert!(contact4 as f64 / steps as f64 > 0.95);
}

#[test]
fn stacked_and_forked_highway_tunnels_have_room_for_a_vehicle() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let elevation = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let ground = Arc::new(
        StreetGround::build_joined(&city, elevation, Some(StructureOptions { tunnels: true }))
            .unwrap(),
    );
    let roads = ground.decks().unwrap();
    // Both levels on Yerba Buena Island and both arms of the Presidio
    // Parkway split. These four openings used to contain a solid headwall,
    // abutment, or the neighbouring bore's sidewall.
    for way in ["11415208", "50691047", "167952616", "167952621"] {
        let chain = roads
            .chains()
            .iter()
            .find(|c| c.pieces.iter().any(|p| p.way == way))
            .expect("mapped tunnel");
        assert!(chain.is_built());
        let mut world = CollisionWorld::new(ground.clone(), Arc::new(BuildingSet::empty()), 1);
        // Check continuous segments at both front corners and the centre,
        // inside the space occupied by the car above the pavement.
        let mut previous: Option<[DVec3; 3]> = None;
        let count = (chain.length() / 2.0).ceil() as usize;
        for i in 0..=count {
            let s = chain.length() * i as f64 / count as f64;
            let f = chain.frame(s);
            let y = chain.y_at(s);
            let centre = DVec3::new(f.x, y, f.z);
            if i == 0 || i % 30 == 0 {
                world.load_blocking(centre, world::LOAD_RADIUS_M);
            }
            let points = [-0.8, 0.0, 0.8].map(|u| {
                let (x, z) = (f.x + u * f.nx, f.z + u * f.nz);
                DVec3::new(x, roads.deck_top(chain.id, x, z).unwrap_or(y) + 1.0, z)
            });
            if let Some(prev) = previous {
                for (a, b) in prev.into_iter().zip(points) {
                    let delta = b - a;
                    assert!(
                        world
                            .raycast(
                                a,
                                delta.normalize(),
                                delta.length(),
                                world::layer::WORLD,
                                None
                            )
                            .is_none(),
                        "solid through OSM way {way} at station {s:.1}"
                    );
                }
            }
            previous = Some(points);
        }
    }
    // A car can follow the floor even when its ceiling or the wall between
    // twin tubes has been cut away. Probe the solid collision triangles.
    let mut world = CollisionWorld::new(ground.clone(), Arc::new(BuildingSet::empty()), 1);
    for way in ["50691047", "23874453", "59766608"] {
        let tunnel = roads
            .chains()
            .iter()
            .find(|c| c.pieces.iter().any(|piece| piece.way == way))
            .unwrap();
        for bore in roads.bores().iter().filter(|b| b.chain == tunnel.id) {
            let count = ((bore.s1 - bore.s0) / 8.0).ceil() as usize;
            for i in 0..count {
                let s = bore.s0 + (i as f64 + 0.5) * (bore.s1 - bore.s0) / count as f64;
                let f = tunnel.frame(s);
                let eye = DVec3::new(f.x, tunnel.y_at(s) + 1.25, f.z);
                if i % 6 == 0 {
                    world.load_blocking(eye, world::LOAD_RADIUS_M);
                }
                assert!(
                    world
                        .raycast(
                            eye,
                            DVec3::Y,
                            eo_structures::BORE_CLEAR_M,
                            world::layer::WORLD,
                            None
                        )
                        .is_some(),
                    "missing ceiling above OSM way {way} at station {s:.1}"
                );
                if way != "50691047" {
                    for sign in [-1.0, 1.0] {
                        let side = sign * DVec3::new(f.nx, 0.0, f.nz);
                        assert!(
                            world
                                .raycast(
                                    eye,
                                    side,
                                    2.0 * tunnel.half_at(s),
                                    world::layer::WORLD,
                                    None
                                )
                                .is_some(),
                            "missing sidewall on OSM way {way} at station {s:.1}"
                        );
                    }
                }
            }
        }
    }
}
