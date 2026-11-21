//! UVs of the six measured faces of the oak chopping board in the fitted room.
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, CookedModel};
use glam::{Vec3, Vec4};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub(super) struct Board {
    #[serde(flatten)]
    pub asset: AssetRef,
    pub material: String,
    triangles: usize,
    pub fitted_geometry_sha256: String,
    pub camera_glare: f32,
    local_from_parcel: [[f32; 4]; 3],
    min: [f32; 3],
    size: [f32; 3],
    widths: [u32; 3],
}

impl Board {
    pub fn apply(&self, model: &mut CookedModel, uv: &mut Vec<[f32; 2]>) -> Result<()> {
        let material = model.materials.iter().position(|m| m.name == self.material);
        let part = model
            .parts
            .iter_mut()
            .find(|p| Some(p.material as usize) == material)
            .ok_or_else(|| anyhow::anyhow!("missing fitted chopping board"))?;
        let size = Vec3::from(self.size);
        ensure!(
            size.is_finite()
                && size.min_element() > 0.
                && self
                    .local_from_parcel
                    .iter()
                    .flatten()
                    .all(|x| x.is_finite())
                && self.widths.iter().all(|&w| (6..512).contains(&w))
                && self.widths.iter().sum::<u32>() == 512,
            "invalid board projection"
        );
        let rows = self.local_from_parcel.map(Vec4::from);
        let bounds_min = Vec3::from(self.min);
        let mut mapped = HashMap::new();
        let first = part.first_index as usize;
        let end = first + part.index_count as usize;
        let mut other = Vec::new();
        let mut board = Vec::new();
        for tri in model.indices[first..end].chunks_exact(3) {
            let ids = [tri[0], tri[1], tri[2]];
            let local = ids.map(|id| {
                let p = Vec3::from(model.positions[id as usize]).extend(1.);
                Vec3::from(rows.map(|r| r.dot(p))) - bounds_min
            });
            // The same oak also occurs on a shelf and the upper stair. Split
            // only the triangles of the measured board from the material batch.
            if !local.iter().all(|p| {
                p.cmpge(Vec3::splat(-0.00002)).all() && p.cmple(size + Vec3::splat(0.00002)).all()
            }) {
                other.extend_from_slice(tri);
                continue;
            }
            let normal = (local[1] - local[0]).cross(local[2] - local[0]);
            let n = normal.abs();
            let axis = if n.x >= n.y && n.x >= n.z {
                0
            } else if n.y >= n.z {
                1
            } else {
                2
            };
            let positive = normal[axis] > 0.;
            let (a, b) = [(1, 2), (0, 2), (0, 1)][axis];
            for (corner, &id) in ids.iter().enumerate() {
                let index = mapped.entry((id, axis, positive)).or_insert_with(|| {
                    let t = (local[corner] / size).clamp(Vec3::ZERO, Vec3::ONE);
                    let left: u32 = self.widths[..axis].iter().sum();
                    // Cycles bakes with V up; the image and glTF use V down.
                    uv.push([
                        (left as f32 + 2.5 + (self.widths[axis] - 5) as f32 * t[a]) / 512.,
                        1. - (256. * u8::from(positive) as f32 + 2.5 + 251. * t[b]) / 512.,
                    ]);
                    let new = model.positions.len() as u32;
                    model.positions.push(model.positions[id as usize]);
                    model.normals.push(model.normals[id as usize]);
                    new
                });
                board.push(*index);
            }
        }
        ensure!(
            board.len() == 3 * self.triangles && !board.is_empty(),
            "the registered board triangles do not match"
        );
        let bounds = |ids: &[u32]| {
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for &i in ids {
                let p = Vec3::from(model.positions[i as usize]);
                lo = lo.min(p);
                hi = hi.max(p);
            }
            (lo.to_array(), hi.to_array())
        };
        let pivot = first + other.len();
        model.indices[first..pivot].copy_from_slice(&other);
        model.indices[pivot..end].copy_from_slice(&board);
        let (min, max) = bounds(&board);
        let mut separated = part.clone();
        separated.first_index = pivot as u32;
        separated.index_count = board.len() as u32;
        separated.material = model.materials.len() as u32;
        (separated.min, separated.max) = (min, max);
        part.index_count = other.len() as u32;
        if !other.is_empty() {
            (part.min, part.max) = bounds(&other);
        }
        model.parts.retain(|p| p.index_count > 0);
        model.parts.push(separated);
        Ok(())
    }
}
