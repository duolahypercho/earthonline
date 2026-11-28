//! Preserve drivable clearance at stacked bores, underground forks and their
//! approaches. The renderer and collision world share the clipped faces.

use crate::joins::CUT_REACH_M;
use crate::sweep::{self, Draw, Sweep};
use crate::{Chain, ChainKind, StructureMesh};
use eo_street::StreetWorld;
use eo_street::joins::Rise;
use eo_street::road_openings::RoadOpenings;

pub(crate) fn freeway_crossings(
    chains: &[Chain],
    streets: &StreetWorld,
    standards: &eo_data::CaStandards,
    landings: &mut Vec<([f64; 2], Rise)>,
) -> anyhow::Result<()> {
    let paved = crate::index::Paved::new(
        &streets.plan,
        CUT_REACH_M,
        streets.o.min_sidewalk_width,
        streets.o.max_step,
    );
    let grade = standards.metric_key("freeway-profile-grade-max", "rolling")?;
    for c in chains
        .iter()
        .filter(|c| c.is_built() && c.kind == ChainKind::Tunnel)
    {
        // Only an actual crossing, away from a portal shared with the
        // surface street, asks its overlying road to carry clearance.
        let mut crossing_entries = std::collections::BTreeSet::new();
        for pair in c.points.windows(2) {
            let middle = [
                (pair[0][0] + pair[1][0]) * 0.5,
                (pair[0][1] + pair[1][1]) * 0.5,
            ];
            for hit in paved.under_section(middle, pair[0], pair[1]) {
                let e = &streets.plan.entries[hit.entry as usize];
                if !crate::index::freeway_class(&e.highway)
                    || e.structure_leg.is_some()
                    || c.ends.iter().any(|end| {
                        [e.points[0], *e.points.last().unwrap()].iter().any(|p| {
                            (p[0] - end.at[0]).hypot(p[1] - end.at[1]) <= streets.o.node_snap
                        })
                    })
                {
                    continue;
                }
                if e.points
                    .windows(2)
                    .any(|w| crate::index::crossing(pair[0], pair[1], w[0], w[1]).is_some())
                {
                    crossing_entries.insert(hit.entry);
                }
            }
        }
        for &s in &c.stations {
            let f = c.frame(s);
            let centre = [f.x, f.z];
            for near in paved.runs_near(centre, c.max_half() + CUT_REACH_M) {
                if !crossing_entries.contains(&near.entry) {
                    continue;
                }
                let entry = &streets.plan.entries[near.entry as usize];
                if near.dist > c.half_at(s) + entry.half {
                    continue;
                }
                let q = entry.frame(near.s, false);
                let y = streets
                    .datum_at_station(&streets.geoms[near.entry as usize], near.s)
                    .unwrap_or_else(|| streets.ground(q.x, q.z) + streets.o.road_lift);
                let to = c.y_at(s) + crate::BORE_CLEAR_M + crate::tunnel::ROOF_M;
                if y < to {
                    landings.push(([q.x, q.z], Rise { to, grade }));
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn portal_landing(streets: &StreetWorld, c: &Chain, e: usize) -> Option<Rise> {
    let s = if e == 0 { 0.0 } else { c.length() };
    let f = c.frame(s);
    let sign = if e == 0 { -1.0 } else { 1.0 };
    let grade = if e == 0 {
        c.grade_limits[0]
    } else {
        *c.grade_limits.last().unwrap()
    };
    let y = c.y_at(s);
    // A hillside can also make the ordinary approach leap
    // *up* into its portal. Carry the portal datum back
    // along its connected streets at the same grade.
    let step = streets.o.max_step;
    let steep = (1..=(CUT_REACH_M / step).ceil() as usize).any(|i| {
        let d = i as f64 * step;
        let q = [f.x + sign * f.tx * d, f.z + sign * f.tz * d];
        streets
            .carriageway_near(q[0], q[1], y)
            .is_some_and(|h| h < y - grade * d - crate::joins::CUT_MIN_M)
    });
    steep.then_some(Rise { to: y, grade })
}

pub(crate) fn openings(
    sweeps: &[Option<Sweep>],
    streets: &StreetWorld,
    chains: &[Chain],
) -> RoadOpenings {
    let mut openings = RoadOpenings::default();
    for (chain, sweep) in chains.iter().zip(sweeps) {
        let Some(sw) = sweep.as_ref().filter(|sw| sw.draw == Draw::Tunnel) else {
            continue;
        };
        let section = |st: &sweep::Station| [-st.half, st.half].map(|u| st.at(u, sw.top(st, 0.0)));
        let shape = sw.tunnel.as_ref().and_then(|info| info.island_section);
        let min_height = shape.map_or(crate::tunnel::SPRING_M, |section| section.height(1.0));
        let add = |openings: &mut RoadOpenings, a, b| {
            openings.add(
                chain.id,
                a,
                b,
                crate::tunnel::ROOF_M,
                min_height,
            );
        };
        for pair in sw.st.windows(2) {
            if let Some(arch @ crate::IslandTunnelSection::Vault { .. }) = shape {
                crate::island_section::vault_openings(arch, chain.id, sw, [&pair[0], &pair[1]], &mut openings);
            } else {
                add(&mut openings, section(&pair[0]), section(&pair[1]));
            }
        }
        for (st, dir, end) in [
            (&sw.st[0], -1.0, sw.ends[0]),
            (sw.st.last().unwrap(), 1.0, sw.ends[1]),
        ] {
            if end != crate::EndKind::Paved {
                continue;
            }
            let inside = section(st);
            let reach = crate::tunnel::PORTAL_OPEN_M;
            let shift = dir * reach;
            // Keep the opening across the seam, including the two corners where
            // the street and bore turn. Its far elevation follows the connected
            // street. Extrapolating the bore grade can cut the viaduct overhead.
            let connected_y = shape.and_then(|_| {
                let joins = streets.o.structures.as_ref()?;
                streets.plan.entries.iter().enumerate().find_map(|(ei, entry)| {
                    if entry.structure_leg.is_some() { return None; }
                    entry.structure_ends.iter().enumerate().find_map(|(end, index)| {
                        let join = &joins.ends[(*index)?];
                        if (join.at[0]-st.x).hypot(join.at[1]-st.z) > streets.o.node_snap
                            || !chain.pieces.iter().any(|p| p.segment == join.segment) {
                            return None;
                        }
                        let s = if end == 0 { entry.s0() + reach } else { entry.s1() - reach };
                        let datum = streets.datum_at_station(&streets.geoms[ei], s.clamp(entry.s0(), entry.s1()))?;
                        Some(streets.o.cross_section_y(datum, 0., entry.half))
                    })
                })
            });
            let outside = inside.map(|p| [
                p[0] + shift * st.tx,
                connected_y.unwrap_or(p[1] + shift * st.slope),
                p[2] + shift * st.tz,
            ]);
            add(&mut openings, inside, outside);
        }
    }
    // The mouths also need clear space on the connected approach, which
    // may already turn away from the tunnel before it reaches open ground.
    if !openings.is_empty() {
        for (ei, entry) in streets.plan.entries.iter().enumerate() {
            if entry.structure_leg.is_some() {
                continue;
            }
            let geom = &streets.geoms[ei];
            for (end, join) in entry.structure_ends.iter().enumerate() {
                let Some(j) = join.and_then(|j| streets.o.structures.as_ref()?.ends.get(j)) else {
                    continue;
                };
                let Some(chain) = chains.iter().find(|c| {
                    c.is_built()
                        && c.kind == ChainKind::Tunnel
                        && c.pieces.iter().any(|p| p.segment == j.segment)
                }) else {
                    continue;
                };
                let height = sweeps[chain.id as usize].as_ref()
                    .and_then(|s| s.tunnel.as_ref()?.island_section)
                    .map_or(crate::tunnel::SPRING_M, |section| section.height(1.0));
                let section = |i: usize| {
                    [-entry.half, entry.half].map(|u| {
                        let q = geom.stations[i].at(u);
                        [q[0], geom.datums[i], q[1]]
                    })
                };
                for (i, pair) in geom.stations.windows(2).enumerate() {
                    let d = if end == 0 {
                        pair[0].s
                    } else {
                        entry.length - pair[1].s
                    };
                    if d < crate::joins::CUT_REACH_M {
                        openings.add(
                            chain.id,
                            section(i),
                            section(i + 1),
                            crate::tunnel::ROOF_M,
                            height,
                        );
                    }
                }
            }
        }
    }
    // Where a landmark approach meets ground-level paving, a swept sidewalk
    // corner can extend into its lanes. Clear only the curb-height band over
    // the connecting carriageway. Street layers on another level survive.
    let curb = streets.o.curb_face_height + streets.o.gutter_depth;
    for (chain, sweep) in chains.iter().zip(sweeps) {
        let Some(sw) = sweep.as_ref().filter(|sw| sw.draw == Draw::Deck) else { continue };
        if !sw.ends.contains(&crate::EndKind::Paved) || !sw.ends.contains(&crate::EndKind::Landmark) {
            continue;
        }
        for pair in sw.st.windows(2) {
            for side in [-1.0, 1.0] {
                let section = |st: &sweep::Station| {
                    [0.0, side * st.half].map(|u| {
                        let u = sw.clamp_u(st, u);
                        st.at(u, sw.top(st, u))
                    })
                };
                openings.add(chain.id, section(&pair[0]), section(&pair[1]), curb, curb);
            }
        }
    }
    openings
}

pub(crate) fn clip_concrete(
    m: &mut StructureMesh,
    openings: &RoadOpenings,
    linings: &[(std::ops::Range<usize>, u32)],
    chains: &[Chain],
) {
    if !openings.is_empty() && !m.indices.is_empty() {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in &m.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if openings.intersects_bounds(lo, hi) {
            let old = std::mem::take(&mut m.indices);
            let mut linings = linings.iter().peekable();
            for (face_id, face) in old.chunks_exact(3).enumerate() {
                let index = face_id * 3;
                while linings.peek().is_some_and(|(range, _)| range.end <= index) {
                    linings.next();
                }
                let lining = linings
                    .peek()
                    .filter(|(range, _)| range.contains(&index))
                    .map(|(_, id)| &chains[*id as usize]);
                let ids = [face[0], face[1], face[2]].map(|i| i as usize);
                let class = m.classes[ids[0]];
                let pieces = (class == crate::StructureClass::Concrete as u8)
                    .then(|| {
                        openings.clip_where(ids.map(|i| m.positions[i]), |other| {
                            let neighbor = &chains[other as usize];
                            neighbor.kind == crate::ChainKind::Tunnel && lining.is_none_or(|c| {
                                c.id != other && c.level == neighbor.level && c.shares_end(neighbor)
                            })
                        })
                    })
                    .flatten();
                let Some(pieces) = pieces else {
                    m.indices.extend_from_slice(face);
                    continue;
                };
                for triangle in pieces {
                    let k = m.positions.len() as u32;
                    for v in triangle {
                        m.positions.push(v.pos.map(|x| x as f32));
                        m.normals.push(v.interpolate(ids.map(|i| m.normals[i])));
                        m.colors.push(v.color(ids.map(|i| m.colors[i])));
                        m.uv.push(v.interpolate(ids.map(|i| m.uv[i])));
                        m.classes.push(class);
                        m.eras.push(m.eras[ids[0]]);
                    }
                    m.indices.extend_from_slice(&[k, k + 1, k + 2]);
                }
            }
        }
    }
}
