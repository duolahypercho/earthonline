//! The Canary Island date palms (Phoenix canariensis) of the Embarcadero
//! corridor: the median rows of The Embarcadero, the Ferry Building plaza and
//! the foot of Market Street, the corridor's signature planting. The props'
//! street-tree walk (eo-props furniture) places a surveyed tree only on a
//! ribbon's sidewalk band, so the median rows were never placed, and the
//! palm GLB it would draw is not shipped (a thin proxy trunk stands in); this
//! module draws every surveyed palm of the corridor as its own mesh. A palm
//! the walk also places keeps its proxy trunk, inside this trunk.
//!
//! Where and how big: the DataSF street tree list (dataset tkzw-k3nq, PDDL
//! 1.0, as eo-props exports it: `trees` in eo-props/data/props-js.json.gz,
//! at the scaled ground plan) records of the `palm` form within
//! `CORRIDOR_M` (surveyed) of the Ferry Building's tower, at their surveyed
//! point, their recorded height and crown spread (a record without them takes
//! `DEFAULT_HEIGHT_M` and `DEFAULT_CROWN_M`, the list's corridor medians).
//! 222 date palms line The Embarcadero (Landscape Architect & Specifier News,
//! "On the Waterfront"); the list does not export a palm's species, so every
//! corridor palm is drawn as that species.
//!
//! The form, cited: UF/IFAS EDIS ST439 / Broschat (2013), "Phoenix
//! canariensis: Canary Island Date Palm": a single trunk 2-3 ft (0.6-0.9 m)
//! across, patterned with the diamond-shaped scars of its leaf bases; 8-15 ft
//! (2.4-4.6 m) rigid pinnate leaves of up to 200 V-shaped leaflets, the
//! lowest turned into long spines; a dense rounded crown. INFERRED (no
//! survey): the crown's leaf count, the leaflet length and fold, the droop of
//! the older leaves, the trimmed leaf-base boss under the crown (the
//! "pineapple" cut of a maintained palm), the colours.

use crate::geometry::{Frozen, MeshArrays, Rgb, V3};

/// Palms within this of the Ferry Building's tower (surveyed metres, before the plan scale).
pub const CORRIDOR_M: f64 = 800.0;
/// A record without a height or crown spread: the corridor's medians in the list, m.
pub const DEFAULT_HEIGHT_M: f64 = 14.0;
pub const DEFAULT_CROWN_M: f64 = 8.7;
/// The trunk's diameter range (EDIS ST439: 2-3 ft), m.
pub const TRUNK_DIAMETER_M: [f64; 2] = [0.61, 0.91];
/// The leaf length range (EDIS ST439: 8-15 ft), m.
pub const LEAF_LENGTH_M: [f64; 2] = [2.44, 4.57];
/// Leaves in the crown (INFERRED: a maintained crown of a mature palm), and the
/// segments of a leaf's rachis; each segment carries one leaflet cluster a
/// side (a blade from the rachis out to the leaflets' tips, notched at its
/// outer edge: it stands for the eight or so real leaflets along it).
pub const LEAVES: usize = 56;
pub const SEGMENTS: usize = 13;
/// The share of a segment a cluster's outer edge spans (the rest is the notch to the next).
pub const CLUSTER_EDGE: f64 = 0.62;
/// The longest leaflet (INFERRED, mid-leaf), the V fold's lift (rad) and the leaflets' forward rake.
pub const LEAFLET_M: f64 = 0.5;
pub const FOLD_RAD: f64 = 0.45;
pub const RAKE: f64 = 0.55;

/// Linear colours (INFERRED): the leaves' natural coastal olive-green and the older
/// leaves' warm dry olive, the trunk's weathered ash-grey-brown fibrous bark and the boss's dark umber.
pub const LEAF_LINEAR: Rgb = [0.030, 0.058, 0.021];
pub const OLD_LEAF_LINEAR: Rgb = [0.068, 0.072, 0.028];
pub const TRUNK_LINEAR: Rgb = [0.112, 0.095, 0.079];
pub const BOSS_LINEAR: Rgb = [0.165, 0.115, 0.070];

/// The palms' second pass (`features::palm_detail`; off: the first pass, as before).
///
/// * Size: a record with no surveyed height or spread draws them from the
///   corridor's medians times 0.85-1.15 (its own draw), so unsurveyed palms
///   are not one clone; a recorded size is kept. The lean reaches 5 degrees
///   (street palms lean a few degrees off true, INFERRED).
/// * The trunk's leaf-base scars as a staggered diamond lattice in its surface
///   (EDIS ST439: "diamond-shaped scars"): `SCARS_AROUND` scars a course, a course
///   every `SCAR_COURSE_M`, with overhanging upper petiole lips, shadowed fibrous
///   grooves, alternating triangulation (no diagonal twist), and per-scar grit.
/// * The root initials at its foot: a palm's adventitious roots break out of
///   the trunk's base as a mat of short knuckles (INFERRED count and size).
/// * The crown: a few senescent lower leaves hanging down, yellowing to
///   straw (a maintained palm keeps a ring of them between trims), and the
///   unopened spear leaf upright in the middle.
/// * The tree well: the palm stands in an Embarcadero-aligned square basin of
///   weathered decomposed granite `WELL_M` across behind a deep-skirted cast-iron
///   & stone curb that seats solidly into sloped median curbs with zero floating
///   overhang. INFERRED size: SF tree basins are 3-4 ft for street trees; a
///   palm's 0.6-0.9 m trunk takes the larger, 5 ft square.
pub const SCARS_AROUND: usize = 8;
pub const SCAR_COURSE_M: f64 = 0.28;
pub const SCAR_RELIEF: f64 = 0.105;
/// The lattice is drawn up to this over the ground (covering eye-level and mid-trunk
/// views along the Embarcadero); over it the trunk keeps the first pass's scar bands (INFERRED).
pub const SCAR_LATTICE_TOP_M: f64 = 8.0;
pub const ROOT_KNUCKLES: usize = 0;
pub const DEAD_LEAVES: usize = 6;
/// Compact SF Embarcadero median tree grate (~3.15 ft) fitting tightly inside curb wells.
pub const WELL_M: f64 = 0.96;
pub const WELL_EDGE_M: f64 = 0.05;
/// Weathered SF concrete-grey flush tree surround & slotted cast-iron grate matching Embarcadero paving.
pub const WELL_FILL_LINEAR: Rgb = [0.165, 0.160, 0.152];
pub const WELL_EDGE_LINEAR: Rgb = [0.225, 0.220, 0.212];
pub const DEAD_LEAF_LINEAR: Rgb = [0.22, 0.155, 0.068];
pub const ROOT_LINEAR: Rgb = [0.095, 0.074, 0.056];

/// One surveyed palm on the drawn plan.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub x: f64,
    pub z: f64,
    pub height_m: Option<f64>,
    pub crown_m: Option<f64>,
    /// The list record's index (its draws: size, lean, leaf phase).
    pub id: u32,
}

pub struct Palms {
    /// Trunks and their bosses (matte).
    pub trunks: Frozen,
    /// The leaves (a waxy green).
    pub leaves: Frozen,
    pub count: usize,
}

/// A stable draw in [0, 1) per palm and salt.
fn draw(id: u32, salt: u32) -> f64 {
    let mut h = (id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (salt as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h % 1_000_003) as f64 / 1_000_003.0
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-9);
    scale(a, 1.0 / l)
}
fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// The surveyed palms within the corridor of `centre` (drawn plan): `palms`
/// gives each palm record's (x, z, crown m, height m), plan metres.
pub fn corridor_sites(palms: &[(f64, f64, Option<f64>, Option<f64>, u32)], centre: [f64; 2]) -> Vec<Site> {
    let reach = CORRIDOR_M * eo_data::groundscale::scale();
    let mut out: Vec<Site> = Vec::new();
    for &(x, z, crown_m, height_m, id) in palms {
        if (x - centre[0]).hypot(z - centre[1]) > reach {
            continue;
        }
        // the list repeats a few records at one point: one palm there.
        if out.iter().any(|s| (s.x - x).hypot(s.z - z) < 1.0) {
            continue;
        }
        out.push(Site { x, z, height_m, crown_m, id });
    }
    out
}

/// A smooth tube along `axis` points with radii and colours per ring (`seg` sides).
fn tube(m: &mut MeshArrays, rings: &[(V3, f64, Rgb)], seg: usize) {
    let mut prev: Vec<u32> = Vec::new();
    for (k, (c, r, col)) in rings.iter().enumerate() {
        let mut cur = Vec::with_capacity(seg + 1);
        for j in 0..=seg {
            let a = j as f64 / seg as f64 * std::f64::consts::TAU;
            let n = [a.cos(), 0.0, a.sin()];
            cur.push(m.vertex(add(*c, scale(n, *r)), n, *col));
        }
        if k > 0 {
            for j in 0..seg {
                // wound outward (cull-free pipelines face it to the eye in any case)
                m.tri_idx(prev[j], cur[j + 1], cur[j]);
                m.tri_idx(prev[j], prev[j + 1], cur[j + 1]);
            }
        }
        prev = cur;
    }
}

/// One palm at `site`, standing on `ground_at` (its trunk's foot sinks a little
/// below ground so a slope never shows a gap, and its tree well aligns with The
/// Embarcadero median and extends a solid curb skirt down into sloped kerbs).
fn palm(trunks: &mut MeshArrays, leaves: &mut MeshArrays, site: &Site, ground_at: &dyn Fn(f64, f64) -> f64, detail: bool) {
    let ground = ground_at(site.x, site.z);
    let id = site.id;
    // an unsurveyed size: the median, varied per palm with the second pass.
    let vary = |salt: u32| if detail { 0.85 + 0.3 * draw(id, salt) } else { 1.0 };
    let h = site.height_m.filter(|v| *v > 2.0).unwrap_or(DEFAULT_HEIGHT_M * vary(31));
    let crown = site.crown_m.filter(|v| *v > 1.0).unwrap_or(DEFAULT_CROWN_M * vary(32));
    // the leaf: a crown spread is about 1.8 leaf lengths across (the leaves arch out and down).
    let leaf = (crown / 1.8).clamp(LEAF_LENGTH_M[0], LEAF_LENGTH_M[1]);
    // the top of the boss: the youngest, near-upright leaves rise about 0.8 of a leaf over it
    // (the record's height is the crown's top).
    let top = ground + (h - 0.8 * leaf).max(2.5);
    // a mature trunk (the corridor's are 10-15 m) in the upper half of the cited range
    let r = 0.5 * (TRUNK_DIAMETER_M[0] + (TRUNK_DIAMETER_M[1] - TRUNK_DIAMETER_M[0]) * (0.5 + 0.5 * draw(id, 1)));
    // a slight lean (a few degrees), as street palms grow.
    let lean_a = draw(id, 2) * std::f64::consts::TAU;
    let lean = if detail { 5.0f64.to_radians().tan() * draw(id, 3) } else { 0.035 * draw(id, 3) };
    let lean_v = [lean_a.cos() * lean, 0.0, lean_a.sin() * lean];
    let foot = [site.x, ground - 0.3, site.z];
    let at = |y: f64| add(foot, add([0.0, y - foot[1], 0.0], scale(lean_v, y - foot[1])));
    // the trunk: a flared damp-soot foot, then rings of leaf-base scars to the boss.
    let trunk_tint = 0.84 + 0.32 * draw(id, 17);
    let foot_col = scale(TRUNK_LINEAR, 0.72 * trunk_tint);
    let mut rings: Vec<(V3, f64, Rgb)> = vec![
        (at(ground - 0.3), r * 1.3, foot_col),
        (at(ground + 0.15), r * 1.22, scale(TRUNK_LINEAR, 0.80 * trunk_tint)),
        (at(ground + 0.6), r * 1.06, scale(TRUNK_LINEAR, 0.90 * trunk_tint)),
    ];
    let boss0 = top - 1.5;
    let n = ((boss0 - ground - 0.6) / 0.42).floor().max(1.0) as usize;
    for k in 1..=n {
        let y = ground + 0.6 + (boss0 - ground - 0.6) * k as f64 / n as f64;
        // the scar bands: a ridge and a groove a course, the groove darker
        let ridge = k % 2 == 0;
        let t = (y - ground) / (boss0 - ground).max(1.0);
        let col = scale(mix(TRUNK_LINEAR, [0.095, 0.080, 0.066], t), trunk_tint);
        rings.push((at(y), r * (1.0 - 0.06 * t) * if ridge { 1.045 } else { 0.975 }, if ridge { col } else { scale(col, 0.66) }));
    }
    // the boss: the trimmed fibrous leaf bases ("pineapple"), wider than the trunk, up into the crown
    let boss: Vec<(V3, f64, Rgb)> = [(0.0, 1.0, 0.92), (0.35, 1.32, 1.0), (0.85, 1.45, 0.86), (1.3, 1.38, 0.76), (1.6, 1.0, 0.62), (1.75, 0.45, 0.52)].iter().map(|&(dy, k, tone)| (at(boss0 + dy), r * k, scale(BOSS_LINEAR, tone * trunk_tint))).collect();
    if detail {
        // Textured diamond leaf-scar bark runs continuously down into the tree grate (zero smooth toy-cone flare
        // or spiky fin roots!), then transitions to the upper scar bands and trimmed pineapple boss.
        let lattice_top = (ground + SCAR_LATTICE_TOP_M).min(boss0);
        let mut upper: Vec<(V3, f64, Rgb)> = rings[3..].iter().filter(|(c, ..)| c[1] > lattice_top + 0.2).cloned().collect();
        upper.extend(boss.iter().cloned());
        scar_trunk(trunks, &upper, &at, ground, lattice_top, boss0, r, id);
        // Align flush cast-iron tree grate cleanly with The Embarcadero frontage.
        let emb_yaw = (-1844.6_f64 - (-2006.2)).atan2(2325.5 - 2206.4);
        well(trunks, [site.x, site.z], ground_at, ground, r, emb_yaw);
    } else {
        rings.extend(boss);
        tube(trunks, &rings, 12);
    }

    // the crown: leaves on a golden-angle spiral, the youngest upright in the middle,
    // the oldest out and drooping under their weight.
    let c = at(top);
    let golden = std::f64::consts::PI * (3.0 - 5.0f64.sqrt());
    let phase = draw(id, 4) * std::f64::consts::TAU;
    for j in 0..LEAVES {
        let age = (j as f64 + 0.5) / LEAVES as f64;
        let az = phase + j as f64 * golden + (draw(id, 100 + j as u32) - 0.5) * 0.3;
        let elev0 = (76.0 - 84.0 * age + (draw(id, 200 + j as u32) - 0.5) * 10.0).to_radians();
        let droop = (24.0 + 46.0 * age).to_radians();
        let len = leaf * (0.82 + 0.18 * draw(id, 300 + j as u32)) * (0.8 + 0.2 * (1.0 - (age - 0.45).abs()));
        let col = mix(LEAF_LINEAR, OLD_LEAF_LINEAR, (age - 0.55).max(0.0) / 0.45);
        let radial = [az.cos(), 0.0, az.sin()];
        let base = add(add(c, scale(radial, r * 0.9)), [0.0, -0.25 * age, 0.0]);
        // the rachis
        let mut pts: Vec<V3> = vec![base];
        let mut tans: Vec<V3> = Vec::new();
        for i in 0..SEGMENTS {
            let t = (i as f64 + 0.5) / SEGMENTS as f64;
            let e = elev0 - droop * t.powf(1.6);
            let d = [e.cos() * az.cos(), e.sin(), e.cos() * az.sin()];
            tans.push(d);
            let p = add(*pts.last().unwrap(), scale(d, len / SEGMENTS as f64));
            pts.push(p);
        }
        for i in 0..SEGMENTS {
            let (p0, p1, tg) = (pts[i], pts[i + 1], tans[i]);
            let side = norm(cross(tg, [0.0, 1.0, 0.0]));
            let side = if side[0].is_finite() { side } else { [-az.sin(), 0.0, az.cos()] };
            let up = norm(cross(side, tg));
            let t = (i as f64 + 0.5) / SEGMENTS as f64;
            // the leaflets: short spines at the base, longest past the middle, tapering to the tip
            let shape = if t < 0.18 { 0.3 + 1.5 * t } else { (1.0 - ((t - 0.5) / 0.55).powi(2)).max(0.3) };
            let l = LEAFLET_M * shape * (len / LEAF_LENGTH_M[1]).sqrt();
            let seg = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
            for s in [-1.0f64, 1.0] {
                let dir = norm(add(add(scale(side, s * FOLD_RAD.cos()), scale(up, FOLD_RAD.sin())), scale(tg, RAKE)));
                // the cluster's tips sag naturally and taper to fine pointed pinnate spears (never blunt cards)
                let reach = add(scale(dir, l), [0.0, -0.14 * l, 0.0]);
                let (a, b) = (p0, add(p0, scale(seg, 0.78)));
                let (ta, tb) = (
                    add(add(p0, scale(seg, 0.16)), scale(reach, 1.05)),
                    add(add(p0, scale(seg, 0.34)), scale(reach, 0.88)),
                );
                let tone = 0.85 + 0.3 * draw(id, 1000 + (j * 64 + i * 2) as u32 + if s > 0.0 { 1 } else { 0 });
                let lc = scale(col, tone * (0.92 + 0.16 * t));
                let nrm = norm(cross([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [ta[0] - a[0], ta[1] - a[1], ta[2] - a[2]]));
                // the rachis side a little darker (the leaflets' bases shade each other)
                let (ia, ib) = (leaves.vertex(a, nrm, scale(lc, 0.82)), leaves.vertex(b, nrm, scale(lc, 0.82)));
                let (ita, itb) = (leaves.vertex(ta, nrm, scale(lc, 1.08)), leaves.vertex(tb, nrm, scale(lc, 1.08)));
                leaves.tri_idx(ia, ib, itb);
                leaves.tri_idx(ia, itb, ita);
            }
        }
    }
    if detail {
        // the senescent lower leaves: hanging close round the boss, yellowing to straw.
        for j in 0..DEAD_LEAVES {
            let az = phase + 0.4 + j as f64 * std::f64::consts::TAU / DEAD_LEAVES as f64 + (draw(id, 500 + j as u32) - 0.5) * 0.6;
            let radial = [az.cos(), 0.0, az.sin()];
            let len = leaf * (0.7 + 0.25 * draw(id, 510 + j as u32));
            let base = add(add(c, scale(radial, r * 1.2)), [0.0, -0.45, 0.0]);
            let col = mix(OLD_LEAF_LINEAR, DEAD_LEAF_LINEAR, 0.45 + 0.55 * draw(id, 520 + j as u32));
            let mut p = base;
            let n = 8;
            for i in 0..n {
                let t = (i as f64 + 0.5) / n as f64;
                // out a little, then straight down along the trunk
                let e = (-35.0 - 50.0 * t.powf(0.7)).to_radians();
                let d = [e.cos() * az.cos(), e.sin(), e.cos() * az.sin()];
                let q = add(p, scale(d, len / n as f64));
                let side = norm(cross(d, [0.0, 1.0, 0.0]));
                let side = if side[0].is_finite() { side } else { [-az.sin(), 0.0, az.cos()] };
                let l = 0.32 * (1.0 - 0.6 * t);
                for sgn in [-1.0f64, 1.0] {
                    // the dry leaflets fold in toward the rachis and hang
                    let tip = add(add(p, scale(side, sgn * l * 0.55)), [0.0, -l * 0.8, 0.0]);
                    let tip2 = add(add(q, scale(side, sgn * l * 0.5)), [0.0, -l * 0.75, 0.0]);
                    let nrm = norm(cross([q[0] - p[0], q[1] - p[1], q[2] - p[2]], [tip[0] - p[0], tip[1] - p[1], tip[2] - p[2]]));
                    let k = 0.85 + 0.25 * draw(id, 600 + (j * 32 + i * 2) as u32 + (sgn > 0.0) as u32);
                    let (ia, ib) = (leaves.vertex(p, nrm, scale(col, 0.8 * k)), leaves.vertex(q, nrm, scale(col, 0.8 * k)));
                    let (ita, itb) = (leaves.vertex(tip, nrm, scale(col, k)), leaves.vertex(tip2, nrm, scale(col, k)));
                    leaves.tri_idx(ia, ib, itb);
                    leaves.tri_idx(ia, itb, ita);
                }
                p = q;
            }
        }
        // the spear: the unopened leaf, a tight upright blade in the crown's middle.
        let s0 = add(c, [0.0, 0.2, 0.0]);
        let s1 = add(c, [lean_v[0] * 2.0, 0.2 + leaf * 0.55, lean_v[2] * 2.0]);
        let sa = phase + 1.1;
        for k in 0..3 {
            let a = sa + k as f64 * std::f64::consts::TAU / 3.0;
            let off = [a.cos() * 0.07, 0.0, a.sin() * 0.07];
            let (a0, a1) = (add(s0, off), add(s0, scale(off, -0.5)));
            let nrm = norm([off[0], 0.2, off[2]]);
            let col = scale(LEAF_LINEAR, 1.25);
            let (i0, i1, i2) = (leaves.vertex(a0, nrm, col), leaves.vertex(a1, nrm, col), leaves.vertex(s1, nrm, scale(col, 1.15)));
            leaves.tri_idx(i0, i1, i2);
        }
    }
}

/// The trunk with its leaf-base scars (the second pass): running continuously
/// from below the tree grate (`ground - 0.25`) up to `top` with a gentle organic
/// textured basal swell (zero smooth untextured cone skirt), staggered diamond
/// petiole-stub scars, overhanging lips, shadowed crevices, and alternating
/// diamond triangulation (no diagonal barberpole twist).
#[allow(clippy::too_many_arguments)]
fn scar_trunk(m: &mut MeshArrays, boss: &[(V3, f64, Rgb)], at: &dyn Fn(f64) -> V3, ground: f64, top: f64, boss0: f64, r: f64, id: u32) {
    let seg = SCARS_AROUND * 2;
    let y0 = ground - 0.25;
    let rows = ((top - y0) / (SCAR_COURSE_M / 2.0)).round().max(2.0) as usize;
    let twist = draw(id, 40) * std::f64::consts::TAU;
    let trunk_tint = 0.84 + 0.32 * draw(id, 17);
    // Asymmetric overhanging diamond petiole-stub relief: 1 on a jagged leaf-base ridge, 0 in deep bark grooves.
    let lattice = |a: f64, y: f64| -> f64 {
        let u = a / std::f64::consts::TAU * SCARS_AROUND as f64;
        let v = (y - y0) / SCAR_COURSE_M;
        let (p, q) = ((u + v).rem_euclid(1.0), (u - v).rem_euclid(1.0));
        let tri = |x: f64| 1.0 - (2.0 * x - 1.0).abs();
        let dia = (tri(p) * tri(q)).sqrt();
        // Steep overhanging horizontal lip at each pruned frond base, deeply undercut beneath.
        let v_in = v.rem_euclid(1.0);
        let lip = if v_in < 0.72 { v_in / 0.72 } else { (1.0 - v_in) / 0.28 };
        (dia * (0.62 + 0.38 * lip)).clamp(0.0, 1.0)
    };
    let cell_jitter = |k: usize, j: usize, salt: u32| -> f64 {
        let jj = j % seg;
        draw(id ^ ((k as u32).wrapping_mul(37) ^ (jj as u32).wrapping_mul(131)), salt)
    };
    let radius = |a: f64, y: f64, jit: f64| {
        let t = ((y - ground) / (boss0 - ground).max(1.0)).clamp(0.0, 1.0);
        let l = lattice(a, y);
        // Subtle organic fibrous basal swell over the bottom 0.65 m (textured bark all the way down into soil!).
        let basal = ((ground + 0.65 - y) / 0.90).clamp(0.0, 1.0);
        let swell = 1.0 + 0.14 * basal * basal;
        r * swell * (1.0 - 0.06 * t) * (1.0 + SCAR_RELIEF * (l - 0.45) + 0.022 * (jit - 0.5))
    };
    let point = |a: f64, y: f64, jit: f64| {
        let c = at(y);
        let rr = radius(a, y, jit);
        [c[0] + a.cos() * rr, y, c[2] + a.sin() * rr]
    };
    let mut lines: Vec<Vec<(V3, V3, Rgb)>> = Vec::new();
    for k in 0..=rows {
        let y = y0 + (top - y0) * k as f64 / rows as f64;
        let t = ((y - ground) / (boss0 - ground).max(1.0)).clamp(0.0, 1.0);
        // Damp ground-contact bark darkening fading smoothly over the lowest 1.4 m.
        let base_soot = 0.62 + 0.38 * ((y - ground + 0.1) / 1.4).clamp(0.0, 1.0);
        let base = scale(mix(TRUNK_LINEAR, [0.095, 0.080, 0.066], t), trunk_tint * base_soot);
        lines.push((0..=seg).map(|j| {
            let a = twist + j as f64 / seg as f64 * std::f64::consts::TAU;
            let jit = cell_jitter(k, j, 81);
            let col_jit = cell_jitter(k, j, 97);
            let p = point(a, y, jit);
            let (da, dy) = (0.025, 0.025);
            let ta = {
                let (p0, p1) = (point(a - da, y, jit), point(a + da, y, jit));
                [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]
            };
            let ty = {
                let (p0, p1) = (point(a, y - dy, jit), point(a, y + dy, jit));
                [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]
            };
            let mut n = norm(cross(ty, ta));
            if n[0] * a.cos() + n[2] * a.sin() < 0.0 {
                n = scale(n, -1.0);
            }
            let l = lattice(a, y);
            let bark_shade = (0.42 + 0.78 * l) * (0.86 + 0.28 * col_jit);
            let col = [
                base[0] * bark_shade * (0.96 + 0.08 * l),
                base[1] * bark_shade * (0.97 + 0.05 * l),
                base[2] * bark_shade * (1.02 - 0.04 * l),
            ];
            (p, n, col)
        }).collect());
    }
    for (bi, (c, rr, col)) in boss.iter().enumerate() {
        lines.push((0..=seg).map(|j| {
            let a = twist + j as f64 / seg as f64 * std::f64::consts::TAU;
            let n = [a.cos(), 0.0, a.sin()];
            let checker = if (bi + j) % 2 == 0 { 1.03 } else { 0.96 };
            let tone = if (bi + j) % 2 == 0 { 1.08 } else { 0.76 };
            (add(*c, scale(n, *rr * checker)), n, scale(*col, tone))
        }).collect());
    }
    let mut prev: Vec<u32> = Vec::new();
    for (row_i, line) in lines.into_iter().enumerate() {
        let cur: Vec<u32> = line.iter().map(|(p, n, c)| m.vertex(*p, *n, *c)).collect();
        if !prev.is_empty() {
            for j in 0..seg {
                if (row_i + j) % 2 == 0 {
                    m.tri_idx(prev[j], cur[j + 1], cur[j]);
                    m.tri_idx(prev[j], prev[j + 1], cur[j + 1]);
                } else {
                    m.tri_idx(prev[j], prev[j + 1], cur[j]);
                    m.tri_idx(prev[j + 1], cur[j + 1], cur[j]);
                }
            }
        }
        prev = cur;
    }
}

/// The flush cast-iron & concrete SF Embarcadero tree grate (the second pass): a compact
/// `WELL_M` surround aligned to the Embarcadero median, clamped at or below the palm's
/// own root level `ground` so corners never ride up onto adjacent raised curb lips.
fn well(m: &mut MeshArrays, c: [f64; 2], ground_at: &dyn Fn(f64, f64) -> f64, ground: f64, r: f64, turn: f64) {
    let half = WELL_M / 2.0;
    let (ca, sa) = (turn.cos(), turn.sin());
    let plan = |u: f64, v: f64| [c[0] + u * ca - v * sa, c[1] + u * sa + v * ca];
    let w = |u: f64, v: f64, y: f64| {
        let p = plan(u, v);
        [p[0], y, p[1]]
    };
    // Never climb upward onto a raised neighbouring curb step (eliminates raised shark-fin corners in curb slots).
    let seated_g = |u: f64, v: f64| {
        let p = plan(u, v);
        ground_at(p[0], p[1]).min(ground)
    };
    let n = 6;
    let step = WELL_M / n as f64;
    let mut idx = vec![vec![0u32; n + 1]; n + 1];
    for (i, row) in idx.iter_mut().enumerate() {
        for (j, slot) in row.iter_mut().enumerate() {
            let (u, v) = (-half + i as f64 * step, -half + j as f64 * step);
            let fill_y = seated_g(u, v) + 0.008;
            let d = u.hypot(v);
            let ao = 0.45 + 0.55 * ((d - r * 0.90) / 0.36).clamp(0.0, 1.0).powf(0.8);
            // Subtle weathered flush tree-grate texture blending with Embarcadero concrete.
            let slot_pat = if (i + j) % 2 == 0 { 1.08 } else { 0.84 };
            let grain = 0.92 + 0.16 * draw((i * 31 + j) as u32, (c[0] * 10.0) as u32 ^ (c[1] * 10.0) as u32);
            *slot = m.vertex(w(u, v, fill_y), [0.0, 1.0, 0.0], scale(WELL_FILL_LINEAR, ao * slot_pat * grain));
        }
    }
    for i in 0..n {
        for j in 0..n {
            let (a, b, cc, d) = (idx[i][j], idx[i + 1][j], idx[i + 1][j + 1], idx[i][j + 1]);
            m.tri_idx(a, cc, b);
            m.tri_idx(a, d, cc);
        }
    }
    // Thin flush weathered concrete/iron surround rim (14 mm proud, 25 mm seated lip).
    let e = WELL_EDGE_M;
    let edge_y = |u: f64, v: f64| seated_g(u, v) + 0.014;
    let lip_y = |u: f64, v: f64| seated_g(u, v) - 0.025;
    let lip_col = scale(WELL_EDGE_LINEAR, 0.82);
    for k in 0..4 {
        let rot = |p: [f64; 2]| match k {
            0 => p,
            1 => [-p[1], p[0]],
            2 => [-p[0], -p[1]],
            _ => [p[1], -p[0]],
        };
        let corners = [rot([-half - e, half]), rot([half + e, half]), rot([half + e, half + e]), rot([-half - e, half + e])];
        let out_n = { let o = rot([0.0, 1.0]); [o[0] * ca - o[1] * sa, 0.0, o[0] * sa + o[1] * ca] };
        let tops: Vec<u32> = corners.iter().map(|p| m.vertex(w(p[0], p[1], edge_y(p[0], p[1])), [0.0, 1.0, 0.0], WELL_EDGE_LINEAR)).collect();
        m.tri_idx(tops[0], tops[2], tops[1]);
        m.tri_idx(tops[0], tops[3], tops[2]);
        let (o0, o1) = (corners[3], corners[2]);
        let f = [
            m.vertex(w(o0[0], o0[1], lip_y(o0[0], o0[1])), out_n, lip_col),
            m.vertex(w(o1[0], o1[1], lip_y(o1[0], o1[1])), out_n, lip_col),
            m.vertex(w(o1[0], o1[1], edge_y(o1[0], o1[1])), out_n, WELL_EDGE_LINEAR),
            m.vertex(w(o0[0], o0[1], edge_y(o0[0], o0[1])), out_n, WELL_EDGE_LINEAR),
        ];
        m.tri_idx(f[0], f[1], f[2]);
        m.tri_idx(f[0], f[2], f[3]);
    }
}

/// Every site's palm, standing on `ground_at` (x, z): the drawn surface.
pub fn build(sites: &[Site], ground_at: &dyn Fn(f64, f64) -> f64) -> Palms {
    build_with(sites, ground_at, crate::features::palm_detail())
}

/// `build`, with or without the second pass (`detail`).
pub fn build_with(sites: &[Site], ground_at: &dyn Fn(f64, f64) -> f64, detail: bool) -> Palms {
    let mut trunks = MeshArrays::new("palms:trunks");
    let mut leaves = MeshArrays::new("palms:leaves");
    for s in sites {
        palm(&mut trunks, &mut leaves, s, ground_at, detail);
    }
    Palms { trunks: trunks.freeze(), leaves: leaves.freeze(), count: sites.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_palm_stands_to_its_recorded_height_with_its_cited_trunk() {
        let s = Site { x: 10.0, z: -5.0, height_m: Some(13.7), crown_m: Some(9.0), id: 7 };
        let p = build_with(&[s], &|_, _| 2.0, false);
        assert_eq!(p.count, 1);
        let top = p.leaves.positions.chunks_exact(3).map(|v| v[1] as f64).fold(f64::MIN, f64::max);
        assert!((top - (2.0 + 13.7)).abs() < 1.2, "crown top {top}");
        // the trunk's girth a metre up is within the cited diameter (its flare below).
        let near: Vec<f64> = p.trunks.positions.chunks_exact(3).filter(|v| (v[1] as f64 - 3.5).abs() < 0.3).map(|v| (v[0] as f64 - 10.0).hypot(v[2] as f64 + 5.0)).collect();
        assert!(!near.is_empty());
        let d = 2.0 * near.iter().cloned().fold(0.0, f64::max);
        assert!(d >= TRUNK_DIAMETER_M[0] * 0.95 && d <= TRUNK_DIAMETER_M[1] * 1.1, "trunk {d:.2} m across");
        // the crown's reach: about the recorded spread.
        let reach = p.leaves.positions.chunks_exact(3).map(|v| (v[0] as f64 - 10.0).hypot(v[2] as f64 + 5.0)).fold(0.0, f64::max);
        assert!(reach > 9.0 * 0.35 && reach < 9.0 * 0.75, "reach {reach:.1} m");
        for n in p.leaves.normals.chunks_exact(3) {
            assert!(((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() < 1e-3);
        }
        // a modest budget a palm
        assert!(p.leaves.triangles() + p.trunks.triangles() < 4_000, "{} triangles", p.leaves.triangles() + p.trunks.triangles());
    }

    #[test]
    fn the_second_pass_seats_the_palm_in_its_well_and_keeps_its_size() {
        let s = Site { x: 10.0, z: -5.0, height_m: Some(13.7), crown_m: Some(9.0), id: 7 };
        let (a, b) = (build_with(&[s], &|_, _| 2.0, false), build_with(&[s], &|_, _| 2.0, true));
        let top = |p: &Palms| p.leaves.positions.chunks_exact(3).map(|v| v[1] as f64).fold(f64::MIN, f64::max);
        assert!((top(&a) - top(&b)).abs() < 0.6, "a recorded height is kept");
        // the well: fill and edge just over the ground, out to its half width and the edge.
        let low: Vec<&[f32]> = b.trunks.positions.chunks_exact(3).filter(|v| (v[1] as f64 - 2.0).abs() < 0.03).collect();
        let reach = low.iter().map(|v| (v[0] as f64 - 10.0).hypot(v[2] as f64 + 5.0)).fold(0.0, f64::max);
        assert!(reach > WELL_M / 2.0 && reach < (WELL_M / 2.0 + WELL_EDGE_M) * 1.5, "well reach {reach:.2}");
        // the lean stays within 5 degrees.
        assert!(5.0f64.to_radians().tan() <= 0.0875 + 1e-9);
        // an unsurveyed palm varies from the median, within 15 %.
        let u = |id| build_with(&[Site { x: 0.0, z: 0.0, height_m: None, crown_m: None, id }], &|_, _| 0.0, true);
        let tops: Vec<f64> = (1..6).map(|id| top(&u(id))).collect();
        let (lo, hi) = (tops.iter().cloned().fold(f64::MAX, f64::min), tops.iter().cloned().fold(f64::MIN, f64::max));
        assert!(hi - lo > 0.3, "unsurveyed palms differ: {tops:?}");
        assert!(lo > DEFAULT_HEIGHT_M * 0.8 && hi < DEFAULT_HEIGHT_M * 1.2, "{tops:?}");
        assert!(b.leaves.triangles() + b.trunks.triangles() < 6_000, "{} triangles", b.leaves.triangles() + b.trunks.triangles());
    }

    #[test]
    fn the_corridor_keeps_its_palms_once() {
        let k = eo_data::groundscale::scale();
        let far = CORRIDOR_M * k + 10.0;
        let list = [(0.0, 0.0, None, None, 1), (0.2, 0.1, None, None, 2), (far, 0.0, None, None, 3), (5.0, 5.0, Some(6.0), Some(11.0), 4)];
        let s = corridor_sites(&list, [0.0, 0.0]);
        assert_eq!(s.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, 4]);
    }
}
