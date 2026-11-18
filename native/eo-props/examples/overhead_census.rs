//! Plan the street overhead of every street tile of the city and count it.
//!
//!   cargo run --release -p eo-props --example overhead_census [-- --threads N]
use eo_props::overhead::{self, PoleKind, WireKind};
use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let threads = args.iter().position(|a| a == "--threads").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8));
    let t0 = std::time::Instant::now();
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = std::sync::Arc::new(eo_props::load_world(&paths).unwrap());
    let city = eo_data::City::load(&paths).unwrap();
    eprintln!("world {:.1} s", t0.elapsed().as_secs_f64());
    let ov = overhead::install(&w, overhead::DropBuilding::from_city(&city.buildings)).unwrap();
    drop(city);
    println!("overhead index {:.0} ms", ov.build_ms);
    let keys = w.tile_keys();
    let t1 = std::time::Instant::now();
    let chunk = keys.len().div_ceil(threads);
    // Interleave so every thread sees every part of the city.
    let parts: Vec<Vec<(i32, i32)>> = (0..threads).map(|t| keys.iter().skip(t).step_by(threads).copied().collect()).collect();
    let _ = chunk;
    let results: Vec<_> = std::thread::scope(|s| {
        parts
            .iter()
            .map(|part| {
                let (w, ov) = (w.clone(), ov.clone());
                s.spawn(move || {
                    let mut by = BTreeMap::<String, usize>::new();
                    let mut times = vec![];
                    let mut wire_points = 0usize;
                    for &k in part {
                        let t = std::time::Instant::now();
                        let p = ov.plan_tile(&w, k);
                        times.push(t.elapsed().as_secs_f64() * 1e3);
                        for x in &p.poles {
                            *by.entry(format!("pole {:?} {:?}", x.kind, x.material)).or_default() += 1;
                            if x.kind == PoleKind::Distribution {
                                *by.entry("luminaire arm".into()).or_default() += 1;
                            }
                        }
                        for x in &p.wires {
                            *by.entry(format!("wire {}{}", x.kind.js_name(), if x.kind == WireKind::LanternString { " (lantern string)" } else { "" })).or_default() += 1;
                            wire_points += x.points.len();
                        }
                        *by.entry("cabinets".into()).or_default() += p.cabinets.len();
                        *by.entry("sign picket posts".into()).or_default() += p.signs.iter().filter(|s| !s.on_pole).count();
                        *by.entry("sign pole stacks".into()).or_default() += p.signs.iter().filter(|s| s.on_pole).count();
                        *by.entry("sign plates".into()).or_default() += p.signs.iter().map(|s| s.plates.len()).sum::<usize>();
                        *by.entry("corner signs".into()).or_default() += p.corner_signs.len();
                        *by.entry("lanterns".into()).or_default() += p.lanterns.len();
                    }
                    (by, times, wire_points)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });
    let mut by = BTreeMap::<String, usize>::new();
    let mut times = vec![];
    let mut wire_points = 0;
    for (b, t, wp) in results {
        for (k, v) in b {
            *by.entry(k).or_default() += v;
        }
        times.extend(t);
        wire_points += wp;
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
    let pct = |p: f64| times[((times.len() - 1) as f64 * p) as usize];
    println!(
        "{} tiles planned in {:.1} s on {threads} threads: per tile mean {mean:.1} ms, median {:.1}, p95 {:.1}, max {:.0} ms (cold caches, inputs planned on demand)",
        keys.len(),
        t1.elapsed().as_secs_f64(),
        pct(0.5),
        pct(0.95),
        times.last().copied().unwrap_or(0.0)
    );
    println!(
        "  of which planning the inputs (signal poles and hardware, SFMTA posts, surveyed trees via eo-props) {:.1} s thread time ({:.0} %), mean {:.1} ms a tile",
        ov.input_ns() as f64 / 1e9,
        100.0 * ov.input_ns() as f64 / 1e6 / times.iter().sum::<f64>(),
        ov.input_ns() as f64 / 1e6 / times.len().max(1) as f64
    );
    for (k, v) in &by {
        println!("  {k:<28} {v}");
    }
    println!("  wire sample points           {wire_points}");
}
