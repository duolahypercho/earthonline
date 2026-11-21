//! The dedicated floorboard ORM array reads roughness from green. Its other
//! three channels carry linear, prefiltered room radiance for the dining oak.
use super::{AssetRef, Finishes};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Copy)]
pub(super) struct Finish {
    pub parameters: [[f32; 4]; 3],
}

impl Finish {
    pub fn load(root: &Path, directory: &Path, house_sha: &str, orm: &mut [u8]) -> Result<Self> {
        #[derive(Deserialize)]
        struct Survey {
            #[serde(flatten)]
            asset: AssetRef,
            house_sha256: String,
            tile_size: [usize; 2],
            source_rows: [String; 7],
            transfer_encoding: String,
            probe_rectangle: [f32; 4],
            tabletop_bounds: [f32; 4],
            daylight_map: [usize; 2],
            daylight_first_row: usize,
            normal_strength: f32,
            coat_weight: f32,
            coat_roughness: f32,
            coat_ior: f32,
        }
        let survey: Survey = serde_json::from_slice(&std::fs::read(
            directory.join("oak-reflection.provenance.json"),
        )?)?;
        ensure!(
            survey.house_sha256 == house_sha
                && survey.tile_size == [64, 32]
                && survey.daylight_map == [256, 64]
                && survey.daylight_first_row == 224
                && survey.source_rows
                    == [
                        "fixtures",
                        "uniform-sky",
                        "clear-sky",
                        "front-25",
                        "front-57",
                        "rear-25",
                        "rear-57"
                    ]
                && survey.transfer_encoding == "sqrt(L/(1+L))",
            "unexpected oak reflection layout"
        );
        let [x, z, right, rear] = survey.probe_rectangle;
        let [left, front, end, back] = survey.tabletop_bounds;
        ensure!(
            left < x && front < z && right < end && rear < back,
            "invalid oak receiver bounds"
        );
        ensure!(
            x >= 0.
                && x < right
                && right <= 7.
                && z >= 0.
                && z < rear
                && rear <= 13.
                && (0.0..=1.).contains(&survey.normal_strength)
                && (0.0..=1.).contains(&survey.coat_weight)
                && (0.0..=1.).contains(&survey.coat_roughness)
                && (1.0..=3.).contains(&survey.coat_ior),
            "invalid oak surface or probe position"
        );
        let pixels = Finishes::read_pixels(root, &survey.asset, 1)?;
        ensure!(orm.len() == pixels.len(), "missing oak roughness scan");
        for (destination, source) in orm.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
            destination[0] = source[0];
            destination[2] = source[1];
            destination[3] = source[2];
        }
        let f0 = ((survey.coat_ior - 1.) / (survey.coat_ior + 1.)).powi(2);
        Ok(Self {
            parameters: [
                survey.probe_rectangle,
                [
                    survey.normal_strength,
                    survey.coat_weight,
                    survey.coat_roughness,
                    f0,
                ],
                survey.tabletop_bounds,
            ],
        })
    }
}
