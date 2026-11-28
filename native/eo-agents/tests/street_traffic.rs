use eo_sim::{
    geom, native,
    network::{ControlKind, Network},
};
use eo_street::{StreetCity, StreetOptions, plan};

#[test]
fn folsom_traffic_and_walks_follow_their_paved_cross_section() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = StreetCity::load().unwrap();
    let options = StreetOptions::load().unwrap();
    let plan = plan::build_plan(&city, &options);
    let sections = plan::traffic_sections(&city, &plan, &options);
    let folsom = sections.iter().find(|s| s.way == "419633751").unwrap();
    let mut painted = folsom.lanes.clone();
    painted.sort_by(|a, b| b.lateral.total_cmp(&a.lateral));
    assert_eq!(painted.len(), 3);
    for adjacent in painted.windows(2) {
        let middle = (adjacent[0].lateral + adjacent[1].lateral) / 2.0;
        let divider = folsom.paint.iter().find(|mark| mark.divider == 1 && (mark.lateral - middle).abs() < 1e-6).unwrap();
        assert!(divider.broken && !divider.yellow);
        assert!(!divider.points.is_empty());
    }
    let [a, b] = [folsom.points[0], *folsom.points.last().unwrap()];
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let lateral = |p: [f64; 2]| ((p[0] - a[0]) * dz - (p[1] - a[1]) * dx) / dx.hypot(dz);
    let midpoint = |points: &[[f64; 2]]| {
        let cum = geom::cumulative(points);
        let (x, z, ..) = geom::at_arc(points, &cum, cum.last().unwrap() / 2.0);
        [x, z]
    };
    let half = folsom.half;
    let walks = folsom.walks;
    let moulton = sections.iter().find(|s| s.way == "287133787").unwrap();
    assert!(moulton.shared_bidirectional && moulton.lanes.is_empty());

    let source = native::load(&paths).unwrap();
    let [x, z] = eo_data::groundscale::plan([1539.0, -366.0]);
    let tiles = source.resident_tiles(x, z, 600.0);
    let plain = source.build(&tiles);
    let source = source.with_traffic_sections(sections);
    let aligned = source.build(&tiles);
    for (i, painted) in painted.iter().enumerate() {
        let key = format!("n419633751:0-forward-{}", i + 1);
        let before = plain.lanes.iter().find(|l| l.key == key).unwrap();
        let after = aligned.lanes.iter().find(|l| l.key == key).unwrap();
        assert_eq!(after.start_node, before.start_node);
        assert_eq!(after.end_node, before.end_node);
        assert_eq!(after.width, painted.width);
        assert!((lateral(midpoint(&after.points)) - painted.lateral).abs() < 0.02, "{key}");
        assert!(matches!(after.control, ControlKind::Signal { .. }));
    }
    let footways: Vec<_> = aligned.footways.iter().filter(|w| w.segment == "n419633751:0").collect();
    assert_eq!(footways.len(), 2);
    for walk in footways {
        let offset = lateral(midpoint(&walk.points)) * f64::from(walk.road_side);
        let width = walks[usize::from(walk.road_side > 0)];
        assert!(offset > half && offset < half + width, "{offset}");
    }

    let mut network = Network::new(source.timing.clone());
    network.add(aligned);
    let turn = network.lane_by_key("n419633751:0-forward-2").unwrap();
    let fourth = network.lane_by_key("n397145255:0-forward-1").unwrap();
    assert!(network.lanes[turn as usize].links.iter().any(|link| link.to == fourth && link.legal && link.drivable));

    let [x, z] = eo_data::groundscale::plan([-5027.0, -3328.0]);
    let tiles = source.resident_tiles(x, z, 600.0);
    let approach = source.build(&tiles);
    let plain = native::load(&paths).unwrap().build(&tiles);
    let ramp = "n543637222:0-forward-1";
    let baseline = plain.lanes.iter().find(|lane| lane.key == ramp).unwrap();
    let aligned = approach.lanes.iter().find(|lane| lane.key == ramp).unwrap();
    assert_eq!(baseline.points, aligned.points);
    assert_eq!(baseline.width, aligned.width);
    let service = "n485772590:0";
    assert!(plain.lanes.iter().any(|lane| lane.segment == service));
    assert!(approach.lanes.iter().all(|lane| lane.segment != service));

    // The road extract still tags Moulton as two lanes. Its drawn sidewalk
    // runs through one of those fallback lanes.
    let [x, z] = eo_data::groundscale::plan([-1314.2, -2392.6]);
    let tiles = source.resident_tiles(x, z, 600.0);
    let plain = native::load(&paths).unwrap().build(&tiles);
    let aligned = source.build(&tiles);
    assert_eq!(plain.lanes.iter().filter(|lane| lane.segment == "n287133787:0").count(), 2);
    assert!(aligned.lanes.iter().all(|lane| lane.segment != "n287133787:0"));
    assert_eq!(aligned.footways.iter().filter(|walk| walk.segment == "n287133787:0").count(), 2);
    assert!(aligned.lanes.iter().any(|lane| lane.segment == "n1105502012:0"));

    // Brady and Stevenson are mapped as two lanes but each has one paved
    // through lane. The second fallback lane put cars on the Brady sidewalk.
    let [x, z] = eo_data::groundscale::plan([-123.4, 533.9]);
    let tiles = source.resident_tiles(x, z, 600.0);
    let plain = native::load(&paths).unwrap().build(&tiles);
    let aligned = source.build(&tiles);
    for segment in ["n154647380:0", "n8917455:0"] {
        assert_eq!(plain.lanes.iter().filter(|lane| lane.segment == segment).count(), 2);
        let lane: Vec<_> = aligned.lanes.iter().filter(|lane| lane.segment == segment).collect();
        assert_eq!(lane.len(), 1, "{segment}");
        assert_eq!(lane[0].key, format!("{segment}-forward-1"));
        let [x, z] = midpoint(&lane[0].points);
        let footways: Vec<_> = aligned.footways.iter().filter(|walk| walk.segment == segment).collect();
        assert_eq!(footways.len(), 2, "{segment}");
        for walk in footways {
            assert!(geom::dist_to_polyline(&walk.points, x, z) > lane[0].width / 2.0);
        }
    }
}
