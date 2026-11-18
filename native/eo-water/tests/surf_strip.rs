//! the surf strip on the real shore field and beaches (sf-city.json.gz,
//! sf-lots-parks.json.gz): where it lies along the shore, and that every
//! point the sea hands over to it is covered by it.

use eo_water::surf_strip::{self, N_LAND_M, Strip, surf_cover};
use std::path::PathBuf;
use std::sync::OnceLock;

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

struct Fixture {
    field: eo_water::field::ShoreField,
    strips: Vec<Strip>,
}

fn fixture() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| {
        let root = repo_root();
        let data = eo_water::load(&root, None).expect("water data");
        let beaches = eo_water::load_beaches(&root).expect("beaches");
        let strips = surf_strip::build_strips(&data.field, &beaches);
        for s in &strips {
            let p = s.points[s.points.len() / 2];
            eprintln!("strip {:.0} m at ({:.0}, {:.0}) {}", s.length(), p[0], p[1], if s.climate.pacific { "Pacific" } else { "bay" });
        }
        Fixture { field: data.field, strips }
    })
}

#[test]
fn ocean_beach_has_a_long_pacific_strip() {
    let f = fixture();
    let pac: Vec<&Strip> = f.strips.iter().filter(|s| s.climate.pacific).collect();
    assert!(!pac.is_empty(), "no Pacific strip");
    let longest = pac.iter().map(|s| s.length()).fold(0.0, f64::max);
    assert!(longest > 3000.0, "Ocean Beach strip only {longest:.0} m");
}

#[test]
fn the_strip_lies_on_the_waterline_with_offshore_normals() {
    let f = fixture();
    for s in &f.strips {
        let mut worst: f64 = 0.0;
        for (i, (p, n)) in s.points.iter().zip(&s.normals).enumerate() {
            let d = f.field.signed_at(p[0], p[1]);
            // the smoothed polyline stays within a few metres of the zero line along the beach
            if s.beach_w[i] > 0.9 {
                worst = worst.max(d.abs());
            }
            let ahead = f.field.signed_at(p[0] + n[0] * 40.0, p[1] + n[1] * 40.0);
            let behind = f.field.signed_at(p[0] - n[0] * 40.0, p[1] - n[1] * 40.0);
            if s.beach_w[i] > 0.9 {
                assert!(ahead > behind, "normal {i} points inland");
            }
            let l = (n[0] * n[0] + n[1] * n[1]).sqrt();
            assert!((l - 1.0).abs() < 1e-6);
        }
        assert!(worst < 8.0, "strip off the waterline by {worst:.1} m");
    }
}

#[test]
fn the_offsets_never_fold() {
    let f = fixture();
    for s in &f.strips {
        for n in [N_LAND_M, 0.0, s.climate.n_max * 0.5, s.climate.n_max] {
            for i in 0..s.points.len() - 1 {
                let a = s.at(i as f64 * surf_strip::SAMPLE_M, n);
                let b = s.at((i + 1) as f64 * surf_strip::SAMPLE_M, n);
                let t = [s.points[i + 1][0] - s.points[i][0], s.points[i + 1][1] - s.points[i][1]];
                let fwd = (b[0] - a[0]) * t[0] + (b[1] - a[1]) * t[1];
                assert!(fwd > 0.0, "fold at sample {i} of {}, n {n}, strip at {:?}", s.points.len(), s.points[s.points.len() / 2]);
            }
        }
    }
}

/// Every field texel the sea hands over (cover > 0) lies inside a strip's
/// reach: its (s, n) on that strip is inside the strip and maps back to it.
#[test]
fn the_strip_covers_everything_the_sea_hands_over() {
    let f = fixture();
    let fld = &f.field;
    let step = [fld.size[0] / (fld.width - 1) as f64, fld.size[1] / (fld.height - 1) as f64];
    let boxes: Vec<[f64; 4]> = f.strips.iter().map(|s| s.bounds()).collect();
    let (mut handed, mut missed) = (0usize, 0usize);
    let mut worst = (0.0, [0.0, 0.0]);
    for r in 0..fld.height {
        for c in 0..fld.width {
            let i = r * fld.width + c;
            let (signed, beach) = (fld.signed(i) as f64, fld.beach(i) as f64);
            if beach < 0.8 || signed > 400.0 || signed < surf_strip::LAND_FADE_M[0] - 1.0 {
                continue;
            }
            let (x, z) = (fld.origin[0] + step[0] * c as f64, fld.origin[1] + step[1] * r as f64);
            let cov = surf_cover(x, signed, beach);
            if cov <= 0.0 {
                continue;
            }
            handed += 1;
            let mut ok = false;
            for (s, b) in f.strips.iter().zip(&boxes) {
                if x < b[0] || x > b[2] || z < b[1] || z > b[3] {
                    continue;
                }
                let (ss, nn) = s.project(x, z);
                if ss < 0.0 || ss > s.length() || nn < N_LAND_M || nn > s.climate.n_max {
                    continue;
                }
                let q = s.at(ss, nn);
                if ((q[0] - x).powi(2) + (q[1] - z).powi(2)).sqrt() < 12.0 {
                    ok = true;
                    break;
                }
            }
            if !ok {
                missed += 1;
                if missed % 15 == 1 {
                    let near = f.strips.iter().enumerate().map(|(k, s)| { let (ss, nn) = s.project(x, z); let q = s.at(ss.clamp(0.0, s.length()), nn); (k, ss, nn, s.length(), ((q[0]-x).powi(2)+(q[1]-z).powi(2)).sqrt()) }).min_by(|a, b| a.4.partial_cmp(&b.4).unwrap()).unwrap();
                    eprintln!("missed ({x:.0}, {z:.0}) signed {signed:.0} beach {beach:.2} cover {cov:.2}: strip {} s {:.0} n {:.0} len {:.0} err {:.0}", near.0, near.1, near.2, near.3, near.4);
                }
                if cov > worst.0 {
                    worst = (cov, [x, z]);
                }
            }
        }
    }
    eprintln!("handed over: {handed} texels, outside every strip: {missed} (worst cover {:.2} at {:?})", worst.0, worst.1);
    assert!(handed > 1000, "the sea hands over almost nothing: {handed}");
    assert_eq!(missed, 0, "texels the sea hands over that no strip covers (worst cover {:.2} at {:?})", worst.0, worst.1);
}
