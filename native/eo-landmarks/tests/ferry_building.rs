//! The Ferry Building (ferry_building.rs) against the city record it is drawn
//! in place of, and its built geometry's measured heights; the corridor's
//! streetcar rails on the repo's rail ways; the kill switch's claim.

use eo_landmarks::ferry_building as fb;

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The spec's corners are the record's (sf-city, surveyed), and on this
/// process's ground plan the record the player collides with is that
/// footprint scaled: the walls the generator draws stand on it.
#[test]
fn the_spec_is_the_city_record_on_the_drawn_plan() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let s = eo_data::groundscale::scale();
    let city = eo_data::City::load_at(&paths, s).unwrap();
    let sp = fb::spec();
    let rec = city.buildings.iter().find(|b| b.id.trim_start_matches("sf-building-") == fb::RECORD).expect("the Ferry Building record");
    let ring: Vec<[f64; 2]> = rec.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
    let r = &sp.record;
    for c in [r.north_west, r.north_bay, r.south_bay, r.south_west].iter().chain(r.pavilion.iter()) {
        let want = [c[0] * s, c[1] * s];
        let near = ring.iter().map(|p| dist(*p, want)).fold(f64::INFINITY, f64::min);
        assert!(near < 0.25, "corner {c:?} (x{s}) is {near:.2} m from the drawn record");
    }
    // the tower's surveyed centre is inside the footprint, behind the plaza front.
    let tc = sp.tower.centre_surveyed;
    assert!(eo_landmarks::waterfront::point_in_ring(&rec.ring.iter().map(|p| [p[0] as f64 / s, p[1] as f64 / s]).collect::<Vec<_>>(), tc[0], tc[1]));
}

#[test]
fn it_stands_on_its_floor_to_the_cited_heights() {
    let b = fb::build(&|_, _| 3.0);
    let sp = fb::spec();
    assert_eq!(b.base_y, 3.0);
    let s = eo_data::groundscale::scale();
    // the drawn plan's length and depth (the surveyed 200.7 x 47.2 m, scaled).
    assert!((b.length_m - 200.7 * s).abs() < 1.0, "length {}", b.length_m);
    assert!((b.depth_m - 47.2 * s).abs() < 1.0, "depth {}", b.depth_m);
    // every frontage carries bays of the surveyed pitch (22 over the surveyed length).
    let pitch = 200.7 / sp.body.arches_per_frontage;
    let bay_side = b.bays[2] as f64;
    assert!((bay_side - (b.length_m - 4.4) / pitch).abs() <= 1.0, "{bay_side} bays on the bay front");
    let (mut top, mut stone_top) = (f64::MIN, f64::MIN);
    for p in &b.parts {
        for v in p.mesh.positions.chunks_exact(3) {
            top = top.max(v[1] as f64);
            if p.key == "stone" {
                stone_top = stone_top.max(v[1] as f64);
            }
        }
        for n in p.mesh.normals.chunks_exact(3) {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-3, "{}: a normal of length {l}", p.key);
        }
    }
    // the flagstaff (OSM part 406710839), and the stone up to the last stage under the drum.
    assert!((top - (3.0 + sp.tower.flagstaff_top_m)).abs() < 0.05, "top {top}");
    let last = sp.tower.stages.last().unwrap();
    assert!(stone_top > 3.0 + last.top_m - 1.0 && stone_top <= 3.0 + last.top_m, "stone to {stone_top}");
    assert!(b.parts.iter().any(|p| p.key == "glass") && b.parts.iter().any(|p| p.key == "dial"));
    // within a modest budget (one landmark building).
    assert!(b.triangles < 150_000, "{} triangles", b.triangles);
}

#[test]
fn the_claim_follows_the_kill_switch() {
    assert!(eo_landmarks::set::claimed_records().contains(fb::RECORD));
    eo_landmarks::features::set_ferry_building(false);
    let off = eo_landmarks::set::claimed_records();
    let mut set = eo_landmarks::LandmarkSet::default();
    let built = set.add_ferry_building(&|_, _| 0.0, &|_, _| None, &|_, _| 0.0);
    eo_landmarks::features::set_ferry_building(true);
    assert!(!off.contains(fb::RECORD), "--no-ferry-building keeps the record's box");
    assert!(built.is_none() && set.structures.is_empty());
}

/// The F and E lines' tracks are in the corridor: rails along The Embarcadero and Market Street.
#[test]
fn the_corridor_has_its_streetcar_tracks() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let ways = eo_landmarks::tram::load_corridor_ways(&paths.repo_root, fb::spec().tower.centre_surveyed).unwrap();
    assert!(ways.len() >= 10, "{} tram ways", ways.len());
    let t = eo_landmarks::tram::build(&ways, &|_, _| 2.0, 0.018);
    // the corridor's share of the network only (its runs end within a segment of the corridor's edge).
    let s = eo_data::groundscale::scale();
    assert!(t.metres > 5_000.0 && t.metres < 60_000.0, "{:.0} m of track", t.metres);
    let c = eo_data::groundscale::plan(fb::spec().tower.centre_surveyed);
    for line in &ways {
        for seg in line.windows(2) {
            let near = seg.iter().map(|p| dist(*p, c)).fold(f64::INFINITY, f64::min);
            assert!(near <= eo_landmarks::tram::CORRIDOR_M * s + 1e-6, "a segment {near:.0} m off the tower");
        }
    }
    // a groove a head; with the polish, each band split round its burnished
    // strip and each rail's two sealant joints in the grooves' mesh as well.
    if t.polished {
        assert_eq!(t.grooves.triangles(), t.polish.triangles() * 3);
        assert_eq!(t.heads.triangles(), t.polish.triangles() * 2);
    } else {
        assert_eq!(t.heads.triangles(), t.grooves.triangles());
    }
}
