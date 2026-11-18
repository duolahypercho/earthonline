//! The groves round the landmark towers: the tree canopy of Pioneer Park on
//! Telegraph Hill round Coit Tower and of the Mount Sutro woods round Sutro
//! Tower, drawn as one landmark each (crowns, trunks, a far item) so the
//! hilltops read wooded at the distances the towers are seen from (the
//! props tiles' park trees reach only the tiles round the camera, and the
//! far trees ride the HLOD layer, which this adapter does not draw).
//!
//! Where a tree stands is data, not authored: a point of a world-anchored
//! jittered lattice inside the site's reach carries a tree where
//!
//! * a mapped green of a tree class covers it (OpenStreetMap greens of
//!   public/data/sf/sf-lots-parks.json.gz: `ground_cover::tree_class_of`;
//!   Pioneer Park is OSM way 224941774, the hill's slopes way 28377936),
//! * the ESA WorldCover 10 m 2021 class there is tree cover (10;
//!   public/data/sf/sf-land-cover.bin.gz), so the lawn, the parking circle
//!   and the plaza stay open as the classifier saw them,
//! * no mapped car park (the Coit Tower parking circle is OSM relation
//!   4221904), no carriageway or path of sf-city.json `detailRoads`
//!   (one city lane, `CaStandards` `CITY_LANE_ID`, either side of a way's
//!   half width) and no building of its `detailBuildings` (the crown's
//!   inner half kept clear of the walls) is under its trunk, and it stands
//!   clear of the tower's own base.
//!
//! The species and their sizes:
//!
//! | Site | Species | Height, crown | Source |
//! | --- | --- | --- | --- |
//! | Pioneer Park | Monterey cypress, Monterey pine, tea tree | cypress 12-20 m, crown 0.75-0.95 of the height wide; pine 14-24 m, 0.55-0.7; tea tree 4-7 m | the 1930s WPA planting of 'cypress, pine, and tea trees' (Wikipedia, Pioneer Park (San Francisco); The Cultural Landscape Foundation, Pioneer Park); 'the surrounding cypress and Monterey pine trees got the upper hand' (San Francisco Chronicle, on the forest ringing Coit Tower). Monterey pine 15-30 m (Wikipedia, Pinus radiata); Monterey cypress broad and flat-topped in exposed sites (Wikipedia, Hesperocyparis macrocarpa); coastal tea tree to about 5 m, rarely 8 (Wikipedia, Leptospermum laevigatum). Heights within those on an exposed hilltop INFERRED |
//! | Mount Sutro | blue gum eucalyptus | 25-40 m, crown 0.35-0.5 of the height wide | the Mount Sutro forest of Tasmanian blue gum planted from the 1880s (Wikipedia, Mount Sutro; UCSF, Mount Sutro Open Space Reserve); blue gum to 45-55 m (Wikipedia, Eucalyptus globulus). Heights in a dense urban stand INFERRED |
//!
//! Foliage albedo: conifer and eucalyptus canopies reflect about 3-5 % of
//! blue and red light and 6-10 % of green (vegetation reflectance spectra,
//! USGS Spectral Library v7, conifer and eucalyptus leaf spectra); the
//! linear tones below sit in that band (INFERRED within it). Lattice
//! pitch, lumps per crown and the crowns' shading are INFERRED.
//!
//! `eo-app --no-grove` (or `--no-towers3`) draws none, the previous output.

use crate::LandmarkLight;
use crate::geometry::{MeshArrays, Rgb, V3};
use crate::set::{Draw, Landmark, Role};
use crate::skyline::plain_material;
use anyhow::Result;
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};

/// On unless `eo-app --no-grove` set `EO_GROVE=off` (and only with the third pass).
pub fn grove_on() -> bool {
    crate::towers3_on() && std::env::var("EO_GROVE").map(|v| v != "off").unwrap_or(true)
}

/// ESA WorldCover's tree-cover class.
pub const WORLDCOVER_TREE: u8 = 10;

/// A species: linear foliage albedo, height range (m), crown width as a
/// share of the height, the crown's foot as a share of the height, lumps
/// per crown, and the crown's flatness (vertical over horizontal radius).
#[derive(Clone, Copy, Debug)]
pub struct Species {
    pub name: &'static str,
    pub foliage: Rgb,
    pub height_m: [f64; 2],
    pub width_share: [f64; 2],
    pub crown_foot: f64,
    pub lumps: usize,
    pub flat: f64,
}

pub const MONTEREY_CYPRESS: Species = Species { name: "monterey-cypress", foliage: [0.034, 0.062, 0.026], height_m: [12.0, 20.0], width_share: [0.75, 0.95], crown_foot: 0.2, lumps: 5, flat: 0.55 };
pub const MONTEREY_PINE: Species = Species { name: "monterey-pine", foliage: [0.038, 0.066, 0.030], height_m: [14.0, 24.0], width_share: [0.55, 0.7], crown_foot: 0.32, lumps: 4, flat: 0.7 };
pub const TEA_TREE: Species = Species { name: "tea-tree", foliage: [0.058, 0.074, 0.044], height_m: [4.0, 7.0], width_share: [0.9, 1.2], crown_foot: 0.15, lumps: 3, flat: 0.6 };
pub const BLUE_GUM: Species = Species { name: "blue-gum", foliage: [0.046, 0.062, 0.040], height_m: [25.0, 40.0], width_share: [0.4, 0.55], crown_foot: 0.16, lumps: 3, flat: 1.1 };
/// Bark: a weathered grey-brown, linear (INFERRED).
pub const BARK: Rgb = [0.045, 0.036, 0.028];

/// A grove: the landmark it rings (its registry anchor, unscaled), how far
/// it reaches (m, scaled ground), how far its trees keep from the
/// landmark's axis (m), the lattice pitch (m) and the species mix (weights).
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub id: &'static str,
    pub name: &'static str,
    pub landmark: &'static str,
    pub reach_m: f64,
    pub keep_out_m: f64,
    pub pitch_m: f64,
    pub mix: &'static [(Species, f64)],
}

/// Pioneer Park round Coit Tower: the tower's base building and the terrace
/// round it (the registry's 16 m radius and 6 m more) stay open (INFERRED).
/// Mount Sutro round Sutro Tower: the tower's compound (its legs' base
/// triangle, 45.72 m a side, circumradius 26.4 m, and 14 m more) stays open (INFERRED).
pub const SITES: &[Site] = &[
    Site { id: "grove-pioneer-park", name: "Pioneer Park", landmark: "coit-tower", reach_m: 260.0, keep_out_m: 22.0, pitch_m: 6.5, mix: &[(MONTEREY_CYPRESS, 0.45), (MONTEREY_PINE, 0.4), (TEA_TREE, 0.15)] },
    Site { id: "grove-mount-sutro", name: "Mount Sutro", landmark: "sutro-tower", reach_m: 520.0, keep_out_m: 40.0, pitch_m: 8.5, mix: &[(BLUE_GUM, 1.0)] },
];

/// A road or path near a grove: its points (scaled) and half width (m).
pub struct Way {
    pub points: Vec<[f64; 2]>,
    pub half_m: f64,
}

fn hash(a: i64, b: i64, salt: u32) -> f64 {
    let mut h = (a as u32).wrapping_mul(0x9e37_79b1) ^ (b as u32).wrapping_mul(0x85eb_ca77) ^ salt.wrapping_mul(0xc2b2_ae3d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h as f64 / u32::MAX as f64
}

/// Value noise on a direction (the crown's lumpiness), about [-1, 1].
fn noise3(p: V3, seed: u32) -> f64 {
    let f = [p[0].floor(), p[1].floor(), p[2].floor()];
    let t = [p[0] - f[0], p[1] - f[1], p[2] - f[2]].map(|x| x * x * (3.0 - 2.0 * x));
    let at = |dx: i64, dy: i64, dz: i64| {
        let (x, y, z) = (f[0] as i64 + dx, f[1] as i64 + dy, f[2] as i64 + dz);
        hash(x.wrapping_mul(7919) ^ z, y, seed) * 2.0 - 1.0
    };
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = lerp(at(0, 0, 0), at(1, 0, 0), t[0]);
    let x10 = lerp(at(0, 1, 0), at(1, 1, 0), t[0]);
    let x01 = lerp(at(0, 0, 1), at(1, 0, 1), t[0]);
    let x11 = lerp(at(0, 1, 1), at(1, 1, 1), t[0]);
    lerp(lerp(x00, x10, t[1]), lerp(x01, x11, t[1]), t[2])
}

fn norm(a: V3) -> V3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// The unit icosphere subdivided `level` times: vertices and triangles (outward, counter-clockwise).
pub fn icosphere(level: usize) -> (Vec<V3>, Vec<[u32; 3]>) {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let mut v: Vec<V3> = [[-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0], [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t], [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0]].iter().map(|p| norm(*p)).collect();
    let mut f: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..level {
        let mut mid = std::collections::HashMap::new();
        let mut midpoint = |a: u32, b: u32, v: &mut Vec<V3>| -> u32 {
            let key = (a.min(b), a.max(b));
            *mid.entry(key).or_insert_with(|| {
                let (p, q) = (v[a as usize], v[b as usize]);
                v.push(norm([p[0] + q[0], p[1] + q[1], p[2] + q[2]]));
                (v.len() - 1) as u32
            })
        };
        let mut g = Vec::with_capacity(f.len() * 4);
        for [a, b, c] in f {
            let ab = midpoint(a, b, &mut v);
            let bc = midpoint(b, c, &mut v);
            let ca = midpoint(c, a, &mut v);
            g.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        f = g;
    }
    (v, f)
}

/// One tree as placed: trunk foot, height, crown radius, species, seed.
#[derive(Clone, Copy, Debug)]
pub struct Tree {
    pub at: V3,
    pub height: f64,
    pub radius: f64,
    pub species: Species,
    pub seed: u32,
}

/// Push a smooth-shaded vertex (the crowns are not flat-shaded solids).
fn vertex(m: &mut MeshArrays, p: V3, n: V3, c: Rgb) {
    m.positions.extend_from_slice(&p);
    m.normals.extend_from_slice(&n);
    m.colors.extend_from_slice(&c);
    m.min_y = m.min_y.min(p[1]);
    m.max_y = m.max_y.max(p[1]);
}

/// A crown: `lumps` displaced ellipsoids round the upper trunk. Each
/// vertex's normal leans toward the whole crown's ellipsoid (so the tree
/// shades as one mass, its lumps showing in the silhouette), its colour
/// darkens into the crown's underside and between lumps (sky occlusion,
/// INFERRED: 0.55 at the underside), and varies a little per tree and per
/// leaf cluster.
pub fn crown(m: &mut MeshArrays, t: &Tree, sphere: &(Vec<V3>, Vec<[u32; 3]>), lumps: usize) {
    let s = &t.species;
    let foot = t.height * s.crown_foot;
    let ch = t.height - foot;
    let centre = [t.at[0], t.at[1] + foot + ch * 0.5, t.at[2]];
    let (rx, ry) = (t.radius, (ch * 0.5).max(1.0));
    let tint = 0.85 + 0.3 * hash(t.seed as i64, 1, 41);
    let warm = 0.94 + 0.12 * hash(t.seed as i64, 2, 43);
    let base: Rgb = [s.foliage[0] * tint * warm, s.foliage[1] * tint, s.foliage[2] * tint / warm];
    for k in 0..lumps.max(1) {
        let (lc, lr, ly) = if k == 0 {
            ([centre[0], centre[1] + ry * 0.1, centre[2]], rx * 0.78, ry * 0.8)
        } else {
            let a = TAU * (k as f64 / (lumps - 1).max(1) as f64) + hash(t.seed as i64, k as i64, 47) * 1.3;
            let d = rx * (0.38 + 0.18 * hash(t.seed as i64, k as i64, 53));
            let h = ry * (-0.55 + 0.8 * hash(t.seed as i64, k as i64, 59));
            ([centre[0] + a.cos() * d, centre[1] + h, centre[2] + a.sin() * d], rx * (0.5 + 0.12 * hash(t.seed as i64, k as i64, 61)), ry * (0.55 + 0.15 * hash(t.seed as i64, k as i64, 67)))
        };
        let first = m.vertex_count();
        for d in &sphere.0 {
            let bump = 1.0 + 0.16 * noise3([d[0] * 2.3, d[1] * 2.3, d[2] * 2.3], t.seed.wrapping_add(k as u32 * 101)) + 0.07 * noise3([d[0] * 5.1, d[1] * 5.1, d[2] * 5.1], t.seed.wrapping_add(7));
            let p = [lc[0] + d[0] * lr * bump, lc[1] + d[1] * ly * bump, lc[2] + d[2] * lr * bump];
            // the whole crown's ellipsoid normal at p, and the lump's own.
            let q = [(p[0] - centre[0]) / (rx * rx), (p[1] - centre[1]) / (ry * ry), (p[2] - centre[2]) / (rx * rx)];
            let own = norm([d[0] / lr, d[1] / ly, d[2] / lr]);
            let whole = norm(q);
            let n = norm([whole[0] * 0.6 + own[0] * 0.4, whole[1] * 0.6 + own[1] * 0.4, whole[2] * 0.6 + own[2] * 0.4]);
            // sky occlusion: the underside and the lumps' inner faces take less sky.
            let rel = ((p[1] - (centre[1] - ry)) / (2.0 * ry)).clamp(0.0, 1.0);
            let out = ((p[0] - centre[0]) * own[0] + (p[2] - centre[2]) * own[2]) / rx.max(0.1);
            let occ = (0.55 + 0.45 * rel.powf(0.8)) * (0.8 + 0.2 * out.clamp(0.0, 1.0));
            let cluster = 0.88 + 0.24 * hash((p[0] * 1.7) as i64, (p[2] * 1.7 + p[1] * 3.1) as i64, 71);
            vertex(m, p, n, base.map(|c| c * occ * cluster));
        }
        for f in &sphere.1 {
            m.indices.extend_from_slice(&[first + f[0], first + f[1], first + f[2]]);
        }
    }
}

/// A trunk: a tapered square post from the ground into the crown (flat-shaded).
pub fn trunk(m: &mut MeshArrays, t: &Tree) {
    let top = t.height * (t.species.crown_foot + 0.15);
    let r0 = (t.height * 0.018).clamp(0.12, 0.6);
    let r1 = r0 * 0.55;
    let yaw = hash(t.seed as i64, 3, 73) * PI;
    let ring = |r: f64, y: f64| -> [V3; 4] {
        let mut out = [[0.0; 3]; 4];
        for (i, o) in out.iter_mut().enumerate() {
            let a = yaw - i as f64 * PI / 2.0; // clockwise in (x, z), as `box_`'s rings
            *o = [t.at[0] + a.cos() * r, t.at[1] + y, t.at[2] + a.sin() * r];
        }
        out
    };
    let (b, k) = (ring(r0, -0.4), ring(r1, top));
    m.hexahedron(b, k, BARK, crate::geometry::NO_BOTTOM);
}

/// Inside a ring (even-odd).
fn inside(r: &[[f64; 2]], x: f64, z: f64) -> bool {
    crate::waterfront::point_in_ring(r, x, z)
}

fn seg_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dz * t)
}

/// What a grove stands on: the greens of a tree class, the car parks,
/// the ways and the buildings near it (all on the scaled plan), and the land cover.
pub struct Ground<'a> {
    pub greens: Vec<(Vec<[f64; 2]>, Vec<Vec<[f64; 2]>>)>,
    pub lots: Vec<Vec<[f64; 2]>>,
    pub ways: &'a [Way],
    /// Building footprints with their bounds (min x, min z, max x, max z).
    pub buildings: &'a [(Vec<[f64; 2]>, [f64; 4])],
    pub tree_cover: &'a dyn Fn(f64, f64) -> bool,
    pub height_at: &'a dyn Fn(f64, f64) -> Option<f64>,
}

/// The trees of a site about `centre` (the landmark's scaled anchor).
pub fn plant(site: &Site, centre: [f64; 2], g: &Ground) -> Vec<Tree> {
    let s = site.pitch_m;
    let total: f64 = site.mix.iter().map(|m| m.1).sum();
    let mut out = Vec::new();
    let (i0, i1) = (((centre[0] - site.reach_m) / s).floor() as i64, ((centre[0] + site.reach_m) / s).ceil() as i64);
    let (j0, j1) = (((centre[1] - site.reach_m) / s).floor() as i64, ((centre[1] + site.reach_m) / s).ceil() as i64);
    for j in j0..=j1 {
        for i in i0..=i1 {
            let x = (i as f64 + 0.1 + 0.8 * hash(i, j, 1)) * s;
            let z = (j as f64 + 0.1 + 0.8 * hash(i, j, 2)) * s;
            let r = (x - centre[0]).hypot(z - centre[1]);
            if r > site.reach_m || r < site.keep_out_m {
                continue;
            }
            if !g.greens.iter().any(|(ring, holes)| inside(ring, x, z) && !holes.iter().any(|h| inside(h, x, z))) {
                continue;
            }
            // the trunk (up to 0.6 m in radius) and a little more clear of a car park.
            let foot = [[x, z], [x + 1.0, z], [x - 1.0, z], [x, z + 1.0], [x, z - 1.0]];
            if !(g.tree_cover)(x, z) || g.lots.iter().any(|l| foot.iter().any(|p| inside(l, p[0], p[1]))) {
                continue;
            }
            if g.ways.iter().any(|w| w.points.windows(2).any(|q| seg_distance([x, z], q[0], q[1]) < w.half_m)) {
                continue;
            }
            let Some(y) = (g.height_at)(x, z) else { continue };
            // the species by its weight, then its size.
            let mut roll = hash(i, j, 3) * total;
            let mut sp = site.mix[0].0;
            for (m, w) in site.mix {
                if roll < *w {
                    sp = *m;
                    break;
                }
                roll -= w;
            }
            let height = sp.height_m[0] + (sp.height_m[1] - sp.height_m[0]) * hash(i, j, 4);
            let width = sp.width_share[0] + (sp.width_share[1] - sp.width_share[0]) * hash(i, j, 5);
            // no trunk in a building, and the crown's inner half clear of its walls.
            let rc = height * width / 4.0;
            let probes = [[x, z], [x + rc, z], [x - rc, z], [x, z + rc], [x, z - rc]];
            if g.buildings.iter().any(|(r, b)| x + rc >= b[0] && x - rc <= b[2] && z + rc >= b[1] && z - rc <= b[3] && probes.iter().any(|p| inside(r, p[0], p[1]))) {
                continue;
            }
            out.push(Tree { at: [x, y - 0.1, z], height, radius: height * width / 2.0, species: sp, seed: (hash(i, j, 6) * 1.0e9) as u32 });
        }
    }
    out
}

/// The grove's landmark: crowns and trunks near, a crown each far.
pub fn landmark(site: &Site, centre: [f64; 2], trees: &[Tree]) -> Option<Landmark> {
    if trees.is_empty() {
        return None;
    }
    let t0 = std::time::Instant::now();
    let near_sphere = icosphere(1);
    let far_sphere = icosphere(0);
    let mut crowns = MeshArrays::new(&format!("{}:crowns", site.id));
    let mut trunks = MeshArrays::new(&format!("{}:trunks", site.id));
    let mut far = MeshArrays::new(&format!("{}:far", site.id));
    for t in trees {
        crown(&mut crowns, t, &near_sphere, t.species.lumps);
        trunk(&mut trunks, t);
        crown(&mut far, t, &far_sphere, 1);
    }
    // foliage: a rough dielectric (F0 0.04), no surface kind; bark rougher.
    let leaf = plain_material(0.85, 0.0, "facade-masonry", None);
    let bark = plain_material(0.95, 0.0, "facade-masonry", None);
    let draws = vec![
        Draw { key: "crowns".into(), role: Role::Near, material: leaf.clone(), mesh: crowns.freeze() },
        Draw { key: "trunks".into(), role: Role::Near, material: bark, mesh: trunks.freeze() },
    ];
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for d in &draws {
        for p in d.mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
    }
    let mut draws = draws;
    draws.push(Draw { key: "far".into(), role: Role::Far, material: leaf, mesh: far.freeze() });
    let count = |n: &str| trees.iter().filter(|t| t.species.name == n).count();
    let names: Vec<&str> = site.mix.iter().map(|m| m.0.name).collect();
    Some(Landmark {
        id: site.id.into(),
        name: site.name.into(),
        version: "grove-v1".into(),
        anchor: centre,
        box_min: mn,
        box_max: mx,
        draws,
        colliders: vec![],
        stats: json!({ "trees": trees.len(), "species": names.iter().map(|n| (n.to_string(), count(n))).collect::<std::collections::BTreeMap<_, _>>(), "landmark": site.landmark }),
        build_ms: t0.elapsed().as_secs_f64() * 1e3,
    })
}

/// The ways of sf-city.json `detailRoads` within `reach` of `centres`,
/// scaled to the ground plan, each with its half width: half its lanes
/// of the city lane, plus one lane (a path: one lane) as the trunk's
/// clearance; and the `detailBuildings` footprints there (the kit draws
/// them on the scaled plan), with their bounds.
pub fn city_near(repo_root: &std::path::Path, centres: &[([f64; 2], f64)], lane_m: f64) -> Result<(Vec<Way>, Vec<(Vec<[f64; 2]>, [f64; 4])>)> {
    #[derive(serde::Deserialize)]
    struct Building {
        #[serde(default)]
        points: Vec<f64>,
    }
    #[derive(serde::Deserialize)]
    struct Road {
        #[serde(default)]
        highway: String,
        #[serde(default)]
        lanes: Option<f64>,
        #[serde(default)]
        points: Vec<f64>,
    }
    #[derive(serde::Deserialize)]
    struct CityFile {
        #[serde(rename = "detailRoads")]
        detail_roads: Vec<Road>,
        #[serde(rename = "detailBuildings")]
        detail_buildings: Vec<Building>,
    }
    let path = repo_root.join("public/data/sf/sf-city.json.gz");
    let f = std::fs::File::open(&path)?;
    let city: CityFile = serde_json::from_reader(std::io::BufReader::new(flate2::read::GzDecoder::new(f)))?;
    let s = eo_data::groundscale::scale();
    let mut out = Vec::new();
    for r in city.detail_roads {
        let pts: Vec<[f64; 2]> = r.points.chunks_exact(2).map(|p| [p[0] * s, p[1] * s]).collect();
        if !pts.iter().any(|p| centres.iter().any(|(c, reach)| (p[0] - c[0]).hypot(p[1] - c[1]) < reach + 200.0)) {
            continue;
        }
        let path_like = matches!(r.highway.as_str(), "footway" | "path" | "steps" | "pedestrian" | "cycleway");
        let lanes = if path_like { 1.0 } else { r.lanes.unwrap_or(2.0).max(1.0) };
        out.push(Way { points: pts, half_m: (lanes * lane_m / 2.0 + if path_like { 0.0 } else { lane_m / 2.0 }) * s });
    }
    let mut buildings = Vec::new();
    for b in city.detail_buildings {
        let r: Vec<[f64; 2]> = b.points.chunks_exact(2).map(|p| [p[0] * s, p[1] * s]).collect();
        if r.len() < 3 {
            continue;
        }
        let bb = r.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].max(p[0]), a[3].max(p[1])]);
        if !centres.iter().any(|(c, reach)| bb[2] >= c[0] - reach && bb[0] <= c[0] + reach && bb[3] >= c[1] - reach && bb[1] <= c[1] + reach) {
            continue;
        }
        buildings.push((r, bb));
    }
    Ok((out, buildings))
}

/// Every grove, on the drawn ground `height_at` (the scaled plan: a grove
/// is ground cover, so it is planted where the scaled greens are).
pub fn build_all(repo_root: &std::path::Path, height_at: &(dyn Fn(f64, f64) -> Option<f64> + Sync), ca: &eo_data::CaStandards) -> Result<(Vec<Landmark>, Vec<LandmarkLight>)> {
    let js = crate::LandmarksJs::get();
    let s = eo_data::groundscale::scale();
    let centres: Vec<([f64; 2], f64)> = SITES
        .iter()
        .map(|site| {
            let a = &js.registry[site.landmark].anchor;
            ([a["x"].as_f64().unwrap_or(0.0) * s, a["z"].as_f64().unwrap_or(0.0) * s], site.reach_m)
        })
        .collect();
    let lane = ca.metric(eo_data::standards::CITY_LANE_ID)?;
    let (ways, buildings) = city_near(repo_root, &centres, lane)?;
    let (greens, _) = crate::plaza::load_areas(repo_root)?;
    let lots = load_lots(repo_root)?;
    let cover = eo_data::LandCover::load(&eo_data::DataPaths { repo_root: repo_root.to_path_buf() })?;
    let tree_cover = |x: f64, z: f64| cover.class_at(x, z) == WORLDCOVER_TREE;
    let mut out = Vec::new();
    for (site, (c, reach)) in SITES.iter().zip(&centres) {
        // a ring whose bounds meet the site's square (a large green may have no vertex near it).
        let near = |r: &[[f64; 2]]| {
            let b = r.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].max(p[0]), a[3].max(p[1])]);
            b[2] >= c[0] - reach && b[0] <= c[0] + reach && b[3] >= c[1] - reach && b[1] <= c[1] + reach
        };
        let g = Ground {
            greens: greens.iter().filter(|a| crate::ground_cover::tree_class_of(&a.kind) > 0 && near(&a.polygon)).map(|a| (a.polygon.clone(), a.holes.clone())).collect(),
            lots: lots.iter().filter(|l| near(l)).cloned().collect(),
            ways: &ways,
            buildings: &buildings,
            tree_cover: &tree_cover,
            height_at,
        };
        let trees = plant(site, *c, &g);
        if let Some(l) = landmark(site, *c, &trees) {
            out.push(l);
        }
    }
    Ok((out, Vec::new()))
}

/// The car parks of sf-lots-parks.json.gz (`lots`), on the scaled plan.
pub fn load_lots(repo_root: &std::path::Path) -> Result<Vec<Vec<[f64; 2]>>> {
    use std::io::Read;
    let bytes = std::fs::read(repo_root.join("public/data/sf/sf-lots-parks.json.gz"))?;
    let mut s = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut s)?;
    let v: Value = serde_json::from_str(&s)?;
    let gs = eo_data::groundscale::scale();
    Ok(v["lots"]
        .as_array()
        .map(|a| a.iter().filter_map(|l| l["polygon"].as_array().map(|p| p.chunks_exact(2).map(|c| [c[0].as_f64().unwrap_or(0.0) * gs, c[1].as_f64().unwrap_or(0.0) * gs]).collect::<Vec<_>>())).filter(|r| r.len() >= 3).collect())
        .unwrap_or_default())
}
