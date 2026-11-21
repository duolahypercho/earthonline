//! The second flora pass round the Golden Gate (`--no-flora2` restores the
//! flora pass's output): the Battery Spencer knoll's bare chert ground and
//! eroded cut face, and the Baker bluff's serpentinite rock and its beach
//! boulders.
//!
//! Source of truth: the Golden Gate field guide (sections 4.1 to 4.3: the
//! knoll's measured soil and face colours, serpentinite's scaly fabric and
//! colours, the boulders' sizes and spread), the USGS 3DEP 1 m DEM (the
//! knoll's convexity and slope) and the NAIP image (its green). Values
//! marked INFERRED are this pass's choices; [P:spencer] and [P:baker-sunset]
//! are the guide's photographs.

use crate::vegetation::smoothstep;

/// The pass's switch names (the roll-up first); eo-app accepts them,
/// [`Flora2::from_args`] reads them.
pub const FLORA2_SWITCHES: &[&str] = &["--no-flora2", "--no-knoll-ground", "--no-knoll-relief", "--no-bluff-rock", "--no-boulder-surf"];

/// The pass's features, each with its `--no-` switch ([`FLORA2_SWITCHES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flora2 {
    /// `--no-knoll-ground`: the Marin chert's convex knolls and noses ([`knoll`]:
    /// the drawn ground's convexity and slope, the NAIP image without green) are
    /// bare: no scrub, the knoll top rust soil, the cut face eroded chert with
    /// rubble and sparse cured grass (terrain material and props).
    pub knoll: bool,
    /// `--no-knoll-relief`: the cut face's gullies and rills in the 2 m relief
    /// ([`gully_m`]) and in the terrain shader.
    pub relief: bool,
    /// `--no-bluff-rock`: the Baker and Marshall's bluff faces steeper than about
    /// 40 degrees are serpentinite rock: the terrain's colour and a sheared, scaly
    /// rock in the shader ([`BLUFF_ROCK_SRGB`], rust staining, darker where wet
    /// near the waterline), and the hill rock there as broken, blocky slabs
    /// that cover the face (not single rounded knobs).
    pub bluff_rock: bool,
    /// `--no-boulder-surf`: the beach boulders dark ([`BOULDER_DRY_SRGB`]), wet
    /// below the swash line with a sheen, and clustered in the surf and on the
    /// lower beach ([`boulder_chance`]).
    pub boulders: bool,
}

impl Flora2 {
    pub const ALL: Flora2 = Flora2 { knoll: true, relief: true, bluff_rock: true, boulders: true };
    pub const NONE: Flora2 = Flora2 { knoll: false, relief: false, bluff_rock: false, boulders: false };

    pub fn from_args(args: &[String]) -> Self {
        let has = |s: &str| args.iter().any(|a| a == s);
        if has("--no-flora2") {
            return Self::NONE;
        }
        Flora2 { knoll: !has("--no-knoll-ground"), relief: !has("--no-knoll-relief"), bluff_rock: !has("--no-bluff-rock"), boulders: !has("--no-boulder-surf") }
    }
}

/// The pass's switches of this process (read once from the command line). It
/// builds on the flora pass: the knoll on its chert ground, the bluff and the
/// boulders on its serpentinite; with those off it is off too.
pub fn flora2() -> Flora2 {
    if cfg!(test) {
        return Flora2::NONE;
    }
    static S: std::sync::OnceLock<Flora2> = std::sync::OnceLock::new();
    *S.get_or_init(|| {
        let f = Flora2::from_args(&std::env::args().collect::<Vec<_>>());
        let fl = crate::flora::flora();
        Flora2 { knoll: f.knoll && fl.chert, relief: f.relief && f.knoll && fl.chert, bluff_rock: f.bluff_rock && fl.serpentinite, boulders: f.boulders && fl.serpentinite }
    })
}

/// The (lon, lat) of game (x, z): `vegetation::game_xz` inverted.
pub fn geo_of(x: f64, z: f64) -> (f64, f64) {
    let f = eo_data::Frame::sf();
    let s = eo_data::groundscale::scale();
    (f.center_lon + x / (f.m_per_deg_lon * s), f.center_lat - z / (f.m_per_deg_lat * s))
}

// ------------------------------------------------------------------ the knoll

/// A knoll top or nose: the ground this far over the mean of a ring 12 m out
/// (game m; `Site::hollow_m` negated), ramped. The Battery Spencer knoll's top
/// and cut face stand 2-6 m over their ring in the 1 m DEM (probed along the
/// headland view's rays); the overlook's rounded shoulders 0.5-1.5 m.
pub const KNOLL_CONVEX_M: (f64, f64) = (1.4, 2.6);
/// ... and a knoll, not the convex edge of a bench or platform: the ground this
/// far over the mean of a ring 25 m out (game m), ramped. Probed along the
/// headland view's rays on the drawn ground (the plants' `Site`): the knoll's top
/// and face stand 8.6-16 m over their 25 m ring, the battery platform's convex
/// edge before the eye 5.0-7.0 m (its 12 m ring reads it as convex as the
/// knoll's). The 1 m DEM itself reads 1-2 m more (10-16 and 4.8-7.9 m: the drawn
/// relief is clamped and laid on the 8 m grid): [`KNOLL_CONVEX25_DEM_M`].
pub const KNOLL_CONVEX25_M: (f64, f64) = (7.0, 8.5);
pub const KNOLL_CONVEX25_DEM_M: (f64, f64) = (8.2, 9.6);
/// The ring radii (game m).
pub const KNOLL_RING_M: (f64, f64) = (12.0, 25.0);
/// The cut face: steeper than this ramp (degrees; the knoll's face 35-54 in the
/// DEM, the steepest grassland about 35, guide 5).
pub const KNOLL_FACE_DEG: (f64, f64) = (30.0, 40.0);
/// The NAIP image's green share (pixels passing the green test, `Naip::green`)
/// over which the ground is plants, not soil, whatever its shape. The face reads
/// dark grey (77-117, 83-107, 76-96) in the summer image, which the classifier
/// calls sage canopy (dark, blue under green), not green: on a convex nose
/// steeper than 35 degrees the same dark grey is the eroded chert's (red chert
/// weathers dark; the photograph shows the face bare) (INFERRED).
pub const KNOLL_GREEN: (f64, f64) = (0.2, 0.45);
/// Over this height (game m) only: the chert, not the waterline basalt.
pub const KNOLL_MIN_Y: f64 = crate::flora::CHERT_MIN_Y;

/// How bare a point of Marin chert ground is as the knoll's (top, face), 0-1
/// each: convex (`hollow_m` negative: over its 12 m ring) and not green in the
/// image; the face where steep, the top where flatter.
/// `c25`: the 25 m ring's convexity weight (0-1; [`KNOLL_CONVEX25_M`]).
pub fn knoll(slope_deg: f64, hollow_m: f64, c25: f64, green: f64) -> (f64, f64) {
    let convex = smoothstep(KNOLL_CONVEX_M.0, KNOLL_CONVEX_M.1, -hollow_m) * c25;
    let bare = convex * (1.0 - smoothstep(KNOLL_GREEN.0, KNOLL_GREEN.1, green));
    let face = smoothstep(KNOLL_FACE_DEG.0, KNOLL_FACE_DEG.1, slope_deg);
    // the steepest faces past 58 degrees are rock the hill rock dresses.
    (bare * (1.0 - face), bare * face)
}

/// [`knoll`] at a planting site (x, z): the Marin chert over [`KNOLL_MIN_Y`] within
/// the NAIP image only (none elsewhere), the pass on; the 25 m ring read from
/// `site` where the 12 m one is convex.
pub fn knoll_site(site: &dyn Fn(f64, f64) -> Option<crate::vegetation::Site>, s: &crate::vegetation::Site, x: f64, z: f64, naip: Option<crate::vegetation::Naip>) -> (f64, f64) {
    if !flora2().knoll || s.lithology != crate::vegetation::Lithology::Chert || z >= crate::vegetation::marin_z() || s.y < KNOLL_MIN_Y || -s.hollow_m < KNOLL_CONVEX_M.0 {
        return (0.0, 0.0);
    }
    let Some(n) = naip else { return (0.0, 0.0) };
    if n.green as f64 >= KNOLL_GREEN.1 {
        return (0.0, 0.0);
    }
    let r = KNOLL_RING_M.1;
    let mut ring = 0.0;
    for k in 0..8 {
        let (sn, cs) = (k as f64 * std::f64::consts::FRAC_PI_4).sin_cos();
        let Some(q) = site(x + cs * r, z + sn * r) else { return (0.0, 0.0) };
        ring += q.y;
    }
    knoll(s.slope_deg(), s.hollow_m, smoothstep(KNOLL_CONVEX25_M.0, KNOLL_CONVEX25_M.1, s.y - ring / 8.0), n.green as f64)
}

/// The cured grass on the bare knoll: the share of 1 m cells with a tuft, top
/// and face (the photograph's "sparse grass"; INFERRED).
pub const KNOLL_GRASS: (f64, f64) = (0.12, 0.22);
/// Chert rubble on the face: the share of 1 m cells with a stone, and the
/// stones' size (real m; the guide's talus 5-30 cm, 4.1, estimate).
pub const KNOLL_RUBBLE: f64 = 0.12;
pub const KNOLL_RUBBLE_M: (f64, f64) = (0.12, 0.35);

/// The knoll's albedo targets (sRGB): the top's trampled rust soil and the
/// cut face's eroded chert. The rendered top and face measure (190,129,75) and
/// (154,120,74) in the April afternoon light [P:spencer]; these are the
/// colours that render near them under the frame's light (tuned on the
/// headland view; the flora pass took the measured colours as the albedo).
pub const KNOLL_TOP_SRGB: [u8; 3] = [200, 118, 52];
pub const KNOLL_FACE_SRGB: [u8; 3] = [165, 110, 55];

/// The cut face's gullies (game m): spacing across the slope, depth, and the
/// wander of their lines. A V-gully every 6-9 m, 0.3-0.8 m deep, on the face
/// of a knoll this size (estimate from [P:spencer]: the face's dark streaks);
/// the 2 m relief draws them, the shader draws the rills between.
pub const GULLY_SPACING_M: f64 = 7.5;
pub const GULLY_DEPTH_M: f64 = 1.0;

/// The gullies' depth at (x, z) (game m, negative: down) on a face whose
/// downslope direction is `down` (unit, x and z), `face` its weight (0-1).
pub fn gully_m(x: f64, z: f64, down: [f64; 2], face: f64) -> f64 {
    if face <= 0.0 {
        return 0.0;
    }
    let along = x * down[0] + z * down[1];
    let across = -x * down[1] + z * down[0];
    // the lines wander and branch: the across coordinate warped by a field along them.
    let w = (crate::vegetation::field(along, across, 9.0, 701) - 0.5) * GULLY_SPACING_M * 0.9;
    let c = (across + w) / GULLY_SPACING_M;
    let v = 1.0 - (2.0 * (c - c.floor()) - 1.0).abs(); // 0 on the interfluve, 1 in the gully line
    let depth = GULLY_DEPTH_M * (0.45 + 0.55 * crate::vegetation::field(x, z, 11.0, 702));
    -depth * v * v * face
}

// ------------------------------------------------------------------ the Baker bluff

/// The bluff rock's albedo (sRGB): serpentinite blue-green-grey, the guide's
/// fresh (90-130, 110-140, 100-130) toward its low end and greyed by the
/// sheared matrix's weathering (estimate, 4.2).
pub const BLUFF_ROCK_SRGB: [u8; 3] = [78, 88, 82];
/// The weathered rust staining (buff to rusty brown, 4.2; INFERRED value).
pub const BLUFF_RUST_SRGB: [u8; 3] = [104, 72, 50];
/// The share of the face stained rust (a 12 m field over this ramp; INFERRED).
pub const BLUFF_RUST: (f64, f64) = (0.35, 0.6);
/// A bluff face: steeper than this ramp (degrees): "faces steeper than about 40" on
/// the ground, which the 8 m grid draws 25-44 degrees where the photograph shows
/// the rock face, and 20-30 on its foot and shoulders, which the photograph shows
/// as rock and grass too (probed along the Baker view's rays; the grid's cells
/// smooth it): a departure from the brief's 40, for the grid.
pub const BLUFF_FACE_DEG: (f64, f64) = (20.0, 30.0);
/// The wet band over the sea (game m of height over mean sea level), darker by
/// [`BLUFF_WET_RATIO`] (the wet rock's measured albedo ratio, rock_draw.wgsl's
/// `GC_WET_RATIO`).
pub const BLUFF_WET_M: (f64, f64) = (1.5, 4.5);
pub const BLUFF_WET_RATIO: f64 = 0.55;
/// The bluff's grass on its ledges: the grass share on a face (the guide's
/// 20-50 % plant cover over rock, its low end where it is steepest).
pub const BLUFF_GRASS: f64 = 0.25;

/// How much (x, z) at slope `slope_deg` (degrees) is the bluff's rock face, 0-1:
/// inside the serpentinite shore's box ([`crate::flora::BOULDER_BOX`]) and steep.
pub fn bluff_face(lon: f64, lat: f64, slope_deg: f64) -> f64 {
    let b = crate::flora::BOULDER_BOX;
    if lon < b[0] || lon > b[2] || lat < b[1] || lat > b[3] {
        return 0.0;
    }
    smoothstep(BLUFF_FACE_DEG.0, BLUFF_FACE_DEG.1, slope_deg)
}

/// The bluff face's albedo (linear) at (x, z) (game m) and height `h` over the
/// sea: the rock, rust-stained by a field, darker in the wet band.
pub fn bluff_albedo(x: f64, z: f64, h: f64) -> [f64; 3] {
    let lin = |c: [u8; 3]| c.map(|v| crate::vegetation::srgb_to_linear(v) as f64);
    let (rock, rust) = (lin(BLUFF_ROCK_SRGB), lin(BLUFF_RUST_SRGB));
    let t = smoothstep(BLUFF_RUST.0, BLUFF_RUST.1, crate::vegetation::warped(x, z, 12.0, 711));
    let wet = 1.0 - (1.0 - BLUFF_WET_RATIO) * (1.0 - smoothstep(BLUFF_WET_M.0, BLUFF_WET_M.1, h));
    let v = 0.85 + 0.3 * crate::vegetation::field(x, z, 5.0, 712);
    [0, 1, 2].map(|i| (rock[i] + (rust[i] - rock[i]) * t) * wet * v)
}

/// The hill rock on a bluff face: the chance a 5 m lattice point carries a
/// block, and the blocks' size (real m): broken slabs 1.5-4 m that overlap into
/// a face (INFERRED from [P:baker-sunset]: the bluff reads as a continuous
/// fractured face), tabular (height over width) and tilted with the face.
pub const BLUFF_BLOCK_CHANCE: f64 = 0.85;
pub const BLUFF_BLOCK_M: (f64, f64) = (1.5, 4.0);
pub const BLUFF_BLOCK_TALL: (f32, f32) = (0.35, 0.6);

// ------------------------------------------------------------------ boulders

/// The beach boulders' dry albedo (sRGB): dark grey-green serpentinite and
/// greenstone, luminance about 0.04 linear (the guide's base albedo estimate,
/// (40-70, 45-70, 40-60), toward its low end; the photo's (35,24,15) in the
/// surf and (61,39,15) in the foreground under the sunset).
pub const BOULDER_DRY_SRGB: [u8; 3] = [50, 54, 48];
/// Wet below the swash line: the albedo ratio and the film's roughness
/// (rock_draw.wgsl `GC_WET_RATIO`, `GC_WET_ROUGH`), the band (drawn m of the
/// shore field's signed distance, positive over the sea) from the swash's top
/// down, where a boulder is wet through.
pub const BOULDER_WET_BAND_M: (f64, f64) = (-8.0, 2.0);

/// The chance a 4 m lattice point carries a boulder: clusters on a 14 m field
/// (only its upper part, so most of the beach is clear between them), thicker
/// toward and in the surf (`sd`: the signed distance, positive over the sea)
/// (INFERRED from [P:baker-sunset]).
pub fn boulder_chance(f: f64, sd: f64) -> f64 {
    let toward_sea = smoothstep(-25.0, 5.0, sd);
    0.015 + 0.95 * smoothstep(0.58, 0.8, f) * (0.25 + 0.75 * toward_sea)
}

/// How wet a boulder is at the shore's signed distance `sd`: 1 in the surf, 0
/// above the swash band.
pub fn boulder_wet(sd: f64) -> f32 {
    smoothstep(BOULDER_WET_BAND_M.0, BOULDER_WET_BAND_M.1, sd) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switches_roll_up() {
        let a = |v: &[&str]| Flora2::from_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&[]), Flora2::ALL);
        assert_eq!(a(&["--no-flora2"]), Flora2::NONE);
        assert_eq!(a(&["--no-knoll-ground"]), Flora2 { knoll: false, ..Flora2::ALL });
        assert_eq!(a(&["--no-boulder-surf"]), Flora2 { boulders: false, ..Flora2::ALL });
        assert_eq!(flora2(), Flora2::NONE, "the unit tests run with the pass off");
    }

    #[test]
    fn the_knoll_is_convex_and_not_green() {
        // the knoll's face (probed: 45 degrees, 5 m over its ring, no green) and top
        let (t, f) = knoll(45.0, -5.0, 1.0, 0.0);
        assert!(f > 0.95 && t < 0.05);
        let (t, f) = knoll(10.0, -3.5, 1.0, 0.05);
        assert!(t > 0.95 && f < 0.05);
        // a shoulder (1 m over its ring), a hollow, a platform's edge (convex at 12 m,
        // not at 25), green scrub: not the knoll
        assert_eq!(knoll(30.0, -1.0, 1.0, 0.0), (0.0, 0.0));
        assert_eq!(knoll(45.0, 2.0, 1.0, 0.0), (0.0, 0.0));
        assert_eq!(knoll(15.0, -3.5, smoothstep(KNOLL_CONVEX25_M.0, KNOLL_CONVEX25_M.1, 6.0), 0.0), (0.0, 0.0));
        let (t, f) = knoll(45.0, -5.0, 1.0, 0.6);
        assert!(t + f < 1e-9);
    }

    #[test]
    fn the_gullies_cut_down_only_on_the_face() {
        assert_eq!(gully_m(3.0, 4.0, [1.0, 0.0], 0.0), 0.0);
        let mut lo = 0.0f64;
        for k in 0..200 {
            let g = gully_m(0.0, k as f64 * 0.37, [1.0, 0.0], 1.0);
            assert!(g <= 0.0 && g >= -GULLY_DEPTH_M - 1e-9);
            lo = lo.min(g);
        }
        assert!(lo < -0.3, "the gullies reach {lo} m");
    }

    #[test]
    fn the_bluff_rock_is_dark_and_wet_at_the_waterline() {
        let y = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let dry = bluff_albedo(10.0, 20.0, 30.0);
        let wet = bluff_albedo(10.0, 20.0, 0.5);
        assert!(y(dry) > 0.04 && y(dry) < 0.15, "dry {dry:?}");
        assert!((y(wet) / y(dry) - BLUFF_WET_RATIO).abs() < 1e-6);
        assert_eq!(bluff_face(-122.47, 37.80, 60.0), 0.0, "east of the box");
        assert!(bluff_face(-122.48, 37.80, 50.0) > 0.99);
    }

    #[test]
    fn the_boulders_cluster_toward_the_surf() {
        assert!(boulder_chance(0.3, 0.0) < 0.02, "between the clusters");
        assert!(boulder_chance(0.9, 5.0) > 0.9);
        assert!(boulder_chance(0.9, -30.0) < 0.3);
        assert_eq!(boulder_wet(10.0), 1.0);
        assert_eq!(boulder_wet(-20.0), 0.0);
    }
}
