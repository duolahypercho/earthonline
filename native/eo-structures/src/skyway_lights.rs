//! Median standards on the eastern Skyway, registered to the USGS returns.
//! MTC's Lighting the East Span shows the tapered, five-faced shafts and
//! forward-facing banks of optics. Dimensions and sources are in eo-data.
use crate::mesh::{Builder, V3, add, cross, norm, scale, sub};
use crate::section::Dims;
use crate::sweep::Sweep;
use crate::{Chain, Era, StructureClass};
use eo_data::skyway_readings;
use std::sync::OnceLock;

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let mut on = false;
        for arg in std::env::args() {
            match arg.as_str() {
                "--east-span-lighting" => on = true,
                "--no-east-span-lighting" => return false,
                _ => {}
            }
        }
        on
    })
}

/// Source coverage ends with the registered way. The suspension system
/// west of pier E2 has a separate lighting installation.
pub fn coverage(c: &Chain) -> Option<[f64; 2]> {
    if !enabled() {
        return None;
    }
    let readings = crate::east_span::readings(c)?;
    let at = crate::east_span::registered_xz(readings);
    Some([
        at(385.)?[0],
        readings.last()?[0] * eo_data::groundscale::scale(),
    ])
}

#[derive(Clone, Debug, PartialEq)]
pub struct Standard {
    pub chain: u32,
    pub station: f64,
    pub foot: V3,
    pub road_y: f64,
    pub top: f64,
    forward: V3,
    inward: V3,
    family: usize,
}

#[derive(Clone, Copy)]
pub struct Optic {
    pub face: V3,
    pub axis: V3,
}

impl Standard {
    pub fn optics(&self) -> impl ExactSizeIterator<Item = Optic> + '_ {
        let (count, first, pitch, width, _) = skyway_readings::LAMP_FAMILIES[self.family];
        (0..count).map(move |i| {
            // The forward projections are visible in the returns used to
            // distinguish the optical rows from the shaft. Housing depth
            // and aim within the traffic direction are inferred (MTC photos).
            let y = self.top - first - pitch * i as f64;
            let tip = add(
                [self.foot[0], y, self.foot[2]],
                scale(self.forward, width * 1.35),
            );
            let across = 0.35 + 0.6 * i as f64 / (count - 1) as f64;
            let axis = norm(add(
                add(scale(self.forward, 0.8), scale(self.inward, across)),
                [0., -1., 0.],
            ));
            Optic { face: tip, axis }
        })
    }
}

pub(crate) fn place(chains: &[Chain], sweeps: &[Option<Sweep>], dims: &Dims) -> Vec<Standard> {
    if !enabled() {
        return Vec::new();
    }
    let plan = eo_data::groundscale::scale();
    let mut result = Vec::new();
    for (way, readings, direction) in [
        ("236348361", skyway_readings::WESTBOUND, -1.),
        ("237731428", skyway_readings::EASTBOUND, 1.),
    ] {
        let sections: Vec<_> = chains
            .iter()
            .filter(|c| {
                c.pieces.first().is_some_and(|p| p.way == way)
                    && crate::east_span::readings(c).is_some()
            })
            .collect();
        for &(xz, top, family) in readings {
            let [x, z] = xz.map(|v| v * plan);
            let nearest = sections
                .iter()
                .map(|&c| (c, c.project(x, z)))
                .min_by(|(_, a), (_, b)| a.1.total_cmp(&b.1));
            let Some((c, (s, _))) = nearest else { continue };
            let Some(sw) = &sweeps[c.id as usize] else {
                continue;
            };
            let st = sw.lerp(s);
            let side = ((x - st.x) * st.nx + (z - st.z) * st.nz).signum();
            if !st.barrier[usize::from(side < 0.)] {
                continue;
            }
            let road_y = sw.top(&st, side * st.half);
            let foot = st.at(
                side * (st.half + dims.barrier_base / 2.),
                road_y + dims.barrier_h,
            );
            let along = direction * st.tx.signum();
            result.push(Standard {
                chain: c.id,
                station: s,
                foot,
                road_y,
                top: road_y + top,
                forward: [along * st.tx, 0., along * st.tz],
                inward: [-side * st.nx, 0., -side * st.nz],
                family,
            });
        }
    }
    result
}

const SHAFT: [u8; 3] = [184, 190, 189];
const BRACKET: [u8; 3] = [110, 118, 117];
const LENS: [u8; 3] = [219, 226, 222];

fn face(b: &mut Builder, corners: [V3; 4], outward: V3, rgb: [u8; 3]) {
    let n = norm(cross(
        sub(corners[1], corners[0]),
        sub(corners[2], corners[0]),
    ));
    let n = if crate::mesh::dot(n, outward) < 0. {
        scale(n, -1.)
    } else {
        n
    };
    let idx = corners.map(|p| b.v(p, n, rgb, 0., [0.; 2], StructureClass::Metal));
    b.quad(idx, n);
}

pub(crate) fn shaft(b: &mut Builder, p: &Standard) {
    b.era = Era::New as u8;
    let (_, _, _, width, depth) = skyway_readings::LAMP_FAMILIES[p.family];
    let ring = |i: usize, radius: f64, y| {
        let t = std::f64::consts::TAU * i as f64 / 5.;
        let f = scale(p.forward, width * radius * t.cos());
        let s = scale(p.inward, depth * radius * t.sin());
        add([p.foot[0], y, p.foot[2]], add(f, s))
    };
    for i in 0..5 {
        let j = (i + 1) % 5;
        let corners = [
            ring(i, 0.62, p.foot[1]),
            ring(j, 0.62, p.foot[1]),
            ring(j, 0.37, p.top),
            ring(i, 0.37, p.top),
        ];
        let mid = scale(add(corners[0], corners[1]), 0.5);
        face(b, corners, sub(mid, p.foot), SHAFT);
        let cap = [corners[3], corners[2], [p.foot[0], p.top, p.foot[2]]]
            .map(|v| b.v(v, [0., 1., 0.], SHAFT, 0., [0.; 2], StructureClass::Metal));
        b.tri(cap, [0., 1., 0.]);
    }
}

pub(crate) fn draw(b: &mut Builder, poles: &[Standard], lod: u8) {
    for p in poles {
        shaft(b, p);
        let (_, _, _, width, _) = skyway_readings::LAMP_FAMILIES[p.family];
        for opt in p.optics() {
            let horizontal = norm(cross([0., 1., 0.], opt.axis));
            let vertical = cross(opt.axis, horizontal);
            let side = scale(horizontal, width * 0.36);
            let up = scale(vertical, width * 0.19);
            let a = [
                add(side, up),
                sub(up, side),
                scale(add(side, up), -1.),
                sub(side, up),
            ];
            let glass = a.map(|v| add(opt.face, v));
            face(b, glass, opt.axis, LENS);
            if lod == 2 {
                continue;
            }
            let back = [p.foot[0], opt.face[1] + width * 0.42, p.foot[2]];
            let neck = a.map(|v| add(back, scale(v, 0.6)));
            for i in 0..4 {
                let j = (i + 1) % 4;
                face(
                    b,
                    [neck[i], neck[j], glass[j], glass[i]],
                    add(a[i], a[j]),
                    BRACKET,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_measured_families_keep_their_lenses_on_the_forward_side_of_the_shaft() {
        for family in 0..skyway_readings::LAMP_FAMILIES.len() {
            for forward in [-1., 1.] {
                let p = Standard {
                    chain: 0,
                    station: 0.,
                    foot: [0., 1., 0.],
                    top: 22.,
                    road_y: 0.,
                    forward: [forward, 0., 0.],
                    inward: [0., 0., forward],
                    family,
                };
                let optics: Vec<_> = p.optics().collect();
                assert_eq!(optics.len(), skyway_readings::LAMP_FAMILIES[family].0);
                assert!(
                    optics
                        .windows(2)
                        .all(|pair| pair[0].face[1] > pair[1].face[1])
                );
                for o in optics {
                    assert!(o.face[0] * forward > 0. && o.face[1] < p.top && o.face[1] > p.foot[1]);
                    assert!(o.axis[0] * forward > 0. && o.axis[1] < 0. && o.axis[2] * forward > 0.);
                    assert!((crate::mesh::dot(o.axis, o.axis) - 1.).abs() < 1e-12);
                }
            }
        }
    }
}
