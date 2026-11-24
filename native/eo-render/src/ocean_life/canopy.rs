//! Represent the blade area of the visible kelp in four depth bands. Bin each
//! mesh once at load, then rebuild the world-space cover on habitat tile changes.
use super::{Vertex, field};
use glam::{Vec2, Vec3};
use std::collections::BTreeMap;

pub const SIDE: u32 = 120;
const BANDS: [f32; 4] = [0.85, 2.55, 5.1, 8.2];
const WIDTH: f32 = field::TILE * (field::RING * 2 + 1) as f32;
const METRES: f32 = WIDTH / SIDE as f32;

pub struct LeafArea {
    centre: Vec3,
    area: f32,
}

pub(super) fn measure(vertices: &[Vertex], indices: &[u32]) -> Vec<LeafArea> {
    let mut bins = BTreeMap::new();
    for triangle in indices.chunks_exact(3) {
        let a = vertices[triangle[0] as usize];
        if a.color[3] == 0 {
            continue;
        } // stipes and floats
        let a = Vec3::from(a.pos);
        let b = Vec3::from(vertices[triangle[1] as usize].pos);
        let c = Vec3::from(vertices[triangle[2] as usize].pos);
        let normal = (b - a).cross(c - a) * 0.5;
        let area = 0.72 * normal.y.abs() + 0.28 * normal.length();
        if area <= f32::EPSILON {
            continue;
        }
        let centre = (a + b + c) / 3.;
        let key = (centre * Vec3::new(10., 4., 10.))
            .floor()
            .as_ivec3()
            .to_array();
        let bin = bins.entry(key).or_insert((Vec3::ZERO, 0.));
        bin.0 += centre * area;
        bin.1 += area;
    }
    bins.into_values()
        .map(|(moment, area)| LeafArea {
            centre: moment / area,
            area,
        })
        .collect()
}

pub(super) fn write<'a>(
    q: &wgpu::Queue,
    texture: &wgpu::Texture,
    centre: Vec2,
    stands: impl Iterator<Item = (&'a [LeafArea], &'a [field::Instance])>,
) -> u8 {
    let mut stands = stands
        .filter(|(leaves, plants)| !leaves.is_empty() && !plants.is_empty())
        .peekable();
    // An empty stand cannot intercept light. No texture upload is needed:
    // the camera uniform makes the previous tile's map inaccessible.
    if stands.peek().is_none() {
        return 0;
    }
    let mut occupied = 0;
    let mut density = vec![[0f32; 4]; (SIDE * SIDE) as usize];
    let origin = centre - WIDTH * 0.5;
    for (leaves, plants) in stands {
        for plant in plants {
            let (sin, cos) = plant.at[3].sin_cos();
            let scale = Vec3::from_array([plant.shape[0], plant.shape[1], plant.shape[2]]);
            for leaf in leaves {
                let p = leaf.centre * scale;
                let xz = Vec2::new(
                    plant.at[0] + cos * p.x - sin * p.z,
                    plant.at[2] + sin * p.x + cos * p.z,
                );
                let texel = (xz - origin) / METRES - 0.5;
                let depth = field::SEA - plant.at[1] - p.y;
                let band = BANDS
                    .windows(2)
                    .position(|w| depth < (w[0] + w[1]) * 0.5)
                    .unwrap_or(3);
                // Spread the sub-cell projection over its footprint; the normalized
                // tent carries the same blade area for a small shoot or a mature crown.
                let radius = (0.075 * scale.x.max(scale.z) / METRES).max(1.);
                let lo = (texel - radius).ceil().as_ivec2().max(glam::IVec2::ZERO);
                let hi = (texel + radius)
                    .floor()
                    .as_ivec2()
                    .min(glam::IVec2::splat(SIDE as i32 - 1));
                let mass = leaf.area * scale.x * scale.z / (METRES * METRES * radius * radius);
                if lo.x <= hi.x && lo.y <= hi.y {
                    occupied |= 1 << band;
                }
                for z in lo.y..=hi.y {
                    let weight_z = 1. - (z as f32 - texel.y).abs() / radius;
                    for x in lo.x..=hi.x {
                        let weight_x = 1. - (x as f32 - texel.x).abs() / radius;
                        density[z as usize * SIDE as usize + x as usize][band] +=
                            mass * weight_x * weight_z;
                    }
                }
            }
        }
    }
    let mask: Vec<[u8; 4]> = density
        .into_iter()
        .map(|bands| {
            bands.map(|area| {
                if area > 0. {
                    (255. * -(-area).exp_m1()).round() as u8
                } else {
                    0
                }
            })
        })
        .collect();
    q.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(&mask),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * SIDE),
            rows_per_image: None,
        },
        texture.size(),
    );
    occupied
}
