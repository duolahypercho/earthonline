//! Restore a collapsed, one-way mapped circuit from the surveyed pavement.
use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, ensure};
use eo_data::sim_source::{SurveyedTrafficLine, SurveyedTrafficRoutes};

use super::{NativeSource, geom};
use crate::network::{ControlKind, LaneBuild, Network, NetworkBuild, TileKey, VehicleEnvelope};

pub(super) struct Routes {
    pub(super) tiles: Vec<TileKey>,
    owner: TileKey,
    approaches: HashMap<String, Approach>,
    circulation: Vec<(LaneBuild, Vec<[f64; 2]>)>,
    envelope: VehicleEnvelope,
}

struct Approach {
    inbound: bool,
    points: Vec<[f64; 2]>,
    surface: Vec<[f64; 2]>,
    control: Option<ControlKind>,
}

fn surface(line: &SurveyedTrafficLine) -> Result<Vec<[f64; 2]>> {
    ensure!(
        line.points.len() >= 2 && line.points.len() == line.heights.len(),
        "missing measured lane heights"
    );
    ensure!(
        line.points
            .iter()
            .flatten()
            .chain(&line.heights)
            .all(|v| v.is_finite()),
        "non-finite lane"
    );
    let stations = geom::cumulative(&line.points);
    ensure!(
        stations.windows(2).all(|s| s[0] < s[1]),
        "repeated or invalid lane station"
    );
    Ok(stations
        .into_iter()
        .zip(&line.heights)
        .map(|(s, &y)| [s, y])
        .collect())
}

impl Routes {
    pub(super) fn compile(source: &NativeSource, layout: SurveyedTrafficRoutes) -> Result<Self> {
        let mapped = source
            .collapsed_roads
            .get(&layout.collapsed_way)
            .context("no collapsed source way")?;
        ensure!(
            !mapped.bridge && !mapped.tunnel && source.level_of(mapped) == 0,
            "circulation must be at grade"
        );
        let [length, width] = layout.vehicle_envelope_m;
        let envelope =
            VehicleEnvelope::new(length, width).context("invalid measured vehicle envelope")?;
        ensure!(
            layout.circulation.len() >= 2 && !layout.approaches.is_empty(),
            "incomplete circuit"
        );
        for (line, next) in layout
            .circulation
            .iter()
            .zip(layout.circulation.iter().cycle().skip(1))
        {
            ensure!(
                !line.points.is_empty() && line.points.last() == next.points.first(),
                "open circuit"
            );
            ensure!(
                line.heights.last() == next.heights.first(),
                "circuit grade gap"
            );
        }
        let [x, z] = layout.circulation[0].points[0];
        let owner = TileKey::of(x, z);
        let mut tiles: Vec<_> = layout
            .approaches
            .iter()
            .map(|a| TileKey {
                x: a.tile[0],
                z: a.tile[1],
            })
            .collect();
        tiles.push(owner);
        tiles.sort_unstable();
        tiles.dedup();
        let mut base = source.build(&tiles);
        let mut approaches = HashMap::new();
        for approach in layout.approaches {
            let [x, z] = approach.tile;
            let old = base
                .lanes
                .iter()
                .find(|lane| lane.key == approach.lane && lane.tile == Some(TileKey { x, z }))
                .with_context(|| format!("missing approach {}", approach.lane))?;
            ensure!(
                old.level == 0 && old.outgoing.is_none(),
                "unsupported approach {}",
                approach.lane
            );
            let profile = surface(&approach.line).with_context(|| approach.lane.clone())?;
            let points = approach.line.points;
            let (outer, old_outer, inner, inner_height) = if approach.inbound {
                (
                    points.first(),
                    old.points.first(),
                    points.last(),
                    profile.last(),
                )
            } else {
                (
                    points.last(),
                    old.points.last(),
                    points.first(),
                    profile.first(),
                )
            };
            ensure!(
                outer == old_outer,
                "outer lane endpoint moved on {}",
                approach.lane
            );
            ensure!(
                layout.circulation.iter().any(|l| l.points.first() == inner
                    && l.heights.first().copied() == inner_height.map(|h| h[1])),
                "approach {} does not meet the circuit at its grade",
                approach.lane
            );
            let way = approach
                .lane
                .strip_prefix('n')
                .and_then(|key| key.split_once(':'))
                .map(|(way, _)| way)
                .context("invalid native lane key")?;
            let control = match approach.stop_id {
                Some(id) if approach.inbound => {
                    ensure!(
                        old.control == ControlKind::None
                            || matches!(old.control, ControlKind::MappedStop { id: old, .. } if old == id),
                        "cannot replace a different mapped control"
                    );
                    Some(
                        source
                            .stops
                            .surveyed_control(way, id, &points, source.dims.limit_line_width_m)
                            .with_context(|| {
                                format!("stop {id} does not bind to {}", approach.lane)
                            })?,
                    )
                }
                None if !approach.inbound || old.control == ControlKind::None => None,
                _ => anyhow::bail!("invalid signed approach {}", approach.lane),
            };
            let route = Approach {
                inbound: approach.inbound,
                points,
                surface: profile,
                control,
            };
            ensure!(
                approaches.insert(approach.lane, route).is_none(),
                "repeated approach"
            );
        }
        ensure!(
            approaches.values().any(|a| a.inbound) && approaches.values().any(|a| !a.inbound),
            "a circuit needs both entries and exits"
        );

        let mut circulation = Vec::new();
        for (i, line) in layout.circulation.into_iter().enumerate() {
            let profile = surface(&line)?;
            ensure!(
                line.points.iter().all(|&[x, z]| TileKey::of(x, z) == owner),
                "circulation crosses its owning tile"
            );
            let key = format!("surveyed-{}:{i}", mapped.id);
            ensure!(
                !base.lanes.iter().any(|lane| lane.key == key),
                "duplicate route {key}"
            );
            let lane = LaneBuild {
                start_node: line.points[0],
                end_node: *line.points.last().unwrap(),
                points: line.points,
                segment: key.clone(),
                key,
                street: mapped.name.clone(),
                highway: mapped.highway.clone(),
                role: geom::LaneRole::Through,
                painted_through_mapped: false,
                width: source.lane_width(mapped),
                max_speed: geom::autonomous_speed(&mapped.highway, &mapped.maxspeed, None, ""),
                control: ControlKind::None,
                outgoing: None,
                legal: None,
                tile: Some(owner),
                level: 0,
            };
            circulation.push((lane, profile));
        }
        let routes = Self {
            tiles,
            owner,
            approaches,
            circulation,
            envelope,
        };
        routes.apply(&mut base, &routes.tiles.iter().copied().collect());
        let mut net = Network::new(source.timing.clone());
        net.add(base);
        let ring: Vec<_> = routes
            .circulation
            .iter()
            .map(|(l, _)| net.lane_by_key(&l.key).unwrap())
            .collect();
        let links = |a, b| {
            net.lanes[a as usize]
                .links
                .iter()
                .any(|link| link.to == b && link.drivable && link.legal)
        };
        ensure!(
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .all(|(&a, &b)| links(a, b)),
            "circuit has an undrivable link"
        );
        for (key, approach) in &routes.approaches {
            let id = net.lane_by_key(key).unwrap();
            ensure!(
                ring.iter().any(|&r| if approach.inbound {
                    links(id, r)
                } else {
                    links(r, id)
                }),
                "undrivable connection for {key}"
            );
        }
        Ok(routes)
    }

    pub(super) fn apply(&self, build: &mut NetworkBuild, wanted: &HashSet<TileKey>) {
        if !self.tiles.iter().any(|tile| wanted.contains(tile)) {
            return;
        }
        let mut changed = HashSet::new();
        for (index, lane) in build.lanes.iter_mut().enumerate() {
            let Some(approach) = self.approaches.get(&lane.key) else {
                continue;
            };
            let i = index as u32;
            lane.points.clone_from(&approach.points);
            if approach.inbound {
                lane.end_node = *lane.points.last().unwrap();
                build.vehicle_envelopes.push((i, self.envelope));
            } else {
                lane.start_node = lane.points[0];
            }
            if let Some(control) = approach.control {
                lane.control = control;
            }
            changed.insert(i);
        }
        build.lane_surfaces.retain(|(i, _)| !changed.contains(i));
        build
            .lane_carriageway_spans
            .retain(|(i, _)| !changed.contains(i));
        build.lane_junctions.retain(|(i, _)| !changed.contains(i));
        build
            .passing
            .retain(|(i, pass)| !changed.contains(i) && !changed.contains(&pass.to));
        for i in changed {
            build.lane_surfaces.push((
                i,
                self.approaches[&build.lanes[i as usize].key]
                    .surface
                    .clone(),
            ));
        }
        if wanted.contains(&self.owner) {
            for (lane, surface) in &self.circulation {
                let i = build.lanes.len() as u32;
                build.lanes.push(lane.clone());
                build.lane_surfaces.push((i, surface.clone()));
                build.vehicle_envelopes.push((i, self.envelope));
            }
        }
    }
}
