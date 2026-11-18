//! stage 2, the district (skyline) level. Districts are 8 x 8
//! neighbourhood cells (2,048 m) on the same grid, made from what the cell
//! bake learnt of each building (`BuildingSummary`: its facade's mean colour
//! and window share, its roof colour, foot and wall top) and its footprint,
//! simplified (Douglas-Peucker, `SIMPLIFY_M`): one merged prism mesh per
//! district, flat colours (the facade mean on the walls, the roof colour on
//! top: the shader tells them apart by the face normal), the window share
//! lighting the walls by night. Shared ring vertices, no per-face normal and
//! no atlas: small enough that every district stays resident.
//!
//! Cached beside the cells: `cache/hlod/<key>/district_<dx>_<dz>.eohd`
//! (payload SHA-256 at the end, a damaged file is rebuilt), made from the
//! cells' summary files (`<cx>_<cz>.eohs`), baking any cell not yet baked.

use crate::hlod_bake::{self, BakeContext, BuildingSummary, HlodSet, HlodVertex};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Cells per district side.
pub const DISTRICT_CELLS: i32 = 8;
/// Footprint simplification tolerance, metres (a pixel at 1.4 km is 1.3 m).
pub const SIMPLIFY_M: f64 = 1.5;
pub const MAGIC: &[u8; 4] = b"EOHD";
pub const VERSION: u32 = 2; // 2: the window rhythm in the normal word, the opaque wall colour, the tower flag
/// District colour flag (the vertex colour's alpha byte).
pub const VF_DISTRICT: u32 = 4;

/// the window rhythm packed in a district vertex's normal word (the
/// district has no per-vertex normal): bay period (0.06 m steps), storey period
/// (0.03 m steps), window width and height shares (/255), each a byte; 0: unknown.
pub fn pack_rhythm(r: [f32; 5]) -> u32 {
    if !(r[0] > 0.0 && r[1] > 0.0) {
        return 0;
    }
    let q = |v: f32, step: f32| ((v / step).round() as u32).clamp(1, 255);
    let s = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32).clamp(1, 255);
    q(r[0], 0.06) | (q(r[1], 0.03) << 8) | (s(r[2]) << 16) | (s(r[3]) << 24)
}

/// `pack_rhythm` read back (hlod_draw.wgsl `hl_rhythm` reads it the same way).
pub fn unpack_rhythm(w: u32) -> [f32; 4] {
    [(w & 255) as f32 * 0.06, ((w >> 8) & 255) as f32 * 0.03, ((w >> 16) & 255) as f32 / 255.0, (w >> 24) as f32 / 255.0]
}

/// a box-filtered pulse train, the mirror of hlod_draw.wgsl `hl_pulse`:
/// the share of `[x - w / 2, x + w / 2]` covered by pulses of `duty` x
/// `period` starting at each multiple of `period` (x in periods, w in periods).
pub fn pulse_filtered(x: f32, duty: f32, w: f32) -> f32 {
    let integral = |t: f32| t.floor() * duty + (t - t.floor()).min(duty);
    if w < 1e-4 {
        return if x - x.floor() < duty { 1.0 } else { 0.0 };
    }
    (integral(x + 0.5 * w) - integral(x - 0.5 * w)) / w
}

pub fn district_key(cell: (i32, i32)) -> (i32, i32) {
    (cell.0.div_euclid(DISTRICT_CELLS), cell.1.div_euclid(DISTRICT_CELLS))
}

pub fn district_path(dir: &Path, key: &str, d: (i32, i32)) -> PathBuf {
    dir.join("hlod").join(key).join(format!("district_{}_{}.eohd", d.0, d.1))
}

#[derive(Clone, Debug)]
pub struct District {
    pub key: (i32, i32),
    /// Its cells (indices into `HlodSet::cells`).
    pub cells: Vec<u32>,
    /// Its first entry in the GPU cell table (after the cells): one per
    /// cell (its chunk, `chunk_by_cell`), then one for anything else.
    pub entry: u32,
    pub min: [f32; 2],
    pub max: [f32; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DistrictMesh {
    pub entry: u32,
    pub key: (i32, i32),
    pub vertices: Vec<HlodVertex>,
    pub indices: Vec<u32>,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub buildings: u32,
}

/// The districts of a set, in a stable order; entries follow the cells.
pub fn districts_of(set: &HlodSet) -> Vec<District> {
    let mut by: BTreeMap<(i32, i32), Vec<u32>> = BTreeMap::new();
    for (i, c) in set.cells.iter().enumerate() {
        by.entry(district_key(c.key)).or_default().push(i as u32);
    }
    let mut next = set.cells.len() as u32;
    by.into_iter()
        .map(|(key, cells)| {
            let (mut mn, mut mx) = ([f32::MAX; 2], [f32::MIN; 2]);
            for &c in &cells {
                let cell = &set.cells[c as usize];
                mn = [mn[0].min(cell.min[0]), mn[1].min(cell.min[1])];
                mx = [mx[0].max(cell.max[0]), mx[1].max(cell.max[1])];
            }
            let entry = next;
            next += cells.len() as u32 + 1;
            District { key, cells, entry, min: mn, max: mx }
        })
        .collect()
}

fn perp_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    if l2 < 1e-12 {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    ((p[0] - a[0]) * dz - (p[1] - a[1]) * dx).abs() / l2.sqrt()
}

fn dp(points: &[[f64; 2]], tol: f64, out: &mut Vec<[f64; 2]>) {
    if points.len() < 3 {
        out.extend_from_slice(&points[..points.len().saturating_sub(1)]);
        return;
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let (mut best, mut at) = (0.0, 0);
    for (i, p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let d = perp_dist(*p, a, b);
        if d > best {
            best = d;
            at = i;
        }
    }
    if best > tol {
        dp(&points[..=at], tol, out);
        dp(&points[at..], tol, out);
    } else {
        out.push(a);
    }
}

/// Douglas-Peucker on a closed ring (open input, no closing duplicate), split
/// at the vertex farthest from the first; never fewer than three points (the
/// footprint's own bounding triangle stands in when it would collapse).
pub fn simplify_ring(ring: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let n = ring.len();
    if n <= 3 {
        return ring.to_vec();
    }
    let far = (1..n).max_by(|&i, &j| {
        let d = |k: usize| (ring[k][0] - ring[0][0]).powi(2) + (ring[k][1] - ring[0][1]).powi(2);
        d(i).total_cmp(&d(j))
    });
    let far = far.unwrap_or(n / 2);
    let mut out = vec![];
    let first: Vec<[f64; 2]> = ring[..=far].to_vec();
    let mut second: Vec<[f64; 2]> = ring[far..].to_vec();
    second.push(ring[0]);
    dp(&first, tol, &mut out);
    dp(&second, tol, &mut out);
    if out.len() >= 3 { out } else { ring.to_vec() }
}

fn rgb565(c: [f32; 3]) -> u32 {
    let s = |v: f32, bits: u32| ((hlod_bake::linear_to_srgb_byte(v) as f32 / 255.0) * ((1u32 << bits) - 1) as f32).round() as u32;
    s(c[0], 5) | (s(c[1], 6) << 5) | (s(c[2], 5) << 11)
}

/// One district's mesh from its buildings' summaries.
/// a district building's vertex words: the colour (the opaque wall with
/// `glass_body`, else the facade mean; the district and tower flags), the uv
/// word (roof rgb565, window share, storey phase) and the normal word (the rhythm).
pub fn vertex_words(s: &BuildingSummary, glass_body: bool) -> (u32, u32, u32) {
    let wall = if glass_body { s.wall } else { s.facade };
    let tower = if s.tower > 0.5 { hlod_bake::VF_TOWER } else { 0 };
    let colour = hlod_bake::linear_to_srgb_byte(wall[0]) as u32
        | ((hlod_bake::linear_to_srgb_byte(wall[1]) as u32) << 8)
        | ((hlod_bake::linear_to_srgb_byte(wall[2]) as u32) << 16)
        | ((VF_DISTRICT | tower) << 24);
    let phase = ((s.rhythm[4].rem_euclid(1.0) * 255.0).round() as u32).min(255);
    let uv = rgb565(s.roof) | (((s.glass.clamp(0.0, 1.0) * 255.0).round() as u32) << 16) | (phase << 24);
    (colour, uv, pack_rhythm(s.rhythm))
}

/// with `glass_body` (the bake's) the walls carry the opaque wall colour,
/// the window share, the rhythm and the tower flag; the far shader mixes in the
/// glass. Without it, the facade's mean colour as before.
pub fn build_district(city: &eo_data::City, entry: u32, key: (i32, i32), summaries: &[BuildingSummary], glass_body: bool) -> DistrictMesh {
    let mut vertices = vec![];
    let mut indices = vec![];
    let mut ear = earcut::Earcut::<f64>::new();
    let mut tri: Vec<u32> = vec![];
    let mut buildings = 0;
    for s in summaries {
        let Some(b) = city.buildings.get(s.building as usize) else { continue };
        let ring: Vec<[f64; 2]> = b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        if ring.len() < 3 || !(s.top > s.base) {
            continue;
        }
        let r = simplify_ring(&ring, SIMPLIFY_M);
        let n = r.len() as u32;
        let base = vertices.len() as u32;
        let (colour, uv, rhythm) = vertex_words(s, glass_body);
        for y in [s.base, s.top] {
            for p in &r {
                vertices.push(HlodVertex { pos: [p[0] as f32, y, p[1] as f32], normal: rhythm, uv, color: colour, group: s.group });
            }
        }
        for i in 0..n {
            let j = (i + 1) % n;
            let (b0, b1, t0, t1) = (base + i, base + j, base + n + i, base + n + j);
            indices.extend_from_slice(&[b0, b1, t1, b0, t1, t0]);
        }
        tri.clear();
        ear.earcut(r.iter().copied(), &[] as &[u32], &mut tri);
        indices.extend(tri.iter().map(|t| base + n + t));
        buildings += 1;
    }
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &vertices {
        for k in 0..3 {
            mn[k] = mn[k].min(v.pos[k]);
            mx[k] = mx[k].max(v.pos[k]);
        }
    }
    if vertices.is_empty() {
        mn = [0.0; 3];
        mx = [0.0; 3];
    }
    DistrictMesh { entry, key, vertices, indices, min: mn, max: mx, buildings }
}

impl DistrictMesh {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![];
        let u = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(MAGIC);
        u(&mut b, VERSION);
        u(&mut b, self.key.0 as u32);
        u(&mut b, self.key.1 as u32);
        u(&mut b, self.vertices.len() as u32);
        u(&mut b, self.indices.len() as u32);
        u(&mut b, self.buildings);
        for v in self.min.iter().chain(&self.max) {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(bytemuck::cast_slice(&self.vertices));
        b.extend_from_slice(bytemuck::cast_slice(&self.indices));
        let d = Sha256::digest(&b);
        b.extend_from_slice(&d);
        b
    }

    pub fn decode(bytes: &[u8], entry: u32) -> Result<Self, String> {
        if bytes.len() < 32 + 52 || &bytes[0..4] != MAGIC {
            return Err("not a district".into());
        }
        let (body, digest) = bytes.split_at(bytes.len() - 32);
        if Sha256::digest(body).as_slice() != digest {
            return Err("payload digest mismatch".into());
        }
        let w = |k: usize| u32::from_le_bytes(body[4 + 4 * k..8 + 4 * k].try_into().unwrap());
        if w(0) != VERSION {
            return Err("district version".into());
        }
        let key = (w(1) as i32, w(2) as i32);
        let (nv, ni, buildings) = (w(3) as usize, w(4) as usize, w(5));
        let f = |k: usize| f32::from_bits(w(6 + k));
        let min = [f(0), f(1), f(2)];
        let max = [f(3), f(4), f(5)];
        let at = 4 + 4 * 12;
        let vb = nv * std::mem::size_of::<HlodVertex>();
        if body.len() != at + vb + ni * 4 {
            return Err("truncated".into());
        }
        let vertices: Vec<HlodVertex> = bytemuck::pod_collect_to_vec(&body[at..at + vb]);
        let indices: Vec<u32> = bytemuck::pod_collect_to_vec(&body[at + vb..]);
        if indices.iter().any(|&i| i as usize >= nv) {
            return Err("index out of range".into());
        }
        Ok(Self { entry, key, vertices, indices, min, max, buildings })
    }
}

/// One cell's part of a district mesh: an index range and its box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistrictChunk {
    /// Its slot among the district's entries (`District::entry` + slot): the
    /// cell's position in `District::cells`, or `cells.len()` for the rest.
    pub slot: u32,
    /// The cell (an index into `HlodSet::cells`), when the chunk is one cell's.
    pub cell: Option<u32>,
    pub first_index: u32,
    pub index_count: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Reorders the mesh's triangles by their building's cell (`group_cell`, per
/// LOD group) and returns one chunk per cell, so the cull can drop the part of
/// a district that the resident cells around the camera draw instead.
pub fn chunk_by_cell(mesh: &mut DistrictMesh, group_cell: &[u32], cells: &[u32]) -> Vec<DistrictChunk> {
    let slot_of: std::collections::HashMap<u32, u32> = cells.iter().enumerate().map(|(j, &c)| (c, j as u32)).collect();
    let rest = cells.len() as u32;
    let mut buckets: Vec<Vec<u32>> = vec![vec![]; cells.len() + 1];
    for t in mesh.indices.chunks_exact(3) {
        let g = mesh.vertices.get(t[0] as usize).map(|v| v.group).unwrap_or(u32::MAX);
        let slot = group_cell.get(g as usize).and_then(|c| slot_of.get(c)).copied().unwrap_or(rest);
        buckets[slot as usize].extend_from_slice(t);
    }
    let mut indices = Vec::with_capacity(mesh.indices.len());
    let mut out = vec![];
    for (slot, b) in buckets.into_iter().enumerate() {
        if b.is_empty() {
            continue;
        }
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for &i in &b {
            let p = mesh.vertices[i as usize].pos;
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        let cell = cells.get(slot).copied();
        out.push(DistrictChunk { slot: slot as u32, cell, first_index: indices.len() as u32, index_count: b.len() as u32, min: mn, max: mx });
        indices.extend(b);
    }
    mesh.indices = indices;
    out
}

/// Read a district from the cache, or make it from its cells' summaries
/// (reading each cell's summary file, baking the cells not yet baked) and cache it.
pub fn load_or_build(ctx: &BakeContext, d: &District) -> DistrictMesh {
    if let Some(dir) = &ctx.cache_dir {
        if let Ok(b) = std::fs::read(district_path(dir, &ctx.key, d.key)) {
            if let Ok(m) = DistrictMesh::decode(&b, d.entry) {
                if m.key == d.key {
                    return m;
                }
            }
        }
    }
    let mut all = vec![];
    for &c in &d.cells {
        let s = hlod_bake::read_summaries(ctx, c).unwrap_or_else(|| hlod_bake::load_or_bake(ctx, c).0.summaries);
        all.extend(s);
    }
    let m = build_district(&ctx.city, d.entry, d.key, &all, ctx.opts.glass_body);
    if let Some(dir) = &ctx.cache_dir {
        let p = district_path(dir, &ctx.key, d.key);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, m.encode()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_keeps_corners_and_drops_collinear_points() {
        // A 20 x 10 rectangle with extra points on its edges and a 0.4 m kink.
        let ring = vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.4], [20.0, 0.0], [20.0, 10.0], [10.0, 10.0], [0.0, 10.0], [0.0, 5.0]];
        let s = simplify_ring(&ring, SIMPLIFY_M);
        assert_eq!(s.len(), 4, "{s:?}");
        for c in [[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]] {
            assert!(s.contains(&c), "corner {c:?} kept: {s:?}");
        }
        let tri = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        assert_eq!(simplify_ring(&tri, 5.0), tri, "a triangle stays");
    }

    #[test]
    fn chunks_are_one_contiguous_range_per_cell_and_keep_every_triangle() {
        // Groups 0..6: 0, 2, 4 in cell 10; 1, 5 in cell 11; 3 in no cell; 6 in a cell of another district.
        let group_cell = vec![10, 11, 10, hlod_bake::NO_CELL, 10, 11, 99];
        let mut vertices = vec![];
        let mut indices = vec![];
        for g in 0..7u32 {
            let b = vertices.len() as u32;
            for k in 0..3 {
                vertices.push(HlodVertex { pos: [g as f32 * 10.0 + k as f32, k as f32, -(g as f32)], normal: 0, uv: 0, color: 0, group: g });
            }
            indices.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b]);
        }
        let mut m = DistrictMesh { entry: 0, key: (0, 0), vertices, indices: indices.clone(), min: [0.0; 3], max: [0.0; 3], buildings: 7 };
        let chunks = chunk_by_cell(&mut m, &group_cell, &[11, 10]);
        assert_eq!(chunks.len(), 3, "{chunks:?}");
        assert_eq!((chunks[0].slot, chunks[0].cell, chunks[0].index_count), (0, Some(11), 12));
        assert_eq!((chunks[1].slot, chunks[1].cell, chunks[1].index_count), (1, Some(10), 18));
        assert_eq!((chunks[2].slot, chunks[2].cell, chunks[2].index_count), (2, None, 12), "no cell, or another district's: never skipped");
        let mut at = 0;
        for c in &chunks {
            assert_eq!(c.first_index, at);
            at += c.index_count;
            for t in m.indices[c.first_index as usize..(c.first_index + c.index_count) as usize].chunks(3) {
                let g = m.vertices[t[0] as usize].group;
                assert_eq!(group_cell[g as usize] == 11, c.cell == Some(11));
                for &i in t {
                    let p = m.vertices[i as usize].pos;
                    assert!((0..3).all(|k| p[k] >= c.min[k] && p[k] <= c.max[k]), "box holds its triangles");
                }
            }
        }
        let mut a: Vec<[u32; 3]> = indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
        let mut b: Vec<[u32; 3]> = m.indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "the same triangles");
    }

    #[test]
    fn the_window_filter_keeps_its_mean_and_is_sharp_up_close() {
        // averaged over many periods the filtered pulse is the duty, at any filter width.
        for duty in [0.1f32, 0.35, 0.5, 0.9] {
            for w in [0.0f32, 0.2, 0.7, 1.0, 1.3, 3.7, 20.0] {
                let n = 4000;
                let mean: f32 = (0..n).map(|k| pulse_filtered(k as f32 * 0.01237 + 0.3, duty, w)).sum::<f32>() / n as f32;
                assert!((mean - duty).abs() < 0.01, "duty {duty} width {w}: mean {mean}");
            }
            // Wider than a few periods: the duty everywhere (no shimmer).
            for k in 0..200 {
                let v = pulse_filtered(k as f32 * 0.173, duty, 8.0);
                assert!((v - duty).abs() <= duty.min(1.0 - duty) / 8.0 + 1e-4, "{v}");
            }
            // A whole number of periods wide: exactly the duty.
            assert!((pulse_filtered(0.42, duty, 2.0) - duty).abs() < 1e-5);
            // Unfiltered: inside the pulse 1, outside 0.
            assert_eq!(pulse_filtered(3.0 + duty * 0.5, duty, 0.0), 1.0);
            assert_eq!(pulse_filtered(3.0 + duty + (1.0 - duty) * 0.5, duty, 0.0), 0.0);
        }
    }

    #[test]
    fn the_rhythm_packs_into_the_normal_word() {
        let w = pack_rhythm([3.1, 3.2, 0.4, 0.55, 0.3]);
        let r = unpack_rhythm(w);
        assert!((r[0] - 3.1).abs() <= 0.03 && (r[1] - 3.2).abs() <= 0.015, "{r:?}");
        assert!((r[2] - 0.4).abs() <= 0.002 && (r[3] - 0.55).abs() <= 0.002, "{r:?}");
        assert_eq!(pack_rhythm([0.0; 5]), 0, "unknown rhythm: zero");
        assert!(pack_rhythm([20.0, 9.0, 1.0, 1.0, 0.0]) != 0);
    }

    #[test]
    fn a_district_wall_carries_the_opaque_wall_rhythm_and_tower_flag() {
        let s = BuildingSummary { group: 3, building: 0, facade: [0.3, 0.3, 0.3], glass: 0.4, roof: [0.1; 3], base: 0.0, top: 40.0, wall: [0.5, 0.4, 0.3], rhythm: [3.0, 3.6, 0.5, 0.6, 0.25], tower: 1.0 };
        let (color, uv, normal) = vertex_words(&s, true);
        assert_eq!(color & 255, hlod_bake::linear_to_srgb_byte(0.5) as u32, "the opaque wall, not the mean");
        assert_eq!((color >> 24) & (VF_DISTRICT | hlod_bake::VF_TOWER), VF_DISTRICT | hlod_bake::VF_TOWER);
        assert_eq!(normal, pack_rhythm(s.rhythm));
        assert_eq!((uv >> 16) & 255, 102, "the window share");
        assert_eq!(uv >> 24, 64, "the phase");
        let (old, _, _) = vertex_words(&s, false);
        assert_eq!(old & 255, hlod_bake::linear_to_srgb_byte(0.3) as u32, "glass body off: the facade mean as before");
    }

    #[test]
    fn district_round_trips_and_refuses_damage() {
        let v = HlodVertex { pos: [1.0, 2.0, 3.0], normal: 0, uv: 5, color: 6, group: 7 };
        let m = DistrictMesh { entry: 9, key: (-1, 2), vertices: vec![v, v, v], indices: vec![0, 1, 2], min: [1.0; 3], max: [3.0; 3], buildings: 1 };
        let b = m.encode();
        assert_eq!(DistrictMesh::decode(&b, 9).unwrap(), m);
        let mut bad = b.clone();
        bad[60] ^= 1;
        assert!(DistrictMesh::decode(&bad, 9).is_err());
        assert!(DistrictMesh::decode(&b[..b.len() - 3], 9).is_err());
    }
}
