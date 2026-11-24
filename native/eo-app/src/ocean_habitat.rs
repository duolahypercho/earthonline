//! The renderer and dive camera sample the same mapped ocean domain.
use anyhow::{Context, Result};
use eo_render::ocean_life::field::{SEA, Sampler, Site};
use glam::Vec2;
use std::sync::Arc;

/// The city lens clips at half a metre. A diver can inspect objects within arm's reach.
pub fn dive_lens(world: &crate::World, cam: &mut eo_render::camera::Camera) {
    if cam.eye.y <= SEA + 0.3
        && let Some(sample) = &world.marine_floor
        && sample(Vec2::new(cam.eye.x, cam.eye.z))
            .is_some_and(|s| s.shore > 0. && s.floor < cam.eye.y)
    {
        cam.near = cam.near.min(0.045);
    }
}

pub fn sampler(world: &crate::World, cinematic: bool) -> Result<Sampler> {
    let grid =
        crate::props::vegetation::TerrainGrid::from_scene(&world.scene, &world.elevation.meta)
            .context("missing rendered ocean ground")?;
    let water = world
        .water
        .clone()
        .context("ocean habitats need the shoreline")?;
    let shelf = eo_data::shelf::Shelf::load(&world.paths)?;
    let m = &world.elevation.meta;
    let geo = eo_data::geo::GeoFrame::of_city(&world.city.meta);
    let scale = eo_data::groundscale::scale();
    let min = Vec2::new(m.origin_x as f32, m.origin_z as f32);
    let max = Vec2::new(
        world.elevation.max_x() as f32,
        world.elevation.max_z() as f32,
    );
    let cell = m.cell_size as f32;
    Ok(Arc::new(move |p| {
        let south = if cinematic {
            eo_render::water::CINEMATIC_SOUTH_REACH_M
        } else {
            0.
        };
        if p.x < min.x - 60000.
            || p.x > max.x + 60000.
            || p.y < min.y - 60000.
            || p.y > max.y + south
        {
            return None;
        }
        let x = p.x as f64;
        let z = p.y as f64;
        let pacific = ((-x / scale - 6000.) / 800.).clamp(0., 1.) as f32;
        if let Some(h) = grid.height(x, z) {
            return Some(Site {
                floor: h as f32,
                shore: water.field.signed_exact(x, z) as f32,
                beach: water.field.beach_at(x, z) as f32,
                pacific,
                offshore: false,
            });
        }
        let (lat, lon) = geo.lat_lon(x, z);
        let mut height = shelf.height_msl(lon, lat)? + SEA;
        // Join the outer relief to the exact city carpet over one coarse shelf cell.
        let edge = p.clamp(
            min + Vec2::splat(cell * 0.51),
            max - Vec2::splat(cell * 0.51),
        );
        let distance = p.distance(edge);
        if distance < 420.
            && let Some(h) = grid.height(edge.x as f64, edge.y as f64)
        {
            let u = distance / 420.;
            height = h as f32 + (height - h as f32) * u * u * (3. - 2. * u);
        }
        Some(Site {
            floor: height,
            shore: 400.,
            beach: 0.,
            pacific,
            offshore: true,
        })
    }))
}

use eo_player::world::{GroundProvider, SurfaceMesh, TileKey};
use glam::DVec3;

struct SeaGround {
    land: Arc<dyn GroundProvider>,
    sea: Sampler,
}

impl GroundProvider for SeaGround {
    fn water_column(&self, x: f64, z: f64) -> Option<(f64, f64)> {
        let sample = (self.sea)(Vec2::new(x as f32, z as f32))?;
        (sample.shore > 0. && sample.floor < SEA).then_some((sample.floor as f64, SEA as f64))
    }
    fn ground_y(&self, x: f64, z: f64) -> f64 {
        self.water_column(x, z)
            .map_or_else(|| self.land.ground_y(x, z), |column| column.0)
    }
    fn multi_level(&self) -> bool {
        self.land.multi_level()
    }
    fn surface_tile(&self, key: TileKey) -> Option<SurfaceMesh> {
        self.land.surface_tile(key)
    }
    fn terrain_cut(&self, x: f64, z: f64) -> Option<f64> {
        self.land.terrain_cut(x, z)
    }
}
pub fn swim_ground(world: &crate::World, land: Arc<dyn GroundProvider>) -> Arc<dyn GroundProvider> {
    match &world.marine_floor {
        Some(sea) => Arc::new(SeaGround {
            land,
            sea: sea.clone(),
        }),
        None => land,
    }
}

// Named entries are camera bookmarks. The distribution itself has no site list.
// Fixed bookmarks in surveyed plan metres, with the measured floor at each.
// Keeping the reference lens independent of the feature gives matched bench paths.
pub const STARTS: &[(&str, f64, f64, f64, f64)] = &[
    ("reef", -8004.0, -1636.0, 0.8, -8.549),
    ("bay", -312.3512, -3311.1768, -0.8, -4.144),
    ("black-point", -639.4992, -3527.8256, -1.8, -5.412),
    ("pacific", -8513.0784, 866.3344, -1.4, -5.036),
    ("channel", -5310.3664, -4556.6784, -1.4, -91.197),
    ("east-bay", 5242.3696, -4386.96, 1.2, -8.642),
    ("south-bay", 5145.5968, 5992.596, -1.7, -7.576),
    ("shelf", -27328.528, 1437.4624, -2.1, -38.14),
];

pub fn reference_start(name: &str) -> Result<(DVec3, f64)> {
    let &(_, x, z, heading, floor) = STARTS
        .iter()
        .find(|s| s.0 == name)
        .context("unknown dive region")?;
    let scale = eo_data::groundscale::scale();
    Ok((DVec3::new(x * scale, floor + 0.18, z * scale), heading))
}

pub fn start(sample: &Sampler, name: &str) -> Result<(DVec3, f64)> {
    let (mut feet, heading) = reference_start(name)?;
    let column = sample(Vec2::new(feet.x as f32, feet.z as f32))
        .filter(|s| s.shore > 0. && s.floor < SEA - 1.6)
        .with_context(|| format!("no swimmable entry at {name}"))?;
    feet.y = if name == "reef" {
        (f64::from(column.floor) + 3.3).min(f64::from(SEA) - 1.6)
    } else {
        f64::from(column.floor) + 0.18
    };
    Ok((feet, heading))
}
/// Restore the measured sounding to wet carpet vertices. Keep the constructed quay
/// tops and blend across the last few metres of the foreshore.
pub fn survey_floor(
    elev: &eo_data::ElevationGrid,
    shelf: &eo_data::shelf::Shelf,
    geo: eo_data::geo::GeoFrame,
    field: &eo_water::field::ShoreField,
    x: f32,
    z: f32,
    original: f32,
) -> f32 {
    if original > SEA + 0.05 {
        return original;
    }
    let (x, z) = (f64::from(x), f64::from(z));
    let shore = field.signed_exact(x, z);
    if shore <= 0. {
        return original;
    }
    let m = &elev.meta;
    let i = ((x - m.origin_x) / m.cell_size).floor() as isize;
    let j = ((z - m.origin_z) / m.cell_size).floor() as isize;
    let survey = if (0..m.width as isize).contains(&i) && (0..m.height as isize).contains(&j) {
        let v = elev.topobathy[j as usize * m.width + i as usize];
        (v != m.nodata as i16).then(|| {
            eo_data::groundscale::lift_by(
                f64::from(v) * 0.01 + eo_data::NAVD88_TO_GAME_Y,
                elev.height_scale,
            ) as f32
        })
    } else {
        None
    };
    let (lat, lon) = geo.lat_lon(x, z);
    let h = survey.filter(|&h| h < SEA).or_else(|| {
        shelf
            .height_msl(lon, lat)
            .map(|v| SEA + v)
            .filter(|&h| h < SEA)
    });
    let u = (shore as f32 / 24.).clamp(0., 1.);
    h.map_or(original, |h| {
        original + (h - original) * u * u * (3. - 2. * u)
    })
}
