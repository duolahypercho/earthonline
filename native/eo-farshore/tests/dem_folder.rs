//! Where the far shore's DEM tiles are looked for. Its own test binary: the
//! crate's unit tests change EO_DEM_ROOT while they run.

use std::path::PathBuf;

/// `<repo>/external/dem` unless EO_DEM_ROOT names another folder.
#[test]
fn dem_folder_defaults_to_external() {
    let repo = eo_data::DataPaths::discover().expect("the repository").repo_root;
    match std::env::var_os("EO_DEM_ROOT").filter(|v| !v.is_empty()) {
        Some(v) => assert_eq!(eo_farshore::dem_root(), Some(PathBuf::from(v))),
        None => assert_eq!(eo_farshore::dem_root(), Some(repo.join("external").join("dem"))),
    }
}
