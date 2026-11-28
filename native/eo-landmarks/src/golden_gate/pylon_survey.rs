//! Exposed pylon crowns in the 2023 USGS point cloud. The survey is in the
//! tower coordinate system. Centre each pair on the mapped carriageways and
//! seat the caps on their supports at the registered bridge stations.
use super::{Frame, MeshArrays, Rgb};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Terrace {
    height: f64,
    // Across, south limit, north limit. Heights and coordinates in metres.
    sections: Vec<[f64; 3]>,
}

#[derive(Deserialize)]
pub(super) struct Crown {
    pylon: String,
    side: String,
    terraces: Vec<Terrace>,
}

fn survey_options() -> (bool, bool) {
    static OPTIONS: OnceLock<(bool, bool)> = OnceLock::new();
    *OPTIONS.get_or_init(|| {
        let mut surveyed = false;
        let mut distant = false;
        let mut disable_distant = false;
        for arg in std::env::args() {
            match arg.as_str() {
                "--golden-gate-pylons" => surveyed = true,
                "--no-golden-gate-pylons" => return (false, false),
                "--golden-gate-pylon-silhouette" => distant = true,
                "--no-golden-gate-pylon-silhouette" => disable_distant = true,
                _ => {}
            }
        }
        let distant = distant && !disable_distant;
        (surveyed || distant, distant)
    })
}

pub(super) fn enabled() -> bool {
    survey_options().0
}

pub(super) fn visible_at_distance() -> bool {
    survey_options().1
}

fn records() -> &'static [Crown] {
    static ROWS: OnceLock<Vec<Crown>> = OnceLock::new();
    ROWS.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/golden-gate-pylons.json"))
            .expect("Golden Gate pylon survey")
    })
}

pub(super) fn pair(name: &str) -> [&'static Crown; 2] {
    let get = |side| {
        records()
            .iter()
            .find(|r| r.pylon == name && r.side == side)
            .expect("two surveyed shafts per pylon")
    };
    [get("west"), get("east")]
}

impl Crown {
    pub(super) fn centre(&self) -> f64 {
        let rows = &self.terraces[0].sections;
        (rows[0][0] + rows[rows.len() - 1][0]) * 0.5
    }

    pub(super) fn height(&self) -> f64 {
        self.terraces.last().unwrap().height
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        mesh: &mut MeshArrays,
        frame: &Frame,
        c: f64,
        s: f64,
        deck: f64,
        opening: f64,
        base: f64,
        cable_cover: f64,
        vertical_scale: f64,
        colour: Rgb,
    ) {
        let plan_scale = eo_data::groundscale::scale();
        let rows = &self.terraces[0].sections;
        let south = rows.iter().map(|r| r[1]).fold(f64::INFINITY, f64::min);
        let north = rows.iter().map(|r| r[2]).fold(f64::NEG_INFINITY, f64::max);
        let c0 = (south + north) / 2.;
        let s0 = self.centre();
        let place = |r: [f64; 3], y| {
            let side = s + (r[0] - s0) * plan_scale;
            [
                frame.P(c + (r[1] - c0) * plan_scale, side, y),
                frame.P(c + (r[2] - c0) * plan_scale, side, y),
            ]
        };
        // Leave the through-walk under the inboard portion of the crown.
        let cut = s0 - s.signum() * cable_cover / plan_scale;
        let mut jamb = Vec::new();
        for w in rows.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (a[0] - s0) * s.signum() >= -cable_cover / plan_scale {
                jamb.push(a);
            }
            if a[0] < cut && cut < b[0] {
                let t = (cut - a[0]) / (b[0] - a[0]);
                jamb.push([cut, a[1] + t * (b[1] - a[1]), a[2] + t * (b[2] - a[2])]);
            }
        }
        if (rows.last().unwrap()[0] - s0) * s.signum() >= -cable_cover / plan_scale {
            jamb.push(*rows.last().unwrap());
        }
        tier(mesh, &jamb, base, opening, false, false, &place, colour);
        let mut bottom = opening;
        for (i, t) in self.terraces.iter().enumerate() {
            let top = deck + t.height * vertical_scale;
            tier(mesh, &t.sections, bottom, top, i == 0, true, &place, colour);
            bottom = top;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tier(
    mesh: &mut MeshArrays,
    rows: &[[f64; 3]],
    bottom: f64,
    top: f64,
    soffit: bool,
    lid: bool,
    place: &impl Fn([f64; 3], f64) -> [[f64; 3]; 2],
    colour: Rgb,
) {
    for w in rows.windows(2) {
        let ([al, ar], [bl, br]) = (place(w[0], bottom), place(w[1], bottom));
        let ([at, au], [bt, bu]) = (place(w[0], top), place(w[1], top));
        mesh.quad(al, bl, bt, at, colour);
        mesh.quad(br, ar, au, bu, colour);
        if lid {
            mesh.quad(at, bt, bu, au, colour);
        }
        if soffit {
            mesh.quad(ar, br, bl, al, colour);
        }
    }
    let ([lo, hi], [lt, ht]) = (place(rows[0], bottom), place(rows[0], top));
    mesh.quad(hi, lo, lt, ht, colour);
    let last = *rows.last().unwrap();
    let ([lo, hi], [lt, ht]) = (place(last, bottom), place(last, top));
    mesh.quad(lo, hi, ht, lt, colour);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surveyed_crowns_keep_the_travel_lanes_open_and_reach_the_measured_ledges() {
        let frame = Frame::new([0., 0.], [1., 0.]);
        let scale = eo_data::groundscale::scale();
        let gate = super::super::data(crate::LandmarksJs::get());
        let road_edge = super::super::num(&gate.dimensions, "roadwayM") / 2.;
        for name in ["S2", "S1", "N1", "N2"] {
            let both = pair(name);
            let middle = (both[0].centre() + both[1].centre()) / 2.;
            for cap in both {
                let mut m = MeshArrays::new("pylon crown");
                cap.draw(
                    &mut m,
                    &frame,
                    0.,
                    (cap.centre() - middle) * scale,
                    0.,
                    3.6,
                    -0.35,
                    1.2,
                    1.,
                    [0.3; 3],
                );
                let mut high = f64::NEG_INFINITY;
                for p in m.positions.chunks_exact(3) {
                    assert!(p[2].abs() > road_edge, "{name} crown over traffic");
                    high = high.max(p[1]);
                }
                assert!((high - cap.height()).abs() < 1e-8);
                assert_eq!(
                    cap.terraces.len(),
                    if name.starts_with('S') { 3 } else { 2 }
                );
            }
        }
        assert!(
            pair("N2")[1].centre() - pair("N2")[0].centre()
                > pair("N1")[1].centre() - pair("N1")[0].centre() + 10.
        );
    }
}
