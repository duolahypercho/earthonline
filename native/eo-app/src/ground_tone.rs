//! The terrain's vertex tone and the far land's height, from the open-space
//! ground (eo-landmarks `ground_cover`), the shore field and the land-cover
//! raster: the closures the scene build reads (`SceneInputs::ground_tone`)
//! and the carpet press consults (`far_land`). main.rs builds them here from
//! the loaded inputs.

use eo_data::{ElevationGrid, LandCover};
use eo_landmarks::ground_cover::GroundCover;
use eo_water::WaterData;

/// Ground more than this over the sea, and more than `FAR_LAND_SHORE_M` out
/// from every ring edge, is far land (main.rs, the carpet press).
pub const FAR_LAND_ABOVE_SEA_M: f64 = 2.0;
pub const FAR_LAND_SHORE_M: f64 = 60.0;

/// A linear colour as the terrain's sRGB vertex bytes (draw.wgsl decodes them
/// with a 2.2 power), with `a` in the alpha byte.
fn pack(c: [f64; 3], a: u8) -> u32 {
    let b = |v: f64| (v.max(0.0).powf(1.0 / 2.2).min(1.0) * 255.0).round() as u32;
    b(c[0]) | (b(c[1]) << 8) | (b(c[2]) << 16) | ((a as u32) << 24)
}

/// The far land's ground height at a point (None where it is not far land):
/// beyond the city's shoreline rings, more than `FAR_LAND_SHORE_M` out from
/// every ring edge and more than `FAR_LAND_ABOVE_SEA_M` over the sea. None
/// everywhere without the ground cover or the shore field.
pub fn far_land<'a>(cover: Option<&'a GroundCover>, water: Option<&'a WaterData>, elev: &'a ElevationGrid) -> impl Fn(f64, f64) -> Option<f64> + Sync + 'a {
    move |x: f64, z: f64| -> Option<f64> {
        cover?;
        let w = water?;
        if w.field.signed_at(x, z) <= FAR_LAND_SHORE_M {
            return None;
        }
        elev.sample(x, z).filter(|h| *h > eo_water::SEA_LEVEL_Y + FAR_LAND_ABOVE_SEA_M)
    }
}

/// A terrain vertex's colour from the open-space ground: beach sand (its
/// shore distance, metres inland, in the alpha byte: 1.5 a metre from 64 m
/// offshore; 255 is no beach) or a green's tone; sRGB bytes as the terrain's
/// vertex colours are. None without the ground cover (the terrain keeps its
/// land-cover tint).
pub fn ground_tone<'a>(cover: Option<&'a GroundCover>, water: Option<&'a WaterData>, elev: &'a ElevationGrid, land: Option<&'a LandCover>) -> impl Fn(f32, f32) -> Option<u32> + Sync + 'a {
    let far_land_fn = far_land(cover, water, elev);
    move |x: f32, z: f32| -> Option<u32> {
        let cover = cover?;
        let (x, z) = (x as f64, z as f64);
        // the far land's ground by its land-cover class (tile-ground-cover.js
        // `DERIVED_TONES`: unmapped open ground, wild grass or coastal scrub; under trees understorey).
        if far_land_fn(x, z).is_some() {
            let class = land.map(|l| l.class_at(x, z)).unwrap_or(0);
            return Some(pack(eo_landmarks::ground_cover::far_land_tone(class, x, z), 255));
        }
        if let Some(w) = water {
            let s = w.field.signed_at(x, z);
            if cover.beach_sand(x, z, s.abs()) {
                let a = ((64.0 - s) * 1.5).round().clamp(0.0, 253.0) as u8;
                return Some(pack(cover.sand_at(x, z), a));
            }
        }
        // the open ground's measured green (yards, lots, hills), inland of the shore.
        let park = cover.tone_name_at(x, z).and_then(|n| cover.tone_at(x, z).map(|c| (n, c)));
        let class = land.map(|l| l.class_at(x, z)).unwrap_or(0);
        let shore = water.map(|w| w.field.signed_at(x, z).abs());
        let h = |dx: f64, dz: f64| elev.sample(x + dx, z + dz);
        let slope = match (h(-6.0, 0.0), h(6.0, 0.0), h(0.0, -6.0), h(0.0, 6.0)) {
            (Some(a), Some(b), Some(c), Some(d)) => ((b - a).hypot(d - c)) / 12.0,
            _ => 0.0,
        };
        if let Some(c) = eo_render::ground2::open_ground(x, z, class, park, shore, h(0.0, 0.0).map(|h| h - eo_water::SEA_LEVEL_Y), slope) {
            return Some(pack(c, 255));
        }
        cover.tone_at(x, z).map(|c| pack(c, 255))
    }
}

#[cfg(test)]
mod tests {
    use super::pack;

    #[test]
    fn pack_puts_srgb_bytes_and_alpha_in_place() {
        assert_eq!(pack([0.0, 0.0, 0.0], 0), 0);
        assert_eq!(pack([1.0, 1.0, 1.0], 255), 0xffff_ffff);
        // linear 0.5 is sRGB-ish 0.5^(1/2.2) = 0.7297 -> 186
        assert_eq!(pack([0.5, 0.0, 2.0], 7), 186 | (255 << 16) | (7 << 24));
    }
}
