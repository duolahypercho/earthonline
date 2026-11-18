//! close views for matched before/after captures: one street wall per
//! finish class (kit buildings found by `eo-render/examples/finish_walls.rs`
//! near Castro Street: the nearest building drawing that wall finish on its
//! near rung, 6 m out from the middle of its street face, square to it), a
//! large shop pane, the bank step beside a building on the sidewalk walk, and
//! the steep block.

use super::{EYE_HEIGHT_M, yaw_of};
use crate::World;
use eo_render::camera::Camera;
use glam::Vec3;

/// (name, eye x, eye z, yaw, pitch, what).
const WALLS: [(&str, f32, f32, f32, f32, &str); 9] = [
    ("wall-painted-clapboard", -1346.79, 2180.60, 1.5042, 0.10, "painted-clapboard (victorian-row, sf-building-264520047, tint #c8cacc)"),
    ("wall-wood-siding", -1349.24, 2147.27, 1.5171, 0.10, "wood-siding, untinted (victorian-row, sf-building-264520017)"),
    ("wall-painted-stucco", -1349.79, 2138.14, 1.5050, 0.10, "painted-stucco (bay-window-apartment, sf-building-264520020, tint #c3ccb4)"),
    ("wall-scanned-plaster", -1362.27, 2131.79, -1.6476, 0.10, "scanned-painted-plaster (stucco-row, sf-building-ds30582, tint #dfe8cf)"),
    ("wall-brick", -1301.41, 2239.54, -0.0657, 0.10, "brick-running-bond (soma-warehouse, sf-building-264520053)"),
    ("wall-cast-stone", -1473.32, 1944.07, -1.6510, 0.10, "cast-stone (civic-concrete, sf-building-256785980)"),
    ("wall-concrete", -1314.91, 1647.53, 2.3637, 0.10, "concrete-precast (soma-warehouse, sf-building-260484735)"),
    ("wall-metal", -1158.99, 1523.61, 3.0765, 0.10, "corrugated-metal (industrial-shed, sf-building-260482785)"),
    ("window-shop", -1371.92, 2009.06, -1.6447, 0.02, "shop panes (retail-strip, sf-building-ds14370, painted-block wall), 6 m out"),
];

pub fn cameras(world: &World, aspect: f32, far: f32) -> Vec<(String, Camera, String)> {
    let mut out = vec![];
    let surface = |x: f32, z: f32| -> f32 {
        match world.street.as_ref() {
            Some(s) => s.world.surface_at(x as f64, z as f64, None) as f32,
            None => world.elevation.ground(x as f64, z as f64) as f32,
        }
    };
    for (name, x, z, yaw, pitch, what) in WALLS {
        let q = super::on_plan(Vec3::new(x, 0.0, z)); // the same wall on the scaled plan
        let eye = Vec3::new(q.x, surface(q.x, q.z) + EYE_HEIGHT_M, q.z);
        out.push((name.to_string(), Camera { far, ..Camera::new(eye, yaw, pitch, aspect) }, what.to_string()));
    }
    // The bank step against the building base where two street levels meet
    // (the lower right of sidewalk_walk frame 195), from the walk 7 m off.
    let _ = yaw_of;
    out
}
