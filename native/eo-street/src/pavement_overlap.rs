//! Intersecting road polygons can cover the same strip even where neither
//! centreline yields. Keep the higher-priority street's pavement in that strip.
use crate::{Layer, StreetWorld, TileMesh, plan::Frame, road_openings::RoadOpenings};
use std::collections::{HashMap, HashSet};

type Point = [f64; 2];
type SpanId = (usize, usize);

#[derive(Default)]
pub(crate) struct PavementOverlap {
    cuts: HashMap<usize, RoadOpenings>,
    clipped: HashSet<SpanId>,
    detailed: HashSet<SpanId>,
}

#[derive(Clone, Copy)]
struct Span {
    id: SpanId,
    corners: [Point; 4],
    limits: [Point; 2],
}

impl Span {
    fn new(id: SpanId, frames: [Frame; 2], clip_a: (f64, f64), clip_b: (f64, f64)) -> Option<Self> {
        if clip_a.1 - clip_a.0 <= 1e-4 && clip_b.1 - clip_b.0 <= 1e-4 {
            return None;
        }
        let [a, b] = frames;
        let corners = [a.at(clip_a.0), b.at(clip_b.0), b.at(clip_b.1), a.at(clip_a.1)];
        let limits = [
            std::array::from_fn(|axis| {
                corners
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::INFINITY, f64::min)
            }),
            std::array::from_fn(|axis| {
                corners
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::NEG_INFINITY, f64::max)
            }),
        ];
        Some(Self {
            id,
            corners,
            limits,
        })
    }

    fn cells(&self) -> impl Iterator<Item = (i32, i32)> {
        let [lo, hi] = self.limits.map(|p| p.map(|v| (v / 32.0).floor() as i32));
        (lo[0]..=hi[0]).flat_map(move |x| (lo[1]..=hi[1]).map(move |z| (x, z)))
    }

    fn overlaps(&self, other: &Span) -> bool {
        if (0..2).any(|i| {
            self.limits[0][i] >= other.limits[1][i] || other.limits[0][i] >= self.limits[1][i]
        }) {
            return false;
        }
        // Exclude footprints touching at a shared edge. Both rings run
        // counterclockwise; test each supporting line of each convex span.
        let separated = |a: &[Point; 4], b: &[Point; 4]| {
            (0..4).any(|i| {
                let [x, z] = a[i];
                let [xx, zz] = a[(i + 1) % 4];
                let (dx, dz) = (xx - x, zz - z);
                b.iter().all(|p| dx * (p[1] - z) - dz * (p[0] - x) <= 1e-7)
            })
        };
        !(separated(&self.corners, &other.corners) || separated(&other.corners, &self.corners))
    }
}

impl PavementOverlap {
    pub(crate) fn build(w: &StreetWorld) -> Self {
        let mut out = Self::default();
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        let mut spans: Vec<Span> = Vec::new();
        let mut registered = HashSet::new();
        let mut candidates = Vec::new();
        for ei in crate::plan::carriageway_order(&w.plan.entries, &w.city) {
            let e = &w.plan.entries[ei];
            if w.city.segments[e.road].separated {
                continue;
            }
            let g = &w.geoms[ei];
            let divided = w.divided_name(ei);
            for (i, &keep) in g.keep.iter().enumerate() {
                if !keep {
                    continue;
                }
                let Some(span) = Span::new(
                    (ei, i),
                    [g.stations[i], g.stations[i + 1]],
                    g.clip(i, e.half),
                    g.clip(i + 1, e.half),
                ) else {
                    continue;
                };
                candidates.clear();
                for key in span.cells() {
                    if let Some(ids) = cells.get(&key) {
                        candidates.extend_from_slice(ids);
                    }
                }
                candidates.sort_unstable();
                candidates.dedup();
                for &j in &candidates {
                    let owner = &spans[j];
                    if owner.id.0 == ei || !span.overlaps(owner) {
                        continue;
                    }
                    out.clipped.insert(span.id);
                    out.detailed.insert(span.id);
                    out.detailed.insert(owner.id);
                    let same_carriageway =
                        divided.is_some() && divided == w.divided_name(owner.id.0);
                    // Two mapped halves of the same boulevard can overlap
                    // at the median. Cars already ride the higher asphalt.
                    // Give it the buried strip, with a single owner where
                    // the two surfaces meet within paint clearance.
                    if same_carriageway {
                        out.clipped.insert(owner.id);
                        if registered.insert((owner.id.0, spans.len())) {
                            out.cut_by(
                                w,
                                owner.id.0,
                                span.id,
                                w.o.curb_face_height,
                                -w.o.junction_paint_lift,
                            );
                        }
                    }
                    if registered.insert((ei, j)) {
                        let below = if same_carriageway {
                            w.o.curb_face_height
                        } else {
                            w.o.marking_lift
                        };
                        out.cut_by(w, ei, owner.id, below, w.o.junction_paint_lift);
                    }
                }
                let j = spans.len();
                for k in span.cells() {
                    cells.entry(k).or_default().push(j);
                }
                spans.push(span);
            }
        }
        out
    }

    fn cut_by(
        &mut self,
        w: &StreetWorld,
        target: usize,
        (entry, j): SpanId,
        below: f64,
        above: f64,
    ) {
        let half = w.plan.entries[entry].half;
        let g = &w.geoms[entry];
        let vertex = |i: usize, u: f64| {
            let [x, z] = g.stations[i].at(u);
            [x, g.pavement_y(&w.o, i, u, half), z].map(|v| v as f32)
        };
        let ((cla, cha), (clb, chb)) = (g.clip(j, half), g.clip(j + 1, half));
        let cuts = self.cuts.entry(target).or_default();
        for band in g.section_offsets(&w.o, half).windows(2) {
            let (a0, a1) = (band[0].clamp(cla, cha), band[1].clamp(cla, cha));
            let (b0, b1) = (band[0].clamp(clb, chb), band[1].clamp(clb, chb));
            if a1 - a0 <= 1e-4 && b1 - b0 <= 1e-4 {
                continue;
            }
            let a = vertex(j, a0);
            let b = vertex(j + 1, b0);
            let c = vertex(j + 1, b1);
            let d = vertex(j, a1);
            cuts.add_surface([a, b, c], below, above);
            cuts.add_surface([a, c, d], below, above);
        }
    }

    pub(crate) fn needs_detail(&self, ei: usize, i: usize) -> bool {
        self.detailed.contains(&(ei, i))
    }

    pub(crate) fn finish_span(&self, m: &mut TileMesh, ei: usize, i: usize, first_face: usize) {
        if self.clipped.contains(&(ei, i)) {
            m.clip_faces(first_face, &self.cuts[&ei], |l| {
                matches!(l, Layer::Carriageway | Layer::Marking)
            });
        }
    }
}
