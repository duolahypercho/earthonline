use std::sync::Arc;

use eo_player::routes::DrivePilot;
use eo_player::street_ground::StreetGround;
use eo_player::world::{BuildingSet, CollisionWorld};
use eo_player::world;
use eo_player::{Mode, Player, StepInput};
use eo_structures::StructureOptions;
use glam::DVec3;

#[test]
fn drive_the_restored_city_bridges_from_their_streets() {
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

    let mut failures=vec![];
    for way in ["27239369","27145834","59763602","27322342"] {
        let c=decks.chains().iter().find(|c|c.is_built() && c.ways().contains(&way)).unwrap();
        let mut path=vec![];
        let mut leads=[0.;2];
        for end in 0..2 {
            let forward=if end==0 {-1.} else {1.};
            let station=if end==0 {0.} else {c.length()};
            let f=c.frame(station);
            let plan=&streets.world.plan;
            let mut node=[f.x,f.z];
            for n in &plan.nodes {
                for a in &n.approaches {
                    let leg=&plan.entries[a.entry];
                    if leg.structure_leg.is_some(){
                        let m=eo_street::plan::approach_end_frame(&plan.entries,a);
                        if (m.x-f.x).hypot(m.z-f.z)<0.1 {node=n.position;}
                    }
                }
            }
            let mut entries=vec![];
            for e in plan.entries.iter().filter(|e|e.structure_leg.is_none()){
                for (i,seg) in e.points.windows(2).enumerate(){
                    let (dx,dz)=(seg[1][0]-seg[0][0],seg[1][1]-seg[0][1]);let len=dx.hypot(dz);
                    if len<=0.{continue;}
                    let t=(((node[0]-seg[0][0])*dx+(node[1]-seg[0][1])*dz)/(len*len)).clamp(0.,1.);
                    let d=(seg[0][0]+t*dx-node[0]).hypot(seg[0][1]+t*dz-node[1]);
                    if d>streets.world.o.node_snap {continue;}
                    let s=e.cum[i]+t*len;
                    for dir in [1.,-1.]{
                        let length=if dir>0.{e.length-s}else{s};
                        if length<10.{continue;}
                        let align=dir*forward*(dx*f.tx+dz*f.tz)/len;
                        entries.push((align,e,s,dir,length));
                    }
                }
            }
            let Some(&(_,e,s,dir,available))=entries.iter().max_by(|a,b|a.0.total_cmp(&b.0)) else {
                println!("LEAD {way} end {end} at {:?} node {:?}",(f.x,f.z),node);
                failures.push(way);break;
            };
            let length=available.min(40.).floor() as usize;leads[end]=length as f64;
            let mut lead:Vec<_>=(0..=length).step_by(2).map(|d|{let f=e.frame(s+dir*d as f64,false);[f.x,f.z]}).collect();
            if end==0 {
                lead.reverse();path.extend(lead);
                for s in (2..c.length() as usize).step_by(2){let f=c.frame(s as f64);path.push([f.x,f.z]);}
            }else{path.extend(lead);}
        }
        if leads.contains(&0.) {continue;}
        let [x,z]=path[0];let h=streets.world.paved_at(x,z).expect("lead-in paving");let heading=(path[1][0]-x).atan2(path[1][1]-z);
        let mut world=CollisionWorld::new(streets.clone(),buildings.clone(),4);
        world.load_blocking(DVec3::new(x,h,z),world::LOAD_RADIUS_M);
        let mut player=Player::new(DVec3::new(x,h,z),heading);
        player.spawn_vehicle_below(&mut world,DVec3::new(x,h+0.5,z),heading);
        let d=player.vehicles[0].datum(&world);
        player.character.teleport_feet(d+DVec3::new(heading.cos()*2.,0.,-heading.sin()*2.));
        for _ in 0..60 {player.fixed_step(&mut world,&StepInput::default());}
        player.fixed_step(&mut world,&StepInput{vehicle_toggle:true,..Default::default()});
        assert!(matches!(player.mode,Mode::Driving(0)));
        let dest=*path.last().unwrap();let mut pilot=DrivePilot::new(path,8.);
        let mut worst=0f64;let mut achieved=0f64;let mut last=d;let mut logged=false;
        for _ in 0..16000 {
            let input=pilot.input(&player,&world);player.fixed_step(&mut world,&input);
            let q=player.vehicles[0].datum(&world);world.pump(q);
            let(s,lat)=c.project(q.x,q.z);
            if s>0.5 && s<c.length()-0.5 && lat<c.half_at(s) {
                let error=(q.y-c.y_at(s)).abs();
                if error>0.5 && !logged{println!("OFF {way} at {s:.1} {lat:.2} driven {:.2} expected {:.2}",q.y,c.y_at(s));logged=true;}
                worst=worst.max(error);achieved=achieved.max(s);
            }
            if (q-last).length()>75.{world.load_blocking(q,world::LOAD_RADIUS_M);last=q;}
            if pilot.done(){break;}
        }
        let q=player.vehicles[0].datum(&world);let away=(q.x-dest[0]).hypot(q.z-dest[1]);
        println!("DRIVE {} ({way}): lead in {:.0}, deck {:.1}, lead out {:.0} m; reached {achieved:.1}; final waypoint distance {away:.1}, max deck error {worst:.3}, finished {}",c.name(),leads[0],c.length(),leads[1],pilot.done());
        if !(pilot.done() && worst<0.5 && achieved>c.length()-5.){failures.push(way);}
    }
    assert!(failures.is_empty(), "drives requiring inspection {failures:?}");
}
