//! Debug probe: time each placement rule on the tiles round a point.
//!
//!   cargo run --release -p eo-props --example probe -- 1600 -400
fn main() {
    let a: Vec<f64> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let (x, z) = (a.first().copied().unwrap_or(1600.0), a.get(1).copied().unwrap_or(-400.0));
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = eo_props::load_world(&paths).unwrap();
    let key = eo_street::tile_key(x, z);
    let mut entries = vec![];
    for dz in -1..=1 {
        for dx in -1..=1 {
            if let Some(t) = w.street.tiles.get(&(key.0 + dx, key.1 + dz)) {
                entries.extend(t.spans.iter().map(|s| s.0 as usize));
            }
        }
    }
    entries.sort_unstable();
    entries.dedup();
    let t = std::time::Instant::now();
    let _ = eo_props::parking::kerb_mouths(&w);
    let _ = eo_props::signs::entry_cells(&w);
    println!("indices {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = std::time::Instant::now();
    let mut n = 0;
    for &e in &entries {
        n += eo_props::trees::plan_entry(&w, e).len();
    }
    println!("trees: {} entries {n} trees {:.0} ms", entries.len(), t.elapsed().as_secs_f64() * 1e3);
    let t = std::time::Instant::now();
    let mut n = 0;
    for &e in &entries {
        n += eo_props::parking::plan_entry(&w, e).len();
    }
    println!("parking: {n} cars {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = std::time::Instant::now();
    let mut n = vec![];
    for &e in &entries {
        eo_props::pads::entry_pads(&w, &w.street.plan.entries[e], &mut n);
    }
    println!("pads: {} {:.0} ms", n.len(), t.elapsed().as_secs_f64() * 1e3);
    let t = std::time::Instant::now();
    let s = eo_props::signs::plan_tile(&w, key);
    println!("signs: {} props {:.0} ms", s.len(), t.elapsed().as_secs_f64() * 1e3);
    let t = std::time::Instant::now();
    let mut n = vec![];
    for (ni, node) in w.street.plan.nodes.iter().enumerate() {
        if node.signalised() && eo_street::tile_key(node.position[0], node.position[1]) == key {
            eo_props::signals::plan_node(&w, ni, &mut n);
        }
    }
    println!("signals: {} props {:.0} ms", n.len(), t.elapsed().as_secs_f64() * 1e3);
    let t = std::time::Instant::now();
    let p = w.plan_tile(key);
    println!("plan_tile (cached entries): {} props {} parked {:.0} ms", p.props.len(), p.parked.len(), t.elapsed().as_secs_f64() * 1e3);
}
