//! The reef uses the shipped, verified 2 m rock and two sediment scans at their
//! recorded world scales. Build the mip chains before the first frame.
use crate::{GpuContext, authored::mip_chain, seabed::rock_maps};
use anyhow::Result;
use eo_assets::kit::BakedMaps;
use std::path::Path;
use wgpu::util::DeviceExt;

pub(super) struct Materials {
    pub layout: wgpu::BindGroupLayout,
    pub group: wgpu::BindGroup,
    pub shader: String,
    pub bytes: u64,
}

impl Materials {
    pub fn new(ctx: &GpuContext, root: &Path) -> Result<Self> {
        let scans = [
            rock_maps::load(root)?,
            rock_maps::load_named(root, "coast-sand-04")?,
            rock_maps::load_named(root, "damp-beach-sand")?,
        ];
        let side = scans[0].res as u32;
        let levels = side.ilog2() + 1;
        let bytes = (0..levels)
            .map(|m| u64::from((side >> m).pow(2)) * 4 * 3 * 3)
            .sum();
        let texture = |label, format, image: fn(&BakedMaps) -> &[u8]| {
            let mut data = Vec::new();
            for scan in &scans {
                for mip in mip_chain(
                    image(scan),
                    scan.res,
                    format == wgpu::TextureFormat::Rgba8UnormSrgb,
                ) {
                    data.extend(mip);
                }
            }
            let map = ctx.device.create_texture_with_data(
                &ctx.queue,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 3,
                    },
                    mip_level_count: levels,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &data,
            );
            map.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let maps = [
            texture("reef colour", wgpu::TextureFormat::Rgba8UnormSrgb, |s| {
                &s.albedo
            }),
            texture("reef normals", wgpu::TextureFormat::Rgba8Unorm, |s| {
                &s.normal
            }),
            texture(
                "reef occlusion and roughness",
                wgpu::TextureFormat::Rgba8Unorm,
                |s| &s.orm,
            ),
        ];
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reef scan filter"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        let entries = std::array::from_fn::<_, 4, _>(|i| wgpu::BindGroupLayoutEntry {
            binding: i as u32,
            visibility: wgpu::ShaderStages::FRAGMENT,
            count: None,
            ty: if i == 3 {
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
            } else {
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                }
            },
        });
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("reef material scans"),
                entries: &entries,
            });
        let bindings = std::array::from_fn::<_, 4, _>(|i| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: if i == 3 {
                wgpu::BindingResource::Sampler(&sampler)
            } else {
                wgpu::BindingResource::TextureView(&maps[i])
            },
        });
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reef material scans"),
            layout: &layout,
            entries: &bindings,
        });
        let metres = scans.each_ref().map(|s| s.metres_per_tile);
        let shader = format!(
            "const SCAN_METRES: vec3f=vec3f({:.6},{:.6},{:.6});\n{}",
            metres[0],
            metres[1],
            metres[2],
            include_str!("materials.wgsl")
        );
        Ok(Self {
            layout,
            group,
            shader,
            bytes,
        })
    }
}

pub(super) const STONE_KINDS: [usize; 7] =
    [7, 8, 9, 14, 15, super::field::LEDGE, super::field::BOULDER];

/// Approximate the fraction of the diffuse sky blocked at a bed vertex by
/// a nearby stone. The rotated footprint and elevation come from its mesh.
pub(super) fn bed_contact(
    bed: &mut [super::Vertex],
    rocks: &[super::Instance],
    extent: glam::Vec3,
) {
    let Some(first) = bed.first() else {
        return;
    };
    let origin = glam::Vec2::new(first.pos[0], first.pos[2]);
    let side = super::BED_SIDE;
    for stone in rocks {
        let centre = glam::Vec2::new(stone.at[0], stone.at[2]) - origin;
        let axes = glam::Vec2::new(extent.x * stone.shape[0], extent.z * stone.shape[2]);
        let height = extent.y * stone.shape[1];
        let reach = axes.max_element() + 1.5 * height;
        let lo = (centre - reach).ceil().as_ivec2().max(glam::IVec2::ZERO);
        let hi = (centre + reach)
            .floor()
            .as_ivec2()
            .min(glam::IVec2::splat(side as i32 - 1));
        let (sin, cos) = stone.at[3].sin_cos();
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let v = &mut bed[z as usize * side + x as usize];
                if v.color[3] == 0 {
                    continue;
                }
                let p = glam::Vec2::new(x as f32, z as f32) - centre;
                let local = glam::Vec2::new(cos * p.x + sin * p.y, -sin * p.x + cos * p.y);
                let radius = (local / axes.max(glam::Vec2::splat(0.01))).length();
                let gap = (p.length() * (1. - 1. / radius.max(1.))).max(0.);
                let above = (stone.at[1] + height - v.pos[1]).max(0.);
                let shade = (1. - gap / (1.5 * above).max(0.01)).clamp(0., 1.);
                v.color[3] = (f32::from(v.color[3]) * (1. - 0.58 * shade * shade))
                    .round()
                    .max(1.) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sediment_receives_contact_from_nearby_stone_only() {
        let side = super::super::BED_SIDE;
        let mut bed: Vec<_> = (0..side * side)
            .map(|i| super::super::Vertex {
                pos: [(i % side) as f32, 0., (i / side) as f32],
                normal: [0., 1., 0.],
                color: [0, 0, 0, 255],
            })
            .collect();
        bed[side * 2 + 1].color[3] = 0;
        let stone = super::super::Instance {
            at: [2., 0., 2., 0.],
            shape: [1., 1., 1., 0.],
            traits: [0.; 4],
        };
        bed_contact(&mut bed, &[stone], glam::Vec3::ONE);
        assert!(bed[side * 2 + 2].color[3] < 200);
        assert_eq!(bed[side * 2 + 6].color[3], 255);
        assert_eq!(bed[side * 2 + 1].color[3], 0);
    }
    #[test]
    fn shipped_sediment_scans_include_physical_scale_and_normal_relief() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (name, size) in [("coast-sand-04", 3.94), ("damp-beach-sand", 2.)] {
            let m = rock_maps::load_named(&root, name).unwrap();
            assert_eq!(m.metres_per_tile, size);
            assert_eq!(m.albedo.len(), m.orm.len());
            assert!(m.normal.chunks_exact(4).any(|p| p[..3] != m.normal[..3]));
        }
    }
}
