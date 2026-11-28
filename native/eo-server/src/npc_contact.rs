//! One authoritative dynamic body per contacted fleet vehicle. The region
//! keeps its route cursor while its driver follows that route in physics.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use eo_net::game::GameWorld;
use eo_player::traffic::{TrafficChassis, TrafficPose};
use eo_player::traffic_contact::{TrafficDriver, TrafficModel, TrafficTarget};
use eo_player::vehicle::{VehicleModel, resolve_peer_contact};
use eo_player::world::{CollisionWorld, TileFeed};
use eo_player::{WeatherPhysics, tuning::FIXED_STEP_S};
use eo_sim::Network;
use eo_sim::grid::Grid;
use eo_sim::vehicle_catalogue::RENDERED_SEED;
use eo_sim::vehicles::{Fleet, TrafficObservation, TrafficObstacle};
use glam::{DQuat, DVec3};

use crate::{Regions, Session};

const QUIET_STEPS: u32 = (0.5 / FIXED_STEP_S) as u32;
const REJOIN_DISTANCE_M: f64 = 0.20;
const REJOIN_SPEED_MPS: f64 = 0.5;
const REJOIN_ANGLE_COS: f64 = 0.999;

struct Peer {
    session: usize,
    vehicle: usize,
    centre: DVec3,
    reach: f64,
    observation: TrafficObservation,
}

struct Active {
    driver: TrafficDriver,
    quiet: u32,
    level: i8,
}

struct RouteContact {
    target: TrafficTarget,
    level: i8,
    peers: Vec<usize>,
}

pub(super) struct NpcContacts {
    world: CollisionWorld,
    bodies: BTreeMap<u32, Active>,
    grid: Grid,
}

impl NpcContacts {
    pub(super) fn new(game: &GameWorld, tiles: Arc<dyn TileFeed>) -> Self {
        Self {
            world: CollisionWorld::fed(game.ground.clone(), game.buildings.clone(), tiles),
            bodies: BTreeMap::new(),
            grid: Grid::new(eo_sim::consts::PERSON_CELL_M),
        }
    }

    pub(super) fn pose(&self, id: u32) -> Option<(DVec3, DQuat, f64)> {
        let body = &self.bodies.get(&id)?.driver.car;
        Some((
            body.datum(&self.world),
            body.pose(&self.world).1,
            body.forward_speed(&self.world),
        ))
    }

    pub(super) fn step(
        &mut self,
        game: &GameWorld,
        sessions: &mut [Session],
        regions: &mut Regions,
        weather: &WeatherPhysics,
    ) {
        let peers = peers(game, sessions);
        let bounds = peers.iter().map(|p| p.reach).fold(0.0_f64, f64::max);
        let xs: Vec<_> = peers.iter().map(|p| p.centre.x).collect();
        let zs: Vec<_> = peers.iter().map(|p| p.centre.z).collect();
        self.grid
            .build(&xs, &zs, eo_sim::consts::PERSON_CELL_M, |_| true);

        let mut contacts = Vec::new();
        let mut live = BTreeSet::new();
        for region in &regions.world.regions {
            let fleet = &region.fleet;
            for (i, car) in fleet.cars.iter().enumerate() {
                let active = self.bodies.get_mut(&car.id);
                if active.is_none() && peers.is_empty() {
                    continue;
                }
                let (x, z, radius) = active.as_ref().map_or_else(
                    || {
                        (
                            fleet.x[i],
                            fleet.z[i],
                            car.len.hypot(car.wid) / 2.0 + fleet.spd[i].abs() * FIXED_STEP_S,
                        )
                    },
                    |active| {
                        let car = &active.driver.car;
                        let p = car.pose(&self.world).0;
                        let body = &self.world.physics.bodies[car.body];
                        let size = car.spec.length.hypot(car.spec.width).hypot(car.spec.height);
                        (
                            p.x,
                            p.z,
                            size / 2.0 + f64::from(body.linvel().length()) * FIXED_STEP_S,
                        )
                    },
                );
                let reach = bounds + radius;
                let mut nearby = Vec::new();
                self.grid
                    .query(x - reach, z - reach, x + reach, z + reach, |j| {
                        let peer = &peers[j as usize];
                        if (x - peer.centre.x).powi(2) + (z - peer.centre.z).powi(2)
                            < (radius + peer.reach).powi(2)
                        {
                            nearby.push(j as usize);
                        }
                    });
                if active.is_none() && nearby.is_empty() {
                    continue;
                }
                let target = route_target(game, &regions.world.net, fleet, i);
                if let Some(body) = active {
                    // Relinquish physics only at the same position, heading and speed.
                    if nearby.is_empty()
                        && body.quiet >= QUIET_STEPS
                        && rejoined(&self.world, body, &target)
                    {
                        continue;
                    }
                    body.driver.target = target;
                    body.level = fleet.level[i];
                    body.quiet = body.quiet.saturating_add(1);
                } else {
                    nearby.retain(|&j| {
                        let p = &peers[j];
                        let s = &sessions[p.session];
                        target.approaching(&s.player.vehicles[p.vehicle], &s.cw)
                    });
                    if nearby.is_empty() {
                        continue;
                    }
                }
                nearby.sort_unstable_by_key(|&j| {
                    (sessions[peers[j].session].player_id, peers[j].vehicle)
                });
                live.insert(car.id);
                contacts.push(RouteContact {
                    target,
                    level: fleet.level[i],
                    peers: nearby,
                });
            }
        }
        self.bodies.retain(|id, body| {
            if live.contains(id) {
                true
            } else {
                self.world.physics.remove_body(body.driver.car.body);
                false
            }
        });

        contacts.sort_unstable_by_key(|r| r.target.pose.id);
        let footprints: Vec<_> = contacts
            .iter()
            .map(|r| {
                let centre = self
                    .pose(r.target.pose.id as u32)
                    .map_or_else(|| datum(&r.target), |p| p.0);
                (centre, f64::from(r.target.pose.chassis.length()))
            })
            .collect();
        self.world.sync_traffic_tiles(&footprints);
        let mut corrected = BTreeSet::new();
        for route in contacts {
            let id = route.target.pose.id as u32;
            let p = datum(&route.target);
            if !self.bodies.contains_key(&id) && !self.world.is_resident(p.x, p.z) {
                continue;
            }
            let active = self.bodies.entry(id).or_insert_with(|| Active {
                driver: TrafficDriver::spawn(&mut self.world, route.target),
                quiet: 0,
                level: route.level,
            });
            for j in route.peers {
                let peer = &peers[j];
                let s = &mut sessions[peer.session];
                if resolve_peer_contact(
                    &s.player.vehicles[peer.vehicle],
                    &mut s.cw,
                    &active.driver.car,
                    &mut self.world,
                ) {
                    active.quiet = 0;
                    corrected.insert(peer.session);
                }
            }
        }
        for session in corrected {
            let s = &mut sessions[session];
            s.stats.traffic_contact_ticks += 1;
            s.queue_state_correction();
        }
        if !self.bodies.is_empty() {
            for active in self.bodies.values_mut() {
                active.driver.drive(&mut self.world, weather);
            }
            self.world.step_physics();
        }

        let physical = self.bodies.iter().map(|(&id, a)| {
            let car = &a.driver.car;
            observed(
                Some(id),
                car.datum(&self.world),
                car.yaw(&self.world),
                car.spec.length,
                car.spec.width,
                a.level,
            )
        });
        let observations: Vec<_> = peers
            .iter()
            .map(|p| p.observation)
            .chain(physical)
            .collect();
        for region in &mut regions.world.regions {
            region.traffic_observations.clear();
            region
                .traffic_observations
                .extend(observations.iter().copied().filter(|o| {
                    let [x, z] = o.obstacle.pos;
                    let reach = eo_sim::consts::LOCAL_RECYCLE_RADIUS;
                    let claim = region.claim;
                    claim.map_or_else(
                        || (x - region.focus.x).hypot(z - region.focus.z) <= reach,
                        |c| {
                            (c.min[0] - reach..=c.max[0] + reach).contains(&x)
                                && (c.min[1] - reach..=c.max[1] + reach).contains(&z)
                        },
                    )
                }));
        }
    }
}

fn datum(target: &TrafficTarget) -> DVec3 {
    DVec3::from(target.pose.datum.map(f64::from))
}

fn rejoined(world: &CollisionWorld, active: &Active, target: &TrafficTarget) -> bool {
    let car = &active.driver.car;
    let rotation = target.rotation();
    let agreement = car.pose(world).1.dot(rotation);
    let velocity = DVec3::from(
        world.physics.bodies[car.body]
            .linvel()
            .to_array()
            .map(f64::from),
    );
    let desired = rotation * DVec3::Z * f64::from(target.speed);
    car.datum(world).distance(datum(target)) <= REJOIN_DISTANCE_M
        && 2.0 * agreement * agreement - 1.0 >= REJOIN_ANGLE_COS
        && velocity.distance(desired) <= REJOIN_SPEED_MPS
}

fn observed(
    fleet_id: Option<u32>,
    datum: DVec3,
    yaw: f64,
    length: f64,
    width: f64,
    level: i8,
) -> TrafficObservation {
    TrafficObservation {
        fleet_id,
        surface_y: datum.y,
        obstacle: TrafficObstacle {
            pos: [datum.x, datum.z],
            yaw,
            length,
            width,
            level,
        },
    }
}

fn peers(game: &GameWorld, sessions: &[Session]) -> Vec<Peer> {
    let mut peers = Vec::new();
    for (session, s) in sessions.iter().enumerate().filter(|(_, s)| s.online) {
        for (vehicle, car) in s.player.vehicles.iter().enumerate() {
            let body = &s.cw.physics.bodies[car.body];
            if !body.is_enabled() {
                continue;
            }
            let at = car.datum(&s.cw);
            let mut separation = (at.y - game.ground.ground_y(at.x, at.z)).abs();
            let mut level = 0;
            if let Some(decks) = &game.decks {
                for (candidate, deck) in decks.decks_at(at.x, at.z) {
                    let distance = (deck.y - at.y).abs();
                    if distance < separation {
                        separation = distance;
                        level = candidate;
                    }
                }
            }
            peers.push(Peer {
                session,
                vehicle,
                centre: car.pose(&s.cw).0,
                reach: car.spec.length.hypot(car.spec.width).hypot(car.spec.height) / 2.0
                    + f64::from(body.linvel().length()) * FIXED_STEP_S,
                observation: observed(
                    None,
                    at,
                    car.yaw(&s.cw),
                    car.spec.length,
                    car.spec.width,
                    level,
                ),
            });
        }
    }
    peers
}

fn route_target(game: &GameWorld, net: &Network, fleet: &Fleet, i: usize) -> TrafficTarget {
    let car = &fleet.cars[i];
    let (x, z) = (fleet.x[i], fleet.z[i]);
    let height = fleet.surface_y[i];
    let level = fleet.level[i];
    let deck = game
        .decks
        .as_ref()
        .filter(|_| level != 0)
        .and_then(|world| world.deck_at(x, z, level).map(|d| (world, d.chain)));
    let measured = net.driving_model(RENDERED_SEED, car.id, car.kind);
    let (chassis, model, span) = match measured.and_then(|m| m.physical_chassis.map(|g| (m, g))) {
        Some((m, geometry)) => (
            TrafficChassis::Mesh {
                min: glam::Vec3::from_array(geometry.min_xyz_m),
                max: glam::Vec3::from_array(geometry.max_xyz_m),
                wheels: geometry.wheels_xz_m,
                wheel_radius: geometry.wheel_radius_m,
            },
            TrafficModel::from_body(&m.model),
            geometry.half_wheelbase_m(),
        ),
        None => (
            TrafficChassis::Class {
                length: car.len as f32,
                width: car.wid as f32,
            },
            match car.kind {
                1 => TrafficModel::Taxi,
                2 => TrafficModel::BoxTruck,
                3 => TrafficModel::CityBus,
                _ => TrafficModel::Road(VehicleModel::Sedan),
            },
            car.len / 4.0,
        ),
    };
    let support = |x, z| {
        let own_deck = deck.and_then(|(world, chain)| world.deck_top(chain, x, z));
        if height.is_finite() {
            [own_deck, game.ground.arrival_road_y(x, z, height)]
                .into_iter()
                .flatten()
                .map(|y| ((y - height).abs(), y))
                .filter(|(rise, _)| *rise <= span)
                .min_by(|(a, _), (b, _)| a.total_cmp(b))
                .map_or(height, |(_, y)| y)
        } else {
            own_deck
                .or_else(|| deck.and_then(|(world, _)| world.deck_at(x, z, level).map(|d| d.y)))
                .unwrap_or_else(|| game.ground.ground_y(x, z))
        }
    };
    let (dx, dz) = fleet.yaw[i].sin_cos();
    let front = support(x + dx * span, z + dz * span);
    let rear = support(x - dx * span, z - dz * span);
    TrafficTarget {
        pose: TrafficPose {
            id: u64::from(car.id),
            datum: [x as f32, ((front + rear) / 2.0) as f32, z as f32],
            yaw: fleet.yaw[i] as f32,
            pitch: ((front - rear) / (2.0 * span)).atan() as f32,
            chassis,
        },
        speed: fleet.spd[i] as f32,
        model,
        parked: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Joining, Server, ServerConfig, Viewer, regions::RegionKey};
    use eo_net::{
        auth::Account,
        conn::Connection,
        proto::{self, Kind},
        snapshot::{self, Decoded, QState},
        wire::read_player_state,
    };
    use eo_player::{BuildingSet, GroundProvider};
    use eo_sim::{NetworkBuild, People};
    use eo_sim::{
        geom::LaneRole,
        network::{ControlKind, LaneBuild},
        rng::Mulberry32,
    };
    use std::{
        net::UdpSocket,
        time::{Duration, Instant},
    };

    struct Flat;

    impl GroundProvider for Flat {
        fn ground_y(&self, _: f64, _: f64) -> f64 {
            0.0
        }

        fn arrival_road_y(&self, _: f64, _: f64, _: f64) -> Option<f64> {
            Some(0.0)
        }
    }

    fn join(server: &mut Server, id: u64, spawn: [f64; 3]) -> UdpSocket {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut cw = CollisionWorld::fed(
            server.game.ground.clone(),
            server.game.buildings.clone(),
            server.tiles.clone(),
        );
        cw.load_blocking(
            DVec3::new(spawn[0], 0.0, spawn[1]),
            eo_player::world::LOAD_RADIUS_M,
        );
        server.joining.push(Joining {
            vehicle: Default::default(),
            connection: id,
            addr: client.local_addr().unwrap(),
            account: Account {
                id,
                display_name: format!("driver-{id}"),
            },
            spawn,
            driving: true,
            cw,
            since: Instant::now(),
        });
        server.pump_joining();
        assert_eq!(server.sessions.last().unwrap().connection, id);
        client
    }

    fn first_snapshot(client: &UdpSocket, connection: u64) -> Arc<Decoded> {
        let mut bytes = [0; 2048];
        let mut conn = Connection::new();
        loop {
            let (n, _) = client.recv_from(&mut bytes).unwrap();
            let (kind, mut packet) = proto::peek(&bytes[..n]).unwrap();
            if kind != Kind::Snapshot {
                continue;
            }
            assert_eq!(packet.u64().unwrap(), connection);
            conn.receive(&mut packet).unwrap().unwrap();
            let seq = packet.u32().unwrap();
            assert_eq!((packet.u8().unwrap(), packet.u8().unwrap()), (0, 1));
            let snapshot = snapshot::Store::default().decode(packet.rest()).unwrap();
            assert_eq!(snapshot.header.seq, seq);
            return snapshot;
        }
    }

    fn lane(level: i8) -> LaneBuild {
        let (start, end) = ([64.0, 0.0], [64.0, 100.0]);
        LaneBuild {
            key: format!("test-{level}"),
            segment: format!("test-{level}"),
            street: "fixture".into(),
            highway: "residential".into(),
            role: LaneRole::Through,
            painted_through_mapped: false,
            width: 3.65,
            points: vec![start, end],
            start_node: start,
            end_node: end,
            max_speed: 14.0,
            control: ControlKind::None,
            outgoing: Some(vec![]),
            legal: None,
            tile: None,
            level,
        }
    }

    #[test]
    fn one_traffic_impact_corrects_the_driver_and_reaches_both_clients_on_the_right_floor() {
        impact_reaches_both_clients(None);
    }

    #[test]
    fn authoritative_impact_uses_the_drawn_model_chassis_and_wheels() {
        let pools = ["sedan", "taxi", "truck", "bus"].map(|kind| serde_json::json!({
            "kind": kind, "catalogue_models": ["compactHatch"]
        }));
        let profile = serde_json::json!({
            "avatar_height_m": 1.75,
            "vehicles": [{
                "model": "compactHatch",
                "scale": 1.0,
                "physical_chassis": {
                    "min_xyz_m": [-1.0, 0.19, -2.0],
                    "max_xyz_m": [1.0, 1.42, 2.0],
                    "wheels_xz_m": [[-0.84, 1.3], [0.84, 1.3], [-0.84, -1.3], [0.84, -1.3]],
                    "wheel_radius_m": 0.315
                }
            }],
            "simulation_footprints": pools
        });
        let report =
            eo_sim::vehicle_catalogue::ScaleReport::read(profile.to_string().as_bytes()).unwrap();
        impact_reaches_both_clients(Some(Arc::new(report)));
    }

    fn impact_reaches_both_clients(report: Option<Arc<eo_sim::vehicle_catalogue::ScaleReport>>) {
        let (base, source) = crate::load(false).unwrap();
        let measured = report.is_some();
        let source = match report {
            Some(report) => Arc::new(
                Arc::try_unwrap(source)
                    .ok()
                    .unwrap()
                    .with_model_footprints(report),
            ),
            None => source,
        };
        let game = Arc::new(GameWorld::from_parts(
            base.city.clone(),
            base.elevation.clone(),
            Arc::new(Flat),
            Arc::new(BuildingSet::empty()),
            false,
            None,
        ));
        let mut server = Server::start(
            ServerConfig {
                bind: "127.0.0.1:0".into(),
                player_threads: 1,
                sim_threads: 1,
                tile_threads: 1,
                traffic_contact: true,
                ..Default::default()
            },
            game,
            source,
        )
        .unwrap();
        let driver = join(&mut server, 10, [64.4, 42.0, 0.0]);
        let witness = join(&mut server, 11, [80.0, 45.0, 0.0]);

        let net = &mut server.regions.world.net;
        net.add(NetworkBuild {
            tiles: vec![eo_sim::network::TileKey::of(64.0, 42.0)],
            lanes: vec![lane(0), lane(1)],
            lane_surfaces: vec![
                (0, vec![[0.0, 0.0], [100.0, 0.0]]),
                (1, vec![[0.0, 8.0], [100.0, 8.0]]),
            ],
            ..Default::default()
        });
        let viewers: Vec<_> = server
            .sessions
            .iter()
            .map(|s| Viewer {
                player: s.player_id,
                x: s.pos.x,
                z: s.pos.z,
                yaw: 0.0,
            })
            .collect();
        server.regions.update(&viewers);
        let cell = server
            .regions
            .find(RegionKey::of(server.regions.zone, 64.0, 42.0))
            .unwrap();
        let world = &mut server.regions.world;
        for region in &mut world.regions {
            region.cfg.local_life = false;
            region.cfg.people = People::None;
            region.cfg.lod = false;
        }
        let region = &mut world.regions[cell];
        let net = &world.net;
        let s = &mut server.sessions[0];
        let car = &s.player.vehicles[0];
        let player_body = car.body;
        let player_length = car.spec.length;
        let player_z = car.datum(&s.cw).z;
        let mut rng = Mulberry32::new(1);
        for i in 0..2 {
            assert!(region.fleet.spawn(net, &[i], &mut rng, 1));
            let npc = &mut region.fleet.cars[i as usize];
            npc.arc = player_z + (player_length + npc.len) / 2.0 + 0.02;
            npc.speed = 0.0;
            region.fleet.write_pose(net, i as usize);
        }
        let ground = region.fleet.cars[0].id;
        let upper = region.fleet.cars[1].id;
        let start_z = region.fleet.z[0];
        let target = route_target(&server.game, net, &region.fleet, 0);
        eprintln!("traffic model {:?}, chassis {:?}", target.model, target.pose.chassis);
        if measured {
            assert_eq!(target.model, TrafficModel::Road(VehicleModel::Hatchback));
            assert!(matches!(target.pose.chassis, TrafficChassis::Mesh { .. }));
        }
        s.cw.physics.bodies[player_body].set_linvel([0.0, 0.0, 10.0].into(), true);
        assert!(
            target.approaching(car, &s.cw),
            "player={:?} {:?}, target={:?} at {:?}",
            car.datum(&s.cw),
            car.pose(&s.cw),
            target.pose.datum,
            target.pose.yaw
        );

        assert!(server.tick().snapshot_tick);
        let contacts = server.npc_contacts.as_ref().unwrap();
        if measured {
            let spec = &contacts.bodies[&ground].driver.car.spec;
            assert!((spec.height - 1.42).abs() < 1e-6);
            assert!((spec.wheelbase - 2.6).abs() < 1e-6);
            assert!((spec.wheel_radius - 0.315).abs() < 1e-6);
            assert!((spec.track_front - 1.68).abs() < 1e-6);
        }
        assert_eq!(
            server.sessions[0].stats.traffic_contact_ticks,
            1,
            "pose={:?}",
            contacts.pose(ground)
        );
        assert_eq!(server.sessions[1].stats.traffic_contact_ticks, 0);
        let driver_speed = server.sessions[0].cw.physics.bodies[player_body].linvel().z;
        let (datum, rotation, speed) = contacts.pose(ground).expect("ground-level car was hit");
        assert!(
            driver_speed < 9.5 && speed > 0.5 && datum.z > start_z,
            "impact: driver={driver_speed} m/s, traffic={speed} m/s, displacement={} m",
            datum.z - start_z
        );
        eprintln!(
            "driver slowed from 10 m/s to {driver_speed:.3} m/s, NPC reached {speed:.3} m/s and moved {:.3} m",
            datum.z - start_z
        );
        assert!(
            contacts.pose(upper).is_none(),
            "traffic on the upper deck was untouched"
        );

        let a = first_snapshot(&driver, 10);
        let b = first_snapshot(&witness, 11);
        let state = |snapshot: &Decoded, id| -> QState {
            snapshot
                .entities
                .iter()
                .find(|(key, _)| *key == snapshot::net_id(snapshot::CLASS_CAR, id))
                .unwrap()
                .1
        };
        assert_eq!(a.header.tick, b.header.tick);
        assert_eq!(state(&a, ground), state(&b, ground));
        let published = state(&a, ground);
        assert_ne!(published.flags & snapshot::TRAFFIC_PHYSICAL, 0);
        assert_eq!(
            published.pos(),
            datum
                .to_array()
                .map(|v| f64::from(snapshot::q_pos(v)) * snapshot::POS_UNIT)
        );
        let (yaw, pitch, roll) = crate::ypr(rotation);
        assert_eq!(published.yaw, snapshot::q_angle(yaw));
        assert_eq!(
            published.tilt,
            u32::from(snapshot::q_angle(pitch)) | u32::from(snapshot::q_angle(roll)) << 16
        );
        assert_eq!(published.speed, snapshot::q_speed(speed));
        assert_eq!(state(&a, upper).pos()[1], 8.0);
        assert_eq!(state(&b, upper).flags & snapshot::TRAFFIC_PHYSICAL, 0);
        let correction = a.header.correction.as_ref().unwrap();
        assert_eq!(
            read_player_state(&correction.state).unwrap().vehicles[0].linvel[2],
            driver_speed
        );
        assert!(b.header.correction.is_none());
    }
}
