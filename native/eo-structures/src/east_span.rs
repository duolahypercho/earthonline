//! Bay Bridge east span profiles and piers. The registered decks are surveyed
//! individually. The support lines follow Caltrans' elevation in fig. 1.1-1
//! (see eo-data/data/bridge-roads.NOTICE.txt). No generic freeway columns
//! stand between W2 and E2 in the self-anchored suspension span.
use crate::Chain;
use eo_data::bridge_survey;

// Tower centre in source coordinates (USGS returns above 150 m NAVD88).
const TOWER: [f64; 2] = [5361.05, -4122.52];
// Pier stations, metres east of the tower. W2 and E2 use the published
// 180 m back span and 385 m main span. Other stations are digitized from
// the same elevation, through the last mapped Oakland touchdown support.
const PIER_STATIONS: &[f64] = &[
    -630.0, -594.0, -558.0, -526.0, -471.0, -404.0, -333.0, -258.0, -180.0, 385.0, 545.0, 705.0,
    865.0, 1025.0, 1185.0, 1345.0, 1505.0, 1665.0, 1825.0, 1985.0, 2135.0, 2270.0, 2390.0, 2485.0,
    2555.0, 2620.0, 2675.0, 2725.0, 2775.0, 2825.0, 2875.0,
];

// The orthotropic boxes and the concrete skyway at midspan are 5.5 m deep.
pub const BOX_DEPTH_M: f64 = 5.5;

pub fn readings(c: &Chain) -> Option<&'static [[f64; 3]]> {
    let way = &c.pieces.first()?.way;
    if c.pieces.iter().any(|p| p.way != *way) {
        return None;
    }
    let profile = bridge_survey::east_span(way)?;
    // A future extension of either way needs its own survey and support plan.
    // The street graph can snap the end of a chain to the adjoining roadway.
    let scale = eo_data::groundscale::scale();
    let covered = c.kind == crate::ChainKind::Deck
        && c.points.iter().all(|p| {
            let point = [p[0] / scale, p[1] / scale];
            profile.windows(2).any(|s| {
                // Five metres is the radius of each LiDAR sample in the register.
                crate::index::project(point, [s[0][0], s[0][1]], [s[1][0], s[1][1]]).1 <= 5.0
            })
        });
    covered.then_some(profile)
}

/// Register a pier against the complete source carriageway, then project it
/// onto this chain only if it falls inside it (the street splits one way).
pub(crate) fn registered_xz(readings: &[[f64; 3]]) -> impl Fn(f64) -> Option<[f64; 2]> {
    let scale = eo_data::groundscale::scale();
    let points: Vec<_> = readings.iter().map(|p| [p[0], p[1]]).collect();
    let cum = eo_street::plan::arc_table(&points);
    let mut nearest = (f64::INFINITY, 0.0);
    for (i, arm) in points.windows(2).enumerate() {
        let (t, dist, _) = crate::index::project(TOWER, arm[0], arm[1]);
        if dist < nearest.0 {
            nearest = (dist, cum[i] + t * (cum[i + 1] - cum[i]));
        }
    }
    move |offset| {
        let along = nearest.1 + offset;
        if along <= 0.0 || along >= *cum.last().unwrap() {
            return None;
        }
        let i = cum.partition_point(|&s| s < along) - 1;
        let t = (along - cum[i]) / (cum[i + 1] - cum[i]);
        let point = |k| (points[i][k] + t * (points[i + 1][k] - points[i][k])) * scale;
        Some([point(0), point(1)])
    }
}

/// A transverse joint on the two registered carriageways, as an east-facing
/// line (nx, nz, n · point). The left and right edges of each road share
/// the same joint location at every mesh detail level.
pub fn pavement_joints() -> [[f64; 3]; 3] {
    use eo_data::bridge_survey::{EAST_EASTBOUND, EAST_WESTBOUND};
    let west = registered_xz(EAST_WESTBOUND);
    let east = registered_xz(EAST_EASTBOUND);
    // Caltrans SAS QA/QC report, App. F: hinge K is 10 m west of W2 and
    // hinge A is 49.385 m east of E2. Caltrans A354 report, Fig. 1.1-1:
    // the concrete Skyway extends 2084.823 m to the Oakland approach.
    const HINGE_A: f64 = 385.0 + 49.385;
    [-180.0 - 10.0, HINGE_A, HINGE_A + 2084.823].map(|at| {
        let a = west(at).expect("westbound bridge survey covers the paving joint");
        let b = east(at).expect("eastbound bridge survey covers the paving joint");
        let across = [b[0] - a[0], b[1] - a[1]];
        let length = across[0].hypot(across[1]);
        let nx = across[1].abs() / length;
        let nz = -across[0] * across[1].signum() / length;
        [nx, nz, nx * a[0] + nz * a[1]]
    })
}

/// The source-specific paving applies only to the two measured deck ways.
pub fn pavement_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::args().any(|s| s == "--no-east-span-pavement")
        && std::env::args().any(|s| s == "--east-span-pavement"))
}

/// West abutment W2 separates the island transition viaduct's short
/// concrete spans from the long-span orthotropic and skyway box girders.
pub fn west_abutment_x(readings: &[[f64; 3]]) -> Option<f64> {
    registered_xz(readings)(-180.0).map(|p| p[0])
}

pub fn piers(c: &Chain, readings: &[[f64; 3]]) -> Vec<f64> {
    let scale = eo_data::groundscale::scale();
    let at = registered_xz(readings);
    let mut stations = vec![];
    for &offset in PIER_STATIONS {
        let Some([x, z]) = at(offset) else { continue };
        let (s, dist) = c.project(x, z);
        if dist <= 1.0 * scale && s > 0.0 && s < c.length() {
            stations.push(s);
        }
    }
    stations.sort_by(f64::total_cmp);
    stations
}

/// The tower and cable system ride in the same tiles as the decks. The
/// four shafts and splayed hangers are a simplified drawing of the Caltrans
/// elevation, 160 m tower, 10 m suspender spacing and 0.78 m main cable.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub a: [f64; 3],
    pub b: [f64; 3],
    width: f64,
    tower: bool,
}

pub fn steel(chains: &[Chain], streets: &eo_street::StreetWorld) -> Vec<Member> {
    let scale = eo_data::groundscale::scale();
    let hs = eo_data::groundscale::height_scale();
    let [x, z] = TOWER.map(|v| v * scale);
    let mut members = vec![];
    let mut west_loop = vec![];
    let mut tower_drawn = false;
    for c in chains
        .iter()
        .filter(|c| c.is_built() && readings(c).is_some())
    {
        let (s_tower, dist) = c.project(x, z);
        let east = if c.points[0][0] < c.points.last().unwrap()[0] {
            1.0
        } else {
            -1.0
        };
        let s = |offset: f64| s_tower + east * offset * scale;
        // One of the eastbound way's chains ends before the tower.
        if dist > 50.0 * scale
            || ![s(-180.0), s(385.0)]
                .iter()
                .all(|&v| (0.0..=c.length()).contains(&v))
        {
            continue;
        }
        let f = c.frame(s_tower);
        let top = eo_data::SEA_LEVEL_Y + 160.0 * hs;
        if !tower_drawn {
            let ground = streets.ground(x, z);
            // Inferred square sections for the four tapering steel shafts.
            let corner = |u: f64, v: f64, h: f64| {
                let reach = scale * (5.5 - 3.0 * h);
                [
                    x + (f.tx * u + f.nx * v) * reach,
                    ground + h * (top - ground),
                    z + (f.tz * u + f.nz * v) * reach,
                ]
            };
            for u in [-1.0, 1.0] {
                for v in [-1.0, 1.0] {
                    members.push(Member {
                        a: corner(u, v, 0.0),
                        b: corner(u, v, 1.0),
                        width: 3.0 * scale,
                        tower: true,
                    });
                    for h in [0.32, 0.50, 0.70, 0.90, 1.0] {
                        if u < 0.0 {
                            members.push(Member {
                                a: corner(u, v, h),
                                b: corner(-u, v, h),
                                width: 1.0 * scale,
                                tower: true,
                            });
                        }
                        if v < 0.0 {
                            members.push(Member {
                                a: corner(u, v, h),
                                b: corner(u, -v, h),
                                width: 1.0 * scale,
                                tower: true,
                            });
                        }
                    }
                }
            }
            tower_drawn = true;
        }
        let outside = if (x - f.x) * f.nx + (z - f.z) * f.nz > 0.0 {
            -1.0
        } else {
            1.0
        };
        let saddle = [
            x + outside * f.nx * 2.5 * scale,
            top,
            z + outside * f.nz * 2.5 * scale,
        ];
        for extent in [-180.0_f64, 385.0] {
            let count = (extent.abs() / 10.0).ceil() as usize;
            let mut last = saddle;
            for i in 1..=count {
                let t = i as f64 / count as f64;
                let station = s(t * extent);
                let frame = c.frame(station);
                let edge = frame.at(outside * c.half_at(station));
                let deck = c.y_at(station);
                // Parabolic suspension profile. Its axis splays from the saddle
                // towards the outboard girder, terminating on the deck.
                let centre = c.frame(s_tower);
                let ref_edge = centre.at(outside * c.half_at(s_tower));
                let cable = [
                    edge[0] + (saddle[0] - ref_edge[0]) * (1.0 - t),
                    deck + (top - c.y_at(s_tower)) * (1.0 - t).powi(2),
                    edge[1] + (saddle[2] - ref_edge[1]) * (1.0 - t),
                ];
                members.push(Member {
                    a: last,
                    b: cable,
                    width: 0.78 * scale,
                    tower: false,
                });
                if i < count {
                    // Inferred rendered hanger width, between the cable and the outer edge.
                    members.push(Member {
                        a: cable,
                        b: [edge[0], deck, edge[1]],
                        width: 0.10 * scale,
                        tower: false,
                    });
                } else if extent < 0.0 {
                    west_loop.push([edge[0], deck - BOX_DEPTH_M * hs, edge[1]]);
                    members.push(Member {
                        a: cable,
                        b: *west_loop.last().unwrap(),
                        width: 0.78 * scale,
                        tower: false,
                    });
                }
                last = cable;
            }
        }
    }
    if let [a, b] = west_loop.as_slice() {
        members.push(Member {
            a: *a,
            b: *b,
            width: 0.78 * scale,
            tower: false,
        });
    }
    members
}

/// Emit the same members into the visible mesh and the solid tower collider.
pub fn draw(members: &[Member], b: &mut crate::mesh::Builder, tower_only: bool) {
    if members.is_empty() {
        return;
    }
    let mut mesh = eo_landmarks::geometry::MeshArrays::new("east-span-steel");
    for m in members.iter().filter(|m| !tower_only || m.tower) {
        mesh.beam(m.a, m.b, m.width, m.width, [1.0; 3]);
    }
    b.era = crate::Era::New as u8;
    let base = b.m.positions.len() as u32;
    for (p, n) in mesh
        .positions
        .chunks_exact(3)
        .zip(mesh.normals.chunks_exact(3))
    {
        b.v(
            [p[0], p[1], p[2]],
            [n[0], n[1], n[2]],
            [196, 200, 198],
            0.0,
            [0.0; 2],
            crate::StructureClass::Metal,
        );
    }
    b.m.indices.extend(mesh.indices.iter().map(|i| base + i));
}
