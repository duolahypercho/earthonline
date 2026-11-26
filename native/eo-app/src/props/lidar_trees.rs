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
//! ground. Its position follows the ground plan's scale
//! (`eo_data::groundscale`); its height and crown stay real size, as every
//! model does.
//!
//! Its form ([`form_of`]): most of these hills' trees are planted blue gum,
//! Monterey cypress and Monterey pine groves (the coastal species of
//! eo-props `eztree/coastal.rs`); the one whose height over crown radius
//! (the model's visible crown, [`MODEL_CROWN_R_M`]) is nearest the
//! measured tree's, a blue gum only from [`GUM_MIN_M`] up: the tall narrow
//! crowns blue gum, the broad ones cypress; over [`GUM_OVER_M`] (taller than
//! either conifer grows) a blue gum. A small broad tree (under
//! [`SMALL_TREE_M`]: coast live oak, willow, toyon) is a broadleaf, the
//! street species with the broadest crown ([`BROADLEAF_SPECIES`]). The model
//! is scaled to the measured height, and across to the measured crown within
//! [`ANISO_MAX`] of the height's scale.

use eo_hills::trees::LidarTrees;
use std::sync::OnceLock;

/// The switch.
pub const SWITCH: &str = "--no-lidar-trees";

/// The coastal species' drawn height at scale 1 (m; `Kind::Tree` order:
/// cypress, pine, blue gum): eo-props coastal.rs, `FAR_TREE_SIZE`.
pub fn model_height_m(sp: u8) -> f64 {
    eo_landmarks::vegetation::FAR_TREE_SIZE[sp as usize].0 as f64
}

/// The coastal species' visible crown radius at scale 1 (m; `Kind::Tree`
/// order): the radius of a circle of the area of the convex hull of the near
/// build's leaves over 30 % of its height (the lidar crown's own floor,
/// eo-hills cook trees `CROWN_BASE_FRACTION`). Measured from the builds by
/// the test `model_crowns_are_the_builds`.
pub const MODEL_CROWN_R_M: [f64; 3] = [8.37, 6.30, 6.05];
/// The small broadleaf's model: the street species with the broadest crown
/// for its height (`street-full-wrap`, height over visible crown radius 2.5;
/// the street oak's is 5.3), its height and visible crown radius at scale 1
/// (m), measured as above.
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

/// What a measured tree is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `eo_landmarks::vegetation::Kind::Tree` species: 0 cypress, 1 pine, 2 blue gum.
    Coastal(u8),
    /// A small broadleaf ([`BROADLEAF_SPECIES`]'s ez-tree).
    Broadleaf,
}

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
    pub yaw: f64,
    /// A per-tree draw in [0, 1): its leaf tint's variation (the near tier's
    /// 0.85 + 0.3 draw, as `eo_landmarks::vegetation::far_tint`'s).
    pub draw: f64,
}

impl Placed {
    /// The model's scale: [across, up, across] (the height's exactly, the crown's within `ANISO_MAX`).
    pub fn scale(&self) -> [f32; 3] {
        let (h, r) = match self.form {
            Form::Coastal(sp) => (model_height_m(sp), MODEL_CROWN_R_M[sp as usize]),
            Form::Broadleaf => (BROADLEAF_HEIGHT_M, BROADLEAF_CROWN_R_M),
        };
        let sy = self.height_m / h;
        let sx = (self.crown_r_m / r).clamp(sy / ANISO_MAX, sy * ANISO_MAX);
        [sx as f32, sy as f32, sx as f32]
    }
}

/// The measured trees whose position lies in [x0, x1) x [z0, z1) (game
/// metres) and that stand where the lattice lets a park tree stand: nothing
/// `blocked` (a footprint, the drawn street, water) within one 2 m cell, and
/// the ground cover's `tree_ground_ok` on the drawn ground `ground`
/// (`--no-cliff-rock-cover`: not tested, as the lattice). None without the
/// file or with [`SWITCH`]. Both tiers call this with their own ground and
/// `blocked`, which are the same street world's.
pub fn placed_in(x0: f64, z0: f64, x1: f64, z1: f64, ground: &dyn Fn(f64, f64) -> f64, blocked: &dyn Fn(f64, f64) -> bool) -> Vec<Placed> {
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
        let y = ground(x, z);
        if !y.is_finite() {
            continue;
        }
        if cliff_rule && cover.is_some_and(|c| !c.tree_ground_ok(x, z, ground)) {
            continue;
        }
        // the coastal trees' own draws (the near tint's and `far_tint`'s cell).
        let (ix, iz) = ((x * 8.0) as i64, (z * 8.0) as i64);
        out.push(Placed {
            x,
            y: y - ROOT_SINK_M,
            z,
            height_m: t.height_m as f64,
            crown_r_m: t.crown_r_m as f64,
            form: form_of(t.height_m as f64, t.crown_r_m as f64),
            yaw: eo_landmarks::vegetation::hash(ix, iz, 93) * std::f64::consts::TAU,
            draw: eo_landmarks::vegetation::hash(ix, iz, 92),
        });
    }
    out
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
            let tint = eo_landmarks::vegetation::far_tint(sp, p.x, p.z);
            // the baked canopy's model (cove_canopy): the park pine's for the conifers, the ash's for the gum (unused outside Black Point).
            let model = if sp == 2 { 2 } else { 3 };
            FarTree { pos, height: h * s[1], crown: c * s[0], tint: pack(tint), seed, species: (if far_species { sp as u32 + 1 } else { 0 }) | (model << SPECIES_MODEL_SHIFT) }
        }
        Form::Broadleaf => FarTree {
            pos,
            height: (BROADLEAF_HEIGHT_M * s[1] as f64) as f32,
            crown: (BROADLEAF_CARD_CROWN * BROADLEAF_CROWN_R_M * s[0] as f64) as f32,
            tint: pack(BROADLEAF_CARD_SRGB),
            seed,
            species: 1 << SPECIES_MODEL_SHIFT,
        },
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
        let got: Vec<(f64, f64)> = eo_props::eztree::COASTAL_TREES.iter().chain([BROADLEAF_SPECIES].iter()).map(|id| crown_of(id)).collect();
        for (id, (r, h)) in eo_props::eztree::COASTAL_TREES.iter().chain([BROADLEAF_SPECIES].iter()).zip(&got) {
            eprintln!("{id}: visible crown radius {r:.2} m, height {h:.2} m");
        }

        for sp in 0..3 {
            let (r, h) = got[sp];
            assert!((r - MODEL_CROWN_R_M[sp]).abs() < 0.03 * r, "species {sp}: crown radius {r:.2}, the table {}", MODEL_CROWN_R_M[sp]);
            assert!((h - model_height_m(sp as u8)).abs() < 0.03 * h, "species {sp}: height {h:.2}");
        }
        let (r, h) = got[3];
        assert!((r - BROADLEAF_CROWN_R_M).abs() < 0.03 * r && (h - BROADLEAF_HEIGHT_M).abs() < 0.03 * h, "{BROADLEAF_SPECIES}: {r:.2} {h:.2}");
        // the broadleaf is the broadest street species for its height.
        for id in eo_props::eztree::street_tree_species().unwrap().iter().filter(|id| id.as_str() != BROADLEAF_SPECIES && !id.contains("bare")) {
            let (ri, hi) = crown_of(id);
            assert!(hi / ri >= h / r, "{id}: height over crown {:.2} under {BROADLEAF_SPECIES}'s {:.2}", hi / ri, h / r);
        }
    }

    #[test]
    fn forms_follow_height_and_crown() {
        // tall and narrow: blue gum; broad: cypress; small and broad: a broadleaf; a young narrow tree: no gum.
        assert_eq!(form_of(35.0, 5.0), Form::Coastal(2));
        assert_eq!(form_of(14.0, 8.0), Form::Coastal(0));
        assert_eq!(form_of(20.0, 5.5), Form::Coastal(1));
        assert_eq!(form_of(36.0, 12.0), Form::Coastal(2));
        assert_eq!(form_of(6.0, 3.0), Form::Broadleaf);
        assert_ne!(form_of(12.0, 2.5), Form::Coastal(2));
        // the height is drawn exactly; the crown within ANISO_MAX of it.
        let p = Placed { x: 0.0, y: 0.0, z: 0.0, height_m: 30.0, crown_r_m: 30.0, form: Form::Coastal(2), yaw: 0.0, draw: 0.0 };
        let s = p.scale();
        assert!((s[1] as f64 * model_height_m(2) - 30.0).abs() < 1e-3);
        assert!((s[0] as f64 - s[1] as f64 * ANISO_MAX).abs() < 1e-4);
    }
}
