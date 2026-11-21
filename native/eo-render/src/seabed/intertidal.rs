//! A small section of the rocky west end of Aquatic Park beach. The park's
//! intertidal survey describes attached algae and barnacles along this shore.
//! Individual rocks and colonies are deterministic habitat illustrations.

use super::{Chunk, Sequence, add_vertex, point};
use glam::{Vec2, Vec3};

const SIDES: u32 = 18;
const RINGS: u32 = 6;

struct Rock {
    centre: Vec3,
    width: f32,
    height: f32,
    axis: Vec2,
    phases: [f32; 3],
}

impl Rock {
    fn surface(&self, r: f32, a: f32) -> Vec3 {
        let u = Vec2::from_angle(a);
        let [p, q, s] = self.phases;
        let rim = 0.90
            + 0.075 * (3.0 * a + p).cos()
            + 0.06 * (5.0 * a + q).sin()
            + 0.035 * (7.0 * a + s).cos();
        let local = u * r * rim;
        let xz = self.axis.rotate(Vec2::new(local.x, local.y * 0.73)) * self.width;
        // Broad upper faces broken by joints, tapering into buried flanks.
        let fault = (local.x * 6.3 + local.y * 2.2 + p).sin().abs();
        let top = (1.0 - r.powf(2.7)).max(0.0).sqrt()
            * (0.88 + 0.09 * (3.4 * local.y + q).sin() + 0.11 * local.x
                - 0.075 * (1.0 - 7.0 * fault).max(0.0));
        self.centre + point(xz, self.height * top)
    }

    fn build(&self, mesh: &mut Chunk, tone: Vec3) -> u32 {
        let first = mesh.verts.len() as u32;
        for j in 0..=RINGS {
            let r = j as f32 / RINGS as f32;
            for k in 0..SIDES {
                let a = std::f32::consts::TAU * k as f32 / SIDES as f32;
                let p = self.surface(r, a);
                let around =
                    self.surface(r.max(0.005), a + 0.015) - self.surface(r.max(0.005), a - 0.015);
                let across =
                    self.surface((r + 0.015).min(1.0), a) - self.surface((r - 0.015).max(0.0), a);
                let n = around.cross(across).normalize_or(Vec3::Y);
                let band = (p.y * 20.0 + p.x * 1.9 + self.phases[1]).sin();
                let face_tone = tone * (0.86 + 0.09 * band + 0.08 * n.y);
                add_vertex(mesh, p, if j == 0 { Vec3::Y } else { n }, face_tone, 0.0);
                mesh.verts.last_mut().unwrap().normal |= 0xc0 << 24; // mineral surface
                if j > 0 {
                    let inner = first + (j - 1) * SIDES + k;
                    let next = first + (j - 1) * SIDES + (k + 1) % SIDES;
                    if j > 1 {
                        mesh.idx.extend([inner, next, inner + SIDES]);
                    }
                    mesh.idx.extend([next, next + SIDES, inner + SIDES]);
                }
            }
        }
        first
    }

    // Barycentric point on the rendered triangles. Organisms stay attached
    // on angular faces as well as on the gently sloped tops.
    fn attachment(&self, mesh: &Chunk, first: u32, r: f32, turns: f32) -> (Vec3, Vec3) {
        let ring = r.clamp(0.01, 0.99) * RINGS as f32;
        let sector = turns.rem_euclid(1.0) * SIDES as f32;
        let row = first + ring.floor() as u32 * SIDES;
        let a = row + sector.floor() as u32;
        let b = row + (sector.floor() as u32 + 1) % SIDES;
        let (x, y) = (sector.fract(), ring.fract());
        let v = |i: u32| Vec3::from(mesh.verts[i as usize].pos);
        let (p, q, t, w) = if ring < 1.0 {
            (
                v(a),
                v(b + SIDES),
                v(a + SIDES),
                Vec2::new(x * y, (1.0 - x) * y),
            )
        } else if x + y < 1.0 {
            (v(a), v(b), v(a + SIDES), Vec2::new(x, y))
        } else {
            (
                v(b + SIDES),
                v(a + SIDES),
                v(b),
                Vec2::new(1.0 - x, 1.0 - y),
            )
        };
        (
            p + (q - p) * w.x + (t - p) * w.y,
            (q - p).cross(t - p).normalize_or(Vec3::Y),
        )
    }
}

fn barnacle(mesh: &mut Chunk, root: Vec3, n: Vec3, size: f32, rng: &mut Sequence) {
    let (u, v) = n.any_orthonormal_pair();
    let first = mesh.verts.len() as u32;
    let phase = rng.unit() * std::f32::consts::TAU;
    let height = size * (0.5 + 0.55 * rng.unit());
    let shell = Vec3::new(174.0, 164.0, 139.0) * (0.70 + rng.unit() * 0.35);
    // Cemented foot, fluted wall, lip and recessed, dark opercular opening.
    for (j, (width, lift)) in [
        (1.0, -0.13),
        (0.88, 0.22),
        (0.45, 1.0),
        (0.33, 0.80),
        (0.08, 0.56),
    ]
    .into_iter()
    .enumerate()
    {
        for k in 0..10u32 {
            let angle = k as f32 * std::f32::consts::TAU / 10.0 + phase;
            let radial = u * angle.cos() + v * angle.sin();
            let rib = (k % 2) as f32;
            let p = root
                + radial * size * width * (1.0 - 0.13 * rib)
                + n * (lift * height * (1.0 - 0.12 * rib));
            let normal = if j < 3 {
                (radial * 0.72 + n * 0.65).normalize()
            } else {
                n
            };
            let color = if j < 3 {
                shell * (0.85 + 0.15 * rib)
            } else {
                shell * (if j == 3 { 0.46 } else { 0.25 })
            };
            add_vertex(mesh, p, normal, color, 0.0);
            if j > 0 {
                let a = first + (j as u32 - 1) * 10 + k;
                let b = first + (j as u32 - 1) * 10 + (k + 1) % 10;
                mesh.idx.extend([a, b, a + 10, b, b + 10, a + 10]);
            }
        }
    }
}

fn frond(
    mesh: &mut Chunk,
    root: Vec3,
    axis: Vec3,
    length: f32,
    width: f32,
    tone: Vec3,
    rng: &mut Sequence,
) {
    let (u, v) = axis.any_orthonormal_pair();
    let blade_angle = rng.unit() * std::f32::consts::TAU;
    let side = u * blade_angle.cos() + v * blade_angle.sin();
    let curl = rng.unit() * std::f32::consts::TAU;
    let first = mesh.verts.len() as u32;
    for k in 0..=8u32 {
        let t = k as f32 / 8.0;
        let lean = Vec3::new(0.70, -0.18, -0.23) * t;
        let sway = side * (t * (t * 5.8 + curl).sin() * 0.20);
        let p = root + (axis + lean + sway) * (t * length);
        let twist = (curl + 4.0 * t).sin() * 0.55;
        let across = (side + axis.cross(side) * twist).normalize();
        let normal = across.cross(axis + lean * 2.0).normalize_or(Vec3::Y);
        let w = width
            * (0.07 + (std::f32::consts::PI * t).sin().max(0.0).powf(0.7))
            * (1.0 + 0.18 * (t * 19.0 + curl).sin());
        for j in [-1.0, 0.0, 1.0] {
            let ridge = if j == 0.0 { 1.0 } else { 0.0 };
            add_vertex(
                mesh,
                p + across * (j * w) + normal * (ridge * w * 0.18),
                (normal + across * (j * 0.28)).normalize(),
                tone * (0.68 + 0.24 * t + ridge * 0.09),
                0.45 * t * t,
            );
        }
        if k > 0 {
            let a = first + (k - 1) * 3;
            for b in [a, a + 1] {
                mesh.idx.extend([b, b + 1, b + 3, b + 1, b + 4, b + 3]);
            }
        }
    }
}

pub(super) fn append(mesh: &mut Chunk, centre: Vec2, bed: &mut impl FnMut(Vec2) -> f32) {
    let mut rng = Sequence(0x465241);
    let along = Vec2::new(0.94, 0.34);
    let mut stones: Vec<(Vec2, f32)> = Vec::new();
    for i in 0..230 {
        let along_m = (rng.unit() - 0.5) * 28.0;
        let cross_m = (rng.unit() - 0.5) * (3.0 + 1.6 * (along_m * 0.67).cos());
        let p = if i == 0 {
            centre
        } else {
            centre + along * along_m + along.perp() * cross_m
        };
        let floor = bed(p);
        let water_depth = eo_water::SEA_LEVEL_Y as f32 - floor;
        if !(-0.3..2.0).contains(&water_depth) {
            continue;
        }
        let width = if i == 0 {
            0.78
        } else {
            0.07 + 0.86 * rng.unit().powf(2.7)
        };
        if stones
            .iter()
            .any(|&(other, radius)| p.distance_squared(other) < (0.82 * (width + radius)).powi(2))
        {
            continue;
        }
        stones.push((p, width));
        let height = width * (0.50 + 0.45 * rng.unit());
        let rock = Rock {
            centre: point(p, floor - 0.14 * height),
            width,
            height,
            axis: Vec2::from_angle(rng.unit() * std::f32::consts::TAU),
            phases: std::array::from_fn(|_| rng.unit() * std::f32::consts::TAU),
        };
        let tone = Vec3::new(83.0, 88.0, 78.0).lerp(Vec3::new(146.0, 117.0, 91.0), rng.unit())
            * (0.87 + rng.unit() * 0.23);
        let first = rock.build(mesh, tone);
        if width < 0.24 {
            continue;
        }
        let colonies = 1 + (width * 4.0) as usize;
        for _ in 0..colonies {
            let turns = rng.unit();
            let r = 0.30 + 0.55 * rng.unit();
            let count = 6 + (width * 32.0 * rng.unit()) as usize;
            for _ in 0..count {
                let (root, n) = rock.attachment(
                    mesh,
                    first,
                    r + (rng.unit() - 0.5) * 0.26,
                    turns + (rng.unit() - 0.5) * 0.13,
                );
                let size = 0.006 + 0.012 * rng.unit().powi(2);
                barnacle(mesh, root, n, size, &mut rng);
            }
            let (root, n) = rock.attachment(mesh, first, r, turns + 0.14);
            if root.y < eo_water::SEA_LEVEL_Y as f32 - 0.11 && n.y > 0.1 && rng.unit() < 0.76 {
                let shade =
                    Vec3::new(99.0, 94.0, 44.0).lerp(Vec3::new(123.0, 73.0, 47.0), rng.unit());
                let blades = 3 + (rng.unit() * 7.0) as usize;
                for _ in 0..blades {
                    let a = Vec2::from_angle(std::f32::consts::TAU * rng.unit());
                    let dir = (0.40 * n + point(a, 0.35 + 0.7 * rng.unit())).normalize();
                    let length = 0.07 + 0.26 * rng.unit();
                    let width = length * (0.045 + 0.055 * rng.unit());
                    frond(mesh, root, dir, length, width, shade, &mut rng);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;
    fn empty_scene() -> Scene {
        Scene {
            vertices: vec![],
            indices: vec![],
            objects: vec![],
            stats: Default::default(),
            world_min: [0.0; 3],
            world_max: [0.0; 3],
        }
    }
    #[test]
    fn black_point_camera_and_contact_match_when_the_habitat_is_hidden() {
        let mut shown = empty_scene();
        let mut hidden = empty_scene();
        let mut bed = |p: Vec2| eo_water::SEA_LEVEL_Y as f32 - 1.16 + p.y * 0.018;
        let a = super::super::intertidal_site(&mut shown, Vec2::ZERO, &mut bed, true, 0).unwrap();
        let b = super::super::intertidal_site(&mut hidden, Vec2::ZERO, &mut bed, false, 0).unwrap();
        assert_eq!(a, b);
        assert!(a.0.y < eo_water::SEA_LEVEL_Y as f32 - 0.18);
        assert_eq!(shown.objects.len(), 1);
        assert!(hidden.objects.is_empty());
        assert!(
            shown
                .indices
                .iter()
                .all(|&i| (i as usize) < shown.vertices.len())
        );
        assert!(shown.vertices.iter().any(|v| v.normal >> 24 == 0xc0));
        assert!(shown.vertices.iter().any(|v| v.color >> 24 > 0));
    }
}
