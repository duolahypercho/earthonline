//! (`--no-hill-outcrops`, Docs/HILLS-PLAN.md H7b) The scanned rock on the
//! city's hills: the chert of Corona Heights, Twin Peaks, Glen Canyon,
//! Mount Davidson and Bernal Heights, the greenstone and serpentinite where
//! the bedrock map has them, drawn with the survey set's CC0 rock scans
//! (group S1, Poly Haven: the faces `rock_face_01` and `rock_face_02`, the
//! rock clusters `coast_rocks_05` and `coast_land_rocks_02`, the stone
//! `boulder_01`) instead of the props' procedural blocks, which read as
//! smooth blobs (they step back where a scan stands: `scan::Placed::claims`).
//!
//! Where, on a 4 m (real) lattice over the bedrock map's cells and the
//! mapped bare rock's areas:
//!
//! * the city's bedrock map (`eo_data::lithology`, the USGS geologic map
//!   units) maps bedrock there, or OSM maps bare rock (`natural=bare_rock`,
//!   the ground cover's `bareRock`);
//! * the aerial image, through the hill builder's planting map (eo-app
//!   installs it: [`install_aerial`]), has open ground there: data (the map
//!   holds only open ground outside the footprints, so a roof, a yard or a
//!   street is never a site) under [`OPEN_CANOPY_MAX`] canopy;
//! * the USGS 3DEP 1 m DEM (`external/dem/1m`, the six tiles the elevation
//!   grid was cooked from) is steep there: steeper than [`STEEP_FACE_DEG`],
//!   or than [`FACE_DEG`] on bare ground ([`BARE_SHARE`]), or than
//!   [`MAPPED_FACE_DEG`] on mapped rock. From [`FACE_MIN_DEG`] a face stands
//!   there, turned down the slope; gentler rock takes a scanned rock cluster,
//!   a broken patch or a stone. Below a face, a stone now and then (its talus).
//!
//! Each piece is tinted to its rock's weathered colour ([`rock_srgb`]). The
//! landmark build places the pieces on the 8 m grid (their footprints claim
//! the props' hill rock from the start) and marks them pending
//! (`scan::hills_pending`); once the scene has built the drawn ground (the
//! hill builder's 2 m relief, `eo_data::detail_ground`), eo-app runs
//! [`late_pass`], which places them again on that ground, and only then does
//! the renderer build the scans. Seated on the 8 m grid, the relief's knobs
//! (up to 4 m, exactly where the rock is) buried them.
//!
//! The scans load from `EO_SURVEY_ROOT`, else from `external/survey`
//! ([`library_root`]), with every file checked against
//! `native/assets/survey-manifest.json`. Found only that way (no
//! `EO_SURVEY_ROOT`), the scans dress the hills alone: the Golden Gate's
//! survey features stay as they were (set.rs `shore_rocks`), and no hill
//! piece stands in the Golden Gate's own boxes (place.rs `REGIONS`).

use super::place::{self, World};
use super::scan::{self, BOULDER, COAST_ROCKS, FACE_1, FACE_2, LAND_ROCKS, Library, Piece, Role, Source};
use eo_data::lithology::RockClass;
use eo_farshore::survey::TiledRaster;
use std::sync::OnceLock;

/// The switch.
pub fn switch_on() -> bool {
    static S: OnceLock<bool> = OnceLock::new();
    *S.get_or_init(|| !std::env::args().any(|a| a == "--no-hill-outcrops"))
}

/// The survey folder the scans load from: `EO_SURVEY_ROOT`, else (the switch
/// on) `<repo>/external/survey` when it is a folder.
pub fn library_root() -> Option<eo_farshore::survey::Root> {
    if let Some(r) = eo_farshore::survey::Root::discover() {
        return Some(r);
    }
    if !switch_on() {
        return None;
    }
    let repo = eo_data::DataPaths::discover().ok()?.repo_root;
    eo_farshore::survey::Root::at(repo.join(eo_data::external::DIR).join("survey"))
}

/// What the aerial image holds round a point: the shares of its classified
/// pixels (eo-hills planting: `eo_landmarks::vegetation::Naip`).
#[derive(Clone, Copy, Debug)]
pub struct Aerial {
    pub bare: f32,
    pub soil: f32,
    pub canopy: f32,
}

type AerialFn = dyn Fn(f64, f64, f64) -> Option<Aerial> + Send + Sync;
static AERIAL: OnceLock<Box<AerialFn>> = OnceLock::new();

/// Install the aerial reader: (game x, z, box side in game m) to the shares
/// there, None where the image has no data. eo-app installs the hill
/// builder's planting map before the landmark build starts.
pub fn install_aerial(f: Box<AerialFn>) {
    let _ = AERIAL.set(f);
}

fn aerial(x: f64, z: f64, side: f64) -> Option<Aerial> {
    AERIAL.get().and_then(|f| f(x, z, side))
}

/// The aerial box round a site (real m).
pub const AERIAL_BOX_M: f64 = 6.0;
/// Open ground: the image has data there (the planting map holds only open
/// ground, outside the footprints) and under this share of it is canopy
/// (a steep wooded slope is soil under trees, not rock).
pub const OPEN_CANOPY_MAX: f32 = 0.45;
/// A face on bare ground: the DEM steeper than [`FACE_DEG`] and at least
/// this share of the image's pixels bare (rock or soil).
pub const BARE_SHARE: f32 = 0.5;
/// The DEM's slope (degrees, over 3 m) for a face on bare ground. INFERRED:
/// steeper than the 32 degrees the bluffs take (place.rs `FACE_SLOPE_DEG`),
/// since an inland slope at 32 degrees is mostly grass on soil; a chert
/// outcrop stands out of the slope as a step the 1 m DEM resolves.
pub const FACE_DEG: f64 = 36.0;
/// A face whatever the image says, on open ground: steeper than this
/// (degrees, over 3 m). A cliff's face is in its own shadow in the summer
/// midday image, so it reads dark there, not bare (Corona Heights' quarry
/// face); soil and grass do not hold at this slope.
pub const STEEP_FACE_DEG: f64 = 45.0;
/// On the mapped bare rock (OSM `natural=bare_rock`, the ground cover's
/// `bareRock`: Corona Heights' summit, Glen Canyon's outcrops), rock from this
/// slope (degrees); every site of it takes rock (a cluster, a patch or a stone
/// where too gentle for a face), so the outcrop the props' blocks drew stays rock.
pub const MAPPED_FACE_DEG: f64 = 14.0;
/// The sites' lattice (real m). (The scanned sea cliff, `coastal_cliff_02`, an
/// overhung sandstone shell, is not used on the hills: from the side it read as a
/// floating lip.)
pub const SITE_LATTICE_M: f64 = 4.0;
/// How a hill's face is laid. The faces are open shells scanned from their front: on
/// the bluffs (place.rs) they stand 6 degrees steeper than the slope (at most 75) and
/// are lifted half the ground's departure from their plane, seen from the beach; on a
/// hill a walker beside one saw its dark inner back. Here they stand 3 degrees over
/// the drawn slope (at most 60), on their plane, sunk [`FACE_SINK`] of their height;
/// sinking them by the whole departure buried them in the relief's knobs.
pub const FACE_SEAT: place::FaceSeat = place::FaceSeat { steepen_deg: 3.0, max_tilt_deg: 60.0, residual_lift: 0.0 };
pub const FACE_SINK: f64 = 0.05;
/// A face only where the DEM is at least this steep (degrees): there the slope
/// itself hides its back from a walker above it; gentler rock (the mapped bare
/// rock's lower parts) takes the scanned rock patches, whose own ground is cut
/// away so their rocks stand out of ours from every side.
pub const FACE_MIN_DEG: f64 = 30.0;
/// Below a face, its talus: a stone or broken rock this often, laid this far
/// (real m) down the slope from it.
pub const TALUS_CHANCE: f64 = 0.45;
pub const TALUS_M: (f64, f64) = (2.0, 5.5);

/// The rock's colour by class, as its weathered outcrops show it (sRGB). The
/// props' blocks take the fresh rock's chips (hill_rock.rs: chert 10R 3/2, the
/// USGS "dull brownish red"), but an outcrop's face is weathered and lichened:
/// the NAIP 0.3 m image reads Corona Heights' summit rock at (149, 136, 135)
/// and Twin Peaks' at (140, 147, 149), haze included, against (169, 154, 147)
/// for the bare soil beside it. INFERRED: chert 10R 5/2 (pale reddish
/// brown), the others a step lighter than their chips in value at their hue.
pub fn rock_srgb(c: RockClass) -> [u8; 3] {
    match c {
        RockClass::Chert => [138, 114, 106],
        RockClass::Serpentinite => [112, 126, 116],
        RockClass::Greenstone => [112, 118, 106],
        _ => [146, 138, 118],
    }
}

/// The 1 m DEM tiles in `external/dem/1m` (each checked against its
/// `PROVENANCE.json` size), primaries first.
pub struct Dem {
    tiles: Vec<TiledRaster>,
}

impl Dem {
    pub fn open() -> Option<Dem> {
        let dir = match std::env::var_os("EO_DEM1M_DIR").filter(|v| !v.is_empty()) {
            Some(v) => std::path::PathBuf::from(v),
            None => eo_data::DataPaths::discover().ok()?.repo_root.join(eo_data::external::DIR).join("dem").join("1m"),
        };
        let prov: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("PROVENANCE.json")).ok()?).ok()?;
        let mut files: Vec<(bool, String, u64)> = prov["files"]
            .as_array()?
            .iter()
            .filter_map(|f| Some((f["role"].as_str()? == "primary", f["path"].as_str()?.to_string(), f["bytes"].as_u64()?)))
            .collect();
        files.sort_by_key(|f| !f.0);
        let mut tiles = vec![];
        for (_, path, bytes) in files {
            let p = dir.join(&path);
            match std::fs::metadata(&p) {
                Ok(m) if m.len() == bytes => {}
                _ => {
                    eprintln!("hill outcrops: {} missing or not the size PROVENANCE.json records; no hill outcrops", p.display());
                    return None;
                }
            }
            tiles.push(TiledRaster::open(&p).map_err(|e| eprintln!("hill outcrops: {}: {e:#}", p.display())).ok()?);
        }
        (!tiles.is_empty()).then_some(Dem { tiles })
    }

    /// NAVD88 height at (lon, lat).
    pub fn height(&self, lon: f64, lat: f64) -> Option<f64> {
        let (e, n) = eo_farshore::survey::dem1m::UTM10N_NAD83.forward(lon, lat);
        self.tiles.iter().find_map(|t| t.at(e, n)).map(|v| v as f64)
    }

    /// Slope (degrees), downhill direction (east, north) and height, over `r` real m.
    pub fn slope(&self, lon: f64, lat: f64, r: f64, mlat: f64, mlon: f64) -> Option<(f64, [f64; 2], f64)> {
        let h = |dx: f64, dy: f64| self.height(lon + dx / mlon, lat + dy / mlat);
        let (e, w_, n, s, c) = (h(r, 0.0)?, h(-r, 0.0)?, h(0.0, r)?, h(0.0, -r)?, h(0.0, 0.0)?);
        let (gx, gy) = ((e - w_) / (2.0 * r), (n - s) / (2.0 * r));
        let g = (gx * gx + gy * gy).sqrt();
        let down = if g > 1e-6 { [-gx / g, -gy / g] } else { [0.0, 1.0] };
        Some((g.atan().to_degrees(), down, c))
    }
}

fn hash(a: i64, b: i64, salt: u32) -> f64 {
    crate::rocks::hash(a, b, salt)
}

/// Whether (lon, lat) lies in one of the Golden Gate's own boxes
/// (place.rs `REGIONS`), which the survey features dress.
fn in_golden_gate(lon: f64, lat: f64) -> bool {
    place::REGIONS.iter().any(|r| lon >= r[0] && lon <= r[2] && lat >= r[1] && lat <= r[3])
}

/// The hills' scanned rock (see the module notes). `bedrock` is the city's
/// bedrock map, `mapped_rock` the mapped bare rock at game (x, z) and
/// `mapped_boxes` its areas' boxes (game x0, z0, x1, z1), sites whatever the
/// bedrock map or the image say there.
pub fn outcrops(lib: &Library, w: &World, dem: &Dem, bedrock: &eo_data::lithology::LithologyMap, mapped_rock: &(dyn Fn(f64, f64) -> bool + Sync), mapped_boxes: &[[f64; 4]]) -> Vec<Piece> {
    if AERIAL.get().is_none() {
        eprintln!("hill outcrops: no aerial reader installed; none placed");
        return vec![];
    }
    let frame = eo_data::Frame::sf();
    let (mlat, mlon) = eo_farshore::survey::metres_per_degree(frame.center_lat);
    // the bedrock cells (surveyed plan, CELL_M apart), the sites' candidates
    let cells: Vec<(f64, f64, RockClass)> = bedrock.cells().into_iter().filter(|c| c.2.bedrock()).collect();
    let mut out: Vec<Piece> = vec![];
    {
        let lattice = SITE_LATTICE_M;
        // the lattice's nodes over the bedrock cells, each once
        let mut nodes: Vec<(i64, i64, RockClass)> = vec![];
        let mut seen = std::collections::HashSet::new();
        let reach = (eo_data::lithology::CELL_M / lattice).ceil() as i64;
        for &(px, pz, c) in &cells {
            let (i0, j0) = ((px / lattice).floor() as i64, (pz / lattice).floor() as i64);
            for dj in 0..reach.max(1) {
                for di in 0..reach.max(1) {
                    if seen.insert((i0 + di, j0 + dj)) {
                        nodes.push((i0 + di, j0 + dj, c));
                    }
                }
            }
        }
        // the mapped bare rock's areas, whatever the bedrock map says under them (its class where it
        // maps bedrock, else chert: the city's mapped outcrops are its chert knobs)
        for b in mapped_boxes {
            let (sx0, sz0, sx1, sz1) = (b[0] / w.plan, b[1] / w.plan, b[2] / w.plan, b[3] / w.plan);
            for j in (sz0 / lattice).floor() as i64..=(sz1 / lattice).ceil() as i64 {
                for i in (sx0 / lattice).floor() as i64..=(sx1 / lattice).ceil() as i64 {
                    let (cx, cz) = ((i as f64 + 0.5) * lattice, (j as f64 + 0.5) * lattice);
                    if !mapped_rock(cx * w.plan, cz * w.plan) || !seen.insert((i, j)) {
                        continue;
                    }
                    let c = bedrock.class_at(cx, cz).filter(|c| c.bedrock()).unwrap_or(RockClass::Chert);
                    nodes.push((i, j, c));
                }
            }
        }
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
        let chunk = nodes.len().div_ceil(threads).max(1);
        let found: Vec<Piece> = std::thread::scope(|s| {
            let hs: Vec<_> = nodes
                .chunks(chunk)
                .map(|part| {
                    s.spawn(move || {
                        let mut v = vec![];
                        for &(i, j, class) in part {
                            let u = |k: u32| hash(i, j, 7400 + k);
                            let (lo, span) = (0.25, 0.5);
                            // surveyed plan to game and lon/lat
                            let (sx, sz) = ((i as f64 + lo + span * u(0)) * lattice, (j as f64 + lo + span * u(1)) * lattice);
                            let (x, z) = (sx * w.plan, sz * w.plan);
                            let (lat, lon) = frame.unproject(sx, sz);
                            if in_golden_gate(lon, lat) {
                                continue;
                            }
                            // open ground in the image (no image: not open ground, no rock)
                            let side = AERIAL_BOX_M * w.plan;
                            let mapped = mapped_rock(x, z);
                            let a = match aerial(x, z, side) {
                                Some(a) => a,
                                None if mapped => Aerial { bare: 1.0, soil: 0.0, canopy: 0.0 },
                                None => continue,
                            };
                            let bare = if mapped { 1.0 } else { a.bare };
                            if a.canopy > OPEN_CANOPY_MAX && !mapped {
                                continue;
                            }
                            let Some((slope, down, _)) = dem.slope(lon, lat, 1.5, mlat, mlon) else { continue };
                            // soil (the eroded red-brown and warm classes) is never a face, except on chert,
                            // whose rock is that red-brown itself
                            let soily = class != RockClass::Chert && !mapped && a.soil > 0.5 * a.bare;
                            if soily {
                                continue;
                            }
                            let face = slope >= STEEP_FACE_DEG || (slope >= FACE_DEG && bare >= BARE_SHARE) || (mapped && slope >= MAPPED_FACE_DEG);
                            let (kind, s0) = if face && slope >= FACE_MIN_DEG {
                                // larger on steeper ground (a big face on a moderate bank shows its edge from the side)
                                let big = ((slope - FACE_MIN_DEG) / 20.0).clamp(0.0, 1.0);
                                if u(3) < 0.55 { (FACE_1, (0.8 + 0.7 * big) + (0.4 + 0.6 * big) * u(4)) } else { (FACE_2, (0.8 + 0.7 * big) + (0.5 + 0.8 * big) * u(4)) }
                            } else if face || mapped {
                                // gentler rock: a scanned rock cluster or broken patch, or a stone
                                match (u(3) * 10.0) as u32 {
                                    0..=3 => (COAST_ROCKS, 0.9 + 0.6 * u(4)),
                                    4..=7 => (LAND_ROCKS, 0.9 + 0.6 * u(4)),
                                    _ => (BOULDER, crate::rocks::boulder_d(u(4)).max(0.9) / 1.84),
                                }
                            } else {
                                continue;
                            };
                            let lean = if kind == BOULDER { 0.5 } else { 0.0 };
                            let s = [s0 * w.plan; 3];
                            let k = &lib.kinds[kind as usize];
                            let face_dir = if scan::role(kind) == Role::Face { Some([down[0], -down[1]]) } else { None }; // north is -z
                            let sink = if kind == FACE_1 || kind == FACE_2 { FACE_SINK * s[1] } else { 0.0 };
                            // (half the bluffs' jitter: a face turned off its slope shows its edge)
                            let twist = if face_dir.is_some() { [(2.0 * u(11) - 1.0) * 0.5 * place::FACE_YAW_JITTER_DEG.to_radians(), (2.0 * u(12) - 1.0) * 0.5 * place::FACE_ROLL_JITTER_DEG.to_radians()] } else { [0.0, 0.0] };
                            let tone = rock_srgb(class).map(|c| eo_data::roof_colours::srgb_to_linear(c as f32 / 255.0) as f64);
                            if let Some((pos, rot, _)) = place::seat_capped(lib, kind, w, x, z, s, face_dir, u(8) * std::f64::consts::TAU, lean, sink, twist, FACE_SEAT) {
                                v.push(Piece { kind, pos, rot, scale: s, tint: place::tint_to(k, tone, 0.85 + 0.3 * u(9)), wet: 0.0, source: Source::Hill });
                            }
                        }
                        v
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        out.extend(found);
    }
    // the talus below the faces: a stone or a broken patch laid down the slope from some
    let talus: Vec<Piece> = out
        .iter()
        .enumerate()
        .filter_map(|(n, f)| {
            let u = |k: u32| hash(n as i64, (f.pos[0] * 4.0) as i64 ^ (f.pos[2] * 4.0) as i64, 7500 + k);
            if u(0) >= TALUS_CHANCE {
                return None;
            }
            // down the slope: the drawn ground's fall line round the face
            let (gx, gz) = (f.pos[0], f.pos[2]);
            let h = |x: f64, z: f64| (w.ground)(x, z);
            let (e, ww, n_, s_) = (h(gx + 2.0, gz)?, h(gx - 2.0, gz)?, h(gx, gz - 2.0)?, h(gx, gz + 2.0)?);
            let (dx, dz) = (-(e - ww), -(s_ - n_));
            let l = dx.hypot(dz).max(1e-6);
            let d = (TALUS_M.0 + (TALUS_M.1 - TALUS_M.0) * u(1)) * w.plan;
            let a = (u(2) - 0.5) * 1.2;
            let (c, sn) = (a.cos(), a.sin());
            let (ux, uz) = ((dx / l) * c - (dz / l) * sn, (dx / l) * sn + (dz / l) * c);
            let (x, z) = (gx + ux * d, gz + uz * d);
            let (kind, s0, lean) = match (u(3) * 3.0) as u32 {
                0 => (LAND_ROCKS, 0.45 + 0.35 * u(4), 0.0),
                _ => (BOULDER, crate::rocks::boulder_d(u(4)) / 1.84, 0.5),
            };
            let s = [s0 * w.plan; 3];
            let k = &lib.kinds[kind as usize];
            let (pos, rot, _) = place::seat(lib, kind, w, x, z, s, None, u(5) * std::f64::consts::TAU, lean, 0.0)?;
            // its rock's tone (the bedrock map's class where it lies), on its own scan
            let class = bedrock.class_at(x / w.plan, z / w.plan).filter(|c| c.bedrock()).unwrap_or(RockClass::Sandstone);
            let tone = rock_srgb(class).map(|c| eo_data::roof_colours::srgb_to_linear(c as f32 / 255.0) as f64);
            Some(Piece { kind, pos, rot, scale: s, tint: place::tint_to(k, tone, 0.85 + 0.3 * u(6)), wet: 0.0, source: Source::Hill })
        })
        .collect();
    out.extend(talus);
    out
}

/// The mapped bare rock's areas (OSM `natural=bare_rock` greens), as game boxes (x0, z0, x1, z1).
pub fn mapped_boxes(repo_root: &std::path::Path) -> Vec<[f64; 4]> {
    crate::plaza::load_areas(repo_root)
        .map(|(greens, _)| {
            greens
                .iter()
                .filter(|g| crate::ground_cover::surface_tone(g) == Some("bareRock"))
                .map(|g| g.polygon.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |m, p| [m[0].min(p[0]), m[1].min(p[1]), m[2].max(p[0]), m[3].max(p[1])]))
                .collect()
        })
        .unwrap_or_default()
}

/// The hills' pieces on the ground `w` sees: the DEM, the bedrock map and the mapped rock's
/// areas read here (None when the DEM or the bedrock map is missing).
pub fn place_all(lib: &Library, w: &World, repo_root: &std::path::Path, mapped_rock: &(dyn Fn(f64, f64) -> bool + Sync)) -> Option<Vec<Piece>> {
    let t = std::time::Instant::now();
    let bedrock = eo_data::DataPaths::discover().ok().and_then(|p| eo_data::lithology::LithologyMap::load(&p).map_err(|e| eprintln!("hill outcrops: bedrock map not read ({e:#})")).ok());
    let (Some(dem), Some(bed)) = (Dem::open(), bedrock) else {
        eprintln!("hill outcrops: no 1 m DEM or bedrock map; none placed");
        return None;
    };
    let pieces = outcrops(lib, w, &dem, &bed, mapped_rock, &mapped_boxes(repo_root));
    eprintln!("hill outcrops: {} scanned pieces on the hills in {:.0} ms", pieces.len(), t.elapsed().as_secs_f64() * 1e3);
    Some(pieces)
}

/// (eo-app, once the scene has installed the drawn ground) The hills' pieces placed again on the
/// drawn ground `ground` (the detail ground where it has one, else the 8 m terrain as drawn) and
/// published in place of the landmark build's; nothing pending afterwards, whatever happens.
pub fn late_pass(repo_root: &std::path::Path, ground: &(dyn Fn(f64, f64) -> Option<f64> + Sync), mapped_rock: &(dyn Fn(f64, f64) -> bool + Sync)) {
    if !scan::hills_pending() {
        return;
    }
    let Some(lib) = scan::library() else {
        scan::set_hills_pending(false);
        return;
    };
    let (plan, hs) = (eo_data::groundscale::scale(), eo_data::groundscale::height_scale());
    let signed = |_: f64, _: f64| -1.0e4; // inland: the shore field plays no part in the hills' seats
    let w = World { ground, signed: &signed, sea: eo_water::SEA_LEVEL_Y, tide: [0.0, 0.0], plan, hs };
    match place_all(&lib, &w, repo_root, mapped_rock) {
        Some(p) => scan::replace_hill_pieces(p),
        None => scan::set_hills_pending(false),
    }
}
