//! A small camera-local window onto stable habitat tiles across the SF sea.
//! Model buffers are permanent. Only a tile crossing uploads new placements.
pub mod field;
use crate::{GpuContext, camera::Camera, look};
use anyhow::{Context, Result, ensure};
use bytemuck::{Pod, Zeroable};
use field::{Instance, KINDS, RING, Sampler, TILE};
use glam::Vec2;
use std::{collections::HashMap, ops::Range, path::Path, sync::Mutex};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    color: [u8; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct View {
    vp: [[f32; 4]; 4],
    eye_time: [f32; 4],
}

struct Draw {
    indices: Range<u32>,
    instances: Range<u32>,
}
struct Window {
    cell: Option<(i32, i32)>,
    visible: bool,
    groups: [Draw; KINDS.len()],
    sediment_indices: u32,
    bed: Vec<Vertex>,
    tiles: HashMap<(i32, i32), [Vec<Instance>; KINDS.len()]>,
}
pub struct OceanLife {
    reef: bool,
    coastal_habitat: bool,
    sampler: Sampler,
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    view: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    instances: wgpu::Buffer,
    sediment: wgpu::Buffer,
    sediment_indices: wgpu::Buffer,
    sediment_instance: wgpu::Buffer,
    window: Mutex<Window>,
}
const BED_SIDE: usize = 43;
const MAX_INSTANCES: usize = 8192;

impl OceanLife {
    pub fn new(
        ctx: &GpuContext,
        look: &look::LookGpu,
        samples: u32,
        root: &Path,
        sampler: Sampler,
    ) -> Result<Self> {
        let dir = root.join("public/assets/ocean");
        let load = |stem: &str| -> Result<gltf::Gltf> {
            let source: eo_assets::AssetRef = serde_json::from_slice(&std::fs::read(
                dir.join(format!("{stem}.provenance.json")),
            )?)?;
            let bytes = eo_assets::glb::read_verified(&source.path(root), &source)?;
            Ok(gltf::Gltf::from_slice(&bytes)?)
        };
        let native = load("sf-habitat")?;
        let reef = look
            .options
            .ocean_reef
            .then(|| load("reef-forms"))
            .transpose()?;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut ranges = Vec::new();
        for (kind, base_name) in KINDS.iter().copied().enumerate() {
            let name = match base_name {
                "eelgrass" if look.options.coastal_habitat => "eelgrass-shoots",
                "bull-kelp" if look.options.coastal_habitat => "bull-kelp-crown",
                "kelp-column" if look.options.dive_detail => "coastal-kelp",
                _ => base_name,
            };
            let glb = if kind < field::CORE_KINDS && !matches!(name, "eelgrass-shoots" | "bull-kelp-crown") {
                &native
            } else if let Some(library) = &reef {
                library
            } else {
                ranges.push(0..0);
                continue;
            };
            let mesh = glb
                .meshes()
                .find(|m| m.name() == Some(name))
                .with_context(|| format!("missing habitat mesh {name}"))?;
            let start = indices.len() as u32;
            for p in mesh.primitives() {
                let reader = p.reader(|_| glb.blob.as_deref());
                let pos = reader.read_positions().context("habitat positions")?;
                let norm = reader.read_normals().context("habitat normals")?;
                let rgba = reader
                    .read_colors(0)
                    .context("habitat vertex pigment")?
                    .into_rgba_u8();
                ensure!(
                    pos.len() == norm.len() && norm.len() == rgba.len(),
                    "invalid habitat vertex attributes"
                );
                let base = vertices.len() as u32;
                let nv = pos.len() as u32;
                vertices.extend(
                    pos.zip(norm)
                        .zip(rgba)
                        .map(|((pos, normal), color)| Vertex { pos, normal, color }),
                );
                for id in reader.read_indices().context("habitat indices")?.into_u32() {
                    ensure!(id < nv, "invalid habitat index");
                    indices.push(base + id);
                }
            }
            ranges.push(start..indices.len() as u32);
        }
        let d = &ctx.device;
        let fixed = |name, contents, usage| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(name),
                contents,
                usage,
            })
        };
        let dynamic = |name, size, usage| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(name),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let view = dynamic(
            "ocean view",
            size_of::<View>() as u64,
            wgpu::BufferUsages::UNIFORM,
        );
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean habitat view"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean habitat view"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view.as_entire_binding(),
            }],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("marine life"),
            bind_group_layouts: &[Some(&layout), Some(look.main_bind_group_layout())],
            immediate_size: 0,
        });
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("marine life"),
            source: wgpu::ShaderSource::Wgsl(format!(
                "const DIVE_DETAIL: bool = {};\nconst COASTAL_HABITAT: bool = {};\nconst SWELL_DIR: f32 = {:.7};\nconst SWELL_OMEGA: f32 = {:.7};\n{}",
                look.options.dive_detail, look.options.coastal_habitat, eo_water::waves::SWELL_DIR,
                std::f64::consts::TAU / eo_water::waves::SWELL_TP[0], include_str!("ocean_life/habitat.wgsl"),
            ).into()),
        });
        let pipeline=d.create_render_pipeline(&wgpu::RenderPipelineDescriptor{
            label:Some("marine life in the opaque underwater scene"),layout:Some(&pl),
            vertex:wgpu::VertexState{module:&shader,entry_point:Some("vs"),compilation_options:Default::default(),
                buffers:&[
                    Some(wgpu::VertexBufferLayout{array_stride:size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4]}),
                    Some(wgpu::VertexBufferLayout{array_stride:size_of::<Instance>() as u64,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![3=>Float32x4,4=>Float32x4,5=>Float32x4]})]},
            fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format:look::HDR_FORMAT,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),
            primitive:wgpu::PrimitiveState::default(),depth_stencil:Some(wgpu::DepthStencilState{format:crate::DEPTH_FORMAT,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::GreaterEqual),stencil:Default::default(),bias:Default::default()}),
            multisample:wgpu::MultisampleState{count:samples,..Default::default()},multiview_mask:None,cache:None});
        let bed_inst = Instance {
            at: [0.; 4],
            shape: [1., 1., 1., 0.],
            traits: [255., 0., 0., 1.],
        };
        Ok(Self {
            reef: reef.is_some(),
            coastal_habitat: look.options.coastal_habitat,
            sampler,
            pipeline,
            group,
            view,
            vertices: fixed(
                "Blender habitat vertices",
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: fixed(
                "Blender habitat indices",
                bytemuck::cast_slice(&indices),
                wgpu::BufferUsages::INDEX,
            ),
            instances: dynamic(
                "nearby habitat instances",
                (MAX_INSTANCES * size_of::<Instance>()) as u64,
                wgpu::BufferUsages::VERTEX,
            ),
            sediment: dynamic(
                "nearby sea floor",
                (BED_SIDE * BED_SIDE * size_of::<Vertex>()) as u64,
                wgpu::BufferUsages::VERTEX,
            ),
            sediment_indices: dynamic(
                "wet floor triangles",
                ((BED_SIDE - 1).pow(2) * 6 * 4) as u64,
                wgpu::BufferUsages::INDEX,
            ),
            sediment_instance: fixed(
                "sea floor placement",
                bytemuck::bytes_of(&bed_inst),
                wgpu::BufferUsages::VERTEX,
            ),
            window: Mutex::new(Window {
                cell: None,
                visible: false,
                sediment_indices: 0,
                bed: Vec::new(),
                tiles: HashMap::new(),
                groups: std::array::from_fn(|i| Draw {
                    indices: ranges[i].clone(),
                    instances: 0..0,
                }),
            }),
        })
    }

    pub fn update(&self, q: &wgpu::Queue, cam: &Camera, clock: f64) -> bool {
        let mut w = self.window.lock().unwrap();
        let p = Vec2::new(cam.eye.x, cam.eye.z);
        let column = (cam.eye.y < field::SEA + 4.)
            .then(|| (self.sampler)(p))
            .flatten();
        let wet = column.is_some_and(|s| s.shore > 0. && cam.eye.y > s.floor - 1.);
        w.visible = column.is_some_and(|s| s.shore > -12. && cam.eye.y > s.floor - 1.);
        if !w.visible {
            return wet;
        }
        q.write_buffer(
            &self.view,
            0,
            bytemuck::bytes_of(&View {
                vp: cam.view_proj().to_cols_array_2d(),
                eye_time: [cam.eye.x, cam.eye.y, cam.eye.z, (clock % 4096.) as f32],
            }),
        );
        let cell = (p / TILE).floor().as_ivec2();
        if w.cell == Some((cell.x, cell.y)) {
            return wet;
        }
        w.tiles
            .retain(|&(x, z), _| (x - cell.x).abs() <= RING && (z - cell.y).abs() <= RING);
        for z in cell.y - RING..=cell.y + RING {
            for x in cell.x - RING..=cell.x + RING {
                w.tiles.entry((x, z)).or_insert_with(|| {
                    let mut groups = Default::default();
                    field::populate(x, z, &self.sampler, &mut groups);
                    if self.reef {
                        field::reef(x, z, &self.sampler, &mut groups);
                        if self.coastal_habitat {
                            field::coastal_canopy(&mut groups);
                            field::coastal_grass(x, z, &self.sampler, &mut groups[3]);
                        }
                    }
                    groups
                });
            }
        }
        // Emit tiles in world order so a boundary crossing never reshuffles overlapping draws.
        let mut all = Vec::new();
        for i in 0..KINDS.len() {
            let start = all.len() as u32;
            for z in cell.y - RING..=cell.y + RING {
                for x in cell.x - RING..=cell.x + RING {
                    all.extend_from_slice(&w.tiles[&(x, z)][i]);
                }
            }
            w.groups[i].instances = start..all.len() as u32;
        }
        assert!(all.len() <= MAX_INSTANCES);
        if !all.is_empty() {
            q.write_buffer(&self.instances, 0, bytemuck::cast_slice(&all));
        }
        let (mut v, ix) = sediment((cell.as_vec2() + 0.5) * TILE, &self.sampler, &w.bed);
        if self.coastal_habitat {
            let plants = |kind: usize| {
                let r = &w.groups[kind].instances;
                &all[r.start as usize..r.end as usize]
            };
            shade_plant_beds(&mut v, plants(3), plants(11));
        }
        if !ix.is_empty() {
            q.write_buffer(&self.sediment, 0, bytemuck::cast_slice(&v));
            q.write_buffer(&self.sediment_indices, 0, bytemuck::cast_slice(&ix));
        }
        w.bed = v;
        w.sediment_indices = ix.len() as u32;
        w.cell = Some((cell.x, cell.y));
        wet
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, look: &look::LookGpu) -> bool {
        let w = self.window.lock().unwrap();
        if !w.visible {
            return false;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        look.bind_main(pass);
        if w.sediment_indices > 0 {
            pass.set_vertex_buffer(0, self.sediment.slice(..));
            pass.set_vertex_buffer(1, self.sediment_instance.slice(..));
            pass.set_index_buffer(self.sediment_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..w.sediment_indices, 0, 0..1);
        }
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for g in &w.groups {
            if !g.instances.is_empty() {
                pass.draw_indexed(g.indices.clone(), 0, g.instances.clone());
            }
        }
        true
    }
}
/// The bed receives less diffuse sky beneath a canopy. Accumulate the solid
/// angle of nearby crowns on the metre grid. Alpha zero still denotes dry land.
fn shade_plant_beds(bed: &mut [Vertex], grass: &[Instance], kelp: &[Instance]) {
    let Some(first) = bed.first() else {
        return;
    };
    let origin = Vec2::new(first.pos[0], first.pos[2]);
    for v in bed.iter_mut().filter(|v| v.color[3] != 0) {
        v.color[3] = 255;
    }
    for plant in grass.iter().chain(kelp) {
        let centre = Vec2::new(plant.at[0], plant.at[2]) - origin;
        let canopy = plant.traits[0] == 11.;
        let width = if canopy {
            plant.shape[0] * 0.46
        } else {
            plant.shape[0] * 0.83
        };
        let radius = (width + 0.5).max(0.55);
        let low = (centre - radius).floor().as_ivec2().max(glam::IVec2::ZERO);
        let high = (centre + radius)
            .ceil()
            .as_ivec2()
            .min(glam::IVec2::splat(BED_SIDE as i32 - 1));
        for z in low.y..=high.y {
            for x in low.x..=high.x {
                let v = &mut bed[z as usize * BED_SIDE + x as usize];
                if v.color[3] == 0 {
                    continue;
                }
                let d2 =
                    (Vec2::new(x as f32, z as f32) - centre).length_squared() / (radius * radius);
                let covered = (1. - d2).max(0.).powi(2);
                let loss = if canopy { 0.44 } else { 0.19 };
                v.color[3] = ((v.color[3] as f32 * (1. - loss * covered)).round() as u8).max(1);
            }
        }
    }
}

fn sediment(centre: Vec2, sample: &Sampler, previous: &[Vertex]) -> (Vec<Vertex>, Vec<u32>) {
    let mut verts = Vec::with_capacity(BED_SIDE * BED_SIDE);
    let mut wet = Vec::with_capacity(verts.capacity());
    let old_origin = previous.first().map(|v| Vec2::new(v.pos[0], v.pos[2]));
    for z in 0..BED_SIDE {
        for x in 0..BED_SIDE {
            let q = centre + Vec2::new(x as f32 - 21., z as f32 - 21.);
            if let Some(origin) = old_origin {
                let offset = (q - origin).as_ivec2();
                if (0..BED_SIDE as i32).contains(&offset.x)
                    && (0..BED_SIDE as i32).contains(&offset.y)
                {
                    let v = previous[offset.y as usize * BED_SIDE + offset.x as usize];
                    wet.push(v.color[3] != 0);
                    verts.push(v);
                    continue;
                }
            }
            let s = sample(q).filter(|s| s.shore > 0.05 && s.floor < field::SEA - 0.02);
            wet.push(s.is_some());
            let rgb = s.map_or([0; 4], |s| {
                [
                    if s.offshore { 0 } else { 255 },
                    (s.beach * 255.) as u8,
                    (s.pacific * 255.) as u8,
                    255,
                ]
            });
            verts.push(Vertex {
                pos: [q.x, s.map_or(0., |s| s.floor) + 0.012, q.y],
                normal: [0., 1., 0.],
                color: rgb,
            });
        }
    }
    let mut ix = Vec::with_capacity((BED_SIDE - 1).pow(2) * 6);
    for z in 0..BED_SIDE - 1 {
        for x in 0..BED_SIDE - 1 {
            let a = z * BED_SIDE + x;
            for f in [
                [a, a + BED_SIDE, a + 1],
                [a + 1, a + BED_SIDE, a + BED_SIDE + 1],
            ] {
                if f.iter().all(|&i| wet[i]) {
                    ix.extend(f.map(|i| i as u32));
                }
            }
        }
    }
    (verts, ix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn scrolling_the_bed_reuses_only_the_overlapping_world_samples() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let sample: Sampler = Arc::new(move |p| {
            count.fetch_add(1, Ordering::Relaxed);
            Some(field::Site {
                floor: -2.0 - p.y * 0.01,
                shore: p.x + p.y,
                beach: 0.3,
                pacific: 0.8,
                offshore: false,
            })
        });
        let (before, _) = sediment(Vec2::new(-3., 3.), &sample, &[]);
        calls.store(0, Ordering::Relaxed);
        let centre = Vec2::new(-9., 9.);
        let (mut v, ix) = sediment(centre, &sample, &before);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            BED_SIDE * BED_SIDE - (BED_SIDE - 6).pow(2)
        );
        let (fresh, indices) = sediment(centre, &sample, &[]);
        assert_eq!(ix, indices);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&v),
            bytemuck::cast_slice::<_, u8>(&fresh)
        );
    }
}
