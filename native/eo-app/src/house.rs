//! Site and collision integration for the Blender-built terrace at 21 Beaver Street.
mod door;
use anyhow::{Result, ensure};
pub use door::Door;
use eo_assets::{AssetRef, CookedModel, KitLibrary};
use eo_player::world::{GroundProvider, SurfaceMesh, TileKey};
use eo_render::authored::MaterialTable;
use eo_render::scene::{BLEND_BIT, GpuObject, ObjectKind, Scene, Vertex};
use glam::{DVec2, DVec3, Mat3, Vec3};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

const FINISH_DIR: &str = "public/assets/buildings/beaver-terrace/finishes";
const FLOOR_MATERIALS: [&str; 4] = [
    "oiled-oak@ba9a79",
    "oiled-oak@b49575",
    "oiled-oak@ba9e82",
    "oiled-oak@b6987a",
];
pub const ID: &str = "266146886";
pub fn owns(id: &str) -> bool {
    id.trim_start_matches("sf-building-") == ID
}

type Point = [f64; 3];
#[derive(Deserialize)]
struct Manifest {
    #[serde(flatten)]
    asset: AssetRef,
    door: DoorSpec,
    boxes: Vec<[Point; 2]>,
    ramps: Vec<[Point; 4]>,
}
#[derive(Deserialize)]
struct DoorSpec {
    hinge: Point,
    bounds: [Point; 2],
    materials: Vec<String>,
}

#[derive(Clone, Deserialize)]
struct ProbeVolume {
    min: [f32; 3],
    max: [f32; 3],
}

#[derive(Deserialize)]
struct ProbeManifest {
    #[serde(flatten)]
    asset: AssetRef,
    house_sha256: String,
    probes: [ProbeVolume; 2],
}

pub struct Finishes {
    manifest: ProbeManifest,
    pub atlas: eo_assets::kit::BakedMaps,
    pub floorboards: eo_assets::kit::BakedMaps,
}

impl Finishes {
    pub fn load(root: &Path) -> Result<Self> {
        let manifest: ProbeManifest = serde_json::from_slice(&std::fs::read(
            root.join(FINISH_DIR)
                .join("kitchen-reflections.provenance.json"),
        )?)?;
        let floor: AssetRef = serde_json::from_slice(&std::fs::read(
            root.join(FINISH_DIR).join("floorboards.provenance.json"),
        )?)?;
        let atlas = Self::read_maps(root, &manifest.asset, 2)?;
        let floorboards = Self::read_maps(root, &floor, 3)?;
        Ok(Self {
            manifest,
            atlas,
            floorboards,
        })
    }

    fn read_maps(root: &Path, asset: &AssetRef, pages: usize) -> Result<eo_assets::kit::BakedMaps> {
        let bytes = eo_assets::glb::read_verified(&asset.path(root), asset)?;
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
        let res = eo_assets::kit::HIGH_RES;
        let info = reader.info();
        ensure!(
            info.width as usize == res
                && info.height as usize == pages * res
                && info.color_type == png::ColorType::Rgba
                && info.bit_depth == png::BitDepth::Eight,
            "unexpected image format for {}",
            asset.url
        );
        let size = res * res * 4;
        let mut albedo = vec![0; pages * size];
        reader.next_frame(&mut albedo)?;
        let orm = if pages == 3 {
            albedo.split_off(size * 2)
        } else {
            vec![0; size]
        };
        let normal = albedo.split_off(size);
        Ok(eo_assets::kit::BakedMaps {
            res,
            metres_per_tile: 2.,
            normal,
            orm,
            albedo,
        })
    }
}

const FACES: [[usize; 4]; 6] = [
    [0, 3, 2, 1],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [3, 7, 6, 2],
    [0, 4, 7, 3],
    [1, 2, 6, 5],
];
fn corners([x, y, z]: Point, [u, v, w]: Point) -> [Point; 8] {
    [
        [x, y, z],
        [u, y, z],
        [u, v, z],
        [x, v, z],
        [x, y, w],
        [u, y, w],
        [u, v, w],
        [x, v, w],
    ]
}

pub struct House {
    origin: DVec2,
    across: DVec2,
    back: DVec2,
    floor: f64,
    height_scale: f64,
    model: CookedModel,
    door: DoorSpec,
    materials: Vec<u32>,
    light_transfer: Vec<[f32; 2]>,
    probes: Option<[ProbeVolume; 2]>,
    collision: Vec<[Point; 3]>,
    steps: Vec<[Point; 2]>,
    pavers: Vec<([Point; 4], u32)>,
    aggregate: Vec<([Point; 4], u32)>,
    aggregate_material: u32,
    entry: Option<DVec3>,
}

impl House {
    pub fn build(
        interior_lighting: bool,
        finishes: Option<&Finishes>,
        b: &eo_data::Building,
        ground: &dyn Fn(f64, f64) -> f64,
        street: Option<&eo_street::StreetWorld>,
        root: &Path,
        kit: &KitLibrary,
        materials: &mut MaterialTable,
    ) -> Result<Self> {
        ensure!(b.ring.len() == 4, "unexpected 21 Beaver footprint");
        let ring: Vec<_> = b
            .ring
            .iter()
            .map(|p| DVec2::new(p[0] as f64, p[1] as f64))
            .collect();
        let dir = if finishes.is_some() {
            FINISH_DIR
        } else {
            "public/assets/buildings/beaver-terrace"
        };
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(
            root.join(dir).join("terrace.provenance.json"),
        )?)?;
        if let Some(r) = finishes {
            ensure!(
                r.manifest.house_sha256 == manifest.asset.sha256,
                "kitchen reflections were baked for a different house"
            );
        }
        let raw = eo_assets::glb::read_verified(&manifest.asset.path(root), &manifest.asset)?;
        let (model, light_transfer) = if interior_lighting {
            eo_assets::glb::cook_glb_with_uv0(&raw)?
        } else {
            (eo_assets::glb::cook_glb(&raw)?, Vec::new())
        };
        let resolved: Vec<u32> = model
            .materials
            .iter()
            .map(|m| materials.resolve(kit, m))
            .collect();
        let aggregate_material = model
            .materials
            .iter()
            .position(|m| m.name == "decomposed-granite@b4a087")
            .map_or(0, |i| resolved[i]);
        let floor = ring
            .iter()
            .map(|p| ground(p.x, p.y))
            .fold(f64::NEG_INFINITY, f64::max)
            + 0.22;
        let mut h = Self {
            origin: ring[1],
            across: (ring[2] - ring[1]) / 7.,
            back: (ring[0] - ring[1]) / 13.,
            floor,
            height_scale: b.height as f64 / 10.,
            model,
            door: manifest.door,
            materials: resolved,
            light_transfer,
            probes: finishes.map(|r| r.manifest.probes.clone()),
            collision: vec![],
            steps: vec![],
            pavers: vec![],
            aggregate: vec![],
            aggregate_material,
            entry: None,
        };
        for [a, b] in manifest.boxes {
            h.solid_box(a, b);
        }
        for ramp in manifest.ramps {
            h.solid_quad(ramp);
        }
        // The front stoop follows the surveyed ground at this scale.
        for i in 0..48 {
            let z = -(i as f64 + 1.) * 0.33;
            let y = -(i as f64) * 0.145;
            let a = [5.15, y - 0.50, z];
            let b = [6.75, y, z + 0.33];
            // The street falls across the lot. A horizontal tread that clears
            // the centre can still penetrate the uphill corner.
            if finishes.is_some()
                && i > 0
                && [a[0], b[0]].iter().any(|&x| {
                    let p = h.point(x, y, z);
                    p.y < ground(p.x, p.z) + 0.036
                })
            {
                break;
            }
            h.solid_box(a, b);
            h.steps.push([a, b]);
            let p = h.point(5.95, y, z);
            if p.y <= ground(p.x, p.z) + 0.12 {
                break;
            }
        }
        if finishes.is_some()
            && let Some(street) = street
        {
            h.approach(street, ground);
        }
        if finishes.is_some() {
            h.tile_treads();
        }
        Ok(h)
    }

    /// Individually jointed clay tiles wrap around the noses of the
    /// ground-fitted private steps. The structural tread owns collision.
    fn tile_treads(&mut self) {
        let fired_clay = [0xff6382a8, 0xff6686b0, 0xff6b8bb2, 0xff6583a7, 0xff6e8bb4];
        for (step, &[a, b]) in self.steps.iter().enumerate() {
            let top = b[1] + 0.007;
            let front = a[2];
            let width = (b[0] - a[0]) / 5.;
            for tile in 0..5 {
                let x = a[0] + width * tile as f64 + 0.003;
                let u = x + width - 0.006;
                let clay = fired_clay[(step * 3 + tile * 2) % fired_clay.len()];
                // The mortar joint continues over the curved edge and down its lip.
                let profile = [
                    (b[2] - 0.005, top),
                    (front + 0.009, top),
                    (front - 0.005, top - 0.004),
                    (front - 0.011, top - 0.013),
                    (front - 0.011, top - 0.034),
                ];
                for ends in profile.windows(2) {
                    let (z, y) = ends[0];
                    let (w, h) = ends[1];
                    self.pavers
                        .push(([[x, y, z], [u, y, z], [u, h, w], [x, h, w]], clay));
                }
            }
        }
    }

    /// Fit the private entry path between the last tread and the first mapped
    /// paving. Both edges are clipped independently on the sloping lot.
    fn approach(&mut self, street: &eo_street::StreetWorld, ground: &dyn Fn(f64, f64) -> f64) {
        let Some(&[a, b]) = self.steps.last() else {
            return;
        };
        let start = a[2];
        let edge = |x, start| -> Option<f64> {
            let paved = |z| {
                let p = self.point(x, 0., z);
                street.paved_layer_at(p.x, p.z)
            };
            if paved(start).is_some() {
                return None;
            }
            let mut inside = start;
            for i in 1..=64 {
                let mut outside = start - f64::from(i) * 0.25;
                if let Some(layer) = paved(outside) {
                    if !matches!(
                        layer,
                        eo_street::Layer::Sidewalk
                            | eo_street::Layer::Verge
                            | eo_street::Layer::Ramp
                    ) {
                        return None;
                    }
                    for _ in 0..12 {
                        let mid = (inside + outside) * 0.5;
                        if paved(mid).is_some() {
                            outside = mid;
                        } else {
                            inside = mid;
                        }
                    }
                    return Some(inside);
                }
                inside = outside;
            }
            None
        };
        let Some(left) = edge(a[0], start) else {
            return;
        };
        let Some(right) = edge(b[0], start) else {
            return;
        };
        let end = (left + right) * 0.5;
        if start - end < 0.03 {
            return;
        }
        let span = b[0] - a[0];
        let terrain = |x, z| {
            let p = self.point(x, 0., z);
            (ground(p.x, p.z) - self.floor) / self.height_scale
        };
        // Start the landing just under the last level tread. Ease the raised
        // downhill corner back to grade before the public paving.
        let landing = (b[1] - 0.13)
            .max(terrain(a[0], start) + 0.012)
            .max(terrain(b[0], start) + 0.012);
        let pos = |x: f64, t: f64| {
            let z = start + ((left + (right - left) * ((x - a[0]) / span)) - start) * t;
            let surface = terrain(x, z) + 0.012;
            let lift = (landing - terrain(x, start) - 0.012).max(0.) * (1. - t) * (1. - t);
            [x, surface + lift, z]
        };
        let courses = ((start - end) / 0.37).ceil() as usize;
        let mut tiles = Vec::new();
        let mut walk = Vec::new();
        for j in 0..courses {
            let t0 = j as f64 / courses as f64;
            let t1 = (j + 1) as f64 / courses as f64;
            let course = [pos(a[0], t0), pos(b[0], t0), pos(b[0], t1), pos(a[0], t1)];
            walk.push(course);
            // Bed the pavers in mortar and close the exposed downhill edge
            // of the fill. The terrain can be well below the raised landing.
            let mortar = course.map(|[x, y, z]| [x, y - 0.008, z]);
            tiles.push((mortar, 0xff829198));
            for (from, to) in [(mortar[0], mortar[3]), (mortar[2], mortar[1])] {
                let toe = |[x, _, z]: Point| [x, terrain(x, z) - 0.028, z];
                tiles.push(([from, to, toe(to), toe(from)], 0xff80909c));
            }
            for k in 0..5 {
                let x = a[0] + (k as f64 - 0.5 * (j % 2) as f64) * span / 4.;
                let u = (x + span / 4.).min(b[0]) - 0.004;
                let x = x.max(a[0]) + 0.004;
                if u <= x {
                    continue;
                }
                let inset = 0.003 / (start - end);
                let quad = [
                    pos(x, t0 + inset),
                    pos(u, t0 + inset),
                    pos(u, t1 - inset),
                    pos(x, t1 - inset),
                ];
                let color = [0xff698cae, 0xff7c9eb7, 0xff7195b5, 0xff6f90b1][(j * 7 + k * 3) % 4];
                tiles.push((quad, color));
            }
        }
        // The retaining wall enters a graded gravel bed on the private side
        // of the sidewalk. Clip every strip independently to the surveyed edge.
        let mut aggregate = Vec::new();
        for (lo, hi) in [(0.10, a[0]), (b[0], 6.91)] {
            let columns = ((hi - lo) / 0.26).ceil() as usize;
            let mut last = None;
            for i in 0..=columns {
                let x = lo + (hi - lo) * i as f64 / columns as f64;
                let rear = -0.04;
                let Some(outer) = edge(x, rear) else {
                    last = None;
                    continue;
                };
                if let Some((prev_x, prev_outer)) = last {
                    let grade = |xx, zz| [xx, terrain(xx, zz) + 0.006, zz];
                    for row in 0..12 {
                        let t = row as f64 / 12.;
                        let u = (row + 1) as f64 / 12.;
                        aggregate.push((
                            [
                                grade(prev_x, rear + (prev_outer - rear) * t),
                                grade(x, rear + (outer - rear) * t),
                                grade(x, rear + (outer - rear) * u),
                                grade(prev_x, rear + (prev_outer - rear) * u),
                            ],
                            0xffffffff,
                        ));
                    }
                }
                last = Some((x, outer));
            }
        }
        self.aggregate = aggregate;
        for q in walk {
            self.solid_quad(q);
        }
        self.pavers = tiles;
        self.entry = Some(self.point((a[0] + b[0]) * 0.5, 0., end));
    }

    pub fn entry_yaw(&self) -> f64 {
        self.back.x.atan2(self.back.y)
    }

    pub fn entry_position(&self) -> DVec3 {
        self.entry.unwrap_or_else(|| self.point(5.95, 0., -3.3))
    }

    #[cfg(test)]
    pub fn lowest_tread_corners(&self) -> [DVec3; 2] {
        let [a, b] = self.steps.last().expect("the house has a private stair");
        [a[0], b[0]].map(|x| self.point(x, b[1], a[2]))
    }

    pub fn point(&self, x: f64, y: f64, z: f64) -> DVec3 {
        let p = self.origin + self.across * x + self.back * z;
        DVec3::new(p.x, self.floor + y * self.height_scale, p.y)
    }
    fn solid_quad(&mut self, q: [Point; 4]) {
        let pts = q.map(|[x, y, z]| self.point(x, y, z).to_array());
        for tri in [[0, 1, 2], [0, 2, 3]] {
            self.collision.push(tri.map(|i| pts[i]));
        }
    }
    fn solid_box(&mut self, a: Point, b: Point) {
        let pts = corners(a, b);
        for face in FACES {
            self.solid_quad(face.map(|i| pts[i]));
        }
    }
    pub fn wake_position(&self) -> DVec3 {
        self.point(3.35, 10. / 3. + 0.03, 10.8)
    }
    pub fn by_bed(&self, p: DVec3) -> bool {
        p.distance(self.wake_position()) < 1.45
    }
    fn bounds(&self, min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = -lo;
        for [x, y, z] in corners(min.map(f64::from), max.map(f64::from)) {
            let p = self.point(x, y, z).as_vec3();
            lo = lo.min(p);
            hi = hi.max(p);
        }
        (lo.to_array(), hi.to_array())
    }
    pub fn append(&self, scene: &mut Scene, materials: &MaterialTable) {
        let base = scene.vertices.len();
        let first = scene.indices.len();
        let objects = scene.objects.len();
        let normals = Mat3::from_cols(
            Vec3::new(self.across.x as f32, 0., self.across.y as f32),
            Vec3::Y * self.height_scale as f32,
            Vec3::new(self.back.x as f32, 0., self.back.y as f32),
        )
        .inverse()
        .transpose();
        // The grain runs down the long axis of the boards. Give the shader the
        // surveyed angle so grain stays parallel to each seam on rotated lots.
        let angle = self
            .across
            .y
            .atan2(self.across.x)
            .rem_euclid(std::f64::consts::TAU);
        let angle_byte = (angle / std::f64::consts::TAU * 255.0).round() as u32;
        let mut floor_uv = if self.probes.is_some() {
            vec![None; self.model.positions.len()]
        } else {
            Vec::new()
        };
        let mut finish = vec![0_u8; self.model.positions.len()];
        for part in &self.model.parts {
            let name = &self.model.materials[part.material as usize].name;
            let code = if name.starts_with("oiled-oak@") {
                255
            } else if name.starts_with("lime-plaster@") {
                100
            } else {
                0
            };
            let appliance = matches!(
                name.split('@').next().unwrap_or(""),
                "appliance-steel"
                    | "brushed-brass"
                    | "porcelain"
                    | "satin-nickel"
                    | "ivory-enamel"
                    | "smoked-enamel"
                    | "enamel-primer"
                    | "exposed-steel"
                    | "cast-iron"
                    | "iron-oxide"
                    | "seasoned-iron"
            );
            if self.probes.is_some()
                && let Some(paint) = FLOOR_MATERIALS.iter().position(|m| m == name)
            {
                let start = part.first_index as usize;
                for &i in &self.model.indices[start..start + part.index_count as usize] {
                    let [x, h, z] = self.model.positions[i as usize];
                    let section = if h > 1.0 && x >= 5.0999 { 5.1 } else { 0.0 };
                    let board = ((x - section) / 0.18 + 0.0001).max(0.0);
                    let level = (h / (10.0 / 3.0)).round() as usize;
                    let row = (board as usize * 5 + paint * 3 + level) % 8;
                    let across = (row as f32 + 0.07 + 0.86 * board.fract()) / 8.0;
                    floor_uv[i as usize] = Some([across, (z / 13.0).clamp(0.0, 1.0)]);
                }
            } else if appliance && let Some(probes) = &self.probes {
                let first = part.first_index as usize;
                for &index in &self.model.indices[first..first + part.index_count as usize] {
                    let point = self.model.positions[index as usize];
                    if let Some(row) = probes
                        .iter()
                        .position(|p| (0..3).all(|k| (p.min[k]..=p.max[k]).contains(&point[k])))
                    {
                        // Blue bytes 1 and 2 select the two rows in the reflection atlas.
                        finish[index as usize] = row as u8 + 1;
                    }
                }
            } else if code != 0 {
                let start = part.first_index as usize;
                for &i in &self.model.indices[start..start + part.index_count as usize] {
                    finish[i as usize] = code;
                }
            }
        }
        for (i, (&[x, y, z], &n)) in self
            .model
            .positions
            .iter()
            .zip(&self.model.normals)
            .enumerate()
        {
            let mut normal =
                eo_assets::glb::pack_normal(normals * eo_assets::glb::unpack_normal(n));
            let mut color = if let Some(&[day, lamp]) = self.light_transfer.get(i) {
                // Signed normal W is unused on authored models. Mark the two light channels
                // so their values survive interpolation independently of the paint colour.
                normal |= 0x7f00_0000;
                let pack = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
                (angle_byte << 24) | (u32::from(finish[i]) << 16) | pack(day) | (pack(lamp) << 8)
            } else if finish[i] != 0 {
                normal |= 0x4000_0000;
                (angle_byte << 24) | (u32::from(finish[i]) << 16)
            } else {
                0xffff_ffff
            };
            if let Some(&Some([u, v])) = floor_uv.get(i) {
                normal |= if self.light_transfer.is_empty() {
                    0x4000_0000
                } else {
                    0x7f00_0000
                };
                color = (color & 0xffff)
                    | (((u * 255.).round() as u32) << 16)
                    | (((v * 255.).round() as u32) << 24);
            }
            scene.vertices.push(Vertex {
                pos: self
                    .point(x.into(), y.into(), z.into())
                    .as_vec3()
                    .to_array(),
                color,
                normal,
            });
        }
        scene.indices.extend_from_slice(&self.model.indices);
        for part in &self.model.parts {
            let (lo, hi) = if self
                .door
                .materials
                .contains(&self.model.materials[part.material as usize].name)
            {
                // The cull and shadow tables cover the full swing, including
                // the handle. At rest or in motion it is one piece of joinery.
                let [x, y, z] = self.door.hinge;
                let hinge = self.point(x, y, z).as_vec3();
                let reach = ((self.door.bounds[1][0] - self.door.bounds[0][0] + 0.18)
                    * self.across.length()) as f32;
                let height = (self.door.bounds[1][1] * self.height_scale) as f32;
                (
                    (hinge - Vec3::new(reach, 0.01, reach)).to_array(),
                    (hinge + Vec3::new(reach, height + 0.01, reach)).to_array(),
                )
            } else {
                self.bounds(part.min, part.max)
            };
            let mut o = GpuObject::new(
                ObjectKind::Model,
                lo,
                hi,
                first as u32 + part.first_index,
                part.index_count,
                base as i32,
            );
            if self.probes.is_some()
                && FLOOR_MATERIALS
                    .contains(&self.model.materials[part.material as usize].name.as_str())
            {
                o.kind |= eo_render::scene::FLOOR_SCAN_BIT;
            }
            o.material = self.materials[part.material as usize];
            if self.aggregate_material != 0 && o.material == self.aggregate_material {
                o.kind |= eo_render::scene::GRAVEL_BIT;
            }
            if materials.gpu[o.material as usize].is_blend() {
                o.kind |= BLEND_BIT;
            }
            scene.objects.push(o);
        }
        // The fixed mesh stops at its threshold. Only the ground-fitted stoop is generated here.
        for &[a, b] in &self.steps {
            let pts = corners(a, b);
            let start = scene.indices.len();
            for face in FACES {
                let p = face.map(|i| {
                    let [x, y, z] = pts[i];
                    self.point(x, y, z).as_vec3()
                });
                let normal = eo_assets::glb::pack_normal((p[1] - p[0]).cross(p[2] - p[0]));
                let v = scene.vertices.len() as u32;
                scene.vertices.extend(p.map(|p| Vertex {
                    pos: p.to_array(),
                    normal,
                    color: if self.probes.is_some() {
                        0xffa8b6be
                    } else {
                        0xff9aa4aa
                    },
                }));
                scene.indices.extend([v, v + 1, v + 2, v, v + 2, v + 3]);
            }
            let (lo, hi) = self.bounds(a.map(|v| v as f32), b.map(|v| v as f32));
            scene.objects.push(GpuObject::new(
                ObjectKind::Landmark,
                lo,
                hi,
                start as u32,
                36,
                0,
            ));
        }
        for (quads, kind, material) in [
            (&self.pavers, ObjectKind::Landmark, 0),
            (&self.aggregate, ObjectKind::Model, self.aggregate_material),
        ] {
            if quads.is_empty() {
                continue;
            }
            let start = scene.indices.len() as u32;
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = -lo;
            for &(quad, color) in quads {
                let p = quad.map(|[x, y, z]| self.point(x, y, z).as_vec3());
                let normal = eo_assets::glb::pack_normal((p[1] - p[0]).cross(p[2] - p[0]));
                let v = scene.vertices.len() as u32;
                for point in p {
                    lo = lo.min(point);
                    hi = hi.max(point);
                    scene.vertices.push(Vertex {
                        pos: point.to_array(),
                        normal,
                        color,
                    });
                }
                scene.indices.extend([v, v + 1, v + 2, v, v + 2, v + 3]);
            }
            let mut object = GpuObject::new(
                kind,
                lo.to_array(),
                hi.to_array(),
                start,
                scene.indices.len() as u32 - start,
                0,
            );
            object.material = material;
            if material != 0 && material == self.aggregate_material {
                object.kind |= eo_render::scene::GRAVEL_BIT;
            }
            scene.objects.push(object);
        }
        scene.stats.objects += scene.objects.len() - objects;
        scene.stats.vertices += scene.vertices.len() - base;
        scene.stats.indices += scene.indices.len() - first;
        scene.stats.triangles += ((scene.indices.len() - first) / 3) as u64;
    }
}

pub struct HouseGround {
    pub base: Arc<dyn GroundProvider>,
    pub house: Arc<House>,
}
impl GroundProvider for HouseGround {
    fn ground_y(&self, x: f64, z: f64) -> f64 {
        self.base.ground_y(x, z)
    }
    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        let base = self.base.surface_tile(key);
        if key != TileKey::of(self.house.origin.x, self.house.origin.y) {
            return base;
        }
        let mut mesh = base.unwrap_or_default();
        mesh.barrier.extend_from_slice(&self.house.collision);
        Some(mesh)
    }
}
