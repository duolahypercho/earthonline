//! Offline traffic diagnostic: tiny synthetic maps, the production clock and
//! lane builder, and a self-contained replay report. No city or graphics assets.
use anyhow::{Context, Result, ensure};
use eo_data::sim_source::{Oneway, SimRoad, SimSource};
use eo_sim::geom::{Movement, P2, at_arc};
use eo_sim::native::NativeSource;
use eo_sim::network::{ControlKind, NONE, TileKey};
use eo_sim::{
    Focus, Network, People, Region, RegionConfig, SignalTiming, Standards, StreetDims, World,
};
use serde_json::{Value, json};

const SAMPLE_STEPS: usize = 12;
const SECONDS: usize = 90;

struct Scenario {
    name: &'static str,
    description: &'static str,
    roads: Vec<SimRoad>,
    signals: Vec<P2>,
    cars: usize,
    people: usize,
    probe: Option<PedestrianProbe>,
    turn: Option<TurnProbe>,
}

#[derive(Clone, Copy)]
struct TurnProbe {
    movement: Movement,
    opposing: bool,
    cyclist: bool,
}

#[derive(Clone, Copy)]
enum PedestrianProbe {
    AlreadyAhead,
    UnexpectedObservation,
}

impl PedestrianProbe {
    // Scripted observation windows exercise the game's safety response, not
    // a real-world collision model. Both leave room for yielding and clearing.
    fn present(self, step: usize) -> bool {
        let observed = match self {
            Self::AlreadyAhead => true,
            Self::UnexpectedObservation => step >= 60,
        };
        observed && step < 12 * 60
    }
}

// Coordinates are synthetic fixture extents, not prescribed road dimensions.
// Lane widths, signal timing, walking lines and bars are built by NativeSource.
fn road(id: &str, highway: &str, lanes: u32, points: &[P2]) -> SimRoad {
    SimRoad {
        id: id.into(),
        name: id.into(),
        highway: highway.into(),
        oneway: Oneway::Both,
        lanes: Some(lanes),
        maxspeed: String::new(),
        sidewalk: "both".into(),
        bridge: false,
        tunnel: false,
        layer: 0,
        points: points.to_vec(),
    }
}

fn scenarios() -> Vec<Scenario> {
    let east_west = road("Main", "secondary", 2, &[[-90., 0.], [0., 0.], [90., 0.]]);
    let north_south = road(
        "Cross",
        "residential",
        2,
        &[[0., -90.], [0., 0.], [0., 90.]],
    );
    vec![
        Scenario {
            name: "01 / Following",
            description: "Two-way corridor: following gaps, braking, acceleration and terminal recycling.",
            roads: vec![road("Main", "secondary", 2, &[[-90., 0.], [90., 0.]])],
            signals: vec![],
            cars: 12,
            people: 0,
            probe: None,
            turn: None,
        },
        Scenario {
            name: "02 / Signal junction",
            description: "Four approaches, actual phase plan, turns, queue discharge and pedestrian yielding.",
            roads: vec![east_west.clone(), north_south],
            signals: vec![[0., 0.]],
            cars: 14,
            people: 12,
            probe: None,
            turn: None,
        },
        Scenario {
            name: "03 / Uncontrolled T",
            description: "No signal or stop-sign control: crossing gaps, movement claims and conflicting turns. Not a stop-sign test.",
            roads: vec![
                east_west,
                road("Side", "residential", 2, &[[0., 0.], [0., 85.]]),
            ],
            signals: vec![],
            cars: 10,
            people: 8,
            probe: None,
            turn: None,
        },
        Scenario {
            name: "04 / Short-block queues",
            description: "Two signal junctions on a short block: downstream occupancy, spillback and gridlock relief.",
            roads: vec![
                road(
                    "Main",
                    "secondary",
                    4,
                    &[[-115., 0.], [-35., 0.], [35., 0.], [115., 0.]],
                ),
                road(
                    "West",
                    "residential",
                    2,
                    &[[-35., -80.], [-35., 0.], [-35., 80.]],
                ),
                road(
                    "East",
                    "residential",
                    2,
                    &[[35., -80.], [35., 0.], [35., 80.]],
                ),
            ],
            signals: vec![[-35., 0.], [35., 0.]],
            cars: 30,
            people: 12,
            probe: None,
            turn: None,
        },
        Scenario {
            name: "05 / Person ahead",
            description: "Controlled safety probe: a person occupies the route, the driver should yield, wait, then resume after the person clears.",
            roads: vec![road("Main", "secondary", 2, &[[-90., 0.], [90., 0.]])],
            signals: vec![],
            cars: 1,
            people: 0,
            probe: Some(PedestrianProbe::AlreadyAhead),
            turn: None,
        },
        Scenario {
            name: "06 / Unexpected person",
            description: "Controlled safety probe: a new pedestrian observation appears ahead during driving; inspect the actual engine's response, hold and release. Not a physical jump animation.",
            roads: vec![road("Main", "secondary", 2, &[[-90., 0.], [90., 0.]])],
            signals: vec![],
            cars: 1,
            people: 0,
            probe: Some(PedestrianProbe::UnexpectedObservation),
            turn: None,
        },
        Scenario {
            name: "07 / Left turn",
            description: "Controlled intended left turn with opposing through traffic. The production fleet owns junction admission, speed and motion.",
            roads: vec![
                road("Main", "secondary", 2, &[[-90., 0.], [0., 0.], [90., 0.]]),
                road(
                    "Cross",
                    "residential",
                    2,
                    &[[0., -90.], [0., 0.], [0., 90.]],
                ),
            ],
            signals: vec![],
            cars: 2,
            people: 0,
            probe: None,
            turn: Some(TurnProbe {
                movement: Movement::Left,
                opposing: true,
                cyclist: false,
            }),
        },
        Scenario {
            name: "08 / Right turn",
            description: "Controlled intended right turn; inspect actual approach, corner path, indicator and departure lane.",
            roads: vec![
                road("Main", "secondary", 2, &[[-90., 0.], [0., 0.], [90., 0.]]),
                road(
                    "Cross",
                    "residential",
                    2,
                    &[[0., -90.], [0., 0.], [0., 90.]],
                ),
            ],
            signals: vec![],
            cars: 1,
            people: 0,
            probe: None,
            turn: Some(TurnProbe {
                movement: Movement::Right,
                opposing: false,
                cyclist: false,
            }),
        },
        Scenario {
            name: "09 / Right turn + cyclist",
            description: "Right turn with a scripted cyclist observation on the exit. Uses the pedestrian sensing channel as a proxy; bicycle dynamics and dedicated bike-lane priority are not implemented.",
            roads: vec![
                road("Main", "secondary", 2, &[[-90., 0.], [0., 0.], [90., 0.]]),
                road(
                    "Cross",
                    "residential",
                    2,
                    &[[0., -90.], [0., 0.], [0., 90.]],
                ),
            ],
            signals: vec![],
            cars: 1,
            people: 0,
            probe: None,
            turn: Some(TurnProbe {
                movement: Movement::Right,
                opposing: false,
                cyclist: true,
            }),
        },
    ]
}

fn record(s: Scenario, dims: &StreetDims, timing: &SignalTiming) -> Result<Value> {
    let source = NativeSource::new(
        SimSource {
            roads: s.roads,
            signals: s.signals,
        },
        dims.clone(),
        timing.clone(),
    );
    let tiles = [
        TileKey { x: -1, z: -1 },
        TileKey { x: -1, z: 0 },
        TileKey { x: 0, z: -1 },
        TileKey { x: 0, z: 0 },
    ];
    let build = source.build(&tiles);
    let roads: Vec<_> = build
        .segments
        .iter()
        .map(|s| json!({"pts":s.points,"width":2.0*s.half}))
        .collect();
    let mut net = Network::new(timing.clone());
    net.add(build);
    ensure!(!net.lanes.is_empty(), "empty fixture");
    let lanes: Vec<_> = net.alive_lanes().map(|id| {
        let l = &net.lanes[id as usize];
        let (x, z, dx, dz, _) = at_arc(&l.pts, &l.cum, l.length * 0.5);
        let bar = match l.control {
            ControlKind::Signal { junction, axis, stop_offset_m, bar_depth_m } => {
                let (x,z,dx,dz,_) = at_arc(&l.pts,&l.cum,l.length-stop_offset_m);
                json!({"x":x,"z":z,"dx":dx,"dz":dz,"width":l.width,"depth":bar_depth_m,"junction":junction,"axis":axis})
            }
            _ => Value::Null,
        };
        let links: Vec<_> = l.links.iter().enumerate().map(|(i, link)| {
            let path = link.plan.as_ref().map(|p| (0..=16).map(|n| {
                let (x,z,_) = p.pose_at(p.arc_len * n as f64 / 16.0); [x,z]
            }).collect::<Vec<_>>());
            json!({"index":i,"to":link.to,"movement":format!("{:?}",link.movement),"legal":link.legal,"drivable":link.drivable,"path":path})
        }).collect();
        json!({"id":id,"key":l.key,"pts":l.pts,"width":l.width,"speed":l.max_speed,"arrow":[x,z,dx,dz],"bar":bar,"links":links})
    }).collect();
    let footways: Vec<_> = net
        .alive_footways()
        .map(|id| net.footways[id as usize].pts.clone())
        .collect();
    let junctions: Vec<_> = net.junctions.iter().map(|j| json!({"centre":j.centre,"cycle":j.cycle_s,"axes":j.axes.iter().map(|a| json!({"green":a.green_s,"yellow":a.yellow_s,"allRed":a.all_red_s})).collect::<Vec<_>>()})).collect();

    let mut world = World::new(net);
    let mut cfg = RegionConfig::js_pool(1, s.people);
    cfg.vehicle_pool = s.cars;
    cfg.people = if s.people == 0 {
        People::None
    } else {
        People::Pool(s.people)
    };
    world.regions.push(Region::new(
        &world.net,
        0,
        cfg,
        Focus {
            x: 0.0,
            z: 0.0,
            yaw: 0.0,
        },
    ));
    ensure!(!world.regions[0].fleet.is_empty(), "no vehicles spawned");
    // The probe is an externally supplied pedestrian observation. A valid
    // walker placeholder lets World::step use its usual sensing, driving,
    // clock and output; the fixture owns the observation's location. It is
    // not an autonomous crowd route or a collision setup.
    let mut turn_target = None;
    let mut opposing_target = None;
    let mut cyclist = false;
    let probe = if let Some(turn) = s.turn {
        // Pin only the driver's intended legal movement. All movement, gap
        // acceptance, claims and collision diagnostics remain production code.
        let net = &world.net;
        let route = |from_west: bool, movement| -> Result<(u32, usize)> {
            net.alive_lanes()
                .find_map(|id| {
                    let lane = &net.lanes[id as usize];
                    let p = lane.pts[0];
                    let approach = if from_west { p[0] < -40.0 } else { p[0] > 40.0 };
                    if !approach {
                        return None;
                    }
                    lane.links
                        .iter()
                        .position(|l| {
                            l.movement == movement && l.legal && l.drivable && l.plan.is_some()
                        })
                        .map(|k| (id, k))
                })
                .context("legal turn fixture route")
        };
        let (from, k) = route(true, turn.movement)?;
        let lane = &net.lanes[from as usize];
        let dest = lane.links[k].to;
        let r = &mut world.regions[0];
        r.fleet
            .place(net, 0, from, lane.length * 0.5, lane.max_speed * 0.5);
        r.fleet.cars[0].next = k as u16;
        r.fleet.cars[0].turn_side = lane.links[k].plan.as_ref().unwrap().turn_side;
        turn_target = Some((r.fleet.cars[0].id, dest));
        if turn.opposing {
            let (other, k) = route(false, Movement::Through)?;
            let lane = &net.lanes[other as usize];
            r.fleet
                .place(net, 1, other, lane.length * 0.5, lane.max_speed * 0.5);
            r.fleet.cars[1].next = k as u16;
            r.fleet.cars[1].turn_side = lane.links[k].plan.as_ref().unwrap().turn_side;
            opposing_target = Some((r.fleet.cars[1].id, lane.links[k].to));
        }
        if turn.cyclist {
            cyclist = true;
            let exit = &net.lanes[dest as usize];
            let (x, z, _, _, _) = at_arc(&exit.pts, &exit.cum, exit.length * 0.25);
            let footway = net
                .alive_footways()
                .next()
                .context("cyclist observation placeholder")?;
            Some((PedestrianProbe::AlreadyAhead, [x, z], footway))
        } else {
            None
        }
    } else if let Some(mode) = s.probe {
        let lane_id = world.net.alive_lanes().next().context("probe lane")?;
        let lane = &world.net.lanes[lane_id as usize];
        let (x, z, _, _, _) = at_arc(&lane.pts, &lane.cum, lane.length * 0.5);
        let footway = world.net.alive_footways().next().context("probe footway")?;
        let r = &mut world.regions[0];
        r.fleet.place(
            &world.net,
            0,
            lane_id,
            lane.length / 3.0,
            lane.max_speed * 0.5,
        );
        Some((mode, [x, z], footway))
    } else {
        None
    };
    let (seconds, sample_steps) = if s.turn.is_some() {
        (18, 1)
    } else if probe.is_some() {
        (18, 1)
    } else {
        (SECONDS, SAMPLE_STEPS)
    };
    let mut frames = Vec::new();
    let mut previous_speed = world.regions[0].fleet.cars[0].speed;
    let mut safety_clamps = 0u64;
    let mut turn_completed = false;
    let mut opposing_completed = false;
    let mut left_before_opposing = false;
    for step in 1..=seconds * 60 {
        if let Some((mode, pos, footway)) = probe {
            let c = &mut world.regions[0].crowd;
            if mode.present(step) {
                if c.walkers.is_empty() {
                    let mut w = eo_sim::pedestrians::Walker::new(
                        0, footway, 0.0, 1, 0.0, 0.0, 1, false, 0.0,
                    );
                    w.activity = eo_sim::consts::Activity::Stand;
                    w.activity_timer = seconds as f64;
                    c.adopt(w, pos[0], pos[1]);
                } else {
                    c.x[0] = pos[0];
                    c.z[0] = pos[1];
                }
            } else if !c.walkers.is_empty() {
                c.remove(0);
            }
        }
        world.step();
        // Keep the scripted observation at the same location for reporting;
        // the normal crowd routing step otherwise reprojects the placeholder.
        if let Some((mode, pos, _)) = probe {
            if mode.present(step) {
                let c = &mut world.regions[0].crowd;
                c.x[0] = pos[0];
                c.z[0] = pos[1];
            }
            let speed = world.regions[0].fleet.cars[0].speed;
            if previous_speed - speed > eo_sim::consts::DECEL * eo_sim::consts::FIXED_STEP_S + 1e-6
            {
                safety_clamps += 1;
            }
            previous_speed = speed;
        }
        if step % sample_steps != 0 {
            continue;
        }
        let r = &world.regions[0];
        let f = &r.fleet;
        opposing_completed |= opposing_target.is_some_and(|(id, dest)| {
            f.cars
                .iter()
                .any(|c| c.id == id && c.lane == dest && !c.cornering())
        });
        turn_completed |= turn_target.is_some_and(|(id, dest)| {
            f.cars
                .iter()
                .any(|c| c.id == id && c.lane == dest && !c.cornering())
        });
        // In the deliberately conflicting left-turn fixture the intended
        // order is opposing-through first. Record the actual order; do not
        // change production priority or disguise an unsuccessful scenario.
        left_before_opposing |= opposing_target.is_some() && turn_completed && !opposing_completed;
        let cars: Vec<_> = f.cars.iter().enumerate().map(|(i,c)| {
            let (person, crossing) = f.pedestrian_observation(i);
            json!({
            "id":c.id,"kind":c.kind,"x":f.x[i],"z":f.z[i],"yaw":f.yaw[i],"length":c.len,"width":c.wid,
            "speed":c.speed,"lane":c.lane,"next": (c.next != eo_sim::vehicles::NO_LINK).then_some(c.next),
            "braking":c.braking,"turn":c.turn_side,"corner":c.cornering(),"claim":(c.claim_node != NONE).then_some(c.claim_node),
            "wait":c.junction_wait,"arc":c.arc,"stopHold":c.stop_hold,"stopReleased":c.stop_released,
            "personObserved":person,"crossingObserved":crossing,
            "movement": if c.cornering() { world.net.lanes[c.corner_from as usize].links.get(c.corner_link as usize).map(|l|format!("{:?}",l.movement)) } else { world.net.lanes[c.lane as usize].links.get(c.next as usize).map(|l|format!("{:?}",l.movement)) },
            "cornerFrom": (c.corner_from != NONE).then_some(c.corner_from), "cornerLink": (c.corner_link != eo_sim::vehicles::NO_LINK).then_some(c.corner_link)
        })}).collect();
        let walkers: Vec<_> = r.crowd.walkers.iter().enumerate().map(|(i,w)| json!({
            "x":r.crowd.x[i],"z":r.crowd.z[i],"crossing":w.leg.is_some_and(|l|l.kind==eo_sim::pedestrians::LegKind::Cross),"waiting":w.kerb.is_some(),"cyclistProxy":cyclist
        })).collect();
        let aspects: Vec<_> = world
            .net
            .junctions
            .iter()
            .map(|j| {
                (0..j.axes.len())
                    .map(|i| j.aspect_at(i, world.time))
                    .collect::<Vec<_>>()
            })
            .collect();
        let stats = eo_sim::stats::sample(&world.net, r, world.time);
        // Identify the vehicles involved in the sampled person/body diagnostic,
        // using the same geometry and predicate as stats::sample.
        let person_conflicts: Vec<_> = f
            .cars
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                r.crowd.x.iter().zip(&r.crowd.z).any(|(&x, &z)| {
                    eo_sim::geom::hypot(f.x[*i] - x, f.z[*i] - z) <= 8.0
                        && eo_sim::geom::obb_clearance(
                            x,
                            z,
                            (f.x[*i], f.z[*i], f.yaw[*i], c.len, c.wid),
                        ) <= 0.0
                })
            })
            .map(|(_, c)| c.id)
            .collect();
        frames.push(json!({"time":world.time,"cars":cars,"walkers":walkers,"aspects":aspects,"metrics":{
            "moving":stats.moving,"overlaps":stats.overlaps,"offLane":stats.off_lane,"personInCar":stats.person_in_car,
            "redEntries":f.stats.red_entries,"stopSignStops":f.stats.stop_sign_stops,"outsideWalk":r.crowd.stats.starts_outside_walk,
            "respawns":f.stats.respawns,"terminalRespawns":f.stats.terminus_respawns,"gridlockRespawns":f.stats.gridlock_respawns,"claims":f.stats.claims_granted
        },"personConflicts":person_conflicts,"safetyClamps":safety_clamps,"turnCompleted":turn_completed,"opposingCompleted":opposing_completed,"leftBeforeOpposing":left_before_opposing}));
    }
    println!(
        "{}: {} lanes, {} signals, {} frames",
        s.name,
        lanes.len(),
        junctions.len(),
        frames.len()
    );
    Ok(
        json!({"name":s.name,"description":s.description,"roads":roads,"lanes":lanes,"footways":footways,"junctions":junctions,"frames":frames,"sampleHz":60/sample_steps,"probe":probe.is_some(),"turn":s.turn.map(|t|format!("{:?}",t.movement)),"opposing":opposing_target.is_some(),"cyclistProxy":cyclist}),
    )
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: traffic_lab <output.html>")?;
    let paths = eo_data::DataPaths::discover()?;
    let standards = Standards::load(&paths)?;
    let dims = StreetDims::from_standards(&standards)?;
    let timing = SignalTiming::from_standards(&standards)?;
    let traces = scenarios()
        .into_iter()
        .map(|s| record(s, &dims, &timing))
        .collect::<Result<Vec<_>>>()?;
    let data = serde_json::to_string(&traces)?.replace('<', "\\u003c");
    let html = include_str!("traffic_lab/replay.html").replace("/*__TRACE__*/", &data);
    std::fs::write(&path, html)?;
    println!("Report: {path}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pedestrian_probes_yield_hold_and_resume() -> Result<()> {
        let paths = eo_data::DataPaths::discover()?;
        let standards = Standards::load(&paths)?;
        let dims = StreetDims::from_standards(&standards)?;
        let timing = SignalTiming::from_standards(&standards)?;
        for s in scenarios().into_iter().filter(|s| s.probe.is_some()) {
            let name = s.name;
            let trace = record(s, &dims, &timing)?;
            let frames = trace["frames"].as_array().context("frames")?;
            assert_eq!(trace["sampleHz"], 60);
            let mut detected = false;
            let mut braked = false;
            let mut held = false;
            for f in frames {
                let car = &f["cars"][0];
                let present = !f["walkers"].as_array().unwrap().is_empty();
                let speed = car["speed"].as_f64().unwrap();
                detected |= car["personObserved"].as_bool().unwrap();
                braked |=
                    car["personObserved"].as_bool().unwrap() && car["braking"].as_bool().unwrap();
                if present {
                    if held {
                        assert!(speed <= 0.05, "{name}: moved while holding for person");
                    }
                    held |= speed <= 0.05;
                }
                assert_eq!(f["metrics"]["personInCar"], 0, "{name}: body overlap");
                assert_eq!(f["safetyClamps"], 0, "{name}: abrupt speed clamp");
            }
            assert!(
                detected && braked && held,
                "{name}: no complete yield response"
            );
            let last = frames.last().unwrap();
            assert!(last["walkers"].as_array().unwrap().is_empty());
            assert!(
                last["cars"][0]["speed"].as_f64().unwrap() > 0.5,
                "{name}: failed to resume"
            );
        }
        Ok(())
    }

    #[test]
    fn turn_replays_record_real_corner_and_exit() -> Result<()> {
        let paths = eo_data::DataPaths::discover()?;
        let standards = Standards::load(&paths)?;
        let dims = StreetDims::from_standards(&standards)?;
        let timing = SignalTiming::from_standards(&standards)?;
        for s in scenarios().into_iter().filter(|s| s.turn.is_some()) {
            let name = s.name;
            let turn = s.turn.unwrap();
            let trace = record(s, &dims, &timing)?;
            let frames = trace["frames"].as_array().context("frames")?;
            let movement = format!("{:?}", turn.movement);
            assert!(frames.iter().any(|f| f["cars"][0]["corner"] == true && f["cars"][0]["movement"] == movement), "{name}: intended corner not traversed");
            assert!(
                frames.last().unwrap()["turnCompleted"] == true,
                "{name}: exit not reached"
            );
            assert!(
                frames.iter().all(|f| f["metrics"]["overlaps"] == 0
                    && f["metrics"]["offLane"] == 0
                    && f["metrics"]["personInCar"] == 0),
                "{name}: sampled overlap or off-lane pose"
            );
            if turn.opposing {
                assert!(
                    frames.last().unwrap()["opposingCompleted"] == true,
                    "{name}: opposing vehicle did not cross"
                );
                // The fixture deliberately exposes whether the opposing turn
                // order is wrong; this trace-integrity test is not a priority gate.
                assert!(frames.last().unwrap()["leftBeforeOpposing"].is_boolean());
            }
            if turn.cyclist {
                assert!(frames.iter().any(|f| f["cars"][0]["personObserved"] == true
                    && f["walkers"][0]["cyclistProxy"] == true));
                assert!(
                    frames
                        .iter()
                        .any(|f| f["walkers"][0]["cyclistProxy"] == true
                            && f["cars"][0]["speed"].as_f64().unwrap() <= 0.05)
                );
                assert!(frames.last().unwrap()["cars"][0]["speed"].as_f64().unwrap() > 0.5);
            }
        }
        Ok(())
    }
}
