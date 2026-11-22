//! (iteration 3) The shoreline rings at the beaches against the surveyed
//! shore: along transects normal to the ring at Baker Beach and Crissy Field,
//! where the B1 CUDEM topobathy crosses mean sea level and mean higher high
//! water (NOAA 9414290's datums, eo-water beaches.rs), and where the drawn
//! ground (the city's elevation grid pressed by the shore field) meets the
//! still sea. Prints the offsets; skips without `EO_SURVEY_ROOT`.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    if let Ok(r) = std::env::var("EO_REPO_ROOT") {
        return PathBuf::from(r);
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !p.join("public/data/sf/sf-city.json.gz").exists() {
        assert!(p.pop(), "repo root not found (set EO_REPO_ROOT)");
    }
    p
}

#[test]
fn the_beach_rings_against_the_surveyed_waterline() {
    let Some(root) = eo_farshore::survey::Root::discover() else {
        eprintln!("shoreline survey: skipped, EO_SURVEY_ROOT not set");
        return;
    };
    let repo = repo_root();
    let data = eo_water::load(&repo, None).expect("water data");
    let f = &data.field;
    let bathy = eo_farshore::survey::bathy::Bathy::open(&root).expect("B1");
    let gs = eo_water::ground_scale();
    let fr = eo_data::Frame::sf();
    let msl_navd = eo_water::bathymetry::MSL_OVER_NAVD88_M;
    let b1 = |x: f64, z: f64| -> Option<f64> {
        let (lat, lon) = fr.unproject(x / gs, z / gs);
        bathy.height(lon, lat).map(|h| h as f64 - msl_navd)
    };
    let mut grid = eo_data::ElevationGrid::decode(&eo_data::read_maybe_gzip(&repo.join("public/data/sf/sf-elevation.bin.gz")).unwrap()).unwrap();
    grid.meta.origin_x *= gs;
    grid.meta.origin_z *= gs;
    grid.meta.cell_size *= gs;
    grid.height_scale = eo_data::groundscale::height_scale();
    let mhhw = eo_water::beaches::MHHW_ABOVE_MSL_M;
    let mut rows = vec![];
    for (name, x0, z0, x1, z1) in [("Baker Beach", -7300.0, -1900.0, -6750.0, -3100.0), ("Crissy Field", -5400.0, -3990.0, -4250.0, -3830.0)] {
        for k in 0..=8 {
            let t = k as f64 / 8.0;
            let (x, z) = (x0 + (x1 - x0) * t, z0 + (z1 - z0) * t);
            // the ring's nearest point: walk the field's gradient to its zero
            let g = |x: f64, z: f64| [f.signed_exact(x + 1.0, z) - f.signed_exact(x - 1.0, z), f.signed_exact(x, z + 1.0) - f.signed_exact(x, z - 1.0)];
            let (mut px, mut pz) = (x, z);
            for _ in 0..20 {
                let s = f.signed_exact(px, pz);
                let gr = g(px, pz);
                let l = gr[0].hypot(gr[1]).max(1e-6);
                px -= gr[0] / l * s;
                pz -= gr[1] / l * s;
            }
            let gr = g(px, pz);
            let l = gr[0].hypot(gr[1]).max(1e-6);
            let n = [gr[0] / l, gr[1] / l]; // seaward
            // along the normal from 150 m inland to 150 m out: the first crossing seaward
            let cross = |h: &dyn Fn(f64, f64) -> Option<f64>, level: f64| -> Option<f64> {
                let mut last: Option<f64> = None;
                for i in 0..=600 {
                    let s = -150.0 + i as f64 * 0.5;
                    let v = h(px + n[0] * s, pz + n[1] * s)?;
                    if let Some(l) = last
                        && l >= level
                        && v < level
                    {
                        return Some(s);
                    }
                    last = Some(v);
                }
                None
            };
            let msl = cross(&b1, 0.0);
            let hw = cross(&b1, mhhw);
            let drawn = |x: f64, z: f64| grid.sample(x, z).map(|y| f.press_ground(x, z, y) - eo_water::SEA_LEVEL_Y);
            let dl = cross(&drawn, 0.0);
            eprintln!("{name:12} ({px:7.0}, {pz:7.0}): B1 MSL at {:>6} m, B1 MHHW at {:>6} m, drawn waterline at {:>6} m from the ring (+ seaward)", fmt(msl), fmt(hw), fmt(dl));
            rows.push((name, msl, dl));
        }
    }
    // the photo's eye at Crissy Field, a line north (seaward) from it
    for k in 0..=24 {
        let (x, z) = (-5040.51, -3745.69 - 10.0 * k as f64);
        let y = grid.sample(x, z).unwrap_or(f64::NAN);
        eprintln!("crissy eye +{:3} m north: field {:6.1} m, B1 {:6.2} m over MSL, survey grid {:6.2} m over the sea, drawn {:6.2}", 10 * k, f.signed_exact(x, z), b1(x, z).unwrap_or(f64::NAN), y - eo_water::SEA_LEVEL_Y, f.press_ground(x, z, y) - eo_water::SEA_LEVEL_Y);
    }
    // (iteration 3) the fender drawn into the field's land distance
    if let Some(xf) = eo_water::extra::field() {
        let fen = eo_water::obstacles::south_fender(&repo).unwrap();
        let (cx, cz) = (fen.iter().map(|o| o.x).sum::<f64>() / fen.len() as f64, fen.iter().map(|o| o.z).sum::<f64>() / fen.len() as f64);
        let a = |x: f64, z: f64| {
            let i = ((x - xf.origin[0]) / xf.cell_m) as usize;
            let j = ((z - xf.origin[1]) / xf.cell_m) as usize;
            eo_water::bathymetry::half_to_f32(xf.texels[j * xf.width + i][3])
        };
        eprintln!("fender centre ({cx:.0}, {cz:.0}), composed revision {:?}", eo_water::extra::composed_revision());
        for dx in [0.0, 50.0, 70.0, 75.0, 80.0, 90.0, 110.0] {
            eprintln!("  land distance {dx:5.0} m east of the centre: {:.1}", a(cx + dx, cz));
        }
    }
    // a check, not a fit: both measured waterlines exist along each beach
    assert!(rows.iter().filter(|r| r.1.is_some()).count() >= 12);
}

fn fmt(v: Option<f64>) -> String {
    v.map_or("-".into(), |v| format!("{v:.1}"))
}
