//! (`--no-lidar-trees`, Docs/HILLS-PLAN.md H6c) The hill parks' trees as the
//! USGS 3DEP lidar measured them: each tree's position, height and crown
//! radius from the hill builder's `cook trees` (eo-hills `trees`,
//! `<cache>/hills/trees-v1.bin`). Inside the parks the lidar covers (the
//! areas' green rings), they replace the greens' 14 m park-tree lattice in
//! both tiers: the near tier (props.rs `cook`) and the distant cards
//! (main.rs `hlod_trees`) read the same trees through [`placed_in`], so
//! nothing pops between them. With the switch, or without the file, the
//! lattice as before.
//!
//! A tree stands where the lattice's own tests let a park tree stand
//! (`blocked`: a footprint, the drawn street, water; the ground cover's
//! `tree_ground_ok`: not on a mapped beach or a face too steep), on the drawn
//! ground (in the near tier on the drawn relief, [`placed_in`]). Its position
//! follows the ground plan's scale (`eo_data::groundscale`); its height and
//! crown stay real size, as every model does.
//!
//! Its species ([`species_of`]): each park its own mix ([`area_mix`], what
//! the city's natural areas plan and the parks' own accounts say grows
//! there), each species' likelihood scaled by how well the measured tree fits
//! it (its height within the species' range, its height over crown radius
//! near the model's) and by a grove field per species (a place's trees come
//! in stands of one kind, not a salt-and-pepper mix), the coast's exposure
//! (the wind-shorn cypress toward the ocean, the broadleaves away from it);
//! drawn by the tree's own hash, so each tree has one species in both tiers.
//! A tree on the exposed coast leans downwind ([`lean_of`]: the onshore
//! north-westerly, the far cards' `FT_DOWNWIND`), every tree a little its own
//! way, and each tree has its own hue (`eo_landmarks::vegetation::tree_hue`)
//! in both tiers; a distant card keeps its species' own shade
//! ([`far_tint_own`]). The model is scaled to the
//! measured height, and across to the measured crown within [`ANISO_MAX`]
//! of the height's scale.
//!
//! With [`VARIETY_SWITCH`] the trees are drawn as before the parks' own mixes
//! ([`legacy`]): three coastal species and one street broadleaf picked by
//! height and crown alone, upright, the species' card colours.

use eo_hills::trees::LidarTrees;
use std::sync::OnceLock;

/// The switch.
pub const SWITCH: &str = "--no-lidar-trees";
/// The parks' own species mixes, the lean, the hue and the cards' own shade
/// off: the species by height and crown alone ([`legacy`]).
pub const VARIETY_SWITCH: &str = "--no-tree-variety";

/// Whether the parks' own species mixes are drawn ([`VARIETY_SWITCH`] not given).
pub fn variety_on() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !std::env::args().any(|a| a == VARIETY_SWITCH))
}

/// The coastal species' drawn height at scale 1 (m; `Kind::Tree` order):
/// eo-props coastal.rs, `FAR_TREE_SIZE`.
pub fn model_height_m(sp: u8) -> f64 {
    eo_landmarks::vegetation::FAR_TREE_SIZE[sp as usize].0 as f64
}

/// The species' visible crown radius at scale 1 (m; `Kind::Tree` order):
/// the radius of a circle of the area of the convex hull of the near build's
/// leaves over 30 % of its height (the lidar crown's own floor, eo-hills cook
/// trees `CROWN_BASE_FRACTION`). Measured from the builds by the test
/// `model_crowns_are_the_builds`.
pub const MODEL_CROWN_R_M: [f64; MODELS] = [8.37, 6.30, 6.05, 5.76, 6.06, 7.28, 4.38, 6.44, 4.39, 3.79, 2.99, 5.72, 5.70, 7.83, 6.33, 6.36, 3.88];
/// The tree species (their second builds are drawn for half the trees: `tree_model`).
pub const SPECIES: usize = eo_landmarks::vegetation::TREE_BASE_SPECIES;
/// The tree models (`eo_landmarks::vegetation::Kind::Tree`): the species and the second builds.
pub const MODELS: usize = eo_landmarks::vegetation::TREE_SPECIES;

/// The heights each species is drawn at (m), the published range of a
/// grown tree in these parks (INFERRED within: conifers.org, Monterey cypress
/// to 25 m, Monterey pine 15-30 m; Cal-IPC, blue gum 30-55 m, blackwood
/// acacia to 15 m; Calflora, coast live oak 10-25 m, arroyo willow to 10 m):
/// cypress, pine, blue gum, stand gum, wind-shorn cypress, old pine,
/// windbreak cypress, live oak, multi-stem oak, acacia, willow.
pub const HEIGHT_RANGE_M: [(f64, f64); SPECIES] = [(6.0, 25.0), (8.0, 32.0), (15.0, 60.0), (20.0, 60.0), (3.0, 15.0), (16.0, 34.0), (8.0, 26.0), (4.0, 22.0), (3.0, 14.0), (4.0, 16.0), (2.0, 10.0)];
/// How sharply a tree's height over crown radius picks a species (the
/// spread of its log ratio about the model's). INFERRED.
pub const SHAPE_SPREAD: f64 = 0.35;
/// A height this far outside a species' range halves its likelihood about
/// every metre (m). INFERRED.
pub const HEIGHT_SOFT_M: f64 = 2.5;
/// The grove field: a cell this size (real m) per species, and how far it
/// moves each species' likelihood (INFERRED: groves of a few dozen metres).
pub const GROVE_CELL_M: f64 = 45.0;
pub const GROVE_SWING: f64 = 1.4;
/// How far the crown's scale may part from the height's (either way).
pub const ANISO_MAX: f64 = 1.4;
/// How far a tree stands into the ground (m): the park lattice's root sink.
pub const ROOT_SINK_M: f64 = 0.08;

/// The file, read once: None with [`SWITCH`] or without it.
pub fn installed() -> Option<&'static LidarTrees> {
    static T: OnceLock<Option<LidarTrees>> = OnceLock::new();
    T.get_or_init(|| {
        if std::env::args().any(|a| a == SWITCH) {
            return None;
        }
        let p = eo_hills::trees::resolve_path();
        if !p.is_file() {
            eprintln!("lidar trees: no file at {} (eo-hills cook trees); the park-tree lattice everywhere", p.display());
            return None;
        }
        match LidarTrees::open(&p) {
            Ok(t) => {
                let names: Vec<&str> = t.header().areas.iter().map(|a| a.name.as_str()).collect();
                eprintln!("lidar trees: {} trees from the USGS 3DEP lidar in {} ({})", t.header().trees, names.join(", "), p.display());
                Some(t)
            }
            Err(e) => {
                eprintln!("lidar trees: {} not used ({e:#})", p.display());
                None
            }
        }
    })
    .as_ref()
}

/// Whether the lidar speaks for the trees at game (x, z): inside an area's
/// rings, where the greens' lattice plants no park tree.
pub fn covers(x: f64, z: f64) -> bool {
    installed().is_some_and(|t| {
        let gs = eo_data::groundscale::scale();
        t.covers(x / gs, z / gs)
    })
}

/// The hill parks' species mix (`Kind::Tree` order: cypress, pine, blue gum,
/// stand gum, wind-shorn cypress, old pine, windbreak cypress, live oak,
/// multi-stem oak, acacia, willow), by the lidar area's name (eo-hills
/// trees `Area::name`): what grows in each park's wooded parts; relative
/// weights (the shape, the grove field and the exposure then move each
/// tree's draw).
pub fn area_mix(area: &str) -> [f64; SPECIES] {
    // Sources: SF Recreation and Park Department, Significant Natural Resource Areas
    // Management Plan (SNRAMP, 2006), each park's section (forest acres by type);
    // Golden Gate Park Master Plan, Park Landscape (tree census by species); NPS Lands
    // End and Sutro Heights histories. "Mixed exotic forest" (SNRAMP's largest type at
    // McLaren, Twin Peaks, Corona Heights and Bernal) is split among pine, cypress, gum
    // and acacia (INFERRED). Docs/HILLS-PLAN.md H6c quotes the figures.
    match area {
        // "almost entirely invasive blue gum forest (29.96 acres)" of 30.06; large
        // cypress at the edges; a dense, even-aged Sutro plantation of the 1880s.
        "mount-davidson" => [0.02, 0.0, 0.25, 0.71, 0.0, 0.0, 0.0, 0.0, 0.0, 0.01, 0.01],
        // blue gum 15.36 of 17.22 forest acres; 10.03 acres of arroyo willow scrub
        // along Islais Creek; a few planted coast live oaks near the creek.
        "glen-canyon" => [0.02, 0.03, 0.25, 0.40, 0.0, 0.0, 0.0, 0.03, 0.02, 0.01, 0.24],
        // mixed exotic forest 29.42 acres (the WPA's "eucalyptus, cypress, and pine"),
        // blue gum 25.67, cypress, pine and oak under an acre each; 2.86 acres of willow.
        "mclaren" => [0.14, 0.14, 0.28, 0.14, 0.0, 0.05, 0.03, 0.04, 0.02, 0.10, 0.06],
        // the park: Monterey cypress 24 %, Monterey pine 17 %, eucalyptus 20 %, other
        // 39 % (leptospermum, myoporum, acacia, pittosporum); cypress windbreaks at the west edge.
        "ggp-west" => [0.10, 0.10, 0.14, 0.06, 0.12, 0.08, 0.14, 0.02, 0.02, 0.18, 0.04],
        // "cypress trees that came to dominate the area", "thousands of cypress and pine",
        // "shady stands of cypress and eucalyptus"; "flattened and molded by the nearly
        // constant ocean winds".
        "lands-end" => [0.22, 0.14, 0.12, 0.02, 0.28, 0.08, 0.06, 0.0, 0.0, 0.05, 0.03],
        // blue gum 17.11 of 17.16 forest acres; coast live oaks "in the understory of
        // eucalyptus trees and scattered through the northern grassland".
        "bayview-hill" => [0.02, 0.02, 0.28, 0.36, 0.0, 0.0, 0.0, 0.14, 0.14, 0.02, 0.02],
        // 87 trees on the lower slopes (mixed exotic, cypress); Mount Sutro next to it:
        // blue gum, Monterey pine and cypress, blackwood acacia.
        "twin-peaks" => [0.26, 0.15, 0.14, 0.04, 0.04, 0.04, 0.03, 0.02, 0.04, 0.22, 0.02],
        // about 100 trees: mixed exotic 0.77 acres, blue gum 0.56, willow scrub 0.12.
        "bernal" => [0.18, 0.18, 0.30, 0.04, 0.02, 0.02, 0.02, 0.02, 0.02, 0.14, 0.06],
        // about 200 trees: mixed exotic 2.01 acres "mostly on the northern side" (pine
        // heavy: the plan removes 15 pines and "many pines will remain"), blue gum 0.25,
        // coast live oak 0.16 "in the central portion".
        "corona-heights" => [0.12, 0.30, 0.12, 0.02, 0.0, 0.10, 0.02, 0.12, 0.06, 0.12, 0.02],
        _ => [0.25, 0.20, 0.20, 0.05, 0.0, 0.05, 0.05, 0.08, 0.04, 0.06, 0.02],
    }
}

/// How exposed a point is to the ocean wind, 0-1: from 122.47 W inland to
/// 122.505 W at the coast (Lands End's and Ocean Beach's headlands and the
/// park's west end; INFERRED, the prevailing onshore wind off the Pacific).
pub fn exposure(lon: f64) -> f64 {
    ((-lon - 122.470) / (122.505 - 122.470)).clamp(0.0, 1.0)
}

/// A measured tree's species (see the module notes): `draw` its own hash in [0, 1).
pub fn species_of(area: &str, height_m: f64, crown_r_m: f64, x: f64, z: f64, lon: f64, draw: f64) -> u8 {
    let mix = area_mix(area);
    let q = (height_m / crown_r_m.max(0.1)).ln();
    let e = exposure(lon);
    let mut w = [0.0f64; SPECIES];
    for sp in 0..SPECIES {
        if mix[sp] <= 0.0 {
            continue;
        }
        let (lo, hi) = HEIGHT_RANGE_M[sp];
        let out = (lo - height_m).max(height_m - hi).max(0.0);
        let fit_h = 0.5f64.powf(out / HEIGHT_SOFT_M);
        let dq = q - (model_height_m(sp as u8) / MODEL_CROWN_R_M[sp]).ln();
        let fit_q = (-0.5 * (dq / SHAPE_SPREAD).powi(2)).exp();
        let grove = (1.0 - 0.5 * GROVE_SWING) + GROVE_SWING * eo_landmarks::vegetation::field(x, z, GROVE_CELL_M, 500 + sp as u32);
        // the wind-shorn cypress wants the exposed coast; the broadleaves and the stand gum its lee
        let wind = match sp {
            4 => 0.05 + 2.0 * e,
            7..=10 | 3 => 1.0 - 0.6 * e,
            _ => 1.0,
        };
        w[sp] = mix[sp] * fit_h * fit_q * grove.max(0.05) * wind;
    }
    // a species under a fiftieth of the likeliest's is not this tree (no 45 m cypress)
    let top = w.iter().copied().fold(0.0, f64::max);
    for wi in w.iter_mut() {
        if *wi < top / 50.0 {
            *wi = 0.0;
        }
    }
    let total: f64 = w.iter().sum();
    if total <= 0.0 {
        // nothing fits: the species whose height range is nearest
        return (0..SPECIES).min_by(|&a, &b| {
            let d = |s: usize| (HEIGHT_RANGE_M[s].0 - height_m).max(height_m - HEIGHT_RANGE_M[s].1).max(0.0);
            d(a).total_cmp(&d(b))
        }).unwrap_or(0) as u8;
    }
    let mut acc = 0.0;
    for (sp, wi) in w.iter().enumerate() {
        acc += wi / total;
        if draw < acc {
            return sp as u8;
        }
    }
    (SPECIES - 1) as u8
}

/// The onshore wind's downwind direction (game x east, z south): the far
/// cards' `FT_DOWNWIND` (hlod_trees.wgsl).
pub const DOWNWIND: [f64; 2] = [0.940, 0.342];

/// A tree's lean (horizontal shear per metre of height, game x, z): downwind
/// on the exposed coast (the wind-shorn cypress most, `0.2` at full exposure;
/// the others a quarter of that), and every tree a little its own way
/// (`own`, `turn` its hashes; up to 0.03). INFERRED.
pub fn lean_of(sp: u8, lon: f64, own: f64, turn: f64) -> [f32; 2] {
    let e = exposure(lon);
    let k = if sp == 4 { 0.2 } else { 0.05 } * e;
    let (sa, ca) = (turn * std::f64::consts::TAU).sin_cos();
    let r = 0.03 * own;
    [(DOWNWIND[0] * k + ca * r) as f32, (DOWNWIND[1] * k + sa * r) as f32]
}

/// One measured tree placed on the drawn ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Game metres; `y` the drawn ground less the root sink.
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub height_m: f64,
    pub crown_r_m: f64,
    pub form: Form,
    /// Horizontal shear per metre of height ([`lean_of`]; none with [`VARIETY_SWITCH`]).
    pub lean: [f32; 2],
    pub yaw: f64,
    /// A per-tree draw in [0, 1): its leaf tint's variation (the near tier's
    /// 0.85 + 0.3 draw, as `eo_landmarks::vegetation::far_tint`'s).
    pub draw: f64,
}

/// What a measured tree is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `eo_landmarks::vegetation::Kind::Tree` model (a species or its second build).
    Coastal(u8),
    /// ([`VARIETY_SWITCH`] only) A small broadleaf ([`legacy::BROADLEAF_SPECIES`]'s ez-tree).
    Broadleaf,
}

impl Placed {
    /// The model's scale: [across, up, across] (the height's exactly, the crown's within `ANISO_MAX`).
    pub fn scale(&self) -> [f32; 3] {
        let (h, r) = match self.form {
            Form::Coastal(sp) => (model_height_m(sp), MODEL_CROWN_R_M[sp as usize]),
            Form::Broadleaf => (legacy::BROADLEAF_HEIGHT_M, legacy::BROADLEAF_CROWN_R_M),
        };
        let sy = self.height_m / h;
        let sx = (self.crown_r_m / r).clamp(sy / ANISO_MAX, sy * ANISO_MAX);
        [sx as f32, sy as f32, sx as f32]
    }
}

/// The measured trees whose position lies in [x0, x1) x [z0, z1) (game
/// metres) and that stand where the lattice lets a park tree stand: nothing
/// `blocked` (a footprint, the drawn street, water) within one 2 m cell, and
/// the ground cover's `tree_ground_ok` on the ground `ground` (the 8 m
/// surface; `--no-cliff-rock-cover`: not tested, as the lattice). None
/// without the file or with [`SWITCH`]. Both tiers call this with their own
/// ground and `blocked`, which are the same street world's. With `relief`
/// a tree stands on the drawn relief where the hill builder's 2 m relief
/// moved the ground (`eo_data::detail_ground`, as the coastal plants do),
/// else on `ground`: the near tier's, cooked once the scene has set it; the
/// distant cards are gathered while the scene loads, so they keep `ground`
/// (at least 270 m off, the relief's metre or so does not show).
pub fn placed_in(x0: f64, z0: f64, x1: f64, z1: f64, ground: &dyn Fn(f64, f64) -> f64, blocked: &dyn Fn(f64, f64) -> bool, relief: bool) -> Vec<Placed> {
    let Some(lt) = installed() else { return vec![] };
    let gs = eo_data::groundscale::scale();
    let cover = eo_landmarks::ground_cover::installed();
    let cliff_rule = eo_landmarks::vegetation::gg3().cliff_rock_cover;
    let mut out = vec![];
    for t in lt.in_box(x0 / gs, z0 / gs, x1 / gs, z1 / gs) {
        let (x, z) = (t.x as f64 * gs, t.z as f64 * gs);
        // the box test again on the drawn plan (the division can round a tree across an edge).
        if !(x >= x0 && x < x1 && z >= z0 && z < z1) {
            continue;
        }
        if [-1.0, 0.0, 1.0].iter().any(|dz| [-1.0, 0.0, 1.0].iter().any(|dx| blocked(x + dx * 2.0, z + dz * 2.0))) {
            continue;
        }
        let y = if relief { eo_data::detail_ground::height(x, z).unwrap_or_else(|| ground(x, z)) } else { ground(x, z) };
        if !y.is_finite() {
            continue;
        }
        if cliff_rule && cover.is_some_and(|c| !c.tree_ground_ok(x, z, ground)) {
            continue;
        }
        // the coastal trees' own draws (the near tint's and `far_tint`'s cell).
        let (ix, iz) = ((x * 8.0) as i64, (z * 8.0) as i64);
        let h = |salt: u32| eo_landmarks::vegetation::hash(ix, iz, salt);
        let (form, lean) = if variety_on() {
            let area = lt.header().areas.get(t.area as usize).map(|a| a.name.as_str()).unwrap_or("");
            let (lon, _) = eo_landmarks::rocks::place::lon_lat(x, z, gs);
            let base = species_of(area, t.height_m as f64, t.crown_r_m as f64, t.x as f64, t.z as f64, lon, h(94));
            // half the trees of the species with a second build draw it
            (Form::Coastal(eo_landmarks::vegetation::tree_model(base, h(97))), lean_of(base, lon, h(95), h(96)))
        } else {
            (legacy::form_of(t.height_m as f64, t.crown_r_m as f64), [0.0; 2])
        };
        out.push(Placed {
            x,
            y: y - ROOT_SINK_M,
            z,
            height_m: t.height_m as f64,
            crown_r_m: t.crown_r_m as f64,
            form,
            lean,
            yaw: h(93) * std::f64::consts::TAU,
            draw: h(92),
        });
    }
    out
}

/// The species' far albedo targets' luminance range (`eo_landmarks::vegetation::FAR_TREE_ALBEDO`:
/// the live oak's, darkest, to the willow's).
pub const FAR_ALBEDO_LUM: [f32; 2] = [0.044, 0.075];
/// How far inside the far shader's leaf range (`eo_render::hlod_trees::LEAF_LUM`)
/// the species' own shades sit, so a tree's own light (+-15 %) is not all clipped.
pub const FAR_LEAF_MARGIN: f32 = 0.005;

/// A far card's crown colour (sRGB bytes) with its species' own shade, for a
/// lidar tree of model `sp` at (x, z). The far shader holds a crown's leaf
/// luminance in a measured leaf's range and pulls its tint's chroma most of
/// the way to one leaf's (`eo_render::hlod_trees::LEAF_LUM`, `LEAF_CHROMA`,
/// `LEAF_PULL`); every species' target lies under that range, so every
/// card drew the same leaf. Here the targets' luminance range maps onto the
/// leaf's (the oak darkest, the willow and the blue gum lightest) and the
/// tint's chroma is set so the pull lands on the species' own (the gum's
/// grey-blue, the cypress's deep green), the tree's own hue on it
/// (`eo_landmarks::vegetation::tree_hue`) and its light as `far_tint`'s.
pub fn far_tint_own(sp: u8, x: f64, z: f64) -> [u8; 3] {
    use eo_render::hlod_trees::{LEAF_CHROMA, LEAF_LUM, LEAF_PULL};
    let a = eo_landmarks::vegetation::FAR_TREE_ALBEDO[sp as usize];
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let la = lum(a);
    let t = ((la - FAR_ALBEDO_LUM[0]) / (FAR_ALBEDO_LUM[1] - FAR_ALBEDO_LUM[0])).clamp(0.0, 1.0);
    let leaf = LEAF_LUM[0] + FAR_LEAF_MARGIN + t * (LEAF_LUM[1] - LEAF_LUM[0] - 2.0 * FAR_LEAF_MARGIN);
    let hue = eo_landmarks::vegetation::tree_hue(x, z);
    let want = [a[0] / la * hue[0], a[1] / la * hue[1], a[2] / la * hue[2]];
    // the chroma the shader's pull takes to `want`
    let c = [0, 1, 2].map(|i| ((want[i] - LEAF_PULL * LEAF_CHROMA[i]) / (1.0 - LEAF_PULL)).max(0.05));
    let k = leaf * eo_landmarks::vegetation::far_tint_gain(x, z) / lum(c);
    c.map(|ci| eo_landmarks::vegetation::srgb_byte(ci * k))
}

/// The distant card of a placed tree: the near tier's model at its scale (a
/// coastal species' card in its form, `--no-far-species`: the round crown;
/// the broadleaf as a round card).
pub fn far_tree(p: &Placed, far_species: bool) -> eo_render::hlod_trees::FarTree {
    use eo_render::hlod_trees::{FarTree, SPECIES_MODEL_SHIFT};
    let s = p.scale();
    let seed = {
        let mut h = ((p.x * 10.0) as i64 as u32 ^ ((p.z * 10.0) as i64 as u32).rotate_left(16)).wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
        h ^= h >> 16;
        h = h.wrapping_mul(0x7feb_352d);
        h ^ (h >> 15)
    };
    let pack = |c: [u8; 3]| c[0] as u32 | ((c[1] as u32) << 8) | ((c[2] as u32) << 16) | (255 << 24);
    let pos = [p.x as f32, p.y as f32, p.z as f32];
    match p.form {
        Form::Coastal(sp) => {
            let (h, c) = eo_landmarks::vegetation::FAR_TREE_SIZE[sp as usize];
            let tint = if variety_on() {
                far_tint_own(sp, p.x, p.z)
            } else {
                eo_landmarks::vegetation::far_tint(sp, p.x, p.z)
            };
            // the card's form: the far shader's three silhouettes (cypress, pine, gum) and its round crown
            let form = eo_landmarks::vegetation::far_card_form(sp);
            // the baked canopy's model (cove_canopy): the park pine's for the conifers, the ash's for the gum (unused outside Black Point).
            let model = if form == 3 { 2 } else { 3 };
            FarTree { pos, height: h * s[1], crown: c * s[0], tint: pack(tint), seed, species: (if far_species { form } else { 0 }) | (model << SPECIES_MODEL_SHIFT) }
        }
        Form::Broadleaf => FarTree {
            pos,
            height: (legacy::BROADLEAF_HEIGHT_M * s[1] as f64) as f32,
            crown: (legacy::BROADLEAF_CARD_CROWN * legacy::BROADLEAF_CROWN_R_M * s[0] as f64) as f32,
            tint: pack(legacy::BROADLEAF_CARD_SRGB),
            seed,
            species: 1 << SPECIES_MODEL_SHIFT,
        },
    }
}

/// ([`VARIETY_SWITCH`]) The species rule before the parks' own mixes: most of
/// these hills' trees are planted blue gum, Monterey cypress and Monterey pine
/// groves; the one whose height over crown radius (the model's visible
/// crown, [`MODEL_CROWN_R_M`]) is nearest the measured tree's, a blue gum
/// only from [`GUM_MIN_M`](legacy::GUM_MIN_M) up; over
/// [`GUM_OVER_M`](legacy::GUM_OVER_M) a blue gum. A small broad tree is a
/// broadleaf, the street species with the broadest crown.
pub mod legacy {
    use super::{Form, MODEL_CROWN_R_M, model_height_m};

    /// The small broadleaf's model: the street species with the broadest crown
    /// for its height (`street-full-wrap`, height over visible crown radius 2.5;
    /// the street oak's is 5.3), its height and visible crown radius at scale 1
    /// (m), measured as [`MODEL_CROWN_R_M`] is.
    pub const BROADLEAF_SPECIES: &str = "street-full-wrap";
    pub const BROADLEAF_HEIGHT_M: f64 = 8.60;
    pub const BROADLEAF_CROWN_R_M: f64 = 3.42;
    /// The broadleaf's distant card: its tint (the park oak's card, hlod_trees
    /// `park_tree`) and its crown half width over the visible crown radius (the
    /// coastal species' cards are 1.1-1.25 times theirs).
    pub const BROADLEAF_CARD_SRGB: [u8; 3] = [62, 82, 46];
    pub const BROADLEAF_CARD_CROWN: f64 = 1.2;
    /// Under this height (m) a broad tree is a broadleaf (INFERRED: the hills'
    /// small trees, coast live oak, willow, toyon, acacia, are 4-9 m).
    pub const SMALL_TREE_M: f64 = 9.0;
    /// A small tree is broad while its height over crown radius is under this
    /// (between the broadleaf model's 2.5 and the pine's 3.8).
    pub const SMALL_BROAD_RATIO: f64 = 3.2;
    /// A blue gum only from this height up (m; INFERRED: a young gum reads as a pine).
    pub const GUM_MIN_M: f64 = 15.0;
    /// Over this height a tree is a blue gum whatever its crown (m): the Monterey
    /// pine's published height bound, the cypress's is lower (eo-props
    /// coastal.rs: conifers.org, pine 15-30 m, cypress to 25 m; blue gum 30-55 m).
    pub const GUM_OVER_M: f64 = 30.0;

    /// The form of a tree `height_m` tall with a crown `crown_r_m` across.
    pub fn form_of(height_m: f64, crown_r_m: f64) -> Form {
        let q = height_m / crown_r_m.max(0.1);
        if height_m < SMALL_TREE_M && q < SMALL_BROAD_RATIO {
            return Form::Broadleaf;
        }
        if height_m > GUM_OVER_M {
            return Form::Coastal(2);
        }
        let mut best = (f64::MAX, 0u8);
        for sp in 0..3u8 {
            if sp == 2 && height_m < GUM_MIN_M {
                continue;
            }
            let d = (q.ln() - (model_height_m(sp) / MODEL_CROWN_R_M[sp as usize]).ln()).abs();
            if d < best.0 {
                best = (d, sp);
            }
        }
        Form::Coastal(best.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The radius of a circle of the convex hull's area of the build's leaves
    /// over 30 % of its height, and the build's height.
    fn crown_of(id: &str) -> (f64, f64) {
        let m = eo_props::eztree::build_ez_tree(id, "near", 0).unwrap();
        let h = m.positions.iter().map(|p| p[1]).fold(0.0f32, f32::max) as f64;
        let mut pts: Vec<(f64, f64)> = m.positions.iter().zip(&m.part).filter(|(p, part)| **part == 1 && p[1] as f64 >= 0.3 * h).map(|(p, _)| (p[0] as f64, p[2] as f64)).collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        pts.dedup();
        let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
        let mut hull: Vec<(f64, f64)> = vec![];
        for pass in 0..2 {
            let start = hull.len();
            let it: Box<dyn Iterator<Item = &(f64, f64)>> = if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
            for &p in it {
                while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
                    hull.pop();
                }
                hull.push(p);
            }
            hull.pop();
        }
        let area = (0..hull.len()).map(|i| hull[i].0 * hull[(i + 1) % hull.len()].1 - hull[(i + 1) % hull.len()].0 * hull[i].1).sum::<f64>().abs() / 2.0;
        ((area / std::f64::consts::PI).sqrt(), h)
    }

    #[test]
    fn model_crowns_are_the_builds() {
        let got: Vec<(f64, f64)> = eo_props::eztree::COASTAL_TREES.iter().map(|id| crown_of(id)).collect();
        for (id, (r, h)) in eo_props::eztree::COASTAL_TREES.iter().zip(&got) {
            eprintln!("{id}: visible crown radius {r:.2} m, height {h:.2} m");
        }
        assert_eq!(got.len(), MODELS);
        for (sp, &(r, h)) in got.iter().enumerate() {
            assert!((r - MODEL_CROWN_R_M[sp]).abs() < 0.03 * r, "species {sp}: crown radius {r:.2}, the table {}", MODEL_CROWN_R_M[sp]);
            assert!((h - model_height_m(sp as u8)).abs() < 0.03 * h, "species {sp}: height {h:.2}");
        }
    }

    #[test]
    fn each_park_draws_its_own_mix() {
        // the species a park's trees draw, over a spread of sizes and places
        let count = |area: &str, lon: f64| {
            let mut n = [0usize; SPECIES];
            for k in 0..4000 {
                let (h, r) = (4.0 + (k % 40) as f64, 2.5 + (k % 13) as f64 * 0.6);
                let (x, z) = ((k * 37 % 997) as f64 * 3.1, (k * 53 % 991) as f64 * 2.7);
                n[species_of(area, h, r, x, z, lon, ((k as f64) * 0.618_034).fract()) as usize] += 1;
            }
            n
        };
        let davidson = count("mount-davidson", -122.454);
        assert!(davidson[2] + davidson[3] > 2000, "Mount Davidson is a gum forest: {davidson:?}");
        let lands_end = count("lands-end", -122.505);
        assert!(lands_end[4] > lands_end[7] + lands_end[8] && lands_end[4] > 500, "Lands End's exposed headland is wind-shorn cypress: {lands_end:?}");
        let glen = count("glen-canyon", -122.442);
        assert!(glen[10] > 100 && glen[7] + glen[8] > 300, "Glen Canyon has willows and oaks: {glen:?}");
        // no wind-shorn cypress inland
        assert!(count("mount-davidson", -122.454)[4] == 0);
        // every park draws several species
        for (a, lon) in [("mclaren", -122.42), ("ggp-west", -122.50), ("bayview-hill", -122.39), ("twin-peaks", -122.447), ("bernal", -122.414), ("corona-heights", -122.438)] {
            let n = count(a, lon);
            assert!(n.iter().filter(|&&c| c > 100).count() >= 4, "{a}: {n:?}");
        }
    }

    #[test]
    fn species_follow_height_and_the_coast_leans() {
        // over 40 m only a gum
        for k in 0..50 {
            let sp = species_of("mclaren", 45.0, 7.0, k as f64 * 31.0, 0.0, -122.42, (k as f64 * 0.37).fract());
            assert!(sp == 2 || sp == 3, "a 45 m tree drew species {sp}");
        }
        // the height is drawn exactly; the crown within ANISO_MAX of it.
        let p = Placed { x: 0.0, y: 0.0, z: 0.0, height_m: 30.0, crown_r_m: 30.0, form: Form::Coastal(2), lean: [0.0; 2], yaw: 0.0, draw: 0.0 };
        let s = p.scale();
        assert!((s[1] as f64 * model_height_m(2) - 30.0).abs() < 1e-3);
        assert!((s[0] as f64 - s[1] as f64 * ANISO_MAX).abs() < 1e-4);
        // downwind on the coast, near straight inland
        let l = lean_of(4, -122.51, 0.0, 0.0);
        assert!(l[0] > 0.15 && l[1] > 0.05, "{l:?}");
        let l = lean_of(4, -122.43, 1.0, 0.3);
        assert!(l[0].hypot(l[1]) <= 0.031, "{l:?}");
    }

    #[test]
    fn far_cards_keep_their_species_shade() {
        // what the far shader makes of a card's tint (hlod_trees.wgsl `ft_leaf_r`, before its own per-tree light)
        use eo_render::hlod_trees::{LEAF_CHROMA, LEAF_LUM, LEAF_PULL};
        let dec = |b: u8| {
            let e = b as f32 / 255.0;
            if e <= 0.04045 { e / 12.92 } else { ((e + 0.055) / 1.055).powf(2.4) }
        };
        let leaf = |t: [u8; 3]| {
            let t = t.map(dec);
            let l = 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
            let c = [0, 1, 2].map(|i| t[i] / l + LEAF_PULL * (LEAF_CHROMA[i] - t[i] / l));
            c.map(|ci| ci * l.clamp(LEAF_LUM[0], LEAF_LUM[1]))
        };
        // the mean over many trees of a species
        let mean = |sp: u8, own: bool| {
            let mut m = [0.0f32; 3];
            for k in 0..400 {
                let (x, z) = (k as f64 * 7.3, (k * 13 % 97) as f64 * 5.1);
                let r = leaf(if own { far_tint_own(sp, x, z) } else { eo_landmarks::vegetation::far_tint(sp, x, z) });
                for i in 0..3 {
                    m[i] += r[i] / 400.0;
                }
            }
            m
        };
        let g = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        // before: the oak, the cypress and the willow drew within a few per cent of one leaf
        let (oak0, cyp0, wil0) = (mean(7, false), mean(0, false), mean(10, false));
        assert!(g(wil0) / g(oak0) < 1.12, "{oak0:?} {wil0:?}");
        // now: the willow and the gum lighter than the cypress, the cypress than the oak
        let (oak, cyp, gum, wil) = (mean(7, true), mean(0, true), mean(2, true), mean(10, true));
        assert!(g(wil) / g(oak) > 1.25 && g(gum) > g(cyp) && g(cyp) > g(oak), "oak {oak:?} cypress {cyp:?} gum {gum:?} willow {wil:?}");
        // the gum grey-blue, the cypress deep green (blue over green)
        assert!(gum[2] / gum[1] > 0.72 && cyp[2] / cyp[1] < 0.56, "gum {gum:?} cypress {cyp:?} (before {cyp0:?})");
    }

    #[test]
    fn legacy_forms_follow_height_and_crown() {
        // (--no-tree-variety) tall and narrow: blue gum; broad: cypress; small and broad: a broadleaf; a young narrow tree: no gum.
        use legacy::form_of;
        assert_eq!(form_of(35.0, 5.0), Form::Coastal(2));
        assert_eq!(form_of(14.0, 8.0), Form::Coastal(0));
        assert_eq!(form_of(20.0, 5.5), Form::Coastal(1));
        assert_eq!(form_of(36.0, 12.0), Form::Coastal(2));
        assert_eq!(form_of(6.0, 3.0), Form::Broadleaf);
        assert_ne!(form_of(12.0, 2.5), Form::Coastal(2));
        let (r, h) = crown_of(legacy::BROADLEAF_SPECIES);
        assert!((r - legacy::BROADLEAF_CROWN_R_M).abs() < 0.03 * r && (h - legacy::BROADLEAF_HEIGHT_M).abs() < 0.03 * h, "{}: {r:.2} {h:.2}", legacy::BROADLEAF_SPECIES);
    }
}
