//! the open ground between buildings and on the hills, coloured from
//! the measured ground-green table (`native/assets/ground-green-v1.bin.gz`,
//! 16 m cells of the surveyed plan, from USDA NAIP aerial imagery: the
//! vegetation share of each cell's open ground and the footprints' share).
//!
//! The terrain's vertex colour decides its tone and, near, which CC0 ground
//! scans draw (`cc0.wgsl` `cc0_ground_sets`). Before this the city's open
//! ground took WorldCover's "built-up" grey-beige, which the ground scans
//! turned to gravel and park dirt: tan between every house row; the hills
//! took a park lawn green. Here, at each terrain vertex (8 m apart):
//!
//! * `green` (`EO_GROUND_GREEN=off`, `--no-ground-green`): in the built city
//!   (WorldCover built-up, or a cell with footprints), a vertex is yard
//!   vegetation (garden green; under WorldCover tree cover, dark canopy) or
//!   paving (cool grey concrete or asphalt, which the ground scans draw as
//!   worn concrete and asphalt), drawn per vertex against a stable hash so
//!   the share of green vertices is the measured share: a patchwork, not a
//!   uniform olive.
//! * `hills` (`EO_GROUND_HILLS=off`, `--no-ground-hills`): unbuilt ground and
//!   the vegetated parks run from dry summer grass (straw, the withered-grass
//!   scan) through dark scrub to groves (or the park's own lawn where the
//!   imagery says it is green: irrigated lawns stay green) by the measured
//!   vegetation share, jittered per vertex.
//! * `contact` (`EO_GROUND_CONTACT=off`, `--no-ground-contact`): open ground
//!   among buildings darkens with the footprints' share of its cell (rear
//!   yards and light wells hemmed in by walls: up to 40 %), an albedo stand-in
//!   for the occlusion the far ground cannot get from the light.
//!
//! Never touched: the beaches, the waterline and anything within
//! `SHORE_KEEP_M` of a shoreline or below `SEA_KEEP_M` over the sea (the
//! surf and the sand have their own passes), mapped bare, sand, court, play and paved
//! surfaces, inland water, the land beyond the city (Marin). No table, or all
//! three switches off: the ground as before. With `--bare-ground` (`set_bare`)
//! the shore band is drawn as inland, and WorldCover's bare ground and the
//! revetments at the shore are marked for the city shader (`bare_ground`).

use eo_data::roof_colours::{GreenGrid, native_assets, srgb_to_linear};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

pub type Linear = [f64; 3];

/// Ground within this distance of a shoreline keeps its old colour.
pub const SHORE_KEEP_M: f64 = 40.0;
/// Ground less than this over the sea keeps its old colour.
pub const SEA_KEEP_M: f64 = 1.0;
/// The darkest the footprints' share makes open ground (x 1 - CONTACT_MAX x built).
pub const CONTACT_MAX: f64 = 0.4;

static COVER: AtomicBool = AtomicBool::new(false);

/// `--ground-cover` (eo-render look::NATURE_COVER, off by default): the hills'
/// tones vary over a smooth field instead of vertex by vertex, and the city
/// shader (landmarks_surface.wgsl `nt_cover`) draws the mosaic, the crowns and
/// the chert at the pixel. Set before the terrain is built.
pub fn set_cover(on: bool) {
    COVER.store(on, Ordering::Relaxed);
}

/// Whether `--ground-cover` is on (`set_cover`).
pub fn cover() -> bool {
    COVER.load(Ordering::Relaxed)
}

static BARE: AtomicBool = AtomicBool::new(false);

/// `--bare-ground` (eo-render look::NATURE_BARE, off by default): the ground
/// the land-cover classes left in their flat placeholder tones (WorldCover's
/// bare or sparsely vegetated ground anywhere, every class in the shore band)
/// is decided here (`bare_ground`, and the shore band's built and vegetated
/// ground as inland in `open_ground`), and the city shader
/// (landmarks_surface.wgsl `nt_bare`) draws the bare ground and the
/// revetments. Set before the terrain is built.
pub fn set_bare(on: bool) {
    BARE.store(on, Ordering::Relaxed);
}

/// Whether `--bare-ground` is on (`set_bare`).
pub fn bare() -> bool {
    BARE.load(Ordering::Relaxed)
}

/// `--bare-ground`: the alpha byte of a vertex the city shader draws as bare
/// ground or as a revetment's rock (landmarks_surface.wgsl `nt_bare`). The
/// beach sand's alpha carries its shore distance (0-253); every other vertex has 255.
pub const BARE_ALPHA: u8 = 254;
/// `--bare-ground`: a revetment's rock lies within this distance of the
/// shoreline (m). The bay's riprap slopes (1.5-2 run to 1 rise) climb about 4 m
/// from below the low water to a crest a few metres wide.
pub const REVETMENT_M: f64 = 9.0;
/// `--bare-ground`: and at most this high over the sea (m)...
pub const REVETMENT_TOP_M: f64 = 5.0;
/// `--bare-ground`: ...from its foot just under the waterline (the rock carries on under the water).
pub const REVETMENT_FOOT_M: f64 = -0.3;
/// `--bare-ground`: and only on a bank at least this steep (rise over run; a
/// riprap slope is 0.5-0.67, smoothed by the 12 m difference), or at the waterline itself: flat made ground
/// behind a seawall is bare ground, not rock.
pub const REVETMENT_SLOPE: f64 = 0.35;
/// `--bare-ground`: the measured vegetation share over which bare ground is at
/// its weediest (WorldCover's class keeps it under about 10 %; the 16 m cell
/// counts the street and the kerb's weeds with it).
pub const BARE_WEEDY_SHARE: f64 = 0.35;
// Bare ground (sRGB): dry fines and gravel with a few weeds, the same weedier;
// the revetment's quarry rock. The shader reads which and how weedy back from
// the tone (r/b: rock under 1.35; g/r: 0.71 bare to 0.93 weedy) and draws its own albedo.
const BARE_SOIL: [u8; 3] = [138, 118, 94];
const BARE_WEEDY: [u8; 3] = [120, 116, 82];
const RIPRAP: [u8; 3] = [112, 110, 104];
/// WorldCover's bare or sparse vegetation (never over 10 % vegetated in a year).
const BARE_CLASS: u8 = 60;

/// (`--bare-ground`) The tone of a terrain vertex the city shader draws as
/// bare ground or as a revetment's rock (linear; packed with `BARE_ALPHA`),
/// or None (and always None with the switch off). The rock: any ground left
/// unmapped within `REVETMENT_M` of the shoreline and under `REVETMENT_TOP_M`
/// over the sea (San Francisco's bay shore is mostly riprap where no seawall
/// or quay stands). Bare ground: WorldCover's bare or sparse class, and in the
/// shore band (`SHORE_KEEP_M`) its water and wetland pixels standing on land
/// (the 10 m classifier's mixed pixels at the waterline), weedier where the
/// measured vegetation share is higher; and the greens mapped as bare earth
/// (bare pitches and courts of decomposed granite, construction sites,
/// brownfields), as raked and weedless as their tone allows. `park`: the tone
/// name of a green, inland water or a beach mapped there (the others keep
/// theirs); `slope`: the ground's gradient (rise over run).
pub fn bare_ground(x: f64, z: f64, class: u8, park: Option<&str>, shore_m: Option<f64>, above_sea_m: Option<f64>, slope: f64) -> Option<Linear> {
    bare_ground_on(bare(), x, z, class, park, shore_m, above_sea_m, slope)
}

#[allow(clippy::too_many_arguments)]
fn bare_ground_on(on: bool, x: f64, z: f64, class: u8, park: Option<&str>, shore_m: Option<f64>, above_sea_m: Option<f64>, slope: f64) -> Option<Linear> {
    if !on {
        return None;
    }
    let h = above_sea_m?;
    if park == Some("bareEarth") && h >= SEA_KEEP_M {
        return Some(hex(BARE_SOIL));
    }
    if park.is_some() {
        return None;
    }
    if shore_m.is_some_and(|s| s < REVETMENT_M) && (REVETMENT_FOOT_M..REVETMENT_TOP_M).contains(&h) && (slope >= REVETMENT_SLOPE || h < SEA_KEEP_M) {
        return Some(hex(RIPRAP));
    }
    if h < SEA_KEEP_M {
        return None;
    }
    let band = shore_m.is_some_and(|s| s < SHORE_KEEP_M);
    if class != BARE_CLASS && !(band && matches!(class, 0 | 80 | 90 | 95)) {
        return None;
    }
    let share = grid().and_then(|g| {
        let s = eo_data::groundscale::scale();
        sample(g, x / s, z / s, false)
    });
    Some(mix(hex(BARE_SOIL), hex(BARE_WEEDY), smoothstep(0.02, BARE_WEEDY_SHARE, share.unwrap_or(0.05))))
}

/// Value noise in [0, 1) over cells of `cell` m (smoothstep between the corners' hashes).
pub fn value_noise(x: f64, z: f64, cell: f64, salt: u32) -> f64 {
    let (fx, fz) = (x / cell, z / cell);
    let (ix, iz) = (fx.floor(), fz.floor());
    let (tx, tz) = (fx - ix, fz - iz);
    let (ux, uz) = (tx * tx * (3.0 - 2.0 * tx), tz * tz * (3.0 - 2.0 * tz));
    // corner hashes on a lattice a metre apart (hash01 rounds to the metre)
    let h = |i: f64, j: f64| hash01(i, j, salt);
    let a = h(ix, iz) + (h(ix + 1.0, iz) - h(ix, iz)) * ux;
    let b = h(ix, iz + 1.0) + (h(ix + 1.0, iz + 1.0) - h(ix, iz + 1.0)) * ux;
    a + (b - a) * uz
}

/// A switch: `EO_GROUND_<NAME>=off` (or 0) turns it off.
pub fn on(name: &str) -> bool {
    !matches!(std::env::var(format!("EO_GROUND_{}", name.to_ascii_uppercase())).as_deref(), Ok("off") | Ok("0"))
}

/// The table, read once (None: no file, or unreadable).
pub fn grid() -> Option<&'static GreenGrid> {
    static G: OnceLock<Option<GreenGrid>> = OnceLock::new();
    G.get_or_init(|| {
        let p = native_assets().join("ground-green-v1.bin.gz");
        match eo_data::read_maybe_gzip(&p).and_then(|b| GreenGrid::decode(&b)) {
            Ok(g) => {
                eprintln!("ground2: ground-green table {} x {} cells of {} m", g.nx, g.nz, g.cell);
                Some(g)
            }
            Err(e) => {
                eprintln!("ground2: no ground-green table ({e}); the ground as before");
                None
            }
        }
    })
    .as_ref()
}

/// Bilinear sample between cell centres of one layer (`built`: false = green),
/// at a surveyed point, cells without data left out (None: none has data).
pub fn sample(g: &GreenGrid, x: f64, z: f64, built: bool) -> Option<f64> {
    let fx = (x - g.x0 as f64) / g.cell as f64 - 0.5;
    let fz = (z - g.z0 as f64) / g.cell as f64 - 0.5;
    let (i0, j0) = (fx.floor(), fz.floor());
    let (tx, tz) = (fx - i0, fz - j0);
    let (mut acc, mut w) = (0.0, 0.0);
    for (di, dj, wt) in [(0, 0, (1.0 - tx) * (1.0 - tz)), (1, 0, tx * (1.0 - tz)), (0, 1, (1.0 - tx) * tz), (1, 1, tx * tz)] {
        let (i, j) = (i0 as i64 + di, j0 as i64 + dj);
        if i < 0 || j < 0 || i >= g.nx as i64 || j >= g.nz as i64 || wt <= 0.0 {
            continue;
        }
        let k = j as usize * g.nx as usize + i as usize;
        let v = if built { g.built[k] } else { g.green[k] };
        if v == 255 || (built && g.green[k] == 255 && v == 0) {
            continue;
        }
        acc += wt * v as f64 / 250.0;
        w += wt;
    }
    (w > 1e-6).then(|| (acc / w).clamp(0.0, 1.0))
}

fn hex(c: [u8; 3]) -> Linear {
    c.map(|v| srgb_to_linear(v as f32 / 255.0) as f64)
}

fn mix(a: Linear, b: Linear, t: f64) -> Linear {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A stable hash of a point (to the metre) in [0, 1).
pub fn hash01(x: f64, z: f64, salt: u32) -> f64 {
    let mut h = (x.round() as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (z.round() as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (salt as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Whether a vertex is drawn as the measured share says: P(true) = share.
pub fn dithered(share: f64, h: f64) -> f64 {
    smoothstep(h - 0.08, h + 0.08, share)
}

// Tones (sRGB). Each lands in the CC0 ground set the shader picks by colour:
// paving a neutral grey with green equal to red (worn concrete; no mapped
// tone has that), yard green bright enough for the lawn scans, canopy and
// scrub dark (forest floor), dry grass with red close to green (withered
// grass), a muted straw rather than a yellow.
const ASPHALT: [u8; 3] = [90, 90, 87];
const CONCRETE: [u8; 3] = [122, 122, 117];
const YARD: [u8; 3] = [74, 92, 56];
const CANOPY: [u8; 3] = [46, 60, 38];
const DRY_A: [u8; 3] = [146, 140, 108];
const DRY_B: [u8; 3] = [160, 154, 120];
const SCRUB: [u8; 3] = [60, 68, 44];
const GROVE: [u8; 3] = [40, 54, 34];

/// WorldCover's built-up class.
const BUILT_UP: u8 = 50;
/// WorldCover's tree cover.
const TREES: u8 = 10;

/// The measured green share counts the streets as open ground; their own
/// surfaces draw over the terrain, so the yards between them are greener than
/// the cell: about 45 % of a residential cell's open ground is street (a
/// stated estimate, not measured per cell), so the share is scaled by 1.8.
pub const YARD_SHARE_SCALE: f64 = 1.8;
/// A mapped lawn on flat ground stays the lawn it is mapped as (irrigated
/// meadows, playing fields, turf, which read dry in the imagery); on slopes
/// steeper than this it runs dry to green by the measured share.
pub const LAWN_FLAT: f64 = 0.06;
pub const LAWN_SLOPE: f64 = 0.18;

/// The surveyed vegetation share a vegetated tone means where the table has
/// no data (WorldCover class).
fn class_green(class: u8) -> f64 {
    match class {
        10 => 0.75,
        20 => 0.45,
        30 => 0.15,
        _ => 0.3,
    }
}

/// Dry grass to scrub to `green` by the vegetation share (per vertex jitter;
/// with `--ground-cover` a smooth field of 40 and 14 m, whose spread is stretched
/// to the jitter's, so the terrain's triangles do not show).
pub fn hill_tone(share: f64, green: Linear, x: f64, z: f64) -> Linear {
    let (h, k) = if cover() {
        let f = value_noise(x, z, 40.0, 7) * 0.65 + value_noise(x + 17.0, z - 29.0, 14.0, 13) * 0.35;
        ((0.5 + (f - 0.5) * 2.2).clamp(0.0, 1.0), value_noise(x, z, 60.0, 11))
    } else {
        (hash01(x, z, 7), hash01(x, z, 11))
    };
    let level = share * 2.2 + (h - 0.5) * 0.9;
    let dry = mix(hex(DRY_A), hex(DRY_B), k);
    if level < 1.0 { mix(dry, hex(SCRUB), smoothstep(0.4, 0.75, level)) } else { mix(hex(SCRUB), green, smoothstep(1.15, 1.5, level)) }
}

/// The built city's open ground: yard green or paving by the measured share,
/// darkened by the footprints round it.
pub fn yard_tone(green_share: f64, built: f64, class: u8, x: f64, z: f64, contact: bool) -> Linear {
    // Paving: lot by lot (48 m value noise), not vertex by vertex.
    let paving = mix(hex(ASPHALT), hex(CONCRETE), smoothstep(0.25, 0.75, eo_landmarks::ground_cover::ground_mottle(x + 911.0, z - 377.0)));
    let veg = if class == TREES { hex(CANOPY) } else { hex(YARD) };
    let c = mix(paving, veg, dithered((green_share * YARD_SHARE_SCALE).min(1.0), hash01(x, z, 5)));
    let k = if contact { 1.0 - CONTACT_MAX * built.clamp(0.0, 1.0) } else { 1.0 };
    c.map(|v| v * k)
}

/// Open-ground switches shared by the workers building one scene.
pub struct OpenGroundSettings {
    green: bool,
    hills: bool,
    contact: bool,
    band: bool,
}

impl OpenGroundSettings {
    pub fn from_env() -> Self {
        Self { green: on("green"), hills: on("hills"), contact: on("contact"), band: bare() }
    }

    /// The open ground's colour at a drawn terrain vertex (linear), or None to
    /// keep the colour it had. `park`: the mapped green's tone name and colour
    /// there (`GroundCover::tone_name_at` / `tone_at`); `class`: WorldCover;
    /// `shore_m`: distance to the nearest shoreline; `above_sea_m`: ground height
    /// over the sea; `slope`: the ground's gradient (rise over run).
    #[allow(clippy::too_many_arguments)]
    pub fn sample(&self, x: f64, z: f64, class: u8, park: Option<(&str, Linear)>, shore_m: Option<f64>, above_sea_m: Option<f64>, slope: f64) -> Option<Linear> {
        if !(self.green || self.hills || self.contact) {
            return None;
        }
        // (`--bare-ground`) the shore band's ground as inland's (its bare ground and
        // revetments are `bare_ground`'s): no longer the land-cover's flat tones.
        if (shore_m.is_some_and(|s| s < SHORE_KEEP_M) && !self.band) || above_sea_m.is_none_or(|h| h < SEA_KEEP_M) {
            return None;
        }
        let g = grid()?;
        let s = eo_data::groundscale::scale();
        let (sx, sz) = (x / s, z / s);
        let share = sample(g, sx, sz, false);
        let built = sample(g, sx, sz, true).unwrap_or(0.0);
        match park {
            Some((name, tone)) => {
                if !self.hills {
                    return None;
                }
                let (green, keep) = match name {
                    "grass" => (tone, 1.0 - smoothstep(LAWN_FLAT, LAWN_SLOPE, slope)),
                    "understory" | "wildScrub" => (hex(GROVE), 0.0),
                    "wildGrass" | "dryGrass" => (hex(SCRUB), 0.0),
                    _ => return None, // sand, bare, rock, lids, courts, play, water, paving: as mapped
                };
                Some(mix(hill_tone(share.unwrap_or(class_green(class)), green, x, z), tone, keep))
            }
            None => {
                if class == BUILT_UP || built > 0.08 {
                    if !(self.green || self.contact) {
                        return None;
                    }
                    if !self.green {
                        // Contact alone: the old built-up ground, darkened.
                        let old = hex([150, 146, 136]);
                        return Some(old.map(|v| v * (1.0 - CONTACT_MAX * built)));
                    }
                    let share = share?;
                    Some(yard_tone(share, built, class, x, z, self.contact))
                } else if matches!(class, 10 | 20 | 30 | 100) && self.hills {
                    let green = if class == TREES { hex(GROVE) } else { hex(SCRUB) };
                    Some(hill_tone(share.unwrap_or(class_green(class)), green, x, z))
                } else {
                    None
                }
            }
        }
    }
}

/// Sample a single point using the current ground switches.
pub fn open_ground(x: f64, z: f64, class: u8, park: Option<(&str, Linear)>, shore_m: Option<f64>, above_sea_m: Option<f64>, slope: f64) -> Option<Linear> {
    open_ground_in(bare(), x, z, class, park, shore_m, above_sea_m, slope)
}

/// `open_ground`, with the shore band drawn as inland (`band`: `--bare-ground`) or left alone.
#[allow(clippy::too_many_arguments)]
fn open_ground_in(band: bool, x: f64, z: f64, class: u8, park: Option<(&str, Linear)>, shore_m: Option<f64>, above_sea_m: Option<f64>, slope: f64) -> Option<Linear> {
    let mut settings = OpenGroundSettings::from_env();
    settings.band = band;
    settings.sample(x, z, class, park, shore_m, above_sea_m, slope)
}

/// The far ground's paving branch in `cc0_ground_sets` (WGSL constant).
pub fn wgsl_constants() -> String {
    format!("const CC0_GROUND2_PAVING: bool = {};\n", on("green"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(c: Linear) -> f64 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    /// cc0.wgsl `cc0_ground_sets`, mirrored: 0 lawn, 1 understory, 2 dry
    /// grass, 3 dirt, 4 paving (ground2), 5 hard yards.
    fn set_of(c: Linear) -> u32 {
        let l = luma(c);
        let warm = c[0] / c[2].max(1e-4);
        if c[1] > c[0] * 1.08 && c[1] > c[2] * 1.2 {
            return if l > 0.055 { 0 } else { 1 };
        }
        if (c[0] - c[1]).abs() < 0.12 * c[1] && warm > 1.25 {
            return 2;
        }
        if c[0] > c[1] && c[1] > c[2] && warm > 1.25 {
            return 3;
        }
        if c[1] >= c[0] * 0.999 && c[1] <= c[0] * 1.06 && c[0] >= c[2] && c[0] <= c[2] * 1.2 {
            return 4;
        }
        5
    }

    #[test]
    fn each_tone_lands_in_its_ground_scan() {
        assert_eq!(set_of(hex(YARD)), 0);
        assert_eq!(set_of(hex(CANOPY)), 1);
        assert_eq!(set_of(hex(SCRUB)), 1);
        assert_eq!(set_of(hex(GROVE)), 1);
        assert_eq!(set_of(hex(DRY_A)), 2);
        assert_eq!(set_of(hex(DRY_B)), 2);
        assert_eq!(set_of(hex(ASPHALT)), 4);
        assert_eq!(set_of(hex(CONCRETE)), 4);
        // Darkened by contact: the same set.
        assert_eq!(set_of(hex(CONCRETE).map(|v| v * 0.6)), 4);
        // The old built-up grey-beige, mapped hard yards, bare rock and
        // reservoir lids are not caught by the paving branch.
        for t in [[150, 146, 136], [0x6a, 0x67, 0x5f], [0x7b, 0x7a, 0x73], [0x86, 0x85, 0x7f], [0x56, 0x5c, 0x64]] {
            assert_ne!(set_of(hex(t)), 4, "{t:?}");
        }
    }

    #[test]
    fn the_green_vertices_follow_the_measured_share() {
        for share in [0.0, 0.1, 0.176, 0.5, 0.9] {
            let mut green = 0;
            let n = 40_000;
            for i in 0..n {
                let (x, z) = ((i % 200) as f64 * 8.0, (i / 200) as f64 * 8.0);
                if dithered(share, hash01(x, z, 5)) > 0.5 {
                    green += 1;
                }
            }
            let got = green as f64 / n as f64;
            assert!((got - share).abs() < 0.02, "share {share}: {got}");
        }
    }

    #[test]
    fn contact_darkens_with_the_footprints() {
        let open = yard_tone(0.0, 0.0, 50, 4.0, 4.0, true);
        let hemmed = yard_tone(0.0, 0.8, 50, 4.0, 4.0, true);
        assert!((luma(hemmed) / luma(open) - (1.0 - CONTACT_MAX * 0.8)).abs() < 1e-9);
        assert_eq!(yard_tone(0.0, 0.8, 50, 4.0, 4.0, false), open);
    }

    #[test]
    fn hills_run_dry_to_green_with_the_share() {
        let mean = |share: f64| {
            let n = 4000;
            let mut g = 0.0;
            for i in 0..n {
                let c = hill_tone(share, hex([86, 118, 63]), (i % 64) as f64 * 8.0, (i / 64) as f64 * 8.0);
                g += c[1] / (c[0] + c[1] + c[2]);
            }
            g / n as f64
        };
        let (dry, mid, wet) = (mean(0.05), mean(0.35), mean(0.9));
        assert!(dry < mid && mid < wet, "{dry} {mid} {wet}");
        // Dry hills are mostly straw (the withered-grass scan).
        let straw = (0..2000).filter(|i| set_of(hill_tone(0.05, hex(GROVE), (*i % 50) as f64 * 8.0, (*i / 50) as f64 * 8.0)) == 2).count();
        assert!(straw > 1500, "{straw} of 2000 straw");
    }

    #[test]
    fn the_cover_field_is_smooth_and_spread_like_the_jitter() {
        // neighbouring vertices (8 m) differ little; the field still spans most of [0, 1)
        let (mut lo, mut hi, mut step) = (1.0f64, 0.0f64, 0.0f64);
        for i in 0..4000 {
            let (x, z) = ((i % 80) as f64 * 8.0, (i / 80) as f64 * 8.0);
            let a = value_noise(x, z, 40.0, 7);
            lo = lo.min(a);
            hi = hi.max(a);
            step = step.max((value_noise(x + 8.0, z, 40.0, 7) - a).abs());
        }
        assert!(lo < 0.15 && hi > 0.85, "{lo} {hi}");
        assert!(step < 0.35, "{step}");
        assert!(!cover(), "off unless set");
    }

    #[test]
    fn the_sample_is_bilinear_and_skips_no_data() {
        let g = GreenGrid { x0: 0.0, z0: 0.0, cell: 16.0, nx: 2, nz: 1, green: vec![0, 250], built: vec![0, 125] };
        assert!((sample(&g, 8.0, 8.0, false).unwrap() - 0.0).abs() < 1e-9);
        assert!((sample(&g, 16.0, 8.0, false).unwrap() - 0.5).abs() < 1e-9);
        assert!((sample(&g, 24.0, 8.0, true).unwrap() - 0.5).abs() < 1e-9);
        let n = GreenGrid { x0: 0.0, z0: 0.0, cell: 16.0, nx: 2, nz: 1, green: vec![255, 250], built: vec![0, 0] };
        assert!((sample(&n, 16.0, 8.0, false).unwrap() - 1.0).abs() < 1e-9);
        assert!(sample(&n, 4.0, 8.0, false).is_none(), "only a cell without data in reach");
        assert!(sample(&n, -100.0, 8.0, false).is_none());
    }

    #[test]
    fn the_shore_and_the_sea_are_left_alone() {
        assert!(open_ground(0.0, 0.0, 50, None, Some(10.0), Some(20.0), 0.0).is_none());
        assert!(open_ground(0.0, 0.0, 50, None, Some(500.0), Some(0.5), 0.0).is_none());
        assert!(open_ground(0.0, 0.0, 50, None, Some(500.0), None, 0.0).is_none());
    }

    /// landmarks_surface.wgsl `nt_bare_weight`, mirrored: (rock, weedy) from a marked tone.
    fn bare_kind(c: Linear) -> (bool, f64) {
        let rb = c[0] / c[2].max(1e-4);
        let gr = c[1] / c[0].max(1e-4);
        (rb < 1.3 && gr > 0.9, ((gr - 0.71) / 0.22).clamp(0.0, 1.0))
    }

    #[test]
    fn bare_ground_marks_the_bare_class_and_the_shore_rock() {
        let bs = |class: u8, mapped: bool, shore: f64, h: f64, slope: f64| bare_ground_on(true, 0.0, 0.0, class, mapped.then_some("grass"), Some(shore), Some(h), slope);
        let b = |class: u8, mapped: bool, shore: f64, h: f64| bs(class, mapped, shore, h, 0.0);
        assert!(!bare(), "off unless set");
        assert!(bare_ground(0.0, 0.0, BARE_CLASS, None, Some(500.0), Some(20.0), 0.0).is_none(), "nothing while off");
        // the revetment: a bank near the shoreline, from its foot under the waterline to its crest, whatever the class
        let rock = bs(50, false, 4.0, 2.0, 0.5).expect("rock on the shore's bank");
        assert!(bare_kind(rock).0);
        assert!(!bare_kind(b(BARE_CLASS, false, 4.0, 2.0).expect("flat made ground behind a seawall")).0, "is bare ground, not rock");
        assert!(b(80, false, 4.0, 0.2).is_some_and(|c| bare_kind(c).0), "the rock carries on to the waterline");
        assert!(bs(50, false, 4.0, -1.0, 0.5).is_none(), "not on the seabed");
        assert!(bs(50, false, 4.0, REVETMENT_TOP_M + 1.0, 0.5).is_none(), "a high bank is not a revetment");
        // a green mapped as bare earth is bare ground, unweeded
        let pitch = bare_ground_on(true, 0.0, 0.0, 50, Some("bareEarth"), Some(500.0), Some(10.0), 0.0).expect("a bare pitch");
        assert!(!bare_kind(pitch).0 && bare_kind(pitch).1 < 0.05);
        assert!(bare_ground_on(true, 0.0, 0.0, BARE_CLASS, Some("court"), Some(500.0), Some(10.0), 0.0).is_none(), "a court keeps its tone");
        // bare ground: the bare class anywhere, the water classes on land in the shore band only
        let bare = b(BARE_CLASS, false, 500.0, 20.0).expect("bare ground inland");
        assert!(!bare_kind(bare).0);
        assert!(b(80, false, 20.0, 3.0).is_some(), "a water pixel on the shore band's land");
        assert!(b(80, false, 500.0, 3.0).is_none(), "inland water classes keep theirs");
        assert!(b(80, false, 20.0, 0.5).is_none(), "the waterline behind the rock keeps its own");
        assert!(b(50, false, 20.0, 3.0).is_none(), "built ground in the band is open_ground's");
        // a mapped green, water or beach keeps its tone
        assert!(b(BARE_CLASS, true, 500.0, 20.0).is_none());
        assert!(bs(50, true, 4.0, 2.0, 0.5).is_none());
        // the tones read back: soil bare, weedy weedy, rock rock
        assert!(!bare_kind(hex(BARE_SOIL)).0 && bare_kind(hex(BARE_SOIL)).1 < 0.05);
        assert!(bare_kind(hex(BARE_WEEDY)).1 > 0.95 && !bare_kind(hex(BARE_WEEDY)).0);
        assert!(bare_kind(hex(RIPRAP)).0);
        // half a bare vertex blended with paving still reads as bare, not rock
        let half = mix(hex(BARE_SOIL), hex(ASPHALT), 0.5);
        assert!(!bare_kind(half).0, "{half:?}");
        // the shore band's built ground is drawn as inland's with the switch, left alone without it
        assert!(open_ground_in(false, 0.0, 0.0, 50, None, Some(10.0), Some(5.0), 0.0).is_none());
        if let Some(g) = grid() {
            let s = eo_data::groundscale::scale();
            let (x, z) = (g.x0 as f64 + g.nx as f64 * g.cell as f64 * 0.5, g.z0 as f64 + g.nz as f64 * g.cell as f64 * 0.5);
            if sample(g, x, z, false).is_some() {
                assert!(open_ground_in(true, x * s, z * s, 50, None, Some(10.0), Some(5.0), 0.0).is_some(), "the band's built ground takes a tone with the switch");
            }
        }
    }

    #[test]
    fn a_flat_lawn_stays_the_lawn_a_slope_runs_dry() {
        let lawn = hex([0x56, 0x76, 0x3f]);
        // Where the table reaches (the city's plan); a dry share there.
        let Some(g) = grid() else { return };
        let (x, z) = (g.x0 as f64 * 0.5 + 0.0, g.z0 as f64 * 0.5);
        let flat = open_ground(x, z, 30, Some(("grass", lawn)), Some(1e4), Some(50.0), 0.0);
        if let Some(f) = flat {
            for k in 0..3 {
                assert!((f[k] - lawn[k]).abs() < 1e-12);
            }
        }
        let steep = open_ground(x, z, 30, Some(("grass", lawn)), Some(1e4), Some(50.0), 0.4);
        assert!(steep.is_some());
    }
}
