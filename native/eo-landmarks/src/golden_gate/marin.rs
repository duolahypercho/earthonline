//! Marin approach. The District's Phase 1 retrofit description gives four
//! four-legged towers on 50 by 75 ft footprints and five connected deck trusses.
//! The 2008 Historic Resources Evaluation Report, §3 and the chief engineer's
//! plan and elevation (Fig. 2), gives a 1,100 ft viaduct and 175 ft truss spans.
//! https://www.goldengate.org/district/district-projects/seismic-retrofit/phase-1/
//! https://www.goldengate.org/assets/1/6/suicide-deterrent-historic-resources-evaluation-report.pdf
use super::{Collider, Ctx, Plan, Site};
use crate::data::GoldenGate;
use crate::data::{num, pair};
use crate::geometry::Rgb;
use crate::geometry::{ALL, MeshArrays, V3};
use crate::suspension::DeckStandards;
use std::sync::OnceLock;

/// Candidate detail is opt-in until the visual review. Gameplay and captures
/// read the same choice, and an explicit kill switch takes precedence.
pub fn approach_finish() -> bool {
    finish_switches().0
}

/// Draw the camera-independent wet deck and the registered ground imagery.
pub fn surveyed_surfaces() -> bool {
    finish_switches().1
}

/// Scanned aggregate for the epoxy-asphalt course and wear at the two Marin joints.
pub fn deck_pavement() -> bool {
    finish_switches().2
}

/// Lane-aligned aggregate polish on the wearing course.
pub fn wheel_wear() -> bool {
    finish_switches().3
}

/// The District locates the viaduct expansion joints at Pylon N2 and the north abutment.
pub fn expansion_joints(g: &GoldenGate, site: &Site) -> [f64; 2] {
    let [x, z] = g.supports["northAnchor"].centre;
    let n2 = site.chainage(x, z) + super::pylons::north_pylon_spacing();
    let [x, z] = g.way_ends["north"];
    [n2, site.chainage(x, z)]
}

fn finish_switches() -> (bool, bool, bool, bool) {
    static ON: OnceLock<(bool, bool, bool, bool)> = OnceLock::new();
    *ON.get_or_init(|| {
        let mut finish = std::env::var("EO_GOLDEN_GATE_FINISH").is_ok_and(|v| v == "on");
        let mut surfaces = std::env::var("EO_GOLDEN_GATE_SURFACES").is_ok_and(|v| v == "on");
        let (mut disable_finish, mut disable_surfaces) = (false, false);
        let (mut pavement, mut disable_pavement) = (false, false);
        let (mut wheels, mut disable_wheels) = (false, false);
        for arg in std::env::args() {
            match arg.as_str() {
                "--golden-gate-finish" => finish = true,
                "--golden-gate-wheel-wear" => wheels = true,
                "--no-golden-gate-wheel-wear" => disable_wheels = true,
                "--golden-gate-pavement" => pavement = true,
                "--no-golden-gate-pavement" => disable_pavement = true,
                "--golden-gate-surfaces" => surfaces = true,
                "--no-golden-gate-finish" => disable_finish = true,
                "--no-golden-gate-surfaces" => disable_surfaces = true,
                _ => {}
            }
        }
        let surfaces = surfaces && !disable_surfaces && !disable_finish;
        let wheels = wheels && !disable_wheels && !disable_pavement && !disable_finish;
        let pavement = (pavement || wheels) && !disable_pavement && !disable_finish;
        ((finish || surfaces || pavement) && !disable_finish, surfaces, pavement, wheels)
    })
}

const FT: f64 = 0.3048;
const SPAN: f64 = 175.0 * FT;
const TOWER_LENGTH: f64 = 50.0 * FT;
const TOWER_WIDTH: f64 = 75.0 * FT;
const VIADUCT: f64 = 1100.0 * FT;

// Chainage is a projection onto the straight suspension span. Measure backwards
// on the deck instead; otherwise the spans grow on the bend above Vista Point.
fn step_back(site: &Site, end: f64, length: f64) -> f64 {
    let [x, z] = site.deck_point(end, 0.0);
    let mut lo = end - length;
    let mut hi = end;
    for _ in 0..30 {
        let mid = (lo + hi) * 0.5;
        let [u, v] = site.deck_point(mid, 0.0);
        if (x - u).hypot(z - v) > length {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// Close the short skew between the nominal deck end (the average of the
/// carriageway tips) and the back of the abutment. The landward station is
/// the outermost surveyed road vertex. Six lanes share a crowned hillside
/// landing, sampled across the road; the visual mesh and tyres share triangles.
pub(crate) fn abutment(
    g: &GoldenGate,
    site: &Site,
    p: &Plan<'_>,
    ground: &dyn Fn(f64, f64) -> f64,
) -> Vec<[V3; 4]> {
    let mut end = p.c_north;
    let mut direction = [0.0; 2];
    for way in &g.carriageways {
        let mut points: Vec<_> = way.chunks_exact(2).map(|v| [v[0], v[1]]).collect();
        if points.len() < 2 {
            continue;
        }
        if site.chainage(points[0][0], points[0][1]) > site.chainage(points[1][0], points[1][1]) {
            points.reverse();
        }
        let a = points[points.len() - 2];
        let b = points[points.len() - 1];
        end = end.max(site.chainage(b[0], b[1]));
        let length = (b[0] - a[0]).hypot(b[1] - a[1]);
        if length > 0.0 {
            for k in 0..2 {
                direction[k] += (b[k] - a[k]) / length;
            }
        }
    }
    let advance = direction[0] * site.axis[0] + direction[1] * site.axis[1];
    if advance <= 0.0 || end <= p.c_north {
        return Vec::new();
    }
    let run = (end - p.c_north) / advance;
    let centre = site.deck_point(p.c_north, 0.);
    let land = [
        centre[0] + direction[0] * run,
        centre[1] + direction[1] * run,
    ];
    let length = direction[0].hypot(direction[1]);
    let across = [-direction[1] / length, direction[0] / length];
    let road = num(&g.dimensions, "roadwayM");
    let lanes = num(&g.dimensions, "lanesPerDirection") as usize * 2;
    let row = |i: usize| {
        let offset = road * (i as f64 / lanes as f64 - 0.5);
        let [x, z] = site.deck_point(p.c_north, offset);
        // Square the landing to the surveyed approach heading. Keeping it on the
        // suspension tower axis crosses the steep cut beneath the outer lane.
        let (u, v) = (land[0] + across[0] * offset, land[1] + across[1] * offset);
        ([x, p.y_north, z], [u, ground(u, v), v])
    };
    (0..lanes)
        .map(|i| {
            let (a, d) = row(i);
            let (b, c) = row(i + 1);
            [a, b, c, d]
        })
        .collect()
}

/// Sample the same two triangles that carry the tyres. At the back of the
/// abutment the seven lane boundaries meet different surveyed hillside heights.
fn apron_point(panels: &[[V3; 4]], width: f64, offset: f64, along: f64, rise: f64) -> V3 {
    let lane = ((offset / width + 0.5) * panels.len() as f64).clamp(0., panels.len() as f64);
    let index = (lane.floor() as usize).min(panels.len() - 1);
    let across = lane - index as f64;
    let [a, b, c, d] = panels[index];
    let mut p = std::array::from_fn(|i| {
        if along < across {
            (1. - across) * a[i] + (across - along) * b[i] + along * c[i]
        } else {
            (1. - along) * a[i] + across * c[i] + (along - across) * d[i]
        }
    });
    p[1] += rise;
    p
}

pub(crate) fn apron_median(
    panels: &[[V3; 4]],
    width: f64,
    ca: &DeckStandards,
    mesh: &mut MeshArrays,
    colour: Rgb,
) {
    if panels.is_empty() {
        return;
    }
    crate::bridge_roads::median_profile(
        mesh,
        [false, true],
        ca,
        colour,
        |relative_height, body| {
            let h = relative_height * ca.k_height;
            [(-0.5, 0.), (0.5, 0.), (0.5, 1.), (-0.5, 1.)]
                .map(|(side, t)| apron_point(panels, width, body * side, t, h))
        },
    );
}

struct Tower {
    feet: [V3; 4],
    heads: [V3; 4],
    yaw: f64,
}

impl Ctx<'_> {
    fn marin_towers(&self) -> [Tower; 4] {
        let scale = eo_data::groundscale::scale();
        // The terminal seats take the balance of the surveyed viaduct length.
        let seat = (VIADUCT - 5.0 * SPAN - 4.0 * TOWER_LENGTH) * 0.5;
        let mut c = step_back(self.site, self.plan.c_north, seat * scale);
        let depth = num(self.d, "trussDepthM");
        std::array::from_fn(|_| {
            let north = step_back(self.site, c, SPAN * scale);
            let south = step_back(self.site, north, TOWER_LENGTH * scale);
            c = south;
            let a = self.site.deck_point(south, 0.0);
            let b = self.site.deck_point(north, 0.0);
            let yaw = (b[1] - a[1]).atan2(b[0] - a[0]);
            let feet = [(a, -1.0), (a, 1.0), (b, 1.0), (b, -1.0)].map(|(p, sign)| {
                let s = sign * TOWER_WIDTH * scale / 2.0;
                let (x, z) = (p[0] - yaw.sin() * s, p[1] + yaw.cos() * s);
                [x, (self.ground)(x, z), z]
            });
            let heads = [(south, -1.0), (south, 1.0), (north, 1.0), (north, -1.0)]
                .map(|(c, s)| self.P(c, s * self.site.half_plane, self.plan.deck_top(c) - depth));
            Tower { feet, heads, yaw }
        })
    }

    pub(super) fn marin_viaduct(
        &self,
        steel: &mut MeshArrays,
        concrete: &mut MeshArrays,
        far: &mut MeshArrays,
    ) {
        let [w, d] = pair(self.d, "bentColumnM");
        let foot_h = w.min(d);
        for t in self.marin_towers() {
            let mut bases = t.feet;
            for (i, base) in bases.iter_mut().enumerate() {
                let [x, y, z] = *base;
                base[1] += foot_h;
                if base[1] >= t.heads[i][1] {
                    continue;
                }
                concrete.box_(x, z, y - foot_h, base[1], w, d, t.yaw, self.k.concrete, ALL);
                steel.beam(*base, t.heads[i], w, d, self.k.orange);
                far.beam(*base, t.heads[i], w, d, self.k.orange);
            }
            // Infer the bracing bays from the existing member proportions:
            // short on the hillside, more storeys on the downhill frames.
            for i in 0..4 {
                let j = (i + 1) % 4;
                let height = (t.heads[i][1] - bases[i][1]).min(t.heads[j][1] - bases[j][1]);
                if height <= 0.0 {
                    continue;
                }
                let bays = (height / num(self.d, "trussDepthM"))
                    .round()
                    .clamp(1.0, 6.0) as usize;
                let at = |leg: usize, level: usize| -> V3 {
                    std::array::from_fn(|k| {
                        bases[leg][k]
                            + (t.heads[leg][k] - bases[leg][k]) * level as f64 / bays as f64
                    })
                };
                for level in 0..bays {
                    let (a, b, c, d1) = (
                        at(i, level),
                        at(j, level),
                        at(i, level + 1),
                        at(j, level + 1),
                    );
                    super::suspension::bridge_member(steel, far, a, d1, w * 0.32, d * 0.32, self.k.orange);
                    super::suspension::bridge_member(steel, far, b, c, w * 0.32, d * 0.32, self.k.orange);
                    super::suspension::bridge_member(steel, far, c, d1, w * 0.50, d * 0.50, self.k.orange_dark);
                }
            }
        }
    }

    pub(super) fn marin_abutment(
        &self,
        deck: &mut MeshArrays,
        concrete: &mut MeshArrays,
        far: &mut MeshArrays,
        contact: &dyn Fn(f64, f64) -> f64,
        ca: &DeckStandards,
    ) {
        let panels = abutment(self.g, self.site, self.plan, contact);
        if panels.is_empty() {
            return;
        }
        let width = num(self.d, "roadwayM");
        apron_median(&panels, width, ca, deck, self.k.barrier);
        let mut paint = |s: f64, t0: f64, t1: f64, colour| {
            let vertices = [(-0.5, t0), (0.5, t0), (0.5, t1), (-0.5, t1)].map(|(side, t)| {
                apron_point(
                    &panels,
                    width,
                    s + side * ca.line_w,
                    t,
                    super::suspension::PAINT_LIFT_M,
                )
            });
            let [a, b, c, d] = vertices;
            deck.marking_quad(a, b, c, d, colour);
        };
        let curb = width * 0.5;
        let median = ca.median_edge();
        let from = self.plan.c_north - self.plan.c_south - ca.broken_gap * 0.5;
        let a = apron_point(&panels, width, 0., 0., 0.);
        let b = apron_point(&panels, width, 0., 1., 0.);
        let run = (b[0] - a[0]).hypot(b[2] - a[2]);
        let cycle = ca.broken_line + ca.broken_gap;
        for side in [-1., 1.] {
            paint(
                side * (curb - super::suspension::EDGE_LINE_INSET_M),
                0.,
                1.,
                self.k.white,
            );
            paint(
                side * (median + super::suspension::MEDIAN_LINE_OFFSET_M),
                0.,
                1.,
                self.k.yellow,
            );
            let lanes = panels.len() / 2;
            for lane in 1..lanes {
                let s = side * (median + (curb - median) * lane as f64 / lanes as f64);
                let mut dash = -from.rem_euclid(cycle);
                while dash < run {
                    if dash + ca.broken_line > 0. {
                        paint(
                            s,
                            dash.max(0.) / run,
                            ((dash + ca.broken_line) / run).min(1.),
                            self.k.white,
                        );
                    }
                    dash += cycle;
                }
            }
        }
        for &[a, b, c, d] in &panels {
            deck.quad(a, b, c, d, self.k.asphalt);
            far.quad(a, b, c, d, self.k.asphalt);
            let foot = |p: V3| [p[0], (self.ground)(p[0], p[2]).min(p[1]), p[2]];
            for mesh in [&mut *concrete, &mut *far] {
                mesh.quad(b, a, foot(a), foot(b), self.k.concrete);
            }
        }
        if let (Some(first), Some(last)) = (panels.first(), panels.last()) {
            for (a, b) in [(first[0], first[3]), (last[2], last[1])] {
                let low = |p: V3| [p[0], (self.ground)(p[0], p[2]).min(p[1]), p[2]];
                for mesh in [&mut *concrete, &mut *far] {
                    mesh.quad(a, b, low(b), low(a), self.k.concrete);
                }
            }
        }
    }

    pub(super) fn marin_footings(&self, out: &mut Vec<Collider>) {
        let [w, d] = pair(self.d, "bentColumnM");
        for (index, t) in self.marin_towers().iter().enumerate() {
            for (leg, &[x, y, z]) in t.feet.iter().enumerate() {
                if y + w.min(d) >= t.heads[leg][1] {
                    continue;
                }
                out.push(Collider {
                    part: format!("marin-tower-{}-foot-{}", index + 1, leg + 1),
                    x,
                    z,
                    y0: y - w.min(d),
                    y1: y + w.min(d),
                    hx: w,
                    hz: d,
                    yaw: t.yaw,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LandmarksJs, golden_gate};
    #[test]
    fn bending_the_approaches_keeps_the_surveyed_chords_under_their_lanes() {
        let g = golden_gate::data(LandmarksJs::get());
        let fitted = Site::with_curved_approaches(&g, true);
        let survey = Site::with_curved_approaches(&g, false);
        let start = survey.profiles.iter().map(|p| p[0][0]).fold(f64::NEG_INFINITY, f64::max);
        let end = survey.profiles.iter().map(|p| p.last().unwrap()[0]).fold(f64::INFINITY, f64::min);
        let lane = num(&g.dimensions, "roadwayM") / (2. * num(&g.dimensions, "lanesPerDirection"));
        for metre in start.ceil() as i32..=end.floor() as i32 {
            let c = f64::from(metre);
            let shift = (fitted.centre_offset(c) - survey.centre_offset(c)).abs();
            assert!(shift < lane / 4., "centre shifted by {shift} at station {c}");
        }
    }

    #[test]
    fn the_median_keeps_its_heading_at_the_staggered_north_carriageway_tips() {
        let g = golden_gate::data(LandmarksJs::get());
        let site = Site::with_curved_approaches(&g, true);
        let tip = site.profiles.iter().map(|p| p.last().unwrap()[0]).fold(f64::INFINITY, f64::min);
        let delta = 0.10 * eo_data::groundscale::scale();
        let heading = |a, b| f64::atan((site.centre_offset(b)-site.centre_offset(a))/(b-a));
        let turn = (heading(tip-delta, tip)-heading(tip, tip+delta)).abs().to_degrees();
        assert!(turn < 0.1, "heading changed by {turn} degrees at one source vertex");
    }

    #[test]
    fn apron_meets_every_lane_and_ends_at_the_last_mapped_station() {
        let js = LandmarksJs::get();
        let g = golden_gate::data(js);
        let site = Site::new(&g);
        let ground = |x: f64, z: f64| x * 0.02 - z * 0.03;
        let p = golden_gate::plan(js, &site, &ground);
        let patches = abutment(&g, &site, &p, &ground);
        assert_eq!(patches.len(), 6);
        let end = g
            .carriageways
            .iter()
            .flat_map(|w| w.chunks_exact(2))
            .map(|pt| site.chainage(pt[0], pt[1]))
            .fold(f64::NEG_INFINITY, f64::max);
        for pair in patches.windows(2) {
            assert_eq!(pair[0][1], pair[1][0]);
            assert_eq!(pair[0][2], pair[1][3]);
        }
        let left = patches.first().unwrap()[3];
        let right = patches.last().unwrap()[2];
        assert!(
            (site.chainage((left[0] + right[0]) / 2., (left[2] + right[2]) / 2.) - end).abs()
                < 1e-6
        );
        let near = site.deck_point(p.c_north, 0.);
        let outward = [
            (left[0] + right[0]) / 2. - near[0],
            (left[2] + right[2]) / 2. - near[1],
        ];
        let edge = [right[0] - left[0], right[2] - left[2]];
        assert!(
            (edge[0] * outward[0] + edge[1] * outward[1]).abs() < 1e-6,
            "the back edge must be perpendicular to the carriageways at the landing"
        );
        let width = num(&g.dimensions, "roadwayM");
        assert!(((right[0] - left[0]).hypot(right[2] - left[2]) - width).abs() < 1e-6);
        for [a, b, c, d] in patches {
            for t in [a, b] {
                assert!((site.chainage(t[0], t[2]) - p.c_north).abs() < 1e-6);
                assert_eq!(t[1], p.deck_top(p.c_north));
            }
            for t in [c, d] {
                assert_eq!(t[1], ground(t[0], t[2]));
            }
        }
    }
}
