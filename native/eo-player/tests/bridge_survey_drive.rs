use std::sync::Arc;

use eo_player::routes::DrivePilot;
use eo_player::street_ground::StreetGround;
use eo_player::world::{BuildingSet, CollisionWorld};
use eo_player::{Mode, Player, StepInput};
use eo_structures::StructureOptions;
use glam::DVec3;

#[test]
fn drive_out_of_the_yerba_buena_transition_onto_the_east_span() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let plain = StreetGround::build(&city, grid).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let joined = eo_structures::build_joined(&plain.world, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions { tunnels: true }).unwrap();
    let decks = Arc::new(joined.structures);
    let streets = Arc::new(StreetGround::new(Arc::new(joined.streets.unwrap()), Some(decks.clone())));
    let lift: serde_json::Value = serde_json::from_slice(&std::fs::read("../eo-assets/data/authored-placement.json").unwrap()).unwrap();
    let buildings = Arc::new(BuildingSet::from_city(&city, lift["constructionLiftM"].as_f64().unwrap()));
    let mut world = CollisionWorld::new(streets, buildings, 4);
    let mut legs: Vec<_> = decks.chains().iter().filter(|c| c.is_built() && c.ways().contains(&"237731428")).collect();
    legs.sort_by(|a,b| a.points[0][0].total_cmp(&b.points[0][0]));
    let [approach, main] = legs.as_slice() else {panic!("island transition and ocean span")};
    let seam = approach.frame(approach.length());
    assert_eq!(approach.ends[1].node, main.ends[0].node);
    let start = approach.length()-150.;
    // The pilot brakes ahead of its last waypoint. Place it beyond the measured 140 m.
    let path = (0..=65).map(|i| {
        let d = i as f64*5.;
        let f = if d<=150. { approach.frame(start+d) } else { main.frame(d-150.) };
        [f.x, f.z]
    }).collect();
    let f=approach.frame(start);
    let y=approach.y_at(start);
    world.load_blocking(DVec3::new(f.x,y,f.z),500.);
    let mut player=Player::new(DVec3::new(f.x,y,f.z),0.);
    player.spawn_vehicle_below(&mut world,DVec3::new(f.x,y+1.,f.z),f.tx.atan2(f.tz));
    let d=player.vehicles[0].datum(&world);
    player.character.teleport_feet(d+DVec3::new(f.nx*2.,0.,f.nz*2.));
    for _ in 0..60{player.fixed_step(&mut world,&StepInput::default());}
    player.fixed_step(&mut world,&StepInput{vehicle_toggle:true,..Default::default()});
    assert!(matches!(player.mode,Mode::Driving(0)));
    let mut pilot=DrivePilot::new(path,14.);
    let mut crossed=0.;
    let mut worst=0f64;
    for _ in 0..3600 {
        let input=pilot.input(&player,&world);
        player.fixed_step(&mut world,&input);
        let c=player.vehicles[0].datum(&world);world.pump(c);
        let (s, da)=approach.project(c.x,c.z);
        let (t, db)=main.project(c.x,c.z);
        let expected=if da<db {approach.y_at(s)} else {crossed=t;main.y_at(t)};
        worst=worst.max((c.y-expected).abs());
        assert!((c.y-expected).abs()<0.3,"lost the deck at {c:?}, expected {expected}");
        if pilot.done(){break}
    }
    println!("Crossed the join at ({:.1}, {:.1}), drove {:.1} m on the ocean span, max deck error {:.3} m",seam.x,seam.z,crossed,worst);
    assert!(pilot.done()&&crossed>140.);
}
