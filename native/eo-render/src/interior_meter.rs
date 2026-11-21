//! Daylight metering in an enclosed space. The glass that is visible through
//! the lens contributes exterior luminance even when it occupies little of the frame.
use crate::camera::Camera;
use glam::{Mat4, Vec3};
use std::collections::BTreeMap;

pub struct InteriorMeter {
    to_local: Mat4,
    min: Vec3,
    max: Vec3,
    storey: f32,
    patches: Vec<Patch>,
}

struct Patch {
    centre: Vec3,
    area_normal: Vec3,
    level: i32,
}

impl InteriorMeter {
    /// The caller supplies one side of each glazed face, in building coordinates.
    pub fn new(
        to_world: Mat4,
        min: Vec3,
        max: Vec3,
        storey: f32,
        glass: impl Iterator<Item = [Vec3; 3]>,
    ) -> Self {
        let mut bins = BTreeMap::new();
        for tri in glass {
            let local = (tri[0] + tri[1] + tri[2]) / 3.;
            let [a, b, c] = tri.map(|p| to_world.transform_point3(p));
            let projected_area = (b - a).cross(c - a) * 0.5;
            let area = projected_area.length();
            if area < 1e-7 {
                continue;
            }
            let cell = (local * 2.).floor().as_ivec3().to_array();
            let (weighted, normal, weight) =
                bins.entry(cell).or_insert((Vec3::ZERO, Vec3::ZERO, 0.));
            *weighted += (a + b + c) * (area / 3.);
            *normal += projected_area;
            *weight += area;
        }
        let to_local = to_world.inverse();
        let patches = bins
            .into_values()
            .map(|(sum, area_normal, area)| {
                let centre = sum / area;
                let level = (to_local.transform_point3(centre).y / storey).floor() as i32;
                Patch {
                    centre,
                    area_normal,
                    level,
                }
            })
            .collect();
        Self {
            to_local,
            min,
            max,
            storey,
            patches,
        }
    }

    pub fn world_to_local(&self) -> Mat4 {
        self.to_local
    }

    pub fn daylight_gain(&self, cam: &Camera) -> f64 {
        let eye = self.to_local.transform_point3(cam.eye);
        let inside = (eye - self.min).min(self.max - eye).min_element();
        if inside <= 0. {
            return 1.;
        }
        let level = (eye.y / self.storey).floor() as i32;
        let forward = cam.forward();
        let right = cam.right();
        let up = right.cross(forward);
        let half_y = (0.5 * cam.fov_y_deg.to_radians()).tan();
        let half_x = half_y * cam.aspect;
        let frame_angle =
            4. * (half_x * half_y / (1. + half_x * half_x + half_y * half_y).sqrt()).atan();
        let mut visible_angle = 0.;
        for p in self.patches.iter().filter(|p| p.level == level) {
            let delta = p.centre - cam.eye;
            let depth = delta.dot(forward);
            if depth <= 0. {
                continue;
            }
            let x = delta.dot(right).abs() / (depth * half_x);
            let y = delta.dot(up).abs() / (depth * half_y);
            // Taper the edge over the finite extent of a patch.
            let coverage = ((1.12 - x.max(y)) / 0.24).clamp(0., 1.);
            let dist2 = delta.length_squared().max(0.01);
            visible_angle += coverage * p.area_normal.dot(delta).abs() / (dist2 * dist2.sqrt());
        }
        let window = (visible_angle / frame_angle).clamp(0., 1.);
        // Log-average metering of an enclosed diffuse field and bright glazing.
        // Keep the highlights' latitude within a stop and a half.
        let gain = 2.0_f32.powf(1.5 * (1. - window.sqrt()));
        let transition = (inside / 0.45).clamp(0., 1.);
        (1. + (gain - 1.) * transition) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metering_accounts_for_glazing_on_the_observers_floor() {
        let at = |x, y| Vec3::new(x, y, 0.);
        let panes = [0., 3.].into_iter().flat_map(|h| {
            let [a, b, c, d] = [
                at(-1., h + 0.6),
                at(1., h + 0.6),
                at(1., h + 2.4),
                at(-1., h + 2.4),
            ];
            [[a, b, c], [a, c, d]]
        });
        let to_world = Mat4::from_rotation_y(0.31);
        let meter = InteriorMeter::new(
            to_world,
            Vec3::new(-2., 0., 0.),
            Vec3::new(2., 6., 4.),
            3.,
            panes,
        );
        let camera = |local, yaw| Camera {
            eye: to_world.transform_point3(local),
            yaw,
            pitch: 0.,
            fov_y_deg: 50.,
            aspect: 1.6,
            near: 0.05,
            far: 100.,
            jitter: [0.; 2],
        };
        let eye = Vec3::new(0., 1.5, 1.8);
        let bright_window = meter.daylight_gain(&camera(eye, -0.31));
        let dark_wall = meter.daylight_gain(&camera(eye, std::f32::consts::PI - 0.31));
        assert!(bright_window < dark_wall - 0.3);
        assert!(
            (meter.daylight_gain(&camera(eye + Vec3::Y * 3., -0.31)) - bright_window).abs() < 1e-5
        );
        assert_eq!(
            meter.daylight_gain(&camera(Vec3::new(0., 1.5, -1.), 0.)),
            1.
        );
    }
}
