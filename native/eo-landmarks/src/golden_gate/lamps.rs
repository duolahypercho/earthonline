//! Model and illumination share the measured station and the underside of each
//! housing. USGS 3DEP source and extraction method accompany the inventory.
//! PNNL-21894 (2012), fig. 1.0 and appendices D/E, identify the amber lens.
use super::{ALL, Ctx, Frame, MeshArrays, NO_BOTTOM, Plan, Rgb, num, srgb_hex};
use std::sync::OnceLock;

pub(crate) struct Lamp {
    pub station: f64,
    pole: f64,
    pub head: f64,
    pub crown: f64,
    pub tall: bool,
}

impl Lamp {
    /// The shell and five-sided enclosure taken together are about 0.30 m
    /// deep in the profile of the head returns.
    pub fn lens_height(&self) -> f64 {
        self.crown - if self.tall { 0.20 } else { 0.30 }
    }

    fn mesh(&self, frame: &Frame, deck: f64, curb: f64, steel: &mut MeshArrays, colour: Rgb) {
        let at = |s, y| frame.P(self.station, s, deck + y);
        let [x, z] = frame.point(self.station, self.pole);
        let reach = self.head - self.pole;
        // Shared casting profile fitted to the isolated main-span standards.
        // The taller south approach has a longer swept arm and a shallow lens.
        let arm = |portion, below| at(self.pole + reach * portion, self.crown - below);
        steel.box_(
            x,
            z,
            deck + curb,
            deck + curb + 0.40,
            0.22,
            0.22,
            frame.yaw,
            colour,
            NO_BOTTOM,
        );
        steel.beam(at(self.pole, curb + 0.4), arm(0., 1.05), 0.23, 0.23, colour);
        let bend = [
            arm(0., 1.05),
            arm(0.06, 0.82),
            arm(0.19, 0.59),
            arm(0.39, 0.38),
            arm(0.64, 0.22),
            arm(1., 0.16),
        ];
        for pair in bend.windows(2) {
            steel.beam(pair[0], pair[1], 0.19, 0.19, colour);
        }
        // Figure 1.0 shows a second, upper curved finger and a backplate.
        // The laboratory measured the luminous opening at 24 by 22.5 inches.
        let toward = reach.signum();
        if !self.tall {
            let high = [
                arm(0., 1.05),
                arm(0.07, 0.64),
                arm(0.23, 0.32),
                arm(0.42, 0.13),
                arm(0.61, 0.08),
                arm(0.72, 0.08),
            ];
            for pair in high.windows(2) {
                steel.beam(pair[0], pair[1], 0.13, 0.13, colour);
            }
            steel.beam(arm(0.62, 0.08), arm(0.62, 0.32), 0.13, 0.13, colour);
        }
        let cabinet_offset = if self.tall { 0. } else { 0.18 * toward };
        let [hx, hz] = frame.point(self.station, self.head - cabinet_offset);
        let (along, across) = if self.tall {
            (0.26, 0.42)
        } else {
            (0.33, 0.50)
        };
        let lens = self.lens_height();
        let rim = lens + if self.tall { 0.05 } else { 0.12 };
        steel.box_(
            hx,
            hz,
            deck + rim,
            deck + self.crown,
            along,
            across,
            frame.yaw,
            colour,
            ALL,
        );
        // Taper the side walls of the enclosure to the luminous underside.
        let (lens_along, lens_across) = if self.tall {
            (along * 0.94, across * 0.94)
        } else {
            (24. * 0.0254 / 2., 22.5 * 0.0254 / 2.)
        };
        let lens_ring = |size: f64, y| {
            [(-1., -1.), (-1., 1.), (1., 1.), (1., -1.)].map(|(c, s)| {
                frame.P(
                    self.station + c * lens_along * size,
                    self.head + s * lens_across * size,
                    deck + y,
                )
            })
        };
        let glass = if self.tall {
            srgb_hex(0xb9b7a6)
        } else {
            srgb_hex(0xc2a146)
        };
        steel.hexahedron(lens_ring(0.98, lens), lens_ring(1., rim), glass, ALL);
    }
}

pub(crate) fn fixtures(p: &Plan, width: f64) -> impl Iterator<Item = Lamp> + use<> {
    static RECORDS: OnceLock<Vec<(f64, i8, f64, u8)>> = OnceLock::new();
    let records = RECORDS.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/golden-gate-lamps.json"))
            .expect("Golden Gate lamp survey")
    });
    let scale = eo_data::groundscale::scale();
    let (south, north) = (p.c_south, p.c_north);
    records.iter().filter_map(move |&(c, side, crown, kind)| {
        let station = c * scale;
        if station < south || station > north {
            return None;
        }
        // Half the measured shaft-to-shaft width in the straight main span.
        // Centre it on the game's two carriageways through the approach curves.
        let pole = f64::from(side) * width * (12.84 / 27.432);
        let reach = if kind == 1 { 2.52 } else { 1.55 };
        Some(Lamp {
            station,
            pole,
            head: pole - f64::from(side) * reach,
            crown,
            tall: kind == 1,
        })
    })
}

impl Ctx<'_> {
    pub(super) fn measured_standards(&self, steel: &mut MeshArrays) {
        for lamp in fixtures(self.plan, num(self.d, "widthM")) {
            lamp.mesh(
                self.frame,
                self.plan.deck_top(lamp.station),
                num(self.d, "curbM"),
                steel,
                self.k.orange,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_luminaires_fit_the_mapped_deck_and_share_the_drawn_lens() {
        let js = crate::LandmarksJs::get();
        let gate = super::super::data(js);
        let site = super::super::Site::new(&gate);
        let p = super::super::plan(js, &site, &|_, _| 0.);
        let width = num(&gate.dimensions, "widthM");
        let lamps: Vec<_> = fixtures(&p, width).collect();
        assert_eq!(lamps.len(), 122);
        assert_eq!(lamps.iter().filter(|l| l.tall).count(), 9);
        for l in lamps {
            assert!(l.pole.abs() < width / 2. && l.head.abs() < l.pole.abs());
            let mut mesh = MeshArrays::new("surveyed fixture");
            l.mesh(
                &Frame::new([0., 0.], [1., 0.]),
                0.,
                0.2,
                &mut mesh,
                [0.5; 3],
            );
            let face: Vec<_> = mesh
                .positions
                .chunks_exact(3)
                .filter(|v| (v[1] - l.lens_height()).abs() < 1e-8)
                .collect();
            assert!(face.iter().any(|v| v[0] < l.station && v[2] < l.head));
            assert!(face.iter().any(|v| v[0] > l.station && v[2] > l.head));
        }
    }
}
