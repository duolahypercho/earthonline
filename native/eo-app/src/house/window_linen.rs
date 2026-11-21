//! The installed Roman blind and the light measured on each side of the flax.
use super::fitted::Geometry;
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, glb};
use eo_render::authored::{GpuMaterial, MaterialTable};
use serde::Deserialize;
use std::path::Path;

const FLAX: &str = "Sun-filtering plainweave flax";
const REPLACED: &str = "Relaxed linen Roman blind";

#[derive(Deserialize)]
pub(super) struct Window {
    #[serde(flatten)]
    model: AssetRef,
    pub maps: AssetRef,
    house_sha256: String,
    fitted_geometry_sha256: String,
    source_sun: [f32; 3],
    weave: [f32; 4],
    triangles: u64,
    materials: Vec<String>,
}

impl Window {
    pub fn read(dir: &Path, shell: &str, room: &str) -> Result<Self> {
        let window: Self =
            serde_json::from_slice(&std::fs::read(dir.join("window-linen.provenance.json"))?)?;
        ensure!(
            window.weave.iter().all(|v| v.is_finite())
                && window.weave[0] > 0.
                && window.weave[3] > window.weave[2]
                && window.house_sha256 == shell
                && window.fitted_geometry_sha256 == room
                && (0.99..1.01).contains(&glam::Vec3::from(window.source_sun).length_squared())
                && window
                    .materials
                    .iter()
                    .filter(|s| s.as_str() == FLAX)
                    .count()
                    == 1,
            "window linen does not match the fitted house"
        );
        Ok(window)
    }

    pub fn replaces(name: &str) -> bool {
        name == REPLACED
    }

    pub fn load(
        &self,
        root: &Path,
        table: &mut MaterialTable,
        layer: usize,
        measured: bool,
    ) -> Result<Geometry> {
        let data = glb::read_verified(&self.model.path(root), &self.model)?;
        let (model, uv) = glb::cook_glb_with_uv0(&data)?;
        ensure!(
            model.triangles() == self.triangles
                && model
                    .materials
                    .iter()
                    .map(|m| &m.name)
                    .eq(self.materials.iter())
                && uv
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "invalid installed linen, hardware or light coordinates"
        );
        let [x, y, z] = self.source_sun;
        let mut draws = Vec::new();
        for mat in &model.materials {
            let cloth = mat.name == FLAX;
            let mut gpu = GpuMaterial {
                base: mat.base_color,
                flags: 0,
                params: [mat.roughness, mat.metallic, 1., 1.],
                ..GpuMaterial::vertex_colour()
            };
            if cloth {
                gpu.base = [0.61, 0.43, 0.24, 1.];
                if measured {
                    // The fitted cloth shader uses the physical noon sun as its
                    // eighth reference direction and this layer for both faces.
                    gpu.thin_absorption = -1.;
                    gpu.layer = layer as i32;
                    gpu.base = self.weave;
                    gpu.emissive = [x, z, -y, 0.];
                }
            }
            let id = table.named(&format!("21-beaver-fitted/Window/{}", mat.name), gpu);
            draws.push((id, false, cloth && !measured));
        }
        Ok(Geometry { model, uv, draws, visibility: Vec::new() })
    }
}
