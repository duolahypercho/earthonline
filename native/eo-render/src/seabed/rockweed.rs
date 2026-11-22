//! A flattened, forked blade attached to the rocky shore. The two tips share
//! their junction with the stipe so moving either surface cannot open a seam.

use super::{Chunk, add_vertex};
use glam::{Vec2, Vec3};

// The high normal byte stores the full tissue thickness in quarter-millimetres.
// Sand and the scanned rock have separate tags in the same seabed draw.
fn tissue_vertex(mesh: &mut Chunk, p: Vec3, n: Vec3, tone: Vec3, bend: f32, thickness: f32) {
    let tag = 32 + (thickness * 4000.0).round().clamp(0.0, 31.0) as u32;
    add_vertex(mesh, p, n, tone, bend);
    mesh.verts.last_mut().unwrap().normal |= tag << 24;
}

struct Blade {
    root: Vec3,
    axis: Vec3,
    face: Vec3,
    length: f32,
    width: f32,
    curl: f32,
    tone: Vec3,
}

impl Blade {
    // Both lobes follow the same water motion. They progressively open along
    // the blade's width beyond the fork and roll towards different faces.
    fn frame(&self, t: f32, lobe: f32) -> (Vec3, Vec3, Vec3) {
        let after = (t - 0.46).max(0.0);
        let flow = Vec3::new(0.42, -0.12, -0.16);
        let bow = 0.12 * (self.curl + t * 3.5).sin();
        let drift = self.face * lobe * (0.33 * after + 0.19 * after * after);
        let centre = self.root
            + self.length
                * (self.axis * t + (flow + self.axis.cross(self.face) * bow) * t * t + drift);
        let tangent = (self.axis
            + flow * (2.0 * t)
            + self.axis.cross(self.face)
                * (2.0 * t * bow + 0.42 * t * t * (self.curl + t * 3.5).cos())
            + self.face * lobe * (0.33 + 0.38 * after))
            .normalize_or(self.axis);
        let twist = 0.36 * (self.curl + 3.8 * t).sin() + after * lobe * 0.85;
        let width_axis = self.face * twist.cos() + self.axis.cross(self.face) * twist.sin();
        let normal = width_axis
            .cross(tangent)
            .normalize_or(self.axis.cross(self.face));
        (centre, tangent.cross(normal).normalize(), normal)
    }

    fn ring<const N: usize>(
        &self,
        mesh: &mut Chunk,
        t: f32,
        lobe: f32,
        half_width: f32,
        thickness: f32,
        section: &[Vec2; N],
    ) -> [u32; N] {
        let (centre, across, face) = self.frame(t, lobe);
        std::array::from_fn(|i| {
            let p = section[i];
            let surface = centre + across * (p.x * half_width) + face * (p.y * thickness);
            let normal = (across * (p.x * thickness / half_width) + face * p.y).normalize();
            let colour = self.tone * (0.74 + 0.24 * t + 0.16 * p.y.abs());
            let index = mesh.verts.len() as u32;
            tissue_vertex(
                mesh,
                surface,
                normal,
                colour,
                0.45 * t * t,
                2.0 * thickness * p.y.abs(),
            );
            index
        })
    }
}

fn join<const N: usize>(mesh: &mut Chunk, a: [u32; N], b: [u32; N]) {
    for i in 0..N {
        let next = (i + 1) % N;
        mesh.idx
            .extend([a[i], a[next], b[i], a[next], b[next], b[i]]);
    }
}

fn cap(mesh: &mut Chunk, rim: &[u32], tip: u32, upper: bool) {
    for i in 0..rim.len() {
        let (a, b) = (rim[i], rim[(i + 1) % rim.len()]);
        mesh.idx
            .extend(if upper { [b, tip, a] } else { [a, tip, b] });
    }
}

pub(super) fn append(
    mesh: &mut Chunk,
    root: Vec3,
    axis: Vec3,
    length: f32,
    width: f32,
    tone: Vec3,
    angle: f32,
    curl: f32,
) {
    let (u, v) = axis.any_orthonormal_pair();
    let blade = Blade {
        root,
        axis,
        face: u * angle.cos() + v * angle.sin(),
        length: length.min(0.25),
        width: width.clamp(0.004, 0.0125),
        curl,
        tone,
    };
    let midrib = [
        Vec2::new(-1.0, 0.0),
        Vec2::new(-0.42, 0.35),
        Vec2::Y,
        Vec2::new(0.42, 0.35),
        Vec2::X,
        Vec2::new(0.42, -0.35),
        Vec2::NEG_Y,
        Vec2::new(-0.42, -0.35),
    ];
    let w = blade.width;
    let heel = mesh.verts.len() as u32;
    tissue_vertex(mesh, root, -axis, tone * 0.74, 0.0, w * 0.34);
    let mut rim = blade.ring(mesh, 0.0, 0.0, w * 0.22, w * 0.17, &midrib);
    cap(mesh, &rim, heel, false);
    for (t, width) in [(0.10, 0.30), (0.24, 0.63), (0.36, 1.0), (0.46, 1.12)] {
        let next = blade.ring(mesh, t, 0.0, w * width, w * 0.15, &midrib);
        join(mesh, rim, next);
        rim = next;
    }
    // Split one closed perimeter into two. The seam down the middle is
    // shared by both lobes and becomes the crotch of the fork.
    let seam = mesh.verts.len() as u32;
    let (centre, _, face) = blade.frame(0.46, 0.0);
    tissue_vertex(mesh, centre, face, tone, 0.45 * 0.46 * 0.46, w * 0.30);
    let lobes = [
        (-1.0, [rim[0], rim[1], rim[2], seam, rim[6], rim[7]]),
        (1.0, [seam, rim[2], rim[3], rim[4], rim[5], rim[6]]),
    ];
    let section = [
        Vec2::NEG_X,
        Vec2::new(-0.28, 1.0),
        Vec2::new(0.28, 1.0),
        Vec2::X,
        Vec2::new(0.28, -1.0),
        Vec2::new(-0.28, -1.0),
    ];
    for (side, mut edge) in lobes {
        let end = 0.96 + side * 0.04 * curl.cos();
        for (s, radius, depth) in [
            (0.15, 0.63, 0.11),
            (0.43, 0.66, 0.16),
            (0.70, 0.75, 0.29),
            (0.88, 0.64, 0.32),
            (0.97, 0.32, 0.19),
        ] {
            let t = 0.46 + s * (end - 0.46);
            let next = blade.ring(mesh, t, side, w * radius, w * depth, &section);
            join(mesh, edge, next);
            edge = next;
        }
        let tip = mesh.verts.len() as u32;
        tissue_vertex(
            mesh,
            blade.frame(end, side).0,
            axis,
            tone * 1.08,
            0.45 * end * end,
            w * 0.25,
        );
        cap(mesh, &edge, tip, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn both_lobes_and_the_fork_form_one_closed_surface() {
        let mut mesh = Chunk {
            verts: vec![],
            idx: vec![],
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        let root = Vec3::new(-668.0, -0.7, -4000.0);
        append(
            &mut mesh,
            root,
            Vec3::Y,
            0.23,
            0.012,
            Vec3::splat(100.0),
            0.85,
            0.37,
        );
        let mut edges = BTreeMap::new();
        for face in mesh.idx.chunks_exact(3) {
            let vertices =
                [face[0], face[1], face[2]].map(|i| Vec3::from(mesh.verts[i as usize].pos));
            assert!(
                (vertices[1] - vertices[0])
                    .cross(vertices[2] - vertices[0])
                    .length_squared()
                    > 1.0e-16
            );
            for (a, b) in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
                let pair = edges.entry((a.min(b), a.max(b))).or_insert([0, 0]);
                pair[usize::from(a > b)] += 1;
            }
        }
        assert!(
            edges.values().all(|pair| *pair == [1, 1]),
            "open or overlapping seam"
        );
        assert_eq!(mesh.verts.len() + mesh.idx.len() / 3, edges.len() + 2);
        assert_eq!(mesh.verts[0].pos, root.to_array());
        assert_eq!(mesh.verts[0].color >> 24, 0);
        assert!(
            mesh.verts
                .iter()
                .all(|v| (32..=63).contains(&(v.normal >> 24)))
        );
        assert!(mesh.verts.iter().any(|v| v.normal >> 24 == 32));
        assert!(mesh.verts.iter().any(|v| v.normal >> 24 > 48));
    }
}
