//! The sun's penumbra (eo-render `--soft-shadows`, look_light.wgsl
//! `lk_cascade_soft`): the sun is a disc 0.5334 degrees across (the drawn
//! disc's, `LookFrame::disc_radius`), so a shadow's edge is not a line but a
//! band whose width grows with the distance from the edge that casts it to
//! the surface it falls on, measured along the sunbeam:
//!
//! ```text
//!     w = d x tan(0.5334 deg) = 0.00931 d
//! ```
//!
//! Sharp where an object meets the ground (d -> 0), 9 cm wide 10 m on, 0.8 m
//! where a deck 10 m up throws its edge 87 m along a sun 6.6 degrees high.
//!
//! The renderer estimates d per pixel from the cascade's depth map
//! (percentage-closer soft shadows, Fernando, SIGGRAPH 2005 sketch): the
//! average depth of the casters in a search disc round the point (the disc
//! as wide as the widest penumbra kept, `MAX_SPREAD_M` of the half width),
//! then the comparison filtered over a disc of the penumbra's half width
//! (never wider than the search, so the band is the same either side of the
//! edge: at most `MAX_SPREAD_M` each way). A point with no caster in the
//! search is lit and one with casters at every tap is in the umbra; only the
//! band runs the filter.
//! The receiver's own plane (its geometric normal in the cascade's clip
//! space) sets the depth each tap compares against, so a wide filter on a
//! slope does not shadow its own surface.

/// The sun's angular diameter, degrees (the disc the sky draws).
pub const SUN_DIAMETER_DEG: f64 = 0.5334;
/// The largest penumbra half width the search reaches for, metres: an edge
/// up to about 130 m along the beam (a deck or a cornice at a low sun).
pub const MAX_SPREAD_M: f64 = 0.6;
/// Taps of the caster search and of the filter (a rotated Vogel disc each,
/// turned per pixel and per frame; the frame's temporal resolve averages them).
pub const SEARCH_TAPS: u32 = 12;
pub const FILTER_TAPS: u32 = 16;
/// The search and the filter never span more texels than this (radius).
pub const MAX_RADIUS_TEXELS: f64 = 48.0;
/// Below this filter radius (texels) the cascade's own 3 x 3 tent is used.
pub const TENT_TEXELS: f64 = 1.0;
/// A tap's depth is moved along the receiver's plane at most this many
/// texels' worth of the cascade's depth per texel of offset: the plane is
/// followed down to about 2.4 degrees between the surface and the beam (a
/// facade raked by a low sun, nl 0.04, keeps its light; at 4, about 14
/// degrees, such a facade shadowed itself), and only a receiver nearly
/// edge-on to the sun, which the key barely lights, keeps a bounded slope.
pub const MAX_PLANE_SLOPE: f64 = 24.0;

/// tan of the sun's angular diameter.
pub fn tan_diameter() -> f64 {
    (SUN_DIAMETER_DEG.to_radians()).tan()
}

/// The penumbra's full width, metres, at `d` metres from its edge along the beam.
pub fn width_m(d: f64) -> f64 {
    d.max(0.0) * tan_diameter()
}

/// The filter's radius in texels of `texel_m` metres for an edge `d` metres up the beam.
pub fn filter_radius_texels(d: f64, texel_m: f64) -> f64 {
    (0.5 * width_m(d) / texel_m.max(1e-6)).min(MAX_RADIUS_TEXELS)
}

/// The search's radius in texels of `texel_m` metres.
pub fn search_radius_texels(texel_m: f64) -> f64 {
    (MAX_SPREAD_M / texel_m.max(1e-6)).clamp(1.0, MAX_RADIUS_TEXELS)
}

/// The share of the sun's disc a point sees past a straight edge, `x`
/// metres into the light from the edge's geometric shadow line, the edge
/// `d` metres up the beam (0.5 on the line; 0 and 1 a half width either
/// side): the disc's area past a chord, the reference a soft edge follows.
pub fn disc_share_past_edge(x: f64, d: f64) -> f64 {
    let r = 0.5 * width_m(d);
    if r <= 0.0 {
        return if x >= 0.0 { 1.0 } else { 0.0 };
    }
    let u = (x / r).clamp(-1.0, 1.0);
    // area of the unit disc with abscissa below u, over pi.
    0.5 + (u * (1.0 - u * u).sqrt() + u.asin()) / std::f64::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_follow_the_sun_disc() {
        assert!((tan_diameter() - 0.009310).abs() < 2e-6);
        assert_eq!(width_m(0.0), 0.0);
        assert!((width_m(10.0) - 0.0931).abs() < 1e-3);
        // a deck 10 m over the street at a sun 6.6 degrees high: its edge 87 m up the beam.
        let d = 10.0 / (6.6f64.to_radians()).sin();
        assert!((width_m(d) - 0.81).abs() < 0.01, "{}", width_m(d));
        // the filter in texels: that edge on a 1.2 cm cascade texel, and on a 0.8 cm one (the cap).
        assert!((filter_radius_texels(d, 0.012) - 0.5 * width_m(d) / 0.012).abs() < 1e-9);
        assert_eq!(filter_radius_texels(d, 0.008), MAX_RADIUS_TEXELS);
        assert!((filter_radius_texels(10.0, 0.012) - 3.88).abs() < 0.01);
        assert!(search_radius_texels(0.05) >= filter_radius_texels(MAX_SPREAD_M / (0.5 * tan_diameter()), 0.05) - 1e-9);
    }

    #[test]
    fn a_soft_edge_is_the_disc_past_a_chord() {
        assert_eq!(disc_share_past_edge(0.0, 50.0), 0.5);
        let r = 0.5 * width_m(50.0);
        assert!((disc_share_past_edge(r, 50.0) - 1.0).abs() < 1e-12);
        assert!(disc_share_past_edge(-r, 50.0).abs() < 1e-12);
        // monotone across the band, and symmetric about the line.
        let mut last = -1.0;
        for i in 0..=20 {
            let x = -r + 2.0 * r * i as f64 / 20.0;
            let s = disc_share_past_edge(x, 50.0);
            assert!(s >= last);
            assert!((s + disc_share_past_edge(-x, 50.0) - 1.0).abs() < 1e-12);
            last = s;
        }
        // with no distance, a hard step.
        assert_eq!(disc_share_past_edge(0.01, 0.0), 1.0);
        assert_eq!(disc_share_past_edge(-0.01, 0.0), 0.0);
    }
}
