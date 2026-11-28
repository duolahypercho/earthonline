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
/// (`--matte-wheel-paths`, look::MATTE_WHEEL_PATHS) the wheel paths' polish
/// as drawn with it (a shading choice, matched to a photograph): at 0.52
/// the paths in a building's shade reflected the bright sky down the street
/// as sky-blue bands from the eye's feet to the far end (Mission at 3rd,
/// noon); a photograph of that street from the same corner (Wikimedia
/// Commons, CC0, 2011) shows the near carriageway matte and the polish only
/// as a faint sheen far down the road toward grazing, which 0.68 keeps.
pub const ASPHALT_ROUGH_POLISHED_MATTE: f64 = 0.68;
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

/// The crack web's cell (m): one crack line per few metres of road either
/// way, as SF's mild climate cracks a road every 5-20 m across it (a shading
/// choice), and the shares of its edges that crack: running across the road
/// (thermal cracks), along it (block cracking, where the road is older), at
/// a junction pad either way (the turning traffic's and the utility cuts'
/// stresses: denser).
pub const WEAR_WEB_CELL_M: f64 = 3.4;
pub const WEAR_WEB_ACROSS_SHARE: f64 = 0.8;
pub const WEAR_WEB_ALONG_SHARE: f64 = 0.4;
pub const WEAR_WEB_PAD_SHARE: f64 = 0.9;
/// An open crack's half-width (m): a hairline (most: under 3 mm across, LTPP's
/// low severity), a few millimetres (some) and up to a couple of centimetres
/// (a few: LTPP's moderate severity, 6-19 mm, with spalled lips), and those
/// shares; how dark the crack's floor and its broken-out lips read.
pub const WEAR_HAIR_HALF_M: f64 = 0.0011;
pub const WEAR_OPEN_HALF_M: f64 = 0.004;
pub const WEAR_WIDE_HALF_M: f64 = 0.009;
pub const WEAR_OPEN_SHARE: f64 = 0.25;
pub const WEAR_WIDE_SHARE: f64 = 0.08;
pub const WEAR_CRACK_DARK: f64 = 0.8;
pub const WEAR_SPALL_DARK: f64 = 0.35;
/// The dirty margin along an open crack (m, half-width: grit and water held
/// at its lips over a centimetre or two) and how much it darkens the road.
pub const WEAR_MARGIN_M: f64 = 0.012;
pub const WEAR_MARGIN_DARK: f64 = 0.34;
/// Fatigue cracking's pieces (m; LTPP: "many-sided, sharp-angled pieces,
/// usually less than 0.3 m on the longest side") and the share of a wheel
/// path's length it covers (a shading choice: an old street's worst stretches).
pub const WEAR_FATIGUE_CELL_M: f64 = 0.2;
pub const WEAR_FATIGUE_SHARE: f64 = 0.22;
/// The share of the cracks sealed (SF seals the wider and older ones; the
/// hairlines of the web are left), and the overband's half-width (m: 50-80
/// mm across, the narrow end of FHWA-RD-99-147's band, worn down by the traffic).
pub const WEAR_SEALED_SHARE: f64 = 0.3;
/// How far the cracks are drawn (m): their lines are under a pixel past it
/// on the cards' views (a cost choice; the weathering's web went to 170 m).
pub const WEAR_CRACK_FAR_M: f64 = 110.0;
pub const WEAR_SEAL_HALF_M: f64 = 0.032;
/// The sealant's finish with the web (perceptual roughness): fresh, a cast
/// film (as SEAL_ROUGH); oxidised, its surface crazed and holding the
/// street's grit, nearly as matte as the road (a shading choice: the street
/// pass's aged 0.56 drew old bands as bright ribbons toward a low sun).
pub const WEAR_SEAL_ROUGH: f64 = 0.34;
pub const WEAR_SEAL_ROUGH_AGED: f64 = 0.7;
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
/// A marking wider than these (m) is a coloured surfacing (a bike or bus lane:
/// an anti-skid resin carrying its own aggregate, matte), not a line's cast
/// film: its layer this much less (a crosswalk's bars, 0.3-0.6 m, mostly kept).
pub const GLARE_WIDE_MIN_M: f64 = 0.7;
pub const GLARE_WIDE_MAX_M: f64 = 1.2;
pub const GLARE_WIDE_MATTE: f64 = 0.7;
pub const GLARE_SETT_ROUGH: f64 = 0.5;

// ---- `--street-aggregate` (look::STREET_AGGREGATE; street_pbr.wgsl `sa_*`) --------------
//
// A dense-graded hot mix seen from a few metres: the coarse aggregate's stones
// set in the mastic (binder, fines and the street's dust), each stone its own
// rock, its top worn out of the binder film by the traffic (most in the wheel
// paths) and polished, its fractured faces tilted each its own way, its
// arris rounded down into the mastic. SF's streets are paved with Caltrans
// Type A hot mix at 1/2 in (12.5 mm) maximum aggregate (Caltrans Standard
// Specifications section 39; SF Public Works' standard asphalt section): the
// stones that show at the surface are the 4.75-12.5 mm fraction, 6-12 mm
// across, set at about one per 9.5 mm (the 3/8 in sieve). The layer is drawn
// where a stone spans a few pixels and goes to its mean (every term divided
// by the layout's measured mean, `aggregate_means`) as it shrinks, so the
// road's calibrated tone and finish hold at every distance.

/// The stones' pitch (m): a Voronoi cell per coarse stone.
pub const AGG_STONE_M: f64 = 0.0095;
/// The share of cells holding a coarse stone at the surface (the rest a
/// pocket of mastic and fines; a dense-graded mix shows about four in five).
pub const AGG_STONE_SHARE: f64 = 0.8;
/// The mastic between two stones (half of it on each side), as a share of
/// the pitch: from stones nearly touching to a few millimetres of fines.
pub const AGG_GAP_MIN: f64 = 0.02;
pub const AGG_GAP_MAX: f64 = 0.1;
/// The rounded arris's width (share of the pitch).
pub const AGG_EDGE: f64 = 0.08;
/// The share of a stone's top worn out of its binder film, between the wheel
/// paths and in them (a shading choice: an SF street ten to twenty years
/// down, the film worn off half the tops and nearly all in the tracks).
pub const AGG_EXPOSED: f64 = 0.45;
pub const AGG_EXPOSED_WHEEL: f64 = 0.85;
/// Reflectances: the binder film on a stone (LBNL Cool Pavements: fresh
/// asphalt 0.04-0.06, its binder), the mastic (binder, fines, dust: an aged
/// surface's low end) and the rocks the Bay Area quarries crush (shading
/// values inside the published bands of the USGS spectral library's rock
/// samples: greywacke and sandstone 0.2-0.3, basalt and greenstone about
/// 0.1, pale chert and quartzite 0.3-0.4, red chert about 0.2).
pub const AGG_FILM_ALBEDO: f64 = 0.055;
pub const AGG_MASTIC_ALBEDO: f64 = 0.09;
pub const AGG_GREY: f64 = 0.23;
pub const AGG_DARK: f64 = 0.11;
pub const AGG_PALE: f64 = 0.33;
pub const AGG_RED: f64 = 0.19;
/// The lithologies' shares (the rest greywacke).
pub const AGG_DARK_SHARE: f64 = 0.22;
pub const AGG_PALE_SHARE: f64 = 0.13;
pub const AGG_RED_SHARE: f64 = 0.1;
/// Each lithology's tint (linear RGB over its luminance-ish tone).
pub const AGG_GREY_TINT: [f64; 3] = [1.0, 0.98, 0.94];
pub const AGG_DARK_TINT: [f64; 3] = [0.95, 1.0, 1.01];
pub const AGG_PALE_TINT: [f64; 3] = [1.0, 0.99, 0.96];
pub const AGG_RED_TINT: [f64; 3] = [1.17, 0.93, 0.8];
/// Finishes (perceptual roughness): a polished top, between the tracks and
/// in them (the "polished aggregate" of skid-resistance studies), the film
/// on an unworn top, and the open mastic.
pub const AGG_TOP_ROUGH: f64 = 0.5;
pub const AGG_TOP_ROUGH_WHEEL: f64 = 0.36;
pub const AGG_FILM_ROUGH: f64 = 0.62;
pub const AGG_MASTIC_ROUGH: f64 = 0.92;
/// The relief (slopes of the normal's lean): a fractured face's tilt (about
/// 9 degrees either way), the top's crown, and the arris falling into the
/// mastic (the mean texture depth of 0.5-1 mm over a stone's edge).
pub const AGG_FACET: f64 = 0.16;
pub const AGG_CROWN: f64 = 0.3;
pub const AGG_ARRIS_SLOPE: f64 = 0.8;
/// The mastic's sky occlusion under the tops (a shading choice: a millimetre down between stones).
pub const AGG_MASTIC_AO: f64 = 0.85;
/// How much of the asphalt scans' relief the aggregate takes the place of
/// (their blotchy 5-10 cm normal field: what the scans are at the road's
/// scale is the mean of the aggregate, which this layer now draws).
pub const AGG_SCAN_RELIEF_OUT: f64 = 0.65;
/// (`--aged-aggregate`, look::AGED_AGGREGATE) An old street's grime film (tyre
/// rubber, brake dust, soil) over stones and mastic alike: each stone's tone
/// keeps this share of its departure from the layout's mean, the rest the
/// mean's own. A shading choice matched to a photograph: Mission at 3rd (the
/// same corner, CC0, 2011) shows the shaded carriageway 2-4 m from the eye a
/// near-even grey, its fine contrast (linear luminance less a 6 px blur, its
/// spread over its mean) 0.13, where the stones' full tones drew 0.76.
pub const AGG_AGED_TONE: f64 = 0.3;
/// (`--aged-aggregate`) The stones' tones resolved only while a stone spans a
/// few pixels or more across the view: whole up to a footprint of this share
/// of the pitch (a stone over ten pixels), gone at the second (four pixels),
/// as a camera's lens and sensor take a three-pixel grain's contrast down (the
/// relief and finish keep their own fade). Without it the tones held to a
/// stone of a pixel, the tops widened by the footprint brighter than the
/// layout's mean (the street 16% lighter than its calibrated tone at 3 m).
pub const AGG_AGED_NEAR: f64 = 0.1;
pub const AGG_AGED_FAR: f64 = 0.25;

// ---- `--plaza-setts` (look::PLAZA_SETTS; street_pbr.wgsl `ss_*`) ------------------------
//
// The grey-stone plazas' cobble scan (cobblestone_floor_08: split granite
// setts in sand) with what its one patch cannot carry over a plaza: the
// joints' fill at a level of its own along each stretch (swept in nearly
// flush, or washed out to the stones' feet), drawn as its own material
// (jointing sand, packed grit and soil, moss) where it lies over the scan's
// floor, the parallax ray stopping on it; each sett's own wear over the tops;
// the crowns (the scan's height over its local mean) cleaned and polished by
// soles. Heights are in the scan's units (0 its deepest joint, 1 its highest
// crown, over SP_PAVE_DEPTH): measured on the 2k displacement map, the joints'
// floor lies under 0.35 (15 % of the plan), the tops over 0.5 (61 %).

/// The fill's level, lowest and highest (scan height units: 0.2 just over the
/// joints' floor, 0.33 up to the foot of the stones' rounded sides, which
/// start at 0.35; a fill over them would sink the setts in a bed), and the
/// stretch over which it changes (m: a few setts; a shading choice).
pub const SETT_FILL_LOW: f64 = 0.2;
pub const SETT_FILL_HIGH: f64 = 0.33;
pub const SETT_FILL_M: f64 = 0.83;
/// The fill's materials (linear albedo): kiln-dried jointing sand dulled by
/// the street's grit (dry sand 0.35-0.45, Oke's "Boundary Layer Climates"
/// table 1.1; less in a joint's dirt), dark packed soil (0.05-0.15, the same
/// table), and moss (green, ~0.05-0.08 in the visible).
pub const SETT_SAND: [f64; 3] = [0.23, 0.215, 0.18];
pub const SETT_SOIL: [f64; 3] = [0.085, 0.075, 0.06];
pub const SETT_MOSS: [f64; 3] = [0.045, 0.07, 0.028];
/// The scan's own joint luminance (linear, its heights under 0.35 on the 2k
/// diffuse map: sand and grit), which the fill's grain is taken over.
pub const SETT_SCAN_JOINT_LUMA: f64 = 0.17;
/// The most of a damp, unswept stretch's fill that is moss, and the fill's sky
/// occlusion at its deepest (a joint 1.5 cm under the crowns, 1-2 cm wide:
/// shading choices).
pub const SETT_MOSS_COVER: f64 = 0.7;
pub const SETT_FILL_AO: f64 = 0.5;
/// The local mean the crowns stand over: the scan's height blurred by this
/// footprint multiple (about a stone across at the scan's scale).
pub const SETT_BLUR: f64 = 6.0;
/// Each sett's wear and soiling: a field at the stones' scale (m) over the
/// tops, its tone spread (+- this share), and the crowns' cleaning (lighter)
/// and polish (smoother) where the soles run (shading choices).
pub const SETT_STONE_M: f64 = 0.23;
pub const SETT_STONE_TONE: f64 = 0.12;
pub const SETT_CROWN_CLEAN: f64 = 0.1;
pub const SETT_CROWN_POLISH: f64 = 0.35;

// ---- `--walk-relief` (look::WALK_RELIEF; street_pbr.wgsl `wr_*`) ---------------------------
//
// The kerbs as cast concrete in segments, spalled at random along the arris,
// with a silt and water line at their foot; the walk's tooled joints in
// relief between slabs that each settled to a height of their own. The
// segment's length is a count of the walk's scoring squares (the registry's
// 'sidewalk-scoring-grid-sf' pitch: the kerb's joints fall on the walk's
// lines); the rest are material and wear values, named as shading choices.

/// The kerb's cast segments: a contraction joint at every third scoring line
/// (about 2.7 m: cast-in-place kerb joints run 3-4.5 m, precast units 0.9-1
/// m; a shading choice between them that keeps the joints on the walk's), its
/// gap's half-width (m), each segment's tone spread (+- this share) and yaw
/// out of line (radians).
pub const KERB_SEG_SQUARES: f64 = 3.0;
pub const KERB_JOINT_HALF_M: f64 = 0.003;
pub const KERB_TONE: f64 = 0.1;
pub const KERB_YAW: f64 = 0.012;
/// Each segment's nose worn to its own radius: this range times the street
/// pass's arris (ARRIS_SHARE of the kerb height).
pub const KERB_NOSE_MIN: f64 = 0.6;
pub const KERB_NOSE_MAX: f64 = 1.8;
/// Spalls: one cell (m) in this share holds one, its half-length between
/// these (m: breaks 4-20 cm long), its fracture this much lighter than the
/// weathered skin (fresh paste and aggregate).
pub const KERB_SPALL_CELL_M: f64 = 0.6;
pub const KERB_SPALL_SHARE: f64 = 0.45;
pub const KERB_SPALL_MIN_M: f64 = 0.03;
pub const KERB_SPALL_MAX_M: f64 = 0.11;
pub const KERB_SPALL_LIGHT: f64 = 0.35;
/// The share of spalls recent enough to be lighter (the rest hold the street's
/// dirt in their open fracture: this much darker).
pub const KERB_SPALL_FRESH: f64 = 0.25;
pub const KERB_SPALL_DIRT: f64 = 0.18;
/// The silt and water line at the face's foot: its mean height (m) and how
/// much darker the silt is (a shading choice from SF kerbs: the run-off's
/// line a few centimetres up).
pub const KERB_TIDE_M: f64 = 0.035;
pub const KERB_TIDE_DARK: f64 = 0.55;
/// The walk's tooled joints: the groove's half-width and depth (m: an edger's
/// 6-10 mm groove a quarter of the slab deep, ACI 302.1R's contraction joint
/// depth), the tooled nose's radius (m: 6-10 mm edgers), and the slabs'
/// settlement lip either way and a heaved slab's lift (m: ADA's 1/4 in
/// vertical change is the trip-hazard line SF repairs past; roots lift
/// slabs a centimetre or more).
pub const WALK_JOINT_HALF_M: f64 = 0.005;
pub const WALK_JOINT_DEPTH_M: f64 = 0.02;
pub const WALK_NOSE_M: f64 = 0.008;
pub const WALK_LIP_M: f64 = 0.0025;
pub const WALK_HEAVE_M: f64 = 0.012;
/// A slab replaced with cold-patch asphalt settles under its neighbours (m).
pub const WALK_PATCH_SINK_M: f64 = 0.006;
/// The groove's floor (its dirt and shade: tone and sky occlusion), the far
/// riser's sky occlusion inside the groove and above the near slab (its lip).
pub const WALK_FLOOR_TONE: f64 = 0.3;
pub const WALK_FLOOR_AO: f64 = 0.25;
/// The far riser's tone inside the groove (its face in the groove's shade and
/// dirt: the sub-pixel cavity's darkening carried in the albedo, as a cavity
/// map would, since the frame's ambient terms read the occlusion only weakly).
pub const WALK_RISER_TONE: f64 = 0.55;
/// The band either side of a joint over which the slabs' own terms fade (m: the
/// along coordinate's noise, a few centimetres at the far end of a block).
pub const WALK_SEAM_M: f64 = 0.03;
/// The dirt soles grind into the tooled noses and a margin past them (m), and
/// its tone (what keeps an old joint legible as a line down the walk).
pub const WALK_MARGIN_M: f64 = 0.012;
pub const WALK_MARGIN_TONE: f64 = 0.8;
pub const WALK_RISER_AO: f64 = 0.45;
pub const WALK_LIP_AO: f64 = 0.85;
/// The steepest ray descent the joint's profile is taken at (looking down
/// along a joint: its plan, no parallax).
pub const WALK_T_MAX: f64 = 40.0;

/// The shader's hash (draw.wgsl `st_hash`), for measuring the layout here.
fn st_hash(p: [f32; 2]) -> f32 {
    let fr = |v: f32| v - v.floor();
    let q = [fr(p[0] * 0.1031), fr(p[1] * 0.1030)];
    let d = q[0] * (q[1] + 33.33) + q[1] * (q[0] + 33.33);
    let r = [q[0] + d, q[1] + d];
    fr((r[0] + r[1]) * r[0])
}

/// One sample of the aggregate layout (street_pbr.wgsl `sa_aggregate`, its
/// near terms without the pixel's widening): (stone cover, exposed-top
/// cover, the stone's tone as linear RGB) at `p` in pitches.
fn aggregate_sample(p: [f32; 2], expo_mean: f32) -> (f32, f32, [f32; 3]) {
    let b = [p[0].floor(), p[1].floor()];
    let (mut f1, mut f2) = (1e9f32, 1e9f32);
    let (mut c1, mut c2, mut k1) = ([0f32; 2], [0f32; 2], [0f32; 2]);
    for j in -1..=1 {
        for i in -1..=1 {
            let c = [b[0] + i as f32, b[1] + j as f32];
            let pt = [c[0] + 0.1 + 0.8 * st_hash([c[0] + 2.3, c[1] + 8.1]), c[1] + 0.1 + 0.8 * st_hash([c[0] + 6.7, c[1] + 3.9])];
            let d = [p[0] - pt[0], p[1] - pt[1]];
            let dd = d[0] * d[0] + d[1] * d[1];
            if dd < f1 {
                f2 = f1;
                c2 = c1;
                f1 = dd;
                c1 = pt;
                k1 = c;
            } else if dd < f2 {
                f2 = dd;
                c2 = pt;
            }
        }
    }
    let _ = f2;
    let mut e = [c2[0] - c1[0] + 1e-6, c2[1] - c1[1]];
    let el = (e[0] * e[0] + e[1] * e[1]).sqrt();
    e = [e[0] / el, e[1] / el];
    let mid = [0.5 * (c1[0] + c2[0]), 0.5 * (c1[1] + c2[1])];
    let inner = (mid[0] - p[0]) * e[0] + (mid[1] - p[1]) * e[1];
    let h = |o: [f32; 2]| st_hash([k1[0] + o[0], k1[1] + o[1]]);
    let (h0, h1, h2, h3) = (h([4.1, 1.7]), h([9.3, 5.2]), h([1.1, 7.7]), h([5.9, 2.6]));
    let ss = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let gap = AGG_GAP_MIN as f32 + (AGG_GAP_MAX - AGG_GAP_MIN) as f32 * h1;
    let m = if h0 < AGG_STONE_SHARE as f32 { ss(gap, gap + AGG_EDGE as f32, inner) } else { 0.0 };
    let (tone, tint) = if h2 < AGG_DARK_SHARE as f32 {
        (AGG_DARK, AGG_DARK_TINT)
    } else if h2 < (AGG_DARK_SHARE + AGG_PALE_SHARE) as f32 {
        (AGG_PALE, AGG_PALE_TINT)
    } else if h2 < (AGG_DARK_SHARE + AGG_PALE_SHARE + AGG_RED_SHARE) as f32 {
        (AGG_RED, AGG_RED_TINT)
    } else {
        (AGG_GREY, AGG_GREY_TINT)
    };
    let t = tone as f32 * (0.85 + 0.3 * h3);
    let inner_n = ((inner - gap) / (0.5 - gap)).clamp(0.0, 1.0);
    let expo = (expo_mean + (h3 - 0.5) * 0.5).clamp(0.0, 1.0);
    let ex = m * ss(1.0 - expo.sqrt() - 0.08, 1.0 - expo.sqrt() + 0.08, inner_n);
    (m, ex, [t * tint[0] as f32, t * tint[1] as f32, t * tint[2] as f32])
}

/// `aggregate_means` between the tracks (AGG_EXPOSED, polish 0) and in them
/// (AGG_EXPOSED_WHEEL, polish 1), written out (the test
/// `the_aggregate_averages_to_weathered_asphalt` measures them again).
pub const AGG_MEANS: [([f64; 3], f64, f64, f64); 2] = [
    ([0.12038539964444765, 0.11855698706342105, 0.11598795633427789], 0.7271058790312914, 0.9285496144920468, 0.29829076655337583),
    ([0.1492969290532521, 0.1463453334343689, 0.14222029987935714], 0.6364667495738108, 0.9285496144920468, 0.48628469785557044),
];

/// The layout's means at the binder film's worn share `expo_mean` (measured
/// over 96 x 96 pitches): (albedo RGB, roughness at the tracks' polish
/// `wheel`, the mastic's occlusion, the exposed tops' cover).
pub fn aggregate_means(expo_mean: f64, wheel: f64) -> ([f64; 3], f64, f64, f64) {
    let n = 96;
    let s = 6;
    let (mut alb, mut rough, mut ao, mut tops) = ([0f64; 3], 0f64, 0f64, 0f64);
    let top_rough = AGG_TOP_ROUGH + (AGG_TOP_ROUGH_WHEEL - AGG_TOP_ROUGH) * wheel;
    for y in 0..n * s {
        for x in 0..n * s {
            let p = [(x as f32 + 0.5) / s as f32, (y as f32 + 0.5) / s as f32];
            let (m, ex, t) = aggregate_sample(p, expo_mean as f32);
            let (m, ex) = (m as f64, ex as f64);
            for c in 0..3 {
                let stone = AGG_FILM_ALBEDO + (t[c] as f64 - AGG_FILM_ALBEDO) * (ex / m.max(1e-6)).min(1.0);
                alb[c] += AGG_MASTIC_ALBEDO + (stone - AGG_MASTIC_ALBEDO) * m;
            }
            let stone_r = AGG_FILM_ROUGH + (top_rough - AGG_FILM_ROUGH) * (ex / m.max(1e-6)).min(1.0);
            rough += AGG_MASTIC_ROUGH + (stone_r - AGG_MASTIC_ROUGH) * m;
            ao += AGG_MASTIC_AO + (1.0 - AGG_MASTIC_AO) * m;
            tops += ex;
        }
    }
    let k = 1.0 / (n * s * n * s) as f64;
    ([alb[0] * k, alb[1] * k, alb[2] * k], rough * k, ao * k, tops * k)
}

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
        ("SP_ASPHALT_ROUGH_POLISHED_MATTE", ASPHALT_ROUGH_POLISHED_MATTE),
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
        ("SW_WEB_CELL_M", WEAR_WEB_CELL_M),
        ("SW_WEB_ACROSS_SHARE", WEAR_WEB_ACROSS_SHARE),
        ("SW_WEB_ALONG_SHARE", WEAR_WEB_ALONG_SHARE),
        ("SW_WEB_PAD_SHARE", WEAR_WEB_PAD_SHARE),
        ("SW_HAIR_HALF_M", WEAR_HAIR_HALF_M),
        ("SW_OPEN_HALF_M", WEAR_OPEN_HALF_M),
        ("SW_WIDE_HALF_M", WEAR_WIDE_HALF_M),
        ("SW_OPEN_SHARE", WEAR_OPEN_SHARE),
        ("SW_WIDE_SHARE", WEAR_WIDE_SHARE),
        ("SW_CRACK_DARK", WEAR_CRACK_DARK),
        ("SW_SPALL_DARK", WEAR_SPALL_DARK),
        ("SW_MARGIN_M", WEAR_MARGIN_M),
        ("SW_MARGIN_DARK", WEAR_MARGIN_DARK),
        ("SW_FATIGUE_CELL_M", WEAR_FATIGUE_CELL_M),
        ("SW_FATIGUE_SHARE", WEAR_FATIGUE_SHARE),
        ("SW_SEALED_SHARE", WEAR_SEALED_SHARE),
        ("SW_CRACK_FAR_M", WEAR_CRACK_FAR_M),
        ("SW_SEAL_HALF_M", WEAR_SEAL_HALF_M),
        ("SW_SEAL_ROUGH", WEAR_SEAL_ROUGH),
        ("SW_SEAL_ROUGH_AGED", WEAR_SEAL_ROUGH_AGED),
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
        ("SG_WIDE_MIN_M", GLARE_WIDE_MIN_M),
        ("SG_WIDE_MAX_M", GLARE_WIDE_MAX_M),
        ("SG_WIDE_MATTE", GLARE_WIDE_MATTE),
        ("SG_SETT_ROUGH", GLARE_SETT_ROUGH),
        ("SA_STONE_M", AGG_STONE_M),
        ("SA_AGED_TONE", AGG_AGED_TONE),
        ("SA_AGED_NEAR", AGG_AGED_NEAR),
        ("SA_AGED_FAR", AGG_AGED_FAR),
        ("SA_STONE_SHARE", AGG_STONE_SHARE),
        ("SA_GAP_MIN", AGG_GAP_MIN),
        ("SA_GAP_MAX", AGG_GAP_MAX),
        ("SA_EDGE", AGG_EDGE),
        ("SA_EXPOSED", AGG_EXPOSED),
        ("SA_EXPOSED_WHEEL", AGG_EXPOSED_WHEEL),
        ("SA_FILM_ALBEDO", AGG_FILM_ALBEDO),
        ("SA_MASTIC_ALBEDO", AGG_MASTIC_ALBEDO),
        ("SA_GREY", AGG_GREY),
        ("SA_DARK", AGG_DARK),
        ("SA_PALE", AGG_PALE),
        ("SA_RED", AGG_RED),
        ("SA_DARK_SHARE", AGG_DARK_SHARE),
        ("SA_PALE_SHARE", AGG_PALE_SHARE),
        ("SA_RED_SHARE", AGG_RED_SHARE),
        ("SA_TOP_ROUGH", AGG_TOP_ROUGH),
        ("SA_TOP_ROUGH_WHEEL", AGG_TOP_ROUGH_WHEEL),
        ("SA_FILM_ROUGH", AGG_FILM_ROUGH),
        ("SA_MASTIC_ROUGH", AGG_MASTIC_ROUGH),
        ("SA_FACET", AGG_FACET),
        ("SA_CROWN", AGG_CROWN),
        ("SA_ARRIS_SLOPE", AGG_ARRIS_SLOPE),
        ("SA_MASTIC_AO", AGG_MASTIC_AO),
        ("SA_SCAN_RELIEF_OUT", AGG_SCAN_RELIEF_OUT),
        ("WR_KERB_SQUARES", KERB_SEG_SQUARES),
        ("WR_KERB_JOINT_M", KERB_JOINT_HALF_M),
        ("WR_KERB_TONE", KERB_TONE),
        ("WR_KERB_YAW", KERB_YAW),
        ("WR_NOSE_MIN", KERB_NOSE_MIN),
        ("WR_NOSE_MAX", KERB_NOSE_MAX),
        ("WR_SPALL_CELL_M", KERB_SPALL_CELL_M),
        ("WR_SPALL_SHARE", KERB_SPALL_SHARE),
        ("WR_SPALL_MIN_M", KERB_SPALL_MIN_M),
        ("WR_SPALL_MAX_M", KERB_SPALL_MAX_M),
        ("WR_SPALL_LIGHT", KERB_SPALL_LIGHT),
        ("WR_SPALL_FRESH", KERB_SPALL_FRESH),
        ("WR_SPALL_DIRT", KERB_SPALL_DIRT),
        ("WR_TIDE_M", KERB_TIDE_M),
        ("WR_TIDE_DARK", KERB_TIDE_DARK),
        ("WR_JOINT_HALF_M", WALK_JOINT_HALF_M),
        ("WR_JOINT_DEPTH_M", WALK_JOINT_DEPTH_M),
        ("WR_NOSE_M", WALK_NOSE_M),
        ("WR_LIP_M", WALK_LIP_M),
        ("WR_HEAVE_M", WALK_HEAVE_M),
        ("WR_PATCH_SINK_M", WALK_PATCH_SINK_M),
        ("WR_FLOOR_TONE", WALK_FLOOR_TONE),
        ("WR_RISER_TONE", WALK_RISER_TONE),
        ("WR_SEAM_M", WALK_SEAM_M),
        ("WR_MARGIN_M", WALK_MARGIN_M),
        ("WR_MARGIN_TONE", WALK_MARGIN_TONE),
        ("WR_FLOOR_AO", WALK_FLOOR_AO),
        ("WR_RISER_AO", WALK_RISER_AO),
        ("WR_LIP_AO", WALK_LIP_AO),
        ("WR_T_MAX", WALK_T_MAX),
        ("SS_FILL_LOW", SETT_FILL_LOW),
        ("SS_FILL_HIGH", SETT_FILL_HIGH),
        ("SS_FILL_M", SETT_FILL_M),
        ("SS_SCAN_JOINT_LUMA", SETT_SCAN_JOINT_LUMA),
        ("SS_MOSS_COVER", SETT_MOSS_COVER),
        ("SS_FILL_AO", SETT_FILL_AO),
        ("SS_BLUR", SETT_BLUR),
        ("SS_STONE_M", SETT_STONE_M),
        ("SS_STONE_TONE", SETT_STONE_TONE),
        ("SS_CROWN_CLEAN", SETT_CROWN_CLEAN),
        ("SS_CROWN_POLISH", SETT_CROWN_POLISH),
    ] {
        s.push_str(&format!("const {k}: f32 = {};\n", f(v)));
    }
    let v3 = |v: [f64; 3]| format!("vec3<f32>({}, {}, {})", f(v[0]), f(v[1]), f(v[2]));
    for (k, v) in [
        ("SA_GREY_TINT", AGG_GREY_TINT),
        ("SA_DARK_TINT", AGG_DARK_TINT),
        ("SA_PALE_TINT", AGG_PALE_TINT),
        ("SA_RED_TINT", AGG_RED_TINT),
        ("SS_SAND", SETT_SAND),
        ("SS_SOIL", SETT_SOIL),
        ("SS_MOSS", SETT_MOSS),
    ] {
        s.push_str(&format!("const {k}: vec3<f32> = {};\n", v3(v)));
    }
    // the layout's measured means between the tracks (0) and in them (1): every term over its mean
    for (i, (alb, rough, ao, tops)) in AGG_MEANS.iter().enumerate() {
        s.push_str(&format!("const SA_MEAN_ALB{i}: vec3<f32> = {};\n", v3(*alb)));
        s.push_str(&format!("const SA_MEAN_ROUGH{i}: f32 = {};\n", f(*rough)));
        s.push_str(&format!("const SA_MEAN_AO{i}: f32 = {};\n", f(*ao)));
        s.push_str(&format!("const SA_MEAN_TOPS{i}: f32 = {};\n", f(*tops)));
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
        // (`--matte-wheel-paths`) still polished, short of the open binder
        assert!(ASPHALT_ROUGH_POLISHED < ASPHALT_ROUGH_POLISHED_MATTE && ASPHALT_ROUGH_POLISHED_MATTE < ASPHALT_ROUGH);
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
    /// under LTPP's 0.3 m, under half the web sealed, the open cracks from a
    /// hairline to LTPP's moderate severity; the tops' layer smoother than
    /// the road and its wheel paths no rougher than the street pass's polish,
    /// its share the plan's at normal view and growing toward grazing.
    #[test]
    fn wear_and_glare_stay_in_their_bands() {
        assert!((0.025..=0.04).contains(&WEAR_SEAL_HALF_M), "{WEAR_SEAL_HALF_M}");
        assert!(WEAR_FATIGUE_CELL_M < 0.3 && WEAR_FATIGUE_SHARE < 0.5);
        assert!(WEAR_SEALED_SHARE > 0.2 && WEAR_SEALED_SHARE < 0.5);
        assert!(WEAR_SEAL_ROUGH < WEAR_SEAL_ROUGH_AGED && WEAR_SEAL_ROUGH_AGED < ASPHALT_ROUGH);
        assert!(WEAR_MARGIN_M > WEAR_WIDE_HALF_M && WEAR_MARGIN_M < 0.025 && WEAR_MARGIN_DARK < 0.5);
        assert!(WEAR_WEB_ALONG_SHARE < WEAR_WEB_ACROSS_SHARE && WEAR_WEB_ACROSS_SHARE <= 1.0 && WEAR_WEB_PAD_SHARE <= 1.0 && WEAR_DRIP_DARK < 0.6);
        assert!(2.0 * WEAR_HAIR_HALF_M * 1.4 <= 0.0032, "a hairline stays under 3 mm across (LTPP low severity)");
        assert!(WEAR_HAIR_HALF_M < WEAR_OPEN_HALF_M && WEAR_OPEN_HALF_M < WEAR_WIDE_HALF_M && 2.0 * WEAR_WIDE_HALF_M <= 0.019, "LTPP moderate: 6-19 mm");
        assert!(WEAR_OPEN_SHARE + WEAR_WIDE_SHARE < 0.5, "most of the web hairlines");
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
        for k in ["SW_WEB_CELL_M", "SW_HAIR_HALF_M", "SW_SEAL_HALF_M", "SW_DRIP_DARK", "SG_MACRO_SLOPE", "SG_COAT_ROUGH", "SG_SETT_ROUGH"] {
            assert!(w.contains(&format!("const {k}: f32 = ")), "{k}");
        }
    }

    /// (`--street-aggregate`) The layout's means: the stones, the binder film and
    /// the mastic average to a weathered asphalt's tone (LBNL: 0.10-0.18), the
    /// tracks' worn tops lighter and smoother than between them, the mastic's
    /// occlusion mild; a sample's stones cover what a dense mix shows.
    #[test]
    fn the_aggregate_averages_to_weathered_asphalt() {
        let (a0, r0, ao0, t0) = aggregate_means(AGG_EXPOSED, 0.0);
        let (a1, r1, ao1, t1) = aggregate_means(AGG_EXPOSED_WHEEL, 1.0);
        // the shader's constants are this measurement (AGG_MEANS, written out so the
        // startup does not run it)
        for (i, (a, r, ao, t)) in [(a0, r0, ao0, t0), (a1, r1, ao1, t1)].into_iter().enumerate() {
            let m = AGG_MEANS[i];
            assert!((0..3).all(|c| (m.0[c] - a[c]).abs() < 1e-6) && (m.1 - r).abs() < 1e-6 && (m.2 - ao).abs() < 1e-6 && (m.3 - t).abs() < 1e-6, "{i}: {a:?} {r} {ao} {t}");
        }
        let l = |a: [f64; 3]| 0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2];
        assert!((0.08..=0.18).contains(&l(a0)), "{a0:?}");
        assert!(l(a1) > l(a0) && l(a1) < 0.2, "{a1:?}");
        assert!(r1 < r0 && r0 < AGG_MASTIC_ROUGH && r1 > AGG_TOP_ROUGH_WHEEL, "{r0} {r1}");
        assert!(ao0 > 0.85 && ao0 < 1.0 && (ao0 - ao1).abs() < 1e-9);
        assert!(t1 > t0 && t0 > 0.1 && t1 < 0.8, "{t0} {t1}");
        // the stones' cover (the mastic's occlusion is its complement)
        let cover = (ao0 - AGG_MASTIC_AO) / (1.0 - AGG_MASTIC_AO);
        assert!((0.4..=0.8).contains(&cover), "{cover}");
        assert!(AGG_TOP_ROUGH_WHEEL < AGG_TOP_ROUGH && AGG_TOP_ROUGH < AGG_FILM_ROUGH && AGG_FILM_ROUGH < AGG_MASTIC_ROUGH);
        // (`--aged-aggregate`) the grime keeps a share of each tone, and the tones fade out
        // over footprints of a tenth to a quarter of the pitch: before the layer's own fade begins
        assert!(AGG_AGED_TONE > 0.0 && AGG_AGED_TONE < 1.0);
        assert!(AGG_AGED_NEAR < AGG_AGED_FAR && AGG_AGED_FAR <= 0.3);
        let w = wgsl();
        for k in ["SA_STONE_M", "SA_MEAN_ALB0", "SA_MEAN_ROUGH1", "SA_MEAN_TOPS0", "SA_GREY_TINT", "SA_AGED_TONE", "SA_AGED_NEAR", "SA_AGED_FAR"] {
            assert!(w.contains(&format!("const {k}: ")), "{k}");
        }
    }

    /// (`--walk-relief`, `--plaza-setts`) The kerb's segments fall on the walk's
    /// scoring lines and stay inside cast kerbs' joint spacing; spalls are a
    /// sparse scatter of breaks a few centimetres to two decimetres long; the
    /// walk's lips stay under ADA's quarter inch but for heaved slabs, the groove
    /// narrower than its depth; the setts' fill lies between the scan's joint
    /// floor and its tops, its materials inside their measured bands.
    #[test]
    fn walk_relief_and_setts_fill_stay_in_their_bands() {
        let scoring = crate::cc0::scoring_pitch_m() as f64;
        assert!(scoring > 0.8 && scoring < 1.0, "{scoring}");
        let seg = scoring * KERB_SEG_SQUARES;
        assert!(KERB_SEG_SQUARES.fract() == 0.0 && (1.5..=4.5).contains(&seg), "{seg}");
        assert!(KERB_SPALL_SHARE < 0.5 && KERB_SPALL_MIN_M < KERB_SPALL_MAX_M && 2.0 * KERB_SPALL_MAX_M <= 0.25);
        assert!(KERB_SPALL_CELL_M * 0.2 > KERB_SPALL_MAX_M, "a spall stays inside its own cell");
        assert!(KERB_NOSE_MIN < 1.0 && KERB_NOSE_MAX > 1.0);
        assert!(WALK_LIP_M <= 0.00635 && WALK_HEAVE_M > 0.00635 && WALK_HEAVE_M < 0.03);
        assert!(2.0 * WALK_JOINT_HALF_M < WALK_JOINT_DEPTH_M && WALK_NOSE_M < 0.012);
        assert!(WALK_FLOOR_AO < WALK_RISER_AO && WALK_RISER_AO < WALK_LIP_AO && WALK_LIP_AO < 1.0);
        assert!(SETT_FILL_LOW >= 0.2 && SETT_FILL_LOW < SETT_FILL_HIGH && SETT_FILL_HIGH < 0.35, "over the scan's joint floor, under its stones' sides");
        let l = |a: [f64; 3]| 0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2];
        assert!(l(SETT_SOIL) < l(SETT_SAND) && l(SETT_SAND) < 0.45 && l(SETT_SOIL) > 0.05);
        assert!(SETT_MOSS[1] > SETT_MOSS[0] && SETT_MOSS[1] > SETT_MOSS[2] && l(SETT_MOSS) < 0.1);
        let w = wgsl();
        for k in ["WR_KERB_SQUARES", "WR_SPALL_SHARE", "WR_JOINT_HALF_M", "WR_LIP_M", "SS_FILL_LOW", "SS_SAND"] {
            assert!(w.contains(&format!("const {k}: ")), "{k}");
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
