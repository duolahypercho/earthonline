//! The map: a GPU mesh of the city's building footprints and roads (from
//! `eo-data`), a land-cover fill, and render-to-texture targets for the HUD
//! radar, the pause map and the title backdrop. A target is redrawn only
//! when its view changes (the radar when the player leaves the middle of its
//! texture, the pause map on a pan or zoom, the title once).

use crate::theme::{self, Color};
use bytemuck::{Pod, Zeroable};
use eo_data::{City, LandCover};

/// Mesh layers, in draw order: footprints, then road classes (path, alley,
/// residential, collector, arterial, freeway).
pub const LAYER_BUILDINGS: usize = 0;
const TUNNELS: usize = 7;
pub const LAYERS: usize = TUNNELS + 1;
const CHUNK_M: f32 = 1000.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct MapVertex {
    pub pos: [f32; 2],
    pub dir: [f32; 2],
    pub ext: [f32; 2],
    pub class: u32,
}

/// `cityRoadClassOf` in src/ui/city-map.js.
pub fn road_class(highway: &str) -> usize {
    let h = highway;
    let is = |names: &[&str]| names.iter().any(|n| h == *n || h.strip_suffix("_link") == Some(n));
    if h == "motorway" || h == "motorway_link" {
        5
    } else if is(&["trunk", "primary"]) {
        4
    } else if is(&["secondary", "tertiary"]) {
        3
    } else if h == "service" {
        1
    } else if ["footway", "path", "pedestrian", "cycleway", "steps", "track", "bridleway", "corridor"].contains(&h) {
        0
    } else {
        2
    }
}

/// CPU side of the map mesh, sorted by (layer, chunk).
pub struct MapMesh {
    pub verts: Vec<MapVertex>,
    pub idx: Vec<u32>,
    /// Per layer: (chunk bounds [minx, minz, maxx, maxz], first index, count).
    pub ranges: [Vec<([f32; 4], u32, u32)>; LAYERS],
    pub bounds: [f32; 4],
    pub build_ms: f64,
}

impl MapMesh {
    pub fn build(city: &City) -> MapMesh {
        Self::build_with_tunnels(city, false)
    }

    pub fn build_with_tunnels(city: &City, tunnels: bool) -> MapMesh {
        let t = std::time::Instant::now();
        // Bucket every primitive by (layer, chunk) first.
        let chunk_of = |x: f32, z: f32| ((x / CHUNK_M).floor() as i32, (z / CHUNK_M).floor() as i32);
        let mut buckets: std::collections::BTreeMap<(usize, i32, i32), (Vec<MapVertex>, Vec<u32>)> = Default::default();
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let mut grow = |x: f32, z: f32| {
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(z);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(z);
        };
        let mut ear = earcut::Earcut::<f32>::new();
        let mut tri: Vec<u32> = vec![];
        for b in &city.buildings {
            if b.ring.len() < 3 {
                continue;
            }
            let (sx, sz) = b.ring.iter().fold((0.0, 0.0), |a, p| (a.0 + p[0], a.1 + p[1]));
            let n = b.ring.len() as f32;
            let key = chunk_of(sx / n, sz / n);
            ear.earcut(b.ring.iter().copied(), &[] as &[u32], &mut tri);
            if tri.is_empty() {
                continue;
            }
            let e = buckets.entry((LAYER_BUILDINGS, key.0, key.1)).or_default();
            let base = e.0.len() as u32;
            for p in &b.ring {
                grow(p[0], p[1]);
                e.0.push(MapVertex { pos: *p, dir: [0.0; 2], ext: [0.0; 2], class: 0 });
            }
            e.1.extend(tri.iter().map(|i| base + i));
        }
        for r in &city.roads {
            if r.tunnel && !(tunnels && eo_data::standards::is_vehicle_way(&r.highway)) {
                continue;
            }
            let cls = road_class(&r.highway);
            for w in r.points.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l = (dx * dx + dz * dz).sqrt();
                if l < 1e-3 {
                    continue;
                }
                let dir = [dx / l, dz / l];
                grow(a[0], a[1]);
                grow(b[0], b[1]);
                let key = chunk_of((a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5);
                let layer = if r.tunnel { TUNNELS } else { 1 + cls };
                let e = buckets.entry((layer, key.0, key.1)).or_default();
                let base = e.0.len() as u32;
                let c = (1 + cls) as u32 | if r.tunnel { 8 } else { 0 };
                // Square caps: the quad runs half a width past each end, so
                // consecutive segments overlap at their joints.
                e.0.push(MapVertex { pos: a, dir, ext: [-1.0, -1.0], class: c });
                e.0.push(MapVertex { pos: b, dir, ext: [1.0, -1.0], class: c });
                e.0.push(MapVertex { pos: b, dir, ext: [1.0, 1.0], class: c });
                e.0.push(MapVertex { pos: a, dir, ext: [-1.0, 1.0], class: c });
                e.1.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        let mut verts = vec![];
        let mut idx = vec![];
        let mut ranges: [Vec<([f32; 4], u32, u32)>; LAYERS] = Default::default();
        for ((layer, cx, cz), (v, i)) in buckets {
            let base = verts.len() as u32;
            let first = idx.len() as u32;
            verts.extend_from_slice(&v);
            idx.extend(i.iter().map(|k| base + k));
            // Chunk bounds padded for the widest road and footprints that straddle the line.
            let pad = 200.0;
            let b = [cx as f32 * CHUNK_M - pad, cz as f32 * CHUNK_M - pad, (cx + 1) as f32 * CHUNK_M + pad, (cz + 1) as f32 * CHUNK_M + pad];
            ranges[layer].push((b, first, i.len() as u32));
        }
        MapMesh { verts, idx, ranges, bounds, build_ms: t.elapsed().as_secs_f64() * 1e3 }
    }
}

/// Nearest named street to a point (`closestStreet` in src/ui/pause-map.js).
pub struct StreetIndex {
    cell: f32,
    cells: std::collections::HashMap<(i32, i32), Vec<(u32, u32)>>,
    names: Vec<String>,
    segs: Vec<([f32; 2], [f32; 2])>,
}

impl StreetIndex {
    pub fn build(city: &City) -> StreetIndex {
        let cell = 64.0f32;
        let mut cells: std::collections::HashMap<(i32, i32), Vec<(u32, u32)>> = Default::default();
        let mut names = vec![];
        let mut segs = vec![];
        for r in &city.roads {
            let name = r.name.trim();
            if name.is_empty() || (name.starts_with("SF Rd") && name[5..].trim_start().starts_with(|c: char| c.is_ascii_digit())) {
                continue;
            }
            let ni = names.len() as u32;
            names.push(name.to_string());
            for w in r.points.windows(2) {
                let si = segs.len() as u32;
                segs.push((w[0], w[1]));
                let (x0, x1) = (w[0][0].min(w[1][0]), w[0][0].max(w[1][0]));
                let (z0, z1) = (w[0][1].min(w[1][1]), w[0][1].max(w[1][1]));
                for cx in (x0 / cell).floor() as i32..=(x1 / cell).floor() as i32 {
                    for cz in (z0 / cell).floor() as i32..=(z1 / cell).floor() as i32 {
                        cells.entry((cx, cz)).or_default().push((ni, si));
                    }
                }
            }
        }
        StreetIndex { cell, cells, names, segs }
    }

    /// The nearest named street within `max_m`, and its distance.
    pub fn nearest(&self, x: f32, z: f32, max_m: f32) -> Option<(&str, f32)> {
        let (cx, cz) = ((x / self.cell).floor() as i32, (z / self.cell).floor() as i32);
        let reach = (max_m / self.cell).ceil() as i32;
        let mut best: Option<(u32, f32)> = None;
        for dx in -reach..=reach {
            for dz in -reach..=reach {
                let Some(list) = self.cells.get(&(cx + dx, cz + dz)) else { continue };
                for &(ni, si) in list {
                    let (a, b) = self.segs[si as usize];
                    let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
                    let l2 = (ex * ex + ez * ez).max(1e-6);
                    let t = (((x - a[0]) * ex + (z - a[1]) * ez) / l2).clamp(0.0, 1.0);
                    let d = ((x - a[0] - t * ex).powi(2) + (z - a[1] - t * ez).powi(2)).sqrt();
                    if d <= max_m && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((ni, d));
                    }
                }
            }
        }
        best.map(|(ni, d)| (self.names[ni as usize].as_str(), d))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
struct ViewU {
    view: [f32; 4],
    size: [f32; 4],
    lc: [f32; 4],
    lcs: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct PaletteU {
    pub colors: [[f32; 4]; 8],
    pub widths: [[f32; 4]; 8],
    pub land: [[f32; 4]; 6],
}

fn c4(c: Color) -> [f32; 4] {
    c.to_f32()
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Which cartography a target draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Style {
    /// The HUD radar and title plate: `--hud-map-*` tokens.
    Hud,
    /// The pause map: `MAP_PALETTE`, widths and fades by zoom.
    Pause,
}

/// Palettes for one render: the main pass and the pass drawn under the
/// roads (HUD sidewalks, the pause map's freeway casing).
pub fn palettes(style: Style, s: f32) -> (PaletteU, PaletteU) {
    let mut main = PaletteU::default();
    let mut under = PaletteU::default();
    match style {
        Style::Hud => {
            let h = theme::hud::MAP_GROUND;
            main.land = [c4(h), c4(theme::hud::MAP_WATER), c4(theme::hud::MAP_PARK), c4(theme::hud::MAP_PARK), c4(h), c4(h)];
            under.land = main.land;
            main.colors[0] = c4(theme::hud::MAP_ROOF);
            main.widths[0] = [0.0, 0.0, 1.0, 0.0];
            for cls in 0..6 {
                // `hudMapRoadStyle`: width clamp(4, 22), sidewalk 2.4 m each side.
                let w = theme::map::ROAD_WIDTH_M[cls].clamp(4.0, 22.0);
                let major = cls >= 3;
                let col = if major { theme::hud::MAP_ROAD } else { theme::hud::MAP_ROAD_MINOR };
                let alpha = if cls == 0 { 0.0 } else { 1.0 };
                main.colors[1 + cls] = c4(col);
                main.widths[1 + cls] = [w * 0.5, 1.0, alpha, 0.0];
                under.colors[1 + cls] = c4(theme::hud::MAP_SIDEWALK);
                under.widths[1 + cls] = [w * 0.5 + 2.4, 1.0, alpha, 0.0];
            }
        }
        Style::Pause => {
            let m = theme::map::LAND;
            // WorldCover "bare" is not the JS map's sand (beach polygons): it stays land.
            main.land = [c4(m), c4(theme::map::WATER), c4(theme::map::PARK), c4(theme::map::WOOD), c4(m), c4(theme::map::WATER_DEEP)];
            under.land = main.land;
            main.colors[0] = c4(theme::map::BUILDING);
            main.widths[0] = [0.0, 0.0, smooth(0.55, 1.1, s), 0.0];
            let far = smooth(0.05, 0.6, s);
            for cls in 0..6 {
                let metres = theme::map::ROAD_WIDTH_M[cls] * if cls == 0 { 0.6 } else { 0.82 };
                let floor = theme::map::ROAD_FLOOR_PX[cls] * (0.6 + 0.6 * far);
                let mut alpha = match cls {
                    0 => smooth(1.2, 2.0, s) * 0.8,
                    1 => smooth(0.45, 0.9, s),
                    _ => 1.0,
                };
                if (cls == 0 && s < 1.2) || (cls == 1 && s < 0.45) {
                    alpha = 0.0;
                }
                main.colors[1 + cls] = c4(theme::map::ROADS[cls]);
                main.widths[1 + cls] = [metres * 0.5, floor, alpha, 0.0];
                if cls == 5 {
                    // Casing: the freeway's width plus 2.2 px.
                    under.colors[6] = c4(theme::map::FREEWAY_CASING);
                    let wpx = floor.max(metres * s) + 2.2;
                    under.widths[6] = [wpx * 0.5 / s.max(1e-4), 0.0, 1.0, 0.0];
                }
            }
        }
    }
    (main, under)
}

/// The view a target shows: centre (metres), scale (px per metre), size (px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapView {
    pub cx: f32,
    pub cz: f32,
    pub s: f32,
    pub w: u32,
    pub h: u32,
}

/// One render-to-texture map image.
pub struct MapTarget {
    pub style: Style,
    pub texture: wgpu::Texture,
    pub view_tex: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
    vbuf: wgpu::Buffer,
    pbuf: [wgpu::Buffer; 2],
    bind0: wgpu::BindGroup,
    bind_pal: [wgpu::BindGroup; 2],
    pub current: Option<MapView>,
    pub renders: u32,
}

/// GPU side of the map: the mesh, the land-cover texture and the pipelines.
pub struct MapGpu {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    ranges: [Vec<([f32; 4], u32, u32)>; LAYERS],
    landcover_view: wgpu::TextureView,
    lc: [f32; 4],
    lcs: [f32; 4],
    layout0: wgpu::BindGroupLayout,
    layout1: wgpu::BindGroupLayout,
    fill: [wgpu::RenderPipeline; 2],
    mesh: [wgpu::RenderPipeline; 2],
    pub bounds: [f32; 4],
    pub triangles: u64,
}

pub const MAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl MapGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, mesh: &MapMesh, land: Option<&LandCover>) -> MapGpu {
        use wgpu::util::DeviceExt;
        let empty = [MapVertex::default()];
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("eo-ui map vertices"),
            contents: bytemuck::cast_slice(if mesh.verts.is_empty() { &empty[..] } else { &mesh.verts }),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("eo-ui map indices"),
            contents: bytemuck::cast_slice(if mesh.idx.is_empty() { &[0u32, 0, 0][..] } else { &mesh.idx }),
            usage: wgpu::BufferUsages::INDEX,
        });
        let (lw, lh, classes, lc) = match land {
            Some(l) => {
                let f = l.frame;
                let ax = 1.0 / (f.m_per_deg_lon * l.d_lon);
                let bx = (f.center_lon - l.west) / l.d_lon;
                let az = 1.0 / (f.m_per_deg_lat * l.d_lat);
                let bz = (l.north - f.center_lat) / l.d_lat;
                (l.width as u32, l.height as u32, l.classes.clone(), [ax as f32, bx as f32, az as f32, bz as f32])
            }
            None => (1, 1, vec![0u8], [0.0; 4]),
        };
        let lct = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("eo-ui land cover"),
                size: wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &classes,
        );
        let landcover_view = lct.create_view(&Default::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("eo-ui map"), source: wgpu::ShaderSource::Wgsl(include_str!("map.wgsl").into()) });
        let uniform = |binding, vis| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("eo-ui map view"),
            entries: &[
                uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });
        let layout1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("eo-ui map palette"), entries: &[uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT)] });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("eo-ui map"), bind_group_layouts: &[Some(&layout0), Some(&layout1)], immediate_size: 0 });
        let mk = |vs: &str, fs: &str, mesh: bool, samples: u32| {
            let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Uint32];
            let buffers = [Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<MapVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("eo-ui map"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: if mesh { &buffers } else { &[] } },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: MAP_FORMAT,
                        blend: if mesh { Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING) } else { None },
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                multiview_mask: None,
                cache: None,
            })
        };
        let fill = [mk("vs_fill", "fs_fill", false, 1), mk("vs_fill", "fs_fill", false, 4)];
        let meshp = [mk("vs_mesh", "fs_mesh", true, 1), mk("vs_mesh", "fs_mesh", true, 4)];
        MapGpu {
            vbuf,
            ibuf,
            ranges: mesh.ranges.clone(),
            landcover_view,
            lc,
            lcs: [lw as f32, lh as f32, 0.0, 0.0],
            layout0,
            layout1,
            fill,
            mesh: meshp,
            bounds: mesh.bounds,
            triangles: mesh.idx.len() as u64 / 3,
        }
    }

    /// A target texture of `w` x `h` (4x MSAA resolved when `msaa`).
    pub fn target(&self, device: &wgpu::Device, style: Style, w: u32, h: u32, msaa: bool) -> MapTarget {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eo-ui map image"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MAP_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view_tex = texture.create_view(&Default::default());
        let msaa = msaa.then(|| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("eo-ui map msaa"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 4,
                    dimension: wgpu::TextureDimension::D2,
                    format: MAP_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let mk = |label, size| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let vbuf = mk("eo-ui map view", std::mem::size_of::<ViewU>() as u64);
        let pbuf = [mk("eo-ui map palette", std::mem::size_of::<PaletteU>() as u64), mk("eo-ui map palette under", std::mem::size_of::<PaletteU>() as u64)];
        let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("eo-ui map view"),
            layout: &self.layout0,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: vbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.landcover_view) },
            ],
        });
        let bp = |b: &wgpu::Buffer| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("eo-ui map palette"), layout: &self.layout1, entries: &[wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() }] });
        let bind_pal = [bp(&pbuf[0]), bp(&pbuf[1])];
        MapTarget { style, texture, view_tex, msaa, vbuf, pbuf, bind0, bind_pal, current: None, renders: 0 }
    }

    /// Redraw a target for `view` (does nothing if it already shows it).
    pub fn render(&self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, t: &mut MapTarget, view: MapView) -> bool {
        if t.current == Some(view) {
            return false;
        }
        t.current = Some(view);
        t.renders += 1;
        let u = ViewU { view: [view.cx, view.cz, view.s, 0.0], size: [view.w as f32, view.h as f32, 0.0, 0.0], lc: self.lc, lcs: self.lcs };
        queue.write_buffer(&t.vbuf, 0, bytemuck::bytes_of(&u));
        let (main, under) = palettes(t.style, view.s);
        queue.write_buffer(&t.pbuf[0], 0, bytemuck::bytes_of(&main));
        queue.write_buffer(&t.pbuf[1], 0, bytemuck::bytes_of(&under));
        let ms = t.msaa.is_some() as usize;
        let (attach, resolve) = match &t.msaa {
            Some(m) => (m, Some(&t.view_tex)),
            None => (&t.view_tex, None),
        };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("eo-ui map"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: attach,
                depth_slice: None,
                resolve_target: resolve,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &t.bind0, &[]);
        pass.set_bind_group(1, &t.bind_pal[0], &[]);
        pass.set_pipeline(&self.fill[ms]);
        pass.draw(0..3, 0..1);
        // The visible rectangle in metres.
        let (hw, hh) = (view.w as f32 * 0.5 / view.s, view.h as f32 * 0.5 / view.s);
        let rect = [view.cx - hw, view.cz - hh, view.cx + hw, view.cz + hh];
        pass.set_pipeline(&self.mesh[ms]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        let draw_layer = |pass: &mut wgpu::RenderPass<'_>, layer: usize| {
            let mut run: Option<(u32, u32)> = None;
            for (b, first, count) in &self.ranges[layer] {
                let visible = b[0] <= rect[2] && b[2] >= rect[0] && b[1] <= rect[3] && b[3] >= rect[1];
                if !visible {
                    continue;
                }
                run = match run {
                    Some((s, e)) if e == *first => Some((s, first + count)),
                    Some((s, e)) => {
                        pass.draw_indexed(s..e, 0, 0..1);
                        Some((*first, first + count))
                    }
                    None => Some((*first, first + count)),
                };
            }
            if let Some((s, e)) = run {
                pass.draw_indexed(s..e, 0, 0..1);
            }
        };
        draw_layer(&mut pass, LAYER_BUILDINGS);
        draw_layer(&mut pass, TUNNELS);
        pass.set_bind_group(1, &t.bind_pal[1], &[]);
        match t.style {
            Style::Hud => {
                for layer in 1..TUNNELS {
                    draw_layer(&mut pass, layer);
                }
            }
            Style::Pause => draw_layer(&mut pass, 6),
        }
        pass.set_bind_group(1, &t.bind_pal[0], &[]);
        for layer in 1..TUNNELS {
            draw_layer(&mut pass, layer);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_underground_map_layer_contains_every_vehicle_tunnel_span() {
        let paths = eo_data::DataPaths::discover().unwrap();
        let city = City::load(&paths).unwrap();
        let spans = city.roads.iter()
            .filter(|r| r.tunnel && eo_data::standards::is_vehicle_way(&r.highway))
            .flat_map(|r| r.points.windows(2))
            .filter(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) >= 1e-3)
            .count();
        assert!(spans > 0);
        assert!(MapMesh::build(&city).ranges[TUNNELS].is_empty());
        let mesh = MapMesh::build_with_tunnels(&city, true);
        let indices: usize = mesh.ranges[TUNNELS].iter().map(|r| r.2 as usize).sum();
        assert_eq!(indices, 6 * spans);
        for &(_, first, len) in &mesh.ranges[TUNNELS] {
            for &i in &mesh.idx[first as usize..(first + len) as usize] {
                assert_ne!(mesh.verts[i as usize].class & 8, 0);
            }
        }
    }
}
