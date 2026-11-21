//! Light the sunlit ground bounces up onto what faces it (the ambient
//! volume's ground bounce, eo-render `ambient.wgsl` `amb_bounce`): the
//! underside of a deck, a cap beam's soffit, a girder web or a wall facing a
//! sunlit street. Pure functions of the look frame; the renderer takes the
//! open ground's share of the lower hemisphere round the normal from the
//! ambient bake and the cloud transmittance from the cloud shadow.
//!
//! The rig's flat ground lobe (`LookFrame::hemi_ground`) stands for the
//! street's bounce of sun and sky together (eo-sky `frame::apply`), as if
//! the whole lower hemisphere were sunlit street. Where the bake knows the
//! ground, the lobe's sun part (the lobe less its sky part, the ground's
//! albedo x the fill on it as the night lobe is built; never more than the
//! sunlit ground's own exitance) leaves the whole lower hemisphere, and the
//! sunlit ground's exitance from the sun comes back over the open ground's
//! share. So nothing is counted twice: over open ground a surface takes the
//! sunlit street's light, over covered ground (under a deck, in a closed
//! yard) only the lobe's sky part; facing up, with the sun down or in a
//! cloud's shadow nothing changes.

use crate::LookFrame;
use crate::math::*;
use crate::rig::key_envelope;

/// The ground's average albedo under the sun for the bounce. INFERRED from
/// the reference sheet (2.1): aged asphalt 0.15 (LBNL East Bay
/// measurements; US EPA 2012 "15-20 %"), the carriageway the larger part of a
/// street, and concrete footways 0.25-0.40 (EPA 2012: new grey concrete
/// about 0.40, weathered lower), an area blend of about 0.20.
pub const GROUND_ALBEDO: f64 = 0.20;

/// The albedo the lobe's sky part is taken at: eo-sky's
/// `frame::NIGHT_GROUND_ALBEDO`, with which the night's lobe is built from the
/// fill (the same number; eo-render's tests check that they agree).
pub const LOBE_SKY_ALBEDO: f64 = 0.18;

/// The sunlit ground's exitance from the sun alone (irradiance units, as the
/// rig's lobes): the key's irradiance on a horizontal surface x
/// `GROUND_ALBEDO`. Zero with the sun down: the key's envelope on the sun's
/// own altitude (the moon, which keys the night, bounces nothing here).
pub fn sun_exitance(f: &LookFrame) -> Rgb {
    let up = f.key_dir[1].max(0.0);
    scale3(f.key, GROUND_ALBEDO * up * key_envelope(f.model.sun.altitude_deg))
}

/// The sun's part of the rig's ground lobe: the lobe less its sky part
/// (`LOBE_SKY_ALBEDO` x the sky lobe and the ambient), never below zero.
pub fn lobe_sun(f: &LookFrame) -> Rgb {
    [0, 1, 2].map(|i| (f.hemi_ground[i] - LOBE_SKY_ALBEDO * (f.hemi_sky[i] + f.ambient[i])).max(0.0))
}

/// The irradiance the bounce changes on a surface whose lower hemisphere
/// holds `share` of open ground and `lower` in all (each 0..1, the share of a
/// whole hemisphere's cosine weight: facing straight down over open ground 1
/// and 1, a wall 0.5 and 0.5, facing up 0 and 0), under a cloud transmittance
/// `cloud` (0..1): the CPU reference of the shader's `amb_bounce` inside the
/// volume's levels. Negative where little of the lower hemisphere is open
/// ground (the lobe's sun part leaves, little comes back).
pub fn added_irradiance(f: &LookFrame, share: f64, lower: f64, cloud: f64) -> Rgb {
    let e = sun_exitance(f);
    let l = lobe_sun(f);
    let lower = lower.clamp(0.0, 1.0);
    let s = share.clamp(0.0, lower);
    [0, 1, 2].map(|i| s * e[i] * cloud - lower * l[i].min(e[i] * cloud))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bounce_follows_the_sun_and_stops_at_night() {
        let noon = LookFrame::new(12.0);
        let afternoon = LookFrame::new(15.0);
        let evening = LookFrame::new(18.0);
        let night = LookFrame::new(23.0);
        let lum = |c: Rgb| luminance(c);
        let (en, ea, ee) = (lum(sun_exitance(&noon)), lum(sun_exitance(&afternoon)), lum(sun_exitance(&evening)));
        println!(
            "sun exitance noon {en:.3} (lobe {:.3}, its sun part {:.3}), 15 h {ea:.3} (lobe {:.3}, sun part {:.3}), 18 h {ee:.3}; facing down at 15 h: open ground {:.3}, half open {:.3}, covered {:.3}",
            lum(noon.hemi_ground),
            lum(lobe_sun(&noon)),
            lum(afternoon.hemi_ground),
            lum(lobe_sun(&afternoon)),
            lum(added_irradiance(&afternoon, 1.0, 1.0, 1.0)),
            lum(added_irradiance(&afternoon, 0.5, 1.0, 1.0)),
            lum(added_irradiance(&afternoon, 0.0, 1.0, 1.0)),
        );
        assert!(en > ea && ea > ee && ee > 0.0, "the bounce falls with the sun: {en} {ea} {ee}");
        // by day the sunlit ground outshines the rig's flat lobe (the reason for the term).
        assert!(en > 2.0 * lum(noon.hemi_ground), "noon: {en} against the lobe {}", lum(noon.hemi_ground));
        // nothing at night, whatever the shares.
        assert_eq!(sun_exitance(&night), [0.0; 3]);
        for (s, l) in [(1.0, 1.0), (0.0, 1.0), (0.25, 0.5)] {
            assert_eq!(added_irradiance(&night, s, l, 1.0), [0.0; 3]);
        }
        // nothing facing up (no lower hemisphere) or under a cloud's full shadow.
        assert_eq!(added_irradiance(&noon, 0.0, 0.0, 1.0), [0.0; 3]);
        assert_eq!(added_irradiance(&noon, 1.0, 1.0, 0.0), [0.0; 3]);
        // over open ground: the sunlit ground's exitance less the lobe's sun part; over covered ground: the lobe's sun part taken.
        let (open, covered) = (added_irradiance(&noon, 1.0, 1.0, 1.0), added_irradiance(&noon, 0.0, 1.0, 1.0));
        for i in 0..3 {
            assert!((open[i] - (sun_exitance(&noon)[i] - lobe_sun(&noon)[i])).abs() < 1e-12);
            assert!((covered[i] + lobe_sun(&noon)[i]).abs() < 1e-12 && covered[i] <= 0.0);
            // never below the lobe's sky part: a surface keeps the lobe less its sun part.
            assert!(noon.hemi_ground[i] + covered[i] >= 0.0);
        }
        // linear in the open share.
        let (a, b, c) = (added_irradiance(&noon, 0.25, 1.0, 1.0), added_irradiance(&noon, 0.5, 1.0, 1.0), added_irradiance(&noon, 0.75, 1.0, 1.0));
        for i in 0..3 {
            assert!((a[i] + c[i] - 2.0 * b[i]).abs() < 1e-12);
        }
    }

    #[test]
    fn the_lobe_keeps_its_sky_part() {
        // the lobe's sun part is never more than the lobe.
        for h in [7.0, 9.0, 12.0, 15.0, 17.5, 20.0, 23.0] {
            let f = LookFrame::new(h);
            let l = lobe_sun(&f);
            for i in 0..3 {
                assert!(l[i] >= 0.0 && l[i] <= f.hemi_ground[i] + 1e-12, "{h}: {l:?} of {:?}", f.hemi_ground);
            }
        }
    }
}
