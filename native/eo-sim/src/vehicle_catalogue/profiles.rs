//! Cooked model bounds and the weighted catalogue from the headless scale report.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Report {
    avatar_height_m: f64,
    avatar: Option<Avatar>,
    vehicles: Vec<Vehicle>,
    simulation_footprints: Vec<Pool>,
}

#[derive(Deserialize)]
struct Avatar {
    model: String,
}

#[derive(Deserialize)]
struct Pool {
    kind: String,
    catalogue_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Vehicle {
    pub model: String,
    pub chassis_contact: Option<Bounds>,
    pub physical_chassis: Option<ChassisGeometry>,
    scale: Option<f32>,
}

impl Vehicle {
    /// A rectangle centred on the driving datum that contains the measured model.
    pub fn driving_dimensions(&self) -> Option<(f64, f64)> {
        let bounds = self.chassis_contact?;
        Some((bounds.extent(1), bounds.extent(0)))
    }
}

/// Cooked chassis and wheel geometry, in scaled metres from the road datum.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct ChassisGeometry {
    pub min_xyz_m: [f32; 3],
    pub max_xyz_m: [f32; 3],
    pub wheels_xz_m: [[f32; 2]; 4],
    pub wheel_radius_m: f32,
}

impl ChassisGeometry {
    pub fn half_wheelbase_m(self) -> f64 {
        f64::from((self.wheels_xz_m[0][1] - self.wheels_xz_m[2][1]).abs()) / 2.0
    }

    pub fn valid(self) -> bool {
        let [fl, fr, rl, rr] = self.wheels_xz_m;
        (0..3).all(|i| {
            let size = self.max_xyz_m[i] - self.min_xyz_m[i];
            size.is_finite() && size > 0.0
        }) && self.wheels_xz_m.iter().flatten().all(|v| v.is_finite())
            && fl[0] != fr[0]
            && rl[0] != rr[0]
            && fl[1] + fr[1] > rl[1] + rr[1]
            && self.half_wheelbase_m().is_finite()
            && self.half_wheelbase_m() > 0.0
            && self.wheel_radius_m.is_finite()
            && self.wheel_radius_m > 0.0
    }

    pub fn footprint(self) -> Bounds {
        Bounds {
            min_xz_m: [self.min_xyz_m[0], self.min_xyz_m[2]].map(f64::from),
            max_xz_m: [self.max_xyz_m[0], self.max_xyz_m[2]].map(f64::from),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Bounds {
    pub min_xz_m: [f64; 2],
    pub max_xz_m: [f64; 2],
}

impl Bounds {
    fn valid(self) -> bool {
        (0..2).all(|i| {
            let size = self.max_xz_m[i] - self.min_xz_m[i];
            size.is_finite() && size > 0.0 && self.extent(i).is_finite()
        })
    }

    fn extent(self, axis: usize) -> f64 {
        2.0 * self.min_xz_m[axis].abs().max(self.max_xz_m[axis].abs())
    }

    pub fn at(self, x: f64, z: f64, yaw: f64) -> (f64, f64, f64, f64, f64) {
        let [left, back] = self.min_xz_m;
        let [right, front] = self.max_xz_m;
        let [side, along] = [left.midpoint(right), back.midpoint(front)];
        let (sin, cos) = yaw.sin_cos();
        (
            x + cos * side + sin * along,
            z - sin * side + cos * along,
            yaw,
            front - back,
            right - left,
        )
    }
}

#[derive(Debug)]
pub struct ScaleReport {
    pub avatar_height_m: f64,
    pub avatar_model: Option<String>,
    vehicles: HashMap<String, Vehicle>,
    pools: HashMap<String, Vec<String>>,
}

impl ScaleReport {
    /// The runtime loads the same instance scale for the bodies it draws.
    pub fn require_vehicle_scale(&self, scale: f32) -> Result<()> {
        for vehicle in self
            .vehicles
            .values()
            .filter(|v| v.chassis_contact.is_some())
        {
            ensure!(
                vehicle.scale == Some(scale),
                "model footprint for {} was measured at {:?}, but the game uses {scale}; regenerate the scale report",
                vehicle.model,
                vehicle.scale
            );
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let file =
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::read(std::io::BufReader::new(file))
            .with_context(|| format!("reading scale report {}", path.display()))
    }

    pub fn read(reader: impl std::io::Read) -> Result<Self> {
        let report: Report = serde_json::from_reader(reader)?;
        ensure!(
            report.avatar_height_m.is_finite() && report.avatar_height_m > 0.0,
            "invalid standing avatar height"
        );
        let mut vehicles = HashMap::new();
        for mut v in report.vehicles {
            if let Some(chassis) = v.physical_chassis {
                ensure!(chassis.valid(), "invalid physical chassis for {}", v.model);
                let footprint = chassis.footprint();
                ensure!(
                    v.chassis_contact.is_none_or(|bounds| bounds.valid()
                        && bounds.min_xz_m.map(|v| v as f32)
                            == footprint.min_xz_m.map(|v| v as f32)
                        && bounds.max_xz_m.map(|v| v as f32)
                            == footprint.max_xz_m.map(|v| v as f32)),
                    "physical chassis does not match the measured footprint for {}",
                    v.model
                );
                v.chassis_contact = Some(footprint);
            }
            ensure!(
                v.chassis_contact.is_none_or(Bounds::valid),
                "invalid chassis bounds for {}",
                v.model
            );
            let key = v.model.clone();
            ensure!(
                vehicles.insert(key, v).is_none(),
                "duplicate vehicle in scale report"
            );
        }
        let mut pools = HashMap::new();
        for pool in report.simulation_footprints {
            ensure!(
                pools.insert(pool.kind, pool.catalogue_models).is_none(),
                "duplicate class in scale report"
            );
        }
        for kind in 0..4 {
            let name = super::kind_name(kind);
            ensure!(
                pools.contains_key(name),
                "scale report is missing the {name} catalogue"
            );
        }
        Ok(Self {
            avatar_height_m: report.avatar_height_m,
            avatar_model: report.avatar.map(|avatar| avatar.model),
            vehicles,
            pools,
        })
    }

    pub fn physical_chassis(&self) -> impl Iterator<Item = (&str, ChassisGeometry)> {
        self.vehicles
            .values()
            .filter_map(|v| Some((v.model.as_str(), v.physical_chassis?)))
    }

    pub fn driving_dimensions(&self, seed: u32, id: u32, kind: u8) -> Option<(f64, f64)> {
        self.vehicle(seed, id, kind)?.driving_dimensions()
    }

    pub fn vehicle(&self, seed: u32, id: u32, kind: u8) -> Option<&Vehicle> {
        let kind = u16::from(kind);
        let pool = &self.pools[super::kind_name(kind)];
        super::resolve(seed, id, kind, pool, |name| self.vehicles.get(name)).map(|(_, v)| v)
    }
}
