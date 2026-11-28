//! Battery and Main Post use cut-and-cover concrete boxes. See the SFCTA
//! Final EIS, Appendix A, preferred alternative typical cross sections,
//! and MTC's July 2015 Presidio Parkway interior photo by Karl Nielsen.
//! The model keeps the solved roadway, crown height and existing feed of
//! luminous flux per longitudinal station. Four apertures share that flux
//! and each emits from the underside of its own housing.
use super::{BORE_CLEAR_M, Builder, Chain, LIGHT_DROP_M, LIGHT_PITCH_M, LUMINAIRE_RGB, StructureClass, Station, Sweep, V3, light_groups};
use std::sync::OnceLock;

pub(super) fn applies(chain: &Chain) -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    let enabled = ENABLED.get_or_init(|| {
        let mut enable = false;
        for a in std::env::args() {
            match a.as_str() {
                "--presidio-tunnel-finish" => enable = true,
                "--no-presidio-tunnel-finish" => return false,
                _ => {}
            }
        }
        enable
    });
    // The Main Post exit ramp shares the portal but has no street name.
    *enabled && chain.pieces.iter().any(|p| p.name == "Presidio Parkway" || p.way == "167952616")
}

pub(super) fn fixture_offsets(st: &Station, group: &crate::TunnelLight) -> [f64; 4] {
    let axis = st.at(0.0, st.y);
    let centre = ((group.pos[0] - axis[0]) * st.nx + (group.pos[2] - axis[2]) * st.nz)
        / (st.ox * st.nx + st.oz * st.nz);
    [-0.62, -0.38, 0.38, 0.62].map(|f| centre + f * group.half_width)
}

pub(super) fn ceiling_lights(b: &mut Builder, chain: &Chain, sw: &Sweep, idx: &[usize]) {
    let (Some(first), Some(last)) = (idx.first(), idx.last()) else { return };
    let range = sw.st[*first].s..sw.st[*last].s;
    for light in light_groups(chain, sw).into_iter().filter(|l| range.contains(&l.s)) {
        let st = sw.lerp(light.s);
        // Relative spacing and luminous aperture length estimated from MTC's
        // photograph. Dimensions follow the local bore and the existing pitch.
        let half_length = LIGHT_PITCH_M * 0.055;
        for u in fixture_offsets(&st, &light) {
            // A housing stays rigid where the segmented road changes heading.
            // Sweeping its two ends separately can twist the lens off its lamp.
            let centre = st.at(u, st.y + BORE_CLEAR_M);
            let ring = |drop: f64| [(-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)].map(|(end, side)| {
                let along = end * half_length;
                let across = side * LIGHT_DROP_M;
                [centre[0] + along * st.tx + across * st.nx, centre[1] + along * st.slope - drop, centre[2] + along * st.tz + across * st.nz]
            });
            let top = ring(0.0);
            let lens = ring(LIGHT_DROP_M);
            let face = |b: &mut Builder, points: [V3; 4], normal: V3, tint| {
                let verts = points.map(|point| b.v(point, normal, tint, 0.0, [light.s, u], StructureClass::Metal));
                b.quad(verts, normal);
            };
            for (k, normal) in [[-st.tx, 0.0, -st.tz], [st.nx, 0.0, st.nz], [st.tx, 0.0, st.tz], [-st.nx, 0.0, -st.nz]].into_iter().enumerate() {
                let j = (k + 1) % 4;
                face(b, [top[k], top[j], lens[j], lens[k]], normal, [64, 64, 62]);
            }
            face(b, lens, super::mesh::normal3(&st, [0.0, -1.0]), LUMINAIRE_RGB);
        }
    }
}
