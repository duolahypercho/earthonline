//! Fitted clay courses for the installed private stair at 21 Beaver Street.
use super::{Finishes, House, fitted::Geometry};
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, glb, kit::BakedMaps};
use eo_render::{
    authored::{GpuMaterial, MaterialTable},
    scene::Scene,
};
use glam::{Mat3, Vec3};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct Course {
    asset: AssetRef,
    triangles: u64,
}

#[derive(Deserialize)]
struct Finish {
    material: String,
    asset: AssetRef,
}

#[derive(Deserialize)]
pub(super) struct Recipe {
    house_sha256: String,
    tiles: usize,
    courses: Vec<Course>,
    maps: Vec<Finish>,
    light: AssetRef,
    pier_bases: [Pier; 2],
}

#[derive(Clone, Deserialize)]
struct Pier {
    plan: [[f64; 2]; 4],
}

pub(super) struct Stoop {
    landing: Geometry,
    courses: Vec<Geometry>,
    piers: [Pier; 2],
}

impl Recipe {
    pub fn read(root: &Path, directory: &Path, house: &str) -> Result<(Self, Vec<BakedMaps>)> {
        let recipe: Self = serde_json::from_slice(&std::fs::read(
            directory.join("stoop/stoop.provenance.json"),
        )?)?;
        ensure!(
            recipe.house_sha256 == house
                && recipe.courses.len() >= 2
                && recipe.tiles == 5 * recipe.courses.len()
                && recipe.maps.len() == 2
                && recipe.maps[0].material != recipe.maps[1].material,
            "invalid fitted entrance recipe"
        );
        ensure!(
            recipe
                .pier_bases
                .iter()
                .flat_map(|pier| &pier.plan)
                .all(|[x, z]| (4.9..=7.0).contains(x) && (-0.9..=0.5).contains(z)),
            "pier outside the installed entrance"
        );
        let mut maps: Vec<_> = recipe
            .maps
            .iter()
            .map(|m| Finishes::read_maps(root, &m.asset, 3))
            .collect::<Result<_>>()?;
        maps.push(Finishes::read_maps(root, &recipe.light, 3)?);
        Ok((recipe, maps))
    }

    pub fn extend_piers(
        &self,
        model: &mut eo_assets::CookedModel,
        grade: impl Fn(f64, f64) -> f64,
    ) {
        let mut visited = vec![false; model.positions.len()];
        for part in &mut model.parts {
            if !model.materials[part.material as usize]
                .name
                .starts_with("painted-stucco@")
            {
                continue;
            }
            for &idx in &model.indices
                [part.first_index as usize..(part.first_index + part.index_count) as usize]
            {
                let p = &mut model.positions[idx as usize];
                if (0.0..=0.0251).contains(&p[1])
                    && self.pier_bases.iter().any(|pier| {
                        let [x, y] = pier.plan[0];
                        (x - 0.001..=pier.plan[2][0] + 0.001).contains(&f64::from(p[0]))
                            && (y - 0.001..=pier.plan[2][1] + 0.001).contains(&f64::from(p[2]))
                    })
                    && !visited[idx as usize]
                {
                    p[1] += grade(p[0].into(), p[2].into()) as f32;
                    visited[idx as usize] = true;
                }
                part.min[1] = part.min[1].min(p[1]);
            }
            model.min[1] = model.min[1].min(part.min[1]);
        }
    }

    pub fn load(&self, root: &Path, table: &mut MaterialTable, first: usize) -> Result<Stoop> {
        let slots: Vec<_> = self
            .maps
            .iter()
            .enumerate()
            .map(|(i, m)| {
                table.named(
                    &format!("21-beaver-fitted/Entrance/{}", m.material),
                    GpuMaterial {
                        base: [1.; 4],
                        params: [1., 0., 1., (self.maps.len() - i) as f32],
                        flags: eo_render::authored::F_STOOP_LIGHT,
                        layer: (first + i) as i32,
                        ..GpuMaterial::vertex_colour()
                    },
                )
            })
            .collect();
        let read = |course: &Course, landing: bool| -> Result<Geometry> {
            let bytes = glb::read_verified(&course.asset.path(root), &course.asset)?;
            let (model, uv, visibility) =
                glb::cook_glb_with_visibility(&bytes, &Default::default())?;
            let z = if landing { (0., 0.36) } else { (-0.36, 0.) };
            ensure!(
                model.triangles() == course.triangles
                    && model.min[0] >= 5.14
                    && model.max[0] <= 6.76
                    && model.min[1] >= -0.15
                    && model.max[1] <= 0.012
                    && model.min[2] >= z.0 - 0.001
                    && model.max[2] <= z.1 + 0.001
                    && uv.len() == model.positions.len()
                    && visibility.len() == uv.len()
                    && uv.iter().flatten().all(|v| (0.0..=1.0).contains(v)),
                "invalid fired-clay course"
            );
            let draws = model
                .materials
                .iter()
                .map(|m| {
                    let id = self
                        .maps
                        .iter()
                        .position(|p| p.material == m.name)
                        .ok_or_else(|| anyhow::anyhow!("unknown kiln surface {}", m.name))?;
                    Ok((slots[id], true, false))
                })
                .collect::<Result<_>>()?;
            Ok(Geometry {
                model,
                uv,
                draws,
                visibility,
            })
        };
        let landing = read(&self.courses[0], true)?;
        let courses = self.courses[1..]
            .iter()
            .map(|c| read(c, false))
            .collect::<Result<_>>()?;
        Ok(Stoop {
            landing,
            courses,
            piers: self.pier_bases.clone(),
        })
    }
}

impl Stoop {
    pub fn replaces(name: &str) -> bool {
        name.starts_with("entrance-casing - terracotta-paver@")
    }

    pub fn append(&self, house: &House, scene: &mut Scene, basis: Mat3) {
        self.landing.append(house, scene, basis);
        for (i, [_, b]) in house.steps.iter().enumerate() {
            let offset = Vec3::new(0., b[1] as f32, b[2] as f32);
            self.courses[i % self.courses.len()].append_at(house, scene, basis, offset);
        }
    }

    pub fn pier_plans(&self) -> [[[f64; 2]; 4]; 2] {
        self.piers.each_ref().map(|pier| pier.plan)
    }
}
