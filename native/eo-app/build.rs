//! The commit the binary is built from, for the capture manifests
//! (`EO_BUILD_COMMIT`, "unknown" without git). Rebuilt when HEAD moves.

use std::path::PathBuf;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=EO_BUILD_COMMIT={commit}");
    // HEAD, the branch it names and the packed refs: the commit changes only with one of them.
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let branch = git(&["rev-parse", "--symbolic-full-name", "HEAD"]);
    let mut watch = vec![git(&["rev-parse", "--git-path", "HEAD"]), git(&["rev-parse", "--git-path", "packed-refs"])];
    if let Some(b) = branch.filter(|b| b.starts_with("refs/")) {
        watch.push(git(&["rev-parse", "--git-path", &b]));
    }
    for p in watch.into_iter().flatten() {
        let p = PathBuf::from(p);
        let p = if p.is_absolute() { p } else { dir.join(p) };
        // a missing file would rerun this script on every build (a packed ref has no file of its own).
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
}
