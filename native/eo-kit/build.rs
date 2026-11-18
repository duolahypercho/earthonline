// A digest of the planner's sources and data, baked into the crate as
// KIT_SOURCE_HASH so a cooked-tile cache from another build is never read.
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
    files.push(root.join("data/kit-exclusions.json"));
    files.push(root.join("data/district-era-prior.data.js"));
    files.push(root.join("../eo-assets/data/kit-material-specs.json"));
    files.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        f.strip_prefix(&root).unwrap_or(f).to_string_lossy().replace('\\', "/").hash(&mut h);
        std::fs::read(f).unwrap_or_default().hash(&mut h);
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rustc-env=KIT_SOURCE_HASH={:016x}", h.finish());
}
