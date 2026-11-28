//! Road conditions at the fixed input step, including a replay of that step.
//! A connection anchors its input sequence to the server's weather time.
//! Time of day and its visual playback rate are independent of this clock.
use crate::bits::Truncated;
use crate::presence::{WEATHER_SCHEDULE, weather_name};
use eo_player::WeatherPhysics;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeatherEpoch {
    mode: u8,
    seed: u32,
    tick_offset: u32,
}

impl WeatherEpoch {
    pub const BYTES: usize = 9;

    pub fn new(mode: u8, seed: u32, server_tick: u32, input_seq: u32) -> Self {
        Self {
            mode,
            seed,
            tick_offset: server_tick.wrapping_sub(input_seq),
        }
    }

    pub fn tick(self, seq: u32) -> u32 {
        self.tick_offset.wrapping_add(seq)
    }

    /// A suspended client can resume after the schedule has advanced.
    pub fn needs_reanchor(self, seq: u32, world_tick: u32) -> bool {
        self.mode == WEATHER_SCHEDULE
            && (world_tick.wrapping_sub(self.tick(seq)) as i32).unsigned_abs()
                > crate::proto::TICK_HZ
    }

    pub fn seconds(self, seq: u32) -> f64 {
        f64::from(self.tick(seq)) / f64::from(crate::proto::TICK_HZ)
    }

    pub(crate) fn bytes(self) -> [u8; Self::BYTES] {
        let mut out = [0; Self::BYTES];
        out[0] = self.mode;
        out[1..5].copy_from_slice(&self.seed.to_le_bytes());
        out[5..].copy_from_slice(&self.tick_offset.to_le_bytes());
        out
    }

    pub(crate) fn read(bytes: &[u8]) -> Result<Self, Truncated> {
        let mut r = crate::bits::ByteReader::new(bytes);
        Ok(Self {
            mode: r.u8()?,
            seed: r.u32()?,
            tick_offset: r.u32()?,
        })
    }
}

pub fn start_tick(seconds: f64) -> anyhow::Result<u32> {
    let ticks = (seconds * f64::from(crate::proto::TICK_HZ)).floor();
    anyhow::ensure!(
        (0.0..=f64::from(u32::MAX)).contains(&ticks),
        "weather start outside the simulation tick range"
    );
    Ok(ticks as u32)
}

pub fn default_seed() -> u32 {
    eo_sky::world_seed()
}

/// The five quantities local and online play pass to the controller.
pub fn physical_conditions(w: &eo_sky::WeatherState) -> WeatherPhysics {
    WeatherPhysics {
        wetness: w.wetness,
        puddle_fill: w.puddle_fill,
        rain_mm_h: w.rain_mm_h,
        // The sky supplies instantaneous wind. The controller applies the gust
        // to its steady wind when computing air-relative velocity.
        wind: w.wind.map(|component| component / w.gust),
        gust: w.gust,
    }
}

#[derive(Default)]
pub struct RoadWeather {
    pub epoch: WeatherEpoch,
    cached: Option<(WeatherEpoch, u32, WeatherPhysics)>,
}

impl RoadWeather {
    pub fn new(epoch: WeatherEpoch) -> Self {
        Self {
            epoch,
            cached: None,
        }
    }

    /// Like the local renderer, integrate the scheduled water film once per
    /// world second. Sampling an earlier second during replay is supported.
    pub fn at(&mut self, seq: u32) -> WeatherPhysics {
        let epoch = self.epoch;
        let second = if epoch.mode == WEATHER_SCHEDULE {
            epoch.tick(seq) / crate::proto::TICK_HZ
        } else {
            0
        };
        if let Some((at, sec, value)) = self.cached {
            if at == epoch && second == sec {
                return value;
            }
        }
        let weather = if epoch.mode == WEATHER_SCHEDULE {
            eo_sky::weather::state_at(epoch.seed, f64::from(second))
        } else {
            eo_sky::weather::preset(
                eo_sky::Kind::parse(weather_name(epoch.mode)).unwrap_or(eo_sky::Kind::Clear),
            )
        };
        let value = physical_conditions(&weather);
        self.cached = Some((epoch, second, value));
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::step_player;
    use crate::predict::{Predictor, STEP_S};
    use crate::presence::weather_code;
    use eo_player::{BuildingSet, CollisionWorld, GroundProvider, Player, StepInput};
    use glam::DVec3;
    use std::sync::Arc;

    #[test]
    fn replay_samples_the_same_seed_and_second_as_local_play() {
        let seq = u32::MAX - 80;
        let epoch = WeatherEpoch::new(
            WEATHER_SCHEDULE,
            123,
            21000 * crate::proto::TICK_HZ + 42,
            seq,
        );
        let mut road = RoadWeather::new(WeatherEpoch::read(&epoch.bytes()).unwrap());
        let mut winds = vec![];
        for offset in [0, 15, 22, 105, 307, 0, 105] {
            let input = seq.wrapping_add(offset);
            let t = 21000 + (42 + offset) / crate::proto::TICK_HZ;
            let sky = eo_sky::weather::state_at(123, f64::from(t));
            let local = physical_conditions(&sky);
            let wind = road.at(input).effective_wind();
            assert!(
                wind.into_iter()
                    .zip(sky.wind)
                    .all(|(actual, expected)| (actual - expected).abs() < 1e-12)
            );
            assert_eq!(road.at(input), local);
            winds.push(wind);
        }
        assert!(winds.windows(2).any(|w| w[0] != w[1]));
    }

    struct Flat;
    impl GroundProvider for Flat {
        fn ground_y(&self, _: f64, _: f64) -> f64 {
            0.0
        }
    }

    fn moving_car() -> (Player, CollisionWorld) {
        let mut world = CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::empty()), 1);
        let spawn = DVec3::new(8.0, 0.0, 120.0);
        world.load_blocking(spawn, eo_player::world::LOAD_RADIUS_M);
        let mut player = Player::new(spawn, 0.0);
        player.spawn_config.model = eo_player::vehicle::VehicleModel::Sedan;
        let car = player.spawn_vehicle(&mut world, spawn.x, spawn.z, 0.);
        player.enter_vehicle(car);
        for _ in 0..180 {
            player.fixed_step(&mut world, &StepInput::default());
        }
        world.physics.bodies[player.vehicles[car].body].set_linvel([0., 0., 15.].into(), true);
        (player, world)
    }

    #[test]
    fn wet_braking_matches_local_play_at_each_predicted_step() {
        let mut distances = vec![];
        for kind in [eo_sky::Kind::Clear, eo_sky::Kind::Storm] {
            let epoch = WeatherEpoch::new(weather_code(kind.name()).unwrap(), 7, 12345, 0);
            let (p, w) = moving_car();
            let start = p.position(&w);
            let (mut local, mut road) = moving_car();
            local.set_weather(physical_conditions(&eo_sky::weather::preset(kind)));
            let mut predicted = Predictor::with_weather(p, w, 0, epoch);
            let mut inputs = vec![];
            let brake = StepInput {
                brake: 1.,
                ..Default::default()
            };
            for _ in 0..300 {
                inputs.clear();
                assert_eq!(predicted.frame(&brake, STEP_S, &mut inputs), 1);
                step_player(&mut local, &mut road, &inputs[0].1);
                assert_eq!(predicted.player.state(&predicted.cw), local.state(&road));
                if local.vehicles[0].forward_speed(&road) < 0.3 {
                    break;
                }
            }
            assert!(local.vehicles[0].forward_speed(&road) < 0.3);
            distances.push(local.position(&road).z - start.z);
        }
        println!(
            "15 m/s local and predicted stop: clear {:.2} m, storm {:.2} m",
            distances[0], distances[1]
        );
        assert!(distances[1] > distances[0] * 1.10);
    }
}
