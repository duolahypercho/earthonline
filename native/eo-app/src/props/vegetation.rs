//! The props tiles' vegetation and hill rock: the drawn ground the plants
//! stand on (the scene's own terrain vertices, so a plant meets the surface
//! the frame draws, not a resampling of it), what grows where (the open
//! spaces' green tones, the land-cover raster, the far land north of the
//! strait), and the plants as props instances (`eo_landmarks::vegetation`
//! places them; eo-render's `props_lib2::vegetation_models` draws them).

use eo_landmarks::vegetation::{Cover, Kind, Lithology, Plant, Site, Switches};
use eo_render::props::GpuPropInstance;
use eo_render::props_lib2::vegetation_models as vm;
use std::collections::HashSet;
use std::sync::Arc;

/// Steeper than this the sand tone is a cliff, not a beach (degrees; a dry
/// sand dune's angle of repose is about 34 degrees: INFERRED threshold).
pub const SAND_CLIFF_DEG: f64 = 34.0;

/// The fine layer's cell (m) and how near it loads and drops (3D, m): the
/// grass's draw distance (`vegetation_models::THIN`, 100 m) and a margin.
pub const FINE_CELL_M: f64 = 32.0;
pub const FINE_LOAD_M: f32 = 104.0;
pub const FINE_DROP_M: f32 = 120.0;

/// The terrain's drawn vertices (heights and colours) on the elevation grid:
/// vertex (i, j) at (ox + (i + 0.5) cell, oz + (j + 0.5) cell), the cells
/// split along the (i + 1, j)-(i, j + 1) diagonal, as `scene::build_scene`
/// lays them.
pub struct TerrainGrid {
    pub ox: f64,
    pub oz: f64,
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    pub y: Vec<f32>,
    pub rgba: Vec<u32>,
}

impl TerrainGrid {
    /// Copy the terrain tiles' vertices out of the scene.
    pub fn from_scene(scene: &eo_render::scene::Scene, meta: &eo_data::ElevationMeta) -> Option<Self> {
        let (w, h) = (meta.width + 1, meta.height + 1);
        let mut g = TerrainGrid { ox: meta.origin_x, oz: meta.origin_z, cell: meta.cell_size, w, h, y: vec![f32::NAN; w * h], rgba: vec![0; w * h] };
        let side = eo_render::scene::TILE_CELLS + 1;
        let mut tiles = 0;
        for o in &scene.objects {
            if o.kind & 255 != eo_render::scene::ObjectKind::Terrain as u32 {
                continue;
            }
            let base = o.base_vertex as usize;
            let Some(first) = scene.vertices.get(base) else { continue };
            let ci = ((first.pos[0] as f64 - g.ox) / g.cell - 0.5).round() as i64;
            let cj = ((first.pos[2] as f64 - g.oz) / g.cell - 0.5).round() as i64;
            for vz in 0..side {
                for vx in 0..side {
                    let Some(v) = scene.vertices.get(base + vz * side + vx) else { continue };
                    let (i, j) = (ci + vx as i64, cj + vz as i64);
                    if i < 0 || j < 0 || i as usize >= w || j as usize >= h {
                        continue;
                    }
                    let k = j as usize * w + i as usize;
                    g.y[k] = v.pos[1];
                    g.rgba[k] = v.color;
                }
            }
            tiles += 1;
        }
        (tiles > 0).then_some(g)
    }

    fn at(&self, i: i64, j: i64) -> Option<usize> {
        (i >= 0 && j >= 0 && (i as usize) < self.w && (j as usize) < self.h).then(|| j as usize * self.w + i as usize)
    }

    /// The cell and the fractions inside it.
    fn locate(&self, x: f64, z: f64) -> Option<(i64, i64, f64, f64)> {
        let fx = (x - self.ox) / self.cell - 0.5;
        let fz = (z - self.oz) / self.cell - 0.5;
        let (i, j) = (fx.floor() as i64, fz.floor() as i64);
        self.at(i + 1, j + 1)?;
        self.at(i, j)?;
        Some((i, j, fx - i as f64, fz - j as f64))
    }

    /// The drawn ground's height at (x, z): the triangle the scene draws there.
    pub fn height(&self, x: f64, z: f64) -> Option<f64> {
        let (i, j, u, v) = self.locate(x, z)?;
        let y = |a: i64, b: i64| self.y[self.at(a, b).unwrap()] as f64;
        let (y00, y10, y01, y11) = (y(i, j), y(i + 1, j), y(i, j + 1), y(i + 1, j + 1));
        let h = if u + v <= 1.0 { y00 + u * (y10 - y00) + v * (y01 - y00) } else { y11 + (1.0 - u) * (y01 - y11) + (1.0 - v) * (y10 - y11) };
        h.is_finite().then_some(h)
    }

    /// The terrain's vertex colour at (x, z), bilinear: linear RGB (the
    /// vertex bytes decode with a 2.2 power, as draw.wgsl reads them) and the
    /// lowest alpha byte of the four (under 255: beach sand).
    pub fn colour(&self, x: f64, z: f64) -> Option<([f32; 3], u8)> {
        let (i, j, u, v) = self.locate(x, z)?;
        let mut c = [0.0f32; 3];
        let mut a = 255u8;
        for (di, dj, wt) in [(0, 0, (1.0 - u) * (1.0 - v)), (1, 0, u * (1.0 - v)), (0, 1, (1.0 - u) * v), (1, 1, u * v)] {
            let p = self.rgba[self.at(i + di, j + dj).unwrap()];
            for k in 0..3 {
                c[k] += ((((p >> (8 * k)) & 255) as f32 / 255.0).powf(2.2)) * wt as f32;
            }
            a = a.min((p >> 24) as u8);
        }
        Some((c, a))
    }
}

/// Game x, z of a geographic point (the sheet's formula: the surveyed frame
/// times the ground-plan scale).
fn game_xz(lat: f64, lon: f64) -> (f64, f64) {
    let f = eo_data::Frame::sf();
    let s = eo_data::groundscale::scale();
    ((lon - f.center_lon) * f.m_per_deg_lon * s, (f.center_lat - lat) * f.m_per_deg_lat * s)
}

/// Longitude and latitude of a game point (the inverse of `game_xz`).
pub fn lon_lat(x: f64, z: f64) -> (f64, f64) {
    let f = eo_data::Frame::sf();
    let s = eo_data::groundscale::scale();
    (f.center_lon + x / (f.m_per_deg_lon * s), f.center_lat - z / (f.m_per_deg_lat * s))
}

/// The NAIP tiles' true geometry. Each tile was cut by the image service's
/// `exportImage` from a box 2048 x `GRID.dlon` wide by 2048 x `GRID.dlat`
/// high into 2048 x 2048 pixels, and the service keeps a pixel square in
/// degrees: it widened each box north-south about its centre, so a tile's
/// pixel is `GRID.dlon` degrees in latitude too and neighbouring tile rows
/// overlap by 2048 (1 - dlat / dlon) = 424 rows. Measured: tile r001's first
/// row is tile r000's row 1624 (mean absolute difference 1.5 grey levels over
/// 60 x 800 pixels; the same 1624 between r001/r002 and r002/r003), as the
/// widening predicts. The seam's `GRID.pixel` reads latitude with `GRID.dlat`
/// (up to 212 rows, 160 m, off at a tile's north and south edges; hand-off
/// to the survey reader's owner): this finds the right pixel and names it in
/// `GRID`'s rows so `Naip::sample_rgb` reads it. (column, row) or None outside
/// the delivered tile rows.
pub fn naip_pixel(lon: f64, lat: f64) -> Option<(i64, i64)> {
    use eo_farshore::survey::naip::GRID;
    let n = GRID.tile as f64;
    let tile_lat = n * GRID.dlat;
    let t = ((GRID.north - lat) / tile_lat).floor();
    let c = ((lon - GRID.west) / GRID.dlon).floor();
    if t < 0.0 || t >= GRID.rows as f64 || c < 0.0 || c >= (GRID.cols * GRID.tile) as f64 {
        return None;
    }
    let centre = GRID.north - (t + 0.5) * tile_lat;
    let r = (n / 2.0 + (centre - lat) / GRID.dlon).floor().clamp(0.0, n - 1.0);
    Some((c as i64, t as i64 * GRID.tile as i64 + r as i64))
}

/// The NAIP pixel's north-south size on the ground (m): `GRID.dlon` degrees
/// of latitude (see [`naip_pixel`]), 0.754 m; east-west it is 0.6 m.
pub const NAIP_ROW_M: f64 = 6.816516843464343e-06 * 110_574.0;
pub const NAIP_COL_M: f64 = 0.6;

/// A NAIP box of `side` game metres at ground-plan scale `scale`: (columns,
/// rows), each odd, 1 to 15.
pub fn naip_box(side: f64, scale: f64) -> (i64, i64) {
    let odd = |v: f64| {
        let k = (v.round() as i64).clamp(1, 15);
        if k % 2 == 0 { k + 1 } else { k }
    };
    let real = side / scale;
    (odd(real / NAIP_COL_M), odd(real / NAIP_ROW_M))
}

/// The NAIP shares (`eo_landmarks::vegetation::naip_shares`) in a box of
/// side `side` game metres round (x, z) on ground of `slope_deg`: odd in
/// columns and rows ([`naip_box`]) about the pixel holding (x, z), with the
/// two-pixel margin the classifier's 5 x 5 brightness reads (None outside the
/// image). The planting reads it; so can the terrain's tone.
pub fn naip_shares_at(n: &eo_farshore::survey::naip::Naip, x: f64, z: f64, side: f64, slope_deg: f64) -> Option<eo_landmarks::vegetation::Naip> {
    let (lon, lat) = lon_lat(x, z);
    let (c0, r0) = naip_pixel(lon, lat)?;
    let (kc, kr) = naip_box(side, eo_data::groundscale::scale());
    let (w, h) = (kc + 4, kr + 4);
    let mut px = Vec::with_capacity((w * h) as usize);
    for j in 0..h {
        for i in 0..w {
            px.push(naip_rgb(n, c0 + i - w / 2, r0 + j - h / 2));
        }
    }
    if px.iter().filter(|p| p.is_some()).count() < px.len() / 2 {
        return None;
    }
    eo_landmarks::vegetation::naip_shares(&px, w as usize, h as usize, slope_deg)
}

/// Whether the survey reader's `GRID.pixel` already reads the widened tiles
/// (the hand-off applied): it then agrees with [`naip_pixel`] at tile r001's
/// first row, which the unwidened grid puts 157 rows away.
fn reader_registered() -> bool {
    use eo_farshore::survey::naip::GRID;
    let lon = GRID.west + 100.5 * GRID.dlon;
    let lat = GRID.north - 1.5 * GRID.tile as f64 * GRID.dlat + 1023.5 * GRID.dlon;
    GRID.pixel(lon, lat).map(|(c, r)| (c as i64, r as i64)) == naip_pixel(lon, lat)
}

/// One NAIP pixel by its (column, `GRID` row) of [`naip_pixel`] (None outside the image).
fn naip_rgb(n: &eo_farshore::survey::naip::Naip, c: i64, r: i64) -> Option<[u8; 3]> {
    use eo_farshore::survey::naip::GRID;
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if c < 0 || r < 0 {
        return None;
    }
    let lon = GRID.west + (c as f64 + 0.5) * GRID.dlon;
    let lat = if *REGISTERED.get_or_init(reader_registered) {
        // the pixel's true latitude: its tile's centre, square pixels of dlon degrees.
        let (t, rr) = ((r / GRID.tile as i64) as f64, (r % GRID.tile as i64) as f64);
        GRID.north - (t + 0.5) * GRID.tile as f64 * GRID.dlat + (GRID.tile as f64 / 2.0 - rr - 0.5) * GRID.dlon
    } else {
        GRID.north - (r as f64 + 0.5) * GRID.dlat
    };
    n.sample_rgb(lon, lat)
}

/// The annual grass's colour on the season model's day (linear): the far
/// layer's green and cured-grass albedos (`eo_render::farshore`) mixed by
/// `eo_look::season::current()`, mottled +-15 % as the far land's tone is.
pub fn season_grass(x: f64, z: f64) -> [f32; 3] {
    let d = eo_look::season::current();
    let m = 0.85 + 0.3 * eo_landmarks::ground_cover::ground_mottle(x, z) as f32;
    let (g, c) = (eo_render::farshore::FAR_GRASS_GREEN, eo_render::farshore::FAR_GRASS_GOLDEN);
    [0, 1, 2].map(|k| (g[k] + (c[k] - g[k]) * d) * m)
}

/// A box round the Presidio of San Francisco (lat 37.7865..37.8105 N, lon
/// -122.4870..-122.4460 W, rounded, INFERRED): only a bound. The park itself is
/// its mapped polygon in the greens (`PRESIDIO_POINT`); the box takes in the
/// Presidio Heights and Lake Street blocks, whose yard trees are not the forest.
fn presidio() -> (f64, f64, f64, f64) {
    let (x0, z1) = game_xz(37.7865, -122.4870);
    let (x1, z0) = game_xz(37.8105, -122.4460);
    (x0, z0, x1, z1)
}

/// A point inside the Presidio: the Main Post's parade ground (37.7989 N,
/// 122.4662 W, Presidio Trust maps). The largest park green round it is the
/// Presidio's mapped polygon (OSM relation 8346137 in sf-lots-parks, 6.05 km2).
const PRESIDIO_POINT: (f64, f64) = (37.7989, -122.4662);

/// Baker Beach's strip, which the Presidio's mapped polygon leaves out but whose
/// planted cypress stands behind the beach are the Presidio's (NPS Baker Beach):
/// inside the box, within this of the shore and north of Lobos Creek's mouth,
/// the Presidio's south-west corner (lat 37.7885 N, INFERRED), so Sea Cliff's
/// yards stay out.
const BAKER_STRIP_SHORE_M: f64 = 300.0;
const BAKER_STRIP_SOUTH_LAT: f64 = 37.7885;

/// The SF side's serpentinite: the coastal bluffs from Fort Point to Baker
/// Beach (NPS Baker Beach, Presidio Trust Marshall's Beach): west of lon
/// -122.4770, north of Baker Beach's south end (`BAKER_STRIP_SOUTH_LAT`) and
/// within `SERPENTINE_SHORE_M` of the shore (the bluff band, INFERRED).
/// Elsewhere south of the strait the rock is the Franciscan sandstone.
fn serpentine_bound() -> (f64, f64) {
    (game_xz(37.80, -122.4770).0, game_xz(BAKER_STRIP_SOUTH_LAT, -122.48).1)
}
pub const SERPENTINE_SHORE_M: f64 = 200.0;

/// The Presidio's mapped park polygon: the largest park green holding
/// `PRESIDIO_POINT` (None when the greens cannot be read).
fn presidio_ring(repo_root: &std::path::Path) -> Option<Vec<[f64; 2]>> {
    let (greens, _) = eo_landmarks::plaza::load_areas(repo_root).map_err(|e| eprintln!("vegetation: greens not read ({e:#})")).ok()?;
    let (px, pz) = game_xz(PRESIDIO_POINT.0, PRESIDIO_POINT.1);
    let area = |r: &[[f64; 2]]| (0..r.len()).map(|i| r[i][0] * r[(i + 1) % r.len()][1] - r[(i + 1) % r.len()][0] * r[i][1]).sum::<f64>().abs() / 2.0;
    greens
        .into_iter()
        .filter(|g| g.kind == "park" && g.polygon.len() >= 3 && eo_landmarks::waterfront::point_in_ring(&g.polygon, px, pz))
        .max_by(|a, b| area(&a.polygon).total_cmp(&area(&b.polygon)))
        .map(|g| g.polygon)
}

/// What the plants read: the drawn terrain, the street world (footprints,
/// paved surfaces), the open spaces' greens, the land cover, the shore field.
pub struct VegGround {
    pub grid: TerrainGrid,
    pub street: Arc<eo_street::StreetWorld>,
    pub land: Option<eo_data::LandCover>,
    pub water: Option<Arc<eo_water::WaterData>>,
    pub switches: Switches,
    /// The round-3 switches (`eo_landmarks::vegetation::gg3`).
    pub g3: eo_landmarks::vegetation::Gg3,
    /// The 0.6 m NAIP image (survey group A1, read through `EO_SURVEY_ROOT`;
    /// None without it or with `--no-naip-planting`: the round-2 rules).
    naip: Option<eo_farshore::survey::naip::Naip>,
    presidio: (f64, f64, f64, f64),
    /// The Presidio's mapped park polygon (game metres), if the greens were read.
    presidio_ring: Option<Vec<[f64; 2]>>,
    baker_south_z: f64,
    serpentine: (f64, f64),
    /// The 128 m props tiles with land on them (the ones without a street tile plant vegetation only).
    pub tiles: HashSet<(i32, i32)>,
}

impl VegGround {
    pub fn new(grid: TerrainGrid, street: Arc<eo_street::StreetWorld>, land: Option<eo_data::LandCover>, water: Option<Arc<eo_water::WaterData>>, switches: Switches, repo_root: &std::path::Path) -> Self {
        let tm = eo_street::STREET_TILE_M;
        let mut tiles = HashSet::new();
        for j in 0..grid.h {
            for i in 0..grid.w {
                let y = grid.y[j * grid.w + i];
                if y.is_finite() && (y as f64) > eo_water::SEA_LEVEL_Y + 0.5 {
                    let (x, z) = (grid.ox + (i as f64 + 0.5) * grid.cell, grid.oz + (j as f64 + 0.5) * grid.cell);
                    tiles.insert(((x / tm).floor() as i32, (z / tm).floor() as i32));
                }
            }
        }
        // the shore distance the coastal trees' species mix and the trees' sand test read
        // (eo_landmarks::vegetation: the first install wins; the far tier estimates it
        // from the drawn sea until then, eo-render hlod_trees `shore_estimate`).
        if let Some(w) = water.clone() {
            eo_landmarks::vegetation::install_shore_distance(Box::new(move |x, z| w.field.signed_at(x, z).abs()));
        }
        let presidio_ring = presidio_ring(repo_root);
        if presidio_ring.is_none() {
            eprintln!("vegetation: the Presidio's park polygon not found in the greens; its coastal stands follow the mapped woods only");
        }
        let g3 = eo_landmarks::vegetation::gg3();
        let naip = if g3.naip_planting {
            match eo_farshore::survey::Root::discover() {
                Some(root) => Some(eo_farshore::survey::naip::Naip::open(&root)),
                None => {
                    eprintln!("vegetation: {} not set or not a folder; planting by the round-2 rules (no NAIP image)", eo_farshore::survey::ROOT_ENV);
                    None
                }
            }
        } else {
            None
        };
        let g = VegGround { grid, street, land, water, switches, g3, naip, presidio: presidio(), presidio_ring, baker_south_z: game_xz(BAKER_STRIP_SOUTH_LAT, -122.48).1, serpentine: serpentine_bound(), tiles };
        g.probe_from_env();
        g
    }

    /// `EO_VEG_PROBE="x,z x,z ..."`: print what the sampler sees at those points (a diagnostic).
    pub fn probe_from_env(&self) {
        let Ok(v) = std::env::var("EO_VEG_PROBE") else { return };
        for p in v.split_whitespace() {
            let c: Vec<f64> = p.split(',').filter_map(|t| t.parse().ok()).collect();
            if c.len() != 2 {
                continue;
            }
            let (x, z) = (c[0], c[1]);
            let y = self.grid.height(x, z);
            let col = self.grid.colour(x, z);
            let s = self.water.as_ref().map(|w| w.field.signed_at(x, z));
            let class = self.land.as_ref().map(|l| l.class_at(x, z));
            let green = eo_landmarks::ground_cover::installed().map(|c| (c.tone_name_at(x, z), c.tree_class_at(x, z)));
            eprintln!(
                "vegetation probe ({x:.1}, {z:.1}): y {y:?} colour {col:?} shore {s:?} class {class:?} green {green:?} building {} paved {:?} street tile {} naip {:?} -> {:?}",
                self.street.buildings.contains(x, z),
                self.street.paved_at(x, z),
                self.street.tiles.contains_key(&eo_street::tile_key(x, z)),
                self.naip_at(x, z, eo_landmarks::vegetation::NAIP_PLANT_BOX_M),
                self.site(x, z)
            );
        }
    }

    /// Inside the Presidio: its mapped park polygon, or Baker Beach's strip.
    pub fn in_presidio(&self, x: f64, z: f64) -> bool {
        let (x0, z0, x1, z1) = self.presidio;
        if !(x >= x0 && x <= x1 && z >= z0 && z <= z1) {
            return false;
        }
        if self.presidio_ring.as_ref().is_some_and(|r| eo_landmarks::waterfront::point_in_ring(r, x, z)) {
            return true;
        }
        z < self.baker_south_z && self.shore_m(x, z) < BAKER_STRIP_SHORE_M
    }

    /// The shore distance at (x, z) (m; the shore field's magnitude, 1 km without it).
    pub fn shore_m(&self, x: f64, z: f64) -> f64 {
        self.water.as_ref().map(|w| w.field.signed_at(x, z).abs()).unwrap_or(1e3)
    }

    /// The drawn ground's height at (x, z): the detail ground where one is
    /// set (`eo_data::detail_ground`, none yet), else the terrain triangle.
    pub fn ground_y(&self, x: f64, z: f64) -> Option<f64> {
        eo_data::detail_ground::height(x, z).or_else(|| self.grid.height(x, z))
    }

    /// The cliff over a beach (`--no-cliff-rock-cover`: never): the ground the
    /// sand limit takes from a beach and draws as cliff (eo-app ground_tone:
    /// steeper than `SAND_SLOPE_MAX` across 12 m, inland of the shoreline,
    /// within `BEACH_SAND_REACH_M` of a mapped beach). Rock cover: nothing grows
    /// on it (`vegetation::plantable`). `s` is the signed shore distance.
    pub fn cliff_band(&self, x: f64, z: f64, s: f64) -> bool {
        if !self.g3.cliff_rock_cover || s >= 0.0 || !eo_landmarks::features::shore_ground("beach-sand-limit") {
            return false;
        }
        let h = |dx: f64, dz: f64| self.grid.height(x + dx, z + dz);
        let slope = match (h(-6.0, 0.0), h(6.0, 0.0), h(0.0, -6.0), h(0.0, 6.0)) {
            (Some(a), Some(b), Some(c), Some(d)) => (b - a).hypot(d - c) / 12.0,
            _ => return false,
        };
        slope > eo_landmarks::ground_cover::SAND_SLOPE_MAX && eo_landmarks::ground_cover::installed().is_some_and(|c| c.near_mapped_beach(x, z))
    }

    /// The NAIP shares in a box of side `side` m (game) round (x, z) (None: no
    /// image there, or `--no-naip-planting`): the box in real metres (the
    /// ground-plan scale), odd in columns (0.6 m) and rows ([`NAIP_ROW_M`]) so it
    /// is centred on the pixel holding (x, z), at most 15 a side, with the two-pixel
    /// margin the classifier's 5 x 5 brightness reads.
    pub fn naip_at(&self, x: f64, z: f64, side: f64) -> Option<eo_landmarks::vegetation::Naip> {
        let n = self.naip.as_ref()?;
        naip_shares_at(n, x, z, side, self.site_slope_deg(x, z).unwrap_or(0.0))
    }

    /// Whether (x, z) is inside the NAIP image and the image is read (`--no-naip-planting`: never).
    pub fn in_naip(&self, x: f64, z: f64) -> bool {
        let (lon, lat) = lon_lat(x, z);
        self.naip.is_some() && naip_pixel(lon, lat).is_some()
    }

    /// The drawn ground's slope at (x, z) across one grid cell (degrees).
    fn site_slope_deg(&self, x: f64, z: f64) -> Option<f64> {
        let c = self.grid.cell;
        let y = self.ground_y(x, z)?;
        let h = |dx: f64, dz: f64| self.ground_y(x + dx, z + dz).unwrap_or(y);
        let (gx, gz) = ((h(c, 0.0) - h(-c, 0.0)) / (2.0 * c), (h(0.0, c) - h(0.0, -c)) / (2.0 * c));
        Some((gx * gx + gz * gz).sqrt().atan().to_degrees())
    }

    /// The round-3 planting context for `plant_coarse_with` / `plant_fine_with`.
    pub fn extra(&self) -> eo_landmarks::vegetation::Extra<'_> {
        eo_landmarks::vegetation::Extra { g3: self.g3, naip: Box::new(move |x, z, side| self.naip_at(x, z, side)) }
    }

    /// Whether a park tree may stand at (x, z) (round 3): on plantable ground
    /// (`--no-cliff-rock-cover`), under canopy in the NAIP image (`--no-naip-planting`).
    pub fn tree_ground_ok(&self, x: f64, z: f64) -> bool {
        if self.g3.cliff_rock_cover && self.site(x, z).is_some_and(|s| s.cover.is_some_and(|c| !eo_landmarks::vegetation::plantable(c, self.g3))) {
            return false;
        }
        eo_landmarks::vegetation::naip_tree_ok(&self.extra(), x, z)
    }

    /// The drawn ground at (x, z) and what grows there (None: nothing can).
    pub fn site(&self, x: f64, z: f64) -> Option<Site> {
        let sea = eo_water::SEA_LEVEL_Y;
        let y = self.ground_y(x, z)?;
        if y < sea + 0.4 {
            return None;
        }
        let (ground, alpha) = self.grid.colour(x, z)?;
        let c = self.grid.cell;
        let h = |dx: f64, dz: f64| self.ground_y(x + dx, z + dz).unwrap_or(y);
        let (gx, gz) = ((h(c, 0.0) - h(-c, 0.0)) / (2.0 * c), (h(0.0, c) - h(0.0, -c)) / (2.0 * c));
        // what grows is classified on the 8 m grid's slope (the mapped cover and the land cover
        // are drawn at that scale, and a lawn is mown whatever its 2 m relief); the detail ground
        // (eo_data::detail_ground) gives the seat, its height and gradient, only
        let g8 = |dx: f64, dz: f64| self.grid.height(x + dx, z + dz);
        let slope = match (g8(c, 0.0), g8(-c, 0.0), g8(0.0, c), g8(0.0, -c)) {
            (Some(a), Some(b), Some(d), Some(e)) => ((a - b) / (2.0 * c)).hypot((d - e) / (2.0 * c)).atan().to_degrees(),
            _ => (gx * gx + gz * gz).sqrt().atan().to_degrees(),
        };
        // beach sand (ground_tone.rs puts the shore distance in the alpha byte): nothing grows,
        // but a steep face over the beach is a cliff the sand tone runs up (the sand limit is
        // the shore ground's to draw): rock there (INFERRED: steeper than a dune's 34 degree
        // angle of repose, `SAND_CLIFF_DEG`, and clear of the swash).
        // So is a mapped bare-rock green the sand tone reaches (the serpentinite at Marshall's and
        // Baker): its rock is drawn.
        let mapped_rock = alpha < 250 && y > sea + 1.0 && eo_landmarks::ground_cover::installed().is_some_and(|c| c.tone_name_at(x, z) == Some("bareRock"));
        let sand_cliff = alpha < 250 && !mapped_rock && slope > SAND_CLIFF_DEG && y > sea + 3.0;
        if alpha < 250 && !sand_cliff && !mapped_rock {
            return None;
        }
        // the shore field: negative inland of the city's rings, positive out over the water
        // and on the far land north of the strait (ground_tone.rs `far_land`).
        let s = self.water.as_ref().map(|w| w.field.signed_at(x, z)).unwrap_or(-1e3);
        let far = s > crate::ground_tone::FAR_LAND_SHORE_M && y > sea + crate::ground_tone::FAR_LAND_ABOVE_SEA_M;
        if !far && s > -1.5 {
            return None; // the water, the shore line itself
        }
        if self.street.buildings.contains(x, z) || self.street.paved_at(x, z).is_some() {
            return None;
        }
        let class = self.land.as_ref().map(|l| l.class_at(x, z)).unwrap_or(0);
        let cover = if mapped_rock {
            Some(Cover::Rock)
        } else if sand_cliff || self.cliff_band(x, z, s) {
            Some(Cover::Cliff)
        } else if far {
            match class {
                10 => Some(Cover::TallScrub),
                20 | 90 => Some(Cover::Scrub),
                30 | 40 | 100 | 0 => Some(Cover::WildGrass),
                60 => Some(Cover::Bare),
                _ => None,
            }
        } else {
            let green = eo_landmarks::ground_cover::installed().and_then(|c| c.tone_name_at(x, z).map(|n| (n, c.tree_class_at(x, z))));
            match green {
                Some((name, tc)) => match name {
                    "grass" if tc == 7 => Some(Cover::WildGrass),
                    // the Presidio's park polygon spans forest, scrub and bluffs: there the land cover
                    // decides; a park's slope too steep to mow (INFERRED 18 degrees) is wild grass.
                    "grass" if self.in_presidio(x, z) && class == 10 => Some(Cover::Wood),
                    "grass" if self.in_presidio(x, z) && (class == 20 || class == 90) => Some(Cover::Scrub),
                    "grass" if slope > 18.0 || (self.in_presidio(x, z) && class == 60) => Some(Cover::WildGrass),
                    "grass" => Some(Cover::Lawn),
                    "understory" if tc == 1 => Some(Cover::WoodGreen),
                    "understory" => Some(Cover::Scrub),
                    "duneScrub" => Some(Cover::DuneScrub),
                    "dryGrass" => Some(Cover::WildGrass),
                    "bareEarth" | "wornDirt" => Some(Cover::Bare),
                    "bareRock" => Some(Cover::Rock),
                    _ => None,
                },
                None => {
                    let presidio = self.in_presidio(x, z);
                    match class {
                        10 if presidio => Some(Cover::Wood),
                        20 | 90 => Some(Cover::Scrub),
                        30 | 100 => Some(Cover::WildGrass),
                        60 => Some(Cover::Bare),
                        _ => None,
                    }
                }
            }
        };
        let l = (gx * gx + 1.0 + gz * gz).sqrt();
        let normal = [-gx / l, 1.0 / l, -gz / l];
        let r = 12.0;
        let ring: f64 = (0..8).map(|k| (k as f64 * std::f64::consts::FRAC_PI_4).sin_cos()).map(|(sn, cs)| h(cs * r, sn * r)).sum::<f64>() / 8.0;
        let lithology = if far {
            Lithology::Chert
        } else if x < self.serpentine.0 && z < self.serpentine.1 && s.abs() < SERPENTINE_SHORE_M {
            Lithology::Serpentinite
        } else {
            Lithology::Sandstone
        };
        // round 3 (`--no-naip-planting`): on the open ground the NAIP image plants, the blades
        // take the season's grass (eo_look::season on the day, the far layer's green and
        // cured-grass albedos, which the terrain's far land uses too), not the drawn
        // ground's tone: that is a paved grey or a summer straw wherever the terrain's
        // own classes or its load-time season disagree with the image and the date.
        let ground = match cover {
            Some(Cover::WildGrass | Cover::Scrub | Cover::TallScrub | Cover::DuneScrub | Cover::Bare) if self.in_naip(x, z) => season_grass(x, z),
            _ => ground,
        };
        Some(Site { y, normal, ground, cover, lithology, shore_m: s.abs(), hollow_m: ring - y })
    }
}

/// The render model of a plant.
fn model_of(k: Kind) -> u32 {
    match k {
        Kind::Tree(i) => vm::V_TREE + i as u32,
        Kind::Shrub(i) => vm::V_SHRUB + i as u32,
        Kind::Grass => vm::V_GRASS,
        Kind::Tuft => vm::V_TUFT,
        Kind::Flower(i) => vm::V_FLOWER + i as u32,
        Kind::Rock(i) => vm::V_ROCK + (i as u32 % vm::V_ROCKS),
    }
}

/// A linear tint as the lamp word props_draw.wgsl unpacks (sRGB of tint / 4; alpha 255, never 0).
pub fn pack_tint(t: [f32; 3]) -> u32 {
    let e = |v: f32| {
        let l = (v / 4.0).clamp(0.0, 1.0);
        let s = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0).round() as u32
    };
    e(t[0]) | (e(t[1]) << 8) | (e(t[2]) << 16) | (255 << 24)
}

/// The ground gradient as the pad word props_types.wgsl unpacks (`unpack2x16snorm` x 4).
pub fn pack_grad(g: [f32; 2]) -> u32 {
    let q = |v: f32| ((v / 4.0).clamp(-1.0, 1.0) * 32767.0).round() as i16 as u16 as u32;
    q(g[0]) | (q(g[1]) << 16)
}

/// The plants as props instances (`slope` carries the rank, `lamp` the tint).
pub fn instances(plants: &[Plant], out: &mut Vec<GpuPropInstance>, counts: &mut std::collections::HashMap<&'static str, usize>) {
    for p in plants {
        let name = match p.kind {
            Kind::Tree(_) => "coastal tree",
            Kind::Shrub(_) => "coastal shrub",
            Kind::Grass => "grass bunch",
            Kind::Tuft => "grass tuft",
            Kind::Flower(_) => "wildflowers",
            Kind::Rock(_) => "hill rock",
        };
        *counts.entry(name).or_insert(0) += 1;
        out.push(GpuPropInstance {
            pos: [p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32],
            yaw: p.yaw as f32,
            scale: p.scale,
            model: model_of(p.kind),
            variant: (p.rank * 255.0) as u32,
            // the alpha byte carries the grass tip word (eo_landmarks::vegetation::meadow_tip; 255 none).
            lamp: (pack_tint(p.tint) & 0x00ff_ffff) | ((p.tip as u32) << 24),
            slope: p.rank,
            pad: pack_grad(p.grad),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_packs_as_the_shader_unpacks() {
        let w = pack_tint([1.0, 0.25, 4.0]);
        let un = |b: u32| {
            let s = ((w >> (8 * b)) & 255) as f32 / 255.0;
            (if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }) * 4.0
        };
        assert!((un(0) - 1.0).abs() < 0.02 && (un(1) - 0.25).abs() < 0.01 && (un(2) - 4.0).abs() < 0.01);
        assert_eq!(w >> 24, 255);
    }

    #[test]
    fn gradient_packs_as_snorm16_quarters() {
        let w = pack_grad([1.0, -2.0]);
        let un = |v: u32| (v as u16 as i16) as f32 / 32767.0 * 4.0;
        assert!((un(w & 0xffff) - 1.0).abs() < 1e-3 && (un(w >> 16) + 2.0).abs() < 1e-3);
    }

    #[test]
    fn naip_rows_follow_the_widened_tiles() {
        use eo_farshore::survey::naip::GRID;
        let tl = GRID.tile as f64 * GRID.dlat;
        let lon = GRID.west + 100.5 * GRID.dlon;
        // a tile's requested centre is its middle row.
        let centre = |t: f64| GRID.north - (t + 0.5) * tl;
        assert_eq!(naip_pixel(lon, centre(0.0)), Some((100, 1024)));
        assert_eq!(naip_pixel(lon, centre(3.0)), Some((100, 3 * 2048 + 1024)));
        // tile r001's first row (its centre plus 1023.5 square pixels) is r000's row 1624.
        assert_eq!(naip_pixel(lon, centre(1.0) + 1023.5 * GRID.dlon), Some((100, 1624)));
        // a tile's requested north edge is its row 211 (1024 (1 - dlat / dlon), half the overlap).
        assert_eq!(naip_pixel(lon, GRID.north - 1e-9).map(|p| p.1), Some(211));
        assert_eq!(naip_pixel(lon, GRID.north + 1e-6), None);
        assert_eq!(naip_pixel(GRID.west - 1e-6, centre(0.0)), None);
    }

    #[test]
    fn naip_boxes_are_odd_and_in_real_metres() {
        // the 3 m (game) plant box at the 1.25 ground scale: 2.4 m real, 4 columns of
        // 0.6 m (odd: 5) by 3.2 rows of 0.754 m (3); at scale 1.0: 5 by 4 (odd: 5).
        assert_eq!(naip_box(3.0, 1.25), (5, 3));
        assert_eq!(naip_box(3.0, 1.0), (5, 5));
        // the 6 m tree box: 4.8 m real, 8 by 6.4 (9, 7); at 1.0: 10 by 8 (11, 9).
        assert_eq!(naip_box(6.0, 1.25), (9, 7));
        assert_eq!(naip_box(6.0, 1.0), (11, 9));
        assert_eq!(naip_box(100.0, 1.25), (15, 15));
        assert!((NAIP_ROW_M - 0.7537).abs() < 1e-3);
    }

    #[test]
    fn the_tree_sand_berm_is_the_ground_tones() {
        assert_eq!(eo_landmarks::ground_cover::SAND_BERM_SURVEYED_M, crate::ground_tone::BERM_CREST_SURVEYED_M);
    }

    #[test]
    fn the_grid_draws_the_scene_triangle() {
        let g = TerrainGrid { ox: 0.0, oz: 0.0, cell: 10.0, w: 2, h: 2, y: vec![0.0, 10.0, 0.0, 0.0], rgba: vec![0xffff_ffff; 4] };
        // vertices at cell centres: (5, 5) 0, (15, 5) 10, (5, 15) 0, (15, 15) 0.
        assert_eq!(g.height(5.0, 5.0), Some(0.0));
        assert!((g.height(15.0 - 1e-9, 5.0).unwrap() - 10.0).abs() < 1e-6);
        // on the diagonal from (15, 5) to (5, 15) the height is linear between them.
        assert!((g.height(10.0, 10.0).unwrap() - 5.0).abs() < 1e-9);
        // past the diagonal the upper triangle (with (15, 15)) answers.
        assert!((g.height(14.0, 14.0).unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(g.height(-1.0, 5.0), None);
    }
}
