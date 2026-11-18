//! Every building's ground floor against the street surface it stands beside
//! (slice 4). No GPU.
//!
//! For every kit building and every Blender-built building of the city:
//!   floor          where its ground floor is drawn (kit: the street-front floor
//!                  the cook plans it at; Blender: the JS placement's baseY)
//!   bottom         the lowest point of its walls (the foundation skirt's foot)
//!   front gap      floor - mean drawn surface 0.3 m outside its street face
//!                  (the JS floor probe), for a building with a street face
//!   float gap      bottom - lowest drawn surface round its outline (every
//!                  0.25 m, on the wall line, 0.1 m and 0.3 m out): > 0 is sky
//!                  under a wall
//!
//! FAILS if any building floats (float gap > FLOAT_TOL_M) or has its entrance
//! buried (front gap < -BURIED_TOL_M) other than where the JS's own deep-lot
//! rule holds the floor (`clampDeepFootprint`: relief over 8 m, floor kept
//! within 3 m of the centre ground), which is reported, not failed.
//!
//! Tolerances. FLOAT_TOL_M = 0.02 m: the skirt is sized on the same drawn
//! surface 0.12 m (`embedM`) below its lowest sample, so anything over 2 cm is
//! a defect, not sampling. BURIED_TOL_M = 0.05 m: a third of the 6 in kerb
//! (`curb-height-sf-standard`), the step a doorway threshold hides; the JS floor
//! is the mean of the same probes, so a kit floor differs from it only where
//! the deep-lot clamp moved it, and a Blender floor by how far the JS's
//! terrain-based placement differs from the drawn walk.
//!
//! The same numbers for slice 3 (kit floor = lowest ground under the ring, no
//! skirts, no streets drawn: measured against the bare terrain it drew, and
//! against the street surface now drawn) are printed as "before".

use eo_kit::city::KitCity;
use eo_render::kit::kit_floor;
use eo_street::grounding::{FrontEdge, Grounding, measure};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::collections::HashSet;
use std::sync::Arc;

const FLOAT_TOL_M: f64 = 0.02;
const BURIED_TOL_M: f64 = 0.05;

struct Row {
    id: String,
    kind: &'static str,
    after: Grounding,
    before_vs_new_streets: Grounding,
    before_vs_bare_terrain: (f64, Option<f64>),
}

fn pct(v: &mut Vec<f64>, p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((p / 100.0) * (v.len() - 1) as f64).round() as usize]
}

#[test]
fn every_building_meets_the_street() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = Arc::new(eo_data::City::load(&paths).unwrap());
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings: Vec<Vec<[f64; 2]>> = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let sw = Arc::new(StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev.clone()), carpet_lift: lift }, BuildingIndex::new(rings.clone())));
    let tenancy = eo_kit::tenancy::Tenancy::load(&paths.repo_root).unwrap();
    let kc = KitCity::build(&city, &tenancy, &eo_kit::city::load_exclusions());
    let manifest = eo_assets::load_blender_buildings(&paths.repo_root).unwrap();
    let constants = eo_assets::placement::PlacementConstants::load().unwrap();
    let authored_keys: HashSet<String> = manifest.iter().map(|b| b.record_key().to_string()).collect();
    let mut excluded: HashSet<String> = authored_keys.clone();
    for id in eo_kit::city::load_exclusions() {
        excluded.insert(id.trim_start_matches("sf-building-").to_string());
    }
    // Blender placements, as the renderer places them.
    let by_key: std::collections::HashMap<&str, usize> = city.buildings.iter().enumerate().map(|(i, b)| (b.id.as_str(), i)).collect();
    let mut authored_base: std::collections::HashMap<usize, (f64, usize)> = std::collections::HashMap::new();
    for b in &manifest {
        let Some(&i) = by_key.get(b.record_key()) else { continue };
        let ring: Vec<glam::DVec2> = city.buildings[i].ring.iter().map(|p| glam::DVec2::new(p[0] as f64, p[1] as f64)).collect();
        if let Ok(p) = eo_assets::placement::place(b, &ring, city.buildings[i].height as f64, &constants, &|x, z| elev.sample(x, z)) {
            authored_base.insert(i, (p.base_y, b.frame.edge_index));
        }
    }
    // Work in tile order so the street meshes each thread reads stay cached.
    let mut order: Vec<usize> = (0..city.buildings.len()).collect();
    order.sort_by_key(|&i| {
        let r = &rings[i];
        eo_street::tile_key(r[0][0], r[0][1])
    });
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let chunk = order.len().div_ceil(threads);
    let t0 = std::time::Instant::now();
    let rows: Vec<Row> = std::thread::scope(|s| {
        let hs: Vec<_> = order
            .chunks(chunk)
            .map(|c| {
                let (sw, kc, rings, city, elev, excluded, authored_base) = (&sw, &kc, &rings, &city, &elev, &excluded, &authored_base);
                s.spawn(move || {
                    let mut out = vec![];
                    for &i in c {
                        let b = &city.buildings[i];
                        let ring = &rings[i];
                        let lowest_ground = b.ring.iter().map(|p| elev.ground(p[0] as f64, p[1] as f64)).fold(f64::MAX, f64::min);
                        let mut js_floor = None;
                        let (kind, floor, bottom, front, clamped) = if let Some(&(js_base, edge)) = authored_base.get(&i) {
                            let record: Vec<[f64; 2]> = b.record_polygon().iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
                            let (base, front) = sw.authored_floor(&record, edge, js_base);
                            js_floor = Some(js_base);
                            let f = sw.foundation(ring, base, sw.o.archetype_foundation_skirt_m);
                            ("blender", base, f.bottom_y, front, false)
                        } else if !excluded.contains(&b.id) {
                            let (floor, f, front) = kit_floor(kc, sw, i as u32);
                            ("kit", floor.y, f.bottom_y, front, floor.clamped)
                        } else {
                            continue; // drawn by a code-built authored module: a footprint box
                        };
                        let after = measure(sw, ring, floor, bottom, front.as_ref(), clamped);
                        let before_floor = js_floor.unwrap_or(lowest_ground);
                        let before = measure(sw, ring, before_floor, before_floor, front.as_ref(), false);
                        // Against the bare terrain slice 3 drew: no street, no lift.
                        let bare_front = front.as_ref().map(|e: &FrontEdge| {
                            let mean = (0..sw.o.floor_probe_stations)
                                .map(|k| {
                                    let t = sw.floor_probe_t(k);
                                    elev.ground(e.start[0] + e.u[0] * e.length * t + e.n[0] * sw.o.walk_probe_m, e.start[1] + e.u[1] * e.length * t + e.n[1] * sw.o.walk_probe_m)
                                })
                                .sum::<f64>()
                                / sw.o.floor_probe_stations as f64;
                            before_floor - mean
                        });
                        let bare_float = before_floor - lowest_ground;
                        out.push(Row { id: b.id.clone(), kind, after, before_vs_new_streets: before, before_vs_bare_terrain: (bare_float, bare_front) });
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    println!("measured {} buildings in {:.1} s", rows.len(), t0.elapsed().as_secs_f64());
    let mut fails = vec![];
    for kind in ["kit", "blender"] {
        let rs: Vec<&Row> = rows.iter().filter(|r| r.kind == kind).collect();
        let report = |name: &str, g: &dyn Fn(&Row) -> &Grounding| {
            let floating = rs.iter().filter(|r| g(r).float_gap > FLOAT_TOL_M).count();
            let buried = rs.iter().filter(|r| g(r).front_gap.map(|x| x < -BURIED_TOL_M).unwrap_or(false) && !g(r).clamped).count();
            let clamped_buried = rs.iter().filter(|r| g(r).front_gap.map(|x| x < -BURIED_TOL_M).unwrap_or(false) && g(r).clamped).count();
            let raised = rs.iter().filter(|r| g(r).front_gap.map(|x| x > BURIED_TOL_M).unwrap_or(false)).count();
            let mut fg: Vec<f64> = rs.iter().filter_map(|r| g(r).front_gap).collect();
            let mut fl: Vec<f64> = rs.iter().map(|r| g(r).float_gap).collect();
            let mut corners: Vec<f64> = rs.iter().flat_map(|r| g(r).corner_gaps.clone()).collect();
            println!(
                "{kind} {name}: n {} (street face {}); FLOATING {floating}; ENTRANCE BURIED {buried} (+{clamped_buried} held by the JS deep-lot clamp); floor over the walk by > {BURIED_TOL_M} m: {raised}\n    front gap m  p1 {:.3} p50 {:.3} p99 {:.3} min {:.3} max {:.3}\n    float gap m  p50 {:.3} p99 {:.3} max {:.3}\n    corner gap m (floor - surface outside each corner) p1 {:.3} p50 {:.3} p99 {:.3}",
                rs.len(),
                fg.len(),
                pct(&mut fg, 1.0),
                pct(&mut fg, 50.0),
                pct(&mut fg, 99.0),
                pct(&mut fg, 0.0),
                pct(&mut fg, 100.0),
                pct(&mut fl, 50.0),
                pct(&mut fl, 99.0),
                pct(&mut fl, 100.0),
                pct(&mut corners, 1.0),
                pct(&mut corners, 50.0),
                pct(&mut corners, 99.0),
            );
            (floating, buried)
        };
        let (f, b) = report("AFTER", &|r: &Row| &r.after);
        report("BEFORE (slice 3 floors, no skirts) vs the street surface now drawn", &|r: &Row| &r.before_vs_new_streets);
        let bare_float = rs.iter().filter(|r| r.before_vs_bare_terrain.0 > FLOAT_TOL_M).count();
        let bare_buried = rs.iter().filter(|r| r.before_vs_bare_terrain.1.map(|x| x < -BURIED_TOL_M).unwrap_or(false)).count();
        println!("{kind} BEFORE vs the bare terrain slice 3 drew: floating {bare_float}, entrance below the ground in front by > {BURIED_TOL_M} m {bare_buried}");
        let mut worst: Vec<&Row> = rs.iter().copied().filter(|r| r.after.front_gap.is_some()).collect();
        worst.sort_by(|a, b| a.after.front_gap.unwrap().partial_cmp(&b.after.front_gap.unwrap()).unwrap());
        for r in worst.iter().take(5) {
            println!("    lowest floor vs walk: {} front gap {:.3} m (front spread {:.2} m, clamped {})", r.id, r.after.front_gap.unwrap(), r.after.front_spread, r.after.clamped);
        }
        let mut fw: Vec<&Row> = rs.clone();
        fw.sort_by(|a, b| b.after.float_gap.partial_cmp(&a.after.float_gap).unwrap());
        for r in fw.iter().take(3) {
            println!("    highest wall foot: {} float gap {:.3} m", r.id, r.after.float_gap);
        }
        if f > 0 {
            fails.push(format!("{kind}: {f} floating"));
        }
        if b > 0 {
            fails.push(format!("{kind}: {b} with the entrance buried"));
        }
    }
    assert!(fails.is_empty(), "{fails:?}");
}
