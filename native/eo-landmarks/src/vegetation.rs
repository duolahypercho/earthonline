//! The coastal tree stands, ground vegetation and hill rock placed on the
//! land-cover classes, slopes and aspects of the headlands, bluffs and the
//! Presidio. Pure rules: the app hands in a sampler of the drawn ground
//! ([`Site`]: height, normal, tone, cover, rock type) and gets back plants
//! ([`Plant`]); eo-render's props library has the models
//! (`props_lib2/vegetation_models.rs`).
//!
//! Two layers, both deterministic in world position (a hash of the lattice
//! cell, never of the camera or the call), so a tile planned twice is the same:
//!
//! * the coarse layer ([`plant_coarse`], per 128 m props tile): coastal trees
//!   where the land cover says tree outside the mapped woods (the mapped
//!   woods' trees come from the park-tree lattice, re-speciated by
//!   [`coastal_tree`]), coastal scrub shrubs, and rock outcrops on steep
//!   slopes and mapped bare rock;
//! * the fine layer ([`plant_fine`], per 32 m cell near the eye): grass
//!   bunches, short tufts and (in the bloom season) wildflower clumps.
//!
//! Sources for the rules (numbers that are not published are INFERRED):
//! * Presidio forest: planted 1880s-1940s; about 75,000 trees, five species
//!   90 % (Monterey cypress, Monterey pine, blue gum, blackwood acacia, coast
//!   redwood), blue gum nearly half (NPS Presidio Forest; Presidio Trust
//!   forestry fact sheet, June 2023). Shares here: blue gum 0.45, cypress
//!   0.30, pine 0.25 (INFERRED from those words; the acacia and redwood are
//!   not drawn), cypress dominant on the exposed bluffs within
//!   [`BLUFF_CYPRESS_M`] of the shore (INFERRED from "broad-spreading,
//!   especially on exposed headlands", conifers.org).
//! * Marin Headlands: "Cool, windy, foggy conditions keep the Headlands
//!   largely treeless"; coastal scrub of coyote brush, California sage and
//!   poison oak on steep rocky soils, darker in the gullies; grassland on the
//!   ridges (NPS Golden Gate, Scrublands and Prairies pages). So the
//!   land-cover "tree" class north of the strait is drawn as dense coastal
//!   scrub, not forest (the reference sheet's note that ESA WorldCover counts
//!   the dense scrub there as tree is UNVERIFIED against imagery).
//! * Wildflowers: annuals "carpet" the grasslands in spring; rain falls almost
//!   all between November and April (NPS Golden Gate, Prairies; Weather). The
//!   bloom window here is days [`BLOOM_DAYS`] (March to mid-June, INFERRED).
//! * Rock: radiolarian chert underlies about half the Marin Headlands (NPS,
//!   Chert), "prevailingly dull brownish red" (USGS Geolex, Ingleside chert);
//!   "blue-green serpentinite" and grey-green serpentine outcrops in the cliffs
//!   between Fort Point and Baker Beach (Presidio Trust, Marshall's Beach; NPS,
//!   Baker Beach and Serpentinite). Colours are the Munsell chips those words
//!   name (chert 10R 3/3 dusky red, serpentinite 5G 4/2 greyish green, the
//!   Franciscan sandstone elsewhere 5Y 5/2 olive grey), as sRGB, INFERRED.
//!
//! Scale: every plant and rock is drawn at its real (published) size, not at
//! the 1.25 height scale of the terrain under it, as every model in the game is
//! (`eo_data::groundscale`: the plan and the terrain heights are scaled, the
//! models stay real size). So a 16 m cypress on a Marin hill drawn 1.25 times
//! its real height reads a fifth smaller against the hill than in life; scaling
//! the vegetation with the terrain is the owner's call (the plan's rule 4). The
//! test `plants_stand_at_real_size` holds it.

/// What the drawn ground carries at a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    /// A mown park or garden lawn: nothing drawn above it (the trees are the park's).
    Lawn,
    /// Unmown grassland (open ground, coastal prairie).
    WildGrass,
    /// Coastal scrub (a mapped scrub or heath, the land cover's shrubland).
    Scrub,
    /// Marin's dense coastal scrub (the land cover's tree class north of the strait).
    TallScrub,
    /// Dune scrub (low, grey-green, open sand between).
    DuneScrub,
    /// A mapped wood: its trees come from the park-tree lattice; understorey here.
    WoodGreen,
    /// Tree cover outside the mapped woods (south of the strait): coastal trees here.
    Wood,
    /// Bare earth: sparse tufts, a few stones.
    Bare,
    /// Mapped bare rock: outcrops.
    Rock,
    /// A face too steep for sand that the beach's sand tone runs up (Baker
    /// Beach's and Marshall's cliffs): outcrop with scrub between.
    Cliff,
}

/// The rock a slope is cut in (its colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lithology {
    Chert,
    Serpentinite,
    Sandstone,
}

/// The drawn ground at a point, as the app samples it.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    /// The drawn ground's height (m).
    pub y: f64,
    /// The drawn ground's unit normal.
    pub normal: [f64; 3],
    /// The ground's own colour there (linear), what a grass blade takes.
    pub ground: [f32; 3],
    /// What grows there; None: nothing (paved, built, water, sand, a footprint).
    pub cover: Option<Cover>,
    pub lithology: Lithology,
    /// Distance to the shoreline (m), for the bluff cypress.
    pub shore_m: f64,
    /// Ground curvature: positive in hollows and gullies (m of height below
    /// the mean of a ring 12 m out), negative on ridges.
    pub hollow_m: f64,
}

impl Site {
    /// Slope angle (degrees).
    pub fn slope_deg(&self) -> f64 {
        self.normal[1].clamp(-1.0, 1.0).acos().to_degrees()
    }
    /// The ground's gradient (dy/dx, dy/dz), held under 3.9 (75 degrees).
    pub fn grad(&self) -> [f32; 2] {
        let ny = self.normal[1].max(0.25);
        [(-self.normal[0] / ny).clamp(-3.9, 3.9) as f32, (-self.normal[2] / ny).clamp(-3.9, 3.9) as f32]
    }
    /// How much the slope faces north (+1 a north face, -1 south; 0 flat or east/west).
    pub fn north(&self) -> f64 {
        // world z points south: a north-facing slope's normal has -z.
        let h = (self.normal[0] * self.normal[0] + self.normal[2] * self.normal[2]).sqrt();
        if h < 1e-6 { 0.0 } else { -self.normal[2] / h * h.min(0.7) / 0.7 }
    }
}

/// What a plant is (the app maps it to a render model).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `eo_props::eztree::COASTAL_TREES[i]`: 0 cypress, 1 pine, 2 blue gum.
    Tree(u8),
    /// `COASTAL_SHRUBS[i]`: 0 coyote brush, 1 California sagebrush.
    Shrub(u8),
    Grass,
    Tuft,
    /// 0 yellow, 1 violet.
    Flower(u8),
    /// One of the rock shapes (0, 1 bedded).
    Rock(u8),
    /// A meadow patch (`--no-grass2` off): one of [`MEADOW_SWARD`] .. [`MEADOW_THATCH`].
    Meadow(u8),
}

/// One placed plant or rock.
#[derive(Clone, Copy, Debug)]
pub struct Plant {
    pub kind: Kind,
    pub pos: [f64; 3],
    pub yaw: f64,
    pub scale: [f32; 3],
    /// Uniform in [0, 1): the order the GPU thins it in with distance.
    pub rank: f32,
    /// Linear colour multiplier on the model (the shader packs it sRGB x 4).
    pub tint: [f32; 3],
    /// The ground's gradient under it (dy/dx, dy/dz): everything but the trees
    /// is sheared onto its slope by the shader (props_types.wgsl `pr_veg_grad`).
    pub grad: [f32; 2],
    /// The grass blades' tip colour (the lamp word's alpha, props_draw.wgsl): [`TIP_NONE`]
    /// none (every other plant, and `--no-grass-color-variety`), else [`meadow_tip`].
    pub tip: u8,
}

/// The kill switches this module's plants answer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switches {
    /// `--no-coastal-trees`: the mapped woods keep the park species; no coastal stands.
    pub coastal_trees: bool,
    /// `--no-ground-cover`: no grass, tufts, flowers or scrub.
    pub ground_cover: bool,
    /// `--no-hill-rock`: no rock outcrops.
    pub hill_rock: bool,
}

impl Switches {
    pub const ALL: Switches = Switches { coastal_trees: true, ground_cover: true, hill_rock: true };
    pub const NONE: Switches = Switches { coastal_trees: false, ground_cover: false, hill_rock: false };

    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-gg2-vegetation") {
            return Self::NONE;
        }
        Switches { coastal_trees: !has("--no-coastal-trees"), ground_cover: !has("--no-ground-cover"), hill_rock: !has("--no-hill-rock") }
    }

    pub fn any(&self) -> bool {
        self.coastal_trees || self.ground_cover || self.hill_rock
    }
}

/// The switches of this process (read once from the command line).
pub fn switches() -> Switches {
    static S: std::sync::OnceLock<Switches> = std::sync::OnceLock::new();
    *S.get_or_init(|| Switches::from_args(&std::env::args().collect::<Vec<_>>()))
}

/// The switch names (eo-app accepts them; `--no-hill-rock`, which `Switches`
/// also reads, is in `hill_rock::SWITCHES`).
pub const SWITCHES: &[&str] = &["--no-gg2-vegetation", "--no-coastal-trees", "--no-ground-cover"];

/// The coastal-vegetation track's switch names (the roll-up first); eo-app
/// accepts them, [`Gg3::from_args`] reads them (`--no-veg-scans` is reserved:
/// nothing reads it yet).
pub const GG3_SWITCHES: &[&str] = &["--no-gg3-vegetation", "--no-naip-planting", "--no-veg-scans", "--no-cliff-rock-cover", "--no-far-species", "--no-far-tree-light", "--no-bloom-date", "--no-grass-color-variety"];

/// The round-3 vegetation features, each with its `--no-` switch
/// ([`GG3_SWITCHES`]); `--no-gg3-vegetation` (or `--no-gg3`) turns them all off,
/// which restores the round-2 planting and far trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gg3 {
    /// `--no-naip-planting`: where scrub, trees and grass stand from the 0.6 m NAIP image ([`naip_class`]).
    pub naip_planting: bool,
    /// `--no-cliff-rock-cover`: no plant on a cliff over a beach, mapped bare rock or a sand cliff ([`plantable`]).
    pub cliff_rock_cover: bool,
    /// `--no-far-species`: the far trees take the near tier's coastal species and forms.
    pub far_species: bool,
    /// `--no-far-tree-light`: the far tree cards lit as a leaf volume (sphere normal from the eye, leaf transmission).
    pub far_tree_light: bool,
    /// `--no-bloom-date`: the cited bloom window ([`BLOOM_DAYS_CITED`]) and the flower heads in their own colour.
    pub bloom_date: bool,
    /// `--no-grass-color-variety`: the meadow's colour mosaic ([`meadow_tint`]): off, every
    /// bunch takes the ground's tone with the round-2 +-28 % value and cast.
    pub grass_color_variety: bool,
}

impl Gg3 {
    pub const ALL: Gg3 = Gg3 { naip_planting: true, cliff_rock_cover: true, far_species: true, far_tree_light: true, bloom_date: true, grass_color_variety: true };
    pub const NONE: Gg3 = Gg3 { naip_planting: false, cliff_rock_cover: false, far_species: false, far_tree_light: false, bloom_date: false, grass_color_variety: false };

    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-gg3-vegetation") || has("--no-gg3") {
            return Self::NONE;
        }
        Gg3 {
            naip_planting: !has("--no-naip-planting"),
            cliff_rock_cover: !has("--no-cliff-rock-cover"),
            far_species: !has("--no-far-species"),
            far_tree_light: !has("--no-far-tree-light"),
            bloom_date: !has("--no-bloom-date"),
            grass_color_variety: !has("--no-grass-color-variety"),
        }
    }
}

/// The second grass pass's switch names (the roll-up first); eo-app accepts
/// them, [`Grass2::from_args`] reads them. `--no-grass2` restores the first
/// pass's meadow (`--no-grass-color-variety` on, bunches and tufts on the
/// ground tone).
pub const GRASS2_SWITCHES: &[&str] = &["--no-grass2", "--no-meadow-cover", "--no-meadow-types", "--no-blade-shading"];

/// The second grass pass's features, each with its `--no-` switch ([`GRASS2_SWITCHES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grass2 {
    /// `--no-meadow-cover`: a meadow patch (a matted thatch and short-sward mat
    /// with blades over it) in every 1 m cell of grass and scrub cover, in place
    /// of the scattered bunches and tufts ([`plant_meadow`]).
    pub cover: bool,
    /// `--no-meadow-types`: the local types (wild oats, fescue tussocks, clover
    /// and forbs, ice plant, thatch) placed by slope, aspect, bluff edge and
    /// scrub, and low scrub mounds in the scrub's gaps; off, every patch is sward.
    pub types: bool,
    /// `--no-blade-shading`: the blades' leaf optics in props_draw.wgsl
    /// (transmission, midrib and curl normals, clump occlusion, waxy sheen).
    pub blade_shading: bool,
}

impl Grass2 {
    pub const ALL: Grass2 = Grass2 { cover: true, types: true, blade_shading: true };
    pub const NONE: Grass2 = Grass2 { cover: false, types: false, blade_shading: false };

    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-grass2") {
            return Self::NONE;
        }
        Grass2 { cover: !has("--no-meadow-cover"), types: !has("--no-meadow-types"), blade_shading: !has("--no-blade-shading") }
    }
}

/// The second grass pass's switches of this process (read once from the command line).
pub fn grass2() -> Grass2 {
    static S: std::sync::OnceLock<Grass2> = std::sync::OnceLock::new();
    *S.get_or_init(|| Grass2::from_args(&std::env::args().collect::<Vec<_>>()))
}

/// The third grass pass's switch names (the roll-up first); eo-app accepts
/// them, [`Grass3::from_args`] reads them. `--no-grass3` restores the second
/// pass's output. Every feature acts inside the second pass's meadow cover
/// (`--no-meadow-cover` or `--no-grass2` leave nothing for it to change).
pub const GRASS3_SWITCHES: &[&str] = &["--no-grass3", "--no-scrub-mounds", "--no-scrub-bloom", "--no-bare-ground", "--no-meadow-breakup"];

/// The third grass pass's features, each with its `--no-` switch ([`GRASS3_SWITCHES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grass3 {
    /// `--no-scrub-mounds`: lumpy coastal scrub mounds (coyote brush, California
    /// sagebrush, bush lupine; 0.5-1.5 m tall, leaf clumps) where the land cover
    /// and the NAIP image show scrub and on the Marin headland's seaward slopes
    /// ([`scrub_share`], [`MEADOW_COYOTE`] ..).
    pub scrub: bool,
    /// `--no-scrub-bloom`: in the bloom window the bush lupine mounds carry
    /// their yellow flower spikes, the lupines grouped in stands (off: coyote brush there).
    pub bloom: bool,
    /// `--no-bare-ground`: eroded ground (steep faces and red-brown soil in the
    /// NAIP image, [`bare_soil`]) stays bare red-brown soil with sparse grass
    /// instead of carrying the meadow; the terrain draws its soil there.
    pub bare: bool,
    /// `--no-meadow-breakup`: the meadow's type and cure fields domain-warped into
    /// irregular shapes and mixed at their edges, the patch levels' distances
    /// jittered per patch (no level band across the view) and the mats' rims
    /// lobed so the terrain's tone shows between them.
    pub breakup: bool,
}

impl Grass3 {
    pub const ALL: Grass3 = Grass3 { scrub: true, bloom: true, bare: true, breakup: true };
    pub const NONE: Grass3 = Grass3 { scrub: false, bloom: false, bare: false, breakup: false };

    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-grass3") {
            return Self::NONE;
        }
        Grass3 { scrub: !has("--no-scrub-mounds"), bloom: !has("--no-scrub-bloom"), bare: !has("--no-bare-ground"), breakup: !has("--no-meadow-breakup") }
    }
}

/// The third grass pass's switches of this process (read once from the command
/// line), with the second pass's meadow cover they act in.
pub fn grass3() -> Grass3 {
    static S: std::sync::OnceLock<Grass3> = std::sync::OnceLock::new();
    *S.get_or_init(|| {
        let g = Grass3::from_args(&std::env::args().collect::<Vec<_>>());
        if grass2().cover { g } else { Grass3::NONE }
    })
}

/// The round-3 switches of this process (read once from the command line).
pub fn gg3() -> Gg3 {
    static S: std::sync::OnceLock<Gg3> = std::sync::OnceLock::new();
    *S.get_or_init(|| Gg3::from_args(&std::env::args().collect::<Vec<_>>()))
}

/// Whether a cover carries plants (`--no-cliff-rock-cover`: every cover but a
/// lawn does, as in round 2). A cliff over a beach (eo-app's cliff band, the
/// ground the sand limit takes from the beach: steeper than
/// `ground_cover::SAND_SLOPE_MAX`) and mapped bare rock are rock cover: the
/// rock layer dresses them, nothing grows on them. Sand, paving, paths,
/// buildings and water never reach here (the site is None there).
pub fn plantable(cover: Cover, g3: Gg3) -> bool {
    match cover {
        Cover::Lawn => false,
        Cover::Cliff | Cover::Rock => !g3.cliff_rock_cover,
        _ => true,
    }
}

/// The bloom window, days of the year, before `--no-bloom-date` (round 2's,
/// INFERRED from the NPS words "spring"; March to mid-June).
pub const BLOOM_DAYS: (f64, f64) = (60.0, 170.0);
/// The bloom window of the headlands' spring wildflowers, days of the year:
/// 1 March to 31 May. The California Native Plant Society's Calscape pages
/// (read 18 Nov 2026) give California poppy (Eschscholzia californica)
/// flowers "from February to September, though primarily in spring" and sky
/// lupine (Lupinus nanus) the season "Spring, Summer"; the window is the
/// spring both share, taken as the meteorological spring, March to May
/// (INFERRED: the month bounds of "spring"; the Jepson eFlora treatments
/// could not be re-read past their bot check). The poppy's tail past May is
/// sparse on the summer-dry headlands and is not drawn.
pub const BLOOM_DAYS_CITED: (f64, f64) = (60.0, 151.0);

/// How much of a 0.6 m NAIP box is tall woody canopy (scrub, tree crowns),
/// low cover (grass, herbs; the summer image's cured annual grass), and bare
/// ground (paving, rock, sand, eroded soil): shares of the pixels it could
/// read (shadow and water are left out).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Naip {
    pub canopy: f32,
    pub low: f32,
    pub bare: f32,
    /// The part of `low` that passed the green test (bright, green over red):
    /// in the summer image the annual grass has cured, so a bright green pixel
    /// on the headlands is sunlit evergreen scrub (coyote brush, lupine) that the
    /// canopy rule misses (its misses are "sunlit scrub brighter than 0.38").
    pub lush: f32,
    /// Red-brown soil: the pixels that are not green, red over green by
    /// [`NAIP_SOIL_RG`] and not pale ([`NAIP_BARE_V`]): eroded Tamalpais soil
    /// (5YR hue), where cured grass is a yellower tan (red near green).
    pub soil: f32,
    /// Warm bare ground: not green and red over green by [`NAIP_WARM_RG`]
    /// (thin, eroded soil showing through; on a knoll top, [`bare_soil`]).
    pub warm: f32,
}

/// Red over green past which a non-green pixel is warm (soil showing): over
/// the cured grass's median (1.00) by its spread; the headland knoll tops the
/// photographs show bare read 1.08-1.14 in the image (probed). INFERRED.
pub const NAIP_WARM_RG: f32 = 1.06;

/// Red over green past which a non-green pixel is red-brown soil, not cured
/// grass ([`Naip::soil`]). The Tamalpais series' eroded B2t horizon, dry 5YR 4/4
/// (sRGB 126, 90, 68, terrain_detail.rs `SOIL_ERODED_SRGB`), is 1.40; the
/// cured grass's red over green median on the headlands is 1.00, its p90
/// 1.09 ([`MEADOW_LEAN`]'s measurement). INFERRED between them.
pub const NAIP_SOIL_RG: f32 = 1.16;

/// One NAIP pixel's class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NaipClass {
    Canopy,
    Low,
    Bare,
    /// Shadow, water, too dark to read.
    Unknown,
}

// The classifier. Green vegetation is excess green ExG = 2g - r - b over 0
// on the chromatic coordinates (r = R / (R + G + B), ...; Woebbecke, Meyer,
// Von Bargen and Mortensen 1995, Trans. ASAE 38(1) 259-269) with green over
// red (the sign of VARI = (G - R) / (G + R - B), Gitelson, Kaufman, Stark
// and Rundquist 2002, Remote Sens. Environ. 80, 76-87). What separates woody
// canopy from grass in the summer image is brightness: a shrub or crown is
// self-shaded, a sward is not. The brightness is V = (R + G + B) / 765,
// averaged over the 5 x 5 pixels round the pixel (the JPEG's and the
// canopy's pixel noise). The three brightness thresholds were chosen on a
// hand-labelled set of NAIP cells (16 x 16 pixels, labelled from the image
// only: Marin scrub, cypress and pine, cured grass, trails and the battery's
// concrete; 316 cells) as the best mean pixel recall, and checked on a
// held-out set (the Baker bluff, Lincoln Park, the chert faces west of
// Kirby Cove; 72 cells). Held-out pixel recall: canopy 0.66, low 0.98, bare
// 0.94 (round-3 first rule: 0.61, 0.92, 0.98); held-out cells (canopy where
// its share is a third or more): canopy 43 of 54, low 4 of 4, bare 13 of 14.
// The misses are sunlit scrub and crowns brighter than 0.38. A per-pixel
// texture (luma spread in 3 x 3 or 5 x 5) does not separate the classes at
// 0.6 m (5 x 5 spread medians: canopy 0.020-0.025, grass 0.010-0.023), so
// none is used.

/// A green pixel darker than this (5 x 5 mean V) is canopy, else low cover.
pub const NAIP_CANOPY_V: f32 = 0.38;
/// A pixel that misses the green test but is darker than this (5 x 5 mean V)
/// and greener than blue (B < G) is canopy too: the evergreen sage scrub's
/// grey-green, where the summer's cured grass is a brighter tan.
pub const NAIP_SAGE_V: f32 = 0.39;
/// Not green and brighter than this (5 x 5 mean V): bare (paving, pale rock,
/// sand, the battery's concrete; the cured grass's median is 0.44-0.53).
pub const NAIP_BARE_V: f32 = 0.55;
/// Darker than this (one pixel) and not green: unknown (shadow, water);
/// darker than [`NAIP_SHADOW_V`]: unknown whatever its hue (INFERRED: deep shadow).
pub const NAIP_DARK_V: f32 = 0.30;
pub const NAIP_SHADOW_V: f32 = 0.20;
/// Not green on ground steeper than this (degrees): an eroded face or rock,
/// not cured grass (INFERRED: the steepest grassland slopes of the headlands
/// are about 35 degrees; the red-brown chert faces are steeper).
pub const NAIP_BARE_SLOPE_DEG: f64 = 35.0;

/// The class of one NAIP pixel (sRGB bytes as delivered), with the mean
/// brightness of the 5 x 5 pixels round it (`v5`, 0-1) and the ground's
/// slope (degrees). NAIP is a summer image: a tan, non-green pixel on gentle
/// ground is cured annual grass (green in spring: the season model colours
/// it, not this).
pub fn naip_class(rgb: [u8; 3], v5: f32, slope_deg: f64) -> NaipClass {
    let [r, g, b] = rgb.map(|c| c as f32);
    let s = r + g + b;
    if s < 1.0 {
        return NaipClass::Unknown;
    }
    let exg = (2.0 * g - r - b) / s;
    let v = s / 765.0;
    if v < NAIP_SHADOW_V {
        return NaipClass::Unknown;
    }
    if exg > 0.0 && g > r {
        return if v5 < NAIP_CANOPY_V { NaipClass::Canopy } else { NaipClass::Low };
    }
    if b < g && v5 < NAIP_SAGE_V {
        return NaipClass::Canopy;
    }
    if v < NAIP_DARK_V {
        return NaipClass::Unknown;
    }
    // bright, or grey-blue (serpentinite, concrete in the sky's light): bare.
    if v5 > NAIP_BARE_V || (b >= g && b >= r) || slope_deg > NAIP_BARE_SLOPE_DEG {
        return NaipClass::Bare;
    }
    NaipClass::Low
}

/// The shares of a box of NAIP pixels (None: nothing readable). `px` is a
/// `w` x `h` block, row-major, that holds the box and a margin of two pixels
/// on every side (the 5 x 5 brightness windows); a pixel outside the image
/// is None. Only the box's pixels are counted.
pub fn naip_shares(px: &[Option<[u8; 3]>], w: usize, h: usize, slope_deg: f64) -> Option<Naip> {
    if w < 5 || h < 5 || px.len() != w * h {
        return None;
    }
    let v: Vec<Option<f32>> = px.iter().map(|p| p.map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 765.0)).collect();
    let mut n = [0u32; 4];
    let (mut lush, mut soil, mut warm) = (0u32, 0u32, 0u32);
    for j in 2..h - 2 {
        for i in 2..w - 2 {
            let Some(p) = px[j * w + i] else { continue };
            let (mut sum, mut k) = (0.0f32, 0u32);
            for dj in 0..5 {
                for di in 0..5 {
                    if let Some(x) = v[(j + dj - 2) * w + i + di - 2] {
                        sum += x;
                        k += 1;
                    }
                }
            }
            let v5 = sum / k as f32;
            let class = naip_class(p, v5, slope_deg);
            n[class as usize] += 1;
            let [r, g, b] = p.map(|c| c as f32);
            let green = 2.0 * g - r - b > 0.0 && g > r;
            if class == NaipClass::Low && green {
                lush += 1;
            }
            if matches!(class, NaipClass::Low | NaipClass::Bare) && !green && r > b && v5 <= NAIP_BARE_V {
                if r >= NAIP_SOIL_RG * g {
                    soil += 1;
                }
                if r >= NAIP_WARM_RG * g {
                    warm += 1;
                }
            }
        }
    }
    let known = (n[0] + n[1] + n[2]) as f32;
    (known > 0.0).then(|| Naip { canopy: n[0] as f32 / known, low: n[1] as f32 / known, bare: n[2] as f32 / known, lush: lush as f32 / known, soil: soil as f32 / known, warm: warm as f32 / known })
}

/// What a planting pass reads besides the ground: the round-3 switches and
/// the NAIP shares at a point (`naip`: None outside the image or without it).
pub struct Extra<'a> {
    pub g3: Gg3,
    /// The second grass pass ([`Grass2`]).
    pub g2: Grass2,
    /// The third grass pass ([`Grass3`]).
    pub g3x: Grass3,
    /// The flora pass ([`crate::flora::Flora`]).
    pub fl: crate::flora::Flora,
    /// The shares round a point in a box of about the given side (m).
    pub naip: Box<dyn Fn(f64, f64, f64) -> Option<Naip> + 'a>,
}

impl Extra<'_> {
    /// Round 2's planting (no NAIP, every round-3 switch off).
    pub fn none() -> Extra<'static> {
        Extra { g3: Gg3::NONE, g2: Grass2::NONE, g3x: Grass3::NONE, fl: crate::flora::Flora::NONE, naip: Box::new(|_, _, _| None) }
    }
    fn at(&self, x: f64, z: f64, side: f64) -> Option<Naip> {
        if self.g3.naip_planting { (self.naip)(x, z, side) } else { None }
    }
}

/// A tree needs this much canopy in the NAIP box round its trunk (a 6 m box:
/// a crown is 8-16 m across, so its trunk box is crown where a tree stands;
/// INFERRED).
pub const NAIP_TREE_CANOPY: f32 = 0.3;
/// The box a tree's canopy is read in (m).
pub const NAIP_TREE_BOX_M: f64 = 6.0;
/// The box a shrub or grass bunch reads (m; at the 1.25 ground scale 2.4 m
/// real: five NAIP columns by three rows, eo-app `naip_at`).
pub const NAIP_PLANT_BOX_M: f64 = 3.0;
/// Cypress dominate the stands this near the shore (m, INFERRED).
pub const BLUFF_CYPRESS_M: f64 = 160.0;
/// The coarse layer's tree lattice (m): the park-tree lattice's own pitch (tile-trees.js).
pub const TREE_PITCH_M: f64 = 14.0;
/// The shrub lattice (m): a coyote brush mound is 2.6 m across at scale 1 (coastal.rs), so a
/// 3.4 m pitch at the scrub's full chance gives a closed canopy with gaps (INFERRED).
pub const SHRUB_PITCH_M: f64 = 3.4;
/// How far a grass bunch, tuft or flower clump stands into the ground (m).
pub const GRASS_SINK_M: f64 = 0.07;
/// The fine layer's lattice (m): one grass patch (1.1 m across) a cell, so the
/// patches overlap into a sward.
pub const GRASS_PITCH_M: f64 = 1.0;
/// Grass blade tint over the ground tone: 1 / (the atlas grass cell's mean
/// linear albedo 0.311, measured over its texels with alpha over 0.5, x the
/// card's mean linear vertex shade, about 0.57: tips at sRGB 200-255, the base
/// at three quarters of that, `vegetation_models::GRASS_BASE_SHADE_PER_4`), so
/// a bunch's mean albedo is the ground tone's (the ground's tone is what the
/// grass is) and its lit tips a little over it.
pub const GRASS_GAIN: f32 = 5.6;
/// Leaf tints of the coastal trees and shrubs (`Kind::Tree`, `Kind::Shrub`
/// order): what brings each species' mean leaf albedo (its cooked leaf
/// vertex colour times the atlas cell's mean linear colour, measured with
/// `eo-props --example coastal_preview`) to a green leaf's visible reflectance,
/// about 0.03-0.05 blue, 0.06-0.10 green, 0.03-0.05 red (the general
/// green-vegetation curve; the conifers at its dark end, the blue gum's
/// glaucous leaf and the sage's grey hairs bluer and lighter: INFERRED).
/// Targets (linear r, g, b): cypress 0.035/0.060/0.030, pine 0.040/0.065/0.030,
/// blue gum 0.050/0.075/0.060, coyote brush 0.040/0.075/0.030, sage 0.070/0.085/0.050.
///
/// The shrubs' blue is then cut to what the frame needs (calibrated in the
/// render, not from a photograph): a shrub's leaves sit mostly in its own
/// shade, lit by the blue sky, so the albedo targets drew coyote brush at
/// sRGB (87, 104, 88) and the sage, whose atlas cell (the pine needles, mean
/// linear 0.118/0.181/0.030) needs a blue gain near three to reach its target,
/// lavender at (124, 125, 142) on the Marin slopes (linear r/g 0.67 and 0.99,
/// b/g 0.69 and 1.32, against the targets' 0.53 and 0.82, 0.40 and 0.59). Each
/// channel's gain is scaled by target ratio over drawn ratio (green kept), so
/// blue gain over green gain falls from 3.3 to 1.9 for coyote brush and from 2.4
/// to 1.1 for the sage (see the test `shrub_leaves_are_green_not_blue`).
pub const TREE_LEAF_TINT: [[f32; 3]; 3] = [[1.19, 1.08, 3.31], [1.13, 1.04, 2.88], [1.58, 1.00, 2.96]];
pub const SHRUB_LEAF_TINT: [[f32; 3]; 2] = [[1.27, 0.98, 1.89], [1.30, 1.15, 1.24]];
/// The shrubs' measured mean leaf albedo before their tint (`Kind::Shrub`
/// order): the cooked leaf vertex colour (`eo-props --example coastal_preview`,
/// near tier; coarse and distant within 3 %) times the atlas cell's mean linear
/// colour over its texels with alpha over 0.5 (coyote brush: aspen
/// 0.083/0.214/0.030 x vertex 0.295/0.354/0.295; sage: pine 0.118/0.181/0.030
/// x vertex 0.377/0.406/0.580).
pub const SHRUB_LEAF_ALBEDO: [[f32; 3]; 2] = [[0.0246, 0.0756, 0.0090], [0.0445, 0.0733, 0.0176]];

pub use crate::hill_rock::{CHERT_SRGB, ROCK_PITCH_M, ROCK_SHADE, ROCK_SHEAR, SANDSTONE_SRGB, SERPENTINITE_SRGB, outcrop_patch, rock_seat};

/// A hash of a lattice cell and a salt, uniform in [0, 1).
pub fn hash(ix: i64, iz: i64, salt: u32) -> f64 {
    let mut h = (ix as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (iz as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f) ^ (salt as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise in [0, 1] at `cell` metres.
pub fn field(x: f64, z: f64, cell: f64, salt: u32) -> f64 {
    let (gx, gz) = (x / cell, z / cell);
    let (ix, iz) = (gx.floor(), gz.floor());
    let (fx, fz) = (gx - ix, gz - iz);
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let (ix, iz) = (ix as i64, iz as i64);
    let a = hash(ix, iz, salt);
    let b = hash(ix + 1, iz, salt);
    let c = hash(ix, iz + 1, salt);
    let d = hash(ix + 1, iz + 1, salt);
    (a + (b - a) * ux) * (1.0 - uz) + (c + (d - c) * ux) * uz
}

pub(crate) fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The coastal species and its scale for a tree of a mapped wood (the
/// park-tree lattice's own point, `hash` its cell's draw): (species index,
/// uniform scale, height factor).
pub fn coastal_tree(x: f64, z: f64, shore_m: f64, draw: f64) -> (u8, f32, f32) {
    let (ix, iz) = ((x * 4.0).floor() as i64, (z * 4.0).floor() as i64);
    let bluff = 1.0 - smoothstep(BLUFF_CYPRESS_M * 0.5, BLUFF_CYPRESS_M * 1.5, shore_m);
    // blue gum 0.45, cypress 0.30, pine 0.25 inland; the bluffs mostly cypress.
    let cypress = 0.30 + 0.55 * bluff;
    let gum = 0.45 * (1.0 - bluff);
    let species = if draw < cypress {
        0
    } else if draw < cypress + gum {
        2
    } else {
        1
    };
    let s = 0.78 + 0.4 * hash(ix, iz, 71) as f32;
    // the exposed bluff trees are lower for their spread (wind-shorn, INFERRED).
    let h = 1.0 - 0.22 * bluff as f32;
    (species, s, h)
}

/// A point inside the Presidio: the Main Post's parade ground (37.7989 N,
/// 122.4662 W, Presidio Trust maps); the largest park green round it is the
/// Presidio's mapped polygon (eo-app props/vegetation.rs reads the same point).
pub const PRESIDIO_POINT: (f64, f64) = (37.7989, -122.4662);

/// Game x, z of a geographic point (lat, lon): the surveyed frame times the ground-plan scale.
pub fn game_xz(lat: f64, lon: f64) -> (f64, f64) {
    let f = eo_data::Frame::sf();
    let s = eo_data::groundscale::scale();
    ((lon - f.center_lon) * f.m_per_deg_lon * s, (f.center_lat - lat) * f.m_per_deg_lat * s)
}

type ShoreFn = Box<dyn Fn(f64, f64) -> f64 + Send + Sync>;
static SHORE: std::sync::OnceLock<ShoreFn> = std::sync::OnceLock::new();

/// Install the distance to the shoreline (m, at game x, z) for the far tier's
/// bluff cypress (the first install wins). The app installs the citywide shore
/// field's magnitude once the water is loaded.
pub fn install_shore_distance(f: ShoreFn) {
    let _ = SHORE.set(f);
}

/// The installed shore distance at (x, z), if any.
pub fn shore_distance(x: f64, z: f64) -> Option<f64> {
    SHORE.get().map(|f| f(x, z))
}

/// The near tier's coastal species' mature size at scale 1 (height, half
/// crown, m; `Kind::Tree` order): eo-props eztree/coastal.rs (cypress 16 m and
/// 10 m, pine 24 m and 7 m, blue gum 34 m and 7.5 m), so a far card is the
/// tree the near tier draws.
pub const FAR_TREE_SIZE: [(f32, f32); 3] = [(16.0, 10.0), (24.0, 7.0), (34.0, 7.5)];
/// The species' leaf albedo targets (linear; [`TREE_LEAF_TINT`]'s notes): the
/// far card's crown colour.
pub const FAR_TREE_ALBEDO: [[f32; 3]; 3] = [[0.035, 0.060, 0.030], [0.040, 0.065, 0.030], [0.050, 0.075, 0.060]];

/// A far card's form for a coastal tree (`--no-far-species`: not used): the
/// species ([`coastal_tree`], the near tier's own draw), its size and crown
/// colour (sRGB bytes) varied as the near tier's tint is.
pub fn far_form(x: f64, z: f64, shore_m: f64, draw: f64) -> (u8, f32, f32, [u8; 3]) {
    let (sp, s, hf) = coastal_tree(x, z, shore_m, draw);
    let (h, c) = FAR_TREE_SIZE[sp as usize];
    let v = 0.85 + 0.3 * hash((x * 8.0) as i64, (z * 8.0) as i64, 92) as f32;
    let enc = |l: f32| {
        let l = l.clamp(0.0, 1.0);
        let e = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        (e * 255.0).round() as u8
    };
    let a = FAR_TREE_ALBEDO[sp as usize];
    (sp, h * s * hf, c * s, [enc(a[0] * v), enc(a[1] * v), enc(a[2] * v)])
}

/// The coarse layer over [x0, x1) x [z0, z1): coastal trees on unmapped tree
/// cover, shrubs, rock outcrops.
pub fn plant_coarse(x0: f64, z0: f64, x1: f64, z1: f64, site: &dyn Fn(f64, f64) -> Option<Site>, sw: Switches, out: &mut Vec<Plant>) {
    plant_coarse_with(x0, z0, x1, z1, site, sw, &Extra::none(), out)
}

/// Whether the NAIP image lets a tree stand at (x, z) (no image there, or
/// enough canopy round the trunk).
pub fn naip_tree_ok(ex: &Extra, x: f64, z: f64) -> bool {
    ex.at(x, z, NAIP_TREE_BOX_M).is_none_or(|n| n.canopy >= NAIP_TREE_CANOPY)
}

/// `plant_coarse` with the round-3 rules: nothing on rock cover
/// ([`plantable`]); within the NAIP image, trees only under mapped canopy and
/// the scrub where the image shows canopy (closed patches with open grass
/// between, as the NPS describes the headlands), not by a noise field.
#[allow(clippy::too_many_arguments)]
pub fn plant_coarse_with(x0: f64, z0: f64, x1: f64, z1: f64, site: &dyn Fn(f64, f64) -> Option<Site>, sw: Switches, ex: &Extra, out: &mut Vec<Plant>) {
    let g3 = ex.g3;
    // trees (the park-tree lattice's pitch, its jitter, a grove field).
    if sw.coastal_trees {
        lattice(x0, z0, x1, z1, TREE_PITCH_M, 1, &mut |x, z, ix, iz| {
            let Some(s) = site(x, z) else { return };
            if s.cover != Some(Cover::Wood) || s.slope_deg() > 42.0 || !naip_tree_ok(ex, x, z) {
                return;
            }
            let grove = 0.55 + 0.45 * field(x, z, 70.0, 11);
            if hash(ix, iz, 3) >= 0.8 * grove {
                return;
            }
            let (sp, sc, hf) = coastal_tree(x, z, s.shore_m, hash(ix, iz, 4));
            let v = 0.85 + 0.3 * hash(ix, iz, 6) as f32;
            out.push(Plant { kind: Kind::Tree(sp), pos: [x, s.y - 0.08, z], yaw: hash(ix, iz, 5) * std::f64::consts::TAU, scale: [sc, sc * hf, sc], rank: hash(ix, iz, 8) as f32, tint: TREE_LEAF_TINT[sp as usize].map(|c| c * v), grad: [0.0; 2], tip: TIP_NONE });
        });
    }
    if sw.ground_cover {
        lattice(x0, z0, x1, z1, SHRUB_PITCH_M, 2, &mut |x, z, ix, iz| {
            let Some(s) = site(x, z) else { return };
            let Some(cover) = s.cover else { return };
            if !plantable(cover, g3) {
                return;
            }
            let slope = s.slope_deg();
            // a patch field (about 30 m) so the scrub reads as a mosaic, darker and denser in hollows and on north faces.
            let patch = field(x, z, 30.0, 21);
            let shelter = (s.north() * 0.25 + (s.hollow_m * 0.35).clamp(-0.3, 0.4)) as f64;
            // within the NAIP image: the scrub where the image shows canopy, closed where
            // it is dense (a shrub at nearly every 3.4 m point, grown to close the gaps),
            // none on the open grass and bare ground between. Under the mapped woods the
            // canopy is the trees': the understorey keeps its round-2 share there.
            let naip = ex.at(x, z, NAIP_PLANT_BOX_M).filter(|_| !matches!(cover, Cover::Wood | Cover::WoodGreen));
            let chance = if let Some(n) = naip {
                0.02 + 0.98 * smoothstep(0.12, 0.55, n.canopy as f64)
            } else {
                match cover {
                    Cover::TallScrub => 0.62 + 0.3 * patch + shelter,
                    Cover::Scrub => 0.5 + 0.35 * patch + shelter,
                    Cover::DuneScrub => 0.2 + 0.35 * patch,
                    Cover::WildGrass => (smoothstep(0.38, 0.72, patch) * 0.8 + shelter * 0.8).max(0.0),
                    Cover::Wood | Cover::WoodGreen => 0.12 + 0.12 * patch,
                    Cover::Bare | Cover::Rock => 0.05,
                    // scrub in the cliff's gaps between the outcrops (the rock layer's patches).
                    Cover::Cliff => 0.45 * (1.0 - outcrop_patch(x, z, cover)) * (0.6 + 0.4 * patch),
                    Cover::Lawn => 0.0,
                }
            } * (1.0 - smoothstep(38.0, 55.0, slope) * if cover == Cover::Cliff { 0.6 } else { 1.0 });
            if hash(ix, iz, 3) >= chance {
                return;
            }
            // closed scrub: the mounds in a dense patch a third wider (not taller), so neighbours touch.
            let close = naip.map(|n| 1.0 + 0.35 * smoothstep(0.4, 0.8, n.canopy as f64) as f32).unwrap_or(1.0);
            let sage = match cover {
                Cover::DuneScrub => 0.7,
                _ => 0.22 + 0.2 * field(x, z, 18.0, 23),
            };
            let kind = if hash(ix, iz, 4) < sage { Kind::Shrub(1) } else { Kind::Shrub(0) };
            let big = match cover {
                Cover::TallScrub => 1.15,
                Cover::DuneScrub => 0.75,
                Cover::Wood | Cover::WoodGreen => 0.85,
                _ => 1.0,
            } as f32;
            let sc = big * (0.75 + 0.6 * hash(ix, iz, 5) as f32);
            let wide = 0.85 + 0.35 * hash(ix, iz, 9) as f32;
            // hollow-grown scrub a touch darker; each mound its own shade (INFERRED variation).
            let v = 0.82 + 0.3 * hash(ix, iz, 6) as f32 - (s.hollow_m.clamp(0.0, 1.0) * 0.1) as f32;
            // the mound follows the slope (the shear); sunk a little so no edge floats.
            let sink = 0.06 + 0.06 * sc as f64;
            let mut base = SHRUB_LEAF_TINT[if kind == Kind::Shrub(1) { 1 } else { 0 }];
            if g3.grass_color_variety {
                // the scrub's own mosaic: olive (warmer, coyote brush in sun) to grey-green
                // (sage, lizardtail) by a 25 m patch and each mound (+-10 % red, +-12 % blue, INFERRED).
                let o = (1.4 * (field(x, z, 25.0, 68) - 0.5) + 0.6 * (hash(ix, iz, 69) - 0.5)).clamp(-1.0, 1.0) as f32;
                base = [base[0] * (1.0 + 0.1 * o), base[1], base[2] * (1.0 - 0.12 * o)];
            }
            out.push(Plant { kind, pos: [x, s.y - sink, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc * wide * close, sc, sc * wide * close], rank: hash(ix, iz, 8) as f32, tint: base.map(|c| c * v), grad: s.grad(), tip: TIP_NONE });
        });
    }
    // rock outcrops (crate::hill_rock), after the trees and scrub as before.
    if sw.hill_rock {
        crate::hill_rock::plant(x0, z0, x1, z1, site, out);
    }
}

/// The fine layer over [x0, x1) x [z0, z1): grass bunches, short tufts and,
/// on `day_of_year` inside the bloom window, wildflowers.
pub fn plant_fine(x0: f64, z0: f64, x1: f64, z1: f64, site: &dyn Fn(f64, f64) -> Option<Site>, day_of_year: f64, sw: Switches, out: &mut Vec<Plant>) {
    plant_fine_with(x0, z0, x1, z1, site, day_of_year, sw, &Extra::none(), out)
}

/// How far into its bloom window a day is (0 out of it, 1 in its middle;
/// three-week ramps at each end).
pub fn bloom(day_of_year: f64, g3: Gg3) -> f64 {
    let (a, b) = if g3.bloom_date { BLOOM_DAYS_CITED } else { BLOOM_DAYS };
    smoothstep(a, a + 21.0, day_of_year) * (1.0 - smoothstep(b - 21.0, b, day_of_year))
}

/// `plant_fine` with the round-3 rules: nothing on rock cover; within the
/// NAIP image the sward on the low cover (and in the scrub's gaps), tufts on
/// the bare ground, and in the bloom window the wildflowers on the open grass.
#[allow(clippy::too_many_arguments)]
pub fn plant_fine_with(x0: f64, z0: f64, x1: f64, z1: f64, site: &dyn Fn(f64, f64) -> Option<Site>, day_of_year: f64, sw: Switches, ex: &Extra, out: &mut Vec<Plant>) {
    if !sw.ground_cover {
        return;
    }
    let g3 = ex.g3;
    let bloom = bloom(day_of_year, g3);
    // the flora pass (`--no-grass-ramp`): the guide's month table for the meadow's cure.
    let season = if ex.fl.ramp { crate::flora::grass_cure(day_of_year) } else { eo_look::season::dryness(day_of_year) as f64 };
    lattice(x0, z0, x1, z1, GRASS_PITCH_M, 4, &mut |x, z, ix, iz| {
        let Some(s) = site(x, z) else { return };
        let Some(cover) = s.cover else { return };
        if !plantable(cover, g3) {
            return;
        }
        let slope = s.slope_deg();
        let patch = field(x, z, 9.0, 41);
        let naip = ex.at(x, z, NAIP_PLANT_BOX_M).filter(|_| !matches!(cover, Cover::Wood | Cover::WoodGreen | Cover::Lawn));
        // (grass, tuft, flower) chances by cover; within the NAIP image by what it shows:
        // a near-continuous sward on the low cover (the 1.3 m patches overlap at 0.9),
        // grass at the scrub's edges and gaps, sparse tufts on bare ground, and the
        // spring flowers on the open grass (INFERRED shares).
        let (g, t, f) = if let Some(n) = naip {
            let (c, l, b) = (n.canopy as f64, n.low as f64, n.bare as f64);
            (0.92 * l + 0.45 * c, 0.12 * b + 0.05 * l, 0.16 * l)
        } else {
            match cover {
                Cover::WildGrass => (0.6 + 0.3 * patch, 0.1, 0.06),
                Cover::Scrub | Cover::TallScrub => (0.45 + 0.3 * patch, 0.1, 0.03),
                Cover::DuneScrub => (0.08, 0.35 + 0.2 * patch, 0.02),
                // a mown lawn carries nothing a bunch or tuft would draw (its blades are under 5 cm).
                Cover::Lawn => return,
                Cover::Wood | Cover::WoodGreen => (0.06, 0.18 * patch, 0.0),
                Cover::Bare => (0.04, 0.1 * patch, 0.01),
                Cover::Rock => (0.1, 0.08, 0.02),
                Cover::Cliff => (0.12 * (1.0 - outcrop_patch(x, z, cover)), 0.12, 0.01),
            }
        };
        let steep = 1.0 - 0.6 * smoothstep(40.0, 60.0, slope);
        let r = hash(ix, iz, 3);
        let flowers = f * bloom * smoothstep(0.35, 0.8, field(x, z, 14.0, 43));
        if ex.fl.bluff && ex.g2.cover && s.lithology == Lithology::Serpentinite && z > marin_z() && matches!(cover, Cover::Cliff | Cover::Bare | Cover::Rock) {
            // the flora pass: the serpentine bluff faces' sparse low scrub over the rock
            // (20-50 % plant cover, field guide 1.4, estimate), none on the steepest faces.
            let share = 0.45 * (1.0 - outcrop_patch(x, z, cover)) * (1.0 - smoothstep(48.0, 62.0, slope)) + 0.1;
            if hash(ix, iz, 151) < share {
                let cured = meadow_cured(x, z, &s, season, ex.g3x.breakup);
                let kind = crate::flora::bluff_kind(x, z, ix, iz);
                let sc = 0.7 + 0.3 * hash(ix, iz, 4) as f32;
                out.push(Plant { kind: Kind::Meadow(kind), pos: [x, s.y - GRASS_SINK_M, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc; 3], rank: hash(ix, iz, 8) as f32, tint: meadow_patch_tint(kind, cured, 0.93 + 0.14 * hash(ix, iz, 20) as f32), grad: s.grad(), tip: TIP_NONE });
            }
            return;
        }
        if ex.g2.cover && meadow_ground(cover, naip) {
            // the third pass: bare eroded ground stays bare (a sparse tuft in some
            // cells), and the scrub stands as mounds in place of the meadow patch.
            if ex.g3x.bare && hash(ix, iz, 110) < bare_soil(x, z, &s, naip) {
                // scrub stands over the soil on a ridge's flanks (the seaward slopes'
                // mounds with the eroded soil between: the photographs' foreground),
                // not on its flatter top or on a steep face.
                let flank = smoothstep(16.0, 26.0, slope) * (1.0 - smoothstep(36.0, 44.0, slope));
                if ex.g3x.scrub && hash(ix, iz, 113) < scrub_share(x, z, &s, cover, naip, ex.g3x) * scrub_terrain(&s, ex.fl) * MOUNDS_PER_CELL * flank {
                    plant_mound(x, z, ix, iz, &s, bloom, ex.g3x, ex.fl, day_of_year, out);
                }
                {
                    let cured = meadow_cured(x, z, &s, season, ex.g3x.breakup);
                    let sc = 0.8 + 0.4 * hash(ix, iz, 112) as f32;
                    out.push(Plant { kind: Kind::Meadow(MEADOW_BARE), pos: [x, s.y - GRASS_SINK_M, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc; 3], rank: hash(ix, iz, 8) as f32, tint: meadow_patch_tint(MEADOW_BARE, cured, 0.9 + 0.2 * hash(ix, iz, 20) as f32), grad: s.grad(), tip: TIP_NONE });
                }
                return;
            }
            if ex.g3x.scrub && hash(ix, iz, 113) < scrub_share(x, z, &s, cover, naip, ex.g3x) * scrub_terrain(&s, ex.fl) * MOUNDS_PER_CELL {
                plant_mound(x, z, ix, iz, &s, bloom, ex.g3x, ex.fl, day_of_year, out);
                return;
            }
            // the second pass: a continuous meadow (a patch in every cell), the
            // spring flowers over it where the first pass drew them.
            if r < flowers {
                let tint = meadow_tint(x, z, ix, iz, &s, cover, season, bloom).0.map(|c| (c * GRASS_GAIN).min(4.0));
                let sc = 0.75 + 0.55 * hash(ix, iz, 4) as f32;
                out.push(Plant { kind: Kind::Flower(if field(x, z, 25.0, 47) < 0.6 { 0 } else { 1 }), pos: [x, s.y - GRASS_SINK_M, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc; 3], rank: hash(ix, iz, 8) as f32, tint, grad: s.grad(), tip: TIP_NONE });
            }
            if ex.fl.bluff && s.lithology == Lithology::Serpentinite && z > marin_z() {
                // the flora pass: the serpentine bluff's low scrub in place of the meadow.
                let cured = meadow_cured(x, z, &s, season, ex.g3x.breakup);
                let kind = crate::flora::bluff_kind(x, z, ix, iz);
                let sc = 0.85 + 0.3 * hash(ix, iz, 4) as f32;
                out.push(Plant { kind: Kind::Meadow(kind), pos: [x, s.y - GRASS_SINK_M, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc; 3], rank: hash(ix, iz, 8) as f32, tint: meadow_patch_tint(kind, cured, 0.93 + 0.14 * hash(ix, iz, 20) as f32), grad: s.grad(), tip: TIP_NONE });
                return;
            }
            let n0 = out.len();
            plant_meadow(x, z, ix, iz, &s, cover, naip, season, ex.g2, ex.g3x.breakup, out);
            if ex.fl.ramp {
                // September and October's bleached grey-tan on the patch.
                if let Some(Plant { kind: Kind::Meadow(k), tint, .. }) = out.get_mut(n0) {
                    *tint = crate::flora::bleached_tint(*tint, MEADOW_LIVE[*k as usize], crate::flora::grass_bleach(day_of_year));
                }
                // the annual grasses' seed stalks over the open grassland (not the scrub's
                // cells: those returned above), in stands, cured with the season.
                let share = crate::flora::seed_stalks(day_of_year) * smoothstep(0.3, 0.65, warped(x, z, 6.0, 144)) * (1.0 - smoothstep(30.0, 45.0, slope));
                if hash(ix, iz, 145) < share {
                    let cured = meadow_cured(x, z, &s, season, ex.g3x.breakup);
                    let k = crate::flora::MEADOW_SEED_STALKS;
                    let sc = 0.8 + 0.45 * hash(ix, iz, 146) as f32;
                    let (jx, jz) = (hash(ix, iz, 147) - 0.5, hash(ix, iz, 148) - 0.5);
                    let tint = crate::flora::bleached_tint(meadow_patch_tint(k, cured + 0.1, 1.0), MEADOW_LIVE[k as usize], crate::flora::grass_bleach(day_of_year));
                    out.push(Plant { kind: Kind::Meadow(k), pos: [x + jx * 0.6, s.y - GRASS_SINK_M, z + jz * 0.6], yaw: hash(ix, iz, 149) * std::f64::consts::TAU, scale: [sc; 3], rank: hash(ix, iz, 150) as f32, tint, grad: s.grad(), tip: TIP_NONE });
                }
            }
            return;
        }
        let kind = if r < flowers {
            Kind::Flower(if field(x, z, 25.0, 47) < 0.6 { 0 } else { 1 })
        } else if r < flowers + g * steep {
            Kind::Grass
        } else if r < flowers + (g + t) * steep {
            Kind::Tuft
        } else {
            return;
        };
        let sc = 0.75 + 0.55 * hash(ix, iz, 4) as f32;
        let tall = 0.8 + 0.45 * hash(ix, iz, 9) as f32;
        // the blade takes the ground's tone (see GRASS_GAIN), each bunch a little different.
        // value +-28 %, and a drier (redder) or greener cast per bunch (INFERRED spread): past
        // 25 m a bunch is the ground's tone (props_draw.wgsl) and this its only mark.
        let w = 0.72 + 0.56 * hash(ix, iz, 20) as f32;
        let (tint, tip) = if g3.grass_color_variety {
            let m = meadow_tint(x, z, ix, iz, &s, cover, season, bloom);
            (m.0.map(|c| c * GRASS_GAIN), m.1)
        } else {
            let dry = (hash(ix, iz, 24) as f32 - 0.5) * 0.24;
            ([s.ground[0] * GRASS_GAIN * w * (1.0 + dry), s.ground[1] * GRASS_GAIN * w, s.ground[2] * GRASS_GAIN * w * (1.0 - dry)], TIP_NONE)
        };
        let tint = match kind {
            Kind::Flower(_) => tint.map(|c| c.min(4.0)),
            _ => tint,
        };
        // sunk 7 cm: the atlas cell's root rows (a third of the tips' value) go under the
        // ground instead of drawing a dark crescent round the bunch's foot.
        out.push(Plant { kind, pos: [x, s.y - GRASS_SINK_M, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc, sc * tall, sc], rank: hash(ix, iz, 8) as f32, tint, grad: s.grad(), tip });
    });
}

/// The meadow patch types (`Kind::Meadow`), in the render models' order
/// (eo-render props_lib2/vegetation_models.rs `V_MEADOW`). Each patch is a
/// matted mat of dead thatch and short sward about 1.6 m across (the cells are
/// 1 m apart, so the mats overlap into a continuous cover) with its type's
/// plants over it. The headlands' grassland is coastal prairie invaded by
/// European annuals (wild oats, Avena barbata and A. fatua), with native
/// perennial bunchgrasses (red fescue, Festuca rubra; Pacific reedgrass) on
/// the steeper and fog-side slopes, clovers and forbs in the moist swales,
/// coyote brush and sagebrush scrub, and ice plant (Carpobrotus edulis)
/// sheets on the disturbed bluff edges (NPS Golden Gate, "Coastal Prairie",
/// "Coastal Scrub" and "Invasive Plants: Iceplant" pages; Ford and Hayes 2007,
/// "Northern coastal scrub and coastal prairie", in Terrestrial Vegetation of
/// California, 3rd ed., UC Press). Shares and placement weights are INFERRED.
pub const MEADOW_SWARD: u8 = 0;
/// Wild oats: tall (0.6-1.0 m) culms with drooping spikelets, over sward.
pub const MEADOW_OATS: u8 = 1;
/// Fine fescue and bunchgrass tussocks: blue-grey-green fountains of fine blades.
pub const MEADOW_FESCUE: u8 = 2;
/// Clover and broad-leaved forbs, low and dark green.
pub const MEADOW_CLOVER: u8 = 3;
/// Ice plant: a mat of fleshy three-sided leaves, green with red tips.
pub const MEADOW_ICE_PLANT: u8 = 4;
/// Dead thatch: matted pale straw with a little green sward through it.
pub const MEADOW_THATCH: u8 = 5;
/// The third pass's coastal scrub mounds (`Grass3::scrub`): rounded, lumpy
/// shrubs of small leaves in clumps, 1 m tall and 1.6 m across at scale 1
/// (planted 0.5-1.5 m tall). Northern coastal scrub on the headlands is coyote
/// brush (Baccharis pilularis) with California sagebrush (Artemisia
/// californica) and bush lupine (Lupinus arboreus) (NPS Golden Gate "Coastal
/// Scrub"; Ford and Hayes 2007). Coyote brush: bright green, small toothed leaves.
pub const MEADOW_COYOTE: u8 = 6;
/// California sagebrush: grey-green, fine and feathery.
pub const MEADOW_SAGEBRUSH: u8 = 7;
/// Bush lupine in flower: grey-green palmate leaves under clustered spikes
/// of yellow flowers (the yellow form of Lupinus arboreus, the common one on
/// the Marin headlands' bluffs: Calscape, Jepson eFlora; spring bloom).
pub const MEADOW_LUPINE: u8 = 8;
/// Bare eroded soil (`Grass3::bare`): a ragged mat of red-brown soil (the
/// Tamalpais series' eroded horizon) with a few sparse tufts of short grass.
pub const MEADOW_BARE: u8 = 9;
/// The flora pass's scrub species follow (`crate::flora::MEADOW_POISON_OAK` .. `MEADOW_TOYON`).
pub const MEADOW_TYPES: u8 = 17;

/// Each type's living tissue in its green state (linear albedo, the mean of its
/// blades' vertex colours, eo-render) and cured (or, for ice plant, drought-
/// reddened), and how far it follows the season's curing (0 never, 1 fully).
/// Green leaf visible reflectance 0.04-0.10 with the green peak and red over
/// blue, dry grass 0.15-0.30 red over green (USGS Spectral Library v7, Kokaly
/// et al. 2017, doi:10.5066/F7RR1WDJ; Woolley 1971, Plant Physiol. 47,
/// 656-662). The fescue's glaucous wax is bluer (the blue-green of red fescue
/// leaves); clover leaves are darker (horizontal, thick mesophyll); ice plant
/// reddens under drought (betalain pigments). Values INFERRED inside those ranges.
pub const MEADOW_LIVE: [[f32; 3]; MEADOW_TYPES as usize] = [
    [0.060, 0.095, 0.032],
    [0.072, 0.104, 0.036],
    [0.066, 0.092, 0.074],
    [0.032, 0.070, 0.020],
    [0.062, 0.112, 0.030],
    [0.060, 0.095, 0.032],
    // the scrub's leaves: coyote brush a glossy bright green, sagebrush grey-green,
    // lupine leaves grey-green (green leaf 0.04-0.12 in the green, USGS v7; the
    // photographs' sunlit coyote brush is the brightest green on the headland:
    // INFERRED inside the range), the bare patch's sparse grass the sward's.
    [0.066, 0.150, 0.026],
    [0.088, 0.112, 0.066],
    [0.070, 0.124, 0.046],
    [0.060, 0.095, 0.032],
    // the flora pass's species (`crate::flora::SPECIES`, their leaf colours linear).
    [0.127, 0.205, 0.038],
    [0.053, 0.091, 0.021],
    [0.040, 0.065, 0.021],
    [0.032, 0.061, 0.021],
    // seed stalks (the wild oats'), coast buckwheat's pale woolly leaves (140-170,
    // 150-175, 135-160) estimate, low end), the bluff herbs (the seaside daisy's
    // (90-120, 120-140, 90-110) estimate).
    [0.072, 0.104, 0.036],
    [0.230, 0.262, 0.205],
    [0.110, 0.200, 0.110],
];
pub const MEADOW_CURED: [[f32; 3]; MEADOW_TYPES as usize] = [
    [0.215, 0.178, 0.100],
    [0.280, 0.228, 0.112],
    [0.150, 0.135, 0.100],
    [0.075, 0.072, 0.036],
    [0.150, 0.075, 0.040],
    [0.215, 0.178, 0.100],
    [0.066, 0.150, 0.026],
    [0.088, 0.112, 0.066],
    [0.070, 0.124, 0.046],
    [0.215, 0.178, 0.100],
    [0.127, 0.205, 0.038],
    [0.053, 0.091, 0.021],
    [0.040, 0.065, 0.021],
    [0.032, 0.061, 0.021],
    [0.280, 0.228, 0.112],
    [0.250, 0.230, 0.180],
    [0.150, 0.170, 0.100],
];
/// The scrub is evergreen (its summer dullness is the dust and the older leaves: not drawn).
pub const MEADOW_CURE: [f32; MEADOW_TYPES as usize] = [1.0, 1.0, 0.55, 0.35, 0.4, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.2, 0.3];
/// Dead thatch, weathered pale straw (linear albedo; dry grass litter 0.15-0.25
/// in the visible, USGS v7 "dry grass" and litter spectra; INFERRED): the mat's
/// and the lying blades' colour (eo-render bakes it in the dead material).
pub const MEADOW_THATCH_ALBEDO: [f32; 3] = [0.165, 0.155, 0.112];

/// Whether a cell carries the continuous meadow (`Grass2::cover`): grass and
/// scrub cover, and the land cover's bare class inside the NAIP image. The
/// 10 m land cover's bare class ("bare / sparse vegetation") takes in the
/// headland tops, which photographs show under grass; and the NAIP image is a
/// summer one, whose cured grass is as bright as bare soil, so its bare share
/// cannot tell the two apart: there the patch leans to thatch
/// ([`meadow_kind`]), so a bright summer cell reads as dead grass, not dirt.
/// Mapped paths, trails and paving never reach here (the site is None); outside
/// the image the bare class stays bare.
pub fn meadow_ground(cover: Cover, naip: Option<Naip>) -> bool {
    match cover {
        Cover::WildGrass | Cover::Scrub | Cover::TallScrub => true,
        Cover::Bare => naip.is_some(),
        _ => false,
    }
}

/// The share of a patch's sward that has cured (0 green .. 1 cured): the
/// season's dryness leaned by the same patch field, aspect, slope and hollows
/// as [`meadow_tint`], without its per-bunch term (a patch's mat is smooth
/// with its neighbours).
///
/// `breakup` (`Grass3::breakup`): the patch field domain-warped ([`warped`]).
pub fn meadow_cured(x: f64, z: f64, s: &Site, season: f64, breakup: bool) -> f64 {
    let d0 = season.clamp(0.0, 1.0);
    let f = |c: f64, k: u32| if breakup { warped(x, z, c, k) } else { field(x, z, c, k) };
    let p = 0.65 * (f(32.0, 61) - 0.5) + 0.35 * (f(11.0, 62) - 0.5);
    let patch = 2.0 * smoothstep(-0.22, 0.22, p) - 1.0;
    let lean = MEADOW_LEAN * patch - 0.1 * s.north() + 0.06 * smoothstep(10.0, 35.0, s.slope_deg()) - 0.12 * (s.hollow_m * 0.5).clamp(-1.0, 1.0);
    let cured = smoothstep(0.42, 0.7, d0 + lean);
    (0.35 * d0 + 0.65 * cured).clamp(0.0, 1.0)
}

/// The patch type at a cell. Each type has a weight from the site (slope,
/// aspect, hollow, distance to the shore edge, scrub nearby, bare ground in
/// the NAIP box, the cure) times its own patch field, cubed so one type holds
/// an area and the edges between them mix (INFERRED rules from the habitat
/// notes at [`MEADOW_SWARD`]):
/// * oats: gentle, sun-facing (south and west) open ground, away from scrub;
/// * fescue tussocks: steeper slopes, the fog-side north faces and the scrub's edges;
/// * clover and forbs: moist hollows and north faces on gentle ground;
/// * ice plant: within about 45 m of the shore on the bluff top and its face;
/// * thatch: dry, trampled and eroded ground (the NAIP's bare share).
///
/// `breakup` (`Grass3::breakup`): the type fields domain-warped ([`warped`])
/// into lobed patches and the weights squared, not cubed, so neighbouring
/// types mix over a wider edge instead of meeting in blobs (the camouflage
/// pattern the 40-100 m slopes drew).
#[allow(clippy::too_many_arguments)]
pub fn meadow_kind(x: f64, z: f64, ix: i64, iz: i64, s: &Site, cover: Cover, naip: Option<Naip>, cured: f64, breakup: bool) -> u8 {
    let field = |x: f64, z: f64, c: f64, k: u32| if breakup { warped(x, z, c, k) } else { field(x, z, c, k) };
    let slope = s.slope_deg();
    let north = s.north();
    let wet = (s.hollow_m * 0.5).clamp(-1.0, 1.0);
    let edge = 1.0 - smoothstep(12.0, 45.0, s.shore_m);
    let in_scrub: f64 = if matches!(cover, Cover::Scrub | Cover::TallScrub) { 1.0 } else { 0.0 };
    let scrub = in_scrub.max(naip.map(|n| smoothstep(0.08, 0.4, n.canopy as f64)).unwrap_or(0.0));
    let bare = naip.map(|n| smoothstep(0.3, 0.8, n.bare as f64)).unwrap_or(0.0);
    let gentle = 1.0 - smoothstep(25.0, 42.0, slope);
    let w = [
        0.6 * gentle.max(0.35),
        1.5 * (0.45 + 0.6 * (-north).max(0.0) + 0.3 * cured) * gentle * (1.0 - 0.7 * scrub) * smoothstep(0.35, 0.7, field(x, z, 14.0, 91)),
        (0.3 + 0.9 * smoothstep(18.0, 38.0, slope) + 0.6 * scrub + 0.4 * north.max(0.0)) * smoothstep(0.3, 0.65, field(x, z, 8.0, 92)),
        1.3 * (0.25 + 0.6 * north.max(0.0) + 0.8 * wet.max(0.0)) * gentle * smoothstep(0.45, 0.75, field(x, z, 6.0, 93)),
        2.2 * edge * (0.35 + 0.65 * smoothstep(12.0, 32.0, slope)) * smoothstep(0.3, 0.55, field(x, z, 9.0, 94)),
        (0.2 + 0.5 * cured + 0.4 * bare) * smoothstep(0.4, 0.75, field(x, z, 11.0, 95)),
    ]
    .map(|v: f64| if breakup { v.max(0.0).powi(2) } else { v.max(0.0).powi(3) });
    let sum: f64 = w.iter().sum();
    if sum <= 0.0 {
        return MEADOW_SWARD;
    }
    let mut r = hash(ix, iz, 96) * sum;
    for (k, v) in w.iter().enumerate() {
        if r < *v {
            return k as u8;
        }
        r -= v;
    }
    MEADOW_SWARD
}

/// A meadow patch's tint: the multiplier that takes its type's green-state
/// blades ([`MEADOW_LIVE`], the vertex colours) to its cure (wild oats cure
/// earliest, the annuals' seed set; INFERRED), a little value spread per patch.
pub fn meadow_patch_tint(kind: u8, cured: f64, w: f32) -> [f32; 3] {
    let k = kind.min(MEADOW_TYPES - 1) as usize;
    let lead = if kind == MEADOW_OATS { 0.15 } else { 0.0 };
    let d = ((cured + lead).clamp(0.0, 1.0) as f32) * MEADOW_CURE[k];
    let c = mix3(MEADOW_LIVE[k], MEADOW_CURED[k], d);
    [0, 1, 2].map(|i| (c[i] / MEADOW_LIVE[k][i] * w).min(3.9))
}

/// One meadow patch at a fine cell (`Grass2::cover`), its type
/// ([`meadow_kind`]; all sward with `--no-meadow-types`) and, in the scrub and
/// at its edges (`Grass2::types`), now and then a low coyote brush or sage
/// mound between the coarse layer's shrubs.
#[allow(clippy::too_many_arguments)]
pub fn plant_meadow(x: f64, z: f64, ix: i64, iz: i64, s: &Site, cover: Cover, naip: Option<Naip>, season: f64, g2: Grass2, breakup: bool, out: &mut Vec<Plant>) {
    let cured = meadow_cured(x, z, s, season, breakup);
    let kind = if g2.types { meadow_kind(x, z, ix, iz, s, cover, naip, cured, breakup) } else { MEADOW_SWARD };
    let sc = 0.88 + 0.24 * hash(ix, iz, 4) as f32;
    // the sward's height in clumps (a 3 m field and each patch), so the canopy is lumpy, not mown.
    let tall = (0.6 + 0.7 * field(x, z, 3.0, 106) + 0.3 * hash(ix, iz, 9)) as f32;
    let w = 0.93 + 0.14 * hash(ix, iz, 20) as f32;
    out.push(Plant { kind: Kind::Meadow(kind), pos: [x, s.y - GRASS_SINK_M, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc, sc * tall, sc], rank: hash(ix, iz, 8) as f32, tint: meadow_patch_tint(kind, cured, w), grad: s.grad(), tip: TIP_NONE });
    if !g2.types {
        return;
    }
    let in_scrub = matches!(cover, Cover::Scrub | Cover::TallScrub);
    let canopy = naip.map(|n| n.canopy as f64).unwrap_or(if in_scrub { 0.5 } else { 0.0 });
    let chance = 0.16 * smoothstep(0.05, 0.3, canopy) * (1.0 - smoothstep(0.6, 0.9, canopy)) * (1.0 - smoothstep(35.0, 50.0, s.slope_deg()));
    if hash(ix, iz, 97) < chance {
        let sage = hash(ix, iz, 98) < 0.25 + 0.2 * field(x, z, 18.0, 23);
        let k = if sage { 1 } else { 0 };
        let v = 0.82 + 0.3 * hash(ix, iz, 99) as f32;
        let o = (1.4 * (field(x, z, 25.0, 68) - 0.5)) as f32;
        let b = SHRUB_LEAF_TINT[k];
        let tint = [b[0] * (1.0 + 0.1 * o) * v, b[1] * v, b[2] * (1.0 - 0.12 * o) * v];
        let h = 0.3 + 0.2 * hash(ix, iz, 100) as f32;
        let wd = 0.45 + 0.3 * hash(ix, iz, 101) as f32;
        let (jx, jz) = (hash(ix, iz, 102) - 0.5, hash(ix, iz, 103) - 0.5);
        out.push(Plant { kind: Kind::Shrub(k as u8), pos: [x + jx * 0.6, s.y - 0.08, z + jz * 0.6], yaw: hash(ix, iz, 104) * std::f64::consts::TAU, scale: [wd, h, wd], rank: hash(ix, iz, 105) as f32, tint, grad: s.grad(), tip: TIP_NONE });
    }
}

/// The flora pass's ground-shape weight on the scrub share
/// ([`crate::flora::terrain_weight`]; 1 without `Flora::species`).
fn scrub_terrain(s: &Site, fl: crate::flora::Flora) -> f64 {
    let w = if fl.species { crate::flora::terrain_weight(s) } else { 1.0 };
    // the serpentine bluff's coyote brush is sparse (`Flora::bluff`; guide 6: "sparse coyote brush").
    if fl.bluff && s.lithology == Lithology::Serpentinite { w * 0.35 } else { w }
}

/// Mounds a 1 m cell carries where the scrub is closed: a mound is 1.3-2.2 m
/// across (2-3 m2), so half the cells give a closed canopy with dark gaps.
pub const MOUNDS_PER_CELL: f64 = 0.5;

/// The game z north of which the ground is the Marin Headlands' (37.8135 N, the
/// strait's mid-line under the bridge; eo-render terrain_detail `MARIN_LAT`).
fn marin_z() -> f64 {
    static Z: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *Z.get_or_init(|| game_xz(37.8135, -122.4783).1)
}

/// A field warped by two others (`Grass3::breakup`): its patches have
/// irregular, lobed outlines instead of the value noise's rounded cells.
pub fn warped(x: f64, z: f64, cell: f64, salt: u32) -> f64 {
    let wx = (field(x, z, cell * 0.7, salt + 500) - 0.5) * cell * 1.1;
    let wz = (field(x, z, cell * 0.7, salt + 501) - 0.5) * cell * 1.1;
    0.75 * field(x + wx, z + wz, cell, salt) + 0.25 * field(x - wz, z + wx, cell * 0.37, salt + 502)
}

/// How much of a cell is bare eroded soil (0-1; the chance its cell stays
/// bare): within the NAIP image (none outside it), a steep face the image
/// shows bare (past 36 degrees non-green ground is eroded, not grass: the
/// classifier's `NAIP_BARE_SLOPE_DEG`; the photographs' knoll faces) or
/// red-brown soil in the image ([`Naip::soil`]), or a knoll top or ridge
/// whose image is warm bare ground ([`Naip::warm`]: the eroded knoll tops),
/// less where the image shows any green.
pub fn bare_soil(x: f64, z: f64, s: &Site, naip: Option<Naip>) -> f64 {
    let Some(n) = naip else { return 0.0 };
    let slope = s.slope_deg();
    let face = smoothstep(36.0, 46.0, slope) * smoothstep(0.45, 0.85, n.bare as f64);
    let soil = smoothstep(0.12, 0.45, n.soil as f64);
    // a knoll top or ridge (the ground 2-4 m over the mean of a ring 12 m out) whose
    // image is warm bare ground: wind and foot traffic strip the thin soil there.
    let ridge = smoothstep(1.5, 3.5, -s.hollow_m) * smoothstep(0.4, 0.8, n.warm as f64);
    let green = smoothstep(0.1, 0.4, (n.canopy + n.lush) as f64);
    // a ragged edge: the soil breaks into the sward along a warped field.
    let edge = 0.75 + 0.5 * warped(x, z, 5.0, 114);
    (face.max(soil).max(ridge) * (1.0 - green) * edge).clamp(0.0, 0.97)
}

/// How much of a cell the coastal scrub holds (0-1): the NAIP image's canopy
/// and bright green (summer) share, the mapped scrub where the image is not
/// read, and on the Marin headland's seaward (south-facing) slopes, where the
/// scrub runs down to the bluffs (the photographs from Battery Spencer), a
/// share of its own; broken by a warped patch field into stands with grass
/// between (NPS: "closed patches with open grass between"). Steep faces past
/// 50 degrees carry none.
pub fn scrub_share(x: f64, z: f64, s: &Site, cover: Cover, naip: Option<Naip>, g: Grass3) -> f64 {
    let slope = s.slope_deg();
    let mapped = match cover {
        Cover::TallScrub => 0.75,
        Cover::Scrub => 0.6,
        _ => 0.0,
    };
    // where the image's canopy is closed the coarse layer's shrubs already close it
    // (`plant_coarse_with`): the mounds fill its edges, gaps and the bright scrub it misses.
    let image = naip.map(|n| smoothstep(0.1, 0.5, (n.canopy + n.lush) as f64) * (1.0 - 0.85 * smoothstep(0.55, 0.9, n.canopy as f64))).unwrap_or(mapped);
    let marin = smoothstep(0.0, 60.0, marin_z() - z);
    let south = smoothstep(0.05, 0.3, -s.north());
    let seaward = 0.8 * marin * south * smoothstep(5.0, 14.0, slope);
    let p = if g.breakup { warped(x, z, 9.0, 115) } else { field(x, z, 9.0, 115) };
    let stands = smoothstep(0.3, 0.55, p);
    (image.max(seaward).max(mapped * 0.8) * (0.3 + 0.7 * stands) * (1.0 - smoothstep(44.0, 52.0, slope))).clamp(0.0, 1.0)
}

/// One coastal scrub mound at a fine cell (`Grass3::scrub`): coyote brush,
/// sagebrush in its own patches, bush lupine in stands (in flower in the bloom
/// window, `Grass3::bloom`; coyote brush out of it), 0.5-1.5 m tall.
#[allow(clippy::too_many_arguments)]
pub fn plant_mound(x: f64, z: f64, ix: i64, iz: i64, s: &Site, bloom: f64, g: Grass3, fl: crate::flora::Flora, day_of_year: f64, out: &mut Vec<Plant>) {
    let lupine = g.bloom && bloom > 0.3 && warped(x, z, 11.0, 116) > 0.62;
    if fl.species {
        // the flora pass: the species by the ground, each its own form and height.
        let (kind, h) = crate::flora::pick(x, z, ix, iz, s, lupine);
        let form = crate::flora::species(kind).expect("a scrub species").form;
        let sy = h / form.h;
        let sx = sy * (0.85 + 0.3 * hash(ix, iz, 120) as f32);
        let (jx, jz) = (hash(ix, iz, 121) - 0.5, hash(ix, iz, 122) - 0.5);
        let v = (0.88 + 0.22 * hash(ix, iz, 123) + 0.16 * (field(x, z, 25.0, 124) - 0.5)) as f32;
        let season = if kind == crate::flora::MEADOW_POISON_OAK { crate::flora::poison_oak_season(day_of_year) } else { [1.0; 3] };
        out.push(Plant { kind: Kind::Meadow(kind), pos: [x + jx * 0.7, s.y - 0.12, z + jz * 0.7], yaw: hash(ix, iz, 125) * std::f64::consts::TAU, scale: [sx, sy, sx * (0.85 + 0.3 * hash(ix, iz, 126) as f32)], rank: hash(ix, iz, 127) as f32, tint: season.map(|c| c * v), grad: s.grad(), tip: TIP_NONE });
        return;
    }
    let sage = !lupine && warped(x, z, 16.0, 117) < 0.2;
    let kind = if lupine {
        MEADOW_LUPINE
    } else if sage {
        MEADOW_SAGEBRUSH
    } else {
        MEADOW_COYOTE
    };
    // taller where the scrub is older (a 20 m field), each mound its own size.
    let h = (0.5 + 0.55 * field(x, z, 20.0, 118) + 0.45 * hash(ix, iz, 119)) as f32;
    let w = (0.8 + 0.5 * hash(ix, iz, 120)) as f32 * (0.85 + 0.25 * h);
    let (jx, jz) = (hash(ix, iz, 121) - 0.5, hash(ix, iz, 122) - 0.5);
    // each mound its own value, and a 25 m field of lighter and darker stands.
    let v = (0.82 + 0.26 * hash(ix, iz, 123) + 0.2 * (field(x, z, 25.0, 124) - 0.5)) as f32;
    out.push(Plant { kind: Kind::Meadow(kind), pos: [x + jx * 0.7, s.y - 0.12, z + jz * 0.7], yaw: hash(ix, iz, 125) * std::f64::consts::TAU, scale: [w, h, w * (0.85 + 0.3 * hash(ix, iz, 126) as f32)], rank: hash(ix, iz, 127) as f32, tint: [v; 3], grad: s.grad(), tip: TIP_NONE });
}

/// The lamp word's alpha for a plant with no tip colour (what every instance carried before).
pub const TIP_NONE: u8 = 255;

/// The meadow's end members (linear albedo). Green and cured annual grass are
/// the far layer's (`eo_render::farshore::FAR_GRASS_GREEN` / `FAR_GRASS_GOLDEN`,
/// repeated here: this crate does not depend on eo-render), so the near sward
/// and the far land agree. Grey-green bunchgrass and sage-edge grass, and the
/// brown of thatch and trampled soil, sit in the green-vegetation and dry
/// grass / soil ranges of the USGS Spectral Library v7 (Kokaly et al. 2017,
/// doi:10.5066/F7RR1WDJ: green grass visible reflectance about 0.04-0.10, dry
/// grass 0.15-0.25 red over green, soils 0.10-0.20 with red over blue);
/// the exact values are INFERRED inside those ranges.
pub const MEADOW_GREEN: [f32; 3] = [0.065, 0.100, 0.034];
pub const MEADOW_GOLDEN: [f32; 3] = [0.215, 0.165, 0.092];
pub const MEADOW_SAGE: [f32; 3] = [0.080, 0.100, 0.050];
pub const MEADOW_BROWN: [f32; 3] = [0.115, 0.085, 0.056];

/// How far a bunch's dryness leans off the season's (0-1 green to golden):
/// the spread measured on the NAIP summer image of the headlands
/// (EO_SURVEY_ROOT/A1, 0.6 m, all 20 tiles at 1/8 resolution, open low
/// vegetation only: shadow, water, bright bare ground and dark canopy left
/// out): red over green p10 0.86, median 1.00, p90 1.09, and 28 % green
/// (r/g under 0.95), 60 % olive, 12 % tan. The end members' r/g run 0.65 to
/// 1.30, so the image's p10-p90 spread is about +-0.18 of dryness; the
/// patch field carries most of it, slope, aspect and hollows the rest.
pub const MEADOW_LEAN: f64 = 0.3;

/// Linear luminance (Rec. 709).
fn lum(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

/// A grass bunch's colour in the meadow (`--no-grass-color-variety` off: the
/// ground's tone with a per-bunch value and cast). `season` is the season
/// model's dryness on the day (`eo_look::season::dryness`, 0 green .. 1
/// golden): read as the share of the sward that has cured, not as one olive
/// over all of it, so a patch is green or cured and the season moves the
/// line between them (spring mostly green with the first cured patches,
/// late summer golden with green left in the hollows). Where the line falls
/// is leaned by a patch field of 11 m and 32 m octaves (whole areas greener or
/// drier), the slope's aspect (south faces cure first, north faces and
/// the fog side stay green: INFERRED), steepness (thinner soil) and hollows
/// (wetter); then grey-green bunchgrass patches (more under the scrub),
/// browner thatch patches where it is dry, and each bunch its own value and
/// cast. The value follows the ground's tone (its mottle). Returns the linear
/// albedo (before `GRASS_GAIN`) and its tip word ([`meadow_tip`]).
#[allow(clippy::too_many_arguments)]
pub fn meadow_tint(x: f64, z: f64, ix: i64, iz: i64, s: &Site, cover: Cover, season: f64, bloom: f64) -> ([f32; 3], u8) {
    let g = s.ground;
    let d0 = season.clamp(0.0, 1.0);
    // patches: two octaves, sharpened so an area reads as one lean, not as noise.
    let p = 0.65 * (field(x, z, 32.0, 61) - 0.5) + 0.35 * (field(x, z, 11.0, 62) - 0.5);
    let patch = 2.0 * smoothstep(-0.22, 0.22, p) - 1.0;
    let slope = s.slope_deg();
    let lean = MEADOW_LEAN * patch - 0.1 * s.north() + 0.06 * smoothstep(10.0, 35.0, slope) - 0.12 * (s.hollow_m * 0.5).clamp(-1.0, 1.0) + 0.12 * (hash(ix, iz, 24) - 0.5);
    // cured or green by patch (a soft line), part of the season's mean kept: in the
    // green season only the driest patches on steep south faces begin to turn.
    let cured = smoothstep(0.42, 0.7, d0 + lean);
    let d = (0.35 * d0 + 0.65 * cured).clamp(0.0, 1.0);
    let at = |d: f64| mix3(MEADOW_GREEN, MEADOW_GOLDEN, d as f32);
    let mut c = at(d);
    // grey-green bunchgrass (fescue, needlegrass) in patches, and at the scrub's edges.
    let scrubby = matches!(cover, Cover::Scrub | Cover::TallScrub | Cover::DuneScrub);
    let sage = smoothstep(0.62, 0.85, field(x, z, 15.0, 64)) * 0.4 + if scrubby { 0.15 } else { 0.0 };
    c = mix3(c, MEADOW_SAGE, (sage * (1.0 - 0.5 * d)) as f32);
    // brown thatch and trampled soil, a patch here and there, more where it is dry.
    let brown = smoothstep(0.72, 0.88, field(x, z, 20.0, 63)) * (0.25 + 0.5 * d);
    c = mix3(c, MEADOW_BROWN, brown as f32);
    // relative to the ground: the ground's own tone where the bunch sits at the season's
    // mean, shifted by the end members' ratios where it leans; the ground's value (its
    // mottle) kept, the golden grass a little lighter than the green.
    let m = at(d0);
    let f = [0, 1, 2].map(|k| g[k] * c[k] / m[k].max(1e-4));
    let v = lum(g) * (lum(c) / lum(m).max(1e-4)).powf(0.3) / lum(f).max(1e-5);
    let w = 0.8 + 0.4 * hash(ix, iz, 20) as f32;
    (f.map(|k| k * v * w), meadow_tip(ix, iz, d, bloom))
}

/// A grass bunch's tip word (the lamp alpha, props_draw.wgsl): kind in the top
/// two bits (0 straw seed heads, 1 yellow, 2 purple, 3 white flowers), amount
/// 0-62 in the low six (63 with kind 3 is [`TIP_NONE`]). Seed heads and
/// awns are lighter than the stems, more so once the grass has cured; one
/// bunch in about 60 carries flower tints (more in the bloom window): yellow
/// (gumplant, mustards, poppies' gold), purple (lupine, asters), white
/// (yarrow, cow parsnip), the headlands' common colours (NPS Golden Gate
/// "Wildflowers", species list; shares INFERRED).
pub fn meadow_tip(ix: i64, iz: i64, dryness: f64, bloom: f64) -> u8 {
    let r = hash(ix, iz, 65);
    let share = 0.012 + 0.03 * bloom;
    if r < share {
        let k = 1 + (hash(ix, iz, 66) * 3.0) as u8 % 3;
        return (k << 6) | 40;
    }
    let a = (0.2 + 0.5 * dryness) * (0.7 + 0.6 * hash(ix, iz, 67));
    (a.clamp(0.0, 1.0) * 62.0).round() as u8
}

/// A jittered lattice of pitch `p` over [x0, x1) x [z0, z1): `f(x, z, ix, iz)`
/// once per cell whose jittered point falls inside (world-anchored, so
/// adjacent rectangles tile without seams or repeats).
pub(crate) fn lattice(x0: f64, z0: f64, x1: f64, z1: f64, p: f64, salt: u32, f: &mut dyn FnMut(f64, f64, i64, i64)) {
    for iz in (z0 / p).floor() as i64 - 1..=(z1 / p).ceil() as i64 {
        for ix in (x0 / p).floor() as i64 - 1..=(x1 / p).ceil() as i64 {
            let (cx, cz) = (ix.wrapping_mul(7919) ^ salt as i64, iz);
            let x = (ix as f64 + 0.1 + hash(cx, cz, 1) * 0.8) * p;
            let z = (iz as f64 + 0.1 + hash(cx, cz, 2) * 0.8) * p;
            if x < x0 || x >= x1 || z < z0 || z >= z1 {
                continue;
            }
            f(x, z, cx, cz);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(cover: Cover) -> impl Fn(f64, f64) -> Option<Site> {
        move |_x, _z| Some(Site { y: 10.0, normal: [0.0, 1.0, 0.0], ground: [0.1, 0.12, 0.06], cover: Some(cover), lithology: Lithology::Chert, shore_m: 500.0, hollow_m: 0.0 })
    }

    #[test]
    fn meadow_follows_the_season_and_varies_in_patches() {
        let site = |ground: [f32; 3]| Site { y: 10.0, normal: [0.0, 1.0, 0.0], ground, cover: Some(Cover::WildGrass), lithology: Lithology::Chert, shore_m: 500.0, hollow_m: 0.0 };
        let mean_rg = |season: f64| {
            let s = site(mix3(MEADOW_GREEN, MEADOW_GOLDEN, season as f32));
            let (mut sum, mut lo, mut hi) = (0.0, f32::MAX, f32::MIN);
            for k in 0..400 {
                let (x, z) = ((k % 20) as f64 * 7.0, (k / 20) as f64 * 7.0);
                let (c, tip) = meadow_tint(x, z, k % 20, k / 20, &s, Cover::WildGrass, season, 0.0);
                assert_ne!(tip, TIP_NONE);
                let rg = c[0] / c[1];
                (sum, lo, hi) = (sum + rg, lo.min(rg), hi.max(rg));
            }
            (sum / 400.0, lo, hi)
        };
        let (spring, _, _) = mean_rg(0.0);
        let (early, s_lo, s_hi) = mean_rg(0.4);
        let (summer, _, _) = mean_rg(1.0);
        assert!(spring < early && early < summer, "spring {spring} early summer {early} summer {summer}");
        assert!(spring < summer, "spring {spring} summer {summer}");
        assert!(s_hi - s_lo > 0.2, "a meadow has drier and greener patches: {s_lo}..{s_hi}");
        // a south face cures sooner than a north face.
        let face = |nz: f64| {
            let mut s = site(mix3(MEADOW_GREEN, MEADOW_GOLDEN, 0.5));
            s.normal = [0.0, 0.85, nz];
            (0..100).map(|k| { let c = meadow_tint(k as f64 * 9.0, 0.0, k, 0, &s, Cover::WildGrass, 0.5, 0.0).0; c[0] / c[1] }).sum::<f32>()
        };
        assert!(face(0.5) > face(-0.5));
    }

    #[test]
    fn switches_follow_the_command_line() {
        let a = |v: &[&str]| Switches::from_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&["eo-app"]), Switches::ALL);
        assert_eq!(a(&["eo-app", "--no-gg2-vegetation"]), Switches::NONE);
        assert!(!a(&["--no-ground-cover"]).ground_cover);
        assert!(a(&["--no-ground-cover"]).hill_rock);
        assert!(!a(&["--no-hill-rock"]).hill_rock);
        assert!(!a(&["--no-coastal-trees"]).coastal_trees);
    }

    #[test]
    fn nothing_is_planted_with_every_switch_off() {
        let mut out = vec![];
        plant_coarse(0.0, 0.0, 128.0, 128.0, &flat(Cover::Wood), Switches::NONE, &mut out);
        plant_fine(0.0, 0.0, 32.0, 32.0, &flat(Cover::WildGrass), 100.0, Switches::NONE, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn layers_tile_without_seams_and_repeat_exactly() {
        let s = flat(Cover::WildGrass);
        let mut whole = vec![];
        plant_fine(0.0, 0.0, 64.0, 32.0, &s, 100.0, Switches::ALL, &mut whole);
        let mut halves = vec![];
        plant_fine(0.0, 0.0, 32.0, 32.0, &s, 100.0, Switches::ALL, &mut halves);
        plant_fine(32.0, 0.0, 64.0, 32.0, &s, 100.0, Switches::ALL, &mut halves);
        assert_eq!(whole.len(), halves.len());
        let key = |p: &Plant| ((p.pos[0] * 1000.0) as i64, (p.pos[2] * 1000.0) as i64);
        let mut a: Vec<_> = whole.iter().map(key).collect();
        let mut b: Vec<_> = halves.iter().map(key).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
        // grassland: most lattice cells carry a bunch or a tuft.
        let cells = (64.0 * 32.0) / (GRASS_PITCH_M * GRASS_PITCH_M);
        assert!(whole.len() as f64 > cells * 0.5, "{} of {cells}", whole.len());
    }

    #[test]
    fn flowers_only_in_the_bloom_window() {
        let s = flat(Cover::WildGrass);
        let count = |day: f64| {
            let mut out = vec![];
            plant_fine(0.0, 0.0, 200.0, 200.0, &s, day, Switches::ALL, &mut out);
            out.iter().filter(|p| matches!(p.kind, Kind::Flower(_))).count()
        };
        assert!(count(110.0) > 50);
        assert_eq!(count(265.0), 0);
    }

    #[test]
    fn steep_slopes_carry_rock_and_flat_lawns_none() {
        let steep = |_x: f64, _z: f64| {
            let a = 50f64.to_radians();
            Some(Site { y: 0.0, normal: [a.sin(), a.cos(), 0.0], ground: [0.1; 3], cover: Some(Cover::WildGrass), lithology: Lithology::Serpentinite, shore_m: 50.0, hollow_m: 0.0 })
        };
        let mut out = vec![];
        plant_coarse(0.0, 0.0, 128.0, 128.0, &steep, Switches::ALL, &mut out);
        let rocks: Vec<_> = out.iter().filter(|p| matches!(p.kind, Kind::Rock(_))).collect();
        assert!(rocks.len() > 30, "{}", rocks.len());
        // serpentinite takes the two unbedded shapes and its greyish green.
        assert!(rocks.iter().all(|p| matches!(p.kind, Kind::Rock(2) | Kind::Rock(3))));
        assert!(rocks.iter().all(|p| p.tint[1] > p.tint[0]));
        let mut lawn = vec![];
        plant_coarse(0.0, 0.0, 128.0, 128.0, &flat(Cover::Lawn), Switches::ALL, &mut lawn);
        assert!(lawn.is_empty());
    }

    #[test]
    fn woods_by_the_shore_are_mostly_cypress() {
        let mut n = [0usize; 3];
        for i in 0..2000 {
            let (sp, s, h) = coastal_tree(i as f64 * 3.1, 7.0, 20.0, hash(i, 0, 1));
            n[sp as usize] += 1;
            assert!((0.78..=1.18).contains(&s) && h < 1.0);
        }
        assert!(n[0] > 1500, "{n:?}");
        let mut m = [0usize; 3];
        for i in 0..2000 {
            m[coastal_tree(0.0, 0.0, 2000.0, hash(i, 0, 1)).0 as usize] += 1;
        }
        // inland: blue gum the largest share (about 0.45).
        assert!(m[2] > m[0] && m[2] > m[1] && (800..1000).contains(&m[2]), "{m:?}");
    }

    #[test]
    fn plants_stand_at_real_size() {
        // the trees' scale is natural spread about the published size (0.78-1.18), not the
        // terrain's 1.25 height scale (see the module notes).
        let mut sum = 0.0;
        for i in 0..4000 {
            let (_, s, h) = coastal_tree(i as f64 * 2.3, i as f64 * 0.7, 2000.0, hash(i, 3, 1));
            assert!((0.78..=1.18).contains(&s) && h == 1.0);
            sum += s as f64;
        }
        assert!((sum / 4000.0 - 0.98).abs() < 0.02);
    }

    #[test]
    fn shrub_leaves_are_green_not_blue() {
        // the measured albedo times the tint: green over red over blue, the sage greyer
        // (its red and blue nearer its green) than the coyote brush.
        let lit: Vec<[f32; 3]> = (0..2).map(|k| [0, 1, 2].map(|c| SHRUB_LEAF_ALBEDO[k][c] * SHRUB_LEAF_TINT[k][c])).collect();
        for c in &lit {
            assert!(c[1] > c[0] && c[0] > c[2], "{c:?}");
        }
        assert!(lit[1][0] / lit[1][1] > lit[0][0] / lit[0][1] && lit[1][2] / lit[1][1] > lit[0][2] / lit[0][1]);
        // blue held under a third of green before the sky's blue fill (it lit lavender at
        // the target's 0.59).
        assert!(lit[1][2] / lit[1][1] < 0.34, "{:?}", lit[1]);
    }

    #[test]
    fn outcrops_fill_whole_patches_with_proud_blocks() {
        let cliff = |_x: f64, _z: f64| {
            let a = 45f64.to_radians();
            Some(Site { y: 20.0, normal: [a.sin(), a.cos(), 0.0], ground: [0.3; 3], cover: Some(Cover::Cliff), lithology: Lithology::Serpentinite, shore_m: 60.0, hollow_m: 0.0 })
        };
        let mut out = vec![];
        plant_coarse(0.0, 0.0, 256.0, 256.0, &cliff, Switches::ALL, &mut out);
        let rocks: Vec<_> = out.iter().filter(|p| matches!(p.kind, Kind::Rock(_))).collect();
        // inside the patches nearly every 5 m cell carries a block of 3.2 m or more (they overlap).
        let (mut inside, mut hit) = (0, 0);
        for iz in 0..51 {
            for ix in 0..51 {
                let (x, z) = (ix as f64 * 5.0 + 2.5, iz as f64 * 5.0 + 2.5);
                if outcrop_patch(x, z, Cover::Cliff) > 0.99 {
                    inside += 1;
                    if rocks.iter().any(|p| (p.pos[0] - x).abs() < 5.0 && (p.pos[2] - z).abs() < 5.0) {
                        hit += 1;
                    }
                }
            }
        }
        assert!(inside > 300 && hit as f64 > 0.95 * inside as f64, "{hit} of {inside}");
        for p in &rocks {
            // a third of the slope sheared, the block taller than half its width.
            let g = (p.grad[0] * p.grad[0] + p.grad[1] * p.grad[1]).sqrt();
            assert!((g - ROCK_SHEAR).abs() < 0.01, "{g}");
            assert!(p.scale[1] > 0.5 * p.scale[2] && p.scale[2] >= 4.5);
            // sunk at least so the downhill edge meets the ground.
            let (_, sink) = rock_seat([1.0, 0.0], p.scale[2], p.scale[1] / p.scale[2]);
            assert!(((20.0 - p.pos[1]) - sink).abs() < 1e-6 && sink > 0.0);
        }
        // scrub grows in the gaps, not on the blocks' patches.
        let shrubs: Vec<_> = out.iter().filter(|p| matches!(p.kind, Kind::Shrub(_))).collect();
        assert!(!shrubs.is_empty() && shrubs.iter().all(|p| outcrop_patch(p.pos[0], p.pos[2], Cover::Cliff) < 1.0));
    }

    #[test]
    fn round_three_switches_follow_the_command_line() {
        let a = |v: &[&str]| Gg3::from_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&["eo-app"]), Gg3::ALL);
        assert_eq!(a(&["eo-app", "--no-gg3-vegetation"]), Gg3::NONE);
        assert_eq!(a(&["eo-app", "--no-gg3"]), Gg3::NONE);
        assert!(!a(&["--no-naip-planting"]).naip_planting && a(&["--no-naip-planting"]).cliff_rock_cover);
        assert!(!a(&["--no-cliff-rock-cover"]).cliff_rock_cover);
        assert!(!a(&["--no-far-species"]).far_species && !a(&["--no-far-tree-light"]).far_tree_light && !a(&["--no-bloom-date"]).bloom_date);
        for s in GG3_SWITCHES {
            assert!(s.starts_with("--no-"));
        }
    }

    #[test]
    fn naip_pixels_classify_by_index_brightness_and_slope() {
        // the hand-labelled cells' median colours (A1, 0.6 m; the classifier notes),
        // with their own brightness as the 5 x 5 mean.
        let c = |p: [u8; 3], slope: f64| naip_class(p, (p[0] as f32 + p[1] as f32 + p[2] as f32) / 765.0, slope);
        // Kirby Cove's grey-green scrub (the slope round-3's first rule called grass),
        // the scrub west of it, the cypress and pine wood, the scrub by the battery.
        assert_eq!(c([87, 93, 87], 25.0), NaipClass::Canopy);
        assert_eq!(c([68, 83, 74], 20.0), NaipClass::Canopy);
        assert_eq!(c([59, 76, 65], 20.0), NaipClass::Canopy);
        assert_eq!(c([84, 91, 83], 15.0), NaipClass::Canopy);
        // cured grass on Kirby's slope and by the road, green grass on the ridge.
        assert_eq!(c([118, 116, 105], 25.0), NaipClass::Low);
        assert_eq!(c([147, 134, 122], 15.0), NaipClass::Low);
        assert_eq!(c([110, 115, 98], 20.0), NaipClass::Low);
        // the battery's concrete, Lincoln Park's road, a chert face (by its slope).
        assert_eq!(c([169, 157, 143], 15.0), NaipClass::Bare);
        assert_eq!(c([172, 173, 176], 5.0), NaipClass::Bare);
        assert_eq!(c([140, 124, 112], 40.0), NaipClass::Bare);
        assert_eq!(c([140, 124, 112], 10.0), NaipClass::Low);
        // a known miss: sunlit scrub on the Baker bluff reads as grass.
        assert_eq!(c([100, 105, 89], 15.0), NaipClass::Low);
        // the sea in shadow.
        assert_eq!(c([40, 50, 70], 0.0), NaipClass::Unknown);
        // shares count only the box inside the 2-pixel margin.
        let (w, h) = (7, 5);
        let mut px = vec![Some([68u8, 83, 74]); w * h];
        px[0] = Some([169, 157, 143]); // in the margin: not counted
        px[w - 1] = Some([169, 157, 143]);
        px[2 * w + 4] = None;
        let n = naip_shares(&px, w, h, 5.0).unwrap();
        assert!((n.canopy - 1.0).abs() < 1e-6, "{n:?}");
        assert!(naip_shares(&vec![Some([40, 50, 70]); 25], 5, 5, 0.0).is_none());
        assert!(naip_shares(&vec![None; 9], 3, 3, 0.0).is_none());
    }

    #[test]
    fn rock_cover_carries_no_plant_and_naip_places_the_scrub() {
        let all = Switches::ALL;
        let none = |_: f64, _: f64, _: f64| None;
        // a cliff over a beach and mapped bare rock: rock only (hill_rock), no plant.
        for cover in [Cover::Cliff, Cover::Rock] {
            let mut out = vec![];
            let ex = Extra { g3: Gg3::ALL, g2: Grass2::NONE, g3x: Grass3::NONE, fl: crate::flora::Flora::NONE, naip: Box::new(none) };
            plant_coarse_with(0.0, 0.0, 128.0, 128.0, &flat(cover), all, &ex, &mut out);
            plant_fine_with(0.0, 0.0, 32.0, 32.0, &flat(cover), 100.0, all, &ex, &mut out);
            assert!(out.iter().all(|p| matches!(p.kind, Kind::Rock(_))), "{cover:?}");
            // round 2 (the switch off) planted there.
            let mut old = vec![];
            plant_fine(0.0, 0.0, 32.0, 32.0, &flat(cover), 100.0, all, &mut old);
            assert!(!old.is_empty());
        }
        // the image's canopy: closed scrub; its open grass: none.
        let count = |n: Naip| {
            let ex = Extra { g3: Gg3::ALL, g2: Grass2::NONE, g3x: Grass3::NONE, fl: crate::flora::Flora::NONE, naip: Box::new(move |_, _, _| Some(n)) };
            let mut out = vec![];
            plant_coarse_with(0.0, 0.0, 128.0, 128.0, &flat(Cover::TallScrub), all, &ex, &mut out);
            out.iter().filter(|p| matches!(p.kind, Kind::Shrub(_))).count()
        };
        let cells = (128.0 * 128.0 / (SHRUB_PITCH_M * SHRUB_PITCH_M)) as usize;
        assert!(count(Naip { canopy: 1.0, low: 0.0, bare: 0.0, ..Default::default() }) > cells * 9 / 10);
        assert!(count(Naip { canopy: 0.0, low: 1.0, bare: 0.0, ..Default::default() }) < cells / 20);
        // and the sward on the open grass.
        let ex = Extra { g3: Gg3::ALL, g2: Grass2::NONE, g3x: Grass3::NONE, fl: crate::flora::Flora::NONE, naip: Box::new(|_, _, _| Some(Naip { canopy: 0.0, low: 1.0, bare: 0.0, ..Default::default() })) };
        let mut out = vec![];
        plant_fine_with(0.0, 0.0, 32.0, 32.0, &flat(Cover::WildGrass), 265.0, all, &ex, &mut out);
        assert!(out.iter().filter(|p| p.kind == Kind::Grass).count() > 32 * 32 * 8 / 10);
    }

    #[test]
    fn the_cited_bloom_window_is_march_to_may() {
        assert_eq!(bloom(103.0, Gg3::ALL), 1.0); // 13 Apr, the headland photograph
        assert_eq!(bloom(40.0, Gg3::ALL), 0.0);
        assert_eq!(bloom(160.0, Gg3::ALL), 0.0); // 9 Jun: past May
        assert!(bloom(160.0, Gg3::NONE) > 0.0); // round 2's window ran to mid-June
        assert_eq!(bloom(265.0, Gg3::ALL), 0.0);
    }

    #[test]
    fn far_forms_are_the_near_tiers_trees() {
        for i in 0..500 {
            let (sp, h, c, rgb) = far_form(i as f64 * 7.3, 11.0, 2000.0, hash(i, 2, 3));
            let (h0, c0) = FAR_TREE_SIZE[sp as usize];
            assert!(h >= h0 * 0.78 - 1e-3 && h <= h0 * 1.18 + 1e-3 && c <= c0 * 1.18 + 1e-3);
            // green over red over (or near) blue: a leaf, not a grey card.
            assert!(rgb[1] > rgb[0] && rgb[1] > rgb[2], "{rgb:?}");
        }
    }

    #[test]
    fn grass_takes_the_ground_tone() {
        let mut out = vec![];
        plant_fine(0.0, 0.0, 32.0, 32.0, &flat(Cover::WildGrass), 265.0, Switches::ALL, &mut out);
        let mean: f32 = out.iter().map(|p| p.tint[1]).sum::<f32>() / out.len() as f32;
        assert!((mean / (0.12 * GRASS_GAIN) - 1.0).abs() < 0.05, "{mean}");
    }

    #[test]
    fn grass3_rolls_up_and_reads_each_switch() {
        let a = |v: &[&str]| Grass3::from_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&[]), Grass3::ALL);
        assert_eq!(a(&["--no-grass3"]), Grass3::NONE);
        assert_eq!(a(&["--no-bare-ground"]), Grass3 { bare: false, ..Grass3::ALL });
        assert_eq!(a(&["--no-scrub-mounds", "--no-meadow-breakup"]), Grass3 { scrub: false, breakup: false, ..Grass3::ALL });
    }

    #[test]
    fn eroded_ground_is_bare_and_scrub_keeps_off_cliffs() {
        let site = |ny: f64, hollow: f64| Site { y: 0.0, normal: [0.0, ny, (1.0 - ny * ny).sqrt()], ground: [0.1; 3], cover: Some(Cover::Bare), lithology: Lithology::Chert, shore_m: 400.0, hollow_m: hollow };
        let n = |bare: f32, warm: f32, lush: f32| Some(Naip { canopy: 0.0, low: 1.0 - bare, bare, lush, soil: 0.0, warm });
        let steep = site(50f64.to_radians().cos(), 0.0);
        let gentle = site(10f64.to_radians().cos(), 0.0);
        let knoll = site(10f64.to_radians().cos(), -4.0);
        // a steep face the image shows bare, and a warm bare knoll top, are mostly bare soil;
        let mean = |s: &Site, nn: Option<Naip>| (0..400).map(|k| bare_soil(k as f64 * 3.1, k as f64 * 1.7, s, nn)).sum::<f64>() / 400.0;
        assert!(mean(&steep, n(1.0, 0.0, 0.0)) > 0.6);
        assert!(mean(&knoll, n(0.0, 1.0, 0.0)) > 0.6);
        // gentle cured grass, green scrub and ground outside the image are not.
        assert_eq!(mean(&gentle, n(0.0, 1.0, 0.0)), 0.0);
        assert!(mean(&knoll, n(0.0, 1.0, 0.6)) < 0.05);
        assert_eq!(mean(&steep, None), 0.0);
        // no mound on a face past 52 degrees, however scrubby.
        let cliff = site(55f64.to_radians().cos(), 0.0);
        assert_eq!(scrub_share(0.0, 0.0, &cliff, Cover::TallScrub, n(0.0, 0.0, 1.0), Grass3::ALL), 0.0);
    }

}
