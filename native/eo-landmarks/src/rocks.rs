//! The shore's rock: boulders on the bare rock, at the cliff toes and on the
//! steep faces over the sea (`--no-rocks`), and the armour-stone revetment
//! against the Fort Point and Marine Drive seawall (`--no-riprap`). One
//! system: procedural stones, size-graded, each an icosphere cut by a few
//! fracture planes (quarried or broken faces) and roughened, lit with the
//! CC0 `seaside_rock` and `granite_wall` scans (eo-render cc0.rs
//! `structure_class`). No new asset.
//!
//! Placement reads only the city's own data and the drawn ground:
//!
//! - mapped bare rock (OSM `natural=bare_rock` greens, the game's `bareRock`
//!   ground: Marshall's Beach's serpentinite), its foreshore and the sea in
//!   front of it to the low water;
//! - the cliff toes: beach sand within a few metres of ground steeper than
//!   sand holds (eo-app ground_tone `SAND_SLOPE_MAX`, mirrored here);
//! - ground over the sea steeper than 45 degrees (rise over run 1) within the
//!   sand's reach of the shoreline: outcrops, set deep into the slope;
//! - the quay wall's edges (waterfront.rs) on the Fort Point and Marine
//!   Drive shore, where the NPS placed boulders on the seawall's seaward
//!   side when it rebuilt it (the research sheet's [S2]).
//!
//! Every size is a published or stated dimension scaled by the ground plan
//! (`eo_data::groundscale::scale()`, 1.25: the stones lie on ground drawn at
//! that scale), and the tide heights by the height scale.

use crate::geometry::{MeshArrays, Rgb, V3};
use std::sync::OnceLock;

/// The armour stone's median diameter, metres (real size). INFERRED: Hudson's
/// formula (USACE EM 1110-2-1614, Design of Coastal Revetments, Seawalls and
/// Bulkheads, 1995, eq. 2-21), W50 = rho H^3 / (K_D (rho/rho_w - 1)^3 cot a),
/// for a 2 m design wave on the strait's shore (INFERRED: no wave record was
/// read), quarrystone of 2650 kg/m3 in sea water (1025 kg/m3), K_D 2 (rough
/// angular stone, two layers, breaking waves) and a 1:2 slope: W50 = 1.34 t,
/// D50 = (W50 / rho)^(1/3) = 0.80 m.
pub const ARMOUR_D50_M: f64 = 0.80;
/// The armour's gradation: 0.75 W50 to 1.25 W50 (EM 1110-2-1614's
/// recommended band), as diameters (the cube roots).
pub const ARMOUR_GRADATION: [f64; 2] = [0.909, 1.077];
/// The revetment's slope, horizontal over vertical (EM 1110-2-1614: armour
/// slopes of 1V:1.5H to 1V:3H; 1:2 taken, as in the D50 above).
pub const REVETMENT_SLOPE_H: f64 = 2.0;
/// The armour's crest over mean sea level, real metres: the station's mean
/// higher high water (0.829 m, NOAA 9414290) plus the design wave (INFERRED).
pub const ARMOUR_CREST_OVER_MSL_M: f64 = 0.829 + 2.0;
/// The armour's toe under mean sea level, real metres (one D50 under the
/// lowest tide's reach, INFERRED; the station's MLLW is 0.95 m under MSL).
pub const ARMOUR_TOE_UNDER_MSL_M: f64 = 1.8;
/// The Fort Point and Marine Drive revetment: quay wall edges whose middle
/// lies in this box (surveyed metres in the city frame: west, north, east,
/// south), from the fort's north-west breakwater (37.81389 N, 122.47787 W,
/// x -5144) east past the Warming Hut (37.80832 N, 122.47073 W, x -4516),
/// the shore Marine Drive runs along; the only shore in the box.
pub const REVETMENT_BOX_SURVEYED: [f64; 4] = [-5200.0, -4000.0, -4480.0, -3300.0];

/// The stones' albedo: the mean linear albedo of the CC0 scan each draws
/// with (Poly Haven `seaside_rock` and `granite_wall`, 2k diffuse maps under
/// `EO_CC0_ROOT`, provenance in native/assets/cc0-manifest.json), so the
/// scan's detail sits on its own measured tone: the boulders' dark sea-worn
/// rock (luminance 0.056) and the armour's quarried granite (0.129).
pub const BOULDER_ALBEDO: Rgb = [0.0637, 0.0546, 0.0403];
pub const ARMOUR_ALBEDO: Rgb = [0.1747, 0.1215, 0.0715];
/// A stone's tone from a uniform `u`: its scan's mean, 0.85 to 1.15 of it.
pub fn stone_tone(base: Rgb, u: f64) -> Rgb {
    base.map(|v| v * (0.85 + 0.3 * u))
}

/// Boulders: diameters (real metres) from `BOULDER_D_M[0]` to `[1]` by a
/// power law of exponent `BOULDER_POWER` (number of stones over D falling as
/// D^-2: a talus and boulder-beach grading; INFERRED, no survey of the beach's
/// stones was read).
pub const BOULDER_D_M: [f64; 2] = [0.45, 3.2];
pub const BOULDER_POWER: f64 = 2.0;
/// The steepest ground sand holds (eo-app ground_tone `SAND_SLOPE_MAX`).
pub const SAND_SLOPE_MAX: f64 = 0.36;
/// A face steeper than 45 degrees.
pub const OUTCROP_SLOPE: f64 = 1.0;
/// The lattice the boulders are drawn on, drawn metres.
pub const LATTICE_M: f64 = 3.0;
/// Chance of a boulder per lattice cell, by zone: on and in front of bare
/// rock, at a cliff toe, on a steep face (INFERRED densities).
pub const DENSITY_BARE: f64 = 0.30;
pub const DENSITY_TOE: f64 = 0.22;
pub const DENSITY_OUTCROP: f64 = 0.06;
/// How far around mapped bare rock its boulders spread, and how far inland
/// of the shoreline the rocks reach at most (`BEACH_SAND_REACH_M`), drawn metres.
pub const BARE_SPREAD_M: f64 = 24.0;
pub const INLAND_REACH_M: f64 = 150.0;
/// The deepest a boulder stands in the sea (its foot), drawn metres.
pub const SEA_FOOT_M: f64 = 1.2;
/// A stone over this diameter gets the finer sphere (320 triangles, else 80).
pub const FINE_D_M: f64 = 1.9;
/// Mapped bare rock carries boulders only on ground a boulder rests on (no
/// steeper than this, rise over run: about 31 degrees, a talus's repose).
pub const BOULDER_REST_SLOPE: f64 = 0.6;
/// The share of the bare rock's boulders that lie on the dry sand above the
/// strand line (the rest in the intertidal and the shallows, where the waves
/// have left them), and the scale of their clusters (drawn metres).
pub const DRY_SHARE: f64 = 0.25;
pub const CLUSTER_M: f64 = 22.0;
/// The armour is built down to this far under the still sea (the stones under
/// it are hidden by the water), drawn metres.
pub const ARMOUR_VISIBLE_UNDER_M: f64 = 0.5;

/// One stone: centre, semi-axes, orientation, seed and tone.
#[derive(Clone, Copy, Debug)]
pub struct Stone {
    pub c: V3,
    pub r: V3,
    pub yaw: f64,
    pub tilt: [f64; 2],
    pub seed: u32,
    pub tone: Rgb,
    /// Quarried (more and flatter cuts) or a weathered boulder.
    pub angular: bool,
    /// The bed it lies in (a point on it and its upward unit normal: the
    /// ground, or the revetment's face): the faces wholly under it are not
    /// built (buried), and the stone is occluded toward it.
    pub bed: (V3, V3),
}

/// The occlusion a stone takes toward its bed (an ambient-occlusion stand-in
/// baked in the vertex tone: the voids between stones and the contact with
/// the ground stay dark under the sky): the darkest a face turned into the
/// bed reads, and the height over the bed (in stone diameters) the crevice's
/// darkening reaches.
pub const BED_OCCLUSION_MIN: f64 = 0.35;
pub const CREVICE_REACH_D: f64 = 0.35;

pub fn hash(a: i64, b: i64, salt: u32) -> f64 {
    let mut h = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (salt as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn norm(a: V3) -> V3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A unit icosphere at `level` subdivisions (vertices, triangles).
pub fn icosphere(level: usize) -> &'static (Vec<V3>, Vec<[u32; 3]>) {
    static L: [OnceLock<(Vec<V3>, Vec<[u32; 3]>)>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    L[level.min(2)].get_or_init(|| {
        let t = (1.0 + 5f64.sqrt()) / 2.0;
        let mut v: Vec<V3> = [[-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0], [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t], [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0]].iter().map(|p| norm(*p)).collect();
        let mut f: Vec<[u32; 3]> = vec![[0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8], [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1]];
        for _ in 0..level.min(2) {
            let mut mid = std::collections::HashMap::new();
            let mut nf = Vec::with_capacity(f.len() * 4);
            let mut m = |a: u32, b: u32, v: &mut Vec<V3>| -> u32 {
                let k = (a.min(b), a.max(b));
                *mid.entry(k).or_insert_with(|| {
                    let (p, q) = (v[a as usize], v[b as usize]);
                    v.push(norm([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0, (p[2] + q[2]) / 2.0]));
                    (v.len() - 1) as u32
                })
            };
            for t in &f {
                let (a, b, c) = (m(t[0], t[1], &mut v), m(t[1], t[2], &mut v), m(t[2], t[0], &mut v));
                nf.extend_from_slice(&[[t[0], a, c], [t[1], b, a], [t[2], c, b], [a, b, c]]);
            }
            f = nf;
        }
        (v, f)
    })
}

/// Smooth value noise on a point (unit-ish scale), [-1, 1].
fn noise3(p: V3, seed: u32) -> f64 {
    let (ix, iy, iz) = (p[0].floor(), p[1].floor(), p[2].floor());
    let (fx, fy, fz) = (p[0] - ix, p[1] - iy, p[2] - iz);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (ux, uy, uz) = (s(fx), s(fy), s(fz));
    let h = |dx: i64, dy: i64, dz: i64| hash(ix as i64 + dx + (iz as i64 + dz) * 7919, iy as i64 + dy, seed) * 2.0 - 1.0;
    let l = |a: f64, b: f64, t: f64| a + (b - a) * t;
    l(
        l(l(h(0, 0, 0), h(1, 0, 0), ux), l(h(0, 1, 0), h(1, 1, 0), ux), uy),
        l(l(h(0, 0, 1), h(1, 0, 1), ux), l(h(0, 1, 1), h(1, 1, 1), ux), uy),
        uz,
    )
}

/// The stone's tone at a world height: the dry tone; under the higher high
/// water the seawall's algae stain (shore_surface.wgsl `lm_seawall`, 0.62,
/// 0.67, 0.55); under the high water wet too, at the measured wet ratio of a
/// rough mineral surface (Angstrom's sand, ground_cover `WET_SAND_RATIO`).
pub fn tide_tone(tone: Rgb, y: f64, sea: f64, mhhw: f64, mhw: f64, rag: f64) -> Rgb {
    let ss = |e0: f64, e1: f64, x: f64| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let h = y - sea + rag;
    let stain = ss(mhhw + 0.1, mhhw - 0.1, h);
    let wet = ss(mhw + 0.1, mhw - 0.2, h);
    let k = crate::ground_cover::WET_SAND_RATIO;
    [0.62, 0.67, 0.55].iter().enumerate().fold(tone, |mut c, (i, s)| {
        c[i] *= (1.0 + (s - 1.0) * stain) * (1.0 + (k - 1.0) * wet);
        c
    })
}

/// Append one stone to `m`: an icosphere scaled to the semi-axes, cut by
/// fracture planes, roughened, turned and placed; vertex tones through the tide.
pub fn stone_mesh(m: &mut MeshArrays, s: &Stone, sea: f64, tide: [f64; 2]) {
    let d = 2.0 * s.r[0].max(s.r[1]).max(s.r[2]);
    let (verts, tris) = icosphere(if d > FINE_D_M { 2 } else { 1 });
    let lumpy = if s.angular { 0.08 } else { 0.2 };
    let h = |k: u32| hash(s.seed as i64, k as i64, 991);
    // fracture planes (in the stone's own frame): a quarried stone has five
    // flat faces cut deep, a boulder three shallow ones.
    let cuts: Vec<(V3, f64)> = (0..if s.angular { 6 } else { 4 })
        .map(|i| {
            let n = norm([h(i * 3) * 2.0 - 1.0, h(i * 3 + 1) * 2.0 - 1.0, h(i * 3 + 2) * 2.0 - 1.0]);
            (n, if s.angular { 0.5 + 0.25 * h(40 + i) } else { 0.62 + 0.28 * h(40 + i) })
        })
        .collect();
    let (cy, sy) = (s.yaw.cos(), s.yaw.sin());
    let (ca, sa) = (s.tilt[0].cos(), s.tilt[0].sin());
    let (cb, sb) = (s.tilt[1].cos(), s.tilt[1].sin());
    let rot = |p: V3| -> V3 {
        // tilt about x, then z, then yaw about y
        let p = [p[0], p[1] * ca - p[2] * sa, p[1] * sa + p[2] * ca];
        let p = [p[0] * cb - p[1] * sb, p[0] * sb + p[1] * cb, p[2]];
        [p[0] * cy + p[2] * sy, p[1], -p[0] * sy + p[2] * cy]
    };
    let rough = 0.05;
    // each vertex cut back onto the deepest fracture plane it crosses: that
    // face's normal (so the cut reads flat, its edge sharp), else the smooth one
    let mut cut_n: Vec<Option<V3>> = vec![None; verts.len()];
    let local: Vec<V3> = verts
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let mut q = *v;
            for (n, off) in &cuts {
                let e = dot(q, *n) - off;
                if e > 0.0 {
                    q = [q[0] - n[0] * e, q[1] - n[1] * e, q[2] - n[2] * e];
                    cut_n[i] = Some(*n);
                }
            }
            // lumps over the uncut skin, a finer roughness everywhere
            let lump = if cut_n[i].is_some() { 0.0 } else { noise3([q[0] * 1.3 + s.seed as f64 * 0.37, q[1] * 1.3, q[2] * 1.3], s.seed) };
            let fine = noise3([q[0] * 4.7, q[1] * 4.7 + s.seed as f64 * 0.11, q[2] * 4.7], s.seed ^ 7);
            let k = 1.0 + lumpy * lump + rough * fine;
            [q[0] * s.r[0] * k, q[1] * s.r[1] * k, q[2] * s.r[2] * k]
        })
        .collect();
    // smooth normals from the faces (area-weighted)
    let mut nrm = vec![[0.0f64; 3]; local.len()];
    for t in tris {
        let (a, b, c) = (local[t[0] as usize], local[t[1] as usize], local[t[2] as usize]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        for i in t {
            for k in 0..3 {
                nrm[*i as usize][k] += n[k];
            }
        }
    }
    for (i, c) in cut_n.iter().enumerate() {
        if let Some(n) = c {
            // the plane's normal through the stone's axes (the inverse scale), mostly
            let pn = norm([n[0] / s.r[0], n[1] / s.r[1], n[2] / s.r[2]]);
            let sn = norm(nrm[i]);
            nrm[i] = norm([pn[0] * 0.8 + sn[0] * 0.2, pn[1] * 0.8 + sn[1] * 0.2, pn[2] * 0.8 + sn[2] * 0.2]);
        }
    }
    let (bp, bn) = s.bed;
    let world: Vec<V3> = local.iter().map(|p| {
        let w = rot(*p);
        [s.c[0] + w[0], s.c[1] + w[1], s.c[2] + w[2]]
    }).collect();
    let over = |w: V3| dot([w[0] - bp[0], w[1] - bp[1], w[2] - bp[2]], bn);
    // only the faces with a corner over the bed (less a margin) are built
    let kept: Vec<&[u32; 3]> = tris.iter().filter(|t| t.iter().any(|i| over(world[*i as usize]) > -0.03 * d)).collect();
    let mut remap = vec![u32::MAX; world.len()];
    let ss = |e0: f64, e1: f64, x: f64| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let rag = (hash(s.seed as i64, 3, 5) - 0.5) * 0.25;
    for t in &kept {
        for i in t.iter() {
            let i = *i as usize;
            if remap[i] != u32::MAX {
                continue;
            }
            let w = world[i];
            let n = norm(rot(nrm[i]));
            let jitter = 0.94 + 0.12 * hash(s.seed as i64, i as i64, 17);
            // toward the bed: faces turned into it, and the crevice just over it
            let facing = 0.5 + 0.5 * dot(n, bn);
            let ao = (BED_OCCLUSION_MIN + (1.0 - BED_OCCLUSION_MIN) * facing) * (BED_OCCLUSION_MIN + (1.0 - BED_OCCLUSION_MIN) * ss(0.0, CREVICE_REACH_D * d, over(w)));
            let c = tide_tone(s.tone.map(|v| v * jitter * ao), w[1], sea, tide[0], tide[1], rag);
            remap[i] = m.vertex(w, n, c);
        }
    }
    for t in kept {
        m.tri_idx(remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]);
    }
}

/// The dark bed under a revetment's armour (the voids between the stones and
/// their underlayer, in their shade): a strip on the revetment's face, under
/// the stones, in the stone tone at the bed occlusion's floor.
pub fn armour_bed(m: &mut MeshArrays, edges: &[([f64; 2], [f64; 2], [f64; 2])], crest_at: &dyn Fn(f64, f64) -> f64, sea: f64, plan: f64, hs: f64, tone: Rgb) {
    let d50 = ARMOUR_D50_M * plan;
    let top = sea + ARMOUR_CREST_OVER_MSL_M * hs;
    let toe = sea - (ARMOUR_TOE_UNDER_MSL_M * hs).min(ARMOUR_VISIBLE_UNDER_M);
    let c = tone.map(|v| v * BED_OCCLUSION_MIN * BED_OCCLUSION_MIN);
    for (a, b, land) in edges {
        let crest = crest_at((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        let y0 = top.min(crest - d50) - 0.2 * d50;
        let w = (y0 - toe) * REVETMENT_SLOPE_H + 0.5 * d50;
        let o = [-land[0], -land[1]];
        let p = |e: [f64; 2], off: f64| -> V3 { [e[0] + o[0] * off, y0 - (off / REVETMENT_SLOPE_H).max(0.0), e[1] + o[1] * off] };
        let n = norm([o[0], REVETMENT_SLOPE_H, o[1]]);
        let (i0, i1, i2, i3) = (m.vertex(p(*a, 0.0), n, c), m.vertex(p(*b, 0.0), n, c), m.vertex(p(*b, w), n, c), m.vertex(p(*a, w), n, c));
        // facing up and out (counter-clockwise seen from above the face)
        let (u, v) = ([b[0] - a[0], 0.0, b[1] - a[1]], [o[0], 0.0, o[1]]);
        let up = u[2] * v[0] - u[0] * v[2]; // y of u x v
        if up > 0.0 {
            m.tri_idx(i0, i1, i2);
            m.tri_idx(i0, i2, i3);
        } else {
            m.tri_idx(i0, i2, i1);
            m.tri_idx(i0, i3, i2);
        }
    }
}

/// The armour stones of a revetment along `edges` (each a wall edge's two
/// ends and its landward unit normal), on a 1:`REVETMENT_SLOPE_H` face from
/// the wall's foot out into the sea: rows of stones from the armour's crest
/// (never over the wall's crest less a stone) down to its toe. `plan`: the
/// ground-plan scale; `hs`: the height scale.
pub fn revetment(edges: &[([f64; 2], [f64; 2], [f64; 2])], crest_at: &dyn Fn(f64, f64) -> f64, sea: f64, plan: f64, hs: f64, tone: &dyn Fn(f64) -> Rgb) -> Vec<Stone> {
    let d50 = ARMOUR_D50_M * plan;
    let top = sea + ARMOUR_CREST_OVER_MSL_M * hs;
    let toe = sea - (ARMOUR_TOE_UNDER_MSL_M * hs).min(ARMOUR_VISIBLE_UNDER_M);
    let mut out = Vec::new();
    for (ei, (a, b, land)) in edges.iter().enumerate() {
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len < 0.1 {
            continue;
        }
        let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let out_n = [-land[0], -land[1]];
        let crest = crest_at((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        let start_y = top.min(crest - d50);
        let rows = (((start_y - toe) * REVETMENT_SLOPE_H) / (0.85 * d50)).ceil().max(1.0) as usize;
        for row in 0..rows {
            let off = 0.45 * d50 + row as f64 * 0.85 * d50;
            let y_face = start_y - off / REVETMENT_SLOPE_H;
            let n_along = (len / (0.9 * d50)).ceil().max(1.0) as usize;
            for k in 0..n_along {
                let seed = hash(ei as i64 * 131 + row as i64, k as i64, 7);
                let t = ((k as f64 + 0.5 + (seed - 0.5) * 0.5 + if row % 2 == 1 { 0.5 } else { 0.0 }) / n_along as f64).clamp(0.0, 1.0);
                let g = ARMOUR_GRADATION[0] + (ARMOUR_GRADATION[1] - ARMOUR_GRADATION[0]) * hash(ei as i64, (row * 1000 + k) as i64, 11);
                let d = d50 * g;
                let jit = (hash(ei as i64, (row * 1000 + k) as i64, 13) - 0.5) * 0.3 * d50;
                let x = a[0] + u[0] * len * t + out_n[0] * (off + jit);
                let z = a[1] + u[1] * len * t + out_n[1] * (off + jit);
                let s = (seed * 4.0e9) as u32;
                let bed_n = norm([out_n[0], REVETMENT_SLOPE_H, out_n[1]]);
                out.push(Stone {
                    bed: ([x, y_face - 0.3 * d, z], bed_n),
                    c: [x, y_face - 0.15 * d, z],
                    // quarried blocks: as long as wide or up to 1.6 times, the shortest axis 0.6-0.9 of the middle
                    r: [d * (0.5 + 0.15 * hash(s as i64, 7, 3)), d * (0.3 + 0.15 * hash(s as i64, 1, 3)), d * (0.38 + 0.12 * hash(s as i64, 2, 3))],
                    yaw: hash(s as i64, 3, 3) * std::f64::consts::TAU,
                    tilt: [(hash(s as i64, 4, 3) - 0.5) * 1.2, (hash(s as i64, 5, 3) - 0.5) * 1.2],
                    seed: s,
                    tone: tone(hash(s as i64, 6, 3)),
                    angular: true,
                });
            }
        }
    }
    out
}

/// Whether a quay wall edge's middle (drawn) is on the Fort Point and Marine
/// Drive revetment.
pub fn on_revetment(mid: [f64; 2], plan: f64) -> bool {
    let b = REVETMENT_BOX_SURVEYED.map(|v| v * plan);
    mid[0] >= b[0] && mid[0] <= b[2] && mid[1] >= b[1] && mid[1] <= b[3]
}

/// Where boulders lie: the zone a lattice point is in, from what the caller
/// reads there. `s`: signed shore distance (positive over the sea);
/// `ground`: the drawn ground over the sea; `slope`: rise over run there;
/// `bare`: on or within `BARE_SPREAD_M` of mapped bare rock; `toe`: beach
/// sand within a few metres of ground steeper than sand holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zone {
    Bare,
    Toe,
    Outcrop,
}

pub fn zone(s: f64, ground_over_sea: f64, slope: f64, bare: bool, toe: bool) -> Option<Zone> {
    if s < -INLAND_REACH_M || ground_over_sea < -SEA_FOOT_M {
        return None;
    }
    if bare && s < BARE_SPREAD_M && slope <= BOULDER_REST_SLOPE {
        return Some(Zone::Bare);
    }
    if s > 0.0 {
        return None;
    }
    if toe && slope <= SAND_SLOPE_MAX {
        return Some(Zone::Toe);
    }
    if slope > OUTCROP_SLOPE {
        return Some(Zone::Outcrop);
    }
    None
}

/// A boulder's diameter (real metres) from a uniform `u`: the power law
/// between `BOULDER_D_M` (inverse transform of a truncated Pareto).
pub fn boulder_d(u: f64) -> f64 {
    let (a, b, k) = (BOULDER_D_M[0], BOULDER_D_M[1], BOULDER_POWER);
    let (fa, fb) = (a.powf(-k), b.powf(-k));
    (fa - u * (fa - fb)).powf(-1.0 / k)
}

/// One boulder at lattice cell (i, j), if the cell's hash keeps it for its
/// zone; `ground_min`: the lowest drawn ground under its footprint (centre and
/// four points at its radius); `down`: the slope's downhill direction.
/// `dry`: the ground stands over the strand line (the higher high water).
pub fn boulder(i: i64, j: i64, x: f64, z: f64, zone: Zone, dry: bool, ground_min: &dyn Fn(f64, f64, f64) -> f64, plan: f64, tone: &dyn Fn(f64) -> Rgb) -> Option<Stone> {
    let cluster = (noise3([x / CLUSTER_M, 0.5, z / CLUSTER_M], 23) * 0.5 + 0.5) * 1.6;
    let p = match zone {
        Zone::Bare => DENSITY_BARE * if dry { DRY_SHARE } else { 1.0 } * cluster,
        Zone::Toe => DENSITY_TOE * cluster,
        Zone::Outcrop => DENSITY_OUTCROP,
    };
    if hash(i, j, 101) >= p {
        return None;
    }
    let mut d = boulder_d(hash(i, j, 102)) * plan;
    if zone == Zone::Outcrop {
        d = d.max(2.0 * plan);
    }
    let x = x + (hash(i, j, 103) - 0.5) * LATTICE_M * 0.8;
    let z = z + (hash(i, j, 104) - 0.5) * LATTICE_M * 0.8;
    let flat = if zone == Zone::Outcrop { 0.4 } else { 0.55 + 0.3 * hash(i, j, 105) };
    let r = [d * 0.5, d * 0.5 * flat, d * (0.4 + 0.15 * hash(i, j, 106))];
    // sunk into the ground: a third of its height, an outcrop half
    let sink = if zone == Zone::Outcrop { 0.55 } else { 0.38 };
    let g = ground_min(x, z, r[0].max(r[2]));
    let s = (hash(i, j, 107) * 4.0e9) as u32;
    Some(Stone {
        bed: ([x, g, z], [0.0, 1.0, 0.0]),
        c: [x, g + r[1] * (1.0 - 2.0 * sink), z],
        r,
        yaw: hash(i, j, 108) * std::f64::consts::TAU,
        tilt: [(hash(i, j, 109) - 0.5) * 0.5, (hash(i, j, 110) - 0.5) * 0.5],
        seed: s,
        tone: tone(hash(i, j, 111)),
        angular: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_armour_follows_hudson_and_the_ground_plan() {
        // Hudson: W50 = rho H^3 / (K_D Delta^3 cot a), D50 = (W50 / rho)^(1/3)
        let (rho, rho_w, h, kd) = (2650.0f64, 1025.0f64, 2.0f64, 2.0f64);
        let delta = rho / rho_w - 1.0;
        let w = rho * h.powi(3) / (kd * delta.powi(3) * REVETMENT_SLOPE_H);
        let d = (w / rho).cbrt();
        assert!((d - ARMOUR_D50_M).abs() < 0.01, "D50 {d}");
        assert!((ARMOUR_GRADATION[0] - 0.75f64.cbrt()).abs() < 0.002 && (ARMOUR_GRADATION[1] - 1.25f64.cbrt()).abs() < 0.002);
        // drawn at the ground plan (1.25): one metre
        let edges = [([0.0, 0.0], [10.0, 0.0], [0.0, -1.0])];
        let st = revetment(&edges, &|_, _| 5.0, 0.0, 1.25, 1.25, &|_| [0.2; 3]);
        assert!(!st.is_empty());
        for s in &st {
            let dmax = 2.0 * s.r[0].max(s.r[1]).max(s.r[2]);
            assert!(dmax <= ARMOUR_D50_M * 1.25 * ARMOUR_GRADATION[1] * 0.65 * 2.0 + 1e-9);
            assert!(s.c[2] > 0.0, "the stones lie seaward of the wall: {:?}", s.c);
            assert!(s.c[1] < 0.829 * 1.25 + 2.0 * 1.25, "under the armour's crest");
        }
        assert!((ARMOUR_D50_M * 1.25 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_boulders_follow_the_power_law() {
        assert!((boulder_d(0.0) - BOULDER_D_M[0]).abs() < 1e-9);
        assert!((boulder_d(1.0) - BOULDER_D_M[1]).abs() < 1e-9);
        // half the stones under 1.42 a D_min (2^(1/2) for exponent 2, truncated)
        let n = 10_000;
        let small = (0..n).filter(|i| boulder_d((*i as f64 + 0.5) / n as f64) < BOULDER_D_M[0] * 2f64.sqrt()).count();
        assert!((small as f64 / n as f64 - 0.5).abs() < 0.03, "{small}");
    }

    #[test]
    fn zones() {
        assert_eq!(zone(-10.0, 1.0, 0.1, true, false), Some(Zone::Bare));
        assert_eq!(zone(-10.0, 1.0, 0.9, true, false), None, "no boulder rests on a cliff face");
        assert_eq!(zone(10.0, -0.5, 0.05, true, false), Some(Zone::Bare));
        assert_eq!(zone(10.0, -0.5, 0.05, false, true), None, "no toe rocks in the sea");
        assert_eq!(zone(-10.0, 2.0, 0.1, false, true), Some(Zone::Toe));
        assert_eq!(zone(-40.0, 20.0, 1.3, false, false), Some(Zone::Outcrop));
        assert_eq!(zone(-400.0, 20.0, 1.3, false, false), None);
        assert_eq!(zone(5.0, -3.0, 0.1, true, false), None, "too deep");
    }

    #[test]
    fn a_stone_is_closed_and_wet_below_the_tide() {
        let mut m = MeshArrays::new("t");
        let s = Stone { c: [0.0, 0.0, 0.0], r: [1.0, 0.6, 0.8], yaw: 0.3, tilt: [0.1, -0.2], seed: 42, tone: [0.2, 0.2, 0.2], angular: true, bed: ([0.0, -10.0, 0.0], [0.0, 1.0, 0.0]) };
        stone_mesh(&mut m, &s, 0.0, [0.3, 0.1]);
        assert_eq!(m.triangles(), 320);
        // bedded at its middle: the faces under the bed are not built
        let mut half = MeshArrays::new("h");
        stone_mesh(&mut half, &Stone { bed: ([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]), ..s }, 0.0, [0.3, 0.1]);
        assert!(half.triangles() < 220 && half.triangles() > 120, "{}", half.triangles());
        let ys: Vec<f64> = m.positions.chunks(3).map(|p| p[1]).collect();
        assert!(ys.iter().all(|y| y.abs() <= 0.7));
        // the wet foot is darker than the dry top
        let (mut low, mut high) = (f64::MAX, 0.0f64);
        for (i, p) in m.positions.chunks(3).enumerate() {
            let c = m.colors[i * 3];
            if p[1] < -0.3 { low = low.min(c); }
            if p[1] > 0.4 { high = high.max(c); }
        }
        assert!(low < high * 0.5, "{low} {high}");
    }
}
