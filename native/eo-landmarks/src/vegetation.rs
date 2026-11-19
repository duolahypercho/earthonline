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

/// The switch names (eo-app accepts them).
pub const SWITCHES: &[&str] = &["--no-gg2-vegetation", "--no-coastal-trees", "--no-ground-cover", "--no-hill-rock"];

/// The bloom window, days of the year (see the module notes).
pub const BLOOM_DAYS: (f64, f64) = (60.0, 170.0);
/// Cypress dominate the stands this near the shore (m, INFERRED).
pub const BLUFF_CYPRESS_M: f64 = 160.0;
/// The coarse layer's tree lattice (m): the park-tree lattice's own pitch (tile-trees.js).
pub const TREE_PITCH_M: f64 = 14.0;
/// The shrub lattice (m): a coyote brush mound is 2.6 m across at scale 1 (coastal.rs), so a
/// 3.4 m pitch at the scrub's full chance gives a closed canopy with gaps (INFERRED).
pub const SHRUB_PITCH_M: f64 = 3.4;
/// The rock lattice (m).
pub const ROCK_PITCH_M: f64 = 5.0;
/// The fine layer's lattice (m): one grass patch (1.1 m across) a cell, so the
/// patches overlap into a sward.
pub const GRASS_PITCH_M: f64 = 1.0;
/// Grass blade tint over the ground tone: 1 / (the atlas grass cell's mean
/// linear albedo 0.309 x the card's mean linear vertex shade, about 0.45 with
/// its dark base), so a bunch's mean albedo is the ground tone's (the ground's
/// tone is what the grass is) and its lit tips a little over it.
pub const GRASS_GAIN: f32 = 7.2;
/// Leaf tints of the coastal trees and shrubs (`Kind::Tree`, `Kind::Shrub`
/// order): what brings each species' mean leaf albedo (its cooked leaf
/// vertex colour times the atlas cell's mean linear colour, measured with
/// `eo-props --example coastal_preview`) to a green leaf's visible reflectance,
/// about 0.03-0.05 blue, 0.06-0.10 green, 0.03-0.05 red (the general
/// green-vegetation curve; the conifers at its dark end, the blue gum's
/// glaucous leaf and the sage's grey hairs bluer and lighter: INFERRED).
/// Targets (linear r, g, b): cypress 0.035/0.060/0.030, pine 0.040/0.065/0.030,
/// blue gum 0.050/0.075/0.060, coyote brush 0.040/0.075/0.030, sage 0.070/0.085/0.050.
pub const TREE_LEAF_TINT: [[f32; 3]; 3] = [[1.19, 1.08, 3.31], [1.13, 1.04, 2.88], [1.58, 1.00, 2.96]];
pub const SHRUB_LEAF_TINT: [[f32; 3]; 2] = [[1.60, 0.98, 3.26], [1.56, 1.15, 2.78]];

/// Rock colours, sRGB (see the module notes).
pub const CHERT_SRGB: [u8; 3] = [98, 64, 57];
pub const SERPENTINITE_SRGB: [u8; 3] = [88, 104, 95];
pub const SANDSTONE_SRGB: [u8; 3] = [128, 122, 101];
/// The rock mesh's mean vertex shade (vegetation_models.rs `rock`: 0.78 + up light,
/// cavities darker; about 0.8): the tint divides it out so the rock's mean is its colour.
pub const ROCK_SHADE: f32 = 0.8;

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

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn srgb_to_linear(c: u8) -> f32 {
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

/// The coarse layer over [x0, x1) x [z0, z1): coastal trees on unmapped tree
/// cover, shrubs, rock outcrops.
pub fn plant_coarse(x0: f64, z0: f64, x1: f64, z1: f64, site: &dyn Fn(f64, f64) -> Option<Site>, sw: Switches, out: &mut Vec<Plant>) {
    // trees (the park-tree lattice's pitch, its jitter, a grove field).
    if sw.coastal_trees {
        lattice(x0, z0, x1, z1, TREE_PITCH_M, 1, &mut |x, z, ix, iz| {
            let Some(s) = site(x, z) else { return };
            if s.cover != Some(Cover::Wood) || s.slope_deg() > 42.0 {
                return;
            }
            let grove = 0.55 + 0.45 * field(x, z, 70.0, 11);
            if hash(ix, iz, 3) >= 0.8 * grove {
                return;
            }
            let (sp, sc, hf) = coastal_tree(x, z, s.shore_m, hash(ix, iz, 4));
            let v = 0.85 + 0.3 * hash(ix, iz, 6) as f32;
            out.push(Plant { kind: Kind::Tree(sp), pos: [x, s.y - 0.08, z], yaw: hash(ix, iz, 5) * std::f64::consts::TAU, scale: [sc, sc * hf, sc], rank: hash(ix, iz, 8) as f32, tint: TREE_LEAF_TINT[sp as usize].map(|c| c * v), grad: [0.0; 2] });
        });
    }
    if sw.ground_cover {
        lattice(x0, z0, x1, z1, SHRUB_PITCH_M, 2, &mut |x, z, ix, iz| {
            let Some(s) = site(x, z) else { return };
            let Some(cover) = s.cover else { return };
            let slope = s.slope_deg();
            // a patch field (about 30 m) so the scrub reads as a mosaic, darker and denser in hollows and on north faces.
            let patch = field(x, z, 30.0, 21);
            let shelter = (s.north() * 0.25 + (s.hollow_m * 0.35).clamp(-0.3, 0.4)) as f64;
            let chance = match cover {
                Cover::TallScrub => 0.62 + 0.3 * patch + shelter,
                Cover::Scrub => 0.5 + 0.35 * patch + shelter,
                Cover::DuneScrub => 0.2 + 0.35 * patch,
                Cover::WildGrass => (smoothstep(0.38, 0.72, patch) * 0.8 + shelter * 0.8).max(0.0),
                Cover::Wood | Cover::WoodGreen => 0.12 + 0.12 * patch,
                Cover::Bare | Cover::Rock => 0.05,
                Cover::Lawn => 0.0,
            } * (1.0 - smoothstep(38.0, 55.0, slope));
            if hash(ix, iz, 3) >= chance {
                return;
            }
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
            let base = SHRUB_LEAF_TINT[if kind == Kind::Shrub(1) { 1 } else { 0 }];
            out.push(Plant { kind, pos: [x, s.y - sink, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc * wide, sc, sc * wide], rank: hash(ix, iz, 8) as f32, tint: base.map(|c| c * v), grad: s.grad() });
        });
    }
    if sw.hill_rock {
        lattice(x0, z0, x1, z1, ROCK_PITCH_M, 3, &mut |x, z, ix, iz| {
            let Some(s) = site(x, z) else { return };
            let Some(cover) = s.cover else { return };
            let slope = s.slope_deg();
            // outcrops on steep slopes (cliffs), on mapped bare rock; a few stones on bare earth;
            // grouped by a field so they read as outcrops, not a scatter.
            let group = field(x, z, 22.0, 31);
            let steep = smoothstep(30.0, 50.0, slope);
            let chance = match cover {
                Cover::Rock => 0.4 + 0.45 * group,
                Cover::Bare => 0.06 + 0.5 * steep,
                Cover::Lawn => 0.0,
                _ => steep * steep * smoothstep(0.35, 0.75, group) * 0.9,
            };
            if hash(ix, iz, 3) >= chance {
                return;
            }
            // mapped bare rock is outcrop: blocks of 1.6-6 m that overlap into rock faces; elsewhere
            // single blocks and stones (sizes INFERRED).
            let size = if cover == Cover::Rock { 1.6 + 4.4 * hash(ix, iz, 4).powf(1.3) } else { 0.8 + 2.6 * hash(ix, iz, 4).powf(1.6) * (0.6 + 0.8 * steep) } as f32;
            let shape = (hash(ix, iz, 5) * 4.0) as u8 % 4;
            let shape = match s.lithology {
                Lithology::Chert => shape % 2,
                Lithology::Serpentinite => 2 + shape % 2,
                Lithology::Sandstone => shape,
            };
            let base = match s.lithology {
                Lithology::Chert => CHERT_SRGB,
                Lithology::Serpentinite => SERPENTINITE_SRGB,
                Lithology::Sandstone => SANDSTONE_SRGB,
            };
            let v = 0.85 + 0.3 * hash(ix, iz, 6) as f32;
            let tint = base.map(|c| srgb_to_linear(c) / ROCK_SHADE * v);
            // proud of a steep face (a slab lying flush with the slope reads as a stain).
            let flat = 0.7 + 0.6 * hash(ix, iz, 9) as f32 + 0.6 * steep as f32;
            // the block lies on the slope (the shear); a tenth of it further into the ground.
            let sink = 0.1 * size as f64;
            out.push(Plant {
                kind: Kind::Rock(shape),
                pos: [x, s.y - sink, z],
                yaw: hash(ix, iz, 7) * std::f64::consts::TAU,
                scale: [size * (0.9 + 0.4 * hash(ix, iz, 10) as f32), size * flat, size],
                rank: hash(ix, iz, 8) as f32,
                tint,
                grad: s.grad(),
            });
        });
    }
}

/// The fine layer over [x0, x1) x [z0, z1): grass bunches, short tufts and,
/// on `day_of_year` inside the bloom window, wildflowers.
pub fn plant_fine(x0: f64, z0: f64, x1: f64, z1: f64, site: &dyn Fn(f64, f64) -> Option<Site>, day_of_year: f64, sw: Switches, out: &mut Vec<Plant>) {
    if !sw.ground_cover {
        return;
    }
    let bloom = {
        let (a, b) = BLOOM_DAYS;
        // ramps of three weeks at each end of the window.
        smoothstep(a, a + 21.0, day_of_year) * (1.0 - smoothstep(b - 21.0, b, day_of_year))
    };
    lattice(x0, z0, x1, z1, GRASS_PITCH_M, 4, &mut |x, z, ix, iz| {
        let Some(s) = site(x, z) else { return };
        let Some(cover) = s.cover else { return };
        let slope = s.slope_deg();
        let patch = field(x, z, 9.0, 41);
        // (grass, tuft, flower) chances by cover.
        let (g, t, f) = match cover {
            Cover::WildGrass => (0.6 + 0.3 * patch, 0.1, 0.06),
            Cover::Scrub | Cover::TallScrub => (0.45 + 0.3 * patch, 0.1, 0.03),
            Cover::DuneScrub => (0.08, 0.35 + 0.2 * patch, 0.02),
            // a mown lawn carries nothing a bunch or tuft would draw (its blades are under 5 cm).
            Cover::Lawn => return,
            Cover::Wood | Cover::WoodGreen => (0.06, 0.18 * patch, 0.0),
            Cover::Bare => (0.04, 0.1 * patch, 0.01),
            Cover::Rock => (0.1, 0.08, 0.02),
        };
        let steep = 1.0 - 0.6 * smoothstep(40.0, 60.0, slope);
        let r = hash(ix, iz, 3);
        let flowers = f * bloom * smoothstep(0.35, 0.8, field(x, z, 14.0, 43));
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
        // value +-20 %, and a drier (redder) or greener cast per bunch (INFERRED spread).
        let w = 0.8 + 0.4 * hash(ix, iz, 20) as f32;
        let dry = (hash(ix, iz, 24) as f32 - 0.5) * 0.24;
        let tint = [s.ground[0] * GRASS_GAIN * w * (1.0 + dry), s.ground[1] * GRASS_GAIN * w, s.ground[2] * GRASS_GAIN * w * (1.0 - dry)];
        let tint = match kind {
            Kind::Flower(_) => tint.map(|c| c.min(4.0)),
            _ => tint,
        };
        out.push(Plant { kind, pos: [x, s.y - 0.03, z], yaw: hash(ix, iz, 7) * std::f64::consts::TAU, scale: [sc, sc * tall, sc], rank: hash(ix, iz, 8) as f32, tint, grad: s.grad() });
    });
}

/// A jittered lattice of pitch `p` over [x0, x1) x [z0, z1): `f(x, z, ix, iz)`
/// once per cell whose jittered point falls inside (world-anchored, so
/// adjacent rectangles tile without seams or repeats).
fn lattice(x0: f64, z0: f64, x1: f64, z1: f64, p: f64, salt: u32, f: &mut dyn FnMut(f64, f64, i64, i64)) {
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
    fn grass_takes_the_ground_tone() {
        let mut out = vec![];
        plant_fine(0.0, 0.0, 32.0, 32.0, &flat(Cover::WildGrass), 265.0, Switches::ALL, &mut out);
        let mean: f32 = out.iter().map(|p| p.tint[1]).sum::<f32>() / out.len() as f32;
        assert!((mean / (0.12 * GRASS_GAIN) - 1.0).abs() < 0.05, "{mean}");
    }
}
