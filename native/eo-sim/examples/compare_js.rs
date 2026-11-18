//! Run the Rust core on the shipped JS window with the JS oracle's own
//! configuration (42 cars, the ambient pool, no camera) and print the same
//! per-second statistics the JS run recorded beside them.
//!
//!   cargo run --release -p eo-sim --example compare_js

use eo_sim::region::{Focus, Region, RegionConfig};
use eo_sim::stats::{self, mean};

fn main() -> anyhow::Result<()> {
    let paths = eo_data::DataPaths::discover()?;
    let (net, oracle) = eo_sim::window_network(&paths)?;
    let cfg = RegionConfig::js_pool(oracle.meta.seed_int, oracle.run.ambient_walkers);
    let mut world = eo_sim::World::new(net);
    world.regions.push(Region::new(&world.net, 0, cfg, Focus { x: 1596.6, z: -318.4, yaw: f64::NAN }));
    let mut samples = vec![];
    let t0 = std::time::Instant::now();
    for i in 1..=oracle.run.steps {
        world.step();
        if i % 60 == 0 {
            samples.push(stats::sample(&world.net, &world.regions[0], world.time));
        }
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / oracle.run.steps as f64;
    let js = &oracle.run.samples;
    println!("{:>4} | {:>18} | {:>18} | {:>14} | {:>16} | {:>16}", "t", "moving cars js/rs", "mean speed js/rs", "walking js/rs", "waiting js/rs", "crossing js/rs");
    for (s, j) in samples.iter().zip(js).step_by(30) {
        println!(
            "{:>4.0} | {:>8} {:>9} | {:>8.2} {:>9.2} | {:>6} {:>7} | {:>7} {:>8} | {:>7} {:>8}",
            s.t, j.moving, s.moving, j.mean_speed, s.mean_speed, j.walking, s.walking, j.waiting, s.waiting, j.crossing, s.crossing
        );
    }
    let jm = |f: &dyn Fn(&eo_sim::import_js::OSample) -> f64| js.iter().map(f).sum::<f64>() / js.len() as f64;
    println!("means over the run (JS / Rust):");
    println!("  moving share      {:.3} / {:.3}", jm(&|s| s.moving as f64 / s.cars as f64), mean(&samples, |s| s.moving as f64 / s.cars as f64));
    println!("  mean car speed    {:.2} / {:.2}", jm(&|s| s.mean_speed), mean(&samples, |s| s.mean_speed));
    println!("  walking share     {:.3} / {:.3}", jm(&|s| s.walking as f64 / s.walkers as f64), mean(&samples, |s| s.walking as f64 / s.walkers as f64));
    println!("  waiting           {:.2} / {:.2}", jm(&|s| s.waiting as f64), mean(&samples, |s| s.waiting as f64));
    println!("  waiting at signal {:.2} / {:.2}", jm(&|s| s.waiting_signal as f64), mean(&samples, |s| s.waiting_signal as f64));
    println!("  crossing          {:.2} / {:.2}", jm(&|s| s.crossing as f64), mean(&samples, |s| s.crossing as f64));
    println!("  ground speed      {:.3} / {:.3}", jm(&|s| s.mean_ground_speed), mean(&samples, |s| s.mean_ground_speed));
    println!("  on road           {:.2} / {:.2}", jm(&|s| s.on_road as f64), mean(&samples, |s| s.on_road as f64));
    println!("  on road off-leg   {:.2} / {:.2}", jm(&|s| s.on_road_outside_crossing as f64), mean(&samples, |s| s.on_road_outside_crossing as f64));
    println!("  car overlaps      {:.3} / {:.3}", jm(&|s| s.overlaps as f64), mean(&samples, |s| s.overlaps as f64));
    println!("  person in car     {:.3} / {:.3}", jm(&|s| s.person_in_car as f64), mean(&samples, |s| s.person_in_car as f64));
    println!("  cars off lane     {:.3} / {:.3}", jm(&|s| s.off_lane as f64), mean(&samples, |s| s.off_lane as f64));
    let r = &world.regions[0];
    let last = js.last().unwrap();
    println!(
        "crossings started {} completed {} (JS {} / {}); signalised starts outside WALK: Rust {}, JS {}",
        r.crowd.stats.crossings_started,
        r.crowd.stats.crossings_completed,
        last.crossings_started,
        last.crossings_completed,
        r.crowd.stats.starts_outside_walk,
        last.starts_outside_walk
    );
    println!("cars run through red: Rust {}", r.fleet.stats.red_entries);
    println!("fleet: {:?}", r.fleet.stats);
    println!("rust ms/step {:.4} (JS median {:.2})", ms, oracle.run.step_ms.median);
    Ok(())
}
