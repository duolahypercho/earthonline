//! Drive across each restored Islais Creek road leaf and both of its hinges.
use std::sync::Arc;

use eo_player::routes::DrivePilot;
use eo_player::street_ground::StreetGround;
use eo_player::world::{BuildingSet, CollisionWorld, LOAD_RADIUS_M};
use eo_player::{Mode, Player, StepInput};
use eo_structures::Chain;
use glam::DVec3;

fn sample(chain: &Chain, start: f64, end: f64) -> Vec<[f64; 2]> {
    let steps = ((end - start).abs() / 2.0).ceil() as usize;
    (0..=steps)
        .map(|i| {
            let f = chain.frame(start + (end - start) * i as f64 / steps as f64);
            [f.x, f.z]
        })
        .collect()
}

/// From the hinge outward along the fixed approach.
fn approach<'a>(leaf: &Chain, end: usize, chains: &'a [Chain]) -> (&'a Chain, f64, f64) {
    let node = leaf.ends[end].node;
    for c in chains.iter().filter(|c| c.id != leaf.id && c.is_built()) {
        for (i, edge) in c.ends.iter().enumerate() {
            if edge.node == node {
                let run = c.length().min(30.0);
                let (hinge, outer) = if i == 0 {
                    (0.0, run)
                } else {
                    (c.length(), c.length() - run)
                };
                return (c, hinge, outer);
            }
        }
    }
    panic!("missing fixed approach to {} at node {node}", leaf.name());
}

#[test]
fn drive_both_hinges_of_each_islais_road_span() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let streets = Arc::new(
        StreetGround::build_joined(
            &city,
            grid,
            Some(eo_structures::StructureOptions { tunnels: true }),
        )
        .unwrap(),
    );
    let sw = streets.decks().expect("built road structures");
    let placement: serde_json::Value =
        serde_json::from_str(include_str!("../../eo-assets/data/authored-placement.json")).unwrap();
    let buildings = Arc::new(BuildingSet::from_city(
        &city,
        placement["constructionLiftM"].as_f64().unwrap(),
    ));
    for span in eo_data::movable_spans::spans() {
        let way = span.road.id.key();
        let leaf = sw
            .chains()
            .iter()
            .find(|c| c.is_built() && c.ways().contains(&way.as_str()))
            .expect("road leaf");
        for start in 0..2 {
            if span.road.oneway.as_bool().unwrap() && (start == 1) != leaf.pieces[0].reversed {
                continue;
            }
            let (incoming, hinge, outer) = approach(leaf, start, sw.chains());
            let (outgoing, far_hinge, far_outer) = approach(leaf, 1 - start, sw.chains());
            let (s0, s1) = if start == 0 {
                (0.0, leaf.length())
            } else {
                (leaf.length(), 0.0)
            };
            let mut path = sample(incoming, outer, hinge);
            path.extend(sample(leaf, s0, s1).into_iter().skip(1));
            path.extend(sample(outgoing, far_hinge, far_outer).into_iter().skip(1));

            let [x, z] = path[0];
            let y = incoming.y_at(outer);
            let heading = (path[1][0] - x).atan2(path[1][1] - z);
            let mut world = CollisionWorld::new(streets.clone(), buildings.clone(), 4);
            world.load_blocking(DVec3::new(x, y, z), LOAD_RADIUS_M);
            let mut player = Player::new(DVec3::new(x, y, z), heading);
            player.spawn_vehicle_below(&mut world, DVec3::new(x, y + 0.5, z), heading);
            let initial = player.vehicles[0].datum(&world);
            player.character.teleport_feet(
                initial + DVec3::new(2.0 * heading.cos(), 0.0, -2.0 * heading.sin()),
            );
            for _ in 0..60 {
                player.fixed_step(&mut world, &StepInput::default());
            }
            player.fixed_step(
                &mut world,
                &StepInput {
                    vehicle_toggle: true,
                    ..Default::default()
                },
            );
            assert!(matches!(player.mode, Mode::Driving(0)));
            let mut pilot = DrivePilot::new(path, 8.0);
            let mut worst = 0.0_f64;
            let mut crossed_middle = false;
            for _ in 0..3600 {
                let input = pilot.input(&player, &world);
                player.fixed_step(&mut world, &input);
                let car = player.vehicles[0].datum(&world);
                let (c, s, distance) = [incoming, leaf, outgoing]
                    .into_iter()
                    .map(|c| {
                        let (s, _) = c.project(car.x, car.z);
                        let f = c.frame(s);
                        (c, s, (car.x - f.x).hypot(car.z - f.z))
                    })
                    .min_by(|a, b| a.2.total_cmp(&b.2))
                    .unwrap();
                assert!(distance < c.half_at(s), "left the carriageway on {way}");
                worst = worst.max((car.y - c.y_at(s)).abs());
                crossed_middle |= c.id == leaf.id && (s - leaf.length() / 2.0).abs() < 1.0;
                if pilot.done() {
                    break;
                }
            }
            let car = player.vehicles[0].datum(&world);
            let (s, _) = outgoing.project(car.x, car.z);
            let beyond = (s - far_hinge).abs();
            println!(
                "{} way {way}, direction {start}: {beyond:.1} m onto far approach, maximum deck error {worst:.3} m",
                leaf.name()
            );
            assert!(
                pilot.done() && crossed_middle && beyond > 2.0 && worst < 0.35,
                "drive across {way}"
            );
        }
    }
}
