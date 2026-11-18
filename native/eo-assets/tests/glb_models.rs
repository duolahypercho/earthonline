//! GLB digests, the refusal of a corrupted GLB, and the cooked model against
//! its own header counts and its cache round trip.

use eo_assets::cache::{self, LoadSource};
use eo_assets::glb::{self, cook_glb, read_verified, verify_bytes};
use eo_assets::manifest::{AssetRef, load_blender_buildings};
use std::path::PathBuf;

fn repo() -> PathBuf {
    eo_data::DataPaths::discover().unwrap().repo_root
}

#[test]
fn every_exterior_glb_matches_its_manifest_digest() {
    let repo = repo();
    let list = load_blender_buildings(&repo).unwrap();
    let modules = std::fs::read_dir(repo.join(eo_assets::manifest::GENERATED_DIR))
        .unwrap()
        .filter(|e| {
            let n = e.as_ref().unwrap().file_name().into_string().unwrap();
            n.starts_with('b') && n.ends_with(".js")
        })
        .count();
    assert_eq!(list.len(), modules, "the index lists every generated module");
    assert_eq!(list.len(), 343, "343 Blender buildings at this revision");
    let t = std::time::Instant::now();
    let mut bytes = 0u64;
    for b in &list {
        let a = &b.assets.exterior;
        let data = read_verified(&a.path(&repo), a).unwrap_or_else(|e| panic!("{}: {e:#}", b.id));
        bytes += data.len() as u64;
    }
    eprintln!("{} exteriors, {:.1} MB verified in {:.0} ms", list.len(), bytes as f64 / 1e6, t.elapsed().as_secs_f64() * 1e3);
}

#[test]
fn a_planted_corrupt_glb_is_refused() {
    let repo = repo();
    let list = load_blender_buildings(&repo).unwrap();
    let asset = &list[0].assets.exterior;
    let good = std::fs::read(asset.path(&repo)).unwrap();
    verify_bytes(&good, asset).unwrap();

    // One flipped byte in the binary chunk: same length, different digest.
    let mut bad = good.clone();
    let k = bad.len() - 100;
    bad[k] ^= 0x5a;
    let e = verify_bytes(&bad, asset).unwrap_err().to_string();
    assert!(e.contains("SHA-256 mismatch"), "{e}");
    // Truncated.
    let e = verify_bytes(&good[..good.len() - 4], asset).unwrap_err().to_string();
    assert!(e.contains("byte length mismatch"), "{e}");

    // Through the loader: a repo root whose GLB is the corrupted copy. The
    // load fails and nothing is written to the cache for that digest.
    let tmp = std::env::temp_dir().join(format!("eo-native-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let p = asset.path(&tmp);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, &bad).unwrap();
    let cache_dir = tmp.join("cache");
    let r = cache::load_model(&tmp, Some(&cache_dir), asset);
    assert!(r.is_err(), "the loader drew a corrupted GLB");
    assert!(!cache::model_path(&cache_dir, &asset.sha256).exists(), "a corrupted GLB reached the cache");
    // And the same GLB with a manifest that lies about the digest is refused too.
    let lying = AssetRef { sha256: "0".repeat(64), ..asset.clone() };
    std::fs::write(&p, &good).unwrap();
    assert!(cache::load_model(&tmp, Some(&cache_dir), &lying).is_err());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn cooked_models_round_trip_the_cache() {
    let repo = repo();
    let list = load_blender_buildings(&repo).unwrap();
    let step = (list.len() / 10).max(1);
    let tmp = std::env::temp_dir().join(format!("eo-native-cook-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    for b in list.iter().step_by(step) {
        let asset: AssetRef = b.assets.exterior.clone();
        let bytes = read_verified(&asset.path(&repo), &asset).unwrap();
        let m = cook_glb(&bytes).unwrap();
        assert!(!m.positions.is_empty() && !m.indices.is_empty() && !m.parts.is_empty(), "{} cooked empty", b.id);
        assert!((0..3).all(|k| m.min[k] <= m.max[k]), "{} bounds", b.id);
        // The header peek (used to reserve GPU space) agrees with the cook.
        let c = glb::peek_counts(&asset.path(&repo)).unwrap();
        assert_eq!((c.vertices, c.indices), (m.positions.len() as u32, m.indices.len() as u32), "{} peek", b.id);
        // Cache round trip: cooked bytes come back equal, and a flipped cache
        // byte is refused (the loader then re-cooks from the verified GLB).
        let (first, src) = cache::load_model(&repo, Some(&tmp), &asset).unwrap();
        assert_eq!(src, LoadSource::Cooked);
        let (second, src) = cache::load_model(&repo, Some(&tmp), &asset).unwrap();
        assert_eq!(src, LoadSource::Cache);
        assert_eq!(first, second, "{} cache round trip", b.id);
        assert_eq!(second, m);
        let p = cache::model_path(&tmp, &asset.sha256);
        let mut b = std::fs::read(&p).unwrap();
        let k = b.len() - 7;
        b[k] ^= 1;
        std::fs::write(&p, &b).unwrap();
        assert!(cache::read(&p, &asset.sha256).is_err(), "altered cache file accepted");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
