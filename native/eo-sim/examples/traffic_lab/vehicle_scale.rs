use std::collections::HashMap;

pub use eo_sim::vehicle_catalogue::ScaleReport;
use eo_sim::vehicle_catalogue::Vehicle;
use eo_sim::{Network, geom, vehicles::Fleet};
use serde::Serialize;

type Body = (f64, f64, f64, f64, f64);

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overlap {
    pub ids: [u32; 2],
    pub depth_m: f64,
    pub proxy_depth_m: f64,
    pub time: f64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub steps: usize,
    pub steps_with_overlap: usize,
    pub steps_with_missed_overlap: usize,
    pub missing_footprints: usize,
    pub worst: Option<Overlap>,
}

pub struct Audit<'a> {
    report: &'a ScaleReport,
    seed: u32,
    models: HashMap<(u8, u32), Option<&'a Vehicle>>,
    bodies: Vec<(usize, Body, f64)>,
    pub overlaps: Vec<Overlap>,
    pub summary: Summary,
}

impl<'a> Audit<'a> {
    pub fn new(report: &'a ScaleReport, seed: u32) -> Self {
        Self {
            report,
            seed,
            models: HashMap::new(),
            bodies: Vec::new(),
            overlaps: Vec::new(),
            summary: Summary::default(),
        }
    }

    pub fn vehicle(&self, id: u32, kind: u8) -> Option<&'a Vehicle> {
        self.models.get(&(kind, id)).copied().flatten()
    }

    pub fn observe(&mut self, net: &Network, fleet: &Fleet, time: f64) {
        self.bodies.clear();
        self.overlaps.clear();
        self.summary.steps += 1;
        for (i, car) in fleet.cars.iter().enumerate() {
            let model = self
                .models
                .entry((car.kind, car.id))
                .or_insert_with(|| self.report.vehicle(self.seed, car.id, car.kind));
            if let Some(bounds) = model.and_then(|v| v.chassis_contact) {
                let body = bounds.at(fleet.x[i], fleet.z[i], fleet.yaw[i]);
                self.bodies.push((i, body, body.3.hypot(body.4) / 2.0));
            } else {
                self.summary.missing_footprints += 1;
            }
        }
        let surface = |i: usize| (fleet.level[i], fleet.surface_y[i]);
        let proxy = |i: usize| {
            (
                fleet.x[i],
                fleet.z[i],
                fleet.yaw[i],
                fleet.cars[i].len,
                fleet.cars[i].wid,
            )
        };
        self.bodies
            .sort_unstable_by(|a, b| (a.1.0 - a.2).total_cmp(&(b.1.0 - b.2)).then(a.0.cmp(&b.0)));
        for (a, &(i, body, radius)) in self.bodies.iter().enumerate() {
            for &(j, other, other_radius) in &self.bodies[a + 1..] {
                if other.0 - other_radius >= body.0 + radius {
                    break;
                }
                if (body.0 - other.0).powi(2) + (body.1 - other.1).powi(2)
                    >= (radius + other_radius).powi(2)
                    || !net.shares_surface(surface(i), surface(j))
                {
                    continue;
                }
                let depth_m = geom::obb_penetration(body, other);
                if depth_m > eo_sim::stats::BODY_OVERLAP_MIN_M {
                    let pair = Overlap {
                        ids: [fleet.cars[i].id, fleet.cars[j].id],
                        depth_m,
                        proxy_depth_m: geom::obb_penetration(proxy(i), proxy(j)),
                        time,
                    };
                    if self
                        .summary
                        .worst
                        .is_none_or(|worst| depth_m > worst.depth_m)
                    {
                        self.summary.worst = Some(pair);
                    }
                    self.overlaps.push(pair);
                }
            }
        }
        self.summary.steps_with_overlap += usize::from(!self.overlaps.is_empty());
        self.summary.steps_with_missed_overlap += usize::from(
            self.overlaps
                .iter()
                .any(|pair| pair.proxy_depth_m <= eo_sim::stats::BODY_OVERLAP_MIN_M),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use eo_data::sim_source::SimSource;
    use eo_sim::{
        SignalTiming, Standards, StreetDims, native::NativeSource, network::TileKey,
        rng::Mulberry32,
    };

    #[test]
    fn audit_finds_missed_overlap_only_on_the_same_surface() -> Result<()> {
        let paths = eo_data::DataPaths::discover()?;
        let standards = Standards::load(&paths)?;
        let timing = SignalTiming::from_standards(&standards)?;
        let source = NativeSource::new(
            SimSource {
                roads: vec![crate::road(
                    "test",
                    "residential",
                    2,
                    &[[2.0, 2.0], [2.0, 200.0]],
                )],
                signals: vec![],
            },
            StreetDims::from_standards(&standards)?,
            timing.clone(),
        );
        let mut net = Network::new(timing);
        net.add(source.build(&[TileKey { x: 0, z: 0 }]));
        net.surface_clearance_m = Some(2.0);

        let lanes: Vec<_> = net.alive_lanes().collect();
        let mut fleet = Fleet::default();
        let mut rng = Mulberry32::new(1);
        for _ in 0..2 {
            assert!(fleet.spawn(&net, &lanes, &mut rng, 1));
        }
        for car in &mut fleet.cars {
            car.kind = 0;
            car.len = 4.0;
            car.wid = 2.0;
        }
        fleet.x.copy_from_slice(&[0.0, 2.25]);
        fleet.z.fill(0.0);
        fleet.yaw.fill(0.0);
        fleet.level.fill(0);
        fleet.surface_y.fill(12.0);

        let simulation_footprints = ["sedan", "taxi", "truck", "bus"].map(|kind| serde_json::json!({
            "kind": kind, "catalogue_models": ["test-body"],
        }));
        let report = ScaleReport::read(serde_json::json!({
            "avatar_height_m": 1.75,
            "vehicles": [{"model":"test-body", "chassis_contact":{"min_xz_m":[-1.20,-2.0],"max_xz_m":[1.40,2.0]}}],
            "simulation_footprints": simulation_footprints,
        }).to_string().as_bytes())?;
        let mut audit = Audit::new(&report, 1);
        audit.observe(&net, &fleet, 1.0);
        assert_eq!(audit.overlaps.len(), 1);
        let worst = audit.summary.worst.unwrap();
        assert_eq!(worst.ids, [0, 1]);
        assert!((worst.depth_m - 0.35).abs() < 1e-9 && worst.proxy_depth_m == 0.0);

        fleet.surface_y[1] = 18.0;
        audit.observe(&net, &fleet, 2.0);
        assert!(audit.overlaps.is_empty());
        assert_eq!(audit.summary.steps, 2);
        assert_eq!(audit.summary.steps_with_overlap, 1);
        assert_eq!(audit.summary.steps_with_missed_overlap, 1);
        Ok(())
    }
}
