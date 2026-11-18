//! cook (or read from the cache) the agent library with the Rocketbox
//! people and print what each avatar became; `--sheet DIR` also writes each
//! avatar's atlas (256 px level) as a PNG for review.
//!
//!   cargo run --release -p eo-agents --example people_cook [-- --no-cache] [--sheet DIR]

use eo_agents::library::{Library, PeopleSource};

fn main() -> anyhow::Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let args: Vec<String> = std::env::args().collect();
    let no_cache = args.iter().any(|a| a == "--no-cache");
    let cache = (!no_cache).then(eo_assets::default_cache_dir);
    let people = PeopleSource::from_env();
    let t = std::time::Instant::now();
    let lib = Library::load_with(&paths.repo_root, cache.as_deref(), &people)?;
    let m = &lib.meta;
    println!(
        "people {:?}: {} ({}), {} models, {} meshes, {:.2} M vertices, {:.2} M triangles, bones {:.2} MB ({} people groups, people tables {:.2} MB), {} atlases at {} px ({:.1} MB), {} in {:.0} ms (people cook {:.0} ms)",
        people,
        m.people_source,
        m.people_fallback.as_deref().unwrap_or("drawn"),
        lib.models.len(),
        lib.meshes.len(),
        lib.vertices.len() as f64 / 1e6,
        lib.indices.len() as f64 / 3e6,
        lib.bones.len() as f64 * 16.0 / 1e6,
        m.people_groups,
        m.people_bone_bytes as f64 / 1e6,
        lib.atlases.len(),
        m.atlas_res,
        lib.atlases.iter().map(|a| a.maps.iter().chain(a.bc.iter()).map(|v| v.len()).sum::<usize>()).sum::<usize>() as f64 / 1e6,
        if lib.from_cache { "read from the cache" } else { "cooked" },
        t.elapsed().as_secs_f64() * 1e3,
        m.people_cook_ms
    );
    for &pm in &m.people_models {
        let model = lib.models[pm as usize];
        let tris: Vec<u32> = model.meshes.iter().map(|&k| lib.triangles(k)).collect();
        let h = m.family_heights.get(m.people_models.iter().position(|&x| x == pm).unwrap()).copied().unwrap_or(0.0);
        println!("  {:>3} {:<22} height {:.2} m, skeleton {} triangles hero/near/mid/far {:?}", pm, m.model_names[pm as usize], h, model.skeleton, tris);
    }
    if !m.people_models.is_empty() {
        println!("player: {} ({:?} triangles)", m.model_names[m.player_model as usize], lib.models[m.player_model as usize].meshes.map(|k| lib.triangles(k)));
    }
    for n in &m.people_notes {
        println!("note: {n}");
    }
    for miss in &m.missing {
        println!("missing: {miss}");
    }
    for (role, ids) in &m.reserved {
        println!("reserved {role}: {}", ids.join(", "));
    }
    // the slots and what fills them.
    let mo = &m.motion;
    println!(
        "motion: walk {} ({} styles, {} frames a loop), standing {} ({} loops); clip frames {:?} base {:?}; people tables {:.2} MB, all joint tables {:.2} MB",
        if mo.walk.is_empty() { "none" } else { &mo.walk },
        mo.styles.len(),
        mo.walk_frames,
        if mo.standing.is_empty() { "none" } else { &mo.standing },
        mo.loops.iter().map(|l| l.len()).sum::<usize>(),
        m.clip_frames,
        m.clip_base,
        m.people_bone_bytes as f64 / 1e6,
        lib.bones.len() as f64 * 16.0 / 1e6
    );
    for (g, row) in mo.group_walk.iter().enumerate() {
        let s: Vec<String> = row.iter().zip(&mo.styles).map(|(w, s)| format!("{} {:.2} m {:.2} m/s", s.name, w[0], w[1])).collect();
        println!("  group {g} ({}): {}", if mo.group_female.get(g) == Some(&true) { "women" } else { "men" }, s.join(", "));
    }
    for n in mo.notes.iter().chain(&mo.fallback) {
        println!("motion note: {n}");
    }
    // `--check-all`: cook every person in the manifest (the pool, the uniforms, the
    // children, the reserve) and report each tier that fails the checks.
    if args.iter().any(|a| a == "--check-all") {
        if let Some(root) = eo_agents::rocketbox::root_from_env() {
            let m = eo_agents::rocketbox::Manifest::load()?;
            let mut bad = 0;
            for a in &m.avatars {
                match eo_agents::rocketbox::cook_avatar(&root, &m, a) {
                    Ok(av) => {
                        let tris: Vec<usize> = av.tiers.iter().map(|t| t.indices.len() / 3).collect();
                        println!("  check {:<24} {:<26} ok, height {:.2} m, triangles {:?}, {:.0} ms{}", a.id, a.role, av.height, tris, av.cook_ms, if av.notes.is_empty() { String::new() } else { format!(" NOTES: {}", av.notes.join("; ")) });
                        bad += usize::from(!av.notes.is_empty());
                    }
                    Err(e) => {
                        bad += 1;
                        println!("  check {:<24} {:<26} FAILED: {e:#}", a.id, a.role);
                    }
                }
            }
            println!("checked {} people: {} with a refused file or tier", m.avatars.len(), bad);
        }
    }
    // `--sheet-ids a,b,c DIR`: cook those people (pool or not) and write their atlases.
    if let Some(i) = args.iter().position(|a| a == "--sheet-ids") {
        let ids: Vec<String> = args.get(i + 1).map(|s| s.split(',').map(|x| x.to_string()).collect()).unwrap_or_default();
        let dir = std::path::PathBuf::from(args.get(i + 2).cloned().unwrap_or_else(|| ".".into()));
        std::fs::create_dir_all(&dir)?;
        if let Some(root) = eo_agents::rocketbox::root_from_env() {
            let m = eo_agents::rocketbox::Manifest::load()?;
            for a in m.avatars.iter().filter(|a| ids.contains(&a.id)) {
                let av = eo_agents::rocketbox::cook_avatar(&root, &m, a)?;
                let r = av.atlas.res >> 2;
                let off: usize = (0..2).map(|l| eo_agents::rocketbox::bc_level_bytes(0, av.atlas.res >> l)).sum();
                let rgba = eo_agents::rocketbox::bc_level_rgba(0, &av.atlas.bc[0][off..off + eo_agents::rocketbox::bc_level_bytes(0, r)], r);
                let f = std::io::BufWriter::new(std::fs::File::create(dir.join(format!("atlas-{}.png", a.id)))?);
                let mut e = png::Encoder::new(f, r, r);
                e.set_color(png::ColorType::Rgba);
                e.set_depth(png::BitDepth::Eight);
                e.write_header()?.write_image_data(&rgba)?;
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--sheet") {
        let dir = std::path::PathBuf::from(args.get(i + 1).cloned().unwrap_or_else(|| ".".into()));
        std::fs::create_dir_all(&dir)?;
        for (layer, a) in lib.atlases.iter().enumerate() {
            if a.bc[0].is_empty() {
                continue;
            }
            // Level 2 (256 px) of the basecolor.
            let off: usize = (0..2).map(|l| eo_agents::rocketbox::bc_level_bytes(0, a.res >> l)).sum();
            let r = a.res >> 2;
            let rgba = eo_agents::rocketbox::bc_level_rgba(0, &a.bc[0][off..off + eo_agents::rocketbox::bc_level_bytes(0, r)], r);
            let name = lib.materials.iter().position(|mt| mt.layer == layer as i32).and_then(|mi| {
                lib.models.iter().enumerate().find(|(_, md)| md.class == eo_agents::CLASS_PERSON && md.meshes[0] != eo_agents::NO_MESH && (lib.vertices[lib.meshes[md.meshes[0] as usize].base_vertex as usize].tag & 0xffff) as usize == mi).map(|(k, _)| m.model_names[k].clone())
            });
            let p = dir.join(format!("atlas-{layer:02}-{}.png", name.unwrap_or_default()));
            let f = std::io::BufWriter::new(std::fs::File::create(&p)?);
            let mut e = png::Encoder::new(f, r, r);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()?.write_image_data(&rgba)?;
        }
        println!("wrote atlases to {}", dir.display());
    }
    Ok(())
}
