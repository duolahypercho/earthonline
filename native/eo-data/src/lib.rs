//! Loaders for the San Francisco data the browser game ships under
//! `public/data/sf/`. Plain structs, no rendering. Each decoder mirrors the
//! JavaScript reader named in its doc comment; `native/README.md` documents
//! the formats.

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

pub mod groundscale; // the street size (the approved ground-plan deviation, 1.25)
pub mod roof_colours; // measured roof colours (aerial imagery)
pub mod sim_source;
pub mod standards;
pub use standards::CaStandards;

/// Where the repository's data lives.
#[derive(Clone, Debug)]
pub struct DataPaths {
    /// Repository root (holds `public/` and `scripts/`).
    pub repo_root: PathBuf,
}

impl DataPaths {
    /// `EO_REPO_ROOT` if set, else the first ancestor of the current directory
    /// or of this crate's manifest directory that holds `public/data/sf/sf-city.json.gz`.
    pub fn discover() -> Result<Self> {
        if let Ok(root) = std::env::var("EO_REPO_ROOT") {
            let p = PathBuf::from(root);
            if p.join("public/data/sf/sf-city.json.gz").is_file() {
                return Ok(Self { repo_root: p });
            }
            bail!("EO_REPO_ROOT={} has no public/data/sf/sf-city.json.gz", p.display());
        }
        let mut starts = vec![];
        if let Ok(cwd) = std::env::current_dir() {
            starts.push(cwd);
        }
        if let Ok(exe) = std::env::current_exe() {
            starts.push(exe);
        }
        starts.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        for start in starts {
            for dir in start.ancestors() {
                if dir.join("public/data/sf/sf-city.json.gz").is_file() {
                    return Ok(Self { repo_root: dir.to_path_buf() });
                }
            }
        }
        bail!("could not find public/data/sf/sf-city.json.gz above the working directory; set EO_REPO_ROOT")
    }

    pub fn sf(&self, name: &str) -> PathBuf {
        self.repo_root.join("public/data/sf").join(name)
    }
}

/// Read a file, gunzipping it when it starts with the gzip magic.
pub fn read_maybe_gzip(path: &Path) -> Result<Vec<u8>> {
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut out = Vec::with_capacity(raw.len() * 6);
        GzDecoder::new(&raw[..])
            .read_to_end(&mut out)
            .with_context(|| format!("gunzip {}", path.display()))?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

// ---------------------------------------------------------------------------
// sf-city.json(.gz), sf-footprints.json.gz, sf-footprint-supplement.json
// ---------------------------------------------------------------------------

/// A record id: OSM ids are numbers, DataSF pack ids are strings ("ds10").
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum RecordId {
    Num(u64),
    Str(String),
}

impl RecordId {
    pub fn key(&self) -> String {
        match self {
            RecordId::Num(n) => n.to_string(),
            RecordId::Str(s) => s.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct RawBuilding {
    pub id: RecordId,
    #[serde(default)]
    pub building: Option<String>,
    #[serde(default)]
    pub levels: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub area: Option<f64>,
    #[serde(default)]
    pub centroid: Option<[f64; 2]>,
    #[serde(default)]
    pub points: Vec<f64>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub supersedes: Vec<RecordId>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub amenity: Option<String>,
    /// `tradeSource` (an OSM POI embedded in the footprint), e.g. "osm-poi".
    #[serde(rename = "tradeSource", default)]
    pub trade_source: Option<String>,
}

/// The source record's tags the kit classifier reads (slice 3), exactly as
/// the file carries them.
#[derive(Clone, Debug, Default)]
pub struct BuildingTags {
    /// OSM `building` (pack records: "yes").
    pub building: String,
    pub amenity: String,
    pub name: String,
    pub levels: Option<f64>,
    /// The file's own `height` and `area` (before `resolveSourceHeight`).
    pub height: Option<f64>,
    pub area: Option<f64>,
    pub trade_source: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RawRoad {
    pub id: RecordId,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub highway: String,
    /// `true`, `false`, or `-1` (one-way against the way's direction).
    #[serde(default)]
    pub oneway: serde_json::Value,
    #[serde(default)]
    pub lanes: Option<f64>,
    #[serde(default)]
    pub maxspeed: String,
    #[serde(default)]
    pub layer: serde_json::Value,
    #[serde(default)]
    pub bridge: bool,
    #[serde(default)]
    pub tunnel: bool,
    #[serde(default)]
    pub points: Vec<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Projection {
    #[serde(rename = "metersPerDegreeLat")]
    pub meters_per_degree_lat: f64,
    #[serde(rename = "metersPerDegreeLon")]
    pub meters_per_degree_lon: f64,
    pub axes: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StreetDesign {
    #[serde(rename = "roadSurfaceLift", default)]
    pub road_surface_lift: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CityCounts {
    pub roads: usize,
    #[serde(rename = "detailBuildings")]
    pub detail_buildings: usize,
    #[serde(rename = "coarseBuildings")]
    pub coarse_buildings: usize,
    pub signals: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CityMeta {
    pub center: LatLon,
    pub projection: Projection,
    pub counts: CityCounts,
    #[serde(rename = "streetDesign")]
    pub street_design: Option<StreetDesign>,
}

#[derive(Debug, Deserialize)]
struct RawCity {
    meta: CityMeta,
    #[serde(default)]
    roads: Vec<RawRoad>,
    #[serde(rename = "detailBuildings", default)]
    detail_buildings: Vec<RawBuilding>,
    #[serde(default)]
    signals: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
struct RawPack {
    #[serde(rename = "detailBuildings")]
    detail_buildings: Vec<RawBuilding>,
}

#[derive(Debug, Deserialize)]
struct RawSupplement {
    buildings: Vec<RawBuilding>,
}

/// Which file a building came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildingSource {
    /// `sf-city.json` `detailBuildings` (OpenStreetMap extract).
    Extract,
    /// `sf-footprint-supplement.json` (records keyed to authored buildings).
    Supplement,
    /// `sf-footprints.json.gz` (DataSF footprint pack).
    Pack,
}

/// One building as this slice draws it: a footprint ring extruded to `height`.
#[derive(Clone, Debug)]
pub struct Building {
    pub id: String,
    pub source: BuildingSource,
    /// Resolved height in metres (`resolveSourceHeight`, then the 3 m floor).
    pub height: f32,
    /// Footprint ring, world metres (x east, z south), as the file lists it
    /// minus a closing duplicate of the first point.
    pub ring: Vec<[f32; 2]>,
    /// The file repeated the first point at the end (`ring` dropped it).
    pub closed: bool,
    /// The flat `points` array as the file lists it (f64, closing duplicate
    /// kept): what `flatToPoints` gives the JS record.
    pub points: Vec<f64>,
    /// The tags the kit classifier reads.
    pub tags: BuildingTags,
}

impl Building {
    /// The polygon exactly as the JS record carries it (`flatToPoints`, which
    /// keeps a closing duplicate): what index-based code such as a front edge
    /// number refers to.
    pub fn record_polygon(&self) -> Vec<[f32; 2]> {
        let mut p = self.ring.clone();
        if self.closed {
            p.push(self.ring[0]);
        }
        p
    }
}

/// A road way. Only geometry and the tags this slice reads.
#[derive(Clone, Debug)]
pub struct Road {
    pub id: String,
    pub name: String,
    pub highway: String,
    pub lanes: u32,
    pub maxspeed: String,
    pub bridge: bool,
    pub tunnel: bool,
    pub points: Vec<[f32; 2]>,
    /// The same points at the file's f64 precision (slice 3: the kit's street index).
    pub points64: Vec<[f64; 2]>,
}

/// Counts that explain how the building set was merged.
#[derive(Clone, Debug, Default)]
pub struct MergeReport {
    pub extract_records: usize,
    pub pack_records: usize,
    pub supplement_records: usize,
    pub pack_dropped_for_supplement: usize,
    pub superseded_ids: usize,
    pub extract_dropped_superseded: usize,
    pub dropped_not_at_grade: usize,
    pub dropped_landmark_excluded: usize,
    pub dropped_degenerate: usize,
}

#[derive(Debug)]
pub struct City {
    pub meta: CityMeta,
    pub buildings: Vec<Building>,
    pub roads: Vec<Road>,
    pub signals: usize,
    pub merge: MergeReport,
    /// what the ground plan did (None at scale 1).
    pub ground_plan: Option<groundscale::PlanReport>,
}

/// Centroid distance under which a supplement record replaces its pack copy
/// (`SUPPLEMENT_PACK_MATCH_M` in src/citygen/sf-data.js).
pub const SUPPLEMENT_PACK_MATCH_M: f64 = 1.2;

/// Footprint-pack rings a surveyed street has replaced
/// (`SUPERSEDED_FOOTPRINTS` in src/world/buildings/surface-footprints.js).
pub const SUPERSEDED_FOOTPRINTS: &[&str] = &["ds182", "ds1311"];

/// Records a landmark excludes as not-a-building
/// (`landmark.excludes` in src/world/landmarks/landmark-registry.js).
pub const LANDMARK_EXCLUDED_RECORDS: &[&str] = &["ds9087", "ds62170", "432712476"];

/// `defaultLevels` in src/citygen/sf-data.js.
fn default_levels(area: f64) -> f64 {
    if area > 2600.0 {
        8.0
    } else if area > 1200.0 {
        5.0
    } else if area > 450.0 {
        3.0
    } else {
        2.0
    }
}

/// The ground plan's one start-up line (once a process).
fn plan_report(r: &groundscale::PlanReport, s: f64) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        eprintln!(
            "groundscale: ground plan x{s} ({}): {} hand-built records moved whole ({} groups of touching buildings, largest {}; {} touching pairs left apart, {} seams between them filled, widest {:.1} m), {} facade edges held on their scaled street line (fronts within {:.2} m, other street faces within {:.2} m), {} vertices of {} neighbouring records pulled to them (largest pull {:.2} m)",
            groundscale::source(),
            r.moved,
            r.clusters.len(),
            r.clusters.first().copied().unwrap_or(1),
            r.open_seams,
            r.seams_filled,
            r.widest_seam_m,
            r.street_edges,
            r.worst_front_m,
            r.worst_street_edge_m,
            r.pulled_vertices,
            r.pulled_records,
            r.worst_pull_m
        )
    });
}

/// Shoelace area of a flat ring, square metres.
pub fn ring_area(ring: &[[f32; 2]]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0f64;
    for i in 0..n {
        let p = ring[i];
        let q = ring[(i + 1) % n];
        a += p[0] as f64 * q[1] as f64 - q[0] as f64 * p[1] as f64;
    }
    (a * 0.5).abs()
}

/// `resolveSourceHeight` in src/citygen/sf-data.js, then the `Math.max(3, h)`
/// floor its callers apply. The civic LiDAR override (`civicHeightOr`) is not
/// applied by this slice.
pub fn resolve_height(b: &RawBuilding, area: f64) -> f64 {
    if let Some(h) = b.height {
        if h > 0.0 {
            return h.max(3.0);
        }
    }
    let declared = b.levels.unwrap_or(0.0);
    let levels = if declared > 1.0 { declared } else { default_levels(area) };
    (levels * 3.2).max(3.0)
}

fn flat_to_ring(points: &[f64]) -> (Vec<[f32; 2]>, bool) {
    let mut ring: Vec<[f32; 2]> = points.chunks_exact(2).map(|c| [c[0] as f32, c[1] as f32]).collect();
    // Drop a closing duplicate if present.
    let closed = ring.len() > 1 && ring.first() == ring.last();
    if closed {
        ring.pop();
    }
    (ring, closed)
}

fn parse_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = read_maybe_gzip(path)?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

impl City {
    /// Load the extract, the footprint pack and the supplement, and merge them
    /// the way `fetchSfSources` in src/citygen/sf-data.js does when the
    /// footprint pack is enabled (the stream worker and the boot window).
    /// on this process's ground plan (`groundscale::scale()`).
    pub fn load(paths: &DataPaths) -> Result<City> {
        Self::load_at(paths, groundscale::scale())
    }

    /// the city on the ground plan of scale `s` (1.0: as surveyed),
    /// whatever this process's own scale is (the ground-plan test compares both).
    pub fn load_at(paths: &DataPaths, s: f64) -> Result<City> {
        let raw: RawCity = parse_json(&paths.sf("sf-city.json.gz"))?;
        if raw.meta.projection.axes != "x east, z south, metres" {
            bail!("sf-city.json: unexpected frame '{}'", raw.meta.projection.axes);
        }
        let pack: RawPack = parse_json(&paths.sf("sf-footprints.json.gz"))?;
        let supplement: Vec<RawBuilding> = match parse_json::<RawSupplement>(&paths.sf("sf-footprint-supplement.json")) {
            Ok(s) => s.buildings,
            Err(_) => vec![],
        };
        let mut merge = MergeReport {
            extract_records: raw.detail_buildings.len(),
            pack_records: pack.detail_buildings.len(),
            supplement_records: supplement.len(),
            ..Default::default()
        };

        // Pack records within 1.2 m of a supplement centroid give way to it.
        // Bucket supplement centroids on a 4 m grid so this stays linear.
        let mut grid: std::collections::HashMap<(i64, i64), Vec<[f64; 2]>> = Default::default();
        for s in &supplement {
            if let Some(c) = s.centroid {
                grid.entry(((c[0] / 4.0).floor() as i64, (c[1] / 4.0).floor() as i64)).or_default().push(c);
            }
        }
        let near_supplement = |c: [f64; 2]| -> bool {
            let (gx, gz) = ((c[0] / 4.0).floor() as i64, (c[1] / 4.0).floor() as i64);
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = grid.get(&(gx + dx, gz + dz)) {
                        if v.iter().any(|s| (s[0] - c[0]).hypot(s[1] - c[1]) <= SUPPLEMENT_PACK_MATCH_M) {
                            return true;
                        }
                    }
                }
            }
            false
        };
        let pack_kept: Vec<RawBuilding> = pack
            .detail_buildings
            .into_iter()
            .filter(|b| !b.centroid.map(near_supplement).unwrap_or(false))
            .collect();
        merge.pack_dropped_for_supplement = merge.pack_records - pack_kept.len();

        let superseded: HashSet<String> = pack_kept.iter().flat_map(|b| b.supersedes.iter().map(|i| i.key())).collect();
        merge.superseded_ids = superseded.len();
        let extract_kept: Vec<RawBuilding> =
            raw.detail_buildings.into_iter().filter(|b| !superseded.contains(&b.id.key())).collect();
        merge.extract_dropped_superseded = merge.extract_records - extract_kept.len();

        let mut buildings = Vec::with_capacity(extract_kept.len() + supplement.len() + pack_kept.len());
        let tagged = extract_kept
            .into_iter()
            .map(|b| (b, BuildingSource::Extract))
            .chain(supplement.into_iter().map(|b| (b, BuildingSource::Supplement)))
            .chain(pack_kept.into_iter().map(|b| (b, BuildingSource::Pack)));
        for (b, source) in tagged {
            let key = b.id.key();
            if b.location.as_deref() == Some("underground") || SUPERSEDED_FOOTPRINTS.contains(&key.as_str()) {
                merge.dropped_not_at_grade += 1;
                continue;
            }
            if LANDMARK_EXCLUDED_RECORDS.contains(&key.as_str()) {
                merge.dropped_landmark_excluded += 1;
                continue;
            }
            let (ring, closed) = flat_to_ring(&b.points);
            let area = ring_area(&ring);
            if ring.len() < 3 || area <= 0.0 {
                merge.dropped_degenerate += 1;
                continue;
            }
            let height = resolve_height(&b, area) as f32; // on the unscaled area (heights stay real)
            let tags = BuildingTags {
                building: b.building.clone().unwrap_or_default(),
                amenity: b.amenity.clone().unwrap_or_default(),
                name: b.name.clone().unwrap_or_default(),
                levels: b.levels,
                height: b.height,
                area: b.area,
                trade_source: b.trade_source.clone(),
            };
            buildings.push(Building { id: key, source, height, ring, closed, points: b.points, tags });
        }

        // the records on the scaled ground plan (`groundscale::GroundPlan`):
        // the hand-built ones moved whole, real size; every other one scaled, its
        // vertices beside a hand-built one pulled to that one's real-size walls.
        let plan_report_of = if s != 1.0 {
            let moved = groundscale::hand_built_records(&paths.repo_root)?;
            let ways: Vec<Vec<[f64; 2]>> = raw.roads.iter().filter(|r| standards::is_vehicle_way(&r.highway)).map(|r| r.points.chunks_exact(2).map(|c| [c[0], c[1]]).collect()).collect();
            let records: Vec<(&str, &[f64])> = buildings.iter().map(|b| (b.id.as_str(), b.points.as_slice())).collect();
            let mut plan = groundscale::GroundPlan::build(s, &moved, &records, &ways);
            for b in buildings.iter_mut() {
                plan.place(&b.id, &mut b.points);
                b.ring = flat_to_ring(&b.points).0;
            }
            // each seam that opens between two back-to-back hand-built
            // buildings is built up by an infill record (a pack record `seam-A-B`,
            // as tall as the lower of the two, so its roof stays under both).
            for (a, b, ring) in &plan.seams {
                let h = |k: &str| buildings.iter().find(|r| r.id == k).map(|r| r.height).unwrap_or(0.0);
                let height = h(a).min(h(b));
                if height <= 0.0 {
                    continue;
                }
                let mut points: Vec<f64> = ring.iter().flat_map(|p| [p[0], p[1]]).collect();
                points.extend_from_slice(&[ring[0][0], ring[0][1]]);
                let tags = BuildingTags { building: "yes".into(), height: Some(height as f64), ..Default::default() };
                buildings.push(Building { id: format!("seam-{a}-{b}"), source: BuildingSource::Pack, height, ring: flat_to_ring(&points).0, closed: true, points, tags });
            }
            if s == groundscale::scale() {
                groundscale::set_record_shifts(plan.shifts.clone());
                plan_report(&plan.report, s);
            }
            Some(plan.report)
        } else {
            None
        };

        let gs = s;
        let roads = raw
            .roads
            .into_iter()
            .map(|r| Road {
                id: r.id.key(),
                name: r.name,
                highway: r.highway,
                lanes: r.lanes.map(|l| l.max(1.0) as u32).unwrap_or(1),
                maxspeed: r.maxspeed,
                bridge: r.bridge,
                tunnel: r.tunnel,
                points: r.points.chunks_exact(2).map(|c| [(c[0] * gs) as f32, (c[1] * gs) as f32]).collect(), // * gs
                points64: r.points.chunks_exact(2).map(|c| [c[0] * gs, c[1] * gs]).collect(),                 // * gs
            })
            .collect();

        Ok(City { signals: raw.signals.len(), meta: raw.meta, buildings, roads, merge, ground_plan: plan_report_of })
    }
}

// ---------------------------------------------------------------------------
// sf-elevation.bin.gz  (src/world/terrain/sf-elevation-grid.js)
// ---------------------------------------------------------------------------

pub const SF_ELEVATION_GRID_VERSION: &str = "sf-elevation-3dep-cudem-v2";
/// Game y of NAVD88 zero (`SF_ELEVATION_NAVD88_TO_GAME_Y`).
pub const NAVD88_TO_GAME_Y: f64 = -(0.969 + 0.55);
/// `SF_ELEVATION_SEA_FLOOR_Y`.
pub const SEA_FLOOR_Y: f64 = -0.55 - 2.5;
/// Mean sea level in game y (`SEA_LEVEL_Y` in src/citygen/shoreline.js).
pub const SEA_LEVEL_Y: f64 = -0.55;

#[derive(Clone, Debug, Deserialize)]
pub struct ElevationMeta {
    pub version: String,
    #[serde(rename = "originX")]
    pub origin_x: f64,
    #[serde(rename = "originZ")]
    pub origin_z: f64,
    #[serde(rename = "cellSize")]
    pub cell_size: f64,
    pub width: usize,
    pub height: usize,
    pub nodata: i32,
}

#[derive(Clone)]
pub struct ElevationGrid {
    pub meta: ElevationMeta,
    /// Lidar plane in game y; NaN where the lidar has nothing.
    pub lidar: Vec<f32>,
    /// Topobathy plane, NAVD88 centimetres; `meta.nodata` where absent.
    pub topobathy: Vec<i16>,
    /// the height scale `sample` applies about mean sea level
    /// (`groundscale::lift_by`; 1.0 from `decode`, the process's
    /// `groundscale::height_scale()` from `load`).
    pub height_scale: f64,
}

impl ElevationGrid {
    pub fn load(paths: &DataPaths) -> Result<Self> {
        let mut g = Self::decode(&read_maybe_gzip(&paths.sf("sf-elevation.bin.gz"))?)?;
        // the grid's horizontal frame scaled (origin and cell), its heights not:
        // the ground under a scaled street is the ground that was under the real one.
        let gs = groundscale::scale();
        if gs != 1.0 {
            g.meta.origin_x *= gs;
            g.meta.origin_z *= gs;
            g.meta.cell_size *= gs;
        }
        // and the heights above sea level follow the plan (every grade the surveyed one).
        g.height_scale = groundscale::height_scale();
        Ok(g)
    }

    /// `decodeSfElevation(bytes, { approachFills: false })`.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 || &bytes[0..4] != b"EOEL" {
            bail!("sf-elevation: bad magic");
        }
        let header_bytes = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let meta: ElevationMeta = serde_json::from_slice(&bytes[8..8 + header_bytes])?;
        if meta.version != SF_ELEVATION_GRID_VERSION {
            bail!("sf-elevation: expected {SF_ELEVATION_GRID_VERSION}, got {}", meta.version);
        }
        let count = meta.width * meta.height;
        let body = 8 + header_bytes;
        if bytes.len() != body + count * 4 {
            bail!("sf-elevation: {} bytes, expected {}", bytes.len(), body + count * 4);
        }
        let nodata = meta.nodata as i16;
        let rd = |i: usize| i16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let mut lidar = vec![0f32; count];
        let mut topobathy = vec![0i16; count];
        for z in 0..meta.height {
            let row = z * meta.width;
            let mut prev: i32 = 0;
            for x in 0..meta.width {
                let d = rd(body + (row + x) * 2);
                if d == nodata {
                    lidar[row + x] = f32::NAN;
                    continue;
                }
                prev += d as i32;
                lidar[row + x] = (prev as f64 / 100.0 + NAVD88_TO_GAME_Y) as f32;
            }
        }
        let fill = body + count * 2;
        for z in 0..meta.height {
            let row = z * meta.width;
            let mut prev: i32 = 0;
            for x in 0..meta.width {
                let d = rd(fill + (row + x) * 2);
                if d == nodata {
                    topobathy[row + x] = nodata;
                    continue;
                }
                prev += d as i32;
                topobathy[row + x] = prev as i16;
            }
        }
        Ok(Self { meta, lidar, topobathy, height_scale: 1.0 })
    }

    pub fn max_x(&self) -> f64 {
        self.meta.origin_x + self.meta.width as f64 * self.meta.cell_size
    }
    pub fn max_z(&self) -> f64 {
        self.meta.origin_z + self.meta.height as f64 * self.meta.cell_size
    }

    /// The bilinear sampler of `createGridSampler` (no bridge-approach profile).
    /// None outside the grid. hills: on this grid's heights (`height_scale`
    /// about mean sea level; the surveyed height at 1.0).
    pub fn sample(&self, x: f64, z: f64) -> Option<f64> {
        let y = self.sample_surveyed(x, z)?;
        Some(groundscale::lift_by(y, self.height_scale))
    }

    /// `sample` on the surveyed heights (before `height_scale`).
    pub fn sample_surveyed(&self, x: f64, z: f64) -> Option<f64> {
        let m = &self.meta;
        if !(x >= m.origin_x && x <= self.max_x() && z >= m.origin_z && z <= self.max_z()) {
            return None;
        }
        let (w, h) = (m.width as i64, m.height as i64);
        let inv = 1.0 / m.cell_size;
        let fx = (x - m.origin_x) * inv - 0.5;
        let fz = (z - m.origin_z) * inv - 0.5;
        let mut x0 = fx.floor() as i64;
        let mut z0 = fz.floor() as i64;
        let tx = fx - x0 as f64;
        let tz = fz - z0 as f64;
        let mut x1 = x0 + 1;
        let mut z1 = z0 + 1;
        if x0 < 0 {
            x0 = 0;
        }
        if x1 >= w {
            x1 = w - 1;
        }
        if z0 < 0 {
            z0 = 0;
        }
        if z1 >= h {
            z1 = h - 1;
        }
        if x0 >= w {
            x0 = w - 1;
        }
        if z0 >= h {
            z0 = h - 1;
        }
        let idx = |xx: i64, zz: i64| (zz * w + xx) as usize;
        let hs = [
            self.lidar[idx(x0, z0)] as f64,
            self.lidar[idx(x1, z0)] as f64,
            self.lidar[idx(x0, z1)] as f64,
            self.lidar[idx(x1, z1)] as f64,
        ];
        let ws = [(1.0 - tx) * (1.0 - tz), tx * (1.0 - tz), (1.0 - tx) * tz, tx * tz];
        let nd = m.nodata as i16;
        let ts = [
            self.topobathy[idx(x0, z0)],
            self.topobathy[idx(x1, z0)],
            self.topobathy[idx(x0, z1)],
            self.topobathy[idx(x1, z1)],
        ];
        let lidar_ground = (0..4).any(|i| !hs[i].is_nan() && ts[i] == nd);
        if lidar_ground {
            if hs.iter().all(|v| !v.is_nan()) {
                return Some(hs[0] * ws[0] + hs[1] * ws[1] + hs[2] * ws[2] + hs[3] * ws[3]);
            }
            let (mut acc, mut wsum) = (0.0, 0.0);
            for i in 0..4 {
                if !hs[i].is_nan() {
                    acc += hs[i] * ws[i];
                    wsum += ws[i];
                }
            }
            if wsum > 1e-6 {
                return Some(acc / wsum);
            }
        }
        let (mut acc, mut wsum) = (0.0, 0.0);
        for i in 0..4 {
            if ts[i] != nd {
                acc += ts[i] as f64 * ws[i];
                wsum += ws[i];
            }
        }
        if wsum > 1e-6 {
            return Some(acc / wsum / 100.0 + NAVD88_TO_GAME_Y);
        }
        Some(SEA_FLOOR_Y)
    }

    /// Ground y, or the sea floor outside the grid.
    pub fn ground(&self, x: f64, z: f64) -> f64 {
        self.sample(x, z).unwrap_or(SEA_FLOOR_Y)
    }

    /// True where the cell holds lidar ground (not water, not topobathy only).
    pub fn is_lidar_ground_cell(&self, cx: usize, cz: usize) -> bool {
        let i = cz * self.meta.width + cx;
        !self.lidar[i].is_nan() && self.topobathy[i] == self.meta.nodata as i16
    }
}

// ---------------------------------------------------------------------------
// sf-land-cover.bin.gz  (src/world/stream/land-cover.js)
// ---------------------------------------------------------------------------

pub const LAND_COVER_MAGIC: u32 = 0x564f_434c; // 'LCOV'
pub const LAND_COVER_VERSION: u32 = 1;

/// The atlas frame (`SF_FRAME` in src/world/geo/sf-frame.js). Used to
/// unproject world metres to lat/lon for the land-cover raster.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub center_lat: f64,
    pub center_lon: f64,
    pub m_per_deg_lat: f64,
    pub m_per_deg_lon: f64,
}

impl Frame {
    pub fn from_meta(meta: &CityMeta) -> Self {
        Self {
            center_lat: meta.center.lat,
            center_lon: meta.center.lon,
            m_per_deg_lat: meta.projection.meters_per_degree_lat,
            m_per_deg_lon: meta.projection.meters_per_degree_lon,
        }
    }
    /// `SF_FRAME`: centre 37.778, -122.4194; 110574 m/deg lat; 111320*cos(lat) m/deg lon.
    pub fn sf() -> Self {
        Self {
            center_lat: 37.778,
            center_lon: -122.4194,
            m_per_deg_lat: 110574.0,
            m_per_deg_lon: 111320.0 * (37.778f64).to_radians().cos(),
        }
    }
    /// `unprojectXZ` (z is south, so north = -z).
    pub fn unproject(&self, x: f64, z: f64) -> (f64, f64) {
        (self.center_lat + (-z) / self.m_per_deg_lat, self.center_lon + x / self.m_per_deg_lon)
    }
}

pub struct LandCover {
    pub width: usize,
    pub height: usize,
    pub west: f64,
    pub north: f64,
    pub d_lon: f64,
    pub d_lat: f64,
    pub classes: Vec<u8>,
    pub frame: Frame,
}

impl LandCover {
    pub fn load(paths: &DataPaths) -> Result<Self> {
        // a scaled world point unprojects to the lat/lon it had unscaled.
        let mut frame = Frame::sf();
        frame.m_per_deg_lat *= groundscale::scale();
        frame.m_per_deg_lon *= groundscale::scale();
        Self::decode(&read_maybe_gzip(&paths.sf("sf-land-cover.bin.gz"))?, frame)
    }

    /// `decodeLandCover`: Uint32 header [magic, version, width, height, 0,0,0,0],
    /// Float64 [west, north, dLon, dLat], Uint8 classes row 0 north.
    pub fn decode(bytes: &[u8], frame: Frame) -> Result<Self> {
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        let f64_at = |o: usize| f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
        if bytes.len() < 64 || u32_at(0) != LAND_COVER_MAGIC || u32_at(1) != LAND_COVER_VERSION {
            bail!("land cover pack: bad header");
        }
        let (width, height) = (u32_at(2) as usize, u32_at(3) as usize);
        if bytes.len() < 64 + width * height {
            return Err(anyhow!("land cover pack: truncated"));
        }
        Ok(Self {
            width,
            height,
            west: f64_at(32),
            north: f64_at(40),
            d_lon: f64_at(48),
            d_lat: f64_at(56),
            classes: bytes[64..64 + width * height].to_vec(),
            frame,
        })
    }

    /// `classAt`: bilinear class vote of the four pixel centres; 0 off the raster.
    pub fn class_at(&self, x: f64, z: f64) -> u8 {
        let (lat, lon) = self.frame.unproject(x, z);
        let fx = (lon - self.west) / self.d_lon - 0.5;
        let fz = (self.north - lat) / self.d_lat - 0.5;
        let c0 = fx.floor() as i64;
        let r0 = fz.floor() as i64;
        if c0 < 0 || r0 < 0 || c0 + 1 >= self.width as i64 || r0 + 1 >= self.height as i64 {
            return 0;
        }
        let (u, v) = (fx - c0 as f64, fz - r0 as f64);
        let (c0, r0) = (c0 as usize, r0 as usize);
        let votes = [
            (self.classes[r0 * self.width + c0], (1.0 - u) * (1.0 - v)),
            (self.classes[r0 * self.width + c0 + 1], u * (1.0 - v)),
            (self.classes[(r0 + 1) * self.width + c0], (1.0 - u) * v),
            (self.classes[(r0 + 1) * self.width + c0 + 1], u * v),
        ];
        // Accumulate in first-seen order; the first class to reach the max wins,
        // matching the JS `seen` iteration with a strict `>`.
        let mut seen: Vec<(u8, f64)> = Vec::with_capacity(4);
        for (k, w) in votes {
            if let Some(e) = seen.iter_mut().find(|e| e.0 == k) {
                e.1 += w;
            } else {
                seen.push((k, w));
            }
        }
        let mut best = 0u8;
        let mut best_w = -1.0;
        for (k, w) in seen {
            if w > best_w {
                best_w = w;
                best = k;
            }
        }
        best
    }
}
