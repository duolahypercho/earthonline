//! Export or audit the native street surface without a renderer.
#[path = "traffic_streets/wheel_audit.rs"]
mod wheel_audit;

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use eo_data::sim_source::TrafficStructures;
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain, plan};

fn save_json(output: &Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = output.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(std::fs::File::create(output).with_context(|| format!("creating {}", output.display()))?);
    serde_json::to_writer(&mut writer, value)?;
    writer.flush()?;
    Ok(())
}

fn main() -> Result<()> {
    let usage = "usage: traffic_streets <output.json> [--structure-levels <levels.json> [--tunnels]]\n       traffic_streets <report.json> --wheel-audit <replay.json> [--structures [--tunnels]] [--body-probes]";
    let mut body_probes = false;
    let mut args = std::env::args().skip(1).filter(|arg| match arg.as_str() {
        "--body-probes" => {
            body_probes = true;
            false
        }
        "--no-surveyed-junction" => false,
        _ => true,
    });
    let output = PathBuf::from(args.next().context(usage)?);
    let (structure_levels, replay) = match args.next().as_deref() {
        None => (None, None),
        Some("--structure-levels") => (Some(PathBuf::from(args.next().context(usage)?)), None),
        Some("--wheel-audit") => (None, Some(PathBuf::from(args.next().context(usage)?))),
        _ => anyhow::bail!("{usage}"),
    };
    let mut next = args.next();
    let joined_audit = replay.is_some() && next.as_deref() == Some("--structures");
    if joined_audit {
        next = args.next();
    }
    let with_structures = structure_levels.is_some() || joined_audit;
    let tunnels = match next.as_deref() {
        None => false,
        Some("--tunnels") if with_structures => true,
        _ => anyhow::bail!("{usage}"),
    };
    ensure!(args.next().is_none(), "{usage}");
    ensure!(!body_probes || replay.is_some(), "{usage}");
    ensure!(structure_levels.as_ref() != Some(&output), "use distinct output files");
    ensure!(replay.as_ref() != Some(&output), "use separate replay and report files");
    let recording = replay.as_deref().map(wheel_audit::load).transpose()?;

    let street_city = StreetCity::load()?;
    let options = StreetOptions::load()?;
    let plan = plan::build_plan(&street_city, &options);
    let paths = eo_data::DataPaths::discover()?;
    let city = eo_data::City::load(&paths)?;
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths)?);
    let buildings = BuildingIndex::new(city.buildings.iter().map(|b| {
        b.ring.iter().map(|p| p.map(f64::from)).collect()
    }).collect());
    let terrain = Terrain { grid, carpet_lift: options.open_ground_lift };
    let streets = StreetWorld::from_plan(options, street_city, plan, terrain, buildings);
    let joined = with_structures.then(|| eo_structures::build_joined(
        &streets, &city, &eo_data::CaStandards::load(&paths)?,
        eo_landmarks::LandmarksJs::get(), eo_structures::StructureOptions { tunnels },
    )).transpose()?;
    let street_surface = joined.as_ref().and_then(|world| world.streets.as_ref()).unwrap_or(&streets);
    if let Some(recording) = recording {
        let structures = joined.as_ref().map(|world| &world.structures);
        return save_json(
            &output,
            &wheel_audit::run(recording, street_surface, structures, tunnels, body_probes)?,
        );
    }
    save_json(&output, &street_surface.traffic_plan())?;
    let (Some(level_path), Some(joined)) = (structure_levels, joined) else {
        return Ok(());
    };
    let structures = TrafficStructures {
        levels: joined.structures.way_levels().clone().into_iter().collect(),
        surfaces: joined.structures.traffic_surfaces(),
    };
    save_json(&level_path, &structures)
}
