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
//!    the patch quilt in the road's own frame with sealed saw cuts and a lip,
//!    the cracks' sealant as bitumen (black, glossy, a few millimetres proud)
//!    and open cracks occluding their floor;
//!  * the gutter pan as poured concrete with its cold joint to the asphalt and
//!    the silt of its flow line;
//!  * kerbs with a rounded arris, chipped in pieces, and the concave
//!    corner's occlusion at the flow line;
//!  * markings worn through to the road in the asphalt's own texture, the
//!    road's cracks and sealant running on through them;
//!  * scored walks whose slabs settle each at its own small tilt, with dark
//!    recessed joints, a few cracked slabs and broken corners;
//!  * the plazas' paving from its scan, stochastically tiled (the random
//!    setts: no repeat, no rails), parallax-occlusion mapped near the eye and
//!    self-shadowed toward the sun (a march over the height near, the
//!    relief's mean masking far), crowns polished along the foot traffic,
//!    at the measured albedo of its material;
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
/// How much further down the face a broken-out piece of the arris reaches
/// than the rounded edge (a multiple of its radius: chips of a few
/// centimetres; a shading choice from the look of SF kerbs).
pub const ARRIS_BREAK: f64 = 2.2;
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

// ---- `--street-wear` (look::STREET_WEAR; street_pbr.wgsl `sw_*`) ----------------------
//
// The carriageway's distress laid out by its causes, as the FHWA's Distress
// Identification Manual for the LTPP (FHWA-RD-03-031) sorts it: transverse
// cracks across the road (thermal: from one edge, or across it, meandering),
// longitudinal cracks along the paving lanes' joints and in the wheel paths,
// fatigue ("alligator") cracking only in the wheel paths, in pieces under
// 0.3 m; sealed cracks under an overband of hot-poured sealant (FHWA-RD-99-147,
// "Materials and Procedures for Sealing and Filling Cracks in Asphalt-Surfaced
// Pavements": overband ("band-aid") configurations 75-125 mm wide over the
// crack; SF's are narrower, worn down by the traffic).

/// One chance of a transverse crack per this length of road (m), and that
/// chance (a shading choice: SF's mild climate cracks a road every 5-20 m).
pub const WEAR_TRANSVERSE_CELL_M: f64 = 9.0;
pub const WEAR_TRANSVERSE_SHARE: f64 = 0.7;
/// The share of a lane joint's or wheel path's length cracked along it.
pub const WEAR_LONGITUDINAL_SHARE: f64 = 0.55;
/// Fatigue cracking's pieces (m; LTPP: "many-sided, sharp-angled pieces,
/// usually less than 0.3 m on the longest side") and the share of a wheel
/// path's length it covers (a shading choice: an old street's worst stretches).
pub const WEAR_FATIGUE_CELL_M: f64 = 0.2;
pub const WEAR_FATIGUE_SHARE: f64 = 0.22;
/// The share of the cracks sealed (SF seals most), and the overband's
/// half-width (m: 50-80 mm across, the narrow end of FHWA-RD-99-147's band,
/// worn down by the traffic).
pub const WEAR_SEALED_SHARE: f64 = 0.6;
pub const WEAR_SEAL_HALF_M: f64 = 0.032;
/// Oil drips down a lane's middle: one chance of a drip's pool per cell of
/// this size across (m; 1.6 x it along), and how much a fresh pool darkens
/// the road (a shading choice: drips 3-9 cm across, black when fresh).
pub const WEAR_DRIP_CELL_M: f64 = 0.2;
pub const WEAR_DRIP_DARK: f64 = 0.45;

// ---- `--street-glare` (look::STREET_GLARE; street_pbr.wgsl `sg_*`) ----------------------
//
// Asphalt reflects as two surfaces: the oxidised binder and fines in the
// texture's valleys (rough, the road's own roughness) and the coarse
// aggregate's tops, which the traffic polishes and the binder films over
// (smoother). Toward grazing the valleys are masked by the tops (Smith's
// masking of the macrotexture, its slope spread from a dense-graded mix's
// 0.5-1 mm mean texture depth over 9.5-12.5 mm aggregate), so the share of
// what the eye sees that is polished tops grows, and the road's specular
// response with it: the low sun's glare down a road, brightest on the wheel
// paths. The tops are drawn as a smooth layer over the road (look_light.wgsl's
// coat: F0 0.04 on the surface's plane), weighted by the share of tops seen.

/// The macrotexture's slope spread (GGX alpha of the tops-and-valleys relief).
pub const GLARE_MACRO_SLOPE: f64 = 0.25;
/// The share of the road's plan that is exposed aggregate tops, between the
/// wheel paths and in them (worn to the stone), and how polished they are.
pub const GLARE_TOPS: f64 = 0.28;
pub const GLARE_TOPS_WHEEL: f64 = 0.45;
pub const GLARE_POLISH: f64 = 0.4;
pub const GLARE_POLISH_WHEEL: f64 = 0.85;
/// The polished tops' roughness (perceptual; at 0.5 and over the screen-space
/// trace leaves them to the sky: no trace over the road's whole area, which
/// cost up to a millisecond on the wheel paths at 0.46).
pub const GLARE_COAT_ROUGH: f64 = 0.5;
pub const GLARE_COAT_ROUGH_WHEEL: f64 = 0.5;
/// The coarse aggregate's size (m): near the eye, the tops polished stone by stone.
pub const GLARE_AGGREGATE_M: f64 = 0.009;
/// Granite setts' crowns polished by feet (the share of the crown's layer)
/// and their roughness; the setts' own relief spread is SP_PAVE_SLOPE.
pub const GLARE_SETT_POLISH: f64 = 0.55;
pub const GLARE_SETT_ROUGH: f64 = 0.5;

/// (`--street-glare`) The share of what the eye sees that is the macrotexture's
/// tops at the view's cosine `mu` to the surface (street_pbr.wgsl `sg_top_share`):
/// `tops` of the plan, the valleys masked by Smith's G1 (GGX) at `slope`.
pub fn glare_top_share(mu: f64, slope: f64, tops: f64) -> f64 {
    let mu = mu.clamp(0.02, 1.0);
    let t2 = (1.0 - mu * mu) / (mu * mu);
    let g1 = 2.0 / (1.0 + (1.0 + slope * slope * t2).sqrt());
    tops / (tops + (1.0 - tops) * g1)
}

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
/// How far the wet film flattens the asphalt's fine relief toward its own
/// plane (at full wetness): a film of 0.2-0.5 mm over a dense-graded mix's
/// mean texture depth of about 0.6 mm (ASTM E965) fills a little over half of
/// it (a shading choice inside that band).
pub const WET_FILL: f64 = 0.55;
/// The gutter pan's concrete soaks as the wet layer has it (no extra).
pub const GUTTER_WET_DARKEN: f64 = 1.0;
/// Thermoplastic and paint do not soak: the film darkens them only by its
/// own interface (Lekner & Dorf: a few to ten percent for a bright,
/// non-porous surface) and makes them glossy (a water film's roughness).
pub const PAINT_WET_DARKEN: f64 = 0.9;
pub const PAINT_WET_ROUGH: f64 = 0.15;

/// Crack sealant (hot-poured rubberised bitumen, ASTM D6690, laid as an
/// overband over the routed crack): fresh, its reflectance is the binder's
/// (LBNL Cool Pavements: new asphalt 0.04-0.05, the black of its binder);
/// oxidised and holding the road's dust it greys toward a weathered
/// surface's low end. Its finish is a cast film, near-smooth: glossy fresh
/// and dulled as it ages (shading choices from the look of SF's sealed
/// streets, still smoother than the open-textured binder round it).
pub const SEAL_ALBEDO: f64 = 0.04;
pub const SEAL_ALBEDO_AGED: f64 = 0.085;
pub const SEAL_ROUGH: f64 = 0.3;
pub const SEAL_ROUGH_AGED: f64 = 0.56;
/// The overband's crown over the road (m): squeegeed bands stand 1/16-1/8 in
/// proud (Caltrans Maintenance Manual, crack sealing), the middle of it.
pub const SEAL_PROUD_M: f64 = 0.0024;
/// How much of the sky an open crack's walls hide from its floor (the
/// specular occlusion's input at the crack: a few millimetres wide, as deep).
pub const CRACK_OCCLUSION: f64 = 0.7;
/// Worn marking paint: the share of a line's area worn through to the road,
/// on a lately re-striped line and on an old one (between them the
/// weathering's slow age field), and the band at the line's edges where the
/// wear is heavier (the edge of a film chips first) (shading choices from
/// the look of SF's markings).
pub const PAINT_WORN: f64 = 0.06;
pub const PAINT_WORN_OLD: f64 = 0.22;
pub const PAINT_EDGE_M: f64 = 0.012;
/// A line's finish: thermoplastic is a cast resin film with its glass beads,
/// smooth when laid and chalking as it weathers (a shading choice: the fresh
/// film smoother than the road's polished wheel paths, so a line in a low
/// sun's glare off the road glares with it and stays read).
pub const PAINT_ROUGH: f64 = 0.4;
pub const PAINT_ROUGH_AGED: f64 = 0.52;
/// The open spaces' paving at its material's reflectance (the plaza styles'
/// display tones are 0.45-0.7 linear): grey granite setts (granite's solar
/// reflectance 0.25-0.40 clean, dark grey setts soiled by foot traffic and
/// the street's dust below its low end: 0.2-0.25), clay brick pavers (LBNL Cool Pavements: brick about 0.3) and grey
/// concrete pavers (ACPA / LBNL weathered concrete 0.20-0.30, the upper end:
/// pavers are swept).
pub const SETT_ALBEDO: f64 = 0.22;
pub const BRICK_PAVER_ALBEDO: f64 = 0.28;
pub const CONCRETE_PAVER_ALBEDO: f64 = 0.3;

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
        ("SP_ARRIS_BREAK", ARRIS_BREAK),
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
        ("SP_WET_FILL", WET_FILL),
        ("SP_CONCRETE_WET_DARKEN", GUTTER_WET_DARKEN),
        ("SP_PAINT_WET_DARKEN", PAINT_WET_DARKEN),
        ("SP_PAINT_WET_ROUGH", PAINT_WET_ROUGH),
        ("SP_SEAL_ALBEDO", SEAL_ALBEDO),
        ("SP_SEAL_ALBEDO_AGED", SEAL_ALBEDO_AGED),
        ("SP_SEAL_ROUGH", SEAL_ROUGH),
        ("SP_SEAL_ROUGH_AGED", SEAL_ROUGH_AGED),
        ("SP_SEAL_PROUD_M", SEAL_PROUD_M),
        ("SP_CRACK_OCCLUSION", CRACK_OCCLUSION),
        ("SP_PAINT_WORN", PAINT_WORN),
        ("SP_PAINT_WORN_OLD", PAINT_WORN_OLD),
        ("SP_PAINT_EDGE_M", PAINT_EDGE_M),
        ("SP_PAINT_ROUGH", PAINT_ROUGH),
        ("SP_PAINT_ROUGH_AGED", PAINT_ROUGH_AGED),
        ("SP_SETT_ALBEDO", SETT_ALBEDO),
        ("SP_BRICK_PAVER_ALBEDO", BRICK_PAVER_ALBEDO),
        ("SP_CONCRETE_PAVER_ALBEDO", CONCRETE_PAVER_ALBEDO),
        ("SW_TRANSVERSE_CELL_M", WEAR_TRANSVERSE_CELL_M),
        ("SW_TRANSVERSE_SHARE", WEAR_TRANSVERSE_SHARE),
        ("SW_LONGITUDINAL_SHARE", WEAR_LONGITUDINAL_SHARE),
        ("SW_FATIGUE_CELL_M", WEAR_FATIGUE_CELL_M),
        ("SW_FATIGUE_SHARE", WEAR_FATIGUE_SHARE),
        ("SW_SEALED_SHARE", WEAR_SEALED_SHARE),
        ("SW_SEAL_HALF_M", WEAR_SEAL_HALF_M),
        ("SW_DRIP_CELL_M", WEAR_DRIP_CELL_M),
        ("SW_DRIP_DARK", WEAR_DRIP_DARK),
        ("SG_MACRO_SLOPE", GLARE_MACRO_SLOPE),
        ("SG_TOPS", GLARE_TOPS),
        ("SG_TOPS_WHEEL", GLARE_TOPS_WHEEL),
        ("SG_POLISH", GLARE_POLISH),
        ("SG_POLISH_WHEEL", GLARE_POLISH_WHEEL),
        ("SG_COAT_ROUGH", GLARE_COAT_ROUGH),
        ("SG_COAT_ROUGH_WHEEL", GLARE_COAT_ROUGH_WHEEL),
        ("SG_AGGREGATE_M", GLARE_AGGREGATE_M),
        ("SG_SETT_POLISH", GLARE_SETT_POLISH),
        ("SG_SETT_ROUGH", GLARE_SETT_ROUGH),
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

    /// The sealant is darker and smoother than the road it seals (the tar
    /// snakes catch the sky), fresh more than aged; the open spaces' paving
    /// sits inside its materials' measured bands, darker than its display tones.
    #[test]
    fn sealant_and_paving_are_their_materials() {
        assert!(SEAL_ALBEDO < SEAL_ALBEDO_AGED && SEAL_ALBEDO_AGED < ASPHALT_ALBEDO);
        assert!(SEAL_ROUGH < ASPHALT_ROUGH_POLISHED && SEAL_ROUGH < SEAL_ROUGH_AGED && SEAL_ROUGH_AGED < ASPHALT_ROUGH);
        assert!(SEAL_PROUD_M > 0.0015 && SEAL_PROUD_M < 0.0032, "1/16-1/8 in");
        assert!(PAINT_WORN < PAINT_WORN_OLD && PAINT_WORN_OLD < 0.5);
        assert!(PAINT_ROUGH < PAINT_ROUGH_AGED && PAINT_ROUGH_AGED <= ASPHALT_ROUGH_POLISHED);
        for a in [SETT_ALBEDO, BRICK_PAVER_ALBEDO, CONCRETE_PAVER_ALBEDO] {
            assert!((0.2..=0.4).contains(&a), "{a}");
        }
        let w = wgsl();
        for k in ["SP_SEAL_ALBEDO", "SP_SEAL_ROUGH", "SP_CRACK_OCCLUSION", "SP_PAINT_WORN", "SP_SETT_ALBEDO"] {
            assert!(w.contains(&format!("const {k}: f32 = ")), "{k}");
        }
    }

    /// (`--street-wear`, `--street-glare`) The distress and the polished tops
    /// inside their sources' bands: an overband 50-80 mm across, fatigue's pieces
    /// under LTPP's 0.3 m, more sealed than open; the tops' layer smoother than
    /// the road and its wheel paths no rougher than the street pass's polish,
    /// its share the plan's at normal view and growing toward grazing.
    #[test]
    fn wear_and_glare_stay_in_their_bands() {
        assert!((0.025..=0.04).contains(&WEAR_SEAL_HALF_M), "{WEAR_SEAL_HALF_M}");
        assert!(WEAR_FATIGUE_CELL_M < 0.3 && WEAR_FATIGUE_SHARE < 0.5);
        assert!(WEAR_SEALED_SHARE > 0.5 && WEAR_SEALED_SHARE < 1.0);
        assert!(WEAR_TRANSVERSE_SHARE <= 1.0 && WEAR_LONGITUDINAL_SHARE <= 1.0 && WEAR_DRIP_DARK < 0.6);
        assert!(GLARE_COAT_ROUGH_WHEEL <= GLARE_COAT_ROUGH && GLARE_COAT_ROUGH < ASPHALT_ROUGH && GLARE_COAT_ROUGH_WHEEL <= ASPHALT_ROUGH_POLISHED);
        assert!(GLARE_TOPS < GLARE_TOPS_WHEEL && GLARE_POLISH < GLARE_POLISH_WHEEL && GLARE_POLISH_WHEEL <= 1.0);
        let normal = glare_top_share(1.0, GLARE_MACRO_SLOPE, GLARE_TOPS);
        assert!((normal - GLARE_TOPS).abs() < 1e-9, "{normal}");
        let mut last = normal;
        for deg in [30.0f64, 60.0, 75.0, 85.0, 88.0] {
            let t = glare_top_share(deg.to_radians().cos(), GLARE_MACRO_SLOPE, GLARE_TOPS);
            assert!(t > last && t < 1.0, "{deg}: {t}");
            last = t;
        }
        let w = wgsl();
        for k in ["SW_TRANSVERSE_CELL_M", "SW_SEAL_HALF_M", "SW_DRIP_DARK", "SG_MACRO_SLOPE", "SG_COAT_ROUGH", "SG_SETT_ROUGH"] {
            assert!(w.contains(&format!("const {k}: f32 = ")), "{k}");
        }
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
