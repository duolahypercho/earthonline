//! Install Blender's grass and citrus at the height of their individual surveyed roots.
use super::House;
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, KitLibrary, glb};
use eo_render::authored::MaterialTable;
use eo_render::scene::{GpuObject, ObjectKind, Scene, THIN_FIBRE_BIT, Vertex};
use glam::{DVec2, Mat3, Vec3};
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
pub(super) struct Planting {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    objects: Vec<GpuObject>,
}

impl Planting {
    pub fn load(
        dir: &Path,
        root: &Path,
        house: &House,
        ground: &dyn Fn(f64, f64) -> f64,
        street: Option<&eo_street::StreetWorld>,
        kit: &KitLibrary,
        materials: &mut MaterialTable,
    ) -> Result<Self> {
        let asset: AssetRef = serde_json::from_slice(&std::fs::read(
            dir.join("courtyard-planting.provenance.json"),
        )?)?;
        let bytes = glb::read_verified(&asset.path(root), &asset)?;
        let (model, anchors) = glb::cook_glb_with_uv0(&bytes)?;
        ensure!(
            model.positions.len() == anchors.len(),
            "missing plant roots"
        );
        ensure!(
            model.indices.chunks_exact(3).all(|tri| {
                let a = anchors[tri[0] as usize];
                a == anchors[tri[1] as usize] && a == anchors[tri[2] as usize]
            }),
            "each planting triangle must share one ground anchor"
        );
        let across = house.across.normalize();
        let depth = across.perp() * house.across.perp_dot(house.back).signum();
        let xyz = |p: DVec2| Vec3::new(p.x as f32, 0., p.y as f32);
        let metric = Mat3::from_cols(xyz(across), Vec3::Y, xyz(depth));

        let mut roots = HashMap::new();
        let mut planting = Self::default();
        let mut visible = Vec::with_capacity(anchors.len());
        for ((offset, &normal), uv) in model.positions.iter().zip(&model.normals).zip(anchors) {
            let (point, allowed) = *roots.entry(uv.map(f32::to_bits)).or_insert_with(|| {
                let coord =
                    house.origin + house.across * f64::from(uv[0]) + house.back * f64::from(uv[1]);
                let above_ground = ground(coord.x, coord.y) + 0.014;
                let inside = uv[1] >= 13.
                    && !street.is_some_and(|s| {
                        s.buildings.contains(coord.x, coord.y)
                            || s.paved_layer_at(coord.x, coord.y).is_some()
                    });
                (
                    Vec3::new(coord.x as f32, above_ground as f32, coord.y as f32),
                    inside,
                )
            });
            visible.push(allowed);
            planting.vertices.push(Vertex {
                pos: (point + metric * Vec3::from(*offset)).to_array(),
                normal: glb::pack_normal(metric * glb::unpack_normal(normal)),
                color: 0xffffffff,
            });
        }
        for part in &model.parts {
            let start = part.first_index as usize;
            let first = planting.indices.len() as u32;
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = -lo;
            for tri in model.indices[start..start + part.index_count as usize].chunks_exact(3) {
                // Every blade belongs to one root, and every branch to the same trunk.
                if visible[tri[0] as usize] {
                    for &index in tri {
                        let p = Vec3::from(planting.vertices[index as usize].pos);
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                    planting.indices.extend_from_slice(tri);
                }
            }
            let count = planting.indices.len() as u32 - first;
            if count == 0 {
                continue;
            }
            let material = &model.materials[part.material as usize];
            let mut object = GpuObject::new(
                ObjectKind::Model,
                lo.to_array(),
                hi.to_array(),
                first,
                count,
                0,
            );
            object.material = materials.resolve(kit, material);
            if !material.name.starts_with("citrus-bark@") {
                object.kind |= THIN_FIBRE_BIT;
            }
            planting.objects.push(object);
        }
        Ok(planting)
    }

    pub fn append(&self, scene: &mut Scene) {
        let v = scene.vertices.len() as i32;
        let i = scene.indices.len() as u32;
        scene.vertices.extend_from_slice(&self.vertices);
        scene.indices.extend_from_slice(&self.indices);
        scene
            .objects
            .extend(self.objects.iter().map(|&o| GpuObject {
                first_index: o.first_index + i,
                base_vertex: v,
                ..o
            }));
    }
}
