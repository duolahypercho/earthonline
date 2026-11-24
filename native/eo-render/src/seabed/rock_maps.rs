//! The packed, attributed coastal scan ships with the game. Its three square
//! panels have the same layout as the existing mipmapped kit material arrays.
use crate::authored::{F_TEXTURED, F_VERTEX_COLOUR, GpuMaterial, MaterialTable};
use anyhow::{Result, ensure};
use eo_assets::kit::{BakedMaps, HIGH_RES};
use std::path::Path;

pub const NAME: &str = "black-point-coastal-rock";

pub fn load(root: &Path) -> Result<BakedMaps> {
    load_named(root, "seaside-rock")
}

pub(crate) fn load_named(root: &Path, name: &str) -> Result<BakedMaps> {
    let dir = root.join("public/assets/shore");
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.provenance.json")))?)?;
    let bytes = std::fs::read(dir.join(format!("{name}.png")))?;
    ensure!(
        record["sha256"] == eo_assets::glb::sha256_hex(&bytes),
        "{name} scan checksum mismatch"
    );
    let (w, h, mut panels) = crate::props_lib::decode_png(&bytes)?;
    ensure!(
        w == HIGH_RES
            && h == 3 * w
            && record["panels"] == serde_json::json!(["albedo", "normal", "arm"]),
        "invalid coastal rock panels"
    );
    let metres = record["size_m"][0].as_f64().unwrap_or_default();
    ensure!(
        metres > 0.0 && record["size_m"][1].as_f64() == Some(metres),
        "invalid coastal rock scale"
    );
    let orm = panels.split_off(8 * w * w);
    let normal = panels.split_off(4 * w * w);
    Ok(BakedMaps {
        res: w,
        metres_per_tile: metres,
        albedo: panels,
        normal,
        orm,
    })
}

pub fn register(table: &mut MaterialTable, layer: usize, maps: &BakedMaps) -> u32 {
    table.named(
        NAME,
        GpuMaterial {
            flags: F_VERTEX_COLOUR | F_TEXTURED,
            layer: layer as i32,
            params: [0.76, 0.0, 0.78, maps.metres_per_tile.recip() as f32],
            ..GpuMaterial::vertex_colour()
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn shipped_coastal_rock_has_verified_colour_normals_and_roughness() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let maps = super::load(&root).unwrap();
        assert_eq!(maps.metres_per_tile, 2.0);
        let n = maps.res * maps.res;
        for panel in [&maps.albedo, &maps.normal, &maps.orm] {
            assert_eq!(panel.len(), 4 * n);
            assert!(panel.chunks_exact(4).any(|p| p[..3] != panel[..3]));
        }
    }
}
