//! Convert the photographic material pages into the existing kit texture array.
use super::Finishes;
use anyhow::Result;
use eo_assets::{AssetRef, glb::CookedMaterial, kit::BakedMaps};
use eo_render::authored::{F_TEXTURED, GpuMaterial, MaterialTable};
use serde::Deserialize;
use std::path::Path;

pub const NAMES: [&str; 5] = [
    "limewash",
    "aggregate",
    "fired-earth",
    "painted-wood",
    "linen",
];

pub const SITE_NAMES: [&str; 4] = ["apron-concrete", "yard-earth", "trunk-bark", "porch-terracotta"];

pub const FURNITURE_NAMES: [&str; 2] = ["wool-carpet", "joinery"];

#[derive(Deserialize)]
struct Spec {
    #[serde(flatten)]
    asset: AssetRef,
    families: Vec<String>,
    metres_per_tile: f32,
    normal_strength: f32,
    paint_mix: f32,
    #[serde(default)]
    cut_pattern: bool,
}

pub struct Scan {
    spec: Spec,
    pub maps: BakedMaps,
}

impl Scan {
    pub fn load(root: &Path, directory: &Path, name: &str) -> Result<Self> {
        let spec: Spec = serde_json::from_slice(&std::fs::read(
            directory.join(format!("{name}.provenance.json")),
        )?)?;
        let mut maps = Finishes::read_maps(root, &spec.asset, 3)?;
        maps.metres_per_tile = f64::from(spec.metres_per_tile);
        Ok(Self { spec, maps })
    }

    /// The kit registers its complete palette after the house. Keep a private
    /// material for each house finish and bind its scan after those layers.
    pub fn material(
        scans: &[Self],
        mat: &CookedMaterial,
        table: &mut MaterialTable,
        cut_pattern: bool,
    ) -> Option<u32> {
        let family = mat.name.split('@').next()?;
        let (slot, scan) = scans
            .iter()
            .enumerate()
            .find(|(_, scan)| {
                (cut_pattern || !scan.spec.cut_pattern)
                    && scan.spec.families.iter().any(|f| f == family)
            })?;
        let paint = scan.spec.paint_mix;
        let mut base = [1.; 4];
        for (c, source) in base[..3].iter_mut().zip(&mat.base_color) {
            *c = 1. + paint * (source - 1.);
        }
        let surface = GpuMaterial {
            base,
            flags: if super::door_paint(&mat.name) || matches!(family, "woven-carpet" | "mission-roof-clay") || cut_pattern && family == "cotton-coverlet" {
                0
            } else {
                F_TEXTURED
            },
            params: [
                mat.roughness,
                0.,
                scan.spec.normal_strength,
                1. / scan.spec.metres_per_tile,
            ],
            layer: slot as i32,
            ..GpuMaterial::vertex_colour()
        };
        Some(table.named(&format!("21-beaver-scan/{}", mat.name), surface))
    }

    pub fn bind(table: &mut MaterialTable) {
        let first = table.layers.len() as i32;
        for (name, material) in table.names.iter().zip(&mut table.gpu) {
            if name.starts_with("21-beaver-scan/") || name.starts_with("21-beaver-fitted/") && material.layer >= 0 {
                material.layer += first;
            }
        }
    }
}
