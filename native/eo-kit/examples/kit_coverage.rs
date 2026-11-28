//! How much of each footprint the kit's drawn building covers (a diagnostic).
//!
//!   cargo run --release -p eo-kit --example kit_coverage -- <lat> <lon> <half m> <out dir>
//!
//! For every footprint wholly inside the square (on the drawn plan, at the
//! process's ground scale: `EO_GROUND_SCALE` / the approved 1.25 by default),
//! the kit plans the building as the game does (`pipeline::plan_building`),
//! builds the near rung (0) and the mid rung (2), and lays each rung's
//! triangles flat on a 0.25 m grid: the cells they cover inside the footprint
//! and outside it. Writes `kit-coverage.png` (footprint outlines white, drawn
//! cover orange, footprint the rung leaves open red, hand-built records blue)
//! and `kit-coverage.json` (per record: id, house type, footprint and frame
//! areas, cover at each rung), and prints the totals.

use eo_kit::frame::P2;
use std::path::PathBuf;

const CELL: f64 = 0.25;

fn in_ring(x: f64, z: f64, ring: &[P2]) -> bool {
    let mut inside = false;
    let n = ring.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a.z > z) != (b.z > z) && x < (b.x - a.x) * (z - a.z) / (b.z - a.z) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn area(ring: &[P2]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| ring[i].x * ring[(i + 1) % n].z - ring[(i + 1) % n].x * ring[i].z).sum::<f64>().abs() / 2.0
}

/// Mark every cell whose centre a triangle (laid flat) covers.
fn raster_tri(mark: &mut [bool], n: usize, x0: f64, z0: f64, t: [[f64; 2]; 3]) {
    let (mut lx, mut lz, mut hx, mut hz) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &t {
        lx = lx.min(p[0]);
        lz = lz.min(p[1]);
        hx = hx.max(p[0]);
        hz = hz.max(p[1]);
    }
    let d = (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1]);
    if d.abs() < 1e-6 {
        return; // a wall: nothing laid flat
    }
    let i0 = (((lx - x0) / CELL).floor().max(0.0)) as usize;
    let j0 = (((lz - z0) / CELL).floor().max(0.0)) as usize;
    let i1 = (((hx - x0) / CELL).ceil().max(0.0) as usize).min(n);
    let j1 = (((hz - z0) / CELL).ceil().max(0.0) as usize).min(n);
    for j in j0..j1 {
        for i in i0..i1 {
            let (px, pz) = (x0 + (i as f64 + 0.5) * CELL, z0 + (j as f64 + 0.5) * CELL);
            let w0 = (t[1][0] - px) * (t[2][1] - pz) - (t[2][0] - px) * (t[1][1] - pz);
            let w1 = (t[2][0] - px) * (t[0][1] - pz) - (t[0][0] - px) * (t[2][1] - pz);
            let w2 = (t[0][0] - px) * (t[1][1] - pz) - (t[1][0] - px) * (t[0][1] - pz);
            if (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0) {
                mark[j * n + i] = true;
            }
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (lat, lon, half): (f64, f64, f64) = (a[0].parse().unwrap(), a[1].parse().unwrap(), a[2].parse().unwrap());
    let out = PathBuf::from(&a[3]);
    std::fs::create_dir_all(&out).unwrap();
    let root = eo_kit::repo_root().expect("repo root");
    let (city, kc) = eo_kit::city::KitCity::load(&root).expect("kit city");
    let pf = eo_data::roof_colours::PlanFrame::from_meta(&city.meta);
    let gs = eo_data::groundscale::scale();
    let (cx, cz) = pf.to_plan(lon, lat);
    let [cx, cz] = eo_data::groundscale::plan([cx, cz]);
    let half = half * gs;
    let n = (2.0 * half / CELL) as usize;
    let (x0, z0) = (cx - half, cz - half);
    let mut fp = vec![0u32; n * n]; // footprint owner + 1
    let mut cover0 = vec![false; n * n];
    let mut cover2 = vec![false; n * n];
    let mut handbuilt = vec![false; n * n];
    let mut rows = Vec::new();
    let (mut planned_n, mut unplanned, mut hand) = (0, 0, 0);
    let mut unplanned_types: std::collections::BTreeMap<String, usize> = Default::default();
    for (i, rec) in kc.records.iter().enumerate() {
        let poly = &rec.polygon;
        if poly.is_empty() || !poly.iter().all(|p| (p.x - cx).abs() < half - 1.0 && (p.z - cz).abs() < half - 1.0) {
            continue;
        }
        let (mut lx, mut lz, mut hx, mut hz) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in poly {
            lx = lx.min(p.x);
            lz = lz.min(p.z);
            hx = hx.max(p.x);
            hz = hz.max(p.z);
        }
        let mut own = Vec::new();
        for j in ((lz - z0) / CELL) as usize..=(((hz - z0) / CELL) as usize).min(n - 1) {
            for ii in ((lx - x0) / CELL) as usize..=(((hx - x0) / CELL) as usize).min(n - 1) {
                let (px, pz) = (x0 + (ii as f64 + 0.5) * CELL, z0 + (j as f64 + 0.5) * CELL);
                if in_ring(px, pz, poly) {
                    fp[j * n + ii] = i as u32 + 1;
                    own.push(j * n + ii);
                }
            }
        }
        let ty = eo_kit::pipeline::classify_loaded(rec).map(|c| c.archetype.to_string()).unwrap_or_else(|| "unclassified".into());
        if kc.excluded.contains(&i) {
            hand += 1;
            for &k in &own {
                handbuilt[k] = true;
            }
            rows.push(serde_json::json!({ "id": rec.id, "type": "hand-built", "footprint_m2": area(poly) }));
            continue;
        }
        let Some(planned) = eo_kit::pipeline::plan_building(rec, &kc.world, i as u32, 0.0) else {
            unplanned += 1;
            *unplanned_types.entry(ty.clone()).or_default() += 1;
            rows.push(serde_json::json!({ "id": rec.id, "type": ty, "footprint_m2": area(poly), "planned": false }));
            continue;
        };
        planned_n += 1;
        let mut mine0 = vec![false; n * n];
        let mut mine2 = vec![false; n * n];
        for (rung, mark) in [(0u32, &mut mine0), (2u32, &mut mine2)] {
            let mesh = eo_kit::pipeline::build_rung(&planned, rung);
            for g in &mesh.groups {
                let s = &g.2;
                for t in s.indices.chunks_exact(3) {
                    let p = |k: u32| [s.positions[k as usize][0], s.positions[k as usize][2]];
                    raster_tri(mark, n, x0, z0, [p(t[0]), p(t[1]), p(t[2])]);
                }
            }
        }
        let fa = own.len() as f64 * CELL * CELL;
        let in0 = own.iter().filter(|&&k| mine0[k]).count() as f64 * CELL * CELL;
        let in2 = own.iter().filter(|&&k| mine2[k]).count() as f64 * CELL * CELL;
        for k in 0..n * n {
            cover0[k] |= mine0[k];
            cover2[k] |= mine2[k];
        }
        let f = &planned.frame;
        rows.push(serde_json::json!({
            "id": rec.id, "type": ty, "footprint_m2": (fa * 10.0).round() / 10.0,
            "frame_w_m": (f.width * 10.0).round() / 10.0, "frame_d_m": (f.depth * 10.0).round() / 10.0,
            "frame_area_m2": (f.area * 10.0).round() / 10.0, "front_source": f.front_source,
            "cover_rung0": (in0 / fa.max(1e-9) * 1000.0).round() / 1000.0, "cover_rung2": (in2 / fa.max(1e-9) * 1000.0).round() / 1000.0,
            "vertices": f.polygon.len(), "source_vertices": rec.polygon.len(), "height_m": rec.height,
            "ring": rec.polygon.iter().map(|p| [(p.x * 100.0).round() / 100.0, (p.z * 100.0).round() / 100.0]).collect::<Vec<_>>(),
        }));
    }
    let total_fp = fp.iter().filter(|&&o| o > 0).count() as f64 * CELL * CELL;
    let kit_fp = (0..n * n).filter(|&k| fp[k] > 0 && !handbuilt[k]).count() as f64 * CELL * CELL;
    let c0 = (0..n * n).filter(|&k| fp[k] > 0 && !handbuilt[k] && cover0[k]).count() as f64 * CELL * CELL;
    let c2 = (0..n * n).filter(|&k| fp[k] > 0 && !handbuilt[k] && cover2[k]).count() as f64 * CELL * CELL;
    let out0 = (0..n * n).filter(|&k| fp[k] == 0 && cover0[k]).count() as f64 * CELL * CELL;
    println!("ground scale x{gs}; square {:.0} m on the drawn plan", 2.0 * half);
    println!("{} footprints inside: {planned_n} planned by the kit, {unplanned} the kit does not plan {:?}, {hand} hand-built", planned_n + unplanned + hand, unplanned_types);
    println!(
        "footprint area {total_fp:.0} m^2 ({kit_fp:.0} m^2 kit); the kit's near rung covers {c0:.0} m^2 of it ({:.1} %), its mid rung {c2:.0} m^2 ({:.1} %); near rung outside the footprints {out0:.0} m^2",
        100.0 * c0 / kit_fp.max(1.0),
        100.0 * c2 / kit_fp.max(1.0)
    );
    // per-type cover
    let mut by: std::collections::BTreeMap<String, (f64, f64, usize)> = Default::default();
    for r in &rows {
        if let (Some(t), Some(fa), Some(c)) = (r["type"].as_str(), r["footprint_m2"].as_f64(), r["cover_rung0"].as_f64()) {
            let e = by.entry(t.to_string()).or_default();
            e.0 += fa;
            e.1 += fa * c;
            e.2 += 1;
        }
    }
    for (t, (fa, cv, k)) in &by {
        println!("  {t:<22} {k:>3} buildings, footprint {fa:>7.0} m^2, near rung covers {:.1} %", 100.0 * cv / fa.max(1.0));
    }
    std::fs::write(out.join("kit-coverage.json"), serde_json::to_string_pretty(&rows).unwrap()).unwrap();
    // picture: north up
    let mut img = vec![40u8; n * n * 3];
    for k in 0..n * n {
        let c: [u8; 3] = if handbuilt[k] {
            [70, 110, 200]
        } else if fp[k] > 0 && cover0[k] {
            [230, 150, 60]
        } else if fp[k] > 0 {
            [210, 40, 40]
        } else if cover0[k] {
            [150, 110, 60]
        } else {
            [40, 40, 40]
        };
        img[k * 3..k * 3 + 3].copy_from_slice(&c);
    }
    for k in 0..n * n {
        let (i, j) = (k % n, k / n);
        if fp[k] > 0 && [(1i64, 0i64), (0, 1), (-1, 0), (0, -1)].iter().any(|(di, dj)| {
            let (a, b) = (i as i64 + di, j as i64 + dj);
            a < 0 || b < 0 || a >= n as i64 || b >= n as i64 || fp[b as usize * n + a as usize] != fp[k]
        }) {
            img[k * 3..k * 3 + 3].copy_from_slice(&[255, 255, 255]);
        }
    }
    let f = std::fs::File::create(out.join("kit-coverage.png")).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), n as u32, n as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&img).unwrap();
    println!("wrote {}", out.join("kit-coverage.png").display());
}
