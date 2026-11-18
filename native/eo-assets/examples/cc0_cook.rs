//! cook (or read from the cache) every CC0 set and print what it became.
//!
//!   cargo run --release -p eo-assets --example cc0_cook [-- --no-cache]

use eo_assets::cc0;

fn main() -> anyhow::Result<()> {
    let Some(root) = cc0::root() else {
        println!("cc0: not available (EO_CC0_ROOT is not set)");
        return Ok(());
    };
    if let Err(e) = cc0::available(&root) {
        println!("cc0: not available ({e:#})");
        return Ok(());
    }
    let cache = (!std::env::args().any(|a| a == "--no-cache")).then(eo_assets::default_cache_dir);
    let t = std::time::Instant::now();
    let sets = cc0::all_sets();
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).saturating_sub(2).max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out: Vec<_> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut got = vec![];
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let Some((kind, layer, def)) = sets.get(i) else { break };
                        got.push((i, *kind, *layer, cc0::load_cached(&root, def, cache.as_deref())));
                    }
                    got
                })
            })
            .collect();
        let mut all: Vec<_> = hs.into_iter().flat_map(|h| h.join().unwrap()).collect();
        all.sort_by_key(|x| x.0);
        all
    });
    let mut bytes = 0u64;
    for (_, kind, layer, r) in out {
        match r {
            Ok(c) => {
                bytes += c.bytes();
                println!(
                    "{kind:?} {layer:2} {:28} {} px, {:.2} x {:.2} m, mean rgb {:.3} {:.3} {:.3} rough {:.2} ao {:.2} height {:.2}, {:.1} MB, {} {:.0} ms",
                    c.id,
                    c.res,
                    c.size_m[0],
                    c.size_m[1],
                    c.mean_rgb[0],
                    c.mean_rgb[1],
                    c.mean_rgb[2],
                    c.mean_rough,
                    c.mean_ao,
                    c.mean_height,
                    c.bytes() as f64 / 1e6,
                    if c.from_cache { "cache" } else { "cooked" },
                    c.ms
                );
            }
            Err(e) => println!("{kind:?} {layer:2} FAILED: {e:#}"),
        }
    }
    println!("{:.1} MB of GPU blocks, {:.1} s on {threads} threads", bytes as f64 / 1e6, t.elapsed().as_secs_f64());
    Ok(())
}
