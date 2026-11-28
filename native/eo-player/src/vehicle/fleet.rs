//! Mechanical references and modelling assumptions are in Docs/VEHICLE-PHYSICS.md.
//! Wheel centres and radii are in the shipped vehicle bodies' local coordinates.
use super::VehicleSpec;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum VehicleModel {
    #[default]
    CatalogueSedan,
    Sedan,
    Hatchback,
    DeliveryVan,
}

impl VehicleModel {
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::CatalogueSedan),
            1 => Some(Self::Sedan),
            2 => Some(Self::Hatchback),
            3 => Some(Self::DeliveryVan),
            _ => None,
        }
    }

    pub fn body(self) -> &'static str {
        match self {
            Self::CatalogueSedan | Self::Sedan => "sedan",
            Self::Hatchback => "compactHatch",
            Self::DeliveryVan => "deliveryVan",
        }
    }

    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "sedan" => Ok(Self::Sedan),
            "hatchback" => Ok(Self::Hatchback),
            "van" => Ok(Self::DeliveryVan),
            "catalogue-sedan" => Ok(Self::CatalogueSedan),
            _ => anyhow::bail!("vehicle must be sedan, hatchback, van, or catalogue-sedan"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VehicleConfig {
    pub model: VehicleModel,
    pub occupants_kg: u16,
    pub cargo_kg: u16,
}

impl VehicleConfig {
    pub fn validated(self) -> Result<Self> {
        let limit = match self.model {
            VehicleModel::CatalogueSedan => 0,
            VehicleModel::Hatchback => 400,
            VehicleModel::Sedan => 450,
            VehicleModel::DeliveryVan => 1500,
        };
        ensure!(
            u32::from(self.occupants_kg) + u32::from(self.cargo_kg) <= limit,
            "combined occupant and cargo mass exceeds the configured {limit} kg payload"
        );
        Ok(self)
    }

    pub fn spec(self) -> VehicleSpec {
        let mut s = VehicleSpec::sedan();
        let reference = match self.model {
            VehicleModel::CatalogueSedan => return s,
            VehicleModel::Sedan => &SEDAN,
            VehicleModel::Hatchback => &HATCH,
            VehicleModel::DeliveryVan => &VAN,
        };
        let r = reference;
        s.config = self;
        s.powertrain = Some(r);
        [s.length, s.width, s.height] = r.body_m;
        [
            s.wheel_radius,
            s.front_axle_z,
            s.rear_axle_z,
            s.track_front,
            s.track_rear,
        ] = r.wheels_m;
        s.wheelbase = s.front_axle_z - s.rear_axle_z;
        s.sill_y = r.clearance_m;
        let curb_mass = r.curb_lb * 0.453_592_37;
        let cargo_mass = f64::from(self.cargo_kg);
        let occupants = f64::from(self.occupants_kg);
        s.curb_mass_kg = curb_mass;
        s.curb_front_share = r.front_share;
        s.front_torque_share = if r.front_drive { 1.0 } else { 0.0 };
        s.mass_kg = curb_mass + cargo_mass + occupants;
        let curb_z = s.rear_axle_z + r.front_share * s.wheelbase;
        // Occupants sit in the cabin. Cargo is centred inside the rear axle.
        let seat_z = 0.20 * s.wheelbase;
        let cargo_z = 0.85 * s.rear_axle_z;
        let cargo_y = r.cargo_height_m;
        let seat_y = 0.85;
        let com_y =
            (curb_mass * r.cg_height_m + cargo_mass * cargo_y + occupants * seat_y) / s.mass_kg;
        s.com_offset_y = com_y - s.height / 2.0;
        s.com_offset_z =
            (curb_mass * curb_z + cargo_mass * cargo_z + occupants * seat_z) / s.mass_kg;
        // Principal inertias of a uniform sprung envelope, translated to the
        // loaded centre with the parallel-axis theorem (loads on the centreline).
        let pitch0 = curb_mass * (s.length.powi(2) + s.height.powi(2)) / 12.0;
        let yaw0 = curb_mass * (s.length.powi(2) + s.width.powi(2)) / 12.0;
        let roll0 = curb_mass * (s.width.powi(2) + s.height.powi(2)) / 12.0;
        let mut dy2 = 0.0;
        let mut dz2 = 0.0;
        for (m, y, z) in [
            (curb_mass, r.cg_height_m, curb_z),
            (occupants, seat_y, seat_z),
            (cargo_mass, cargo_y, cargo_z),
        ] {
            dy2 += m * (y - com_y).powi(2);
            dz2 += m * (z - s.com_offset_z).powi(2);
        }
        s.pitch_inertia = pitch0 + dy2 + dz2;
        s.yaw_inertia = yaw0 + dz2;
        s.roll_inertia = roll0 + dy2;
        s.rolling_resistance_n = 0.013 * s.mass_kg * crate::tuning::GRAVITY_MPS2;
        s.aero_factor = 0.5 * 1.225 * r.drag_area_m2;
        s.engine_brake_force_n = 0.35 * curb_mass;
        s.braking_scale = 1.0;
        s.tyre_friction = r.dry_tire_grip;
        s.engine_power_w = r.power_hp * 745.699872;
        s.max_suspension_force = 3.0 * curb_mass * crate::tuning::GRAVITY_MPS2 / 4.0;
        s
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Powertrain {
    body_m: [f64; 3],
    wheels_m: [f64; 5],
    curb_lb: f64,
    clearance_m: f64,
    cg_height_m: f64,
    cargo_height_m: f64,
    front_share: f64,
    drag_area_m2: f64,
    dry_tire_grip: f64,
    front_drive: bool,
    power_hp: f64,
    torque_lb_ft: f64,
    torque_rpm: f64,
    rated_rpm: f64,
    gears: &'static [f64],
    final_drive: f64,
    reverse: f64,
    pub(super) shift_duration_ticks: u8,
}

impl Powertrain {
    /// Automatic gear selection at the rated speed. Interpolate engine torque
    /// between the launch, peak-torque, and rated-power points. The clutch
    /// slips at launch, then engine speed follows wheel speed and gear ratio.
    pub fn wheel_force(&self, speed: f64, radius: f64, reverse: bool) -> f64 {
        let rpm = self.axle_rpm(speed, radius);
        let gear = self.select_gear(rpm, 0);
        self.force_in_gear(rpm, radius, gear, reverse)
    }

    fn axle_rpm(&self, speed: f64, radius: f64) -> f64 {
        speed.abs() / radius * (60.0 / std::f64::consts::TAU) * self.final_drive
    }

    fn select_gear(&self, rpm: f64, current: u8) -> usize {
        let i = usize::from(current).min(self.gears.len() - 1);
        // Hold the new gear while the clutch is open and the car coasts.
        if rpm * self.gears[i] < self.rated_rpm
            && (i == 0 || rpm * self.gears[i - 1] >= self.rated_rpm * 0.75)
        {
            return i;
        }
        self.gears
            .iter()
            .position(|g| rpm * g < self.rated_rpm)
            .unwrap_or(self.gears.len() - 1)
    }

    pub(super) fn step_force(
        &self,
        speed: f64,
        radius: f64,
        reverse: bool,
        state: &mut Transmission,
    ) -> f64 {
        let rpm = self.axle_rpm(speed, radius);
        let gear = if reverse {
            0
        } else {
            self.select_gear(rpm, state.gear)
        };
        if gear != usize::from(state.gear) {
            state.gear = gear as u8;
            state.shift_left = self.shift_duration_ticks;
        }
        let engagement = if state.shift_left == 0 {
            1.0
        } else {
            state.shift_left -= 1;
            let progress = 1.0 - f64::from(state.shift_left) / f64::from(self.shift_duration_ticks);
            ((progress - 0.65) / 0.35).clamp(0.0, 1.0)
        };
        engagement * self.force_in_gear(rpm, radius, gear, reverse)
    }

    fn force_in_gear(&self, axle_rpm: f64, radius: f64, gear: usize, reverse: bool) -> f64 {
        let gear = if reverse {
            self.reverse
        } else {
            self.gears[gear]
        };
        let rpm = (axle_rpm * gear).max(1800.0);
        let peak = self.torque_lb_ft * 1.35581795;
        let rated = self.power_hp * 745.699872 / (self.rated_rpm * std::f64::consts::TAU / 60.0);
        let torque = if rpm < self.torque_rpm {
            peak * (0.70 + 0.30 * rpm / self.torque_rpm)
        } else {
            let t = ((rpm - self.torque_rpm) / (self.rated_rpm - self.torque_rpm)).min(1.0);
            (peak + t * (rated - peak)) * (1.0 - (rpm - self.rated_rpm).max(0.0) / 350.0).max(0.0)
        };
        torque * gear * self.final_drive * 0.9 / radius
    }
}

/// Selected range, zero-based forward gear and remaining shift time at 60 Hz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Transmission {
    pub reverse_selected: bool,
    pub gear: u8,
    pub shift_left: u8,
}

const HATCH: Powertrain = Powertrain {
    body_m: [4.06, 1.75, 1.48],
    wheels_m: [0.315, 1.17, -1.39, 1.53, 1.52],
    curb_lb: 2536.0,
    clearance_m: 0.18,
    cg_height_m: 0.52,
    cargo_height_m: 0.70,
    front_share: 0.60,
    drag_area_m2: 0.64,
    dry_tire_grip: 0.885,
    front_drive: true,
    power_hp: 120.0,
    torque_lb_ft: 112.0,
    torque_rpm: 5000.0,
    rated_rpm: 6350.0,
    gears: &[3.846, 2.038, 1.28, 0.95, 0.756],
    final_drive: 4.07,
    reverse: 3.615,
    shift_duration_ticks: 40,
};
const SEDAN: Powertrain = Powertrain {
    body_m: [4.85, 1.84, 1.45],
    wheels_m: [0.34, 1.465, -1.355, 1.58, 1.57],
    curb_lb: 3472.0,
    clearance_m: 0.205,
    cg_height_m: 0.54,
    cargo_height_m: 0.72,
    front_share: 0.59,
    drag_area_m2: 0.68,
    dry_tire_grip: 0.95,
    front_drive: true,
    power_hp: 175.0,
    torque_lb_ft: 175.0,
    torque_rpm: 4500.0,
    rated_rpm: 6000.0,
    gears: &[4.58, 2.96, 1.91, 1.45, 1.0, 0.75],
    final_drive: 3.07,
    reverse: 2.94,
    shift_duration_ticks: 12,
};
const VAN: Powertrain = Powertrain {
    body_m: [5.53, 2.06, 2.55],
    wheels_m: [0.375, 1.765, -1.535, 1.74, 1.72],
    curb_lb: 5115.0,
    clearance_m: 0.24,
    cg_height_m: 0.82,
    cargo_height_m: 0.95,
    front_share: 0.54,
    drag_area_m2: 1.58,
    dry_tire_grip: 0.95,
    front_drive: false,
    power_hp: 275.0,
    torque_lb_ft: 262.0,
    torque_rpm: 3750.0,
    rated_rpm: 6500.0,
    gears: &[
        4.696, 2.985, 2.146, 1.769, 1.520, 1.275, 1.0, 0.854, 0.689, 0.636,
    ],
    final_drive: 3.73,
    reverse: 4.866,
    shift_duration_ticks: 12,
};
