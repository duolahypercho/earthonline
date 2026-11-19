//! Obstacles in the surf: rocks and piers that waves break on, registered by
//! whatever places them (game metres: centre x, z, radius, height of the top
//! above the sea level), for the water to foam round. A process-wide list
//! with a revision, so a reader can tell when to rebuild what it derived from
//! it. Nobody registers any yet.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// One obstacle: centre (x, z), radius (m) and its top over the sea level (m), game metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    pub x: f64,
    pub z: f64,
    pub radius: f64,
    pub height: f64,
}

static LIST: RwLock<Option<Arc<Vec<Obstacle>>>> = RwLock::new(None);
static REVISION: AtomicU64 = AtomicU64::new(0);

/// Replace the list (bumps `revision`).
pub fn set(obstacles: Vec<Obstacle>) {
    *LIST.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(obstacles));
    REVISION.fetch_add(1, Ordering::AcqRel);
}

/// The list in force (empty before the first `set`).
pub fn get() -> Arc<Vec<Obstacle>> {
    LIST.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// How many times the list has been set (0: never).
pub fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_replaces_and_counts() {
        assert_eq!(revision(), 0);
        assert!(get().is_empty());
        let a = Obstacle { x: 1.0, z: 2.0, radius: 3.0, height: 0.5 };
        set(vec![a]);
        assert_eq!(revision(), 1);
        assert_eq!(*get(), vec![a]);
        set(vec![]);
        assert_eq!(revision(), 2);
        assert!(get().is_empty());
    }
}
