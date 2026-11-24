//! Each place's waves in the physics' sea (eo-water `sea::Places`) are the
//! drawn sea's (water.wgsl `w_expo_set`, water_common.wgsl `wg_cascade_amp`):
//! the exposure field's bay wind sea at its wind ratio and fetch factor and
//! its swell share, the measured bed's share over it, the ramp across the
//! Gate without the field; and with the real water loaded the physics reads
//! the fields the water draws with. (Its own test binary: `eo_water::load`
//! installs the fields for the whole process.)

use eo_water::exposure::Exposure;
use eo_water::extra::ExtraField;
use eo_water::field::to_half_three;
use eo_water::sea::Places;
use eo_water::waves::{self, CASCADES, Plan};

fn plan() -> Plan {
    Plan::new(&waves::sea_state(16.0, [1.8, 0.0], 0.0))
}

/// A 4 x 4 exposure of 100 m cells from (-200, -200), every cell `rgb`.
fn exposure(rgb: [u8; 3]) -> &'static Exposure {
    Box::leak(Box::new(Exposure { origin: [-200.0, -200.0], cell_m: 100.0, width: 4, height: 4, texels: vec![[rgb[0], rgb[1], rgb[2], 255]; 16], bake_ms: 0.0, from_cache: false }))
}

fn close(a: [f64; CASCADES], b: [f64; CASCADES]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-12 * y.abs().max(1.0))
}

#[test]
fn without_the_field_the_ramp_across_the_gate() {
    let p = plan();
    let places = Places::with(&p, None, None);
    for x in [-30_000.0, -9_000.0, -6_000.0, -2_000.0, 4_000.0] {
        let o = eo_water::surf_strip::ocean_share(x);
        let want: [f64; CASCADES] = std::array::from_fn(|c| p.amp_bay[c] + (p.amp_pac[c] - p.amp_bay[c]) * o);
        assert!(close(places.amp(x, -4000.0), want), "x {x}");
    }
}

#[test]
fn the_exposure_sets_the_bay_wind_sea_and_the_swell_share() {
    let p = plan();
    // all the swell: the Pacific's amplitudes, whatever the wind and fetch
    assert!(close(Places::with(&p, Some(exposure([255, 40, 90])), None).amp(0.0, 0.0), p.amp_pac));
    // no swell, the lowest wind and fetch nodes (both below their first node: held there)
    assert!(close(Places::with(&p, Some(exposure([0, 0, 0])), None).amp(0.0, 0.0), p.expo_amp[0][0]));
    // no swell, the top nodes (G 255 is a wind ratio of 2, B 255 a fetch factor of 1)
    assert!(close(Places::with(&p, Some(exposure([0, 255, 255])), None).amp(0.0, 0.0), p.expo_amp[7][7]));
    // between nodes: bilinear in the table, then blended by the swell share
    let e = exposure([128, 120, 140]);
    let (wind, fetch, share) = (120.0 / 255.0 * 2.0, 140.0 / 255.0, 128.0 / 255.0);
    let (wn, fnodes) = (waves::EXPO_WIND_NODES, waves::EXPO_FETCH_NODES);
    let g = (0..7).find(|&k| wind <= wn[k + 1]).unwrap();
    let b = (0..7).find(|&k| fetch <= fnodes[k + 1]).unwrap();
    let (gf, bf) = ((wind - wn[g]) / (wn[g + 1] - wn[g]), (fetch - fnodes[b]) / (fnodes[b + 1] - fnodes[b]));
    let want: [f64; CASCADES] = std::array::from_fn(|c| {
        let xa = |i: usize, j: usize| p.expo_amp[i][j][c];
        let bay = (xa(g, b) * (1.0 - bf) + xa(g, b + 1) * bf) * (1.0 - gf) + (xa(g + 1, b) * (1.0 - bf) + xa(g + 1, b + 1) * bf) * gf;
        bay + (p.amp_pac[c] - bay) * share
    });
    let got = Places::with(&p, Some(e), None).amp(0.0, 0.0);
    println!("wind {wind:.3} (nodes {g}, {gf:.3}), fetch {fetch:.3} (nodes {b}, {bf:.3}), swell {share:.3}: {got:?}");
    assert!(close(got, want), "{got:?} against {want:?}");
}

#[test]
fn the_measured_bed_takes_over_the_swell_share_where_it_lies() {
    let p = plan();
    // a 2 km square of 50 m cells from (-1000, -1000), two layers, the bed's swell share 0.3 in layer 0's G
    let (w, rows) = (40usize, 40usize);
    let mut texels = vec![[0u16; 4]; w * rows * 2];
    for t in texels.iter_mut().take(w * rows) {
        *t = [to_half_three(12.0), to_half_three(0.3), 0, 0];
    }
    let bed: &'static ExtraField = Box::leak(Box::new(ExtraField { origin: [-1000.0, -1000.0], cell_m: 50.0, width: w, height: rows * 2, layers: 2, texels }));
    let e = exposure([200, 128, 200]);
    let (with, without) = (Places::with(&p, Some(e), Some(bed)), Places::with(&p, Some(e), None));
    let share = |a: [f64; CASCADES], c: usize, bay: f64| (a[c] - bay) / (p.amp_pac[c] - bay);
    // the bay's part is the same with and without the bed: recover it from a share of 0
    let bay = Places::with(&p, Some(exposure([0, 128, 200])), None).amp(0.0, 0.0);
    let c = 0;
    // in the middle (1 km from every edge): the bed's 0.3
    let mid = share(with.amp(0.0, 0.0), c, bay[c]);
    assert!((mid - 0.3).abs() < 1e-3, "the bed's share in its middle: {mid}");
    // 100 m inside its edge: half way along its 200 m smoothstep
    let edge = share(with.amp(-900.0, 0.0), c, bay[c]);
    let ex = 200.0 / 255.0;
    assert!((edge - (ex + (0.3 - ex) * 0.5)).abs() < 1e-3, "100 m in: {edge}");
    // outside: the exposure's own
    assert!(close(with.amp(-1500.0, 0.0), without.amp(-1500.0, 0.0)));
}

#[test]
fn with_the_water_loaded_the_physics_reads_its_fields() {
    let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !root.join("public/data/sf/sf-city.json.gz").exists() {
        assert!(root.pop(), "repo root not found");
    }
    let _data = eo_water::load(&root, None).expect("water data");
    let e = eo_water::exposure::installed().expect("the exposure installed by load");
    let p = plan();
    let (drawn, plain) = (Places::drawn(&p), Places::with(&p, None, None));
    let s = eo_water::ground_scale();
    let game = |lon: f64, lat: f64| ((lon + 122.4194) * 87_986.247_476_406_54 * s, (37.778 - lat) * 110_574.0 * s);
    let mut differ = 0;
    for (name, lon, lat) in [("SF Marina mouth", -122.4385, 37.8082), ("off Crissy Field", -122.455, 37.8085), ("off Aquatic Park", -122.4225, 37.8105), ("Pacific off Ocean Beach", -122.56, 37.76)] {
        let (x, z) = game(lon, lat);
        let (a, b) = (drawn.amp(x, z), plain.amp(x, z));
        let v = e.at(x, z);
        println!("{name:24} swell {:.2} wind {:.2} fetch {:.2}: cascade amplitudes drawn {:.3?}, the plan's ramp {:.3?}", v[0], v[1], v[2], &a[..3], &b[..3]);
        assert!(close(a, Places::with(&p, Some(e), eo_water::extra::installed()).amp(x, z)), "{name}: drawn() reads the installed fields");
        differ += usize::from(!close(a, b));
    }
    assert!(differ >= 3, "the exposure changes the bay's waves");
}
