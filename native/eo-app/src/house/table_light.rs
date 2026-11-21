//! The above-board survey occupies the unused upper-right quarter of the room atlas.
use super::AssetRef;
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Deserialize)]
struct Surface {
    material: String,
    first_index: usize,
    index_count: usize,
    vertex_count: u32,
    atlas_start: u32,
}

#[derive(Clone, Deserialize)]
pub(super) struct TableLight {
    surfaces: Vec<Surface>,
    surface_vertices: usize,
    minimum: [f32; 3],
    maximum: [f32; 3],
    tabletop_height_m: f32,
    #[serde(skip)]
    edge_thickness: f32,
    lamp_parcel_xyz: [f32; 3],
    lamp_radius_m: f32,
    lamp_linear_rgb: [f32; 3],
}

impl TableLight {
    pub(super) fn install(
        root: &Path,
        directory: &Path,
        house_sha: &str,
        orm: &mut [u8],
    ) -> Result<Self> {
        #[derive(Deserialize)]
        struct Survey {
            #[serde(flatten)]
            asset: AssetRef,
            #[serde(flatten)]
            table: TableLight,
            house_sha256: String,
            board_map: [usize; 2],
            reverse_face_row: usize,
            atlas_origin: [usize; 2],
            transfer_encoding: String,
        }
        let src: Survey = serde_json::from_slice(&std::fs::read(
            directory.join("tabletop-light.provenance.json"),
        )?)?;
        let table = src.table;
        ensure!(
            src.house_sha256 == house_sha
                && !table.surfaces.is_empty()
                && table.surface_vertices <= 96 * 256
                && src.reverse_face_row == 96
                && src.board_map == [256, 64]
                && src.atlas_origin == [256, 0]
                && src.transfer_encoding == "sqrt(L/(1+L))",
            "unexpected tabletop light survey"
        );
        ensure!(
            (0..3).all(|i| table.minimum[i].is_finite()
                && table.maximum[i].is_finite()
                && table.maximum[i] > table.minimum[i]
                && table.lamp_parcel_xyz[i].is_finite()
                && table.lamp_linear_rgb[i] > 0.
                && table.lamp_linear_rgb[i] <= 1.)
                && table.lamp_radius_m > 0.
                && table.lamp_radius_m < 1.
                && (0.0..0.02).contains(&(table.minimum[1] - table.tabletop_height_m)),
            "invalid tabletop or lamp geometry"
        );
        let bytes = eo_assets::glb::read_verified(&src.asset.path(root), &src.asset)?;
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
        let info = reader.info();
        ensure!(
            (info.width, info.height, info.color_type, info.bit_depth)
                == (256, 256, png::ColorType::Rgba, png::BitDepth::Eight),
            "invalid tabletop light image"
        );
        let mut pixels = vec![0; 256 * 256 * 4];
        reader.next_frame(&mut pixels)?;
        ensure!(
            orm.len() == 512 * 512 * 4,
            "missing full-resolution house light atlas"
        );
        for (dst, src) in orm
            .chunks_exact_mut(512 * 4)
            .zip(pixels.chunks_exact(256 * 4))
        {
            dst[256 * 4..].copy_from_slice(src);
        }
        Ok(table)
    }

    pub(super) fn install_joinery(
        &mut self,
        root: &Path,
        directory: &Path,
        house_sha: &str,
        place_settings: bool,
        room: &mut [u8],
        wood: &mut [u8],
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Joinery {
            #[serde(flatten)]
            asset: AssetRef,
            house_sha256: String,
            surfaces: Vec<Surface>,
            surface_vertices: usize,
            minimum: [f32; 3],
            maximum: [f32; 3],
            board_bottom_m: f32,
            tabletop_height_m: f32,
            cut_map: [usize; 2],
            lamp_first_row: usize,
            cut_encoding: String,
            transfer_encoding: String,
        }

        let survey: Joinery = serde_json::from_slice(&std::fs::read(
            directory.join("oak-joinery.provenance.json"),
        )?)?;
        let thickness = survey.tabletop_height_m - survey.board_bottom_m;
        let mut boards: Vec<_> = survey
            .surfaces
            .iter()
            .filter_map(|s| super::dining_plank(&s.material))
            .collect();
        boards.sort_unstable();
        ensure!(
            boards == [0, 1, 2, 3]
                && survey.surfaces.len() == 4
                && survey.house_sha256 == house_sha
                && survey.minimum == self.minimum
                && survey.maximum == self.maximum
                && survey.tabletop_height_m == self.tabletop_height_m
                && (0.0..0.2).contains(&thickness)
                && thickness > 0.
                && survey.cut_map == [512, 48]
                && survey.lamp_first_row == 96
                && survey.cut_encoding == "sqrt(linear RGB)"
                && survey.transfer_encoding == "sqrt(L/(1+L))",
            "unregistered oak joinery survey"
        );
        if !place_settings {
            self.surfaces.clear();
            self.surface_vertices = 0;
        }
        let base = self.surface_vertices;
        let count = survey.surface_vertices;
        ensure!(
            count > 0
                && count <= 96 * 256 - base
                && room.len() == 512 * 512 * 4
                && wood.len() == room.len(),
            "oak light survey exceeds texture bounds"
        );
        let pixels = super::Finishes::read_pixels(root, &survey.asset, 1)?;
        // Green is still the photographed roughness, including on the floor.
        for (dst, src) in wood[416 * 512 * 4..]
            .chunks_exact_mut(4)
            .zip(pixels.chunks_exact(4))
        {
            dst[0] = src[0];
            dst[2] = src[1];
            dst[3] = src[2];
        }
        // Append both sides after the last place setting. Row 192 and below
        // continue to hold the independent high-resolution horizontal receiver.
        for side in 0..2 {
            for i in 0..count {
                let address = base + i;
                let dst = 4 * ((address / 256 + 96 * side) * 512 + 256 + address % 256);
                let src = 4 * (survey.lamp_first_row * 512 + count * side + i);
                room[dst..dst + 4].copy_from_slice(&pixels[src..src + 4]);
            }
        }
        for mut surface in survey.surfaces {
            surface.atlas_start = surface
                .atlas_start
                .checked_add(base as u32)
                .ok_or_else(|| anyhow::anyhow!("invalid joinery vertex address"))?;
            self.surfaces.push(surface);
        }
        self.surface_vertices += count;
        self.edge_thickness = thickness;
        Ok(())
    }

    pub(super) fn material(&self, name: &str) -> bool {
        self.surfaces.iter().any(|s| s.material == name)
    }

    pub(super) fn vertices(
        &self,
        model: &eo_assets::CookedModel,
    ) -> Result<eo_assets::glb::CutPattern> {
        let mut vertices = eo_assets::glb::CutPattern::new();
        for s in &self.surfaces {
            let part = model
                .parts
                .iter()
                .find(|p| model.materials[p.material as usize].name == s.material)
                .ok_or_else(|| anyhow::anyhow!("missing measured table material {}", s.material))?;
            ensure!(
                s.index_count > 0
                    && s.first_index
                        .checked_add(s.index_count)
                        .is_some_and(|end| end <= part.index_count as usize),
                "invalid table primitive index range"
            );
            let first = part.first_index as usize + s.first_index;
            let indices = &model.indices[first..first + s.index_count];
            let base = *indices.iter().min().unwrap();
            let end = *indices.iter().max().unwrap();
            ensure!(
                end - base + 1 == s.vertex_count
                    && s.atlas_start as usize == vertices.len()
                    && s.atlas_start + s.vertex_count <= 96 * 256,
                "invalid table vertex survey"
            );
            for i in 0..s.vertex_count {
                let address = s.atlas_start + i;
                ensure!(
                    vertices
                        .insert(base + i, [address as u8, (address / 256) as u8])
                        .is_none(),
                    "duplicate table light vertex"
                );
            }
        }
        ensure!(
            vertices.len() == self.surface_vertices,
            "incomplete table surface survey"
        );
        Ok(vertices)
    }

    pub(super) fn uniforms(&self) -> [[f32; 4]; 3] {
        [
            glam::Vec3::from(self.minimum)
                .extend(self.tabletop_height_m)
                .to_array(),
            glam::Vec3::from(self.maximum)
                .extend(self.edge_thickness)
                .to_array(),
            glam::Vec3::from(self.lamp_linear_rgb).extend(0.).to_array(),
        ]
    }

    pub(super) fn lamp(&self) -> [f32; 4] {
        glam::Vec3::from(self.lamp_parcel_xyz)
            .extend(self.lamp_radius_m)
            .to_array()
    }
}
