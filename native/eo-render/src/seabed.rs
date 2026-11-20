//! Two small, original underwater habitats. Both are opaque world geometry,
//! so refraction, depth testing, shadows received and water extinction all
//! see the same bed. The scene builds them once, in metres on the drawn floor.

use crate::landmarks::{Chunk, push_chunks};
use crate::scene::{LOD_ALWAYS, MATERIAL_VERTEX_COLOUR, NO_GROUP, ObjectKind, Scene, Vertex};
use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Habitat {
    Eelgrass,
    Sand,
}

const RADIUS: f32 = 5.5;
pub(crate) const SWAY_M: f32 = 0.065; // maximum horizontal tip travel in draw.wgsl

fn colour(rgb: Vec3, bend: f32) -> u32 {
    u32::from_le_bytes([rgb.x as u8, rgb.y as u8, rgb.z as u8, (bend * 255.0) as u8])
}

// A fixed integer sequence gives the same shoots, shells and relief in every pass.
struct Sequence(u32);
impl Sequence {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 8) as f32 / 16777216.0
    }
}

fn add_vertex(mesh: &mut Chunk, p: Vec3, n: Vec3, rgb: Vec3, bend: f32) {
    let travel = Vec3::new(SWAY_M, 0.0, SWAY_M) * bend;
    mesh.min = Vec3::from(mesh.min).min(p - travel).to_array();
    mesh.max = Vec3::from(mesh.max).max(p + travel).to_array();
    mesh.verts.push(Vertex {
        pos: p.to_array(),
        normal: eo_assets::glb::pack_normal(n),
        color: colour(rgb, bend),
    });
}

fn point(xz: Vec2, y: f32) -> Vec3 {
    Vec3::new(xz.x, y, xz.y)
}

/// Wave-worked sand. A second, long wave wanders across the crest and
/// occasional crests merge. At the perimeter the relief sinks into the terrain.
fn relief(p: Vec2, habitat: Habitat) -> (f32, f32) {
    let envelope = ((RADIUS - p.length()) / 1.2).clamp(0.0, 1.0);
    let phase = p.x * 20.0
        + p.y * 2.2
        + 1.8 * (p.y * 1.1 + 0.4 * p.x).sin()
        + 0.65 * (p.x * 1.9 - p.y * 1.6).sin();
    let crest = (0.5 + 0.5 * phase.sin()).powi(2);
    let amplitude = if habitat == Habitat::Sand {
        0.034
    } else {
        0.010
    };
    (envelope * (0.023 + amplitude * crest) - 0.014, crest)
}

fn blade(mesh: &mut Chunk, root: Vec3, rng: &mut Sequence) {
    let dir = Vec2::from_angle(rng.unit() * std::f32::consts::TAU);
    let width = 0.003 + 0.003 * rng.unit();
    let length = 0.19 + 0.37 * rng.unit();
    let lean = dir * (0.05 + 0.22 * rng.unit()) + Vec2::new(0.09, -0.045);
    let twist = (rng.unit() - 0.5) * 1.9;
    let tip = Vec3::new(92.0, 103.0, 44.0) + Vec3::new(34.0, 37.0, 10.0) * rng.unit();
    let first = mesh.verts.len() as u32;
    for step in 0..=5 {
        let t = step as f32 / 5.0;
        let axis = Vec2::from_angle(twist * t).rotate(dir.perp());
        let across = point(axis, 0.0);
        let tangent = point(lean * (2.0 * t), length * (1.0 - 0.48 * t));
        let n = across.cross(tangent).normalize_or(Vec3::Y);
        let spine = root + point(lean * t * t, length * t * (1.0 - 0.24 * t));
        let w = width * (1.0 - 0.85 * t.powi(5));
        let rgb = Vec3::new(54.0, 61.0, 25.0).lerp(tip, t.sqrt());
        add_vertex(mesh, spine - across * w, n, rgb * 0.86, t * t);
        add_vertex(mesh, spine + n * (w * 0.22), n, rgb, t * t);
        add_vertex(mesh, spine + across * w, n, rgb * 0.91, t * t);
        if step > 0 {
            let a = first + 3 * (step - 1);
            for j in [a, a + 1] {
                mesh.idx.extend([j, j + 1, j + 3, j + 1, j + 4, j + 3]);
            }
        }
    }
}

/// A partly buried valve with a hinge and ribs fanning around its dome.
fn shell(mesh: &mut Chunk, root: Vec3, rng: &mut Sequence) {
    let axis = Vec2::from_angle(rng.unit() * std::f32::consts::TAU);
    let size = 0.019 + 0.031 * rng.unit();
    let tint = Vec3::new(147.0, 131.0, 110.0).lerp(Vec3::new(202.0, 190.0, 156.0), rng.unit());
    let first = mesh.verts.len() as u32;
    for ring in 0..=3 {
        let r = ring as f32 / 3.0;
        for k in 0..=12 {
            let angle = (k as f32 / 12.0 - 0.5) * 3.65;
            let spoke = axis.rotate(Vec2::from_angle(angle));
            let ridge = (k % 2) as f32;
            let p = root
                + point(
                    spoke * r * size,
                    size * (0.28 * (1.0 - r * r) + ridge * r * 0.06),
                );
            add_vertex(
                mesh,
                p,
                point(spoke * r * 0.55, 1.0).normalize(),
                tint * (0.76 + 0.15 * ridge + 0.09 * r),
                0.0,
            );
            if ring > 0 && k > 0 {
                let j = first + (ring - 1) * 13 + k - 1;
                mesh.idx.extend([j, j + 1, j + 13, j + 1, j + 14, j + 13]);
            }
        }
    }
}

pub fn append(
    scene: &mut Scene,
    habitat: Habitat,
    centre: Vec2,
    bed: &mut impl FnMut(Vec2) -> f32,
) {
    append_bed(scene, habitat, centre, bed, false);
}

/// Sheltered Bay silt carries scattered shoots and subdued ripple relief.
pub fn append_cove(scene: &mut Scene, centre: Vec2, bed: &mut impl FnMut(Vec2) -> f32) {
    append_bed(scene, Habitat::Eelgrass, centre, bed, true);
}

fn append_bed(
    scene: &mut Scene,
    habitat: Habitat,
    centre: Vec2,
    bed: &mut impl FnMut(Vec2) -> f32,
    cove: bool,
) {
    let mut mesh = Chunk {
        verts: vec![],
        idx: vec![],
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    let mut rng = Sequence(0x35a6cb);
    // Fine enough to resolve the crest. A circular irregular margin exposes the
    // underlying terrain between the edge ridges.
    const STEPS: u32 = 160;
    let step = 2.0 * RADIUS / STEPS as f32;
    for z in 0..=STEPS {
        for x in 0..=STEPS {
            let p = Vec2::new(x as f32, z as f32) * step - RADIUS;
            let (dy, crest) = relief(p, habitat);
            let world = centre + p;
            let base = bed(world);
            let grad = Vec2::new(
                bed(world + Vec2::X * step) - base,
                bed(world + Vec2::Y * step) - base,
            ) / step
                + Vec2::new(
                    relief(p + Vec2::X * 0.005, habitat).0 - dy,
                    relief(p + Vec2::Y * 0.005, habitat).0 - dy,
                ) / 0.005;
            let sand = if habitat == Habitat::Sand {
                Vec3::new(172.0, 154.0, 123.0)
            } else {
                if cove {
                    Vec3::new(100.0, 94.0, 78.0)
                } else {
                    Vec3::new(118.0, 107.0, 78.0)
                }
            };
            let mottling = (p.x * 1.7 + (p.y * 1.3).sin()).sin() * (p.y * 2.4).cos();
            add_vertex(
                &mut mesh,
                point(world, base + dy),
                point(-grad, 1.0).normalize(),
                sand * (0.83 + 0.09 * crest + 0.05 * mottling + 0.06 * rng.unit()),
                0.0,
            );
            if x > 0 && z > 0 && p.length() < RADIUS {
                let j = (z - 1) * (STEPS + 1) + x - 1;
                mesh.idx
                    .extend([j, j + STEPS + 1, j + 1, j + 1, j + STEPS + 1, j + STEPS + 2]);
            }
        }
    }
    if cove {
        // Packed normal.w identifies only the sand; shells, stones and blades
        // retain their own materials where they emerge through the sediment.
        for v in &mut mesh.verts {
            v.normal |= 127 << 24;
        }
    }
    let placements = if habitat == Habitat::Eelgrass {
        1320
    } else {
        780
    };
    let placements = if cove { 260 } else { placements };
    for i in 0..placements {
        // A denser stand inside the meadow gives way to isolated shoots.
        let extent = if i >= 780 { 2.1 } else { RADIUS };
        let p = Vec2::new(rng.unit() - 0.5, rng.unit() - 0.5) * (2.0 * extent);
        if p.length() > RADIUS - 0.35 {
            continue;
        }
        let world = centre + p;
        let y = bed(world) + relief(p, habitat).0.max(0.0);
        let root = point(world, y);
        if rng.unit() < 0.24 {
            shell(&mut mesh, root, &mut rng);
        }
        let meadow = (p.x * 1.6 + 0.8 * (p.y * 1.3).sin()).sin() + (p.y * 1.8 + p.x * 0.42).cos();
        if habitat == Habitat::Eelgrass
            && meadow > (if cove { 0.55 } else { -0.65 })
            && y < eo_water::SEA_LEVEL_Y as f32 - 0.65
        {
            for _ in 0..4 {
                blade(&mut mesh, root - Vec3::Y * 0.012, &mut rng);
            }
        }
    }
    push_chunks(
        scene,
        vec![mesh],
        ObjectKind::Seabed,
        LOD_ALWAYS,
        NO_GROUP,
        MATERIAL_VERTEX_COLOUR,
    );
}

/// An intertidal band of wave-rounded cobbles partly embedded in the drawn beach.
/// The same geometry is sampled by the refracting surface and the submerged view.
pub fn cove_shore(scene: &mut Scene, shore: Vec2, bed: &mut impl FnMut(Vec2) -> f32) {
    let mut mesh = Chunk {
        verts: vec![],
        idx: vec![],
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    let mut rng = Sequence(0x98734);
    for i in 0..1450 {
        let reach = if i < 1100 { 34.0 } else { 146.0 };
        let p = shore + Vec2::new((rng.unit() - 0.5) * 31.0, 3.5 - rng.unit() * reach);
        let radius = 0.024 + 0.34 * rng.unit().powi(5);
        let axis = Vec2::from_angle(rng.unit() * std::f32::consts::TAU);
        let narrow = 0.56 + 0.38 * rng.unit();
        let high = radius * (0.38 + 0.55 * rng.unit());
        let root = bed(p) - high * 0.21;
        let tone = match (rng.unit() * 4.0) as u8 {
            0 => Vec3::new(103.0, 97.0, 83.0),
            1 => Vec3::new(110.0, 84.0, 66.0),
            2 => Vec3::new(74.0, 82.0, 74.0),
            _ => Vec3::new(87.0, 86.0, 83.0),
        } * (0.82 + 0.35 * rng.unit());
        let first = mesh.verts.len() as u32;
        let phases: [f32; 8] = std::array::from_fn(|_| 0.85 + rng.unit() * 0.28);
        let lean = (rng.unit() - 0.5) * 0.2;
        for (ring, (spread, y)) in [(0.93, 0.0), (1.0, 0.34), (0.76, 0.76), (0.24, 0.97)]
            .into_iter()
            .enumerate()
        {
            for (k, shape) in phases.iter().enumerate() {
                let v = Vec2::from_angle(k as f32 * std::f32::consts::TAU / 8.0);
                let offset = axis.rotate(Vec2::new(v.x * spread + lean * y, v.y * narrow * spread))
                    * radius
                    * *shape;
                let normal =
                    point(axis.rotate(Vec2::new(v.x, v.y / narrow)) * (1.0 - y), y).normalize();
                add_vertex(
                    &mut mesh,
                    point(p + offset, root + high * y * (0.94 + 0.06 * *shape)),
                    normal,
                    tone * (0.89 + 0.11 * y),
                    0.0,
                );
                if ring > 0 {
                    let a = first + ((ring - 1) * 8 + k) as u32;
                    let b = first + ((ring - 1) * 8 + (k + 1) % 8) as u32;
                    mesh.idx.extend([a, a + 8, b, b, a + 8, b + 8]);
                }
            }
        }
        for k in 1..7 {
            mesh.idx
                .extend([first + 24, first + 25 + k, first + 24 + k]);
        }
    }
    // Small drifts of shell fragments in the silt, growing sparser offshore.
    for _ in 0..900 {
        let drift = shore + Vec2::new((rng.unit() - 0.5) * 32.0, -rng.unit() * 146.0);
        for _ in 0..(1 + (rng.unit() * 6.0) as u8) {
            let p = drift + Vec2::new(rng.unit() - 0.5, rng.unit() - 0.5) * 0.85;
            let floor = bed(p);
            shell(&mut mesh, point(p, floor + 0.007), &mut rng);
        }
    }
    push_chunks(
        scene,
        vec![mesh],
        ObjectKind::Seabed,
        LOD_ALWAYS,
        NO_GROUP,
        MATERIAL_VERTEX_COLOUR,
    );
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_beds_stay_rooted_and_inside_their_animated_bounds() {
        for habitat in [Habitat::Eelgrass, Habitat::Sand] {
            let mut scene = Scene {
                vertices: vec![],
                indices: vec![],
                objects: vec![],
                stats: Default::default(),
                world_min: [0.0; 3],
                world_max: [0.0; 3],
            };
            let bed = |p: Vec2| -2.1 + 0.018 * p.x - 0.009 * p.y;
            append(&mut scene, habitat, Vec2::ZERO, &mut |p| bed(p));
            assert_eq!(scene.objects.len(), 1);
            let o = scene.objects[0];
            assert!(
                scene
                    .indices
                    .iter()
                    .all(|&i| (i as usize) < scene.vertices.len())
            );
            let mut flexible = 0;
            for v in scene.vertices {
                let p = Vec3::from(v.pos);
                let bend = v.color.to_le_bytes()[3] as f32 / 255.0;
                flexible += usize::from(bend > 0.0);
                // The vertex format rounds bend to a byte.
                let reach = Vec3::new(SWAY_M, 0.0, SWAY_M) * bend;
                assert!((p - reach).cmpge(Vec3::from(o.min) - 0.0003).all());
                assert!((p + reach).cmple(Vec3::from(o.max) + 0.0003).all());
                let above_ground = p.y - bed(Vec2::new(p.x, p.z));
                assert!(
                    (-0.04..0.46).contains(&above_ground),
                    "detached vertex at {p:?}"
                );
            }
            assert_eq!(flexible != 0, habitat == Habitat::Eelgrass);
        }
    }
}
