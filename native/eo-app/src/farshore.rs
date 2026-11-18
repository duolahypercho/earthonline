//! the far terrain's load (on its own thread beside the world
//! load: the cooked file from the cache, else cooked from the USGS DEM in
//! `EO_DEM_ROOT`, else none) and its attach to the renderer. `--no-farshore`
//! or `EO_FARSHORE=0` leaves it out (the "before" of every comparison).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread::JoinHandle;

type Job = JoinHandle<anyhow::Result<Option<eo_farshore::FarTerrain>>>;

static JOB: Mutex<Option<Job>> = Mutex::new(None);

pub fn enabled() -> bool {
    !std::env::args().any(|a| a == "--no-farshore") && std::env::var("EO_FARSHORE").map_or(true, |v| v.trim() != "0")
}

/// Start reading (or cooking) the far terrain.
pub fn spawn(repo_root: &Path, cache: Option<PathBuf>) {
    if !enabled() {
        return;
    }
    let root = repo_root.to_path_buf();
    *JOB.lock().unwrap() = Some(std::thread::spawn(move || eo_farshore::load_or_bake(&root, cache.as_deref())));
}

/// Hand the far terrain to the renderer over the world's elevation grid (the
/// ground plan's extent) and the sea's reach. Returns the bytes allocated.
pub fn attach(ctx: &eo_render::GpuContext, r: &eo_render::Renderer, world: &crate::World) -> u64 {
    let Some(job) = JOB.lock().unwrap().take() else {
        return 0;
    };
    let t = std::time::Instant::now();
    let terrain = match job.join() {
        Ok(Ok(Some(t))) => t,
        Ok(Ok(None)) => return 0,
        Ok(Err(e)) => {
            eprintln!("farshore: no far terrain: {e:#}");
            return 0;
        }
        Err(_) => {
            eprintln!("farshore: no far terrain: the load thread panicked");
            return 0;
        }
    };
    let waited = t.elapsed().as_secs_f64() * 1e3;
    let g = &world.elevation;
    let grid = [g.meta.origin_x as f32, g.meta.origin_z as f32, g.max_x() as f32, g.max_z() as f32];
    // water_common.wgsl `wg_reach`: the grid's extent + 60 km to the west, north and east.
    let reach = [grid[0] - 60_000.0, grid[1] - 60_000.0, grid[2] + 60_000.0, grid[3]];
    let init = eo_render::farshore::FarInit {
        terrain: &terrain,
        grid,
        reach,
        plan_scale: eo_data::groundscale::scale(),
        height_scale: eo_data::groundscale::height_scale(),
    };
    r.attach_farshore(ctx, &init);
    let bytes = r.farshore.get().map_or(0, |f| f.bytes);
    eprintln!(
        "farshore: far terrain attached ({} x {} at {} m, from the {}; {:.0} ms waiting for it, {:.0} ms to upload; {:.1} MB on the GPU)",
        terrain.header.width,
        terrain.header.height,
        terrain.header.texel_m,
        terrain.from,
        waited,
        t.elapsed().as_secs_f64() * 1e3 - waited,
        bytes as f64 / 1e6
    );
    bytes
}
