//! Site and collision integration for the Blender-built terrace at 21 Beaver Street.
use anyhow::{Result, ensure};
use eo_assets::{AssetRef, CookedModel, KitLibrary};
use eo_player::world::{GroundProvider, SurfaceMesh, TileKey};
use eo_render::authored::MaterialTable;
use eo_render::scene::{BLEND_BIT, GpuObject, ObjectKind, Scene, Vertex};
use glam::{DVec2, DVec3, Mat3, Vec3};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

pub const ID: &str = "266146886";
pub fn owns(id: &str) -> bool {
    id.trim_start_matches("sf-building-") == ID
}

type Point = [f64; 3];
#[derive(Deserialize)]
struct Manifest {
    #[serde(flatten)]
    asset: AssetRef,
    boxes: Vec<[Point; 2]>,
    ramps: Vec<[Point; 4]>,
}
const FACES: [[usize; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [3, 7, 6, 2], [0, 4, 7, 3], [1, 2, 6, 5]];
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
    materials: Vec<u32>,
    light_transfer: Vec<[f32; 2]>,
    collision: Vec<[Point; 3]>,
    steps: Vec<[Point; 2]>,
}

impl House {
    pub fn build(
        interior_lighting: bool,
        b: &eo_data::Building,
        ground: &dyn Fn(f64, f64) -> f64,
        root: &Path,
        kit: &KitLibrary,
        materials: &mut MaterialTable,
    ) -> Result<Self> {
        ensure!(b.ring.len() == 4, "unexpected 21 Beaver footprint");
        let ring: Vec<_> = b.ring.iter().map(|p| DVec2::new(p[0] as f64, p[1] as f64)).collect();
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(
            root.join("public/assets/buildings/beaver-terrace/terrace.provenance.json"),
        )?)?;
        let raw = eo_assets::glb::read_verified(&manifest.asset.path(root), &manifest.asset)?;
        let (model, light_transfer) = if interior_lighting {
            eo_assets::glb::cook_glb_with_uv0(&raw)?
        } else {
            (eo_assets::glb::cook_glb(&raw)?, Vec::new())
        };
        let resolved = model.materials.iter().map(|m| materials.resolve(kit, m)).collect();
        let floor = ring.iter().map(|p| ground(p.x, p.y)).fold(f64::NEG_INFINITY, f64::max) + 0.22;
        let mut h = Self {
            origin: ring[1],
            across: (ring[2] - ring[1]) / 7.,
            back: (ring[0] - ring[1]) / 13.,
            floor,
            height_scale: b.height as f64 / 10.,
            model,
            materials: resolved,
            light_transfer,
            collision: vec![],
            steps: vec![],
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
            h.solid_box(a, b);
            h.steps.push([a, b]);
            let p = h.point(5.95, y, z);
            if p.y <= ground(p.x, p.z) + 0.12 {
                break;
            }
        }
        Ok(h)
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
        let angle = self.across.y.atan2(self.across.x).rem_euclid(std::f64::consts::TAU);
        let angle_byte = (angle / std::f64::consts::TAU * 255.0).round() as u32;
        let mut finish = vec![0_u8; self.model.positions.len()];
        for part in &self.model.parts {
            let name = &self.model.materials[part.material as usize].name;
            let code = if name.starts_with("oiled-oak@") { 255 } else if name.starts_with("lime-plaster@") { 100 } else { 0 };
            if code != 0 {
                let start = part.first_index as usize;
                for &i in &self.model.indices[start..start + part.index_count as usize] {
                    finish[i as usize] = code;
                }
            }
        }
        for (i, (&[x, y, z], &n)) in self.model.positions.iter().zip(&self.model.normals).enumerate() {
            let mut normal = eo_assets::glb::pack_normal(normals * eo_assets::glb::unpack_normal(n));
            let color = if let Some(&[day, lamp]) = self.light_transfer.get(i) {
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
            scene.vertices.push(Vertex {
                pos: self.point(x.into(), y.into(), z.into()).as_vec3().to_array(),
                color,
                normal,
            });
        }
        scene.indices.extend_from_slice(&self.model.indices);
        for part in &self.model.parts {
            let (lo, hi) = self.bounds(part.min, part.max);
            let mut o = GpuObject::new(
                ObjectKind::Model,
                lo,
                hi,
                first as u32 + part.first_index,
                part.index_count,
                base as i32,
            );
            o.material = self.materials[part.material as usize];
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
                    color: 0xff9aa4aa,
                }));
                scene.indices.extend([v, v + 1, v + 2, v, v + 2, v + 3]);
            }
            let (lo, hi) = self.bounds(a.map(|v| v as f32), b.map(|v| v as f32));
            scene
                .objects
                .push(GpuObject::new(ObjectKind::Landmark, lo, hi, start as u32, 36, 0));
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
