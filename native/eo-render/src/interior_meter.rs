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
    room_responses: BTreeMap<i32, RoomResponse>,
}

struct RoomResponse {
    sky_return: f32,
    max_gain: f32,
    photographic_exposure: Option<f32>,
    exposed_beam: f32,
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
            room_responses: BTreeMap::new(),
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

    /// Locate the first face of the room envelope on a metering ray. Faces are
    /// returned in world space, including the surveyed parcel's orientation.
    fn facade_normal(&self, eye: Vec3, direction: Vec3) -> Option<Vec3> {
        let ray = self.to_local.transform_vector3(direction);
        let mut entry = 0.0_f32;
        let mut exit = f32::INFINITY;
        let mut face = Vec3::ZERO;
        for axis in 0..3 {
            if ray[axis].abs() < 1e-6 {
                if eye[axis] < self.min[axis] || eye[axis] > self.max[axis] {
                    return None;
                }
                continue;
            }
            let a = (self.min[axis] - eye[axis]) / ray[axis];
            let b = (self.max[axis] - eye[axis]) / ray[axis];
            if a.min(b) > entry {
                entry = a.min(b);
                face = Vec3::ZERO;
                face[axis] = -ray[axis].signum();
            }
            exit = exit.min(a.max(b));
        }
        (entry < exit && face != Vec3::ZERO).then(|| {
            self.to_local
                .transpose()
                .transform_vector3(face)
                .normalize()
        })
    }

    /// Sky metering normally treats every upward ray as open sky. At a close
    /// facade, meter the incident light on the wall that actually fills the lens.
    pub fn scene_gain(
        &self,
        cam: &Camera,
        f: &eo_look::LookFrame,
        weather: &eo_sky::WeatherState,
        directions: &[([f64; 3], f64)],
        sky_meter: f64,
        outdoor_target: f64,
    ) -> f64 {
        use eo_look::math::luminance;
        use eo_sky::night::{METER_GAIN, SIDEWALK_ENV, ground_fill, key_on_ground, mean_beam};
        let eye = self.to_local.transform_point3(cam.eye);
        let delta = eye - eye.clamp(self.min, self.max);
        let range = self.to_local.inverse().transform_vector3(delta).length();
        if range <= 0.001 {
            // Both renderers measure the same horizontal white card. Convert the
            // source camera latitude into the live outdoor meter's exposure units.
            let beam = key_on_ground(f) * mean_beam(weather);
            let daylight = (beam as f32).clamp(0.,1.);
            let level = (eye.y / self.storey).floor() as i32;
            let outside_exposure = (f.exposure * sky_meter).max(1e-5);
            let response = self.room_responses.get(&level);
            let exposure_ratio = response.map_or(1., |r| {
                (r.exposed_beam as f64 / (beam * outside_exposure).max(1e-5)).min(1.) as f32
            });
            // The sky applies this gain to its current target. Dividing by last
            // frame's adapted exposure feeds the lens response back into itself.
            let photographic_gain = response.and_then(|r| r.photographic_exposure)
                .map(|exposure| (f64::from(exposure) / (outdoor_target * sky_meter).max(1e-5)) as f32);
            return self.enclosed_gain(cam, daylight, exposure_ratio, photographic_gain);
        }
        // Outside the parcel approach, nearer buildings may obscure this envelope.
        if range >= 35. || f.key_dir[1] <= 0. {
            return 1.;
        }
        let weight = f64::from(((35. - range) / 15.).clamp(0., 1.) * (range / 0.45).min(1.));
        let beam = mean_beam(weather);
        let reference = (ground_fill(f, SIDEWALK_ENV) + key_on_ground(f) * beam).max(1e-6);
        let sun = Vec3::from_array(f.key_dir.map(|c| c as f32));
        let mut stops = 0.;
        let mut total = 0.;
        let mut occluded = directions.to_vec();
        for (direction, wt) in &mut occluded {
            total += *wt;
            let Some(n) = self.facade_normal(eye, Vec3::from_array(direction.map(|v| v as f32)))
            else {
                continue;
            };
            let hemisphere = (n.y as f64 + 1.) * 0.5;
            let sh = eo_look::sh_eval(&f.env_sh, n.to_array().map(f64::from));
            let fill = [0, 1, 2].map(|i| {
                f.hemi_ground[i] * (1. - hemisphere)
                    + f.hemi_sky[i] * hemisphere
                    + f.ambient[i]
                    + sh[i] * f.rig.environment_intensity * f.rig.env_painted
            });
            let irradiance =
                luminance(fill) + luminance(f.key) * beam * f64::from(n.dot(sun).max(0.));
            // Preserve highlight latitude when a bright opening borders a shaded pier.
            stops += *wt * (reference / irradiance.max(1e-6)).log2().clamp(-0.5, 1.35);
            // The sky meter uses a grey card for directions occluded by solid ground.
            direction[1] = -direction[1].abs();
        }
        let corrected = eo_sky::night::meter(f, weather, &occluded);
        let stops = METER_GAIN * stops / total.max(1.) + (corrected / sky_meter.max(1e-6)).log2();
        2.0_f64.powf(weight * stops)
    }

    /// The calibrated source-camera latitude limits exposure when little glass is in frame.
    pub fn set_room_response(&mut self, level: i32, sky_return: f32, exposure_range: f32, exposed_beam: f32) {
        self.set_room_camera_response(level, sky_return, exposure_range, exposed_beam, None);
    }

    /// A registered photograph can supply the room's maximum daylight exposure.
    /// Large windows still stop the lens down and night uses the live city meter.
    pub fn set_room_camera_response(&mut self, level: i32, sky_return: f32, exposure_range: f32,
                                    exposed_beam: f32, photographic_exposure: Option<f32>) {
        self.room_responses.insert(level, RoomResponse {
            sky_return, max_gain: exposure_range.exp2(), exposed_beam, photographic_exposure,
        });
    }

    pub fn daylight_gain(&self, cam: &Camera) -> f64 {
        self.enclosed_gain(cam, 1., 1., None)
    }

    fn enclosed_gain(&self, cam: &Camera, daylight: f32, exposure_ratio: f32, photographic_gain: Option<f32>) -> f64 {
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
        let mut gain = 2.0_f32.powf(1.5 * (1. - window.sqrt()));
        if let Some(r) = self.room_responses.get(&level) {
            // Weighted reflected light from the visible glazing and the room.
            let available = (window + (1.-window)*r.sky_return).recip().min(r.max_gain);
            let measured = photographic_gain.map_or(available * exposure_ratio, |cap| available.min(cap)).max(1.);
            let y = eye.y - level as f32 * self.storey;
            let blend = (y.min(self.storey-y) / 0.35).clamp(0.,1.) * daylight;
            gain += blend * (measured-gain);
        }
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
        let mut meter = InteriorMeter::new(
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
        let ray = to_world.transform_vector3(Vec3::Z);
        let front = meter.facade_normal(Vec3::new(0., 1.5, -2.), ray).unwrap();
        assert!((front + ray).length() < 1e-5);
        assert!(meter.facade_normal(Vec3::new(0., 1.5, -2.), -ray).is_none());
        assert!(meter.facade_normal(Vec3::new(4., 1.5, -2.), ray).is_none());
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
        meter.set_room_response(1, 0.01, 2.6, 2.4);
        let upstairs = camera(eye + Vec3::Y * 3., std::f32::consts::PI-0.31);
        assert!((meter.daylight_gain(&upstairs) - 2.6_f64.exp2()).abs() < 1e-5);
        assert!((meter.enclosed_gain(&upstairs, 0., 1., None) - dark_wall).abs() < 1e-5);
        assert!((meter.daylight_gain(&camera(eye, -0.31)) - bright_window).abs() < 1e-5);
        meter.set_room_camera_response(0, 0.005, 2.63, 1.33, Some(2.25));
        let downstairs = camera(eye, std::f32::consts::PI-0.31);
        assert!((meter.enclosed_gain(&downstairs, 1., 1., Some(2.25)) - 2.25).abs() < 1e-5);
        assert!((meter.enclosed_gain(&downstairs, 0., 1., Some(2.25)) - dark_wall).abs() < 1e-5);
        assert!((meter.daylight_gain(&upstairs) - 2.6_f64.exp2()).abs() < 1e-5);
        let weather = eo_sky::weather::preset(eo_sky::Kind::Clear);
        let mut frame = eo_look::LookFrame::new(12.);
        for previous_exposure in [0.5, 1.5, 2.25] {
            frame.exposure = previous_exposure;
            let gain = meter.scene_gain(&downstairs, &frame, &weather, &[], 1., 0.8);
            assert!((0.8 * gain - 2.25).abs() < 1e-5);
        }
    }
}
