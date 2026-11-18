//! Where a Blender-built building stands, computed the way the JS game does.
//!
//! - Frame: `authoredFootprintFrame` (src/world/buildings/authored/physics.js)
//!   on the REFLECTED record (`reflectBuildingRecord`, z := -z: every authored
//!   module was written in a z-north frame and the world runs z-south). The
//!   named front edge gives +X, the outward normal +Z, the bounding box in
//!   those axes gives the centre, width and depth; heading = atan2(front.x, front.z).
//! - Base height: `prepareBuildingPlacements` (building-placement.js): the
//!   finished floor is the ground anchor plus the street construction lift
//!   (`footwayLiftM`). For a Blender building the anchor is its street-front
//!   grade (`streetFrontGrades` in street-front-grade.js over its front edge,
//!   the `authoredFronts` list of city-contracts-street-inputs.js).
//! - Transform: the module's root is at (centre.x, baseY, centre.z) turned
//!   `heading` about +Y (three's `rotation.y`), under the registry's mirror
//!   `diag(1, 1, -1)` (`mirrorAuthoredRoot`). So world = M * T * Ry * v.
//! - The GLB was built in the frame of the record at build time; the JS
//!   refuses to draw it if the live frame drifted more than 5 cm
//!   (`BLENDER_FRAME_TOLERANCE_M`), and so does this.

use anyhow::{Result, anyhow, bail};
use glam::{DMat4, DVec2, DVec3};
use serde::Deserialize;

use crate::manifest::BlenderBuilding;

/// `BLENDER_FRAME_TOLERANCE_M` in blender/runtime.js.
pub const BLENDER_FRAME_TOLERANCE_M: f64 = 0.05;

const PLACEMENT_JSON: &str = include_str!("../data/authored-placement.json");

#[derive(Clone, Debug, Deserialize)]
pub struct FrontGrade {
    pub samples: usize,
    #[serde(rename = "maxReliefM")]
    pub max_relief_m: f64,
    #[serde(rename = "clampM")]
    pub clamp_m: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Hlod {
    #[serde(rename = "enterRadiusM")]
    pub enter_radius_m: f64,
    #[serde(rename = "exitRadiusM")]
    pub exit_radius_m: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct OracleFrame {
    #[serde(rename = "centerX")]
    pub center_x: f64,
    #[serde(rename = "centerZ")]
    pub center_z: f64,
    pub heading: f64,
    pub width: f64,
    pub depth: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct OracleSample {
    pub id: String,
    #[serde(rename = "edgeIndex")]
    pub edge_index: usize,
    pub frame: OracleFrame,
    #[serde(rename = "frontGroundY")]
    pub front_ground_y: Option<f64>,
    #[serde(rename = "baseY")]
    pub base_y: Option<f64>,
}

/// Numbers the JS computes, exported by the earlier browser version's exporter.
#[derive(Clone, Debug, Deserialize)]
pub struct PlacementConstants {
    #[serde(rename = "constructionLiftM")]
    pub construction_lift_m: f64,
    #[serde(rename = "frontGrade")]
    pub front_grade: FrontGrade,
    pub hlod: Hlod,
    /// JS results for a sample of buildings (tests only).
    pub samples: Vec<OracleSample>,
}

impl PlacementConstants {
    pub fn load() -> Result<Self> {
        Ok(serde_json::from_str(PLACEMENT_JSON)?)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub edge_index: usize,
    pub width: f64,
    pub depth: f64,
    pub center: DVec2,
    pub right: DVec2,
    pub front: DVec2,
    pub heading: f64,
}

/// `authoredFootprintFrame(building, frontEdgeIndex)`; `polygon` as the record lists it.
pub fn footprint_frame(polygon: &[DVec2], front_edge: usize) -> Result<Frame> {
    if polygon.len() < 3 {
        bail!("footprint needs at least three points");
    }
    let edge_index = front_edge.min(polygon.len() - 1);
    let a = polygon[edge_index];
    let b = polygon[(edge_index + 1) % polygon.len()];
    let e = b - a;
    let len = e.length();
    if !(len > 1e-3) {
        bail!("front edge {edge_index} is degenerate");
    }
    let mut centroid = DVec2::ZERO;
    for p in polygon {
        centroid += *p / polygon.len() as f64;
    }
    let right = e / len;
    let midpoint = (a + b) / 2.0;
    let mut front = DVec2::new(-right.y, right.x);
    if (midpoint - centroid).dot(front) < 0.0 {
        front = -front;
    }
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in polygon {
        let x = p.dot(right);
        let z = p.dot(front);
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_z = min_z.min(z);
        max_z = max_z.max(z);
    }
    let center = right * ((min_x + max_x) / 2.0) + front * ((min_z + max_z) / 2.0);
    Ok(Frame {
        edge_index,
        width: max_x - min_x,
        depth: max_z - min_z,
        center,
        right,
        front,
        heading: front.x.atan2(front.y),
    })
}

/// `streetFrontGrades` for one front: the mean ground over `samples` points
/// along the front edge, clamped to the footprint-centre ground +- `clampM`
/// when the ground under the footprint's vertices spans more than `maxReliefM`.
/// `polygon` is the world (z-south) record polygon. None where the ground is unknown.
pub fn front_ground(polygon: &[DVec2], front_edge: usize, c: &FrontGrade, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Option<f64> {
    if polygon.len() < 3 || front_edge >= polygon.len() {
        return None;
    }
    let a = polygon[front_edge];
    let b = polygon[(front_edge + 1) % polygon.len()];
    let mut sum = 0.0;
    for i in 0..c.samples {
        let t = i as f64 / (c.samples - 1) as f64;
        sum += height_at(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)?;
    }
    let mut ground = sum / c.samples as f64;
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for q in polygon {
        min_x = min_x.min(q.x);
        max_x = max_x.max(q.x);
        min_z = min_z.min(q.y);
        max_z = max_z.max(q.y);
        if let Some(y) = height_at(q.x, q.y) {
            lo = lo.min(y);
            hi = hi.max(y);
        }
    }
    if let Some(centre) = height_at((min_x + max_x) / 2.0, (min_z + max_z) / 2.0) {
        if hi - lo > c.max_relief_m {
            ground = ground.clamp(centre - c.clamp_m, centre + c.clamp_m);
        }
    }
    Some(ground)
}

#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// GLB frame -> world (x east, z south, y up). Determinant -1 (the mirror).
    pub world_from_model: DMat4,
    pub frame: Frame,
    pub base_y: f64,
    pub drift_m: f64,
    /// The JS HLOD entry's centre (frame centre, baseY + height / 2) and
    /// bounds radius (half the diagonal of the footprint box and height).
    pub lod_center: DVec3,
    pub lod_radius: f64,
}

/// Place `b` on its record's polygon. `ring` is the record polygon in world
/// metres exactly as the record lists it (including a closing duplicate if
/// the source has one), `height` the record height.
pub fn place(
    b: &BlenderBuilding,
    ring: &[DVec2],
    height: f64,
    c: &PlacementConstants,
    height_at: &dyn Fn(f64, f64) -> Option<f64>,
) -> Result<Placement> {
    let reflected: Vec<DVec2> = ring.iter().map(|p| DVec2::new(p.x, -p.y)).collect();
    let frame = footprint_frame(&reflected, b.frame.edge_index)?;
    // on the scaled ground plan the record was moved whole, real
    // size (eo-data `groundscale::GroundPlan`); the frame moved with it (z reflected).
    let shift = eo_data::groundscale::record_shift(b.record_key()).map(|d| DVec2::new(d[0], -d[1])).unwrap_or(DVec2::ZERO);
    let drift = (frame.center.x - (b.frame.center.x + shift.x))
        .abs()
        .max((frame.center.y - (b.frame.center.z + shift.y)).abs())
        .max((frame.width - b.frame.width).abs())
        .max((frame.depth - b.frame.depth).abs());
    if drift > BLENDER_FRAME_TOLERANCE_M {
        return Err(anyhow!("{} footprint drifted {drift:.3} m since its build; the JS refuses it too", b.id));
    }
    let (mut min_x, mut max_x, mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for q in ring {
        min_x = min_x.min(q.x);
        max_x = max_x.max(q.x);
        min_z = min_z.min(q.y);
        max_z = max_z.max(q.y);
    }
    let centre_ground = height_at((min_x + max_x) / 2.0, (min_z + max_z) / 2.0);
    let anchor = front_ground(ring, b.frame.edge_index, &c.front_grade, height_at)
        .or(centre_ground)
        .ok_or_else(|| anyhow!("{}: no ground under the footprint", b.id))?;
    let base_y = anchor + c.construction_lift_m;
    let mirror = DMat4::from_scale(DVec3::new(1.0, 1.0, -1.0));
    let world_from_model = mirror
        * DMat4::from_translation(DVec3::new(frame.center.x, base_y, frame.center.y))
        * DMat4::from_rotation_y(frame.heading);
    let lod_center = DVec3::new(frame.center.x, base_y + height * 0.5, -frame.center.y);
    let lod_radius = DVec3::new(max_x - min_x, height, max_z - min_z).length() * 0.5;
    Ok(Placement { world_from_model, frame, base_y, drift_m: drift, lod_center, lod_radius })
}
