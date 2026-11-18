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
