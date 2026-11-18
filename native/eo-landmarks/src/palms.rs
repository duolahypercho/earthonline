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

/// Linear colours (INFERRED): the leaves' dark glossy green and the older
/// leaves' olive, the trunk's grey-brown and the trimmed boss's tan.
pub const LEAF_LINEAR: Rgb = [0.032, 0.066, 0.022];
pub const OLD_LEAF_LINEAR: Rgb = [0.07, 0.078, 0.03];
pub const TRUNK_LINEAR: Rgb = [0.2, 0.165, 0.125];
pub const BOSS_LINEAR: Rgb = [0.27, 0.19, 0.11];

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

/// One palm at `site`, standing on `ground` (its trunk's foot sinks a little
/// below it so a slope never shows a gap).
fn palm(trunks: &mut MeshArrays, leaves: &mut MeshArrays, site: &Site, ground: f64) {
    let id = site.id;
    let h = site.height_m.filter(|v| *v > 2.0).unwrap_or(DEFAULT_HEIGHT_M);
    let crown = site.crown_m.filter(|v| *v > 1.0).unwrap_or(DEFAULT_CROWN_M);
    // the leaf: a crown spread is about 1.8 leaf lengths across (the leaves arch out and down).
    let leaf = (crown / 1.8).clamp(LEAF_LENGTH_M[0], LEAF_LENGTH_M[1]);
    // the top of the boss: the youngest, near-upright leaves rise about 0.8 of a leaf over it
    // (the record's height is the crown's top).
    let top = ground + (h - 0.8 * leaf).max(2.5);
    // a mature trunk (the corridor's are 10-15 m) in the upper half of the cited range
    let r = 0.5 * (TRUNK_DIAMETER_M[0] + (TRUNK_DIAMETER_M[1] - TRUNK_DIAMETER_M[0]) * (0.5 + 0.5 * draw(id, 1)));
    // a slight lean (a few degrees), as street palms grow.
    let lean_a = draw(id, 2) * std::f64::consts::TAU;
    let lean = 0.035 * draw(id, 3);
    let lean_v = [lean_a.cos() * lean, 0.0, lean_a.sin() * lean];
    let foot = [site.x, ground - 0.3, site.z];
    let at = |y: f64| add(foot, add([0.0, y - foot[1], 0.0], scale(lean_v, y - foot[1])));
    // the trunk: a flared foot, then rings of leaf-base scars to the boss.
    let mut rings: Vec<(V3, f64, Rgb)> = vec![(at(ground - 0.3), r * 1.3, TRUNK_LINEAR), (at(ground + 0.15), r * 1.22, TRUNK_LINEAR), (at(ground + 0.6), r * 1.06, TRUNK_LINEAR)];
    let boss0 = top - 1.5;
    let n = ((boss0 - ground - 0.6) / 0.42).floor().max(1.0) as usize;
    for k in 1..=n {
        let y = ground + 0.6 + (boss0 - ground - 0.6) * k as f64 / n as f64;
        // the scar bands: a ridge and a groove a course, the groove darker
        let ridge = k % 2 == 0;
        let t = (y - ground) / (boss0 - ground).max(1.0);
        let col = mix(TRUNK_LINEAR, [0.16, 0.135, 0.105], t) ;
        rings.push((at(y), r * (1.0 - 0.06 * t) * if ridge { 1.035 } else { 0.985 }, if ridge { col } else { scale(col, 0.78) }));
    }
    // the boss: the trimmed leaf bases, wider than the trunk, up into the crown
    for (dy, k, tone) in [(0.0, 1.0, 1.0), (0.35, 1.32, 1.0), (0.85, 1.45, 0.92), (1.3, 1.38, 0.85), (1.6, 1.0, 0.7), (1.75, 0.45, 0.6)] {
        rings.push((at(boss0 + dy), r * k, scale(BOSS_LINEAR, tone)));
    }
    tube(trunks, &rings, 12);

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
                // the cluster's tips sag a little under their own weight
                let reach = add(scale(dir, l), [0.0, -0.12 * l, 0.0]);
                let (a, b) = (p0, p1);
                let (ta, tb) = (add(p0, reach), add(add(p0, scale(seg, CLUSTER_EDGE)), scale(reach, 0.96)));
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
}

/// Every site's palm, standing on `ground_at` (x, z): the drawn surface.
pub fn build(sites: &[Site], ground_at: &dyn Fn(f64, f64) -> f64) -> Palms {
    let mut trunks = MeshArrays::new("palms:trunks");
    let mut leaves = MeshArrays::new("palms:leaves");
    for s in sites {
        palm(&mut trunks, &mut leaves, s, ground_at(s.x, s.z));
    }
    Palms { trunks: trunks.freeze(), leaves: leaves.freeze(), count: sites.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_palm_stands_to_its_recorded_height_with_its_cited_trunk() {
        let s = Site { x: 10.0, z: -5.0, height_m: Some(13.7), crown_m: Some(9.0), id: 7 };
        let p = build(&[s], &|_, _| 2.0);
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
    fn the_corridor_keeps_its_palms_once() {
        let k = eo_data::groundscale::scale();
        let far = CORRIDOR_M * k + 10.0;
        let list = [(0.0, 0.0, None, None, 1), (0.2, 0.1, None, None, 2), (far, 0.0, None, None, 3), (5.0, 5.0, Some(6.0), Some(11.0), 4)];
        let s = corridor_sites(&list, [0.0, 0.0]);
        assert_eq!(s.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, 4]);
    }
}
