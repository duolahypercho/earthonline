//! the facade pass's per-building inputs against the node-run JS
//! (the earlier browser version's exporter -> data/facade-oracle.json).
//!
//!   cargo test --release -p eo-render --test facade -- --nocapture

use eo_render::facade;
use serde_json::Value;

/// Every sampled kit building: the appearance (year, source, maintenance,
/// wear, paint, grime seed, coastal), the sill rhythm and the packed
/// `archetypeAge` vec4 equal the JS's `createArchetypeBuilding` result and
/// `packAppearance` (exactly, as f32: every field is a quantised value).
#[test]
fn kit_appearance_matches_the_js() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    eo_kit::typology::pin(false); // ... and draws the port's one window module per archetype
    let root = eo_kit::repo_root().expect("repo root");
    let (_city, kc) = eo_kit::city::KitCity::load(&root).expect("city");
    let by_id: std::collections::HashMap<&str, usize> = kc.records.iter().enumerate().map(|(i, r)| (r.id.as_str(), i)).collect();
    let o = facade::oracle();
    let kit = o["kit"].as_array().expect("kit sample");
    let (mut n, mut eq_app, mut eq_sill, mut eq_pack) = (0, 0, 0, 0);
    let mut first = vec![];
    for e in kit {
        let id = e["id"].as_str().unwrap();
        let Some(&i) = by_id.get(id) else { continue };
        let rec = &kc.records[i];
        let Some(planned) = eo_kit::pipeline::plan_building(rec, &kc.world, i as u32, 0.0) else { continue };
        n += 1;
        let a = eo_kit::appearance::building_appearance(&rec.id, &planned.classified, rec.year_built);
        let ja = &e["appearance"];
        let app_ok = a.year_built == ja["yearBuilt"].as_f64().unwrap()
            && a.year_source == ja["yearSource"].as_str().unwrap()
            && a.maintenance == ja["maintenance"].as_f64().unwrap()
            && a.wear == ja["wear"].as_f64().unwrap()
            && a.paint == ja["paint"].as_f64().unwrap()
            && a.grime_seed as u64 == ja["grimeSeed"].as_u64().unwrap()
            && a.coastal == ja["coastal"].as_bool().unwrap();
        let (fs, pitch) = eo_kit::appearance::sill_rhythm(&planned.plan);
        let js_sill = &e["sillRhythm"];
        let sill_ok = (fs - js_sill["firstSillM"].as_f64().unwrap()).abs() < 1e-9 && (pitch - js_sill["pitchM"].as_f64().unwrap()).abs() < 1e-9;
        let packed = eo_kit::appearance::planned_appearance(rec, &planned, 0.0);
        let jp: Vec<f32> = e["pack"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let pack_ok = packed.iter().zip(&jp).all(|(a, b)| a == b);
        eq_app += app_ok as usize;
        eq_sill += sill_ok as usize;
        eq_pack += pack_ok as usize;
        if (!app_ok || !sill_ok || !pack_ok) && first.len() < 6 {
            first.push(format!("{id}: rust {a:?} sill ({fs}, {pitch}) pack {packed:?}; js {ja} sill {js_sill} pack {jp:?}"));
        }
    }
    eprintln!("kit appearance: {n} buildings, appearance equal {eq_app}, sill rhythm {eq_sill}, packed vec4 {eq_pack}");
    for f in &first {
        eprintln!("  {f}");
    }
    assert!(n >= 400, "compared {n}");
    assert_eq!(eq_app, n, "appearance equal to the JS");
    assert_eq!(eq_sill, n, "sill rhythm equal to the JS");
    assert_eq!(eq_pack, n, "packed archetypeAge equal to the JS");
}

/// Every Blender building: the weathering object blender/runtime.js builds
/// (at the oracle's test base height) and the scalars and structure the
/// shared weathering program reads for it equal the JS.
#[test]
fn authored_weathering_matches_the_js() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let root = eo_kit::repo_root().expect("repo root");
    let modules = eo_assets::manifest::load_blender_buildings(&root).expect("Blender modules");
    let o = facade::oracle();
    let base_y = o["testBaseY"].as_f64().unwrap();
    let (mut n, mut eq) = (0, 0);
    for e in o["authored"].as_array().unwrap() {
        let id = e["id"].as_str().unwrap();
        let m = modules.iter().find(|m| m.id == id).expect("module");
        n += 1;
        let w = facade::authored_weathering(&m.levels, &m.weathering, base_y, [m.frame.center.x, m.frame.center.z]);
        let ok = match (&w, e["scalars"].as_object()) {
            (None, None) => true,
            (Some(w), Some(js)) => {
                let (s, flags) = facade::weathering_scalars(w);
                let plan = e["plan"].as_str().unwrap_or("");
                let want = (plan.starts_with("wall") as u32)
                    | (plan.contains("+patches") as u32) << 1
                    | (plan.contains("+mottle") as u32) << 2
                    | (plan.contains("+base") as u32) << 3
                    | (plan.contains("+damp") as u32) << 4;
                let mut good = flags == want;
                for (k, v) in js {
                    let r = s.get(k).copied().unwrap_or(f64::NAN);
                    if (r - v.as_f64().unwrap()).abs() > 1e-9 {
                        if good {
                            eprintln!("  {id} {k}: rust {r} js {v}");
                        }
                        good = false;
                    }
                }
                good
            }
            _ => false,
        };
        eq += ok as usize;
    }
    eprintln!("authored weathering: {eq} of {n} Blender buildings equal (scalars and structure)");
    assert_eq!(n, 343);
    assert_eq!(eq, n);
}

/// The generated WGSL tables are the JS's (spot checks of rows the shader reads).
#[test]
fn tables_are_the_js() {
    eo_data::groundscale::pin(1.0); // the JS game this compares with is 1:1
    let s = facade::facade_constants_wgsl();
    let t = &facade::oracle()["tables"];
    assert_eq!(t["ageingWeathering"]["sootStrength"]["perUnit"].as_f64(), Some(0.32));
    assert!(s.contains("const AW_SOOT_STRENGTH: vec3<f32> = vec3<f32>(0.000000, 0.320000, 0.0);"), "{s}");
    assert!(s.contains("const AGE_BANDS: u32 = 6u;"));
    assert_eq!(facade::age_class("painted-stucco"), facade::AGE_RENDER);
    assert_eq!(facade::age_class("brick-running-bond"), facade::AGE_MASONRY);
    assert_eq!(facade::age_class("glass-shell"), facade::AGE_NONE);
    assert_eq!(facade::age_class("corrugated-metal"), facade::AGE_METAL);
    let _: &Value = t;
}
