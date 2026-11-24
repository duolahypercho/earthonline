//! The crowd by place and hour (`RegionConfig::crowd`, eo-app `--crowd`):
//! the density model's shape, the disc's clipping, the frontage survey and
//! walks of their own on a natively built network, and the resident disc
//! holding what its footways hold, cuts included.

use eo_sim::consts::{self, CrowdDensity, FootActivity, footway_density, hour_footfall, night_footfall};
use eo_sim::geom::length_in_disc;
use eo_sim::native::{self, FrontEdge, NativeSource, PathWay};
use eo_sim::region::{Focus, Region, RegionConfig};
use eo_sim::{Network, World};

/// SoMa on the shipped (scaled) plan (tests/world.rs's bench spawn on Folsom Street's walk).
static SOMA: std::sync::LazyLock<[f64; 2]> = std::sync::LazyLock::new(|| eo_data::groundscale::plan([1539.0, -366.0]));

#[test]
fn a_line_is_clipped_to_the_disc() {
    let r = 150.0;
    assert!((length_in_disc(&[[-500.0, 0.0], [500.0, 0.0]], [0.0, 0.0], r) - 2.0 * r).abs() < 1e-9, "through the centre");
    assert_eq!(length_in_disc(&[[-500.0, 200.0], [500.0, 200.0]], [0.0, 0.0], r), 0.0, "outside");
    assert!((length_in_disc(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], [0.0, 0.0], r) - 20.0).abs() < 1e-9, "inside");
    let half = length_in_disc(&[[0.0, 90.0], [500.0, 90.0]], [0.0, 0.0], r);
    assert!((half - (r * r - 90.0f64 * 90.0).sqrt()).abs() < 1e-9, "a chord's half");
}

#[test]
fn density_follows_the_place_and_the_hour() {
    let none = FootActivity::default();
    let towers = FootActivity { shops: 0.5, night: 0.1, storeys: 15.0, place: 0.0 };
    let bars = FootActivity { shops: 0.3, night: 0.5, storeys: 3.0, place: 0.0 };
    let plaza = FootActivity { place: consts::PLACE_PEDESTRIAN, ..none };
    let per100 = |w: f64, a: FootActivity, h: f64| footway_density(w, a, h) * 100.0;
    // a residential street at noon: a person or two every 100 m; a service lane less.
    let res = per100(consts::sidewalk_class_weight("residential"), FootActivity { storeys: 2.5, ..none }, 12.0);
    assert!((0.5..3.0).contains(&res), "residential {res}");
    assert!(per100(consts::sidewalk_class_weight("service"), none, 12.0) < res);
    // a downtown block of towers and shops at its peak: 10-20 every 100 m.
    let dt = per100(1.0, towers, 12.0);
    assert!((10.0..20.0).contains(&dt), "downtown {dt}");
    // the night: a street of bars outlives the towers' evening, and both thin out before dawn.
    assert!(per100(0.95, bars, 21.0) > per100(1.0, FootActivity { night: 0.0, shops: 0.0, ..towers }, 21.0));
    assert!(per100(0.95, bars, 4.0) < 0.25 * per100(0.95, bars, 21.0));
    assert!(night_footfall(21.0) > night_footfall(13.0) && night_footfall(13.0) > night_footfall(4.0));
    // a plaza is busy of its own.
    assert!(per100(consts::sidewalk_class_weight("pedestrian"), plaza, 13.0) > 5.0);
    // more of any input is never fewer people.
    for h in [3.0, 9.0, 12.0, 18.5, 21.0, 23.5] {
        let base = footway_density(0.8, bars, h);
        for more in [FootActivity { shops: 0.9, ..bars }, FootActivity { night: 0.9, ..bars }, FootActivity { storeys: 9.0, ..bars }, FootActivity { place: 1.0, ..bars }] {
            assert!(footway_density(0.8, more, h) >= base, "{h} h");
        }
        assert!(footway_density(0.8, bars, h) > 0.0 && hour_footfall(h) > 0.0);
    }
}

/// A network round SoMa with one building front along every footway's outer
/// side (six storeys, shops all along, or bars with `night`) and one plaza
/// path through the focus.
fn soma(frontage: bool, path: bool) -> (Network, NativeSource) {
    soma_with(frontage, path, false)
}

fn soma_with(frontage: bool, path: bool, night: bool) -> (Network, NativeSource) {
    let base = native::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
    let tiles = native::tiles_around(SOMA[0], SOMA[1], 600.0);
    // the fronts: each built footway's line pushed out 3 m (its outer side).
    let plain = base.build(&tiles);
    let mut edges = vec![];
    for f in &plain.footways {
        let out = [f.normal[0] * f.road_side as f64 * 3.0, f.normal[1] * f.road_side as f64 * 3.0];
        for w in f.points.windows(2) {
            edges.push(FrontEdge { a: [w[0][0] + out[0], w[0][1] + out[1]], b: [w[1][0] + out[0], w[1][1] + out[1]], storeys: 6.0, shops: !night, night });
        }
    }
    let mut src = native::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
    if frontage {
        src = src.with_frontage(edges);
    }
    if path {
        let p = vec![[SOMA[0] - 40.0, SOMA[1] + 7.0], [SOMA[0], SOMA[1] + 9.0], [SOMA[0] + 40.0, SOMA[1] + 7.0]];
        src = src.with_paths(vec![PathWay { id: "test".into(), highway: "pedestrian".into(), points: p }]);
    }
    let mut net = Network::new(src.timing.clone());
    net.add(src.build(&tiles));
    (net, src)
}

#[test]
fn the_survey_measures_fronts_and_adds_walks_of_their_own() {
    let (plain, _) = soma(false, false);
    assert!(plain.footways.iter().all(|f| f.activity == FootActivity::default()), "no survey, no activity");
    let (net, _) = soma(true, true);
    let alive: Vec<_> = net.footways.iter().filter(|f| f.alive).collect();
    assert_eq!(alive.len(), plain.footways.iter().filter(|f| f.alive).count() + 1, "one walk of its own more");
    // most footways find their front along most of their length (corners and short stubs less).
    let fronted = alive.iter().filter(|f| f.activity.shops > 0.8 && (f.activity.storeys - 6.0).abs() < 1.5).count();
    assert!(fronted * 10 > alive.len() * 7, "{fronted} of {} fronted", alive.len());
    assert!(alive.iter().all(|f| f.activity.night == 0.0));
    let walk = alive.iter().find(|f| f.activity.place > 0.0).expect("the plaza path");
    assert_eq!(walk.activity.place, consts::PLACE_PEDESTRIAN);
    assert!(walk.routable);
}

#[test]
fn the_disc_holds_what_its_footways_hold_and_a_cut_rebuilds_it() {
    let (net, _) = soma(true, true);
    let mut world = World::new(net);
    let mut cfg = RegionConfig::game_default(1);
    cfg.hour = 12.0;
    cfg.crowd = Some(CrowdDensity::standard(1.0));
    let d = cfg.crowd.unwrap();
    let focus = Focus { x: SOMA[0], z: SOMA[1], yaw: 0.0 };
    world.regions.push(Region::new(&world.net, 0, cfg, focus));
    for _ in 0..120 {
        world.step();
    }
    let r = &world.regions[0];
    let want = r.crowd_census(&world.net, d, 12.0 + world.time / 3600.0);
    let n = r.crowd.walkers.len();
    assert!(want.people > 20.0, "{want:?}");
    assert!((n as f64 - want.people.min(d.max_bodies as f64)).abs() <= 3.0 + 0.05 * want.people, "{n} walkers for {:.1} expected", want.people);
    assert!(n <= d.max_bodies);
    // every walker inside the keep radius.
    for i in 0..n {
        assert!((r.crowd.x[i] - SOMA[0]).hypot(r.crowd.z[i] - SOMA[1]) <= d.keep_radius_m);
    }
    // the same disc at 03:00 holds far fewer: the hour, through a cut (a fresh region there).
    let mut night_cfg = cfg;
    night_cfg.hour = 3.0;
    let mut night = Region::new(&world.net, 0, night_cfg, focus);
    for s in 1..=120 {
        night.step(&world.net, s as f64 / 60.0, 1.0 / 60.0);
    }
    assert!((night.crowd.walkers.len() as f64) < 0.3 * n as f64, "{} at 03:00 against {n} at noon", night.crowd.walkers.len());
    // the JS disc without the crowd: its own fixed count, whatever the footways.
    let mut js = Region::new(&world.net, 0, RegionConfig::game_default(1), focus);
    for s in 1..=120 {
        js.step(&world.net, s as f64 / 60.0, 1.0 / 60.0);
    }
    assert!(js.crowd.walkers.len() <= consts::CrowdBubble::for_level(consts::DEFAULT_PEOPLE_LEVEL).max_bodies);
}

/// At a street of bars in the evening many walkers stand at the doors (the
/// door gathering); the same street of shops at noon keeps the JS pauses.
#[test]
fn nightlife_fronts_gather_people_at_their_doors() {
    let standing = |night: bool, hour: f64| {
        let (net, _) = soma_with(true, false, night);
        let mut cfg = RegionConfig::game_default(1);
        cfg.hour = hour;
        cfg.crowd = Some(CrowdDensity::standard(1.0));
        let mut r = Region::new(&net, 0, cfg, Focus { x: SOMA[0], z: SOMA[1], yaw: 0.0 });
        for s in 1..=(240 * 60) {
            r.step(&net, s as f64 / 60.0, 1.0 / 60.0);
        }
        let n = r.out.walkers.len().max(1);
        r.out.walkers.iter().filter(|w| w.speed <= 0.0).count() as f64 / n as f64
    };
    let bars = standing(true, 21.0);
    let shops = standing(false, 21.0);
    assert!(bars > shops + 0.15, "standing share: bars {bars:.2}, shops {shops:.2}");
}

/// (`with_sections`) The footways stand on the drawn walks: each street drawn
/// wider than the class model (here every street's carriageway 3 m wider a
/// side, its walks 4 m) moves its walking lines out onto the drawn walk, a
/// line half a throughway in from the frontage zone, and its kerb trims to
/// the drawn kerbs; a walk of its own over a drawn carriageway is cut there.
#[test]
fn the_footways_stand_on_the_drawn_walks() {
    let base = native::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
    let tiles = native::tiles_around(SOMA[0], SOMA[1], 600.0);
    let plain = base.build(&tiles);
    // the drawn street: every built carriageway, 3 m wider a side, walks of 4 m.
    let pieces: Vec<native::DrawnSection> = plain
        .segments
        .iter()
        .flat_map(|s| s.points.windows(2).map(move |w| native::DrawnSection { a: w[0], b: w[1], half: s.half + 3.0, walks: [4.0, 4.0] }))
        .collect();
    let dims = base.dims.clone();
    let path = vec![[SOMA[0] - 60.0, SOMA[1] - 60.0], [SOMA[0] + 60.0, SOMA[1] + 60.0]];
    let drawn = native::load(&eo_data::DataPaths::discover().unwrap()).unwrap().with_sections(pieces.clone()).with_paths(vec![PathWay { id: "diag".into(), highway: "footway".into(), points: path }]).build(&tiles);
    // each street's pair of footways (each the other's far kerb): half their
    // separation at their middles is the drawn kerb (the class half + 3 m)
    // plus the walking line's place on the 4 m walk.
    let line = (4.0 - dims.frontage_zone_m - dims.throughway_m / 2.0).max(2.0);
    let mid = |f: &eo_sim::network::FootwayBuild| {
        let cum = eo_sim::geom::cumulative(&f.points);
        let (x, z, _, _, _) = eo_sim::geom::at_arc(&f.points, &cum, cum[cum.len() - 1] / 2.0);
        [x, z]
    };
    let (mut on, mut all) = (0, 0);
    for (i, f) in drawn.footways.iter().enumerate() {
        let Some(j) = f.cross_index.map(|j| j as usize).filter(|&j| j > i) else { continue };
        let (p, q) = (mid(f), mid(&drawn.footways[j]));
        let c = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
        let Some(seg) = plain.segments.iter().find(|s| eo_sim::geom::dist_to_polyline(&s.points, c[0], c[1]) < 0.3) else { continue };
        all += 1;
        let half_sep = (p[0] - q[0]).hypot(p[1] - q[1]) / 2.0;
        if (half_sep - (seg.half + 3.0 + line)).abs() < 0.3 {
            on += 1;
        }
    }
    assert!(all > 100 && on * 10 > all * 8, "{on} of {all} street pairs on the drawn walks' walking line");
    let on_walk = |p: [f64; 2]| {
        // the distance from the nearest drawn centreline past its kerb, as a share of the 4 m walk
        let mut best = f64::INFINITY;
        let mut half = 0.0;
        for s in &pieces {
            let (dx, dz) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
            let l2 = dx * dx + dz * dz;
            if l2 < 1e-9 {
                continue;
            }
            let t = (((p[0] - s.a[0]) * dx + (p[1] - s.a[1]) * dz) / l2).clamp(0.0, 1.0);
            let d = (s.a[0] + dx * t - p[0]).hypot(s.a[1] + dz * t - p[1]);
            if d < best {
                best = d;
                half = s.half;
            }
        }
        (best - half) / 4.0
    };
    // the kerb trims are the drawn kerbs: the class model's plus the drawn street's 3 m.
    let mut more: Vec<f64> = vec![];
    for (a, b) in plain.footways.iter().zip(drawn.footways.iter()).filter(|(a, _)| !a.segment.starts_with('p')) {
        for e in 0..2 {
            if a.ends[e].kerb_trim > 0.0 {
                more.push(b.ends[e].kerb_trim - a.ends[e].kerb_trim);
            }
        }
    }
    more.sort_by(|a, b| a.total_cmp(b));
    assert!(!more.is_empty() && (more[more.len() / 2] - 3.0).abs() < 0.5, "kerb trims {} more", more.get(more.len() / 2).copied().unwrap_or(f64::NAN));
    // the diagonal walk across the blocks is cut at every carriageway it crosses: no piece on one.
    let walks: Vec<_> = drawn.footways.iter().filter(|f| f.segment.starts_with("pdiag")).collect();
    assert!(walks.len() >= 2, "{} pieces", walks.len());
    for w in walks {
        for p in &w.points {
            assert!(on_walk(*p) > -0.75 / 4.0 - 1e-6, "a walk's point on a carriageway");
        }
    }
}

/// (`with_crossings`) With the crosswalks known, a walker crosses an
/// unsignalled junction's street only where one is mapped: none mapped, no
/// footway end may be crossed but at a signal; every band mapped, all may.
#[test]
fn walkers_cross_where_a_crosswalk_is_mapped() {
    let tiles = native::tiles_around(SOMA[0], SOMA[1], 600.0);
    let load = || native::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
    let plain = load().build(&tiles);
    assert!(plain.footways.iter().all(|f| !f.ends[0].no_crossing && !f.ends[1].no_crossing), "as before without the survey");
    let far = load().with_crossings(vec![[1.0e7, 1.0e7]]).build(&tiles);
    let street_ends = far.footways.iter().filter(|f| !f.segment.starts_with('p')).count() * 2;
    let refused = far.footways.iter().map(|f| f.ends.iter().filter(|e| e.no_crossing).count()).sum::<usize>();
    assert_eq!(refused, street_ends, "nothing mapped near: every band refused");
    // a crosswalk mapped across every street's end at its band (the middle of
    // the band's two kerbs): nearly every end may be crossed.
    let band = |f: &eo_sim::network::FootwayBuild, e: usize| {
        let cum = eo_sim::geom::cumulative(&f.points);
        let len = cum[cum.len() - 1];
        let s = if e == 0 { f.ends[0].crossing_arc } else { len - f.ends[1].crossing_arc };
        let (x, z, _, _, _) = eo_sim::geom::at_arc(&f.points, &cum, s.clamp(0.0, len));
        [x, z]
    };
    let mut mapped = vec![];
    for f in &plain.footways {
        let Some(j) = f.cross_index else { continue };
        for e in 0..2 {
            let (p, q) = (band(f, e), band(&plain.footways[j as usize], e));
            mapped.push([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]);
        }
    }
    let all = load().with_crossings(mapped).build(&tiles);
    let near = all.footways.iter().filter(|f| !f.segment.starts_with('p')).map(|f| f.ends.iter().filter(|e| !e.no_crossing).count()).sum::<usize>();
    assert!(near * 10 > street_ends * 9, "{near} of {street_ends} bands near their crosswalk");
}

/// (`plaza_walks`) A plaza is crossed by straight walks wholly inside it,
/// joined at their ends to its outline cut there: one routable network.
#[test]
fn a_plaza_is_crossed_by_walks_inside_it() {
    // an L-shaped plaza, 120 x 80 m less a 60 x 40 m corner
    let ring = vec![[0.0, 0.0], [120.0, 0.0], [120.0, 40.0], [60.0, 40.0], [60.0, 80.0], [0.0, 80.0], [0.0, 0.0]];
    let walks = native::plaza_walks("t", &ring);
    let chords: Vec<_> = walks.iter().filter(|w| w.id.starts_with("tw")).collect();
    let edges: Vec<_> = walks.iter().filter(|w| w.id.starts_with("te")).collect();
    assert!(chords.len() >= 3, "{} walks across", chords.len());
    assert_eq!(edges.len(), chords.len() * 2, "the outline cut at each walk's two ends");
    let inside = |x: f64, z: f64| !(x > 60.0 + 1e-6 && z > 40.0 + 1e-6) && (-1e-6..=120.0 + 1e-6).contains(&x) && (-1e-6..=80.0 + 1e-6).contains(&z);
    for c in &chords {
        let (a, b) = (c.points[0], c.points[1]);
        for k in 0..=20 {
            let t = k as f64 / 20.0;
            assert!(inside(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t), "{} leaves the plaza", c.id);
        }
    }
    // every walk's ends are an outline piece's ends (shared nodes), and the pieces close the ring.
    let key = |p: [f64; 2]| ((p[0] * 100.0).round() as i64, (p[1] * 100.0).round() as i64);
    let ends: std::collections::HashSet<_> = edges.iter().flat_map(|e| [key(e.points[0]), key(e.points[e.points.len() - 1])]).collect();
    for c in &chords {
        assert!(ends.contains(&key(c.points[0])) && ends.contains(&key(c.points[1])));
    }
    let around: f64 = edges.iter().map(|e| e.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f64>()).sum();
    assert!((around - 400.0).abs() < 1.0, "the outline's pieces are its 400 m: {around}");
    // deterministic, and nothing for a plaza too small to cross.
    assert_eq!(native::plaza_walks("t", &ring).len(), walks.len());
    assert!(native::plaza_walks("s", &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]).is_empty());
}

/// (`--people-groups`, `RegionConfig::groups`) A social group keeps
/// together: after four minutes every member still stands within its
/// formation's reach of its leader (none further than a talking circle's
/// width and a side step), walks, waits and stops with it, and the groups'
/// members are flagged with their places; strangers are left as they were.
/// Without the option nobody is in a party (the JS spawn, as before).
#[test]
fn social_groups_keep_together() {
    use eo_sim::output::{FLAG_PARTY, FLAG_PARTY_SLOT_MASK, FLAG_PARTY_SLOT_SHIFT};
    let run = |groups: bool| {
        let (net, _) = soma(true, false);
        let mut cfg = RegionConfig::game_default(1);
        cfg.hour = 13.0;
        cfg.crowd = Some(CrowdDensity::standard(1.0));
        cfg.groups = groups;
        let mut r = Region::new(&net, 0, cfg, Focus { x: SOMA[0], z: SOMA[1], yaw: 0.0 });
        let mut apart = 0.0f64;
        let mut together = 0usize;
        let mut split_moving = 0usize;
        for s in 1..=(240 * 60) {
            r.step(&net, s as f64 / 60.0, 1.0 / 60.0);
            if s % 60 != 0 {
                continue;
            }
            let at: std::collections::HashMap<u32, usize> = r.crowd.walkers.iter().enumerate().map(|(i, w)| (w.key, i)).collect();
            for (i, w) in r.crowd.walkers.iter().enumerate() {
                let Some(p) = w.party.filter(|p| p.slot > 0) else { continue };
                let Some(&li) = at.get(&p.leader) else { continue };
                let d = (r.crowd.x[i] - r.crowd.x[li]).hypot(r.crowd.z[i] - r.crowd.z[li]);
                apart = apart.max(d);
                together += 1;
                let (a, b) = (w.ground_speed > 0.0, r.crowd.walkers[li].ground_speed > 0.0);
                if a != b {
                    split_moving += 1;
                }
            }
        }
        let flagged = r.out.walkers.iter().filter(|w| w.flags & FLAG_PARTY != 0).count();
        let slots_ok = r.out.walkers.iter().filter(|w| w.flags & FLAG_PARTY != 0).all(|w| ((w.flags & FLAG_PARTY_SLOT_MASK) >> FLAG_PARTY_SLOT_SHIFT) < 3);
        (r.crowd.walkers.iter().filter(|w| w.party.is_some()).count(), apart, together, split_moving, flagged, slots_ok, r.crowd.walkers.len())
    };
    let (off_parties, _, _, _, off_flagged, _, _) = run(false);
    assert_eq!((off_parties, off_flagged), (0, 0), "no parties without the option");
    let (parties, apart, together, split, flagged, slots_ok, n) = run(true);
    println!("{parties} of {n} walkers in groups, {together} member samples, farthest from the leader {apart:.2} m, {split} moving apart");
    assert!(parties * 4 > n, "about half the walkers walk with company: {parties} of {n}");
    assert!(together > 100 && flagged > 0 && slots_ok);
    // the widest place: three abreast end to end, or a trio's circle across
    // (two of its radius), plus a step in transit.
    let reach = (2.0 * consts::GROUP_ABREAST_M).max(2.0 * consts::GROUP_TALK_GAP_M / (2.0 * (std::f64::consts::PI / 3.0).sin())) + 0.2;
    assert!(apart <= reach, "a member {apart:.2} m from its leader");
    assert_eq!(split, 0, "members walk and stop with their leader");
}

/// (`--people-crosswalks`, `NativeSource::with_zebras`) The crossing bands
/// move onto the drawn zebras: a zebra drawn 2 m further out than each
/// street end's class band moves the band there; a street end with no zebra
/// drawn near it is not crossed, at a signal too. Without zebras, as before.
#[test]
fn walkers_cross_on_the_drawn_zebras() {
    let tiles = native::tiles_around(SOMA[0], SOMA[1], 600.0);
    let load = || native::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
    let plain = load().build(&tiles);
    assert!(plain.footways.iter().all(|f| !f.ends[0].zebra_only && !f.ends[1].zebra_only), "as before without zebras");
    // the zebra of each street end: on the street's centreline (the middle of
    // the two footways' bands), 2 m further from the node.
    let point = |f: &eo_sim::network::FootwayBuild, e: usize, extra: f64| {
        let cum = eo_sim::geom::cumulative(&f.points);
        let len = cum[cum.len() - 1];
        let s = if e == 0 { f.ends[0].crossing_arc + extra } else { len - f.ends[1].crossing_arc - extra };
        let (x, z, _, _, _) = eo_sim::geom::at_arc(&f.points, &cum, s.clamp(0.0, len));
        [x, z]
    };
    let mut zebras = vec![];
    for f in &plain.footways {
        let Some(j) = f.cross_index else { continue };
        for e in 0..2 {
            let (p, q) = (point(f, e, 2.0), point(&plain.footways[j as usize], e, 2.0));
            zebras.push([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]);
        }
    }
    let on = load().with_zebras(zebras).build(&tiles);
    let mut moved = 0;
    let mut ends = 0;
    for (f, g) in plain.footways.iter().zip(&on.footways) {
        if f.segment.starts_with('p') || f.cross_index.is_none() {
            continue;
        }
        for e in 0..2 {
            ends += 1;
            assert!(g.ends[e].zebra_only);
            if !g.ends[e].no_crossing && (g.ends[e].crossing_arc - f.ends[e].crossing_arc - 2.0).abs() < 0.5 {
                moved += 1;
            }
        }
    }
    assert!(moved * 10 > ends * 8, "{moved} of {ends} bands on their zebra");
    let none = load().with_zebras(vec![[1.0e7, 1.0e7]]).build(&tiles);
    assert!(none.footways.iter().filter(|f| !f.segment.starts_with('p') && f.cross_index.is_some()).all(|f| f.ends.iter().all(|e| e.no_crossing && e.zebra_only)), "no zebra drawn: no crossing");
}

/// (`--people-crosswalks`) With the drawn zebras known a corner leg keeps to
/// its corner: none of the corner legs walked in four minutes round SoMa
/// crosses the centreline of a street with walks (without them the JS joins
/// every footway end at a junction's node, and most do: a walker steps off
/// one corner across the road to another, with no crosswalk or signal). A
/// walk going on across the mouth of a lane with no walks, or round a dead
/// end's last point, is not a crossing.
#[test]
fn corner_legs_keep_to_their_corner() {
    use eo_sim::pedestrians::LegKind;
    let tiles = native::tiles_around(SOMA[0], SOMA[1], 600.0);
    let load = || native::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
    // a zebra at every street end's class band (so every end keeps its crossings).
    let plain = load().build(&tiles);
    let point = |f: &eo_sim::network::FootwayBuild, e: usize| {
        let cum = eo_sim::geom::cumulative(&f.points);
        let len = cum[cum.len() - 1];
        let s = if e == 0 { f.ends[0].crossing_arc } else { len - f.ends[1].crossing_arc };
        let (x, z, _, _, _) = eo_sim::geom::at_arc(&f.points, &cum, s.clamp(0.0, len));
        [x, z]
    };
    let mut zebras = vec![];
    for f in &plain.footways {
        let Some(j) = f.cross_index else { continue };
        for e in 0..2 {
            let (p, q) = (point(f, e), point(&plain.footways[j as usize], e));
            zebras.push([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]);
        }
    }
    // leg a-b crosses centreline piece c-d, more than 0.75 m from c and d (round
    // a dead end's last point, where the street stops, a walk turns about).
    let crosses = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| {
        let o = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
        if !(o(a, b, c) * o(a, b, d) < 0.0 && o(c, d, a) * o(c, d, b) < 0.0) {
            return false;
        }
        let t = o(a, b, c) / (o(a, b, c) - o(a, b, d));
        let len = (d[0] - c[0]).hypot(d[1] - c[1]);
        t * len > 0.75 && (1.0 - t) * len > 0.75
    };
    let run = |src: NativeSource| {
        let mut net = Network::new(src.timing.clone());
        net.add(src.build(&tiles));
        let mut cfg = RegionConfig::game_default(1);
        cfg.hour = 13.0;
        cfg.crowd = Some(CrowdDensity::standard(1.0));
        let mut r = Region::new(&net, 0, cfg, Focus { x: SOMA[0], z: SOMA[1], yaw: 0.0 });
        // the centrelines of the streets that have walks (the middle of each
        // footway and the one across its street): a walk continuing across
        // the mouth of a lane with no walks of its own is the walk going on.
        let mut centre: Vec<[[f64; 2]; 2]> = vec![];
        for (i, f) in net.footways.iter().enumerate() {
            let Some(c) = net.footways.get(f.cross as usize).filter(|c| f.alive && c.alive && (f.cross as usize) > i && c.pts.len() == f.pts.len()) else { continue };
            let mid: Vec<[f64; 2]> = f.pts.iter().zip(&c.pts).map(|(a, b)| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]).collect();
            centre.extend(mid.windows(2).map(|w| [w[0], w[1]]));
        }
        let mut legs = std::collections::HashSet::new();
        for s in 1..=(240 * 60) {
            r.step(&net, s as f64 / 60.0, 1.0 / 60.0);
            for w in &r.crowd.walkers {
                if let Some(l) = w.leg.filter(|l| l.kind == LegKind::Corner && l.n == 2) {
                    legs.insert(((l.pts[0][0] * 10.0) as i64, (l.pts[0][1] * 10.0) as i64, (l.pts[1][0] * 10.0) as i64, (l.pts[1][1] * 10.0) as i64));
                }
            }
        }
        let over = legs.iter().filter(|&&(a, b, c, d)| {
            let (p, q) = ([a as f64 / 10.0, b as f64 / 10.0], [c as f64 / 10.0, d as f64 / 10.0]);
            centre.iter().any(|s| crosses(p, q, s[0], s[1]))
        }).count();
        (legs.len(), over)
    };
    let (before, over_before) = run(load());
    let (after, over_after) = run(load().with_zebras(zebras));
    println!("corner legs: {before} walked, {over_before} across a street; with the zebras {after}, {over_after}");
    assert!(after > 20, "corner legs are still walked: {after}");
    assert_eq!(over_after, 0, "a corner leg crossed a street");
    assert!(over_before > 0, "the JS routing does cross some (the test sees them)");
}
