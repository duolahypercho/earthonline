//! Stage A9: retaining walls where the street meets the structures
//! (`--structures`; only on a street built with the joins, `build_joined`).
//!
//! * Along a paved stub the joins lower into a tunnel's approach cut
//!   (`eo_street::joins`, the runs with `Entry::structure_ends` /
//!   `structure_ramps`): where the stub stands under its own street datum
//!   (the level the ground beside it stood at) by more than one kerb face (the
//!   street's own retaining-edge rule, `curb_face_height`), a cut wall:
//!   concrete from under the paved edge up to that datum plus a parapet, with
//!   a coping; the street's carpet, held at the stub's level, would otherwise
//!   rise to the hillside as a steep grass bank (up to 14 m at MacArthur, A8).
//! * Where the joins raise a stub over its datum by more than its bank can
//!   carry (`verge_max_drop`), a fill wall: its outer face past the bank's
//!   reach, its top flush with the edge, a parapet of the Type 60 barrier's
//!   height on its outer edge; the steep earth bank stands behind it.
//! * Under a kerbless side the street draws no bank for (a stub at its
//!   datum), where its edge stands over the carpet held under it, an edge
//!   beam down to the ground: the stub's underside showed there from below
//!   (the dark wedge at the Presidio Parkway's chain 203, A8).
//! * No wall where other paving or a building's footprint lies just beyond
//!   the edge (the frontage and the building hold the ground there).
//! * Over the vertical faces the street draws round a junction with a
//!   structure's leg (`eo_street` `ring_edge`: a face where its bank would
//!   lie over the structure's fill), where the structure's own concrete does
//!   not stand: a wall with a coping, so the face is the front of a retaining
//!   wall holding the junction's edge, not a lone panel on the grass (the
//!   leg-214 junction, A8).
//!
//! The walls are concrete in their structure's tint and era, drawn and
//! collided as the structures' own (`mesh::collide`: barrier). Sizes are the
//! tunnels' (`tunnel::WALL_M`, `PARAPET_M`, `EMBED_M`, INFERRED there), the
//! barrier's from the table (`Dims`), the coping's below.

use eo_street::{Layer, StreetWorld};

use crate::mesh::{Builder, V3, norm};
use crate::section::Dims;
use crate::StructureClass;

/// A coping: this deep over the wall's top and this far proud of each face.
/// INFERRED, structure-only (a cast-in-place coping cap).
pub(crate) const COPING_H_M: f64 = 0.15;
pub(crate) const COPING_OUT_M: f64 = 0.06;
/// A wall stands this far off the street's edge (no fighting with the
/// street's own faces there). A drawing offset.
const GAP_M: f64 = 0.03;
/// A stub's edge is read this far inside it, the ground beside it this far
/// out past the wall (the bare earth, not the carpet held under the stub).
const EDGE_IN_M: f64 = 0.05;
const BESIDE_M: f64 = 0.6;
/// A face round a leg junction this tall or taller is one of the street's
/// leg faces (a kerb face is a fraction of this). INFERRED, a drawing rule.
const LEG_FACE_MIN_M: f64 = 0.6;
/// Walls shorter than this are left out. A drawing rule.
const WALL_MIN_LEN_M: f64 = 1.0;
/// A stub is read (and its walls sectioned) at most this far apart. A
/// drawing step (the street's own stations are `max_step`, 6 m, apart).
const SUB_STEP_M: f64 = 3.0;
/// An edge beam's width (`Kind::Skirt`), and the least gap under a
/// kerbless edge it closes. INFERRED, structure-only / a drawing rule.
const SKIRT_M: f64 = 0.3;
const SKIRT_MIN_M: f64 = 0.05;

/// What a wall holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The ground beside stands over the street: the wall holds the hill back.
    Cut,
    /// The street stands over the ground beside: the wall holds its fill.
    Fill,
    /// Over a street's leg face: the wall holds the junction's edge.
    Leg,
    /// Under a kerbless edge standing over the ground by less than a bank
    /// carries (the street draws no bank there): a concrete edge beam down
    /// to the ground, its top flush with the road.
    Skirt,
}

/// One section of a wall: its face on the street's side at `p` (plan),
/// `o` the unit direction away from the street, `s` the running length (the
/// formwork's `u`), `edge` the street's edge height there, `top` the wall's
/// top (a cut: the ground beside plus the parapet; a fill or a leg wall: the
/// edge), `foot` its buried foot, `thick` its depth away from the street.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sec {
    pub p: [f64; 2],
    pub o: [f64; 2],
    pub s: f64,
    pub edge: f64,
    pub top: f64,
    pub foot: f64,
    pub thick: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Wall {
    pub kind: Kind,
    pub era: u8,
    pub rgb: [u8; 3],
    pub secs: Vec<Sec>,
}

impl Wall {
    /// The plan point of section `i`'s middle (its tile).
    pub fn mid(&self, i: usize) -> [f64; 2] {
        let s = &self.secs[i];
        [s.p[0] + s.o[0] * s.thick / 2.0, s.p[1] + s.o[1] * s.thick / 2.0]
    }
}

/// The bare ground (the carpet without the street's hold) at (x, z).
fn natural(streets: &StreetWorld, x: f64, z: f64) -> f64 {
    streets.terrain.grid.ground(x, z) + streets.terrain.carpet_lift
}

/// A run's stations at most `SUB_STEP_M` apart (frames and datums
/// interpolated between its own), each with the index of the nearer of its
/// own stations.
fn frames(st: &[eo_street::plan::Frame], datums: &[f64]) -> Vec<(usize, eo_street::plan::Frame, f64)> {
    let mut out = vec![];
    for i in 0..st.len() {
        out.push((i, st[i], datums[i]));
        if i + 1 == st.len() {
            break;
        }
        let (a, b) = (&st[i], &st[i + 1]);
        let n = ((b.s - a.s) / SUB_STEP_M).ceil().max(1.0) as usize;
        for k in 1..n {
            let t = k as f64 / n as f64;
            let l = |p: f64, q: f64| p + (q - p) * t;
            let (nx, nz) = (l(a.nx, b.nx), l(a.nz, b.nz));
            let nl = nx.hypot(nz).max(1e-12);
            let f = eo_street::plan::Frame { s: l(a.s, b.s), x: l(a.x, b.x), z: l(a.z, b.z), nx: nx / nl, nz: nz / nl, miter: l(a.miter, b.miter), tx: l(a.tx, b.tx), tz: l(a.tz, b.tz) };
            out.push((if t < 0.5 { i } else { i + 1 }, f, l(datums[i], datums[i + 1])));
        }
    }
    out
}

/// The walls along the joined street's stubs (`Kind::Cut`, `Kind::Fill`,
/// `Kind::Skirt`).
/// `structure`: a joined structure end's (`StructureJoins::ends` index)
/// chain's era and tint.
pub(crate) fn stub_walls(streets: &StreetWorld, dims: &Dims, structure: &dyn Fn(usize) -> Option<(u8, [u8; 3])>) -> Vec<Wall> {
    let Some(joins) = streets.o.structures.as_ref() else { return vec![] };
    let o = &streets.o;
    let mut out = vec![];
    for (ei, e) in streets.plan.entries.iter().enumerate() {
        let ends: Vec<usize> = e.structure_ends.iter().flatten().copied().chain(e.structure_ramps.iter().map(|r| r.0)).collect();
        let Some(&j) = ends.first() else { continue };
        let Some(g) = streets.geoms.get(ei) else { continue };
        if g.stations.len() < 2 || j >= joins.ends.len() {
            continue;
        }
        let Some((era, rgb)) = ends.iter().find_map(|&j| structure(j)) else { continue };
        for (k, side) in [(0usize, 1.0f64), (1, -1.0)] {
            let walk = if e.walks[k] >= o.min_sidewalk_width { e.walks[k] } else { 0.0 };
            let mut run: Vec<(Kind, Sec)> = vec![];
            let mut flush = |run: &mut Vec<(Kind, Sec)>| {
                if run.len() >= 2 && run.last().unwrap().1.s - run[0].1.s >= WALL_MIN_LEN_M {
                    out.push(Wall { kind: run[0].0, era, rgb, secs: run.iter().map(|x| x.1).collect() });
                }
                run.clear();
            };
            for (i, st, datum) in frames(&g.stations, &g.datums) {
                let oo = norm([side * st.nx, 0.0, side * st.nz]);
                let o2 = [oo[0], oo[2]];
                let at = |w: f64| st.at(side * (e.half + walk + w / st.miter.max(1e-6)));
                let inside = at(-EDGE_IN_M);
                // the edge as drawn (a kerbless edge at the stub's very end, where the
                // street's cap stands back from it: its section's height).
                let Some(edge) = streets.paved_at(inside[0], inside[1]).or_else(|| (walk == 0.0).then(|| o.cross_section_y(datum, e.half, e.half))) else {
                    flush(&mut run);
                    continue;
                };
                // a building's wall, or other paving (frontage, another street), beside: no wall.
                let beside = at(GAP_M + 0.3);
                let paved_beyond = matches!(streets.paved_layer_at(beside[0], beside[1]), Some(l) if l != Layer::Verge);
                let building = (1..=8).any(|k| {
                    let q = at(GAP_M + 0.5 * k as f64);
                    streets.buildings.contains(q[0], q[1])
                });
                if paved_beyond || building {
                    flush(&mut run);
                    continue;
                }
                // how far the joins raised (+) or lowered (-) the stub here from its
                // street datum (the ground beside it stands at about that datum).
                let raise = datum - (streets.ground(st.x, st.z) + o.road_lift);
                let _ = i;
                let raised = raise > 0.02;
                let bank = walk > 0.0 || raised;
                let ground_at = |w: f64| {
                    let q = at(w);
                    natural(streets, q[0], q[1])
                };
                let s = st.s;
                let fill_thick = if bank { o.verge_reach } else { 0.0 } + crate::tunnel::WALL_M;
                let low = [0.5 * fill_thick, fill_thick + BESIDE_M].iter().map(|&w| ground_at(w)).fold(f64::INFINITY, f64::min);
                // (a kerbless side at its datum has no bank: any drop shows its underside.)
                // (the carpet the street holds under its edge, as drawn, or the bare ground
                // there, whichever is lower; never deeper than a bank would carry.)
                let skirt_low = {
                    let q = at(0.5 * SKIRT_M);
                    ground_at(0.5 * SKIRT_M).min(streets.carpet_y(q[0], q[1])).max(edge - o.verge_max_drop)
                };
                let p = at(GAP_M);
                let kind_sec = if -raise > o.curb_face_height {
                    // lowered into a cut: the wall holds the ground at the street's own datum.
                    let top = edge - raise + crate::tunnel::PARAPET_M;
                    Some((Kind::Cut, Sec { p, o: o2, s, edge, top, foot: edge - crate::tunnel::EMBED_M, thick: crate::tunnel::WALL_M }))
                } else if raise > o.verge_max_drop && edge - low > o.verge_max_drop {
                    // raised past what its bank carries: the wall holds the fill.
                    Some((Kind::Fill, Sec { p, o: o2, s, edge, top: edge + dims.barrier_h, foot: low - crate::tunnel::EMBED_M, thick: fill_thick }))
                } else if !bank && edge - skirt_low > SKIRT_MIN_M {
                    Some((Kind::Skirt, Sec { p: at(0.0), o: o2, s, edge, top: edge, foot: skirt_low - crate::tunnel::EMBED_M, thick: SKIRT_M }))
                } else {
                    None
                };
                match kind_sec {
                    Some((kind, sec)) => {
                        if run.last().is_some_and(|x| x.0 != kind) {
                            flush(&mut run);
                        }
                        run.push((kind, sec));
                    }
                    None => flush(&mut run),
                }
            }
            flush(&mut run);
        }
    }
    out
}

/// The walls over the street's leg faces round each junction with a
/// structure's leg (`Kind::Leg`), where the structures' own concrete does not
/// stand (`blocked`: `StructureWorld::blocked`). `structure`: a leg's (`StructureJoins::ends` index)
/// chain's era and tint.
pub(crate) fn leg_walls(streets: &StreetWorld, blocked: &dyn Fn(f64, f64, f64, f64, f64) -> bool, structure: &dyn Fn(usize) -> Option<(u8, [u8; 3])>) -> Vec<Wall> {
    let o = &streets.o;
    let plan = &streets.plan;
    let mut out = vec![];
    let mut seen: std::collections::HashSet<((i32, i32), u32)> = std::collections::HashSet::new();
    for node in &plan.nodes {
        let Some(leg) = node.approaches.iter().find_map(|a| plan.entries[a.entry].structure_leg) else { continue };
        let Some((era, rgb)) = structure(leg) else { continue };
        let reach = node.approaches.iter().map(|a| a.trim).fold(0.0, f64::max) + o.verge_reach + o.max_step + 4.0 * o.max_step;
        let (cx, cz) = (node.position[0], node.position[1]);
        let k0 = eo_street::tile_key(cx - reach, cz - reach);
        let k1 = eo_street::tile_key(cx + reach, cz + reach);
        for kz in k0.1..=k1.1 {
            for kx in k0.0..=k1.0 {
                let m = streets.cached_mesh((kx, kz));
                for (t, layer) in m.indices.chunks_exact(3).zip(&m.layers) {
                    if *layer != Layer::CurbFace {
                        continue;
                    }
                    // a quad's four vertices follow its first (`TileMesh::quad`).
                    let base = t[0];
                    if base as usize + 3 >= m.positions.len() || t.iter().any(|&i| i < base || i > base + 3) {
                        continue;
                    }
                    if !seen.insert(((kx, kz), base)) {
                        continue;
                    }
                    let q: Vec<V3> = (0..4).map(|k| m.positions[base as usize + k].map(|v| v as f64)).collect();
                    let n = m.normals[base as usize].map(|v| v as f64);
                    if n[1].abs() > 0.2 {
                        continue;
                    }
                    // the top edge (the two higher corners) and the foot.
                    let mut order = [0usize, 1, 2, 3];
                    order.sort_by(|a, b| q[*b][1].total_cmp(&q[*a][1]));
                    let (ta, tb) = (q[order[0]], q[order[1]]);
                    let foot = q[order[2]][1].min(q[order[3]][1]);
                    let top = ta[1].min(tb[1]);
                    if top - foot < LEG_FACE_MIN_M {
                        continue;
                    }
                    let mid = [(ta[0] + tb[0]) / 2.0, (ta[2] + tb[2]) / 2.0];
                    if (mid[0] - cx).hypot(mid[1] - cz) > reach {
                        continue;
                    }
                    // Some street faces point into their paving. Reverse the
                    // normal only when the opposite side has paving at the
                    // wall top. On a steep or narrow edge both probes can
                    // miss that level, and the mesh normal then owns the side.
                    let mut o2 = norm([n[0], 0.0, n[2]]);
                    let paving = |d: f64| {
                        streets.paved_at(mid[0] + o2[0] * d, mid[1] + o2[2] * d)
                    };
                    if paving(-0.3).is_none_or(|y| y < top - 0.5)
                        && paving(0.3).is_some_and(|y| (y - top).abs() <= 0.5)
                    {
                        o2 = [-o2[0], 0.0, -o2[2]];
                    }
                    // a face between two pavings (a step the street closes), not over open ground: no wall.
                    let beyond = [mid[0] + o2[0] * (crate::tunnel::WALL_M + 0.3), mid[1] + o2[2] * (crate::tunnel::WALL_M + 0.3)];
                    if matches!(streets.paved_layer_at(beyond[0], beyond[1]), Some(l) if l != Layer::Verge) {
                        continue;
                    }
                    // where the structure's own concrete stands, it is that wall.
                    if blocked(mid[0] + o2[0] * 0.4, mid[1] + o2[2] * 0.4, 0.35, foot, top) {
                        continue;
                    }
                    let (a, b) = ([ta[0] + o2[0] * GAP_M, ta[2] + o2[2] * GAP_M], [tb[0] + o2[0] * GAP_M, tb[2] + o2[2] * GAP_M]);
                    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                    if len < 0.2 {
                        continue;
                    }
                    let o = [o2[0], o2[2]];
                    let sec = |p: [f64; 2], s: f64, y: f64| Sec { p, o, s, edge: y, top: y, foot: foot - 0.2, thick: crate::tunnel::WALL_M };
                    out.push(Wall { kind: Kind::Leg, era, rgb, secs: vec![sec(a, 0.0, ta[1]), sec(b, len, tb[1])] });
                }
            }
        }
    }
    out
}

/// One prism swept along the sections: `profile` gives a section's convex
/// outline (offset away from the street, height) with each point's wear;
/// faces look out of it. The outline's closing edge (its last point back to
/// its first: every wall's is its buried foot or what it sits on) is not
/// swept; its ends are closed.
fn prism(b: &mut Builder, w: &Wall, rgb: [u8; 3], profile: &dyn Fn(&Sec) -> Vec<(f64, f64, f64)>) {
    let profs: Vec<Vec<(f64, f64, f64)>> = w.secs.iter().map(profile).collect();
    let np = profs.iter().map(|p| p.len()).min().unwrap_or(0);
    if np < 3 {
        return;
    }
    let at = |sec: &Sec, d: f64, y: f64| -> V3 { [sec.p[0] + sec.o[0] * d, y, sec.p[1] + sec.o[1] * d] };
    let centre = |p: &[(f64, f64, f64)]| {
        let n = p.len() as f64;
        (p.iter().map(|q| q.0).sum::<f64>() / n, p.iter().map(|q| q.1).sum::<f64>() / n)
    };
    for j in 0..np - 1 {
        let jn = j + 1;
        let mut prev: Option<(u32, u32, V3)> = None;
        for (i, sec) in w.secs.iter().enumerate() {
            let (a, c) = (profs[i][j], profs[i][jn]);
            let (cd, cy) = centre(&profs[i]);
            // the edge's normal in (offset, height), away from the outline's middle.
            let (dd, dy) = (c.0 - a.0, c.1 - a.1);
            let mut n2 = [-dy, dd];
            let (md, my) = ((a.0 + c.0) / 2.0 - cd, (a.1 + c.1) / 2.0 - cy);
            if n2[0] * md + n2[1] * my < 0.0 {
                n2 = [dy, -dd];
            }
            let n3 = norm([n2[0] * sec.o[0], n2[1], n2[0] * sec.o[1]]);
            let flat = dd.abs() >= dy.abs();
            let uv = |q: (f64, f64, f64)| if flat { [sec.s, q.0] } else { [sec.s, q.1 - sec.top] };
            let ia = b.v(at(sec, a.0, a.1), n3, rgb, a.2, uv(a), StructureClass::Concrete);
            let ic = b.v(at(sec, c.0, c.1), n3, rgb, c.2, uv(c), StructureClass::Concrete);
            if let Some((pa, pc, pn)) = prev {
                b.quad([pa, pc, ic, ia], [pn[0] + n3[0], pn[1] + n3[1], pn[2] + n3[2]]);
            }
            prev = Some((ia, ic, n3));
        }
    }
    // the two ends.
    let n = w.secs.len();
    for (i, sgn) in [(0usize, -1.0f64), (n - 1, 1.0)] {
        let sec = &w.secs[i];
        let other = &w.secs[if i == 0 { 1 } else { n - 2 }];
        let along = norm([sec.p[0] - other.p[0], 0.0, sec.p[1] - other.p[1]]);
        let _ = sgn;
        let p = &profs[i];
        let (cd, cy) = centre(p);
        let vc = b.v(at(sec, cd, cy), along, rgb, 0.45, [cd, cy - sec.top], StructureClass::Concrete);
        let vs: Vec<u32> = p.iter().map(|q| b.v(at(sec, q.0, q.1), along, rgb, q.2, [q.0, q.1 - sec.top], StructureClass::Concrete)).collect();
        for k in 0..vs.len() {
            b.tri([vc, vs[k], vs[(k + 1) % vs.len()]], along);
        }
    }
}

/// Sections `q0..=q1` of wall `w` at a level (`lod`: the coping and the
/// parapet from the far level up, the body alone at the coarse).
pub(crate) fn draw(b: &mut Builder, w: &Wall, q0: usize, q1: usize, lod: u8, dims: &Dims) {
    if q1 <= q0 || q1 >= w.secs.len() {
        return;
    }
    // the far level: every 2nd section, the bodies alone; the coarse: every
    // 4th, the cut and fill walls' bodies alone (an edge beam or a leg's wall
    // is under a pixel there).
    if lod >= 2 && matches!(w.kind, Kind::Skirt | Kind::Leg) {
        return;
    }
    let step = [1usize, 2, 4][lod.min(2) as usize];
    let mut ids: Vec<usize> = (q0..q1).step_by(step).collect();
    ids.push(q1);
    let part = Wall { kind: w.kind, era: w.era, rgb: w.rgb, secs: ids.iter().map(|&i| w.secs[i]).collect() };
    b.era = w.era;
    let rgb = w.rgb;
    let near = lod == 0;
    // wear: run-off from the top, splash and soil at the foot (the shader reads it).
    match w.kind {
        Kind::Cut | Kind::Leg => {
            // the body: the face from its foot (a splash band over the edge) up to the top.
            prism(b, &part, rgb, &|s| {
                let splash = (s.edge + 1.0).min(s.top - 0.05).max(s.foot + 0.05);
                if near {
                    vec![(0.0, s.foot, 0.6), (0.0, splash, 0.3), (0.0, s.top, 0.5), (s.thick, s.top, 0.5), (s.thick, s.foot, 0.4)]
                } else {
                    vec![(0.0, s.foot, 0.6), (0.0, s.top, 0.5), (s.thick, s.top, 0.5), (s.thick, s.foot, 0.4)]
                }
            });
            if near {
                let (c, h) = (COPING_OUT_M, COPING_H_M);
                prism(b, &part, rgb, &|s| vec![(-c, s.top - 0.02, 0.4), (-c, s.top + h, 0.35), (s.thick + c, s.top + h, 0.35), (s.thick + c, s.top - 0.02, 0.4)]);
            }
        }
        Kind::Skirt => {
            prism(b, &part, rgb, &|s| vec![(0.0, s.foot, 0.5), (0.0, s.edge + 0.005, 0.45), (s.thick, s.edge + 0.005, 0.45), (s.thick, s.foot, 0.6)]);
        }
        Kind::Fill => {
            // the body, its top flush with the edge (the bank behind it), a parapet on its outer edge.
            prism(b, &part, rgb, &|s| {
                if near {
                    vec![(0.0, s.foot, 0.4), (0.0, s.edge + 0.01, 0.4), (s.thick, s.edge + 0.01, 0.35), (s.thick, (s.edge - 1.0).max(s.foot + 0.05), 0.3), (s.thick, s.foot, 0.6)]
                } else {
                    vec![(0.0, s.foot, 0.4), (0.0, s.edge + 0.01, 0.4), (s.thick, s.edge + 0.01, 0.35), (s.thick, s.foot, 0.6)]
                }
            });
            if lod < 2 {
                let pw = dims.barrier_top.max(0.15);
                prism(b, &part, rgb, &|s| vec![(s.thick - pw, s.edge, 0.45), (s.thick - pw, s.top, 0.2), (s.thick, s.top, 0.2), (s.thick, s.edge, 0.45)]);
            }
        }
    }
}
