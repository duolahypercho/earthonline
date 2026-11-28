//! Compare recorded wheel positions with the roads drawn by the street and structure meshes.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use eo_sim::vehicle_catalogue::ChassisGeometry;
use eo_street::{Layer, StreetWorld};
use eo_structures::{StructureSurface, StructureWorld};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Recording {
    ground_scale: f64,
    traces: Vec<Trace>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Trace {
    name: String,
    seed: u32,
    sample_hz: usize,
    avatar_model: Option<String>,
    avatar_height_m: f64,
    measured_chassis: HashMap<String, ChassisGeometry>,
    lanes: Vec<Lane>,
    frames: Vec<Frame>,
}

#[derive(Deserialize)]
struct Lane {
    id: u32,
    key: String,
    level: i8,
}

#[derive(Deserialize)]
struct Frame {
    time: f64,
    cars: Vec<Car>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Car {
    id: u32,
    model: Option<String>,
    lane: u32,
    corner_from: Option<u32>,
    x: f64,
    z: f64,
    yaw: f64,
    surface_y: Option<f64>,
}

impl Car {
    fn world_points<const N: usize>(&self, local: [[f64; 2]; N]) -> [[f64; 2]; N] {
        let (sin, cos) = self.yaw.sin_cos();
        local.map(|[side, ahead]| {
            [
                self.x + cos * side + sin * ahead,
                self.z - sin * side + cos * ahead,
            ]
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Report {
    ground_scale: f64,
    structures: bool,
    tunnels: bool,
    traces: Vec<TraceReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceReport {
    name: String,
    seed: u32,
    sample_hz: usize,
    avatar_model: Option<String>,
    avatar_height_m: f64,
    sampled_frames: usize,
    measured_vehicle_samples: usize,
    measured_separated_vehicle_samples: usize,
    skipped_separated_vehicle_samples: usize,
    missing_route_height_samples: usize,
    missing_chassis_samples: usize,
    roof_to_avatar_height: BTreeMap<String, f64>,
    wheel_layers: BTreeMap<&'static str, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<BodyReport>,
    lanes: BTreeMap<String, LaneReport>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct LaneReport {
    wheel_layers: BTreeMap<&'static str, usize>,
    first_non_carriageway: Option<WheelSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<BodyReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WheelSample {
    time: f64,
    vehicle_id: u32,
    model: String,
    wheel: &'static str,
    xz: [f64; 2],
    layer: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    route_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closest_road_y: Option<f64>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct BodyReport {
    probe_layers: BTreeMap<&'static str, usize>,
    poses_with_non_carriageway: usize,
    first_non_carriageway: Option<BodySample>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BodySample {
    time: f64,
    vehicle_id: u32,
    model: String,
    offset_xz: [f64; 2],
    xz: [f64; 2],
    layer: &'static str,
}

/// Corners, edge midpoints and center of the measured chassis bounds.
fn body_probes(chassis: ChassisGeometry) -> [[f64; 2]; 9] {
    let bounds = chassis.footprint();
    let axis = |i| {
        let a: f64 = bounds.min_xz_m[i];
        let b = bounds.max_xz_m[i];
        [a, a.midpoint(b), b]
    };
    let [sides, fore_aft] = [axis(0), axis(1)];
    std::array::from_fn(|i| [sides[i / 3], fore_aft[i % 3]])
}

fn on_carriageway(layer: &str) -> bool {
    matches!(
        layer,
        "carriageway" | "structureCarriageway" | "streetHandoff"
    )
}

fn layer_name(layer: Option<Layer>) -> &'static str {
    match layer {
        Some(Layer::Carriageway) => "carriageway",
        Some(Layer::CurbFace) => "curbFace",
        Some(Layer::CurbTop) => "curbTop",
        Some(Layer::Sidewalk) => "sidewalk",
        Some(Layer::Ramp) => "ramp",
        Some(Layer::Verge) => "verge",
        Some(Layer::Marking) => "marking",
        None => "noPaving",
    }
}

pub(super) fn load(path: &Path) -> Result<Recording> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let recording: Recording = serde_json::from_reader(std::io::BufReader::new(file))
        .with_context(|| {
            format!(
                "reading {} (use a Traffic Lab .json replay with --vehicle-scale-report)",
                path.display()
            )
        })?;
    ensure!(
        eo_data::groundscale::scale() == recording.ground_scale,
        "the replay and the street surface use different ground scales"
    );
    ensure!(
        !recording.traces.is_empty(),
        "the replay contains no traces"
    );
    if let Some(trace) = recording
        .traces
        .iter()
        .find(|trace| trace.measured_chassis.is_empty())
    {
        anyhow::bail!("replay {} has no measured wheel geometry", trace.name);
    }
    Ok(recording)
}

pub(super) fn run(
    recording: Recording,
    streets: &StreetWorld,
    structures: Option<&StructureWorld>,
    tunnels: bool,
    include_body: bool,
) -> Result<Report> {
    let road_at_height = structures.map(|world| {
        move |[x, z]: [f64; 2], reference: f64, reach: f64| {
            let structure_height =
                structure_height_near([x, z], reference, |key| world.surface_tile_shared(key));
            route_support(
                streets.carriageway_near(x, z, reference),
                structure_height.into_iter(),
                reference,
                reach,
            )
        }
    });
    let separated = road_at_height
        .as_ref()
        .map(|sample| sample as &SeparatedSupport);
    let traces = recording
        .traces
        .into_iter()
        .map(|trace| {
            summarize(
                trace,
                |[x, z]| streets.paved_layer_at(x, z),
                separated,
                include_body,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Report {
        ground_scale: recording.ground_scale,
        structures: structures.is_some(),
        tunnels,
        traces,
    })
}

fn structure_height_near(
    position: [f64; 2],
    reference: f64,
    mut surface: impl FnMut((i32, i32)) -> Option<Arc<StructureSurface>>,
) -> Option<f64> {
    let (x, z) = eo_street::tile_key(position[0], position[1]);
    let closest = |a: &f64, b: &f64| (a - reference).abs().total_cmp(&(b - reference).abs());
    // Sweeps and landmark panels belong to their centre tile. Their triangles
    // can support a wheel on the far side of a tile boundary.
    (-1..=1)
        .flat_map(|dx| (-1..=1).map(move |dz| (x + dx, z + dz)))
        .filter_map(|key| {
            let tile = surface(key)?;
            road_triangle_heights(&tile.street, position).min_by(closest)
        })
        .min_by(closest)
}

/// Collision triangles identify the driven deck or bore floor independently
/// of the lane polyline. There may be several road heights over one point.
fn road_triangle_heights(
    triangles: &[[[f64; 3]; 3]],
    [x, z]: [f64; 2],
) -> impl Iterator<Item = f64> {
    triangles.iter().filter_map(move |&[a, b, c]| {
        let [dx, dz] = [x - a[0], z - a[2]];
        let [ux, uz] = [b[0] - a[0], b[2] - a[2]];
        let [vx, vz] = [c[0] - a[0], c[2] - a[2]];
        let det = ux * vz - uz * vx;
        if det.abs() < 1e-9 {
            return None;
        }
        let u = (dx * vz - dz * vx) / det;
        let v = (ux * dz - uz * dx) / det;
        (u >= -1e-6 && v >= -1e-6 && u + v <= 1.0 + 1e-6)
            .then_some(a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]))
    })
}

fn route_support(
    street: Option<f64>,
    structure_heights: impl Iterator<Item = f64>,
    reference: f64,
    reach: f64,
) -> WheelSupport {
    // The road can rise or fall beneath the axle. The feed uses half the
    // measured wheelbase as the limit for matching it to the route.
    let distance = |height: f64| (height - reference).abs();
    let closest = structure_heights
        .map(|y| (y, "structureCarriageway"))
        .chain(street.map(|y| (y, "streetHandoff")))
        .min_by(|a, b| distance(a.0).total_cmp(&distance(b.0)));
    match closest {
        Some((y, layer)) => WheelSupport {
            layer: if distance(y) <= reach {
                layer
            } else {
                "wrongRouteHeight"
            },
            road_y: Some(y),
        },
        None => WheelSupport {
            layer: "noRoutePaving",
            road_y: None,
        },
    }
}

struct WheelSupport {
    layer: &'static str,
    road_y: Option<f64>,
}

type SeparatedSupport<'a> = dyn Fn([f64; 2], f64, f64) -> WheelSupport + 'a;

fn summarize(
    trace: Trace,
    surface: impl Fn([f64; 2]) -> Option<Layer>,
    separated_surface: Option<&SeparatedSupport<'_>>,
    include_body: bool,
) -> Result<TraceReport> {
    ensure!(
        trace.avatar_height_m > 0.0,
        "invalid standing avatar height"
    );
    ensure!(
        trace
            .measured_chassis
            .values()
            .all(|chassis| chassis.valid()),
        "invalid measured chassis"
    );
    let lanes: HashMap<_, _> = trace.lanes.iter().map(|lane| (lane.id, lane)).collect();
    let mut result = TraceReport {
        name: trace.name,
        seed: trace.seed,
        sample_hz: trace.sample_hz,
        avatar_model: trace.avatar_model,
        avatar_height_m: trace.avatar_height_m,
        sampled_frames: trace.frames.len(),
        measured_vehicle_samples: 0,
        measured_separated_vehicle_samples: 0,
        skipped_separated_vehicle_samples: 0,
        missing_route_height_samples: 0,
        missing_chassis_samples: 0,
        roof_to_avatar_height: BTreeMap::new(),
        wheel_layers: BTreeMap::new(),
        body: include_body.then(BodyReport::default),
        lanes: BTreeMap::new(),
    };
    for frame in trace.frames {
        for car in frame.cars {
            let lane = lanes
                .get(&car.lane)
                .with_context(|| format!("missing lane {}", car.lane))?;
            let approach = car.corner_from.and_then(|id| lanes.get(&id));
            let separated = lane.level != 0 || approach.is_some_and(|lane| lane.level != 0);
            if separated && separated_surface.is_none() {
                result.skipped_separated_vehicle_samples += 1;
                continue;
            }
            let reference = if separated {
                let Some(height) = car.surface_y.filter(|height| height.is_finite()) else {
                    result.missing_route_height_samples += 1;
                    continue;
                };
                Some(height)
            } else {
                None
            };
            let Some((model, &chassis)) = car.model.as_ref().and_then(|model| {
                trace
                    .measured_chassis
                    .get(model)
                    .map(|chassis| (model, chassis))
            }) else {
                result.missing_chassis_samples += 1;
                continue;
            };
            result.measured_vehicle_samples += 1;
            result.measured_separated_vehicle_samples += usize::from(separated);
            result
                .roof_to_avatar_height
                .entry(model.clone())
                .or_insert(f64::from(chassis.max_xyz_m[1]) / trace.avatar_height_m);
            let lane_report = result.lanes.entry(lane.key.clone()).or_default();
            let support_at = |xz| {
                if let Some((support, reference)) = separated_surface.zip(reference) {
                    support(xz, reference, chassis.half_wheelbase_m())
                } else {
                    WheelSupport {
                        layer: layer_name(surface(xz)),
                        road_y: None,
                    }
                }
            };
            let wheel_names = ["frontLeft", "frontRight", "rearLeft", "rearRight"];
            let wheels = car.world_points(chassis.wheels_xz_m.map(|offset| offset.map(f64::from)));
            for (wheel, xz) in wheel_names.into_iter().zip(wheels) {
                let support = support_at(xz);
                let layer = support.layer;
                *result.wheel_layers.entry(layer).or_default() += 1;
                *lane_report.wheel_layers.entry(layer).or_default() += 1;
                if !on_carriageway(layer) && lane_report.first_non_carriageway.is_none() {
                    lane_report.first_non_carriageway = Some(WheelSample {
                        time: frame.time,
                        vehicle_id: car.id,
                        model: model.clone(),
                        wheel,
                        xz,
                        layer,
                        route_y: reference,
                        closest_road_y: support.road_y,
                    });
                }
            }
            if let Some(total) = &mut result.body {
                let lane_body = lane_report.body.get_or_insert_with(BodyReport::default);
                let local = body_probes(chassis);
                let points = car.world_points(local);
                let layers = points.map(|xz| support_at(xz).layer);
                let first = layers.iter().position(|layer| !on_carriageway(layer));
                for report in [total, lane_body] {
                    for layer in layers {
                        *report.probe_layers.entry(layer).or_default() += 1;
                    }
                    if let Some(i) = first {
                        report.poses_with_non_carriageway += 1;
                        if report.first_non_carriageway.is_none() {
                            report.first_non_carriageway = Some(BodySample {
                                time: frame.time,
                                vehicle_id: car.id,
                                model: model.clone(),
                                offset_xz: local[i],
                                xz: points[i],
                                layer: layers[i],
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_deck_triangle_can_support_a_wheel_in_another_tile() {
        let on_neighbour = Arc::new(StructureSurface {
            street: vec![[[-9.0, 8.0, 0.0], [3.0, 8.0, 0.0], [3.0, 8.0, 6.0]]],
            barrier: Vec::new(),
        });
        assert_eq!(
            structure_height_near([1.0, 1.0], 8.0, |key| {
                (key == (-1, 0)).then(|| Arc::clone(&on_neighbour))
            }),
            Some(8.0)
        );
    }

    #[test]
    fn the_nearest_drawn_road_supports_each_separated_wheel() {
        let triangles = [-6.0, 12.0].map(|y| [[0.0, y, 0.0], [4.0, y, 0.0], [0.0, y, 4.0]]);
        let support = |xz, route, reach| {
            route_support(
                Some(0.0),
                road_triangle_heights(&triangles, xz),
                route,
                reach,
            )
        };
        assert_eq!(support([1.0, 1.0], 12.1, 1.5).layer, "structureCarriageway");
        assert_eq!(support([1.0, 1.0], -6.1, 1.5).layer, "structureCarriageway");
        assert_eq!(support([1.0, 1.0], 0.2, 1.5).layer, "streetHandoff");
        // The wheel is past the deck edge. An unrelated road lies below.
        let wrong_floor = support([3.0, 3.0], 12.1, 1.5);
        assert_eq!(
            (wrong_floor.layer, wrong_floor.road_y),
            ("wrongRouteHeight", Some(0.0))
        );
        let over_empty_space = route_support(
            None,
            road_triangle_heights(&triangles, [3.0, 3.0]),
            12.1,
            1.5,
        );
        assert_eq!(
            (over_empty_space.layer, over_empty_space.road_y),
            ("noRoutePaving", None)
        );
        assert_eq!(support([1.0, 1.0], 21.0, 1.5).layer, "wrongRouteHeight");
    }

    #[test]
    fn wheel_samples_keep_the_measured_heading_and_ignore_other_levels() -> Result<()> {
        let car = json!({
            "id": 8, "model": "test-car", "lane": 1, "x": 10.0, "z": 20.0,
            "yaw": std::f64::consts::FRAC_PI_2,
        });
        let mut above = car.clone();
        above["lane"] = json!(2);
        above["surfaceY"] = json!(12.0);
        let mut leaving_deck = car.clone();
        leaving_deck["cornerFrom"] = json!(2);
        let mut unmeasured = car.clone();
        unmeasured["model"] = json!(null);
        let trace = json!({
            "name": "wheel test", "seed": 1, "sampleHz": 5,
            "avatarHeightM": 2.0, "avatarModel": "test-player",
            "measuredChassis": {"test-car": {
                "min_xyz_m": [-1.0, 0.0, -1.0], "max_xyz_m": [1.0, 1.5, 2.0],
                "wheels_xz_m": [[-1.0, 2.0], [1.0, 2.0], [-1.0, -1.0], [1.0, -1.0]],
                "wheel_radius_m": 0.35,
            }},
            "lanes": [
                {"id": 1, "key": "street", "level": 0},
                {"id": 2, "key": "deck", "level": 1},
            ],
            "frames": [{"time": 2.0, "cars": [car, above, leaving_deck, unmeasured]}],
        });
        let report = summarize(
            serde_json::from_value(trace.clone())?,
            |[x, z]| {
                Some(if x > 11.0 && z < 20.0 {
                    Layer::Sidewalk
                } else {
                    Layer::Carriageway
                })
            },
            None,
            false,
        )?;
        assert_eq!(report.measured_vehicle_samples, 1);
        assert!(report.body.is_none());
        assert_eq!(report.skipped_separated_vehicle_samples, 2);
        assert_eq!(report.missing_chassis_samples, 1);
        assert_eq!(report.roof_to_avatar_height["test-car"], 0.75);
        assert_eq!(
            report.wheel_layers,
            [("carriageway", 3), ("sidewalk", 1)].into()
        );
        assert_eq!(report.lanes.len(), 1);
        let event = report.lanes["street"]
            .first_non_carriageway
            .as_ref()
            .unwrap();
        assert_eq!(
            (event.vehicle_id, event.wheel, event.xz),
            (8, "frontRight", [12.0, 19.0])
        );
        let joined = summarize(
            serde_json::from_value(trace)?,
            |_| Some(Layer::Carriageway),
            Some(&|_, height, reach| {
                assert_eq!(height, 12.0);
                assert_eq!(reach, 1.5);
                WheelSupport {
                    layer: "structureCarriageway",
                    road_y: Some(height),
                }
            }),
            true,
        )?;
        assert_eq!(joined.measured_separated_vehicle_samples, 1);
        assert_eq!(joined.skipped_separated_vehicle_samples, 0);
        assert_eq!(joined.missing_route_height_samples, 1);
        assert_eq!(joined.lanes["deck"].wheel_layers["structureCarriageway"], 4);
        assert_eq!(
            joined.lanes["deck"].body.as_ref().unwrap().probe_layers["structureCarriageway"],
            9
        );
        assert!(joined.lanes["deck"].first_non_carriageway.is_none());
        Ok(())
    }

    #[test]
    fn body_probes_find_overhang_when_every_wheel_is_on_the_road() -> Result<()> {
        let trace = serde_json::from_value(json!({
            "name": "body test", "seed": 1, "sampleHz": 5,
            "avatarHeightM": 2.0, "avatarModel": "test-player",
            "measuredChassis": {"long-car": {
                "min_xyz_m": [-1.0, 0.0, -2.0], "max_xyz_m": [1.0, 1.5, 3.5],
                "wheels_xz_m": [[-0.8, 2.0], [0.8, 2.0], [-0.8, -1.0], [0.8, -1.0]],
                "wheel_radius_m": 0.35,
            }},
            "lanes": [{"id": 1, "key": "street", "level": 0}],
            "frames": [{"time": 2.0, "cars": [{
                "id": 8, "model": "long-car", "lane": 1, "x": 10.0, "z": 20.0,
                "yaw": std::f64::consts::FRAC_PI_2,
            }]}],
        }))?;
        let report = summarize(
            trace,
            |[x, _]| {
                Some(if x > 13.0 {
                    Layer::CurbTop
                } else {
                    Layer::Carriageway
                })
            },
            None,
            true,
        )?;
        assert_eq!(report.wheel_layers, [("carriageway", 4)].into());
        let body = report.body.unwrap();
        assert_eq!(body.poses_with_non_carriageway, 1);
        assert_eq!(
            body.probe_layers,
            [("carriageway", 6), ("curbTop", 3)].into()
        );
        let lane_body = report.lanes["street"].body.as_ref().unwrap();
        assert_eq!(lane_body.probe_layers, body.probe_layers);
        let event = lane_body.first_non_carriageway.as_ref().unwrap();
        assert_eq!(
            (event.vehicle_id, event.offset_xz, event.layer),
            (8, [-1.0, 3.5], "curbTop")
        );
        assert!((event.xz[0] - 13.5).abs() < 1e-12 && (event.xz[1] - 21.0).abs() < 1e-12);
        Ok(())
    }
}
