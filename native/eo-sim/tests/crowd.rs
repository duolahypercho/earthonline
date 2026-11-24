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
