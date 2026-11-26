//! Plan every street tile of the city and count what is placed.
//!
//!   cargo run --release -p eo-props --example census [threads]
//!
//! counts per model (every street-furniture kind has its own), the
//! district lights, and the plan time per tile (mean, p95, max). The game plans
//! tiles on three worker threads; `threads` (default 3) sets this run's.
//! `EO_TREE_KEEP=1`, `EO_TREE_SIZES=1`, `EO_TREE_SPECIES=1` plan as eo-app's
//! `--tree-keep`, `--tree-sizes`, `--tree-species` do.
fn main() {
    let on = |k: &str| std::env::var(k).is_ok_and(|v| v == "1");
    eo_props::furniture::set_tree_keep(on("EO_TREE_KEEP"));
    eo_props::furniture::set_tree_sizes(on("EO_TREE_SIZES"));
    eo_props::furniture::set_tree_species(on("EO_TREE_SPECIES"));
    let t0 = std::time::Instant::now();
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = std::sync::Arc::new(eo_props::load_world(&paths).unwrap());
    eprintln!("world {:.1} s (props index {:.0} ms)", t0.elapsed().as_secs_f64(), w.build_ms);
    let tf = std::time::Instant::now();
    let f = w.furniture().clone();
    eprintln!("furniture world {:.1} s (index {:.0} ms)", tf.elapsed().as_secs_f64(), f.build_ms);
    let keys = w.tile_keys();
    let t1 = std::time::Instant::now();
    let threads: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let chunk = keys.len().div_ceil(threads);
    let results: Vec<_> = std::thread::scope(|s| {
        keys.chunks(chunk)
            .map(|c| {
                let w = w.clone();
                s.spawn(move || {
                    let mut times = vec![];
                    let mut by = std::collections::BTreeMap::<String, usize>::new();
                    let mut parked = 0;
                    let mut live = 0;
                    let mut lights = 0;
                    for &k in c {
                        let t = std::time::Instant::now();
                        let p = w.plan_tile(k);
                        times.push(t.elapsed().as_secs_f64() * 1e3);
                        for x in &p.props {
                            *by.entry(format!("{:?}", x.model)).or_default() += 1;
                            if !matches!(x.live, eo_props::Live::None) {
                                live += 1;
                            }
                        }
                        parked += p.parked.len();
                        lights += p.lights.len();
                    }
                    (by, parked, live, lights, times)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });
    let mut by = std::collections::BTreeMap::<String, usize>::new();
    let (mut parked, mut live, mut lights) = (0, 0, 0);
    let mut times = vec![];
    for (b, p, l, li, t) in results {
        for (k, v) in b {
            *by.entry(k).or_default() += v;
        }
        parked += p;
        live += l;
        lights += li;
        times.extend(t);
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
    let p95 = times.get(((times.len() as f64) * 0.95) as usize).copied().unwrap_or(0.0);
    let max = times.last().copied().unwrap_or(0.0);
    println!(
        "{} tiles planned in {:.1} s on {threads} threads; per tile mean {mean:.1} ms, p95 {p95:.1} ms, max {max:.0} ms",
        keys.len(),
        t1.elapsed().as_secs_f64()
    );
    for (k, v) in &by {
        println!("  {k:<15} {v}");
    }
    println!("  parked cars     {parked}");
    println!("  live heads      {live}");
    println!("  lamp lights     {lights}");
}
