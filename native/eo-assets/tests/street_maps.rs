//! The street surface's scanned sets decode to what the JS measured on them:
//! the linear mean colour, mean occlusion and mean roughness the JS records
//! beside each set (`STREET_SURFACE_PBR_SETS.*.meanLinear / meanAo /
//! meanRoughness`, exported to eo-street/data/street-material.json), which the
//! JS shading divides by. A wrong colour decode, a wrong transfer function or a
//! wrong NRA unpack moves these.

use eo_data::DataPaths;

#[test]
fn street_sets_decode_to_the_js_means() {
    let paths = DataPaths::discover().unwrap();
    let m: serde_json::Value = serde_json::from_str(include_str!("../../eo-street/data/street-material.json")).unwrap();
    for key in ["carriageway", "concrete"] {
        let s = &m["pbrSets"][key];
        let (set, res, _, _) = eo_assets::street_maps::load_set(
            &paths.repo_root,
            s["id"].as_str().unwrap(),
            s["provenance"].as_str().unwrap(),
            s["diffuse"].as_str().unwrap(),
            s["nra"].as_str().unwrap(),
        )
        .unwrap();
        let js: Vec<f64> = s["meanLinear"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let (ao, rough) = (s["meanAo"].as_f64().unwrap(), s["meanRoughness"].as_f64().unwrap());
        println!(
            "{key} {res}px: mean linear {:.4?} (JS {js:.4?}), ao {:.4} (JS {ao}), roughness {:.4} (JS {rough}), licence {} by {}",
            set.mean_linear, set.mean_ao, set.mean_roughness, set.license, set.author
        );
        for c in 0..3 {
            assert!((set.mean_linear[c] - js[c]).abs() < 0.002, "{key} mean linear channel {c}: {} vs JS {}", set.mean_linear[c], js[c]);
        }
        assert!((set.mean_ao - ao).abs() < 0.005, "{key} mean ao {} vs JS {ao}", set.mean_ao);
        assert!((set.mean_roughness - rough).abs() < 0.005, "{key} mean roughness {} vs JS {rough}", set.mean_roughness);
        assert_eq!(set.license, "CC0-1.0");
    }
}
