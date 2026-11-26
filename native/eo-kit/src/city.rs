//! The whole merged city as the kit planner reads it: every building's loaded
//! record, the street index over the extract's ways and the footprint
//! occlusion index over every record.

use crate::pipeline::{KitWorld, LoadedRecord, SourceRecord, loaded_record};
use crate::street::{OcclusionIndex, StreetIndex};
use crate::tenancy::Tenancy;
use std::collections::HashSet;
use std::path::Path;

pub struct KitCity {
    pub world: KitWorld,
    /// Per eo-data building index.
    pub records: Vec<LoadedRecord>,
    /// Building indices the authored registry draws (never kit).
    pub excluded: HashSet<usize>,
    pub build_ms: f64,
}

/// `native/eo-kit/data/kit-exclusions.json`: the authored registry's ids that
/// this build draws itself. The file (exported from the browser version)
/// lists the ids its Blender builds and its code-built modules drew. Native
/// draws the Blender builds (`native/assets/blender-buildings/b<id>.js`,
/// eo-assets `manifest::GENERATED_DIR`); the code-built modules were never
/// ported, so an id with no build here is left to the kit. Excluding it
/// left nothing but the far layer's flat box in its place at every
/// distance: the Children's Creativity Museum on 4th Street, among six.
/// A landmark that draws such a record still claims it (eo-app main.rs).
pub fn load_exclusions() -> HashSet<String> {
    let native = eo_data::native_dir();
    let path = native.join("eo-kit/data/kit-exclusions.json");
    let Ok(text) = std::fs::read_to_string(path) else { return HashSet::new() };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let builds = native.join("assets/blender-buildings");
    v["ids"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s.as_str()).filter(|id| builds.join(format!("b{}.js", id.trim_start_matches("sf-building-"))).is_file()).map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

impl KitCity {
    pub fn build(city: &eo_data::City, tenancy: &Tenancy, exclusions: &HashSet<String>) -> Self {
        let t = std::time::Instant::now();
        let records: Vec<LoadedRecord> = city
            .buildings
            .iter()
            .map(|b| {
                let src = SourceRecord {
                    key: &b.id,
                    points: &b.points,
                    building: &b.tags.building,
                    amenity: &b.tags.amenity,
                    name: &b.tags.name,
                    levels: b.tags.levels,
                    height_raw: b.tags.height,
                    area_raw: b.tags.area,
                    height: b.height as f64,
                };
                loaded_record(&src, tenancy.units.get(&b.id).cloned().unwrap_or_default())
            })
            .collect();
        let streets = StreetIndex::build(city.roads.iter().map(|r| (r.highway.as_str(), r.points64.as_slice())));
        let occlusion = OcclusionIndex::build(records.iter().enumerate().map(|(i, r)| (i as u32, r.polygon.clone(), r.height)));
        let excluded = records.iter().enumerate().filter(|(_, r)| exclusions.contains(&r.id)).map(|(i, _)| i).collect();
        // the Presidio rule's records: the footprint's centroid inside the post's
        // boundary, or a record the rule documents by id (presidio.rs `on_post`)
        let mut records = records;
        let ring = crate::presidio::boundary(&city.meta);
        for r in records.iter_mut() {
            let n = r.polygon.len().max(1) as f64;
            let (x, z) = r.polygon.iter().fold((0.0, 0.0), |a, p| (a.0 + p.x, a.1 + p.z));
            r.presidio = crate::presidio::on_post(&ring, &r.id, x / n, z / n);
        }
        Self { world: KitWorld { streets, occlusion }, records, excluded, build_ms: t.elapsed().as_secs_f64() * 1e3 }
    }

    /// Load the city, tenancy and exclusions from the repo and build.
    pub fn load(repo_root: &Path) -> Result<(eo_data::City, Self), String> {
        let paths = eo_data::DataPaths { repo_root: repo_root.to_path_buf() };
        let city = eo_data::City::load(&paths).map_err(|e| format!("{e:#}"))?;
        let tenancy = Tenancy::load(repo_root)?;
        let kc = Self::build(&city, &tenancy, &load_exclusions());
        Ok((city, kc))
    }
}
