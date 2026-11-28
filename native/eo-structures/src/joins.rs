//! Join grade-separated roads to the street as it is drawn. A first solve
//! identifies the paved ends and the grade and clearance each needs. The
//! street adds matching junction legs, caps and approaches. Subsequent solves
//! carry landing heights along connected streets until the deck can meet its
//! trimmed mouth. Street and structure meshes use the same end cross-section.
//! The joins are plain data so eo-street does not depend on eo-structures.

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

/// The highest portal reachable from each approach station without exceeding
/// the grade limit. Lower the portal only where the street climbs faster.
fn portal_cut(streets: &StreetWorld, ei: usize, k: usize, chain: &Chain, end_index: usize) -> Option<Rise> {
    let grade = if end_index == 0 { *chain.grade_limits.first()? } else { *chain.grade_limits.last()? };
    let e = &streets.plan.entries[ei];
    let g = &streets.geoms[ei];
    let n = g.stations.len();
    if n < 2 || !(grade > 0.0) {
        return None;
    }
    let end = if k == 0 { 0 } else { n - 1 };
    let mouth = g.datums[end];
    let mut to = mouth;
    let mut descending = mouth;
    for i in 0..n {
        let d = if k == 0 { g.stations[i].s } else { e.length - g.stations[i].s };
        if d <= CUT_REACH_M {
            to = to.min(g.datums[i] + grade * d);
            descending = descending.min(g.datums[i] - grade * d);
        }
    }

    // On a rising hillside the start of a surveyed tunnel can still be too
    // shallow for its roof. Excavate the approach toward the first covered
    // stretch, limited by the same grade as the adjoining street.
    let inward = |d: f64| if end_index == 0 { d } else { chain.length() - d };
    let probe = inward(CUT_REACH_M.min(chain.length() * 0.5));
    let earth = |s| { let f = chain.frame(s); streets.ground(f.x, f.z) };
    let roof = crate::BORE_CLEAR_M + crate::tunnel::ROOF_M;
    let hill = earth(probe);
    // Where the two approach reaches overlap, the crossing itself supplies
    // the cover. Carving two independent hillside landings would lower the
    // road under the crossing a second time.
    if chain.length() > 2.0 * CUT_REACH_M && hill > earth(inward(0.0)) && hill < chain.y_at(probe) + roof
        && chain.stations.iter().any(|&s| earth(s) >= chain.y_at(s) + roof)
    {
        to = to.min((hill - roof).max(descending));
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

/// Where a landmark deck meets a street directly, the fixed deck elevation
/// sets the paved landing. Ordinary bridge links already carry that height
/// through their own solved ends.
fn direct_landmark_landings(world: &StructureWorld, streets: &StreetWorld, grade: f64) -> Vec<(P2, Rise)> {
    let mut connections = std::collections::HashMap::<u32, usize>::new();
    for chain in &world.chains {
        for end in &chain.ends {
            *connections.entry(end.node).or_default() += 1;
        }
    }
    let mut landings = Vec::new();
    for chain in world.chains.iter().filter(|c| c.kind == ChainKind::Landmark && c.is_built()) {
        for end in &chain.ends {
            if connections[&end.node] != 1 {
                continue;
            }
            let Some(top) = end.fixed else { continue };
            let Some((entry, k)) = stub_at(streets, end.at, streets.o.node_snap) else { continue };
            let paved = &streets.plan.entries[entry];
            let at = if k == 0 { paved.points[0] } else { *paved.points.last().unwrap() };
            if at != end.at {
                continue;
            }
            let geom = &streets.geoms[entry];
            let datum = if k == 0 { geom.datums.first() } else { geom.datums.last() };
            let Some(&datum) = datum else { continue };
            let to = top - streets.o.cross_section_y(0.0, 0.0, paved.half);
            if grade > 0.0 && to > datum {
                landings.push((at, Rise { to, grade }));
            }
        }
    }
    landings
}

impl StructureWorld {
    /// The joins the street is asked for (`eo_street::joins`), from this
    /// world as built on `streets` (a street world built without joins):
    /// every built chain's paved ends (decks, roads below other ways and,
    /// with `--tunnels`, tunnels); and every chain held by a grade or clearance bound whose
    /// every too-low end is a paved stub, with the rise to the end height it
    /// needs at the chain's own grade limit (`StructureEnd::rise`; the
    /// street draws it where the ramp fits), its other paved end with it.
    /// Deterministic: chains by id, ends in order.
    pub fn street_joins(&self, streets: &StreetWorld) -> StructureJoins {
        let snap = streets.o.node_snap;
        let barrier = self.barrier_base;
        // One paved stub has one surface. Preserve its clearance-constrained
        // approach when another deck asks for a different grade at the same node.
        let clearance_nodes: std::collections::HashSet<_> = self.chains.iter()
            .filter(|c| matches!(c.hold(), Some(crate::Hold::EndTooLow { .. })))
            .flat_map(|c| c.ends.iter().zip(c.end_wants).filter(|(_, h)| h.is_some()).map(|(end, _)| end.node))
            .collect();
        let mut ends = vec![];
        for c in &self.chains {
            if c.landmark.is_some() || c.kind == ChainKind::Landmark || c.points.len() < 2 {
                continue;
            }
            // rises a held chain asks of its stubs.
            let mut rises: [Option<Rise>; 2] = [None, None];
            if !c.is_built() {
                if !matches!(c.hold(), Some(crate::Hold::EndTooLow { .. } | crate::Hold::Grade)) {
                    continue;
                }
                let mut asked = false;
                let mut fits = true;
                for e in 0..2 {
                    let (Some(want), Some(fixed)) = (c.end_wants[e], c.ends[e].fixed) else { continue };
                    if c.ends[e].kind != EndKind::Paved || want <= fixed {
                        continue;
                    }
                    if matches!(c.hold(), Some(crate::Hold::Grade)) && clearance_nodes.contains(&c.ends[e].node) {
                        fits = false;
                        break;
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
                    (ChainKind::Tunnel, Some((ei, k))) => portal_cut(streets, ei, k, c, e),
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
    rebuild_with_landings(streets, joins, &streets.o.landings)
}

fn rebuild_with_landings(streets: &StreetWorld, joins: StructureJoins, landings: &[(P2, Rise)]) -> StreetWorld {
    let c = &streets.city;
    let city = StreetCity { segments: c.segments.clone(), intersections: c.intersections.clone(), crossings: c.crossings.clone(), crossing_nodes: c.crossing_nodes.clone(), continental: c.continental.clone() };
    let mut o = streets.o.clone().with_structures(Some(std::sync::Arc::new(joins)));
    o.landings = landings.to_vec();
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

fn reaches_mouth(chain: &Chain, mouth: Option<&P2>) -> bool {
    mouth.is_none_or(|m| [chain.points[0], *chain.points.last().unwrap()].iter().any(|p| (p[0] - m[0]).hypot(p[1] - m[1]) < 0.05))
}

/// Solve the structures and their connected street approaches together. Carry
/// the required grade past short stubs to the next paved run. Remove legs a
/// short deck cannot reach, then apply any remaining clearance at its trimmed
/// mouth. Only retained links participate in the final street and deck meshes.
pub fn build_joined(
    streets: &StreetWorld,
    city: &eo_data::City,
    standards: &eo_data::standards::CaStandards,
    landmarks: &eo_landmarks::LandmarksJs,
    options: crate::StructureOptions,
) -> anyhow::Result<Joined> {
    let first = StructureWorld::solve_only(streets, city, standards, landmarks, options)?;
    let mut asked = first.street_joins(streets);
    let approach_grade = standards.metric_key("freeway-profile-grade-max", "rolling")?;
    let direct_landings = direct_landmark_landings(&first, streets, approach_grade);
    let mut landings = direct_landings.clone();
    for c in first.held() {
        for (e, end) in c.ends.iter().enumerate() {
            let Some(want) = c.end_wants[e] else { continue };
            if end.kind != EndKind::Paved {
                continue;
            }
            let grade = if e == 0 { c.grade_limits[0] } else { *c.grade_limits.last().unwrap() };
            landings.push((end.at, Rise { to: want + RISE_MARGIN_M, grade }));
        }
        if c.end_wants.iter().all(Option::is_none) {
            continue;
        }
        for (e, end) in c.ends.iter().enumerate() {
            if end.kind != EndKind::Paved {
                continue;
            }
            let piece = if e == 0 { &c.pieces[0] } else { c.pieces.last().unwrap() };
            if asked.ends.iter().any(|j| j.segment == piece.segment && j.at == end.at) {
                continue;
            }
            asked.ends.push(StructureEnd {
                at: end.at,
                line: line_from(c, e, LEG_LINE_M),
                half: piece.half,
                closed: match c.kind {
                    ChainKind::Deck => piece.half + first.barrier_base,
                    ChainKind::Tunnel => piece.half + crate::tunnel::WALKWAY_M + crate::tunnel::WALL_M,
                    _ => piece.half,
                },
                segment: piece.segment,
                rise: None,
                cut: None,
            });
        }
    }
    if asked.is_empty() && direct_landings.is_empty() {
        let mut w = first;
        w.attach(streets, city, standards, landmarks)?;
        return Ok(Joined { streets: None, structures: w });
    }
    // Where an adjacent junction pad already covers a stub, its upper paving
    // carries the car. Meet that pavement at the approach instead of leaving
    // the deck at the lower ribbon's datum. end_surface then blends its actual
    // cross-section to the final street, including its crown at each wheel track.
    let island = crate::island_section::Dimensions::read(standards)?;
    let local_clearance = standards.metric("structure-vertical-clearance-local").ok();
    let match_overlapping_pad = |c: &Chain, e: usize, street: &StreetWorld| -> Option<(P2, Rise)> {
        let end = &c.ends[e];
        if end.kind != EndKind::Paved {
            return None;
        }
        let fixed = end.fixed?;
        let s = if e == 0 { 0.0 } else { c.length() };
        let f = c.frame(s);
        let dir = if e == 0 { -1.0 } else { 1.0 };
        let q = [f.x + dir * f.tx * crate::solve::END_SAMPLE_M, f.z + dir * f.tz * crate::solve::END_SAMPLE_M];
        let crown = street.o.cross_slope * c.half_at(s);
        let levels = street.paved_heights(q[0], q[1]);
        let near = levels.iter().any(|h| (h - crown - fixed).abs() < crate::solve::END_DRAWN_M);
        let top = levels.into_iter().max_by(f64::total_cmp)? - crown;
        let grade = if e == 0 { c.grade_limits[0] } else { *c.grade_limits.last()? };
        // Separate levels with vehicle clearance are distinct roads. A
        // shallow overlap of two ribbons at a mapped approach is one road.
        if near
            && top - fixed > crate::solve::END_DRAWN_M
            && top - fixed < local_clearance?
            && street.paved_layer_at(q[0], q[1]) == Some(eo_street::Layer::Carriageway)
        {
            Some((end.at, Rise { to: top, grade }))
        } else {
            None
        }
    };
    if options.tunnels {
        crate::tunnel_clearance::freeway_crossings(&first.chains, streets, standards, &mut landings)?;
    }
    let mut second_streets = rebuild_with_landings(streets, asked.clone(), &landings);
    let mut second;
    let mut pass = 0;
    // Residual clearance at a trimmed junction may raise its next connected approach.
    const MAX_LANDING_PASSES: usize = 10;
    let requested = asked.clone();
    loop {
        pass += 1;
        if second_streets.o.structures.as_deref() == Some(&asked) {
            second_streets.update_structure_landings(&landings);
        } else {
            second_streets = rebuild_with_landings(streets, asked.clone(), &landings);
        }
        second = StructureWorld::solve_only(&second_streets, city, standards, landmarks, options)?;
        if pass == 1 {
            let mouths = leg_mouths(&second_streets);
            let before = asked.ends.len();
            let mut k = 0;
            let previously_built: std::collections::HashSet<_> =
                first.chains.iter().filter(|c| c.is_built()).map(|c| c.pieces.iter().map(|p| p.key.as_str()).collect::<Vec<_>>()).collect();
            asked.ends.retain(|end| {
                let mouth = mouths.get(&k);
                k += 1;
                second.chains.iter().filter(|c| c.pieces.iter().any(|p| p.segment == end.segment)).any(|c| {
                    let invalidated = c.hold().is_some()
                        && (mouth.is_some() || end.cut.is_some())
                        && previously_built.contains(&c.pieces.iter().map(|p| p.key.as_str()).collect::<Vec<_>>());
                    reaches_mouth(c, mouth) && !invalidated
                })
            });
            let previous_landings = landings.len();
            for c in second.chains.iter().filter(|c| c.is_built() && c.kind != ChainKind::Landmark) {
                for e in 0..2 {
                    if let Some(landing) = match_overlapping_pad(c, e, &second_streets) {
                        landings.push(landing);
                    }
                }
            }
            if asked.ends.len() < before || landings.len() != previous_landings {
                continue;
            }
        }
        let mut changed = false;
        if pass == MAX_LANDING_PASSES {
            break;
        }
        if options.tunnels {
            for c in second.chains.iter().filter(|c| c.is_built() && c.kind == ChainKind::Tunnel && c.class != crate::RoadClass::Other) {
                for (e, end) in c.ends.iter().enumerate() {
                    if end.kind != EndKind::Paved { continue; }
                    if let Some(landing) = crate::tunnel_clearance::portal_landing(&second_streets, c, e) {
                        // A hillside cut is requested before the two island decks are
                        // solved. Its old ceiling must not cut the joined street below
                        // the landing now carrying traffic out of the lower bore.
                        if island.and_then(|d| d.section(c)).is_some() {
                            let piece = if e == 0 { &c.pieces[0] } else { c.pieces.last().unwrap() };
                            for join in asked.ends.iter_mut().filter(|j| j.segment == piece.segment && j.at == end.at) {
                                if let Some(cut) = &mut join.cut && cut.to + CUT_MIN_M < landing.to {
                                    cut.to = landing.to;
                                    changed = true;
                                }
                            }
                        }
                        if !landings.iter().any(|(at, r)| *at == end.at && r.to >= landing.to - CUT_MIN_M && r.grade == landing.grade) {
                            landings.push((end.at, landing));
                            changed = true;
                        }
                    }
                }
            }
        }
        for c in second.held() {
            for (e, end) in c.ends.iter().enumerate() {
                let (Some(want), Some(fixed)) = (c.end_wants[e], end.fixed) else { continue };
                if end.kind != EndKind::Paved || want <= fixed {
                    continue;
                }
                let at = end.at;
                let grade = if e == 0 { c.grade_limits[0] } else { *c.grade_limits.last().unwrap() };
                if let Some((_, r)) = landings.iter_mut().find(|(p, r)| *p == at && r.grade == grade) {
                    r.to += want - fixed;
                } else {
                    landings.push((at, Rise { to: want, grade }));
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // a join stays where its chain (the one carrying its end segment) is
    // built, and a leg only where the chain could be cut back to its mouth
    // (a chain shorter than the cut is left whole, over the junction's pad).
    let mouths = leg_mouths(&second_streets);
    let kept = StructureJoins {
        ends: asked
            .ends
            .iter()
            .enumerate()
            .filter(|(k, end)| {
                second.chains.iter().any(|c| c.is_built() && c.pieces.iter().any(|p| p.segment == end.segment) && reaches_mouth(c, mouths.get(k)))
            })
            .map(|(_, end)| end.clone())
            .collect(),
    };
    let mut stats = JoinStats::of(&requested, &kept);
    stats.passes = 1 + pass;
    let (mut final_streets, mut w) = if kept == asked {
        (second_streets, second)
    } else if kept.is_empty() && direct_landings.is_empty() {
        let mut w = StructureWorld::solve_only(streets, city, standards, landmarks, options)?;
        w.attach(streets, city, standards, landmarks)?;
        return Ok(Joined { streets: None, structures: w });
    } else {
        let remaining_landings = if kept.is_empty() { &direct_landings } else { &landings };
        let s3 = rebuild_with_landings(streets, kept, remaining_landings);
        let w3 = StructureWorld::solve_only(&s3, city, standards, landmarks, options)?;
        stats.passes += 1;
        (s3, w3)
    };
    if let Some(dims) = island {
        dims.round_lower_landings(&mut w.chains, &mut final_streets);
    }
    stats.note(&final_streets, &w, &first);
    w.census.joins = Some(stats);
    w.attach(&final_streets, city, standards, landmarks)?;
    if !w.tiles.openings.is_empty() {
        final_streets.set_road_openings(w.tiles.openings.clone());
    }
    Ok(Joined { streets: Some(final_streets), structures: w })
}

/// What the joins did (`Census::joins`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JoinStats {
    /// Structure ends the first solve asked the street for, and of them with a rise.
    pub asked: usize,
    pub asked_rise: usize,
    /// Dropped because the chain was held or could not reach its trimmed mouth.
    pub dropped: usize,
    /// Street and landing solves, including the plain street.
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
        writeln!(f, "street joins (stage A6): {} structure ends asked ({} with a rise), {} dropped, {} street and landing solves", self.asked, self.asked_rise, self.dropped, self.passes)?;
        writeln!(f, "  in the street: {} legs in a junction ({} junctions with a leg), {} stub ends capped, {} stubs raised (up to {:.2} m)", self.legs, self.new_junctions, self.caps, self.raised, self.raised_max_m)?;
        write!(f, "  chains built now and held without the joins: {:?}; held now and built without: {:?}", self.rescued, self.lost)
    }
}
