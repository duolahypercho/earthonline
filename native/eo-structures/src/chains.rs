//! From the street plan's separated segments to chains: the segments read
//! and joined to their roads, split at landmark deck ends, their end points
//! snapped into nodes, and the pieces walked into chains through nodes where
//! exactly two of the same level, class and kind meet.

use std::collections::HashMap;

use eo_data::standards::is_vehicle_way;
use eo_street::StreetWorld;
use eo_street::plan::{P2, arc_table, dedupe_points};

use crate::landmark::LandmarkSpec;
use crate::{ChainKind, LandmarkDeck, RoadClass, RoadTags};

/// One separated segment (or the part of one a landmark deck end splits).
#[derive(Clone, Debug)]
pub(crate) struct RawPiece {
    pub segment: usize,
    pub key: String,
    pub way: String,
    pub name: String,
    pub highway: String,
    pub points: Vec<P2>,
    pub half: f64,
    pub level: i8,
    pub kind: ChainKind,
    pub class: RoadClass,
    /// The landmark spec (index) whose deck carries it.
    pub landmark: Option<usize>,
}

impl RawPiece {
    fn same_road(&self, o: &RawPiece) -> bool {
        self.level == o.level && self.kind == o.kind && self.class == o.class && self.landmark == o.landmark
            && eo_data::movable_spans::is_leaf(&self.way) == eo_data::movable_spans::is_leaf(&o.way)
    }
}

/// What reading the segments found.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Gathered {
    /// Separated vehicle segments read.
    pub segments: usize,
    /// Of those, with no road of their way id in the city.
    pub unjoined: usize,
    /// Of those, with no usable points or width.
    pub degenerate: usize,
    /// Of those, carried by a landmark deck (wholly or in part).
    pub landmark: usize,
}

/// The level a way is solved at: its OSM layer; a bridge on layer 0 (or
/// none) is 1, a tunnel on layer 0 or above is -1.
pub(crate) fn level_of(tags: &RoadTags) -> i8 {
    let l = tags.layer.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    if tags.tunnel {
        l.min(-1)
    } else if l != 0 {
        l
    } else if tags.bridge {
        1
    } else {
        // `separated` in the street data but at grade in the roads: a deck
        // on layer 1 (the street contract's own grade-separation rule).
        1
    }
}

/// Split a polyline where its chainage crosses `c0` and `c1`: the parts and
/// whether each lies inside [c0, c1].
fn split_by_chainage(points: &[P2], chainage: &dyn Fn(f64, f64) -> f64, c0: f64, c1: f64) -> Vec<(Vec<P2>, bool)> {
    let cs: Vec<f64> = points.iter().map(|p| chainage(p[0], p[1])).collect();
    let mut out: Vec<(Vec<P2>, bool)> = vec![];
    let mut cur: Vec<P2> = vec![points[0]];
    for i in 0..points.len() - 1 {
        let (a, b) = (points[i], points[i + 1]);
        let (ca, cb) = (cs[i], cs[i + 1]);
        let mut cuts: Vec<f64> = vec![];
        for c in [c0, c1] {
            if (ca - c) * (cb - c) < 0.0 {
                cuts.push((c - ca) / (cb - ca));
            }
        }
        cuts.sort_by(|x, y| x.total_cmp(y));
        for t in cuts {
            let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            cur.push(q);
            let mid = mid_chainage(&cur, chainage);
            out.push((std::mem::replace(&mut cur, vec![q]), mid >= c0 && mid <= c1));
        }
        cur.push(b);
    }
    let mid = mid_chainage(&cur, chainage);
    out.push((cur, mid >= c0 && mid <= c1));
    out.retain(|(p, _)| p.len() >= 2 && arc_table(p).last().copied().unwrap_or(0.0) > 1e-3);
    out
}

fn mid_chainage(points: &[P2], chainage: &dyn Fn(f64, f64) -> f64) -> f64 {
    let cum = arc_table(points);
    let half = cum.last().copied().unwrap_or(0.0) / 2.0;
    let f = eo_street::plan::frame_at(points, &cum, half, false);
    chainage(f.x, f.z)
}

/// Every separated vehicle segment of the street plan as pieces, in the
/// segments' stable-key order (whatever order the city lists them in).
pub(crate) fn gather(streets: &StreetWorld, roads: &HashMap<String, RoadTags>, landmarks: &[LandmarkSpec]) -> (Vec<RawPiece>, Gathered) {
    let o = &streets.o;
    let mut order: Vec<usize> = (0..streets.city.segments.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (&streets.city.segments[a], &streets.city.segments[b]);
        sa.seed().cmp(sb.seed()).then(sa.id.cmp(&sb.id))
    });
    let mut out = vec![];
    let mut g = Gathered::default();
    for i in order {
        let seg = &streets.city.segments[i];
        if !seg.separated || o.exclude_highways.contains(&seg.highway) || !is_vehicle_way(&seg.highway) {
            continue;
        }
        g.segments += 1;
        let Some(sid) = seg.street_id.as_deref() else {
            g.unjoined += 1;
            continue;
        };
        let way = crate::way_key(sid).to_string();
        let Some(tags) = roads.get(&way) else {
            g.unjoined += 1;
            continue;
        };
        let points = dedupe_points(&seg.points);
        let half = if seg.width.is_finite() && seg.width > 0.2 {
            seg.width / 2.0
        } else {
            seg.lane_layout.as_ref().map(|l| l.iter().map(|x| x.width).sum::<f64>() / 2.0).unwrap_or(0.0)
        };
        if points.len() < 2 || half <= 0.1 || arc_table(&points).last().copied().unwrap_or(0.0) < 1e-3 {
            g.degenerate += 1;
            continue;
        }
        let level = level_of(tags);
        let kind = if tags.tunnel {
            ChainKind::Tunnel
        } else if level < 0 {
            ChainKind::Below
        } else {
            ChainKind::Deck
        };
        let base = RawPiece {
            segment: i,
            key: seg.seed().to_string(),
            way: way.clone(),
            name: if seg.street_name.is_empty() { tags.name.clone() } else { seg.street_name.clone() },
            highway: seg.highway.clone(),
            points: points.clone(),
            half,
            level,
            kind,
            class: RoadClass::of(&seg.highway),
            landmark: None,
        };
        let carrier = landmarks.iter().position(|l| l.ways.contains(&way)).or_else(|| landmarks.iter().position(|l| l.in_corridor(&points)));
        match carrier {
            None => out.push(base),
            Some(li) => {
                g.landmark += 1;
                let l = &landmarks[li];
                let parts = split_by_chainage(&points, &*l.chainage, l.c0, l.c1);
                let many = parts.len() > 1;
                for (k, (pts, inside)) in parts.into_iter().enumerate() {
                    let mut p = base.clone();
                    if many {
                        p.key = format!("{}#{k}", base.key);
                    }
                    p.points = pts;
                    if inside {
                        p.kind = ChainKind::Landmark;
                        p.landmark = Some(li);
                    }
                    out.push(p);
                }
            }
        }
    }
    (out, g)
}

/// The end points of the pieces snapped into nodes (`snap` metres): each
/// end goes to the nearest node already made within the snap, else makes one.
pub(crate) struct Nodes {
    pub at: Vec<P2>,
    /// (piece, end 0 = start / 1 = end) at each node, in piece order.
    pub ends: Vec<Vec<(u32, u8)>>,
    /// Each piece's [start node, end node].
    pub of_piece: Vec<[u32; 2]>,
}

pub(crate) fn cluster(pieces: &[RawPiece], snap: f64) -> Nodes {
    let snap = snap.max(1e-6);
    let key = |p: P2| ((p[0] / snap).floor() as i64, (p[1] / snap).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    let mut nodes = Nodes { at: vec![], ends: vec![], of_piece: vec![] };
    for (pi, p) in pieces.iter().enumerate() {
        let mut pair = [0u32; 2];
        for (e, q) in [p.points[0], *p.points.last().unwrap()].into_iter().enumerate() {
            let k = key(q);
            let mut best: Option<(f64, u32)> = None;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    for &n in grid.get(&(k.0 + dx, k.1 + dz)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let a = nodes.at[n as usize];
                        let d = (a[0] - q[0]).hypot(a[1] - q[1]);
                        if d <= snap && best.is_none_or(|b| d < b.0) {
                            best = Some((d, n));
                        }
                    }
                }
            }
            let n = match best {
                Some((_, n)) => n,
                None => {
                    let n = nodes.at.len() as u32;
                    nodes.at.push(q);
                    nodes.ends.push(vec![]);
                    grid.entry(k).or_default().push(n);
                    n
                }
            };
            nodes.ends[n as usize].push((pi as u32, e as u8));
            pair[e] = n;
        }
        nodes.of_piece.push(pair);
    }
    nodes
}

/// A chain as the walk leaves it: its pieces in order, each with its
/// direction (true: against its own point order).
pub(crate) struct Walked {
    pub pieces: Vec<(u32, bool)>,
    pub nodes: [u32; 2],
}

/// Walk the pieces into chains. A chain continues through a node when
/// exactly two piece ends meet there, the two are the same road (level,
/// kind, class, landmark), and the node is not `stop` (a paved street).
/// Seeds are taken in piece order and a chain keeps its seed's direction,
/// so the result does not depend on the input order.
pub(crate) fn walk(pieces: &[RawPiece], nodes: &Nodes, stop: &[bool]) -> Vec<Walked> {
    let mut seen = vec![false; pieces.len()];
    let mut out = vec![];
    // the other piece end at node n, if the chain from `from` continues there.
    let next = |n: u32, from: u32| -> Option<(u32, u8)> {
        let ends = &nodes.ends[n as usize];
        if stop[n as usize] || ends.len() != 2 {
            return None;
        }
        let (a, b) = (ends[0], ends[1]);
        let other = if a.0 == from && b.0 != from {
            b
        } else if b.0 == from && a.0 != from {
            a
        } else {
            return None;
        };
        pieces[from as usize].same_road(&pieces[other.0 as usize]).then_some(other)
    };
    for seed in 0..pieces.len() as u32 {
        if seen[seed as usize] {
            continue;
        }
        seen[seed as usize] = true;
        let mut list: std::collections::VecDeque<(u32, bool)> = std::collections::VecDeque::from([(seed, false)]);
        // backward from the seed's start.
        let mut n = nodes.of_piece[seed as usize][0];
        let mut cur = seed;
        let mut start = n;
        while let Some((q, e)) = next(n, cur) {
            if seen[q as usize] {
                break;
            }
            seen[q as usize] = true;
            // q ends at n with its end `e`: if that is its end (1), it runs forward into n.
            let reversed = e == 0;
            list.push_front((q, reversed));
            n = nodes.of_piece[q as usize][if reversed { 1 } else { 0 }];
            cur = q;
            start = n;
        }
        // forward from the seed's end.
        let mut n = nodes.of_piece[seed as usize][1];
        let mut cur = seed;
        let mut end = n;
        while let Some((q, e)) = next(n, cur) {
            if seen[q as usize] {
                break;
            }
            seen[q as usize] = true;
            let reversed = e == 1;
            list.push_back((q, reversed));
            n = nodes.of_piece[q as usize][if reversed { 0 } else { 1 }];
            cur = q;
            end = n;
        }
        out.push(Walked { pieces: list.into_iter().collect(), nodes: [start, end] });
    }
    out
}

/// The centreline of a walked chain: its pieces' points end to end (a
/// repeated joint point once), and each piece's stretch of it.
pub(crate) fn centreline(pieces: &[RawPiece], w: &Walked) -> (Vec<P2>, Vec<(f64, f64)>) {
    let mut pts: Vec<P2> = vec![];
    let mut spans = vec![];
    let mut s = 0.0;
    for &(pi, rev) in &w.pieces {
        let mut p = pieces[pi as usize].points.clone();
        if rev {
            p.reverse();
        }
        let len = arc_table(&p).last().copied().unwrap_or(0.0);
        for (k, q) in p.into_iter().enumerate() {
            if k == 0 {
                if let Some(l) = pts.last() {
                    let gap = (l[0] - q[0]).hypot(l[1] - q[1]);
                    if gap < 1e-6 {
                        continue;
                    }
                    s += gap;
                }
            }
            pts.push(q);
        }
        spans.push((s, s + len));
        s += len;
    }
    (pts, spans)
}

/// Which landmark deck a landmark spec index is.
pub(crate) fn deck_of(landmarks: &[LandmarkSpec], li: Option<usize>) -> Option<LandmarkDeck> {
    li.map(|i| landmarks[i].deck)
}
