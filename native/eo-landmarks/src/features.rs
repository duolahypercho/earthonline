//! The waterfront corridor's kill switches (`eo-app --no-<feature>`): each
//! restores the previous output when off. Process-wide, set once at start-up
//! before the landmark set is built (as `eo_agents::motion::set_enabled`).

use std::sync::atomic::{AtomicBool, Ordering};

static FERRY_BUILDING: AtomicBool = AtomicBool::new(true);

/// The Ferry Building (ferry_building.rs) in place of its record's plain box
/// (`--no-ferry-building`: the box, as before).
pub fn ferry_building() -> bool {
    FERRY_BUILDING.load(Ordering::Relaxed)
}

pub fn set_ferry_building(on: bool) {
    FERRY_BUILDING.store(on, Ordering::Relaxed);
}

static TRAM_TRACKS: AtomicBool = AtomicBool::new(true);
static QUAY_RAILING: AtomicBool = AtomicBool::new(true);

/// The streetcar rails of the corridor (tram.rs; `--no-tram-tracks`: none, as before).
pub fn tram_tracks() -> bool {
    TRAM_TRACKS.load(Ordering::Relaxed)
}

pub fn set_tram_tracks(on: bool) {
    TRAM_TRACKS.store(on, Ordering::Relaxed);
}

/// The guard rail on the Embarcadero's quay wall (waterfront.rs `quay_railing`; `--no-quay-railing`: none, as before).
pub fn quay_railing() -> bool {
    QUAY_RAILING.load(Ordering::Relaxed)
}

pub fn set_quay_railing(on: bool) {
    QUAY_RAILING.store(on, Ordering::Relaxed);
}

static FERRY_BUILDING_DETAIL: AtomicBool = AtomicBool::new(true);
static RAIL_FINISH: AtomicBool = AtomicBool::new(true);
static CORRIDOR_PALMS: AtomicBool = AtomicBool::new(true);
static GUARD_DETAIL: AtomicBool = AtomicBool::new(true);
static PIER_SHEDS: AtomicBool = AtomicBool::new(true);

/// The Ferry Building's second pass (ferry_building.rs `Detail`): sandstone
/// ashlar at its course size, banded rustication, deep reveals, keystones,
/// modillion cornice, granite plinth, weathering, the tower's balustrades
/// (`--no-ferry-building-detail`: the first pass, as before).
pub fn ferry_building_detail() -> bool {
    FERRY_BUILDING_DETAIL.load(Ordering::Relaxed)
}

pub fn set_ferry_building_detail(on: bool) {
    FERRY_BUILDING_DETAIL.store(on, Ordering::Relaxed);
}

/// The rails' finish (tram.rs): a worn running band, oxidised gauge and field
/// sides and a grimed flangeway (`--no-rail-finish`: one bare-steel head, as before).
pub fn rail_finish() -> bool {
    RAIL_FINISH.load(Ordering::Relaxed)
}

pub fn set_rail_finish(on: bool) {
    RAIL_FINISH.store(on, Ordering::Relaxed);
}

/// The Canary Island date palms of the Embarcadero and the Ferry Building
/// plaza (palms.rs; `--no-corridor-palms`: none, as before).
pub fn corridor_palms() -> bool {
    CORRIDOR_PALMS.load(Ordering::Relaxed)
}

pub fn set_corridor_palms(on: bool) {
    CORRIDOR_PALMS.store(on, Ordering::Relaxed);
}

/// The quay guard's second pass (waterfront.rs `quay_railing_with`): the
/// corridor's quay on its own top at the drawn carpet's level (`quay_lift`,
/// so the guard's 42 in stand over the walk, not over a raised shelf), a kick
/// plate on the coping, base plates, and pickets the wire pass draws
/// coverage-faded (`--no-guard-detail`: the first pass on the JS crest, as before).
pub fn guard_detail() -> bool {
    GUARD_DETAIL.load(Ordering::Relaxed)
}

pub fn set_guard_detail(on: bool) {
    GUARD_DETAIL.store(on, Ordering::Relaxed);
}

/// The Ferry Plaza's gate sheds as steel canopies over their gangway
/// pavilions (waterfront.rs `ferry_sheds`; `--no-pier-sheds`: the JS's boxes, as before).
pub fn pier_sheds() -> bool {
    PIER_SHEDS.load(Ordering::Relaxed)
}

pub fn set_pier_sheds(on: bool) {
    PIER_SHEDS.store(on, Ordering::Relaxed);
}

static RAIL_POLISH: AtomicBool = AtomicBool::new(true);

/// The rails' polish (tram.rs): the running band burnished bare steel
/// (metalness 1, roughness 0.2) and bituminous sealant joints either side of
/// each rail (`--no-rail-polish`: the second pass's worn band, as before).
pub fn rail_polish() -> bool {
    RAIL_POLISH.load(Ordering::Relaxed)
}

pub fn set_rail_polish(on: bool) {
    RAIL_POLISH.store(on, Ordering::Relaxed);
}

static FERRY_BUILDING_RELIEF: AtomicBool = AtomicBool::new(true);

/// The Ferry Building's third pass (ferry_building.rs `Relief`): the cornices'
/// occlusion on the wall under them, darker reveals, the glazing one pane per
/// light with its own room tone or a blind and at a near-mirror roughness,
/// frames standing proud of the glass round the fanlights too, and voussoirs
/// of their own tone in the archivolts (`--no-facade-relief`: the second pass, as before).
pub fn ferry_building_relief() -> bool {
    FERRY_BUILDING_RELIEF.load(Ordering::Relaxed)
}

pub fn set_ferry_building_relief(on: bool) {
    FERRY_BUILDING_RELIEF.store(on, Ordering::Relaxed);
}

static PALM_DETAIL: AtomicBool = AtomicBool::new(true);

/// The corridor palms' second pass (palms.rs): a tree well of decomposed
/// granite with its edge and the trunk's occlusion, root initials at the
/// foot, the leaf-base scars as a diamond lattice in the trunk, senescent
/// lower leaves and the spear, and unsurveyed palms varied in size and lean
/// (`--no-palm-detail`: the first pass, as before).
pub fn palm_detail() -> bool {
    PALM_DETAIL.load(Ordering::Relaxed)
}

pub fn set_palm_detail(on: bool) {
    PALM_DETAIL.store(on, Ordering::Relaxed);
}

/// The Golden Gate shore-ground pass (iteration 2) and its parts, read from
/// the command line (`eo-app --no-<name>`; main.rs passes the names through):
/// `--no-gg2-ground` turns every part off at once, restoring the ground,
/// beaches and waterfront as they were before the pass.
pub const SHORE_GROUND_SWITCHES: [&str; 7] = ["beach-sand-limit", "sand-albedo", "rocks", "riprap", "pier-lines", "ground-steps", "season-seam"];

/// Whether the shore-ground part `name` (one of `SHORE_GROUND_SWITCHES`) is on.
pub fn shore_ground(name: &str) -> bool {
    static ARGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    shore_ground_in(ARGS.get_or_init(|| std::env::args().collect()), name)
}

/// The native cache folder as eo-app's `cache_dir` finds it (`--no-cache`:
/// none; `--cache <dir>`; else `EO_NATIVE_CACHE`, else the workspace's
/// `cache`, as eo-assets `default_cache_dir`), read from the command line,
/// for the loads the landmark build repeats beside main.rs's (the shore field).
pub fn cache_dir() -> Option<std::path::PathBuf> {
    cache_dir_in(&std::env::args().collect::<Vec<_>>(), std::env::var_os("EO_NATIVE_CACHE"))
}

/// `cache_dir` over a given argument list and `EO_NATIVE_CACHE`.
pub fn cache_dir_in(args: &[String], env: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    if args.iter().any(|a| a == "--no-cache") {
        return None;
    }
    if let Some(i) = args.iter().position(|a| a == "--cache") {
        return args.get(i + 1).map(std::path::PathBuf::from);
    }
    Some(env.map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../cache")))
}

/// `shore_ground` over a given argument list.
pub fn shore_ground_in(args: &[String], name: &str) -> bool {
    debug_assert!(SHORE_GROUND_SWITCHES.contains(&name), "unknown shore-ground switch {name}");
    !args.iter().any(|a| a == "--no-gg2-ground" || a.strip_prefix("--no-") == Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shore_ground_switches_turn_off_alone_or_together() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for n in SHORE_GROUND_SWITCHES {
            assert!(shore_ground_in(&args(&["eo-app"]), n));
            assert!(!shore_ground_in(&args(&["eo-app", "--no-gg2-ground"]), n));
            assert!(!shore_ground_in(&args(&["eo-app", &format!("--no-{n}")]), n));
        }
        assert!(shore_ground_in(&args(&["eo-app", "--no-rocks"]), "riprap"));
    }

    #[test]
    fn the_cache_folder_follows_the_app() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(cache_dir_in(&args(&["eo-app", "--no-cache"]), Some("e".into())), None);
        assert_eq!(cache_dir_in(&args(&["eo-app", "--cache", "c"]), Some("e".into())), Some("c".into()));
        assert_eq!(cache_dir_in(&args(&["eo-app"]), Some("e".into())), Some("e".into()));
        assert!(cache_dir_in(&args(&["eo-app"]), None).is_some_and(|p| p.ends_with("../cache")));
    }
}
