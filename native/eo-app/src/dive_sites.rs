//! Locate the illustrative dive sites on the rendered near terrain. The
//! camera, shell beds and grass roots all sample its actual grid triangles.

use eo_render::{camera::Camera, seabed::Habitat};
use glam::{Vec2, Vec3};

#[derive(Clone, Copy)]
pub struct Site {
    pub habitat: Habitat,
    pub floor: Vec3,
    pub beach: Option<Vec2>,
    pub cove_detail: bool,
    cove_depths: [f32; 15],
    rocky_eye: Option<Vec3>,
}

impl Site {
    pub fn surface_camera(self, aspect: f32, far: f32) -> Camera {
        let shore = self.beach.expect("Aquatic Park has a beach waterline");
        Camera {
            near: 0.10,
            far,
            ..Camera::new(
                Vec3::new(shore.x, eo_water::SEA_LEVEL_Y as f32 + 0.82, shore.y - 2.4),
                -1.04,
                -0.095,
                aspect,
            )
        }
    }

    pub fn cove_camera(self, aspect: f32, far: f32, distance: f32) -> Camera {
        let shore = self.beach.expect("Aquatic Park has a beach waterline");
        // A low eye near the bed. The route descends gradually with the beach.
        let station = (distance / 8.0).clamp(0.0, 14.0);
        let i = station.floor() as usize;
        let depth = self.cove_depths[i]
            + station.fract() * (self.cove_depths[(i + 1).min(14)] - self.cove_depths[i]);
        Camera {
            near: 0.08,
            far,
            ..Camera::new(
                Vec3::new(
                    shore.x,
                    eo_water::SEA_LEVEL_Y as f32 - depth,
                    shore.y - 26.0 - distance,
                ),
                -0.42,
                -0.27,
                aspect,
            )
        }
    }

    pub fn camera(self, aspect: f32, far: f32) -> Camera {
        if let Some(eye) = self.rocky_eye {
            let dir = (self.floor - eye).normalize();
            return Camera {
                near: 0.045,
                far,
                ..Camera::new(eye, dir.x.atan2(-dir.z), dir.y.asin(), aspect)
            };
        }
        if self.cove_detail && self.beach.is_some() {
            return self.cove_camera(aspect, far, 0.0);
        }
        let (yaw, pitch, height) = match self.habitat {
            Habitat::Eelgrass => (-0.38, -0.40, 0.37),
            Habitat::Sand => (-1.12, -0.51, 0.49),
            Habitat::Rocky => unreachable!("a rocky site has a surveyed viewing position"),
        };
        Camera {
            near: 0.10,
            far,
            ..Camera::new(self.floor + Vec3::Y * height, yaw, pitch, aspect)
        }
    }
}

/// Match the diagonal used by the terrain mesh and collision carpet.
fn triangle_height(q: Vec2, mut height: impl FnMut(glam::IVec2) -> f32) -> f32 {
    let cell = q.floor().as_ivec2();
    let uv = q - cell.as_vec2();
    let a = height(cell + glam::IVec2::X);
    let b = height(cell + glam::IVec2::Y);
    if uv.x + uv.y <= 1.0 {
        height(cell) * (1.0 - uv.x - uv.y) + a * uv.x + b * uv.y
    } else {
        height(cell + glam::IVec2::ONE) * (uv.x + uv.y - 1.0) + a * (1.0 - uv.y) + b * (1.0 - uv.x)
    }
}

/// Interpolate the drawn terrain triangles and reuse their vertex heights at nearby points.
pub fn terrain_bed<'a>(
    grid: &eo_data::ElevationGrid,
    vertex_height: &'a dyn Fn(f32, f32) -> f32,
) -> impl FnMut(Vec2) -> f32 + 'a {
    use std::collections::HashMap;
    let cell_m = grid.meta.cell_size as f32;
    let origin = Vec2::new(grid.meta.origin_x as f32, grid.meta.origin_z as f32);
    let mut cache = HashMap::new();
    move |p: Vec2| {
        triangle_height((p - origin) / cell_m - 0.5, |cell| {
            *cache.entry(cell).or_insert_with(|| {
                let q = origin + (cell.as_vec2() + 0.5) * cell_m;
                vertex_height(q.x, q.y)
            })
        })
    }
}

pub fn prepare(
    scene: &mut eo_render::scene::Scene,
    field: &eo_water::field::ShoreField,
    grid: &eo_data::ElevationGrid,
    vertex_height: &dyn Fn(f32, f32) -> f32,
    bed: &mut impl FnMut(Vec2) -> f32,
    build_meshes: bool,
    cove_detail: bool,
    rocky_site: Option<eo_render::seabed::IntertidalOptions>,
    profile: Option<&crate::cove_bed::CoveBed>,
) -> Vec<Site> {
    let cell_m = grid.meta.cell_size as f32;
    let origin = Vec2::new(grid.meta.origin_x as f32, grid.meta.origin_z as f32);
    // Shore-normal transects into Aquatic Park Cove and off Ocean Beach.
    // The live ground scale, water mask and depth select each actual site.
    let mut sites = vec![];
    let transect = Vec2::from(eo_data::groundscale::plan32([-365.0, -3070.0]));
    let beach = (0..400)
        .map(|i| transect - Vec2::Y * i as f32)
        .find(|p| field.signed_exact(p.x as f64, p.y as f64) > 0.0);
    // Sample the reconstructed reference profile even for the comparison cameras
    // with the feature off. Both captures therefore use identical eye positions.
    let mut cove_depths = [0.0; 15];
    if let (Some(shore), Some(profile)) = (beach, profile) {
        for (i, depth) in cove_depths.iter_mut().enumerate() {
            let p = shore - Vec2::Y * (26.0 + 8.0 * i as f32);
            let q = (p - origin) / cell_m - 0.5;
            let floor = triangle_height(q, |cell| {
                let v = origin + (cell.as_vec2() + 0.5) * cell_m;
                let original = || vertex_height(v.x, v.y);
                match profile.sample(v.x as f64, v.y as f64) {
                    Some((h, 1.0)) => h as f32,
                    Some((h, w)) => (original() as f64 * (1.0 - w) + h * w) as f32,
                    None => original(),
                }
            });
            let bottom_depth = eo_water::SEA_LEVEL_Y as f32 - floor;
            *depth = (bottom_depth - 0.34 - 0.10 * i as f32 / 14.0).max(0.0);
        }
        if cove_detail {
            eo_render::seabed::cove_shore(scene, shore, bed);
            let p = shore - Vec2::Y * 26.0;
            if build_meshes {
                eo_render::seabed::append_cove(scene, p, bed);
            }
            sites.push(Site {
                habitat: Habitat::Eelgrass,
                beach,
                cove_detail,
                cove_depths,
                rocky_eye: None,
                floor: Vec3::new(p.x, bed(p), p.y),
            });
        }
    }
    if let Some(options) = rocky_site {
        // West tip of mapped beach OSM way 224941181, where the park records
        // rocky intertidal habitat along the boundary wall toward Municipal Pier.
        let start = Vec2::from(eo_data::groundscale::plan32([-458.0, -3156.0]));
        let seaward = Vec2::new(0.14, -0.99).normalize();
        let centre = (0..=64).map(|i| start + seaward * i as f32).find(|p| {
            let shore = field.signed_exact(p.x as f64, p.y as f64);
            let depth = eo_water::SEA_LEVEL_Y as f32 - bed(*p);
            shore > 0.8 && (0.92..1.85).contains(&depth)
        });
        if let Some(centre) = centre
            && let Some((eye, at)) =
                eo_render::seabed::intertidal_site_with_options(scene, centre, bed, options)
        {
            sites.push(Site {
                habitat: Habitat::Rocky,
                floor: at,
                beach: None,
                cove_detail,
                cove_depths,
                rocky_eye: Some(eye),
            });
        }
    }
    for (habitat, start, direction) in [
        (Habitat::Eelgrass, [-365.0, -2860.0], Vec2::NEG_Y),
        (Habitat::Sand, [-7570.0, 1500.0], Vec2::NEG_X),
    ] {
        if habitat == Habitat::Eelgrass && cove_detail && beach.is_some() {
            continue;
        }
        let start = Vec2::from(eo_data::groundscale::plan32(start));
        for step in 0..1100 {
            let p = start + direction * (2.0 * step as f32);
            let shore = field.signed_exact(p.x as f64, p.y as f64);
            if !(15.0..250.0).contains(&shore) {
                continue;
            }
            let floor = bed(p);
            let depth = eo_water::SEA_LEVEL_Y as f32 - floor;
            let depth_range = if habitat == Habitat::Eelgrass {
                0.72..1.55
            } else {
                1.35..3.4
            };
            if !depth_range.contains(&depth) {
                continue;
            }
            if build_meshes {
                eo_render::seabed::append(scene, habitat, p, bed);
            }
            sites.push(Site {
                habitat,
                beach: if habitat == Habitat::Eelgrass {
                    beach
                } else {
                    None
                },
                cove_detail,
                cove_depths,
                rocky_eye: None,
                floor: Vec3::new(p.x, floor, p.y),
            });
            break;
        }
    }
    sites
}
