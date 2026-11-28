//! Yerba Buena Island has one vault over two traffic levels. The cross-section
//! through the westbound roadway and the clear height under the intermediate
//! deck are from WSP's December 2021 assessment for Caltrans, figure 14,
//! table 3 (p. 9), recorded in the street-standards table.
use crate::Chain;
use anyhow::Result;
use eo_data::CaStandards;

/// The mapped upper transition viaduct leaves the east portal on a
/// cast-in-place concrete structure above eastbound traffic. Bare-earth
/// elevations along the cut are not an embankment supporting this deck.
/// Toll Bridge Program Oversight Committee, 2015 Q3, YBITS #1:
/// https://files.mtc.ca.gov/library/pub/25171_2015_3.pdf
pub(crate) fn east_transition(chain: &Chain) -> bool {
    chain.pieces.iter().any(|p| p.way == "497579295")
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Dimensions {
    upper: IslandTunnelSection,
    lower: IslandTunnelSection,
}

impl Dimensions {
    pub fn read(ca: &CaStandards) -> Result<Option<Self>> {
        let mut enabled = false;
        for arg in std::env::args() {
            match arg.as_str() {
                "--yerba-buena-section" => enabled = true,
                "--no-yerba-buena-section" => return Ok(None),
                _ => {}
            }
        }
        if !enabled {
            return Ok(None);
        }
        let measure = |key| ca.metric_key("yerba-buena-tunnel-interior-section", key);
        Ok(Some(Self {
            upper: IslandTunnelSection::Vault {
                half_width: measure("width")? / 2.0,
                wall: measure("spring")?,
                crown: measure("crown")?,
            },
            lower: IslandTunnelSection::Flat {
                clearance: measure("lower")?,
            },
        }))
    }

    pub fn section(self, chain: &Chain) -> Option<IslandTunnelSection> {
        chain
            .pieces
            .iter()
            .find_map(|piece| match piece.way.as_str() {
                "11415208" => Some(self.upper),
                "50691047" => Some(self.lower),
                _ => None,
            })
    }

    /// Carry a vertical curve across both construction meshes at the island
    /// tunnel. Rounding each side independently leaves the two approach
    /// tangents meeting at a point. The tyre path includes pavement camber.
    pub(crate) fn round_lower_landings(
        self,
        chains: &mut [Chain],
        streets: &mut eo_street::StreetWorld,
    ) {
        let Some(bore) = chains
            .iter_mut()
            .find(|c| c.is_built() && self.section(c) == Some(self.lower))
        else {
            return;
        };
        let Some(joins) = streets.o.structures.clone() else {
            return;
        };
        let mut changed = false;
        let length = bore.length();
        for end in 0..2 {
            let Some(piece) = (if end == 0 {
                bore.pieces.first()
            } else {
                bore.pieces.last()
            }) else {
                continue;
            };
            let Some((entry_i, street_end)) =
                streets.plan.entries.iter().enumerate().find_map(|(i, e)| {
                    if e.structure_leg.is_some() {
                        return None;
                    }
                    e.structure_ends.iter().enumerate().find_map(|(k, id)| {
                        let j = &joins.ends[(*id)?];
                        (j.at == bore.ends[end].at && j.segment == piece.segment).then_some((i, k))
                    })
                })
            else {
                continue;
            };
            let entry = &streets.plan.entries[entry_i];
            let geom = &mut streets.geoms[entry_i];
            let inside = |s| if end == 0 { s } else { length - s };
            let outside = |s| {
                if street_end == 0 {
                    s - entry.s0()
                } else {
                    entry.s1() - s
                }
            };
            let end_crown = bore.ends[end]
                .surface
                .iter()
                .find(|p| p[0].abs() < f64::EPSILON)
                .map_or(0., |p| p[1]);
            let crown = |d: f64| end_crown * (1.0 - d / crate::sweep::CROWN_BLEND_M).clamp(0., 1.);
            let street_crown = streets.o.cross_section_y(0., 0., entry.half);
            // Negative stations are inside the tunnel, positive on its street.
            let mut profile: Vec<(f64, f64)> = bore
                .stations
                .iter()
                .zip(&bore.heights)
                .map(|(&s, &y)| {
                    let d = inside(s);
                    (-d, y + crown(d))
                })
                .collect();
            profile.extend(
                geom.stations
                    .iter()
                    .zip(&geom.datums)
                    .map(|(f, &y)| (outside(f.s), y + street_crown)),
            );
            profile.sort_by(|a, b| a.0.total_cmp(&b.0));
            let reach = crate::VERTICAL_CURVE_M * 0.5;
            let left = profile.partition_point(|p| p.0 <= -reach).saturating_sub(1);
            let right = profile.partition_point(|p| p.0 < reach);
            if left == 0 || right + 1 >= profile.len() {
                continue;
            }
            let (a, b) = (profile[left], profile[right]);
            let slope = |p: (f64, f64), q: (f64, f64)| (q.1 - p.1) / (q.0 - p.0);
            let (g0, g1) = (slope(profile[left - 1], a), slope(b, profile[right + 1]));
            let span = b.0 - a.0;
            // A cubic transition matches the unchanged edge heights and their
            // longitudinal grades. Equal tangent intersections give a parabola.
            let top = |d: f64| {
                let t = (d - a.0) / span;
                let u = 1. - t;
                a.1 * u * u * (1. + 2. * t)
                    + b.1 * t * t * (3. - 2. * t)
                    + span * t * u * (g0 * u - g1 * t)
            };
            for (&s, y) in bore.stations.iter().zip(&mut bore.heights) {
                let d = inside(s);
                if -d >= a.0 {
                    *y = top(-d) - crown(d);
                }
            }
            for (frame, y) in geom.stations.iter().zip(&mut geom.datums) {
                let d = outside(frame.s);
                if d <= b.0 {
                    *y = top(d) - street_crown;
                }
            }
            bore.ends[end].fixed = Some(top(0.) - end_crown);
            changed = true;
        }
        if changed {
            streets.refresh_graded_pavement();
        }
    }

    /// Elevation of the exposed intermediate deck above the lower ceiling
    /// lining. Project each upper station onto the other mapped carriageway:
    /// the two centrelines run in opposite directions and their grades differ.
    pub(crate) fn upper_soffits(
        self,
        shape: IslandTunnelSection,
        stations: &[crate::sweep::Station],
        tunnels: &[&Chain],
    ) -> Option<Vec<f64>> {
        if !matches!(shape, IslandTunnelSection::Vault { .. }) {
            return None;
        }
        let lower = tunnels
            .iter()
            .find(|c| self.section(c) == Some(self.lower))?;
        Some(
            stations
                .iter()
                .map(|station| {
                    let (s, _) = lower.project(station.x, station.z);
                    self.lower.lining_top(lower.y_at(s))
                })
                .collect(),
        )
    }
}

/// Inside face above the surveyed roadway. Heights stay in metres while
/// the normalized lateral coordinate follows the width of the game's road.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IslandTunnelSection {
    Vault {
        half_width: f64,
        wall: f64,
        crown: f64,
    },
    Flat {
        clearance: f64,
    },
}

impl IslandTunnelSection {
    pub(crate) fn lining_top(self, road_y: f64) -> f64 {
        road_y + self.crown() + crate::tunnel::ROOF_M
    }

    pub fn crown(self) -> f64 {
        match self {
            Self::Vault { crown, .. } => crown,
            Self::Flat { clearance } => clearance,
        }
    }

    /// Radius in the measured, unscaled cross section. The game can stretch
    /// the lateral span independently of the surveyed vertical dimensions.
    fn circle(self) -> Option<(f64, f64, f64)> {
        let Self::Vault {
            half_width,
            wall,
            crown,
        } = self
        else {
            return None;
        };
        let rise = crown - wall;
        let radius = (half_width * half_width + rise * rise) / (2.0 * rise);
        Some((half_width, crown, radius))
    }

    pub fn height(self, across: f64) -> f64 {
        let Some((half, crown, radius)) = self.circle() else {
            return self.crown();
        };
        let x = across.clamp(-1., 1.) * half;
        crown - x * x / (radius + (radius * radius - x * x).max(0.).sqrt())
    }

    /// Slope in the drawn section and signed developed distance from the
    /// crown in the source section. Both are continuous through the crown.
    pub(crate) fn vault_face(self, across: f64, drawn_half: f64) -> (f64, f64) {
        let Some((half, _, radius)) = self.circle() else {
            return (0., across * drawn_half);
        };
        let x = across.clamp(-1., 1.) * half;
        let slope = -x * half / (drawn_half * (radius * radius - x * x).sqrt());
        (slope, radius * (x / radius).asin())
    }
}

/// Each panel has a different ceiling plane. Preserving the space down to
/// its own road plane also preserves traffic on the deck immediately below.
pub(crate) fn vault_openings(
    shape: IslandTunnelSection,
    road: u32,
    sweep: &crate::sweep::Sweep,
    stations: [&crate::sweep::Station; 2],
    openings: &mut eo_street::road_openings::RoadOpenings,
) {
    let edge = |st: &crate::sweep::Station, fraction: f64| {
        let u = fraction * st.half;
        let point = st.at(u, sweep.top(st, u)).map(|v| v as f32);
        let height = shape.height(u / crate::tunnel::inner(st));
        (point, height)
    };
    // Use the same angular divisions as the near arch mesh.
    let panels = 12;
    for i in 0..panels {
        let position = |j| (std::f64::consts::PI * f64::from(j) / f64::from(panels)).cos();
        let [left, right] = [position(i), position(i + 1)];
        let [a, b] = stations.map(|st| [edge(st, left), edge(st, right)]);
        for tri in [[a[0], b[0], b[1]], [a[0], b[1], a[1]]] {
            openings.add_graded_roof(
                road,
                tri.map(|v| v.0),
                crate::tunnel::ROOF_M,
                tri.map(|v| v.1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_island_vault_reaches_the_surveyed_crown_and_spring() {
        let ca = CaStandards::load(&eo_data::DataPaths::discover().unwrap()).unwrap();
        let m = |key| {
            ca.metric_key("yerba-buena-tunnel-interior-section", key)
                .unwrap()
        };
        let profile = IslandTunnelSection::Vault {
            half_width: m("width") / 2.0,
            wall: m("spring"),
            crown: m("crown"),
        };
        assert_eq!(profile.height(0.), m("crown"));
        for side in [-1., 1.] {
            assert!((profile.height(side) - m("spring")).abs() < 1e-10);
            assert!(profile.height(side * 0.5) > profile.height(side * 0.9));
        }
        // The normal must follow the drawn width at every detail level.
        let half = m("width") * 0.625;
        for across in [-0.98, -0.5, 0., 0.5, 0.98] {
            let h = 1e-5;
            let slope = (profile.height(across + h) - profile.height(across - h)) / (2. * h * half);
            let (actual, arc) = profile.vault_face(across, half);
            assert!((slope - actual).abs() < 1e-7);
            assert!((arc + profile.vault_face(-across, half).1).abs() < 1e-10);
        }
        let slab = IslandTunnelSection::Flat {
            clearance: m("lower"),
        };
        assert_eq!(slab.height(-0.9), slab.height(0.9));
    }
}
