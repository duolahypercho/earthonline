//! Orthographic views of the existing park trees for Black Point's distant crowns.
//! Bake the original bark and alpha-tested leaf triangles on the tree gathering
//! thread. Store coverage-premultiplied colour and view-space normals so filtering
//! preserves fine branches and lighting still follows the sun in the game.

use anyhow::{Context, Result, ensure};
use eo_props::eztree::{EzMesh, build_ez_tree};
use glam::{Vec2, Vec3};
use std::collections::VecDeque;
use std::path::Path;

pub const SIDE: u32 = 64;
pub const VIEWS: u32 = 8;
pub const SPECIES: [&str; 6] = [
    "park-oak",
    "park-ash",
    "park-pine",
    "street-pollard",
    "street-mature",
    "street-round",
];
pub const LAYERS: u32 = VIEWS * SPECIES.len() as u32;
pub const LEVELS: u32 = SIDE.ilog2() + 1;
pub const BYTES: u64 = (4 * SIDE as u64 * SIDE as u64 - 1) / 3 * 4 * LAYERS as u64 * 2;

#[derive(Clone, Debug)]
pub struct Level {
    pub mip: u32,
    pub layer: u32,
    pub bytes: Vec<u8>,
}

type Texel = [u8; 4];

fn half_size(src: &[Texel], side: usize) -> Vec<Texel> {
    let half = side / 2;
    let mut dst = Vec::with_capacity(half * half);
    for y in 0..half {
        for x in 0..half {
            let i = 2 * (y * side + x);
            dst.push(std::array::from_fn(|c| {
                let sum = [i, i + 1, i + side, i + side + 1].map(|j| u16::from(src[j][c]));
                ((sum.iter().sum::<u16>() + 2) / 4) as u8
            }));
        }
    }
    dst
}

struct Sprigs {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
    cutoff: u8,
}

impl Sprigs {
    fn at(&self, uv: Vec2) -> &[u8] {
        let x = ((uv.x.clamp(0.0, 1.0) * self.width as f32) as usize).min(self.width - 1);
        let y = (((1.0 - uv.y.clamp(0.0, 1.0)) * self.height as f32) as usize).min(self.height - 1);
        let i = (y * self.width + x) * 4;
        &self.rgba[i..i + 4]
    }
}

fn project(tree: &EzMesh, sprigs: &Sprigs, angle: f32) -> [Vec<Texel>; 2] {
    // Four source samples per output texel before the first mip level.
    let side = (2 * SIDE) as usize;
    let mut color = vec![[0; 4]; side * side];
    let mut normal = color.clone();
    let mut depth = vec![f32::NEG_INFINITY; side * side];
    let toward = Vec3::new(angle.cos(), 0.0, angle.sin());
    let right = Vec3::new(-toward.z, 0.0, toward.x);
    let radius = tree.user.half_crown_m.max(0.01) as f32;
    let height = tree.user.height_m.max(0.01) as f32;
    let screen: Vec<_> = tree
        .positions
        .iter()
        .map(|&p| {
            let p = Vec3::from(p);
            Vec3::new(
                (0.5 + 0.48 * p.dot(right) / radius) * side as f32,
                (0.99 - 0.98 * p.y / height) * side as f32,
                p.dot(toward),
            )
        })
        .collect();
    for face in tree.indices.chunks_exact(3) {
        let ids = [face[0] as usize, face[1] as usize, face[2] as usize];
        let [a, b, c] = ids.map(|k| screen[k]);
        let (ab, ac) = ((b - a).truncate(), (c - a).truncate());
        let area = ab.perp_dot(ac);
        if area.abs() < 1e-6 {
            continue;
        }
        let lo = a.min(b).min(c).floor().max(Vec3::ZERO);
        let hi = a.max(b).max(c).ceil().min(Vec3::splat(side as f32));
        let leaf = tree.part[ids[0]] != 0;
        for y in lo.y as usize..hi.y as usize {
            for x in lo.x as usize..hi.x as usize {
                let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - a.truncate();
                let v = q.perp_dot(ac) / area;
                let w = ab.perp_dot(q) / area;
                let u = 1.0 - v - w;
                if u.min(v).min(w) < 0.0 {
                    continue;
                }
                let index = y * side + x;
                let z = u * a.z + v * b.z + w * c.z;
                if z <= depth[index] {
                    continue;
                }
                let [ua, ub, uc] = ids.map(|k| Vec2::from(tree.uvs[k]));
                let texel = sprigs.at(u * ua + v * ub + w * uc);
                if leaf && texel[3] < sprigs.cutoff {
                    continue;
                }
                depth[index] = z;
                let blend = |values: &[[f32; 3]]| {
                    let [pa, pb, pc] = ids.map(|k| Vec3::from(values[k]));
                    u * pa + v * pb + w * pc
                };
                let tint = blend(&tree.colors);
                for ch in 0..3 {
                    let linear =
                        crate::props_lib::srgb_to_linear(texel[ch] as f32 / 255.0) * tint[ch];
                    color[index][ch] = crate::props_lib2::to_srgb8(linear);
                }
                color[index][3] = 255;
                let n = blend(&tree.normals).normalize_or(Vec3::Y);
                let packed = (Vec3::new(n.dot(right), n.y, n.dot(toward)) * 0.5 + 0.5) * 255.0;
                normal[index] = [packed.x as u8, packed.y as u8, packed.z as u8, 255];
            }
        }
    }
    [half_size(&color, side), half_size(&normal, side)]
}

pub fn bake(repo: &Path, atlas: &serde_json::Value) -> Result<VecDeque<Level>> {
    let src = repo
        .join("public")
        .join(atlas["url"].as_str().context("vegetation atlas URL")?);
    let (width, height, rgba) = crate::props_lib::decode_png(&std::fs::read(src)?)?;
    ensure!(width > 0 && height > 0, "empty vegetation atlas");
    let sprigs = Sprigs {
        width,
        height,
        rgba,
        cutoff: (255.0 * atlas["alphaTest"].as_f64().unwrap_or(0.42)) as u8,
    };
    let mut levels = VecDeque::new();
    for (species_index, species) in SPECIES.into_iter().enumerate() {
        let first_view = species_index as u32 * VIEWS;
        let start = levels.len();
        for mip in 0..LEVELS {
            for layer in [first_view, first_view + LAYERS] {
                levels.push_back(Level {
                    mip,
                    layer,
                    bytes: Vec::with_capacity(((SIDE >> mip).pow(2) * VIEWS * 4) as usize),
                });
            }
        }
        let tree = build_ez_tree(species, "near", 0)?;
        for i in 0..VIEWS {
            let projected = project(
                &tree,
                &sprigs,
                std::f32::consts::TAU * i as f32 / VIEWS as f32,
            );
            for (channel, mut pixels) in projected.into_iter().enumerate() {
                for mip in 0..LEVELS as usize {
                    levels[start + 2 * mip + channel]
                        .bytes
                        .extend_from_slice(bytemuck::cast_slice(&pixels));
                    if mip + 1 < LEVELS as usize {
                        pixels = half_size(&pixels, (SIDE as usize) >> mip);
                    }
                }
            }
        }
    }
    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlod_trees::WRITE_BYTES_PER_FRAME;

    #[test]
    fn views_preserve_coverage_and_branch_lighting_at_a_distance() {
        let data = eo_data::DataPaths::discover().unwrap();
        let props = eo_props::JsProps::load().unwrap();
        let levels = bake(&data.repo_root, &props.consts["vegetationAtlas"]).unwrap();
        assert_eq!(
            levels.iter().map(|l| l.bytes.len() as u64).sum::<u64>(),
            BYTES
        );
        assert!(
            levels
                .iter()
                .all(|l| l.bytes.len() <= WRITE_BYTES_PER_FRAME)
        );
        for (colors, normals) in levels[0]
            .bytes
            .chunks_exact((SIDE * SIDE * 4) as usize)
            .zip(levels[1].bytes.chunks_exact((SIDE * SIDE * 4) as usize))
        {
            let pixels: Vec<_> = colors.chunks_exact(4).collect();
            let covered = pixels.iter().filter(|c| c[3] > 127).count();
            assert!(
                (60..pixels.len() * 4 / 5).contains(&covered),
                "coverage {covered}"
            );
            assert!(pixels.iter().any(|p| (1..255).contains(&p[3])));
            assert!(
                normals
                    .chunks_exact(4)
                    .zip(pixels)
                    .all(|(n, c)| n[3] == c[3])
            );
        }
        assert_ne!(&levels[0].bytes[..16384], &levels[0].bytes[16384..32768]);
    }
}
