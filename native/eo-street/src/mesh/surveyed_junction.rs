//! Opt-in local pavement from a surveyed outer curb line and traffic island.
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use eo_data::sim_source::SurveyedTrafficRoutes;
use serde::Deserialize;

use super::{CURB_FACE, PAD, VERGE, p3};
use crate::{Layer, StreetWorld, TileMesh, road_openings::RoadOpenings, tile_key};

type Xz = [f64; 2];
type Point = [f64; 3];
type Face = [Point; 3];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Survey {
    ground_scale: f64,
    #[serde(rename = "positionsXZ")]
    positions_xz: Vec<Xz>,
    #[serde(rename = "outerBoundaryXZ")]
    outer_boundary_xz: Vec<Xz>,
    #[serde(rename = "islandBoundaryXZ")]
    island_boundary_xz: Vec<Xz>,
    carriageway_triangles: Vec<[usize; 3]>,
    pub(crate) traffic_routes: Option<SurveyedTrafficRoutes>,
    #[serde(skip)]
    pub(crate) digest: u64,
}

fn cross(a: Xz, b: Xz, p: Xz) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn ring_edges(points: &[Xz]) -> impl Iterator<Item = (Xz, Xz)> + '_ {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
}

fn mean(points: &[Xz]) -> Xz {
    std::array::from_fn(|i| points.iter().map(|p| p[i]).sum::<f64>() / points.len() as f64)
}

fn on_boundary(p: Xz, boundary: &[Xz]) -> bool {
    ring_edges(boundary).any(|(a, b)| {
        cross(a, b, p).abs() < 1e-7
            && (0..2).all(|i| p[i] >= a[i].min(b[i]) - 1e-7 && p[i] <= a[i].max(b[i]) + 1e-7)
    })
}

impl Survey {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let mut survey: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("reading surveyed junction {}", path.display()))?;
        ensure!(
            survey.ground_scale == eo_data::groundscale::scale(),
            "the surveyed junction and street plan use different ground scales"
        );
        for ring in [&survey.outer_boundary_xz, &survey.island_boundary_xz] {
            ensure!(
                ring.len() >= 3 && ring.iter().flatten().all(|x| x.is_finite()),
                "invalid surveyed curb ring"
            );
            ensure!(
                ring_edges(ring).all(|(a, b)| a != b),
                "repeated surveyed curb vertex"
            );
        }
        ensure!(
            survey.positions_xz.iter().flatten().all(|x| x.is_finite()),
            "invalid surveyed pavement position"
        );
        ensure!(
            !survey.carriageway_triangles.is_empty(),
            "the surveyed junction has no carriageway"
        );
        for &ids in &survey.carriageway_triangles {
            ensure!(
                ids.iter().all(|&i| i < survey.positions_xz.len()),
                "invalid surveyed pavement index"
            );
            let [a, b, c] = ids.map(|i| survey.positions_xz[i]);
            ensure!(
                cross(a, b, c) > 0.0,
                "inverted or degenerate surveyed pavement face"
            );
        }
        let center = mean(&survey.island_boundary_xz);
        let sides: Vec<_> = ring_edges(&survey.island_boundary_xz)
            .map(|(a, b)| cross(a, b, center))
            .collect();
        ensure!(
            sides.iter().all(|&v| v > 0.0) || sides.iter().all(|&v| v < 0.0),
            "the island curb must be visible from its center"
        );
        let mut digest = DefaultHasher::new();
        bytes.hash(&mut digest);
        survey.digest = digest.finish();
        Ok(survey)
    }
}

pub(crate) struct Surface {
    pub(crate) owner: (i32, i32),
    pub(crate) pavement: Vec<Face>,
    island: Vec<Face>,
    curb: Vec<[Point; 4]>,
    replacement: RoadOpenings,
    elevation_margin: f64,
}

impl Surface {
    pub(crate) fn build(w: &StreetWorld, survey: &Survey) -> Self {
        let terrain_y = |p: Xz| w.ground(p[0], p[1]) + w.o.road_lift;
        let pavement_vertex = |p: Xz| {
            let y = terrain_y(p);
            // Keep the existing approach height at each boundary station.
            let y = if on_boundary(p, &survey.outer_boundary_xz) {
                w.carriageway_near(p[0], p[1], y).unwrap_or(y)
            } else {
                y
            };
            p3(p, y)
        };
        let positions: Vec<_> = survey
            .positions_xz
            .iter()
            .copied()
            .map(pavement_vertex)
            .collect();
        let pavement = survey
            .carriageway_triangles
            .iter()
            .map(|ids| ids.map(|i| positions[i]))
            .collect::<Vec<_>>();
        let center = mean(&survey.island_boundary_xz);
        let bottom = |p| p3(p, terrain_y(p));
        let top = |p| p3(p, terrain_y(p) + w.o.curb_face_height);
        let mut island = Vec::new();
        let mut curb = Vec::new();
        for (a, b) in ring_edges(&survey.island_boundary_xz) {
            island.push([top(center), top(a), top(b)]);
            curb.push([bottom(a), bottom(b), top(b), top(a)]);
        }
        // Remove the superseded ground faces throughout the patch, including
        // the old paving underneath the island. Separate structure meshes
        // are built elsewhere.
        let elevation_margin = w.o.verge_max_drop + w.o.curb_face_height;
        let mut replacement = RoadOpenings::default();
        for &face in pavement.iter().chain(&island) {
            replacement.add_surface(
                face.map(|p| p.map(|v| v as f32)),
                elevation_margin,
                elevation_margin,
            );
        }
        Self {
            owner: tile_key(center[0], center[1]),
            pavement,
            island,
            curb,
            replacement,
            elevation_margin,
        }
    }

    fn road_y(&self, p: Xz) -> Option<f64> {
        for &[a, b, c] in &self.pavement {
            let xz = |v: Point| [v[0], v[2]];
            let [u, v, w] = [a, b, c].map(xz);
            let area = cross(u, v, w);
            let weights = [cross(v, w, p), cross(w, u, p), cross(u, v, p)];
            if weights.iter().all(|&v| v >= -1e-7) {
                return Some((weights[0] * a[1] + weights[1] * b[1] + weights[2] * c[1]) / area);
            }
        }
        None
    }

    pub(crate) fn draw(&self, tile: (i32, i32), mesh: &mut TileMesh, paint_lift: f64) {
        let mut markings = Vec::new();
        if !mesh.indices.is_empty() {
            let (lo, hi) = mesh.bounds();
            if self
                .replacement
                .overlaps_bounds(lo.map(f64::from), hi.map(f64::from))
            {
                for (face, layer) in mesh.indices.chunks_exact(3).zip(&mesh.layers) {
                    if *layer != Layer::Marking {
                        continue;
                    }
                    let ids = [face[0], face[1], face[2]].map(|i| i as usize);
                    let ground = ids.map(|i| {
                        let [x, y, z] = mesh.positions[i].map(f64::from);
                        let new_y = self.road_y([x, z])?;
                        ((new_y - y).abs() <= self.elevation_margin).then_some([
                            x,
                            new_y + paint_lift,
                            z,
                        ])
                    });
                    if let [Some(a), Some(b), Some(c)] = ground {
                        let colors = ids.map(|i| {
                            let [r, g, b, _] = mesh.colors[i].to_le_bytes();
                            [r, g, b]
                        });
                        markings.push(([a, b, c], colors));
                    }
                }
                mesh.clip_faces(0, &self.replacement, |_| true);
            }
        }
        for (face, colors) in markings {
            mesh.tri(Layer::Marking, face, colors);
        }
        if tile == self.owner {
            for &face in &self.pavement {
                mesh.tri(Layer::Carriageway, [face[0], face[2], face[1]], [PAD(); 3]);
            }
            for &face in &self.island {
                mesh.tri(Layer::Verge, face, [VERGE(); 3]);
            }
            for &quad in &self.curb {
                mesh.quad(Layer::CurbFace, quad, [CURB_FACE(); 4], None);
            }
        }
    }
}
