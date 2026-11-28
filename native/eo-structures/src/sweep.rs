//! The stations a chain's section is swept along, with everything the
//! meshes read at each: a true perpendicular frame (mitred at the
//! centreline's vertices), the deck top and grade from the profile, the
//! roadway half width (tapered between pieces), the superstructure depth
//! (the span's depth, eased toward a shallower neighbour span near a bent),
//! the ground under the deck, whether the superstructure sits on approach
//! fill there, the street crown blended in toward a paved end, which
//! barriers stand (none across another deck's roadway at a merge or
//! diverge), and the wear the renderer reads (joints, drain streaks).

use eo_street::StreetWorld;
use eo_street::material::StreetMaterial;
use eo_street::plan::frame_at;

use crate::section::{Dims, Section, WIDTH_TAPER_M, section};
use crate::solve::FILL_CLEAR_M;
use crate::{Chain, ChainKind, EndKind};

/// Stations are no further apart than this (`STATION_MAX_M`; the profile's own).
const STEP_M: f64 = crate::STATION_MAX_M;

/// Toward a shallower neighbour span the superstructure eases its depth over
/// this length (at most a quarter of the span) on the deeper side of the
/// bent, so the soffit never steps and never drops under the depth the
/// profile was solved with. INFERRED, structure-only.
const DEPTH_EASE_M: f64 = 8.0;

/// The deck top blends from the street's crown and gutters (at a paved end)
/// to a flat deck over this length. INFERRED, structure-only.
pub(crate) const CROWN_BLEND_M: f64 = 12.0;

/// A barrier is left out where it would stand on another deck's roadway
/// within this far of a shared end (a merge or diverge gore), and the two
/// decks are within `GORE_DY_M` of each other there. INFERRED, structure-only.
const GORE_REACH_M: f64 = 150.0;
const GORE_DY_M: f64 = 2.0;

/// The drain inlet of a downspout bent is this far past the cap's face,
/// and its stain streak this wide either side. INFERRED, structure-only.
pub(crate) const DRAIN_PAST_CAP_M: f64 = 0.8;
pub(crate) const STREAK_HALF_M: f64 = 0.45;

/// Concrete tints (sRGB): light warm grey, "not blue-grey" (reference sheet
/// 5.10, I-280 at Selby St; new grey concrete reflects about 0.40, EPA
/// 2012, sheet 2.1); the Central Viaduct's yellow-pigmented concrete (sheet
/// 1.2, Roads & Bridges 2005); the new Presidio Parkway's very light, clean
/// concrete (sheet 5.10).
pub(crate) const CONCRETE_TINT: [u8; 3] = [176, 170, 159];
pub(crate) const CENTRAL_VIADUCT_TINT: [u8; 3] = [186, 172, 136];
pub(crate) const PRESIDIO_TINT: [u8; 3] = [200, 197, 190];

/// The deck top's lateral points across a roadway `half` wide with gutters
/// `gutter` wide (the near level's: edges, gutter lips, centreline).
pub(crate) fn deck_offsets(half: f64, gutter: f64) -> impl Iterator<Item = f64> {
    let (offsets, count) = if half > 2.0 * gutter {
        ([-half, -(half - gutter), 0.0, half - gutter, half], 5)
    } else {
        ([-half, 0.0, half, 0.0, 0.0], 3)
    };
    offsets.into_iter().take(count)
}

/// What a chain's meshes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Draw {
    /// A deck on bents: everything.
    Deck,
    /// An open road below other ways, at grade: deck surface, paint, its slab edge.
    AtGrade,
    /// A landmark's deck: collision only (the landmark draws it).
    Landmark,
    /// A tunnel (stage A7, only with `--tunnels`): its bore, cut and
    /// portals (`tunnel.rs`).
    Tunnel,
}

/// One station of the sweep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Station {
    pub s: f64,
    pub x: f64,
    pub z: f64,
    /// Unit normal (+n) and the offset per metre of `u` (the normal times the miter).
    pub nx: f64,
    pub nz: f64,
    pub ox: f64,
    pub oz: f64,
    /// Unit plan tangent and the grade along it.
    pub tx: f64,
    pub tz: f64,
    pub slope: f64,
    /// Deck top at the centreline.
    pub y: f64,
    pub half: f64,
    pub depth: f64,
    /// Ground under the deck: at -edge, centre, +edge, and the least of them.
    pub ground: [f64; 3],
    pub ground_min: f64,
    /// The superstructure sits on its approach fill here (side walls to the ground).
    pub fill: bool,
    /// How much of each paved end's street surface is blended into the deck
    /// top ([start, end]; 1 at that end, 0 past `CROWN_BLEND_M`).
    pub crown: [f64; 2],
    /// Barriers stand on the +n and -n edges.
    pub barrier: [bool; 2],
    /// Wear the renderer reads: nearness to an expansion joint (0..1), a drain
    /// streak on the fascia below a drain inlet (0..1, +n and -n).
    pub joint: f64,
    pub streak: [f64; 2],
    /// Where a merged deck (`Chain::merged_with`, the major) covers this
    /// minor deck's section (stage A5's gore): the lateral range of the deck
    /// top still drawn here (the major's surface is drawn over the rest; the
    /// full section when nothing covers it), and the major's surface height
    /// at each end of that range (NaN where the end is this deck's own edge).
    pub clip: [f64; 2],
    pub meet: [f64; 2],
}

impl Station {
    /// World position of lateral offset `u` at height `y`.
    pub fn at(&self, u: f64, y: f64) -> [f64; 3] {
        [self.x + self.ox * u, y, self.z + self.oz * u]
    }
}

/// A chain's sweep.
#[derive(Clone, Debug)]
pub(crate) struct Sweep {
    pub draw: Draw,
    pub st: Vec<Station>,
    /// Station indices each level of detail uses (near, far, coarse).
    pub levels: [Vec<usize>; 3],
    /// Each piece's section, and its carriageway tint.
    pub sections: Vec<Section>,
    pub asphalt: Vec<[u8; 3]>,
    pub east_pavement: bool,
    pub tint: [u8; 3],
    /// `StreetOptions` cross slope, gutter width and depth.
    pub cross: [f64; 3],
    /// Each paved end's street surface (`ChainEnd::surface`): (lateral offset, height over the end).
    pub surfaces: [Vec<[f64; 2]>; 2],
    /// Ends: [start, end] closing faces (`Free` and `Landmark` ends get an end
    /// face, `Paved` ends an end wall to the ground, `Shared` ends nothing).
    pub ends: [EndKind; 2],
    /// A tunnel's covered and open stations (`Draw::Tunnel` only).
    pub tunnel: Option<crate::tunnel::TunnelInfo>,
    /// Stage A9: the chain's `crate::Era` (as u8), carried by every vertex
    /// its meshes write (`mesh::Builder::era`).
    pub era: u8,
}

impl Sweep {
    /// What the deck top stands over its centreline height at offset `u` of
    /// a station: the paved ends' street surfaces, blended in toward them.
    pub fn crown_at(&self, st: &Station, u: f64) -> f64 {
        let mut v = 0.0;
        for e in 0..2 {
            let w = st.crown[e];
            let sf = &self.surfaces[e];
            if w <= 0.0 || sf.is_empty() {
                continue;
            }
            // the end's profile, scaled to this station's half width.
            let half_end = sf.last().map(|q| q[0]).unwrap_or(st.half).max(1e-6);
            let ue = u * half_end / st.half.max(1e-6);
            let mut y = sf[0][1];
            for q in sf.windows(2) {
                if ue >= q[0][0] && ue <= q[1][0] {
                    let t = (ue - q[0][0]) / (q[1][0] - q[0][0]).max(1e-9);
                    y = q[0][1] + (q[1][1] - q[0][1]) * t;
                    break;
                }
                if ue > q[1][0] {
                    y = q[1][1];
                }
            }
            v += w * y;
        }
        v
    }

    /// Deck top at offset `u` of a station: the profile with the street's crown
    /// blended in toward a paved end, and toward a merged major deck's
    /// surface where its fascia clips the section (so the two meet with no
    /// step: a linear warp from the far end of the drawn range).
    pub fn top(&self, st: &Station, u: f64) -> f64 {
        let y = st.y + self.crown_at(st, u);
        let mut warp = 0.0;
        for k in 0..2 {
            if !st.meet[k].is_finite() {
                continue;
            }
            let (ub, uf) = (st.clip[k], st.clip[1 - k]);
            let span = ub - uf;
            if span.abs() < 1e-6 {
                continue;
            }
            let t = ((u - uf) / span).clamp(0.0, 1.0);
            warp += (st.meet[k] - (st.y + self.crown_at(st, ub))) * t;
        }
        y + warp
    }

    /// `u` brought inside the drawn range of a station's deck top.
    pub fn clamp_u(&self, st: &Station, u: f64) -> f64 {
        u.clamp(st.clip[0], st.clip[1])
    }

    /// The station at `s` between stations (linear; the sweep is straight between them).
    pub fn lerp(&self, s: f64) -> Station {
        let n = self.st.len();
        let i = match self.st.binary_search_by(|t| t.s.total_cmp(&s)) {
            Ok(i) => return self.st[i],
            Err(0) => return self.st[0],
            Err(i) if i >= n => return self.st[n - 1],
            Err(i) => i - 1,
        };
        let (a, b) = (&self.st[i], &self.st[i + 1]);
        let t = if b.s > a.s { (s - a.s) / (b.s - a.s) } else { 0.0 };
        let l = |p: f64, q: f64| p + (q - p) * t;
        let (nx, nz) = (l(a.nx, b.nx), l(a.nz, b.nz));
        let nl = nx.hypot(nz).max(1e-12);
        Station {
            s,
            x: l(a.x, b.x),
            z: l(a.z, b.z),
            nx: nx / nl,
            nz: nz / nl,
            ox: l(a.ox, b.ox),
            oz: l(a.oz, b.oz),
            tx: l(a.tx, b.tx),
            tz: l(a.tz, b.tz),
            slope: l(a.slope, b.slope),
            y: l(a.y, b.y),
            half: l(a.half, b.half),
            depth: l(a.depth, b.depth),
            ground: [l(a.ground[0], b.ground[0]), l(a.ground[1], b.ground[1]), l(a.ground[2], b.ground[2])],
            ground_min: l(a.ground_min, b.ground_min),
            fill: a.fill && b.fill,
            crown: [l(a.crown[0], b.crown[0]), l(a.crown[1], b.crown[1])],
            barrier: [a.barrier[0] && b.barrier[0], a.barrier[1] && b.barrier[1]],
            joint: l(a.joint, b.joint),
            streak: [l(a.streak[0], b.streak[0]), l(a.streak[1], b.streak[1])],
            clip: [l(a.clip[0], b.clip[0]), l(a.clip[1], b.clip[1])],
            meet: [l(a.meet[0], b.meet[0]), l(a.meet[1], b.meet[1])],
        }
    }
}

/// What the sweeps read besides the chain.
pub(crate) struct Ctx<'a> {
    pub streets: &'a StreetWorld,
    pub chains: &'a [Chain],
    pub dims: &'a Dims,
    /// Class-model lane width of a way (`CaStandards::class_lane_width`; the
    /// city lane where the way is unknown).
    pub class_lane: &'a dyn Fn(&str) -> f64,
    pub scale: f64,
}

fn tint_of(c: &Chain) -> [u8; 3] {
    let names: Vec<&str> = c.pieces.iter().map(|p| p.name.as_str()).collect();
    if names.iter().any(|n| n.contains("Central Freeway")) {
        CENTRAL_VIADUCT_TINT
    } else if names.iter().any(|n| n.contains("Presidio Parkway")) {
        PRESIDIO_TINT
    } else {
        CONCRETE_TINT
    }
}

/// The sweep of chain `c` drawn as `draw`.
pub(crate) fn sweep(ctx: &Ctx, c: &Chain, draw: Draw) -> Sweep {
    let movable = eo_data::movable_spans::is_leaf(&c.pieces[0].way);
    let len = c.length();
    let o = &ctx.streets.o;
    let dims = ctx.dims;
    let sections: Vec<Section> = c
        .pieces
        .iter()
        .map(|p| {
            let seg = &ctx.streets.city.segments[p.segment];
            section(seg, p, c.class, dims, (ctx.class_lane)(&p.way), ctx.scale)
        })
        .collect();
    let asphalt: Vec<[u8; 3]> = c
        .pieces
        .iter()
        .map(|p| {
            let seg = &ctx.streets.city.segments[p.segment];
            StreetMaterial::get().carriageway_tint(&seg.id, seg.street_id.as_deref(), &seg.highway)
        })
        .collect();

    // ---- keys -----------------------------------------------------------------
    let mut keys: Vec<(f64, bool)> = c.stations.iter().map(|&s| (s, false)).collect();
    for &cu in &c.cum[1..c.cum.len().saturating_sub(1)] {
        keys.push((cu, true));
    }
    let mut depth_keys: Vec<f64> = vec![];
    for (k, &b) in c.bents.iter().enumerate() {
        keys.push((b, false));
        if c.depths.len() == c.bents.len() + 1 {
            let (d0, d1) = (c.depths[k], c.depths[k + 1]);
            let (a, bb) = (if k == 0 { 0.0 } else { c.bents[k - 1] }, c.bents.get(k + 1).copied().unwrap_or(len));
            if (d0 - d1).abs() > 1e-6 {
                let ease = if d0 > d1 { DEPTH_EASE_M.min(0.25 * (b - a)) } else { DEPTH_EASE_M.min(0.25 * (bb - b)) };
                depth_keys.push(if d0 > d1 { b - ease } else { b + ease });
            }
        }
    }
    keys.extend(depth_keys.iter().map(|&s| (s, false)));
    for sp in c.supports.iter().filter(|b| b.downspout) {
        let sd = sp.s + sp.cap_width / 2.0 + DRAIN_PAST_CAP_M;
        for d in [-STREAK_HALF_M, 0.0, STREAK_HALF_M] {
            keys.push((sd + d, false));
        }
    }
    for w in c.pieces.windows(2) {
        if (w[0].half - w[1].half).abs() > 1e-6 {
            keys.push((w[0].s1 - WIDTH_TAPER_M / 2.0, false));
            keys.push((w[0].s1 + WIDTH_TAPER_M / 2.0, false));
        }
    }
    for (e, end) in c.ends.iter().enumerate() {
        if end.kind == EndKind::Paved {
            keys.push((if e == 0 { CROWN_BLEND_M } else { len - CROWN_BLEND_M }, false));
        }
    }
    keys.retain(|k| k.0 >= -1e-9 && k.0 <= len + 1e-9);
    keys.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut ks: Vec<(f64, bool)> = vec![];
    for k in keys {
        match ks.last_mut() {
            // (a vertex keeps its own station, so its frame is mitred.)
            Some(l) if (k.0 - l.0).abs() < 1e-4 => {
                if k.1 && !l.1 {
                    *l = k;
                }
            }
            _ => ks.push((k.0.clamp(0.0, len), k.1)),
        }
    }
    // no step longer than the profile's (the keys only add to its stations).
    let mut keys = vec![];
    for w in ks.windows(2) {
        keys.push(w[0]);
        let n = ((w[1].0 - w[0].0) / STEP_M).ceil() as usize;
        for j in 1..n {
            keys.push((w[0].0 + (w[1].0 - w[0].0) * j as f64 / n as f64, false));
        }
    }
    if let Some(&l) = ks.last() {
        keys.push(l);
    }

    // ---- per station ------------------------------------------------------------
    let half_at = |s: f64| -> f64 {
        let mut h = c.half_at(s);
        for w in c.pieces.windows(2) {
            if (w[0].half - w[1].half).abs() > 1e-6 && (s - w[0].s1).abs() < WIDTH_TAPER_M / 2.0 {
                let t = (s - (w[0].s1 - WIDTH_TAPER_M / 2.0)) / WIDTH_TAPER_M;
                h = w[0].half + (w[1].half - w[0].half) * t.clamp(0.0, 1.0);
            }
        }
        h
    };
    let depth_at = |s: f64| -> f64 {
        if c.depths.len() != c.bents.len() + 1 || draw != Draw::Deck {
            return 0.0;
        }
        let mut d = c.depth_at(s);
        for (k, &b) in c.bents.iter().enumerate() {
            let (d0, d1) = (c.depths[k], c.depths[k + 1]);
            if (d0 - d1).abs() < 1e-9 {
                continue;
            }
            let (a, bb) = (if k == 0 { 0.0 } else { c.bents[k - 1] }, c.bents.get(k + 1).copied().unwrap_or(len));
            let (shallow, deep) = (d0.min(d1), d0.max(d1));
            let ease = if d0 > d1 { DEPTH_EASE_M.min(0.25 * (b - a)) } else { DEPTH_EASE_M.min(0.25 * (bb - b)) };
            let dist = if d0 > d1 { b - s } else { s - b };
            if dist >= -1e-9 && dist < ease {
                d = d.min(shallow + (deep - shallow) * (dist / ease).max(0.0));
            }
        }
        d
    };
    // chains sharing each end's node (for the gore barriers).
    let gore: [Vec<&Chain>; 2] = [0, 1].map(|e| {
        if c.ends[e].kind != EndKind::Shared {
            return vec![];
        }
        ctx.chains
            .iter()
            .filter(|d| d.id != c.id && d.is_built() && matches!(d.kind, ChainKind::Deck | ChainKind::Landmark | ChainKind::Below) && d.ends.iter().any(|x| x.node == c.ends[e].node))
            .collect()
    });
    // the decks this one is solved to meet (stage A5): no barrier on either's
    // roadway anywhere along it, and the minor's deck top clipped at the
    // major's fascia (the major of the pair draws the shared surface).
    let merged: Vec<&Chain> = c.merged_with.iter().filter_map(|&id| ctx.chains.get(id as usize)).filter(|d| d.is_built() && d.heights.len() == d.stations.len()).collect();
    let majors: Vec<&Chain> = merged.iter().copied().filter(|d| crate::solve::is_major(d, c)).collect();
    let joints: Vec<f64> = c
        .supports
        .iter()
        .filter(|b| b.joint)
        .map(|b| b.s)
        .chain(c.ends.iter().enumerate().filter(|(_, e)| e.kind != EndKind::Free).map(|(e, _)| if e == 0 { 0.0 } else { len }))
        .collect();
    let drains: Vec<f64> = c.supports.iter().filter(|b| b.downspout).map(|b| b.s + b.cap_width / 2.0 + DRAIN_PAST_CAP_M).collect();
    let b = dims.barrier_base;
    let island_viaduct = draw == Draw::Deck && dims.island.is_some() && crate::island_section::east_transition(c);
    let mut st: Vec<Station> = keys
        .iter()
        .map(|&(s, vertex)| {
            let f = frame_at(&c.points, &c.cum, s, vertex);
            let (tx, tz) = (f.nz, -f.nx);
            let half = half_at(s);
            let y = c.y_at(s);
            let depth = depth_at(s);
            let edge = half + b;
            let g = |u: f64| ctx.streets.ground(f.x + f.nx * f.miter * u, f.z + f.nz * f.miter * u);
            let ground = [g(-edge), g(0.0), g(edge)];
            let ground_min = ground.iter().copied().fold(f64::INFINITY, f64::min);
            let ground_max = ground.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            // The terrain grid can climb a bank beside a lower traffic lane.
            // A full-width fill wall would block a crossing whose solved
            // profile already reserves clearance underneath the girder.
            let on_fill = !movable && !island_viaduct && y - depth < ground_max + FILL_CLEAR_M;
            let crosses_at = |chain: &Chain, station: f64| chain.crossings.iter().any(|crossing| {
                (crossing.s0..=crossing.s1).contains(&station) && crossing.achieved >= crossing.clearance
            });
            let cleared_crossing = on_fill && draw == Draw::Deck && o.pavement_repairs
                && (crosses_at(c, s) || gore.iter().flatten().chain(&merged).any(|neighbour| {
                    // At a fork the adjacent deck edge can overhang the same
                    // lane although only its neighbour's centreline crosses it.
                    let (other_s, distance) = neighbour.project(f.x, f.z);
                    if distance > edge + neighbour.half_at(other_s) || !crosses_at(neighbour, other_s) {
                        return false;
                    }
                    let ceiling = y - depth - FILL_CLEAR_M;
                    [-edge, -half * 0.5, 0.0, half * 0.5, edge].into_iter().any(|u| {
                        let (x, z) = (f.x + f.nx * f.miter * u, f.z + f.nz * f.miter * u);
                        ctx.streets.carriageway_near(x, z, ceiling).is_some_and(|road| road < ceiling)
                    })
                }));
            let fill = draw != Draw::Deck || (on_fill && !cleared_crossing);
            let mut crown = [0.0; 2];
            for (e, end) in c.ends.iter().enumerate() {
                if end.kind == EndKind::Paved {
                    let d = if e == 0 { s } else { len - s };
                    // A short approach still reaches the adjacent span's own
                    // cross-section at the opposite end.
                    crown[e] = (1.0 - d / CROWN_BLEND_M.min(len)).clamp(0.0, 1.0);
                }
            }
            let mut barrier = [draw == Draw::Deck; 2];
            let on_roadway = |d: &Chain, u: f64| {
                let p = [f.x + f.nx * f.miter * u, f.z + f.nz * f.miter * u];
                let (sd, dist) = d.project(p[0], p[1]);
                dist <= d.half_at(sd) + 0.25 && (d.y_at(sd) - y).abs() < GORE_DY_M
            };
            for (e, others) in gore.iter().enumerate() {
                let d_end = if e == 0 { s } else { len - s };
                if others.is_empty() || d_end > GORE_REACH_M {
                    continue;
                }
                for (side, sgn) in [(0usize, 1.0), (1, -1.0)] {
                    if others.iter().any(|d| on_roadway(d, sgn * (half + b / 2.0))) {
                        barrier[side] = false;
                    }
                }
            }
            for (side, sgn) in [(0usize, 1.0), (1, -1.0)] {
                if merged.iter().any(|d| on_roadway(d, sgn * (half + b / 2.0))) {
                    barrier[side] = false;
                }
            }
            // the part of the deck top a major covers (to its fascia), and its surface at the cut.
            let (mut clip, mut meet) = ([-half, half], [f64::NAN; 2]);
            for d in &majors {
                let fascia = |u: f64| -> Option<f64> {
                    let p = [f.x + f.nx * f.miter * u, f.z + f.nz * f.miter * u];
                    let (sd, dist) = d.project(p[0], p[1]);
                    (sd > 1e-6 && sd < d.length() - 1e-6 && dist <= d.half_at(sd)).then(|| d.y_at(sd))
                };
                const N: usize = 32;
                let covered: Vec<bool> = (0..=N).map(|k| fascia(-half + 2.0 * half * k as f64 / N as f64).is_some()).collect();
                let u_of = |k: usize| -half + 2.0 * half * k as f64 / N as f64;
                if covered.iter().all(|&x| x) {
                    clip = [0.0, 0.0];
                    meet = [y, y];
                    break;
                }
                // covered from one edge in: cut where the major's fascia crosses the section.
                let cut = |free: f64, cov: f64| -> f64 {
                    let (mut lo, mut hi) = (free, cov);
                    for _ in 0..24 {
                        let m = (lo + hi) / 2.0;
                        if fascia(m).is_some() { hi = m } else { lo = m }
                    }
                    hi
                };
                if covered[N] && !covered[0] {
                    let k0 = (0..=N).rev().take_while(|&k| covered[k]).last().unwrap();
                    let u = cut(u_of(k0 - 1), u_of(k0));
                    if u < clip[1] {
                        clip[1] = u;
                        meet[1] = fascia(u).unwrap_or(y);
                    }
                } else if covered[0] && !covered[N] {
                    let k1 = (0..=N).take_while(|&k| covered[k]).last().unwrap();
                    let u = cut(u_of(k1 + 1), u_of(k1));
                    if u > clip[0] {
                        clip[0] = u;
                        meet[0] = fascia(u).unwrap_or(y);
                    }
                }
            }
            if clip[0] > clip[1] {
                clip = [0.0, 0.0];
            }
            let joint = joints.iter().map(|&j| (1.0 - (s - j).abs() / 4.0).max(0.0)).fold(0.0, f64::max);
            let streak_v = drains.iter().map(|&d| (1.0 - (s - d).abs() / STREAK_HALF_M).max(0.0)).fold(0.0, f64::max);
            Station {
                s,
                x: f.x,
                z: f.z,
                nx: f.nx,
                nz: f.nz,
                ox: f.nx * f.miter,
                oz: f.nz * f.miter,
                tx,
                tz,
                slope: c.slope_at(s),
                y,
                half,
                depth,
                ground,
                ground_min,
                fill,
                crown,
                barrier,
                joint,
                streak: [streak_v, 0.0],
                clip,
                meet,
            }
        })
        .collect();
    // (a drain inlet is at the +n barrier, the side of the column its
    // downspout runs down; the streak runs down that fascia.)
    // the abutment: where the superstructure leaves its fill, the station is
    // doubled (fill, then open) so the wall between them stands vertical.
    let mut out: Vec<Station> = Vec::with_capacity(st.len() + 4);
    for i in 0..st.len() {
        if i > 0 && st[i].fill != st[i - 1].fill {
            let (a, bq) = (&st[i - 1], &st[i]);
            // where the soffit crosses the fill clearance, linearly.
            let m = |q: &Station| q.y - q.depth - q.ground.iter().copied().fold(f64::NEG_INFINITY, f64::max) - FILL_CLEAR_M;
            let (ma, mb) = (m(a), m(bq));
            let t = if (mb - ma).abs() > 1e-12 { (-ma / (mb - ma)).clamp(0.0, 1.0) } else { 0.5 };
            let s = a.s + (bq.s - a.s) * t;
            if s > a.s + 1e-3 && s < bq.s - 1e-3 {
                let tmp = Sweep { draw, st: vec![*a, *bq], levels: [vec![], vec![], vec![]], sections: vec![], asphalt: vec![], east_pavement: false, tint: [0; 3], cross: [0.0; 3], surfaces: [vec![], vec![]], ends: [EndKind::Free; 2], tunnel: None, era: 0 };
                let mut p = tmp.lerp(s);
                p.fill = a.fill;
                out.push(p);
                p.fill = bq.fill;
                out.push(p);
            } else {
                // the boundary is at a station: double that one.
                let mut p = *bq;
                p.fill = a.fill;
                out.push(p);
            }
        }
        out.push(st[i]);
    }
    st = out;
    // a tunnel's covered and open stations, on the ground as drawn.
    let paved = [c.ends[0].kind == EndKind::Paved, c.ends[1].kind == EndKind::Paved];
    let twins: Vec<&Chain> = ctx.chains.iter().filter(|d| d.id != c.id && d.kind == ChainKind::Tunnel && d.is_built() && d.heights.len() == d.stations.len()).collect();
    let island = ctx.dims.island.and_then(|dims| dims.section(c).map(|shape| (dims, shape)));
    let tunnel = (draw == Draw::Tunnel).then(|| crate::tunnel::info(&st, ctx.streets, o.curb_face_height, paved, &twins, island));
    // ---- levels -------------------------------------------------------------------
    let n = st.len();
    let forced = |i: usize| -> bool {
        // Both sides of a portal must survive decimation. Otherwise a
        // distant view moves the headwall or drops a short roof entirely.
        let portal = tunnel.as_ref().is_some_and(|info| {
            (i > 0 && info.covered[i - 1] != info.covered[i])
                || (i + 1 < n && info.covered[i] != info.covered[i + 1])
        });
        i == 0 || i + 1 == n || portal || (i > 0 && (st[i].s - st[i - 1].s).abs() < 1e-6) || (i + 1 < n && (st[i + 1].s - st[i].s).abs() < 1e-6) || c.bents.iter().any(|&b| (b - st[i].s).abs() < 1e-6)
    };
    let level = |every: usize| -> Vec<usize> { (0..n).filter(|&i| i % every == 0 || forced(i)).collect() };
    let levels = [(0..n).collect(), level(2), level(4)];
    let ends = [c.ends[0].kind, c.ends[1].kind];
    let cross = [o.cross_slope, o.gutter_width, o.gutter_depth];
    let surfaces = [c.ends[0].surface.clone(), c.ends[1].surface.clone()];
    let era = crate::era_of(c, ctx.chains, ctx.scale) as u8;
    let east_pavement = crate::east_span_pavement() && crate::east_span::readings(c).is_some();
    Sweep { draw, st, levels, sections, asphalt, east_pavement, tint: tint_of(c), cross, surfaces, ends, tunnel, era }
}
