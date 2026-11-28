//! Measured concrete behind the southwest curb at the Mariposa rail crossing.
//! Existing street paving owns its footprint. The apron supplies the deck
//! behind it, above the separate terrain and rail bed.
use super::{Layer, StreetWorld, TileMesh, WALK};
use crate::{SurfaceTile, road_openings::RoadOpenings, tile_key};
use eo_data::bridge_survey::{APRON_ORIGIN, APRON_SPACING, MARIPOSA_APRON, game_y};
use std::collections::BTreeMap;

type Triangle = [[f32; 3]; 3];
type Key = (i32, i32);

/// Join a survey sample to the existing curb within one survey interval.
/// Gradually applying the difference on the exposed side avoids a sharp
/// slope where a clipped fragment is very close to the edge of the sidewalk.
fn join_height(
    at: [f64; 3],
    edges: &TileMesh,
    surface: &SurfaceTile,
    reach: f64,
    level: f64,
) -> f32 {
    let same_level = |y: f64| (y - at[1]).abs() < level;
    if let Some(y) = surface
        .hits_at(at[0], at[2])
        .map(|(_, y)| y)
        .filter(|&y| same_level(y))
        .reduce(f64::max)
    {
        return y as f32;
    }
    let (mut nearest, mut height) = (reach * reach, at[1]);
    for tri in edges.indices.chunks_exact(3) {
        for i in 0..3 {
            let a = edges.positions[tri[i] as usize].map(f64::from);
            let b = edges.positions[tri[(i + 1) % 3] as usize].map(f64::from);
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            let length2 = dx * dx + dz * dz;
            if length2 == 0. {
                continue;
            }
            let t = (((at[0] - a[0]) * dx + (at[2] - a[2]) * dz) / length2).clamp(0., 1.);
            let y = a[1] + t * (b[1] - a[1]);
            let d2 = (a[0] + t * dx - at[0]).powi(2) + (a[2] + t * dz - at[2]).powi(2);
            if same_level(y) && (d2 < nearest || (d2 == nearest && y > height)) {
                nearest = d2;
                height = y;
            }
        }
    }
    (height + (at[1] - height) * (nearest.sqrt() / reach)) as f32
}

#[derive(Default)]
pub(crate) struct Apron {
    tiles: BTreeMap<Key, Vec<Triangle>>,
    banks: RoadOpenings,
    owners: Option<(Key, Key)>,
}

impl Apron {
    pub(crate) fn fit_curb(plan: &mut crate::Plan, city: &crate::StreetCity) {
        // The two footways taper to the motorway entrance. Sweeping the
        // wider sidewalk past the arc centre folds the same slab back over
        // itself. The surveyed apron continues from the centre to the rear.
        for node in &mut plan.nodes {
            let way = |arm: usize| {
                city.segments[plan.entries[node.approaches[arm].entry].road]
                    .street_id
                    .as_deref()
            };
            for (path, corner) in node.paths.iter_mut().zip(&node.corners) {
                let Some(corner) = corner else { continue };
                if !matches!(
                    (way(path.a), way(path.b)),
                    (Some("sf-street-8915249"), Some("sf-street-396965550"))
                        | (Some("sf-street-396965550"), Some("sf-street-8915249"))
                ) {
                    continue;
                }
                for station in &mut path.stations {
                    if station.ang.is_some() {
                        station.walk = station.walk.min(corner.radius / station.scale);
                    }
                }
            }
        }
    }

    pub(crate) fn fit(world: &StreetWorld) -> Self {
        let mut apron = Self::default();
        let scale = eo_data::groundscale::scale();
        let plan = |col: usize, row: usize| {
            [
                (APRON_ORIGIN[0] + col as f64 * APRON_SPACING) * scale,
                (APRON_ORIGIN[1] + row as f64 * APRON_SPACING) * scale,
            ]
        };
        let last_col = MARIPOSA_APRON[0].len() - 1;
        let last_row = MARIPOSA_APRON.len() - 1;
        let [x0, z0] = plan(0, 0);
        let [x1, z1] = plan(last_col, last_row);
        let lo = tile_key(x0, z0);
        let hi = tile_key(x1, z1);
        let mut paving = RoadOpenings::default();
        let mut neighbouring = TileMesh::default();
        let mut has_street = false;
        for x in lo.0 - 1..=hi.0 + 1 {
            for z in lo.1 - 1..=hi.1 + 1 {
                let Some(tile) = world.tiles.get(&(x, z)) else {
                    continue;
                };
                has_street |= tile.spans.iter().any(|&(i, _)| {
                    let entry = &world.plan.entries[i as usize];
                    world.city.segments[entry.road].street_id.as_deref()
                        == Some("sf-street-396965550")
                });
                let mesh = world.street_tile((x, z), false);
                for (&layer, indices) in mesh.layers.iter().zip(mesh.indices.chunks_exact(3)) {
                    if !layer.walkable() || layer == Layer::Verge {
                        continue;
                    }
                    let face = [0, 1, 2].map(|k| mesh.positions[indices[k] as usize]);
                    if face.iter().all(|p| f64::from(p[0]) < x0)
                        || face.iter().all(|p| f64::from(p[0]) > x1)
                        || face.iter().all(|p| f64::from(p[2]) < z0)
                        || face.iter().all(|p| f64::from(p[2]) > z1)
                    {
                        continue;
                    }
                    // Only the wearing surface and its curb own this level.
                    let depth = world.o.verge_max_drop;
                    paving.add_surface(face, depth, depth);
                    neighbouring.tri(layer, face.map(|p| p.map(f64::from)), [WALK(); 3]);
                }
            }
        }
        if !has_street {
            return apron;
        }
        apron.owners = Some((lo, hi));
        let mut surfaces = BTreeMap::new();
        for row in 0..last_row {
            for col in 0..last_col {
                let corners = [
                    (col, row),
                    (col + 1, row),
                    (col, row + 1),
                    (col + 1, row + 1),
                ];
                if corners.iter().any(|&(x, z)| MARIPOSA_APRON[z][x] == 0) {
                    continue;
                }
                let points = corners.map(|(x, z)| {
                    let at = plan(x, z);
                    [
                        at[0] as f32,
                        game_y(f64::from(MARIPOSA_APRON[z][x]) / 100.) as f32,
                        at[1] as f32,
                    ]
                });
                let key = tile_key(
                    f64::from(points[0][0] + points[3][0]) / 2.,
                    f64::from(points[0][2] + points[3][2]) / 2.,
                );
                let tris = apron.tiles.entry(key).or_default();
                let surface = surfaces
                    .entry(key)
                    .or_insert_with(|| SurfaceTile::build(&[&neighbouring], key));
                let weld = |mut p: [f32; 3], at: [f64; 3]| {
                    p[1] = join_height(
                        at,
                        &neighbouring,
                        surface,
                        APRON_SPACING * scale,
                        world.o.verge_max_drop,
                    );
                    p
                };
                for face in [
                    [points[0], points[2], points[1]],
                    [points[1], points[2], points[3]],
                ] {
                    // The old earth bank ran from this curb to the bare-earth
                    // grid. Remove that fictitious slope down to its foot.
                    let depth = face
                        .iter()
                        .map(|p| f64::from(p[1]) - world.carpet_y(f64::from(p[0]), f64::from(p[2])))
                        .fold(0.0, f64::max)
                        + world.o.verge_max_drop;
                    apron
                        .banks
                        .add_paving_face(face, depth, world.o.verge_max_drop, true);
                    if let Some(parts) = paving.clip(face) {
                        tris.extend(
                            parts
                                .iter()
                                .map(|part| part.map(|v| weld(v.mesh_position(face), v.pos))),
                        );
                    } else {
                        tris.push(face.map(|p| weld(p, p.map(f64::from))));
                    }
                }
            }
        }
        apron
    }

    pub(crate) fn draw(&self, key: Key, mesh: &mut TileMesh, with_banks: bool) {
        if with_banks && let Some((lo, hi)) = self.owners {
            if key.0 < lo.0 - 1 || key.0 > hi.0 + 1 || key.1 < lo.1 - 1 || key.1 > hi.1 + 1 {
                return;
            }
            let (low, high) = mesh.bounds();
            if !mesh.indices.is_empty() && self.banks.intersects_bounds(low, high) {
                mesh.clip_faces(0, &self.banks, |layer| layer == Layer::Verge);
            }
        }
        if let Some(faces) = self.tiles.get(&key) {
            for face in faces {
                mesh.tri(Layer::Sidewalk, face.map(|p| p.map(f64::from)), [WALK(); 3]);
            }
        }
    }
}
