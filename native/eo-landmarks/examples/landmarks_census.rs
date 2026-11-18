//! `cargo run --release -p eo-landmarks --example landmarks_census`: what the set builds
//! on the real ground (bridges, the citywide wall by tier, the pier, the
//! promenades on the bare ground), with timings. Plazas need the street world
//! (eo-app's load) and are not built here.

fn main() -> anyhow::Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let elevation = eo_data::ElevationGrid::load(&paths)?;
    let ca = eo_data::CaStandards::load(&paths)?;
    let t = std::time::Instant::now();
    let mut set = eo_landmarks::LandmarkSet::build(&paths.repo_root, &|x, z| elevation.sample(x, z), &ca)?;
    let pr = set.add_promenades(&|x, z| elevation.ground(x, z), &|x, z| elevation.ground(x, z));
    println!("built in {:.0} ms (bridges {:.0} ms, shore {:.0} ms)", t.elapsed().as_secs_f64() * 1e3, set.build_ms, set.walls_ms);
    for l in &set.landmarks {
        let tris: Vec<String> = l.draws.iter().map(|d| format!("{}:{:?} {}", d.key, d.role, d.mesh.triangles())).collect();
        println!("{} ({}): {} | box {:?} .. {:?} | {} colliders", l.name, l.version, tris.join(", "), l.box_min, l.box_max, l.colliders.len());
    }
    let mut by_tier = std::collections::BTreeMap::new();
    for w in &set.walls {
        let e = by_tier.entry(format!("{:?}", w.tier)).or_insert((0usize, 0f64, 0usize, 0usize));
        e.0 += 1;
        e.1 += w.run_m;
        e.2 += w.near.triangles();
        e.3 += w.far.triangles();
    }
    for (tier, (n, run, near, far)) in by_tier {
        println!("wall {tier}: {n} chunks, {:.1} km, near {near} / far {far} triangles", run / 1e3);
    }
    for s in &set.structures {
        println!("structure {}: {} triangles (casts {})", s.name, s.mesh.triangles(), s.cast);
    }
    println!("Ocean Beach seawall {} stations, {} stairs; Sunset Dunes {} stations, {} flights", pr[0].0, pr[0].1, pr[1].0, pr[1].1);
    Ok(())
}
