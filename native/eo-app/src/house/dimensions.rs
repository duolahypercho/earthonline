//! Place furniture in physical metres while the enclosing walls follow the surveyed lot.
use super::{Point, ProbeVolume, corners};
use anyhow::{Result, ensure};
use glam::{DMat3, DMat4, DVec2, DVec3, Mat4};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub(super) struct MetricAssembly {
    anchor: Point,
    components: Vec<String>,
    boxes: Vec<usize>,
}

pub(super) struct Layout {
    pub nodes: HashMap<String, Mat4>,
    pub boxes: Vec<Option<DMat4>>,
}

impl Layout {
    pub fn new(
        groups: &[MetricAssembly],
        count: usize,
        across: DVec2,
        back: DVec2,
        height: f64,
    ) -> Result<Self> {
        let mut nodes = HashMap::new();
        let mut boxes = vec![None; count];
        let horizontal = |v: DVec2| DVec3::new(v.x, 0., v.y);
        let site = DMat3::from_cols(horizontal(across), DVec3::Y * height, horizontal(back));
        ensure!(
            site.determinant().abs() > 1e-6,
            "degenerate furnished house footprint"
        );
        let x = across.normalize();
        let z = x.perp() * x.perp_dot(back).signum();
        let metres = DMat3::from_cols(horizontal(x), DVec3::Y, horizontal(z));
        let correction = DMat4::from_mat3(site.inverse() * metres);
        for group in groups {
            let a = DVec3::from(group.anchor);
            let matrix = DMat4::from_translation(a) * correction * DMat4::from_translation(-a);
            for name in &group.components {
                ensure!(
                    nodes.insert(name.clone(), matrix.as_mat4()).is_none(),
                    "duplicate furniture component {name}"
                );
            }
            for &i in &group.boxes {
                ensure!(
                    i < count && boxes[i].is_none(),
                    "invalid furniture collision box {i}"
                );
                boxes[i] = Some(matrix);
            }
        }
        Ok(Self { nodes, boxes })
    }

    pub fn probe(&self, name: &str, volume: &ProbeVolume) -> ProbeVolume {
        let Some(m) = self.nodes.get(name) else {
            return volume.clone();
        };
        let points = corners(volume.min.map(f64::from), volume.max.map(f64::from))
            .map(|p| m.transform_point3(DVec3::from(p).as_vec3()));
        let min = points.into_iter().reduce(|a, b| a.min(b)).unwrap();
        let max = points.into_iter().reduce(|a, b| a.max(b)).unwrap();
        ProbeVolume {
            min: min.to_array(),
            max: max.to_array(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furniture_and_collision_keep_their_metres_on_rotated_skewed_and_scaled_lots() {
        let a = [1.3, 10. / 3., 7.2];
        for (across, back, h) in [
            (DVec2::new(1.3, -0.11), DVec2::new(0.13, 1.29), 1.0),
            (DVec2::new(0., 0.84), DVec2::new(1.67, 0.2), 1.23),
        ] {
            let group = MetricAssembly {
                anchor: a,
                components: vec!["chair".into()],
                boxes: vec![1],
            };
            let layout = Layout::new(&[group], 2, across, back, h).unwrap();
            let world = |p: DVec3| {
                DVec3::new(
                    across.x * p.x + back.x * p.z,
                    h * p.y,
                    across.y * p.x + back.y * p.z,
                )
            };
            let m = layout.nodes["chair"].as_dmat4();
            let origin = DVec3::from(a);
            let p = |v| world(m.transform_point3(origin + v));
            assert!(p(DVec3::ZERO).distance(world(origin)) < 1e-5);
            let width = p(DVec3::X * 0.50) - p(DVec3::ZERO);
            let depth = p(DVec3::Z * 0.57) - p(DVec3::ZERO);
            assert!((width.length() - 0.50).abs() < 1e-6 && (depth.length() - 0.57).abs() < 1e-6);
            assert!(width.dot(depth).abs() < 1e-6);
            let seat = origin + DVec3::Y * 0.454;
            assert!(layout.boxes[0].is_none());
            let contact = world(layout.boxes[1].unwrap().transform_point3(seat));
            assert!((contact.y - world(origin).y - 0.454).abs() < 1e-6);
            assert!(contact.distance(world(m.transform_point3(seat))) < 1e-5);
        }
    }
}
