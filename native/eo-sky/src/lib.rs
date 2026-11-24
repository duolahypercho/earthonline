//! The sky, weather, night and camera-finish models as pure functions (no
//! GPU): the JS game's weather schedule and ground water keyed on a shared
//! world clock (`weather`), the cloud decks and their coverage sheet
//! (`clouds`), the moon (`moon`), the practical clock and exposure
//! (`night`), street lamps as light records (`lights`), the physical
//! references the look is checked against (`photometry`), and what all of
//! it does to the clear-weather light rig (`frame`). Every module names the
//! JS file or the physical source it follows.

pub mod cine_lamps;
pub mod cine_wet;
pub mod clouds;
pub mod frame;
pub mod hash;
pub mod lamp_bounce;
pub mod lamp_units;
pub mod lights;
pub mod lit_fog;
pub mod moon;
pub mod night;
pub mod night_photometry;
pub mod photo;
pub mod photometry;
pub mod shops;
pub mod urban_sky;
pub mod weather;

pub use frame::{Conditions, SkyTerms, apply};
pub use weather::{Kind, WeatherState};

/// The world seed the canonical San Francisco world derives its weather
/// from (`deriveWeatherScheduleSeed('sf', 0)`, the JS's source id).
pub fn world_seed() -> u32 {
    hash::schedule_seed("sf", 0)
}
