//! Transverse floor system between S1 and N1. The District's historic
//! resources evaluation reproduces the National Register description, section
//! 7: floorbeams 8½ ft deep at 50 ft intervals, with knee braces to the lower
//! chords. The existing steel is visible in the June 2025 engineering report,
//! slides 7, 8 and 10. Plate gauges and bearing footprints are photo estimates.
//! https://www.goldengate.org/assets/1/6/suicide-deterrent-historic-resources-evaluation-report.pdf
//! https://www.goldengate.org/assets/1/25/2025-0626-bocomm-no8-seismicpresentation.pdf
use crate::geometry::MeshArrays;
use crate::suspension::{DECK_PLATE_DEPTH_M, Frame, TrussSpec};
use std::sync::OnceLock;

pub(super) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let args: Vec<_> = std::env::args().collect();
        args.iter().any(|s| s == "--golden-gate-floorbeams")
            && !args.iter().any(|s| s == "--no-golden-gate-floorbeams")
    })
}

pub(crate) fn draw(mesh: &mut MeshArrays, frame: &Frame, c: f64, deck: f64, truss: &TrussSpec) {
    let scale = truss.member;
    let depth = 8.5 * 0.3048 * scale;
    let half_span = truss.half_plane;
    let face = deck - DECK_PLATE_DEPTH_M;
    let bearing_height = 0.55 * scale;
    let top = face - bearing_height;
    let bottom = top - depth;
    let flange = 0.07 * scale;
    let width = 0.65 * scale;
    let at = |s, y| frame.P(c, s, y);
    let across = |mesh: &mut MeshArrays, y: f64, width: f64, depth: f64| {
        mesh.beam(
            at(-half_span, y),
            at(half_span, y),
            width,
            depth,
            truss.colour,
        );
    };
    across(mesh, top - flange / 2., width, flange);
    across(mesh, bottom + flange / 2., width, flange);
    across(
        mesh,
        (top + bottom) / 2.,
        0.034 * scale,
        depth - 2. * flange,
    );
    // Existing transverse kickers rise inboard from each lower chord. The
    // transverse section shows a 30 ft 11 in working point from the centre.
    let knee_at = (30. + 11. / 12.) / 45. * half_span;
    for side in [-1., 1.] {
        mesh.beam(
            at(side * half_span, deck - truss.depth),
            at(side * knee_at, bottom),
            0.29 * scale,
            0.35 * scale,
            truss.colour,
        );
    }
    // The orthotropic roadway sits on pedestals, leaving daylight over the
    // floorbeam. Stiffening plates transfer the pedestal loads to its web.
    for bearing in [-3, -1, 1, 3] {
        let s = f64::from(bearing) * half_span / 4.;
        let [x, z] = frame.point(c, s);
        mesh.box_(
            x,
            z,
            bottom + flange,
            top - flange,
            width * 0.44,
            0.0275 * scale,
            frame.yaw,
            truss.colour,
            crate::geometry::ALL,
        );
        mesh.box_(
            x,
            z,
            top,
            face,
            width * 0.31,
            width * 0.38,
            frame.yaw,
            truss.colour,
            crate::geometry::ALL,
        );
    }
}

/// Welded longitudinal ribs under the 1980s replacement roadway. FHWA
/// FHWA-IF-12-027, figure 3-4 (Golden Gate row), and the deck section on
/// slides 37–38 of the 2009 Western Bridge Engineers' Seminar:
/// https://highways.dot.gov/media/212136
/// https://www.wsdot.wa.gov/eesc/bridge/WBES/2009/D/3D2/3D2.pdf
/// The pitch and widths follow the registered roadway across its 62 ft.
/// The underside alone is visible. Adjacent runs share their section.
pub(super) fn deck_ribs(mesh: &mut MeshArrays, ctx: &super::Ctx) {
    let p = ctx.plan;
    let width = super::num(ctx.d, "roadwayM");
    let across = width / (62. * 0.3048);
    let pitch = 0.673 * across;
    let top = 0.356 * across / 2.;
    let bottom = 0.152 * across / 2.;
    let depth = 0.279 * ctx.h;
    let colour = ctx.soffit();
    let count = ((width - 2. * top) / pitch).floor() as usize + 1;
    // The suspended deck rises and falls. Break at the tower stations and
    // sample the deck line at its existing far-strip pitch.
    for span in [p.arch_north_c, 0., p.l, p.anchor_north_c].windows(2) {
        let stations =
            crate::suspension::stations(span[0], span[1], crate::suspension::FAR_STRIP_STEP_M);
        for ends in stations.windows(2) {
            let [a, b] = [ends[0], ends[1]];
            let face = [
                p.deck_top(a) - DECK_PLATE_DEPTH_M,
                p.deck_top(b) - DECK_PLATE_DEPTH_M,
            ];
            for i in 0..count {
                let s = (i as f64 - (count - 1) as f64 * 0.5) * pitch;
                let section = [(-top, 0.), (-bottom, depth), (bottom, depth), (top, 0.)];
                for edge in section.windows(2) {
                    let [(left, down_left), (right, down_right)] = [edge[0], edge[1]];
                    mesh.quad(
                        ctx.P(a, s + left, face[0] - down_left),
                        ctx.P(b, s + left, face[1] - down_left),
                        ctx.P(b, s + right, face[1] - down_right),
                        ctx.P(a, s + right, face[0] - down_right),
                        colour,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bearings_touch_the_soffit_and_the_web_meets_both_flange_plates() {
        let frame = Frame::new([0., 0.], [1., 0.]);
        let deck = 75.;
        let truss = TrussSpec {
            deck_top: &|_| deck,
            c_start: 0.,
            c_end: 50.,
            half_plane: 14.,
            depth: 7.6,
            panel: 7.62,
            colour: [1.; 3],
            dark: [0.; 3],
            gap: &|_, _| false,
            far_half_width: 14.,
            far_above: 0.,
            end_plate_cover: DECK_PLATE_DEPTH_M,
            floorbeam_span: Some([0., 50.]),
            member: 1.,
        };
        let mut mesh = MeshArrays::new("suspended floorbeam");
        draw(&mut mesh, &frame, 0., deck, &truss);
        let soffit = deck - DECK_PLATE_DEPTH_M;
        let verts: Vec<_> = mesh.positions.chunks_exact(3).collect();
        assert!(verts.iter().all(|p| p[1] <= soffit + 1e-9));
        for s in [-10.5, -3.5, 3.5, 10.5] {
            assert!(
                verts
                    .iter()
                    .any(|p| (p[1] - soffit).abs() < 1e-9 && (p[2] - s).abs() < 0.3)
            );
        }
        let top = soffit - 0.55;
        let bottom = top - 8.5 * 0.3048;
        for boundary in [top - 0.07, bottom + 0.07] {
            for side in [-14., 14.] {
                let at = |p: &&[f64]| (p[2] - side).abs() < 1e-9 && (p[1] - boundary).abs() < 1e-9;
                assert!(verts.iter().any(|p| at(p) && p[0].abs() < 0.03), "web");
                assert!(verts.iter().any(|p| at(p) && p[0].abs() > 0.3), "flange");
            }
        }
    }
}
