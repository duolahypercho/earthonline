//! The structures census on the real city: loads the city, the elevation
//! grid and the street world as eo-app does, builds the grade-separated
//! roads with eo-app's `--structures` (add `--tunnels` for its `--tunnels`),
//! and prints the census and every held chain (way ids, names, reason);
//! with `--meshes`, every tile's meshes at every level built and counted;
//! with `--joins` (stage A6), the second pass too: the street world built
//! again with the structures' joins (`StructureWorld::street_joins`) and the
//! structures solved again on it, its census and what the joins did.
//!
//! `cargo run --release -p eo-structures --example census [-- --tunnels] [--meshes] [--joins]`

use std::sync::Arc;
use std::time::Instant;

use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use eo_structures::{StructureOptions, StructureWorld};

fn main() -> anyhow::Result<()> {
    let t0 = Instant::now();
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load(&paths)?;
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths)?);
    let o = StreetOptions::load()?;
    let sc = StreetCity::load()?;
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    let streets = StreetWorld::from_plan(o, sc, plan, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings));
    let ca = eo_data::CaStandards::load(&paths)?;
    let loaded = t0.elapsed().as_secs_f64();
    let t1 = Instant::now();
    let tunnels = std::env::args().any(|a| a == "--tunnels");
    let w = StructureWorld::build(&streets, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions { tunnels })?;
    let built = t1.elapsed().as_secs_f64();
    if std::env::args().any(|a| a == "--joins") {
        let t2 = Instant::now();
        let j = eo_structures::build_joined(&streets, &city, &ca, eo_landmarks::LandmarksJs::get(), StructureOptions { tunnels })?;
        let secs = t2.elapsed().as_secs_f64();
        let w2 = &j.structures;
        println!("eo-structures census with the street joins (stage A6; ground scale {}, tunnels {})", eo_data::groundscale::scale(), if tunnels { "on" } else { "off" });
        println!("{}", w2.census());
        println!();
        println!("held chains ({}):", w2.held().count());
        for c in w2.held() {
            println!("  chain {:4} {:13} {:7.1} m  ends {}/{}  ways {}  reason: {}", c.id, c.class.name(), c.length(), c.ends[0].kind.name(), c.ends[1].kind.name(), c.ways().join(","), c.hold().map(|h| h.to_string()).unwrap_or_default());
        }
        let key = |c: &eo_structures::Chain| c.pieces.iter().map(|p| p.key.clone()).collect::<Vec<_>>();
        let was: std::collections::HashMap<Vec<String>, (bool, String)> = w.chains().iter().map(|c| (key(c), (c.is_built(), c.hold().map(|h| h.to_string()).unwrap_or_default()))).collect();
        println!();
        for c in w2.chains() {
            if let Some((b, why)) = was.get(&key(c)) {
                if *b != c.is_built() {
                    println!("CHANGED chain {} ({}, {:.0} m, ways {}): {} -> {}", c.id, c.name(), c.length(), c.ways().join(","), if *b { "built".to_string() } else { format!("held ({why})") }, if c.is_built() { "built".to_string() } else { format!("held ({})", c.hold().map(|h| h.to_string()).unwrap_or_default()) });
                }
            }
        }
        eprintln!("loaded in {loaded:.1} s, first structures {built:.2} s, joined build (all passes) {secs:.2} s");
        return Ok(());
    }
    println!("eo-structures census (ground scale {}, tunnels {})", eo_data::groundscale::scale(), if tunnels { "on" } else { "off" });
    println!("{}", w.census());
    println!();
    println!("held chains ({}):", w.held().count());
    for c in w.held() {
        let mut names: Vec<&str> = c.pieces.iter().map(|p| p.name.as_str()).filter(|n| !n.is_empty()).collect();
        names.dedup();
        println!(
            "  chain {:4} {:13} {:8} level {:2} {:7.1} m  ends {}/{}  ways {}  names [{}]  reason: {}",
            c.id,
            c.class.name(),
            format!("{:?}", c.kind).to_lowercase(),
            c.level,
            c.length(),
            c.ends[0].kind.name(),
            c.ends[1].kind.name(),
            c.ways().join(","),
            names.join("; "),
            c.hold().map(|h| h.to_string()).unwrap_or_default()
        );
    }
    if std::env::args().any(|a| a == "--meshes") {
        let t2 = Instant::now();
        let st = w.mesh_stats();
        let secs = t2.elapsed().as_secs_f64();
        println!();
        println!("meshes: {} tiles", st.tiles);
        for (lod, name) in ["near", "far", "coarse"].iter().enumerate() {
            println!("  {name:6} {:9} triangles {:9} vertices {:8.1} MB; largest tile {} triangles, {} vertices", st.triangles[lod], st.vertices[lod], st.bytes[lod] as f64 / 1048576.0, st.max_tile_triangles[lod], st.max_tile_vertices[lod]);
        }
        println!("  all    {:9} triangles {:9} vertices {:8.1} MB", st.triangles.iter().sum::<usize>(), st.vertices.iter().sum::<usize>(), st.bytes.iter().sum::<usize>() as f64 / 1048576.0);
        println!("collision: {} deck-surface triangles, {} barrier triangles", st.street_triangles, st.barrier_triangles);
        let cols = w.all_column_footprints();
        println!("column footprints: {} ({} in a street)", cols.len(), cols.iter().filter(|c| c.site.on_street()).count());
        eprintln!("meshes built in {secs:.1} s");
    }
    if std::env::args().any(|a| a == "--merges") {
        println!();
        merges(&w);
    }
    eprintln!("loaded in {loaded:.1} s, structures built in {built:.2} s");
    Ok(())
}

/// The merge survey (stage A4's finding, stage A5's check): every pair of
/// built same-level deck chains, sampled every metre along one where its
/// centreline lies over the other's roadway (within its half width, short
/// of its ends): how far apart the two profiles' deck tops are there, and
/// how far apart the drawn deck surfaces over one point are (at the sample
/// and at four points 0.25 m round it).
fn merges(w: &StructureWorld) {
    use eo_structures::ChainKind;
    let decks: Vec<&eo_structures::Chain> = w.chains().iter().filter(|c| c.is_built() && c.kind == ChainKind::Deck && c.landmark.is_none()).collect();
    // (worst signed difference, its station, metres of overlap, shared node, worst drawn spread)
    let mut rows: Vec<(u32, u32, f64, f64, f64, bool, f64)> = vec![];
    for c in &decks {
        for d in &decks {
            if c.id == d.id || c.level != d.level {
                continue;
            }
            let shared = c.ends.iter().any(|e| d.ends.iter().any(|x| x.node == e.node));
            let (mut worst, mut at, mut metres, mut spread) = (0.0f64, 0.0, 0.0, 0.0f64);
            let n = c.length().floor() as usize;
            for k in 0..=n {
                let s = k as f64;
                let f = c.frame(s);
                let (sd, dist) = d.project(f.x, f.z);
                if sd <= 1e-6 || sd >= d.length() - 1e-6 || dist > d.half_at(sd) {
                    continue;
                }
                metres += 1.0;
                let dy = c.y_at(s) - d.y_at(sd);
                if dy.abs() > worst.abs() {
                    worst = dy;
                    at = s;
                }
                // the drawn deck surfaces over the point: their spread.
                let key = eo_street::tile_key(f.x, f.z);
                // (heights at each of the five points: two surfaces at one point.)
                let mut ys: Vec<Vec<f64>> = vec![vec![]; 5];
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let Some(m) = w.render_tile_shared((key.0 + dx, key.1 + dz), 0) else { continue };
                        for t in m.indices.chunks_exact(3) {
                            if m.classes[t[0] as usize] != eo_structures::StructureClass::DeckSurface as u8 {
                                continue;
                            }
                            let p = |i: u32| m.positions[i as usize].map(|v| v as f64);
                            // (the point and four 0.25 m round it: a step at a cut shows.)
                            for (q, (ox, oz)) in [(0.0, 0.0), (0.25, 0.0), (-0.25, 0.0), (0.0, 0.25), (0.0, -0.25)].into_iter().enumerate() {
                                if let Some(y) = over([p(t[0]), p(t[1]), p(t[2])], f.x + ox, f.z + oz) {
                                    if (y - c.y_at(s)).abs() < 3.0 || (y - d.y_at(sd)).abs() < 3.0 {
                                        ys[q].push(y);
                                    }
                                }
                            }
                        }
                    }
                }
                for at in &ys {
                    if at.len() > 1 {
                        let (lo, hi) = (at.iter().copied().fold(f64::INFINITY, f64::min), at.iter().copied().fold(f64::NEG_INFINITY, f64::max));
                        spread = spread.max(hi - lo);
                    }
                }
            }
            if metres > 0.0 {
                rows.push((c.id, d.id, worst, at, metres, shared, spread));
            }
        }
    }
    rows.sort_by(|a, b| b.2.abs().total_cmp(&a.2.abs()).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    let over_2cm = rows.iter().filter(|r| r.2.abs() > 0.02).count();
    let over_30 = rows.iter().filter(|r| r.2.abs() > 0.3).count();
    let over_1m = rows.iter().filter(|r| r.2.abs() > 1.0).count();
    let shared = rows.iter().filter(|r| r.5).count();
    let doubled = rows.iter().filter(|r| r.6 > 0.02).count();
    println!("merges: {} overlapping same-level pairs ({} sharing a node); tops apart: {} over 2 cm, {} over 0.3 m, {} over 1 m; drawn surfaces apart over 2 cm: {}", rows.len(), shared, over_2cm, over_30, over_1m, doubled);
    println!("# pair (chain over chain): tops apart (first minus second) at station s of the first; metres of overlap; shares a node; the largest height between two drawn deck surfaces over one point (the sample or 0.25 m round it)");
    for r in &rows {
        println!("MERGE chain {} over chain {}: tops {:+.3} m apart at s {:.0} ({:.0} m of overlap){}; drawn spread {:.3} m", r.0, r.1, r.2, r.3, r.4, if r.5 { ", shared node" } else { "" }, r.6);
    }
}

/// Height of a triangle over plan point (x, z), if it is inside it.
fn over(t: [[f64; 3]; 3], x: f64, z: f64) -> Option<f64> {
    let (a, b, c) = (t[0], t[1], t[2]);
    let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
    let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
    let l3 = 1.0 - l1 - l2;
    (l1 >= -1e-9 && l2 >= -1e-9 && l3 >= -1e-9).then(|| l1 * a[1] + l2 * b[1] + l3 * c[1])
}
