//! The facade clutter's census (stage W3, `--facade-clutter`): plan and mesh
//! every Nth building at levels 0, 1 and 2 (rungs 0 and 2) on all cores and
//! report what was placed, the triangles each rung gains, the time, and the
//! buildings with fire escapes in a district (for capture poses).
//!
//!   cargo run --release -p eo-kit --example clutter_census [N=5] [district=Tenderloin]

use eo_kit::city::KitCity;
use eo_kit::clutter::{self, Census};
use eo_kit::pipeline::{build_rung_with, plan_building};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default, Clone)]
struct Tally {
    buildings: usize,
    tri: [[usize; 2]; 3],
    ms: [f64; 3],
    census: [Census; 3],
}

fn add(a: &mut Census, b: &Census) {
    a.fire_escapes += b.fire_escapes;
    a.landings += b.landings;
    a.replaced_escapes += b.replaced_escapes;
    a.awnings += b.awnings;
    a.sign_faces += b.sign_faces;
    a.blade_signs += b.blade_signs;
    a.lettered += b.lettered;
    a.neon_letters += b.neon_letters;
    a.window_signs += b.window_signs;
    a.shutters += b.shutters;
    a.boarded += b.boarded;
    a.grilles += b.grilles;
    a.downspouts += b.downspouts;
    a.meter_sets += b.meter_sets;
    a.window_acs += b.window_acs;
    a.dishes += b.dishes;
    a.scaffolds += b.scaffolds;
    a.chimneys += b.chimneys;
    a.vents += b.vents;
    a.aerials += b.aerials;
}

fn main() {
    let step: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let want = std::env::args().nth(2).unwrap_or_else(|| "Tenderloin".into());
    let root = eo_kit::repo_root().expect("repo root");
    let (_city, kc) = KitCity::load(&root).expect("city");
    let l = clutter::limits();
    eprintln!("limits: {l:?}");
    let picks: Vec<usize> = (0..kc.records.len()).step_by(step).filter(|i| !kc.excluded.contains(i)).collect();
    let next = AtomicUsize::new(0);
    let total = Mutex::new(Tally::default());
    let shown = Mutex::new(Vec::<String>::new());
    let threads = std::thread::available_parallelism().map(|v| v.get()).unwrap_or(8);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut t = Tally::default();
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    if k >= picks.len() {
                        break;
                    }
                    let i = picks[k];
                    let Some(p) = plan_building(&kc.records[i], &kc.world, i as u32, 0.0) else { continue };
                    t.buildings += 1;
                    for level in 0..3u8 {
                        let t0 = std::time::Instant::now();
                        for (j, rung) in [0u32, 2].into_iter().enumerate() {
                            t.tri[level as usize][j] += build_rung_with(&p, rung, level).triangles();
                        }
                        t.ms[level as usize] += t0.elapsed().as_secs_f64() * 1e3;
                        let cp = clutter::plan(&p, level);
                        add(&mut t.census[level as usize], &cp.census);
                        if level == 1 && cp.census.fire_escapes > 0 {
                            let d = p.classified.civic.as_ref().and_then(|c| c.district.clone()).unwrap_or_default();
                            if d == want {
                                let e = &p.frame.edges[p.front.index];
                                shown.lock().unwrap().push(format!(
                                    "  #{i} {} storeys {} escapes {} front mid ({:.1}, {:.1}) out ({:.2}, {:.2}) len {:.1}",
                                    p.classified.archetype,
                                    p.plan.storeys.len(),
                                    cp.census.fire_escapes,
                                    e.midpoint.x,
                                    e.midpoint.z,
                                    e.n.x,
                                    e.n.z,
                                    e.length
                                ));
                            }
                        }
                    }
                }
                let mut g = total.lock().unwrap();
                g.buildings += t.buildings;
                for l in 0..3 {
                    for j in 0..2 {
                        g.tri[l][j] += t.tri[l][j];
                    }
                    g.ms[l] += t.ms[l];
                    let c = t.census[l].clone();
                    add(&mut g.census[l], &c);
                }
            });
        }
    });
    let t = total.into_inner().unwrap();
    println!("{} buildings (every {step}th), on {threads} threads", t.buildings);
    for l in 0..3 {
        let pct = |j: usize| 100.0 * (t.tri[l][j] as f64 / t.tri[0][j].max(1) as f64 - 1.0);
        println!(
            "level {l}: near {} tri ({:+.1} %), mid {} tri ({:+.1} %), plan+mesh {:.0} ms (thread time, {:+.1} %)",
            t.tri[l][0],
            pct(0),
            t.tri[l][1],
            pct(1),
            t.ms[l],
            100.0 * (t.ms[l] / t.ms[0].max(1e-9) - 1.0)
        );
        println!("  {:?}", t.census[l]);
    }
    let mut v = shown.into_inner().unwrap();
    v.sort();
    println!("{} with fire escapes in {want} (level 1):", v.len());
    for line in v.iter().take(40) {
        println!("{line}");
    }
}
