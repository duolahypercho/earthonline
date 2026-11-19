//! The radiance probe's encoding (eo-render look_light.wgsl `lk_probe`,
//! look_camera.wgsl `cam_probe_encode`; `EO_AO_DEBUG` = 10 + term). With the
//! probe on, a surface draws one term of its scene-linear radiance (the
//! look's units) and the camera writes it log2-encoded per channel over
//! 2^`LOG2_MIN` .. 2^(`LOG2_MIN` + `LOG2_SPAN`), with no white balance,
//! exposure, tone or dither, so a capture reads back as radiance:
//! `decode_level` inverts one 8-bit channel.
//!
//! The split is of the surface's own light only. The street lamps and
//! headlamps (`sky_local_lights`), the glass clearcoat layer and the set's
//! capped-sight fog are not split: term 10 (all) includes them, the single
//! terms do not, so the terms do not add up to the whole at night, on glass,
//! or where the set's fog is thick. The aerial perspective is applied after
//! the probe (the post pass), so terms 16 (none: what remains is the aerial
//! in-scatter) and 17 (a unit white: transmittance plus in-scatter) recover it.

/// The encoding's floor, log2 of the darkest radiance it keeps.
pub const LOG2_MIN: f64 = -14.0;
/// The encoding's span in stops (an 8-bit level is `LOG2_SPAN / 255`, 0.07 stop).
pub const LOG2_SPAN: f64 = 18.0;

/// The probe's terms (`EO_AO_DEBUG` values).
pub const TERM_ALL: u32 = 10;
pub const TERM_SUN_DIFFUSE: u32 = 11;
pub const TERM_SUN_SPECULAR: u32 = 12;
pub const TERM_SKY_DIFFUSE: u32 = 13;
pub const TERM_SKY_SPECULAR: u32 = 14;
pub const TERM_BOUNCE: u32 = 15;
pub const TERM_NONE: u32 = 16;
pub const TERM_UNIT: u32 = 17;

/// One channel's encoded value (0..1), as `cam_probe_encode`.
pub fn encode(radiance: f64) -> f64 {
    ((radiance.max(1e-9).log2() - LOG2_MIN) / LOG2_SPAN).clamp(0.0, 1.0)
}

/// The radiance an encoded value (0..1) stands for.
pub fn decode(v: f64) -> f64 {
    (v.clamp(0.0, 1.0) * LOG2_SPAN + LOG2_MIN).exp2()
}

/// The radiance an 8-bit capture level stands for (the capture stores the
/// encoded value itself: on an sRGB target the camera pre-decodes it).
pub fn decode_level(level: u8) -> f64 {
    decode(level as f64 / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoding_round_trips_within_half_a_level() {
        for r in [1e-4, 0.003, 0.18, 1.0, 9.0] {
            assert!((decode(encode(r)) / r - 1.0).abs() < 1e-9, "{r}");
            let level = (encode(r) * 255.0).round() as u8;
            let stops = (decode_level(level) / r).log2().abs();
            assert!(stops <= 0.5 * LOG2_SPAN / 255.0 + 1e-9, "{r}: {stops}");
        }
        // the ends clamp
        assert_eq!(decode_level(0), LOG2_MIN.exp2());
        assert_eq!(decode_level(255), (LOG2_MIN + LOG2_SPAN).exp2());
        assert_eq!(encode(0.0), 0.0);
    }
}
