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
/// How many: the three first species and the hill parks' own (eo-props coastal.rs).
pub const V_TREES: u32 = eo_props::eztree::COASTAL_TREES.len() as u32;
/// Coastal shrubs (`COASTAL_SHRUBS` order).
pub const V_SHRUB: u32 = V_TREE + V_TREES;
/// A tall bunch of wild grass.
pub const V_GRASS: u32 = V_SHRUB + 2;
/// A short tuft (mown or grazed grass, dune grass).
pub const V_TUFT: u32 = V_GRASS + 1;
/// Wildflower clumps: yellow, violet.
pub const V_FLOWER: u32 = V_TUFT + 1;
/// Rock outcrop blocks (four shapes; 0 and 1 bedded).
pub const V_ROCK: u32 = V_FLOWER + 2;
pub const V_ROCKS: u32 = 4;
/// Meadow patches (`eo_landmarks::vegetation::MEADOW_SWARD` .. `MEADOW_THATCH`; the second grass pass).
pub const V_MEADOW: u32 = V_ROCK + V_ROCKS;
pub const V_MEADOWS: u32 = eo_landmarks::vegetation::MEADOW_TYPES as u32;
pub const COUNT: u32 = (V_MEADOW - VEG_BASE) + V_MEADOWS;

/// A material flag of our own: procedural rock grain in the colour pass.
pub const P_ROCK: u32 = 1 << 17;

/// Per model offset: (thin from m, draw distance m, the most a survivor
/// grows); 0 thin = never thins. The draw distances are the models' `lod.w`
/// (set below). The grass survivors grow little (1.1): a bunch grown by the
/// full 1/sqrt(share) turns into a long dark stroke on the bare ground tone,
/// where a shrub grown the same way still reads as a shrub.
pub const THIN: [(f32, f32, f32); COUNT as usize] = [
    // the coastal trees (every species: never thinned, 360 m)
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
    (0.0, 360.0, 1.0),
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
    // the meadow patches: their mats are the cover, so they keep every patch to 70 m
    // (the fine layer loads to 104 m) and the survivors grow a little to close the gaps.
    (70.0, 110.0, 1.3),
    (70.0, 110.0, 1.3),
    (70.0, 110.0, 1.3),
    (70.0, 110.0, 1.3),
    (70.0, 110.0, 1.3),
    (70.0, 110.0, 1.3),
    // the third pass's scrub mounds (their own volume: the survivors grow little)
    // and the bare ground's sparse tufts.
    (70.0, 110.0, 1.15),
    (70.0, 110.0, 1.15),
    (70.0, 110.0, 1.15),
    (70.0, 110.0, 1.3),
    // the flora pass's scrub species (eo_landmarks::flora).
    (70.0, 110.0, 1.15),
    (70.0, 110.0, 1.15),
    (70.0, 110.0, 1.15),
    (70.0, 110.0, 1.15),
    // the flora pass's seed stalks and serpentine bluff patches.
    (40.0, 90.0, 1.1),
    (70.0, 110.0, 1.3),
    (70.0, 110.0, 1.3),
    // the flora pass's mustard and blooms.
    (60.0, 110.0, 1.15),
    (70.0, 110.0, 1.1),
    // the city's annual grassland (`--no-city-grass`): the meadow patches' thinning.
    (70.0, 110.0, 1.3),
];
/// The share kept at the draw distance (thinned models).
pub const THIN_FLOOR: f32 = 0.12;

/// WGSL appended to props_types.wgsl: the vegetation range and its thinning
/// (the same numbers as `THIN`, so the cull, colour and shadow passes agree).
pub fn thin_wgsl() -> String {
    let mut s = format!("const PR_VEG_BASE: u32 = {VEG_BASE}u;\nconst PR_VEG_COUNT: u32 = {COUNT}u;\nconst PR_VEG_TREES: u32 = {}u;\nconst PR_VEG_GRASS: u32 = {}u;\nconst PR_VEG_ROCK: u32 = {}u;\nconst PR_VEG_MEADOW: u32 = {}u;\n", V_SHRUB - V_TREE, V_GRASS - V_TREE, V_ROCK - V_TREE, V_MEADOW - V_TREE);
    s += &format!("const PR_VEG_FLOOR: f32 = {THIN_FLOOR:?};\nconst PR_VEG_GRASS_CELL_MEAN: f32 = {GRASS_CELL_MEAN:?};\n");
    // the third grass pass's break-up (`--no-meadow-breakup`): props_types.wgsl
    // `pr_lod_dist`, props_draw.wgsl's lobed mat rims.
    s += &format!("const PR_MEADOW_BREAKUP: bool = {};\n", eo_landmarks::vegetation::grass3().breakup);
    // the flora pass's chert ground (`--no-chert-ground`): the bare soil mats' top and
    // face colours (linear) and its rubble, props_draw.wgsl.
    let lin = |c: [u8; 3]| c.map(|v| crate::scene::terrain_detail::srgb_lin(v) as f32);
    let (t, f) = (lin(eo_landmarks::flora::CHERT_TOP_SRGB), lin(eo_landmarks::flora::CHERT_FACE_SRGB));
    s += &format!("const PR_CHERT_GROUND: bool = {};\nconst PR_CHERT_TOP: vec3<f32> = vec3<f32>({:?}, {:?}, {:?});\nconst PR_CHERT_FACE: vec3<f32> = vec3<f32>({:?}, {:?}, {:?});\nconst PR_TIP_RIBBON: u32 = {}u;\n", eo_landmarks::flora::flora().chert, t[0], t[1], t[2], f[0], f[1], f[2], eo_landmarks::flora::TIP_RIBBON);
    s += &format!("const PR_TIP_SCALY: u32 = {}u;\n", eo_landmarks::flora::TIP_SCALY);
    // the scrub mounds' model range (props_draw.wgsl's leaf-cluster grain on them).
    // (the flora pass's species follow the bare soil type: a second range.)
    let mound = V_MEADOW - V_TREE + eo_landmarks::vegetation::MEADOW_COYOTE as u32;
    let species = V_MEADOW - V_TREE + eo_landmarks::flora::MEADOW_POISON_OAK as u32;
    // the city's annual canopy (`--no-city-grass`): its depth is in its vertex colours, so
    // props_draw.wgsl leaves out the blades' own clump occlusion on it.
    s += &format!("const PR_VEG_ANNUAL: u32 = {}u;\n", V_MEADOW - V_TREE + eo_landmarks::vegetation::MEADOW_ANNUAL as u32);
    // and its straw's transmission and the sky's light down in it (`ANNUAL_STRAW_T`, `ANNUAL_SKY_K`).
    s += &format!("const PR_ANNUAL_T: f32 = {ANNUAL_STRAW_T:?};\nfn pr_annual_sky(y: f32) -> f32 {{\n    let u = 1.0 - clamp((y - {:?}) / {ANNUAL_TOP_M:?}, 0.0, 1.0);\n    return exp(-{:?} * u * u);\n}}\n", MEADOW_GROUND - 0.01, ANNUAL_SKY_K * ANNUAL_PAI);
    s += &format!("const PR_VEG_MOUND: u32 = {mound}u;\nconst PR_VEG_MOUND_END: u32 = {}u;\nconst PR_VEG_SPECIES: u32 = {species}u;\nconst PR_VEG_SPECIES_END: u32 = {}u;\n", mound + 3, species + 4);
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
/// `blocky` (the second flora pass's sheared serpentinite, eo_landmarks::flora2
/// `bluff_rock`): fewer, deeper joint planes, so the block is an angular,
/// broken slab rather than a rounded knob.
fn rock(b: &mut Builder, m: u32, subdiv: u32, seed: u32, bedded: bool, blocky: bool) {
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
    // (blocky: 11 joints cut deep, 0.42-0.62 of the sphere: flat faces meet at sharp
    // edges all round; INFERRED from the guide's "blocky, sheared" bluff.)
    let (cuts, d0, dd) = if blocky { (11, 0.42, 0.2) } else { (CUTS, 0.62, 0.3) };
    let mut planes: Vec<([f32; 3], f32)> = (0..cuts)
        .map(|k| {
            let u = hash3(k as i32, 1, 3, seed) * 2.0 - 1.0;
            let a = hash3(k as i32, 2, 3, seed) * std::f32::consts::TAU;
            let r = (1.0 - u * u).max(0.0).sqrt();
            ([r * a.cos(), u, r * a.sin()], d0 + dd * hash3(k as i32, 4, 3, seed))
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
            rock(&mut out.b, m_rock, *sub, 501 + k * 17, k < 2, k >= 2 && eo_landmarks::flora2::flora2().bluff_rock);
            ids[l] = out.finish();
        }
        let model = V_ROCK + k;
        let (z, w) = thin(model);
        // the cascades see the near ones only (a block past 110 m casts a shadow under a texel).
        out.set(model, ids, [30.0, 110.0, z, w], 3, 0.0);
    }
    // --- the meadow patches (the second grass pass; drawn only where the app plants them).
    let g2 = eo_landmarks::vegetation::grass2();
    let pbr = if g2.blade_shading { P_BLADE_PBR } else { 0 };
    let thatch = eo_landmarks::vegetation::MEADOW_THATCH_ALBEDO.map(to_srgb8);
    let mats = MeadowMats {
        live: push_mat(out.materials, [255, 255, 255], 0.55, 0.0, P_MEADOW | P_MEADOW_LIVE | pbr, 0),
        dead: push_mat(out.materials, [255, 255, 255], 0.75, 0.0, P_MEADOW | pbr, 0),
        mat: push_mat(out.materials, thatch, 0.9, 0.0, P_MEADOW | P_MEADOW_LIVE | P_MEADOW_MAT, 0),
        flesh: push_mat(out.materials, [255, 255, 255], 0.35, 0.0, P_MEADOW | P_MEADOW_LIVE | P_MEADOW_THICK | pbr, 0),
    };
    let mound_mats = MoundMats {
        leaf: push_mat(out.materials, [255, 255, 255], 0.85, 0.0, P_MEADOW | P_MEADOW_LIVE, 0),
        flower: push_mat(out.materials, [255, 255, 255], 0.5, 0.0, P_MEADOW, 0),
    };
    // the bare ground's soil mat: the Tamalpais series' eroded horizon, dry 5YR 4/4
    // (terrain_detail.rs SOIL_ERODED_SRGB), drawn by the mat shader (its grain, a
    // little of the sward's green through it).
    let soil_mat = push_mat(out.materials, crate::scene::terrain_detail::SOIL_ERODED_SRGB, 0.95, 0.0, P_MEADOW | P_MEADOW_LIVE | P_MEADOW_MAT | P_MEADOW_SOIL, 0);
    // the flora pass's leaf sprigs: the atlas's sprig cells drawn grey (their luminance
    // over `SPRIG_LUMINANCE`, props_draw.wgsl) so the vertex colour is the species' leaf.
    let species = eo_landmarks::flora::flora().species;
    // the flora pass's blooms: white heads under the instance's flower colour (no blade optics).
    let m_bloom = push_mat(out.materials, [255, 255, 255], 0.6, 0.0, P_MEADOW | P_MEADOW_LIVE, 0);
    let m_sprig = if species {
        let m = push_mat(out.materials, [255, 255, 255], 0.6, 0.0, P_TEX | P_CUT | P_BIG | P_LEAF, 0);
        out.materials[m as usize].params[3] = 1.0 / SPRIG_LUMINANCE;
        m
    } else {
        NONE
    };
    for k in 0..V_MEADOWS {
        if k as u8 == eo_landmarks::vegetation::MEADOW_ANNUAL {
            // the city's annual grassland (`--no-city-grass`; drawn only where the app plants it).
            let mut ids = [NONE; 3];
            for (l, id) in ids.iter_mut().enumerate() {
                annual(&mut out.b, &mats, l as u32, 700 + k * 13);
                *id = out.finish();
            }
            let model = V_MEADOW + k;
            let (z, w) = thin(model);
            out.set(model, ids, [ANNUAL_LOD[0], ANNUAL_LOD[1], z, w], 0, 0.1);
            continue;
        }
        if k as u8 >= eo_landmarks::flora::MEADOW_SEED_STALKS {
            // the flora pass's grassland stalks and bluff scrub (drawn only where it plants them).
            let mut ids = [NONE; 3];
            for (l, id) in ids.iter_mut().enumerate() {
                if k as u8 == eo_landmarks::flora::MEADOW_MUSTARD {
                    mustard(&mut out.b, &mats, mound_mats.flower, l as u32, 700 + k * 13);
                } else if k as u8 == eo_landmarks::flora::MEADOW_BLOOM {
                    bloom(&mut out.b, m_bloom, l as u32, 700 + k * 13);
                } else if k as u8 == eo_landmarks::flora::MEADOW_SEED_STALKS {
                    seed_stalks(&mut out.b, &mats, l as u32, 700 + k * 13);
                } else {
                    bluff_patch(&mut out.b, &mats, k as u8, l as u32, 700 + k * 13);
                }
                *id = out.finish();
            }
            let model = V_MEADOW + k;
            let (z, w) = thin(model);
            // (the flowers hold their sprays farther: orange and yellow flecks to 40 m.)
            let edges = if k as u8 >= eo_landmarks::flora::MEADOW_MUSTARD { [12.0, 40.0] } else { [8.0, 24.0] };
            out.set(model, ids, [edges[0], edges[1], z, w], 0, 0.1);
            continue;
        }
        if k as u8 >= eo_landmarks::vegetation::MEADOW_COYOTE {
            let mut ids = [NONE; 3];
            for (l, id) in ids.iter_mut().enumerate() {
                if species && eo_landmarks::flora::is_shrub(k as u8) {
                    species_shrub(&mut out.b, &mound_mats, m_sprig, k as u8, l as u32, 700 + k * 13);
                } else if k as u8 == eo_landmarks::vegetation::MEADOW_BARE {
                    bare_tufts(&mut out.b, &mats, soil_mat, l as u32, 700 + k * 13);
                } else {
                    mound(&mut out.b, &mound_mats, k as u8, l as u32, 700 + k * 13);
                }
                *id = out.finish();
            }
            let model = V_MEADOW + k;
            let (z, w) = thin(model);
            if k as u8 == eo_landmarks::vegetation::MEADOW_BARE {
                out.set(model, ids, [8.0, 24.0, z, w], 0, 0.1);
            } else {
                // the mounds cast into the near cascade (the dark gaps between them).
                // (the species' sprigs hold their leaves farther: their levels change at 14 and 40 m.)
                let edges = if species && eo_landmarks::flora::is_shrub(k as u8) { [14.0, 40.0] } else { [10.0, 32.0] };
                out.set(model, ids, [edges[0], edges[1], z, w], 1, 0.1);
            }
            continue;
        }
        let mut ids = [NONE; 3];
        for (l, id) in ids.iter_mut().enumerate() {
            meadow(&mut out.b, &mats, k as u8, l as u32, 700 + k * 13);
            *id = out.finish();
        }
        let model = V_MEADOW + k;
        let (z, w) = thin(model);
        out.set(model, ids, [8.0, 24.0, z, w], 0, 0.1);
    }
    out.sources.push("vegetation: grass, tufts, wildflowers, rock blocks, meadow patches (props_lib2/vegetation_models.rs)".into());
}

// ------------------------------------------------------------------ scrub mounds (the third grass pass)

struct MoundMats {
    /// Leaves and leaf clumps: their colour times the instance's value.
    leaf: u32,
    /// Flowers: their own colour.
    flower: u32,
}

fn norm3(p: [f32; 3]) -> [f32; 3] {
    let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-6);
    p.map(|v| v / l)
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The mound's height and radius at scale 1 (m).
const MOUND_H: f32 = 1.0;
const MOUND_R: f32 = 0.8;

/// One leaf: a diamond `len` x `wid` at `p` facing `n` (turned about it by
/// `spin`), its colour `c` (linear).
#[allow(clippy::too_many_arguments)]
fn mound_leaf(b: &mut Builder, m: u32, p: [f32; 3], n: [f32; 3], spin: f32, len: f32, wid: f32, c: [f32; 3]) {
    let any = if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let t0 = norm3(cross3(n, any));
    let t1 = cross3(n, t0);
    let (ss, cs) = spin.sin_cos();
    let a = [0, 1, 2].map(|k| t0[k] * cs + t1[k] * ss);
    let w = [0, 1, 2].map(|k| -t0[k] * ss + t1[k] * cs);
    let col = lin3_to_srgb(c);
    // the leaf's own normal leans toward the clump's outward normal (a canopy of leaves shades as a surface).
    let first = b.v.len() as u32;
    for (f, g) in [(-0.5f32, 0.0f32), (0.0, -0.5), (0.5, 0.0), (0.0, 0.5)] {
        b.vert([p[0] + a[0] * f * len + w[0] * g * wid, p[1] + a[1] * f * len + w[1] * g * wid, p[2] + a[2] * f * len + w[2] * g * wid], n, [0.5 + g, 0.5 + f], col, m);
    }
    b.tri(false, first, first + 1, first + 2);
    b.tri(false, first, first + 2, first + 3);
}

/// A coastal scrub mound (`eo_landmarks::vegetation::MEADOW_COYOTE` ..
/// `MEADOW_LUPINE`) at level `lod`: a rounded mound 1 m tall and 1.6 m across
/// built from lumpy leaf clumps (displaced spheres), each clump's colour darker
/// toward the mound's inside and base (self-shading: the dark gaps between
/// clumps), small leaves standing out of the clumps' surfaces near and mid (they
/// break the silhouette), and for the lupine its yellow flower spikes clustered
/// on two or three branches. Vertex colours carry the type's leaf albedo
/// (`MEADOW_LIVE`) times the shading; the instance tint is its value.
fn mound(b: &mut Builder, mats: &MoundMats, kind: u8, lod: u32, seed: u32) {
    use eo_landmarks::vegetation as v;
    let live = v::MEADOW_LIVE[kind as usize];
    let sage = kind == v::MEADOW_SAGEBRUSH;
    let clumps = [12u32, 9, 6][lod as usize];
    let sub = [2u32, 1, 0][lod as usize];
    let (unit, faces) = icosphere(sub);
    let centre = [0.0f32, 0.3 * MOUND_H, 0.0];
    let mut balls: Vec<([f32; 3], f32)> = Vec::new();
    for k in 0..clumps {
        // the first clump the crown; the rest round it, lower toward the rim.
        let (c, r) = if k == 0 {
            ([0.0, 0.62 * MOUND_H, 0.0], 0.42)
        } else {
            let a = (k as f32 / (clumps - 1) as f32 + 0.3 * hash3(k as i32, 1, 7, seed)) * std::f32::consts::TAU;
            let u = 0.45 + 0.5 * hash3(k as i32, 2, 7, seed);
            let y = MOUND_H * (0.62 - 0.38 * u * u) * (0.85 + 0.3 * hash3(k as i32, 3, 7, seed));
            ([MOUND_R * u * a.cos(), y, MOUND_R * u * a.sin()], 0.26 + 0.14 * hash3(k as i32, 4, 7, seed))
        };
        // the far level's fewer clumps a little larger to keep the volume.
        // near and mid the leaves make the surface: the clump's core sits inside them.
        let r = r * [0.82f32, 0.92, 1.22][lod as usize];
        balls.push((c, r));
        let first = b.v.len() as u32;
        for p in &unit {
            // lumpy: two octaves of noise on the radius (leaf masses, not a smooth ball).
            let q = [p[0] * 2.2 + c[0] * 3.0, p[1] * 2.2 + c[1] * 3.0, p[2] * 2.2 + c[2] * 3.0];
            let rr = r * (1.0 + 0.24 * fbm(q, seed + k, 2));
            let mut pos = [c[0] + p[0] * rr, c[1] + p[1] * rr * 0.85, c[2] + p[2] * rr];
            pos[1] = pos[1].max(-0.02);
            let out_dir = norm3([pos[0] - centre[0], pos[1] - centre[1], pos[2] - centre[2]]);
            // the shading normal: the clump's own and the mound's, so each clump catches the
            // sun on its lit side and the mound still reads as one volume.
            let n = norm3([0, 1, 2].map(|i| 0.6 * p[i] + 0.4 * out_dir[i]));
            // self-shading: the clump's side facing into the mound and the mound's base dark.
            let facing = (p[0] * out_dir[0] + p[1] * out_dir[1] + p[2] * out_dir[2]).clamp(-1.0, 1.0);
            let ao = (0.3 + 0.7 * ((facing + 0.4) / 1.4).clamp(0.0, 1.0)) * (0.5 + 0.5 * (pos[1] / (0.75 * MOUND_H)).clamp(0.0, 1.0));
            // near and mid the core is the canopy's inside, seen only between the leaves: dark.
            let vary = (0.85 + 0.3 * hash3(k as i32, 5, 7, seed)) * [0.45f32, 0.6, 1.0][lod as usize];
            let c = [live[0] * ao * vary * 1.04, live[1] * ao * vary, live[2] * ao * vary * (1.15 - 0.15 * ao)];
            b.vert(pos, n, [0.5, 0.5], lin3_to_srgb(c), mats.leaf);
        }
        for f in &faces {
            b.tri(false, first + f[0], first + f[1], first + f[2]);
        }
    }
    // leaves standing out of the clumps' outer surfaces (none on the far level).
    let leaves = [3200u32, 900, 0][lod as usize];
    let size = [1.0f32, 1.45, 1.0][lod as usize];
    // a leaf cluster's size (coyote brush's 1-4 cm leaves crowd on short shoots: a
    // cluster reads as one 5-8 cm blade of colour from the eye).
    let (len, wid) = if sage { (0.06, 0.018) } else if kind == v::MEADOW_LUPINE { (0.065, 0.034) } else { (0.06, 0.03) };
    for k in 0..leaves {
        let (c, r) = balls[(hash3(k as i32, 11, 7, seed) * balls.len() as f32) as usize % balls.len()];
        let d = norm3([hash3(k as i32, 12, 7, seed) * 2.0 - 1.0, hash3(k as i32, 13, 7, seed) * 2.0 - 0.6, hash3(k as i32, 14, 7, seed) * 2.0 - 1.0]);
        let o = [1.08f32, 1.0, 1.0][lod as usize] * (0.95 + 0.3 * hash3(k as i32, 15, 7, seed));
        let p = [c[0] + d[0] * r * o, (c[1] + d[1] * r * 0.85 * o).max(0.02), c[2] + d[2] * r * o];
        let out_dir = norm3([p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]]);
        if d[0] * out_dir[0] + d[1] * out_dir[1] + d[2] * out_dir[2] < -0.1 {
            continue;
        }
        // a leaf's own tilt about the surface (+-40 degrees), its colour a little lighter
        // on the outside of the canopy (younger leaves) and varied.
        let tilt = [hash3(k as i32, 16, 7, seed) - 0.5, hash3(k as i32, 17, 7, seed) - 0.5, hash3(k as i32, 18, 7, seed) - 0.5];
        let n = norm3([d[0] + tilt[0] * 2.2, d[1] + tilt[1] * 2.2 + 0.2, d[2] + tilt[2] * 2.2]);
        // lighter toward the top and outside, every cluster its own value (some in the
        // canopy's shade, some young and yellower), so the surface reads as leaves.
        let r19 = hash3(k as i32, 19, 7, seed);
        let lift = (0.55 + 0.45 * (p[1] / MOUND_H).clamp(0.0, 1.0)) * (0.55 + 0.75 * r19);
        let young = (r19 - 0.7).max(0.0) * 1.5;
        let c = [live[0] * lift * (1.05 + young), live[1] * lift * 1.08, live[2] * lift * (1.0 - 0.5 * young)];
        let s = size * (0.75 + 0.5 * hash3(k as i32, 20, 7, seed));
        mound_leaf(b, mats.leaf, p, n, hash3(k as i32, 21, 7, seed) * std::f32::consts::TAU, len * s, wid * s, c);
    }
    if kind == v::MEADOW_LUPINE {
        // the flower spikes: racemes 12-22 cm tall standing out of the top of two or
        // three clumps, crowded (Lupinus arboreus' yellow form: clear sulphur yellow,
        // linear about 0.62/0.50/0.04, INFERRED from the colour words).
        let yellow = [0.62f32, 0.50, 0.045];
        let per = [6u32, 5, 4][lod as usize];
        for g in 0..3u32 {
            let (c, r) = balls[(1 + g * 3) as usize % balls.len()];
            for j in 0..per {
                let a = hash3((g * 16 + j) as i32, 31, 7, seed) * std::f32::consts::TAU;
                let u = 0.7 * hash3((g * 16 + j) as i32, 32, 7, seed).sqrt();
                let base = [c[0] + r * u * a.cos(), c[1] + r * (1.0 - 0.5 * u * u) * 0.85 - 0.02, c[2] + r * u * a.sin()];
                let h = 0.12 + 0.1 * hash3((g * 16 + j) as i32, 33, 7, seed);
                let col = [0, 1, 2].map(|i| yellow[i] * (0.85 + 0.3 * hash3((g * 16 + j) as i32, 34 + i as i32, 7, seed)));
                if lod == 0 {
                    // whorls of florets up the spike.
                    for f in 0..9u32 {
                        let t = f as f32 / 8.0;
                        let fa = f as f32 * 2.4 + a;
                        let rr = 0.022 * (1.0 - 0.6 * t);
                        let p = [base[0] + rr * fa.cos(), base[1] + h * t, base[2] + rr * fa.sin()];
                        let n = norm3([fa.cos(), 0.6, fa.sin()]);
                        mound_leaf(b, mats.flower, p, n, fa, 0.022, 0.016, col);
                    }
                } else {
                    // two crossed cards, the spike's outline.
                    for q in 0..2u32 {
                        let ya = a + q as f32 * std::f32::consts::FRAC_PI_2;
                        let n = [ya.cos(), 0.0, ya.sin()];
                        let p = [base[0], base[1] + h * 0.5, base[2]];
                        mound_leaf(b, mats.flower, p, n, std::f32::consts::FRAC_PI_2, h * 1.1, 0.05 * [1.0f32, 1.0, 1.4][lod as usize], col);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ the flora pass's scrub species

/// The vegetation atlas's sprig cells in the builder's UVs (rows top first:
/// u0, v top, u1, v bottom; `ez-vegetation-atlas-v1`, 768 px cells of the
/// 2048 px atlas, each sprig's stem at its cell's bottom edge, measured).
fn sprig_cell(s: eo_landmarks::flora::Sprig) -> [f32; 4] {
    use eo_landmarks::flora::Sprig;
    match s {
        Sprig::Round => [0.0, 0.0, 0.375, 0.375],
        Sprig::Fine => [0.375, 0.0, 0.75, 0.375],
        Sprig::Lobed => [0.0, 0.375, 0.375, 0.75],
        Sprig::Oval => [0.375, 0.375, 0.75, 0.75],
    }
}

/// The sprig cells' mean linear luminance over their opaque texels (measured
/// on `ez-vegetation-atlas-v1.png`: 0.17-0.19): the sprig material draws the
/// cell grey (its luminance over this), so the vertex colour is the leaf's albedo.
pub const SPRIG_LUMINANCE: f32 = 0.18;

/// One leaf sprig card: the cell `uv` on a quad `size` tall (`aspect` x as
/// wide) whose stem is at `p`, rising along `up`, its face across `side`;
/// shaded with `n` (the canopy's outward normal blended with the card's), the
/// base `dark` x the tip's colour `c` (linear). The tip sways a little in the wind.
#[allow(clippy::too_many_arguments)]
fn sprig(b: &mut Builder, m: u32, uv: [f32; 4], p: [f32; 3], up: [f32; 3], side: [f32; 3], size: f32, aspect: f32, n: [f32; 3], c: [f32; 3], dark: f32) {
    let w = size * aspect * 0.5;
    let base = b.v.len() as u32;
    let lo = lin3_to_srgb(c.map(|v| v * dark));
    let hi = lin3_to_srgb(c);
    let corners = [(-1.0f32, 0.0f32, uv[0], uv[3], lo), (1.0, 0.0, uv[2], uv[3], lo), (1.0, 1.0, uv[2], uv[1], hi), (-1.0, 1.0, uv[0], uv[1], hi)];
    for (sx, t, u, v, col) in corners {
        let q = [0, 1, 2].map(|k| p[k] + side[k] * sx * w + up[k] * t * size);
        let id = b.vert(q, n, [u, v], col, m);
        b.v[id as usize].pad = (0.25 * t).to_bits();
    }
    b.tri(true, base, base + 1, base + 2);
    b.tri(true, base, base + 2, base + 3);
}

/// A scrub shrub of its species (`eo_landmarks::flora::SPECIES`) at level
/// `lod`: the species' leaf masses (displaced spheres in its crown shape,
/// drawn as the canopy's shadowed inside: dark, seen between the sprigs) and
/// over them leaf sprig cards (`m_sprig`: the atlas sprig its leaf resembles,
/// drawn grey and coloured with the species' leaf albedo) standing out of the
/// masses' surfaces, lighter toward the top and outside, every sprig its own
/// value. The far level keeps fewer, larger sprigs so the silhouette stays
/// broken and the masses' grain (props_draw.wgsl) fills between them.
fn species_shrub(b: &mut Builder, mats: &MoundMats, m_sprig: u32, kind: u8, lod: u32, seed: u32) {
    use eo_landmarks::flora as fl;
    let sp = fl::species(kind).expect("a scrub species");
    let f = sp.form;
    let live = fl::leaf_linear(kind);
    let (unit, faces) = icosphere([2u32, 1, 0][lod as usize]);
    let centre = [0.0f32, 0.35 * f.h, 0.0];
    let clumps = (f.clumps * [3u32, 2, 1][lod as usize] / 3).max(4);
    let mut balls: Vec<([f32; 3], f32)> = Vec::new();
    for k in 0..clumps {
        let (c, r) = if k == 0 {
            ([0.0, 0.62 * f.h, 0.0], 0.42 * f.r)
        } else {
            let a = (k as f32 / (clumps - 1) as f32 + 0.3 * hash3(k as i32, 1, 9, seed)) * std::f32::consts::TAU;
            let u = 0.4 + 0.55 * hash3(k as i32, 2, 9, seed);
            let y = f.h * (0.64 - 0.4 * u * u) * (0.85 + 0.3 * hash3(k as i32, 3, 9, seed));
            ([f.r * u * a.cos(), y, f.r * u * a.sin()], f.r * (0.3 + 0.16 * hash3(k as i32, 4, 9, seed)))
        };
        let r = r * [0.8f32, 0.88, 1.05][lod as usize];
        balls.push((c, r));
        let first = b.v.len() as u32;
        for p in &unit {
            let q = [p[0] * 2.2 + c[0] * 3.0, p[1] * 2.2 + c[1] * 3.0, p[2] * 2.2 + c[2] * 3.0];
            let rr = r * (1.0 + 0.28 * fbm(q, seed + k, 2));
            let mut pos = [c[0] + p[0] * rr, c[1] + p[1] * rr * f.upright, c[2] + p[2] * rr];
            pos[1] = pos[1].max(-0.02);
            let out_dir = norm3([pos[0] - centre[0], pos[1] - centre[1], pos[2] - centre[2]]);
            let n = norm3([0, 1, 2].map(|i| 0.5 * p[i] + 0.5 * out_dir[i]));
            let facing = (p[0] * out_dir[0] + p[1] * out_dir[1] + p[2] * out_dir[2]).clamp(-1.0, 1.0);
            let ao = (0.35 + 0.65 * ((facing + 0.4) / 1.4).clamp(0.0, 1.0)) * (0.35 + 0.65 * (pos[1] / (0.75 * f.h)).clamp(0.0, 1.0));
            // the canopy's inside, seen between the sprigs: darker near, the far level's
            // masses closer to the canopy's mean (fewer sprigs over them).
            let vary = (0.85 + 0.3 * hash3(k as i32, 5, 9, seed)) * [0.28f32, 0.42, 0.75][lod as usize];
            let c = live.map(|v| v * ao * vary);
            b.vert(pos, n, [0.5, 0.5], lin3_to_srgb(c), mats.leaf);
        }
        for t in &faces {
            b.tri(false, first + t[0], first + t[1], first + t[2]);
        }
    }
    // the sprigs, each on a mass weighted by its surface.
    let uv = sprig_cell(f.sprig);
    let n_cards = f.cards[lod as usize];
    let size = f.card * [1.0f32, 1.7, 2.6][lod as usize];
    let area: f32 = balls.iter().map(|b| b.1 * b.1).sum();
    let mut k = 0u32;
    let mut placed = 0u32;
    while placed < n_cards && k < n_cards * 4 {
        k += 1;
        let mut pick = hash3(k as i32, 11, 9, seed) * area;
        let mut ball = balls[0];
        for bb in &balls {
            if pick < bb.1 * bb.1 {
                ball = *bb;
                break;
            }
            pick -= bb.1 * bb.1;
        }
        let (c, r) = ball;
        let d = norm3([hash3(k as i32, 12, 9, seed) * 2.0 - 1.0, hash3(k as i32, 13, 9, seed) * 2.0 - 0.8, hash3(k as i32, 14, 9, seed) * 2.0 - 1.0]);
        // rooted a little inside the mass (depth in the canopy) to just past its surface.
        let o = 0.7 + 0.35 * hash3(k as i32, 15, 9, seed);
        let p = [c[0] + d[0] * r * o, (c[1] + d[1] * r * f.upright * o).max(0.03), c[2] + d[2] * r * o];
        let out_dir = norm3([p[0] - centre[0], (p[1] - centre[1]) * 0.8, p[2] - centre[2]]);
        if d[0] * out_dir[0] + d[1] * out_dir[1] + d[2] * out_dir[2] < -0.15 {
            continue;
        }
        placed += 1;
        // the sprig rises out of the canopy: along the outward direction, turned up
        // (upright forms more), with its own lean.
        let lean = [hash3(k as i32, 16, 9, seed) - 0.5, hash3(k as i32, 17, 9, seed) - 0.5, hash3(k as i32, 18, 9, seed) - 0.5];
        let up = norm3([out_dir[0] * f.reach + lean[0] * 0.6, out_dir[1] * f.reach + 0.3 * f.upright + lean[1] * 0.4, out_dir[2] * f.reach + lean[2] * 0.6]);
        let spin = hash3(k as i32, 19, 9, seed) * std::f32::consts::TAU;
        let any = if up[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
        let t0 = norm3(cross3(up, any));
        let t1 = cross3(up, t0);
        let side = [0, 1, 2].map(|i| t0[i] * spin.cos() + t1[i] * spin.sin());
        let mut face = norm3(cross3(side, up));
        if face[0] * out_dir[0] + face[1] * out_dir[1] + face[2] * out_dir[2] < 0.0 {
            face = face.map(|v| -v);
        }
        // the shading normal: mostly the canopy's (the shrub reads as one lit volume,
        // its sunlit face brighter), a part the sprig's own (each catches the sun its way).
        let n = norm3([0, 1, 2].map(|i| 0.6 * out_dir[i] + 0.4 * face[i] + if i == 1 { 0.15 } else { 0.0 }));
        // lighter toward the top and outside (young growth), each sprig its own value.
        let r20 = hash3(k as i32, 20, 9, seed);
        let lift = (0.62 + 0.45 * (p[1] / f.h).clamp(0.0, 1.0)) * (0.72 + 0.5 * r20) * (0.8 + 0.25 * o);
        let young = (r20 - 0.75).max(0.0) * 1.2;
        let col = [live[0] * lift * (1.0 + young), live[1] * lift * (1.0 + 0.4 * young), live[2] * lift * (1.0 - 0.3 * young)];
        let s = size * (0.7 + 0.6 * hash3(k as i32, 21, 9, seed));
        sprig(b, m_sprig, uv, p, up, side, s, f.aspect, n, col, 0.62);
    }
    if kind == eo_landmarks::vegetation::MEADOW_LUPINE {
        lupine_spikes(b, mats, &balls, lod, seed);
    }
}

/// The bush lupine's flower spikes (the third pass's, `mound`): racemes 12-22 cm
/// tall out of the top of two or three masses.
fn lupine_spikes(b: &mut Builder, mats: &MoundMats, balls: &[([f32; 3], f32)], lod: u32, seed: u32) {
    let yellow = [0.62f32, 0.50, 0.045];
    let per = [6u32, 5, 4][lod as usize];
    for g in 0..3u32 {
        let (c, r) = balls[(1 + g * 3) as usize % balls.len()];
        for j in 0..per {
            let a = hash3((g * 16 + j) as i32, 31, 7, seed) * std::f32::consts::TAU;
            let u = 0.7 * hash3((g * 16 + j) as i32, 32, 7, seed).sqrt();
            let base = [c[0] + r * u * a.cos(), c[1] + r * (1.0 - 0.5 * u * u) * 0.85 - 0.02, c[2] + r * u * a.sin()];
            let h = 0.12 + 0.1 * hash3((g * 16 + j) as i32, 33, 7, seed);
            let col = [0, 1, 2].map(|i| yellow[i] * (0.85 + 0.3 * hash3((g * 16 + j) as i32, 34 + i as i32, 7, seed)));
            for q in 0..2u32 {
                let ya = a + q as f32 * std::f32::consts::FRAC_PI_2;
                let n = [ya.cos(), 0.0, ya.sin()];
                let p = [base[0], base[1] + h * 0.5, base[2]];
                mound_leaf(b, mats.flower, p, n, std::f32::consts::FRAC_PI_2, h * 1.1, 0.05, col);
            }
        }
    }
}

// ------------------------------------------------------------------ the flora pass's grassland and bluff

/// The annual grasses' seed stalks (`eo_landmarks::flora::MEADOW_SEED_STALKS`):
/// 5-9 culms 0.55-1.1 m (wild oats 0.5-1.2 m, ripgut brome 0.3-0.8 m, REF)
/// with nodding panicles of spikelets in their top quarter (oats' drooping
/// spikelets, the brome's long awns: one form), over no mat. Vertex colours
/// are the wild oats' green state; the instance tint carries the cure.
fn seed_stalks(b: &mut Builder, mats: &MeadowMats, lod: u32, seed: u32) {
    use eo_landmarks::vegetation as v;
    let live = v::MEADOW_LIVE[eo_landmarks::flora::MEADOW_SEED_STALKS as usize];
    let g = MEADOW_GROUND;
    let culms = [9u32, 6, 4][lod as usize];
    let wide = [1.0f32, 1.7, 3.0][lod as usize];
    for k in 0..culms {
        let (x, z) = disc(k, 0.35, seed + 3);
        let h = 0.55 + 0.55 * hash3(k as i32, 71, 3, seed);
        let yaw = hash3(k as i32, 72, 3, seed) * std::f32::consts::TAU;
        let reach = 0.06 + 0.14 * hash3(k as i32, 73, 3, seed);
        let c = vary(live, k, seed + 3);
        let cs = [5u32, 3, 2][lod as usize];
        blade(b, mats.live, [x, g - 0.01, z], yaw, h, 0.004 * wide, reach, 0.3, cs, [c[0] * 0.9, c[1] * 0.9, c[2] * 0.9], [c[0] * 1.2, c[1] * 1.12, c[2]], 1.0);
        let (sy, cy) = yaw.sin_cos();
        let spk = [9u32, 5, 2][lod as usize];
        for j in 0..spk {
            let t = 0.72 + 0.28 * j as f32 / spk.max(1) as f32;
            let out_d = reach * t * t.sqrt();
            let p = [x + cy * out_d, g - 0.01 + h * (t - 0.15 * t * t), z + sy * out_d];
            let a = hash3((k * 16 + j) as i32, 74, 3, seed) * std::f32::consts::TAU;
            let len = 0.03 * [1.0f32, 1.5, 2.2][lod as usize];
            // the spikelets a little paler and warmer than the culm (glumes and awns).
            let hc = vary([c[0] * 1.35, c[1] * 1.2, c[2] * 1.1], k * 16 + j, seed);
            blade(b, mats.live, p, a, len, 0.007 * [1.0f32, 1.5, 2.2][lod as usize], len * 0.7, 3.0, 2, hc, [hc[0] * 1.1, hc[1] * 1.05, hc[2]], 1.0);
        }
    }
}

/// A patch of the serpentine bluff's low scrub (`MEADOW_BUCKWHEAT`,
/// `MEADOW_BLUFF_HERBS`) over thin turf (the mat at a low green share: the
/// bare grey-green and brown ground shows through, field guide 1.3). Coast
/// buckwheat: two or three woolly cushions of oval leaves 2-4 cm (REF) to
/// about 0.3 m. The herbs: a seaside daisy mat (spoon-shaped leaves 3-10 cm,
/// REF), a Douglas iris fan (strap leaves 1-2 cm wide, REF, dark green
/// (40-70, 70-95, 40-60) estimate) and a dudleya rosette (chalky grey-green
/// (150-180, 165-185, 150-170) estimate) on the far side.
fn bluff_patch(b: &mut Builder, mats: &MeadowMats, kind: u8, lod: u32, seed: u32) {
    use eo_landmarks::vegetation as v;
    let live = v::MEADOW_LIVE[kind as usize];
    let g = MEADOW_GROUND;
    if !v::grass3().mats_free {
        mat(b, mats.mat, 0.9, 0.3, v::MEADOW_LIVE[v::MEADOW_FESCUE as usize], seed); // (`--no-mat-free-meadow`)
    }
    let f = [1.0f32, 0.4, 0.15][lod as usize];
    let n = |c: u32| ((c as f32 * f).round() as u32).max(1);
    let wide = [1.0f32, 1.6, 2.6][lod as usize];
    // a little red fescue through it: fine blue-green blades.
    sward(b, mats, n(90), 0.8, 0.08, 0.2, 0.004 * wide, if lod == 0 { 2 } else { 1 }, v::MEADOW_LIVE[v::MEADOW_FESCUE as usize], seed + 9);
    if kind == eo_landmarks::flora::MEADOW_BUCKWHEAT {
        for c in 0..3u32 {
            let (cx, cz) = disc(c, 0.45, seed + 11);
            let r = 0.18 + 0.1 * hash3(c as i32, 81, 3, seed);
            let leaves = n(70);
            for k in 0..leaves {
                // leaves round the cushion's dome, tilted to its surface.
                let a = hash3((c * 128 + k) as i32, 82, 3, seed) * std::f32::consts::TAU;
                let u = hash3((c * 128 + k) as i32, 83, 3, seed).sqrt();
                let base = [cx + r * 0.8 * u * a.cos(), g + r * 0.9 * (1.0 - u * u).sqrt() * 0.8, cz + r * 0.8 * u * a.sin()];
                let col = vary(live, c * 128 + k, seed);
                leaflet(b, mats.live, base, a, 0.035 * wide.sqrt(), 0.02 * wide.sqrt(), 0.25 + 0.9 * u, col);
            }
        }
    } else {
        // seaside daisy: a low mat of spoon leaves.
        let daisy = [0.105f32, 0.195, 0.105];
        let (dx, dz) = disc(0, 0.3, seed + 13);
        for k in 0..n(40) {
            let a = hash3(k as i32, 84, 3, seed) * std::f32::consts::TAU;
            let (x, z) = disc(k, 0.18, seed + 14);
            leaflet(b, mats.live, [dx + x, g, dz + z], a, 0.07 * wide.sqrt(), 0.025 * wide.sqrt(), 0.5 + 0.5 * hash3(k as i32, 85, 3, seed), vary(daisy, k, seed));
        }
        // Douglas iris: a fan of arching straps.
        let iris = [0.040f32, 0.085, 0.036];
        let (ix, iz) = disc(1, 0.4, seed + 15);
        let fan = hash3(1, 86, 3, seed) * std::f32::consts::TAU;
        for k in 0..n(14) {
            let a = fan + (hash3(k as i32, 87, 3, seed) - 0.5) * 1.2;
            let h = 0.25 + 0.25 * hash3(k as i32, 88, 3, seed);
            let c = vary(iris, k, seed);
            blade(b, mats.live, [ix, g - 0.01, iz], a, h, 0.014 * wide, h * 0.45, 0.9, if lod == 0 { 3 } else { 2 }, [c[0] * 0.8, c[1] * 0.8, c[2] * 0.8], c, 0.6);
        }
        // dudleya: a chalky rosette of thick leaves.
        let dud = [0.30f32, 0.36, 0.30];
        let (ux, uz) = disc(2, 0.5, seed + 16);
        for k in 0..n(14) {
            let a = k as f32 * 2.4;
            let c = vary(dud, k, seed);
            fleshy(b, mats.flesh, [ux, g - 0.01, uz], a, 0.04 + 0.03 * hash3(k as i32, 89, 3, seed), 0.08, 0.012 * wide.sqrt(), [c[0] * 0.85, c[1] * 0.85, c[2] * 0.85], c);
        }
    }
}

// ------------------------------------------------------------------ the flora pass's flowers

/// The overlook's annual mustard (`eo_landmarks::flora::MEADOW_MUSTARD`), about
/// 1 m at scale 1: a basal clump of broad leaves and leafy stems branching
/// into racemes of small pale-yellow flowers in sprays at the top
/// (`MUSTARD_FLOWER_SRGB`; the flowers their own colour, `m_flower`).
fn mustard(b: &mut Builder, mats: &MeadowMats, m_flower: u32, lod: u32, seed: u32) {
    use eo_landmarks::vegetation as v;
    let live = v::MEADOW_LIVE[eo_landmarks::flora::MEADOW_MUSTARD as usize];
    let g = MEADOW_GROUND;
    let wide = [1.0f32, 1.5, 2.2][lod as usize];
    // the basal leaves: broad, lobed (two leaflets each, offset), arching out.
    for k in 0..[34u32, 14, 6][lod as usize] {
        let a = hash3(k as i32, 91, 3, seed) * std::f32::consts::TAU;
        let (x, z) = disc(k, 0.12, seed + 21);
        let c = vary(live, k, seed + 21);
        let len = (0.14 + 0.1 * hash3(k as i32, 92, 3, seed)) * wide.sqrt();
        leaflet(b, mats.live, [x, g + 0.02, z], a, len, len * 0.5, 0.35 + 0.5 * hash3(k as i32, 93, 3, seed), c);
        leaflet(b, mats.live, [x + a.cos() * len * 0.5, g + 0.04 + len * 0.25, z + a.sin() * len * 0.5], a + 0.3, len * 0.6, len * 0.35, 0.5, vary(c, 1, seed));
    }
    // the stems, each with a few stem leaves and a spray of flowers at its top.
    let stems = [7u32, 5, 3][lod as usize];
    let col = eo_landmarks::flora::MUSTARD_FLOWER_SRGB;
    for k in 0..stems {
        let (x, z) = disc(k, 0.18, seed + 23);
        let yaw = hash3(k as i32, 94, 3, seed) * std::f32::consts::TAU;
        let h = 0.55 + 0.45 * hash3(k as i32, 95, 3, seed);
        let reach = 0.1 + 0.25 * hash3(k as i32, 96, 3, seed);
        let c = vary(live, k + 50, seed);
        blade(b, mats.live, [x, g - 0.01, z], yaw, h, 0.006 * wide, reach, 0.2, 3, c, c, 0.8);
        let (sy, cy) = yaw.sin_cos();
        let top = [x + cy * reach, g + h * 0.92, z + sy * reach];
        if lod < 2 {
            for j in 0..2 {
                let t = 0.35 + 0.25 * j as f32;
                let p = [x + cy * reach * t, g + h * t, z + sy * reach * t];
                leaflet(b, mats.live, p, yaw + 1.6 + 3.1 * j as f32, 0.09 * wide.sqrt(), 0.035 * wide.sqrt(), 0.3, vary(c, j + 7, seed));
            }
        }
        // the spray: racemes of heads round the stem's top, the open flowers above the buds.
        let n = [40u32, 16, 7][lod as usize];
        let base = b.v.len();
        heads(b, m_flower, n, 0.12, 0.0, 0.2, [0.016f32, 0.026, 0.042][lod as usize], col, seed + k * 7);
        for vtx in &mut b.v[base..] {
            vtx.pos[0] += top[0];
            vtx.pos[1] += top[1];
            vtx.pos[2] += top[2];
        }
    }
}

/// A bloom (`eo_landmarks::flora::MEADOW_BLOOM`): sprays of small flower heads
/// standing out of a host's crown (unit height 1, radius 0.6 at scale 1: the
/// planting scales it to its host), white (`BLOOM_WHITE`) under the instance
/// tint, which carries the species' flower colour.
fn bloom(b: &mut Builder, m: u32, lod: u32, seed: u32) {
    let w = (eo_landmarks::flora::BLOOM_WHITE.powf(1.0 / 2.2) * 255.0) as u8;
    let sprays = [9u32, 7, 5][lod as usize];
    for k in 0..sprays {
        let a = hash3(k as i32, 97, 3, seed) * std::f32::consts::TAU;
        let u = 0.25 + 0.75 * hash3(k as i32, 98, 3, seed).sqrt();
        let top = [0.6 * u * a.cos(), 0.95 - 0.35 * u * u, 0.6 * u * a.sin()];
        let n = [14u32, 6, 3][lod as usize];
        let base = b.v.len();
        heads(b, m, n, 0.07, -0.02, 0.06, [0.014f32, 0.024, 0.04][lod as usize], [w, w, w], seed + k * 5);
        for vtx in &mut b.v[base..] {
            vtx.pos[0] += top[0];
            vtx.pos[1] += top[1];
            vtx.pos[2] += top[2];
        }
    }
}

/// Bare eroded soil (`MEADOW_BARE`): a ragged soil mat (`soil`, the mat shader's
/// grain over the soil colour) with two small tufts of short blades and a few straws.
fn bare_tufts(b: &mut Builder, mats: &MeadowMats, soil: u32, lod: u32, seed: u32) {
    use eo_landmarks::vegetation as v;
    let live = v::MEADOW_LIVE[v::MEADOW_BARE as usize];
    if !v::grass3().mats_free {
        mat(b, soil, 0.95, 0.08, live, seed); // (`--no-mat-free-meadow`)
    }
    let wide = [1.0f32, 1.7, 3.0][lod as usize];
    // a tuft in about every other patch's metre (each patch two, the far level one).
    let tufts = [2u32, 2, 1][lod as usize];
    for t in 0..tufts {
        let (cx, cz) = disc(t, 0.7, seed + 3);
        let n = [26u32, 9, 4][lod as usize];
        for k in 0..n {
            let (x, z) = disc(k, 0.07, seed + 5 + t);
            let c = vary(live, k + t * 64, seed);
            let h = 0.06 + 0.14 * hash3((k + t * 64) as i32, 41, 9, seed);
            let yaw = hash3((k + t * 64) as i32, 42, 9, seed) * std::f32::consts::TAU;
            blade(b, mats.live, [cx + x, MEADOW_GROUND - 0.01, cz + z], yaw, h, 0.006 * wide, h * 0.35, 0.4, 1, [c[0] * 0.9, c[1] * 0.9, c[2] * 0.9], c, 0.6);
        }
    }
    if lod < 2 {
        thatch(b, mats, [8u32, 3, 0][lod as usize], 0.6, 0.15, 0.006 * wide, seed);
    }
}

// ------------------------------------------------------------------ the city's annual grassland

/// The annual canopy's levels' edges (m): the near level's fine stems to 6 m (a 3 mm
/// culm is under a pixel past about 4 m at 1920 x 1080 and a 60 degree field, so the
/// mid level's wider stems take over soon after), the mid level's to 24 m (the meadow
/// patches' own), the far level's strokes past them.
pub const ANNUAL_LOD: [f32; 2] = [6.0, 24.0];
/// A patch's disc (m): the cells are 1 m apart, so neighbours overlap.
const ANNUAL_R: f32 = 0.75;
/// The canopy's top in the model (m; the tallest culms' heads), what its light is
/// reckoned down from.
const ANNUAL_TOP_M: f32 = 0.8;
/// The light down in the canopy: Beer-Lambert in a plant stand, I/I0 = exp(-k L)
/// with L the plant area above (Monsi and Saeki 1953, Jpn. J. Bot. 14, 22-52), k
/// about 0.3-0.5 for the near-upright leaves and stems of grasses, and for leaves
/// that scatter, k sqrt(a) with a their absorptance (Campbell and Norman 1998, An
/// Introduction to Environmental Biophysics, 2nd ed., ch. 15), per colour band. k
/// 0.4 and a plant area index of 3.5 for a closed annual grass stand at seed set
/// (INFERRED inside those ranges); the straw's absorptance per band 1 - (r + t), with its
/// reflectance r the cured oats' (`MEADOW_CURED`, inside the dry grass's 0.15-0.30)
/// and its transmittance t taken as 0.8 r (INFERRED), so the light scattered down
/// through the straw is warmer than the sky's: the interior reads brown, not grey.
/// The area thins upward (its density falls linearly from the ground to the top),
/// so the area above height y is L (1 - y / top)^2: the light at the ground is
/// about 0.37, 0.34 and 0.29 of the top's in red, green and blue.
const ANNUAL_K: f32 = 0.4;
const ANNUAL_PAI: f32 = 3.5;
const ANNUAL_STRAW_RT: f32 = 1.8;
/// The straw's diffuse transmittance over its reflectance (as in [`canopy_light`];
/// INFERRED): props_draw.wgsl's light through a blade of this canopy, the straw's own
/// hue (a living blade's transmission is a saturated yellow-green: chlorophyll).
pub const ANNUAL_STRAW_T: f32 = 0.8;
/// The sky's diffuse light down in the canopy beyond what the vertex colours carry:
/// diffuse light is stopped faster than the high sun's beam in a canopy (Campbell and
/// Norman 1998, ch. 15), so the sky term (props_draw.wgsl's occlusion on this canopy,
/// `pr_annual_sky`) falls by exp(-k L (1 - y / top)^2) more, k 0.3 (INFERRED): the
/// lower canopy is lit by the light the straw passes down, not by the blue sky.
const ANNUAL_SKY_K: f32 = 0.3;

fn canopy_light(y: f32) -> [f32; 3] {
    let u = 1.0 - (y / ANNUAL_TOP_M).clamp(0.0, 1.0);
    let r = eo_landmarks::vegetation::MEADOW_CURED[eo_landmarks::vegetation::MEADOW_OATS as usize];
    r.map(|r| (-ANNUAL_K * (1.0 - ANNUAL_STRAW_RT * r).max(0.0).sqrt() * ANNUAL_PAI * u * u).exp())
}

/// How far a blade's shading normal turns up toward the canopy's (the vertical),
/// by height: a closed canopy is lit as a surface at its top, its blades' own
/// facing showing below (so the sunlit tops take the ground's light, not the
/// grazing light of an upright strip). The up weight over the blade's unit normal,
/// at the ground and at the top (INFERRED).
const ANNUAL_UP: (f32, f32) = (0.5, 2.2);

/// The wind's weight at a blade's tip (props_types.wgsl `pr_wind`: the displacement is
/// the weight times the wind's amplitude in metres, whatever the blade's length) for
/// the canopy's culms, low leaves, stem leaves and litter: dry culms are stiff and the
/// low leaves short, so a culm's tip moves a few centimetres in a breeze, not a
/// quarter metre (the meadow's 1.0 bent the 0.6 m culms' top segments flat); the
/// heads move with their culm's top (INFERRED).
const ANNUAL_FLEX: [f32; 4] = [0.35, 0.12, 0.25, 0.03];

/// One level of the annual canopy: (count, width m, segments) of the litter lying
/// on the ground, of the lower canopy's leaves and of the leaves up the stems,
/// (count, width m, segments, spikelets per panicle, head size over the near
/// level's) of the culms.
struct AnnualLevel {
    litter: (u32, f32, u32),
    low: (u32, f32, u32),
    leaf: (u32, f32, u32),
    culm: (u32, f32, u32, u32, f32),
}

/// The levels. Near: 360 culms a patch (a patch a square metre: 360 stems per m2,
/// inside the 200-600 tillers per m2 of a closed annual grass stand, INFERRED), each
/// 2.5-3.5 mm wide (the wild oat's hollow culm is 2-4 mm thick), most within 5-10
/// degrees of upright; a dense lower canopy of short narrow leaves (4.5 mm, the cured
/// leaves rolled); a few withered leaves bending from the stems; and only a short, sparse
/// litter, low and in the floor's light (barely seen between the stems). About 0.5 m2
/// seen side-on per patch, so the ground is mostly hidden past about 10 m from an eye
/// 1.7 m up. Mid: fewer, wider stems and leaves for about the same area, no litter;
/// far: the upper canopy only (the lower canopy and the litter are hidden behind it
/// at grazing angles). INFERRED.
const ANNUAL_LEVELS: [AnnualLevel; 3] = [
    AnnualLevel { litter: (150, 0.005, 1), low: (500, 0.0045, 1), leaf: (24, 0.004, 2), culm: (360, 0.003, 3, 5, 1.0) },
    AnnualLevel { litter: (0, 0.0, 1), low: (180, 0.012, 1), leaf: (30, 0.014, 1), culm: (100, 0.008, 2, 2, 1.6) },
    AnnualLevel { litter: (0, 0.0, 1), low: (0, 0.0, 1), leaf: (16, 0.07, 1), culm: (20, 0.035, 1, 0, 3.0) },
];

/// The city's cured annual grassland (`eo_landmarks::vegetation::MEADOW_ANNUAL`,
/// `--no-city-grass`) at level `lod`: a patch 1.5 m across of
/// * the lower canopy: the grasses' own cured basal leaves, curled and leaning (near
///   and mid levels; hidden behind the upper canopy from farther);
/// * leaves up the stems, arching over;
/// * culms 0.42-0.78 m (the planting's height factor makes them 0.36-0.94 m), most
///   with a head: the wild oats' open panicle of spikelets hanging on their
///   branches (`MEADOW_ANNUAL`'s sources), or a dense nodding spike (brome, barley
///   and ryegrass heads, one form), in the straw of the seed heads' glumes, paler
///   than the stems.
///
/// Each vertex's colour is the wild oats' green times [`canopy_light`] at its
/// height (lit at the top, dark down inside the canopy) and its normal turns up
/// toward the canopy's ([`ANNUAL_UP`]); the instance tint carries the cure.
/// Counts, widths and the forms' shares INFERRED ([`ANNUAL_LEVELS`]).
fn annual(b: &mut Builder, mats: &MeadowMats, lod: u32, seed: u32) {
    annual_with(b, mats, &ANNUAL_LEVELS[lod as usize], lod == 0, seed);
}

/// A cured blade's colour round `c`: value +-25 % (age, weathering, how it is
/// turned), a little warmer or cooler (+-7 %); `vary`'s yellow-to-blue-green swing
/// is a living sward's, and the cure's tint would carry it into saturated orange.
fn vary_dry(c: [f32; 3], k: u32, seed: u32) -> [f32; 3] {
    let v = 0.75 + 0.5 * hash3(k as i32, 21, 3, seed);
    let h = hash3(k as i32, 22, 3, seed) - 0.5;
    [c[0] * v * (1.0 + 0.15 * h), c[1] * v, c[2] * v * (1.0 - 0.15 * h)]
}

fn annual_with(b: &mut Builder, mats: &MeadowMats, lv: &AnnualLevel, near: bool, seed: u32) {
    use eo_landmarks::vegetation as v;
    let live = v::MEADOW_LIVE[v::MEADOW_ANNUAL as usize];
    let g = MEADOW_GROUND - 0.01;
    let first = b.v.len();
    let lit = |c: [f32; 3], y: f32| {
        let k = canopy_light(y);
        [c[0] * k[0], c[1] * k[1], c[2] * k[2]]
    };
    // the litter: last year's straw and this year's fallen leaves, short pieces lying
    // under the canopy, each its own colour between the weathered thatch and this year's
    // straw (the dead material: no seasonal tint; INFERRED), in the floor's light.
    let (n, w, segs) = lv.litter;
    let thatch = v::MEADOW_THATCH_ALBEDO;
    let fresh = v::MEADOW_CURED[v::MEADOW_OATS as usize];
    for k in 0..n {
        let (x, z) = disc(k, ANNUAL_R * 0.85, seed + 4);
        let c = lit(vary_dry(mix3(thatch, fresh, hash3(k as i32, 31, 9, seed)), k, seed + 4), 0.0);
        let len = 0.03 + 0.06 * hash3(k as i32, 32, 9, seed);
        let lift = 0.002 + 0.008 * hash3(k as i32, 33, 9, seed);
        let yaw = hash3(k as i32, 34, 9, seed) * std::f32::consts::TAU;
        let ww = w * (0.7 + 0.6 * hash3(k as i32, 35, 9, seed));
        blade(b, mats.dead, [x, MEADOW_GROUND + 0.002, z], yaw, lift, ww, len, 0.0, segs, [c[0] * 0.85, c[1] * 0.85, c[2] * 0.85], c, ANNUAL_FLEX[3]);
    }
    // the lower canopy: short narrow leaves, mostly upright.
    let (n, w, segs) = lv.low;
    for k in 0..n {
        let (x, z) = disc(k, ANNUAL_R, seed);
        let h = 0.05 + 0.17 * hash3(k as i32, 1, 9, seed);
        let reach = h * (0.15 + 0.35 * hash3(k as i32, 2, 9, seed));
        let droop = 0.2 + 0.7 * hash3(k as i32, 3, 9, seed);
        let yaw = hash3(k as i32, 4, 9, seed) * std::f32::consts::TAU;
        let c = vary_dry(live, k, seed);
        let ww = w * (0.7 + 0.6 * hash3(k as i32, 5, 9, seed));
        blade(b, mats.live, [x, g, z], yaw, h, ww, reach, droop, segs, lit(c, 0.0), lit(c, h * (1.0 - 0.5 * droop)), ANNUAL_FLEX[1]);
    }
    // a few withered leaves up the stems (at seed set the stand's leaves are mostly dead
    // at its base): narrow, rising then bending over, not lying out flat (INFERRED).
    let (n, w, segs) = lv.leaf;
    for k in 0..n {
        let (x, z) = disc(k, ANNUAL_R, seed + 1);
        let y0 = 0.03 + 0.27 * hash3(k as i32, 11, 9, seed).powf(1.5);
        let h = 0.12 + 0.13 * hash3(k as i32, 12, 9, seed);
        let reach = 0.03 + 0.07 * hash3(k as i32, 13, 9, seed);
        let droop = 0.3 + 0.6 * hash3(k as i32, 14, 9, seed);
        let yaw = hash3(k as i32, 15, 9, seed) * std::f32::consts::TAU;
        let c = vary_dry(live, k + 400, seed);
        let ww = w * (0.75 + 0.5 * hash3(k as i32, 16, 9, seed));
        blade(b, mats.live, [x, g + y0, z], yaw, h, ww, reach, droop, segs, lit(c, y0), lit(c, y0 + h * (1.0 - 0.5 * droop)), ANNUAL_FLEX[2]);
    }
    // the culms and their heads: most near upright (the lean's tangent 0.04-0.39, cubed
    // toward its low end: half within about 5 degrees, a tenth past 17; INFERRED).
    let (n, w, segs, spk, big) = lv.culm;
    // the glumes' pale straw in the green state (the tint takes it to the cured head's,
    // about (0.38, 0.32, 0.20): paler and less saturated than the stems; INFERRED).
    let head = [0.098, 0.148, 0.064];
    for k in 0..n {
        let (x, z) = disc(k, ANNUAL_R * 0.9, seed + 2);
        let h = 0.42 + 0.36 * hash3(k as i32, 21, 9, seed);
        let yaw = hash3(k as i32, 22, 9, seed) * std::f32::consts::TAU;
        let reach = h * (0.04 + 0.35 * hash3(k as i32, 23, 9, seed).powi(3));
        let droop = 0.05 + 0.2 * hash3(k as i32, 24, 9, seed);
        let c = vary_dry(live, k + 800, seed);
        let ww = w * (0.83 + 0.34 * hash3(k as i32, 29, 9, seed));
        let top = h * (1.0 - 0.5 * droop);
        blade(b, mats.live, [x, g, z], yaw, h, ww, reach, droop, segs, lit(c, 0.0), lit([c[0] * 1.08, c[1] * 1.05, c[2]], top), ANNUAL_FLEX[0]);
        // the heads sway with the culm's top: every vertex its weight there.
        let head_from = b.v.len();
        let (sy, cy) = yaw.sin_cos();
        let at = |t: f32| [x + cy * reach * t * t.sqrt(), g + h * (t - 0.5 * droop * t * t), z + sy * reach * t * t.sqrt()];
        let hc = vary_dry(head, k + 1200, seed);
        let form = hash3(k as i32, 25, 9, seed);
        let hs = if near { 2 } else { 1 };
        if form < 0.55 && spk > 0 {
            // the oat's open panicle: spikelets hanging from the culm's top fifth at the
            // ends of spreading branches 1.5-4.5 cm long (each one wedge pointing down and
            // a little out: a pendant; INFERRED from the source's "hang like pendants").
            for j in 0..spk {
                let t = 0.8 + 0.2 * j as f32 / spk as f32;
                let a = hash3((k * 16 + j) as i32, 26, 9, seed) * std::f32::consts::TAU;
                let br = 0.015 + 0.03 * hash3((k * 16 + j) as i32, 30, 9, seed);
                let p = at(t);
                let p = [p[0] + a.cos() * br, p[1] - 0.3 * br, p[2] + a.sin() * br];
                let len = (0.02 + 0.008 * hash3((k * 16 + j) as i32, 27, 9, seed)) * big;
                let c = vary_dry(hc, k * 16 + j, seed + 3);
                blade(b, mats.live, p, a, len, 0.005 * big, len * 0.3, 3.0, 1, c, [c[0] * 1.08, c[1] * 1.06, c[2] * 1.04], 1.0);
            }
        } else if form < 0.8 || spk == 0 {
            // a dense spike (brome, barley and ryegrass heads, one form; or, far, the head as
            // one stroke): continuing the culm's top, 4-8 cm, nodding a little (INFERRED).
            let len = (0.04 + 0.04 * hash3(k as i32, 27, 9, seed)) * big.min(1.6);
            blade(b, mats.live, at(0.97), yaw + 0.3 * (hash3(k as i32, 28, 9, seed) - 0.5), len, 0.006 * big.min(1.6), len * 0.25, 0.6, hs, hc, [hc[0] * 1.06, hc[1] * 1.04, hc[2] * 1.02], 1.0);
        }
        for vx in &mut b.v[head_from..] {
            vx.pad = (ANNUAL_FLEX[0] * 0.92f32.powf(1.5)).to_bits();
        }
    }
    // the shading normals turned up toward the canopy's, more toward its top.
    for vx in &mut b.v[first..] {
        let n = vx.normal.map(|c| c as f32 / 127.0);
        let t = ((vx.pos[1] - g) / ANNUAL_TOP_M).clamp(0.0, 1.0);
        let up = ANNUAL_UP.0 + (ANNUAL_UP.1 - ANNUAL_UP.0) * t;
        let m = [n[0], n[1] + up, n[2]];
        let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt().max(1e-6);
        vx.normal = [(m[0] / l * 127.0).round() as i8, (m[1] / l * 127.0).round() as i8, (m[2] / l * 127.0).round() as i8, 0];
    }
}

// ------------------------------------------------------------------ meadow patches

/// Material flags of the meadow (props_draw.wgsl `PR_MEADOW*`, `PR_BLADE_PBR`).
pub const P_MEADOW: u32 = 1 << 18;
/// Living tissue: takes the instance tint (its cure).
pub const P_MEADOW_LIVE: u32 = 1 << 19;
/// The patch's ground mat: matted thatch (the material's base colour) with the sward's green through it.
pub const P_MEADOW_MAT: u32 = 1 << 20;
/// The blades' leaf optics (`--no-blade-shading` leaves it out).
pub const P_BLADE_PBR: u32 = 1 << 21;
/// A fleshy leaf (ice plant): no keel or rolled margins, little transmission.
pub const P_MEADOW_THICK: u32 = 1 << 22;
/// A soil mat (the third pass's bare ground): soil grain and pebbles, not straw, and
/// its own small green share at every distance.
pub const P_MEADOW_SOIL: u32 = 1 << 23;

struct MeadowMats {
    live: u32,
    dead: u32,
    mat: u32,
    flesh: u32,
}

/// Where a meadow plant's ground is in the model (the instance stands
/// `GRASS_SINK_M` into the ground).
const MEADOW_GROUND: f32 = eo_landmarks::vegetation::GRASS_SINK_M as f32;

fn lin3_to_srgb(c: [f32; 3]) -> [u8; 3] {
    c.map(to_srgb8)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

/// One blade: a strip from `base` along a centre line that rises `h * (t -
/// droop t^2 / 2)` (droop over 1 arcs over and down) and reaches `reach` out
/// along `yaw` (t^1.5), `w` wide, parallel-sided then tapering to a point,
/// `segs` segments. The colour runs `c0` (base) to `c1` (tip), linear. UVs:
/// u across (0, 1; the midrib at 0.5), v along (0 base, 1 tip): the shader's
/// keel, margins and clump occlusion. The wind weight rises to the tip by `flex`.
#[allow(clippy::too_many_arguments)]
fn blade(b: &mut Builder, m: u32, base: [f32; 3], yaw: f32, h: f32, w: f32, reach: f32, droop: f32, segs: u32, c0: [f32; 3], c1: [f32; 3], flex: f32) {
    let (sy, cy) = yaw.sin_cos();
    let side = [-sy, 0.0, cy];
    let first = b.v.len() as u32;
    for i in 0..=segs {
        let t = i as f32 / segs as f32;
        let out_d = reach * t * t.sqrt();
        let p = [base[0] + cy * out_d, base[1] + h * (t - 0.5 * droop * t * t), base[2] + sy * out_d];
        let dy = h * (1.0 - droop * t);
        let dd = 1.5 * reach * t.sqrt();
        let tan = [cy * dd, dy, sy * dd];
        // the face normal (tangent x side), turned up a little so a sward shades as a canopy
        // but each blade still catches or loses the sun by how it is turned.
        let mut n = [tan[1] * side[2] - tan[2] * side[1], tan[2] * side[0] - tan[0] * side[2], tan[0] * side[1] - tan[1] * side[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        n = n.map(|v| v / l);
        if n[1] < 0.0 {
            n = n.map(|v| -v);
        }
        n[1] += 0.15;
        let c = lin3_to_srgb(mix3(c0, c1, t));
        let sway = (flex * t * t.sqrt()).to_bits();
        if i < segs {
            let hw = 0.5 * w * (1.0 - t.powf(2.2)).max(0.0).powf(0.6);
            let a = b.vert([p[0] - side[0] * hw, p[1], p[2] - side[2] * hw], n, [0.0, t], c, m);
            let bb = b.vert([p[0] + side[0] * hw, p[1], p[2] + side[2] * hw], n, [1.0, t], c, m);
            b.v[a as usize].pad = sway;
            b.v[bb as usize].pad = sway;
        } else {
            let tip = b.vert(p, n, [0.5, 1.0], c, m);
            b.v[tip as usize].pad = sway;
        }
    }
    for i in 0..segs {
        let a = first + 2 * i;
        if i + 1 < segs {
            b.tri(false, a, a + 1, a + 3);
            b.tri(false, a, a + 3, a + 2);
        } else {
            b.tri(false, a, a + 1, a + 2);
        }
    }
}

/// A fleshy three-sided leaf (ice plant): a tapered prism along a centre line
/// lying out `reach` along `yaw` and curving up `h`, radius `r` at its base.
#[allow(clippy::too_many_arguments)]
fn fleshy(b: &mut Builder, m: u32, base: [f32; 3], yaw: f32, h: f32, reach: f32, r: f32, c0: [f32; 3], c1: [f32; 3]) {
    let (sy, cy) = yaw.sin_cos();
    let dir = [cy, 0.0, sy];
    let side = [-sy, 0.0, cy];
    let first = b.v.len() as u32;
    const SEGS: u32 = 2;
    for i in 0..=SEGS {
        let t = i as f32 / SEGS as f32;
        let p = [base[0] + dir[0] * reach * t, base[1] + h * t * t, base[2] + dir[2] * reach * t];
        // red only toward the tip (the leaf's last quarter and its keel's end).
        let c = lin3_to_srgb(mix3(c0, c1, ((t - 0.55) / 0.45).clamp(0.0, 1.0).powf(1.5)));
        if i < SEGS {
            let rr = r * (1.0 - 0.45 * t);
            for k in 0..3 {
                let a = k as f32 / 3.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
                let (s, co) = a.sin_cos();
                let off = [side[0] * co + 0.0, s, side[2] * co];
                let v = b.vert([p[0] + off[0] * rr, p[1] + off[1] * rr, p[2] + off[2] * rr], off, [k as f32 / 2.0, t], c, m);
                b.v[v as usize].pad = (0.15 * t).to_bits();
            }
        } else {
            let v = b.vert(p, [dir[0], 0.4, dir[2]], [0.5, 1.0], c, m);
            b.v[v as usize].pad = 0.15f32.to_bits();
        }
    }
    for k in 0..3u32 {
        let k1 = (k + 1) % 3;
        b.tri(false, first + k, first + k1, first + 3 + k1);
        b.tri(false, first + k, first + 3 + k1, first + 3 + k);
        b.tri(false, first + 3 + k, first + 3 + k1, first + 6);
    }
}

/// A flat leaflet (clover, a forb leaf): a fan of five points, an oval `len`
/// long and `wid` wide from `base` along `yaw`, tilted up by `tilt` (rad).
#[allow(clippy::too_many_arguments)]
fn leaflet(b: &mut Builder, m: u32, base: [f32; 3], yaw: f32, len: f32, wid: f32, tilt: f32, c: [f32; 3]) {
    let (sy, cy) = yaw.sin_cos();
    let (st, ct) = tilt.sin_cos();
    let along = [cy * ct, st, sy * ct];
    let side = [-sy, 0.0, cy];
    let n = [-cy * st, ct, -sy * st];
    let col = lin3_to_srgb(c);
    let at = |f: f32, g: f32| [base[0] + along[0] * f + side[0] * g, base[1] + along[1] * f, base[2] + along[2] * f + side[2] * g];
    let first = b.v.len() as u32;
    // base, left, far, right, the midrib's middle: u across, v along (the shader's midrib).
    b.vert(at(0.0, 0.0), n, [0.5, 0.0], col, m);
    b.vert(at(0.45 * len, -0.5 * wid), n, [0.0, 0.45], col, m);
    b.vert(at(len, 0.0), n, [0.5, 1.0], col, m);
    b.vert(at(0.45 * len, 0.5 * wid), n, [1.0, 0.45], col, m);
    b.vert(at(0.5 * len, 0.0), n, [0.5, 0.5], col, m);
    for k in 0..5 {
        b.v[(first + k) as usize].pad = 0.05f32.to_bits();
    }
    for (i, j) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
        b.tri(false, first + 4, first + i, first + j);
    }
}

/// The patch's ground mat: a disc `r` across (ragged edge) a little over the
/// ground, slightly domed. UV v carries the sward's green share in the mat.
fn mat(b: &mut Builder, m: u32, r: f32, share: f32, c: [f32; 3], seed: u32) {
    const N: u32 = 12;
    let col = lin3_to_srgb(c);
    let y = MEADOW_GROUND + 0.015;
    let centre = b.vert([0.0, y + 0.01, 0.0], [0.0, 1.0, 0.0], [0.5, share], col, m);
    for k in 0..N {
        let a = k as f32 / N as f32 * std::f32::consts::TAU;
        let rr = r * (0.88 + 0.24 * hash3(k as i32, 1, 5, seed));
        b.vert([rr * a.cos(), y, rr * a.sin()], [0.0, 1.0, 0.0], [0.5, share], col, m);
    }
    for k in 0..N {
        b.tri(false, centre, centre + 1 + k, centre + 1 + (k + 1) % N);
    }
}

/// A point in a disc of radius `r` (k-th of a seeded sequence).
fn disc(k: u32, r: f32, seed: u32) -> (f32, f32) {
    let u = hash3(k as i32, 11, 3, seed);
    let a = hash3(k as i32, 12, 3, seed) * std::f32::consts::TAU;
    let rr = r * u.sqrt();
    (rr * a.cos(), rr * a.sin())
}

/// A blade colour round `c`: value +-30 % (a sward's blades differ by age,
/// water and how they are turned), from yellow-green to blue-green.
fn vary(c: [f32; 3], k: u32, seed: u32) -> [f32; 3] {
    let v = 0.7 + 0.6 * hash3(k as i32, 21, 3, seed);
    let h = hash3(k as i32, 22, 3, seed) - 0.5;
    [c[0] * v * (1.0 + 0.5 * h), c[1] * v, c[2] * v * (1.0 - 0.5 * h)]
}

/// Short sward: `n` blades in a disc of `r`, `h0..h1` tall, `w` wide; a third
/// with dry, paler tips.
#[allow(clippy::too_many_arguments)]
fn sward(b: &mut Builder, mats: &MeadowMats, n: u32, r: f32, h0: f32, h1: f32, w: f32, segs: u32, live: [f32; 3], seed: u32) {
    let straw = eo_landmarks::vegetation::MEADOW_THATCH_ALBEDO;
    for k in 0..n {
        let (x, z) = disc(k, r, seed);
        let h = h0 + (h1 - h0) * hash3(k as i32, 31, 3, seed);
        let c = vary(live, k, seed);
        let tip = if hash3(k as i32, 32, 3, seed) < 0.33 { mix3(c, straw, 0.45) } else { [c[0] * 1.06, c[1] * 1.06, c[2] * 1.0] };
        let yaw = hash3(k as i32, 33, 3, seed) * std::f32::consts::TAU;
        let reach = h * (0.1 + 0.35 * hash3(k as i32, 34, 3, seed));
        let droop = 0.1 + 0.45 * hash3(k as i32, 35, 3, seed);
        // a single tapered triangle (a 7 cm to 25 cm blade bends little), so a patch can carry hundreds.
        let _ = segs;
        blade(b, mats.live, [x, MEADOW_GROUND - 0.01, z], yaw, h, w * (0.8 + 0.4 * hash3(k as i32, 36, 3, seed)), reach, droop, 1, [c[0] * 0.92, c[1] * 0.92, c[2] * 0.95], tip, 0.6);
    }
}

/// Dead thatch: `n` straws lying matted on the ground (a few half raised).
fn thatch(b: &mut Builder, mats: &MeadowMats, n: u32, r: f32, len: f32, w: f32, seed: u32) {
    let straw = eo_landmarks::vegetation::MEADOW_THATCH_ALBEDO;
    // weathered thatch greys (INFERRED from the litter's look in the photographs).
    let grey = [0.14, 0.138, 0.125];
    for k in 0..n {
        let (x, z) = disc(k, r, seed + 1);
        let c = vary(mix3(straw, grey, hash3(k as i32, 41, 3, seed)), k, seed + 1);
        let l = len * (0.6 + 0.8 * hash3(k as i32, 42, 3, seed));
        let lift = 0.01 + 0.05 * hash3(k as i32, 43, 3, seed).powi(2);
        let yaw = hash3(k as i32, 44, 3, seed) * std::f32::consts::TAU;
        blade(b, mats.dead, [x, MEADOW_GROUND + 0.004, z], yaw, lift, w, l, 0.0, 2, [c[0] * 0.85, c[1] * 0.85, c[2] * 0.85], c, 0.1);
    }
}

/// One meadow patch model: `kind` (`eo_landmarks::vegetation::MEADOW_*`) at
/// level `lod` (0 near: the blades; 1 mid: fewer, wider; 2 far: the mat and
/// the tall types' silhouettes). The live colours are the type's green state
/// (`MEADOW_LIVE`); the instance tint carries its cure.
fn meadow(b: &mut Builder, mats: &MeadowMats, kind: u8, lod: u32, seed: u32) {
    use eo_landmarks::vegetation as v;
    let live = v::MEADOW_LIVE[kind as usize];
    let sward_live = v::MEADOW_LIVE[v::MEADOW_SWARD as usize];
    let g = MEADOW_GROUND;
    // (the mat's green share, then per level: sward blades, thatch straws)
    let share = match kind {
        v::MEADOW_THATCH => 0.3,
        v::MEADOW_OATS => 0.4,
        v::MEADOW_FESCUE => 0.35,
        v::MEADOW_CLOVER => 0.7,
        v::MEADOW_ICE_PLANT => 0.92,
        _ => 0.55,
    };
    if !v::grass3().mats_free {
        mat(b, mats.mat, 0.95, share, if kind == v::MEADOW_ICE_PLANT { live } else { sward_live }, seed); // (`--no-mat-free-meadow`)
    }
    let f = [1.0f32, 0.29, 0.04][lod as usize];
    let wide = [1.0f32, 1.7, 3.2][lod as usize];
    let segs = if lod == 0 { 2 } else { 1 };
    let n = |c: u32| ((c as f32 * f).round() as u32).max(if f > 0.0 { 1 } else { 0 });
    match kind {
        v::MEADOW_SWARD => {
            sward(b, mats, n(450), 0.85, 0.12, 0.34, 0.008 * wide, segs, live, seed);
            thatch(b, mats, n(40), 0.8, 0.22, 0.007 * wide, seed);
        }
        v::MEADOW_THATCH => {
            thatch(b, mats, n(110), 0.85, 0.26, 0.008 * wide, seed);
            sward(b, mats, n(220), 0.8, 0.08, 0.22, 0.007 * wide, segs, live, seed);
            // a few dead culms standing from last year.
            for k in 0..n(6) {
                let (x, z) = disc(k, 0.6, seed + 5);
                let c = vary(v::MEADOW_THATCH_ALBEDO, k, seed + 5);
                blade(b, mats.dead, [x, g - 0.01, z], hash3(k as i32, 51, 3, seed) * 6.283, 0.35 + 0.2 * hash3(k as i32, 52, 3, seed), 0.004 * wide, 0.08, 0.5, 2, c, c, 0.8);
            }
        }
        v::MEADOW_OATS => {
            sward(b, mats, n(260), 0.85, 0.1, 0.3, 0.008 * wide, segs, sward_live, seed);
            thatch(b, mats, n(35), 0.8, 0.24, 0.007 * wide, seed);
            let culms = [9u32, 6, 4][lod as usize];
            let straw_head = [0.105, 0.135, 0.05];
            for k in 0..culms {
                let (x, z) = disc(k, 0.55, seed + 7);
                let h = 0.6 + 0.35 * hash3(k as i32, 61, 3, seed);
                let yaw = hash3(k as i32, 62, 3, seed) * std::f32::consts::TAU;
                let reach = 0.05 + 0.12 * hash3(k as i32, 63, 3, seed);
                let c = vary(live, k, seed + 7);
                let cw = 0.0045 * [1.0f32, 1.5, 2.6][lod as usize];
                let cs = [4u32, 3, 2][lod as usize];
                blade(b, mats.live, [x, g - 0.01, z], yaw, h, cw, reach, 0.25, cs, [c[0] * 0.9, c[1] * 0.9, c[2] * 0.9], [c[0] * 1.15, c[1] * 1.1, c[2] * 1.0], 1.0);
                // the panicle: spikelets hanging from the culm's top quarter on short pedicels.
                let (sy, cy) = yaw.sin_cos();
                let spk = [7u32, 4, 0][lod as usize];
                for j in 0..spk {
                    let t = 0.74 + 0.26 * j as f32 / spk.max(1) as f32;
                    let out_d = reach * t * t.sqrt();
                    let p = [x + cy * out_d, g - 0.01 + h * (t - 0.125 * t * t), z + sy * out_d];
                    let a = hash3((k * 16 + j) as i32, 64, 3, seed) * std::f32::consts::TAU;
                    let len = 0.022 * [1.0f32, 1.6, 1.0][lod as usize];
                    let hc = vary(straw_head, k * 16 + j, seed);
                    // droop 3: out a little, then down (the oat's nodding spikelets).
                    blade(b, mats.live, p, a, len, 0.006 * [1.0f32, 1.6, 1.0][lod as usize], len * 0.7, 3.0, 2, hc, [hc[0] * 1.15, hc[1] * 1.1, hc[2]], 1.0);
                }
                // two arching leaves at the culm's foot.
                if lod < 2 {
                    for j in 0..2 {
                        let a = yaw + 2.0 + 2.2 * j as f32;
                        blade(b, mats.live, [x, g - 0.01, z], a, 0.22, 0.008 * wide, 0.12, 1.2, segs + 1, c, [c[0] * 1.05, c[1] * 1.05, c[2]], 0.6);
                    }
                }
            }
        }
        v::MEADOW_FESCUE => {
            sward(b, mats, n(160), 0.85, 0.08, 0.24, 0.007 * wide, segs, sward_live, seed);
            thatch(b, mats, n(40), 0.8, 0.2, 0.007 * wide, seed);
            // one or two tussocks: fine blades in a fountain from a tight crown, a quarter of them dead straw inside.
            for t in 0..2u32 {
                let (cx, cz) = if t == 0 { (0.0, 0.0) } else { disc(3, 0.45, seed + 9) };
                let sz = if t == 0 { 1.0 } else { 0.7 };
                let nb = [55u32, 22, 9][lod as usize];
                for k in 0..nb {
                    let (x, z) = disc(k, 0.06 * sz, seed + 11 + t);
                    let yaw = hash3(k as i32, 71, 3, seed + t) * std::f32::consts::TAU;
                    let dead = hash3(k as i32, 72, 3, seed + t) < 0.25;
                    let c = if dead { vary(v::MEADOW_THATCH_ALBEDO, k, seed) } else { vary(live, k, seed + t) };
                    let h = sz * (0.45 + 0.3 * hash3(k as i32, 73, 3, seed + t));
                    let droop = 1.0 + 0.5 * hash3(k as i32, 74, 3, seed + t);
                    let reach = sz * (0.12 + 0.2 * hash3(k as i32, 75, 3, seed + t));
                    let w = 0.0028 * [1.0f32, 2.0, 3.2][lod as usize];
                    blade(b, if dead { mats.dead } else { mats.live }, [cx + x, g - 0.01, cz + z], yaw, h, w, reach, droop, [3u32, 2, 2][lod as usize], [c[0] * 0.9, c[1] * 0.9, c[2] * 0.92], [c[0] * 1.08, c[1] * 1.06, c[2] * 1.04], 0.7);
                }
            }
        }
        v::MEADOW_CLOVER => {
            sward(b, mats, n(220), 0.85, 0.07, 0.2, 0.007 * wide, segs, sward_live, seed);
            thatch(b, mats, n(20), 0.8, 0.2, 0.007 * wide, seed);
            let leaves = [26u32, 10, 0][lod as usize];
            let big = [1.0f32, 1.7, 1.0][lod as usize];
            for k in 0..leaves {
                let (x, z) = disc(k, 0.72, seed + 13);
                let h = 0.04 + 0.09 * hash3(k as i32, 81, 3, seed);
                let c = vary(live, k, seed + 13);
                if hash3(k as i32, 82, 3, seed) < 0.75 {
                    // a trifoliate clover leaf on its petiole.
                    blade(b, mats.live, [x, g - 0.01, z], hash3(k as i32, 83, 3, seed) * 6.283, h + 0.01, 0.0018 * big, 0.01, 0.0, 1, c, c, 0.3);
                    let a0 = hash3(k as i32, 84, 3, seed) * std::f32::consts::TAU;
                    for j in 0..3 {
                        leaflet(b, mats.live, [x, g - 0.01 + h, z], a0 + j as f32 * 2.094, 0.018 * big, 0.016 * big, 0.25, c);
                    }
                } else {
                    // a forb rosette leaf (plantain, filaree): broad, lanceolate, lying low.
                    blade(b, mats.live, [x, g - 0.005, z], hash3(k as i32, 85, 3, seed) * 6.283, 0.035, 0.03 * big, 0.11, 0.6, segs + 1, [c[0] * 0.95, c[1] * 1.0, c[2] * 0.95], c, 0.2);
                }
            }
        }
        v::MEADOW_ICE_PLANT => {
            let leaves = [80u32, 28, 0][lod as usize];
            let big = [1.0f32, 1.5, 1.0][lod as usize];
            let red = [0.22, 0.05, 0.035];
            for k in 0..leaves {
                let (x, z) = disc(k, 0.78, seed + 17);
                let c = vary(live, k, seed + 17);
                let tip = mix3(c, red, 0.55 + 0.45 * hash3(k as i32, 91, 3, seed));
                fleshy(b, mats.flesh, [x, g + 0.005, z], hash3(k as i32, 92, 3, seed) * 6.283, 0.03 + 0.05 * hash3(k as i32, 93, 3, seed), (0.06 + 0.05 * hash3(k as i32, 94, 3, seed)) * big, 0.0065 * big, c, tip);
            }
        }
        _ => {}
    }
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
        assert_eq!(V_ROCK + V_ROCKS, V_MEADOW);
        assert_eq!(V_MEADOW + V_MEADOWS, VEG_BASE + COUNT);
    }

    /// The meadow patches' triangles per level (the cost the frame pays per
    /// instance): dense near, a few hundred mid, the mat and a few strokes far.
    #[test]
    fn meadow_patches_are_dense_near_and_light_far() {
        let mats = MeadowMats { live: 0, dead: 1, mat: 2, flesh: 3 };
        // (the third pass's mounds and bare soil have their own test)
        for k in 0..eo_landmarks::vegetation::MEADOW_COYOTE {
            let mut tris = [0usize; 3];
            for (l, t) in tris.iter_mut().enumerate() {
                let mut b = Builder::default();
                meadow(&mut b, &mats, k, l as u32, 700);
                assert!(b.cut.is_empty());
                *t = b.opaque.len() / 3;
                // every blade stands on the mat's disc (no stray vertex far off).
                assert!(b.v.iter().all(|v| v.pos[0].hypot(v.pos[2]) < 1.4 && v.pos[1] > -0.05 && v.pos[1] < 1.2), "{k} {l}");
            }
            eprintln!("meadow {k}: triangles near {} mid {} far {}", tris[0], tris[1], tris[2]);
            assert!(tris[0] > tris[1] && tris[1] > tris[2], "{k}: {tris:?}");
            assert!(tris[0] <= 1400 && tris[1] <= 450 && tris[2] <= 80, "{k}: {tris:?}");
        }
    }

    /// The city's annual canopy: its triangles per level (the near level dense, the far a
    /// few strokes), its culms in the cited height range, its blades inside the patch's
    /// disc, and the area it shows side-on kept across the levels (no thinning band).
    #[test]
    fn the_annual_canopy_is_dense_near_and_keeps_its_cover() {
        let mats = MeadowMats { live: 0, dead: 1, mat: 2, flesh: 3 };
        let mut tris = [0usize; 3];
        let mut area = [0f32; 3];
        for (l, t) in tris.iter_mut().enumerate() {
            let mut b = Builder::default();
            annual(&mut b, &mats, l as u32, 947);
            assert!(b.cut.is_empty());
            *t = b.opaque.len() / 3;
            assert!(b.v.iter().all(|v| v.pos[0].hypot(v.pos[2]) < 1.0 && v.pos[1] > -0.05 && v.pos[1] < 0.95), "{l}");
            // the area seen from the side (x), each triangle's projection on the y-z plane.
            for tri in b.opaque.chunks_exact(3) {
                let [p, q, r] = [tri[0], tri[1], tri[2]].map(|i| b.v[i as usize].pos);
                let (u, w) = ([q[1] - p[1], q[2] - p[2]], [r[1] - p[1], r[2] - p[2]]);
                area[l] += 0.5 * (u[0] * w[1] - u[1] * w[0]).abs();
            }
            // the heads stand at the culms' tops: the tallest vertex is over 0.6 m.
            assert!(b.v.iter().any(|v| v.pos[1] > 0.6), "{l}");
        }
        eprintln!("annual canopy: triangles {tris:?}, side area {area:?} m2");
        // each layer's share of the near level's side area (the tuning record).
        for (name, lv) in [("litter", AnnualLevel { low: (0, 0.0, 1), leaf: (0, 0.0, 1), culm: (0, 0.0, 1, 0, 1.0), ..ANNUAL_LEVELS[0] }), ("low", AnnualLevel { litter: (0, 0.0, 1), leaf: (0, 0.0, 1), culm: (0, 0.0, 1, 0, 1.0), ..ANNUAL_LEVELS[0] }), ("leaf", AnnualLevel { litter: (0, 0.0, 1), low: (0, 0.0, 1), culm: (0, 0.0, 1, 0, 1.0), ..ANNUAL_LEVELS[0] }), ("culm", AnnualLevel { litter: (0, 0.0, 1), low: (0, 0.0, 1), leaf: (0, 0.0, 1), ..ANNUAL_LEVELS[0] })] {
            let mut b = Builder::default();
            annual_with(&mut b, &mats, &lv, true, 947);
            let a: f32 = b.opaque.chunks_exact(3).map(|tri| {
                let [p, q, r] = [tri[0], tri[1], tri[2]].map(|i| b.v[i as usize].pos);
                let (u, w) = ([q[1] - p[1], q[2] - p[2]], [r[1] - p[1], r[2] - p[2]]);
                0.5 * (u[0] * w[1] - u[1] * w[0]).abs()
            }).sum();
            eprintln!("annual canopy near level: {name} {a:.3} m2, {} triangles", b.opaque.len() / 3);
        }
        assert!(tris[0] > tris[1] && tris[1] > tris[2], "{tris:?}");
        assert!(tris[0] <= 4000 && tris[1] <= 700 && tris[2] <= 60, "{tris:?}");
        assert!(area[0] > 0.35 && area[1] > 0.6 * area[0] && area[2] > 0.3 * area[0], "{area:?}");
        // the canopy is darker inside: the light at the ground is about half the top's.
        let (floor, top) = (canopy_light(0.0), canopy_light(ANNUAL_TOP_M));
        assert!(floor[0] > floor[1] && floor[1] > floor[2] && floor[2] > 0.25 && floor[0] < 0.45, "{floor:?}");
        assert_eq!(top, [1.0; 3]);
    }

    #[test]
    fn rock_sits_partly_buried_and_is_closed() {
        let mut b = Builder::default();
        rock(&mut b, 0, 2, 7, true, false);
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

    #[test]
    fn scrub_mounds_are_dense_near_and_light_far() {
        let mats = MoundMats { leaf: 0, flower: 1 };
        for k in [eo_landmarks::vegetation::MEADOW_COYOTE, eo_landmarks::vegetation::MEADOW_SAGEBRUSH, eo_landmarks::vegetation::MEADOW_LUPINE] {
            let tris: Vec<usize> = (0..3)
                .map(|l| {
                    let mut b = Builder::default();
                    mound(&mut b, &mats, k, l, 9);
                    assert!(b.v.iter().all(|v| v.pos[1] >= -0.03 && v.pos[1] < 1.4 * MOUND_H));
                    b.opaque.len() / 3
                })
                .collect();
            assert!(tris[0] < 12_000 && tris[1] < 3_000 && tris[2] < 400, "{k}: {tris:?}");
            assert!(tris[0] > tris[1] && tris[1] > tris[2]);
        }
    }

}
