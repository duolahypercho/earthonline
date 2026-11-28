// A digest of the street port's sources and data, baked into the crate as
// STREET_SOURCE_HASH: the kit tile cache names it, since a kit building's
// floor and skirt stand on the street surface this crate draws.
use std::hash::{Hash, Hasher};

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut files = vec![];
    walk(&root.join("src"), &mut files);
    files.push(root.join("data/street-city.json.gz"));
    files.push(root.join("data/street-options.json"));
    files.push(root.join("../eo-data/src/bridge_readings.rs"));
    files.push(root.join("../eo-data/src/bridge_survey.rs"));
    files.push(root.join("../../scripts/world-data/sources/california-street-standards.json"));
    files.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        f.strip_prefix(&root).unwrap_or(f).to_string_lossy().replace('\\', "/").hash(&mut h);
        std::fs::read(f).unwrap_or_default().hash(&mut h);
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rustc-env=STREET_SOURCE_HASH={:016x}", h.finish());
}
