//! Measured direct-light visibility at the porch, in the house's parcel coordinates.
use super::{Finishes, House};
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, kit::BakedMaps};
use glam::Vec3;
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Deserialize)]
pub(super) struct Lamp {
    min: [f32; 3],
    max: [f32; 3],
    new_centre: [f32; 3],
    old_centre: [f32; 3],
    colour: [f32; 3],
    radiance: f32,
    power_w: f32,
    area_m2: f32,
    calibration: f32,
    scales: [f32; 2],
    reference_camera_distance_m: f32,
}

impl Lamp {
    pub(super) fn read(root: &Path, directory: &Path, shell: &str) -> Result<(Self, BakedMaps)> {
        #[derive(Deserialize)]
        struct Survey {
            #[serde(flatten)]
            asset: AssetRef,
            #[serde(flatten)]
            lamp: Lamp,
            house_sha256: String,
            grid: [usize; 3],
            hours: [f32; 4],
            charts: [String; 5],
            encoding: String,
        }
        let survey: Survey = serde_json::from_slice(&std::fs::read(
            directory.join("porch-light.provenance.json"),
        )?)?;
        let lamp = survey.lamp;
        ensure!(
            survey.house_sha256 == shell
                && survey.grid == [48, 64, 48]
                && survey.encoding == "porch-radiance-and-visibility-v1"
                && survey.hours == [8.5, 12., 17., 22.]
                && survey.charts
                    == [
                        "back wall",
                        "east reveal",
                        "west reveal",
                        "arch underside",
                        "arch face"
                    ],
            "incompatible porch light field"
        );
        for i in 0..3 {
            ensure!(
                lamp.min[i] < lamp.max[i]
                    && [lamp.new_centre[i], lamp.old_centre[i]]
                        .iter()
                        .all(|v| (lamp.min[i]..=lamp.max[i]).contains(v)),
                "fixture outside its measured field"
            );
        }
        ensure!(
            [
                lamp.calibration,
                lamp.radiance,
                lamp.power_w,
                lamp.area_m2,
                lamp.scales[0],
                lamp.scales[1]
            ]
            .iter()
            .all(|v| v.is_finite() && *v > 0.)
                && lamp.colour.iter().all(|c| (0.0..=1.).contains(c))
                && (lamp.radiance * lamp.area_m2 * std::f32::consts::PI - lamp.power_w).abs()
                    < lamp.power_w * 0.001,
            "invalid lamp photometry"
        );
        ensure!(
            lamp.reference_camera_distance_m.is_finite() && lamp.reference_camera_distance_m > 0.,
            "invalid registered porch camera"
        );
        let maps = Finishes::read_maps(root, &survey.asset, 3)?;
        Ok((lamp, maps))
    }

    pub(super) fn emission(&self) -> [f32; 3] {
        self.colour.map(|c| c * self.radiance * self.calibration)
    }

    pub(super) fn camera(&self, house: &House) -> (Vec3, f32) {
        let [x, y, z] = self.new_centre;
        (
            house.point(x.into(), y.into(), z.into()).as_vec3(),
            self.reference_camera_distance_m,
        )
    }

    pub(super) fn uniforms(&self, house: &House, layer: usize) -> [[f32; 4]; 7] {
        let world = |p: [f32; 3]| house.point(p[0].into(), p[1].into(), p[2].into()).as_vec3();
        let new = world(self.new_centre);
        let old = world(self.old_centre);
        let outer_radius_sq = super::corners(self.min.map(f64::from), self.max.map(f64::from))
            .map(|[x, y, z]| new.distance_squared(house.point(x, y, z).as_vec3()))
            .into_iter()
            .fold(0_f32, f32::max);
        let [lo, hi] = house.door.bounds;
        let hinge = house.door.hinge;
        let pivot = house.point(hinge[0], lo[1], hinge[2]).as_vec3();
        let door_edge = -house.across.normalize();
        let height = ((hi[1] - lo[1]) * house.height_scale) as f32;
        let width = ((hi[0] - lo[0]) * house.across.length()) as f32;
        let relative_size = Vec3::from(self.max) - Vec3::from(self.min);
        [
            new.extend(outer_radius_sq).to_array(),
            Vec3::from(self.min).extend(layer as f32).to_array(),
            relative_size
                .recip()
                .extend(self.scales[0] * self.radiance * self.calibration)
                .to_array(),
            old.extend(self.scales[1] * self.power_w * self.calibration)
                .to_array(),
            Vec3::from(self.colour)
                .extend((self.area_m2 / (4. * std::f32::consts::PI)).sqrt())
                .to_array(),
            pivot.extend(height).to_array(),
            [door_edge.x as f32, door_edge.y as f32, width, 0.],
        ]
    }
}
