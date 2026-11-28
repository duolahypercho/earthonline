use std::sync::Arc;

use eo_player::routes::DrivePilot;
use eo_player::street_ground::StreetGround;
use eo_player::world::{BuildingSet, CollisionWorld};
use eo_player::world;
use eo_player::{Mode, Player, StepInput};
use eo_structures::StructureOptions;
use glam::DVec3;

#[test]
fn drive_both_east_span_carriageways_between_the_mapped_shores() {
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
    for (way, eastbound) in [("236348361", false), ("237731428", true)] {
    let mut world=CollisionWorld::new(streets.clone(), buildings.clone(),4);
    let mut legs: Vec<_> = decks.chains().iter().filter(|c| c.is_built() && c.ways().contains(&way)).collect();
    legs.sort_by(|a,b| a.points[0][0].total_cmp(&b.points[0][0]));
    let length=legs.iter().map(|c|c.length()).sum::<f64>();
    let station=|mut d:f64| {
        for &ch in &legs {
            if d <= ch.length() {return (ch,d);}
            d-=ch.length();
        }
        let ch=*legs.last().unwrap();(ch,ch.length())
    };
    let path:Vec<_>=(12..length as usize-3).step_by(5).map(|d|{
        let (c,s)=station(d as f64);let f=c.frame(s);[f.x,f.z]
    }).collect();
    let f=legs[0].frame(12.);let y=legs[0].y_at(12.);
    world.load_blocking(DVec3::new(f.x,y,f.z),500.);
    let mut player=Player::new(DVec3::new(f.x,y,f.z),0.);
    player.spawn_vehicle_below(&mut world,DVec3::new(f.x,y+1.,f.z),f.tx.atan2(f.tz));
    let d=player.vehicles[0].datum(&world);
    player.character.teleport_feet(d+DVec3::new(f.nx*2.,0.,f.nz*2.));
    for _ in 0..60{player.fixed_step(&mut world,&StepInput::default());}
    player.fixed_step(&mut world,&StepInput{vehicle_toggle:true,..Default::default()});
    assert!(matches!(player.mode,Mode::Driving(0)));
    let mut pilot=DrivePilot::new(path,14.);
    let mut achieved=0.;let mut worst=0f64;let mut last_loaded=0.;
    for _ in 0..60000 {
        let input=pilot.input(&player,&world);player.fixed_step(&mut world,&input);
        let p=player.vehicles[0].datum(&world); world.pump(p);
        let (i,(s,_))=legs.iter().map(|c|c.project(p.x,p.z)).enumerate().min_by(|a,b| a.1.1.total_cmp(&b.1.1)).unwrap();
        achieved=legs[..i].iter().map(|c|c.length()).sum::<f64>()+s;
        if achieved-last_loaded>90. {world.load_blocking(p,world::LOAD_RADIUS_M);last_loaded=achieved;}
        let err=(p.y-legs[i].y_at(s)).abs();worst=worst.max(err);
        assert!(err<0.3,"{way} lost its deck at {achieved:.1} m, offset {err:.3} m");
        if pilot.done(){break}
    }
    println!("{} way {}: car completed {:.1} of {:.1} game m, max deck error {:.3} m", if eastbound{"eastbound"}else{"westbound"},way,achieved-12.,length,worst);
    assert!(pilot.done() && length-achieved<30.);
    }
}
