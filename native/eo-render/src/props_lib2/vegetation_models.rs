//! The Golden Gate's vegetation and hill rock as props models (render ids
//! from [`VEG_BASE`]): the coastal trees and scrub (`eo_props::eztree`'s
//! coastal species, three levels each), grass bunches, wildflower clumps and
//! rock outcrop blocks. The app plants them (`eo_landmarks::vegetation`
//! decides where); nothing draws without instances.
//!
//! * Grass: bunches of crossed cards on the vegetation atlas's grass cell
//!   (`ez-vegetation-v1`, the ez-tree grass model's card, MIT), alpha-tested,
//!   with near-vertical shading normals and a wind weight rising to the tips.
//!   The atlas cell is grey: each instance carries its tint (the ground it
//!   grows on, see `eo_landmarks::vegetation::Plant::tint`).
//! * Wildflowers: a grass bunch with small opaque flower heads over it.
//! * Rock: displaced, flattened icospheres (value noise; the chert variants
//!   with planar bedding steps), buried about a fifth of their height, white
//!   vertex colour darkened in the hollows: the instance tint is the rock's
//!   colour and the shader adds a fine triplanar grain (`P_ROCK`).
//!
//! Every model past the trees thins with distance on the GPU (`thin_wgsl`):
//! an instance whose rank (`GpuPropInstance::slope`) is over the share kept
//! at its distance shrinks away, and the survivors grow to keep the cover.

use super::*;

/// The first vegetation model id (after the signs).
pub const VEG_BASE: u32 = OH_SIGN_BASE + MAX_SIGN_VARIANTS;
/// Coastal trees (`eo_props::eztree::COASTAL_TREES` order).
pub const V_TREE: u32 = VEG_BASE;
/// Coastal shrubs (`COASTAL_SHRUBS` order).
pub const V_SHRUB: u32 = VEG_BASE + 3;
/// A tall bunch of wild grass.
pub const V_GRASS: u32 = VEG_BASE + 5;
/// A short tuft (mown or grazed grass, dune grass).
pub const V_TUFT: u32 = VEG_BASE + 6;
/// Wildflower clumps: yellow, violet.
pub const V_FLOWER: u32 = VEG_BASE + 7;
/// Rock outcrop blocks (four shapes; 0 and 1 bedded).
pub const V_ROCK: u32 = VEG_BASE + 9;
pub const V_ROCKS: u32 = 4;
pub const COUNT: u32 = 13;

/// A material flag of our own: procedural rock grain in the colour pass.
pub const P_ROCK: u32 = 1 << 17;

/// Per model offset: (thin from m, draw distance m, the most a survivor
/// grows); 0 thin = never thins. The draw distances are the models' `lod.w`
/// (set below). The grass survivors grow little (1.1): a bunch grown by the
/// full 1/sqrt(share) turns into a long dark stroke on the bare ground tone,
/// where a shrub grown the same way still reads as a shrub.
pub const THIN: [(f32, f32, f32); COUNT as usize] = [
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (90.0, 320.0, 1.35),
    (90.0, 320.0, 1.35),
    (14.0, 100.0, 1.1),
    (8.0, 30.0, 1.1),
    (12.0, 60.0, 1.1),
    (12.0, 60.0, 1.1),
    (0.0, 1000.0, 1.0),
    (0.0, 1000.0, 1.0),
    (0.0, 1000.0, 1.0),
    (0.0, 1000.0, 1.0),
];
/// The share kept at the draw distance (thinned models).
pub const THIN_FLOOR: f32 = 0.12;

/// WGSL appended to props_types.wgsl: the vegetation range and its thinning
/// (the same numbers as `THIN`, so the cull, colour and shadow passes agree).
pub fn thin_wgsl() -> String {
    let mut s = format!("const PR_VEG_BASE: u32 = {VEG_BASE}u;\nconst PR_VEG_COUNT: u32 = {COUNT}u;\nconst PR_VEG_TREES: u32 = {}u;\nconst PR_VEG_GRASS: u32 = {}u;\nconst PR_VEG_ROCK: u32 = {}u;\n", V_SHRUB - V_TREE, V_GRASS - V_TREE, V_ROCK - V_TREE);
    s += &format!("const PR_VEG_FLOOR: f32 = {THIN_FLOOR:?};\nconst PR_VEG_GRASS_CELL_MEAN: f32 = {GRASS_CELL_MEAN:?};\n");
    s += "fn pr_veg_thin(model: u32) -> vec3<f32> {\n    switch (model - PR_VEG_BASE) {\n";
    for (k, (a, b, g)) in THIN.iter().enumerate() {
        if *a > 0.0 {
            s += &format!("        case {k}u: {{ return vec3<f32>({a:?}, {b:?}, {g:?}); }}\n");
        }
    }
    s += "        default: { return vec3<f32>(0.0, 1.0, 1.0); }\n    }\n}\n";
    s
}

// ------------------------------------------------------------------ noise

fn hash3(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841) ^ (z as u32).wrapping_mul(0xcb1ab31f) ^ seed.wrapping_mul(0x9e3779b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a2d39);
    h ^= h >> 15;
    (h & 0xffffff) as f32 / 16_777_215.0
}

fn value3(p: [f32; 3], seed: u32) -> f32 {
    let f = p.map(|v| v.floor());
    let t = [p[0] - f[0], p[1] - f[1], p[2] - f[2]].map(|t| t * t * (3.0 - 2.0 * t));
    let i = f.map(|v| v as i32);
    let mut acc = 0.0;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let w = (if dx == 1 { t[0] } else { 1.0 - t[0] }) * (if dy == 1 { t[1] } else { 1.0 - t[1] }) * (if dz == 1 { t[2] } else { 1.0 - t[2] });
                acc += w * hash3(i[0] + dx, i[1] + dy, i[2] + dz, seed);
            }
        }
    }
    acc
}

fn fbm(p: [f32; 3], seed: u32, octaves: u32) -> f32 {
    let (mut a, mut f, mut sum, mut norm) = (1.0, 1.0, 0.0, 0.0);
    for o in 0..octaves {
        sum += a * (value3([p[0] * f, p[1] * f, p[2] * f], seed + o * 31) * 2.0 - 1.0);
        norm += a;
        a *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

// ------------------------------------------------------------------ grass and flowers

/// The atlas grass cell's mean linear albedo over its texels with alpha over
/// 0.5 (`ez-vegetation-atlas-v1.png`, measured; the cell is grey).
pub const GRASS_CELL_MEAN: f32 = 0.311;

/// A card's base shade in quarters of its tip's (sRGB vertex byte).
pub const GRASS_BASE_SHADE_PER_4: u32 = 3;

/// The atlas grass cell in the builder's UVs (rows top first): `ez-vegetation-atlas-v1` `cells.grass`.
const GRASS_UV: [f32; 4] = [0.752, 0.248, 0.998, 0.002];

/// One grass card: `w` wide, `h` tall, standing at (x, z), turned `a`, leaning `lean` (m at the tip).
#[allow(clippy::too_many_arguments)]
fn card(b: &mut Builder, m: u32, x: f32, z: f32, a: f32, w: f32, h: f32, lean: f32, shade: u8) {
    let (s, c) = a.sin_cos();
    let (dx, dz) = (c * w / 2.0, s * w / 2.0);
    // the card's own normal, bent toward up so a bunch shades as a canopy, not as planes.
    let n = [-s * 0.35, 1.0, c * 0.35];
    let (lx, lz) = (-s * lean, c * lean);
    let base = b.v.len() as u32;
    // the base a little in the bunch's own shade, the tips full. Not darker than
    // three quarters (sRGB) of the tip: a darker base reads as a dark crescent cut
    // into the ground at 1.7 m, and as a dark stroke on the ground tone past 20 m.
    let lo = (shade as u32 * GRASS_BASE_SHADE_PER_4 / 4) as u8;
    let col_lo = [lo, lo, lo];
    let col_hi = [shade, shade, shade];
    let u = GRASS_UV;
    b.vert([x - dx, 0.0, z - dz], n, [u[0], u[1]], col_lo, m);
    b.vert([x + dx, 0.0, z + dz], n, [u[2], u[1]], col_lo, m);
    b.vert([x + dx + lx, h, z + dz + lz], n, [u[2], u[3]], col_hi, m);
    b.vert([x - dx + lx, h, z - dz + lz], n, [u[0], u[3]], col_hi, m);
    for k in 0..4 {
        let w = if k >= 2 { 1.0f32 } else { 0.0 };
        b.v[(base + k) as usize].pad = w.to_bits();
    }
    b.tri(true, base, base + 1, base + 2);
    b.tri(true, base, base + 2, base + 3);
}

/// A bunch of `n` cards in a disc of radius `r`, heights `h0..h1`, each card
/// `aspect` times as wide as tall (the atlas cell is square, its tuft about as
/// wide as tall: 1 keeps the blades their drawn width).
#[allow(clippy::too_many_arguments)]
fn bunch(b: &mut Builder, m: u32, n: u32, r: f32, h0: f32, h1: f32, aspect: f32, seed: u32) {
    for k in 0..n {
        let u = hash3(k as i32, 1, 7, seed);
        let v = hash3(k as i32, 2, 7, seed);
        let rr = r * u.sqrt();
        let a = v * std::f32::consts::TAU;
        let (x, z) = (rr * a.cos(), rr * a.sin());
        let h = h0 + (h1 - h0) * hash3(k as i32, 3, 7, seed);
        let turn = hash3(k as i32, 4, 7, seed) * std::f32::consts::PI;
        let lean = (hash3(k as i32, 5, 7, seed) - 0.3) * h * 0.25;
        let shade = 200 + (hash3(k as i32, 6, 7, seed) * 55.0) as u8;
        card(b, m, x, z, turn, h * aspect, h, lean, shade);
    }
}

/// Small opaque flower heads (a squashed octahedron each) over a bunch.
fn heads(b: &mut Builder, m: u32, n: u32, r: f32, h0: f32, h1: f32, size: f32, col: [u8; 3], seed: u32) {
    for k in 0..n {
        let u = hash3(k as i32, 1, 9, seed);
        let v = hash3(k as i32, 2, 9, seed);
        let rr = r * u.sqrt();
        let a = v * std::f32::consts::TAU;
        let c = [rr * a.cos(), h0 + (h1 - h0) * hash3(k as i32, 3, 9, seed), rr * a.sin()];
        let s = size * (0.7 + 0.6 * hash3(k as i32, 4, 9, seed));
        let pts = [[s, 0.0, 0.0], [-s, 0.0, 0.0], [0.0, s * 0.6, 0.0], [0.0, -s * 0.4, 0.0], [0.0, 0.0, s], [0.0, 0.0, -s]];
        let base = b.v.len() as u32;
        for p in pts {
            let id = b.vert([c[0] + p[0], c[1] + p[1], c[2] + p[2]], [p[0], p[1] + s * 0.8, p[2]], [0.5, 0.5], col, m);
            b.v[id as usize].pad = 1.0f32.to_bits();
        }
        for (i, j, l) in [(0, 2, 4), (4, 2, 1), (1, 2, 5), (5, 2, 0), (4, 3, 0), (1, 3, 4), (5, 3, 1), (0, 3, 5)] {
            b.tri(false, base + i, base + j, base + l);
        }
    }
}

// ------------------------------------------------------------------ rock

/// An icosphere of `subdiv` levels (unit radius).
fn icosphere(subdiv: u32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut v: Vec<[f32; 3]> = vec![
        [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
        [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
        [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
    ];
    let norm = |p: [f32; 3]| {
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        [p[0] / l, p[1] / l, p[2] / l]
    };
    for p in v.iter_mut() {
        *p = norm(*p);
    }
    let mut f: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..subdiv {
        let mut mid: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        let mut nf = Vec::with_capacity(f.len() * 4);
        let mut m = |a: u32, b: u32, v: &mut Vec<[f32; 3]>| -> u32 {
            let k = (a.min(b), a.max(b));
            if let Some(&i) = mid.get(&k) {
                return i;
            }
            let (pa, pb) = (v[a as usize], v[b as usize]);
            v.push(norm([(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0, (pa[2] + pb[2]) / 2.0]));
            let i = v.len() as u32 - 1;
            mid.insert(k, i);
            i
        };
        for t in &f {
            let (a, b2, c) = (m(t[0], t[1], &mut v), m(t[1], t[2], &mut v), m(t[2], t[0], &mut v));
            nf.extend_from_slice(&[[t[0], a, c], [t[1], b2, a], [t[2], c, b2], [a, b2, c]]);
        }
        f = nf;
    }
    (v, f)
}

/// One rock: unit-ish size (about 1 m across before the instance scale),
/// flattened, its lowest fifth below y = 0. `bedded`: planar steps along a
/// tilted bedding plane (the ribbon chert's beds).
fn rock(b: &mut Builder, m: u32, subdiv: u32, seed: u32, bedded: bool) {
    let (unit, faces) = icosphere(subdiv);
    let stretch = [0.95 + 0.4 * hash3(1, 0, 0, seed), 0.55 + 0.2 * hash3(2, 0, 0, seed), 0.8 + 0.3 * hash3(3, 0, 0, seed)];
    let bed = {
        let a = hash3(4, 0, 0, seed) * std::f32::consts::TAU;
        let tilt = 0.35 + 0.5 * hash3(5, 0, 0, seed);
        let d = [a.cos() * tilt.sin(), tilt.cos(), a.sin() * tilt.sin()];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        [d[0] / l, d[1] / l, d[2] / l]
    };
    // a block of fractured rock: the convex solid cut by `CUTS` random planes (joint
    // faces), sampled along each sphere direction (the nearest plane the ray meets), so
    // the faces are flat and the edges sharp; a little noise breaks the planes. The
    // bedded (chert) shapes add slabs parallel to the bedding.
    const CUTS: u32 = 20;
    let mut planes: Vec<([f32; 3], f32)> = (0..CUTS)
        .map(|k| {
            let u = hash3(k as i32, 1, 3, seed) * 2.0 - 1.0;
            let a = hash3(k as i32, 2, 3, seed) * std::f32::consts::TAU;
            let r = (1.0 - u * u).max(0.0).sqrt();
            ([r * a.cos(), u, r * a.sin()], 0.62 + 0.3 * hash3(k as i32, 4, 3, seed))
        })
        .collect();
    if bedded {
        for (k, d) in [0.42f32, 0.7].iter().enumerate() {
            planes.push((bed, *d + 0.08 * hash3(k as i32, 5, 3, seed)));
            planes.push(([-bed[0], -bed[1], -bed[2]], *d + 0.08 * hash3(k as i32, 6, 3, seed)));
        }
    }
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(unit.len());
    let mut hollow: Vec<f32> = Vec::with_capacity(unit.len());
    for p in &unit {
        let mut r = 1.25f32;
        for (n, d) in &planes {
            let c = p[0] * n[0] + p[1] * n[1] + p[2] * n[2];
            if c > 1e-3 {
                r = r.min(d / c);
            }
        }
        let q = [p[0] * 2.3, p[1] * 2.3, p[2] * 2.3];
        let big = fbm(q, seed, 3);
        let fine = fbm([q[0] * 4.0, q[1] * 4.0, q[2] * 4.0], seed + 101, 2);
        r *= 1.0 + 0.05 * big + 0.025 * fine;
        if bedded {
            // the beds' partings: a thin groove every quarter of the block along the bedding normal.
            let along = p[0] * bed[0] + p[1] * bed[1] + p[2] * bed[2];
            let f = (along * r * 4.0).fract();
            r *= 1.0 - 0.035 * (1.0 - (f - 0.5).abs() * 2.0).powf(6.0);
        }
        hollow.push(big);
        pos.push([p[0] * r * stretch[0] * 0.5, p[1] * r * stretch[1] * 0.5, p[2] * r * stretch[2] * 0.5]);
    }
    // sit it on the ground: a fifth of its height below y = 0.
    let (ymin, ymax) = pos.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
    let sink = ymin + (ymax - ymin) * 0.2;
    for p in pos.iter_mut() {
        p[1] -= sink;
    }
    // normals: per corner, the faces round the vertex that lie within `CREASE` of the
    // corner's own face (the joint planes stay flat, their edges sharp).
    const CREASE: f32 = 0.82; // cos 35 degrees
    let face_n: Vec<[f32; 3]> = faces
        .iter()
        .map(|t| {
            let (a, bb, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
            let e1 = [bb[0] - a[0], bb[1] - a[1], bb[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
            [n[0] / l, n[1] / l, n[2] / l]
        })
        .collect();
    let mut around: Vec<Vec<usize>> = vec![vec![]; pos.len()];
    for (f, t) in faces.iter().enumerate() {
        for &k in t {
            around[k as usize].push(f);
        }
    }
    for (f, t) in faces.iter().enumerate() {
        let base = b.v.len() as u32;
        for &k in t {
            let p = pos[k as usize];
            let mut n = [0.0f32; 3];
            for &g in &around[k as usize] {
                let d = face_n[g][0] * face_n[f][0] + face_n[g][1] * face_n[f][1] + face_n[g][2] * face_n[f][2];
                if d >= CREASE {
                    for j in 0..3 {
                        n[j] += face_n[g][j];
                    }
                }
            }
            // hollows and the buried underside darker (cavity), the tops lighter.
            let up = (p[1] / (ymax - sink).max(1e-3)).clamp(-1.0, 1.0);
            let cav = (0.78 + 0.22 * up + 0.35 * hollow[k as usize]).clamp(0.45, 1.0);
            let g = (cav * 235.0) as u8;
            b.vert(p, n, [0.5, 0.5], [g, g, g], m);
        }
        b.tri(false, base, base + 1, base + 2);
    }
}

// ------------------------------------------------------------------ the cook

/// Add the vegetation models (`m_bark`, `m_leaf`: the library's atlas bark
/// and leaf materials, also the grass cards').
pub(crate) fn cook(out: &mut Out, ez: Option<&eo_props::eztree::EzLibrary>, m_bark: u32, m_leaf: u32) {
    let thin = |k: u32| {
        let t = THIN[(k - VEG_BASE) as usize];
        (t.0, t.1)
    };
    // --- the coastal trees and shrubs: near, coarse, distant from the cooked builds.
    if let Some(ez) = ez {
        let trees = eo_props::eztree::COASTAL_TREES.iter().enumerate().map(|(k, id)| (V_TREE + k as u32, *id, [70.0f32, 200.0], 3u32));
        let shrubs = eo_props::eztree::COASTAL_SHRUBS.iter().enumerate().map(|(k, id)| (V_SHRUB + k as u32, *id, [22.0f32, 70.0], 2u32));
        for (model, id, edges, cast) in trees.chain(shrubs) {
            let mut ids = [NONE; 3];
            for (k, tier) in ["near", "coarse", "distant"].iter().enumerate() {
                match ez.get(id, tier, 0) {
                    Some(m) => {
                        super::tree_mesh(&mut out.b, m, m_bark, m_leaf);
                        ids[k] = out.finish();
                    }
                    None => out.missing.push(format!("coastal species {id} {tier}: not cooked")),
                }
            }
            if ids.iter().all(|i| *i != NONE) {
                let (z, w) = thin(model);
                out.set(model, ids, [edges[0], edges[1], z, w], cast, 0.15);
            }
        }
        out.sources.push(format!("coastal trees and scrub: {} ez-tree species (eo_props::eztree coastal.rs)", eo_props::eztree::COASTAL_TREES.len() + eo_props::eztree::COASTAL_SHRUBS.len()));
    }
    // --- grass: near (dense), mid, far.
    // a patch of sward about 1.3 m across (the layer sets one a square metre, so
    // they overlap). The mid and far levels are wider, lower cards, more of them
    // than a plain decimation keeps: from 9 m a few tall cards read as single
    // strokes, a low wide card as a stretch of sward.
    // (cards, radius, lowest and tallest card m, width / height)
    let grass_lods: [(u32, f32, f32, f32, f32); 3] = [(28, 0.65, 0.28, 0.65, 0.95), (16, 0.7, 0.22, 0.45, 1.7), (9, 0.7, 0.18, 0.34, 2.4)];
    let mut ids = [NONE; 3];
    for (k, (n, r, h0, h1, a)) in grass_lods.iter().enumerate() {
        bunch(&mut out.b, m_leaf, *n, *r, *h0, *h1, *a, 11 + k as u32);
        ids[k] = out.finish();
    }
    let (z, w) = thin(V_GRASS);
    out.set(V_GRASS, ids, [9.0, 22.0, z, w], 0, 0.1);
    // --- short tufts.
    let mut ids = [NONE; 3];
    for (k, (n, a)) in [(16u32, 1.0f32), (7, 1.2), (3, 1.4)].iter().enumerate() {
        bunch(&mut out.b, m_leaf, *n, 0.45, 0.12, 0.3, *a, 23 + k as u32);
        ids[k] = out.finish();
    }
    let (z, w) = thin(V_TUFT);
    out.set(V_TUFT, ids, [7.0, 16.0, z, w], 0, 0.05);
    // --- wildflowers: a bunch with heads; the far level the heads only, larger.
    // Head colours (sRGB, INFERRED from the species' published flower colour words):
    // yellow (the headlands' yellow composites and mustards), violet (lupine).
    // The heads take the instance tint (the ground's tone, for the bunch's
    // blades) only before `--no-bloom-date`: it drew them a muddy olive. With
    // the cited bloom window they keep their own colour (no leaf flag: the
    // shader tints leaf and rock materials only).
    let head_flags = if eo_landmarks::vegetation::gg3().bloom_date { 0 } else { P_LEAF };
    let m_head = push_mat(out.materials, [255, 255, 255], 0.6, 0.0, head_flags, 0);
    for (k, col) in [[232u8, 196, 40], [120, 96, 196]].into_iter().enumerate() {
        let mut ids = [NONE; 3];
        bunch(&mut out.b, m_leaf, 7, 0.26, 0.25, 0.5, 0.9, 41 + k as u32);
        heads(&mut out.b, m_head, 14, 0.3, 0.4, 0.62, 0.025, col, 43 + k as u32);
        ids[0] = out.finish();
        bunch(&mut out.b, m_leaf, 3, 0.26, 0.25, 0.5, 1.1, 41 + k as u32);
        heads(&mut out.b, m_head, 7, 0.3, 0.4, 0.62, 0.04, col, 43 + k as u32);
        ids[1] = out.finish();
        heads(&mut out.b, m_head, 4, 0.3, 0.35, 0.55, 0.07, col, 43 + k as u32);
        ids[2] = out.finish();
        let model = V_FLOWER + k as u32;
        let (z, w) = thin(model);
        out.set(model, ids, [10.0, 30.0, z, w], 0, 0.05);
    }
    // --- rock blocks: four shapes, three levels.
    let m_rock = push_mat(out.materials, [255, 255, 255], 0.86, 0.0, P_ROCK, 0);
    for k in 0..V_ROCKS {
        let mut ids = [NONE; 3];
        for (l, sub) in [3u32, 2, 1].iter().enumerate() {
            rock(&mut out.b, m_rock, *sub, 501 + k * 17, k < 2);
            ids[l] = out.finish();
        }
        let model = V_ROCK + k;
        let (z, w) = thin(model);
        // the cascades see the near ones only (a block past 110 m casts a shadow under a texel).
        out.set(model, ids, [30.0, 110.0, z, w], 3, 0.0);
    }
    out.sources.push("vegetation: grass, tufts, wildflowers, rock blocks (props_lib2/vegetation_models.rs)".into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_table_matches_the_wgsl() {
        let w = thin_wgsl();
        assert!(w.contains(&format!("const PR_VEG_BASE: u32 = {VEG_BASE}u;")));
        for (k, (a, b, g)) in THIN.iter().enumerate() {
            if *a > 0.0 {
                assert!(w.contains(&format!("case {k}u: {{ return vec3<f32>({a:?}, {b:?}, {g:?}); }}")), "{k}");
                assert!(a < b && *g >= 1.0 && *g <= 1.0 / THIN_FLOOR.sqrt());
            }
        }
        // the grass survivors grow at most a tenth (no long strokes).
        assert!(THIN[(V_GRASS - VEG_BASE) as usize].2 <= 1.1);
        assert_eq!(V_ROCK + V_ROCKS, VEG_BASE + COUNT);
    }

    #[test]
    fn rock_sits_partly_buried_and_is_closed() {
        let mut b = Builder::default();
        rock(&mut b, 0, 2, 7, true);
        let ys: Vec<f32> = b.v.iter().map(|v| v.pos[1]).collect();
        let (lo, hi) = ys.iter().fold((f32::MAX, f32::MIN), |a, y| (a.0.min(*y), a.1.max(*y)));
        assert!(lo < 0.0 && hi > 0.0);
        assert!((-lo / (hi - lo) - 0.2).abs() < 0.01);
        // every edge (by position: the corners are not shared) is shared by two triangles (a closed surface).
        let key = |i: u32| b.v[i as usize].pos.map(|c| (c * 1e5).round() as i64);
        let mut edges: std::collections::HashMap<([i64; 3], [i64; 3]), u32> = std::collections::HashMap::new();
        for t in b.opaque.chunks_exact(3) {
            for (a, c) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (ka, kc) = (key(a), key(c));
                *edges.entry((ka.min(kc), ka.max(kc))).or_default() += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2));
    }
}
