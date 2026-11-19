//! The water's iteration-2 kill switches, read from the command line by this
//! crate (eo-app passes them through). Each restores the earlier output of
//! its part; `--no-gg2-water` restores all of them at once.

use std::sync::OnceLock;

/// The exposure field, the strait's measured wind floor (water-1).
pub const EXPOSURE: u32 = 1;
/// The tidal current on the game's clock and its Doppler steepening (water-1).
pub const CURRENT: u32 = 2;
/// The anisotropic cascade fade and the sub-pixel slope variance (water-4).
pub const LEAN: u32 = 4;
/// The slope-blurred projected reflection (water-5).
pub const PPR_BLUR: u32 = 8;
/// Each beach's own foreshore and the turbid bottom (water-3, water side).
pub const BEACH_PROFILES: u32 = 16;
/// The measured afternoon wind and whitecaps streaked downwind (water-10).
pub const STREAK_CAPS: u32 = 32;
/// The far calm: gusts fading with range, slicks as current lines (water-11).
pub const FAR_CALM: u32 = 64;
/// The open coast's surf where the exposure field finds the Pacific's swell
/// (Baker, Marshall's and China beaches), not only west of the Gate (water-2).
pub const BAKER_SURF: u32 = 128;
pub const ALL: u32 = EXPOSURE | CURRENT | LEAN | PPR_BLUR | BEACH_PROFILES | STREAK_CAPS | FAR_CALM | BAKER_SURF;

/// The switch names, each with its part.
pub const SWITCHES: [(&str, u32); 9] = [
    ("--no-gg2-water", ALL),
    ("--no-water-exposure", EXPOSURE),
    ("--no-water-current", CURRENT),
    ("--no-water-lean", LEAN),
    ("--no-water-ppr-blur", PPR_BLUR),
    ("--no-beach-profiles", BEACH_PROFILES),
    ("--no-water-streak-caps", STREAK_CAPS),
    ("--no-water-far-calm", FAR_CALM),
    ("--no-baker-surf", BAKER_SURF),
];

/// The surf and bathymetry track's switch names (the roll-up first); eo-app
/// accepts them. Reserved: nothing reads them yet.
pub const GG3_SWITCHES: &[&str] = &["--no-gg3-water", "--no-bathymetry", "--no-surf-break", "--no-swash-foam", "--no-obstacle-foam", "--no-water-body2", "--no-ppr-dash"];

/// The parts on for these arguments.
pub fn from_args<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) -> u32 {
    let mut on = ALL;
    for a in args {
        for (name, bits) in SWITCHES {
            if a.as_ref() == name {
                on &= !bits;
            }
        }
    }
    on
}

/// The parts on for this process (its command line, read once).
pub fn on() -> u32 {
    static ON: OnceLock<u32> = OnceLock::new();
    *ON.get_or_init(|| from_args(std::env::args()))
}

pub fn is_on(bit: u32) -> bool {
    on() & bit != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roll_up_turns_every_part_off() {
        assert_eq!(from_args(["eo-app"]), ALL);
        assert_eq!(from_args(["eo-app", "--no-gg2-water"]), 0);
        assert_eq!(from_args(["eo-app", "--no-water-lean"]), ALL & !LEAN);
        assert_eq!(from_args(["--no-water-exposure", "--no-water-current"]), ALL & !(EXPOSURE | CURRENT));
    }
}
