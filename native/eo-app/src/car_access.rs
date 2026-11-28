//! Local driver transfers use the player's fixed ticks for both the body and door.
//!
//! (`--play --car-door-animation`) F (`StepInput::vehicle_toggle`) starts an
//! animated entry or exit when the demo avatar, the sedan's door rig and
//! clear standing room at the driver's door allow it; otherwise the tap goes
//! to the player as before (`Player::try_enter` / `try_exit`). No transfer
//! starts during a landmark lift ride (the ride ignores F as before). Local
//! only: `--connect` takes neither switch, and nothing here is sent.

use eo_agents::library::Library;
use eo_agents::test_characters::{CarSample, PlayerBody};
use eo_player::tuning::FIXED_STEP_S;
use eo_player::vehicle::Vehicle;
use eo_player::world::CollisionWorld;
use eo_player::{Mode, Player, StepInput};
use glam::{DQuat, DVec3};

/// Room to align one short step to the clip's authored standing position.
const ALIGN_REACH_M: f64 = 0.9;
const STATIONARY_MPS: f64 = 0.3;

pub fn sedan_frame(car: &Vehicle, world: &CollisionWorld) -> (DVec3, DQuat) {
    let (_, orientation) = car.pose(world);
    // The source supercar faces -Z. Align its fitted wheel centre to the
    // native sedan's SteeringWheel node at (0.37, 0.92, 0.35).
    let offset = DVec3::new(0.0, 0.24, -0.11) * eo_data::groundscale::vehicles();
    let position = car.datum(world) + orientation * offset;
    (
        position,
        orientation * DQuat::from_rotation_y(std::f64::consts::PI),
    )
}

#[derive(Clone, Copy)]
struct Transfer {
    vehicle: usize,
    clip: usize,
    entering: bool,
    started: u64,
    floor: f64,
    initial_feet: DVec3,
    initial_yaw: DQuat,
}

#[derive(Default)]
pub struct CarAccess {
    active: Option<Transfer>,
}

pub struct Frame {
    pub vehicle: usize,
    pub clip: usize,
    pub time: f32,
    pub door: f32,
    pub position: DVec3,
    pub orientation: DQuat,
    pub entry_blend_key: Option<u64>,
}

impl CarAccess {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn advance(
        &mut self,
        player: &mut Player,
        world: &mut CollisionWorld,
        library: Option<&Library>,
        mut input: StepInput,
        dt: f64,
    ) -> usize {
        let body = library.and_then(|library| library.test_player.as_ref());
        if self.active.is_none() && input.vehicle_toggle && player.lift.is_none() {
            self.active = library.zip(body).and_then(|(library, body)| {
                let vehicle = match player.mode {
                    Mode::OnFoot => player.nearby_vehicle(world)?,
                    Mode::Driving(i) => i,
                };
                let model = library.model(player.vehicles[vehicle].spec.config.model.body())?;
                if library.models[model as usize].driver_door[3] == 0.0 {
                    return None;
                }
                Transfer::start(player, world, body)
            });
        }
        let Some(transfer) = self.active else {
            // no animated transfer: F enters or leaves the car at once, as without the switch.
            return player.advance(world, &input, dt);
        };
        input.vehicle_toggle = false;
        let Some(body) = body else {
            self.active = None;
            return player.advance(world, &input, dt);
        };

        // The capsule and car keep stepping while control is handed over.
        // Equal throttle and brake request full service braking without reverse.
        let hold = StepInput {
            throttle: 1.0,
            brake: 1.0,
            handbrake: true,
            ..Default::default()
        };
        let steps = player.advance(world, &hold, dt);
        let duration = body.rig.seconds(transfer.clip);
        if transfer.elapsed(player.ticks) >= duration {
            if transfer.entering {
                player.finish_entry(world, transfer.vehicle);
            } else {
                let frame = transfer.frame(player, world, body);
                let facing = frame.orientation * DVec3::Z;
                if !player.exit_vehicle_at(
                    world,
                    transfer.vehicle,
                    frame.position,
                    facing.x.atan2(facing.z),
                ) {
                    // the clip's end point is blocked: the door stations as F leaves without the switch.
                    player.try_exit(world);
                }
            }
            self.active = None;
        }
        steps
    }

    pub fn frame(
        &self,
        player: &Player,
        world: &CollisionWorld,
        body: &PlayerBody,
    ) -> Option<Frame> {
        self.active
            .map(|transfer| transfer.frame(player, world, body))
    }
}

impl Transfer {
    fn start(player: &mut Player, world: &CollisionWorld, body: &PlayerBody) -> Option<Self> {
        let entering = player.mode == Mode::OnFoot;
        let vehicle = match player.mode {
            Mode::OnFoot if player.character.grounded => player.nearby_vehicle(world)?,
            Mode::OnFoot => return None,
            Mode::Driving(i) => i,
        };
        let car = &player.vehicles[vehicle];
        if car.forward_speed(world).abs() > STATIONARY_MPS {
            return None;
        }
        let clip = body.rig.car_access_clip(entering)?;
        let duration = body.rig.seconds(clip);
        let standing = body
            .rig
            .car_sample(clip, if entering { 0.0 } else { duration }, entering);
        let (origin, car_basis) = sedan_frame(car, world);
        let (door_station, _) = clip_transform(origin, car_basis, standing);
        // The old interaction range allows any side. This clip needs clear
        // standing space at the driver door and a short approach on that side.
        let footing = player.vehicle_footing(world, vehicle, door_station)?;
        let initial_feet = player.character.feet();
        if entering && initial_feet.distance(footing) > ALIGN_REACH_M {
            return None;
        }
        player.momentum.reset();
        Some(Self {
            vehicle,
            clip,
            entering,
            started: player.ticks,
            floor: footing.y,
            initial_feet,
            initial_yaw: DQuat::from_rotation_y(player.facing_yaw),
        })
    }

    fn elapsed(self, ticks: u64) -> f32 {
        (ticks.saturating_sub(self.started) as f64 * FIXED_STEP_S) as f32
    }

    fn frame(self, player: &Player, world: &CollisionWorld, body: &PlayerBody) -> Frame {
        let time = self.elapsed(player.ticks).min(body.rig.seconds(self.clip));
        let sample = body.rig.car_sample(self.clip, time, self.entering);
        let (origin, car_basis) = sedan_frame(&player.vehicles[self.vehicle], world);
        let (mut position, mut orientation) = clip_transform(origin, car_basis, sample);
        let forward = orientation * DVec3::Z;
        let upright = DQuat::from_rotation_y(forward.x.atan2(forward.z));
        let seat_weight = f64::from(sample.seat_weight);
        position.y = self.floor + (position.y - self.floor) * seat_weight;
        orientation = upright.slerp(orientation, seat_weight);
        if self.entering {
            let align = f64::from((time / body.rig.door_open_start(self.clip)).min(1.0));
            let weight = align * align * (3.0 - 2.0 * align);
            position = self.initial_feet.lerp(position, weight);
            orientation = self.initial_yaw.slerp(orientation, weight);
        }
        Frame {
            vehicle: self.vehicle,
            clip: self.clip,
            time,
            door: sample.door,
            position,
            orientation,
            entry_blend_key: self.entering.then_some(self.started),
        }
    }
}

fn clip_transform(origin: DVec3, car_basis: DQuat, sample: CarSample) -> (DVec3, DQuat) {
    let [x, z, yaw] = sample.root.map(f64::from);
    let position = origin + car_basis * (DVec3::new(x, 0.0, z) * eo_data::groundscale::vehicles());
    let orientation = car_basis * DQuat::from_rotation_y(yaw);
    (position, orientation)
}
