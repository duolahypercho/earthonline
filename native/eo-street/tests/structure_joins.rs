//! The street's joins with the grade-separated roads (`eo_street::joins`,
//! stage A6) on small synthetic inputs over flat ground: a structure end
//! where a street passes through becomes a leg of a junction there, with no
//! kerb across its mouth; a stub ending at a structure is capped and, when
//! the structure asks, raised on a ramp within its grade; without joins the
//! plan and every mesh are exactly as before.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use eo_data::{ElevationGrid, ElevationMeta};
use eo_street::city::Segment;
use eo_street::joins::{Rise, StructureEnd, StructureJoins};
use eo_street::{BuildingIndex, Layer, StreetCity, StreetOptions, StreetWorld, Terrain, TileMesh};

fn grid() -> ElevationGrid {
    let (cell, origin, cells) = (8.0, -600.0, 150usize);
    let nodata = -32768;
    ElevationGrid {
        meta: ElevationMeta { version: "synthetic".into(), origin_x: origin, origin_z: origin, cell_size: cell, width: cells, height: cells, nodata },
        lidar: vec![0.0; cells * cells],
        topobathy: vec![nodata as i16; cells * cells],
        height_scale: 1.0,
    }
}

fn seg(id: &str, highway: &str, width: f64, walk: f64, points: &[[f64; 2]], separated: bool) -> Segment {
    Segment {
        key: Some(format!("sf-street-{id}:0")),
        id: format!("sf-seg-{id}"),
        street_name: format!("Way {id}"),
        street_id: Some(format!("sf-street-{id}")),
        highway: highway.into(),
        width,
        lanes: Some(2.0),
        oneway: "both".into(),
        sidewalk_w: Some(walk),
        sidewalk_left: Some(walk),
        sidewalk_right: Some(walk),
        points: points.to_vec(),
        separated,
        lane_layout: None,
        parking: [0.0; 2],
        cycle: [0.0; 2],
        cycle_kerbside: [false; 2],
        cycle_buffer: [0.0; 2],
        shoulder: [0.0; 2],
        median: 0.0,
        street_type: None,
        maxspeed_kmh: None,
        maxspeed: String::new(),
        right_of_way: None,
        pedestrian_eligible: true,
    }
}

fn world(segments: Vec<Segment>, joins: Option<StructureJoins>) -> StreetWorld {
    let o = StreetOptions::load().unwrap().with_structures(joins.map(Arc::new));
    let city = StreetCity { segments, intersections: vec![], crossings: vec![], crossing_nodes: vec![], continental: vec![] };
    let lift = o.open_ground_lift;
    StreetWorld::build(o, city, Terrain { grid: Arc::new(grid()), carpet_lift: lift }, BuildingIndex::new(vec![]))
}

fn meshes(w: &StreetWorld) -> Vec<TileMesh> {
    w.tile_keys().into_iter().map(|k| w.tile_mesh(k)).collect()
}

fn digest(w: &StreetWorld) -> u64 {
    let mut h = DefaultHasher::new();
    for e in &w.plan.entries {
        (&e.id, &e.pieces, e.trim_start.to_bits(), e.trim_end.to_bits(), e.approach_start, e.approach_end).hash(&mut h);
    }
    for n in &w.plan.nodes {
        (n.position[0].to_bits(), n.position[1].to_bits(), n.approaches.len()).hash(&mut h);
    }
    for k in w.tile_keys() {
        for m in [w.tile_mesh(k), w.far_tile_mesh(k), w.coarse_tile_mesh(k)] {
            for p in &m.positions {
                p.map(f32::to_bits).hash(&mut h);
            }
            m.indices.hash(&mut h);
            m.colors.hash(&mut h);
        }
    }
    h.finish()
}

/// The kerb faces (CurbFace triangles) whose plan footprint passes within
/// `tol` of (x, z): a vertical face is a line in plan.
fn kerb_near(ms: &[TileMesh], x: f64, z: f64, tol: f64) -> bool {
    for m in ms {
        for (t, l) in m.indices.chunks_exact(3).zip(&m.layers) {
            if *l != Layer::CurbFace {
                continue;
            }
            let p = |i: u32| {
                let q = m.positions[i as usize];
                [q[0] as f64, q[2] as f64]
            };
            let [a, b, c] = [p(t[0]), p(t[1]), p(t[2])];
            for (u, v) in [(a, b), (b, c), (c, a)] {
                let (dx, dz) = (v[0] - u[0], v[1] - u[1]);
                let len2 = dx * dx + dz * dz;
                let s = if len2 < 1e-12 { 0.0 } else { (((x - u[0]) * dx + (z - u[1]) * dz) / len2).clamp(0.0, 1.0) };
                if (x - u[0] - dx * s).hypot(z - u[1] - dz * s) <= tol {
                    return true;
                }
            }
        }
    }
    false
}

/// A street passing through (0, 0) as two runs, and a structure (a
/// separated ramp) leaving it there along +z.
fn pass_through() -> Vec<Segment> {
    vec![
        seg("300", "residential", 12.0, 3.0, &[[-200.0, 0.0], [0.0, 0.0]], false),
        seg("301", "tertiary", 12.0, 3.0, &[[0.0, 0.0], [200.0, 0.0]], false),
        seg("302", "motorway_link", 9.0, 0.0, &[[0.0, 0.0], [0.0, 60.0], [0.0, 150.0]], true),
    ]
}

fn leg_end() -> StructureEnd {
    StructureEnd { at: [0.0, 0.0], line: vec![[0.0, 0.0], [0.0, 60.0], [0.0, 80.0]], half: 4.5, closed: 5.1, segment: 2, rise: None, cut: None }
}

#[test]
fn a_structure_leg_gets_a_junction_mouth_with_no_kerb_across_it() {
    let plain = world(pass_through(), None);
    // without joins: two run ends, no junction; the kerb runs straight across
    // where the structure leaves.
    assert!(plain.plan.nodes.is_empty(), "{} nodes", plain.plan.nodes.len());
    let half = plain.plan.entries[0].half;
    let ms = meshes(&plain);
    assert!(kerb_near(&ms, 0.0, half, 0.05) && kerb_near(&ms, 2.0, half, 0.05), "the kerb runs across without joins");
    let joined = world(pass_through(), Some(StructureJoins { ends: vec![leg_end()] }));
    // one leg, in a junction of three approaches at (0, 0).
    let legs: Vec<(usize, &eo_street::plan::Entry)> = joined.plan.entries.iter().enumerate().filter(|(_, e)| e.structure_leg.is_some()).collect();
    assert_eq!(legs.len(), 1);
    let (li, leg) = legs[0];
    assert_eq!(leg.structure_leg, Some(0));
    assert_eq!(joined.plan.nodes.len(), 1);
    let node = &joined.plan.nodes[0];
    assert!(node.position[0].hypot(node.position[1]) < 1e-6);
    assert_eq!(node.approaches.len(), 3);
    let app = node.approaches.iter().find(|a| a.entry == li).expect("the leg is an approach");
    assert!(app.trim > half, "the leg's mouth {:.2} m out, past the street's half width {half:.2}", app.trim);
    // the leg is not drawn: no stations, no paint, no crosswalk on it.
    assert!(joined.geoms[li].stations.is_empty() && joined.lines[li].is_empty());
    assert!(!app.crossing_painted);
    // the corner paths end at the leg's mouth corners (its own end frame).
    let mouths: Vec<_> = node.paths.iter().flat_map(|p| p.stations.iter().filter_map(|s| s.mouth)).filter(|m| m.0 == li).collect();
    assert_eq!(mouths.len(), 2, "both corners meet the leg's mouth");
    for m in &mouths {
        assert!((m.2.abs() - leg.half).abs() < 1e-9 && (m.1 - app.trim).abs() < 1e-9, "{m:?}");
    }
    // no kerb across the mouth: nothing over it from corner to corner (but at
    // the corners themselves), the pad paved up to it.
    let ms = meshes(&joined);
    for k in 1..9 {
        let x = -leg.half * 0.8 + 1.6 * leg.half * k as f64 / 9.0;
        for z in [half, app.trim - 0.05, app.trim] {
            assert!(!kerb_near(&ms, x, z, 0.05), "a kerb face across the mouth at ({x:.2}, {z:.2})");
        }
        assert_eq!(joined.paved_layer_at(x, app.trim - 0.2), Some(Layer::Carriageway), "the pad reaches the mouth at x {x:.2}");
    }
    // the kerb still runs along the far side of the street.
    assert!(kerb_near(&ms, 0.0, -half, 0.05));
    // the paved runs' corridors know nothing of the leg (it is no street).
    assert!(!joined.on_other_carriageway(0.0, 40.0, usize::MAX));
}

#[test]
fn a_stub_meeting_a_structure_is_capped_and_raised_within_its_grade() {
    let segs = vec![seg("400", "residential", 12.0, 3.0, &[[-300.0, 0.0], [0.0, 0.0]], false), seg("401", "motorway", 14.0, 0.0, &[[0.0, 0.0], [120.0, 0.0]], true)];
    let plain = world(segs.clone(), None);
    let datum = plain.ground(0.0, 0.0) + plain.o.road_lift;
    let rise = Rise { to: datum + 2.0, grade: 0.06 };
    let end = StructureEnd { at: [0.0, 0.0], line: vec![[0.0, 0.0], [80.0, 0.0]], half: 7.0, closed: 7.61, segment: 1, rise: Some(rise), cut: None };
    let joined = world(segs, Some(StructureJoins { ends: vec![end] }));
    let e = &joined.plan.entries[0];
    assert_eq!(e.structure_ends, [None, Some(0)]);
    assert!(e.approach_end.is_none());
    // the datum climbs on the ramp to the end, and is the ground's own past it.
    let g = &joined.geoms[0];
    let n = g.stations.len();
    assert!((g.datums[n - 1] - rise.to).abs() < 1e-9, "end datum {}", g.datums[n - 1]);
    for i in 0..n {
        let d = e.length - g.stations[i].s;
        let want = (rise.to - rise.grade * d).max(datum);
        assert!((g.datums[i] - want).abs() < 1e-9, "station {i}: {} vs {want}", g.datums[i]);
    }
    for w in g.datums.windows(2).zip(g.stations.windows(2)) {
        assert!((w.0[1] - w.0[0]).abs() <= rise.grade * (w.1[1].s - w.1[0].s) + 1e-9);
    }
    let len = 2.0 / rise.grade;
    let drawn = |w: &StreetWorld, x: f64| w.paved_at(x, 0.0).unwrap();
    assert!((drawn(&joined, -0.3) - drawn(&plain, -0.3) - (2.0 - 0.3 * rise.grade)).abs() < 0.02);
    assert!((drawn(&joined, -len - 10.0) - drawn(&plain, -len - 10.0)).abs() < 1e-6);
    // the end is capped beyond what the structure's end face closes: a
    // face across the run at its end, on the footway side, none on the roadway.
    let ms = meshes(&joined);
    let half = e.half;
    // (the structure's end face closes to 7.61 m; the footway runs from 6 to 9 m.)
    for z in [half + 2.7, -(half + 2.7)] {
        assert!(kerb_near(&ms, -0.02, z, 0.05), "a cap over the footway at z {z}");
    }
    assert!(!kerb_near(&ms, -0.02, 0.0, 0.05) && !kerb_near(&ms, -0.02, 7.0, 0.05), "no cap across the structure's own end face");
    let plain_ms = meshes(&plain);
    assert!(!kerb_near(&plain_ms, -0.02, half + 2.7, 0.05), "no cap without joins");
    // the raised stretch has its banks (a side without footway would get one too).
    assert!(ms.iter().map(|m| m.layers.iter().filter(|l| **l == Layer::Verge).count()).sum::<usize>() >= plain_ms.iter().map(|m| m.layers.iter().filter(|l| **l == Layer::Verge).count()).sum::<usize>());
}

#[test]
fn a_rise_the_stub_cannot_meet_before_its_far_end_is_not_drawn() {
    // a 20 m stub cannot climb 3 m at 6 %: left at the street datum.
    let segs = vec![seg("500", "residential", 12.0, 3.0, &[[-20.0, 0.0], [0.0, 0.0]], false), seg("501", "motorway", 14.0, 0.0, &[[0.0, 0.0], [120.0, 0.0]], true)];
    let plain = world(segs.clone(), None);
    let datum = plain.ground(0.0, 0.0) + plain.o.road_lift;
    let end = StructureEnd { at: [0.0, 0.0], line: vec![[0.0, 0.0], [80.0, 0.0]], half: 7.0, closed: 7.61, segment: 1, rise: Some(Rise { to: datum + 3.0, grade: 0.06 }), cut: None };
    let joined = world(segs, Some(StructureJoins { ends: vec![end] }));
    assert!(joined.geoms[0].datums.iter().all(|d| (d - datum).abs() < 1e-9));
}

#[test]
fn a_stub_meeting_a_tunnel_portal_is_lowered_into_its_cut_with_the_carpet_under_it() {
    // the ground rises 6 m over the stub's last 30 m (a hillside over a
    // portal, as the bare-earth grid draws it); the portal asks the street to
    // come down to 1 m over the flat ground, at 6 %.
    let hill = |x: f64| if x > -30.0 { 6.0 * (x + 30.0) / 30.0 } else { 0.0 };
    let segs = vec![seg("800", "primary", 12.0, 0.0, &[[-300.0, 0.0], [0.0, 0.0]], false), seg("801", "primary", 12.0, 0.0, &[[0.0, 0.0], [200.0, 0.0]], true)];
    let mut g = grid();
    for j in 0..g.meta.height {
        for i in 0..g.meta.width {
            let x = g.meta.origin_x + (i as f64 + 0.5) * g.meta.cell_size;
            g.lidar[j * g.meta.width + i] = hill(x) as f32;
        }
    }
    let build = |joins: Option<StructureJoins>| {
        let o = StreetOptions::load().unwrap().with_structures(joins.map(Arc::new));
        let city = StreetCity { segments: segs.clone(), intersections: vec![], crossings: vec![], crossing_nodes: vec![], continental: vec![] };
        let lift = o.open_ground_lift;
        StreetWorld::build(o, city, Terrain { grid: Arc::new(g.clone()), carpet_lift: lift }, BuildingIndex::new(vec![]))
    };
    let plain = build(None);
    let lift = plain.o.road_lift;
    let cut = Rise { to: 1.0 + lift, grade: 0.06 };
    let end = StructureEnd { at: [0.0, 0.0], line: vec![[0.0, 0.0], [80.0, 0.0]], half: 6.0, closed: 6.0, segment: 1, rise: None, cut: Some(cut) };
    let joined = build(Some(StructureJoins { ends: vec![end] }));
    let gm = &joined.geoms[0];
    let n = gm.stations.len();
    assert!((gm.datums[n - 1] - cut.to).abs() < 1e-9, "the stub meets the portal at {}", gm.datums[n - 1]);
    for i in 0..n {
        let st = &gm.stations[i];
        let datum = plain.ground(st.x, st.z) + lift;
        assert!((gm.datums[i] - datum.min(cut.to + cut.grade * (300.0 - st.s))).abs() < 1e-9);
    }
    // the carpet is held under the lowered street, and only there (past the
    // stub's end, over the portal, the tunnel's own cut takes the ground).
    let at = |w: &StreetWorld, x: f64| (w.paved_at(x, 0.0).unwrap(), w.carpet_y(x, 0.0));
    let (street, carpet) = at(&joined, -10.0);
    assert!(street < at(&plain, -10.0).0 - 2.0, "lowered by {}", at(&plain, -10.0).0 - street);
    assert!(carpet < street, "the carpet {carpet} under the street {street}");
    assert_eq!(at(&joined, -120.0), at(&plain, -120.0));
}

#[test]
fn without_joins_the_plan_and_every_mesh_are_as_before() {
    let a = world(pass_through(), None);
    let b = world(pass_through(), Some(StructureJoins::default()));
    assert!(b.o.structures.is_none(), "an empty set of joins is no joins");
    assert_eq!(digest(&a), digest(&b));
    assert_eq!(a.o.structures_tag(), "");
    let c = world(pass_through(), Some(StructureJoins { ends: vec![leg_end()] }));
    assert_ne!(digest(&a), digest(&c));
    assert!(c.o.structures_tag().starts_with("street-structure-joins-v1:"));
    let d = world(pass_through(), Some(StructureJoins { ends: vec![leg_end()] }));
    assert_eq!(digest(&c), digest(&d), "joins build the same street twice");
}

#[test]
fn a_junction_keeps_paving_between_its_trimmed_mouths_at_all_three_distances() {
    let joined = world(pass_through(), Some(StructureJoins { ends: vec![leg_end()] }));
    let node = &joined.plan.nodes[0];
    let key = eo_street::tile_key(node.position[0], node.position[1]);
    let covers = |mesh: &TileMesh, p: [f64; 2]| {
        mesh.indices.chunks_exact(3).zip(&mesh.layers).any(|(ids, layer)| {
            if *layer != Layer::Carriageway { return false; }
            let [a,b,c] = [ids[0],ids[1],ids[2]].map(|i| mesh.positions[i as usize].map(f64::from));
            let side = |u:[f64;3],v:[f64;3]| (v[0]-u[0])*(p[1]-u[2])-(v[2]-u[2])*(p[0]-u[0]);
            let area = (b[0]-a[0])*(c[2]-a[2])-(b[2]-a[2])*(c[0]-a[0]);
            area.abs()>1e-9 && [side(a,b),side(b,c),side(c,a)].iter().all(|v|v*area.signum()>=-1e-7)
        })
    };
    for mesh in [joined.tile_mesh(key), joined.far_tile_mesh(key), joined.coarse_tile_mesh(key)] {
        for a in &node.approaches {
            let f=eo_street::plan::approach_end_frame(&joined.plan.entries,a);
            let halfway=[(f.x+node.position[0])*0.5,(f.z+node.position[1])*0.5];
            assert!(covers(&mesh,node.position) && covers(&mesh,halfway),"open junction on {}",a.entry);
        }
    }
}
