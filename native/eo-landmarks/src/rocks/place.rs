//! Where the scanned rock stands: by slope (the USGS 3DEP 1 m DEM), by
//! the mapped geologic unit, and by the NAIP 0.6 m aerial colour, all read
//! through `eo_farshore::survey` (group E1, B1, A1 of the survey set).
//!
//! * **The intertidal rock field** ([`surf_field`]): every metre of shore
//!   whose CUDEM topobathy height lies between [`FIELD_FLOOR_NAVD88_M`] and
//!   the mean high water, and whose NAIP pixel reads as dark rock; the
//!   pixels are grouped into connected rocks, each drawn as a scanned
//!   cluster or stone of its own footprint, wet and dark, with stones of the
//!   boulder size law (rocks.rs `boulder_d`) beside it. Each rock is seated
//!   against the drawn shore, not the surveyed one: its distance seaward of
//!   the CUDEM's mean-sea-level contour is laid off from the drawn
//!   waterline (the shore field's zero) along the field's gradient
//!   ([`seat_on_drawn_shore`]), so a rock in the surveyed surf stands in
//!   the drawn surf wherever the drawn shoreline lies.
//! * **The slopes** ([`slopes`]): on a 4 m lattice inside the claimed boxes,
//!   ground steeper than [`FACE_SLOPE_DEG`] on the 1 m DEM (and on the drawn
//!   ground) whose aerial colour is bare rock gets a rock face turned down
//!   its slope (the cliff dressing where it is within [`CLIFF_SHORE_M`] of
//!   the shore, an outcrop elsewhere), and on a 24 m lattice a whole scanned
//!   coastal cliff where a band of the face is bare; gentler bare ground
//!   ([`PATCH_SLOPE_DEG`]) gets scanned rock patches and stones. Scrub and
//!   grass (the aerial colour green) leave gaps.
//! * **Round 2's stones** ([`from_stones`]): the boulders and the revetment
//!   armour of rocks.rs keep their places and sizes, drawn as scans.
//!
//! The NAIP classes (sRGB bytes as delivered, INFERRED, stated not fitted:
//! read off the imagery's own sand, scrub, surf and rock at Baker Beach):
//! vegetation where the excess green 2G - R - B (Woebbecke et al. 1995)
//! passes [`EXG_VEG`]; bright foam or pale sand over luma [`BRIGHT_LUMA`];
//! warm sand where R exceeds B by [`SAND_WARM`]; teal water where B and G
//! exceed R; rock is what is left under [`ROCK_LUMA`], dark rock under
//! [`DARK_LUMA`].
//!
//! The imagery's tiles are not the grid their export asked for: the
//! service kept square degree pixels (dlat = dlon), so each 2048-row tile
//! spans 2048 x 6.8165e-6 degrees about its box's centre, not its box (a tile's
//! first 64 rows match the tile above at row 1624 in every column and row
//! of the set). [`naip_rgb`] reads a true latitude through that layout.

use super::scan::{self, BOULDER, CLIFF, COAST_ROCKS, FACE_1, FACE_2, Kind, LAND_ROCKS, Library, Piece, Role, SAND_ROCKS, Source};
use eo_farshore::survey::{bathy::Bathy, dem1m::Dem1m, naip::Naip};

/// San Francisco tide station 9414290, NOAA CO-OPS datums (epoch 1983-2001,
/// api.tidesandcurrents.noaa.gov mdapi `datums`, feet on station datum):
/// MSL 9.10, NAVD88 5.92, MHW 11.21, MLLW 5.98. Mean sea level over NAVD88:
/// 3.18 ft = 0.969 m; MHW over NAVD88 1.612 m; MLLW over NAVD88 0.018 m.
pub const MSL_OVER_NAVD88_M: f64 = (9.10 - 5.92) * 0.3048;
pub const MHW_NAVD88_M: f64 = (11.21 - 5.92) * 0.3048;
pub const MLLW_NAVD88_M: f64 = (5.98 - 5.92) * 0.3048;
/// The rock field reaches this far under the datum (NAVD88 m): a metre and a
/// half under the lowest tide's mean, where a rock still breaks the surf
/// (INFERRED).
pub const FIELD_FLOOR_NAVD88_M: f64 = MLLW_NAVD88_M - 1.5;
/// How far a rock of the field may stand over the high water itself (m, INFERRED: the
/// largest of Baker's surf rocks stand a few metres out of the water).
pub const FIELD_ROCK_TOP_M: f64 = 6.0;
/// A rock of the field stands at least this far over the mean sea level
/// (m, INFERRED: the aerial reads it as rock, so it was out of the water
/// at the flight's tide; a summer midday flight's tide is about the mean,
/// and the CUDEM's metre-scale grid smooths a rock's own top into the
/// ground round it, so its surveyed top alone would leave it awash).
pub const FIELD_EXPOSED_OVER_MSL_M: f64 = 0.3;

/// NAIP classes (see the module notes; INFERRED).
pub const EXG_VEG: i32 = 12;
pub const BRIGHT_LUMA: f64 = 0.62;
pub const SAND_WARM: i32 = 18;
pub const ROCK_LUMA: f64 = 0.58;
pub const DARK_LUMA: f64 = 0.36;

/// A face: ground steeper than this on the 1 m DEM (over 3 m) and on the
/// drawn ground (over its footprint; the drawn test keeps walls, batteries
/// and kerbs the DEM resolves off the list). INFERRED: a slope bare rock
/// holds that soil and scrub do not: about the 34-degree repose of loose
/// sand and talus (`SAND_CLIFF_DEG`), less a margin for the DEM's slope
/// taken over 3 m, which flattens a broken face's steps.
pub const FACE_SLOPE_DEG: f64 = 32.0;
pub const FACE_DRAWN_SLOPE: f64 = 0.40;
/// Patches and stones on bare ground steeper than this (INFERRED).
pub const PATCH_SLOPE_DEG: f64 = 22.0;
pub const PATCH_DRAWN_SLOPE: f64 = 0.25;
/// The cliff band: faces within this of the shoreline (drawn metres).
pub const CLIFF_SHORE_M: f64 = 160.0;
/// Lattices (real metres): faces and patches, whole cliffs.
pub const SLOPE_LATTICE_M: f64 = 4.0;
pub const CLIFF_LATTICE_M: f64 = 24.0;

/// The mapped geologic units (USGS: J. Schlocker, 1974, Geology of the San
/// Francisco North quadrangle, Professional Paper 782; W. P. Elder, 2001,
/// Geology of the Golden Gate Headlands, USGS Bulletin 2188; NPS Presidio
/// "Geologic Setting" and Golden Gate "Chert" pages): the Presidio's
/// Franciscan is largely serpentinite, on the coastal bluffs from the south
/// end of the bridge to Baker Beach; the Marin Headlands terrane is pillow
/// basalt (greenstone) under radiolarian chert, the chert forming the
/// ridges (Battery Spencer) and the greenstone low on the shore; Franciscan
/// sandstone elsewhere. The boundaries here are simplified to boxes and an
/// elevation (INFERRED; the map plates were not traced).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Chert,
    Greenstone,
    Serpentinite,
    Sandstone,
}

/// The unit at (lon, lat) at `navd88` metres.
pub fn unit_at(lon: f64, lat: f64, navd88: f64) -> Unit {
    if lat > 37.8135 {
        // north of the strait: the Marin Headlands terrane.
        if navd88 < GREENSTONE_TOP_M { Unit::Greenstone } else { Unit::Chert }
    } else if lon < -122.4740 && lat > 37.7885 {
        Unit::Serpentinite
    } else {
        Unit::Sandstone
    }
}

/// Where the Marin shore's greenstone gives way to the chert (NAVD88 m,
/// INFERRED: the basalt is the base of the sequence, exposed in the sea
/// cliffs; the chert caps the ridges above).
pub const GREENSTONE_TOP_M: f64 = 25.0;

/// Each unit's colour in its published words, as the Munsell chip they name
/// (sRGB, INFERRED): chert "red" (NPS) / "prevailingly dull brownish red"
/// (USGS Geolex, Ingleside chert): 10R 3/3 dusky red; greenstone "green"
/// (NPS, Rodeo Beach) / altered basalt, dark greenish grey 5GY 4/1;
/// serpentinite "blue-green" (Presidio Trust, Marshall's Beach): greyish
/// green 5G 4/2 (round 2's); sandstone (Franciscan greywacke) olive grey
/// 5Y 5/2 (round 2's).
pub fn unit_srgb(u: Unit) -> [u8; 3] {
    match u {
        Unit::Chert => [98, 64, 57],
        Unit::Greenstone => [86, 94, 84],
        Unit::Serpentinite => crate::hill_rock::SERPENTINITE_SRGB,
        Unit::Sandstone => crate::hill_rock::SANDSTONE_SRGB,
    }
}

/// Wet rock against dry: the measured wet/dry albedo ratio of a rough
/// mineral surface (Lekner and Dorf 1988, ground_cover `WET_SAND_RATIO`).
pub const WET_RATIO: f64 = crate::ground_cover::WET_SAND_RATIO;

fn lin(c: u8) -> f64 {
    let v = c as f64 / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The tint that takes a scan's mean albedo to `target` (linear): its
/// luminance wholly, its chromaticity `TINT_CHROMA` of the way (the scan's
/// own colour varies over its rock; a full per-channel ratio pushes that
/// variation to a false hue), times a variation `v` (0.85-1.15).
pub fn tint_to(k: &Kind, target: [f64; 3], v: f64) -> [f32; 3] {
    let y = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let m = k.mean_albedo.map(|c| (c as f64).max(1e-4));
    let (yt, ym) = (y(target).max(1e-4), y(m));
    [0, 1, 2].map(|i| {
        let chroma = (target[i] / yt) / (m[i] / ym);
        ((yt / ym) * (1.0 + TINT_CHROMA * (chroma - 1.0)) * v).clamp(0.05, 4.0) as f32
    })
}

/// The share of a unit's hue the tint takes over the scan's (INFERRED).
pub const TINT_CHROMA: f64 = 0.6;

/// The NAIP colour at a true (lon, lat) (see the module notes).
pub fn naip_rgb(naip: &Naip, lon: f64, lat: f64) -> Option<[u8; 3]> {
    use eo_farshore::survey::naip::GRID;
    let span = GRID.tile as f64 * GRID.dlat;
    let r = ((GRID.north - lat) / span - 0.5).round().clamp(0.0, GRID.rows as f64 - 1.0);
    let centre = GRID.north - (r + 0.5) * span;
    let row = GRID.tile as f64 / 2.0 + (centre - lat) / GRID.dlon;
    if !(0.0..GRID.tile as f64).contains(&row) {
        return None;
    }
    let nominal = GRID.north - (r * GRID.tile as f64 + row.floor() + 0.5) * GRID.dlat;
    naip.sample_rgb(lon, nominal)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    Vegetation,
    Bright,
    Sand,
    Water,
    Rock,
    DarkRock,
}

/// On land: bare ground, rock or soil (anything the aerial colour does
/// not call vegetation, short of the brightest: the serpentinite's
/// blue-grey, the chert's red-brown soil and rubble, a cliff's shadow).
pub fn bare_land(p: [u8; 3]) -> bool {
    let luma = (0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64) / 255.0;
    classify(p) != Cover::Vegetation && luma < BARE_LUMA_MAX
}

pub fn luma(p: [u8; 3]) -> f64 {
    (0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64) / 255.0
}
/// Under this a pixel on a gentle slope is shade (INFERRED).
pub const SHADOW_LUMA: f64 = 0.2;
/// On the city side, slopes this far inland of the shoreline at most (drawn m, INFERRED: the bluffs).
pub const INLAND_M: f64 = 250.0;

/// The brightest a bare pixel is (over it: concrete, a roof, a road's paint; INFERRED).
pub const BARE_LUMA_MAX: f64 = 0.78;
/// On a vegetated slope steeper than this, a block now and then (round 2's
/// hill rock rule: single blocks on steep slopes grouped by its 22 m field),
/// at this chance per lattice cell in a group (INFERRED).
pub const VEG_BLOCK_SLOPE_DEG: f64 = 35.0;
pub const VEG_BLOCK_CHANCE: f64 = 0.15;

pub fn classify(p: [u8; 3]) -> Cover {
    let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
    let luma = (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64) / 255.0;
    if 2 * g - r - b > EXG_VEG {
        Cover::Vegetation
    } else if luma > BRIGHT_LUMA {
        Cover::Bright
    } else if r > b + SAND_WARM && luma > 0.42 {
        Cover::Sand
    } else if b > r + 25 && g > r + 15 {
        Cover::Water
    } else if luma < DARK_LUMA {
        Cover::DarkRock
    } else if luma < ROCK_LUMA {
        Cover::Rock
    } else {
        Cover::Bright
    }
}

/// The survey readers the placement uses (any may be missing).
pub struct Survey {
    pub dem: Option<Dem1m>,
    pub bathy: Option<Bathy>,
    pub naip: Option<Naip>,
}

impl Survey {
    pub fn open() -> Option<Survey> {
        let root = eo_farshore::survey::Root::discover()?;
        let dem = Dem1m::open(&root).map_err(|e| eprintln!("scanned rock: 1 m DEM not read ({e:#})")).ok();
        let bathy = Bathy::open(&root).map_err(|e| eprintln!("scanned rock: CUDEM not read ({e:#})")).ok();
        Some(Survey { dem, bathy, naip: Some(Naip::open(&root)) })
    }
}

/// The far land (eo-app ground_tone `FAR_LAND_SHORE_M`, `FAR_LAND_ABOVE_SEA_M`):
/// the scene does not press it to the sea bed; neither does the rock's ground.
pub const FAR_LAND_SHORE_M: f64 = 60.0;
pub const FAR_LAND_ABOVE_SEA_M: f64 = 2.0;

/// The drawn world the pieces sit in.
pub struct World<'a> {
    /// The drawn ground (the detail ground where one is set, else the pressed terrain).
    pub ground: &'a (dyn Fn(f64, f64) -> Option<f64> + Sync),
    /// The shore field's signed distance (positive over the sea).
    pub signed: &'a (dyn Fn(f64, f64) -> f64 + Sync),
    pub sea: f64,
    /// The higher high and the high water over the sea, drawn.
    pub tide: [f64; 2],
    pub plan: f64,
    pub hs: f64,
}

/// A rock's distance from the surveyed waterline is searched this far
/// (real m) and its seat moved at most this far onto the drawn shore
/// (drawn m; INFERRED: farther, the drawn and surveyed shores are different
/// shores, and the rock keeps its surveyed place).
pub const SHORE_SEARCH_M: f64 = 120.0;
pub const SEAT_SHIFT_MAX_M: f64 = 150.0;

/// The CUDEM ground at (lon, lat) under a rock: the lowest of the point and
/// a ring of eight 6 m out (a rock's own top is not the shore it stands on).
pub fn cudem_base(bathy: &Bathy, lon: f64, lat: f64, mlat: f64, mlon: f64) -> Option<f64> {
    let mut m = bathy.height(lon, lat)? as f64;
    for k in 0..8 {
        let a = k as f64 * std::f64::consts::FRAC_PI_4;
        if let Some(v) = bathy.height(lon + a.cos() * 6.0 / mlon, lat + a.sin() * 6.0 / mlat) {
            m = m.min(v as f64);
        }
    }
    Some(m)
}

/// The signed distance (real m, positive seaward) from the CUDEM's mean
/// sea level contour at (lon, lat), on the rock's base ground
/// (`cudem_base`): the nearest crossing along 16 bearings, searched outward
/// in 1 m rings to `SHORE_SEARCH_M`. None when no crossing is that near.
pub fn cudem_signed(bathy: &Bathy, lon: f64, lat: f64, mlat: f64, mlon: f64) -> Option<f64> {
    let wet = |lo: f64, la: f64| cudem_base(bathy, lo, la, mlat, mlon).map(|e| e < MSL_OVER_NAVD88_M);
    let here = wet(lon, lat)?;
    let dirs: Vec<(f64, f64)> = (0..16).map(|k| (k as f64 * std::f64::consts::TAU / 16.0).sin_cos()).collect();
    let mut t = 1.0;
    while t <= SHORE_SEARCH_M {
        for &(sn, cs) in &dirs {
            if wet(lon + cs * t / mlon, lat + sn * t / mlat).is_some_and(|w| w != here) {
                let d = t - 0.5;
                return Some(if here { d } else { -d });
            }
        }
        t += 1.0;
    }
    None
}

/// Where a rock surveyed `signed_real` metres seaward of the surveyed
/// waterline stands on the drawn shore: from (x, z), along the shore
/// field's gradient (Newton steps on its signed distance) to the point
/// whose drawn signed distance is `signed_real` on the ground plan. None
/// where the field has no clear gradient or the move passes
/// `SEAT_SHIFT_MAX_M`.
pub fn seat_on_drawn_shore(w: &World, x: f64, z: f64, signed_real: f64) -> Option<(f64, f64)> {
    let target = signed_real * w.plan;
    let (mut px, mut pz) = (x, z);
    for _ in 0..8 {
        let s = (w.signed)(px, pz);
        let d = target - s;
        if d.abs() < 0.25 {
            break;
        }
        let g = [((w.signed)(px + 1.0, pz) - (w.signed)(px - 1.0, pz)) * 0.5, ((w.signed)(px, pz + 1.0) - (w.signed)(px, pz - 1.0)) * 0.5];
        let gl = g[0].hypot(g[1]);
        if gl < 0.3 {
            return None;
        }
        // the field is a distance: its gradient about unit; a step of d along it
        px += g[0] / gl * d.clamp(-40.0, 40.0);
        pz += g[1] / gl * d.clamp(-40.0, 40.0);
    }
    (((w.signed)(px, pz) - target).abs() < 2.0 && (px - x).hypot(pz - z) <= SEAT_SHIFT_MAX_M).then_some((px, pz))
}

/// Game (x, z) from (lon, lat) and back (the SF frame on the ground plan).
pub fn game_xz(lon: f64, lat: f64, plan: f64) -> (f64, f64) {
    let f = eo_data::Frame::sf();
    ((lon - f.center_lon) * f.m_per_deg_lon * plan, -(lat - f.center_lat) * f.m_per_deg_lat * plan)
}

pub fn lon_lat(x: f64, z: f64, plan: f64) -> (f64, f64) {
    let f = eo_data::Frame::sf();
    let (lat, lon) = f.unproject(x / plan, z / plan);
    (lon, lat)
}

/// The boxes (west, south, east, north; degrees) where the scans stand in
/// for the hill rock and dress the slopes: the San Francisco coastal bluffs
/// from Fort Point to the south end of Baker Beach, and the Marin shore and
/// ridge from Lime Point past Battery Spencer to Kirby Cove (the photo
/// spots' ground).
pub const REGIONS: [[f64; 4]; 2] = [[-122.4875, 37.7870, -122.4740, 37.8115], [-122.4900, 37.8195, -122.4730, 37.8340]];

/// The footprints the slope pieces (the cliff band's faces and the
/// outcrops) claim from the props' hill rock (`scan::Claims`): a disc about
/// each piece's bounding sphere's centre, `CLAIM_SHARE` of its radius (a
/// piece's sphere holds its corners; its rock covers less), plus
/// `CLAIM_MARGIN_M` so a hill-rock block (2.25 m or more across its half)
/// does not grow through the scan.
pub fn claim_discs(lib: &Library, pieces: &[Piece]) -> Vec<[f64; 3]> {
    pieces
        .iter()
        .filter(|p| matches!(p.source, Source::Cliff | Source::Outcrop))
        .map(|p| {
            let k = &lib.kinds[p.kind as usize];
            let (c, r) = k.sphere();
            let cw = scan::quat_rotate(p.rot, [c[0] as f64 * p.scale[0], c[1] as f64 * p.scale[1], c[2] as f64 * p.scale[2]]);
            let smax = p.scale[0].max(p.scale[1]).max(p.scale[2]);
            [p.pos[0] + cw[0], p.pos[2] + cw[2], CLAIM_SHARE * r as f64 * smax + CLAIM_MARGIN_M]
        })
        .collect()
}

/// INFERRED (see `claim_discs`).
pub const CLAIM_SHARE: f64 = 0.7;
pub const CLAIM_MARGIN_M: f64 = 1.0;

fn hash(a: i64, b: i64, salt: u32) -> f64 {
    crate::rocks::hash(a, b, salt)
}

/// The ground's plane over a disc of `r` about (x, z): (height at the
/// centre, unit normal, the largest residual), from 13 samples.
pub fn ground_plane(w: &World, x: f64, z: f64, r: f64) -> Option<(f64, [f64; 3], f64)> {
    let mut pts = vec![(0.0, 0.0, (w.ground)(x, z)?)];
    for ring in [0.5, 1.0] {
        for k in 0..6 {
            let a = k as f64 * std::f64::consts::FRAC_PI_3 + ring * 0.5;
            let (dx, dz) = (a.cos() * r * ring, a.sin() * r * ring);
            pts.push((dx, dz, (w.ground)(x + dx, z + dz)?));
        }
    }
    // least squares y = a dx + b dz + c
    let n = pts.len() as f64;
    let (sx, sz, sy) = pts.iter().fold((0.0, 0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1, s.2 + p.2));
    let (mx, mz, my) = (sx / n, sz / n, sy / n);
    let (mut xx, mut zz, mut xz, mut xy, mut zy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for p in &pts {
        let (dx, dz, dy) = (p.0 - mx, p.1 - mz, p.2 - my);
        xx += dx * dx;
        zz += dz * dz;
        xz += dx * dz;
        xy += dx * dy;
        zy += dz * dy;
    }
    let det = xx * zz - xz * xz;
    if det.abs() < 1e-9 {
        return None;
    }
    let a = (xy * zz - zy * xz) / det;
    let b = (zy * xx - xy * xz) / det;
    let c = my - a * mx - b * mz;
    let res = pts.iter().map(|p| (p.2 - (a * p.0 + b * p.1 + c)).abs()).fold(0.0, f64::max);
    let l = (a * a + 1.0 + b * b).sqrt();
    Some((c, [-a / l, 1.0 / l, -b / l], res))
}

fn norm(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    v.map(|c| c / l)
}

/// Seat scan `kind` at (x, z) at uniform scale `s`: its bed on the drawn
/// ground's plane under its footprint; its face (a face's +z) turned to
/// the horizontal direction `dir` (downhill), else by `yaw`. `lean`
/// (0-1) blends the bed's normal from the ground's toward the vertical
/// (a stone rests level-ish; a patch and a face follow the ground). Returns
/// the piece's origin, turn and the ground height under it.
pub fn seat(lib: &Library, kind: u8, w: &World, x: f64, z: f64, s: [f64; 3], dir: Option<[f64; 2]>, yaw: f64, lean: f64, sink: f64) -> Option<([f64; 3], [f64; 4], f64)> {
    seat_twisted(lib, kind, w, x, z, s, dir, yaw, lean, sink, [0.0, 0.0])
}

/// `seat`, the seated frame then turned by `twist` (radians): a yaw about
/// its bed's normal and a roll about its face's direction, so pieces on
/// one lattice turned to one slope do not stand in rows.
#[allow(clippy::too_many_arguments)]
pub fn seat_twisted(lib: &Library, kind: u8, w: &World, x: f64, z: f64, s: [f64; 3], dir: Option<[f64; 2]>, yaw: f64, lean: f64, sink: f64, twist: [f64; 2]) -> Option<([f64; 3], [f64; 4], f64)> {
    let k = &lib.kinds[kind as usize];
    let half = 0.5 * ((k.max[0] - k.min[0]) as f64 * s[0]).max((k.max[2] - k.min[2]) as f64 * s[2]);
    let (y, n, res) = ground_plane(w, x, z, half.max(0.5))?;
    let mut bb = norm([n[0] * (1.0 - lean), n[1] * (1.0 - lean) + lean, n[2] * (1.0 - lean)]);
    let mut sink = sink;
    let f0 = match dir {
        Some(d) => {
            // a face: the drawn slope's tilt plus `FACE_STEEPEN` (an outcrop
            // stands steeper than the soil round it, its foot in the slope and
            // its head out of it), turned to the DEM's downhill azimuth `d`;
            // seated uphill on that plane, lifted half the ground's departure
            // from its plane so its relief clears the drawn facets.
            let tilt = (bb[0] * bb[0] + bb[2] * bb[2]).sqrt().atan2(bb[1]) + FACE_STEEPEN_DEG.to_radians();
            let tilt = tilt.min(75f64.to_radians());
            bb = [d[0] * tilt.sin(), tilt.cos(), d[1] * tilt.sin()];
            sink -= 0.5 * res;
            [0.0, 1.0, 0.0]
        }
        None => [yaw.sin(), 0.0, yaw.cos()],
    };
    let d = f0[0] * bb[0] + f0[1] * bb[1] + f0[2] * bb[2];
    let mut ff = norm([f0[0] - d * bb[0], f0[1] - d * bb[1], f0[2] - d * bb[2]]);
    if twist != [0.0, 0.0] {
        // roll the bed about the face's direction, then yaw the face about the rolled bed
        let cr = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let (sr, cr_) = twist[1].sin_cos();
        let fb = cr(ff, bb);
        bb = norm([0, 1, 2].map(|i| bb[i] * cr_ + fb[i] * sr));
        let (sy, cy) = twist[0].sin_cos();
        let bf = cr(bb, ff);
        ff = norm([0, 1, 2].map(|i| ff[i] * cy + bf[i] * sy));
    }
    // the scan's frame: its face across its bed
    let b = k.bed_normal;
    let fd = k.face[0] * b[0] + k.face[1] * b[1] + k.face[2] * b[2];
    let f = norm([k.face[0] - fd * b[0], k.face[1] - fd * b[1], k.face[2] - fd * b[2]]);
    let q = scan::quat_frames(f, b, ff, bb);
    let bp = scan::quat_rotate(q, [k.bed_point[0] * s[0], k.bed_point[1] * s[1], k.bed_point[2] * s[2]]);
    let pos = [x - bp[0] - bb[0] * sink, y - bp[1] - bb[1] * sink, z - bp[2] - bb[2] * sink];
    Some((pos, q, y))
}

/// How much steeper than the drawn slope a face stands (INFERRED).
pub const FACE_STEEPEN_DEG: f64 = 6.0;
/// A face's turn off its slope's own: up to this yaw about its bed's normal
/// and roll about its downhill direction, either way (INFERRED: a broken
/// face's joints do not run parallel to the slope's contour; enough that
/// pieces on one lattice do not line up in rows, within the 20 degrees the
/// review asked for).
pub const FACE_YAW_JITTER_DEG: f64 = 22.0;
pub const FACE_ROLL_JITTER_DEG: f64 = 12.0;

/// The round-2 colours (`--no-rock-geology`): rocks.rs's boulder albedo for
/// the shore and the armour's, the hill rock's lithology box elsewhere.
fn round2_target(source: Source, lon: f64, lat: f64) -> [f64; 3] {
    match source {
        Source::Armour => crate::rocks::ARMOUR_ALBEDO,
        Source::Boulder | Source::Surf => crate::rocks::BOULDER_ALBEDO,
        _ => {
            let c = if lat > 37.8135 {
                crate::hill_rock::CHERT_SRGB
            } else if lon < -122.4770 && lat > 37.7885 {
                crate::hill_rock::SERPENTINITE_SRGB
            } else {
                crate::hill_rock::SANDSTONE_SRGB
            };
            c.map(lin)
        }
    }
}

/// The albedo a piece takes: its unit's colour (or round 2's), wet below the high water.
pub fn target_albedo(geology: bool, source: Source, lon: f64, lat: f64, navd88: f64) -> [f64; 3] {
    if !geology {
        return round2_target(source, lon, lat);
    }
    if source == Source::Armour {
        return crate::rocks::ARMOUR_ALBEDO; // quarried granite, not the bluff's rock
    }
    let unit = unit_srgb(unit_at(lon, lat, navd88)).map(lin);
    if matches!(source, Source::Surf | Source::Boulder) {
        // the shore's loose stones are sea-worn rock, darker than the unit's dry
        // outcrop colour (the colour words describe the outcrop): the measured
        // luminance of the CC0 `seaside_rock` scan (rocks.rs `BOULDER_ALBEDO`,
        // round 2's), in the unit's hue
        let y = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let k = y(crate::rocks::BOULDER_ALBEDO) / y(unit).max(1e-4);
        return unit.map(|c| c * k);
    }
    unit
}

/// How wet a piece standing on ground `g` (drawn) is: through below the
/// high water, drying over the higher high water's spray band (INFERRED: a
/// metre of splash above it).
pub fn wetness(w: &World, g: f64) -> f32 {
    let h = g - w.sea;
    let t = ((w.tide[0] + 1.0 * w.hs - h) / (w.tide[0] + 1.0 * w.hs - w.tide[1]).max(0.1)).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

/// Round 2's stones as scans: each keeps its centre and size; a boulder
/// takes the scanned boulder (and a share of the larger ones a scanned
/// cluster, so the beach's rock reads broken), the armour the boulder turned
/// every way at its quarried proportions.
///
/// With the cliff dressing on, a boulder on ground steeper than
/// `OUTCROP_STONE_SLOPE` (rocks.rs's steep-face zone: its flat blocks set
/// into a cliff, round 2's "boulder slope") is left out: the faces stand there.
pub fn from_stones(lib: &Library, w: &World, stones: &[crate::rocks::Stone], source: Source, geology: bool, cliff_dressing: bool) -> Vec<Piece> {
    let mut out = Vec::with_capacity(stones.len());
    for st in stones {
        if source == Source::Boulder && cliff_dressing {
            if let Some((_, n, _)) = ground_plane(w, st.c[0], st.c[2], 3.0 * w.plan) {
                if (1.0 - n[1] * n[1]).sqrt() / n[1].max(1e-3) > OUTCROP_STONE_SLOPE {
                    continue;
                }
            }
        }
        let u = |k: i64| hash(st.seed as i64, k, 913);
        let (lon, lat) = lon_lat(st.c[0], st.c[2], w.plan);
        let navd = (st.c[1] - w.sea) / w.hs + MSL_OVER_NAVD88_M;
        let cluster = source == Source::Boulder && st.diameter() > 1.6 * w.plan && u(1) < 0.4;
        let kind = if cluster { COAST_ROCKS } else { BOULDER };
        let k = &lib.kinds[kind as usize];
        let ext = [0, 1, 2].map(|i| (k.max[i] - k.min[i]) as f64);
        let tone = target_albedo(geology, source, lon, lat, navd);
        let piece = if source == Source::Armour {
            // the armour: the stone's semi-axes on the boulder's, any turn
            let s = [0, 1, 2].map(|i| 2.0 * st.r[i] / ext[i]);
            let q = scan::quat_mul(scan::quat_axis([u(2) - 0.5, u(3) - 0.5, u(4) - 0.5], u(5) * std::f64::consts::TAU), scan::quat_axis([0.0, 1.0, 0.0], u(6) * std::f64::consts::TAU));
            let (c, _) = k.sphere();
            let cc = scan::quat_rotate(q, [c[0] as f64 * s[0], c[1] as f64 * s[1], c[2] as f64 * s[2]]);
            let wet = wetness(w, st.c[1] - 0.5 * st.r[1]);
            Some(Piece { kind, pos: [st.c[0] - cc[0], st.c[1] - cc[1], st.c[2] - cc[2]], rot: q, scale: s, tint: tint_to(k, tone, 0.85 + 0.3 * u(7)), wet, source })
        } else {
            let d = st.diameter();
            let s0 = d / ext[0].max(ext[2]);
            let s = if cluster { [s0 * 1.6; 3] } else { [s0 * (0.9 + 0.2 * u(8)), s0 * (0.8 + 0.4 * u(9)), s0 * (0.9 + 0.2 * u(10))] };
            seat(lib, kind, w, st.c[0], st.c[2], s, None, u(11) * std::f64::consts::TAU, 0.6, if cluster { 0.0 } else { 0.0 }).map(|(pos, rot, g)| Piece { kind, pos, rot, scale: s, tint: tint_to(k, tone, 0.85 + 0.3 * u(12)), wet: wetness(w, g), source })
        };
        out.extend(piece);
    }
    out
}

/// rocks.rs's steep-face zone, rise over run (its `OUTCROP_SLOPE` less a margin for the drawn ground's smoothing).
pub const OUTCROP_STONE_SLOPE: f64 = 0.8;

/// The intertidal rock field over the claimed boxes (see the module notes).
pub fn surf_field(lib: &Library, w: &World, sv: &Survey, geology: bool) -> Vec<Piece> {
    surf_field_with(lib, w, sv, geology, &mut vec![])
}

/// `surf_field`, also listing each rock's move onto the drawn shore (game
/// metres: dx, dz; NaN and a reason where it kept its surveyed place).
pub fn surf_field_with(lib: &Library, w: &World, sv: &Survey, geology: bool, shifts: &mut Vec<[f64; 2]>) -> Vec<Piece> {
    let (Some(bathy), Some(naip)) = (&sv.bathy, &sv.naip) else { return vec![] };
    let mut out = vec![];
    for r in REGIONS {
        let (mlat, mlon) = eo_farshore::survey::metres_per_degree(0.5 * (r[1] + r[3]));
        let step = 0.6; // the imagery's pixel (real metres)
        let (dlon, dlat) = (step / mlon, step / mlat);
        let (nx, ny) = (((r[2] - r[0]) / dlon) as usize, ((r[3] - r[1]) / dlat) as usize);
        // a coarse pass (every 10th pixel): only the shore's band is read at the pixel
        const C: usize = 10;
        let (cx, cy) = (nx / C + 2, ny / C + 2);
        let coarse: Vec<bool> = (0..cy * cx)
            .map(|k| {
                let (i, j) = ((k % cx) * C, (k / cx) * C);
                bathy.height(r[0] + i as f64 * dlon, r[3] - j as f64 * dlat).is_some_and(|e| (FIELD_FLOOR_NAVD88_M - 3.0..=MHW_NAVD88_M + 3.0).contains(&(e as f64)))
            })
            .collect();
        let near = |i: usize, j: usize| {
            let (a, b) = (i / C, j / C);
            [(a, b), (a + 1, b), (a, b + 1), (a + 1, b + 1)].iter().any(|&(p, q)| p < cx && q < cy && coarse[q * cx + p])
        };
        // the cells: dark rock in the intertidal (rows north to south), found in parallel
        let rows: Vec<Vec<bool>> = {
            let near = &near;
            let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
            let chunk = ny.div_ceil(threads);
            std::thread::scope(|s| {
                let hs: Vec<_> = (0..threads)
                    .map(|t| {
                        s.spawn(move || {
                            (t * chunk..((t + 1) * chunk).min(ny))
                                .map(|j| {
                                    let lat = r[3] - (j as f64 + 0.5) * dlat;
                                    (0..nx)
                                        .map(|i| {
                                            let lon = r[0] + (i as f64 + 0.5) * dlon;
                                            if !near(i, j) {
                                                return false;
                                            }
                                            let Some(e) = bathy.height(lon, lat) else { return false };
                                            let e = e as f64;
                                            // a rock's own top may stand over the high water: the ground
                                            // round it (the lowest of a ring 6 m out) must lie in the band
                                            if !(FIELD_FLOOR_NAVD88_M..=MHW_NAVD88_M + FIELD_ROCK_TOP_M).contains(&e) {
                                                return false;
                                            }
                                            if !naip_rgb(naip, lon, lat).is_some_and(|p| classify(p) == Cover::DarkRock) {
                                                return false;
                                            }
                                            let ring = (0..8).filter_map(|k| {
                                                let a = k as f64 * std::f64::consts::FRAC_PI_4;
                                                bathy.height(lon + a.cos() * 6.0 / mlon, lat + a.sin() * 6.0 / mlat)
                                            }).fold(f64::INFINITY, |m, v| m.min(v as f64));
                                            (FIELD_FLOOR_NAVD88_M - 2.0..=MHW_NAVD88_M).contains(&ring)
                                        })
                                        .collect::<Vec<bool>>()
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            })
        };
        // connected rocks (8-neighbour)
        let mut label = vec![u32::MAX; nx * ny];
        let mut comps: Vec<Vec<(usize, usize)>> = vec![];
        for j in 0..ny {
            for i in 0..nx {
                if !rows[j][i] || label[j * nx + i] != u32::MAX {
                    continue;
                }
                let id = comps.len() as u32;
                let mut stack = vec![(i, j)];
                let mut cells = vec![];
                label[j * nx + i] = id;
                while let Some((a, b)) = stack.pop() {
                    cells.push((a, b));
                    for (da, db) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                        let (aa, bb) = (a as i64 + da, b as i64 + db);
                        if aa < 0 || bb < 0 || aa >= nx as i64 || bb >= ny as i64 {
                            continue;
                        }
                        let (aa, bb) = (aa as usize, bb as usize);
                        if rows[bb][aa] && label[bb * nx + aa] == u32::MAX {
                            label[bb * nx + aa] = id;
                            stack.push((aa, bb));
                        }
                    }
                }
                comps.push(cells);
            }
        }
        for (ci, cells) in comps.iter().enumerate() {
            // a rock of at least 3 pixels (about a square metre; INFERRED: smaller is noise or a shadow)
            if cells.len() < 3 {
                continue;
            }
            let area = cells.len() as f64 * step * step; // real m2
            let (si, sj) = cells.iter().fold((0.0, 0.0), |s, c| (s.0 + c.0 as f64, s.1 + c.1 as f64));
            let (ci_, cj_) = (si / cells.len() as f64, sj / cells.len() as f64);
            let lon = r[0] + (ci_ + 0.5) * dlon;
            let lat = r[3] - (cj_ + 0.5) * dlat;
            let seed = (ci as i64) * 7919 + (lon * 1e6) as i64;
            let u = |k: i64| hash(seed, k, 4111);
            // its seat on the drawn shore: the same distance from the drawn waterline as
            // from the surveyed one; the rock's pieces and stones all move with it
            let (x0, z0) = game_xz(lon, lat, w.plan);
            let surveyed = cudem_signed(bathy, lon, lat, mlat, mlon);
            let drawn = surveyed.and_then(|d| seat_on_drawn_shore(w, x0, z0, d));
            let shift = drawn.map(|(x, z)| [x - x0, z - z0]).unwrap_or([0.0, 0.0]);
            // the list: the move, or why there was none (NaN, then 0: no surveyed waterline near; 1: no seat on the drawn shore)
            shifts.push(match (surveyed, drawn) {
                (None, _) => [f64::NAN, 0.0],
                (Some(_), None) => [f64::NAN, 1.0],
                _ => shift,
            });
            // a large rock is tiled: pieces about 4 m apart over its cells
            let mut sites: Vec<(f64, f64, f64)> = vec![]; // lon, lat, footprint (real m)
            if area <= 16.0 {
                sites.push((lon, lat, area.sqrt()));
            } else {
                let tile = (4.0 / step) as usize;
                let mut seen = std::collections::HashSet::new();
                for &(a, b) in cells {
                    if seen.insert((a / tile, b / tile)) {
                        let n = cells.iter().filter(|c| c.0 / tile == a / tile && c.1 / tile == b / tile).count();
                        if n * 4 >= tile * tile {
                            let (ta, tb) = ((a / tile) as f64 * tile as f64 + tile as f64 / 2.0, (b / tile) as f64 * tile as f64 + tile as f64 / 2.0);
                            sites.push((r[0] + ta * dlon, r[3] - tb * dlat, (n as f64).sqrt() * step * 1.2));
                        }
                    }
                }
            }
            for (si, &(lo, la, foot)) in sites.iter().enumerate() {
                let uu = |k: i64| hash(seed + si as i64 * 31, k, 4127);
                let (x, z) = game_xz(lo, la, w.plan);
                let (x, z) = (x + shift[0], z + shift[1]);
                let e = bathy.height(lo, la).map(|v| v as f64).unwrap_or(0.0);
                // the stone: small rocks the boulder, the rest the scanned clusters (angular)
                let kind = if foot < 1.6 { BOULDER } else if uu(1) < 0.25 { LAND_ROCKS } else { COAST_ROCKS };
                let k = &lib.kinds[kind as usize];
                let ext = (k.max[0] - k.min[0]).max(k.max[2] - k.min[2]) as f64;
                // its footprint on the ground plan; a cluster's scan has its rocks inside a ground patch, so a little larger
                let span = foot * w.plan * if kind == BOULDER { 1.15 } else { 1.35 };
                let s0 = (span / ext).clamp(0.25, 3.0);
                let mut s = [s0, s0 * (0.9 + 0.35 * uu(2)), s0];
                // its height: the topobathy's highest point over its footprint (the rock's
                // own top) over the ground round it, on the drawn height scale; where the
                // drawn ground lies deeper than the surveyed, as far over the sea as surveyed
                let rr = 0.4 * foot;
                let e_top = [(0.0, 0.0), (rr, 0.0), (-rr, 0.0), (0.0, rr), (0.0, -rr)].iter().filter_map(|(dx, dy)| bathy.height(lo + dx / mlon, la + dy / mlat)).fold(e, |m, v| m.max(v as f64));
                let e_base = cudem_base(bathy, lo, la, mlat, mlon).unwrap_or(e);
                let tall = (k.max[1] as f64 - k.bed_point[1]).max(0.1);
                if let Some(g) = (w.ground)(x, z) {
                    let over_ground = (e_top - e_base) * w.hs;
                    let over_sea = w.sea + (e_top - MSL_OVER_NAVD88_M) * w.hs - g;
                    let exposed = w.sea + FIELD_EXPOSED_OVER_MSL_M * w.hs - g;
                    let want = over_ground.max(over_sea).max(exposed);
                    if want > 0.0 {
                        s[1] = (want / tall).clamp(0.6 * s0, 2.5 * s0);
                    }
                }
                let tone = target_albedo(geology, Source::Surf, lo, la, e);
                if let Some((pos, rot, g)) = seat(lib, kind, w, x, z, s, None, uu(3) * std::f64::consts::TAU, if kind == BOULDER { 0.7 } else { 0.2 }, 0.0) {
                    out.push(Piece { kind, pos, rot, scale: s, tint: tint_to(k, tone, 0.85 + 0.3 * uu(4)), wet: wetness(w, g).max(0.6), source: Source::Surf });
                }
            }
            // stones beside it, from the boulder size law
            let n_sat = (u(5) * 3.0 * area.sqrt().min(4.0) / 2.0) as usize;
            for k in 0..n_sat {
                let a = u(10 + k as i64) * std::f64::consts::TAU;
                let rr = area.sqrt() * (0.6 + 0.9 * u(20 + k as i64));
                let (lo, la) = (lon + a.cos() * rr / mlon, lat + a.sin() * rr / mlat);
                let (x, z) = game_xz(lo, la, w.plan);
                let (x, z) = (x + shift[0], z + shift[1]);
                let d = crate::rocks::boulder_d(u(30 + k as i64)) * w.plan;
                let kk = &lib.kinds[BOULDER as usize];
                let ext = (kk.max[0] - kk.min[0]).max(kk.max[2] - kk.min[2]) as f64;
                let s0 = d / ext;
                let s = [s0, s0 * (0.75 + 0.3 * u(40 + k as i64)), s0];
                let e = bathy.height(lo, la).map(|v| v as f64).unwrap_or(0.0);
                if e > MHW_NAVD88_M + 0.5 {
                    continue;
                }
                let tone = target_albedo(geology, Source::Surf, lo, la, e);
                if let Some((pos, rot, g)) = seat(lib, BOULDER, w, x, z, s, None, u(50 + k as i64) * std::f64::consts::TAU, 0.7, 0.0) {
                    out.push(Piece { kind: BOULDER, pos, rot, scale: s, tint: tint_to(kk, tone, 0.85 + 0.3 * u(60 + k as i64)), wet: wetness(w, g).max(0.6), source: Source::Surf });
                }
            }
        }
    }
    out
}

/// The DEM's slope (degrees) and its downhill direction (lon, lat axes as
/// east, north) at (lon, lat), over `r` real metres.
pub fn dem_slope(dem: &Dem1m, lon: f64, lat: f64, r: f64, mlat: f64, mlon: f64) -> Option<(f64, [f64; 2], f64)> {
    let h = |dx: f64, dy: f64| dem.height(lon + dx / mlon, lat + dy / mlat).map(|v| v as f64);
    let (e, w_, n, s, c) = (h(r, 0.0)?, h(-r, 0.0)?, h(0.0, r)?, h(0.0, -r)?, h(0.0, 0.0)?);
    let (gx, gy) = ((e - w_) / (2.0 * r), (n - s) / (2.0 * r));
    let g = (gx * gx + gy * gy).sqrt();
    let down = if g > 1e-6 { [-gx / g, -gy / g] } else { [0.0, 1.0] };
    Some((g.atan().to_degrees(), down, c))
}

/// The slopes' rock over the claimed boxes (see the module notes):
/// (faces and patches, of which the cliff band's).
pub fn slopes(lib: &Library, w: &World, sv: &Survey, geology: bool, cliff_dressing: bool) -> Vec<Piece> {
    let (Some(dem), Some(naip)) = (&sv.dem, &sv.naip) else { return vec![] };
    let mut out = vec![];
    for (ri, r) in REGIONS.iter().enumerate() {
        let (mlat, mlon) = eo_farshore::survey::metres_per_degree(0.5 * (r[1] + r[3]));
        let rock_share = |lon: f64, lat: f64, rad: f64| -> f64 {
            let mut n = 0;
            let mut k = 0;
            for a in -2..=2 {
                for b in -2..=2 {
                    let p = naip_rgb(naip, lon + a as f64 * rad / 2.0 / mlon, lat + b as f64 * rad / 2.0 / mlat);
                    if let Some(p) = p {
                        n += 1;
                        if bare_land(p) {
                            k += 1;
                        }
                    }
                }
            }
            if n == 0 { 0.0 } else { k as f64 / n as f64 }
        };
        // whole cliffs first (a coarse lattice), then faces and patches between
        let mut placed_cliffs: Vec<(f64, f64, f64)> = vec![];
        for (lattice, cliff) in [(CLIFF_LATTICE_M, true), (SLOPE_LATTICE_M, false)] {
            let (dlon, dlat) = (lattice / mlon, lattice / mlat);
            let (nx, ny) = (((r[2] - r[0]) / dlon) as i64, ((r[3] - r[1]) / dlat) as i64);
            let cells: Vec<Vec<Piece>> = {
                let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8) as i64;
                let chunk = (ny + threads - 1) / threads;
                let placed_cliffs = &placed_cliffs;
                std::thread::scope(|s| {
                    let hs: Vec<_> = (0..threads)
                        .map(|t| {
                            let rock_share = &rock_share;
                            s.spawn(move || {
                                let mut v = vec![];
                                for j in t * chunk..((t + 1) * chunk).min(ny) {
                                    for i in 0..nx {
                                        let salt = if cliff { 5300 } else { 5400 } + ri as u32 * 7;
                                        let u = |k: u32| hash(i, j, salt + k);
                                        // a whole cliff anywhere in its 24 m cell (its lattice's rows showed as
                                        // rows of slabs); a face or patch within the middle of its 4 m cell
                                        let (lo, span) = if cliff { (0.0, 1.0) } else { (0.3, 0.4) };
                                        let lon = r[0] + (i as f64 + lo + span * u(0)) * dlon;
                                        let lat = r[3] - (j as f64 + lo + span * u(1)) * dlat;
                                        let (x, z) = game_xz(lon, lat, w.plan);
                                        let sd = (w.signed)(x, z);
                                        // on land (the far shore counts: north of the strait the field is positive), not in the sea
                                        let Some((slope, down, e)) = dem_slope(dem, lon, lat, if cliff { 6.0 } else { 1.5 }, mlat, mlon) else { continue };
                                        if e < MHW_NAVD88_M + 0.5 {
                                            continue;
                                        }
                                        let near_shore = sd > -CLIFF_SHORE_M && lat < 37.8135 || (lat > 37.8135 && e < 60.0);
                                        if lat < 37.8135 && sd < -INLAND_M {
                                            continue; // the city side: the bluffs only, not the Presidio's woods and lots
                                        }
                                        if cliff {
                                            if slope < FACE_SLOPE_DEG || rock_share(lon, lat, 14.0) < 0.45 || (!cliff_dressing && near_shore) {
                                                continue;
                                            }
                                            let Some((s2, _, e2)) = dem_slope(dem, lon, lat, 12.0, mlat, mlon) else { continue };
                                            if s2 < 32.0 || e2 < 6.0 {
                                                continue;
                                            }
                                        } else {
                                            if slope < PATCH_SLOPE_DEG || placed_cliffs.iter().any(|c| (c.0 - x).hypot(c.1 - z) < c.2) {
                                                continue;
                                            }
                                        }
                                        // bare ground (rock or soil) or, on a steep vegetated slope, a block now and then
                                        // (a dark pixel on a gentle slope is a tree's shadow, not rock)
                                        let px = naip_rgb(naip, lon, lat);
                                        let bare = !cliff && px.is_some_and(|p| bare_land(p) && (luma(p) >= SHADOW_LUMA || slope >= FACE_SLOPE_DEG)) && rock_share(lon, lat, 3.0) >= 0.5;
                                        // bare soil (the warm, pale classes: sand, the chert's eroded red-brown
                                        // soil) takes stones, never a face; a face wants grey rock
                                        let soil = px.is_some_and(|p| matches!(classify(p), Cover::Sand | Cover::Bright));
                                        let veg_block = !cliff && !bare && slope >= VEG_BLOCK_SLOPE_DEG && u(10) < VEG_BLOCK_CHANCE * crate::vegetation::smoothstep(0.35, 0.75, crate::vegetation::field(x, z, 22.0, 31));
                                        if !cliff && !bare && !veg_block {
                                            continue;
                                        }
                                        if !cliff {
                                            if slope >= FACE_SLOPE_DEG && !cliff_dressing && near_shore {
                                                continue;
                                            }
                                        }
                                        let source = if near_shore && slope >= FACE_SLOPE_DEG { Source::Cliff } else { Source::Outcrop };
                                        // the drawn ground must be steep too (no faces on a battery's slab)
                                        let Some((_, n, _)) = ground_plane(w, x, z, 3.0 * w.plan) else { continue };
                                        let drawn = (1.0 - n[1] * n[1]).sqrt() / n[1].max(1e-3);
                                        let tone = target_albedo(geology, source, lon, lat, e);
                                        let dir = Some([down[0], -down[1]]); // north is -z
                                        if cliff && soil {
                                            continue;
                                        }
                                        let (kind, s0, lean) = if cliff {
                                            (CLIFF, 0.45 + 0.3 * u(2), 0.0)
                                        } else if veg_block {
                                            if drawn < PATCH_DRAWN_SLOPE {
                                                continue;
                                            }
                                            if u(3) < 0.5 { (FACE_2, 0.6 + 0.8 * u(4), 0.0) } else { (BOULDER, crate::rocks::boulder_d(u(7)).max(0.9) / 1.84, 0.5) }
                                        } else if slope >= FACE_SLOPE_DEG && drawn >= FACE_DRAWN_SLOPE && !soil {
                                            if u(3) < 0.6 { (FACE_1, 1.0 + 0.6 * u(4), 0.0) } else { (FACE_2, 1.0 + 0.8 * u(4), 0.0) }
                                        } else if drawn >= PATCH_DRAWN_SLOPE {
                                            // bare slopes: stones and broken patches; fewer (a bare slope is mostly soil)
                                            if u(5) > if soil { 0.25 } else { 0.45 } {
                                                continue;
                                            }
                                            match (u(6) * 4.0) as u32 {
                                                0 => (BOULDER, crate::rocks::boulder_d(u(7)) / 1.84, 0.5),
                                                1 => (SAND_ROCKS, 0.8 + 0.6 * u(7), 0.0),
                                                2 => (LAND_ROCKS, 0.5 + 0.4 * u(7), 0.0),
                                                _ => (FACE_2, 0.6 + 0.6 * u(7), 0.0),
                                            }
                                        } else {
                                            continue;
                                        };
                                        if cliff && drawn < FACE_DRAWN_SLOPE {
                                            continue;
                                        }
                                        let s = [s0 * w.plan; 3];
                                        let k = &lib.kinds[kind as usize];
                                        let face_dir = if scan::role(kind) == Role::Face { dir } else { None };
                                        let sink = if kind == FACE_1 || kind == FACE_2 { 0.03 * s[1] } else { 0.0 };
                                        let twist = if face_dir.is_some() { [(2.0 * u(11) - 1.0) * FACE_YAW_JITTER_DEG.to_radians(), (2.0 * u(12) - 1.0) * FACE_ROLL_JITTER_DEG.to_radians()] } else { [0.0, 0.0] };
                                        if let Some((pos, rot, g)) = seat_twisted(lib, kind, w, x, z, s, face_dir, u(8) * std::f64::consts::TAU, lean, sink, twist) {
                                            v.push(Piece { kind, pos, rot, scale: s, tint: tint_to(k, tone, 0.85 + 0.3 * u(9)), wet: wetness(w, g), source });
                                        }
                                    }
                                }
                                v
                            })
                        })
                        .collect();
                    hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
                })
            };
            for v in cells {
                for p in v {
                    if p.kind == CLIFF {
                        let k = &lib.kinds[CLIFF as usize];
                        let half = 0.4 * (k.max[0] - k.min[0]) as f64 * p.scale[0];
                        placed_cliffs.push((p.pos[0], p.pos[2], half));
                    }
                    out.push(p);
                }
            }
        }
    }
    out
}

/// The surf's obstacles: every free-standing rock (the intertidal field's
/// and the beach boulders; not the revetment's armour, which the seawall's
/// own edge stands for) in the water's reach (its ground under the higher
/// high water), as a disc of its footprint and its top over the sea.
pub fn obstacles(lib: &Library, w: &World, pieces: &[Piece]) -> Vec<eo_water::obstacles::Obstacle> {
    pieces
        .iter()
        .filter(|p| matches!(p.source, Source::Surf | Source::Boulder))
        .filter_map(|p| {
            let k = &lib.kinds[p.kind as usize];
            let (c, r) = k.sphere();
            let cw = scan::quat_rotate(p.rot, [c[0] as f64 * p.scale[0], c[1] as f64 * p.scale[1], c[2] as f64 * p.scale[2]]);
            let (x, z) = (p.pos[0] + cw[0], p.pos[2] + cw[2]);
            let g = (w.ground)(x, z)?;
            if g - w.sea > w.tide[0] || (w.signed)(x, z) < -60.0 {
                return None;
            }
            let smax = p.scale[0].max(p.scale[2]);
            let top = p.pos[1] + cw[1] + 0.6 * r as f64 * p.scale[1];
            Some(eo_water::obstacles::Obstacle { x, z, radius: 0.6 * r as f64 * smax, height: top - w.sea })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datums_and_units() {
        assert!((MSL_OVER_NAVD88_M - 0.969).abs() < 0.001);
        assert!((MHW_NAVD88_M - 1.612).abs() < 0.001);
        // the station's MHW over MSL is ground_cover's
        assert!((MHW_NAVD88_M - MSL_OVER_NAVD88_M - crate::ground_cover::MHW_OVER_MSL_M).abs() < 0.01);
        assert_eq!(unit_at(-122.4816, 37.8275, 140.0), Unit::Chert, "Battery Spencer");
        assert_eq!(unit_at(-122.4816, 37.8275, 5.0), Unit::Greenstone);
        assert_eq!(unit_at(-122.4800, 37.8010, 10.0), Unit::Serpentinite, "Baker Beach's north bluff");
        assert_eq!(unit_at(-122.4500, 37.8000, 10.0), Unit::Sandstone);
    }

    #[test]
    fn the_classes() {
        assert_eq!(classify([60, 90, 50]), Cover::Vegetation);
        assert_eq!(classify([210, 200, 180]), Cover::Bright);
        assert_eq!(classify([170, 140, 110]), Cover::Sand);
        assert_eq!(classify([60, 60, 62]), Cover::DarkRock);
        assert_eq!(classify([120, 128, 135]), Cover::Rock);
        assert_eq!(classify([70, 100, 125]), Cover::Water);
    }

    #[test]
    fn game_and_geodetic_agree() {
        // the plan's formula: x = (lon + 122.4194) 87986.2475 1.25, z = (37.778 - lat) 110574 1.25
        let (x, z) = game_xz(-122.48080, 37.79995, 1.25);
        assert!((x - (-122.48080 + 122.4194) * 87986.2475 * 1.25).abs() < 0.05, "{x}");
        assert!((z - (37.778 - 37.79995) * 110574.0 * 1.25).abs() < 0.05, "{z}");
        let (lon, lat) = lon_lat(x, z, 1.25);
        assert!((lon + 122.48080).abs() < 1e-9 && (lat - 37.79995).abs() < 1e-9);
    }

    /// The tiles' true layout: with the survey root, the row a true latitude
    /// reads is continuous across a tile edge (the two tiles hold the same
    /// ground there).
    #[test]
    fn the_imagery_is_read_where_it_lies() {
        let Some(root) = eo_farshore::survey::Root::discover() else { return };
        let naip = Naip::open(&root);
        // across the row 2 / row 3 edge of the nominal grid near Baker Beach
        let lon = -122.479;
        let mut diffs = vec![];
        for k in -3..3 {
            let lat = 37.80128 + k as f64 * 2.0e-6;
            if let (Some(a), Some(b)) = (naip_rgb(&naip, lon, lat), naip_rgb(&naip, lon, lat + 2.0e-6)) {
                diffs.push((0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).sum::<i32>());
            }
        }
        assert!(!diffs.is_empty());
        let mean = diffs.iter().sum::<i32>() as f64 / diffs.len() as f64;
        assert!(mean < 60.0, "{diffs:?}");
    }
}
