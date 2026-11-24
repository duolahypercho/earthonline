//! (`--night-photometry`) The night on one photometric scale: the city's
//! practicals (street lamps, headlamps, shop windows, lit rooms, signs and
//! the lenses of lamps and signals) against the night's natural light (the
//! sky's glow, the moon and the unlit fill), and the camera exposed on the
//! lamp-lit street instead of on the unlit ground. Pure functions; the
//! renderer applies them to a frame `frame::apply_with` has built (eo-render
//! look.rs `LookGpu::night_photometry`). Without the switch nothing here runs.
//!
//! Why. The rig's night fill stands for `NIGHT_UNLIT_GROUND_LUX` x
//! `frame::NIGHT_GROUND_FILL` = 1.6 lux on unlit ground (`lamp_units_per_lux`
//! is calibrated on it), so a lamp pool's 30 lux peak is only ~19 x the
//! unlit ground, and the night solve exposes that unlit ground at a fixed
//! display level: the city's natural light carries the frame (a frame drawn
//! with every practical off is nearly as bright as one with them on) and the
//! lamps throw faint pools. Unlit ground in a lit city at night gets under
//! a lux (`NIGHT_UNLIT_PHYSICAL_LUX`), so the practicals are
//! `practical_gain` x brighter against the rig's natural light than drawn.
//! A light meter on the lit street (ISO 2720 incident metering at the RP-8
//! street average) then sets the exposure at the key an eye adapted to that
//! street sees it at (`adapted_key`), so the lamp pools, the shop windows
//! and the signs carry the frame and the unlit city falls darker by its own
//! illuminance, not by a display target.
//!
//! The lenses of signals and pedestrian heads are solved in display terms
//! (eo-render props.rs: x 1 / exposure) and take no gain (r1i1 gave them
//! one, and every aspect of a signal head read lit).

use crate::frame::{NIGHT_GROUND_FILL, SkyTerms};
use crate::night;
use crate::photometry::NIGHT_UNLIT_GROUND_LUX;
use eo_look::LookFrame;

/// Horizontal illuminance of unlit city ground at full night (lux): what
/// the renderer does not model lights it. The glow of a Bortle 8-9 city
/// sky (zenith ~17-18 mag/arcsec^2, 0.007-0.017 cd/m^2, ~0.03-0.05 lux on
/// the horizontal: Kyba et al. 2015, Sci. Rep. 5:8409), the moon when it is
/// up (at most ~0.27 lux full at the zenith: Kyba et al. 2017, Sci. Rep.
/// 7:2321), the spill of the windows, porch lights, signs and far lamps the
/// clusters do not carry, and the street canyon's inter-reflection of the
/// lamps' light: the minimum between the poles of a lit street (ANSI/IES
/// RP-8-18: a local road's 4 lux average at 6:1 average to minimum, 0.67
/// lux; a collector's 6 lux at 4:1, 1.5 lux), taken at 0.8 lux (r1i1 took
/// the local road's floor alone, 0.4 lux, and the unlit half of the frame
/// fell into the display's toe: 50-62 % of the pixels under 16/255 on the
/// night cards). The rig's unlit fill stands for this at full night under
/// `--night-photometry`.
pub const NIGHT_UNLIT_PHYSICAL_LUX: f64 = 0.8;

/// The illuminance the night exposure meters (lux): the average maintained
/// illuminance of a major road at medium pedestrian conflict (ANSI/IES
/// RP-8-18; `photometry::STREET_AVERAGE` 4-17 lux), the light of the lit
/// street a camera is pointed along.
pub const NIGHT_METER_LUX: f64 = 9.0;

/// Photographic middle grey: an 18 % card at L* 50, sRGB 118 (ISO 12232,
/// ISO 2720; `photometry::GREY_CARD_MIDDLE`).
pub const MIDDLE_GREY: f64 = 0.18;
pub const MIDDLE_GREY_DISPLAY_255: f64 = 118.0;

/// The key a scene adapted to a log-average luminance `l_cd` (cd/m^2) is
/// shown at: Krawczyk, Myszkowski and Seidel 2005 ("Lightness perception in
/// tone reproduction for high dynamic range images", Eurographics; the
/// automatic key of Reinhard's operator), a = 1.03 - 2 / (2 + log10(L + 1)):
/// 0.18 near 1.25 cd/m^2, lower as the eye adapts to darker scenes (a lit
/// street at night reads darker than middle grey, as it is seen), higher in
/// daylight. Held in [0.05, 0.18] here (a night key never brightens past the
/// card's middle grey).
pub fn adapted_key(l_cd: f64) -> f64 {
    (1.03 - 2.0 / (2.0 + (l_cd.max(0.0) + 1.0).log10())).clamp(0.05, MIDDLE_GREY)
}

/// The luminance (cd/m^2) of an 18 % card under `NIGHT_METER_LUX` (a
/// Lambertian card: L = rho E / pi): the lit street the eye adapts to.
pub fn meter_card_cd() -> f64 {
    MIDDLE_GREY * NIGHT_METER_LUX / std::f64::consts::PI
}

/// The exposure scale the meter's key puts on the card against middle grey:
/// `adapted_key(meter_card_cd())` / 0.18 (0.63: the lit street 0.67 EV under
/// middle grey; r1i1 exposed it at middle grey and the pools near the
/// camera flooded the foreground and blew the shop spill to white).
pub fn meter_key_scale() -> f64 {
    adapted_key(meter_card_cd()) / MIDDLE_GREY
}

/// The practicals' gain at full night: the rig's unlit fill in the lamps'
/// lux (`NIGHT_UNLIT_GROUND_LUX` x `NIGHT_GROUND_FILL`, 1.6 lux) over the
/// physical unlit ground's (`NIGHT_UNLIT_PHYSICAL_LUX`): 2.
pub fn full_night_gain() -> f64 {
    NIGHT_UNLIT_GROUND_LUX * NIGHT_GROUND_FILL / NIGHT_UNLIT_PHYSICAL_LUX
}

/// How far the practicals have taken their night gain: the JS night
/// blend (`night::nightness`, 1 - smoothstep(-6, 2, sun altitude)). As the
/// sky fades the lamps keep their lux, so they gain on the rig's natural
/// light from sunset (the blue hour already shows lamps and windows
/// brighter than the rig's capped twilight fill lets them).
pub fn practical_share(sun_alt_deg: f64) -> f64 {
    night::nightness(sun_alt_deg)
}

/// The practicals' gain over their calibration on the rig at a sun
/// altitude: `full_night_gain()` ^ `practical_share` (1 by day, a geometric
/// ramp through dusk, so a stop of gain arrives evenly over the sky's fade).
pub fn practical_gain(sun_alt_deg: f64) -> f64 {
    let s = practical_share(sun_alt_deg);
    if s <= 0.0 {
        return 1.0;
    }
    full_night_gain().powf(s)
}

/// The lamps' units per lux under the gain: the night rig's calibration
/// (`night::lamp_units_per_lux`) x `practical_gain`. The twilight fill's cap
/// (`frame::apply_with`'s `lamp_scale`, which turned the lamps down with the
/// capped fill to keep the JS's 7.5 x ratio) is not applied: the lamps keep
/// their lux while the sky fades, and the gain is what they win over the rig.
pub fn lamp_units_per_lux(sun_alt_deg: f64) -> f64 {
    night::lamp_units_per_lux() * practical_gain(sun_alt_deg)
}

/// The night exposure from the meter: the exposure in [0.01, 8] that shows
/// an 18 % card under `NIGHT_METER_LUX` of the lamps at the adapted key
/// (`meter_key_scale` x middle grey) through the delivered display (`cinematic`: the
/// cinematic look's display at full night, `night::cinematic_display_grey`;
/// else the default's, `night::display_grey`), with `units_per_lux` the
/// lamps' scale.
pub fn meter_exposure(units_per_lux: f64, cinematic: bool) -> f64 {
    // the card shown at the adapted key: the exposure that shows the card's radiance over the key's
    // scale at middle grey shows the card itself at key / 0.18 of middle grey's scene value.
    let radiance = MIDDLE_GREY * NIGHT_METER_LUX * units_per_lux / std::f64::consts::PI / meter_key_scale();
    let target = MIDDLE_GREY_DISPLAY_255 / 255.0;
    let (mut lo, mut hi) = (0.01f64, 8.0f64);
    for _ in 0..48 {
        let mid = (lo * hi).sqrt();
        let d = if cinematic { night::cinematic_display_grey(radiance, mid) } else { night::display_grey(radiance, mid) };
        if d < target { lo = mid } else { hi = mid }
    }
    (lo * hi).sqrt()
}

/// The night solve `t.exposure_target` carries for its night share
/// (`night::exposure_nightness`): the cinematic one under the look (look.rs
/// `cinematic_night` moved it there), else the default.
fn current_night_solve(f: &LookFrame, cinematic: bool) -> f64 {
    if cinematic { night::cinematic_night_exposure(f, 0.0) } else { night::night_exposure(f, 0.0) }
}

/// Applies the night's photometric scale to a frame and its terms (after
/// `frame::apply_with` and, under the look, `cinematic_night`): the lamps'
/// units per lux (`lamp_units_per_lux`), and the exposure's night share
/// moved from the unlit-ground solve to the meter (`meter_exposure`), its
/// day share untouched. Returns the practicals' gain the shaders take for
/// the emission they carry in the rig's own units (lit rooms, signs,
/// lenses). By day (no night share, no gain) the frame and terms are
/// returned exactly as given.
pub fn apply(f: &mut LookFrame, t: &mut SkyTerms, cinematic: bool) -> f64 {
    let alt = f.model.sun.altitude_deg;
    let g = practical_gain(alt);
    if g > 1.0 {
        t.lamp_units_per_lux = lamp_units_per_lux(alt);
    }
    let n = night::exposure_nightness(alt);
    if n > 0.0 {
        let meter = meter_exposure(lamp_units_per_lux(alt), cinematic);
        t.exposure_target += (meter - current_night_solve(f, cinematic)) * n;
        f.exposure = t.exposure_target;
    }
    g
}

/// Sun altitudes over which `apply` changes nothing (`practical_share` 0).
pub fn is_day(sun_alt_deg: f64) -> bool {
    practical_share(sun_alt_deg) <= 0.0 && night::exposure_nightness(sun_alt_deg) <= 0.0
}

/// The share of a far luminaire's air light that reaches the eye against
/// the near model's (sky_fx.wgsl `sprite_vs`, `--night-photometry`): past
/// the distance where its halo stops shrinking on screen (its core held at
/// 1.5 px, `core_m` over the lens's `lens_m`), single scattering's airlight
/// at a fixed angle from a point source falls as 1 / distance (Narasimhan &
/// Nayar 2003, the airlight of a point source: L ~ sigma I P(theta) / (D
/// theta)), so the halo keeps the 1 / distance the screen-sized sprite
/// dropped; nearer, the near model's.
pub fn far_air_light_share(core_m: f64, lens_m: f64) -> f64 {
    (lens_m / core_m.max(lens_m)).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{self, Conditions, FrameOptions};
    use crate::weather;

    const CINE: FrameOptions = FrameOptions { smooth: false, cinematic: true };

    fn frame_at(hour: f64, kind: crate::Kind, o: FrameOptions) -> (LookFrame, SkyTerms) {
        let c = Conditions { hour, weather: weather::preset(kind), moon_day: 0.0 };
        let (mut f, mut t) = frame::look_frame_with(&c, None, o);
        if o.cinematic {
            night::cinematic_night_fill(&mut f, t.nightness);
            t.exposure_target += night::cinematic_night_shift(&f);
            f.exposure = t.exposure_target;
        }
        (f, t)
    }

    #[test]
    fn the_gain_is_two_at_night_and_one_by_day() {
        assert!((full_night_gain() - 2.0).abs() < 1e-12);
        assert_eq!(practical_gain(30.0), 1.0);
        assert_eq!(practical_gain(2.0), 1.0);
        assert!((practical_gain(-6.0) - 2.0).abs() < 1e-12 && (practical_gain(-40.0) - 2.0).abs() < 1e-12);
        // monotone through dusk
        let mut last = 1.0;
        for i in 0..=80 {
            let g = practical_gain(2.0 - i as f64 * 0.1);
            assert!(g >= last - 1e-12, "{g} after {last}");
            last = g;
        }
    }

    #[test]
    fn by_day_nothing_changes() {
        for hour in [8.0, 12.0, 16.0, 18.0] {
            for o in [FrameOptions::default(), CINE] {
                let (f0, t0) = frame_at(hour, crate::Kind::Clear, o);
                assert!(is_day(f0.model.sun.altitude_deg), "{hour}");
                let (mut f, mut t) = (f0, t0);
                let g = apply(&mut f, &mut t, o.cinematic);
                assert_eq!(g, 1.0);
                assert_eq!(t.exposure_target, t0.exposure_target);
                assert_eq!(t.lamp_units_per_lux, t0.lamp_units_per_lux);
                assert_eq!(f.exposure, f0.exposure);
            }
        }
    }

    #[test]
    fn at_night_the_meter_exposes_the_lit_street_at_the_adapted_key() {
        // Krawczyk et al. 2005's key for the card's 0.52 cd/m^2: 0.113, 0.67 EV under middle grey.
        assert!((meter_card_cd() - 0.5157).abs() < 1e-3);
        assert!((adapted_key(meter_card_cd()) - 0.1128).abs() < 1e-3, "{}", adapted_key(meter_card_cd()));
        assert!((adapted_key(1.25) - 0.18).abs() < 2e-3, "middle grey near 1.25 cd/m^2");
        assert!(adapted_key(0.05) < adapted_key(0.5) && adapted_key(1e4) == MIDDLE_GREY);
        for o in [FrameOptions::default(), CINE] {
            for kind in [crate::Kind::Clear, crate::Kind::Overcast, crate::Kind::Damp] {
                let (mut f, mut t) = frame_at(22.0, kind, o);
                let g = apply(&mut f, &mut t, o.cinematic);
                assert!((g - full_night_gain()).abs() < 1e-9);
                let u = t.lamp_units_per_lux;
                // the card over the key's scale shows at middle grey (the card itself at the key)
                let card = 0.18 * NIGHT_METER_LUX * u / std::f64::consts::PI / meter_key_scale();
                let d = if o.cinematic { night::cinematic_display_grey(card, t.exposure_target) } else { night::display_grey(card, t.exposure_target) } * 255.0;
                assert!((d - MIDDLE_GREY_DISPLAY_255).abs() < 0.5, "{kind:?} {}: {d}", o.cinematic);
                // the unlit ground stands for the physical lux against the lamps
                let unlit = night::ground_fill(&f, night::SIDEWALK_ENV) / u;
                assert!(unlit < 1.3 && unlit > 0.6, "{kind:?}: unlit {unlit} lux");
                // a pool's peak is ~25-50 x the unlit ground
                let ratio = crate::photometry::LAMP_POOL_PEAK_LUX / unlit;
                assert!(ratio > 22.0 && ratio < 50.0, "{kind:?}: pool / unlit {ratio}");
            }
        }
    }

    #[test]
    fn the_unlit_city_falls_darker_and_the_lamps_brighter_on_screen() {
        // clear, 21 h, the cinematic look: the natural light's display falls,
        // a lamp's rises, against the unlit-ground solve.
        let (f0, t0) = frame_at(21.0, crate::Kind::Clear, CINE);
        let (mut f, mut t) = (f0, t0);
        apply(&mut f, &mut t, true);
        let natural = t.exposure_target / t0.exposure_target;
        let lamps = natural * t.lamp_units_per_lux / t0.lamp_units_per_lux;
        assert!(natural > 0.6 && natural < 0.85, "natural x{natural}");
        assert!(lamps > 1.25 && lamps < 1.7, "lamps x{lamps}");
    }

    #[test]
    fn the_exposure_is_continuous_through_dusk() {
        let mut last: Option<f64> = None;
        for i in 0..=120 {
            let hour = 18.8 + i as f64 * 0.01;
            let (mut f, mut t) = frame_at(hour, crate::Kind::Clear, CINE);
            apply(&mut f, &mut t, true);
            if let Some(p) = last {
                let step = (t.exposure_target / p).ln().abs();
                assert!(step < 0.08, "{hour}: a step of {step} ln");
            }
            last = Some(t.exposure_target);
        }
    }

    #[test]
    fn a_far_halo_falls_as_one_over_distance() {
        assert_eq!(far_air_light_share(0.1, 0.16), 1.0);
        assert_eq!(far_air_light_share(0.16, 0.16), 1.0);
        assert!((far_air_light_share(0.32, 0.16) - 0.5).abs() < 1e-12);
        assert!((far_air_light_share(1.6, 0.16) - 0.1).abs() < 1e-12);
    }
}
