//! The landmark set: every registered landmark this port builds, as the
//! page's main draws (landmark-generators.js `buildLandmark`: the generator's
//! groups merged by the registry's `draw` mapping, each draw with its own
//! material), its far item and its underwater skirt, and the set's distance
//! rules (landmark-set.js: near meshes while the camera is within `nearM` of
//! the landmark's box, hysteresis `hysteresisM`; the far item otherwise; the
//! skirt shown with the near meshes).

use crate::data::{LandmarksJs, Material};
use crate::geometry::{Frozen, GroundGrid, MeshArrays};
use crate::suspension::DeckStandards;
use crate::{Built, Collider, LandmarkLight, bay_bridge, golden_gate};
use anyhow::Result;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Drawn while the camera is within the near distance of the landmark (main draws, skirt).
    Near,
    /// Drawn otherwise (the far item).
    Far,
    /// Always drawn (the pier deck and sheds: one small mesh each in the JS).
    Always,
}

/// A city structure that is not a registered landmark (the Ferry Plaza pier
/// deck and its sheds): no near/far switch, no fog cap, no shadow (as the JS's).
#[derive(Clone, Debug)]
pub struct Structure {
    pub name: String,
    /// Casts a sun shadow (the JS mesh's castShadow).
    pub cast: bool,
    /// Laid in the street surface's concrete (the plaza paving: the JS lays it in the street's path layer).
    pub footway: bool,
    /// drawn in this kit material (the kit's textured finish; `None`: vertex colour).
    pub kit: Option<String>,
    pub material: Material,
    pub mesh: Frozen,
}

#[derive(Clone, Debug)]
pub struct Draw {
    /// The draw key (the registry's material key it is drawn with).
    pub key: String,
    pub role: Role,
    /// The registry's material for the key; the far item and the skirt take
    /// the far batch's (`landmark-set.js`: roughness 0.8, metalness 0.1, vertex colours).
    pub material: Material,
    pub mesh: Frozen,
}

#[derive(Clone, Debug)]
pub struct Landmark {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The noise origin of its surfaces (the registry's anchor, y 0).
    pub anchor: [f64; 2],
    /// Its near reach: the union of the main draws' bounds (the set's `entry.box`).
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    pub draws: Vec<Draw>,
    pub colliders: Vec<Collider>,
    pub stats: serde_json::Value,
    pub build_ms: f64,
}

#[derive(Clone, Debug, Default)]
pub struct LandmarkSet {
    pub landmarks: Vec<Landmark>,
    /// The bridges' night lights (lights.rs).
    pub lights: Vec<LandmarkLight>,
    /// The towers' aviation lights (towers2.rs, skyline.rs); the renderer gathers them with `lights`.
    pub tower_lights: Vec<LandmarkLight>,
    /// The citywide quay wall (shore.rs), chunked.
    pub walls: Vec<crate::shore::WallChunk>,
    /// The seawall material (renderer.js: 0x928d84, roughness 0.93, metalness 0).
    pub wall_material: Option<Material>,
    pub structures: Vec<Structure>,
    pub promenade_surfaces: Vec<crate::promenade::Surfaces>,
    pub build_ms: f64,
    pub walls_ms: f64,
    /// the terrain press of the built waterfront (None with the waterfront off).
    pub press: Option<std::sync::Arc<crate::waterfront::ShorePress>>,
    /// the shore edges' classification.
    pub census: Option<crate::waterfront::EdgeCensus>,
    /// pier decks and piles built.
    pub piers_built: (usize, usize),
    municipal_feet: Vec<(usize, crate::waterfront::PileFeet)>,
    /// the harbours' floating docks (floating_docks.rs) as planned: where the
    /// boats tie up. Empty with `floating_docks` off.
    pub docks: Vec<crate::floating_docks::Dock>,
    /// floating docks and their guide piles built, and their length (m).
    pub docks_built: (usize, usize, f64),
    /// the berths along the floating docks (`floating_docks::plan_berths`), where the boats lie.
    pub berths: Vec<crate::floating_docks::Berth>,
    /// the walkable landmark decks drawn (access.rs; None with `Options::access` off).
    pub access: Option<std::sync::Arc<crate::access::AccessSet>>,
}

/// what the set builds beyond the first landmarks slice. All on
/// by default; `eo-app --no-landmarks2` turns all of it off (the A/B).
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The waterfront (waterfront.rs): the shore-edge rule, piers, the apron, the terrain press.
    pub waterfront: bool,
    pub cove_detail: bool,
    /// The harbours' floating docks (floating_docks.rs), with the waterfront.
    pub floating_docks: bool,
    /// The registered landmark buildings (Transamerica, Salesforce, Coit, Sutro, City Hall).
    pub buildings: bool,
    /// The bridges' night lights.
    pub lights: bool,
    /// The Golden Gate's detail pass (golden_gate.rs): the photographed paint
    /// tone, fluted tower legs, framed portal struts, full arch pylons, lamp
    /// standards. A preview, off by default; `eo-app --bridge-detail` turns it on.
    pub bridge_detail: bool,
    /// The landmark buildings' second generators (towers2.rs; `eo-app
    /// --no-towers2` builds them as the JS export's generators do).
    pub towers2: bool,
    /// The named skyline towers (skyline.rs; `eo-app --no-skyline`: the kit draws them).
    pub skyline: bool,
    /// The landmarks' walkable decks (access.rs): their floors, railings,
    /// glazing and lamps, and the tower bands they open (`--no-landmark-access`).
    pub access: bool,
}

impl Default for Options {
    /// Everything on but the previews (`cove_detail`, `bridge_detail`) and
    /// the switches `eo-app` sets in the environment before any thread
    /// starts (`--no-towers2`: `EO_TOWERS2=off`, `--no-skyline`: `EO_SKYLINE=off`).
    fn default() -> Self {
        let on = |var: &str| std::env::var(var).map(|v| v != "off").unwrap_or(true);
        Self { cove_detail: false, waterfront: true, floating_docks: true, buildings: true, lights: true, bridge_detail: false, towers2: on("EO_TOWERS2"), skyline: on("EO_SKYLINE"), access: true }
    }
}

impl Options {
    /// As the first landmarks slice built the set.
    pub fn off() -> Self {
        Self { cove_detail: false, waterfront: false, floating_docks: false, buildings: false, lights: false, bridge_detail: false, towers2: false, skyline: false, access: false }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PlazaReport {
    pub plazas: usize,
    pub skipped: Vec<(String, &'static str, f64)>,
    pub children: usize,
    pub pieces: usize,
    pub covered_by_street: usize,
    pub triangles: usize,
    pub ms: f64,
    /// (reading the greens, the plan, the footprints near the plazas, the cut pieces, laying them) in ms.
    pub stage_ms: [f64; 5],
}

/// the building records the landmark buildings take over
/// (landmark-registry.js `claims`, drawn by the landmark instead of the kit)
/// and those they exclude (`excludes`: not buildings at all, Sutro Tower's
/// lattice footprints), by the source id without its `sf-building-` prefix.
///
/// The Ferry Building's record (ferry_building.rs) is claimed while that
/// feature is on (`--no-ferry-building` keeps its box).
pub fn claimed_records() -> std::collections::HashSet<String> {
    let js = LandmarksJs::get();
    let mut out: std::collections::HashSet<String> = crate::towers::BUILDING_IDS
        .iter()
        .filter_map(|id| js.registry.get(*id))
        .flat_map(|r| r.claims.iter().chain(r.excludes.iter()))
        .map(|s| s.trim_start_matches("sf-building-").to_string())
        .collect();
    if crate::features::ferry_building() {
        out.insert(crate::ferry_building::RECORD.to_string());
    }
    out
}

/// `claimed_records`, with the skyline towers' records when `opts.skyline`
/// (skyline.rs draws them; the kit and the far boxes do not).
pub fn claimed_records_with(opts: Options) -> std::collections::HashSet<String> {
    let mut c = if opts.buildings { claimed_records() } else { Default::default() };
    if opts.skyline {
        c.extend(crate::skyline::claims());
    }
    c
}

/// The JS's own `roughness` / `metalness` for a city structure's material.
pub fn plain_material(roughness: f64, metalness: f64, env: &str) -> Material {
    Material { roughness, metalness, env_class: env.into(), surface: None, draw: None, wire: false, surface_options: None }
}

/// The quay guard with its second pass (waterfront.rs) as a landmark: its
/// rails and posts near (casting) and as its far item, its pickets a wire
/// draw near (the wire pass fades them to their coverage), all in the kit's
/// painted steel (#6e7478, roughness 0.5) as vertex colour.
fn railing_landmark_of(solid: Frozen, pickets: Frozen) -> Landmark {
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in solid.positions.chunks_exact(3).chain(pickets.positions.chunks_exact(3)) {
        for k in 0..3 {
            mn[k] = mn[k].min(p[k]);
            mx[k] = mx[k].max(p[k]);
        }
    }
    let paint = plain_material(0.5, 0.0, "painted-metal");
    let mut draws = vec![
        Draw { key: "rails".into(), role: Role::Near, material: paint.clone(), mesh: solid.clone() },
        Draw { key: "rails-far".into(), role: Role::Far, material: paint.clone(), mesh: solid },
    ];
    if !pickets.is_empty() {
        draws.push(Draw { key: "pickets".into(), role: Role::Near, material: Material { wire: true, ..paint }, mesh: pickets });
    }
    Landmark {
        id: "quay-railing".into(),
        name: "The Embarcadero quay guard".into(),
        version: "eo-native-quay-guard-v2".into(),
        anchor: [((mn[0] + mx[0]) / 2.0) as f64, ((mn[2] + mx[2]) / 2.0) as f64],
        box_min: mn,
        box_max: mx,
        draws,
        colliders: Vec::new(),
        stats: serde_json::json!({}),
        build_ms: 0.0,
    }
}

/// A frozen mesh with one flat face normal per triangle (no shared vertices).
pub fn flat_mesh(name: &str, positions: &[[f64; 3]], indices: &[u32], colour: [f64; 3]) -> Frozen {
    let mut f = Frozen { name: name.into(), min_y: f64::INFINITY, max_y: f64::NEG_INFINITY, ..Default::default() };
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        for p in [a, b, c] {
            f.indices.push((f.positions.len() / 3) as u32);
            f.positions.extend(p.iter().map(|x| *x as f32));
            f.normals.extend(n.iter().map(|x| (*x / l) as f32));
            f.colors.extend(colour.iter().map(|x| *x as f32));
            f.min_y = f.min_y.min(p[1]);
            f.max_y = f.max_y.max(p[1]);
        }
    }
    f
}

/// The shore's rock meshes (rocks.rs).
struct ShoreRocks {
    /// The revetment's armour stone and its bed, and the stone count.
    armour: Option<(crate::geometry::MeshArrays, usize)>,
    /// The boulders that cast a shadow (`rocks::CAST_MIN_D_M` and over), those
    /// that do not, and the stone count.
    boulders: Option<(crate::geometry::MeshArrays, crate::geometry::MeshArrays, usize)>,
    /// Load and build times, ms: the shore field and ground cover, the armour, the boulders.
    ms: [f64; 3],
    /// The scanned rock placed (rocks/scan.rs), its counts by source, and ms placing it.
    scan: Option<(usize, String, f64)>,
}

/// The shore's rock meshes (rocks.rs): the revetment's armour stone on the
/// Fort Point and Marine Drive wall edges, and the boulders (bare rock, cliff
/// toes, steep faces over the sea) on the drawn ground (the surveyed ground
/// pressed as the carpet is: the beach foreshore and the rings' seabed).
fn shore_rocks(
    repo_root: &std::path::Path,
    height_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync),
    edges: &[crate::waterfront::ShoreEdge],
    crest_at: &dyn Fn(f64, f64) -> f64,
    sea: f64,
    riprap: bool,
    boulders: bool,
) -> ShoreRocks {
    use crate::geometry::MeshArrays;
    use crate::ground_cover::{MHHW_OVER_MSL_M, MHW_OVER_MSL_M};
    use crate::rocks;
    let (plan, hs) = (eo_data::groundscale::scale(), eo_data::groundscale::height_scale());
    let tide = [MHHW_OVER_MSL_M, MHW_OVER_MSL_M].map(|v| v * hs);
    // the scanned rock (rocks/scan.rs): the library, waiting for its load (started with the build)
    let sw = rocks::scan::switches();
    let lib = if sw.scan { rocks::scan::library() } else { None };
    // the shore field from the cache main.rs's water load writes (read, not baked again, once it is there)
    let t = Instant::now();
    let field = if boulders || lib.is_some() { eo_water::load(repo_root, crate::features::cache_dir().as_deref()).ok() } else { None };
    let field_ms = t.elapsed().as_secs_f64() * 1e3;
    // the scans are placed only with both the library and the field: without either, round 2's meshes.
    // The Golden Gate's survey features only with `EO_SURVEY_ROOT` (the library alone may come
    // from `external/survey`, for the hills' rock: rocks/hills.rs, `--no-hill-outcrops`).
    let gg_survey = eo_farshore::survey::Root::discover().is_some();
    let scans_on = lib.is_some() && field.is_some() && gg_survey;
    let hills_on = lib.is_some() && field.is_some() && rocks::hills::switch_on();
    // the armour: quarried granite.
    let t = Instant::now();
    let mut armour_stones: Vec<rocks::Stone> = vec![];
    let armour = riprap.then(|| {
        let list: Vec<([f64; 2], [f64; 2], [f64; 2])> = edges.iter().filter(|e| e.pier.is_none() && rocks::on_revetment([(e.a[0] + e.b[0]) / 2.0, (e.a[1] + e.b[1]) / 2.0], plan)).map(|e| (e.a, e.b, e.land)).collect();
        let stones = rocks::revetment(&list, crest_at, sea, plan, hs, &|u| rocks::stone_tone(rocks::ARMOUR_ALBEDO, u));
        let mut m = MeshArrays::new("shore-rocks:armour");
        rocks::armour_bed(&mut m, &list, crest_at, sea, plan, hs, rocks::ARMOUR_ALBEDO);
        if scans_on && sw.riprap {
            // the stones are drawn as scans (`--no-scan-riprap`: as round 2's quarried blocks)
            armour_stones = stones.clone();
        } else {
            for s in &stones {
                rocks::stone_mesh(&mut m, s, sea, tide);
            }
        }
        (m, stones.len())
    });
    let armour_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let cover = if boulders { crate::ground_cover::GroundCover::load(repo_root).ok() } else { None };
    let load_ms = field_ms + t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let stones = match (&field, &cover) {
        (Some(w), Some(cover)) => {
            let f = &w.field;
            let ground = |x: f64, z: f64| height_at(x, z).map(|y| f.press_ground(x, z, y));
            let slope = |x: f64, z: f64| match (ground(x - 6.0, z), ground(x + 6.0, z), ground(x, z - 6.0), ground(x, z + 6.0)) {
                (Some(a), Some(b), Some(c), Some(d)) => (b - a).hypot(d - c) / 12.0,
                _ => 0.0,
            };
            let bare_at = |x: f64, z: f64| cover.tone_name_at(x, z) == Some("bareRock");
            let near_bare = |x: f64, z: f64| {
                bare_at(x, z)
                    || (0..8).any(|k| {
                        let a = k as f64 * std::f64::consts::FRAC_PI_4;
                        [0.5, 1.0].iter().any(|r| bare_at(x + a.cos() * rocks::BARE_SPREAD_M * r, z + a.sin() * rocks::BARE_SPREAD_M * r))
                    })
            };
            // the lowest drawn ground under the footprint: the detail ground (eo_data::detail_ground,
            // the 2 m relief where it moved the ground) at each point where it has one
            let ground_min = |x: f64, z: f64, r: f64| [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)].iter().filter_map(|(dx, dz)| eo_data::detail_ground::height(x + dx, z + dz).or_else(|| ground(x + dx, z + dz))).fold(f64::INFINITY, f64::min);
            // a cliff over the point: ground `TOE_DROP_MIN_M` (surveyed) higher within `TOE_REACH_M`
            let under_cliff = |x: f64, z: f64, g: f64| {
                (0..8).any(|k| {
                    let a = k as f64 * std::f64::consts::FRAC_PI_4;
                    ground(x + a.cos() * rocks::TOE_REACH_M, z + a.sin() * rocks::TOE_REACH_M).is_some_and(|h| h - g >= rocks::TOE_DROP_MIN_M * hs)
                })
            };
            // the candidate boxes: the beaches and the mapped bare rock, padded by the rocks' reach
            let mut boxes: Vec<[f64; 4]> = crate::shore::load_beaches(repo_root).unwrap_or_default().iter().map(|b| b.polygon.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |m, p| [m[0].min(p[0]), m[1].min(p[1]), m[2].max(p[0]), m[3].max(p[1])])).collect();
            if let Ok((greens, _)) = crate::plaza::load_areas(repo_root) {
                boxes.extend(greens.iter().filter(|g| crate::ground_cover::surface_tone(g) == Some("bareRock")).map(|g| g.polygon.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |m, p| [m[0].min(p[0]), m[1].min(p[1]), m[2].max(p[0]), m[3].max(p[1])])));
            }
            let pad = rocks::INLAND_REACH_M;
            let l = rocks::LATTICE_M;
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for bx in boxes {
                for j in ((bx[1] - pad) / l).floor() as i64..=((bx[3] + pad) / l).ceil() as i64 {
                    for i in ((bx[0] - pad) / l).floor() as i64..=((bx[2] + pad) / l).ceil() as i64 {
                        if !seen.insert((i, j)) || rocks::hash(i, j, 101) >= 1.6 * rocks::DENSITY_BARE.max(rocks::DENSITY_TOE).max(rocks::DENSITY_OUTCROP) {
                            continue;
                        }
                        let (x, z) = ((i as f64 + 0.5) * l, (j as f64 + 0.5) * l);
                        let s = f.signed_at(x, z);
                        if !(-rocks::INLAND_REACH_M..=rocks::BARE_SPREAD_M).contains(&s) {
                            continue;
                        }
                        let Some(g) = ground(x, z) else { continue };
                        let sl = slope(x, z);
                        let bare = near_bare(x, z);
                        let toe = !bare && sl <= rocks::SAND_SLOPE_MAX && cover.beach_sand(x, z, s.abs()) && [(5.0, 0.0), (-5.0, 0.0), (0.0, 5.0), (0.0, -5.0)].iter().any(|(dx, dz)| slope(x + dx, z + dz) > rocks::SAND_SLOPE_MAX) && under_cliff(x, z, g);
                        let Some(zn) = rocks::zone(s, g - sea, sl, bare, toe) else { continue };
                        if let Some(st) = rocks::boulder(i, j, x, z, zn, g - sea > tide[0], &ground_min, plan, &|u| rocks::stone_tone(rocks::BOULDER_ALBEDO, u)) {
                            out.push(st);
                        }
                    }
                }
            }
            Some(out)
        }
        _ => None,
    };
    let boulders_ms = t.elapsed().as_secs_f64() * 1e3;
    // the scanned rock: round 2's stones as scans, the intertidal field, the slopes' faces and
    // outcrops; published for the renderer, the surf's rocks handed to the water.
    let t = Instant::now();
    let mut scan_report = None;
    let mut boulder_stones = stones;
    if let (Some(lib), Some(w), true) = (&lib, &field, scans_on || hills_on) {
        use rocks::{place, scan};
        let f = &w.field;
        // the drawn ground: the detail ground where one is set, else the terrain as the scene
        // lays it (pressed at the city's shore; the far land north of the strait, more than
        // 60 m out from every ring and 2 m over the sea, unpressed: eo-app ground_tone `far_land`)
        let ground = |x: f64, z: f64| {
            eo_data::detail_ground::height(x, z).or_else(|| {
                height_at(x, z).map(|y| if f.signed_at(x, z) > rocks::place::FAR_LAND_SHORE_M && y > sea + rocks::place::FAR_LAND_ABOVE_SEA_M { y } else { f.press_ground(x, z, y) })
            })
        };
        let signed = |x: f64, z: f64| f.signed_at(x, z);
        let world = place::World { ground: &ground, signed: &signed, sea, tide, plan, hs };
        let mut pieces = vec![];
        let mut shifts = vec![];
        if scans_on {
            if let Some(st) = boulder_stones.take() {
                pieces.extend(place::from_stones(lib, &world, &st, scan::Source::Boulder, sw.geology, sw.cliff));
            }
            pieces.extend(place::from_stones(lib, &world, &armour_stones, scan::Source::Armour, sw.geology, false));
            if let Some(sv) = place::Survey::open() {
                if sw.surf {
                    pieces.extend(place::surf_field_with(lib, &world, &sv, sw.geology, &mut shifts));
                }
                pieces.extend(place::slopes(lib, &world, &sv, sw.geology, sw.cliff));
            }
            if crate::flora2::flora2().boulders {
                // the second flora pass: the earlier rounds' Baker stones into its boulder clusters.
                place::cluster_shore_stones(lib, &world, &mut pieces);
            }
            if crate::flora::flora().serpentinite {
                // the flora pass: Baker's and Marshall's serpentinite boulders (rocks/place.rs).
                pieces.extend(place::serpentinite_boulders(lib, &world));
            }
        }
        if hills_on {
            // (`--no-hill-outcrops`) the city's hills' rock: faces, stones and cliffs where the bedrock
            // map has bedrock, the image bare ground and the 1 m DEM a steep slope (rocks/hills.rs),
            // here on the 8 m grid for their claims; eo-app places them again on the drawn ground
            let mapped = |x: f64, z: f64| cover.as_ref().is_some_and(|c| c.tone_name_at(x, z) == Some("bareRock"));
            if let Some(hp) = rocks::hills::place_all(lib, &world, repo_root, &mapped) {
                if !hp.is_empty() {
                    scan::set_hills_pending(true);
                }
                pieces.extend(hp);
            }
        }
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for p in &pieces {
            *counts.entry(format!("{:?}", p.source)).or_default() += 1;
        }
        let obstacles = if sw.surf { place::obstacles(lib, &world, &pieces) } else { vec![] };
        let n_obs = obstacles.len();
        if !obstacles.is_empty() {
            eo_water::obstacles::set(obstacles);
        }
        let n = pieces.len();
        if let Some(path) = std::env::var_os("EO_ROCK_DUMP") {
            // a diagnostic list of the pieces (lon, lat, scan, source, scale), for a map over the imagery
            let rows: Vec<String> = pieces.iter().map(|p| {
                let (lon, lat) = place::lon_lat(p.pos[0], p.pos[2], plan);
                // and the drawn shore field's signed distance and ground over the sea there
                format!("{lon:.7},{lat:.7},{},{:?},{:.3},{:.3},{:.2},{:.2}", scan::KINDS[p.kind as usize], p.source, p.scale[0], p.wet, signed(p.pos[0], p.pos[2]), ground(p.pos[0], p.pos[2]).map(|g| g - sea).unwrap_or(f64::NAN))
            }).collect();
            std::fs::write(path, rows.join("\n")).ok();
        }
        let claims = scan::Claims::new(crate::hill_rock::ROCK_PITCH_M, place::claim_discs(lib, &pieces));
        // the intertidal rocks' moves onto the drawn shore (drawn m): how far the surveyed shore is from the drawn
        let mut moved: Vec<f64> = shifts.iter().filter(|d| !d[0].is_nan()).map(|d| d[0].hypot(d[1])).collect();
        moved.sort_by(f64::total_cmp);
        let pct = |f: f64| moved.get(((moved.len().max(1) - 1) as f64 * f).round() as usize).copied().unwrap_or(0.0);
        let why = |k: f64| shifts.iter().filter(|d| d[0].is_nan() && d[1] == k).count();
        let shift_note = format!(
            "{} of {} intertidal rocks seated on the drawn shore (moved median {:.1} m, p90 {:.1} m); kept in place: {} with no surveyed waterline within {} m, {} with no seat on the drawn shore",
            moved.len(),
            shifts.len(),
            pct(0.5),
            pct(0.9),
            why(0.0),
            place::SHORE_SEARCH_M,
            why(1.0)
        );
        let n_claims = claims.len();
        scan::publish(scan::Placed { pieces, claims });
        scan_report = Some((n, format!("{counts:?}, {n_obs} surf obstacles, {n_claims} hill-rock footprints; {shift_note}"), 0.0));
    }
    let boulders = boulder_stones.map(|stones| {
        let (mut cast, mut small) = (MeshArrays::new("shore-rocks:boulders"), MeshArrays::new("shore-rocks:boulders"));
        for s in &stones {
            rocks::stone_mesh(if s.diameter() >= rocks::CAST_MIN_D_M { &mut cast } else { &mut small }, s, sea, tide);
        }
        (cast, small, stones.len())
    });
    let scan = scan_report.map(|(n, c, _)| (n, c, t.elapsed().as_secs_f64() * 1e3));
    ShoreRocks { armour, boulders, ms: [load_ms, armour_ms, boulders_ms], scan }
}

/// The far batch's material (landmark-set.js `admit`).
pub fn far_material() -> Material {
    Material { roughness: 0.8, metalness: 0.1, env_class: "facade-masonry".into(), surface: None, draw: None, wire: false, surface_options: None }
}

/// `mergeArrays`: several frozen records drawn as one.
pub fn merge(name: &str, list: &[&Frozen]) -> Frozen {
    let list: Vec<&&Frozen> = list.iter().filter(|r| !r.is_empty()).collect();
    let wire = !list.is_empty() && list.iter().all(|r| r.axes.is_some());
    let mut out = Frozen { name: name.into(), min_y: f64::INFINITY, max_y: f64::NEG_INFINITY, ..Default::default() };
    if wire {
        out.axes = Some(vec![]);
        out.dirs = Some(vec![]);
        out.radii = Some(vec![]);
    }
    for r in list {
        let base = (out.positions.len() / 3) as u32;
        out.positions.extend_from_slice(&r.positions);
        out.normals.extend_from_slice(&r.normals);
        out.colors.extend_from_slice(&r.colors);
        out.indices.extend(r.indices.iter().map(|i| i + base));
        out.pavement_quads.extend(r.pavement_quads.iter().map(|i| i + base));
        out.marking_quads.extend(r.marking_quads.iter().map(|i| i + base));
        if wire {
            out.axes.as_mut().unwrap().extend_from_slice(r.axes.as_ref().unwrap());
            out.dirs.as_mut().unwrap().extend_from_slice(r.dirs.as_ref().unwrap());
            out.radii.as_mut().unwrap().extend_from_slice(r.radii.as_ref().unwrap());
        }
        out.min_y = out.min_y.min(r.min_y);
        out.max_y = out.max_y.max(r.max_y);
    }
    out
}

/// The ground grid of a registered landmark, sampled through `height_at`.
pub fn ground_of(js: &LandmarksJs, id: &str, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> GroundGrid {
    js.registry[id].ground.sampled(height_at)
}

/// how far a landmark moves on the scaled ground plan,
/// `(s - 1) * anchor` (a landmark building: built at its real site, real
/// size, then moved whole); [0, 0] at scale 1 and for the bridges, whose plan
/// is scaled with the ground (`LandmarksJs::apply_ground_scale`).
pub fn ground_scale_shift(js: &LandmarksJs, id: &str) -> [f64; 2] {
    let s = eo_data::groundscale::scale();
    if s == 1.0 || crate::data::PLAN_SCALED_LANDMARKS.contains(&id) {
        return [0.0, 0.0];
    }
    let a = &js.registry[id].anchor;
    [(s - 1.0) * a["x"].as_f64().unwrap_or(0.0), (s - 1.0) * a["z"].as_f64().unwrap_or(0.0)]
}

impl Landmark {
    /// move the whole landmark by `d` (x, z): draws, wire axes, colliders, bounds, noise origin.
    pub fn translate(&mut self, d: [f64; 2]) {
        if d == [0.0, 0.0] {
            return;
        }
        let (dx, dz) = (d[0] as f32, d[1] as f32);
        for dr in &mut self.draws {
            for p in dr.mesh.positions.chunks_exact_mut(3) {
                p[0] += dx;
                p[2] += dz;
            }
            if let Some(a) = dr.mesh.axes.as_mut() {
                for p in a.chunks_exact_mut(3) {
                    p[0] += dx;
                    p[2] += dz;
                }
            }
        }
        for c in &mut self.colliders {
            c.x += d[0];
            c.z += d[1];
        }
        self.box_min[0] += dx;
        self.box_min[2] += dz;
        self.box_max[0] += dx;
        self.box_max[2] += dz;
        self.anchor = [self.anchor[0] + d[0], self.anchor[1] + d[1]];
    }
}

/// Build one landmark (`buildLandmark`): the generator, then its draws.
pub fn build_landmark(js: &LandmarksJs, id: &str, ground: &GroundGrid, ca: &DeckStandards, opts: Options) -> Result<Landmark> {
    build_landmark_with(js, id, ground, ca, opts).map(|(l, _)| l)
}

/// `build_landmark`, also with the lights a generator carries: the landmark
/// buildings come from their second generators (towers2.rs) when `opts.towers2`.
pub fn build_landmark_with(js: &LandmarksJs, id: &str, ground: &GroundGrid, ca: &DeckStandards, opts: Options) -> Result<(Landmark, Vec<LandmarkLight>)> {
    build_landmark_opened(js, id, ground, ca, opts, false)
}

/// `build_landmark_with`; `open`: the bands the access decks replace are left
/// out (the bridges' `build_opened`; the buildings' towers.rs, towers2.rs or
/// towers3.rs `build_opened`, whichever generator `opts.towers2` and the
/// passes' switches pick).
pub fn build_landmark_opened(js: &LandmarksJs, id: &str, ground: &GroundGrid, ca: &DeckStandards, opts: Options, open: bool) -> Result<(Landmark, Vec<LandmarkLight>)> {
    build_landmark_at(js, id, ground, ca, opts, open, &|x,z| ground.at(x,z))
}

fn build_landmark_at(js: &LandmarksJs, id: &str, ground: &GroundGrid, ca: &DeckStandards, opts: Options, open: bool, contact: &dyn Fn(f64,f64)->f64) -> Result<(Landmark, Vec<LandmarkLight>)> {
    let t0 = Instant::now();
    // the materials a second generator gives its own groups (in place of the registry's).
    let mut own: Vec<(String, Material)> = Vec::new();
    let mut lights: Vec<LandmarkLight> = Vec::new();
    let built: Built = match id {
        "golden-gate-bridge" => golden_gate::build_at(js, ground, ca, opts.bridge_detail, open, contact),
        "bay-bridge-west" => bay_bridge::build_opened(js, ground, ca, open),
        // the landmark buildings (towers.rs; towers2.rs), opened for the decks with `open`.
        other => match if opts.towers2 { crate::towers2::build_opened(other, &js.buildings[other], ground, open) } else { crate::towers::build_opened(other, &js.buildings[other], ground, open) } {
            Some(o) => {
                own = o.materials;
                lights = o.lights;
                Built { id: other.into(), version: o.version, groups: o.groups, far: o.far, skirt: Frozen::default(), colliders: o.colliders, stats: o.stats }
            }
            None => anyhow::bail!("eo-landmarks: no generator for {other}"),
        },
    };
    let own_material = |k: &str| own.iter().find(|(key, _)| key == k).map(|(_, m)| m.clone());
    let def = &js.registry[id];
    let groups: Vec<&(String, Frozen)> = built.groups.iter().filter(|(_, g)| !g.is_empty()).collect();
    let mut keys: Vec<String> = Vec::new();
    for (key, _) in &groups {
        let target = def.materials.get(key).and_then(|m| m.draw.clone()).unwrap_or_else(|| key.clone());
        if !keys.contains(&target) {
            keys.push(target);
        }
    }
    let mut draws = Vec::new();
    for target in keys {
        let list: Vec<&Frozen> = groups.iter().filter(|(k, _)| def.materials.get(k).and_then(|m| m.draw.clone()).unwrap_or_else(|| k.clone()) == target).map(|(_, g)| g).collect();
        let mesh = if list.len() == 1 { list[0].clone() } else { merge(&format!("{id}:{target}"), &list) };
        let mut material = own_material(&target).or_else(|| def.materials.get(&target).cloned()).unwrap_or_else(far_material);
        crate::suspension::finish_material(id, &target, &mut material); // the bridges' paint as a dielectric film
        draws.push(Draw { key: target, role: Role::Near, material, mesh });
    }
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for d in &draws {
        for p in d.mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
    }
    if !built.far.is_empty() {
        let mut material = own_material("far").unwrap_or_else(far_material);
        crate::suspension::finish_material(id, "far", &mut material);
        draws.push(Draw { key: "far".into(), role: Role::Far, material, mesh: built.far.clone() });
    }
    if !built.skirt.is_empty() {
        draws.push(Draw { key: "skirt".into(), role: Role::Near, material: far_material(), mesh: built.skirt.clone() });
    }
    let anchor = [def.anchor["x"].as_f64().unwrap_or(0.0), def.anchor["z"].as_f64().unwrap_or(0.0)];
    Ok((Landmark {
        id: id.into(),
        name: def.name.clone(),
        version: built.version,
        anchor,
        box_min: mn,
        box_max: mx,
        draws,
        colliders: built.colliders,
        stats: built.stats,
        build_ms: t0.elapsed().as_secs_f64() * 1e3,
    }, lights))
}

impl LandmarkSet {
    /// Fit the Municipal Pier supports once, before uploading the waterfront.
    pub fn seat_municipal_piles(&mut self, mut bed: impl FnMut(f32, f32) -> f32) {
        for (index, feet) in self.municipal_feet.drain(..) {
            feet.seat(&mut self.structures[index].mesh, &mut bed);
        }
    }

    /// Every landmark this port has a generator for, on the ground `height_at`
    /// (the drawn terrain's sampler; None outside its grid, which the JS reads as 0).
    pub fn build(repo_root: &std::path::Path, height_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync), ca: &eo_data::CaStandards) -> Result<Self> {
        Self::build_with(repo_root, height_at, ca, Options::default())
    }

    /// `build` with the landmarks2 parts chosen by `opts`.
    pub fn build_with(repo_root: &std::path::Path, height_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync), ca: &eo_data::CaStandards, opts: Options) -> Result<Self> {
        let t0 = Instant::now();
        crate::rocks::scan::start(); // the scanned rock library loads beside the build (rocks/scan.rs)
        let js = LandmarksJs::get();
        let std = DeckStandards::load(ca)?;
        let mut ids = vec!["golden-gate-bridge", "bay-bridge-west"];
        if opts.buildings {
            ids.extend(crate::towers::BUILDING_IDS);
        }
        let (built, skyline) = std::thread::scope(|s| {
            // the named skyline towers (skyline.rs) beside the landmarks.
            let sky = opts.skyline.then(|| s.spawn(|| crate::skyline::build_all(repo_root, height_at)));
            // the groves round Coit and Sutro towers (grove.rs; `--no-grove`).
            let grove = (opts.buildings && crate::grove::grove_on()).then(|| s.spawn(|| crate::grove::build_all(repo_root, height_at, ca)));
            let hs: Vec<_> = ids
                .iter()
                .map(|id| {
                    let std = &std;
                    s.spawn(move || {
                        // groundscale (the street size, 1.25 by default): positioning only. The
                        // generator builds at its real site on the ground of its scaled
                        // site, then the whole landmark moves there (real size).
                        let d = ground_scale_shift(js, id);
                        let shifted = |x: f64, z: f64| height_at(x + d[0], z + d[1]);
                        let ground = if d == [0.0, 0.0] { ground_of(js, id, height_at) } else { ground_of(js, id, &shifted) };
                        let (mut l, mut lights) = build_landmark_at(js, id, &ground, std, opts, opts.access && opts.buildings, &|x,z| shifted(x,z).unwrap_or_else(||ground.at(x,z)))?;
                        l.translate(d);
                        for k in &mut lights {
                            k.pos[0] += d[0];
                            k.pos[2] += d[1];
                            k.base[0] += d[0];
                            k.base[2] += d[1];
                        }
                        Ok::<(Landmark, Vec<LandmarkLight>), anyhow::Error>((l, lights))
                    })
                })
                .collect();
            let built = hs.into_iter().map(|h| h.join().expect("landmark build thread panicked")).collect::<Result<Vec<_>>>();
            let mut skyline = sky.map(|h| h.join().expect("skyline build thread panicked"));
            if let Some(g) = grove.map(|h| h.join().expect("grove build thread panicked")) {
                skyline = Some(match (skyline, g) {
                    (Some(Ok((mut a, mut b))), Ok((ga, gl))) => {
                        a.extend(ga);
                        b.extend(gl); // the groves' lights (grove.rs `loop_lamps`, the fourth pass)
                        Ok((a, b))
                    }
                    (None, g) => g,
                    (Some(e @ Err(_)), _) | (Some(Ok(_)), e @ Err(_)) => e,
                });
            }
            (built, skyline)
        });
        let mut tower_lights: Vec<LandmarkLight> = Vec::new();
        let mut landmarks: Vec<Landmark> = Vec::new();
        for (l, lights) in built? {
            landmarks.push(l);
            tower_lights.extend(lights);
        }
        if let Some(sk) = skyline {
            let (sl, lights) = sk?;
            landmarks.extend(sl);
            tower_lights.extend(lights);
        }
        let build_ms = t0.elapsed().as_secs_f64() * 1e3;
        // The shore edge: the citywide quay wall and the Ferry Plaza pier.
        let t1 = Instant::now();
        let sh = &js.shore;
        let boundary = eo_water::load_boundary(repo_root)?;
        let oracle = eo_water::shoreline::Oracle::citywide(&boundary).ok_or_else(|| anyhow::anyhow!("no shoreline rings"))?;
        let beaches = crate::shore::load_beaches(repo_root)?;
        let rule = crate::shore::BeachRule { beaches: &beaches, reach_m: sh.beach_wall_reach_m, wide_reach_m: sh.wide_foreshore_reach_m, wide_sources: &sh.wide_foreshore_sources };
        // renderer.js: crest = heightAt + roadLift + 0.04, foot = SEA_LEVEL_Y - 1.6.
        let top_at = |x: f64, z: f64| height_at(x, z).unwrap_or(0.0) + sh.road_lift + sh.crest_above_road_m;
        let wall_colour = crate::geometry::srgb_hex(sh.wall_srgb);
        let fp = &sh.ferry_pier;
        let lift = crate::data::num(&fp.presentation, "deckLiftM");
        let thick = crate::data::num(&fp.presentation, "deckThicknessM");
        let elev = |x: f64, z: f64| height_at(x, z).unwrap_or(0.0) + sh.road_lift;
        let mut structures = Vec::new();
        if let Some(gate) = landmarks.iter_mut().find(|l| l.id == "golden-gate-bridge") {
            let key = golden_gate::safety_net::DRAW;
            if let Some(index) = gate.draws.iter().position(|d| d.key == key) {
                let fabric = gate.draws.remove(index);
                let mut material = plain_material(0.42, 0.82, "facade-metal");
                material.draw = Some(key.into());
                structures.push(Structure { cast: false, footway: false, kit: None, name: key.into(), material, mesh: fabric.mesh });
            }
        }
        let mut municipal_feet = Vec::new();

        let mut railing_landmark: Option<Landmark> = None;
        let (mut press, mut census, mut piers_built) = (None, None, (0usize, 0usize));
        let (mut docks, mut docks_built, mut berths) = (Vec::new(), (0usize, 0usize, 0.0f64), Vec::new());
        let walls = if !opts.waterfront {
            let walls = crate::shore::city_walls(&sh.seawall, js.sea_level_y, &oracle, height_at, &top_at, js.sea_level_y - sh.bottom_below_sea_m, &rule, wall_colour);
            if let Some(deck) = crate::shore::ferry_pier_deck(&fp.outline, lift, thick, js.sea_level_y, &elev, &|x, z| oracle.is_water(x, z)) {
                let mut pos = deck.top.clone();
                pos.extend_from_slice(&deck.bottom);
                structures.push(Structure { cast: false, footway: false, kit: None, name: format!("ferry-pier-deck-{}", fp.source_way_id), material: plain_material(0.91, 0.02, "sidewalk"), mesh: flat_mesh("ferry-pier-deck", &pos, &deck.indices, crate::geometry::srgb_hex(fp.deck_srgb)) });
            }
            if let Some(sheds) = crate::shore::ferry_pier_sheds(&fp.outline, &fp.sheds, lift, &elev) {
                structures.push(Structure { cast: false, footway: false, kit: None, name: format!("ferry-pier-sheds-{}", fp.source_way_id), material: plain_material(0.88, 0.03, "facade-masonry"), mesh: flat_mesh("ferry-pier-sheds", &sheds.0, &sheds.1, crate::geometry::srgb_hex(fp.shed_srgb)) });
            }
            walls
        } else {
            // the waterfront as built places (waterfront.rs): the
            // shore edges classified (the county-line probe out to 64 m), the
            // piers of the citywide map as decks over the water, the quay wall
            // without its landward side under the apron, and the terrain press.
            use crate::waterfront as wf;
            let mut rings = wf::load_pier_rings(repo_root)?;
            // the piers the citywide map draws as lines (Torpedo Wharf, the Coast Guard walkway)
            let line_rings = if crate::features::shore_ground("pier-lines") { wf::load_pier_line_rings(repo_root, &rings)? } else { Vec::new() };
            let first_line_pier = rings.len();
            rings.extend(line_rings.iter().cloned());
            let piers = wf::plan_piers(&rings, &oracle, height_at, js.sea_level_y, sh.road_lift);
            let (mut edges, mut c) = wf::classify_edges(&oracle, height_at, js.sea_level_y, &rule, &piers);
            if opts.cove_detail { wf::align_municipal_edges(&mut edges, &mut c, &piers); }
            let wall_edges: Vec<[f64; 4]> = edges.iter().filter(|e| e.pier.is_none()).map(|e| [e.a[0], e.a[1], e.b[0], e.b[1]]).collect();
            // The crest on the quay's own top (renderer.js's crest over the highest ground within 8 m).
            // With the guard's second pass the corridor's quay stands on that top at the drawn
            // carpet's level (waterfront.rs `QUAY_APRON_LIFT_M`), not at the JS road lift.
            let corridor_c = eo_data::groundscale::plan(crate::ferry_building::spec().tower.centre_surveyed);
            let corridor_r = wf::RAILING_CORRIDOR_M * eo_data::groundscale::scale();
            let corridor = crate::features::guard_detail() && crate::features::quay_railing();
            let js_lift = sh.road_lift + sh.crest_above_road_m;
            let crest_at = |x: f64, z: f64| {
                let lift = if corridor { wf::quay_lift((x - corridor_c[0]).hypot(z - corridor_c[1]), corridor_r, js_lift) } else { js_lift };
                wf::crest_ground(height_at, x, z) + lift
            };
            let walls = crate::shore::walls_on_edges(&sh.seawall, js.sea_level_y, &|x, z| oracle.is_water(x, z), &wall_edges, &crest_at, js.sea_level_y - sh.bottom_below_sea_m, wall_colour, false);
            let concrete = crate::geometry::srgb_hex(sh.wall_srgb);
            // The apron in the street palette's pedestrian-way tone (`path`: the
            // promenade is Herb Caen Way's paving), the piers in the JS pier deck's.
            let hex = |s: &str| crate::geometry::srgb_hex(u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0x808080));
            let apron = hex(js.open_space.colors.get("path").map(|s| s.as_str()).unwrap_or("#e1e3e4"));
            // the decks the citywide map draws as lines are asphalt (Torpedo Wharf: OSM
            // surface=asphalt): the promenades' asphalt albedo (landmarks-js.json
            // `promenades.structureAlbedo.asphalt`); every other pier the ferry pier's deck tone
            let pier_deck = crate::geometry::srgb_hex(fp.deck_srgb);
            let asphalt = {
                let a = &js.promenades["structureAlbedo"]["asphalt"];
                [0, 1, 2].map(|k| a[k].as_f64().unwrap_or(pier_deck[k]))
            };
            let deck_of = |i: usize| if line_rings.iter().any(|r| r == &piers[i].ring) { asphalt } else { pier_deck };
            let m = wf::build_meshes_with(&edges, &piers, &oracle, &crest_at, js.sea_level_y, wf::COPING_LAND_M, concrete, wf::UNDER_DECK_LINEAR, apron, &deck_of);
            let mut inner_feet = wf::PileFeet::default();
            if opts.cove_detail && let Some(mesh) = wf::municipal_parapet_with_feet(&piers, &mut inner_feet) {
                municipal_feet.push((structures.len(), inner_feet));
                structures.push(Structure { cast: true, footway: false, kit: None, name: "waterfront:municipal-pier-parapet".into(), material: plain_material(0.93, 0.0, "facade-masonry"), mesh });
            }
            piers_built = (m.decks, m.piles);
            // their guard on the water edges (the decks the citywide map draws as lines)
            if !line_rings.is_empty() {
                let mut g = crate::geometry::MeshArrays::new("waterfront:pier-guard");
                for p in piers.iter().filter(|p| line_rings.iter().any(|r| r == &p.ring)) {
                    wf::pier_guard(&mut g, p, &|x, z| oracle.is_water(x, z), crate::geometry::srgb_hex(wf::RAIL_PAINT_SRGB));
                }
                eprintln!("pier lines: {} rings ({} piers before them), guard {} triangles", line_rings.len(), first_line_pier, g.triangles());
                if g.triangles() > 0 {
                    structures.push(Structure { cast: true, footway: false, kit: Some(wf::RAILING_KIT.into()), name: "waterfront:pier-guard".into(), material: plain_material(0.5, 0.0, "painted-metal"), mesh: g.freeze() });
                }
            }
            // the harbours' floating docks: the pier lines over open water, off the decked piers.
            if opts.floating_docks {
                let lines = crate::floating_docks::load_pier_lines(repo_root)?;
                docks = crate::floating_docks::plan_docks(&lines, &oracle, &piers);
                let dm = crate::floating_docks::build_meshes(&docks, js.sea_level_y);
                docks_built = (dm.docks, dm.piles, dm.length_m);
                berths = crate::floating_docks::plan_berths(&docks, &|x, z| oracle.is_water(x, z));
                structures.push(Structure { cast: true, footway: false, kit: None, name: "waterfront:floating-docks".into(), material: plain_material(0.92, 0.0, "facade-masonry"), mesh: dm.mesh });
            }
            structures.push(Structure { cast: false, footway: true, kit: None, name: "waterfront:paving".into(), material: plain_material(0.9, 0.0, "sidewalk"), mesh: m.paving });
            if opts.cove_detail {
                municipal_feet.push((structures.len(), m.municipal_feet));
            }
            structures.push(Structure { cast: true, footway: false, kit: None, name: "waterfront:concrete".into(), material: Material { surface: Some("concrete".into()), ..plain_material(0.92, 0.0, "facade-masonry") }, mesh: m.concrete });
            structures.push(Structure { cast: false, footway: false, kit: None, name: "waterfront:under-deck".into(), material: plain_material(0.95, 0.0, "facade-masonry"), mesh: m.under_deck });
            // The Ferry Plaza pier's sheds (createFerryCard04PierSheds) on its
            // deck, walls and roofs in the kit's corrugated metal and membrane.
            let c0 = crate::geometry::ring_centroid(&fp.outline);
            let deck_y = piers.iter().find(|p| p.contains(c0[0], c0[1])).map(|p| p.deck_y).unwrap_or_else(|| elev(c0[0], c0[1]) + lift);
            let gates = if crate::features::pier_sheds() { wf::ferry_gate_sheds(&fp.outline, &fp.sheds, deck_y) } else { None };
            if let Some(g) = gates {
                // the gate sheds as built pieces (waterfront.rs `ferry_gate_sheds`)
                structures.push(Structure { cast: true, footway: false, kit: Some(wf::SHED_WALL_KIT.into()), name: format!("ferry-gate-sheds-{}", fp.source_way_id), material: plain_material(0.88, 0.03, "facade-masonry"), mesh: g.frames });
                structures.push(Structure { cast: true, footway: false, kit: Some(wf::SHED_ROOF_KIT.into()), name: format!("ferry-gate-shed-roofs-{}", fp.source_way_id), material: plain_material(0.9, 0.0, "facade-masonry"), mesh: g.roofs });
                structures.push(Structure { cast: false, footway: false, kit: None, name: format!("ferry-gate-shed-glass-{}", fp.source_way_id), material: plain_material(0.08, 0.0, "facade-glass"), mesh: g.glass });
            } else if let Some((walls_m, roofs_m)) = wf::ferry_sheds(&fp.outline, &fp.sheds, deck_y) {
                structures.push(Structure { cast: true, footway: false, kit: Some(wf::SHED_WALL_KIT.into()), name: format!("ferry-pier-sheds-{}", fp.source_way_id), material: plain_material(0.88, 0.03, "facade-masonry"), mesh: walls_m });
                structures.push(Structure { cast: true, footway: false, kit: Some(wf::SHED_ROOF_KIT.into()), name: format!("ferry-pier-shed-roofs-{}", fp.source_way_id), material: plain_material(0.9, 0.0, "facade-masonry"), mesh: roofs_m });
            }
            // the guard rail on the quay wall along the Ferry Building corridor.
            if crate::features::quay_railing() {
                let fb = eo_data::groundscale::plan(crate::ferry_building::spec().tower.centre_surveyed);
                let reach = wf::RAILING_CORRIDOR_M * eo_data::groundscale::scale();
                let detail = crate::features::guard_detail();
                // (the pickets' wire record carries the kit finish's colour: the wire pass reads its vertex colour.)
                let paint = crate::geometry::srgb_hex(wf::RAIL_PAINT_SRGB);
                let r = wf::quay_railing_with(&edges, &crest_at, fb, reach, if detail { paint } else { [1.0; 3] }, detail);
                if detail {
                    // a landmark: the rails and posts near and far (in the paint's own colour), the pickets near.
                    if !r.solid.is_empty() {
                        railing_landmark = Some(railing_landmark_of(r.solid, r.pickets));
                    }
                } else if !r.solid.is_empty() {
                    structures.push(Structure { cast: true, footway: false, kit: Some(wf::RAILING_KIT.into()), name: "waterfront:railing".into(), material: plain_material(0.5, 0.0, "painted-metal"), mesh: r.solid });
                }
            }
            // the shore's rock (rocks.rs): the armour against the Fort Point
            // and Marine Drive seawall, the boulders on the bare rock, at the
            // cliff toes and on the steep faces over the sea.
            let (riprap, boulders) = (crate::features::shore_ground("riprap"), crate::features::shore_ground("rocks"));
            if riprap || boulders || crate::rocks::scan::switches().scan {
                let t = Instant::now();
                let r = shore_rocks(repo_root, height_at, &edges, &crest_at, js.sea_level_y, riprap, boulders);
                let (nr, nr_s) = r.armour.as_ref().map(|m| (m.0.triangles(), m.1)).unwrap_or((0, 0));
                let (nb, nb_small, nb_s) = r.boulders.as_ref().map(|m| (m.0.triangles(), m.1.triangles(), m.2)).unwrap_or((0, 0, 0));
                if let Some((m, _)) = r.armour {
                    structures.push(Structure { cast: true, footway: false, kit: None, name: "shore-rocks:armour".into(), material: plain_material(0.82, 0.0, "facade-masonry"), mesh: m.freeze() });
                }
                if let Some((cast, small, _)) = r.boulders {
                    // one material (cc0.rs `structure_class` keys on the name): the large stones cast, the small do not
                    structures.push(Structure { cast: true, footway: false, kit: None, name: "shore-rocks:boulders".into(), material: plain_material(0.86, 0.0, "facade-masonry"), mesh: cast.freeze() });
                    structures.push(Structure { cast: false, footway: false, kit: None, name: "shore-rocks:boulders".into(), material: plain_material(0.86, 0.0, "facade-masonry"), mesh: small.freeze() });
                }
                if let Some((n, counts, ms)) = &r.scan {
                    eprintln!("rocks: scanned rock {n} pieces {counts}, placed in {ms:.0} ms");
                }
                eprintln!(
                    "rocks: armour {nr_s} stones {nr} triangles, boulders {nb_s} stones {} triangles ({nb_small} casting none), {:.0} ms (field and cover {:.0}, armour {:.0}, boulders {:.0})",
                    nb + nb_small,
                    t.elapsed().as_secs_f64() * 1e3,
                    r.ms[0],
                    r.ms[1],
                    r.ms[2]
                );
            }
            let waterfront_press = wf::ShorePress::new(js.sea_level_y, &edges, &piers);
            press = Some(std::sync::Arc::new(if opts.cove_detail { waterfront_press.with_municipal_pier(&piers) } else { waterfront_press }));
            census = Some(c);
            walls
        };
        // the bridges' night lights (lights.rs), on the grounds they were built on.
        let lights = if opts.lights {
            // (the bridges' plan is scaled with the ground, so are their lights.)
            let gg = ground_of(js, "golden-gate-bridge", height_at);
            let bb = ground_of(js, "bay-bridge-west", height_at);
            crate::lights::bridge_lights(js, &gg, &bb)
        } else {
            Vec::new()
        };
        landmarks.extend(railing_landmark);
        // the walkable decks (access.rs) on the grounds the landmarks stand on.
        let (mut lights, mut structures) = (lights, structures);
        let access = if opts.access && opts.buildings {
            let a = crate::access::AccessSet::build(height_at);
            structures.extend(a.structures());
            lights.extend(a.lights());
            Some(std::sync::Arc::new(a))
        } else {
            None
        };
        Ok(Self {
            landmarks,
            lights,
            tower_lights,
            walls,
            wall_material: Some(Material { surface: Some("seawall".into()), ..plain_material(0.93, 0.0, "sidewalk") }),
            structures,
            promenade_surfaces: Vec::new(),
            build_ms,
            walls_ms: t1.elapsed().as_secs_f64() * 1e3,
            press,
            census,
            piers_built,
            municipal_feet,
            docks,
            docks_built,
            berths,
            access,
        })
    }

    /// The city's paved plazas (plaza.rs): planned as the JS plans them over
    /// sf-lots-parks with `building_at` as the footprint test, their bays cut
    /// on the region's edges and `footprints`, laid at `level_at` (the footway
    /// datum, never under the ground), skipping a piece the street already
    /// paves (`paved_at` within the street's verge band of the level).
    #[allow(clippy::too_many_arguments)]
    pub fn add_plazas(
        &mut self,
        repo_root: &std::path::Path,
        building_at: &dyn Fn(f64, f64) -> bool,
        footprints: &[Vec<[f64; 2]>],
        level_at: &(dyn Fn(f64, f64) -> f64 + Sync),
        paved_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync),
        band_m: f64,
    ) -> Result<PlazaReport> {
        self.add_plazas_with(repo_root, building_at, footprints, level_at, paved_at, band_m, true)
    }

    /// `add_plazas`, each bay in its own tone (`bay_tones`) or all in the style's.
    #[allow(clippy::too_many_arguments)]
    pub fn add_plazas_with(
        &mut self,
        repo_root: &std::path::Path,
        building_at: &dyn Fn(f64, f64) -> bool,
        footprints: &[Vec<[f64; 2]>],
        level_at: &(dyn Fn(f64, f64) -> f64 + Sync),
        paved_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync),
        band_m: f64,
        bay_tones: bool,
    ) -> Result<PlazaReport> {
        let t0 = Instant::now();
        let js = LandmarksJs::get();
        let os = &js.open_space;
        let mut stage_ms = [0.0; 5];
        let mut lap = {
            let mut t = Instant::now();
            move |slot: &mut f64| {
                *slot = t.elapsed().as_secs_f64() * 1e3;
                t = Instant::now();
            }
        };
        let (greens, water) = crate::plaza::load_areas(repo_root)?;
        lap(&mut stage_ms[0]);
        let plan = crate::plaza::plan(&os.plaza, &greens, &water, Some(building_at));
        lap(&mut stage_ms[1]);
        let fps: Vec<(Vec<[f64; 2]>, crate::plaza::Bounds)> = footprints
            .iter()
            .filter(|r| r.len() >= 3)
            .map(|r| (r.clone(), crate::plaza::footprint_bounds(r)))
            .filter(|(_, b)| plan.plazas.iter().any(|p| b.min_x <= p.bounds.max_x && b.max_x >= p.bounds.min_x && b.min_z <= p.bounds.max_z && b.max_z >= p.bounds.min_z))
            .collect();
        lap(&mut stage_ms[2]);
        let bay = os.plaza_unit.max(0.5) * os.plaza_bay_units.max(1.0);
        let pieces = crate::plaza::pieces(&plan, bay, &fps);
        lap(&mut stage_ms[3]);
        let hex = |s: &str| u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
        let tone = |style: &str| crate::geometry::srgb_hex(hex(os.colors.get(&format!("plaza{}{}", &style[..1].to_uppercase(), &style[1..])).or(os.colors.get("path")).map(|s| s.as_str()).unwrap_or("#808080")));
        // Each piece's street test and heights (the street's surface tiles are
        // built on first use: most of the time), over a few threads, in order.
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
        let chunk = pieces.len().div_ceil(threads).max(1);
        let lifted: Vec<Option<Vec<[f64; 3]>>> = std::thread::scope(|s| {
            let hs: Vec<_> = pieces
                .chunks(chunk)
                .map(|part| {
                    s.spawn(move || {
                        part.iter()
                            .map(|(_, piece)| {
                                let m = crate::plaza::convex_centroid(piece);
                                let level = level_at(m[0], m[1]);
                                if paved_at(m[0], m[1]).is_some_and(|h| h - level <= band_m) {
                                    return None;
                                }
                                Some(piece.iter().map(|p| [p[0], level_at(p[0], p[1]), p[1]]).collect())
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().expect("plaza thread panicked")).collect()
        });
        let mut positions: Vec<[f64; 3]> = Vec::new();
        let mut colours: Vec<[f64; 3]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let (mut laid, mut covered) = (0usize, 0usize);
        for ((pi, piece), lifted) in pieces.iter().zip(lifted) {
            let Some(lifted) = lifted else {
                covered += 1;
                continue;
            };
            let mut c = tone(&plan.plazas[*pi].style);
            if bay_tones {
                // each bay's own tone (street-surface-v2.js `emitPavedPlazas`:
                // `1 + ((hash32('plaza:<id>:<A.k>:<C.k>:00') % 41) - 20) / 20 x plazaUnitTone` on
                // the style's sRGB tone), the bay found from the piece's centre in the plaza frame.
                let p = &plan.plazas[*pi];
                let m = crate::plaza::convex_centroid(piece);
                let (u, v) = (p.frame, [-p.frame[1], p.frame[0]]);
                let (a, cc) = (m[0] * u[0] + m[1] * u[1], m[0] * v[0] + m[1] * v[1]);
                let key = format!("plaza:{}:{}:{}:00", p.id, (a / bay).floor() as i64, (cc / bay).floor() as i64);
                let jitter = 1.0 + (((crate::plaza::hash32(&key) % 41) as f64 - 20.0) / 20.0) * os.plaza_unit_tone;
                let lin = |v: f64| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
                let srgb = |v: f64| if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                c = c.map(|v| lin((srgb(v) * jitter).min(1.0)));
            }
            let base = positions.len() as u32;
            for p in lifted {
                positions.push(p);
                colours.push(c);
            }
            // A convex piece, wound either way: fan it facing up (the sign of
            // the whole ring's area, as a cut can leave its first three points in line).
            let up = (0..piece.len()).map(|i| {
                let (a, b) = (piece[i], piece[(i + 1) % piece.len()]);
                a[0] * b[1] - b[0] * a[1]
            }).sum::<f64>() < 0.0;
            for k in 1..piece.len() as u32 - 1 {
                if up {
                    indices.extend_from_slice(&[base, base + k, base + k + 1]);
                } else {
                    indices.extend_from_slice(&[base, base + k + 1, base + k]);
                }
            }
            laid += 1;
        }
        let mut mesh = flat_mesh("plaza-paving", &positions, &indices, [1.0; 3]);
        // flat_mesh unshares the vertices; give each its piece's tone.
        for (t, tri) in indices.chunks_exact(3).enumerate() {
            for (k, &i) in tri.iter().enumerate() {
                let c = colours[i as usize];
                let v = 3 * t + k;
                mesh.colors[3 * v..3 * v + 3].copy_from_slice(&[c[0] as f32, c[1] as f32, c[2] as f32]);
            }
        }
        lap(&mut stage_ms[4]);
        let report = PlazaReport { plazas: plan.plazas.len(), skipped: plan.skipped.clone(), children: plan.children.len(), pieces: laid, covered_by_street: covered, triangles: mesh.triangles(), ms: t0.elapsed().as_secs_f64() * 1e3, stage_ms };
        if !mesh.is_empty() {
            // The JS lays it in the street surface's path layer (concrete); a matte paving here.
            self.structures.push(Structure { cast: false, footway: true, kit: None, name: "plaza-paving".into(), material: plain_material(0.9, 0.0, "sidewalk"), mesh });
        }
        Ok(report)
    }

    /// (`--quay-fill`) The promenade paved on from the quay apron (waterfront.rs,
    /// `CAP_W_M` behind the wall) to the street's walk where the ground between
    /// them stands lower than both: at every `FILL_STATION_M` along each quay
    /// wall edge, the first walk `walk_at` finds within `FILL_REACH_M` (no
    /// building on the way, `blocked`) is joined to the apron by a ramp at no
    /// more than `max_slope`: from the apron's inner edge, or, for a walk
    /// higher than the crest, from as far out on the apron as that slope needs
    /// (leaving `FILL_EDGE_CLEAR_M` of it at the coping), to just under the
    /// walk's edge. Where no such ramp fits (a difference up to
    /// `FILL_STEP_MAX_M`) the paving is a terrace at the higher of the two and
    /// a cast concrete step face closes it down to the lower. Where the ground (`ground_at`, under the
    /// shore press) is not sunken, or no walk is near, nothing is added.
    /// Returns (stations filled, stations tried).
    pub fn add_quay_fill(
        &mut self,
        height_at: &dyn Fn(f64, f64) -> Option<f64>,
        walk_at: &dyn Fn(f64, f64) -> Option<f64>,
        blocked: &dyn Fn(f64, f64) -> bool,
        ground_at: &dyn Fn(f64, f64) -> f64,
        max_slope: f64,
    ) -> (usize, usize) {
        use crate::waterfront as wf;
        let Some(press) = self.press.clone() else { return (0, 0) };
        let js = LandmarksJs::get();
        let sh = &js.shore;
        let crest_at = |x: f64, z: f64| wf::crest_ground(height_at, x, z) + sh.road_lift + sh.crest_above_road_m;
        let hex = |s: &str| crate::geometry::srgb_hex(u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0x808080));
        let tone = hex(js.open_space.colors.get("path").map(|s| s.as_str()).unwrap_or("#e1e3e4"));
        let concrete = crate::geometry::srgb_hex(sh.wall_srgb);
        let w0 = wf::CAP_W_M + wf::COPING_LAND_M;
        let mut paving = MeshArrays::new("waterfront:quay-fill");
        let mut faces = MeshArrays::new("waterfront:quay-fill-steps");
        let (mut filled, mut tried) = (0usize, 0usize);
        // a quad facing `want` (its normal turned to it)
        let facing = |m: &mut MeshArrays, q: [[f64; 3]; 4], want: [f64; 3], c: crate::geometry::Rgb| {
            let (u, v) = ([q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]], [q[3][0] - q[0][0], q[3][1] - q[0][1], q[3][2] - q[0][2]]);
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            if n[0] * want[0] + n[1] * want[1] + n[2] * want[2] >= 0.0 { m.quad(q[0], q[1], q[2], q[3], c) } else { m.quad(q[3], q[2], q[1], q[0], c) }
        };
        for e in press.edges().iter().filter(|e| e.pier.is_none()) {
            let (a, b, n) = (e.a, e.b, e.land);
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            if len < 0.5 {
                continue;
            }
            let (ya, yb) = (crest_at(a[0], a[1]), crest_at(b[0], b[1]));
            let k = (len / wf::FILL_STATION_M).ceil().max(1.0) as usize;
            // each station: its point on the wall line, the apron's crest there, and the walk found (reach, level)
            let stations: Vec<([f64; 2], f64, Option<(f64, f64)>)> = (0..=k)
                .map(|i| {
                    let f = i as f64 / k as f64;
                    let p = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
                    let y0 = ya + (yb - ya) * f;
                    let at = |s: f64| [p[0] + n[0] * s, p[1] + n[1] * s];
                    let mut s = w0 + 0.25;
                    let mut found = None;
                    while s <= w0 + wf::FILL_REACH_M {
                        let q = at(s);
                        if blocked(q[0], q[1]) {
                            break;
                        }
                        if let Some(level) = walk_at(q[0], q[1]) {
                            found = Some((s, level));
                            break;
                        }
                        s += 0.25;
                    }
                    let found = found.filter(|&(d, level)| {
                        let m = at(0.5 * (w0 + d));
                        let sunk = press.press(m[0], m[1], ground_at(m[0], m[1]));
                        d - w0 > 0.3 && (level - y0).abs() <= wf::FILL_STEP_MAX_M && sunk < y0.max(level) - wf::FILL_SUNKEN_M
                    });
                    (p, y0, found)
                })
                .collect();
            tried += stations.len();
            filled += stations.iter().filter(|s| s.2.is_some()).count();
            // where a ramp from the apron's edge would be too steep for a walk higher than the
            // crest, it starts further out on the apron (a run long enough for `max_slope`),
            // keeping `FILL_EDGE_CLEAR_M` of the apron at the coping
            let start = |y0: f64, d: f64, l: f64| -> Option<f64> {
                let rise = l - 0.02 - y0;
                if rise.abs() / (d - w0) <= max_slope {
                    return Some(w0);
                }
                let s0 = d - rise / max_slope;
                (rise > 0.0 && s0 >= wf::COPING_LAND_M + wf::FILL_EDGE_CLEAR_M).then_some(s0)
            };
            for w in stations.windows(2) {
                let ((p0, y0, Some((d0, l0))), (p1, y1, Some((d1, l1)))) = (w[0], w[1]) else { continue };
                let at = |p: [f64; 2], s: f64, y: f64| [p[0] + n[0] * s, y, p[1] + n[1] * s];
                let up = [0.0, 1.0, 0.0];
                if let (Some(s0), Some(s1)) = (start(y0, d0, l0), start(y1, d1, l1)) {
                    // a ramp: from the apron (a centimetre over it where it starts on it) to just under the walk's edge
                    let lift = |s: f64| if s < w0 { 0.01 } else { 0.0 };
                    facing(&mut paving, [at(p0, s0, y0 + lift(s0)), at(p1, s1, y1 + lift(s1)), at(p1, d1, l1 - 0.02), at(p0, d0, l0 - 0.02)], up, tone);
                    continue;
                }
                // the walk's edge stands over the fill by a tuck (the street draws its own edge)
                let (l0, l1) = (l0 - 0.02, l1 - 0.02);
                // a terrace at the higher level, the step face down to the lower
                let (t0, t1) = (y0.max(l0), y1.max(l1));
                facing(&mut paving, [at(p0, w0, t0), at(p1, w0, t1), at(p1, d1, t1), at(p0, d0, t0)], up, tone);
                if l0 > y0 || l1 > y1 {
                    // the walk higher: the face over the apron, toward the water
                    facing(&mut faces, [at(p0, w0, y0.min(t0) - 0.05), at(p1, w0, y1.min(t1) - 0.05), at(p1, w0, t1), at(p0, w0, t0)], [-n[0], 0.0, -n[1]], concrete);
                }
                if l0 < y0 || l1 < y1 {
                    // the apron higher: the face over the walk's edge, toward the street
                    facing(&mut faces, [at(p0, d0, l0.min(t0) - 0.05), at(p1, d1, l1.min(t1) - 0.05), at(p1, d1, t1), at(p0, d0, t0)], [n[0], 0.0, n[1]], concrete);
                }
            }
        }
        let (paving, faces) = (paving.freeze(), faces.freeze());
        if !paving.is_empty() {
            // as the apron: the street's footway paving, matte
            self.structures.push(Structure { cast: false, footway: true, kit: None, name: "waterfront:quay-fill".into(), material: plain_material(0.9, 0.0, "sidewalk"), mesh: paving });
        }
        if !faces.is_empty() {
            // as the apron's skirt and the pier fascias: cast concrete
            self.structures.push(Structure { cast: true, footway: false, kit: None, name: "waterfront:quay-fill-steps".into(), material: Material { surface: Some("concrete".into()), ..plain_material(0.92, 0.0, "facade-masonry") }, mesh: faces });
        }
        (filled, tried)
    }

    /// The Ocean Beach and Sunset Dunes seawalls (promenade.rs) on the drawn
    /// land (`land_at`) and the drawn beach (`sand_at`). Returns (stations, stairs) of each.
    pub fn add_promenades(&mut self, land_at: &dyn Fn(f64, f64) -> f64, sand_at: &dyn Fn(f64, f64) -> f64) -> [(usize, usize); 2] {
        let js = LandmarksJs::get();
        let p = &js.promenades;
        let albedo = &p["structureAlbedo"];
        let mut out = [(0, 0); 2];
        if let Some(ob) = crate::promenade::ocean_beach(p, albedo, land_at, sand_at) {
            out[0] = (ob.stations, ob.stairs);
            self.promenade_surfaces.push(ob.surfaces);
            // ocean-beach-seawall.js: concrete 0.88 (casts), planting 0.95, sand 1.0 (receive only).
            self.structures.push(Structure { cast: true, footway: false, kit: None, name: "ocean-beach-seawall:concrete".into(), material: plain_material(0.88, 0.0, "sidewalk"), mesh: ob.concrete });
            self.structures.push(Structure { cast: false, footway: false, kit: None, name: "ocean-beach-seawall:planting".into(), material: plain_material(0.95, 0.0, "sidewalk"), mesh: ob.planting });
            self.structures.push(Structure { cast: false, footway: false, kit: None, name: "ocean-beach-seawall:sand".into(), material: plain_material(1.0, 0.0, "sidewalk"), mesh: ob.sand });
        }
        if let Some(sd) = crate::promenade::sunset_dunes(p, albedo, land_at, sand_at) {
            out[1] = (sd.stations, sd.stairs);
            self.promenade_surfaces.push(sd.surfaces);
            // sunset-dunes-seawall.js: structure concrete at roughness 0.9, casts.
            self.structures.push(Structure { cast: true, footway: false, kit: None, name: "sunset-dunes-seawall".into(), material: plain_material(0.9, 0.0, "sidewalk"), mesh: sd.concrete });
        }
        out
    }

    /// The Ferry Building (ferry_building.rs) on `level_at` (the footway
    /// datum), one structure per finish; nothing when `--no-ferry-building`
    /// (its record then keeps its box: `claimed_records`). Returns what was built.
    ///
    /// Its forecourt is laid where `paved_at` (the street's paving) is None and
    /// `carpet_at` (the drawn ground) stands over the sea.
    pub fn add_ferry_building(&mut self, level_at: &dyn Fn(f64, f64) -> f64, paved_at: &dyn Fn(f64, f64) -> Option<f64>, carpet_at: &dyn Fn(f64, f64) -> f64) -> Option<crate::ferry_building::Report> {
        if !crate::features::ferry_building() {
            return None;
        }
        let t0 = Instant::now();
        let b = crate::ferry_building::build_with(level_at, paved_at, carpet_at);
        let report = crate::ferry_building::Report { base_y: b.base_y, length_m: b.length_m, depth_m: b.depth_m, bays: b.bays, triangles: b.triangles, ms: 0.0 };
        for p in b.parts {
            // a part without a kit finish draws its vertex colour at its own roughness and metalness.
            let material = plain_material(p.rough, p.metal, match p.key {
                "floor" => "sidewalk",
                "glass" | "shop" | "skylight" => "facade-glass",
                _ => "facade-masonry",
            });
            self.structures.push(Structure { cast: p.cast, footway: p.footway, kit: p.kit.map(str::to_string), name: format!("ferry-building:{}", p.key), material, mesh: p.mesh });
        }
        Some(crate::ferry_building::Report { ms: t0.elapsed().as_secs_f64() * 1e3, ..report })
    }

    /// The streetcar rails of the corridor (tram.rs) over `surface_at` (the
    /// drawn street surface), `lift` over it; nothing when `--no-tram-tracks`.
    /// Returns (tracks, metres, triangles).
    pub fn add_tram_tracks(&mut self, repo_root: &std::path::Path, surface_at: &dyn Fn(f64, f64) -> f64, lift: f64) -> Result<Option<(usize, f64, usize)>> {
        if !crate::features::tram_tracks() {
            return Ok(None);
        }
        let ways = crate::tram::load_corridor_ways(repo_root, crate::ferry_building::spec().tower.centre_surveyed)?;
        let t = crate::tram::build(&ways, surface_at, lift);
        let tris = t.heads.triangles() + t.grooves.triangles() + t.sides.triangles() + t.polish.triangles();
        // the polished heads as bare steel; the grooves a matte dark. With the
        // finish (tram.rs): the worn band less glossy, the oxidised edges a rough dielectric.
        let (band_rough, band_metal) = if t.sides.is_empty() { (0.36, 1.0) } else { (crate::tram::BAND_ROUGHNESS, crate::tram::BAND_METALNESS) };
        self.structures.push(Structure { cast: false, footway: false, kit: None, name: "tram:heads".into(), material: plain_material(band_rough, band_metal, "facade-metal"), mesh: t.heads });
        self.structures.push(Structure { cast: false, footway: false, kit: None, name: "tram:grooves".into(), material: plain_material(if t.sides.is_empty() { 0.9 } else { 0.95 }, 0.0, "asphalt"), mesh: t.grooves });
        if !t.sides.is_empty() {
            self.structures.push(Structure { cast: false, footway: false, kit: None, name: "tram:sides".into(), material: plain_material(crate::tram::OXIDE_ROUGHNESS, 0.0, "painted-metal"), mesh: t.sides });
        }
        // the polish (tram.rs): the burnished middle of each band, bare steel.
        if !t.polish.is_empty() {
            self.structures.push(Structure { cast: false, footway: false, kit: None, name: "tram:polish".into(), material: plain_material(crate::tram::POLISH_ROUGHNESS, crate::tram::POLISH_METALNESS, "facade-metal"), mesh: t.polish });
        }
        Ok(Some((t.tracks, t.metres, tris)))
    }

    /// The corridor's surveyed palms (palms.rs) on `ground_at` (the drawn
    /// surface); nothing when `--no-corridor-palms`. `palms` gives every palm
    /// record of the street tree list (x, z, crown m, height m, record) on the
    /// drawn plan; `blocked` refuses a site (inside a building). Returns (palms, triangles).
    pub fn add_corridor_palms(&mut self, palms: &[(f64, f64, Option<f64>, Option<f64>, u32)], ground_at: &dyn Fn(f64, f64) -> f64, blocked: &dyn Fn(f64, f64) -> bool) -> Option<(usize, usize)> {
        if !crate::features::corridor_palms() {
            return None;
        }
        let centre = eo_data::groundscale::plan(crate::ferry_building::spec().tower.centre_surveyed);
        let sites: Vec<crate::palms::Site> = crate::palms::corridor_sites(palms, centre).into_iter().filter(|s| !blocked(s.x, s.z)).collect();
        let p = crate::palms::build(&sites, ground_at);
        let tris = p.trunks.triangles() + p.leaves.triangles();
        self.structures.push(Structure { cast: true, footway: false, kit: None, name: "palms:trunks".into(), material: plain_material(0.9, 0.0, "facade-masonry"), mesh: p.trunks });
        // the leaves' waxy cuticle: a dielectric of moderate gloss.
        self.structures.push(Structure { cast: true, footway: false, kit: None, name: "palms:leaves".into(), material: plain_material(0.55, 0.0, "facade-painted"), mesh: p.leaves });
        Some((p.count, tris))
    }

    pub fn triangles(&self, role: Role) -> usize {
        self.landmarks.iter().flat_map(|l| &l.draws).filter(|d| d.role == role).map(|d| d.mesh.triangles()).sum()
    }
}
