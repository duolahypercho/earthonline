//! Cook (or read from the cache) the agent library and print what is in it.
//!
//!   cargo run --release -p eo-agents --example cook_agents [-- --no-cache]

fn main() -> anyhow::Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let no_cache = std::env::args().any(|a| a == "--no-cache");
    let cache = (!no_cache).then(eo_assets::default_cache_dir);
    let lib = eo_agents::library::Library::load(&paths.repo_root, cache.as_deref())?;
    let m = &lib.meta;
    println!(
        "agent library: {} models ({} vehicle bodies, {} people), {} meshes, {} materials, {:.2} M vertices, {:.2} M triangles, {} skeletons, {:.1} MB of joint matrices, {} atlases at {} px; {} in {:.0} ms",
        lib.models.len(),
        m.vehicle_models.len(),
        m.people_models.len(),
        lib.meshes.len(),
        lib.materials.len(),
        lib.vertices.len() as f64 / 1e6,
        lib.indices.len() as f64 / 3e6,
        m.skeletons,
        lib.bones.len() as f64 * 16.0 / 1e6,
        lib.atlases.len(),
        m.atlas_res,
        if lib.from_cache { "read from the cache" } else { "cooked from the GLBs" },
        lib.load_ms
    );
    for (i, model) in lib.models.iter().enumerate() {
        let tris: Vec<u32> = model.meshes.iter().map(|&k| lib.triangles(k)).collect();
        println!("  {:>2} {:<22} class {} radius {:.2} triangles per LOD {:?} wheels {:?}", i, m.model_names[i], model.class, model.radius, tris, model.wheel.map(|w| [w[0], w[1], w[2]]));
    }
    println!("clips: frames {:?} base {:?} seconds {:?}", m.clip_frames, m.clip_base, m.clip_seconds);
    for miss in &m.missing {
        println!("missing: {miss}");
    }
    Ok(())
}
