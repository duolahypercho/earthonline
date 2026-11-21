//! The detail ground: one process-wide provider of a finer drawn ground than
//! the 8 m terrain grid (for example a refined mesh from a 1 m survey DEM
//! where the photo views look), so everything that stands on the ground
//! (plants, rocks) sits on the surface the frame draws.
//!
//! Heights are drawn game metres (game y, the ground plan and the height
//! scale already applied), at game x, z. With no provider set (the default)
//! `height` answers None everywhere and every reader keeps its own ground:
//! the readers call `height(x, z).unwrap_or(<their height>)`, so nothing
//! moves until a provider is installed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// A provider: the drawn ground's height at game (x, z), None where it has
/// no detail (the caller's own ground then).
pub type Provider = Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>;

static PROVIDER: RwLock<Option<Provider>> = RwLock::new(None);
/// 0 until a provider is first set; bumped by every `set_provider`.
static REVISION: AtomicU64 = AtomicU64::new(0);

/// Install (or replace) the provider; bumps `revision`, so what was seated on
/// the previous ground can tell it is stale.
pub fn set_provider(f: Box<dyn Fn(f64, f64) -> Option<f64> + Send + Sync>) {
    *PROVIDER.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::from(f));
    REVISION.fetch_add(1, Ordering::AcqRel);
}

/// The detail ground's drawn height at game (x, z), m; None with no provider
/// or where it has no detail.
pub fn height(x: f64, z: f64) -> Option<f64> {
    if REVISION.load(Ordering::Acquire) == 0 {
        return None; // never set: no lock taken
    }
    let p = PROVIDER.read().unwrap_or_else(|e| e.into_inner()).clone()?;
    p(x, z).filter(|h| h.is_finite())
}

/// How many times a provider has been set (0: never).
pub fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_answers_none_then_the_provider_answers() {
        // the process-wide state is shared with nothing else in this crate's tests.
        assert_eq!(revision(), 0);
        assert_eq!(height(10.0, 20.0), None);
        assert_eq!(height(10.0, 20.0).unwrap_or(3.5), 3.5);
        set_provider(Box::new(|x, z| (x > 0.0).then_some(x + z)));
        assert_eq!(revision(), 1);
        assert_eq!(height(1.0, 2.0), Some(3.0));
        assert_eq!(height(-1.0, 2.0), None);
        set_provider(Box::new(|_, _| Some(f64::NAN)));
        assert_eq!(revision(), 2);
        assert_eq!(height(1.0, 2.0), None, "a non-finite height is no height");
    }
}
