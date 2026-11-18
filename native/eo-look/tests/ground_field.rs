//! the sky below the horizon equals the JS's street field
//! (`sampleSkyRadiance` for 99 directions below the horizon at 9 hours,
//! exported by the earlier browser version's exporter).

use eo_look::sky::SkyModel;

#[test]
fn lower_hemisphere_equals_js_street_field() {
    let o = eo_look::ground_field::oracle();
    let dirs: Vec<[f64; 3]> = o["dirs"].as_array().unwrap().iter().map(|d| [0, 1, 2].map(|k| d[k].as_f64().unwrap())).collect();
    let (mut n, mut worst) = (0, (0.0f64, 0.0, 0usize));
    for s in o["samples"].as_array().unwrap() {
        let h = s["hour"].as_f64().unwrap();
        let m = SkyModel::new(h);
        let mut hw = (0.0f64, 0usize, [0.0; 3], [0.0; 3]);
        for (k, want) in s["radiance"].as_array().unwrap().iter().enumerate() {
            let got = m.sample(dirs[k]);
            for c in 0..3 {
                let w = want[c].as_f64().unwrap();
                let e = (got[c] - w).abs() / w.abs().max(1e-4);
                if e > worst.0 {
                    worst = (e, h, k);
                }
                if e > hw.0 {
                    hw = (e, k, got, [0, 1, 2].map(|i| want[i].as_f64().unwrap()));
                }
            }
            n += 1;
        }
        println!("{h} h: worst {:.2e} at direction {} {:?}: got {:?} want {:?}", hw.0, hw.1, dirs[hw.1], hw.2, hw.3);
    }
    println!("{n} samples below the horizon: worst relative error {:.2e} at {} h direction {}", worst.0, worst.1, worst.2);
    assert!(worst.0 < 1e-6, "street field off the JS by {:.2e} at {} h direction {}", worst.0, worst.1, worst.2);
}
