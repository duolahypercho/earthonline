//! A distance mesh for small authored details in the optional furnished house.
use eo_render::scene::{LOD_AUTHORED_DETAIL, Scene, Vertex};
use glam::Vec3;
use std::collections::{HashMap, HashSet};

/// At the 120 m switch an eight-centimetre simplification is below one pixel
/// at 1080p and the play camera's vertical field of view.
const CELL_M: f32 = 0.08;

fn coarse(vertices: &[Vertex], indices: &[u32], base: usize) -> Vec<u32> {
    let cell = |v: &Vertex| (Vec3::from(v.pos) / CELL_M).floor().as_ivec3().to_array();
    let mut representatives = HashMap::new();
    for &index in indices {
        let v = &vertices[base + index as usize];
        let key = cell(v);
        let center = (Vec3::from_array(key.map(|n| n as f32)) + Vec3::splat(0.5)) * CELL_M;
        let d = Vec3::from(v.pos).distance_squared(center);
        // Keep representatives separate where the surface normal changes across a crease.
        let key = (key, v.normal & 0x00e0e0e0);
        let best = representatives.entry(key).or_insert((d, index));
        if d < best.0 {
            *best = (d, index);
        }
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(indices.len() / 2);
    for tri in indices.chunks_exact(3) {
        let original = [tri[0], tri[1], tri[2]];
        let cells = original.map(|i| cell(&vertices[base + i as usize]));
        if cells[0] == cells[1] || cells[1] == cells[2] || cells[2] == cells[0] {
            continue;
        }
        let mapped = original.map(|i| {
            let v = &vertices[base + i as usize];
            representatives[&(cell(v), v.normal & 0x00e0e0e0)].1
        });
        let min = if mapped[0] <= mapped[1] && mapped[0] <= mapped[2] {
            0
        } else if mapped[1] <= mapped[2] {
            1
        } else {
            2
        };
        let key = [mapped[min], mapped[(min + 1) % 3], mapped[(min + 2) % 3]];
        if seen.insert(key) {
            out.extend_from_slice(&mapped);
        }
    }
    out
}

pub fn install(scene: &mut Scene, first_object: usize) {
    for o in &mut scene.objects[first_object..] {
        if o.index_count < 36 {
            continue;
        }
        let begin = o.first_index as usize;
        let reduced = coarse(
            &scene.vertices,
            &scene.indices[begin..begin + o.index_count as usize],
            o.base_vertex as usize,
        );
        if reduced.len() < o.index_count as usize {
            o.lod_role = LOD_AUTHORED_DETAIL;
            o.far_first_index = scene.indices.len() as u32;
            o.far_index_count = reduced.len() as u32;
            scene.indices.extend(reduced);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coarse_retains_wall_winding_and_drops_subpixel_trim() {
        let v = [
            [0., 0., 0.],
            [2., 0., 0.],
            [2., 1., 0.],
            [0., 1., 0.],
            [0.01, 0.01, 0.01],
            [0.02, 0.01, 0.01],
            [0.01, 0.02, 0.01],
        ]
        .map(|pos| Vertex {
            pos,
            normal: 0x7f0000,
            color: u32::MAX,
        });
        let result = coarse(&v, &[0, 1, 2, 2, 3, 0, 4, 5, 6, 1, 2, 0], 0);
        assert_eq!(result.len(), 6);
        for tri in result.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(v[i as usize].pos));
            assert!((b - a).cross(c - a).z > 1.8);
        }
    }
}
