//! how each bridge's deck meets its approaches on this process's
//! ground (`--height-scale` via `EO_HEIGHT_SCALE`, the plan via
//! `EO_GROUND_SCALE`), and the highest ground point.
//!   cargo run --release -p eo-landmarks --example hills_bridges

use eo_landmarks::{bay_bridge, golden_gate, set, LandmarksJs};

fn main() -> anyhow::Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let elevation = eo_data::ElevationGrid::load(&paths)?;
    let js = LandmarksJs::get();
    println!("ground plan x{}, heights x{}", eo_data::groundscale::scale(), eo_data::groundscale::height_scale());
    // Highest ground point (every grid cell centre).
    let m = &elevation.meta;
    let mut top = (f64::MIN, 0.0, 0.0);
    for j in 0..m.height {
        for i in 0..m.width {
            let (x, z) = (m.origin_x + (i as f64 + 0.5) * m.cell_size, m.origin_z + (j as f64 + 0.5) * m.cell_size);
            let y = elevation.ground(x, z);
            if y > top.0 {
                top = (y, x, z);
            }
        }
    }
    println!("highest ground {:.1} m (game y) at ({:.0}, {:.0}); {:.1} m above mean sea level", top.0, top.1, top.2, top.0 - eo_data::SEA_LEVEL_Y);
    // The Golden Gate on its registry ground (as the set builds it).
    let gg_ground = set::ground_of(js, "golden-gate-bridge", &|x, z| elevation.sample(x, z));
    let site = golden_gate::Site::new(&js.golden_gate);
    let at = |x: f64, z: f64| gg_ground.at(x, z);
    let p = golden_gate::plan(js, &site, &at);
    let g_s = (p.y_tower - p.y_south) / (0.0 - p.c_south);
    let g_n = (p.y_north - p.y_tower) / (p.c_north - p.l);
    println!(
        "golden gate: south way end deck {:.2} m at chainage {:.0}, tower roadway {:.2} m, mid-span {:.2} m, north end {:.2} m at {:.0}; south approach grade {:.2} %, north {:.2} %; structure from {:.0} to {:.0}",
        p.y_south, p.c_south, p.y_tower, p.y_mid, p.y_north, p.c_north, g_s * 100.0, g_n * 100.0, p.c_start, p.c_end
    );
    let bb_ground = set::ground_of(js, "bay-bridge-west", &|x, z| elevation.sample(x, z));
    let bp = bay_bridge::plan(js, &|x, z| bb_ground.at(x, z));
    // Where each end's ramp (at the approach grade) meets the crest.
    let ramp_a = (bp.crest - bp.y_a) / bp.grade;
    let ramp_b = (bp.crest - bp.y_b) / bp.grade;
    println!(
        "bay bridge west: SF end deck {:.2} m, island end {:.2} m, crest {:.2} m, approach grade {:.2} %; the SF ramp reaches the crest {:.0} m from its end, the island ramp {:.0} m (span {:.0} m)",
        bp.y_a, bp.y_b, bp.crest, bp.grade * 100.0, ramp_a, ramp_b, bp.c_b - bp.c_a
    );
    Ok(())
}
