//! Lighting layout of the two Yerba Buena decks. The 1962 Division of Highways
//! reconstruction account places the lower fixtures along both walls. The
//! photographs show individual lower lenses and two lines on the upper vault.
//! https://libraryarchives.metro.net/DPGTL/Californiahighways/chpw_1962_janfeb.pdf
//! https://commons.wikimedia.org/wiki/File:Yerba_Buena_Island_tunnel.jpg
//! https://commons.wikimedia.org/wiki/File:Lower_deck_(5947642641).jpg
use super::{
    Builder, IslandTunnelSection, LIGHT_DROP_M, LIGHT_PITCH_M, LUMINAIRE_RGB, Station,
    StructureClass, Sweep, TunnelInfo, crown_of, mesh, p,
};
use std::sync::OnceLock;

pub(super) fn applies(info: &TunnelInfo) -> Option<IslandTunnelSection> {
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| {
        let mut on = false;
        for arg in std::env::args() {
            match arg.as_str() {
                "--yerba-buena-lighting" => on = true,
                "--no-yerba-buena-lighting" => return false,
                _ => {}
            }
        }
        on
    }) {
        info.island_section
    } else {
        None
    }
}

/// The surveyed vault is round above the spring. Share vertices between
/// its concrete panels so their lighting follows that curvature. Wall and
/// jamb faces retain their own vertices and normals at the spring.
pub(super) fn smooth_vault() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        let mut on = false;
        for arg in std::env::args() {
            match arg.as_str() {
                "--yerba-buena-vault-finish" => on = true,
                "--no-yerba-buena-vault-finish" => return false,
                _ => {}
            }
        }
        on
    })
}

pub(super) fn vault(
    builder: &mut Builder,
    sweep: &Sweep,
    stations: &[usize],
    shape: IslandTunnelSection,
    profile: impl Fn(&Station) -> Vec<mesh::P>,
) {
    let mut previous = Vec::<u32>::new();
    let mut current = Vec::new();
    for &i in stations {
        let st = &sweep.st[i];
        let points = profile(st);
        let roof = &points[1..points.len() - 1];
        let half = (roof[0].u - roof[roof.len() - 1].u) * 0.5;
        let centre = roof[0].u - half;
        for p in roof {
            let (slope, arc) = shape.vault_face((p.u - centre) / half, half * st.ox.hypot(st.oz));
            let normal = mesh::normal3(st, [slope, -1.]);
            current.push(builder.v(
                st.at(p.u, p.y),
                normal,
                sweep.tint,
                p.wear,
                [st.s, arc],
                StructureClass::Concrete,
            ));
        }
        for (a, b) in previous.windows(2).zip(current.windows(2)) {
            builder.quad([a[0], a[1], b[1], b[0]], [0., -1., 0.]);
        }
        std::mem::swap(&mut current, &mut previous);
        current.clear();
    }
}

/// Both the luminous face and the local lighting sample use this section.
/// The upper offset is estimated from the photographs. The lower mounting
/// follows the soffit and sidewall intersection, with its lens tilted inward.
pub(super) fn rows(shape: IslandTunnelSection, st: &Station, walls: [f64; 2]) -> [[f64; 2]; 2] {
    let (centre, half) = crown_of(walls);
    [-1., 1.].map(|side| match shape {
        IslandTunnelSection::Vault { .. } => {
            let across = std::f64::consts::FRAC_1_SQRT_2;
            [
                centre + side * half * across,
                st.y + shape.height(across) - LIGHT_DROP_M,
            ]
        }
        IslandTunnelSection::Flat { .. } => [
            centre + side * (half - LIGHT_DROP_M),
            st.y + shape.crown() - LIGHT_DROP_M,
        ],
    })
}

pub(super) fn strips(
    b: &mut Builder,
    c: &super::Chain,
    sw: &Sweep,
    part: &[usize],
    info: &TunnelInfo,
    shape: IslandTunnelSection,
) {
    let row = |st: &Station, side: usize| {
        let i = super::station_index(sw, st);
        rows(shape, st, info.wall[i])[side]
    };
    for side in 0..2 {
        let sign = if side == 0 { -1. } else { 1. };
        if matches!(shape, IslandTunnelSection::Vault { .. }) {
            mesh::strip(b, sw, part, StructureClass::Metal, [65, 66, 63], &|st| {
                let [u, y] = row(st, side);
                vec![
                    p(u + LIGHT_DROP_M, y + LIGHT_DROP_M, 0.),
                    p(u + LIGHT_DROP_M, y, 0.),
                ]
            });
            mesh::strip(b, sw, part, StructureClass::Metal, LUMINAIRE_RGB, &|st| {
                let [u, y] = row(st, side);
                vec![p(u + LIGHT_DROP_M, y, 0.), p(u - LIGHT_DROP_M, y, 0.)]
            });
            mesh::strip(b, sw, part, StructureClass::Metal, [65, 66, 63], &|st| {
                let [u, y] = row(st, side);
                vec![
                    p(u - LIGHT_DROP_M, y, 0.),
                    p(u - LIGHT_DROP_M, y + LIGHT_DROP_M, 0.),
                ]
            });
            continue;
        }
        // Use the same centres as the light sources. The relative length and
        // separation of the lenses follow the lower-deck photograph.
        let first = sw.st[part[0]].s;
        let last = sw.st[*part.last().unwrap()].s;
        for light in super::light_groups(c, sw)
            .into_iter()
            .map(|l| l.s)
            .filter(|s| (first..last).contains(s))
        {
            let st = sw.lerp(light);
            let [u, y] = row(&st, side);
            let n = mesh::norm([-sign * st.nx, -1., -sign * st.nz]);
            let half = LIGHT_PITCH_M * 0.22;
            let corners =
                [(-half, -1.), (-half, 1.), (half, 1.), (half, -1.)].map(|(along, across)| {
                    let q = st.at(u + across * LIGHT_DROP_M, y - sign * across * LIGHT_DROP_M);
                    b.v(
                        [
                            q[0] + along * st.tx,
                            q[1] + along * st.slope,
                            q[2] + along * st.tz,
                        ],
                        n,
                        LUMINAIRE_RGB,
                        0.,
                        [light + along, u],
                        StructureClass::Metal,
                    )
                });
            b.quad(corners, n);
        }
    }
}
