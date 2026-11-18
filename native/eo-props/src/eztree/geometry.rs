//! geometry.js `buildEzTreeGeometry`, ported: the seed hash, the crown-cover
//! leaf-scale search (`crownLeafScale`), one build (`buildOnce`: trunk bend,
//! metric scale with the crown squeeze, `liftLeaves`, `pruneLeaves`,
//! `crownCoverOf`, `remapBark`, `remapLeaves` with the crown normals, merge).
//! Attributes are held as the JS's Float32Arrays hold them (read as f64, math
//! in f64, written back as f32), so every intermediate rounding matches.

use super::generator::{Buffers, Detail, Tree, TreeOptions, js_hypot, js_max, js_min, js_round};
use super::{EzData, EzMesh, EzSpecies, EzUserData};
use anyhow::{Result, anyhow};
use std::f64::consts::PI;

/// One part (wood or leaves) as a three BufferGeometry holds it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Part {
    pub pos: Vec<f32>,
    pub nrm: Vec<f32>,
    pub uv: Vec<f32>,
    pub color: Vec<f32>,
    pub index: Vec<u32>,
}

impl Part {
    fn from(b: Buffers) -> Self {
        Part { pos: b.verts, nrm: b.normals, uv: b.uvs, color: vec![], index: b.indices }
    }
    fn count(&self) -> usize {
        self.pos.len() / 3
    }
    fn x(&self, i: usize) -> f64 {
        self.pos[i * 3] as f64
    }
    fn y(&self, i: usize) -> f64 {
        self.pos[i * 3 + 1] as f64
    }
    fn z(&self, i: usize) -> f64 {
        self.pos[i * 3 + 2] as f64
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Box3 {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Box3 {
    pub fn empty() -> Self {
        Box3 { min: [f64::INFINITY; 3], max: [f64::NEG_INFINITY; 3] }
    }
    pub fn of(pos: &[f32]) -> Self {
        let mut b = Box3::empty();
        for p in pos.chunks_exact(3) {
            for k in 0..3 {
                b.min[k] = js_min(b.min[k], p[k] as f64);
                b.max[k] = js_max(b.max[k], p[k] as f64);
            }
        }
        b
    }
    fn union(mut self, o: &Box3) -> Self {
        for k in 0..3 {
            self.min[k] = js_min(self.min[k], o.min[k]);
            self.max[k] = js_max(self.max[k], o.max[k]);
        }
        self
    }
    fn is_empty(&self) -> bool {
        self.max[0] < self.min[0] || self.max[1] < self.min[1] || self.max[2] < self.min[2]
    }
    pub fn center(&self) -> [f64; 3] {
        if self.is_empty() {
            [0.0; 3]
        } else {
            [(self.min[0] + self.max[0]) * 0.5, (self.min[1] + self.max[1]) * 0.5, (self.min[2] + self.max[2]) * 0.5]
        }
    }
    fn half(&self) -> [f64; 3] {
        if self.is_empty() {
            [0.0; 3]
        } else {
            [(self.max[0] - self.min[0]) * 0.5, (self.max[1] - self.min[1]) * 0.5, (self.max[2] - self.min[2]) * 0.5]
        }
    }
}

/// geometry.js `hashSeed` (FNV-1a over UTF-16 code units, then `% 100000`).
pub(crate) fn hash_seed(text: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for c in text.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(16777619);
    }
    h % 100000
}

fn tier<'a>(sp: &'a EzSpecies, tier: &str) -> Result<&'a super::EzTier> {
    sp.tiers.get(tier).ok_or_else(|| anyhow!("Unknown ez-tree tier {tier}"))
}

/// `generateRaw`: the generator's wood and leaves for a species at a tier.
fn generate_raw(data: &EzData, sp: &EzSpecies, tier_name: &str, seed: u32, leaf_scale: f64) -> Result<(Part, Part)> {
    let options = TreeOptions::from_value(&data.options_value(&sp.id, seed as f64)?)?;
    let t = tier(sp, tier_name)?;
    let tier_leaf_scale = t.leaf_scale.unwrap_or(1.0);
    let detail = Detail {
        section_stride: t.section_stride.unwrap_or(1.0),
        segment_factor: t.segment_factor.unwrap_or(1.0),
        leaf_stride: t.leaf_stride.unwrap_or(1.0),
        leaf_scale: if leaf_scale == 1.0 { tier_leaf_scale } else { tier_leaf_scale * leaf_scale },
        billboard_double: t.billboard.as_deref().map(|b| b == "double"),
    };
    let (wood, leaves) = Tree::generate(options, &detail);
    Ok((Part::from(wood), Part::from(leaves)))
}

/// `scaleGeometry`: non-uniform scale with the inverse-transpose normal fix.
fn scale_part(p: &mut Part, sx: f64, sy: f64, sz: f64) {
    for i in 0..p.count() {
        p.pos[i * 3] = (p.x(i) * sx) as f32;
        p.pos[i * 3 + 1] = (p.y(i) * sy) as f32;
        p.pos[i * 3 + 2] = (p.z(i) * sz) as f32;
        if !p.nrm.is_empty() {
            let (nx, ny, nz) = (p.nrm[i * 3] as f64 / sx, p.nrm[i * 3 + 1] as f64 / sy, p.nrm[i * 3 + 2] as f64 / sz);
            let l = (nx * nx + ny * ny + nz * nz).sqrt();
            let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
            let s = 1.0 / l;
            p.nrm[i * 3] = (nx * s) as f32;
            p.nrm[i * 3 + 1] = (ny * s) as f32;
            p.nrm[i * 3 + 2] = (nz * s) as f32;
        }
    }
}

/// `bendTrunk` (in the generator's units).
fn bend_trunk(p: &mut Part, sway: f64, lean: f64, sway_to: Option<f64>, height: f64) {
    let to = js_max(0.1, sway_to.filter(|v| *v != 0.0 && !v.is_nan()).unwrap_or(0.6));
    for i in 0..p.count() {
        let t = js_max(0.0, p.y(i)) / height;
        let s = if t < to { ((2.0 * PI * t) / to).sin() * js_min(1.0, t / (to * 0.25)) } else { 0.0 };
        p.pos[i * 3] = (p.x(i) + sway * s + lean * t.powf(1.3)) as f32;
    }
}

/// `remapBark`.
fn remap_bark(data: &EzData, p: &mut Part, tint: [f64; 3]) {
    let b = data.bark;
    p.color = vec![0.0; p.count() * 3];
    for i in 0..p.count() {
        let u = js_min(1.0, js_max(0.0, p.uv[i * 2] as f64));
        let v = js_min(1.0, js_max(0.0, p.uv[i * 2 + 1] as f64));
        p.uv[i * 2] = (b.u0 + u * (b.u1 - b.u0)) as f32;
        p.uv[i * 2 + 1] = (b.v0 + v * (b.v1 - b.v0)) as f32;
        let root = 0.82 + 0.18 * js_min(1.0, p.y(i) / 1.2);
        for k in 0..3 {
            p.color[i * 3 + k] = (tint[k] * root) as f32;
        }
    }
}

/// `remapLeaves`: the sprig cell, the baked crown tone, the crown normals.
fn remap_leaves(data: &EzData, p: &mut Part, cell_id: &str, tint: [f64; 3]) -> Result<()> {
    let cell = *data.cells.get(cell_id).ok_or_else(|| anyhow!("unknown atlas cell {cell_id}"))?;
    // Sources without the crown-normal rule (`EZ_CROWN_NORMAL`) keep the generator's normals.
    let (normals, cn) = match data.crown_normal {
        Some(cn) if data.crown_normals_enabled => (true, cn),
        _ => (false, super::CrownNormal { outward: 0.0, card: 0.0, up: 0.0 }),
    };
    let bx = Box3::of(&p.pos);
    let centre = bx.center();
    let half = bx.half();
    let hx = js_max(half[0], 1e-3);
    let hy = js_max(half[1], 1e-3);
    let hz = js_max(half[2], 1e-3);
    let (mut mx, mut my, mut mz) = (0.0f64, 0.0f64, 0.0f64);
    if normals {
        for i in 0..p.count() {
            mx += p.x(i);
            my += p.y(i);
            mz += p.z(i);
        }
        let count = p.count().max(1) as f64;
        mx /= count;
        my /= count;
        mz /= count;
    }
    p.color = vec![0.0; p.count() * 3];
    for i in 0..p.count() {
        let (u, v) = (p.uv[i * 2] as f64, p.uv[i * 2 + 1] as f64);
        p.uv[i * 2] = (cell.u0 + u * (cell.u1 - cell.u0)) as f32;
        p.uv[i * 2 + 1] = (cell.v0 + v * (cell.v1 - cell.v0)) as f32;
        let dx = (p.x(i) - centre[0]) / hx;
        let dy = (p.y(i) - centre[1]) / hy;
        let dz = (p.z(i) - centre[2]) / hz;
        let outer = js_min(1.0, js_hypot(&[dx, dy, dz]));
        let tone = js_max(0.3, js_min(1.12, 0.45 + 0.5 * outer + 0.14 * dy));
        for k in 0..3 {
            p.color[i * 3 + k] = (tint[k] * tone) as f32;
        }
        if !normals {
            continue;
        }
        let mut ox = (p.x(i) - mx) / (hx * hx);
        let mut oy = (p.y(i) - my) / (hy * hy);
        let mut oz = (p.z(i) - mz) / (hz * hz);
        let reach = js_hypot(&[ox, oy, oz]);
        if reach > 1e-6 {
            ox /= reach;
            oy /= reach;
            oz /= reach;
        } else {
            ox = 0.0;
            oy = 1.0;
            oz = 0.0;
        }
        let (cx, cy, cz) = (p.nrm[i * 3] as f64, p.nrm[i * 3 + 1] as f64, p.nrm[i * 3 + 2] as f64);
        let side = (if cx * ox + cy * oy + cz * oz < 0.0 { -1.0 } else { 1.0 }) * cn.card;
        let nx = cn.outward * ox + side * cx;
        let ny = cn.outward * oy + side * cy + cn.up;
        let nz = cn.outward * oz + side * cz;
        let length = js_hypot(&[nx, ny, nz]);
        p.nrm[i * 3] = (nx / length) as f32;
        p.nrm[i * 3 + 1] = (ny / length) as f32;
        p.nrm[i * 3 + 2] = (nz / length) as f32;
    }
    Ok(())
}

/// `compactToIndex`: keep only the vertices `index` references, first use first.
fn compact_to_index(p: &mut Part, index: Vec<u32>) {
    let mut remap = vec![-1i64; p.count()];
    let mut order = Vec::new();
    for &v in &index {
        if remap[v as usize] < 0 {
            remap[v as usize] = order.len() as i64;
            order.push(v as usize);
        }
    }
    let pick = |a: &Vec<f32>, size: usize| -> Vec<f32> {
        if a.is_empty() {
            return vec![];
        }
        let mut out = Vec::with_capacity(order.len() * size);
        for &o in &order {
            out.extend_from_slice(&a[o * size..o * size + size]);
        }
        out
    };
    p.pos = pick(&p.pos, 3);
    p.nrm = pick(&p.nrm, 3);
    p.uv = pick(&p.uv, 2);
    p.color = pick(&p.color, 3);
    p.index = index.iter().map(|&v| remap[v as usize] as u32).collect();
}

/// `pruneLeaves`: drop leaf triangles reaching below the clearance (and their vertices).
fn prune_leaves(p: &mut Part, clearance: f64) -> u32 {
    if !(clearance > 0.0) {
        return 0;
    }
    let mut kept = Vec::with_capacity(p.index.len());
    for t in p.index.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        if js_min(js_min(p.y(a), p.y(b)), p.y(c)) < clearance {
            continue;
        }
        kept.extend_from_slice(t);
    }
    let dropped = ((p.index.len() - kept.len()) / 3) as u32;
    if dropped == 0 {
        return 0;
    }
    compact_to_index(p, kept);
    dropped
}

/// `liftLeaves`: raise low cards onto the clearance, within a triangle budget.
fn lift_leaves(p: &mut Part, clearance: f64, top_m: f64, budget: f64) -> u32 {
    if !(clearance > 0.0) {
        return 0;
    }
    struct Card {
        v: usize,
        ys: [f64; 4],
        rise: f64,
        squash: bool,
        top: bool,
        cost: f64,
    }
    let mut crown_top = f64::NEG_INFINITY;
    for v in 0..p.count() {
        crown_top = js_max(crown_top, p.y(v));
    }
    let mut cards = Vec::new();
    let mut kept = 0.0f64;
    let mut v = 0;
    while v + 3 < p.count() {
        let ys = [p.y(v), p.y(v + 1), p.y(v + 2), p.y(v + 3)];
        let m3 = |a: f64, b: f64, c: f64| js_min(js_min(a, b), c);
        let survivors = (if m3(ys[0], ys[1], ys[2]) >= clearance { 1.0 } else { 0.0 }) + (if m3(ys[0], ys[2], ys[3]) >= clearance { 1.0 } else { 0.0 });
        kept += survivors;
        let low = js_min(js_min(ys[0], ys[1]), js_min(ys[2], ys[3]));
        if low >= clearance {
            v += 4;
            continue;
        }
        let high = js_max(js_max(ys[0], ys[1]), js_max(ys[2], ys[3]));
        let rise = js_min(clearance - low + 1e-3, top_m - high);
        let squash = rise < clearance - low;
        cards.push(Card { v, ys, rise: clearance - low, squash, top: high >= crown_top - 1e-6, cost: 2.0 - survivors });
        v += 4;
    }
    cards.sort_by(|a, b| {
        (b.top as u8)
            .cmp(&(a.top as u8))
            .then(a.rise.partial_cmp(&b.rise).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.v.cmp(&b.v))
    });
    let mut lifted = 0;
    for card in &cards {
        if kept + card.cost > budget {
            break;
        }
        for k in 0..4 {
            let y = card.ys[k];
            let ny = if card.squash { js_max(y, clearance + 1e-3) } else { js_min(y + card.rise + 1e-3, top_m) };
            p.pos[(card.v + k) * 3 + 1] = ny as f32;
        }
        kept += card.cost;
        lifted += 1;
    }
    lifted
}

/// `crownCoverOf`: opaque leaf area over the crown's side silhouette.
fn crown_cover_of(data: &EzData, p: &Part, cell_id: &str) -> f64 {
    let triangles = p.index.len() / 3;
    if triangles == 0 {
        return 0.0;
    }
    let mut area = 0.0f64;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    let mut reach = 0.0f64;
    let at = |i: u32| [p.x(i as usize), p.y(i as usize), p.z(i as usize)];
    for t in p.index.chunks_exact(3) {
        let (a, b, c) = (at(t[0]), at(t[1]), at(t[2]));
        for v in [a, b, c] {
            y_min = js_min(y_min, v[1]);
            y_max = js_max(y_max, v[1]);
            reach = js_max(reach, js_hypot(&[v[0], v[2]]));
        }
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cx = e1[1] * e2[2] - e1[2] * e2[1];
        let cy = e1[2] * e2[0] - e1[0] * e2[2];
        let cz = e1[0] * e2[1] - e1[1] * e2[0];
        area += (cx * cx + cy * cy + cz * cz).sqrt() / 2.0;
    }
    let silhouette = (PI * reach * js_max(y_max - y_min, 1e-3)) / 2.0;
    (area * data.leaf_opaque_fraction.get(cell_id).copied().unwrap_or(1.0)) / js_max(silhouette, 1e-6)
}

/// `buildOnce`: one uncached build at a leaf-card multiplier.
fn build_once(data: &EzData, sp: &EzSpecies, tier_name: &str, seed: u32, leaf_scale: f64, budget: f64, lift: bool) -> Result<EzMesh> {
    let (mut wood, mut leaves) = generate_raw(data, sp, tier_name, seed, leaf_scale)?;
    let measure = |w: &Part, l: &Part| {
        let b = Box3::of(&w.pos);
        if l.count() > 0 { b.union(&Box3::of(&l.pos)) } else { b }
    };
    let mut bounds = measure(&wood, &leaves);
    let raw_height = js_max(bounds.max[1], 1e-3);
    let s = sp.height_m / raw_height;
    if let Some(b) = sp.trunk_bend {
        let num = |v: Option<f64>| v.filter(|x| !x.is_nan()).unwrap_or(0.0);
        let (sway, lean) = (num(b.sway_m) / s, num(b.lean_m) / s);
        for part in [&mut wood, &mut leaves] {
            bend_trunk(part, sway, lean, b.sway_to, raw_height);
        }
        bounds = measure(&wood, &leaves);
    }
    let raw_half = js_max(js_max(-bounds.min[0], bounds.max[0]), js_max(-bounds.min[2], bounds.max[2]));
    let squeeze = js_min(1.0, sp.max_half_crown_m / js_max(raw_half * s, 1e-3));
    for part in [&mut wood, &mut leaves] {
        scale_part(part, s * squeeze, s, s * squeeze);
    }
    let clearance = sp.clearance_m.unwrap_or(0.0);
    let t = tier(sp, tier_name)?;
    let lifted = if lift && t.lift_leaves { lift_leaves(&mut leaves, clearance, sp.height_m, budget) } else { 0 };
    let pruned = prune_leaves(&mut leaves, clearance);
    let crown_cover = crown_cover_of(data, &leaves, &sp.leaf);
    remap_bark(data, &mut wood, sp.bark_tint);
    remap_leaves(data, &mut leaves, &sp.leaf, sp.leaf_tint)?;
    let wood_triangles = (wood.index.len() / 3) as u32;
    let wood_vertices = wood.count();
    let leaf_triangles = (leaves.index.len() / 3) as u32;

    let mut m = EzMesh::default();
    let mut push = |p: &Part, part: u8| {
        let base = m.positions.len() as u32;
        for i in 0..p.count() {
            m.positions.push([p.pos[i * 3], p.pos[i * 3 + 1], p.pos[i * 3 + 2]]);
            m.normals.push([p.nrm[i * 3], p.nrm[i * 3 + 1], p.nrm[i * 3 + 2]]);
            m.uvs.push([p.uv[i * 2], p.uv[i * 2 + 1]]);
            m.colors.push([p.color[i * 3], p.color[i * 3 + 1], p.color[i * 3 + 2]]);
            m.part.push(part);
        }
        m.indices.extend(p.index.iter().map(|&v| v + base));
    };
    push(&wood, 0);
    if leaf_triangles > 0 {
        push(&leaves, 1);
    }
    let mut trunk_radius = 0.0f64;
    for p in &m.positions[..wood_vertices] {
        if (p[1] as f64) < 0.05 {
            trunk_radius = js_max(trunk_radius, js_hypot(&[p[0] as f64, p[2] as f64]));
        }
    }
    let flat: Vec<f32> = m.positions.iter().flatten().copied().collect();
    let bx = Box3::of(&flat);
    m.user = EzUserData {
        seed,
        height_m: bx.max[1],
        half_crown_m: js_max(js_max(-bx.min[0], bx.max[0]), js_max(-bx.min[2], bx.max[2])),
        trunk_radius_m: trunk_radius,
        wood_vertex_count: wood_vertices as u32,
        wood_triangles,
        leaf_triangles,
        pruned_leaf_triangles: pruned,
        lifted_leaf_cards: lifted,
        clearance_m: clearance,
        crown_cover,
        crown_leaf_scale: 1.0,
        crown_cover_before_rule: f64::NAN,
    };
    Ok(m)
}

struct LeafScale {
    leaf_scale: f64,
    cover_before: f64,
    budget: f64,
}

/// `crownLeafScale`: the multiplier that brings a tier up to EZ_CROWN_COVER.
fn crown_leaf_scale(data: &EzData, sp: &EzSpecies, seed: u32, tier_name: &str) -> Result<LeafScale> {
    if !sp.crown_cover {
        return Ok(LeafScale { leaf_scale: 1.0, cover_before: f64::NAN, budget: f64::INFINITY });
    }
    let start = if tier_name == "near" { 1.0 } else { crown_leaf_scale(data, sp, seed, "near")?.leaf_scale };
    let mut budget = f64::INFINITY;
    if tier(sp, tier_name)?.lift_leaves {
        let base = build_once(data, sp, tier_name, seed, start, 0.0, false)?;
        budget = base.user.leaf_triangles as f64;
    }
    let cover_at = |scale: f64| -> Result<f64> { Ok(build_once(data, sp, tier_name, seed, scale, budget, true)?.user.crown_cover) };
    let rule = data.crown_cover;
    let cap = start * rule.max_leaf_scale;
    let cover_before = cover_at(start)?;
    let mut leaf_scale = start;
    let mut cover = cover_before;
    let mut pass = 0;
    while pass < 6 && cover < rule.min_cover && leaf_scale < cap {
        let step = (rule.min_cover / js_max(cover, 1e-3)).sqrt() * 1.02;
        leaf_scale = js_round(js_min(cap, leaf_scale * step) * 1000.0) / 1000.0;
        cover = cover_at(leaf_scale)?;
        pass += 1;
    }
    Ok(LeafScale { leaf_scale, cover_before, budget })
}

/// geometry.js `buildEzTreeGeometry(speciesId, { tier, variant })`: one tree as
/// a single metric mesh, root at the origin, +Y up. Deterministic per
/// (species, tier, variant); not cached here (the cooker caches).
pub fn build_ez_tree(species_id: &str, tier: &str, variant: u32) -> Result<EzMesh> {
    let data = EzData::get()?;
    let sp = data.species(species_id)?;
    let seed_id = sp.seed_id.as_deref().unwrap_or(species_id);
    let seed = hash_seed(&format!("{}:{}:{}", data.geometry_version, seed_id, variant));
    let ls = crown_leaf_scale(data, sp, seed, tier)?;
    let mut m = build_once(data, sp, tier, seed, ls.leaf_scale, ls.budget, true)?;
    m.user.crown_leaf_scale = ls.leaf_scale;
    m.user.crown_cover_before_rule = ls.cover_before;
    Ok(m)
}
