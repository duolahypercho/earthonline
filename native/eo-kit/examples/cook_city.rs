//! Plan and mesh every building of the city once (rungs 0 and 2) on all
//! cores and report counts, failures, triangles and time. A census, not a test.
//!
//!   cargo run --release -p eo-kit --example cook_city

use eo_kit::city::KitCity;
use eo_kit::pipeline::{build_rung, plan_building};
use std::sync::atomic::{AtomicUsize, Ordering};

fn main() {
    let root = eo_kit::repo_root().expect("repo root");
    let t = std::time::Instant::now();
    let (_city, kc) = KitCity::load(&root).expect("city");
    eprintln!("city + kit world: {:.0} ms (kit records and indices {:.0} ms)", t.elapsed().as_secs_f64() * 1e3, kc.build_ms);
    let n = kc.records.len();
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map(|v| v.get()).unwrap_or(8);
    let t = std::time::Instant::now();
    let results: Vec<(usize, usize, usize, usize, usize, usize, std::collections::BTreeMap<&'static str, usize>)> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let (mut ok, mut none, mut panics, mut t0, mut t2, mut v0) = (0, 0, 0, 0, 0, 0);
                    let mut by = std::collections::BTreeMap::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= n {
                            break;
                        }
                        if kc.excluded.contains(&i) {
                            continue;
                        }
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            plan_building(&kc.records[i], &kc.world, i as u32, 0.0).map(|p| (p.classified.archetype, build_rung(&p, 0), build_rung(&p, 2)))
                        }));
                        match r {
                            Ok(Some((a, m0, m2))) => {
                                ok += 1;
                                t0 += m0.triangles();
                                t2 += m2.triangles();
                                v0 += m0.vertices();
                                *by.entry(a).or_insert(0) += 1;
                            }
                            Ok(None) => none += 1,
                            Err(_) => panics += 1,
                        }
                    }
                    (ok, none, panics, t0, t2, v0, by)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let ok: usize = results.iter().map(|r| r.0).sum();
    let none: usize = results.iter().map(|r| r.1).sum();
    let panics: usize = results.iter().map(|r| r.2).sum();
    let t0: usize = results.iter().map(|r| r.3).sum();
    let t2: usize = results.iter().map(|r| r.4).sum();
    let v0: usize = results.iter().map(|r| r.5).sum();
    let mut by = std::collections::BTreeMap::new();
    for r in &results {
        for (k, v) in &r.6 {
            *by.entry(*k).or_insert(0) += v;
        }
    }
    eprintln!(
        "{n} records, {} excluded (authored); planned {ok}, no plan {none}, panics {panics}; {threads} threads {ms:.0} ms ({:.3} ms per building per thread)",
        kc.excluded.len(),
        ms * threads as f64 / ok.max(1) as f64
    );
    eprintln!("triangles: rung0 {t0} ({:.0}/building), rung2 {t2} ({:.0}/building); rung0 vertices {v0}", t0 as f64 / ok as f64, t2 as f64 / ok as f64);
    eprintln!("{by:?}");
}
