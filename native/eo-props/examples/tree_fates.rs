//! What becomes of every DataSF list tree: placed, refused by the ribbon that
//! owns it (and why), or owned by no ribbon (and why the nearest refuses it).
//!
//!   cargo run --release -p eo-props --example tree_fates [ax az bx bz half]
//!
//! With a block (its centreline from (ax, az) to (bx, bz) in the city data's
//! metres, before the ground-plan scale, and a half width), each tree on it
//! is listed by its station along the block (`s`) and its offset (`n`,
//! positive to the south of an eastbound block). Switches: `EO_TREE_KEEP=1`,
//! `EO_TREE_SIZES=1`, `EO_TREE_SPECIES=1` plan as `--tree-keep`, `--tree-sizes`,
//! `--tree-species` do. `EO_TREE_CSV=path` writes the block's trees,
//! `EO_TREE_ALL=path` every list tree (game metres).
use std::collections::{BTreeMap, HashMap};

fn main() {
    let t0 = std::time::Instant::now();
    let on = |k: &str| std::env::var(k).is_ok_and(|v| v == "1");
    eo_props::furniture::set_tree_keep(on("EO_TREE_KEEP"));
    eo_props::furniture::set_tree_sizes(on("EO_TREE_SIZES"));
    eo_props::furniture::set_tree_species(on("EO_TREE_SPECIES"));
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = std::sync::Arc::new(eo_props::load_world(&paths).unwrap());
    let f = w.furniture().clone();
    let gs = eo_data::groundscale::scale();
    eprintln!("world {:.1} s, ground-plan scale {gs}, tree keep {} sizes {} species {}", t0.elapsed().as_secs_f64(), f.tree_keep, f.tree_sizes, f.tree_species);
    if let Some(j) = &f.tree_join {
        let n = j.record.len();
        println!("join: {} of {n} list trees ({:.1}%) to a DataSF record within {} m; {} ({:.1}%) sized from their DBH", j.joined, j.joined as f64 * 100.0 / n as f64, eo_props::tree_survey::JOIN_M, j.sized, j.sized as f64 * 100.0 / n as f64);
    }
    let species_ids = eo_props::eztree::street_tree_species().map(|s| s.to_vec()).unwrap_or_default();
    // each street form's near mesh: its leaves' mean width across x and z, and its top (model metres).
    let extents: Vec<(f64, f64)> = (0..species_ids.len())
        .map(|v| {
            let Ok(m) = eo_props::eztree::build_street_tree_geometry(v, false, false, eo_props::eztree::StreetTreeMode::default()) else { return (0.0, 0.0) };
            let (mut lo, mut hi, mut top) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2], 0.0f64);
            for (q, &part) in m.mesh.positions.iter().zip(&m.mesh.part) {
                top = top.max(q[1] as f64);
                if part != 0 {
                    lo = [lo[0].min(q[0] as f64), lo[1].min(q[2] as f64)];
                    hi = [hi[0].max(q[0] as f64), hi[1].max(q[2] as f64)];
                }
            }
            (0.5 * ((hi[0] - lo[0]) + (hi[1] - lo[1])), top)
        })
        .collect();
    if std::env::var("EO_TREE_EXTENTS").is_ok() {
        for (v, id) in species_ids.iter().enumerate() {
            println!("form {id:<20} leaves {:.1} m across, {:.1} m tall at scale 1", extents[v].0, extents[v].1);
        }
    }
    // a placed tree's drawn crown and height (m): its street form's mesh extents times its scales.
    let drawn = |v: u32, sc: [f32; 2]| -> (String, f64, f64) {
        let id = species_ids.get(v as usize).cloned().unwrap_or_default();
        let (w, h) = extents.get(v as usize).copied().unwrap_or((0.0, 0.0));
        (id, w * sc[0] as f64, h * sc[1] as f64)
    };
    let n_entries = w.street.plan.entries.len();
    let threads = 6;
    let chunk = n_entries.div_ceil(threads);
    let fates: Vec<(u32, u32, eo_props::furniture::TreeFate)> = std::thread::scope(|s| {
        (0..threads)
            .map(|k| {
                let f = f.clone();
                let w = w.clone();
                s.spawn(move || {
                    let mut out = vec![];
                    for ei in k * chunk..((k + 1) * chunk).min(n_entries) {
                        for t in &f.entry(&w, ei).tree_fates {
                            out.push((t.id, ei as u32, *t));
                        }
                    }
                    out
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    let t = &w.js.trees;
    let n = t.data.len() / t.stride;
    let mut fate: HashMap<u32, (u32, &'static str, [f64; 2])> = HashMap::new();
    let mut look: HashMap<u32, (u32, [f32; 2])> = HashMap::new();
    for &(id, ei, t) in &fates {
        let e = fate.entry(id).or_insert((ei, t.why, t.at));
        if t.why == "placed" {
            *e = (ei, t.why, t.at);
            look.insert(id, (t.variant, t.scale));
        }
    }
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    let mut placed = 0usize;
    let mut reason_of: HashMap<u32, String> = HashMap::new();
    for id in 0..n as u32 {
        let r = match fate.get(&id) {
            Some((_, "placed", _)) => {
                placed += 1;
                "placed".to_string()
            }
            Some((_, why, _)) => format!("owned, refused: {why}"),
            None => match f.tree_owner(&w, id) {
                // owned by a ribbon whose walk never offered it (a side with no band, a short ribbon).
                Some(ei) => {
                    if std::env::var("EO_TREE_DEBUG").is_ok() && by.get("owned, not offered").copied().unwrap_or(0) < 8 {
                        let r = eo_props::trees::record(&w, id);
                        let e = &w.street.plan.entries[ei as usize];
                        eprintln!(
                            "not offered: id {id} owner {ei} {} half {:.1} walks {:?} trims {:.2} {:.2} len {:.1} proj {:?} take {:?}",
                            w.segment_of(e).street_name,
                            e.half,
                            e.walks,
                            e.trim_start,
                            e.trim_end,
                            e.length,
                            eo_props::project(e, r.x, r.z),
                            f.tree_take_why(&w, ei as usize, id)
                        );
                    }
                    "owned, not offered".to_string()
                }
                None => format!("unowned: {}", f.tree_unowned_reason(&w, id).0),
            },
        };
        *by.entry(r.clone()).or_default() += 1;
        reason_of.insert(id, r);
    }
    // (`EO_TREE_ALL=path`) every list tree: its fate, list point and placed foot (game metres).
    if let Ok(path) = std::env::var("EO_TREE_ALL") {
        let mut csv = String::from("id,list_x,list_z,placed_x,placed_z,fate\n");
        for id in 0..n as u32 {
            let r = eo_props::trees::record(&w, id);
            let at = fate.get(&id).filter(|f| f.1 == "placed").map(|f| f.2);
            csv.push_str(&format!(
                "{id},{:.2},{:.2},{},{},{}\n",
                r.x,
                r.z,
                at.map(|a| format!("{:.2}", a[0])).unwrap_or_default(),
                at.map(|a| format!("{:.2}", a[1])).unwrap_or_default(),
                reason_of[&id]
            ));
        }
        std::fs::write(&path, csv).expect("EO_TREE_ALL");
    }
    println!("{n} list trees; placed {placed} ({:.1}%), not placed {} ({:.1}%)", placed as f64 * 100.0 / n as f64, n - placed, (n - placed) as f64 * 100.0 / n as f64);
    for (k, v) in &by {
        println!("  {k:<44} {v:>7}");
    }
    let a: Vec<f64> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    if a.len() >= 5 {
        let (ax, az, bx, bz, half) = (a[0], a[1], a[2], a[3], a[4]);
        let len = (bx - ax).hypot(bz - az);
        let (ux, uz) = ((bx - ax) / len, (bz - az) / len);
        let mut rows = vec![];
        for id in 0..n as u32 {
            let r = eo_props::trees::record(&w, id);
            let (dx, dz) = (r.x / gs - ax, r.z / gs - az);
            let s = dx * ux + dz * uz;
            let nn = -dx * uz + dz * ux;
            if s < -5.0 || s > len + 5.0 || nn.abs() > half {
                continue;
            }
            rows.push((s, nn, id, r));
        }
        rows.sort_by(|a, b| a.1.signum().total_cmp(&b.1.signum()).then(a.0.total_cmp(&b.0)));
        // (`EO_TREE_CSV=path`) the block's list trees for the capture overlays: game x, z of
        // the list point and of the placed foot (empty when not placed), and the fate.
        if let Ok(path) = std::env::var("EO_TREE_CSV") {
            let mut csv = String::from("id,s,n,list_x,list_z,placed_x,placed_z,form,crown_m,model,drawn_crown_m,drawn_height_m,fate\n");
            for (s, nn, id, r) in &rows {
                let at = fate.get(id).filter(|f| f.1 == "placed").map(|f| f.2);
                let d = look.get(id).map(|&(v, sc)| drawn(v, sc));
                csv.push_str(&format!(
                    "{id},{s:.1},{nn:.1},{:.2},{:.2},{},{},{},{},{},{},{},\"{}\"\n",
                    r.x,
                    r.z,
                    at.map(|a| format!("{:.2}", a[0])).unwrap_or_default(),
                    at.map(|a| format!("{:.2}", a[1])).unwrap_or_default(),
                    r.form,
                    r.crown_m.map(|c| format!("{c:.1}")).unwrap_or_default(),
                    d.as_ref().map(|d| d.0.clone()).unwrap_or_default(),
                    d.as_ref().map(|d| format!("{:.1}", d.1)).unwrap_or_default(),
                    d.as_ref().map(|d| format!("{:.1}", d.2)).unwrap_or_default(),
                    reason_of[id]
                ));
            }
            std::fs::write(&path, csv).expect("EO_TREE_CSV");
        }
        println!("block ({ax}, {az}) -> ({bx}, {bz}), {len:.1} m: {} list trees", rows.len());
        for (s, nn, id, r) in rows {
            let owner = f.tree_owner(&w, id).or_else(|| f.tree_unowned_reason(&w, id).1);
            let desc = owner
                .map(|ei| {
                    let e = &w.street.plan.entries[ei as usize];
                    let seg = w.segment_of(e);
                    let (st, _, lat) = eo_props::project(e, r.x, r.z).unwrap_or((0.0, 0.0, 0.0));
                    format!(
                        "ribbon {ei} {} {} half {:.1} walks [{:.1},{:.1}] span [{:.1},{:.1}] of {:.1}; at s {st:.1} lat {lat:.1}",
                        seg.street_name,
                        e.highway,
                        e.half,
                        e.walks[0],
                        e.walks[1],
                        e.trim_start + 1.2,
                        e.length - e.trim_end - 1.2,
                        e.length
                    )
                })
                .unwrap_or_default();
            if std::env::var("EO_TREE_WHY").is_ok_and(|v| v.split(',').any(|x| x == id.to_string())) {
                let key = ((r.x / 32.0).floor() as i32, (r.z / 32.0).floor() as i32);
                for &ei in eo_props::signs::entry_cells(&w).get(&key).map(|v| v.as_slice()).unwrap_or(&[]) {
                    let e = &w.street.plan.entries[ei as usize];
                    let (st, d, lat) = eo_props::project(e, r.x, r.z).unwrap_or((0.0, 0.0, 0.0));
                    println!(
                        "      ribbon {ei} {} {} half {:.1} walks [{:.1},{:.1}] span [{:.1},{:.1}] of {:.1}: s {st:.1} d {d:.1} lat {lat:.1} -> {:?}",
                        w.segment_of(e).street_name,
                        e.highway,
                        e.half,
                        e.walks[0],
                        e.walks[1],
                        e.trim_start + 1.2,
                        e.length - e.trim_end - 1.2,
                        e.length,
                        f.tree_take_why(&w, ei as usize, id)
                    );
                }
            }
            let d = look.get(&id).map(|&(v, sc)| drawn(v, sc));
            let rec = f.tree_join.as_ref().and_then(|j| j.record[id as usize].map(|k| (j, k as usize))).map(|(j, k)| {
                let t = &j.table.trees[k];
                format!(
                    "DataSF {} {} dbh {} planted {}",
                    t.treeid,
                    j.table.species[t.species as usize].split("::").next().unwrap_or("").trim(),
                    t.dbh_in.map(|v| format!("{v}")).unwrap_or("-".into()),
                    t.planted.map(|v| v.to_string()).unwrap_or("-".into())
                )
            });
            println!(
                "  s {s:6.1} n {nn:6.1} id {id:6} {:<7} crown {:>4} h {:>4}  {:<40} drawn {}  {} | {desc}",
                r.form,
                r.crown_m.map(|c| format!("{c:.1}")).unwrap_or("-".into()),
                r.height_m.map(|c| format!("{c:.1}")).unwrap_or("-".into()),
                reason_of[&id],
                d.map(|d| format!("{} crown {:.1} h {:.1}", d.0, d.1, d.2)).unwrap_or("-".into()),
                rec.unwrap_or_default()
            );
        }
    }
}
