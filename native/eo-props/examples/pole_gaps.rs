//! Every vertical pole the props layers place (furniture lamps and sign poles,
//! signal and pedestrian-signal poles, SFMTA sign posts, overhead support, span
//! and distribution poles, sign picket posts), and how close each stands to a
//! pole of another layer or kind. Prints the close pairs by kind pair.
//!
//!   cargo run --release -p eo-props --example pole_gaps [-- --near 2.5 --list 20 --threads N --no-shared-poles --no-joint-dedupe --no-sign-post-merge]
use eo_props::Model;
use eo_props::overhead::{self, PoleKind};
use std::collections::BTreeMap;

#[derive(Clone)]
struct Pole {
    x: f64,
    z: f64,
    kind: &'static str,
    id: String,
}

fn prop_kind(m: Model) -> Option<&'static str> {
    Some(match m {
        Model::LampCobra => "lampCobra",
        Model::LampHistoric => "lampHistoric",
        Model::LampBanner => "lampBanner",
        Model::SignPole => "furnitureSignPole",
        Model::SignPost => "sfmtaSignPost",
        Model::SignalPole => "signalPole",
        Model::PedSignalPost => "pedSignalPost",
        _ => return None,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let near: f64 = arg("--near").and_then(|v| v.parse().ok()).unwrap_or(2.5);
    let shots: Option<String> = arg("--shots");
    let shot_pair: String = arg("--shot-pair").unwrap_or_else(|| "lampBanner + overheadSupport".into());
    let shot_box: Vec<f64> = arg("--shot-box").map(|v| v.split(',').filter_map(|t| t.parse().ok()).collect()).unwrap_or_else(|| vec![1500.0, -2600.0, 3000.0, -800.0]);
    let list: usize = arg("--list").and_then(|v| v.parse().ok()).unwrap_or(0);
    let threads: usize = arg("--threads").and_then(|v| v.parse().ok()).unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8));
    // As the game draws them (`--no-shared-poles` in eo-app): lamps share the trolley support poles.
    eo_props::overhead::set_lamp_standards(!args.iter().any(|a| a == "--no-shared-poles"));
    eo_props::overhead::set_joint_dedupe(!args.iter().any(|a| a == "--no-joint-dedupe"));
    eo_props::signs::set_merge_posts(!args.iter().any(|a| a == "--no-sign-post-merge"));
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = std::sync::Arc::new(eo_props::load_world(&paths).unwrap());
    let city = eo_data::City::load(&paths).unwrap();
    w.install_furniture(&city.buildings).unwrap();
    let ov = overhead::install(&w, overhead::DropBuilding::from_city(&city.buildings)).unwrap();
    drop(city);
    let keys = w.tile_keys();
    let parts: Vec<Vec<(i32, i32)>> = (0..threads).map(|t| keys.iter().skip(t).step_by(threads).copied().collect()).collect();
    let poles: Vec<Pole> = std::thread::scope(|s| {
        parts
            .iter()
            .map(|part| {
                let (w, ov) = (w.clone(), ov.clone());
                s.spawn(move || {
                    let mut v = vec![];
                    for &k in part {
                        for p in w.plan_tile(k).props {
                            if let Some(kind) = prop_kind(p.model) {
                                v.push(Pole { x: p.pos[0], z: p.pos[2], kind, id: String::new() });
                            }
                        }
                        let o = ov.plan_tile(&w, k);
                        for p in &o.poles {
                            if p.lamp_standard {
                                continue; // the lamp drawn on its spot is the pole
                            }
                            let kind = match p.kind {
                                PoleKind::Support => "overheadSupport",
                                PoleKind::Span => "overheadSpan",
                                PoleKind::Distribution => "distribution",
                            };
                            v.push(Pole { x: p.pos[0], z: p.pos[2], kind, id: p.id.clone() });
                        }
                        for sgn in &o.signs {
                            if !sgn.on_pole {
                                v.push(Pole { x: sgn.pos[0], z: sgn.pos[2], kind: "picketPost", id: String::new() });
                            }
                        }
                    }
                    v
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    // grid
    let cell = near.max(1.0);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    for (i, p) in poles.iter().enumerate() {
        grid.entry(((p.x / cell).floor() as i64, (p.z / cell).floor() as i64)).or_default().push(i);
    }
    let mut count = BTreeMap::<String, usize>::new();
    for p in &poles {
        *count.entry(p.kind.to_string()).or_default() += 1;
    }
    println!("poles by kind:");
    for (k, v) in &count {
        println!("  {k:<20} {v}");
    }
    // Each pole's nearest pole of its own kind (within 60 m).
    let mut nn = BTreeMap::<&str, Vec<f64>>::new();
    for p in poles.iter() {
        let (cx, cz) = ((p.x / cell).floor() as i64, (p.z / cell).floor() as i64);
        let r = (60.0 / cell).ceil() as i64;
        let mut best = f64::INFINITY;
        for dz in -r..=r {
            for dx in -r..=r {
                for &j in grid.get(&(cx + dx, cz + dz)).map(|v| v.as_slice()).unwrap_or(&[]) {
                    let q = &poles[j];
                    let d = (p.x - q.x).hypot(p.z - q.z);
                    if q.kind == p.kind && d > 0.0 && d < best {
                        best = d;
                    }
                }
            }
        }
        if best.is_finite() {
            nn.entry(p.kind).or_default().push(best);
        }
    }
    println!("\nnearest pole of the same kind (m): p1 / p5 / median");
    for (k, mut v) in nn {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |f: f64| v[((v.len() - 1) as f64 * f) as usize];
        println!("  {k:<20} {:>6.2} {:>6.2} {:>6.2}", q(0.01), q(0.05), q(0.5));
    }
    let mut pairs = BTreeMap::<String, Vec<(f64, usize, usize)>>::new();
    for (i, p) in poles.iter().enumerate() {
        let (cx, cz) = ((p.x / cell).floor() as i64, (p.z / cell).floor() as i64);
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(c) = grid.get(&(cx + dx, cz + dz)) {
                    for &j in c {
                        if j <= i {
                            continue;
                        }
                        let q = &poles[j];
                        let d = (p.x - q.x).hypot(p.z - q.z);
                        if d < near {
                            let (a, b) = if p.kind <= q.kind { (p.kind, q.kind) } else { (q.kind, p.kind) };
                            pairs.entry(format!("{a} + {b}")).or_default().push((d, i, j));
                        }
                    }
                }
            }
        }
    }
    println!("\npole pairs closer than {near} m (centre to centre), by kind pair:");
    let mut rows: Vec<_> = pairs.iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.len()));
    for (k, v) in &rows {
        let mut d: Vec<f64> = v.iter().map(|t| t.0).collect();
        d.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("  {k:<42} {:>6}   min {:.2}  median {:.2} m", v.len(), d[0], d[d.len() / 2]);
    }
    if let Some(path) = shots {
        // Matched street-level poses on the walk, 9 m back along the street from each close pair.
        let mut out = vec![];
        for (d, i, j) in pairs.get(&shot_pair).map(|v| v.as_slice()).unwrap_or(&[]) {
            let (p, q) = (&poles[*i], &poles[*j]);
            let (mx, mz) = ((p.x + q.x) / 2.0, (p.z + q.z) / 2.0);
            if !(mx > shot_box[0] && mz > shot_box[1] && mx < shot_box[2] && mz < shot_box[3]) {
                continue;
            }
            // The nearest ribbon: its station, the pair's lateral, its frame there.
            let mut best: Option<(f64, usize, f64, f64)> = None; // dist, entry, station, lateral
            for (ei, e) in w.street.plan.entries.iter().enumerate() {
                if e.points.len() < 2 {
                    continue;
                }
                if let Some((s, dist, lat)) = eo_props::project(e, mx, mz)
                    && best.is_none_or(|b| dist < b.0)
                {
                    best = Some((dist, ei, s, lat));
                }
            }
            let Some((_, ei, st, lat)) = best else { continue };
            let e = &w.street.plan.entries[ei];
            // From the roadway 6 m out from the pair and 8 m back along the street, looking at it.
            let f = eo_props::station_at(e, (st - 8.0).max(0.0), false);
            let el = lat - lat.signum() * 6.0;
            let (ex, ez) = (f.x + f.nx * el, f.z + f.nz * el);
            let name = format!("pair{}_{:.0}_{:.0}", out.len(), mx, mz).replace('-', "m");
            out.push(serde_json::json!({
                "name": name,
                "eye": [ex, w.ground(ex, ez) + 1.7, ez],
                "look_at": [mx, w.ground(mx, mz) + 3.0, mz],
                "fov": 60,
                "note": format!("{shot_pair} {d:.2} m apart"),
            }));
            if out.len() >= 6 {
                break;
            }
        }
        std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
        println!("\nwrote {} shots to {path}", out.len());
    }
    if list > 0 {
        for (k, v) in &rows {
            println!("\n{k}:");
            for (d, i, j) in v.iter().take(list) {
                let (p, q) = (&poles[*i], &poles[*j]);
                println!("  {d:.2} m  ({:.1}, {:.1}) {} {}  ({:.1}, {:.1}) {} {}", p.x, p.z, p.kind, p.id, q.x, q.z, q.kind, q.id);
            }
        }
    }
}
