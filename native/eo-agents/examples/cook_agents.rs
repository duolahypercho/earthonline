//! Cook (or read from the cache) the agent library and print what is in it.
//!
//!   cargo run --release -p eo-agents --example cook_agents -- --scale-report

use anyhow::{Context, Result, bail, ensure};
use eo_agents::catalog::AgentsJs;
use eo_agents::feed::player_body;
use eo_agents::library::{CookOptions, Library, PeopleSource};
use eo_agents::test_characters::Selection;
use eo_agents::{LODS, NO_MESH};
use eo_sim::consts::{PRESENTED_LENGTH, PRESENTED_WIDTH};
use eo_sim::output::{KIND_BUS, KIND_SEDAN, KIND_TAXI, KIND_TRUCK};
use glam::{Quat, Vec2, Vec3, Vec3Swizzles};
use serde_json::{Value, json};

fn mesh_bounds(lib: &Library, model: u32, lod: usize, scale: f32) -> Option<(Vec3, Vec3)> {
    let slot = lib.models[model as usize].meshes[lod];
    if slot == NO_MESH {
        return None;
    }
    let mesh = lib.meshes[slot as usize];
    let base = mesh.base_vertex as usize;
    let mut points = [
        (mesh.first_index, mesh.index_count),
        (mesh.glass_first, mesh.glass_count),
    ]
    .into_iter()
    .flat_map(|(first, count)| lib.indices[first as usize..(first + count) as usize].iter())
    .map(|&index| Vec3::from_array(lib.vertices[base + index as usize].pos) * scale);
    let first = points.next()?;
    Some(points.fold((first, first), |(min, max), p| (min.min(p), max.max(p))))
}

fn model_measurements(
    lib: &Library,
    model: u32,
    scale: f32,
    reference_height: f32,
    contact: Option<(Vec2, Vec2)>,
) -> Value {
    let lods: Vec<_> = (0..LODS)
        .map(|lod| {
            mesh_bounds(lib, model, lod, scale).map(|(min, max)| {
                let size = max - min;
                let mut measured = json!({
                    "min_xyz_m": min.to_array(),
                    "max_xyz_m": max.to_array(),
                    "size_xyz_m": size.to_array(),
                    "size_in_avatar_heights": (size / reference_height).to_array()
                });
                if let Some((contact_min, contact_max)) = contact {
                    measured["mesh_minus_contact_edges_xz_m"] = json!({
                        "min": (min.xz() - contact_min).to_array(),
                        "max": (max.xz() - contact_max).to_array(),
                    });
                }
                measured
            })
        })
        .collect();
    json!({ "model": lib.meta.model_names[model as usize], "scale": scale, "lods": lods })
}

fn scale_report(lib: &Library) -> Result<()> {
    let avatar = if let Some(player) = &lib.test_player {
        player.instance([0.0; 3], Quat::IDENTITY)
    } else {
        player_body(lib, [0.0; 3], 0.0, 0.0, 0.0, 0.0).context(
            "scale report requires the playable avatar; install the people asset set or pass --test-character",
        )?
    };
    let (min, max) = mesh_bounds(lib, avatar.model, 0, avatar.scale)
        .context("playable avatar has no indexed near mesh")?;
    let height = max.y - min.y;
    ensure!(
        height.is_finite() && height > 0.0,
        "invalid playable avatar height"
    );

    let mut vehicles: Vec<_> = lib.meta.vehicle_models.iter().collect();
    vehicles.sort_unstable_by_key(|(name, _)| name.as_str());
    let scale = eo_data::groundscale::vehicles() as f32;
    let vehicles: Vec<_> = vehicles
        .into_iter()
        .map(|(_, &model)| {
            // These are the bounds used to build the local traffic chassis, in
            // the same road-datum frame and at the same scale as the mesh.
            let chassis = lib.vehicle_chassis_geometry(model, scale);
            let contact = chassis.map(|g| (Vec3::from_array(g.min_xyz_m).xz(), Vec3::from_array(g.max_xyz_m).xz()));
            let mut measured = model_measurements(lib, model, scale, height, contact);
            measured["physical_chassis"] = json!(chassis.filter(|g| g.valid()));
            measured["chassis_contact"] = json!(contact.map(|(min, max)| {
                let size = max - min;
                json!({
                    "min_xz_m": min.to_array(),
                    "max_xz_m": max.to_array(),
                    "size_xz_m": size.to_array(),
                    "size_xz_in_avatar_heights": (size / height).to_array(),
                })
            }));
            measured
        })
        .collect();
    let catalogue = AgentsJs::load()?;
    let simulation_footprints: Vec<_> = [
        ("sedan", KIND_SEDAN),
        ("taxi", KIND_TAXI),
        ("truck", KIND_TRUCK),
        ("bus", KIND_BUS),
    ]
    .into_iter()
    .map(|(name, kind)| {
        let index = usize::from(kind);
        let size = Vec2::new(PRESENTED_WIDTH[index], PRESENTED_LENGTH[index]);
        json!({
            "kind": name,
            "size_xz_m": size.to_array(),
            "size_xz_in_avatar_heights": (size / height).to_array(),
            "catalogue_models": catalogue.pool(kind),
        })
    })
    .collect();
    let report = json!({
        "avatar_height_m": height,
        "avatar": model_measurements(lib, avatar.model, avatar.scale, height, None),
        "vehicles": vehicles,
        "simulation_footprints": simulation_footprints,
        "missing_assets": lib.meta.missing,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn main() -> Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let mut args = std::env::args().skip(1);
    let mut cache = Some(eo_assets::default_cache_dir());
    let mut opts = CookOptions::default();
    let mut measure = false;
    let mut test_character = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-cache" => cache = None,
            "--scale-report" => measure = true,
            "--car-detail" => opts.far_body = true,
            "--test-character" => {
                test_character = Some(
                    args.next()
                        .context("--test-character requires an avatar ID")?,
                )
            }
            _ => bail!("unknown argument {arg}"),
        }
    }
    ensure!(
        measure || test_character.is_none(),
        "--test-character requires --scale-report"
    );
    let people = PeopleSource::from_env();
    let mut lib = Library::load_opts(&paths.repo_root, cache.as_deref(), &people, opts)?;
    if let Some(id) = test_character {
        lib.install_test_player(&Selection::from_env(id, None)?)?;
    }
    if measure {
        return scale_report(&lib);
    }
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
