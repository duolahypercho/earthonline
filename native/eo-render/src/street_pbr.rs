//! The street's physically based surface pass (`--street-pbr`, off by
//! default; street_pbr.wgsl). Every term sits behind `SF_STREET_PBR`
//! (look::ShaderFeatures::street_pbr): without the switch the code is
//! compiled out and the frame is the one before it.
//!
//! What it draws, on the street surface the earlier stages compose
//! (draw.wgsl `street_surface`, the CC0 scans, the weathering):
//!
//!  * the carriageway at a weathered asphalt's albedo, one world-anchored
//!    macro field across every lane (no tone step at the crown), wheel paths
//!    polished smoother and lighter, an oil-drip band down each lane's middle,
//!    the patch quilt in the road's own frame with sealed saw cuts and a lip;
//!  * the gutter pan as poured concrete with its cold joint to the asphalt and
//!    the silt of its flow line;
//!  * kerbs with a rounded arris and the concave corner's occlusion at the
//!    flow line;
//!  * scored walks whose slabs settle each at its own small tilt, with dark
//!    recessed joints, a few cracked slabs and broken corners;
//!  * the plazas' paving from its scan, stochastically tiled (the random
//!    setts: no repeat, no rails), parallax-occlusion mapped near the eye and
//!    self-shadowed toward the sun (a march over the height near, the
//!    relief's mean masking far), crowns polished along the foot traffic;
//!  * in the wet: the asphalt soaked to the film's tone, water standing where
//!    the relief (ruts, birdbaths, the gutter's flow line) is deeper than
//!    what has drained, the markings wet and glossy.
//!
//! Street dimensions come from the California street standards table
//! (`eo_data::standards`): the lane, parking lane and design vehicle widths,
//! the kerb height and the scoring pitch. The rest are material values with
//! their sources below, or shading choices named as such.

/// Weathered asphalt pavement's solar reflectance: LBNL Heat Island Group
/// (Cool Pavements) gives fresh asphalt 0.04-0.06 and weathered asphalt
/// about 0.10-0.18; the street draws the middle of the weathered band.
pub const ASPHALT_ALBEDO: f64 = 0.14;
/// Aged Portland-cement concrete (the gutter pan): ACPA / LBNL measure new
/// grey concrete 0.35-0.40, weathering and traffic soil it to 0.20-0.30;
/// a gutter holds the street's silt, so the low end.
pub const GUTTER_ALBEDO: f64 = 0.21;
/// Perceptual roughness of oxidised asphalt binder over its aggregate
/// (a matte, open texture), and of wheel paths whose aggregate tops the
/// traffic has polished (still matte at normal incidence, a broad sheen
/// toward a low sun).
pub const ASPHALT_ROUGH: f64 = 0.82;
pub const ASPHALT_ROUGH_POLISHED: f64 = 0.52;
/// The share of the design vehicle's width between its wheel paths' centres
/// (a shading choice: a car's or bus's tyres run inside its body).
pub const TRACK_SHARE: f64 = 0.7;
/// The rounded arris of a kerb as a share of the kerb height (a shading
/// choice: a tooled, worn edge of a few centimetres).
pub const ARRIS_SHARE: f64 = 0.17;
/// A scored slab's settlement tilt (radians, one standard deviation-ish
/// either way), and the share of heaved slabs (a shading choice from the
/// look of SF walks: most within a degree, a few lifted by roots).
pub const SLAB_TILT_RAD: f64 = 0.012;
pub const SLAB_HEAVE_SHARE: f64 = 0.06;
/// The share of slabs with a crack across them, and with a broken corner.
pub const SLAB_CRACK_SHARE: f64 = 0.16;
pub const SLAB_CORNER_SHARE: f64 = 0.07;
/// Each slab its own pour: the batches' tone spread (+- this share), the
/// share of slabs replaced since (a newer, paler paste: new grey concrete's
/// 0.35-0.40 against an aged walk's ~0.30, ACPA / LBNL, so x1.2), and the
/// share darkened by spills (shading choices from the look of SF walks).
pub const SLAB_TONE: f64 = 0.07;
pub const SLAB_NEW_SHARE: f64 = 0.06;
pub const SLAB_NEW_TONE: f64 = 1.2;
pub const SLAB_STAINED_SHARE: f64 = 0.05;

/// The paving's relief (m) from its scan's height 0 to 1: setts' joints sit
/// 5-10 mm under their arrises and their crowns a few millimetres over them
/// (BS 7533-7 laying tolerances; the critic's 5-10 mm joint depth).
pub const PAVE_DEPTH_M: f64 = 0.02;
/// The stochastic tiling's cell (scan repeats across: 2.5 m of a 2 m scan,
/// seams few and far apart), how far a tile's barycentric weight raises its
/// stones over the others' (narrow seams), and the height band in which two
/// tiles share a pixel (the seam follows the stones' edges).
pub const HEX_CELL: f64 = 1.25;
pub const HEX_GAIN: f64 = 3.0;
pub const HEX_SOFT: f64 = 0.1;
/// The stochastic tiling takes over from the scan's own tiling over this
/// distance to 1.5 x it (m): near, whole stones as laid; beyond, where the
/// repeat would line up into rails toward the horizon, none.
pub const HEX_NEAR_M: f64 = 12.0;
/// Parallax occlusion out to this distance (m; a 2 cm relief is under a
/// pixel's parallax past it), the self-shadow march to this one (m; past it
/// the relief's mean masking), and the march's reach (m; a 2 cm stone's
/// shadow at a 4 degree sun).
pub const POM_FAR_M: f64 = 10.0;
pub const SHADOW_FAR_M: f64 = 40.0;
pub const SHADOW_REACH_M: f64 = 0.3;
/// The self-shadow's penumbra (the soft width per metre of reach: the sun's
/// disc is 0.0047, widened to what a pixel resolves of a 2 cm relief).
pub const PENUMBRA: f64 = 0.03;
/// The march's start over its own height (share of the relief): the
/// filtered height field's slope inside a pixel, so a stone never shadows
/// itself where its own mip leans.
pub const SHADOW_BIAS: f64 = 0.03;
/// The paving's RMS slope for its masking far off (Smith G1, Beckmann): the
/// joints and crowns of setts, ~14 degrees.
pub const PAVE_SLOPE: f64 = 0.25;

/// The wheel paths' mean rut depth (m): ASTM D6433 rates rutting from 6 mm
/// (low severity); most of SF's streets are under that, so 4.5 mm mean,
/// varying along the road 0.35-1.65 x as the base gave way.
pub const RUT_M: f64 = 0.0045;
/// Settled depressions ("birdbaths") anywhere on the carriageway, up to this
/// depth (m; ASTM D6433 rates depressions from 13 mm: these are unrated).
pub const BIRDBATH_M: f64 = 0.006;
/// The run-off standing in the gutter's flow line (m at the kerb, none at
/// the pan's edge), between the high spots of the pour.
pub const GUTTER_DIP_M: f64 = 0.008;
/// What has drained (m): no puddle where the relief is shallower than this
/// times (1 - the weather's puddle fill); a long rain (fill 0.6) leaves water
/// in relief over 2.6 mm, an hour after it (damp, 0.45) over 3.6 mm.
pub const POND_DRAIN_M: f64 = 0.0065;
/// Wet asphalt's reflectance is the film's and the binder's: the weathered
/// aggregate's lighter dry tone goes with the water (Lekner & Dorf 1988, "Why
/// some things are darker when wet": darkening grows with the surface's
/// porosity and albedo), so the soaked surface comes back to the scans' aged
/// asphalt (cc0.rs ASPHALT_ALBEDO) before the wet layer's own darkening.
pub fn wet_darken() -> f64 {
    crate::cc0::ASPHALT_ALBEDO as f64 / ASPHALT_ALBEDO
}
/// The gutter pan's concrete soaks as the wet layer has it (no extra).
pub const GUTTER_WET_DARKEN: f64 = 1.0;
/// Thermoplastic and paint do not soak: the film darkens them only by its
/// own interface (Lekner & Dorf: a few to ten percent for a bright,
/// non-porous surface) and makes them glossy (a water film's roughness).
pub const PAINT_WET_DARKEN: f64 = 0.9;
pub const PAINT_WET_ROUGH: f64 = 0.15;

/// The street dimensions the pass reads from the standards table.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Dims {
    /// A travel lane (HDM's urban low-speed minimum, the SF lane at a bulb-out).
    pub lane_m: f64,
    /// A parallel parking lane (the CA MUTCD parking-space marking layout).
    pub parking_m: f64,
    /// The design vehicle's width (CVC 35100).
    pub vehicle_m: f64,
}

/// The dimensions from the table (0 where an entry has no verified value:
/// the term that needs it is not drawn).
pub fn dims() -> Dims {
    static D: std::sync::OnceLock<Dims> = std::sync::OnceLock::new();
    *D.get_or_init(|| {
        let Some(t) = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::CaStandards::load(&p).ok()) else {
            return Dims::default();
        };
        Dims {
            lane_m: t.metric("travel-lane-width-state-highway-urban-low-speed-min").unwrap_or(0.0),
            parking_m: t.metric_key("parking-space-marking-layout", "laneWidth").unwrap_or(0.0),
            vehicle_m: t.metric("vehicle-width-max").unwrap_or(0.0),
        }
    })
}

/// The pass's constants (appended to the city shader with street_pbr.wgsl).
pub fn wgsl() -> String {
    let d = dims();
    let f = |v: f64| format!("{v:.6}");
    let mut s = String::from("\n// generated by street_pbr.rs wgsl\n");
    for (k, v) in [
        ("SP_ASPHALT_ALBEDO", ASPHALT_ALBEDO),
        ("SP_GUTTER_ALBEDO", GUTTER_ALBEDO),
        ("SP_ASPHALT_ROUGH", ASPHALT_ROUGH),
        ("SP_ASPHALT_ROUGH_POLISHED", ASPHALT_ROUGH_POLISHED),
        ("SP_LANE_M", d.lane_m),
        ("SP_PARKING_M", d.parking_m),
        ("SP_TRACK_M", d.vehicle_m * TRACK_SHARE),
        ("SP_ARRIS_SHARE", ARRIS_SHARE),
        ("SP_SLAB_TILT", SLAB_TILT_RAD),
        ("SP_SLAB_HEAVE_SHARE", SLAB_HEAVE_SHARE),
        ("SP_SLAB_CRACK_SHARE", SLAB_CRACK_SHARE),
        ("SP_SLAB_CORNER_SHARE", SLAB_CORNER_SHARE),
        ("SP_SLAB_TONE", SLAB_TONE),
        ("SP_SLAB_NEW_SHARE", SLAB_NEW_SHARE),
        ("SP_SLAB_NEW_TONE", SLAB_NEW_TONE),
        ("SP_SLAB_STAINED_SHARE", SLAB_STAINED_SHARE),
        ("SP_PAVE_DEPTH", PAVE_DEPTH_M),
        ("SP_HEX_CELL", HEX_CELL),
        ("SP_HEX_GAIN", HEX_GAIN),
        ("SP_HEX_SOFT", HEX_SOFT),
        ("SP_HEX_NEAR", HEX_NEAR_M),
        ("SP_POM_FAR", POM_FAR_M),
        ("SP_SHADOW_FAR", SHADOW_FAR_M),
        ("SP_SHADOW_REACH", SHADOW_REACH_M),
        ("SP_PENUMBRA", PENUMBRA),
        ("SP_SHADOW_BIAS", SHADOW_BIAS),
        ("SP_PAVE_SLOPE", PAVE_SLOPE),
        ("SP_RUT_M", RUT_M),
        ("SP_BIRDBATH_M", BIRDBATH_M),
        ("SP_GUTTER_DIP_M", GUTTER_DIP_M),
        ("SP_POND_DRAIN_M", POND_DRAIN_M),
        ("SP_WET_DARKEN", wet_darken()),
        ("SP_CONCRETE_WET_DARKEN", GUTTER_WET_DARKEN),
        ("SP_PAINT_WET_DARKEN", PAINT_WET_DARKEN),
        ("SP_PAINT_WET_ROUGH", PAINT_WET_ROUGH),
    ] {
        s.push_str(&format!("const {k}: f32 = {};\n", f(v)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_come_from_the_standards_table() {
        let d = dims();
        let t = eo_data::CaStandards::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
        assert_eq!(d.lane_m, t.metric("travel-lane-width-state-highway-urban-low-speed-min").unwrap());
        assert_eq!(d.parking_m, t.metric_key("parking-space-marking-layout", "laneWidth").unwrap());
        assert_eq!(d.vehicle_m, t.metric("vehicle-width-max").unwrap());
        assert!(d.lane_m > d.parking_m && d.parking_m > 0.0 && d.vehicle_m > SP_TRACK_FLOOR, "{d:?}");
    }

    const SP_TRACK_FLOOR: f64 = 1.0;

    #[test]
    fn constants_are_generated() {
        let w = wgsl();
        for k in ["SP_ASPHALT_ALBEDO", "SP_LANE_M", "SP_PARKING_M", "SP_TRACK_M", "SP_ARRIS_SHARE", "SP_PAVE_DEPTH", "SP_HEX_CELL", "SP_POND_DRAIN_M", "SP_WET_DARKEN"] {
            assert!(w.contains(&format!("const {k}: f32 = ")), "{k}");
        }
        assert!((0.10..=0.18).contains(&ASPHALT_ALBEDO), "inside the weathered band");
    }

    /// The soaked asphalt comes back to the scans' aged asphalt (no lifted
    /// blacks in the rain), and puddles stand in ruts only past what drains.
    #[test]
    fn the_wet_street_is_the_film_s_tone_and_ruts_hold_water() {
        let wet = ASPHALT_ALBEDO * wet_darken();
        assert!((wet - crate::cc0::ASPHALT_ALBEDO as f64).abs() < 1e-6, "{wet}");
        assert!(wet_darken() < 1.0);
        // the deepest rut holds water in a long rain (fill 0.6) and an hour after it (0.45), the mean one only in the rain
        let deepest = RUT_M * 1.65;
        assert!(deepest > POND_DRAIN_M * (1.0 - 0.45) && deepest > POND_DRAIN_M * (1.0 - 0.6));
        assert!(RUT_M > POND_DRAIN_M * (1.0 - 0.6));
        // a mean rut stays under ASTM D6433's low-severity rutting (6 mm)
        assert!(RUT_M < 0.006);
    }
}
