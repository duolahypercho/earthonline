//! Match the drawn terrain and the road at different elevations of a tunnel mouth.
use eo_player::Player;
use eo_player::routes::DrivePilot;
use eo_player::street_ground::StreetGround;
use eo_player::world::{BuildingSet, CollisionWorld, GroundProvider, LOAD_RADIUS_M, layer};
use eo_structures::{StructureOptions, TerrainCut};
use glam::DVec3;
use std::sync::Arc;

#[test]
fn presidio_parkway_arrives_on_the_pavement_and_has_solid_terrain_below() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let elevation = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let ground = Arc::new(
        StreetGround::build_joined(&city, elevation, Some(StructureOptions { tunnels: true }))
            .unwrap(),
    );
    let roads = ground.decks().unwrap();
    let bore = roads
        .chains()
        .iter()
        .find(|c| c.pieces.iter().any(|p| p.way == "167952618"))
        .unwrap();
    let approach = ground
        .world
        .city
        .segments
        .iter()
        .find(|s| s.street_id.as_deref() == Some("sf-street-778988074") && s.id == "sf-seg-1860")
        .unwrap();
    let [x, z] = approach.points[0];
    let f = bore.frame(0.);
    let yaw = (f.x - x).atan2(f.z - z);
    let mut world = CollisionWorld::new(ground.clone(), Arc::new(BuildingSet::empty()), 2);
    world.load_blocking(DVec3::new(x, 0., z), LOAD_RADIUS_M);
    let terrain = world.floor_y(x, z);
    let mapped = ground.world.carriageway_near(x, z, terrain).unwrap();
    assert!(
        mapped > terrain + 4.,
        "the fixture needs two distinct levels"
    );
    let arrival = world.arrival_y(x, z);
    assert!(
        (arrival - mapped).abs() < 0.025,
        "map arrival at {arrival} under the road at {mapped}"
    );
    let mut player = Player::new(DVec3::new(x, arrival, z), yaw);
    let car = player.spawn_vehicle(&mut world, x, z, yaw);
    player.enter_vehicle(car);
    let mut path = vec![[x, z]];
    for step in 0..=50 {
        let frame = bore.frame(f64::from(step) * 2.);
        path.push([frame.x, frame.z]);
    }
    let mut pilot = DrivePilot::new(path, 8.);
    let mut steps_in_bore = 0;
    let mut max_error = 0.0_f64;
    for _ in 0..1800 {
        let input = pilot.input(&player, &world);
        player.fixed_step(&mut world, &input);
        let p = player.vehicles[car].datum(&world);
        let (s, distance) = bore.project(p.x, p.z);
        if s > 0.1 && distance < bore.half_at(s) {
            let deck = roads.deck_top(bore.id, p.x, p.z).unwrap();
            max_error = max_error.max((p.y - deck).abs());
            assert!(
                max_error < 0.20,
                "car left the pavement by {max_error} m at {p:?}"
            );
            steps_in_bore += 1;
        }
        if pilot.done() {
            break;
        }
    }
    assert!(pilot.done() && steps_in_bore > 350);
    // Under the approach the original terrain is already below the cut floor.
    // A driver or a walker there must still meet the ground before the bore.
    let mut samples = 0;
    for i in 1..60 {
        let t = f64::from(i) / 60.;
        let (px, pz) = (x + t * (f.x - x), z + t * (f.z - z));
        if ground.terrain_cut(px, pz).is_none() {
            continue;
        }
        let floor = world.floor_y(px, pz);
        let h = world
            .raycast(
                DVec3::new(px, floor + 0.08, pz),
                DVec3::NEG_Y,
                0.16,
                layer::WORLD,
                None,
            )
            .unwrap_or_else(|| {
                panic!("open-cut terrain with no physical floor at {px}, {floor}, {pz}")
            });
        assert!((h.point.y - floor).abs() < 0.01);
        samples += 1;
    }
    assert!(samples > 10);
    let (px, pz) = ((x + f.x) * 0.5, (z + f.z) * 0.5);
    let floor = world.floor_y(px, pz);
    let mut walker = eo_player::character::Character::new(DVec3::new(px, floor + 0.7, pz));
    for _ in 0..90 {
        walker.step(&world, 0.0, 0.0, true);
    }
    let feet = walker.feet();
    assert!(
        walker.grounded && (feet.y - world.floor_y(feet.x, feet.z)).abs() < 0.10,
        "walker failed to land beneath the approach at {feet:?}"
    );
    println!(
        "Presidio Parkway entry at {arrival:.3} m above terrain at {terrain:.3} m, max datum error {max_error:.3} m over {steps_in_bore} bore ticks, {samples} open-cut ground probes"
    );
}

#[test]
fn the_terrain_stops_at_the_actual_canopy_edge() {
    struct Portal(TerrainCut);
    impl GroundProvider for Portal {
        fn ground_y(&self, _: f64, _: f64) -> f64 {
            3.0
        }
        fn terrain_cut(&self, x: f64, z: f64) -> Option<f64> {
            self.0.contains(x, z).then_some(self.0.floor)
        }
        fn terrain_cuts_in(&self, min: [f64; 2], max: [f64; 2]) -> Vec<&TerrainCut> {
            if self.0.min[0] > max[0]
                || self.0.max[0] < min[0]
                || self.0.min[1] > max[1]
                || self.0.max[1] < min[1]
            {
                vec![]
            } else {
                vec![&self.0]
            }
        }
    }
    let (l, r) = (1.31, 6.71);
    let c = TerrainCut {
        chain: 0,
        s0: 0.0,
        s1: 6.,
        ring: vec![[l, l], [r, l], [r, r], [l, r]],
        floor: -2.,
        min: [l, l],
        max: [r, r],
        canopy: true,
    };
    let mut w = CollisionWorld::new(Arc::new(Portal(c)), Arc::new(BuildingSet::empty()), 1);
    w.load_blocking(DVec3::ZERO, 40.);
    for (x, solid) in [
        (l - 0.04, true),
        (l + 0.04, false),
        (r - 0.04, false),
        (r + 0.04, true),
    ] {
        let hit = w.raycast(
            DVec3::new(x, 3.10, 3.20),
            DVec3::NEG_Y,
            0.2,
            layer::WORLD | layer::STREET,
            None,
        );
        assert_eq!(hit.is_some(), solid, "terrain query at x={x}");
        assert_eq!(w.floor_y(x, 3.2), if solid { 3.0 } else { -2.0 });
    }
}
