//! Surveyed front planter surfaces and the separate rosettes and grass clumps.
use super::{Finishes, fitted::Geometry};
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, glb, kit::BakedMaps};
use eo_render::authored::{GpuMaterial, MaterialTable};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub(super) struct Garden {
    #[serde(flatten)]
    model: AssetRef,
    maps: Vec<AssetRef>,
    materials: Vec<String>,
    triangles: u64,
    house_sha256: String,
    replaced: Vec<String>,
}

#[derive(Deserialize)]
struct DaylightRecipe {
    #[serde(flatten)]
    data: AssetRef,
    garden_sha256: String,
    hours: [u8; 6],
    across: [f32; 3],
    depth: [f32; 2],
}

impl Garden {
    pub fn read_daylight(
        &self,
        root: &Path,
        directory: &Path,
    ) -> Result<eo_render::front_daylight::FrontDaylight> {
        let recipe: DaylightRecipe = serde_json::from_slice(&std::fs::read(
            directory.join("front-daylight.provenance.json"),
        )?)?;
        ensure!(
            recipe.garden_sha256 == self.model.sha256
                && recipe.hours == [8, 10, 12, 14, 16, 18]
                && recipe.across == [0.68, 2.62, 4.56]
                && recipe.depth == [-1.78, -0.52],
            "front daylight does not match the installed survey"
        );
        let data = glb::read_verified(&recipe.data.path(root), &recipe.data)?;
        eo_render::front_daylight::FrontDaylight::from_bytes(&data)
    }

    pub fn read(root: &Path, directory: &Path, shell: &str) -> Result<(Self, Vec<BakedMaps>)> {
        let garden: Self = serde_json::from_slice(&std::fs::read(
            directory.join("street-garden.provenance.json"),
        )?)?;
        ensure!(
            garden.house_sha256 == shell
                && garden.maps.len() == 3
                && garden.materials.len() == garden.maps.len()
                && garden.replaced.len() == 14
                && garden
                    .replaced
                    .iter()
                    .all(|n| n.starts_with("front-dry-garden - "))
                && garden.replaced.windows(2).all(|p| p[0] < p[1]),
            "front garden does not match the surveyed house"
        );
        let maps = garden
            .maps
            .iter()
            .map(|a| Finishes::read_maps(root, a, 3))
            .collect::<Result<_>>()?;
        Ok((garden, maps))
    }

    pub fn replaces(&self, node: &str) -> bool {
        self.replaced
            .binary_search_by(|n| n.as_str().cmp(node))
            .is_ok()
    }

    pub fn load(
        &self,
        root: &Path,
        table: &mut MaterialTable,
        first: usize,
        daylight: bool,
    ) -> Result<Geometry> {
        let bytes = glb::read_verified(&self.model.path(root), &self.model)?;
        let (model, uv, visibility) = glb::cook_glb_with_visibility(&bytes, &Default::default())?;
        ensure!(
            model.triangles() == self.triangles
                && visibility.iter().any(|&v| v < 0.30)
                && visibility.iter().any(|&v| v > 0.95)
                && model
                    .materials
                    .iter()
                    .map(|m| &m.name)
                    .eq(self.materials.iter())
                && model.min[0] >= 0.
                && model.max[0] < 5.06
                && model.max[2] < 0.10
                && model.min[2] > -2.55
                && uv
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && v.abs() < 65504.),
            "unexpected front bed extent, surface or UV"
        );
        let mut draws = Vec::new();
        for (i, mat) in model.materials.iter().enumerate() {
            let plant = mat.name == "Agave wax, fibres and seed";
            let material = table.named(
                &format!("21-beaver-fitted/Street garden/{}", mat.name),
                GpuMaterial {
                    base: [1.; 4],
                    layer: (first + i) as i32,
                    params: [1., 1., if plant { 0.65 } else { 0.82 }, 1.],
                    flags: if daylight {
                        eo_render::authored::F_FRONT_DAYLIGHT
                    } else {
                        0
                    },
                    ..GpuMaterial::vertex_colour()
                },
            );
            draws.push((material, true, false));
        }
        Ok(Geometry {
            model,
            uv,
            draws,
            visibility,
        })
    }
}
