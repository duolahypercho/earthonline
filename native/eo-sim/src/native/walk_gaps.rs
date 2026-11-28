//! Register the street planner's missing sidewalk spans to the pedestrian graph.

use std::collections::HashMap;

use eo_data::sim_source::{SimRoad, TrafficSection};

use super::{DrawnSection, SECTION_MATCH_COS, SECTION_MATCH_M, SectionIndex, WALK_CLEARANCE_M, front_cell};
use crate::geom::{P2, cumulative, junction_lane_points};

pub(super) struct WalkGap {
    start: P2,
    tangent: P2,
    length: f64,
    /// Side signs in the section's direction, with +1 along (dz, -dx).
    side: i8,
}

impl WalkGap {
    pub(super) fn extend(gaps: &mut Vec<Self>, section: &TrafficSection) {
        let cum = cumulative(&section.points);
        for (edge, stations) in section.points.windows(2).zip(cum.windows(2)) {
            let [a, b] = [edge[0], edge[1]];
            let length = stations[1] - stations[0];
            if length <= 0.0 {
                continue;
            }
            let tangent = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
            for (side, intervals) in [-1, 1].into_iter().zip(&section.walk_gaps) {
                for &[lo, hi] in intervals {
                    let begin = lo.max(stations[0]);
                    let end = hi.min(stations[1]);
                    if lo.is_finite() && hi.is_finite() && end > begin {
                        let offset = begin - stations[0];
                        gaps.push(Self {
                            start: [a[0] + tangent[0] * offset, a[1] + tangent[1] * offset],
                            tangent,
                            length: end - begin,
                            side,
                        });
                    }
                }
            }
        }
    }

    pub(super) fn covers(&self, centre: P2, heading: P2, side: i8) -> bool {
        let [ux, uz] = self.tangent;
        let direction = ux * heading[0] + uz * heading[1];
        let section_side = if direction < 0.0 { -side } else { side };
        if section_side != self.side || direction.abs() < SECTION_MATCH_COS * heading[0].hypot(heading[1]) {
            return false;
        }
        let [x, z] = [centre[0] - self.start[0], centre[1] - self.start[1]];
        (0.0..=self.length).contains(&(x * ux + z * uz)) && (x * uz - z * ux).abs() <= SECTION_MATCH_M
    }
}

pub(super) struct WalkLaneClearance {
    strips: SectionIndex,
    roads: Vec<usize>,
}

impl WalkLaneClearance {
    pub(super) fn from_plan(roads: &[SimRoad], sections: &HashMap<String, Vec<TrafficSection>>) -> Option<Self> {
        let mut strips = Vec::new();
        let mut owners = Vec::new();
        for (ri, road) in roads.iter().enumerate() {
            if road.bridge || road.tunnel || road.layer != 0 {
                continue;
            }
            for section in sections.get(&road.id).into_iter().flatten() {
                if section.shared_bidirectional || section.points.len() < 2 {
                    continue;
                }
                for lane in &section.lanes {
                    let line = junction_lane_points(&section.points, lane.lateral, false);
                    for edge in line.windows(2) {
                        strips.push(DrawnSection { a: edge[0], b: edge[1], half: lane.width / 2.0, walks: [0.0; 2] });
                        owners.push(ri);
                    }
                }
            }
        }
        (!strips.is_empty()).then(|| Self { strips: SectionIndex::with_margin(strips, WALK_CLEARANCE_M), roads: owners })
    }

    /// Skip sampling sidewalks with no nearby lane capable of cutting them.
    pub(super) fn may_block(
        &self,
        points: &[P2],
        road: usize,
        at_grade: impl Fn(usize) -> bool,
        overlap_margin: &impl Fn(usize) -> Option<f64>,
    ) -> bool {
        points.windows(2).any(|edge| {
            let [a, b] = [edge[0], edge[1]];
            let direction = [b[0] - a[0], b[1] - a[1]];
            let min = [a[0].min(b[0]), a[1].min(b[1])];
            let max = [a[0].max(b[0]), a[1].max(b[1])];
            let lo = front_cell(min[0], min[1]);
            let hi = front_cell(max[0], max[1]);
            (lo.0..=hi.0).any(|x| (lo.1..=hi.1).any(|z| {
                self.strips.cells.get(&(x, z)).into_iter().flatten().any(|&i| {
                    let i = i as usize;
                    let strip = &self.strips.pieces[i];
                    let overlaps = |reach: f64| {
                        (0..2).all(|k| min[k] <= strip.a[k].max(strip.b[k]) + reach && max[k] >= strip.a[k].min(strip.b[k]) - reach)
                    };
                    overlaps(strip.half + WALK_CLEARANCE_M)
                        && at_grade(self.roads[i])
                        && self.clearance_margin(i, direction, road, overlap_margin)
                            .is_some_and(|margin| overlaps(strip.half + margin))
                })
            }))
        })
    }

    pub(super) fn blocks(
        &self,
        p: P2,
        direction: P2,
        road: usize,
        at_grade: impl Fn(usize) -> bool,
        overlap_margin: &impl Fn(usize) -> Option<f64>,
    ) -> bool {
        self.strips.cells.get(&front_cell(p[0], p[1])).is_some_and(|list| {
            list.iter().any(|&i| {
                let i = i as usize;
                let strip = &self.strips.pieces[i];
                strip.covers(p, WALK_CLEARANCE_M)
                    && at_grade(self.roads[i])
                    && self.clearance_margin(i, direction, road, overlap_margin)
                        .is_some_and(|margin| margin == WALK_CLEARANCE_M || strip.covers(p, margin))
            })
        })
    }

    fn clearance_margin(
        &self,
        i: usize,
        [dx, dz]: P2,
        road: usize,
        overlap_margin: &impl Fn(usize) -> Option<f64>,
    ) -> Option<f64> {
        let other = self.roads[i];
        let strip = &self.strips.pieces[i];
        let [ux, uz] = [strip.b[0] - strip.a[0], strip.b[1] - strip.a[1]];
        let parallel = (dx * ux + dz * uz).abs() >= SECTION_MATCH_COS * dx.hypot(dz) * ux.hypot(uz);
        // A folded way can bring its own lane across the sidewalk.
        // At a fold it can also run over the sidewalk in parallel.
        if other == road {
            return if parallel { overlap_margin(other) } else { Some(WALK_CLEARANCE_M) };
        }
        if parallel {
            return Some(WALK_CLEARANCE_M);
        }
        overlap_margin(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lanes_own_way_can_cut_a_crossing_sidewalk() {
        let lane = DrawnSection { a: [4., 10.], b: [14., 10.], half: 1., walks: [0.; 2] };
        let clearance = WalkLaneClearance {
            strips: SectionIndex::with_margin(vec![lane], WALK_CLEARANCE_M),
            roads: vec![0],
        };
        let grade = |_| true;
        let no_join = |_| None;
        let point = [9., 10.];
        let across = [[9., 5.], [9., 15.]];
        let along = [[5., 10.], [13., 10.]];
        assert!(clearance.may_block(&across, 0, grade, &no_join));
        assert!(clearance.blocks(point, [0., 1.], 0, grade, &no_join));
        assert!(!clearance.may_block(&along, 0, grade, &no_join));
        assert!(!clearance.blocks(point, [1., 0.], 0, grade, &no_join));
        let inside_closed_way = |_| Some(0.0);
        assert!(clearance.may_block(&along, 0, grade, &inside_closed_way));
        assert!(clearance.blocks(point, [1., 0.], 0, grade, &inside_closed_way));
        assert!(!clearance.blocks([9., 11.1], [1., 0.], 0, grade, &inside_closed_way));
        let inside_folded_way = |_| Some(-crate::consts::PERSON_CAPSULE_R_M);
        assert!(clearance.blocks(point, [1., 0.], 0, grade, &inside_folded_way));
        assert!(!clearance.blocks([9., 10.9], [1., 0.], 0, grade, &inside_folded_way));
        assert!(!clearance.blocks(point, [0., 1.], 0, |_| false, &no_join));
    }
}
