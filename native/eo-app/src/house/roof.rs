//! Substitute the two fitted roof coverings before uploading the furnished house.
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, CookedModel, glb};
use std::path::Path;

pub fn old_clay(name: &str) -> bool {
    name.starts_with("terrace-roof - fired-clay@")
        || name.starts_with("rounded-stair-bay - fired-clay@")
}

pub struct RoofUv {
    start: usize,
    texcoords: Vec<[u8; 2]>,
}

impl RoofUv {
    pub fn get(&self, vertex: usize) -> Option<[u8; 2]> {
        self.texcoords.get(vertex.checked_sub(self.start)?).copied()
    }
}

pub fn install(root: &Path, directory: &Path, house: &mut CookedModel) -> Result<RoofUv> {
    let asset: AssetRef = serde_json::from_slice(&std::fs::read(
        directory.join("mission-roof.provenance.json"),
    )?)?;
    let raw = glb::read_verified(&asset.path(root), &asset)?;
    let (roof, uv) = glb::cook_glb_with_uv0(&raw)?;
    ensure!(
        roof.parts.len() == 1
            && roof.materials.len() == 1
            && roof.materials[0].name == "mission-roof-clay"
            && roof.min[1] > 9.9
            && roof.max[1] < 11.5,
        "unexpected fitted roof layout"
    );
    ensure!(
        uv.iter().flatten().all(|v| (0.0..=1.0).contains(v)),
        "roof texture coordinate lies outside the clay atlas"
    );
    let start = house.positions.len();
    let mut part = roof.parts[0];
    part.material = house.materials.len() as u32;
    part.first_index += house.indices.len() as u32;
    house.materials.extend(roof.materials);
    house
        .indices
        .extend(roof.indices.into_iter().map(|i| i + start as u32));
    house.positions.extend(roof.positions);
    house.normals.extend(roof.normals);
    house.parts.push(part);
    house.primitives += roof.primitives;
    for axis in 0..3 {
        house.min[axis] = house.min[axis].min(roof.min[axis]);
        house.max[axis] = house.max[axis].max(roof.max[axis]);
    }
    Ok(RoofUv {
        start,
        texcoords: uv
            .into_iter()
            .map(|pair| pair.map(|v| (v * 255.0).round() as u8))
            .collect(),
    })
}
