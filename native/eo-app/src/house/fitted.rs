//! The measured Blender dining furniture and its native PBR texture layers.
use super::{Finishes, House, Point};
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, CookedModel, glb, kit::BakedMaps};
use eo_render::{
    authored::{GpuMaterial, MaterialTable},
    scene::{self, GpuObject, ObjectKind, Scene, Vertex},
};
use glam::Mat3;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct Surface {
    name: String,
    layer: Option<usize>,
    normal_strength: f32,
    garden: bool,
    thin: bool,
}
#[derive(Deserialize)]
pub(super) struct Recipe {
    #[serde(flatten)]
    geometry: AssetRef,
    house_sha256: String,
    maps: [AssetRef; 3],
    layers: usize,
    surfaces: Vec<Surface>,
    pub replaced_boxes: Vec<usize>,
    pub boxes: Vec<[Point; 8]>,
    pub replaced_ramps: Vec<usize>,
    pub ramps: Vec<[Point; 4]>,
}

impl Recipe {
    pub fn read(root: &Path, dir: &Path, shell_hash: &str) -> Result<(Self, Vec<BakedMaps>)> {
        let spec: Self =
            serde_json::from_slice(&std::fs::read(dir.join("fitted-room.provenance.json"))?)?;
        ensure!(
            spec.house_sha256 == shell_hash && (1..=64).contains(&spec.layers),
            "mismatched fitted room and shell"
        );
        let [color, normal, orm] = [&spec.maps[0], &spec.maps[1], &spec.maps[2]]
            .map(|asset| Finishes::read_pixels(root, asset, spec.layers));
        let (color, normal, orm) = (color?, normal?, orm?);
        let res = eo_assets::kit::HIGH_RES;
        let len = res * res * 4;
        let pages = (0..spec.layers)
            .map(|i| {
                let range = i * len..(i + 1) * len;
                BakedMaps {
                    res,
                    metres_per_tile: 1.,
                    albedo: color[range.clone()].to_vec(),
                    normal: normal[range.clone()].to_vec(),
                    orm: orm[range].to_vec(),
                }
            })
            .collect();
        Ok((spec, pages))
    }

    pub fn load(&self, root: &Path, table: &mut MaterialTable, first_layer: usize) -> Result<Room> {
        let data = glb::read_verified(&self.geometry.path(root), &self.geometry)?;
        let (model, uv) = glb::cook_glb_with_uv0(&data)?;
        ensure!(
            self.surfaces.len() == model.materials.len()
                && uv
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && v.abs() < 65504.),
            "invalid fitted material coordinates"
        );
        let mut draws = Vec::new();
        for (spec, mat) in self.surfaces.iter().zip(&model.materials) {
            ensure!(
                spec.name == mat.name && spec.layer.is_none_or(|l| l < self.layers),
                "invalid fitted surface {}",
                spec.name
            );
            let layer = spec.layer.map_or(-1, |l| (first_layer + l) as i32);
            let id = table.named(
                &format!("21-beaver-fitted/{}", mat.name),
                GpuMaterial {
                    base: mat.base_color,
                    flags: 0,
                    layer,
                    params: [mat.roughness, mat.metallic, spec.normal_strength, 1.],
                    emissive: [
                        mat.emissive[0] * mat.emissive_strength,
                        mat.emissive[1] * mat.emissive_strength,
                        mat.emissive[2] * mat.emissive_strength,
                        0.,
                    ],
                    ..GpuMaterial::vertex_colour()
                },
            );
            draws.push((id, spec.garden, spec.thin));
        }
        Ok(Room { model, uv, draws })
    }
}

pub(super) struct Room {
    model: CookedModel,
    uv: Vec<[f32; 2]>,
    draws: Vec<(u32, bool, bool)>,
}

impl Room {
    pub fn append(&self, house: &House, scene: &mut Scene, basis: Mat3) {
        let base = scene.vertices.len() as i32;
        let first = scene.indices.len() as u32;
        for ((pos, normal), uv) in self
            .model
            .positions
            .iter()
            .zip(&self.model.normals)
            .zip(&self.uv)
        {
            let [u, v] = uv.map(|s| u32::from(eo_farshore::half::f16::from_f32(s).to_bits()));
            scene.vertices.push(Vertex {
                pos: house
                    .point(pos[0].into(), pos[1].into(), pos[2].into())
                    .as_vec3()
                    .to_array(),
                normal: glb::pack_normal(basis * glb::unpack_normal(*normal)),
                color: u | v << 16,
            });
        }
        scene.indices.extend_from_slice(&self.model.indices);
        for part in &self.model.parts {
            let (min, max) = house.bounds(part.min, part.max);
            let mut obj = GpuObject::new(
                ObjectKind::Model,
                min,
                max,
                first + part.first_index,
                part.index_count,
                base,
            );
            let (id, garden, thin) = self.draws[part.material as usize];
            obj.material = id;
            obj.kind |= scene::FITTED_ROOM_BIT;
            if !garden && house.downstairs_light {
                obj.kind |= scene::DOWNSTAIRS_LIGHT_BIT;
            }
            if thin {
                obj.kind |= scene::THIN_FIBRE_BIT;
            }
            scene.objects.push(obj);
        }
    }
}

/// Exact component names emitted by the canonical furnished house builder.
pub(super) fn replaced(mesh: &str) -> bool {
    let group = mesh.split_once(" - ").map_or(mesh, |(group, _)| group);
    group.starts_with("dining-chair")
        || matches!(
            group,
            "oak-dining-table"
                | "dining-pendant"
                | "painted-fruit-bowl"
                | "red-apple"
                | "red-apple-small"
                | "orchard-pear"
                | "striped-table-runner"
                | "open-recipe-book"
                | "stoneware-mug"
                | "stoneware-mug-rear"
                | "stair-flight-1"
                | "base-cabinets"
                | "oven-range"
                | "range-hood"
                | "brass-faucet"
                | "enamel-stockpot"
                | "chopping-board"
                | "kitchen-shelves"
                | "kitchen-tiles"
                | "pantry-canister-1"
                | "pantry-canister-2"
                | "pantry-canister-3"
                | "mixing-bowl"
                | "sink-basin"
                | "stacked-plates"
                | "kitchen-plant"
                | "enamel-refrigerator"
                | "oak-sideboard"
                | "window-seat"
                | "roman-shade"
        )
}
