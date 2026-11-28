use super::*;

#[test]
fn the_observation_rooms_support_their_drawn_floors_from_the_lift_to_the_windows() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let access = eo_landmarks::access::AccessSet::build(&|x, z| grid.sample(x, z));
    let buildings = BuildingSet::empty().with_decks(eo_net::game::decks_of(&access));
    let mut cw = CollisionWorld::new(Arc::new(ElevationGround(grid)), Arc::new(buildings), 1);
    let mut rooms = 0;
    let mut misses = 0;
    for deck in access.decks.iter().filter(|d| d.interior.is_some()) {
        let mut triangles = Vec::new();
        for surface in &deck.structures {
            let mesh = &surface.mesh;
            for face in mesh.indices.chunks_exact(3) {
                let tri = [0, 1, 2].map(|i| {
                    let v = 3 * face[i] as usize;
                    [mesh.positions[v], mesh.positions[v+1], mesh.positions[v+2]]
                });
                if tri.iter().all(|p| (f64::from(p[1]) - deck.floor_y).abs() < 0.002) {
                    triangles.push(tri);
                }
            }
        }
        let drawn = eo_agents::feed::Decks::new(triangles);
        let planned = eo_agents::feed::Decks::from_world_triangles(deck.walk.iter().copied());
        let mut probes = 0;
        let mut missing = 0;
        let mut landings = 0;
        let mut max_gap = 0.0_f64;
        for start in std::iter::once(&deck.view).chain(deck.lifts.iter().map(|l| &l.top)) {
            let feet = DVec3::from_array(start.feet);
            cw.load_blocking(feet, 8.);
            for spoke in 0..48 {
                let angle = f64::from(spoke) * std::f64::consts::TAU / 48.;
                let dir = DVec3::new(angle.cos(), 0., angle.sin());
                // Stop before the glazing, walls or lift shaft. The drawn slab can
                // extend into an inaccessible core behind them.
                let wall = cw.raycast(feet + DVec3::Y, dir, 80., pworld::layer::WORLD, None)
                    .map_or(80., |hit| hit.distance);
                if spoke % 12 == 0 && wall >= 1. {
                    let p = feet + dir * (wall * 0.5).min(3.);
                    let y = drawn.top(p.x, p.z).expect("occupied side of the room has a drawn floor");
                    let mut body = eo_player::character::Character::new(DVec3::new(p.x, y + 0.45, p.z));
                    for _ in 0..90 {
                        body.step(&cw, 0., 0., true);
                        assert!(body.feet().y >= y - 0.01, "{}: fell through the room at {p:?}", deck.name);
                    }
                    assert!(body.grounded && (body.feet().y - y).abs() < 0.025, "{}: missed landing at {p:?}", deck.name);
                    landings += 1;
                }
                for step in 0..=(wall / 0.4) as usize {
                    let p = feet + dir * (step as f64 * 0.4);
                    let Some(y) = drawn.top(p.x, p.z) else { break };
                    let origin = DVec3::new(p.x, y + 0.03, p.z);
                    let Some(hit) = cw.raycast(origin, DVec3::NEG_Y, 0.06, pworld::layer::STREET, None) else {
                        if missing < 3 {
                            let nearby = [DVec3::X, -DVec3::X, DVec3::Z, -DVec3::Z].map(|v| cw.raycast(origin + v * 0.01, DVec3::NEG_Y, 0.06, pworld::layer::STREET, None).is_some());
                            eprintln!("{} at {p:?}, drawn {y:.5}, planned {:?}, neighbouring centimetre {nearby:?}, start {feet:?}, spoke {spoke} step {step} wall {wall:.4}", deck.name, planned.top(p.x, p.z));
                        }
                        missing += 1;
                        continue;
                    };
                    max_gap = max_gap.max((hit.point.y - y).abs());
                    probes += 1;
                }
            }
        }
        assert!(probes > 100 && max_gap < 0.005, "{}: {probes} probes, gap {max_gap:.4} m", deck.name);
        assert!(landings > 0, "{}: no clear landing space", deck.name);
        eprintln!("{}: {probes} floor probes, {missing} misses, {landings} landings, largest height difference {max_gap:.5} m", deck.name);
        misses += missing;
        rooms += 1;
    }
    assert_eq!(rooms, 3);
    assert_eq!(misses, 0, "missing support under drawn observation floors");
}

fn house_args(switch: &str) -> crate::Args {
    let (mut args, _) = parse(&[switch.into()]).unwrap();
    args.kit = false;
    args.models = false;
    args.landmarks = false;
    args
}

#[test]
fn reopening_a_location_keeps_the_fitted_rooms_support() {
    let _guard = house_tests::world_lock();
    let args = house_args("--house-fitted-dining");
    let (saved, feet) = {
        let world = load_world(&args).unwrap();
        let house = world.house.as_ref().unwrap();
        let table = house.point(3.137, 0., 6.537) + DVec3::Y * 0.75;
        let (mut cw, _) = collision_world(&world, 1, false).unwrap();
        cw.load_blocking(table, 32.);
        let mut player = Player::new(table + DVec3::Y * 0.08, 0.);
        for _ in 0..60 {
            player.fixed_step(&mut cw, &StepInput::default());
        }
        let feet = player.character.feet();
        assert!(player.character.grounded && (feet.y - table.y).abs() < 0.025);
        let (path, _) = trace::Location::capture(&player, &cw, &world.city,
            trace::LocationWorld::of(&args, world.street.as_deref()), args.look.sky, &std::env::temp_dir()).unwrap();
        let saved = trace::Location::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        (saved, feet)
    };
    let mut options = house_args("--house-photo");
    saved.apply_world(&mut options).unwrap();
    let world = load_world(&options).unwrap();
    let (mut cw, _) = collision_world(&world, 1, false).unwrap();
    let mut player = saved.restore(&mut cw).unwrap();
    for _ in 0..120 {
        player.fixed_step(&mut cw, &StepInput::default());
        assert!((player.character.feet().y - feet.y).abs() < 0.025,
            "21 Beaver Street dining room: saved feet {feet:?}, reopened {:?}", player.character.feet());
    }
    assert!(player.character.grounded);
}

#[test]
fn both_interior_stair_flights_support_their_drawn_treads() {
    let _guard = house_tests::world_lock();
    let world = load_world(&house_args("--house-fitted-dining")).unwrap();
    let house = world.house.as_ref().unwrap();
    let (mut cw, _) = collision_world(&world, 1, false).unwrap();
    cw.load_blocking(house.entry_position(), 32.);
    let parcel = house.parcel_to_world().inverse();
    let corners = [(4.82, 4.58), (4.82, 9.), (6.84, 4.58), (6.84, 9.)]
        .map(|(x,z)| house.point(x, 0., z).as_vec3());
    let min = corners.into_iter().reduce(Vec3::min).unwrap();
    let max = corners.into_iter().reduce(Vec3::max).unwrap();
    let mut checked = std::collections::BTreeSet::new();
    let mut worst = (0.0, DVec3::ZERO);
    for object in &world.scene.objects {
        if object.min[0] > max.x || object.max[0] < min.x || object.min[2] > max.z || object.max[2] < min.z {
            continue;
        }
        let first = object.first_index as usize;
        for face in world.scene.indices[first..first + object.index_count as usize].chunks_exact(3) {
            let vertices = [0, 1, 2].map(|i| world.scene.vertices[(i64::from(object.base_vertex) + i64::from(face[i])) as usize]);
            if vertices.iter().any(|v| eo_assets::glb::unpack_normal(v.normal).y < 0.99) { continue; }
            let points = vertices.map(|v| Vec3::from(v.pos).as_dvec3());
            let p = (points[0] + points[1] + points[2]) / 3.;
            let local = parcel.transform_point3(p.as_vec3());
            let tread = (local.y * 6.).round() as i32;
            if !(1..=40).contains(&tread) || !(4.82..6.84).contains(&local.x)
                || !(4.58..9.).contains(&local.z) || (local.y - tread as f32 / 6. - 0.002).abs() > 0.005 { continue; }
            checked.insert(tread);
            let hit = cw.raycast(p + DVec3::Y * 0.025, DVec3::NEG_Y, 0.5, pworld::layer::STREET, None);
            let gap = hit.map_or(0.5, |h| p.y - h.point.y);
            if gap > worst.0 { worst = (gap, p); }
        }
    }
    assert_eq!(checked.len(), 40, "drawn stair treads {checked:?}");
    eprintln!("21 Beaver Street: {} interior treads, largest gap {:.4} m at {:?}", checked.len(), worst.0, worst.1);
    assert!(worst.0 < 0.025, "largest gap under a drawn tread {:.3} m at {:?}", worst.0, worst.1);
}
