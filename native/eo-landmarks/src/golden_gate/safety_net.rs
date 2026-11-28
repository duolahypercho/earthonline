//! The District's 2018 shop inspection report specifies 50 ft bracket bays
//! and diamond openings of 4.72 in. Its completed installation is 20 ft below
//! each sidewalk and projects 20 ft. The fabric is 4 mm stainless wire.
//! https://www.goldengate.org/district/district-projects/suicide-deterrent-net/
//! https://www.goldengate.org/assets/1/25/2018-0823-bocomm-no3-awardsdnsshopinspectionservices.pdf
//! https://www.goldengate.org/assets/1/6/suicide-deterrent-feir-visual-impact-assessment.pdf
//! https://www.goldengate.org/assets/1/25/2025-0724-bocomm-no5-engrpt.pdf (p. 14)
//! https://www.worldstainless.org/files/issf/non-image-files/PDF/Stainless_Steel_in_Infrastructure_Bridges.pdf
use super::{Ctx, Frame, MeshArrays, Plan, num, structure_scale};
use crate::geometry::{Frozen, V3};
use std::sync::OnceLock;

pub const DRAW: &str = "bridge-safety-mesh";
pub const OPENING_M: f64 = 4.72 * 0.0254;
pub const WIRE_M: f64 = 0.004;
const OUTBOARD_M: f64 = 20.0 * 0.3048;
const BRACKET_PITCH_M: f64 = 50.0 * 0.3048;
// The District reports 110 installed panels at the housing. Distribute them
// equally between the two sides in the absence of an individual panel survey.
const UPRIGHT_PANELS_PER_SIDE: usize = 110 / 2;

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let args: Vec<_> = std::env::args().collect();
        !args.iter().any(|s| s == "--no-golden-gate-net")
            && (super::surveyed_surfaces() || args.iter().any(|s| s == "--golden-gate-net"))
    })
}

/// Both the renderer and the collision world consume these four corners.
pub(crate) struct Panel {
    pub corners: [V3; 4],
    pub upright: bool,
    pub side: f64,
    pub last: bool,
}

pub(crate) fn housing_limits(plan: &Plan<'_>) -> [f64; 2] {
    // At the North Anchorage Housing the District specifies a 300 ft vertical
    // barrier, rising 8 ft above a 4 ft wall (2009 FEIR Visual Impact
    // Assessment addendum, figures 1–3). Register it to the surveyed housing.
    let centre = plan.anchor_north_c + super::pylons::north_pylon_spacing() / 2.;
    let half_fence = 150. * 0.3048 * eo_data::groundscale::scale();
    [centre - half_fence, centre + half_fence].map(|s| s.clamp(plan.c_south, plan.c_north))
}

/// The sidewalk widens over the housing roof, with the wall at its outside
/// edge (FEIR addendum, figure 1). The wall thickness is estimated from the
/// section in figure 2. The first quad is pavement, the other three concrete.
pub(crate) fn housing_faces(
    panel: &Panel,
    plan: &Plan<'_>,
    frame: &Frame<'_>,
    width: f64,
    curb: f64,
) -> [[V3; 4]; 4] {
    let [a, _, _, b] = panel.corners;
    let half = super::pylons::housing_half_width(plan.site);
    let row = |p: V3| {
        let c = frame.chainage(p[0], p[2]);
        let y = plan.deck_top(c) + curb;
        let at = |offset, top| frame.P(c, panel.side * offset, top);
        let thickness = 0.3048 * structure_scale();
        [
            at(width / 2., y),
            at(half - thickness / 2., y),
            at(half - thickness / 2., p[1]),
            at(half + thickness / 2., p[1]),
            at(
                half + thickness / 2.,
                super::pylons::north_roof_top(plan.deck_top(c)),
            ),
        ]
    };
    let (a, b) = (row(a), row(b));
    std::array::from_fn(|i| {
        let mut face = [a[i], a[i + 1], b[i + 1], b[i]];
        if panel.side < 0. {
            face.reverse();
        }
        face
    })
}

pub(crate) fn panels(plan: &Plan<'_>, frame: &Frame<'_>, width: f64, curb: f64) -> Vec<Panel> {
    let scale = structure_scale();
    let reach = OUTBOARD_M * scale;
    let pitch = BRACKET_PITCH_M * eo_data::groundscale::scale();
    let (start, end) = (plan.c_south, plan.c_north);
    let [lo, hi] = housing_limits(plan);
    let mut stations = vec![start, lo, hi, end];
    let mut c = (start / pitch).floor() * pitch + pitch;
    while c < end {
        if c < lo || c > hi {
            stations.push(c);
        }
        c += pitch;
    }
    for i in 1..UPRIGHT_PANELS_PER_SIDE {
        stations.push(lo + (hi - lo) * i as f64 / UPRIGHT_PANELS_PER_SIDE as f64);
    }
    stations.sort_by(f64::total_cmp);
    stations.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    let mut out = Vec::with_capacity((stations.len() - 1) * 2);
    for side in [-1., 1.] {
        for interval in stations.windows(2) {
            let (a, b) = (interval[0], interval[1]);
            let upright = lo <= (a + b) / 2. && (a + b) / 2. <= hi;
            let ends = |station: f64| {
                let edge = side
                    * if upright {
                        super::pylons::housing_half_width(plan.site)
                    } else {
                        width / 2.
                    };
                let deck = plan.deck_top(station) + curb;
                if upright {
                    [
                        frame.P(station, edge, deck + 4. * 0.3048 * scale),
                        frame.P(station, edge, deck + 12. * 0.3048 * scale),
                    ]
                } else {
                    [
                        frame.P(station, edge, deck - reach),
                        frame.P(station, edge + side * reach, deck - reach),
                    ]
                }
            };
            let [ai, ao] = ends(a);
            let [bi, bo] = ends(b);
            out.push(Panel {
                corners: [ai, ao, bo, bi],
                upright,
                side,
                last: b == end || b == lo || b == hi,
            });
        }
    }
    out
}

impl Ctx<'_> {
    /// The arms' depths, ties and border diameters are estimates from the
    /// District's completed-system photographs.
    pub(super) fn safety_net(
        &self,
        steel: &mut MeshArrays,
        cables: &mut MeshArrays,
        concrete: &mut MeshArrays,
        deck: &mut MeshArrays,
        far: &mut MeshArrays,
    ) -> Frozen {
        let mut mesh = MeshArrays::new(DRAW);
        let scale = structure_scale();
        for panel in panels(
            self.plan,
            self.frame,
            num(self.d, "widthM"),
            num(self.d, "curbM"),
        ) {
            let [ai, ao, bo, bi] = panel.corners;
            let len = (bi[0] - ai[0]).hypot(bi[2] - ai[2]);
            let tangent = [
                0.5 + 0.5 * (bi[0] - ai[0]) / len,
                0.5 + 0.5 * (bi[2] - ai[2]) / len,
                0.,
            ];
            if panel.upright {
                let [floor, inside, cap, outside] = housing_faces(
                    &panel,
                    self.plan,
                    self.frame,
                    num(self.d, "widthM"),
                    num(self.d, "curbM"),
                );
                let [a, b, c, d] = floor;
                deck.quad(a, b, c, d, self.k.sidewalk);
                let distant = super::suspension::distant_bridge_decks();
                if distant { far.quad(a, b, c, d, self.k.sidewalk); }
                for [a, b, c, d] in [inside, cap, outside] {
                    concrete.quad(a, b, c, d, self.k.concrete);
                    if distant { far.quad(a, b, c, d, self.k.concrete); }
                }
                // Solid rods keep their silhouette against the sky as well as
                // the land. Use the bridge cable renderer's true wire diameter.
                let pitch = 6.5 * 0.0254 * eo_data::groundscale::scale();
                let count = (len / pitch).round().max(1.) as usize;
                for rod in 0..count {
                    let t = (rod as f64 + 0.5) / count as f64;
                    let base = std::array::from_fn(|i| ai[i] + (bi[i] - ai[i]) * t);
                    let top = std::array::from_fn(|i| ao[i] + (bo[i] - ao[i]) * t);
                    cables.wire_segment(base, top, 0.5 * 0.0254 * scale, self.k.orange);
                }
                steel.beam(ao, bo, 0.055 * scale, 0.055 * scale, self.k.orange);
                steel.beam(ai, bi, 0.055 * scale, 0.055 * scale, self.k.orange);
                steel.beam(ai, ao, 0.07 * scale, 0.07 * scale, self.k.orange);
                if panel.last {
                    steel.beam(bi, bo, 0.07 * scale, 0.07 * scale, self.k.orange);
                }
            } else {
                mesh.quad(ai, ao, bo, bi, tangent);
                let mut bracket = |inner: V3, outer: V3| {
                    let root_inset = (num(self.d, "widthM") / 2. - self.site.half_plane).max(0.);
                    let mut root = inner;
                    root[0] -= self.site.across[0] * panel.side * root_inset;
                    root[2] -= self.site.across[1] * panel.side * root_inset;
                    steel.beam(root, outer, 0.19 * scale, 0.26 * scale, self.k.orange);
                    root[1] += 0.65 * OUTBOARD_M * scale;
                    cables.wire_segment(root, outer, 0.027 * scale, self.k.orange_dark);
                };
                bracket(ai, ao);
                if panel.last {
                    bracket(bi, bo);
                }
                cables.wire_segment(ao, bo, 0.026 * scale, [0.42; 3]);
                cables.wire_segment(ai, bi, 0.026 * scale, [0.42; 3]);
            }
        }
        mesh.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_safety_panels_join_on_both_sides_and_turn_upright_at_the_anchorage() {
        let js = crate::LandmarksJs::get();
        let gate = super::super::data(js);
        let site = super::super::Site::new(&gate);
        let p = super::super::plan(js, &site, &|_, _| 0.);
        let offset = |c| site.centre_offset(c);
        let mut f = Frame::new(site.axis_point(0., 0.), site.axis);
        f.offset = Some(&offset);
        let width = num(&gate.dimensions, "widthM");
        let curb = num(&gate.dimensions, "curbM");
        let list = panels(&p, &f, width, curb);
        for side in [-1., 1.] {
            let side_panels: Vec<_> = list.iter().filter(|b| b.side == side).collect();
            let fence: Vec<_> = side_panels.iter().filter(|b| b.upright).collect();
            assert_eq!(fence.len(), UPRIGHT_PANELS_PER_SIDE);
            let first = fence.first().unwrap().corners[0];
            let last = fence.last().unwrap().corners[3];
            assert!(site.chainage(first[0], first[2]) > p.anchor_north_c);
            assert!(
                site.chainage(last[0], last[2])
                    < p.anchor_north_c + super::super::pylons::north_pylon_spacing()
            );
            let mut upright_length = 0.;
            for pair in side_panels.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if a.upright == b.upright {
                    assert_eq!(a.corners[2], b.corners[1]);
                    assert_eq!(a.corners[3], b.corners[0]);
                }
            }
            for panel in side_panels {
                let [a, b, _, d] = panel.corners;
                let c = site.chainage(a[0], a[2]);
                let [x, z] = site.deck_point(c, 0.);
                let lateral = (a[0] - x) * site.across[0] + (a[2] - z) * site.across[1];
                let edge = if panel.upright {
                    super::super::pylons::housing_half_width(&site)
                } else {
                    width / 2.
                };
                assert!((lateral * side - edge).abs() < 1e-6);
                if panel.upright {
                    upright_length += site.chainage(d[0], d[2]) - c;
                    assert!((b[1] - a[1] - 8. * 0.3048 * structure_scale()).abs() < 1e-6);
                } else {
                    assert!(
                        ((a[0] - b[0]).hypot(a[2] - b[2]) - OUTBOARD_M * structure_scale()).abs()
                            < 1e-6
                    );
                    assert!(
                        (p.deck_top(c) + curb - a[1] - OUTBOARD_M * structure_scale()).abs() < 1e-6
                    );
                }
            }
            assert!((upright_length - 300. * 0.3048 * eo_data::groundscale::scale()).abs() < 1e-5);
        }
    }
}
