//! the plan/longitude-latitude mapping the roof survey uses, checked
//! on the real city, and the shipped roof colour table (when present).

use eo_data::roof_colours::{PlanFrame, RoofTable, native_assets};
use eo_data::{City, DataPaths};

fn inside(ring: &[[f32; 2]], x: f64, z: f64) -> bool {
    let mut c = false;
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = ([ring[i][0] as f64, ring[i][1] as f64], [ring[j][0] as f64, ring[j][1] as f64]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// Two towers whose published coordinates must fall inside a tall surveyed footprint.
#[test]
fn surveyed_towers_unproject_to_their_coordinates() {
    let paths = DataPaths::discover().expect("repo root");
    let city = City::load_at(&paths, 1.0).expect("city at 1:1");
    let f = PlanFrame::from_meta(&city.meta);
    for (name, lat, lon, min_h) in [("Transamerica Pyramid", 37.79520, -122.40280, 150.0f32), ("Salesforce Tower", 37.78977, -122.39689, 200.0)] {
        let (x, z) = f.to_plan(lon, lat);
        let (lon2, lat2) = f.to_lonlat(x, z);
        assert!((lon2 - lon).abs() < 1e-9 && (lat2 - lat).abs() < 1e-9);
        let hit = city.buildings.iter().filter(|b| inside(&b.ring, x, z)).map(|b| b.height).fold(0.0f32, f32::max);
        assert!(hit >= min_h, "{name}: the footprint under ({x:.1}, {z:.1}) is {hit} m tall");
    }
}

/// A drawn (scaled) footprint unprojects to the surveyed one: the table is keyed
/// by record, and the drawn plan divides out by the ground-plan scale.
#[test]
fn drawn_plan_divides_out_by_the_scale() {
    let paths = DataPaths::discover().expect("repo root");
    let s = 1.25;
    let a = City::load_at(&paths, 1.0).expect("1:1");
    let b = City::load_at(&paths, s).expect("scaled");
    let f = PlanFrame::from_meta(&a.meta);
    let by: std::collections::HashMap<&str, &eo_data::Building> = b.buildings.iter().map(|r| (r.id.as_str(), r)).collect();
    let (mut checked, mut matched) = (0, 0);
    for r in a.buildings.iter().step_by(97) {
        let Some(d) = by.get(r.id.as_str()) else { continue };
        if d.ring.len() != r.ring.len() {
            continue;
        }
        checked += 1;
        // Most records are scaled about the origin; hand-built ones and their
        // neighbours' pulled vertices differ, so count the plain ones.
        let ok = r.ring.iter().zip(&d.ring).all(|(p, q)| {
            let (l1, t1) = f.to_lonlat(p[0] as f64, p[1] as f64);
            let (l2, t2) = f.drawn_to_lonlat(q[0] as f64, q[1] as f64, s);
            (l1 - l2).abs() * f.m_per_deg_lon < 0.05 && (t1 - t2).abs() * f.m_per_deg_lat < 0.05
        });
        matched += ok as usize;
    }
    assert!(checked > 1500, "{checked}");
    assert!(matched as f64 >= 0.97 * checked as f64, "{matched} of {checked} drawn footprints unproject onto the surveyed ones");
}

/// The shipped table (native/assets/roof-colours-v1.bin.gz) decodes, covers
/// most of the city, and holds plausible colours. Skipped when absent.
#[test]
fn shipped_table_covers_the_city() {
    let p = native_assets().join("roof-colours-v1.bin.gz");
    if !p.is_file() {
        eprintln!("no {} (skipped)", p.display());
        return;
    }
    let t = RoofTable::read(&p).expect("table");
    let paths = DataPaths::discover().expect("repo root");
    let city = City::load_at(&paths, 1.0).expect("city");
    let found = city.buildings.iter().filter(|b| t.get(&b.id).is_some()).count();
    assert!(found as f64 > 0.9 * city.buildings.len() as f64, "{found} of {}", city.buildings.len());
    let confident = city.buildings.iter().filter_map(|b| t.get(&b.id)).filter(|m| m.confidence >= 0.5).count();
    assert!(confident as f64 > 0.5 * found as f64, "{confident} confident of {found}");
    assert!(t.get("no-such-record").is_none());
}
