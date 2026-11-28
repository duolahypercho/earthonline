use super::*;
use eo_landmarks::{bay_bridge, golden_gate, suspension::DeckStandards};

/// Select asphalt triangles from the actual drawn deck group. These are the
/// upward faces spanning the carriageway. Paint, sidewalk and edge beams are narrower.
fn drawn_roads(world: &World) -> Vec<(&'static str, Vec<[DVec3; 3]>)> {
    let js = eo_landmarks::LandmarksJs::get();
    let ca = DeckStandards::load(&eo_data::CaStandards::load(&world.paths).unwrap()).unwrap();
    let sample = |x, z| world.elevation.sample(x, z);
    let mut roads = Vec::new();
    for (id, is_gate) in [("golden-gate-bridge", true), ("bay-bridge-west", false)] {
        let ground = eo_landmarks::set::ground_of(js, id, &sample);
        let gg = golden_gate::data(js);
        let site = golden_gate::Site::new(&gg);
        let bay = bay_bridge::data(js);
        let frame = bay_bridge::frame(&bay);
        let (built, axis, dimensions) = if is_gate {
            (
                golden_gate::build(js, &ground, &ca),
                site.axis,
                &gg.dimensions,
            )
        } else {
            (
                bay_bridge::build(js, &ground, &ca),
                frame.axis,
                &bay.dimensions,
            )
        };
        let width = eo_landmarks::data::num(dimensions, "roadwayM");
        let across = DVec3::new(-axis[1], 0., axis[0]);
        let (_, mesh) = built
            .groups
            .into_iter()
            .find(|(name, _)| name == "deck")
            .unwrap();
        let mut faces = Vec::new();
        for tri in mesh.indices.chunks_exact(3) {
            let pts = [tri[0], tri[1], tri[2]].map(|idx| {
                let i = 3 * idx as usize;
                DVec3::new(
                    mesh.positions[i].into(),
                    mesh.positions[i + 1].into(),
                    mesh.positions[i + 2].into(),
                )
            });
            let lateral = pts.map(|p| p.dot(across));
            let span = lateral.into_iter().fold(f64::NEG_INFINITY, f64::max)
                - lateral.into_iter().fold(f64::INFINITY, f64::min);
            let normal_y = (pts[1] - pts[0])
                .cross(pts[2] - pts[0])
                .normalize_or_zero()
                .y;
            if normal_y > 0.9 && span > 0.95 * width {
                faces.push(pts);
            }
        }
        assert!(
            faces.len() > 100,
            "{id} only {} road triangles",
            faces.len()
        );
        roads.push((id, faces));
    }
    roads
}

#[test]
fn drawn_bridge_traffic_lanes_support_wheels_from_end_to_end() {
    let _guard = super::house_tests::world_lock();
    let mut failures = Vec::new();
    for structures in [false, true] {
        let flags = if structures {
            vec!["--structures".into()]
        } else {
            vec![]
        };
        let (mut args, _) = parse(&flags).unwrap();
        args.models = false;
        args.agents = false;
        args.kit = false;
        let world = load_world(&args).unwrap();
        let (mut cw, _) = collision_world(&world, 1, args.landmark_access).unwrap();
        let mut last_tile = None;
        for (name, faces) in drawn_roads(&world) {
            let mut samples = 0;
            let mut misses = 0;
            let mut worst = 0.0_f64;
            let mut first = None;
            for tri in &faces {
                for weights in [
                    [1. / 3.; 3],
                    [0.02, 0.02, 0.96],
                    [0.02, 0.96, 0.02],
                    [0.96, 0.02, 0.02],
                ] {
                    let p = tri[0] * weights[0] + tri[1] * weights[1] + tri[2] * weights[2];
                    let tile = eo_street::tile_key(p.x, p.z);
                    if last_tile != Some(tile) {
                        cw.load_blocking(p, 50.);
                        last_tile = Some(tile);
                    }
                    samples += 1;
                    let hit = cw.raycast(
                        p + DVec3::Y * 0.05,
                        DVec3::NEG_Y,
                        0.10,
                        pworld::layer::STREET,
                        None,
                    );
                    if let Some(hit) = hit {
                        worst = worst.max((hit.point.y - p.y).abs());
                    } else {
                        misses += 1;
                        first.get_or_insert(p);
                    }
                }
            }
            println!(
                "{name} structures={structures}: {} drawn asphalt triangles, {samples} probes, {misses} missing, max height error {worst:.5} m, first miss {first:?}",
                faces.len()
            );
            if misses != 0 || worst > 0.01 {
                failures.push((name, structures, misses, worst));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "tire support differs from the drawn roads: {failures:?}"
    );
}

/// A lane from the end faces of the asphalt panels in drawing order. The
/// upper and lower Bay Bridge decks start at identical plan coordinates.
fn lanes(faces: &[[DVec3; 3]], across: f64) -> Vec<Vec<DVec3>> {
    let mut roads: Vec<Vec<DVec3>> = Vec::new();
    for quad in faces.chunks_exact(2) {
        assert_eq!(quad[0][0], quad[1][0]);
        assert_eq!(quad[0][2], quad[1][1]);
        let start = quad[0][0].lerp(quad[0][1], across);
        let end = quad[1][2].lerp(quad[0][2], across);
        if roads.last().and_then(|r| r.last()) != Some(&start) {
            roads.push(vec![start]);
        }
        roads.last_mut().unwrap().push(end);
    }
    roads
}

#[test]
fn playable_cars_drive_the_bridge_approaches_towers_and_both_bay_decks() {
    let _guard = super::house_tests::world_lock();
    for structured in [false, true] {
        let flags = if structured {
            vec!["--structures".into()]
        } else {
            vec![]
        };
        let (mut args, _) = parse(&flags).unwrap();
        args.models = false;
        args.agents = false;
        args.kit = false;
        let world = load_world(&args).unwrap();
        for (name, faces) in drawn_roads(&world) {
            let golden = name == "golden-gate-bridge";
            let mut paths = lanes(&faces, 0.75);
            assert_eq!(paths.len(), if golden { 1 } else { 2 });
            if golden {
                paths.push(lanes(&faces, 0.25).remove(0));
            }
            for (level, mut lane) in paths.into_iter().enumerate() {
                if (golden && level == 1) || (!golden && level == 0) {
                    lane.reverse();
                }
                // Stay on the drawn panels from the first complete wheelbase to the last.
                lane.remove(0);
                lane.pop();
                let (mut cw, _) = collision_world(&world, 1, args.landmark_access).unwrap();
                let start = lane[0];
                let tangent = lane[1] - start;
                let yaw = tangent.x.atan2(tangent.z);
                cw.load_blocking(start, pworld::LOAD_RADIUS_M);
                let mut player = Player::new(start, yaw);
                let (model, cargo_kg) = if level == 0 {
                    (VehicleModel::Sedan, 0)
                } else {
                    (VehicleModel::DeliveryVan, 600)
                };
                player.spawn_config = VehicleConfig {
                    model,
                    cargo_kg,
                    occupants_kg: 80,
                };
                let i = player.spawn_vehicle_below(&mut cw, start + DVec3::Y * 0.35, yaw);
                player.enter_vehicle(i);
                let hold = StepInput {
                    handbrake: true,
                    ..Default::default()
                };
                for _ in 0..120 {
                    player.fixed_step(&mut cw, &hold);
                }
                assert!((player.vehicles[i].datum(&cw).y - start.y).abs() < 0.16);
                let mut pilot = DrivePilot::new(lane.iter().map(|p| [p.x, p.z]).collect(), 15.);
                let mut ticks = 0;
                let mut fewest_wheels = 4;
                let mut largest_loss = 0.0_f64;
                let mut largest_error = 0.0_f64;
                let mut panel = 0;
                while !pilot.done() && ticks < 40_000 {
                    cw.ensure_step_tiles(player.position(&cw));
                    let before = player.vehicles[i].forward_speed(&cw);
                    let input = pilot.input(&player, &cw);
                    player.fixed_step(&mut cw, &input);
                    let car = &player.vehicles[i];
                    let at = car.datum(&cw);
                    fewest_wheels = fewest_wheels.min(car.wheels_in_contact());
                    largest_loss = largest_loss.max(before - car.forward_speed(&cw));
                    let fraction = |j: usize| {
                        let d = lane[j + 1] - lane[j];
                        ((at.x - lane[j].x) * d.x + (at.z - lane[j].z) * d.z)
                            / (d.x * d.x + d.z * d.z)
                    };
                    while panel + 2 < lane.len() && fraction(panel) > 1. {
                        panel += 1;
                    }
                    let top = lane[panel].lerp(lane[panel + 1], fraction(panel)).y;
                    largest_error = largest_error.max((at.y - top).abs());
                    assert!(
                        largest_error < 0.25,
                        "{name} {model:?}, left the deck at {at:?}, drawn {top:.3}"
                    );
                    ticks += 1;
                }
                println!(
                    "drive {name} level {level} structures={structured} {model:?} + {cargo_kg} kg: {ticks} ticks, distance {:.2} m, minimum {fewest_wheels} wheels, max datum error {largest_error:.4} m, max one-step speed loss {largest_loss:.4} m/s",
                    player.position(&cw).distance(start)
                );
                assert!(pilot.done() && fewest_wheels == 4 && largest_loss < 0.5);
            }
        }
    }
}

#[test]
fn a_slow_sedan_meets_the_golden_gate_median_and_the_bay_deck_ends_at_its_edge() {
    let _guard = super::house_tests::world_lock();
    let (mut args, _) = parse(&[]).unwrap();
    args.models = false;
    args.agents = false;
    args.kit = false;
    let world = load_world(&args).unwrap();
    let (mut cw, _) = collision_world(&world, 1, true).unwrap();
    for (name, faces) in drawn_roads(&world) {
        let i = if name == "bay-bridge-west" {
            faces.len() * 3 / 8 * 2
        } else {
            faces.len() / 4 * 2
        };
        let [a, b, c] = faces[i];
        let d = faces[i + 1][2];
        let across = (b - a).normalize();
        let centre = (a + b + c + d) / 4.;
        cw.load_blocking(centre, pworld::LOAD_RADIUS_M);
        if name == "bay-bridge-west" {
            // On the lower deck, neither an oversize traffic ribbon nor the
            // upper deck may catch a tire outside the drawn asphalt.
            for side in [1., -1.] {
                let edge = centre + across * (side * (a.distance(b) / 2. + 0.2));
                assert!(
                    cw.raycast(
                        edge + DVec3::Y * 0.06,
                        DVec3::NEG_Y,
                        0.12,
                        pworld::layer::STREET,
                        None
                    )
                    .is_none()
                );
                let inner = edge - across * (side * 0.4);
                assert!(
                    cw.raycast(
                        inner + DVec3::Y * 0.06,
                        DVec3::NEG_Y,
                        0.12,
                        pworld::layer::STREET,
                        None
                    )
                    .is_some()
                );
            }
            continue;
        }
        let ca = DeckStandards::load(&eo_data::CaStandards::load(&world.paths).unwrap()).unwrap();
        let mut previous_tile = None;
        for panel in faces.chunks_exact(2) {
            let [start_left, start_right, end_right] = panel[0];
            let end_left = panel[1][2];
            for t in [0.02, 0.98] {
                let mid = start_left
                    .midpoint(start_right)
                    .lerp(end_left.midpoint(end_right), t);
                let key = eo_street::tile_key(mid.x, mid.z);
                if previous_tile != Some(key) {
                    cw.load_blocking(mid, 50.);
                    previous_tile = Some(key);
                }
                let from = mid + DVec3::Y * (ca.k_height / 2.) + across * 2.;
                let hit = cw
                    .raycast(from, -across, 3., pworld::layer::WORLD, None)
                    .expect("continuous median on the road centreline");
                let edge = (hit.point - mid).dot(across);
                assert!(
                    (edge - ca.k_width / 2.).abs() < 0.02,
                    "median extends to {edge:.3} m at {mid:?}"
                );
            }
        }
        cw.load_blocking(centre, pworld::LOAD_RADIUS_M);
        let side_hit = cw
            .raycast(
                centre + DVec3::Y * (ca.k_height * 0.5) + across * 2.,
                -across,
                3.,
                pworld::layer::WORLD,
                None,
            )
            .expect("median face");
        assert!(((side_hit.point - centre).dot(across) - ca.k_width / 2.).abs() < 0.04);
        let start = centre + across * 6.;
        let yaw = (-across.x).atan2(-across.z);
        let mut player = Player::new(start, yaw);
        player.spawn_config.model = VehicleModel::Sedan;
        let car = player.spawn_vehicle_below(&mut cw, start + DVec3::Y * 0.3, yaw);
        player.enter_vehicle(car);
        for _ in 0..120 {
            player.fixed_step(
                &mut cw,
                &StepInput {
                    handbrake: true,
                    ..Default::default()
                },
            );
        }
        cw.physics.bodies[player.vehicles[car].body]
            .set_linvel((-across * 3.).as_vec3().to_array().into(), true);
        let mut nearest = f64::INFINITY;
        let mut lift = 0.0_f64;
        for _ in 0..180 {
            player.fixed_step(&mut cw, &StepInput::default());
            let at = player.vehicles[car].datum(&cw);
            nearest = nearest.min((at - centre).dot(across));
            lift = lift.max(at.y - centre.y);
        }
        println!(
            "sedan at 3 m/s toward the Golden Gate median: nearest centre {:.3} m from median centre, maximum datum rise {lift:.3} m",
            nearest
        );
        assert!(nearest > ca.k_width / 2. && nearest < 3. && lift < 0.25);
    }
}

#[test]
fn golden_gate_approach_joints_carry_the_playable_sedan() {
    let _guard = super::house_tests::world_lock();
    let (mut args, _) = parse(&["--structures".into()]).unwrap();
    args.models = false;
    args.agents = false;
    args.kit = false;
    let world = load_world(&args).unwrap();
    let structures = world.structures.as_ref().unwrap();
    // Both survey ways end at an angle to the authored asphalt plates.
    let mut failures = Vec::new();
    for way in ["537838948", "595194543"] {
        let road = structures
            .chains()
            .iter()
            .find(|c| c.landmark.is_none() && c.ways().contains(&way))
            .unwrap();
        let (bridge, end, bridge_end) = road
            .ends
            .iter()
            .enumerate()
            .find_map(|(i, abutment)| {
                structures
                    .chains()
                    .iter()
                    .filter(|c| c.landmark == Some(eo_structures::LandmarkDeck::GoldenGate))
                    .find_map(|c| {
                        c.ends
                            .iter()
                            .position(|e| e.node == abutment.node)
                            .map(|j| (c, i, j))
                    })
            })
            .expect("surveyed bridge connection");
        let point = |c: &eo_structures::Chain, end: usize, inward: f64| {
            let s = if end == 0 {
                inward
            } else {
                c.length() - inward
            };
            let f = c.frame(s);
            DVec3::new(f.x, c.y_at(s), f.z)
        };
        let start = point(road, end, road.length().min(14.) - 0.1);
        let join = point(road, end, 0.);
        let lane: Vec<_> = std::iter::once(start)
            .chain(std::iter::once(join))
            .chain((1..=14).map(|n| point(bridge, bridge_end, f64::from(3 * n))))
            .collect();
        let tangent = join - start;
        let yaw = tangent.x.atan2(tangent.z);
        for model in [VehicleModel::Sedan, VehicleModel::CatalogueSedan] {
            let (mut cw, _) = collision_world(&world, 1, args.landmark_access).unwrap();
            cw.load_blocking(start, pworld::LOAD_RADIUS_M);
            let mut player = Player::new(start, yaw);
            player.spawn_config.model = model;
            let car = player.spawn_vehicle_below(&mut cw, start + DVec3::Y * 0.3, yaw);
            player.enter_vehicle(car);
            for _ in 0..120 {
                player.fixed_step(
                    &mut cw,
                    &StepInput {
                        handbrake: true,
                        ..Default::default()
                    },
                );
            }
            let mut pilot = DrivePilot::new(lane.iter().map(|p| [p.x, p.z]).collect(), 8.);
            let mut impact = None;
            let mut min_wheels = 4;
            let mut max_rise = 0.0_f64;
            let mut max_loss = 0.0_f64;
            for _ in 0..1500 {
                if pilot.done() {
                    break;
                }
                cw.ensure_step_tiles(player.position(&cw));
                let before = player.vehicles[car].forward_speed(&cw);
                let input = pilot.input(&player, &cw);
                player.fixed_step(&mut cw, &input);
                let car = &player.vehicles[car];
                let p = car.datum(&cw);
                let (s, _) = bridge.project(p.x, p.z);
                if p.distance(join) < 9. {
                    max_rise = max_rise.max((p.y - bridge.y_at(s)).abs());
                }
                min_wheels = min_wheels.min(car.wheels_in_contact());
                let loss = before - car.forward_speed(&cw);
                if loss > max_loss {
                    max_loss = loss;
                    impact = Some((p, before, car.forward_speed(&cw)));
                }
            }
            println!(
                "Golden Gate approach {way}, {model:?}: crossed {}, {} wheels, largest height error at the joint {max_rise:.3} m, largest one-step speed loss {max_loss:.3} m/s",
                pilot.done(),
                min_wheels
            );
            if !pilot.done() || min_wheels < 4 || max_rise >= 0.35 || max_loss >= 0.5 {
                failures.push((way, model, player.position(&cw), impact));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "approach route, final position and largest loss: {failures:?}"
    );
}


#[test]
fn presidio_portal_is_clear_at_both_terrain_detail_levels() {
    let _guard = super::house_tests::world_lock();
    let (mut args, _) = parse(&["--quality".into(), "high".into(), "--structures".into(), "--tunnels".into()]).unwrap();
    args.kit = false;
    args.models = false;
    let world = load_world(&args).unwrap();
    let roads = world.structures.as_ref().unwrap();
    let scene = &world.scene;
    // Two points along the lane as seen looking out of the Presidio bore,
    // plus solid ground just north of the portal, in the city's coordinates.
    for (x, z, expected) in [(-3850.78, -3449.99, Some(true)), (-3838.26, -3449.81, Some(false)), (-3838.26, -3420.0, None)] {
        let cuts = roads.terrain_cuts_in([x, z], [x, z]);
        let cut = cuts.iter().find(|c| c.contains(x, z));
        assert_eq!(cut.map(|c| c.canopy), expected);
        for coarse in [false, true] {
            let mut heights = Vec::new();
            for obj in &scene.objects {
                if obj.kind & 255 != eo_render::scene::ObjectKind::Terrain as u32 || obj.copy != 0
                    || x < f64::from(obj.min[0]) || x > f64::from(obj.max[0])
                    || z < f64::from(obj.min[2]) || z > f64::from(obj.max[2]) { continue; }
                let (start, count) = if coarse && obj.far_index_count > 0 {
                    (obj.far_first_index, obj.far_index_count)
                } else {
                    (obj.first_index, obj.index_count)
                };
                for tri in scene.indices[start as usize..(start + count) as usize].chunks_exact(3) {
                    let [a, b, c] = [0, 1, 2].map(|k| {
                        let i = (i64::from(obj.base_vertex) + i64::from(tri[k])) as usize;
                        DVec3::from_array(scene.vertices[i].pos.map(f64::from))
                    });
                    let (ab, ac, ap) = (b - a, c - a, DVec3::new(x, a.y, z) - a);
                    let area = ab.x * ac.z - ab.z * ac.x;
                    if area.abs() < 1e-9 { continue; }
                    let u = (ap.x * ac.z - ap.z * ac.x) / area;
                    let v = (ab.x * ap.z - ab.z * ap.x) / area;
                    if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                        heights.push(a.y + u * ab.y + v * ac.y);
                    }
                }
            }
            match cut {
                Some(c) if c.canopy => assert!(heights.is_empty(), "terrain covers the portal at {x}, {z}, coarse={coarse}: {heights:?}"),
                Some(c) => assert!(!heights.is_empty() && heights.iter().all(|h| *h <= c.floor + 0.002),
                    "ground crosses the approach at {x}, {z}, coarse={coarse}, floor {}: {heights:?}", c.floor),
                None => assert!(!heights.is_empty(), "missing hillside beside the road, coarse={coarse}"),
            }
        }
    }
}

#[test]
fn island_upper_tunnel_entrance_has_an_open_driver_sightline() {
    let _guard = super::house_tests::world_lock();
    let (mut args, _) = parse(&[
        "--quality".into(),
        "high".into(),
        "--structures".into(),
        "--tunnels".into(),
    ])
    .unwrap();
    args.kit = false;
    args.models = false;
    let world = load_world(&args).unwrap();
    let roads = world.structures.as_ref().unwrap();
    let upper = roads
        .chains()
        .iter()
        .find(|c| c.ways().contains(&"11415208"))
        .unwrap();
    let road = upper.frame(4.);
    let eye = DVec3::new(road.x, upper.y_at(4.) + 1.45, road.z);
    let direction = DVec3::new(road.tx, 0., road.tz);
    let end = eye + direction * 24.;
    let low = eye.min(end);
    let high = eye.max(end);
    let distance = |a: DVec3, b: DVec3, c: DVec3| -> Option<f64> {
        let ab = b - a;
        let ac = c - a;
        let h = direction.cross(ac);
        let determinant = ab.dot(h);
        if determinant.abs() < 1e-8 {
            return None;
        }
        let from = eye - a;
        let u = from.dot(h) / determinant;
        let q = from.cross(ab);
        let v = direction.dot(q) / determinant;
        let t = ac.dot(q) / determinant;
        (u >= 0. && v >= 0. && u + v <= 1. && t > 0. && t < 24.).then_some(t)
    };
    let mut obstacles = Vec::new();
    let scene = &world.scene;
    for obj in &scene.objects {
        let min = DVec3::from_array(obj.min.map(f64::from));
        let max = DVec3::from_array(obj.max.map(f64::from));
        if obj.copy != 0 || !min.cmple(high).all() || !max.cmpge(low).all() {
            continue;
        }
        for tri in scene.indices
            [obj.first_index as usize..(obj.first_index + obj.index_count) as usize]
            .chunks_exact(3)
        {
            let [a, b, c] = [0, 1, 2].map(|j| {
                let index = i64::from(obj.base_vertex) + i64::from(tri[j]);
                DVec3::from_array(scene.vertices[index as usize].pos.map(f64::from))
            });
            if let Some(t) = distance(a, b, c) {
                obstacles.push(format!(
                    "scene kind {}, role {}, {:.2} m",
                    obj.object_kind(),
                    obj.lod_role,
                    t
                ));
            }
        }
    }
    use eo_landmarks::rocks::scan;
    if let (Some(placed), Some(library)) = (scan::placed(), scan::library()) {
        let spheres: Vec<_> = library.kinds.iter().map(|k| k.sphere()).collect();
        for piece in
            eo_render::groundcover::drawn_rocks(&library, &placed, Some(roads.road_openings()))
        {
            let k = piece.kind as usize;
            let position = DVec3::from_array(piece.pos);
            let to_world = |point: [f32; 3]| {
                position
                    + DVec3::from_array(scan::quat_rotate(
                        piece.rot,
                        [0, 1, 2].map(|i| f64::from(point[i]) * piece.scale[i]),
                    ))
            };
            let center = to_world(spheres[k].0);
            let radius =
                DVec3::from_array(piece.scale).abs().max_element() * f64::from(spheres[k].1);
            if center.distance(center.clamp(low, high)) > radius {
                continue;
            }
            let mesh = &library.kinds[k].mesh;
            for tri in mesh.lods[0].chunks_exact(3) {
                let [a, b, c] = [0, 1, 2].map(|j| to_world(mesh.positions[tri[j] as usize]));
                if let Some(t) = distance(a, b, c) {
                    obstacles.push(format!(
                        "{:?} {} at {:?}, {:.2} m",
                        piece.source, library.kinds[k].name, piece.pos, t
                    ));
                }
            }
        }
    }
    assert!(
        obstacles.is_empty(),
        "occluded upper roadway at {eye:?}: {obstacles:?}"
    );
}
