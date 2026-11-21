//! Stage W1: a lived-in city (`--grime 1|2`, off by default; weathering.wgsl).
//!
//! The street wear decals, the gutter, kerb and wall-foot grime and the facade
//! ageing are all drawn by default, yet at normal viewing distance the city
//! read as freshly built: the walks near white and uniform, the carriageway one
//! clean grey, the walls barely touched. This level raises them to what the
//! photographs of San Francisco show (reference sheet: tmp/plan-run/refs,
//! sections 2 and 5.10; first-hand notes of a Mission St sidewalk, Broadway, and
//! the Selby St / Mariposa St viaducts) and adds what was missing:
//!
//! * walks: aged, soiled concrete at a mid grey (each scored slab its own
//!   pour, a few newer replacement slabs), darker joints, a dense trodden-gum
//!   speckle that holds its mean at any distance, blotchy stains in the busy
//!   stretches, dirt along the kerb and (decals) along the building line;
//! * kerbs: grime up the face, crumbled arrises, and (decals) the corner
//!   daylighting's red paint faded and chipped to bare concrete;
//! * carriageways: a quilt of trench patches and repaved pieces in different
//!   greys with sealed saw cuts, crack-seal "tar snakes", stronger oil down the
//!   lane centres, darker gutter pans, faded and chipped marking paint;
//! * facades: wash-down streaks below every sill line (some rust-stained),
//!   soot under the roof edge and on the upper storeys of older buildings,
//!   splash-back at the foot, sun-faded paint on the walls the sun reaches
//!   most, each building its own age (some fresh, a few filthy);
//! * cars: road dust on the lower body, grime round the wheel arches, dull
//!   paint on older cars, each car its own.
//!
//! Albedo and roughness only: no emission, nothing painted into the lighting.
//! Every term is behind the level (`facade::RUN_GRIME_*` in the frame
//! switches): with it 0 nothing new runs and every frame is as before. Level 2
//! (hyper) adds a second gum scatter and longer reaches.
//!
//! Stage W5 adds the windows (constants `GLASS_*` .. `TAPE` below): a dirt
//! film on the kit glass, chalked and peeling old sashes and dirty ledges
//! (with this level), and with `--street-life` stickers and scratched tags on
//! ground-floor glass, plants on flats' sills and paper inside vacant shops.
//!
//! Values: street dimensions come from the standards table (the scoring pitch,
//! the kerb height, the daylighting length, read where they are used); the
//! reflectances cite the reference sheet; sizes of dirt, chips and repairs are
//! shading choices, marked INFERRED.

/// The aged, soiled walk's mean albedo: the Lawrence Berkeley National
/// Laboratory Heat Island Group's measured band for weathered Portland-cement
/// concrete, 0.20-0.30 (look.rs `CONCRETE_AGED_ALBEDO` draws its top, a swept
/// walk), at its lower middle for a used city footway; reference sheet 2.1
/// (US EPA 2012, Cool Pavements: grey concrete about 0.40 new, falling with
/// weathering and soiling) and 5.10 (Mission St: "mid grey, not near white").
pub const WALK_ALBEDO: f64 = 0.225;

/// The fold from today's walk (`CONCRETE_AGED_ALBEDO`) to `WALK_ALBEDO`.
pub fn walk_fold() -> f64 {
    WALK_ALBEDO / crate::look::CONCRETE_AGED_ALBEDO
}

/// Per scored slab: the spread of its pour tone (+- half of it), the share of
/// newer replacement slabs (lighter, cleaner) and of old dark ones, and their
/// tones. INFERRED from the reference's first-hand notes (patched slabs read
/// clearly lighter than their neighbours).
pub const SLAB_SPREAD: f64 = 0.16;
pub const SLAB_NEW_SHARE: f64 = 0.07;
pub const SLAB_NEW_TONE: f64 = 1.24;
pub const SLAB_OLD_SHARE: f64 = 0.09;
pub const SLAB_OLD_TONE: f64 = 0.84;

/// The slab tone's mean (the far field's tone, so the walk keeps its mean when
/// the slabs are under a pixel).
pub fn slab_mean() -> f64 {
    1.0 + SLAB_NEW_SHARE * (SLAB_NEW_TONE - 1.0) + SLAB_OLD_SHARE * (SLAB_OLD_TONE - 1.0)
}

/// Dirt held in a scoring joint: its reach from the groove (m) and its darkening. INFERRED.
pub const JOINT_DIRT_M: f64 = 0.016;
pub const JOINT_DARK: f64 = 0.34;

/// Trodden chewing gum (reference 5.10, Mission St: "a dense field of dark gum
/// spots, black discs 1-4 cm"): one disc at most per cell of `GUM_CELL_M`,
/// radius from `GUM_R_MIN_M` to `GUM_R_MAX_M` (square-biased toward the small
/// end), present with a probability from `GUM_BASE` to `GUM_BASE + GUM_GAIN`
/// by a slow activity field. Trodden gum is a flattened grey-black disc, matte
/// to faintly waxy (albedo and roughness INFERRED).
pub const GUM_CELL_M: f64 = 0.085;
pub const GUM_R_MIN_M: f64 = 0.005;
pub const GUM_R_MAX_M: f64 = 0.017;
pub const GUM_BASE: f64 = 0.22;
pub const GUM_GAIN: f64 = 0.6;
pub const GUM_ALBEDO: f64 = 0.04;
/// Old gum greys toward the concrete: a disc's albedo from `GUM_ALBEDO` up to this.
pub const GUM_ALBEDO_OLD: f64 = 0.13;
pub const GUM_ROUGH: f64 = 0.62;

/// The mean area of one gum disc (m^2): pi E[r^2], r = min + (max - min) u^2, u uniform.
pub fn gum_mean_area() -> f64 {
    let (a, b) = (GUM_R_MIN_M, GUM_R_MAX_M - GUM_R_MIN_M);
    // E[u^2] = 1/3, E[u^4] = 1/5.
    std::f64::consts::PI * (a * a + 2.0 * a * b / 3.0 + b * b / 5.0)
}

/// Blotchy stains (spills, grease, bin drips): the blotches' scale (m), their
/// darkening tint (darker, faintly warm), and dirt along the kerb: its reach
/// (m) and darkening. INFERRED from the reference's notes.
pub const STAIN_M: f64 = 0.85;
pub const STAIN_TINT: [f64; 3] = [0.54, 0.51, 0.48];
pub const KERB_BAND_M: f64 = 0.7;
pub const KERB_BAND_DARK: f64 = 0.16;

/// Kerbs: grime up the face from the gutter (darkening), the crumbled arris's
/// reach (m), its chips' scale (m) and darkening. INFERRED.
pub const KERB_GRIME: f64 = 0.42;
pub const CHIP_M: f64 = 0.035;
pub const CHIP_SCALE_M: f64 = 0.11;
pub const CHIP_DARK: f64 = 0.5;

/// Kerb paint (CVC 21458 colours, `curb-colors`): red traffic paint new and
/// sun-faded to a chalky pink, linear (sRGB 168 34 30 and 186 118 108;
/// INFERRED from the Mission St note: "red paint faded and chipped").
pub const PAINT_RED: [f64; 3] = [0.392, 0.016, 0.013];
pub const PAINT_RED_FADED: [f64; 3] = [0.491, 0.181, 0.15];

/// The carriageway's patch quilt: cells of `QUILT_CELL_M` along the kerb, two
/// candidate repairs a cell each present with `QUILT_SHARE`, `QUILT_FRESH` of
/// them fresh hot-mix. Tones relative to the aged surface: fresh asphalt 0.05
/// against aged 0.10-0.15 (reference sheet 2.1, LBNL / EPA), so a fresh patch
/// is drawn at 0.62-0.78 of it; old patches with exposed aggregate lighter.
/// The saw cut's sealant band (half-width, m) and darkening. Repair sizes are
/// INFERRED (utility trenches and repaving pieces, not street dimensions).
pub const QUILT_CELL_M: f64 = 26.0;
pub const QUILT_SHARE: f64 = 0.62;
pub const QUILT_FRESH: f64 = 0.55;
pub const PATCH_NEW: [f64; 2] = [0.56, 0.74];
pub const PATCH_OLD: [f64; 2] = [1.16, 1.38];
pub const SEAM_M: f64 = 0.022;
pub const SEAM_DARK: f64 = 0.32;
/// Patches only this close (m): past it the carriageway is a few pixels a lane.
pub const QUILT_FAR_M: f64 = 260.0;

/// Crack sealant ("tar snakes"): meandering bands along the cracks; the
/// block cracking's cell size (m), the band's half-width (overband sealant is laid
/// 3-4 in wide; INFERRED), fresh and aged albedo (the binder is blacker than
/// new asphalt's 0.05, sheet 2.1; it greys as it oxidises) and roughness
/// (fresh sealant is glossy). Drawn this close (m).
pub const SNAKE_PERIOD_M: f64 = 3.6;
pub const SNAKE_HALF_M: f64 = 0.05;
pub const SNAKE_ALBEDO: [f64; 2] = [0.03, 0.065];
pub const SNAKE_ROUGH: [f64; 2] = [0.55, 0.72];
pub const SNAKE_FAR_M: f64 = 170.0;

/// Extra oil darkening down the lane centres (over the decal's own) and the
/// gutter pan's grime. INFERRED from the reference's notes.
pub const OIL_EXTRA: f64 = 0.4;
pub const GUTTER_DARK: f64 = 0.3;
/// Marking paint flaked to bare asphalt at most this share (an old line).
pub const PAINT_BARE: f64 = 0.8;

/// Facades: per building, the share left fresh (repainted lately) and the
/// share filthy, and their weathering gains. INFERRED ("no two neighbours
/// weather alike").
pub const BLDG_FRESH_SHARE: f64 = 0.14;
pub const BLDG_FRESH: f64 = 0.5;
pub const BLDG_HEAVY_SHARE: f64 = 0.22;
pub const BLDG_HEAVY: f64 = 1.7;
/// Wash-down streaks below the sill lines: the column pitch (m), the reach
/// down as a share of the storey, the darkening, its tint, the share of rust
/// stains (below iron sill hardware and grilles) at full wear, and the rust
/// tint (iron oxide run-off). INFERRED.
pub const STREAK_COL_M: f64 = 1.05;
pub const STREAK_REACH: f64 = 0.8;
pub const STREAK_DARK: f64 = 0.6;
pub const STREAK_TINT: [f64; 3] = [0.62, 0.6, 0.58];
pub const RUST_SHARE: f64 = 0.18;
pub const RUST_TINT: [f64; 3] = [0.78, 0.5, 0.33];
/// Soot under the roof edge (reach in m at no wear; doubled at full wear) and
/// its darkening; the upper storeys' darkening of old buildings.
pub const ROOF_SOOT_M: f64 = 1.3;
pub const ROOF_SOOT_DARK: f64 = 0.4;
pub const UPPER_DARK: f64 = 0.16;
/// Splash-back: the band's reach (m, 0.3 at no wear to 1.0 at full: the
/// brief's 0.3-1 m), darkening and soil tint.
pub const SPLASH_M: [f64; 2] = [0.3, 1.0];
pub const SPLASH_DARK: f64 = 0.34;
pub const SPLASH_TINT: [f64; 3] = [0.74, 0.69, 0.62];
/// Sun-faded paint: the fade on a wall facing the day's mean sun.
pub const SUN_FADE: f64 = 0.4;

/// Stage W2c: weathering that reads at 50-200 m, where the sill streaks above
/// have averaged to a tone (their columns are 1 m, the pixel there 0.05-0.2 m
/// and the streak's own detail finer). Per building, metre-scale patterns of
/// the kind the reference photographs show from across a street or a block
/// away (sheet 5.10: Selby St "darker vertical stain streaks on cap beam faces
/// and column tops", Mariposa St "grime streaks running down from deck joints
/// and drains"; SF's painted stucco and clapboard fronts repainted in blocks
/// over graffiti and sun-faded unevenly; soot plumes down old masonry):
///
/// * plumes from the roof edge, one in some columns of a per-building pitch
///   (m), each its own width (m) and reach down the wall (a share of its
///   height), soot-grey on render and masonry, rust under iron hardware on
///   metal and on some old masonry; their darkening and tints;
/// * damp patches at the foot, up to the given heights (m), blotchy along the
///   wall, with a darker tide line at the top; darkening and tint;
/// * on painted and rendered walls, paint blocks per bay (its width, m) and
///   storey: some repainted in a paint that does not quite match (tone spread
///   +- half of it, and a pull toward a neutral), more of them on the ground
///   floor (graffiti abatement), some sun-faded lighter and greyer.
///
/// Every size is metres, every edge at least a few pixels at 200 m.
/// INFERRED (no measured table exists for these; the reference sheet's notes
/// and the brief's 50-200 m reading distance set them).
pub const PLUME_PITCH_M: [f64; 2] = [3.2, 7.5];
pub const PLUME_WIDTH_M: [f64; 2] = [0.8, 2.8];
pub const PLUME_REACH: [f64; 2] = [0.25, 0.85];
pub const PLUME_SHARE: [f64; 2] = [0.3, 0.7];
pub const PLUME_DARK: f64 = 0.5;
pub const PLUME_SOOT_TINT: [f64; 3] = [0.8, 0.8, 0.82];
pub const PLUME_RUST_TINT: [f64; 3] = [0.9, 0.58, 0.38];
pub const DAMP_M: [f64; 2] = [0.6, 2.4];
pub const DAMP_DARK: f64 = 0.38;
pub const DAMP_TINT: [f64; 3] = [0.84, 0.86, 0.8];
pub const BLOCK_BAY_M: [f64; 2] = [2.6, 6.4];
pub const REPAINT_SHARE: f64 = 0.3;
pub const REPAINT_GROUND_SHARE: f64 = 0.6;
pub const REPAINT_SPREAD: f64 = 0.42;
pub const REPAINT_NEUTRAL: f64 = 0.3;
pub const FADE_BLOCK_SHARE: f64 = 0.22;
pub const FADE_BLOCK: f64 = 0.5;

/// Cars: the share clean and filthy, the dirt of the rest, the dust's and the
/// grime's colours (linear: dry road dust light warm grey, wheel-arch grime
/// near black brown), their cover at full dirt, and the dulling of older
/// cars' paint (roughness added). INFERRED.
pub const CAR_CLEAN_SHARE: f64 = 0.15;
pub const CAR_FILTHY_SHARE: f64 = 0.18;
pub const CAR_DIRT: [f64; 3] = [0.08, 0.45, 1.0];
pub const CAR_DUST: [f64; 3] = [0.3, 0.27, 0.22];
pub const CAR_GRIME: [f64; 3] = [0.055, 0.05, 0.045];
pub const CAR_DUST_COVER: f64 = 0.55;
pub const CAR_GRIME_COVER: f64 = 0.75;
pub const CAR_DULL: f64 = 0.28;

// ---- Stage W5: windows people live behind ------------------------------------
//
// The kit glass was spotless and the frames freshly painted on every building.
// The reference (sheet 5.10; the Broadway and Castro notes) shows grime and
// rain spots on the glass, darker at the frame and the bottom rail and on the
// unwashed upper storeys of old buildings, chalked and peeling paint on old
// wooden sashes, dirt on the sills, stickers on shop doors, tags scratched into
// shop glass, paper taped inside vacant shops and plants on flats' sills.
// `--grime` draws the dirt, `--street-life` the rest (weathering.wgsl
// `wx_window`, `wx_window_life`, `wx_window_trim`; rooms.wgsl `rm_room`'s hook,
// `wx_room_life`). Albedo, roughness, metalness and the glass's clearcoat
// and opacity only; the lit room behind a pane is dimmed by what covers it.

/// The dirt film on glass, as an area cover over the clean pane (the
/// partial-cover model: what is filmed scatters as a diffuse dust layer, the
/// rest is the clean glass). Its colour: urban dust deposits, a mix of mineral
/// dust and soot, greyish-tan (INFERRED: between the soiled walk this file
/// draws at 0.225 and the mineral fraction's ~0.3-0.4; no measured value for
/// window grime is in the reference sheet); the grime packed against the frame
/// and in the bottom rail's tide band, browner and darker with soot, still
/// lighter than the dark room it is seen against (INFERRED).
pub const GLASS_DUST: [f64; 3] = [0.33, 0.31, 0.27];
pub const GLASS_SOOT: [f64; 3] = [0.17, 0.15, 0.125];
/// The film's cover in a pane's open middle from no wear to full wear (an
/// unwashed old flat's window reads hazy, a new one clear), the per-window
/// spread round it (some panes washed lately, some never) and the share just
/// washed (their film at `GLASS_FRESH` of it). INFERRED.
pub const GLASS_FILM: [f64; 2] = [0.03, 0.22];
pub const GLASS_SPREAD: [f64; 2] = [0.5, 1.3];
pub const GLASS_FRESH_SHARE: f64 = 0.14;
pub const GLASS_FRESH: f64 = 0.2;
/// Grime packed against the frame: its reach into the pane (m) and its cover
/// at the frame (corners take both sides'); the bottom rail's tide band, where
/// run-off pools and dries: its height (m) and cover. INFERRED (the frame's
/// own 0.07 m width, elevation.rs, sets the scale).
pub const GLASS_RIM_M: f64 = 0.08;
pub const GLASS_RIM: f64 = 0.5;
pub const GLASS_RAIL_M: f64 = 0.055;
pub const GLASS_RAIL: f64 = 0.6;
/// Dirty run-off down from the head: the run columns' pitch (m), the share of
/// columns carrying one, the cover inside a run. INFERRED.
pub const GLASS_RUN_PITCH_M: f64 = 0.11;
pub const GLASS_RUN_SHARE: f64 = 0.3;
pub const GLASS_RUN: f64 = 0.22;
/// Dried rain spots: one at most per cell (m), radius range (m), the share of
/// cells holding one at full wear, and a spot's cover (its mineral ring).
/// INFERRED (drops of 2-6 mm dry to rings of about their size).
pub const GLASS_SPOT_CELL_M: f64 = 0.016;
pub const GLASS_SPOT_R_M: [f64; 2] = [0.0011, 0.0034];
pub const GLASS_SPOT_SHARE: f64 = 0.6;
pub const GLASS_SPOT: f64 = 0.4;
/// Upper storeys of old buildings go unwashed: the film's gain from 3 m to
/// 12 m over the floor, times the building's wear.
pub const GLASS_UPPER: f64 = 0.6;
/// Who washes the glass: shopkeepers often (the film at this share), offices
/// and curtain walls on a contract (this share when new, the second when the
/// building is worn: an old loft office's windows go years); a vacant shop
/// never (this gain).
pub const GLASS_SHOP_CARE: f64 = 0.4;
pub const GLASS_OFFICE_CARE: [f64; 2] = [0.28, 0.85];
pub const GLASS_VACANT: f64 = 1.6;
/// Hand grease on shop doors (hand height, near the edges): its cover. INFERRED.
pub const GLASS_GREASE: f64 = 0.22;
/// What a full film does to the glass: roughness added (the reflection
/// blurred: the lookup reads the probe at the pane's roughness), the share of
/// the clearcoat and of the pane's opacity it takes over, and the share of the
/// lit room's light it holds back. A dust layer cuts a pane's transmittance:
/// up to ~20 % after a month on tilted collector glass in a dry climate (Hegazy,
/// Renewable Energy 22:525-540, 2001; vertical panes gather far less; as
/// recalled, not re-read); years unwashed, a full film (cover 1) is taken to
/// hold back half (INFERRED).
pub const GLASS_HAZE: f64 = 0.5;
pub const GLASS_CC_LOSS: f64 = 0.85;
pub const GLASS_OPAQUE: f64 = 0.65;
pub const GLASS_TRANSMIT_LOSS: f64 = 0.5;
/// The film's cover never past this (a pane stays glass).
pub const GLASS_MAX: f64 = 0.55;

/// Old painted sashes and frames (the kit's `painted-white` finish: the
/// Victorian, Edwardian and bay-window fronts' frames, sills and trim):
/// chalked by the paint's age (lighter, flatter, greyed) and, once the paint
/// is old (`FaD.paint` is the years since the last repaint over 30,
/// eo-kit appearance.rs), alligatored and peeling in cells of about
/// `PEEL_CELL_M`, from `PEEL_FROM` of the paint's age, at most `PEEL_MAX` of
/// it lost, more low on each storey (where water stands); bare weathered wood
/// grey-brown or an older coat showing. INFERRED (sizes); the wood's colour
/// weathered softwood grey (INFERRED).
pub const PEEL_CELL_M: f64 = 0.048;
pub const PEEL_FROM: f64 = 0.3;
pub const PEEL_MAX: f64 = 0.65;
pub const PEEL_WOOD: [f64; 3] = [0.21, 0.18, 0.14];
pub const PEEL_UNDERCOAT: [f64; 3] = [0.52, 0.55, 0.47];
pub const CHALK: f64 = 0.35;
/// Ledges (the tops of sills, lintels, belt courses and copings of the trim
/// finishes): dirt and droppings settle (darkening, tint); the undersides'
/// drip edge holds a dark line of grime (darkening). INFERRED.
pub const LEDGE_DARK: f64 = 0.42;
pub const LEDGE_TINT: [f64; 3] = [0.62, 0.59, 0.54];
pub const DRIP_DARK: f64 = 0.55;

/// Shop glass (`--street-life`): the share of shop doors carrying a cluster of
/// stickers (opening hours, card marks, an alarm firm's), and of other shop
/// panes with one in a corner; plain shapes and colour fields, nothing that
/// reads. Tags scratched into ground-floor glass (scratchiti) where the wall
/// beside it carries tags: the chance a pane takes one per tag level of that
/// wall (street_life.rs `LandUse::tags`, 0..7), the scratch's width (m) and an
/// acid-etched (frosted) tag's. INFERRED.
pub const STICKER_DOOR_SHARE: f64 = 0.8;
pub const STICKER_PANE_SHARE: f64 = 0.3;
pub const SCRATCH_PER_TAG: f64 = 0.07;
pub const SCRATCH_W_M: [f64; 2] = [0.0012, 0.0026];
pub const ETCH_W_M: [f64; 2] = [0.004, 0.008];
/// The share of shopfronts standing vacant, and its gain on a front whose wall
/// is pasted with posters (street_life.rs `LandUse::posters` 4 or more: vacant
/// and industrial fronts). INFERRED (press reports of San Francisco's
/// neighbourhood commercial corridors put storefront vacancy at 10-20 % in
/// 2023-2025; not checked against a primary source).
pub const VACANT_SHARE: f64 = 0.1;
pub const VACANT_POSTER_GAIN: f64 = 2.5;

/// Sill plants (`--street-life`, rooms.wgsl): the share of a flat's windows
/// with plants on the sill, new to old (the building's wear), up to three
/// pots a window. Leaves: a green leaf reflects about 5 % in the blue and red
/// chlorophyll bands and 10-15 % at the 550 nm green peak, and transmits about
/// as much (Jacquemoud & Ustin, Leaf Optical Properties, Cambridge University
/// Press 2019; as recalled, not re-read): linear reflectance and transmittance
/// below. Pots: unglazed terracotta (sRGB about 180 92 60), white glazed,
/// dark plastic, blue glazed (INFERRED).
pub const PLANT_SHARE: [f64; 2] = [0.1, 0.42];
pub const LEAF_ALBEDO: [f64; 3] = [0.045, 0.11, 0.035];
pub const LEAF_TRANSMIT: [f64; 3] = [0.03, 0.1, 0.02];
pub const POT_TERRACOTTA: [f64; 3] = [0.456, 0.107, 0.045];
/// Paper and board taped inside a vacant shop's glass: office paper (ISO 2470
/// brightness about 0.9), newsprint (about 0.6), unbleached kraft paper and
/// corrugated board (about 0.3, brown), masking tape (cream); linear albedo,
/// INFERRED from those brightness classes.
pub const PAPER_WHITE: [f64; 3] = [0.8, 0.8, 0.78];
pub const PAPER_NEWS: [f64; 3] = [0.56, 0.55, 0.51];
pub const PAPER_KRAFT: [f64; 3] = [0.36, 0.22, 0.12];
pub const BOARD: [f64; 3] = [0.32, 0.21, 0.11];
pub const TAPE: [f64; 3] = [0.6, 0.53, 0.38];

/// The detail-finish index + 1 of a kit finish (`facade::detail_finishes`, the
/// material flags' bits 11..15), 0 if it has none.
pub fn detail_index(name: &str) -> u32 {
    crate::facade::detail_finishes().iter().position(|(k, _)| k == name).map(|i| i as u32 + 1).unwrap_or(0)
}

/// The direction (world x, z) of the day's mean sun on a wall: the key's
/// horizontal direction weighted by its strength over the clock (eo-look's
/// own sun), so "sun-faded" follows the renderer's sun: south and west walls.
pub fn sun_axis() -> [f64; 2] {
    let (mut x, mut z) = (0.0, 0.0);
    for i in 0..=40 {
        let f = eo_look::LookFrame::new(7.0 + 12.0 * i as f64 / 40.0);
        let up = f.key_dir[1].max(0.0);
        x += f.key_dir[0] * up;
        z += f.key_dir[2] * up;
    }
    let l = (x * x + z * z).sqrt().max(1e-9);
    [x / l, z / l]
}

fn lin(c: [f64; 3]) -> String {
    format!("vec3<f32>({:.6}, {:.6}, {:.6})", c[0], c[1], c[2])
}

/// WGSL constants for weathering.wgsl (composed into every city shader variant).
pub fn wgsl() -> String {
    let f = |v: f64| format!("{v:.6}");
    let mut s = String::from("\n// generated by weathering.rs wgsl\n");
    s.push_str(&format!("const WX_RUN_SHIFT: u32 = {}u;\n", crate::facade::RUN_GRIME_SHIFT));
    // the street's kerb distance is encoded to 6.35 m (street.rs): patches stay inside it.
    s.push_str("const WX_KD_MAX: f32 = 6.3;\n");
    let a = sun_axis();
    for (k, v) in [
        ("WX_WALK_FOLD", walk_fold()),
        ("WX_SLAB_SPREAD", SLAB_SPREAD),
        ("WX_SLAB_NEW_SHARE", SLAB_NEW_SHARE),
        ("WX_SLAB_NEW_TONE", SLAB_NEW_TONE),
        ("WX_SLAB_OLD_SHARE", SLAB_OLD_SHARE),
        ("WX_SLAB_OLD_TONE", SLAB_OLD_TONE),
        ("WX_SLAB_MEAN", slab_mean()),
        ("WX_JOINT_DIRT_M", JOINT_DIRT_M),
        ("WX_JOINT_DARK", JOINT_DARK),
        ("WX_GUM_CELL_M", GUM_CELL_M),
        ("WX_GUM_R_MIN", GUM_R_MIN_M),
        ("WX_GUM_R_MAX", GUM_R_MAX_M),
        ("WX_GUM_BASE", GUM_BASE),
        ("WX_GUM_GAIN", GUM_GAIN),
        ("WX_GUM_ALBEDO", GUM_ALBEDO),
        ("WX_GUM_ALBEDO_OLD", GUM_ALBEDO_OLD),
        ("WX_GUM_ROUGH", GUM_ROUGH),
        ("WX_GUM_MEAN_AREA", gum_mean_area()),
        ("WX_STAIN_M", STAIN_M),
        ("WX_KERB_BAND_M", KERB_BAND_M),
        ("WX_KERB_BAND_DARK", KERB_BAND_DARK),
        ("WX_KERB_GRIME", KERB_GRIME),
        ("WX_CHIP_M", CHIP_M),
        ("WX_CHIP_SCALE_M", CHIP_SCALE_M),
        ("WX_CHIP_DARK", CHIP_DARK),
        ("WX_QUILT_CELL_M", QUILT_CELL_M),
        ("WX_QUILT_SHARE", QUILT_SHARE),
        ("WX_QUILT_FRESH", QUILT_FRESH),
        ("WX_PATCH_NEW_LO", PATCH_NEW[0]),
        ("WX_PATCH_NEW_HI", PATCH_NEW[1]),
        ("WX_PATCH_OLD_LO", PATCH_OLD[0]),
        ("WX_PATCH_OLD_HI", PATCH_OLD[1]),
        ("WX_SEAM_M", SEAM_M),
        ("WX_SEAM_DARK", SEAM_DARK),
        ("WX_QUILT_FAR", QUILT_FAR_M),
        ("WX_SNAKE_PERIOD_M", SNAKE_PERIOD_M),
        ("WX_SNAKE_HALF_M", SNAKE_HALF_M),
        ("WX_SNAKE_ALBEDO", SNAKE_ALBEDO[0]),
        ("WX_SNAKE_ALBEDO_AGED", SNAKE_ALBEDO[1]),
        ("WX_SNAKE_ROUGH", SNAKE_ROUGH[0]),
        ("WX_SNAKE_ROUGH_AGED", SNAKE_ROUGH[1]),
        ("WX_SNAKE_FAR", SNAKE_FAR_M),
        ("WX_OIL_EXTRA", OIL_EXTRA),
        ("WX_GUTTER_DARK", GUTTER_DARK),
        ("WX_PAINT_BARE", PAINT_BARE),
        ("WX_BLDG_FRESH_SHARE", BLDG_FRESH_SHARE),
        ("WX_BLDG_FRESH", BLDG_FRESH),
        ("WX_BLDG_HEAVY_SHARE", BLDG_HEAVY_SHARE),
        ("WX_BLDG_HEAVY", BLDG_HEAVY),
        ("WX_STREAK_COL_M", STREAK_COL_M),
        ("WX_STREAK_REACH", STREAK_REACH),
        ("WX_STREAK_DARK", STREAK_DARK),
        ("WX_RUST_SHARE", RUST_SHARE),
        ("WX_ROOF_SOOT_M", ROOF_SOOT_M),
        ("WX_ROOF_SOOT_DARK", ROOF_SOOT_DARK),
        ("WX_UPPER_DARK", UPPER_DARK),
        ("WX_SPLASH_LO", SPLASH_M[0]),
        ("WX_SPLASH_HI", SPLASH_M[1]),
        ("WX_SPLASH_DARK", SPLASH_DARK),
        ("WX_SUN_FADE", SUN_FADE),
        ("WX_PLUME_PITCH_LO", PLUME_PITCH_M[0]),
        ("WX_PLUME_PITCH_HI", PLUME_PITCH_M[1]),
        ("WX_PLUME_WIDTH_LO", PLUME_WIDTH_M[0]),
        ("WX_PLUME_WIDTH_HI", PLUME_WIDTH_M[1]),
        ("WX_PLUME_REACH_LO", PLUME_REACH[0]),
        ("WX_PLUME_REACH_HI", PLUME_REACH[1]),
        ("WX_PLUME_SHARE_LO", PLUME_SHARE[0]),
        ("WX_PLUME_SHARE_HI", PLUME_SHARE[1]),
        ("WX_PLUME_DARK", PLUME_DARK),
        ("WX_DAMP_LO", DAMP_M[0]),
        ("WX_DAMP_HI", DAMP_M[1]),
        ("WX_DAMP_DARK", DAMP_DARK),
        ("WX_BLOCK_BAY_LO", BLOCK_BAY_M[0]),
        ("WX_BLOCK_BAY_HI", BLOCK_BAY_M[1]),
        ("WX_REPAINT_SHARE", REPAINT_SHARE),
        ("WX_REPAINT_GROUND_SHARE", REPAINT_GROUND_SHARE),
        ("WX_REPAINT_SPREAD", REPAINT_SPREAD),
        ("WX_REPAINT_NEUTRAL", REPAINT_NEUTRAL),
        ("WX_FADE_BLOCK_SHARE", FADE_BLOCK_SHARE),
        ("WX_FADE_BLOCK", FADE_BLOCK),
        ("WX_CAR_CLEAN_SHARE", CAR_CLEAN_SHARE),
        ("WX_CAR_FILTHY_SHARE", CAR_FILTHY_SHARE),
        ("WX_CAR_CLEAN", CAR_DIRT[0]),
        ("WX_CAR_SOME", CAR_DIRT[1]),
        ("WX_CAR_FILTHY", CAR_DIRT[2]),
        ("WX_CAR_DUST_COVER", CAR_DUST_COVER),
        ("WX_CAR_GRIME_COVER", CAR_GRIME_COVER),
        ("WX_CAR_DULL", CAR_DULL),
        // stage W5: windows
        ("WX_GLASS_FILM_LO", GLASS_FILM[0]),
        ("WX_GLASS_FILM_HI", GLASS_FILM[1]),
        ("WX_GLASS_SPREAD_LO", GLASS_SPREAD[0]),
        ("WX_GLASS_SPREAD_HI", GLASS_SPREAD[1]),
        ("WX_GLASS_FRESH_SHARE", GLASS_FRESH_SHARE),
        ("WX_GLASS_FRESH", GLASS_FRESH),
        ("WX_GLASS_RIM_M", GLASS_RIM_M),
        ("WX_GLASS_RIM", GLASS_RIM),
        ("WX_GLASS_RAIL_M", GLASS_RAIL_M),
        ("WX_GLASS_RAIL", GLASS_RAIL),
        ("WX_GLASS_RUN_PITCH_M", GLASS_RUN_PITCH_M),
        ("WX_GLASS_RUN_SHARE", GLASS_RUN_SHARE),
        ("WX_GLASS_RUN", GLASS_RUN),
        ("WX_GLASS_SPOT_CELL_M", GLASS_SPOT_CELL_M),
        ("WX_GLASS_SPOT_R_LO", GLASS_SPOT_R_M[0]),
        ("WX_GLASS_SPOT_R_HI", GLASS_SPOT_R_M[1]),
        ("WX_GLASS_SPOT_SHARE", GLASS_SPOT_SHARE),
        ("WX_GLASS_SPOT", GLASS_SPOT),
        ("WX_GLASS_UPPER", GLASS_UPPER),
        ("WX_GLASS_SHOP_CARE", GLASS_SHOP_CARE),
        ("WX_GLASS_OFFICE_CARE_LO", GLASS_OFFICE_CARE[0]),
        ("WX_GLASS_OFFICE_CARE_HI", GLASS_OFFICE_CARE[1]),
        ("WX_GLASS_VACANT", GLASS_VACANT),
        ("WX_GLASS_GREASE", GLASS_GREASE),
        ("WX_GLASS_HAZE", GLASS_HAZE),
        ("WX_GLASS_CC_LOSS", GLASS_CC_LOSS),
        ("WX_GLASS_OPAQUE", GLASS_OPAQUE),
        ("WX_GLASS_TRANSMIT_LOSS", GLASS_TRANSMIT_LOSS),
        ("WX_GLASS_MAX", GLASS_MAX),
        ("WX_PEEL_CELL_M", PEEL_CELL_M),
        ("WX_PEEL_FROM", PEEL_FROM),
        ("WX_PEEL_MAX", PEEL_MAX),
        ("WX_CHALK", CHALK),
        ("WX_LEDGE_DARK", LEDGE_DARK),
        ("WX_DRIP_DARK", DRIP_DARK),
        ("WX_STICKER_DOOR_SHARE", STICKER_DOOR_SHARE),
        ("WX_STICKER_PANE_SHARE", STICKER_PANE_SHARE),
        ("WX_SCRATCH_PER_TAG", SCRATCH_PER_TAG),
        ("WX_SCRATCH_W_LO", SCRATCH_W_M[0]),
        ("WX_SCRATCH_W_HI", SCRATCH_W_M[1]),
        ("WX_ETCH_W_LO", ETCH_W_M[0]),
        ("WX_ETCH_W_HI", ETCH_W_M[1]),
        ("WX_VACANT_SHARE", VACANT_SHARE),
        ("WX_VACANT_POSTER_GAIN", VACANT_POSTER_GAIN),
        ("WX_PLANT_SHARE_LO", PLANT_SHARE[0]),
        ("WX_PLANT_SHARE_HI", PLANT_SHARE[1]),
    ] {
        s.push_str(&format!("const {k}: f32 = {};\n", f(v)));
    }
    // stage W5: the trim finishes' detail indices (material flags bits 11..15; 0 none)
    for (k, name) in [("WX_DI_PAINTED_WHITE", "painted-white"), ("WX_DI_TRIM", "cast-stone-trim"), ("WX_DI_CAST_STONE", "cast-stone")] {
        s.push_str(&format!("const {k}: u32 = {}u;\n", detail_index(name)));
    }
    for (k, v) in [
        ("WX_STAIN_TINT", STAIN_TINT),
        ("WX_PAINT_RED", PAINT_RED),
        ("WX_PAINT_RED_FADED", PAINT_RED_FADED),
        ("WX_STREAK_TINT", STREAK_TINT),
        ("WX_RUST_TINT", RUST_TINT),
        ("WX_SPLASH_TINT", SPLASH_TINT),
        ("WX_PLUME_SOOT_TINT", PLUME_SOOT_TINT),
        ("WX_PLUME_RUST_TINT", PLUME_RUST_TINT),
        ("WX_DAMP_TINT", DAMP_TINT),
        ("WX_CAR_DUST", CAR_DUST),
        ("WX_CAR_GRIME", CAR_GRIME),
        ("WX_GLASS_DUST", GLASS_DUST),
        ("WX_GLASS_SOOT", GLASS_SOOT),
        ("WX_PEEL_WOOD", PEEL_WOOD),
        ("WX_PEEL_UNDERCOAT", PEEL_UNDERCOAT),
        ("WX_LEDGE_TINT", LEDGE_TINT),
        ("WX_LEAF_ALBEDO", LEAF_ALBEDO),
        ("WX_LEAF_TRANSMIT", LEAF_TRANSMIT),
        ("WX_POT_TERRACOTTA", POT_TERRACOTTA),
        ("WX_PAPER_WHITE", PAPER_WHITE),
        ("WX_PAPER_NEWS", PAPER_NEWS),
        ("WX_PAPER_KRAFT", PAPER_KRAFT),
        ("WX_BOARD", BOARD),
        ("WX_TAPE", TAPE),
    ] {
        s.push_str(&format!("const {k}: vec3<f32> = {};\n", lin(v)));
    }
    s.push_str(&format!("const WX_SUN_AXIS: vec2<f32> = vec2<f32>({:.6}, {:.6});\n", a[0], a[1]));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walk_lands_in_the_measured_band() {
        // LBNL weathered concrete 0.20-0.30; today's walk at its top, W1 below it.
        assert!(WALK_ALBEDO >= 0.2 && WALK_ALBEDO <= 0.3);
        assert!(walk_fold() < 1.0 && walk_fold() > 0.6, "{}", walk_fold());
        // with the slabs, gum and stains folded in, the walk's mean stays in the band.
        // (the activity field at its middle, the clusters at their mean of about 0.6)
        let gum_cover = (GUM_BASE + GUM_GAIN * 0.5) * 0.6 * gum_mean_area() / (GUM_CELL_M * GUM_CELL_M);
        let mean = WALK_ALBEDO * slab_mean() * (1.0 - gum_cover) + (GUM_ALBEDO + GUM_ALBEDO_OLD) * 0.5 * gum_cover;
        assert!(mean > 0.2 && mean < 0.3, "{mean}");
        assert!(gum_cover > 0.01 && gum_cover < 0.08, "{gum_cover}");
        // gum: 1-4 cm discs, at most one a cell, never past the cell's middle half.
        assert!(GUM_R_MAX_M * 2.0 <= 0.04 + 1e-9 && GUM_R_MIN_M * 2.0 >= 0.01 - 1e-9);
        assert!(GUM_R_MAX_M < GUM_CELL_M * 0.25);
        let a = gum_mean_area();
        assert!(a > std::f64::consts::PI * GUM_R_MIN_M.powi(2) && a < std::f64::consts::PI * GUM_R_MAX_M.powi(2));
    }

    #[test]
    fn the_sun_axis_points_south_and_west() {
        // +z is south, -x west in the world frame (the city's north is -z).
        let a = sun_axis();
        assert!(a[1] > 0.3, "{a:?}");
        assert!((a[0] * a[0] + a[1] * a[1] - 1.0).abs() < 1e-6);
    }

    /// Stage W5: the windows' values stay physical (albedos inside 0..1, the
    /// dust lighter than its soot and darker than paper, leaves greenest and
    /// dim, a pane never all film, the room's light never all held back) and
    /// the trim finishes the frames' code keys on exist in the detail table.
    #[test]
    fn the_window_values_are_physical() {
        let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        for c in [GLASS_DUST, GLASS_SOOT, PEEL_WOOD, PEEL_UNDERCOAT, LEDGE_TINT, LEAF_ALBEDO, LEAF_TRANSMIT, POT_TERRACOTTA, PAPER_WHITE, PAPER_NEWS, PAPER_KRAFT, BOARD, TAPE] {
            assert!(c.iter().all(|v| *v > 0.0 && *v < 0.95), "{c:?}");
        }
        assert!(luma(GLASS_SOOT) < luma(GLASS_DUST) && luma(GLASS_DUST) < luma(PAPER_NEWS) && luma(PAPER_NEWS) < luma(PAPER_WHITE));
        // a green leaf: green the strongest channel, about 5-15 % (Jacquemoud & Ustin)
        for c in [LEAF_ALBEDO, LEAF_TRANSMIT] {
            assert!(c[1] > c[0] && c[1] > c[2] && c[1] >= 0.05 && c[1] <= 0.15, "{c:?}");
        }
        // office paper's brightness about 0.9, kraft and board brown and darker than newsprint
        assert!(luma(PAPER_WHITE) > 0.7 && luma(PAPER_KRAFT) < luma(PAPER_NEWS) && PAPER_KRAFT[0] > PAPER_KRAFT[2] && BOARD[0] > BOARD[2]);
        // a pane stays glass: its film capped, its room's light never all held back, its middle hazy at most
        // (a typical old flat's pane: its middle hazy, not filmed over)
        assert!(GLASS_MAX < 1.0 && GLASS_TRANSMIT_LOSS * GLASS_MAX < 0.5 && GLASS_FILM[1] < 0.5);
        assert!(GLASS_FILM[0] < GLASS_FILM[1] && GLASS_SHOP_CARE < 1.0 && GLASS_OFFICE_CARE[0] < GLASS_OFFICE_CARE[1] && GLASS_OFFICE_CARE[1] < 1.0 && GLASS_VACANT > 1.0);
        // rain spots: drops of a few mm, at most one a cell, inside its middle half
        assert!(GLASS_SPOT_R_M[1] * 2.0 <= 0.008 && GLASS_SPOT_R_M[1] < GLASS_SPOT_CELL_M * 0.25);
        // scratches are hairlines, etched strokes a few mm; peeling cells centimetres
        assert!(SCRATCH_W_M[1] < ETCH_W_M[0] && ETCH_W_M[1] <= 0.01 && PEEL_CELL_M > 0.01 && PEEL_CELL_M < 0.08);
        assert!(PLANT_SHARE[0] < PLANT_SHARE[1] && PLANT_SHARE[1] < 0.5 && VACANT_SHARE * VACANT_POSTER_GAIN < 0.5);
        for name in ["painted-white", "cast-stone-trim", "cast-stone"] {
            let i = detail_index(name);
            assert!(i > 0 && i < 32, "{name}: {i}");
        }
        assert_eq!(detail_index("no-such-finish"), 0);
        let src = wgsl();
        assert!(src.contains("const WX_DI_PAINTED_WHITE: u32 = ") && src.contains("WX_GLASS_DUST") && src.contains("WX_PLANT_SHARE_HI"));
    }

    /// Stage W5 adds no work to the default frame: every call into the windows'
    /// code sits behind its level (draw.wgsl: `sl_level()` before the room,
    /// `wx_level()` or `sl_level()` after it; facade_age.wgsl: inside the
    /// `wx_level()` block; rooms.wgsl: behind `rm_w5`, which only
    /// `wx_window_life_pre` sets), and the room's inputs are cleared once the
    /// room is drawn.
    #[test]
    fn the_windows_run_only_behind_their_levels() {
        let draw = include_str!("draw.wgsl");
        let before = |src: &str, call: &str, guard: &str| {
            let at = src.find(call).unwrap_or_else(|| panic!("{call} not called"));
            assert_eq!(src.matches(call).count(), 1, "{call} called once");
            let line_start = src[..at].rfind("if (").unwrap();
            assert!(src[line_start..at].contains(guard), "{call} behind {guard}: {}", &src[line_start..at]);
        };
        before(draw, "wx_window_life_pre(", "if (sl_level() != 0u) {");
        before(draw, "wx_window(&s, &m,", "if (wx_level() != 0u || sl_level() != 0u) {");
        let fa = include_str!("facade_age.wgsl");
        let block = fa.find("if (wx_level() != 0u) {\n        // weathering (`--grime`, weathering.wgsl): streaks").expect("fa_apply's grime block");
        let call = fa.find("wx_window_trim(").expect("wx_window_trim called");
        assert!(call > block && !fa[block..call].contains("\n    }\n"), "wx_window_trim inside the grime block");
        let rooms = include_str!("rooms.wgsl");
        before(rooms, "wx_room_life(", "if (rm_w5.x > 0.5) {");
        let wx = include_str!("weathering.wgsl");
        assert_eq!(wx.matches("rm_w5 = ").count(), 2, "rm_w5 set by wx_window_life_pre and cleared by wx_window only");
        let w = &wx[wx.find("fn wx_window(").unwrap()..];
        assert!(w[..w.find('}').unwrap()].contains("rm_w5 = vec4<f32>(0.0);"));
    }

    #[test]
    fn the_constants_parse() {
        let src = format!("{}\nfn wx_main() -> f32 {{ return WX_WALK_FOLD + WX_SUN_AXIS.x + WX_CAR_GRIME.x; }}\n", wgsl());
        wgpu::naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    }
}
