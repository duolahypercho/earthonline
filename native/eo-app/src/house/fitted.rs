//! The measured Blender dining furniture and its native PBR texture layers.
use super::{Finishes, House, Point, fitted_board::Board};
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, CookedModel, glb, kit::BakedMaps};
use eo_render::{
    authored::{GpuMaterial, MaterialTable},
    scene::{self, GpuObject, ObjectKind, Scene, Vertex},
};
use glam::Mat3;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Deserialize)]
struct Surface {
    name: String,
    layer: Option<usize>,
    normal_strength: f32,
    garden: bool,
    thin: bool,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
enum Group {
    Rail = 1,
    Bowl = 2,
    Boards = 3,
    Chairs = 4,
    Linen = 5,
    Shade = 6,
    Reflector = 7,
    Tile = 8,
    Kitchen = 9,
    Worktop = 10,
    Fitting = 11,
    Shared = 12,
    Board = 13,
    Grout = 14,
}

#[derive(Deserialize)]
struct Coating {
    group: Group,
    coat: f32,
    roughness: f32,
}

#[derive(Deserialize)]
struct FinishSurvey {
    #[serde(flatten)]
    asset: AssetRef,
    fitted_geometry_sha256: String,
    house_sha256: String,
    table_parcel_bounds: [f32; 4],
    probe_blender_xyz: [[f32; 3]; 8],
    materials: BTreeMap<String, Coating>,
}

#[derive(Deserialize)]
struct KitchenSurvey {
    #[serde(flatten)]
    asset: AssetRef,
    fitted_geometry_sha256: String,
    house_sha256: String,
    kitchen_frame: [[f32; 4]; 3],
    shared_bounds: [[f32; 3]; 2],
    board: Board,
    materials: BTreeMap<String, Group>,
}

#[derive(Deserialize)]
pub(super) struct Recipe {
    #[serde(skip)]
    finish: Option<FinishSurvey>,
    #[serde(skip)]
    kitchen: Option<KitchenSurvey>,
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
    pub fn read(
        root: &Path,
        dir: &Path,
        shell_hash: &str,
        finishes: bool,
        kitchen: bool,
    ) -> Result<(Self, Vec<BakedMaps>)> {
        let mut spec: Self =
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
        let mut pages: Vec<_> = (0..spec.layers)
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
        if finishes {
            let survey: FinishSurvey = serde_json::from_slice(&std::fs::read(
                dir.join("fitted-finish-light.provenance.json"),
            )?)?;
            let [left, front, right, back] = survey.table_parcel_bounds;
            ensure!(
                survey.fitted_geometry_sha256 == spec.geometry.sha256
                    && survey.house_sha256 == shell_hash
                    && left < right
                    && front < back
                    && survey.probe_blender_xyz[..3]
                        .windows(2)
                        .all(|p| p[0][2] < p[1][2])
                    && survey.materials.iter().all(|(name, m)| spec
                        .surfaces
                        .iter()
                        .any(|s| &s.name == name)
                        && (0.0..=1.0).contains(&m.coat)
                        && (0.0..=1.0).contains(&m.roughness)),
                "invalid fitted finish survey"
            );
            // The last fitted layer always belongs to the common floorboards.
            pages.insert(
                spec.layers - 1,
                Finishes::read_maps(root, &survey.asset, 3)?,
            );
            spec.finish = Some(survey);
        }
        if kitchen {
            let survey: KitchenSurvey =
                serde_json::from_slice(&std::fs::read(dir.join("kitchen-light.provenance.json"))?)?;
            let [along, across, heights] = survey.kitchen_frame;
            ensure!(
                survey.fitted_geometry_sha256 == spec.geometry.sha256
                    && survey.house_sha256 == shell_hash
                    && survey.board.fitted_geometry_sha256 == spec.geometry.sha256
                    && survey.board.camera_glare == 0.
                    && (0..3).all(|i| survey.shared_bounds[0][i] < survey.shared_bounds[1][i])
                    && survey.kitchen_frame.iter().flatten().all(|v| v.is_finite())
                    && along[2].abs() > 0.
                    && across[0].abs() > 0.
                    && heights[2] <= heights[0]
                    && heights[0] < heights[1]
                    && heights[1] < heights[3]
                    && !survey.materials.is_empty()
                    && survey.materials.iter().all(|(name, group)| *group as u8
                        >= Group::Shade as u8
                        && spec.surfaces.iter().any(|s| &s.name == name)
                        && spec
                            .finish
                            .as_ref()
                            .is_none_or(|f| !f.materials.contains_key(name))),
                "invalid fitted kitchen light survey"
            );
            pages.insert(
                pages.len() - 1,
                Finishes::read_maps(root, &survey.asset, 3)?,
            );
            pages.insert(
                pages.len() - 1,
                Finishes::read_maps(root, &survey.board.asset, 3)?,
            );
            spec.kitchen = Some(survey);
        }
        Ok((spec, pages))
    }

    pub fn load(&self, root: &Path, table: &mut MaterialTable, first_layer: usize) -> Result<Room> {
        let data = glb::read_verified(&self.geometry.path(root), &self.geometry)?;
        let (mut model, mut uv) = glb::cook_glb_with_uv0(&data)?;
        if let Some(survey) = &self.kitchen {
            survey.board.apply(&mut model, &mut uv)?;
        }
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
            let coat = self
                .finish
                .as_ref()
                .and_then(|survey| survey.materials.get(&spec.name));
            // Fitted materials use authored UVs. The repeat slot addresses their
            // light atlas, and the two kit finish slots hold group and coat roughness.
            let kitchen = self
                .kitchen
                .as_ref()
                .and_then(|k| k.materials.get(&spec.name));
            let optical_layer = first_layer + self.layers - 1;
            let survey_layer = if kitchen.is_some() {
                (optical_layer + usize::from(self.finish.is_some())) as f32
            } else {
                coat.map_or(1., |_| optical_layer as f32)
            };
            let group = kitchen.copied().or_else(|| coat.map(|c| c.group));
            let id = table.named(
                &format!("21-beaver-fitted/{}", mat.name),
                GpuMaterial {
                    base: mat.base_color,
                    flags: 0,
                    layer,
                    params: [
                        mat.roughness,
                        mat.metallic,
                        spec.normal_strength,
                        survey_layer,
                    ],
                    thin_absorption: group.map_or(0., |g| g as u8 as f32),
                    rough_flatten: coat.map_or(0., |c| c.roughness),
                    emissive: [
                        mat.emissive[0] * mat.emissive_strength,
                        mat.emissive[1] * mat.emissive_strength,
                        mat.emissive[2] * mat.emissive_strength,
                        coat.map_or(0., |c| c.coat),
                    ],
                    ..GpuMaterial::vertex_colour()
                },
            );
            draws.push((id, spec.garden, spec.thin));
        }
        if let Some(kitchen) = &self.kitchen {
            let board_source = self
                .surfaces
                .iter()
                .position(|s| s.name == kitchen.board.material)
                .ok_or_else(|| anyhow::anyhow!("missing original oak material"))?;
            let (source_id, garden, thin) = draws[board_source];
            let mut oak = table.gpu[source_id as usize];
            oak.base = [1.; 4];
            oak.thin_absorption = Group::Board as u8 as f32;
            oak.layer = oak.params[3] as i32 + 1;
            draws.push((
                table.named("21-beaver-fitted/Chopping board oak", oak),
                garden,
                thin,
            ));
        }
        let finish_parameters = self
            .finish
            .as_ref()
            .map(|s| {
                [
                    s.table_parcel_bounds,
                    [
                        s.probe_blender_xyz[0][2],
                        s.probe_blender_xyz[1][2],
                        s.probe_blender_xyz[2][2],
                        0.,
                    ],
                ]
            })
            .unwrap_or_default();
        Ok(Room {
            model,
            uv,
            draws,
            finish_parameters,
            kitchen_frame: self
                .kitchen
                .as_ref()
                .map(|s| {
                    let [a, b, h] = s.kitchen_frame;
                    let [min, max] = s.shared_bounds.map(|b| [b[0], b[1], b[2], 0.]);
                    [a, b, h, min, max]
                })
                .unwrap_or_default(),
        })
    }
}

pub(super) struct Room {
    pub(super) kitchen_frame: [[f32; 4]; 5],
    pub(super) finish_parameters: [[f32; 4]; 2],
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
