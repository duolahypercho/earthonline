//! Re-run the fixed-step inputs against a fresh copy of the street route.
//! Millimetres make the first divergent step readable in a saved trace.
use anyhow::{Result, ensure};
use eo_player::world::TileKey;
use eo_player::{CollisionWorld, Mode, Player, StepInput, WeatherPhysics};
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub(super) struct Trace {
    version: u32,
    pub route: String,
    #[serde(default)]
    pub layout: Option<Layout>,
    #[serde(default)]
    pub vehicle: eo_player::vehicle::VehicleConfig,
    pub frames: Vec<Frame>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Layout {
    scale: u32,
    streets: bool,
    #[serde(default)]
    pavement_repairs: Option<bool>,
    structures: u8,
    waterfront: bool,
    access: bool,
    house: bool,
    marine: bool,
}
impl Layout {
    pub fn of(world: &crate::World, args: &crate::Args) -> Self {
        Self {
            scale: eo_net::proto::ground_scale_ppm(),
            streets: world.street.is_some(),
            pavement_repairs: world.street.as_ref().map(|s| s.world.o.pavement_repairs),
            structures: eo_net::proto::structures_code(crate::structures::options(world)),
            waterfront: !world.footway_decks.is_empty(),
            access: args.landmark_access,
            house: world.house.is_some(),
            marine: world.marine_floor.is_some(),
        }
    }

    pub fn matches_recording(mut self, recorded: Self) -> bool {
        // Older traces recorded the street switch alone.
        if recorded.pavement_repairs.is_none() {
            self.pavement_repairs = None;
        }
        self == recorded
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct Frame {
    pub input: StepInput,
    pub yaw: f64,
    pub weather: WeatherPhysics,
    pub contact: Contact,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Contact {
    tick: u64,
    position_mm: [i64; 3],
    velocity_mm_s: [i64; 3],
    surface_below_mm: i64,
    tile: [i32; 2],
    resident: bool,
    grounded: bool,
    wheels: Vec<Wheel>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Wheel {
    hit: bool,
    contact_mm: Option<[i64; 3]>,
    normal: Option<[f32; 3]>,
    spring_length_mm: i64,
    spring_force_n: i64,
    forward_impulse_mns: i64,
    side_impulse_mns: i64,
}

fn mm(x: f64) -> i64 {
    (x * 1000.0).round() as i64
}

impl Contact {
    pub fn sample(player: &Player, world: &CollisionWorld, last: DVec3) -> Self {
        let pos = player.position(world);
        let tile = TileKey::of(pos.x, pos.z);
        let mut velocity = (pos - last) / eo_player::tuning::FIXED_STEP_S;
        let mut wheels = Vec::new();
        let grounded = match player.mode {
            Mode::OnFoot => player.character.grounded,
            Mode::Driving(i) => {
                let car = &player.vehicles[i];
                velocity = DVec3::from_array(
                    world.physics.bodies[car.body]
                        .linvel()
                        .to_array()
                        .map(f64::from),
                );
                for w in car.controller.wheels() {
                    let hit = w.raycast_info();
                    wheels.push(Wheel {
                        hit: hit.is_in_contact,
                        contact_mm: hit
                            .is_in_contact
                            .then(|| hit.contact_point_ws.as_dvec3().to_array().map(mm)),
                        normal: hit.is_in_contact.then(|| hit.contact_normal_ws.to_array()),
                        spring_length_mm: mm(hit.suspension_length.into()),
                        spring_force_n: w.wheel_suspension_force.min(w.max_suspension_force).round()
                            as i64,
                        forward_impulse_mns: mm(w.forward_impulse.into()),
                        side_impulse_mns: mm(w.side_impulse.into()),
                    });
                }
                car.wheels_in_contact() > 0
            }
        };
        Self {
            tick: player.ticks,
            position_mm: pos.to_array().map(mm),
            velocity_mm_s: velocity.to_array().map(mm),
            surface_below_mm: mm(world.surface_below(pos.x, pos.z, pos.y + 0.25)),
            tile: [tile.x, tile.z],
            resident: world.is_resident(pos.x, pos.z),
            grounded,
            wheels,
        }
    }
}

impl Trace {
    pub fn new(route: &str, vehicle: eo_player::vehicle::VehicleConfig, layout: Layout) -> Self {
        Self {
            version: 2,
            layout: Some(layout),
            vehicle,
            route: route.to_owned(),
            frames: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let trace: Self =
            serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(path)?))?;
        ensure!(
            matches!(trace.version, 1 | 2)
                && (trace.version == 1 || trace.layout.is_some())
                && matches!(trace.route.as_str(), "drive_player" | "walk_player")
                && !trace.frames.is_empty(),
            "unsupported physics trace {}",
            path.display()
        );
        trace.vehicle.validated()?;
        Ok(trace)
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.trace.json", self.route));
        let mut writer = std::io::BufWriter::new(std::fs::File::create(&path)?);
        serde_json::to_writer(&mut writer, self)?;
        writer.flush()?;
        eprintln!(
            "physics trace: {} steps in {}",
            self.frames.len(),
            path.display()
        );
        Ok(())
    }
}

/// World switches that select physical floors, barriers and water.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct LocationWorld {
    streets: bool,
    #[serde(default)]
    pavement_repairs: Option<bool>,
    structures_with_tunnels: Option<bool>,
    landmarks: bool,
    landmarks2: bool,
    access: bool,
    floating_docks: bool,
    marine: bool,
    water: bool,
    house: bool,
    finishes: bool,
    patina: bool,
    furniture: bool,
    photo: bool,
    bedroom: bool,
    // Older saves leave the launch setting in effect for these physical variants.
    #[serde(default)]
    fitted_dining: Option<bool>,
    #[serde(default)]
    fired_stoop: Option<bool>,
}
impl LocationWorld {
    pub fn of(a: &crate::Args, street: Option<&eo_render::street::StreetSet>) -> Self {
        Self {
            streets: a.streets,
            pavement_repairs: street.map(|s| s.world.o.pavement_repairs),
            structures_with_tunnels: a.structures.map(|o| o.tunnels),
            landmarks: a.landmarks,
            landmarks2: a.landmarks2,
            access: a.landmark_access,
            floating_docks: a.floating_docks,
            marine: a.look.ocean_life,
            water: a.look.water,
            house: a.beaver_house,
            finishes: a.house_finishes,
            patina: a.house_patina,
            furniture: a.house_furniture,
            photo: a.house_photo,
            bedroom: a.house_blender_bedroom,
            fitted_dining: Some(a.house_fitted_dining),
            fired_stoop: Some(a.house_fired_stoop),
        }
    }

    fn apply(&self, a: &mut crate::Args) {
        a.streets = self.streets;
        if let Some(enabled) = self.pavement_repairs {
            eo_street::options::set_pavement_repairs(enabled);
        }
        a.structures = self
            .structures_with_tunnels
            .map(|tunnels| eo_structures::StructureOptions { tunnels });
        a.landmarks = self.landmarks;
        a.landmarks2 = self.landmarks2;
        a.landmark_access = self.access;
        a.floating_docks = self.floating_docks;
        a.look.ocean_life = self.marine;
        a.look.water = self.water;
        a.beaver_house = self.house;
        a.house_finishes = self.finishes;
        a.house_patina = self.patina;
        a.house_furniture = self.furniture;
        a.house_photo = self.photo;
        a.house_blender_bedroom = self.bedroom;
        if let Some(fitted) = self.fitted_dining {
            a.house_fitted_dining = fitted;
        }
        if let Some(stoop) = self.fired_stoop {
            a.house_fired_stoop = stoop;
        }
    }
}

/// The live source of road weather so the following fixed steps continue
/// the same front even when the checkpoint is opened with another preset.
#[derive(Serialize, Deserialize)]
struct LocationWeather {
    mode: String,
    seed: u32,
    world_time_s: f64,
    effects: bool,
}

impl From<eo_render::sky::SkyOptions> for LocationWeather {
    fn from(sky: eo_render::sky::SkyOptions) -> Self {
        use eo_render::sky::WeatherMode;
        let mode = match sky.weather {
            WeatherMode::Preset(kind) => kind.name(),
            WeatherMode::Schedule => "schedule",
        };
        Self {
            mode: mode.to_owned(),
            seed: sky.seed,
            world_time_s: sky.world_time_s,
            effects: sky.weather_fx,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct Location {
    version: u32,
    plan: u32,
    #[serde(default)]
    world: Option<LocationWorld>,
    #[serde(default)]
    weather_source: Option<LocationWeather>,
    nearest_named_street: Option<String>,
    position_m: [f64; 3],
    cars: Vec<[f32; 3]>,
    state: Vec<u8>,
    contact: Contact,
    weather: WeatherPhysics,
}

impl Location {
    pub fn capture(
        player: &Player,
        world: &CollisionWorld,
        city: &eo_data::City,
        options: LocationWorld,
        sky: eo_render::sky::SkyOptions,
        dir: &Path,
    ) -> Result<(std::path::PathBuf, Option<String>)> {
        let at = player.position(world);
        let mut nearest = None;
        let mut best = f64::INFINITY;
        let here = glam::DVec2::new(at.x, at.z);
        for road in city.roads.iter().filter(|r| !r.name.trim().is_empty()) {
            for segment in road.points64.windows(2) {
                let a = glam::DVec2::from_array(segment[0]);
                let ab = glam::DVec2::from_array(segment[1]) - a;
                let t =
                    ((here - a).dot(ab) / ab.length_squared().max(f64::EPSILON)).clamp(0.0, 1.0);
                let dist = here.distance_squared(a + t * ab);
                if dist < best {
                    best = dist;
                    nearest = Some(road.name.as_str());
                }
            }
        }
        ensure!(
            player.vehicles.len() <= u8::MAX as usize,
            "too many cars for a player checkpoint"
        );
        let cars = player
            .vehicles
            .iter()
            .map(|v| {
                let p = v.datum(world);
                [p.x as f32, p.z as f32, v.yaw(world) as f32]
            })
            .collect();
        let mut state = Vec::new();
        eo_net::wire::write_player_state(&player.state(world), &mut state);
        let velocity = &player.momentum;
        let prior = at
            - DVec3::new(velocity.vx, player.character.vertical_velocity, velocity.vz)
                * eo_player::tuning::FIXED_STEP_S;
        let location = Self {
            version: 2,
            world: Some(options),
            weather_source: Some(sky.into()),
            plan: eo_net::proto::ground_scale_ppm(),
            nearest_named_street: nearest.map(str::to_owned),
            position_m: at.to_array(),
            cars,
            state,
            contact: Contact::sample(player, world, prior),
            weather: player.weather,
        };
        std::fs::create_dir_all(dir)?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = dir.join(format!("physics-location-{timestamp}.json"));
        let mut writer = std::io::BufWriter::new(std::fs::File::create_new(&path)?);
        serde_json::to_writer_pretty(&mut writer, &location)?;
        std::io::Write::flush(&mut writer)?;
        eprintln!(
            "physics location near {} at {at:.2} saved to {}",
            nearest.unwrap_or("unnamed road"),
            path.display()
        );
        Ok((path, location.nearest_named_street))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let loc: Self =
            serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(path)?))?;
        ensure!(
            (loc.version == 1 || (loc.version == 2 && loc.world.is_some()))
                && loc.plan == eo_net::proto::ground_scale_ppm(),
            "incompatible physics location"
        );
        Ok(loc)
    }

    pub fn apply_world(&self, args: &mut crate::Args) -> Result<()> {
        if let Some(options) = &self.world {
            options.apply(args);
        }
        if let Some(source) = &self.weather_source {
            use eo_render::sky::WeatherMode;
            ensure!(
                source.world_time_s.is_finite(),
                "invalid checkpoint weather time"
            );
            args.look.sky.weather =
                if source.mode == "schedule" {
                    WeatherMode::Schedule
                } else {
                    WeatherMode::Preset(eo_sky::Kind::parse(&source.mode).ok_or_else(|| {
                        anyhow::anyhow!("unknown checkpoint weather {}", source.mode)
                    })?)
                };
            args.look.sky.seed = source.seed;
            args.look.sky.world_time_s = source.world_time_s;
            args.look.sky.weather_fx = source.effects;
        }
        Ok(())
    }

    pub fn restore(&self, world: &mut CollisionWorld) -> Result<Player> {
        let loc = self;
        let state = eo_net::wire::read_player_state(&loc.state)
            .map_err(|_| anyhow::anyhow!("invalid player checkpoint"))?;
        ensure!(
            loc.cars.len() == state.vehicles.len()
                && loc.cars.iter().flatten().all(|v| v.is_finite())
                && state.character.position.iter().all(|v| v.is_finite()),
            "invalid location coordinates"
        );
        let mut player = eo_net::game::rebuild_player(world, &state, &loc.cars);
        player.set_weather(loc.weather);
        Ok(player)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_checks_the_recorded_pavement_geometry() {
        let legacy: Layout = serde_json::from_value(serde_json::json!({
            "scale": 1250000, "streets": true, "structures": 0,
            "waterfront": false, "access": false, "house": false, "marine": false
        }))
        .unwrap();
        let repaired = Layout {
            pavement_repairs: Some(true),
            ..legacy
        };
        let original = Layout {
            pavement_repairs: Some(false),
            ..legacy
        };
        assert!(repaired.matches_recording(legacy) && original.matches_recording(legacy));
        assert!(original.matches_recording(original) && repaired.matches_recording(repaired));
        assert!(!original.matches_recording(repaired) && !repaired.matches_recording(original));
        let bridge = Layout {
            structures: 1,
            ..repaired
        };
        assert!(!bridge.matches_recording(legacy));
    }

    #[test]
    fn a_location_reopens_on_the_same_street_with_the_same_car_and_load() {
        use eo_player::vehicle::{VehicleConfig, VehicleModel};
        use eo_player::{BuildingSet, GroundProvider};
        use std::sync::Arc;
        struct Flat;
        impl GroundProvider for Flat {
            fn ground_y(&self, _: f64, _: f64) -> f64 {
                0.0
            }
        }
        let city = eo_data::City::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
        let world = || CollisionWorld::new(Arc::new(Flat), Arc::new(BuildingSet::empty()), 1);
        let mut first = world();
        let road = city
            .roads
            .iter()
            .find(|r| r.name == "Folsom Street" && r.points64.len() >= 2)
            .unwrap();
        let [a, b] = [road.points64[0], road.points64[1]];
        let at = DVec3::new((a[0] + b[0]) / 2.0, 0.0, (a[1] + b[1]) / 2.0);
        first.load_blocking(at, 100.0);
        let mut driver = Player::new(at, 0.0);
        driver.spawn_config = VehicleConfig {
            model: VehicleModel::DeliveryVan,
            occupants_kg: 80,
            cargo_kg: 600,
        };
        let i = driver.spawn_vehicle(&mut first, at.x, at.z, 0.0);
        driver.enter_vehicle(i);
        for _ in 0..120 {
            driver.fixed_step(&mut first, &Default::default());
        }
        driver.fixed_step(&mut first, &eo_player::StepInput { shift_direction: true, ..Default::default() });
        assert!(driver.vehicles[i].reverse_selected());
        let dir = std::env::temp_dir();
        let mut args = super::super::base_args();
        args.structures = Some(eo_structures::StructureOptions { tunnels: true });
        args.beaver_house = true;
        args.house_fitted_dining = true;
        args.house_fired_stoop = true;
        let live_sky = eo_render::sky::SkyOptions {
            weather: eo_render::sky::WeatherMode::Schedule,
            world_time_s: 21000.55,
            seed: 123,
            weather_fx: true,
            ..args.look.sky
        };
        let at = |sky: &eo_render::sky::SkyOptions| {
            eo_net::weather::physical_conditions(&eo_render::sky::weather_of(
                sky,
                sky.world_time_s.floor(),
            ))
        };
        driver.set_weather(at(&live_sky));
        let (path, street) =
            Location::capture(&driver, &first, &city, LocationWorld::of(&args, None), live_sky, &dir).unwrap();
        assert_eq!(street.as_deref(), Some("Folsom Street"));
        let saved: Location = serde_json::from_reader(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(saved.nearest_named_street.as_deref(), Some("Folsom Street"));
        let mut second = world();
        let checkpoint = Location::load(&path).unwrap();
        let mut options = super::super::base_args();
        checkpoint.apply_world(&mut options).unwrap();
        assert_eq!(options.look.sky, live_sky);
        assert_eq!(at(&options.look.sky), driver.weather);
        assert_eq!(LocationWorld::of(&args, None), LocationWorld::of(&options, None));
        let restored = checkpoint.restore(&mut second).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(driver.state(&first), restored.state(&second));
    }
}
