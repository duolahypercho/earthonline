//! Stage A6: what the street is asked to change where the grade-separated
//! roads meet it (`eo_street::joins`), as plain data, and the street world
//! built again with it.
//!
//! The two passes, without a dependency cycle: a street world built without
//! joins -> `StructureWorld::build` -> `StructureWorld::street_joins` (every
//! built chain's paved ends; a chain held as end too low whose paved stub can
//! climb to the end height it needs within its grade, with that rise) ->
//! `rebuild_streets` (the street world again, with the joins) ->
//! `StructureWorld::build` on it. In the second street world a structure end
//! landing where two or more paved runs end is a leg of a junction there
//! (the chain is cut back to the junction's mouth for it, `solve.rs`), a stub
//! is capped, and a stub with a rise climbs to it; the second solve then
//! meets the raised stub (`solve.rs` `end_height`). eo-street reads the joins
//! as plain data only; eo-structures reads `CaStandards` for the grade.

use eo_street::joins::{Rise, StructureEnd, StructureJoins};
use eo_street::plan::P2;
use eo_street::{BuildingIndex, StreetCity, StreetWorld};

use crate::{Chain, ChainKind, EndKind, StructureWorld};

/// How much of a structure's centreline a junction leg carries (from its
/// end outward): more than any junction trims an approach back (the street
/// caps a trim at 85 % of the approach). INFERRED (a drawing reach).
pub const LEG_LINE_M: f64 = 80.0;

/// A rise asks this much over the end height a held chain needs, so the
/// second solve meets it with room for its end tolerance. INFERRED (a solve
/// tolerance).
pub const RISE_MARGIN_M: f64 = 0.02;

/// How far back along a tunnel's paved stub its approach cut looks for the
/// street's own grade (the bare-earth grid draws a portal's hillside over
/// the last cell or two of the stub). INFERRED (a search reach).
pub const CUT_REACH_M: f64 = 60.0;

/// A cut lowers the stub's end by at least this much, or is not asked.
/// INFERRED (a drawing tolerance).
pub const CUT_MIN_M: f64 = 0.05;

/// The lowest end a cone rising at `grade` from a stub's end can have and
/// stay at or under the stub's datum within `CUT_REACH_M`: where the street
/// climbs to its end faster than the grade (a portal's hillside), the cone's
/// foot; None where it does not.
fn portal_cut(streets: &StreetWorld, ei: usize, k: usize, grade: f64) -> Option<Rise> {
    let e = &streets.plan.entries[ei];
    let g = &streets.geoms[ei];
    let n = g.stations.len();
    if n < 2 || !(grade > 0.0) {
        return None;
    }
    let end = if k == 0 { 0 } else { n - 1 };
    let mut to = g.datums[end];
    for i in 0..n {
        let d = if k == 0 { g.stations[i].s } else { e.length - g.stations[i].s };
        if d <= CUT_REACH_M {
            to = to.min(g.datums[i] - grade * d);
        }
    }
    (to < g.datums[end] - CUT_MIN_M).then_some(Rise { to, grade })
}

/// The chain's centreline from end `e` outward, up to `reach` metres.
fn line_from(c: &Chain, e: usize, reach: f64) -> Vec<P2> {
    let len = c.length();
    let reach = reach.min(len);
    let mut out = vec![];
    if e == 0 {
        for (p, &s) in c.points.iter().zip(&c.cum) {
            if s < reach - 1e-6 {
                out.push(*p);
            }
        }
        let f = c.frame(reach);
        out.push([f.x, f.z]);
    } else {
        for (p, &s) in c.points.iter().zip(&c.cum).rev() {
            if s > len - reach + 1e-6 {
                out.push(*p);
            }
        }
        let f = c.frame(len - reach);
        out.push([f.x, f.z]);
    }
    out
}

/// The single paved run end (not at a junction) within `snap` of `p`:
/// (entry, its end 0 = start / 1 = end).
fn stub_at(streets: &StreetWorld, p: P2, snap: f64) -> Option<(usize, usize)> {
    let mut found = vec![];
    for (ei, e) in streets.plan.entries.iter().enumerate() {
        if e.structure_leg.is_some() {
            continue;
        }
        for (k, q) in [(0usize, e.points[0]), (1, *e.points.last().unwrap())] {
            if (q[0] - p[0]).hypot(q[1] - p[1]) <= snap {
                found.push((ei, k));
            }
        }
    }
    let &[(ei, k)] = found.as_slice() else { return None };
    let e = &streets.plan.entries[ei];
    let at_node = if k == 0 { e.approach_start.is_some() } else { e.approach_end.is_some() };
    (!at_node).then_some((ei, k))
}

impl StructureWorld {
    /// The joins the street is asked for (`eo_street::joins`), from this
    /// world as built on `streets` (a street world built without joins):
    /// every built chain's paved ends (decks, roads below other ways and,
    /// with `--tunnels`, tunnels); and every chain held as end too low whose
    /// every too-low end is a paved stub, with the rise to the end height it
    /// needs at the chain's own grade limit (`StructureEnd::rise`; the
    /// street draws it where the ramp fits), its other paved end with it.
    /// Deterministic: chains by id, ends in order.
    pub fn street_joins(&self, streets: &StreetWorld) -> StructureJoins {
        let snap = streets.o.node_snap;
        let barrier = self.barrier_base;
        let mut ends = vec![];
        for c in &self.chains {
            if c.landmark.is_some() || c.kind == ChainKind::Landmark || c.points.len() < 2 {
                continue;
            }
            // rises a held chain asks of its stubs.
            let mut rises: [Option<Rise>; 2] = [None, None];
            if !c.is_built() {
                if !matches!(c.hold(), Some(crate::Hold::EndTooLow { .. })) {
                    continue;
                }
                let mut asked = false;
                let mut fits = true;
                for e in 0..2 {
                    let (Some(want), Some(fixed)) = (c.end_wants[e], c.ends[e].fixed) else { continue };
                    if c.ends[e].kind != EndKind::Paved || want <= fixed {
                        continue;
                    }
                    let grade = if e == 0 { c.grade_limits.first() } else { c.grade_limits.last() }.copied().unwrap_or(0.0);
                    let rise = Rise { to: want + RISE_MARGIN_M, grade };
                    // (the street draws the rise only where its ramp is back at
                    // the street datum before a junction, `StreetWorld::check_rises`;
                    // a rise it does not draw leaves the chain held, and
                    // `build_joined` drops its joins.)
                    match stub_at(streets, c.ends[e].at, snap) {
                        Some(_) if grade > 0.0 => {
                            rises[e] = Some(rise);
                            asked = true;
                        }
                        _ => fits = false,
                    }
                }
                if !asked || !fits {
                    continue;
                }
            }
            for e in 0..2 {
                if c.ends[e].kind != EndKind::Paved {
                    continue;
                }
                let piece = if e == 0 { &c.pieces[0] } else { c.pieces.last().unwrap() };
                let closed = match c.kind {
                    ChainKind::Deck => piece.half + barrier,
                    ChainKind::Tunnel => piece.half + crate::tunnel::WALKWAY_M + crate::tunnel::WALL_M,
                    _ => piece.half,
                };
                // a tunnel's portal: the stub comes down its approach cut at
                // the tunnel's own grade limit (stage A7).
                let cut = match (c.kind, stub_at(streets, c.ends[e].at, snap)) {
                    (ChainKind::Tunnel, Some((ei, k))) => portal_cut(streets, ei, k, if e == 0 { c.grade_limits.first() } else { c.grade_limits.last() }.copied().unwrap_or(0.0)),
                    _ => None,
                };
                ends.push(StructureEnd { at: c.ends[e].at, line: line_from(c, e, LEG_LINE_M), half: piece.half, closed, segment: piece.segment, rise: rises[e], cut });
            }
        }
        StructureJoins { ends }
    }
}

/// `streets` built again with `joins` (`StreetOptions::with_structures`):
/// the same city, terrain and buildings. The host installs the shore's rule
/// (`StreetWorld::set_shore_press`) on it as on the first.
pub fn rebuild_streets(streets: &StreetWorld, joins: StructureJoins) -> StreetWorld {
    let c = &streets.city;
    let city = StreetCity { segments: c.segments.clone(), intersections: c.intersections.clone(), crossings: c.crossings.clone(), crossing_nodes: c.crossing_nodes.clone(), continental: c.continental.clone() };
    let o = streets.o.clone().with_structures(Some(std::sync::Arc::new(joins)));
    StreetWorld::build(o, city, streets.terrain.clone(), BuildingIndex::new(streets.buildings.rings.clone()))
}

/// The mouth (its trimmed end frame's centre) of every structure leg in a
/// junction, by its `StructureEnd` index.
fn leg_mouths(streets: &StreetWorld) -> std::collections::HashMap<usize, P2> {
    let plan = &streets.plan;
    plan.entries
        .iter()
        .filter_map(|e| {
            let k = e.structure_leg?;
            let start = eo_street::plan::leg_at_start(e, &streets.o);
            let (n, a) = if start { e.approach_start? } else { e.approach_end? };
            let f = eo_street::plan::approach_end_frame(&plan.entries, &plan.nodes[n].approaches[a]);
            Some((k, [f.x, f.z]))
        })
        .collect()
}

/// What `build_joined` built.
pub struct Joined {
    /// The street world built again with the joins (None when the
    /// structures asked for none: the first street world stands).
    pub streets: Option<StreetWorld>,
    pub structures: StructureWorld,
}

/// Stage A6's whole sequence: the structures solved on `streets` (built
/// without joins), the joins they ask (`StructureWorld::street_joins`), the
/// street world built again with them and the structures solved on it; a
/// join whose chain that second solve holds (a leg's cut, or a rise that did
/// not rescue it, leaves the deck unable to clear what it crosses) is
/// dropped and the street built once more without it, and the structures'
/// final world built (meshes too) on that street. Deterministic for the
/// same inputs, so a client and a server build the same street and decks.
pub fn build_joined(streets: &StreetWorld, city: &eo_data::City, standards: &eo_data::standards::CaStandards, landmarks: &eo_landmarks::LandmarksJs, options: crate::StructureOptions) -> anyhow::Result<Joined> {
    let first = StructureWorld::solve_only(streets, city, standards, landmarks, options)?;
    let asked = first.street_joins(streets);
    if asked.is_empty() {
        let mut w = first;
        w.attach(streets, city, standards)?;
        return Ok(Joined { streets: None, structures: w });
    }
    let second_streets = rebuild_streets(streets, asked.clone());
    let second = StructureWorld::solve_only(&second_streets, city, standards, landmarks, options)?;
    // a join stays where its chain (the one carrying its end segment) is
    // built, and a leg only where the chain could be cut back to its mouth
    // (a chain shorter than the cut is left whole, over the junction's pad).
    let built = |w: &StructureWorld, seg: usize| w.chains.iter().any(|c| c.is_built() && c.pieces.iter().any(|p| p.segment == seg));
    let mouths = leg_mouths(&second_streets);
    let at_mouth = |w: &StructureWorld, k: usize, seg: usize| match mouths.get(&k) {
        None => true,
        Some(m) => w.chains.iter().filter(|c| c.is_built() && c.pieces.iter().any(|p| p.segment == seg)).any(|c| [c.points[0], *c.points.last().unwrap()].iter().any(|q| (q[0] - m[0]).hypot(q[1] - m[1]) < 0.05)),
    };
    let kept = StructureJoins { ends: asked.ends.iter().enumerate().filter(|(k, e)| built(&second, e.segment) && at_mouth(&second, *k, e.segment)).map(|(_, e)| e.clone()).collect() };
    let mut stats = JoinStats::of(&asked, &kept);
    let (final_streets, mut w) = if kept == asked {
        (second_streets, second)
    } else if kept.is_empty() {
        let w = StructureWorld::solve_only(streets, city, standards, landmarks, options)?;
        let mut w = w;
        w.attach(streets, city, standards)?;
        return Ok(Joined { streets: None, structures: w });
    } else {
        let s3 = rebuild_streets(streets, kept);
        let w3 = StructureWorld::solve_only(&s3, city, standards, landmarks, options)?;
        stats.passes = 3;
        (s3, w3)
    };
    stats.note(&final_streets, &w, &first);
    w.census.joins = Some(stats);
    w.attach(&final_streets, city, standards)?;
    Ok(Joined { streets: Some(final_streets), structures: w })
}

/// What the joins did (`Census::joins`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JoinStats {
    /// Structure ends the first solve asked the street for, and of them with a rise.
    pub asked: usize,
    pub asked_rise: usize,
    /// Dropped because the second solve held their chain anyway.
    pub dropped: usize,
    /// Street builds (2: the joins held; 3: some were dropped).
    pub passes: usize,
    /// In the final street: legs in a junction, junctions more than without
    /// joins, stub ends capped, stubs raised (and the most any is raised).
    pub legs: usize,
    pub new_junctions: i64,
    pub caps: usize,
    pub raised: usize,
    pub raised_max_m: f64,
    /// Chains built now that the first solve held (rescued by a rise), and
    /// built in the first but held now.
    pub rescued: Vec<u32>,
    pub lost: Vec<u32>,
}

impl JoinStats {
    fn of(asked: &StructureJoins, kept: &StructureJoins) -> Self {
        Self { asked: asked.ends.len(), asked_rise: asked.ends.iter().filter(|e| e.rise.is_some()).count(), dropped: asked.ends.len() - kept.ends.len(), passes: 2, ..Default::default() }
    }

    fn note(&mut self, streets: &StreetWorld, w: &StructureWorld, first: &StructureWorld) {
        let plan = &streets.plan;
        self.legs = plan.entries.iter().filter(|e| e.structure_leg.is_some() && (e.approach_start.is_some() || e.approach_end.is_some())).count();
        self.caps = plan.entries.iter().map(|e| (0..2).filter(|&k| e.structure_ends[k].is_some() && if k == 0 { e.approach_start.is_none() } else { e.approach_end.is_none() }).count()).sum();
        let base = plan.nodes.iter().filter(|n| n.approaches.iter().any(|a| plan.entries[a.entry].structure_leg.is_some())).count();
        self.new_junctions = base as i64;
        for ei in 0..plan.entries.len() {
            let g = &streets.geoms[ei];
            let top = (0..g.stations.len()).map(|i| streets.datum_raise(ei, i)).fold(0.0, f64::max);
            if top > 0.02 {
                self.raised += 1;
                self.raised_max_m = self.raised_max_m.max(top);
            }
        }
        // chains by their pieces' keys (the ids differ between the solves).
        let key = |c: &Chain| c.pieces.iter().map(|p| p.key.clone()).collect::<Vec<_>>();
        let before: std::collections::HashMap<Vec<String>, bool> = first.chains.iter().map(|c| (key(c), c.is_built())).collect();
        for c in &w.chains {
            if let Some(&b) = before.get(&key(c)) {
                if !b && c.is_built() {
                    self.rescued.push(c.id);
                }
                if b && !c.is_built() {
                    self.lost.push(c.id);
                }
            }
        }
    }
}

impl std::fmt::Display for JoinStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "street joins (stage A6): {} structure ends asked ({} with a rise), {} dropped (their chain held anyway), {} street builds", self.asked, self.asked_rise, self.dropped, self.passes)?;
        writeln!(f, "  in the street: {} legs in a junction ({} junctions with a leg), {} stub ends capped, {} stubs raised (up to {:.2} m)", self.legs, self.new_junctions, self.caps, self.raised, self.raised_max_m)?;
        write!(f, "  chains built now and held without the joins: {:?}; held now and built without: {:?}", self.rescued, self.lost)
    }
}
