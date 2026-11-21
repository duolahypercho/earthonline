//! Stage A6 (joins) and A7 (tunnels) on the synthetic inputs of
//! `synthetic.rs`: a short bridge held as end too low is built on its
//! approach embankments; a structure end where a street passes through is a
//! leg of a junction and the deck starts at its mouth; a tunnel through a
//! hill has its bore, portals, lights, cuts and collision, and nothing
//! without `--tunnels`.

use eo_street::Layer;

use crate::synthetic::{Case, built, case, flat, solve, standards, way};
use crate::{BORE_CLEAR_M, ChainKind, EndKind, Hold, Status, StructureClass, StructureOptions, StructureWorld, rebuild_streets};

type Tri = [[f64; 3]; 3];

/// Solved and meshed (as `mesh_tests::meshed`).
fn meshed_on(c: &Case, options: StructureOptions) -> StructureWorld {
    let mut w = solve(c, vec![], options);
    let ca = standards();
    let lane = ca.metric("travel-lane-width-state-highway-min").unwrap();
    w.attach_meshes(&c.streets, &ca, &|_| lane).unwrap();
    w
}

/// The two passes of `build_joined` on a synthetic case: the joins the
/// first solve asks, the street built again with them, solved and meshed.
fn joined(c: &Case, options: StructureOptions) -> (Case, StructureWorld, StructureWorld) {
    let first = solve(c, vec![], options);
    let joins = first.street_joins(&c.streets);
    let c2 = Case { streets: rebuild_streets(&c.streets, joins), roads: c.roads.clone() };
    let w2 = meshed_on(&c2, options);
    (c2, first, w2)
}

fn tris(w: &StructureWorld, lod: u8, keep: &dyn Fn(StructureClass) -> bool) -> Vec<(Tri, StructureClass)> {
    let mut out = vec![];
    for k in w.tile_keys() {
        let Some(m) = w.render_tile(k, lod) else { continue };
        let p = |i: u32| m.positions[i as usize].map(|v| v as f64);
        for t in m.indices.chunks_exact(3) {
            let cl = StructureClass::of(m.classes[t[0] as usize]).unwrap();
            if keep(cl) {
                out.push(([p(t[0]), p(t[1]), p(t[2])], cl));
            }
        }
    }
    out
}

fn over(t: &Tri, x: f64, z: f64) -> Option<f64> {
    let (a, b, c) = (t[0], t[1], t[2]);
    let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
    let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
    let l3 = 1.0 - l1 - l2;
    (l1 >= -1e-9 && l2 >= -1e-9 && l3 >= -1e-9).then(|| l1 * a[1] + l2 * b[1] + l3 * c[1])
}

/// A 44 m motorway bridge over a residential street, at grade 300 m either
/// side: too short for its ends, at the street datum, to climb to the
/// clearance within the motorway's grade.
fn short_bridge() -> Case {
    let mut s = vec![];
    way(&mut s, "600", "motorway", 14.0, 0.0, [0.0, -322.0], [0.0, -22.0], false);
    way(&mut s, "601", "motorway", 14.0, 0.0, [0.0, -22.0], [0.0, 22.0], true);
    way(&mut s, "602", "motorway", 14.0, 0.0, [0.0, 22.0], [0.0, 322.0], false);
    way(&mut s, "603", "residential", 10.0, 3.0, [-300.0, 0.0], [300.0, 0.0], false);
    case(s, &[("600", 0, false, false), ("601", 1, true, false), ("602", 0, false, false), ("603", 0, false, false)], &flat)
}

#[test]
fn a_short_bridge_held_as_end_too_low_is_built_on_its_approach_embankments() {
    let c = short_bridge();
    let (c2, first, w) = joined(&c, StructureOptions::default());
    let held = built(&first, "601");
    assert!(matches!(held.hold(), Some(Hold::EndTooLow { .. })), "first pass: {:?}", held.status);
    let wants = held.end_wants;
    assert!(wants.iter().all(|w| w.is_some()), "{wants:?}");
    let d = built(&w, "601");
    assert_eq!(d.status, Status::Built, "{:?}", d.status);
    assert_eq!((d.ends[0].kind, d.ends[1].kind), (EndKind::Paved, EndKind::Paved));
    // each end at the height the first pass wanted (plus the margin), on the
    // raised stub, and the deck clears the street.
    for e in 0..2 {
        let want = wants[e].unwrap() + crate::RISE_MARGIN_M;
        assert!((d.ends[e].fixed.unwrap() - want).abs() < 0.02, "end {e}: {:?} vs {want}", d.ends[e].fixed);
    }
    let x = d.crossings.iter().find(|x| x.over.contains("Way 603")).expect("the street is crossed");
    assert!(x.achieved >= x.clearance - 1e-6, "{x:?}");
    // the stubs climb to it within the motorway's grade, over the runs they
    // continue into, and are back at the street datum a ramp's length out.
    let datum = c.streets.ground(0.0, 0.0) + c.streets.o.road_lift;
    let rise = standards().metric_key("freeway-profile-grade-max", "rolling").unwrap();
    let mut raised = 0;
    for (ei, e) in c2.streets.plan.entries.iter().enumerate() {
        if !e.structure_ends.iter().any(Option::is_some) && e.structure_ramps.is_empty() {
            continue;
        }
        let g = &c2.streets.geoms[ei];
        let top = (0..g.stations.len()).map(|i| c2.streets.datum_raise(ei, i)).fold(0.0, f64::max);
        if top > 0.01 {
            raised += 1;
        }
        for (a, b) in g.datums.windows(2).zip(g.stations.windows(2)) {
            assert!((a[1] - a[0]).abs() <= rise * (b[1].s - b[0].s) + 1e-9, "run {}: steeper than {rise}", e.id);
        }
    }
    assert!(raised >= 4, "{raised} runs raised");
    for e in 0..2 {
        let to = wants[e].unwrap() + crate::RISE_MARGIN_M;
        let sgn = if d.ends[e].at[1] < 0.0 { -1.0 } else { 1.0 };
        let reach = (to - datum) / rise;
        let at = |dz: f64| c2.streets.paved_at(0.0, sgn * (22.0 + dz)).unwrap();
        let plain = |dz: f64| c.streets.paved_at(0.0, sgn * (22.0 + dz)).unwrap();
        assert!((at(reach + 5.0) - plain(reach + 5.0)).abs() < 1e-6, "end {e}: still raised {} m past the ramp", at(reach + 5.0) - plain(reach + 5.0));
        assert!((at(reach / 2.0) - plain(reach / 2.0) - (to - datum) / 2.0).abs() < 0.05, "end {e}: halfway up the ramp");
        // the deck meets the drawn stub at its end.
        let crown = c2.streets.o.cross_slope * d.pieces[0].half;
        assert!((at(0.05) - crown - d.ends[e].fixed.unwrap()).abs() < 0.05, "end {e}: stub {} vs deck {:?}", at(0.05), d.ends[e].fixed);
    }
    // the deck is drawn and collides.
    assert!(!tris(&w, 0, &|c| c == StructureClass::DeckSurface).is_empty());
}

/// A street through (0, 0) as two runs; a ramp deck leaves it there along
/// +z over a street at z = 140 and lands on a stub at z = 260.
fn leg_case() -> Case {
    let mut s = vec![];
    way(&mut s, "700", "residential", 12.0, 3.0, [-200.0, 0.0], [0.0, 0.0], false);
    way(&mut s, "701", "tertiary", 12.0, 3.0, [0.0, 0.0], [200.0, 0.0], false);
    way(&mut s, "702", "motorway_link", 9.0, 0.0, [0.0, 0.0], [0.0, 260.0], true);
    way(&mut s, "703", "motorway_link", 9.0, 0.0, [0.0, 260.0], [0.0, 500.0], false);
    way(&mut s, "704", "residential", 10.0, 3.0, [-300.0, 140.0], [300.0, 140.0], false);
    case(s, &[("700", 0, false, false), ("701", 0, false, false), ("702", 1, true, false), ("703", 0, false, false), ("704", 0, false, false)], &flat)
}

#[test]
fn a_structure_end_where_a_street_passes_through_is_a_junction_leg_and_the_deck_starts_at_its_mouth() {
    let c = leg_case();
    let (c2, first, w) = joined(&c, StructureOptions::default());
    let d0 = built(&first, "702");
    assert!(d0.is_built());
    assert!(c.streets.plan.nodes.is_empty(), "no junction without the joins");
    // a junction at (0, 0) with the leg; the stub at z = 260 capped.
    let plan = &c2.streets.plan;
    let node = plan.nodes.iter().find(|n| n.position[0].hypot(n.position[1]) < 0.7).expect("a junction at the structure's end");
    let app = node.approaches.iter().find(|a| plan.entries[a.entry].structure_leg.is_some()).expect("the leg");
    assert_eq!(node.approaches.len(), 3);
    assert!(plan.entries.iter().any(|e| e.structure_ends.iter().any(Option::is_some)), "the far stub is marked");
    // the deck is cut back to the mouth: its end there is the leg's trim
    // from the node, at the junction pad's surface.
    let d = built(&w, "702");
    assert!(d.is_built(), "{:?}", d.status);
    let start = d.points[0];
    assert!((start[1] - app.trim).abs() < 0.01 && start[0].abs() < 1e-6, "deck starts at {start:?}, the mouth at {}", app.trim);
    assert!((d0.length() - d.length() - app.trim).abs() < 0.01);
    assert_eq!(d.ends[0].kind, EndKind::Paved);
    let pad = c2.streets.paved_at(0.0, app.trim - 0.05).unwrap();
    let crown = c2.streets.o.cross_slope * d.pieces[0].half;
    assert!((d.ends[0].fixed.unwrap() + crown - pad).abs() < 0.05, "deck end {:?} vs pad {pad}", d.ends[0].fixed);
    // no kerb across the mouth: the drawn street over the mouth is the pad,
    // and the deck's own top takes over past it.
    for k in -3..=3 {
        let x = k as f64;
        assert_eq!(c2.streets.paved_layer_at(x, app.trim - 0.2), Some(Layer::Carriageway));
        assert_eq!(c.streets.paved_layer_at(x, c.streets.plan.entries[0].half + 1.0).map(|l| l.walkable()), Some(true), "the footway ran across before");
        let top = w.deck_top(d.id, x, app.trim + 1.0).expect("the deck at its mouth");
        assert!((top - pad).abs() < 0.3, "deck top {top} by the pad {pad}");
    }
}

/// A primary road through a 40 m hill: at grade, a tunnel, at grade.
fn hill_tunnel() -> Case {
    let hill = |x: f64, _z: f64| 40.0 * (-(x * x) / (2.0 * 110.0 * 110.0)).exp();
    let mut s = vec![];
    way(&mut s, "500", "primary", 12.0, 0.0, [-500.0, 0.0], [-320.0, 0.0], false);
    way(&mut s, "501", "primary", 12.0, 0.0, [-320.0, 0.0], [320.0, 0.0], true);
    way(&mut s, "502", "primary", 12.0, 0.0, [320.0, 0.0], [500.0, 0.0], false);
    case(s, &[("500", 0, false, false), ("501", -1, false, true), ("502", 0, false, false)], &hill)
}

#[test]
fn a_tunnel_through_a_hill_has_its_bore_portals_lights_cuts_and_collision() {
    let c = hill_tunnel();
    let off = meshed_on(&c, StructureOptions::default());
    assert_eq!(built(&off, "501").status, Status::TunnelOff);
    assert!(off.tile_keys().is_empty() && off.terrain_cuts().is_empty() && off.all_tunnel_lights().is_empty());
    let w = meshed_on(&c, StructureOptions { tunnels: true });
    let t = built(&w, "501");
    assert!(t.is_built() && t.kind == ChainKind::Tunnel);
    let concrete = tris(&w, 0, &|c| c == StructureClass::Concrete);
    let road = tris(&w, 0, &|c| c == StructureClass::DeckSurface);
    assert!(!concrete.is_empty() && !road.is_empty());
    assert!(!tris(&w, 0, &|c| c == StructureClass::Metal).is_empty(), "the luminaire strip");
    // under the hill: the roadway on the profile, the vault at the bore's
    // clear height over it, the ground over the vault.
    let (s, _) = t.project(0.0, 0.0);
    let y = t.y_at(s);
    let floor = road.iter().filter_map(|(tr, _)| over(tr, 0.0, 0.0)).fold(f64::NEG_INFINITY, f64::max);
    assert!((floor - y).abs() < 0.02, "roadway {floor} vs profile {y}");
    let vault = concrete.iter().filter_map(|(tr, _)| over(tr, 0.0, 0.0)).filter(|&v| v > y + 1.0).fold(f64::INFINITY, f64::min);
    assert!((vault - (y + BORE_CLEAR_M)).abs() < 0.2, "vault {vault} over the roadway {y}");
    assert!(c.streets.ground(0.0, 0.0) > vault + 1.0);
    assert_eq!(w.deck_top(t.id, 0.0, 1.0).map(|v| (v - y).abs() < 0.05), Some(true));
    // lights under the crown along the covered bore, a pitch apart.
    let lights = w.all_tunnel_lights();
    assert!(lights.len() >= 20, "{} lights", lights.len());
    for l in &lights {
        let ty = t.y_at(l.s);
        assert!((l.pos[1] - (ty + BORE_CLEAR_M - crate::tunnel::LIGHT_DROP_M)).abs() < 0.05 && (l.floor - ty).abs() < 0.05);
    }
    for p in lights.windows(2) {
        assert!((p[1].s - p[0].s - crate::tunnel::LIGHT_PITCH_M).abs() < 1e-6);
    }
    // the open stretches by the portals cut the ground, one at each end, and
    // each portal's canopy leaves the ground out a cell back over the bore.
    let cuts = w.terrain_cuts();
    assert_eq!(cuts.iter().filter(|k| !k.canopy).count(), 2, "{cuts:?}");
    assert_eq!(cuts.iter().filter(|k| k.canopy).count(), 2, "{cuts:?}");
    let end = t.frame(1.0);
    let cut = w.terrain_cut_at(end.x, end.z).expect("a cut by the portal");
    assert!(cut.floor < t.y_at(1.0) && cut.s0 <= 1e-6 && !cut.canopy);
    assert!(w.terrain_cut_at(0.0, 0.0).is_none(), "no cut over the covered bore");
    assert!(w.terrain_cut_at(end.x, end.z + 40.0).is_none());
    let portal_s = cut.s1;
    let behind = t.frame(portal_s + crate::tunnel::PORTAL_OPEN_M / 2.0);
    assert!(w.terrain_cut_at(behind.x, behind.z).is_some_and(|k| k.canopy), "the canopy over the bore behind the portal");
    // a portal headwall: concrete facing out along the chain at the face.
    let face = t.frame(portal_s);
    let facing = concrete.iter().filter(|(tr, _)| {
        let n = crate::mesh::norm(crate::mesh::cross(crate::mesh::sub(tr[1], tr[0]), crate::mesh::sub(tr[2], tr[0])));
        let mid = [(tr[0][0] + tr[1][0] + tr[2][0]) / 3.0, (tr[0][2] + tr[1][2] + tr[2][2]) / 3.0];
        (n[0] * face.tx + n[2] * face.tz).abs() > 0.95 && (mid[0] - face.x).hypot(mid[1] - face.z) < 25.0
    });
    assert!(facing.count() >= 4, "a headwall at the portal");
    // collision: the roadway walks, the bore blocks.
    let key = eo_street::tile_key(0.0, 0.0);
    let surf = w.surface_tile(key).expect("the bore collides");
    assert!(surf.street.iter().filter_map(|tr| over(tr, 0.0, 0.0)).any(|v| (v - y).abs() < 0.02));
    assert!(surf.barrier.iter().filter_map(|tr| over(tr, 0.0, 0.0)).any(|v| (v - (y + BORE_CLEAR_M)).abs() < 0.2));
    // the same built twice.
    let w2 = meshed_on(&c, StructureOptions { tunnels: true });
    assert_eq!(w.render_tile(key, 0), w2.render_tile(key, 0));
    assert_eq!(w.terrain_cuts(), w2.terrain_cuts());
}

/// Stage A8: the bore (the covered stretch) is where the renderer shuts out
/// the sun and the sky: a point inside it is found with its station, how far
/// in from the nearer portal and its height over the roadway; the hillside
/// over it, the open cut and the air beside it are not in it; none without
/// `--tunnels`.
#[test]
fn a_tunnels_bore_is_found_inside_its_walls_from_its_floor_to_its_roof() {
    let c = hill_tunnel();
    let off = meshed_on(&c, StructureOptions::default());
    assert!(off.bores().is_empty() && off.bore_at(0.0, 0.0, 0.0).is_none());
    let w = meshed_on(&c, StructureOptions { tunnels: true });
    let t = built(&w, "501");
    assert_eq!(w.bores().len(), 1, "one bore between the two portals");
    let b = &w.bores()[0];
    assert!(b.portal == [true, true] && b.s0 > 0.0 && b.s1 < t.length());
    let (s, _) = t.project(0.0, 0.0);
    let y = t.y_at(s);
    let mid = w.bore_at(0.0, y + 1.7, 0.0).expect("the eye of a driver in the bore");
    assert!((mid.s - s).abs() < 0.5 && (mid.above - 1.7).abs() < 0.05, "{mid:?}");
    assert!((mid.depth - (s - b.s0).min(b.s1 - s)).abs() < 0.5, "{mid:?} in {}..{}", b.s0, b.s1);
    // inside the walls, under the vault; not over the roof, not past the walls' backs.
    let f = t.frame(s);
    let inner = t.half_at(s) + crate::tunnel::WALKWAY_M;
    assert!(w.bore_at(f.x + f.nx * (inner - 0.2), y + 3.0, f.z + f.nz * (inner - 0.2)).is_some());
    assert!(w.bore_at(0.0, y + BORE_CLEAR_M + 3.0, 0.0).is_none(), "the hill over the bore");
    let past = inner + crate::tunnel::WALL_M + 1.0;
    assert!(w.bore_at(f.x + f.nx * past, y + 1.7, f.z + f.nz * past).is_none(), "beside it");
    // near a portal: in it, a few metres in; the open cut outside it is not.
    let near = t.frame(b.s0 + 3.0);
    let h = w.bore_at(near.x, t.y_at(b.s0 + 3.0) + 1.0, near.z).expect("just inside the portal");
    assert!((h.depth - 3.0).abs() < 0.5, "{h:?}");
    // its portal: the face's point, the direction into the bore (3 m in along it).
    let p = h.portal.expect("a portal");
    let face = t.frame(b.s0);
    assert!((p[0] - face.x).hypot(p[1] - face.z) < 0.05 && ((near.x - p[0]) * p[2] + (near.z - p[1]) * p[3] - 3.0).abs() < 0.3, "{p:?}");
    let cut = t.frame(b.s0 - 5.0);
    assert!(w.bore_at(cut.x, t.y_at(b.s0 - 5.0) + 1.0, cut.z).is_none(), "the open cut");
}

/// Stage A9: where the joins lower a stub into a tunnel's approach cut, the
/// stub is walled (concrete along its edges, from under the edge up past the
/// ground its datum stood at, with a coping), and each bore's portal has its
/// own arch standing proud of the headwall; on a street built without the
/// joins no wall stands; a building reaching over the cut is found.
#[test]
fn a_stub_lowered_into_a_portals_cut_is_walled_and_its_portal_has_an_arch() {
    let c = hill_tunnel();
    let plain = meshed_on(&c, StructureOptions { tunnels: true });
    assert_eq!(plain.census().walls, 0, "no joins, no walls");
    let (c2, _, w) = joined(&c, StructureOptions { tunnels: true });
    let t = built(&w, "501");
    assert!(t.is_built() && t.kind == ChainKind::Tunnel);
    assert!(w.census().walls >= 2, "the two stubs' cuts are walled: {}", w.census().walls);
    let concrete = tris(&w, 0, &|c| c == StructureClass::Concrete);
    // along a lowered stub (past the chain's ends, over the street): a wall
    // face, its top over the stub by more than a kerb face.
    let mut walled = 0;
    for (e, g) in c2.streets.plan.entries.iter().zip(&c2.streets.geoms) {
        if e.structure_ends.iter().all(Option::is_none) {
            continue;
        }
        for (st, &datum) in g.stations.iter().zip(&g.datums) {
            let lowered = c2.streets.ground(st.x, st.z) + c2.streets.o.road_lift - datum;
            if lowered < 1.0 {
                continue;
            }
            let edge = st.at(e.half + e.walks[0].max(0.0) + 0.3);
            let top = concrete.iter().filter_map(|(tr, _)| over(tr, edge[0], edge[1])).fold(f64::NEG_INFINITY, f64::max);
            assert!(top > datum + lowered - 0.5, "a wall over the stub's edge at s {:.1}: top {top:.2}, datum {datum:.2}, lowered {lowered:.2}", st.s);
            walled += 1;
        }
    }
    assert!(walled > 0, "no station of a stub was lowered");
    // the portal's arch: concrete facing out along the chain, standing proud of the face.
    let b = &w.bores()[0];
    let face = t.frame(b.s0);
    let out = [-face.tx, -face.tz];
    let proud = concrete.iter().filter(|(tr, _)| {
        let n = crate::mesh::norm(crate::mesh::cross(crate::mesh::sub(tr[1], tr[0]), crate::mesh::sub(tr[2], tr[0])));
        let mid = [(tr[0][0] + tr[1][0] + tr[2][0]) / 3.0, (tr[0][2] + tr[1][2] + tr[2][2]) / 3.0];
        let d = (mid[0] - face.x) * out[0] + (mid[1] - face.z) * out[1];
        (n[0] * face.tx + n[2] * face.tz).abs() > 0.95 && (d - crate::tunnel::ARCH_PROUD_M).abs() < 0.02 && (mid[0] - face.x).hypot(mid[1] - face.z) < 12.0
    });
    assert!(proud.count() >= 8, "the arch's band round the opening");
    // a building over the portal's cut is found; one away from it is not.
    let end = t.frame(1.0);
    let sq = |x: f64, z: f64, r: f64| vec![[x - r, z - r], [x + r, z - r], [x + r, z + r], [x - r, z + r]];
    let found = w.buildings_over_cuts(&[sq(end.x, end.z, 4.0), sq(end.x + 300.0, end.z + 300.0, 4.0)]);
    assert_eq!(found, vec![0]);
    assert!(plain.buildings_over_cuts(&[sq(end.x, end.z, 4.0)]) == vec![0], "the cuts are the tunnel's, joined or not");
    // the same built twice.
    let (_, _, w2) = joined(&c, StructureOptions { tunnels: true });
    for k in w.tile_keys() {
        assert_eq!(w.render_tile(k, 0), w2.render_tile(k, 0));
    }
}

/// Stage A9: a structure's era (how weathered `--grime` draws it) from its
/// name and its place: the Presidio Parkway new, the Central Freeway's
/// viaduct and the Bay Bridge's West Approach rebuilt, the rest old; every
/// vertex of its meshes carries it.
#[test]
fn a_structures_era_follows_its_name_and_the_west_approach() {
    let c = short_bridge();
    let (_, _, w) = joined(&c, StructureOptions::default());
    let bridge = built(&w, "601").clone();
    assert!(bridge.is_built());
    let named = |n: &str| {
        let mut b = bridge.clone();
        for p in b.pieces.iter_mut() {
            p.name = n.to_string();
        }
        b
    };
    let scale = eo_data::groundscale::scale();
    assert_eq!(crate::era_of(&bridge, w.chains(), scale), crate::Era::Old);
    assert_eq!(crate::era_of(&named("Presidio Parkway"), w.chains(), scale), crate::Era::New);
    assert_eq!(crate::era_of(&named("Central Freeway"), w.chains(), scale), crate::Era::Rebuilt);
    // a Bay Bridge deck's end within the West Approach's reach.
    let mut bay = bridge.clone();
    bay.landmark = Some(crate::LandmarkDeck::BayBridgeUpper);
    bay.points = vec![[bridge.points[0][0] + 600.0, bridge.points[0][1]], [bridge.points[0][0] + 900.0, bridge.points[0][1]]];
    let chains = vec![bridge.clone(), bay.clone()];
    assert_eq!(crate::era_of(&bridge, &chains, scale), crate::Era::Rebuilt);
    bay.points = vec![[bridge.points[0][0] + 5000.0, 0.0], [bridge.points[0][0] + 5300.0, 0.0]];
    assert_eq!(crate::era_of(&bridge, &[bridge.clone(), bay], scale), crate::Era::Old);
    // every vertex carries its chain's era.
    for k in w.tile_keys() {
        for lod in 0..3 {
            if let Some(m) = w.render_tile(k, lod) {
                assert_eq!(m.eras.len(), m.positions.len());
                assert!(m.eras.iter().all(|&e| e == crate::Era::Old as u8));
            }
        }
    }
}
