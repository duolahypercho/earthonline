//! The crown core (`--tree-crowns`, off by default; `--no-tree-crowns`): an
//! opaque, lumpy hull inside a street form's leaf cards, so a crown seen from
//! the mid and far rings (70 m to 360 m) reads as a mass of leaves. The window
//! and far forms keep a few hundred or a few dozen sprig cards; their opaque
//! leaf covers about a quarter of the crown seen from above and a third seen
//! from the side, so from the air a yard tree reads as a sapling over bare
//! ground. Nothing here is JS: the browser game drew these forms without it.
//!
//! The hull is the form's own leaf cloud: a geodesic sphere about the cloud's
//! box centre, each vertex pushed out, in the crown's ellipsoidal frame (the
//! box's half extents as the unit), to a quantile of the card vertices' reach
//! inside a cone round its direction, filled and smoothed across its
//! neighbours, then set back by `inset` so the cards still make the crown's
//! edge and break its outline. Normals: the hull's own, bent toward the crown
//! ellipsoid's outward normal and a little up (the leaf cards' crown-normal
//! idea, `EZ_CROWN_NORMAL`). Colours: the cards' tint in the cone (their
//! colour over the tone geometry.js bakes at their place) at the tone of the
//! crown's outer leaves, lighter on the lumps and darker in their hollows;
//! the renderer multiplies in the leaf texture's mean colour, as the cards
//! have it from their texels, and draws the leaves on the hull
//! (props_draw.wgsl `pr_crown_leaves`). `part` 2 marks a core vertex; UVs
//! are left zero.

use super::EzMesh;

/// `--tree-crowns` sets `EO_TREE_CROWNS=on`, `--no-tree-crowns` sets it `off`.
pub fn tree_crowns_enabled() -> bool {
    std::env::var("EO_TREE_CROWNS").is_ok_and(|v| v == "on" || v == "1")
}

/// `EzMesh::part` of a crown-core vertex.
pub const PART_CORE: u8 = 2;

/// The hull's subdivisions in the window form (320 triangles, drawn 70 m to 200 m)
/// and the far form (80 triangles, to the distant cards).
pub const WINDOW_SUBDIVISIONS: u32 = 2;
pub const FAR_SUBDIVISIONS: u32 = 1;
/// The hull's reach, of the cards' (`REACH_QUANTILE` of them in its direction):
/// the cards past that quantile stand out of it and break its outline.
pub const INSET: f64 = 0.85;

/// The quantile of the card vertices' reach a hull vertex takes in its cone.
const REACH_QUANTILE: f64 = 0.85;
/// Card vertices a cone needs before its reach counts.
const MIN_CONE_POINTS: usize = 3;
/// The clumps a crown is made of: a smooth lump pattern over the hull's
/// directions (`LUMP_FREQ` per unit of the unit sphere, six or seven lumps
/// round the crown), `LUMP_DEPTH` of the reach either way; the lumps' hollows
/// `LUMP_SHADE` darker than their tops (the light gets less far into them).
const LUMP_FREQ: f64 = 6.0;
const LUMP_DEPTH: f64 = 0.2;
const LUMP_SHADE: f32 = 0.3;
/// Normal weights: the hull's own, the crown ellipsoid's outward, up.
const NORMAL_HULL: f64 = 0.55;
const NORMAL_CROWN: f64 = 0.45;
const NORMAL_UP: f64 = 0.3;

/// A geodesic unit sphere: the icosahedron, each face split in four
/// `subdivisions` times. Vertices, triangles (outward, counter-clockwise).
pub(crate) fn geodesic(subdivisions: u32) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let mut v: Vec<[f64; 3]> = [
        [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
        [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
        [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
    ]
    .iter()
    .map(|p| norm(*p))
    .collect();
    let mut f: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut mid = std::collections::HashMap::new();
        let mut next = Vec::with_capacity(f.len() * 4);
        let mut half = |a: u32, b: u32, v: &mut Vec<[f64; 3]>| -> u32 {
            *mid.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let (p, q) = (v[a as usize], v[b as usize]);
                v.push(norm([p[0] + q[0], p[1] + q[1], p[2] + q[2]]));
                v.len() as u32 - 1
            })
        };
        for [a, b, c] in f {
            let (ab, bc, ca) = (half(a, b, &mut v), half(b, c, &mut v), half(c, a, &mut v));
            next.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        f = next;
    }
    (v, f)
}

/// geometry.js `remapLeaves`' baked tone of a leaf at `outer` (0 the crown's
/// centre, 1 its box) and `dy` (its height in the box, -1..1).
fn tone(outer: f64, dy: f64) -> f64 {
    (0.45 + 0.5 * outer + 0.14 * dy).clamp(0.3, 1.12)
}

fn norm(p: [f64; 3]) -> [f64; 3] {
    let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-12);
    [p[0] / l, p[1] / l, p[2] / l]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The radius of the disc with the area a crown core covers seen from above
/// (its outline's convex hull), metres; 0 for an empty core.
pub fn footprint_radius(core: &EzMesh) -> f64 {
    let mut p: Vec<[f64; 2]> = core.positions.iter().map(|q| [q[0] as f64, q[2] as f64]).collect();
    if p.len() < 3 {
        return 0.0;
    }
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let turn = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    // Andrew's monotone chain: the lower hull left to right, then the upper back.
    let mut hull: Vec<[f64; 2]> = vec![];
    for q in p.iter().chain(p.iter().rev()) {
        while hull.len() >= 2 && turn(hull[hull.len() - 2], hull[hull.len() - 1], *q) <= 0.0 {
            hull.pop();
        }
        hull.push(*q);
    }
    let area: f64 = hull.windows(2).map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1]).sum::<f64>().abs() / 2.0;
    (area / std::f64::consts::PI).sqrt()
}

/// The crown core of a street form (its window or far form; see the module
/// note): a closed hull of `20 * 4^subdivisions` triangles round the leaf
/// cards' cloud, `inset` of their reach. Empty when the form has too few leaf
/// cards to make a crown (the bare pollard's far form keeps none).
pub fn crown_core(form: &EzMesh, subdivisions: u32, inset: f64) -> EzMesh {
    // The leaf vertices the form draws (a far form keeps unreferenced ones).
    let mut used = vec![false; form.positions.len()];
    for t in form.indices.chunks_exact(3) {
        if t.iter().any(|&v| form.part.get(v as usize).copied().unwrap_or(0) == 1) {
            for &v in t {
                used[v as usize] = true;
            }
        }
    }
    let leaf: Vec<usize> = (0..form.positions.len()).filter(|&i| used[i] && form.part[i] == 1).collect();
    if leaf.len() < 4 * MIN_CONE_POINTS {
        return EzMesh::default();
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for &i in &leaf {
        for k in 0..3 {
            lo[k] = lo[k].min(form.positions[i][k] as f64);
            hi[k] = hi[k].max(form.positions[i][k] as f64);
        }
    }
    let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let half = [((hi[0] - lo[0]) / 2.0).max(0.05), ((hi[1] - lo[1]) / 2.0).max(0.05), ((hi[2] - lo[2]) / 2.0).max(0.05)];
    // Each leaf vertex in the crown's ellipsoidal frame: its direction, its reach
    // and its tint (its colour over the tone baked at its place).
    let pts: Vec<([f64; 3], f64, [f32; 3])> = leaf
        .iter()
        .map(|&i| {
            let p = form.positions[i];
            let q = [(p[0] as f64 - centre[0]) / half[0], (p[1] as f64 - centre[1]) / half[1], (p[2] as f64 - centre[2]) / half[2]];
            let r = dot(q, q).sqrt();
            let t = tone(r.min(1.0), q[1]) as f32;
            let c = form.colors[i];
            (if r > 1e-9 { [q[0] / r, q[1] / r, q[2] / r] } else { [0.0, 1.0, 0.0] }, r, [c[0] / t, c[1] / t, c[2] / t])
        })
        .collect();
    let (dirs, tris) = geodesic(subdivisions);
    let n = dirs.len();
    let mut neighbours: Vec<Vec<usize>> = vec![vec![]; n];
    for t in &tris {
        for k in 0..3 {
            let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
            if !neighbours[a].contains(&b) {
                neighbours[a].push(b);
                neighbours[b].push(a);
            }
        }
    }
    // The cone round a direction: a little wider than the spacing of the sphere's vertices
    // (the icosahedron's edge subtends 63.4 degrees, halved by each subdivision).
    let cone = (63.43f64.to_radians() / f64::from(1u32 << subdivisions) * 1.25).cos();
    let mut reach: Vec<Option<f64>> = vec![None; n];
    let mut colour = vec![[0f32; 3]; n];
    let mut coloured = vec![false; n];
    for (i, d) in dirs.iter().enumerate() {
        let mut along = vec![];
        let mut sum = [0f32; 3];
        for (u, r, c) in &pts {
            if dot(*u, *d) >= cone {
                along.push(r * dot(*u, *d));
                for k in 0..3 {
                    sum[k] += c[k];
                }
            }
        }
        if !along.is_empty() {
            let k = along.len() as f32;
            colour[i] = [sum[0] / k, sum[1] / k, sum[2] / k];
            coloured[i] = true;
        }
        if along.len() >= MIN_CONE_POINTS {
            along.sort_by(|a, b| a.total_cmp(b));
            let at = ((along.len() - 1) as f64 * REACH_QUANTILE).round() as usize;
            reach[i] = Some(along[at]);
        }
    }
    // A direction with too few cards takes its neighbours' reach (repeatedly, outward from the known).
    for _ in 0..n {
        let missing: Vec<usize> = (0..n).filter(|&i| reach[i].is_none()).collect();
        if missing.is_empty() {
            break;
        }
        let mut filled = false;
        for i in missing {
            let known: Vec<f64> = neighbours[i].iter().filter_map(|&j| reach[j]).collect();
            if !known.is_empty() {
                reach[i] = Some(known.iter().sum::<f64>() / known.len() as f64);
                filled = true;
            }
        }
        if !filled {
            break;
        }
    }
    let mut r: Vec<f64> = reach.iter().map(|x| x.unwrap_or(0.5)).collect();
    for _ in 0..2 {
        let prev = r.clone();
        for i in 0..n {
            let m = neighbours[i].iter().map(|&j| prev[j]).sum::<f64>() / neighbours[i].len().max(1) as f64;
            r[i] = 0.5 * prev[i] + 0.5 * m;
        }
    }
    // Colours where no card stood in the cone: the mean of the cloud.
    let mean = {
        let mut s = [0f32; 3];
        for (_, _, c) in &pts {
            for k in 0..3 {
                s[k] += c[k];
            }
        }
        let k = pts.len() as f32;
        [s[0] / k, s[1] / k, s[2] / k]
    };
    // The lumps: a phase per crown from its size (a fixed pattern per form, varied between species).
    let phase = [half[0] * 7.31 + centre[1] * 1.7, half[1] * 3.17 + 0.9, half[2] * 5.53 + 2.3];
    let lump = |d: [f64; 3]| (LUMP_FREQ * d[0] + phase[0]).sin() * (LUMP_FREQ * d[1] + phase[1]).sin() * (LUMP_FREQ * d[2] + phase[2]).sin();
    let mut out = EzMesh::default();
    for i in 0..n {
        let d = dirs[i];
        let l = lump(d);
        let k = r[i] * inset * (1.0 + LUMP_DEPTH * l);
        out.positions.push([(centre[0] + d[0] * k * half[0]) as f32, (centre[1] + d[1] * k * half[1]) as f32, (centre[2] + d[2] * k * half[2]) as f32]);
        let c = if coloured[i] { colour[i] } else { mean };
        // the core stands for the crown's outer leaves: their tone.
        let shade = tone(1.0, d[1] * k) as f32 * (1.0 + LUMP_SHADE * 0.5 * l as f32);
        out.colors.push([c[0] * shade, c[1] * shade, c[2] * shade]);
        out.uvs.push([0.0, 0.0]);
        out.part.push(PART_CORE);
    }
    for t in &tris {
        out.indices.extend_from_slice(t);
    }
    // Normals: the hull's (area-weighted), the crown ellipsoid's outward and up.
    let mut acc = vec![[0f64; 3]; n];
    for t in &tris {
        let p = |v: u32| out.positions[v as usize].map(f64::from);
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let f = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        for &v in t {
            for k in 0..3 {
                acc[v as usize][k] += f[k];
            }
        }
    }
    for i in 0..n {
        let d = dirs[i];
        let hull = norm(acc[i]);
        // the hull is star-shaped about the centre: its normal faces out, as the direction does.
        let hull = if dot(hull, d) < 0.0 { [-hull[0], -hull[1], -hull[2]] } else { hull };
        let crown = norm([d[0] / half[0], d[1] / half[1], d[2] / half[2]]);
        let nrm = norm([
            NORMAL_HULL * hull[0] + NORMAL_CROWN * crown[0],
            NORMAL_HULL * hull[1] + NORMAL_CROWN * crown[1] + NORMAL_UP,
            NORMAL_HULL * hull[2] + NORMAL_CROWN * crown[2],
        ]);
        out.normals.push(nrm.map(|x| x as f32));
    }
    out.user = form.user;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footprint_of_a_disc() {
        let mut m = EzMesh::default();
        for k in 0..360 {
            let a = (k as f64).to_radians();
            m.positions.push([(2.0 * a.cos()) as f32, 0.0, (2.0 * a.sin()) as f32]);
        }
        assert!((footprint_radius(&m) - 2.0).abs() < 1e-3);
        assert_eq!(footprint_radius(&EzMesh::default()), 0.0);
    }

    /// Every leafy street species' window and far forms get a closed core inside
    /// their leaf cloud's box, coloured from its cards; the leafless pollard's far
    /// form (no cards) none. Deterministic.
    #[test]
    fn cores_of_the_street_forms() {
        let lib = super::super::cook_all(None).unwrap();
        for sp in super::super::street_tree_species().unwrap() {
            let leafy = super::super::ez_species(sp).unwrap().crown_cover;
            for (form, sub) in [(super::super::StreetForm::Window, WINDOW_SUBDIVISIONS), (super::super::StreetForm::Far, FAR_SUBDIVISIONS)] {
                let m = &lib.street_form(sp, form).unwrap().mesh;
                let c = crown_core(m, sub, INSET);
                if c.positions.is_empty() {
                    assert!(!leafy || m.indices.chunks_exact(3).filter(|t| m.part[t[0] as usize] == 1).count() < 12, "{sp} {form:?}: a leafy form has a core");
                    continue;
                }
                assert_eq!(c.indices.len() / 3, 20 * 4usize.pow(sub), "{sp} {form:?}");
                assert!(c.part.iter().all(|&p| p == PART_CORE));
                assert_eq!(c.positions.len(), c.normals.len());
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for (q, _) in m.positions.iter().zip(&m.part).filter(|(_, p)| **p == 1) {
                    for k in 0..3 {
                        lo[k] = lo[k].min(q[k]);
                        hi[k] = hi[k].max(q[k]);
                    }
                }
                for q in &c.positions {
                    for k in 0..3 {
                        let pad = (hi[k] - lo[k]) * 0.25;
                        assert!(q[k] >= lo[k] - pad && q[k] <= hi[k] + pad, "{sp} {form:?}: the core keeps to its cards' cloud");
                    }
                }
                for n in &c.normals {
                    assert!((n[0] * n[0] + n[1] * n[1] + n[2] * n[2] - 1.0).abs() < 1e-3);
                }
                assert!(c.colors.iter().all(|c| c.iter().all(|v| v.is_finite() && *v > 0.0)), "{sp} {form:?}: coloured from its cards");
                let again = crown_core(m, sub, INSET);
                assert_eq!(again.positions, c.positions, "deterministic");
                assert!(footprint_radius(&c) > 0.5, "{sp} {form:?}");
            }
        }
    }

    #[test]
    fn geodesic_is_closed_and_outward() {
        for s in 0..3 {
            let (v, f) = geodesic(s);
            assert_eq!(f.len(), 20 * 4usize.pow(s));
            assert_eq!(v.len(), 10 * 4usize.pow(s) + 2, "Euler: V = F / 2 + 2");
            let mut edges = std::collections::HashMap::new();
            for t in &f {
                for k in 0..3 {
                    *edges.entry((t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]))).or_insert(0) += 1;
                    let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
                    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                    assert!(dot(n, a) > 0.0, "counter-clockwise seen from outside");
                }
            }
            assert!(edges.values().all(|&c| c == 2), "every edge has two faces");
        }
    }
}
