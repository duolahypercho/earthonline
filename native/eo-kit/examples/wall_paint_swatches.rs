//! swatch sheets of the wall paint, port against `wall_paint`, per
//! archetype, from every Nth planned kit building (the drawn wall colour:
//! the finish's generator colour x the port's tint, or the primer x the new
//! tint; the bake's grooves take a further 3-7 % off both).
//!
//!   cargo run --release -p eo-kit --example wall_paint_swatches -- <out.json> [step]
//!
//! Writes JSON: per archetype the sRGB walls before and after, and summary
//! statistics (mean sRGB, red minus blue, OKLab chroma and lightness spread).

use eo_data::roof_colours as rc;
use eo_kit::city::KitCity;
use eo_kit::pipeline::{base_material_name, drawn_materials, plan_building};
use eo_kit::wall_paint;
use std::collections::BTreeMap;

fn hex_linear(h: u32) -> [f32; 3] {
    rc::rgb8_linear([(h >> 16) as u8, (h >> 8) as u8, h as u8])
}

fn srgb8(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (rc::linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8)
}

fn wall_colour(p: &eo_kit::pipeline::Planned) -> Option<[f32; 3]> {
    let d = drawn_materials(p, 2);
    let wall = d.materials.get("wall")?;
    let base = wall_paint::drawn_base(wall).or_else(|| eo_kit::roof_measured::material_colour(wall))?;
    let t = d.tints.get(wall).or_else(|| d.tints.get(base_material_name(wall))).copied().map(hex_linear).unwrap_or([1.0; 3]);
    Some([base[0] * t[0], base[1] * t[1], base[2] * t[2]])
}

fn stats(list: &[[u8; 3]]) -> serde_json::Value {
    let n = list.len().max(1) as f64;
    let lin: Vec<[f32; 3]> = list.iter().map(|c| rc::rgb8_linear(*c)).collect();
    let mean = [0, 1, 2].map(|k| lin.iter().map(|c| c[k] as f64).sum::<f64>() / n);
    let m8 = srgb8(mean.map(|v| v as f32));
    let labs: Vec<[f32; 3]> = lin.iter().map(|c| rc::oklab(*c)).collect();
    let chroma = labs.iter().map(|l| ((l[1] * l[1] + l[2] * l[2]) as f64).sqrt()).sum::<f64>() / n;
    let lm = labs.iter().map(|l| l[0] as f64).sum::<f64>() / n;
    let lsd = (labs.iter().map(|l| (l[0] as f64 - lm).powi(2)).sum::<f64>() / n).sqrt();
    let rb = list.iter().map(|c| c[0] as f64 - c[2] as f64).sum::<f64>() / n;
    let warm = list.iter().filter(|c| c[0] as i32 - c[2] as i32 > 12).count() as f64 / n;
    serde_json::json!({ "n": list.len(), "mean_srgb": m8, "r_minus_b": rb, "share_warm_over_12": warm, "oklab_chroma_mean": chroma, "oklab_l_mean": lm, "oklab_l_sd": lsd })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "wall-paint-swatches.json".into());
    let step: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(97);
    eo_data::roof_colours::install(&eo_kit::repo_root().expect("repo root"));
    let root = eo_kit::repo_root().expect("repo root");
    let (_city, kc) = KitCity::load(&root).expect("city");
    let mut by: BTreeMap<String, (Vec<[u8; 3]>, Vec<[u8; 3]>)> = BTreeMap::new();
    for i in (0..kc.records.len()).step_by(step) {
        if kc.excluded.contains(&i) {
            continue;
        }
        let Some(p) = plan_building(&kc.records[i], &kc.world, i as u32, 0.0) else { continue };
        wall_paint::pin(false);
        let before = wall_colour(&p);
        wall_paint::pin(true);
        let after = wall_colour(&p);
        if let (Some(b), Some(a)) = (before, after) {
            let e = by.entry(p.classified.archetype.to_string()).or_default();
            e.0.push(srgb8(b));
            e.1.push(srgb8(a));
        }
    }
    let mut all = (vec![], vec![]);
    let mut types = serde_json::Map::new();
    for (k, (b, a)) in &by {
        all.0.extend(b.iter().cloned());
        all.1.extend(a.iter().cloned());
        types.insert(k.clone(), serde_json::json!({ "before": stats(b), "after": stats(a), "before_srgb": b, "after_srgb": a }));
    }
    let doc = serde_json::json!({ "step": step, "all": { "before": stats(&all.0), "after": stats(&all.1) }, "types": types });
    std::fs::write(&out, serde_json::to_string(&doc).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&doc["all"]).unwrap());
    for (k, v) in doc["types"].as_object().unwrap() {
        println!("{k}: before {} after {}", v["before"], v["after"]);
    }
}
