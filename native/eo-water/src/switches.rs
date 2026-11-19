//! The water's iteration-2 and iteration-3 kill switches, read from the
//! command line by this crate (eo-app passes them through). Each restores the
//! earlier output of its part; `--no-gg2-water` restores all of them at once
//! (the iteration-3 parts build on the iteration-2 ones), `--no-gg3-water`
//! (and every track's `--no-gg3`) the iteration-3 parts.

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

/// (iteration 3) The measured sea bed (B1 CUDEM) and the swell refracted and
/// shoaled over it, in place of the geometric exposure where it covers.
pub const BATHYMETRY: u32 = 256;
/// (iteration 3) The surf breaking where the depth says (McCowan's limit on the
/// measured bed): foam sheets, the lip, run-up and lace.
pub const SURF_BREAK: u32 = 512;
/// (iteration 3) The bay beaches' thin bright swash line.
pub const SWASH_FOAM: u32 = 1024;
/// (iteration 3) White water where the waves meet rocks, the fender and walls.
pub const OBSTACLE_FOAM: u32 = 2048;
/// (iteration 3) The reflected sky without round 2's uncited constant grey
/// (0.28); no cited body term was added (the body's optics are round 2's).
pub const BODY2: u32 = 4096;
/// (iteration 3) Reserved for the mirror image broken into the dashes the chop
/// resolves; nothing reads it yet (the switch is accepted and does nothing).
pub const PPR_DASH: u32 = 8192;
pub const GG3_ALL: u32 = BATHYMETRY | SURF_BREAK | SWASH_FOAM | OBSTACLE_FOAM | BODY2 | PPR_DASH;

/// The switch names, each with its part.
pub const SWITCHES: [(&str, u32); 17] = [
    ("--no-gg2-water", ALL | GG3_ALL),
    ("--no-water-exposure", EXPOSURE),
    ("--no-water-current", CURRENT),
    ("--no-water-lean", LEAN),
    ("--no-water-ppr-blur", PPR_BLUR),
    ("--no-beach-profiles", BEACH_PROFILES),
    ("--no-water-streak-caps", STREAK_CAPS),
    ("--no-water-far-calm", FAR_CALM),
    ("--no-baker-surf", BAKER_SURF),
    ("--no-gg3-water", GG3_ALL),
    ("--no-gg3", GG3_ALL),
    ("--no-bathymetry", BATHYMETRY),
    ("--no-surf-break", SURF_BREAK),
    ("--no-swash-foam", SWASH_FOAM),
    ("--no-obstacle-foam", OBSTACLE_FOAM),
    ("--no-water-body2", BODY2),
    ("--no-ppr-dash", PPR_DASH),
];

/// The surf and bathymetry track's switch names (the roll-up first); eo-app
/// accepts them, this crate reads them (`SWITCHES`).
pub const GG3_SWITCHES: &[&str] = &["--no-gg3-water", "--no-bathymetry", "--no-surf-break", "--no-swash-foam", "--no-obstacle-foam", "--no-water-body2", "--no-ppr-dash"];

/// The parts on for these arguments.
pub fn from_args<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) -> u32 {
    let mut on = ALL | GG3_ALL;
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
        assert_eq!(from_args(["eo-app"]), ALL | GG3_ALL);
        assert_eq!(from_args(["eo-app", "--no-gg2-water"]), 0);
        assert_eq!(from_args(["eo-app", "--no-gg3-water"]), ALL);
        assert_eq!(from_args(["eo-app", "--no-gg3"]), ALL);
        assert_eq!(from_args(["eo-app", "--no-surf-break"]), ALL | (GG3_ALL & !SURF_BREAK));
        // every reserved name is read, and the bits stay exact in the f32 uniform
        for name in GG3_SWITCHES {
            assert!(SWITCHES.iter().any(|(n, _)| n == name), "{name}");
        }
        assert!((ALL | GG3_ALL) < (1 << 24));
        assert_eq!(from_args(["eo-app", "--no-water-lean"]), (ALL | GG3_ALL) & !LEAN);
        assert_eq!(from_args(["--no-water-exposure", "--no-water-current"]), (ALL | GG3_ALL) & !(EXPOSURE | CURRENT));
    }
}
