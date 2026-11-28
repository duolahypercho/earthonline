//! Offline replays of synthetic fixtures or the shipped road graph.
//! The production clock and lane builder run without graphics assets.
use anyhow::{Context, Result, ensure};
use eo_data::sim_source::{Oneway, SimRoad, SimSource, TrafficPaint, TrafficPlan, TrafficSection, TrafficStop, TrafficStructures};
use eo_sim::geom::{Movement, P2, at_arc};
use eo_sim::native::NativeSource;
use eo_sim::network::{ControlKind, NONE, TILE_M, TileKey};
use eo_sim::{
    Focus, Network, People, Region, RegionConfig, SignalTiming, Standards, StreetDims, World,
};
use serde_json::{Value, json};
use std::sync::Arc;

#[path = "traffic_lab/audit.rs"]
mod audit;
#[path = "traffic_lab/vehicle_scale.rs"]
mod vehicle_scale;

use audit::{TileAudit, WindowAudit};

const SAMPLE_STEPS: usize = 12;
const SECONDS: usize = 90;
const CITY_WINDOW_M: f64 = 600.0;
const DETAIL_WINDOW_M: f64 = 94.0;

#[derive(Clone, Copy, Default)]
struct ModelOptions<'a> {
    report: Option<&'a Arc<vehicle_scale::ScaleReport>>,
    drive: bool,
    great_highway_pair: bool,
}

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
            roads: s.roads.clone(),
            signals: s.signals.clone(),
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
    record_network(s, &source, &tiles, None, 1, None, ModelOptions::default())
}

fn poses(r: &Region) -> impl Iterator<Item = (u32, [f64; 3])> + '_ {
    r.fleet.cars.iter().enumerate().map(|(i, c)| (c.id, [r.fleet.x[i], r.fleet.surface_y[i], r.fleet.z[i]]))
}

fn record_network(
    s: Scenario,
    source: &NativeSource,
    tiles: &[TileKey],
    focus: Option<Focus>,
    seed: u32,
    mut audit: Option<&mut WindowAudit>,
    models: ModelOptions<'_>,
) -> Result<Value> {
    let replay = audit.is_none();
    let build = source.build(tiles);
    let roads: Vec<_> = build
        .segments
        .iter()
        .filter(|_| replay)
        .map(|s| json!({"pts":s.points,"width":2.0*s.half}))
        .collect();
    let footway_sources: Vec<_> = build.footways.iter().filter(|_| replay).map(|f| {
        json!({"segment":f.segment,"side":f.road_side,"originalArcStart":f.original_arc_start,
            "kerbTrim":[f.ends[0].kerb_trim,f.ends[1].kerb_trim]})
    }).collect();
    let mut net = Network::new(source.timing.clone());
    if let Some(report) = models.report.filter(|_| models.drive) {
        net = net.with_model_footprints(Arc::clone(report), None);
    }
    net.add(build);
    if let Some(summary) = audit.as_deref_mut() {
        summary.lanes = net.alive_lanes().count();
        summary.signal_controllers = net.junctions.len();
        summary.mapped_stop_ids = net.alive_lanes().filter_map(|lane| match net.lanes[lane as usize].control {
            ControlKind::MappedStop { id, .. } => Some(id),
            _ => None,
        }).collect();
        summary.mapped_stop_lanes = summary.mapped_stop_ids.len();
        summary.mapped_stop_ids.sort_unstable();
        summary.mapped_stop_ids.dedup();
        if summary.lanes == 0 {
            return Ok(Value::Null);
        }
    } else {
        ensure!(!net.lanes.is_empty(), "empty fixture");
    }
    let lanes: Vec<_> = net.alive_lanes().filter(|_| replay).map(|id| {
        let l = &net.lanes[id as usize];
        let (x, z, dx, dz, _) = at_arc(&l.pts, &l.cum, l.length * 0.5);
        let bar = match l.control {
            ControlKind::Signal { junction, axis, stop_offset_m, bar_depth_m } => {
                let (x,z,dx,dz,_) = at_arc(&l.pts,&l.cum,l.length-stop_offset_m);
                json!({"x":x,"z":z,"dx":dx,"dz":dz,"width":l.width,"depth":bar_depth_m,"junction":junction,"axis":axis})
            }
            ControlKind::MappedStop { id, all_way, stop_offset_m, bar_depth_m } => {
                let (x,z,dx,dz,_) = at_arc(&l.pts,&l.cum,l.length-stop_offset_m);
                json!({"x":x,"z":z,"dx":dx,"dz":dz,"width":l.width,"depth":bar_depth_m,"stopId":id,"allWay":all_way})
            }
            _ => Value::Null,
        };
        let links: Vec<_> = l.links.iter().enumerate().map(|(i, link)| {
            let path = link.plan.as_ref().map(|p| (0..=16).map(|n| {
                let (x,z,_) = p.pose_at(p.arc_len * n as f64 / 16.0); [x,z]
            }).collect::<Vec<_>>());
            json!({"index":i,"to":link.to,"movement":format!("{:?}",link.movement),"legal":link.legal,"drivable":link.drivable,"path":path})
        }).collect();
        json!({"id":id,"key":l.key,"pts":l.pts,"width":l.width,"speed":l.max_speed,"level":l.level,"arrow":[x,z,dx,dz],"bar":bar,"links":links})
    }).collect();
    let footways: Vec<_> = net
        .alive_footways()
        .filter(|_| replay)
        .map(|id| net.footways[id as usize].pts.clone())
        .collect();
    let junctions: Vec<_> = net.junctions.iter().filter(|_| replay).map(|j| json!({"centre":j.centre,"cycle":j.cycle_s,"axes":j.axes.iter().map(|a| json!({"green":a.green_s,"yellow":a.yellow_s,"allRed":a.all_red_s})).collect::<Vec<_>>()})).collect();

    let mut world = World::new(net);
    let mut cfg = if focus.is_some() { RegionConfig::game_default(seed) } else { RegionConfig::js_pool(seed, s.people) };
    cfg.vehicle_pool = s.cars;
    cfg.lod = false;
    if models.great_highway_pair {
        cfg.vehicle_pool = 0;
        cfg.people = People::None;
    }
    if focus.is_none() {
        cfg.people = if s.people == 0 { People::None } else { People::Pool(s.people) };
    }
    world.regions.push(Region::new(
        &world.net,
        0,
        cfg,
        focus.unwrap_or(Focus {
            x: 0.0,
            z: 0.0,
            yaw: 0.0,
        }),
    ));
    let mut held_bus = None;
    if models.great_highway_pair {
        use eo_sim::{consts::{PRESENTED_LENGTH, PRESENTED_WIDTH}, output::KIND_BUS, rng::Mulberry32};
        let kind = KIND_BUS as usize;
        let fleet = &mut world.regions[0].fleet;
        let mut rng = Mulberry32::new(seed);
        // Starts from the original run just before its measured footprints
        // collide. The leading bus stays stopped for the clearance probe.
        for (key, arc, speed) in [("n1159787328:0-forward-2", 523.66, 0.0), ("n28576301:0-forward-1", 511.69, 6.53)] {
            let lane = world.net.lane_by_key(key).with_context(|| format!("missing Great Highway fixture lane {key}"))?;
            ensure!(fleet.spawn(&world.net, &[lane], &mut rng, seed), "cannot stage the Great Highway bus");
            let i = fleet.len() - 1;
            let car = &mut fleet.cars[i];
            car.kind = kind as u8;
            let bounds = models.report.filter(|_| models.drive).and_then(|r| r.driving_dimensions(seed, car.id, car.kind));
            (car.len, car.wid) = bounds.unwrap_or((PRESENTED_LENGTH[kind].into(), PRESENTED_WIDTH[kind].into()));
            fleet.place(&world.net, i, lane, arc, speed);
            if i == 0 { held_bus = Some((lane, arc)); }
        }
    }
    ensure!(!replay || !world.regions[0].fleet.is_empty(), "no vehicles spawned");
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
    let mut previous_speed = world.regions[0].fleet.cars.first().map_or(0.0, |c| c.speed);
    let mut safety_clamps = 0u64;
    let mut turn_completed = false;
    let mut opposing_completed = false;
    let mut left_before_opposing = false;
    let mut previous: std::collections::HashMap<_, _> = poses(&world.regions[0]).collect();
    let mut current = std::collections::HashMap::new();
    let mut biggest_step = 0.0_f64;
    let mut jump_id = None;
    let mut jumps = 0;
    let mut surface_jumps = 0;
    let mut biggest_surface_step = 0.0_f64;
    let mut biggest_surface_event = Value::Null;
    let mut model_audit = models.report.map(|report| vehicle_scale::Audit::new(report, seed));
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
        if let Some((lane, arc)) = held_bus {
            world.regions[0].fleet.place(&world.net, 0, lane, arc, 0.0);
        }
        if let Some(models) = &mut model_audit {
            models.observe(&world.net, &world.regions[0].fleet, world.time);
        }
        current.clear();
        let region = &world.regions[0];
        for (i, (id, p)) in poses(region).enumerate() {
            current.insert(id, p);
            if let Some(&[x, y, z]) = previous.get(&id) {
                let distance = (x - p[0]).hypot(z - p[2]);
                biggest_step = biggest_step.max(distance);
                // At 60 Hz an autonomous car cannot cover a metre in one step.
                if distance > 1.0 {
                    jumps += 1;
                    jump_id.get_or_insert(id);
                }
                if y.is_finite() && p[1].is_finite() {
                    let rise = (y - p[1]).abs();
                    if replay && rise > biggest_surface_step {
                        let car = &region.fleet.cars[i];
                        biggest_surface_event = json!({
                            "id": id, "step": step, "time": world.time,
                            "from": [x, y, z], "to": p, "deltaM": rise,
                            "lane": world.net.lanes[car.lane as usize].key,
                            "arc": car.arc, "cornerArc": car.cornering().then_some(car.corner_s),
                            "approach": world.net.lanes.get(car.corner_from as usize).map(|lane| &lane.key),
                        });
                    }
                    biggest_surface_step = biggest_surface_step.max(rise);
                    surface_jumps += usize::from(rise > 1.0);
                }
            }
        }
        std::mem::swap(&mut current, &mut previous);
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
        let stats = eo_sim::stats::sample(&world.net, r, world.time);
        if let Some(summary) = audit.as_deref_mut() {
            summary.observe(r, &stats, step, jumps, surface_jumps, biggest_surface_step);
            continue;
        }
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
            let mut car = json!({
            "id":c.id,"kind":c.kind,"x":f.x[i],"z":f.z[i],"yaw":f.yaw[i],"length":c.len,"width":c.wid,
            "speed":c.speed,"surfaceY":f.surface_y[i],"lane":c.lane,"next": (c.next != eo_sim::vehicles::NO_LINK).then_some(c.next),
            "braking":c.braking,"turn":c.turn_side,"corner":c.cornering(),"claim":(c.claim_node != NONE).then_some(c.claim_node),
            "passing":c.passing(),"horn":f.horn_active(i,world.time),
            "wait":c.junction_wait,"arc":c.arc,"stopHold":c.stop_hold,"stopReleased":c.stop_released,
            "personObserved":person,"crossingObserved":crossing,
            "movement": if c.cornering() { world.net.lanes[c.corner_from as usize].links.get(c.corner_link as usize).map(|l|format!("{:?}",l.movement)) } else { world.net.lanes[c.lane as usize].links.get(c.next as usize).map(|l|format!("{:?}",l.movement)) },
            "cornerFrom": (c.corner_from != NONE).then_some(c.corner_from), "cornerLink": (c.corner_link != eo_sim::vehicles::NO_LINK).then_some(c.corner_link)
            });
            if let Some(model) = model_audit.as_ref().and_then(|audit| audit.vehicle(c.id, c.kind)) {
                car["model"] = json!(model.model);
                car["modelBounds"] = json!(model.chassis_contact);
            }
            car
        }).collect();
        let walkers: Vec<_> = r.crowd.walkers.iter().enumerate().map(|(i,w)| json!({
            "key":w.key,"footway":w.footway,"arc":w.s,
            "leg":w.leg.map(|l|match l.kind { eo_sim::pedestrians::LegKind::Corner => "corner", eo_sim::pedestrians::LegKind::Cross => "crossing" }),
            "destination":w.leg.map(|l|l.next),"waitElapsed":w.kerb.map(|k|k.elapsed),
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
        // Identify the vehicles involved in the sampled person/body diagnostic,
        // using the same geometry and predicate as stats::sample.
        let person_conflicts: Vec<_> = f
            .cars
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                r.crowd.x.iter().zip(&r.crowd.z).any(|(&x, &z)| {
                    f.level[*i] == 0 && eo_sim::geom::hypot(f.x[*i] - x, f.z[*i] - z) <= 8.0
                        && eo_sim::geom::obb_clearance(
                            x,
                            z,
                            (f.x[*i], f.z[*i], f.yaw[*i], c.len, c.wid),
                        ) <= 0.0
                })
            })
            .map(|(_, c)| c.id)
            .collect();
        let mut frame = json!({"time":world.time,"cars":cars,"walkers":walkers,"aspects":aspects,"metrics":{
            "moving":stats.moving,"overlaps":stats.overlaps,"offLane":stats.off_lane,"personInCar":stats.person_in_car,
            "redEntries":f.stats.red_entries,"stopSignStops":f.stats.stop_sign_stops,"outsideWalk":r.crowd.stats.starts_outside_walk,
            "crossingsStarted":r.crowd.stats.crossings_started,"crossingsCompleted":r.crowd.stats.crossings_completed,
            "respawns":f.stats.respawns,"terminalRespawns":f.stats.terminus_respawns,"gridlockRespawns":f.stats.gridlock_respawns,"claims":f.stats.claims_granted,
            "replacements":r.stats.car_recycles,"jumps":jumps,"biggestStep":biggest_step,
            "surfaceHeightJumps":surface_jumps,"biggestSurfaceStepM":biggest_surface_step,
            "passes":f.stats.passes_started,"horns":f.stats.hazard_horns
        },"jumpId":jump_id.take(),"personConflicts":person_conflicts,"safetyClamps":safety_clamps,"turnCompleted":turn_completed,"opposingCompleted":opposing_completed,"leftBeforeOpposing":left_before_opposing});
        if let Some(models) = &model_audit {
            frame["modelOverlaps"] = json!(models.overlaps);
        }
        frames.push(frame);
    }
    if !replay {
        if let (Some(audit), Some(models)) = (audit, model_audit) {
            audit.model_audit = Some(models.summary);
        }
        return Ok(Value::Null);
    }
    println!(
        "{}: {} lanes, {} signals, {} frames",
        s.name,
        lanes.len(),
        junctions.len(),
        frames.len()
    );
    let mut trace = json!({"name":s.name,"description":s.description,"roads":roads,"lanes":lanes,"footways":footways,"footwaySources":footway_sources,"junctions":junctions,"frames":frames,"sampleHz":60/sample_steps,"seed":seed,"personRadius":eo_sim::consts::PERSON_CAPSULE_R_M,"probe":probe.is_some(),"turn":s.turn.map(|t|format!("{:?}",t.movement)),"opposing":opposing_target.is_some(),"cyclistProxy":cyclist,"focus":focus.map(|f|[f.x,f.z])});
    trace["biggestSurfaceEvent"] = biggest_surface_event;
    if let (Some(report), Some(model_audit)) = (models.report, model_audit) {
        trace["avatarHeightM"] = json!(report.avatar_height_m);
        trace["avatarModel"] = json!(report.avatar_model);
        trace["measuredChassis"] = Value::Object(
            report
                .physical_chassis()
                .map(|(model, chassis)| (model.to_owned(), json!(chassis)))
                .collect(),
        );
        trace["drivenModelFootprints"] = json!(models.drive);
        trace["modelAudit"] = json!(model_audit.summary);
        println!("Measured footprints: {} / {} steps with overlap, {} missed by the controller",
            model_audit.summary.steps_with_overlap, model_audit.summary.steps, model_audit.summary.steps_with_missed_overlap);
    }
    Ok(trace)
}

struct CityOptions<'a> {
    street_plan: Option<&'a str>,
    structure_levels: Option<&'a str>,
    driving_models: Option<&'a Arc<vehicle_scale::ScaleReport>>,
    passing: bool,
    bend_braking: bool,
    mapped_stops: bool,
    drawn_walk_gaps: bool,
    seed: u32,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StructureInput {
    Drawn(TrafficStructures),
    Levels(std::collections::BTreeMap<String, i8>),
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StreetInput {
    Drawn(TrafficPlan),
    Sections(Vec<TrafficSection>),
}

fn city_source(
    paths: &eo_data::DataPaths,
    options: &CityOptions<'_>,
    paint_window: Option<(P2, f64)>,
) -> Result<(NativeSource, Vec<TrafficPaint>)> {
    let (levels, surfaces) = if let Some(path) = options.structure_levels {
        let input = std::io::BufReader::new(std::fs::File::open(path).with_context(|| format!("opening {path}"))?);
        let (levels, surfaces) = match serde_json::from_reader(input).with_context(|| format!("reading {path}"))? {
            StructureInput::Drawn(structures) => (structures.levels, structures.surfaces),
            StructureInput::Levels(levels) => (levels, Vec::new()),
        };
        ensure!(levels.values().all(|&l| (-8..=7).contains(&l)), "unsupported structure level in {path}");
        (levels.into_iter().collect(), surfaces)
    } else {
        (SimSource::load(paths)?.roads.into_iter().filter(|r| r.bridge || r.tunnel)
            .map(|r| (r.id, r.layer.clamp(-8, 7) as i8)).collect(), Vec::new())
    };
    let mut source = eo_sim::native::load(paths)?.with_levels(levels)
        .with_deck_surfaces(surfaces).with_passing(options.passing).with_bend_braking(options.bend_braking);
    if let Some(report) = options.driving_models {
        source = source.with_model_footprints(Arc::clone(report));
    }
    if !options.mapped_stops {
        source = source.with_stops(Vec::new());
    }
    let mut markings = Vec::new();
    if let Some(path) = options.street_plan {
        let input = std::io::BufReader::new(std::fs::File::open(path).with_context(|| format!("opening {path}"))?);
        let plan = match serde_json::from_reader(input).with_context(|| format!("reading {path}"))? {
            StreetInput::Drawn(plan) => plan,
            StreetInput::Sections(sections) => TrafficPlan { sections, ..Default::default() },
        };
        if let Some(([x, z], span)) = paint_window {
            for section in &plan.sections {
                if eo_sim::geom::dist_to_polyline(&section.points, x, z) <= span {
                    markings.extend_from_slice(&section.paint);
                }
            }
        }
        source = source.with_traffic_plan(plan);
        if !options.drawn_walk_gaps {
            source = source.without_drawn_walk_gaps();
        }
    }
    Ok((source, markings))
}

fn city_scenario(title: &'static str, paved: bool) -> Scenario {
    Scenario {
        name: title,
        description: if paved {
            "Paved street cross-sections where matched, mapped signals and structure layers in a fixed city window."
        } else {
            "Shipped road geometry, mapped signals and structure layers in a fixed city window."
        },
        roads: vec![],
        signals: vec![],
        cars: eo_sim::consts::VEHICLE_POOL,
        people: 0,
        probe: None,
        turn: None,
    }
}

fn tile_center(tile: TileKey) -> P2 {
    tile.origin().map(|coordinate| coordinate + TILE_M / 2.0)
}

fn city_replay(
    paths: &eo_data::DataPaths,
    name: &str,
    options: &CityOptions<'_>,
    mut models: ModelOptions<'_>,
) -> Result<Value> {
    let plan = eo_data::groundscale::plan;
    let mut stop_id = None;
    let (title, point, yaw, span) = match name {
        "soma" => ("SoMa / Folsom Street", plan([1539.0, -366.0]), 0.0, CITY_WINDOW_M),
        "folsom-fourth" => ("Folsom and 4th Street", plan([1602.81, -445.663]), 0.0, DETAIL_WINDOW_M),
        "golden-gate" => ("Golden Gate / South approach", plan([-5027.0, -3328.0]), std::f64::consts::PI, CITY_WINDOW_M),
        "great-highway-clearance" => {
            ensure!(models.report.is_some(), "Great Highway clearance fixture needs --vehicle-scale-report");
            models.great_highway_pair = true;
            ("Great Highway / Bus clearance", tile_center(TileKey { x: -38, z: 24 }), std::f64::consts::PI, CITY_WINDOW_M)
        }
        _ if name.starts_with("stop:") => {
            let id = name[5..].parse::<u64>().context("OpenStreetMap stop node ID")?;
            let stop = TrafficStop::load()?.into_iter().find(|stop| stop.id == id)
                .with_context(|| format!("stop node {id} has no record in the city pack"))?;
            stop_id = Some(id);
            ("Mapped stop", stop.position, 0.0, DETAIL_WINDOW_M)
        }
        _ => {
            let coords = name.strip_prefix("tile:").and_then(|key| key.split_once(','))
                .with_context(|| format!("unknown map {name:?}. Choose soma, folsom-fourth, golden-gate, great-highway-clearance, tile:X,Z or stop:ID."))?;
            let tile = TileKey { x: coords.0.parse().context("tile x")?, z: coords.1.parse().context("tile z")? };
            ("Road tile", tile_center(tile), 0.0, CITY_WINDOW_M)
        }
    };
    let (source, markings) = city_source(paths, options, Some((point, span)))?;
    let [x, z] = point;
    let tiles = source.resident_tiles(x, z, CITY_WINDOW_M);
    let focus = Focus { x, z, yaw };
    let mut trace = record_network(
        city_scenario(title, options.street_plan.is_some()), &source, &tiles, Some(focus), options.seed, None, models,
    )?;
    trace["paint"] = json!(markings);
    trace["mapSpan"] = json!(span);
    trace["showFootprints"] = json!(span == DETAIL_WINDOW_M);
    if let Some(id) = stop_id {
        trace["name"] = json!(format!("{title} {id}"));
    }
    trace["stopSource"] = json!(stop_source(options));
    trace["drawnWalkGaps"] = json!(options.drawn_walk_gaps && options.street_plan.is_some());
    trace["bendBraking"] = json!(options.bend_braking);
    trace["walkLaneClearance"] = trace["drawnWalkGaps"].clone();
    trace["structureLevelSource"] = json!(level_source(options));
    Ok(trace)
}

fn audit_city(
    paths: &eo_data::DataPaths,
    options: &CityOptions<'_>,
    models: ModelOptions<'_>,
) -> Result<String> {
    let (source, _) = city_source(paths, options, None)?;
    let seed = options.seed;
    let paved = options.street_plan.is_some();
    let road_tiles = source.vehicle_road_tiles();
    ensure!(!road_tiles.is_empty(), "no vehicle-road tiles to audit");
    println!("Auditing {} road windows with seed {seed}", road_tiles.len());
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let windows = std::thread::scope(|scope| -> Result<Vec<_>> {
        let mut jobs = Vec::new();
        for batch in road_tiles.chunks(road_tiles.len().div_ceil(workers)) {
            let source = &source;
            jobs.push(scope.spawn(move || {
                batch.iter().map(|&tile| {
                    audit_tile(source, tile, paved, seed, models)
                }).collect::<Result<Vec<_>>>()
            }));
        }
        let mut windows = Vec::with_capacity(road_tiles.len());
        // Join in input order so worker scheduling cannot change the report.
        for job in jobs {
            windows.extend(job.join().map_err(|_| anyhow::anyhow!("road audit worker panicked"))??);
        }
        Ok(windows)
    })?;
    let mapped_stop_ids: std::collections::BTreeSet<_> = windows.iter()
        .flat_map(|tile| tile.observations.mapped_stop_ids.iter().copied()).collect();
    let mut summary = json!({
        "seed": seed, "secondsPerWindow": SECONDS, "sampleEverySteps": SAMPLE_STEPS,
        "pavedStreetPlan": paved, "passing": options.passing && paved,
        "drawnWalkGaps": options.drawn_walk_gaps && paved,
        "bendBraking": options.bend_braking,
        "walkLaneClearance": options.drawn_walk_gaps && paved,
        "structureLevelSource": level_source(options), "stopSource": stop_source(options),
        "mappedStopRecords": mapped_stop_ids.len(),
        "groundScale": eo_data::groundscale::scale(), "windows": windows,
    });
    if let Some(report) = models.report {
        summary["avatarHeightM"] = json!(report.avatar_height_m);
        summary["drivenModelFootprints"] = json!(models.drive);
    }
    serde_json::to_string_pretty(&summary).context("serializing city audit")
}

fn stop_source(options: &CityOptions<'_>) -> &'static str {
    if options.mapped_stops { "mapped stops" } else { "disabled" }
}

fn level_source(options: &CityOptions<'_>) -> &'static str {
    if options.structure_levels.is_some() { "built structures" } else { "map tags" }
}

fn audit_tile(source: &NativeSource, tile: TileKey, paved: bool, seed: u32, models: ModelOptions<'_>) -> Result<TileAudit> {
    let mut window = TileAudit::new(tile);
    let [x, z] = tile_center(tile);
    let tiles = source.resident_tiles(x, z, CITY_WINDOW_M);
    record_network(
        city_scenario("Road tile", paved),
        source,
        &tiles,
        Some(Focus { x, z, yaw: 0.0 }),
        seed,
        Some(&mut window.observations),
        models,
    ).with_context(|| format!("auditing {} with seed {seed}", window.map))?;
    Ok(window)
}

fn replay(traces: &[Value], path: &std::path::Path) -> Result<String> {
    if path.extension().is_some_and(|extension| extension == "json") {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Recording<'a> {
            ground_scale: f64,
            traces: &'a [Value],
        }
        return Ok(serde_json::to_string(&Recording {
            ground_scale: eo_data::groundscale::scale(),
            traces,
        })?);
    }
    let data = serde_json::to_string(traces)?.replace('<', "\\u003c");
    Ok(include_str!("traffic_lab/replay.html").replace("/*__TRACE__*/", &data))
}

fn main() -> Result<()> {
    let usage = "usage: traffic_lab <output> [--map soma|folsom-fourth|golden-gate|great-highway-clearance|tile:X,Z|stop:ID or --audit-city] [--street-plan <plan.json>] [--structure-levels <levels.json>] [--vehicle-scale-report <scale.json> [--drive-model-footprints]] [--seed <u32>] [--no-traffic-passing] [--traffic-bend-braking] [--no-mapped-stops] [--no-drawn-walk-gaps]";
    let mut args = std::env::args().skip(1);
    let path = std::path::PathBuf::from(args.next().context(usage)?);
    let paths = eo_data::DataPaths::discover()?;
    let mode = args.next();
    let output = match mode.as_deref() {
        None => {
            let standards = Standards::load(&paths)?;
            let dims = StreetDims::from_standards(&standards)?;
            let timing = SignalTiming::from_standards(&standards)?;
            let traces = scenarios().into_iter().map(|s| record(s, &dims, &timing)).collect::<Result<Vec<_>>>()?;
            replay(&traces, &path)?
        }
        Some("--map" | "--audit-city") => {
            let name = if mode.as_deref() == Some("--map") { Some(args.next().context(usage)?) } else { None };
            let mut street_plan = None;
            let mut structure_levels = None;
            let mut scale_report = None;
            let mut drive_models = false;
            let mut passing = true;
            let mut bend_braking = false;
            let mut mapped_stops = true;
            let mut drawn_walk_gaps = true;
            let mut seed = 1;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--street-plan" if street_plan.is_none() => street_plan = Some(args.next().context(usage)?),
                    "--structure-levels" if structure_levels.is_none() => structure_levels = Some(args.next().context(usage)?),
                    "--vehicle-scale-report" if scale_report.is_none() => {
                        let path = args.next().context(usage)?;
                        scale_report = Some(Arc::new(vehicle_scale::ScaleReport::load(std::path::Path::new(&path))?));
                    }
                    "--drive-model-footprints" => drive_models = true,
                    "--no-traffic-passing" => passing = false,
                    "--traffic-bend-braking" => bend_braking = true,
                    "--no-traffic-bend-braking" => bend_braking = false,
                    "--no-mapped-stops" => mapped_stops = false,
                    "--no-drawn-walk-gaps" => drawn_walk_gaps = false,
                    "--seed" => seed = args.next().context(usage)?.parse().context("traffic seed")?,
                    _ => anyhow::bail!("{usage}"),
                }
            }
            let options = CityOptions {
                street_plan: street_plan.as_deref(), structure_levels: structure_levels.as_deref(),
                driving_models: scale_report.as_ref().filter(|_| drive_models), passing, bend_braking, mapped_stops, drawn_walk_gaps, seed,
            };
            ensure!(!drive_models || scale_report.is_some(), "--drive-model-footprints needs --vehicle-scale-report");
            let models = ModelOptions { report: scale_report.as_ref(), drive: drive_models, great_highway_pair: false };
            if let Some(name) = name {
                replay(&[city_replay(&paths, &name, &options, models)?], &path)?
            } else {
                audit_city(&paths, &options, models)?
            }
        }
        _ => anyhow::bail!("{usage}"),
    };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, output)?;
    println!("Report: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_tile_audit_matches_the_replay_for_the_same_seed() -> Result<()> {
        let paths = eo_data::DataPaths::discover()?;
        let options = CityOptions { street_plan: None, structure_levels: None, driving_models: None, passing: true, bend_braking: false, mapped_stops: true, drawn_walk_gaps: true, seed: 7 };
        let (source, _) = city_source(&paths, &options, None)?;
        let [x, z] = tile_center(TileKey { x: 7, z: -2 });
        let focus = Some(Focus { x, z, yaw: 0.0 });
        let tiles = source.resident_tiles(x, z, CITY_WINDOW_M);
        let audit = audit_tile(&source, TileKey { x: 7, z: -2 }, false, 7, ModelOptions::default())?.observations;
        let trace = record_network(city_scenario("Road tile", false), &source, &tiles, focus, 7, None, ModelOptions::default())?;
        let frames = trace["frames"].as_array().context("frames")?;
        let counted = |metric: &str| {
            frames.iter().filter(|f| f["metrics"][metric].as_u64().unwrap() != 0).count()
        };
        let last = &frames.last().context("last frame")?["metrics"];

        assert!(audit.max_cars > 0 && audit.max_people > 0);
        assert_eq!(audit.samples, frames.len());
        assert_eq!(audit.frames_with_vehicle_overlap, counted("overlaps"));
        assert_eq!(audit.frames_with_person_in_car, counted("personInCar"));
        assert_eq!(audit.frames_off_lane_center, counted("offLane"));
        assert_eq!(last["jumps"], audit.same_id_jumps);
        assert_eq!(last["surfaceHeightJumps"], audit.surface_height_jumps);
        assert_eq!(last["crossingsStarted"], audit.crossings_started);
        assert_eq!(last["crossingsCompleted"], audit.crossings_completed);
        assert_eq!(last["biggestSurfaceStepM"], audit.biggest_surface_step_m);
        assert_eq!(last["redEntries"], audit.red_entries);
        assert_eq!(last["outsideWalk"], audit.walk_violations);
        assert_eq!(last["replacements"], audit.replacements);
        Ok(())
    }

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
                assert_eq!(frames.last().unwrap()["leftBeforeOpposing"], false, "{name}: left turn cut off opposing traffic");
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
