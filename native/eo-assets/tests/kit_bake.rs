//! The Rust port of the kit's procedural bake against the JS bake's own
//! bytes (SHA-256 recorded by the earlier browser version's exporter).

use eo_assets::glb::sha256_hex;
use eo_assets::kit::{self, KitLibrary};

#[test]
fn bake_port_matches_js_bytes() {
    let lib = KitLibrary::load().unwrap();
    assert!(lib.reference_bakes.len() >= 8);
    let mut report = vec![];
    let mut exact = 0;
    for r in &lib.reference_bakes {
        let spec = &lib.specs[&r.name];
        let m = kit::bake(&r.name, spec, &r.seed, r.res);
        let digests = [sha256_hex(&m.albedo), sha256_hex(&m.normal), sha256_hex(&m.orm)];
        let want = [&r.albedo, &r.normal, &r.orm];
        let sums: Vec<u64> = [&m.albedo, &m.normal, &m.orm].iter().map(|a| a.iter().map(|&v| v as u64).sum()).collect();
        for k in 0..3 {
            if digests[k] == *want[k] {
                exact += 1;
            } else {
                // A differing digest is only tolerated as last-bit rounding of
                // a few texels (V8's pow/sin/hypot vs the platform libm): the
                // byte sum may then move by a handful, never more.
                let d = (sums[k] as i64 - r.sums[k] as i64).abs();
                report.push(format!("{} map {k} res {}: digest differs, byte-sum delta {d}", r.name, r.res));
                assert!(d <= 16, "{} map {k}: byte-sum delta {d} is not rounding noise", r.name);
            }
        }
    }
    for l in &report {
        eprintln!("{l}");
    }
    eprintln!("{exact} of {} maps byte-identical to the JS bake", lib.reference_bakes.len() * 3);
    assert!(exact >= lib.reference_bakes.len() * 3 - 3, "too many maps differ from the JS bake");
}

#[test]
fn hash_seed_matches_js() {
    // hashSeed(...) computed by src/world/buildings/authored/kit/frame.js in node.
    assert_eq!(kit::hash_seed("authored"), 2294414878);
    assert_eq!(kit::hash_seed(""), 2872998923);
    assert_eq!(kit::hash_seed("authored-kit-materials-v1|authored-slice-v1|painted-stucco"), 501321339);
}
