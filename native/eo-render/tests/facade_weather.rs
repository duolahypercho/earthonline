//! (`--facade-weather`) The marks the kit walls carry for the run-off under
//! each window (eo-render kit.rs `weather_marks`): only upright wall faces in
//! a window's column, whole front faces (four vertices, the piece's two ends),
//! the depth under the sill above them that the opening records give, and the
//! cache key as before without the switch.
//!
//!   cargo test --release -p eo-render --test facade_weather -- --nocapture

use eo_kit::city::KitCity;
use eo_kit::pipeline::{build_rung_with, plan_building};
use eo_render::kit::{WEATHER_DEPTH_STEP_M, tile_cache_key, weather_marks};
use std::sync::OnceLock;

fn city() -> &'static KitCity {
    static C: OnceLock<KitCity> = OnceLock::new();
    C.get_or_init(|| {
        let root = eo_kit::repo_root().expect("repo root");
        KitCity::load(&root).expect("city").1
    })
}

#[test]
fn the_key_is_unchanged_without_the_switch() {
    // the test harness's command line carries neither `--facade-weather` nor `--facade-clutter`
    assert!(!eo_render::facade::weather_from_args());
    assert_eq!(tile_cache_key("0123456789abcdef"), "0123456789abcdef");
}

#[test]
fn marks_lie_on_window_columns_only() {
    let kc = city();
    let (mut buildings, mut marked_buildings, mut quads) = (0, 0, 0);
    for i in (0..kc.records.len()).step_by(97).take(400) {
        if kc.excluded.contains(&i) {
            continue;
        }
        let Some(planned) = plan_building(&kc.records[i], &kc.world, i as u32, 3.25) else { continue };
        buildings += 1;
        let mut any = false;
        for rung in [0u32, 2] {
            for clutter in [0u8, 2] {
                let mesh = build_rung_with(&planned, rung, clutter);
                let marks = weather_marks(&planned, rung, &mesh);
                assert_eq!(marks.len(), mesh.groups.len());
                for (g, (name, _, sink)) in mesh.groups.iter().enumerate() {
                    assert_eq!(marks[g].len(), sink.positions.len(), "{name}: one mark a vertex");
                    assert!(!name.starts_with("glass") || marks[g].iter().all(|m| *m == 0), "{name}: no glass is marked");
                    let mut k = 0;
                    while k < marks[g].len() {
                        if marks[g][k] == 0 {
                            k += 1;
                            continue;
                        }
                        // a whole front face: four marked vertices, upright, the piece's two ends
                        assert!(k + 4 <= marks[g].len());
                        let q: Vec<u16> = marks[g][k..k + 4].to_vec();
                        assert!(q.iter().all(|m| *m != 0), "{name}: a face's four vertices marked together");
                        let (mut wmin, mut wmax) = (u16::MAX, 0u16);
                        let (mut ymin, mut ymax) = (f64::MAX, f64::MIN);
                        let (mut dmin, mut dmax) = (u16::MAX, 0u16);
                        for (j, m) in q.iter().enumerate() {
                            let n = sink.normals[k + j];
                            assert!(n[1].abs() < 1e-6, "{name}: marked faces are upright");
                            let w = m >> 8;
                            assert!((1..=62).contains(&w), "{name}: the place byte stays under the glow codes ({w})");
                            let d = m & 255;
                            assert!(d <= 127, "{name}: the depth fits the y byte");
                            let y = sink.positions[k + j][1];
                            (wmin, wmax) = (wmin.min(w), wmax.max(w));
                            (ymin, ymax) = (ymin.min(y), ymax.max(y));
                            (dmin, dmax) = (dmin.min(d), dmax.max(d));
                        }
                        // u runs across the piece (a vertex the plan clamp moved in stops short of its end)
                        assert!(wmax - wmin >= 30, "{name}: the place runs across the piece ({wmin}..{wmax})");
                        // the depth under the sill grows down the piece by its height
                        let span = (ymax - ymin) / WEATHER_DEPTH_STEP_M;
                        assert!(((dmax - dmin) as f64 - span).abs() <= 1.01, "{name}: the depth grows down the piece ({dmin}..{dmax} over {span:.2} steps)");
                        quads += 1;
                        any = true;
                        k += 4;
                    }
                }
            }
        }
        marked_buildings += any as usize;
    }
    eprintln!("facade weather: {marked_buildings} of {buildings} sampled buildings marked, {quads} faces");
    assert!(buildings > 100, "the sample plans buildings");
    assert!(marked_buildings * 3 > buildings, "most windowed buildings carry marks ({marked_buildings} of {buildings})");
}
