//! The look profile (`--look default|cinematic`, `--no-look`; the cinematic
//! look plan's stage L0): which tone, grade and light palette the frame is
//! drawn with. `Default` is today's frame, graded to the physical references
//! the photometric and delivered-tone tests pin; `Cinematic` is a graded
//! open-world look (punchier contrast, warm lights against cool shadows, a
//! richer golden hour, mixed night lamp colours, wet reflections) that the
//! `high` and `hyper` quality tiers turn on.
//!
//! Every value the cinematic look takes that departs from a physical
//! reference the default keeps is a named constant with a row in
//! `CINEMATIC_DEVIATIONS` (its name, the cinematic value, the default value
//! and why). No street-standard value (lamp positions, heights, spacing,
//! markings) is ever a deviation here: those come from the standards table.
//! Stage L0 adds the switch and the table only; the table is empty until a
//! later stage adds a value.

/// The look's tone curve, per-hour grade, bloom and glare (stage L1).
pub mod grade;

/// The look profile the frame is drawn with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LookProfile {
    /// Today's frame and its physical references (the default, `--no-look`).
    #[default]
    Default,
    /// The graded cinematic look (`--look cinematic`; the high and hyper tiers).
    Cinematic,
}

impl LookProfile {
    pub const ALL: [LookProfile; 2] = [LookProfile::Default, LookProfile::Cinematic];

    pub fn name(self) -> &'static str {
        match self {
            LookProfile::Default => "default",
            LookProfile::Cinematic => "cinematic",
        }
    }

    /// `--look`'s value.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == s)
    }

    pub fn is_cinematic(self) -> bool {
        self == LookProfile::Cinematic
    }
}

/// One value the cinematic look takes that departs from the physical
/// reference the default keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Deviation {
    /// The constant's name (as in the code that uses it).
    pub name: &'static str,
    /// The value under `--look cinematic`, with its unit.
    pub cinematic: &'static str,
    /// The value the default keeps, with its unit and source.
    pub default: &'static str,
    /// Why the cinematic look departs from it.
    pub reason: &'static str,
}

/// Every departure of the cinematic look from the default's physical
/// references (for the owner's review). Empty at stage L0: no value has
/// changed yet. A stage that adds a constant under the cinematic look adds
/// its row here.
pub const CINEMATIC_DEVIATIONS: &[Deviation] = &[
    // stage L1: tone, grade, night exposure, bloom.
    Deviation {
        name: "CINE_WHITE_X",
        cinematic: "scene white at 12 x middle grey (2.16), a hue-preserving shoulder, toe power 1.6, grey at display 0.20 with contrast 1.35 (eo-look cinematic::grade::curve)",
        default: "three's ACES filmic fit after a /0.6 and the JS chain's highlight shoulder, per channel (display 0.99 at about 33 x grey; eo-look tone::display)",
        reason: "a real white point and a soft shoulder that keeps a light's hue until it is very bright: lamp pools and the sky round a low sun roll to white instead of clipping per channel to a pure hue or crawling grey toward white",
    },
    Deviation {
        name: "KEY_NIGHT",
        cinematic: "the per-hour grade's night key: cool deep blacks, warm lights kept saturated (contrast 1.35 about 0.25, sat 0.90 in shadows, 1.3 in highlights)",
        default: "the mesopic fade (CIE 191:2010): pixels under display ~0.3 lose half their colour at full night (look_finish.wgsl finish_color)",
        reason: "the plan's night target: glowing streets with mixed light colours against cool shadows; the fade drained the frame to grey",
    },
    Deviation {
        name: "CINE_NIGHT_UNLIT_DISPLAY_255",
        cinematic: "3.5 / 255 (unlit ground of albedo 0.18 at full night, solved through the cinematic display)",
        default: "8 / 255 (eo-sky night::NIGHT_UNLIT_DISPLAY_255, owner-approved 2026-11-17)",
        reason: "deeper blacks so lamp pools, windows and signs carry the night frame; unlit ground no longer sits in a flat grey toe",
    },
    Deviation {
        name: "CINE_NIGHT_FILL_DESATURATION",
        cinematic: "0.3 of the night fill's colour removed at full night",
        default: "0.6 (eo-sky frame::NIGHT_FILL_DESATURATION, mesopic vision)",
        reason: "the unlit street keeps a cool tint instead of going neutral grey",
    },
    Deviation {
        name: "CINE_BLOOM_THRESHOLD_X",
        cinematic: "bloom from scene value 0.9 (+2.3 EV over grey), knee 1.5, ceiling 24 x, energy-conserving (0.22 of the bright part spread over five levels)",
        default: "from 2.101 / exposure (+3.5 EV, the display's knee), knee 0.55, ceiling 4 x, added on top (the JS chain's BloomNode)",
        reason: "lamps, lit windows and signs glow with a wide soft tail without the bloom brightening the frame",
    },
    // stage L2: night lamp palette, pools, headlights, windows and signs.
    Deviation {
        name: "cine_lamps::street_tint",
        cinematic: "a mixed lamp palette by street (eo-sky cine_lamps): 3000 K LED on most streets, 4000 K on 12 % of arterials and the freeway decks, high-pressure sodium ~2050 K on 26 % of arterials, 12 % of residential and 50 % of industrial/service streets (by name), historic and banner heads 2700 K; all seen by an eye adapted to 3000 K",
        default: "every street lamp 3000 K LED (SFPUC Streetlight Standards Rev. 4 s13, reference sheet 3.3/3.4; 0xffc46a without --light-shadows)",
        reason: "the owner's night palette: warm LED with older orange sodium on some streets and warmer historic heads, so streets differ in colour (the city's conversion to 3000 K LED is nearly complete in reality)",
    },
    Deviation {
        name: "cine_lamps::cobra_shape",
        cinematic: "an IES type II/III cobra-head pool: 1 + 2.2 sin^2 along the street, soft roll-off 62-82 deg along and 42-70 deg across, reach 3.4 mounting heights; the props lamps drawn from their heads, not the JS practical's lifted spot; the nadir's 30 lux kept",
        default: "the batwing to 60 deg cut at 75 deg over 2.6 mounting heights (fallback) and the JS lamp practical's three.js spot (props)",
        reason: "pools long along the street and short across it with soft edges, uneven along the walk (Eavg/Emin 3-6, sheet 3.3), instead of round flat-topped discs with a hard ring",
    },
    Deviation {
        name: "cine_lamps::low_beam_shape",
        cinematic: "the low beam's cut-off 0.4 deg down on the opposing side stepped up to the horizontal on the kerb side, a hot spot 1.5 deg right falling to 0.18 of the peak by ~20 deg; local-light specular widened by the source's radius (cobra 0.3 m, headlamp 0.09 m) and capped at a BRDF of 16/sr",
        default: "20 000 cd across the whole 0.42 rad cone under a horizontal 0.6 deg cut-off; a point source's unbounded GGX highlight",
        reason: "walls, car backs and wet roads in front of a car no longer clip to white while the hot spot stays bright (49 CFR 571.108 test points, sheet 3.6)",
    },
    Deviation {
        name: "rooms::CINE_OCCUPANCY_GAIN",
        cinematic: "the lit share of homes x 1.15 (to 0.95), 18 % of lit homes with a flickering screen in shifting colours, 12 % at a 2200 K amber lamp and 8 % at a 6500 K daylight LED",
        default: "the JS night interiors' occupancy (0.62 at 21 h), 12 % screens, the homes' 2700-5300 K",
        reason: "the reference's 'a few lit upper windows in mixed warm tones' and a livelier night facade (INFERRED, no measured table)",
    },
    Deviation {
        name: "FC_CINE_SIGN_GAIN",
        cinematic: "lit sign faces ~40, letters ~80, neon tubes ~165 cd/m2 (the default's gains x 3, draw.wgsl)",
        default: "a face ~12, a tube ~55 cd/m2 at a lit pane's ~6 (stage S1's gains, set by eye under the default tone curve)",
        reason: "signs at the storefront reference's 40-450 cd/m2 (sheet 3.7) against the lamp pools' ~1 cd/m2: the cinematic shoulder keeps their colour",
    },
    Deviation {
        name: "props::CINE_SIGNAL_LENS_GAIN",
        cinematic: "a lit signal lens at night 3.5 x its tone-solved radiance (over the look's bloom threshold)",
        default: "the lens solved to a fixed display value under the bloom (SIGNAL_LENS_TONE_INPUT)",
        reason: "red and green heads bleed into the bloom at night (reference 7.1, 7.2)",
    },
    // stage L3: wet streets, streaks, reflections, clearcoat.
    Deviation {
        name: "cine_wet::WET_DARKEN",
        cinematic: "wet albedo x 0.45 (asphalt) / 0.55 (concrete), the film's roughness 0.17 / 0.26",
        default: "x 0.58 / 0.66, roughness 0.24 / 0.32 (the JS street surface's wet grade)",
        reason: "wet rough materials about halve their albedo (Lekner & Dorf 1988, sheet 2.3): the road reads dark and glassy, not grey",
    },
    Deviation {
        name: "cine_wet::STREAK_ALPHA_ALONG",
        cinematic: "the lights' reflection in the wet film as a water film's anisotropic GGX (F0 0.02), alpha 0.14 along the ground toward the eye and 0.02 across (a puddle 0.006 both ways), for the 32 brightest lights toward the eye without the clusters' reach; a luminaire's emission toward the film never under 0.35 of its nadir (a headlamp's 0.06 ahead), its reflection bounded by the lens's luminance x Fresnel",
        default: "an isotropic GGX (F0 0.04, the wet roughness) inside each light's cluster reach (2.6-3.4 mounting heights), the pool's cut-off as the emission",
        reason: "lamps, headlamps and shop windows draw the long vertical streaks of a wet night street (sheet 7.2), including lamps far up the street whose pools end short of the reflection point; the view-aligned stretch stands in for the slope spread of real wet asphalt at grazing angles (INFERRED widths)",
    },
    Deviation {
        name: "cine_wet::SSR_ROUGH",
        cinematic: "the screen-space reflection up to roughness 0.5 (fading from 0.3), 24 steps from 0.15 m x 1.32, the hit read as 7 taps along the frame's vertical as long as the film's lobe, its cap 24 x the exposure",
        default: "up to 0.36 (fading from 0.1), 12 steps from 0.22 m x 1.7, one tap, cap 8 x the exposure (the JS wet reflection)",
        reason: "signals, signs, windows and tail lamps streak down the damp road like the lights' analytic streaks and keep their colour",
    },
    Deviation {
        name: "cine_wet::COAT_ROUGH",
        cinematic: "car paint under a clearcoat (F0 0.04, roughness 0.07, up to 0.35 with grime) reflecting the sky or probe grid, the street where the screen-space reflection finds it, the key and the street's lights (bounded by the lens's luminance, not the 16/sr cap)",
        default: "the paint's own GLB roughness, one specular layer",
        reason: "cars catch the street's lights and reflect it (automotive clearcoat, INFERRED values)",
    },
    // stage L4: golden hour and atmosphere.
    Deviation {
        name: "golden::SUN_COLOUR_KNOTS",
        cinematic: "the sun's key coloured by its altitude as a black body: 5900 K from 40 deg up, 5200 K at 20, 4300 K at 12, 3600 K at 8, 3300 K at 6, 2900 K at 4, 2400 K at 2, 2100 K on the horizon (red/blue 1.17 at noon, 4.2 at 18.5 h), the key's luminance kept",
        default: "the JS rig table's sun colour (eo-look data/look-oracle.json rigBaked: red/blue 1.75 at noon, 3.4 at 18.5 h)",
        reason: "a whiter noon and a clearly golden-to-red low sun, so the hours separate (golden hour was only about 2 x warmer than noon)",
    },
    Deviation {
        name: "golden::DIRECTIONAL_SKY_SHARE",
        cinematic: "0.45 of a wall's day fill (0.15 with the sun over 30 deg) from the sky's own upper-hemisphere irradiance (warm toward a low sun, blue away), the rig's constant sky lobe and ambient scaled down to keep an upward face's fill",
        default: "the bound probe's yield: the sky's directional light ~3 % of a wall's fill at golden hour, the rest the rig's constant colours",
        reason: "faces toward the glowing west warm, faces toward the east cool blue: the warm-cool separation of a low sun",
    },
    Deviation {
        name: "golden::RIM_KEEP",
        cinematic: "0.25 of the rig's rim light by day, the rest moved into the hemisphere's sky lobe at what it gave an upward face",
        default: "1 (the JS rim opposite the key, 28 deg up, unshadowed: ~55 % of a wall's fill at golden hour)",
        reason: "an unshadowed blue light lit faces deep in shade; the directional sky fill now carries the light from the sky opposite the sun",
    },
    Deviation {
        name: "golden::EXPOSURE_ADAPTATION",
        cinematic: "0.18 (the day exposure ~1.24 at 18.5 h; noon and the blue hour unchanged)",
        default: "0.20 (the JS EXPOSURE_CURVE, eo-look rig::EXPOSURE_ADAPTATION: 1.30 at 18.5 h)",
        reason: "the evening exposure opens up less, so shade sits darker and the warm key carries the frame",
    },
    Deviation {
        name: "VOL_CINE_AUREOLE_KEEP",
        cinematic: "half of the low sun's aureole damping kept in the drawn sky (sky_volume.wgsl vol_atmo_sky: from 1.5 to 30 deg round a sun under 15 deg the glow takes 0.5 of the cut to the model zenith's scale)",
        default: "the whole cut (the glow over 1.5-30 deg held to the atmosphere tables' own zenith scale against a white-out)",
        reason: "the warm glow round a low sun restored in the drawn sky only (the dome's veil, frame::LOW_SUN_VEIL_KEEP, is left alone: the camera meter reads it as a brighter scene and took ~35 % off the whole golden-hour frame)",
    },
    Deviation {
        name: "VOL_CINE_HAZE",
        cinematic: "the aerial haze's aerosol thins from its own floor with a 600 m scale height (the mean density between the eye's and the point's heights), 1.4 x denser with the sun under 8 deg, and 0.6 of its in-scatter from the key through a Henyey-Greenstein phase of g 0.75 (sky_volume.wgsl VOL_CINE_*)",
        default: "a 400 m law on the height difference alone, the clear air's density at every hour, every in-scattered ray the horizon sky's colour",
        reason: "a haze layer seen over the city from 1-3 km up, distance glowing warm toward a low sun and cooler away from it, and room for light shafts (the shade in the haze is taken off its in-scatter, not added as a second haze)",
    },
];

/// The row named `name`, if the table has one.
pub fn deviation(name: &str) -> Option<&'static Deviation> {
    CINEMATIC_DEVIATIONS.iter().find(|d| d.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_parses_its_names_and_defaults_to_today() {
        assert_eq!(LookProfile::default(), LookProfile::Default);
        for p in LookProfile::ALL {
            assert_eq!(LookProfile::parse(p.name()), Some(p));
        }
        assert_eq!(LookProfile::parse("filmic"), None);
        assert!(LookProfile::Cinematic.is_cinematic() && !LookProfile::Default.is_cinematic());
    }

    #[test]
    fn every_deviation_is_named_once_and_filled_in() {
        let mut names: Vec<&str> = CINEMATIC_DEVIATIONS.iter().map(|d| d.name).collect();
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "a deviation's name appears twice in CINEMATIC_DEVIATIONS");
        for d in CINEMATIC_DEVIATIONS {
            assert!(!d.name.trim().is_empty() && !d.cinematic.trim().is_empty() && !d.default.trim().is_empty() && !d.reason.trim().is_empty(), "{d:?}: every field is filled in");
            assert_ne!(d.cinematic, d.default, "{}: a deviation departs from the default", d.name);
            assert_eq!(deviation(d.name), Some(d));
        }
        assert_eq!(deviation("no such value"), None);
    }
}
