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
//! The fourth pass (`crate::towers4_on`; `eo-app --no-towers4` plants as
//! the third did) carries the planting further:
//!
//! | What | How | Source |
//! | --- | --- | --- |
//! | Coit Tower's summit open | no crown over the parking circle (the whole crown, not only the trunk, clear of OSM relation 4221904's polygon by a metre), and the trees within 160 m of the tower no taller than a cone from the base building's roof (5.97 m over the summit, landmarks-js.json coit-tower `lidarMedianM`) rising 0.2 m a metre outward, nor than a cone from 1 m over the summit at the parking circle's edge rising 0.25 m a metre, so the base, its terrace and the parking loop show over the canopy from the tower's oblique views (a sight line 11 deg down passes 0.2 m a metre over the slope) | Pioneer Park's summit is the parking circle and the tower's terrace, its woods on the slopes below (Wikipedia, Pioneer Park (San Francisco)); the cone's slope INFERRED from the views' depression angles |
//! | Houses and roads keep their ground | a crown and 2 m more clear of every mapped building footprint; the forest's edge along a way thinned (a tree within 6 m of the way's edge kept with odds 0.55, shorter there), the gaps taken by understory shrubs 2-4.5 m high that hide the trunks | the 2 m clearance and the edge's thinning INFERRED; the understory of the Mount Sutro and Pioneer Park woods (ivy, blackberry, tea tree and native scrub: Wikipedia, Mount Sutro; Pioneer Park) |
//! | Crowns by species | Monterey cypress flat-topped and spreading (its lumps over the crown's top), Monterey pine a rounded dome, blue gum tall and ragged (its lumps strung up the stem and off it, gaps between), each tree 0.85-1.15 of its species' width and height again and its foliage's hue and value jittered by up to 8 % | the species' forms as Wikipedia describes them (Hesperocyparis macrocarpa: 'often flat-topped' in exposed sites; Pinus radiata; Eucalyptus globulus: a tall, open, irregular crown); the jitter INFERRED |
//! | Trunks | six-sided, as thick as a stem of the tree's height (diameter at breast height about 1/40 of the height, INFERRED from the species' published sizes), crowns starting lower on cypress and pine; bark by species: blue gum's smooth shed bark a pale cream-grey (linear 0.22), pine and cypress a dark grey-brown | Wikipedia, Eucalyptus globulus: the bark sheds in strips, leaving a smooth, pale surface; Pinus radiata: dark, deeply furrowed bark. Albedos INFERRED |
//! | The forest floor | a litter layer under the canopy (dark brown duff, linear 0.05-0.07), each vertex darkened by the sky the crowns above it hide (the share of the overhead sky they cover, to 0.4 of the open ground's) on the drawn ground 0.25 m up, so the canopy stands on its own shade instead of on sunlit bare earth | litter of pine needles and eucalyptus bark (Mount Sutro Open Space Reserve, UCSF); the occlusion is the sky the crowns subtend (INFERRED: no sun shadow beyond the 160 m of the sun's cascades reaches it) |
//! | Coit's parking loop lit at night | a post-top luminaire every 22 m round the parking circle, 1.5 m outside its edge, 4.5 m up, the street slice's pool photometry | INFERRED (the loop's lamps light the terrace and the canopy's edge round it) |
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
    /// v4: the crown's form (FORM_*), its foot (share of the height) and its bark (linear).
    pub form: u8,
    pub crown_foot4: f64,
    pub bark: Rgb,
}

/// v4 crown forms: a rounded dome, flat-topped and spreading, tall and ragged, a low shrub.
pub const FORM_DOME: u8 = 0;
pub const FORM_FLAT: u8 = 1;
pub const FORM_RAGGED: u8 = 2;
pub const FORM_SHRUB: u8 = 3;

pub const MONTEREY_CYPRESS: Species = Species { name: "monterey-cypress", foliage: [0.034, 0.062, 0.026], height_m: [12.0, 20.0], width_share: [0.75, 0.95], crown_foot: 0.2, lumps: 5, flat: 0.55, form: FORM_FLAT, crown_foot4: 0.12, bark: [0.07, 0.055, 0.045] };
pub const MONTEREY_PINE: Species = Species { name: "monterey-pine", foliage: [0.038, 0.066, 0.030], height_m: [14.0, 24.0], width_share: [0.55, 0.7], crown_foot: 0.32, lumps: 4, flat: 0.7, form: FORM_DOME, crown_foot4: 0.22, bark: [0.06, 0.045, 0.036] };
pub const TEA_TREE: Species = Species { name: "tea-tree", foliage: [0.058, 0.074, 0.044], height_m: [4.0, 7.0], width_share: [0.9, 1.2], crown_foot: 0.15, lumps: 3, flat: 0.6, form: FORM_SHRUB, crown_foot4: 0.08, bark: [0.09, 0.08, 0.07] };
pub const BLUE_GUM: Species = Species { name: "blue-gum", foliage: [0.046, 0.062, 0.040], height_m: [25.0, 40.0], width_share: [0.4, 0.55], crown_foot: 0.16, lumps: 3, flat: 1.1, form: FORM_RAGGED, crown_foot4: 0.3, bark: [0.22, 0.2, 0.17] };
/// v4: the understory at a wood's edge (ivy-hung scrub, blackberry, young
/// tea tree: Wikipedia, Mount Sutro; Pioneer Park), 2-4.5 m, wider than high (INFERRED).
pub const UNDERSTORY: Species = Species { name: "understory", foliage: [0.05, 0.07, 0.035], height_m: [2.0, 4.5], width_share: [1.1, 1.5], crown_foot: 0.05, lumps: 3, flat: 0.6, form: FORM_SHRUB, crown_foot4: 0.04, bark: [0.08, 0.07, 0.06] };
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
    let mut lights = Vec::new();
    let v4 = crate::towers4_on();
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
        if v4 {
            // Coit Tower's summit cone, from its base building's roof over the summit.
            let summit = (site.landmark == "coit-tower").then(|| Summit {
                y: height_at(c[0], c[1]).unwrap_or(0.0),
                lift_m: js.buildings["coit-tower"]["dimensions"]["lidarMedianM"].as_f64().unwrap_or(6.0),
                lot: summit_lot(*c, &g.lots),
            });
            let trees = plant4(site, *c, &g, summit.as_ref());
            if let Some(l) = landmark4(site, *c, &trees, height_at) {
                out.push(l);
            }
            if site.landmark == "coit-tower" {
                lights.extend(loop_lamps(*c, &g.lots, height_at));
            }
            continue;
        }
        let trees = plant(site, *c, &g);
        if let Some(l) = landmark(site, *c, &trees) {
            out.push(l);
        }
    }
    Ok((out, lights))
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

// ---- the fourth pass (`crate::towers4_on`) ----

/// Coit Tower's summit cone: the trees within `SUMMIT_REACH_M` of the axis
/// stay under the base building's roof (`lidarMedianM` over the summit)
/// plus `SUMMIT_SLOPE` a metre out from the keep-out (INFERRED, module doc).
pub const SUMMIT_REACH_M: f64 = 160.0;
pub const SUMMIT_SLOPE: f64 = 0.2;
/// A crown's clearance from a building's walls and from a car park's edge (m, INFERRED).
pub const BUILDING_CLEAR_M: f64 = 2.0;
pub const LOT_CLEAR_M: f64 = 1.0;
/// The wood's edge along a way: within this of the way's edge a tree is kept with these odds (INFERRED).
pub const EDGE_M: f64 = 6.0;
pub const EDGE_KEEP: f64 = 0.55;
/// The litter layer: its grid (m), its lift over the drawn ground (m), its
/// albedo (linear dark brown duff) and the floor of its sky occlusion (INFERRED).
pub const FLOOR_CELL_M: f64 = 3.0;
pub const FLOOR_LIFT_M: f64 = 0.25;
pub const DUFF: Rgb = [0.062, 0.05, 0.036];
pub const FLOOR_OCCLUSION_MIN: f64 = 0.4;
/// The parking loop's luminaires: pitch along its edge, set-out outside it, head height (m, INFERRED).
pub const LOOP_LAMP_PITCH_M: f64 = 22.0;
pub const LOOP_LAMP_OUT_M: f64 = 1.5;
pub const LOOP_LAMP_HEAD_M: f64 = 4.5;

/// The distance from p to a ring's edges, 0 inside it.
pub fn ring_distance(r: &[[f64; 2]], p: [f64; 2]) -> f64 {
    if r.len() >= 3 && inside(r, p[0], p[1]) {
        return 0.0;
    }
    let n = r.len();
    (0..n).map(|i| seg_distance(p, r[i], r[(i + 1) % n])).fold(f64::INFINITY, f64::min)
}

/// What the fourth pass plants a site with besides `Ground`: the summit cone
/// (the summit's y, the cone's foot over it), when the site has one.
pub struct Summit {
    pub y: f64,
    pub lift_m: f64,
    /// The parking circle on the summit: the trees near it under a second
    /// cone from `LOOP_CONE_LIFT_M` over the summit at its edge, so the loop shows.
    pub lot: Option<Vec<[f64; 2]>>,
}

/// The parking loop's cone: its foot over the summit at the loop's edge and its slope (INFERRED, as the tower's).
pub const LOOP_CONE_LIFT_M: f64 = 1.0;
pub const LOOP_CONE_SLOPE: f64 = 0.25;

/// The car park nearest `centre` within 120 m (Coit Tower's parking circle).
pub fn summit_lot(centre: [f64; 2], lots: &[Vec<[f64; 2]>]) -> Option<Vec<[f64; 2]>> {
    lots.iter().map(|l| (ring_distance(l, centre), l)).filter(|(d, _)| *d < 120.0).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, l)| l.clone())
}

/// The fourth pass's trees of a site (`plant` carried further: module doc).
pub fn plant4(site: &Site, centre: [f64; 2], g: &Ground, summit: Option<&Summit>) -> Vec<Tree> {
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
            if !(g.tree_cover)(x, z) {
                continue;
            }
            // how far the trunk stands from the nearest way's edge.
            let way_edge = g.ways.iter().map(|w| w.points.windows(2).map(|q| seg_distance([x, z], q[0], q[1])).fold(f64::INFINITY, f64::min) - w.half_m).fold(f64::INFINITY, f64::min);
            if way_edge < 0.0 {
                continue;
            }
            let Some(y) = (g.height_at)(x, z) else { continue };
            let mut roll = hash(i, j, 3) * total;
            let mut sp = site.mix[0].0;
            for (m, w) in site.mix {
                if roll < *w {
                    sp = *m;
                    break;
                }
                roll -= w;
            }
            let mut height = (sp.height_m[0] + (sp.height_m[1] - sp.height_m[0]) * hash(i, j, 4)) * (0.85 + 0.3 * hash(i, j, 8));
            let width = (sp.width_share[0] + (sp.width_share[1] - sp.width_share[0]) * hash(i, j, 5)) * (0.85 + 0.3 * hash(i, j, 9));
            // the wood's edge: thinned, and shorter toward the way; a gap takes an understory shrub.
            let at_edge = way_edge < EDGE_M;
            let mut shrub = false;
            if at_edge {
                if hash(i, j, 10) > EDGE_KEEP {
                    shrub = true;
                } else {
                    height *= 0.75 + 0.25 * way_edge / EDGE_M;
                }
            }
            if shrub {
                sp = UNDERSTORY;
                height = sp.height_m[0] + (sp.height_m[1] - sp.height_m[0]) * hash(i, j, 11);
            }
            let w_share = if shrub { sp.width_share[0] + (sp.width_share[1] - sp.width_share[0]) * hash(i, j, 12) } else { width };
            // Coit's summit cone.
            if let Some(sm) = summit {
                if r < SUMMIT_REACH_M {
                    let mut cap = sm.y + sm.lift_m + (r - site.keep_out_m).max(0.0) * SUMMIT_SLOPE - y;
                    if let Some(lot) = &sm.lot {
                        cap = cap.min(sm.y + LOOP_CONE_LIFT_M + ring_distance(lot, [x, z]) * LOOP_CONE_SLOPE - y);
                    }
                    if cap < UNDERSTORY.height_m[0] {
                        continue;
                    }
                    if height > cap {
                        // a tree held under the cone is a shorter one of its kind, or the scrub.
                        if cap < sp.height_m[0] * 0.6 {
                            sp = UNDERSTORY;
                        }
                        height = cap;
                    }
                }
            }
            let radius = height * w_share / 2.0;
            // the whole crown clear of the car parks and, with 2 m more, of the buildings.
            if g.lots.iter().any(|l| ring_distance(l, [x, z]) < radius * 0.8 + LOT_CLEAR_M) {
                continue;
            }
            let reach = radius + BUILDING_CLEAR_M;
            if g.buildings.iter().any(|(rg, b)| x + reach >= b[0] && x - reach <= b[2] && z + reach >= b[1] && z - reach <= b[3] && ring_distance(rg, [x, z]) < reach) {
                continue;
            }
            out.push(Tree { at: [x, y - 0.1, z], height, radius, species: sp, seed: (hash(i, j, 6) * 1.0e9) as u32 });
        }
    }
    out
}

/// A fourth-pass crown: `crown`'s displaced ellipsoids, laid out by the
/// species' form, the tree's foliage hue and value jittered (up to 8 %).
pub fn crown4(m: &mut MeshArrays, t: &Tree, sphere: &(Vec<V3>, Vec<[u32; 3]>), lumps: usize) {
    let s = &t.species;
    let foot = t.height * s.crown_foot4;
    let ch = t.height - foot;
    let centre = [t.at[0], t.at[1] + foot + ch * 0.5, t.at[2]];
    let (rx, ry) = (t.radius, (ch * 0.5).max(1.0));
    let h = |k: i64, salt: u32| hash(t.seed as i64, k, salt);
    // value +/-8 %, a hue swing between yellow-green and blue-green of +/-8 %.
    let value = 0.92 + 0.16 * h(1, 41);
    let hue = 0.92 + 0.16 * h(2, 43);
    let base: Rgb = [s.foliage[0] * value * hue, s.foliage[1] * value, s.foliage[2] * value / hue];
    let n = lumps.max(1);
    for k in 0..n {
        let kk = k as i64;
        let a = TAU * (k as f64 / n as f64) + h(kk, 47) * 1.7;
        // each form's lump: centre offset (x, y, z), horizontal and vertical radii.
        let (lc, lr, ly) = match s.form {
            FORM_FLAT => {
                // spreading: the lumps round the crown's top, flattened; one low in the middle.
                if k == 0 {
                    ([centre[0], centre[1] - ry * 0.15, centre[2]], rx * 0.62, ry * 0.65)
                } else {
                    let d = rx * (0.42 + 0.22 * h(kk, 53));
                    ([centre[0] + a.cos() * d, centre[1] + ry * (0.1 + 0.3 * h(kk, 59)), centre[2] + a.sin() * d], rx * (0.48 + 0.12 * h(kk, 61)), ry * (0.42 + 0.12 * h(kk, 67)))
                }
            }
            FORM_RAGGED => {
                // strung up the stem: each lump at its own height, off the axis to its own side.
                let f = (k as f64 + 0.5) / n as f64;
                let d = rx * (0.15 + 0.55 * h(kk, 53));
                let yy = centre[1] - ry + 2.0 * ry * (f + 0.12 * (h(kk, 59) - 0.5));
                ([centre[0] + a.cos() * d, yy, centre[2] + a.sin() * d], rx * (0.42 + 0.3 * h(kk, 61)) * (1.15 - 0.4 * f), ry * (0.28 + 0.12 * h(kk, 67)))
            }
            FORM_SHRUB => {
                let d = rx * (0.3 + 0.3 * h(kk, 53));
                ([centre[0] + a.cos() * d, centre[1] + ry * (-0.2 + 0.3 * h(kk, 59)), centre[2] + a.sin() * d], rx * (0.55 + 0.15 * h(kk, 61)), ry * (0.7 + 0.2 * h(kk, 67)))
            }
            _ => {
                if k == 0 {
                    ([centre[0], centre[1] + ry * 0.1, centre[2]], rx * 0.78, ry * 0.8)
                } else {
                    let d = rx * (0.38 + 0.18 * h(kk, 53));
                    ([centre[0] + a.cos() * d, centre[1] + ry * (-0.55 + 0.8 * h(kk, 59)), centre[2] + a.sin() * d], rx * (0.5 + 0.12 * h(kk, 61)), ry * (0.55 + 0.15 * h(kk, 67)))
                }
            }
        };
        let first = m.vertex_count();
        for d in &sphere.0 {
            let bump = 1.0 + 0.2 * noise3([d[0] * 2.3, d[1] * 2.3, d[2] * 2.3], t.seed.wrapping_add(k as u32 * 101)) + 0.08 * noise3([d[0] * 5.1, d[1] * 5.1, d[2] * 5.1], t.seed.wrapping_add(7));
            let p = [lc[0] + d[0] * lr * bump, lc[1] + d[1] * ly * bump, lc[2] + d[2] * lr * bump];
            let q = [(p[0] - centre[0]) / (rx * rx), (p[1] - centre[1]) / (ry * ry), (p[2] - centre[2]) / (rx * rx)];
            let own = norm([d[0] / lr, d[1] / ly, d[2] / lr]);
            let whole = norm(q);
            let w = if s.form == FORM_RAGGED { 0.35 } else { 0.6 };
            let nn = norm([whole[0] * w + own[0] * (1.0 - w), whole[1] * w + own[1] * (1.0 - w), whole[2] * w + own[2] * (1.0 - w)]);
            let rel = ((p[1] - (centre[1] - ry)) / (2.0 * ry)).clamp(0.0, 1.0);
            let out = ((p[0] - centre[0]) * own[0] + (p[2] - centre[2]) * own[2]) / rx.max(0.1);
            let occ = (0.55 + 0.45 * rel.powf(0.8)) * (0.8 + 0.2 * out.clamp(0.0, 1.0));
            let cluster = 0.88 + 0.24 * hash((p[0] * 1.7) as i64, (p[2] * 1.7 + p[1] * 3.1) as i64, 71);
            vertex(m, p, nn, base.map(|c| c * occ * cluster));
        }
        for f in &sphere.1 {
            m.indices.extend_from_slice(&[first + f[0], first + f[1], first + f[2]]);
        }
    }
}

/// A fourth-pass trunk: six-sided, a diameter at its foot of 1/40 of the
/// tree's height (INFERRED), tapering to 0.55 of it inside the crown, in its species' bark.
pub fn trunk4(m: &mut MeshArrays, t: &Tree) {
    let top = t.height * (t.species.crown_foot4 + 0.2);
    let r0 = (t.height / 80.0).clamp(0.08, 0.6);
    let r1 = r0 * 0.55;
    let yaw = hash(t.seed as i64, 3, 73) * PI;
    let ring = |r: f64, y: f64| -> Vec<V3> { (0..6).map(|i| yaw - i as f64 * PI / 3.0).map(|a| [t.at[0] + a.cos() * r, t.at[1] + y, t.at[2] + a.sin() * r]).collect() };
    let (b, k) = (ring(r0, -0.4), ring(r1, top));
    let c = t.species.bark.map(|v| v * (0.9 + 0.2 * hash(t.seed as i64, 4, 79)));
    for i in 0..6 {
        let j = (i + 1) % 6;
        m.quad(b[i], b[j], k[j], k[i], c);
    }
}

/// The litter layer under a grove's crowns: a grid on the drawn ground,
/// each vertex's tone the duff darkened by the overhead sky the crowns
/// cover there (each crown's disc over the vertex weighted by how near
/// overhead it stands, the sum clamped), only where a crown covers it.
pub fn forest_floor(trees: &[Tree], name: &str, height_at: &dyn Fn(f64, f64) -> Option<f64>) -> MeshArrays {
    let mut m = MeshArrays::new(name);
    if trees.is_empty() {
        return m;
    }
    let c = FLOOR_CELL_M;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for t in trees {
        lo = [lo[0].min(t.at[0] - t.radius * 1.3), lo[1].min(t.at[2] - t.radius * 1.3)];
        hi = [hi[0].max(t.at[0] + t.radius * 1.3), hi[1].max(t.at[2] + t.radius * 1.3)];
    }
    let (i0, j0) = ((lo[0] / c).floor() as i64, (lo[1] / c).floor() as i64);
    let (ni, nj) = (((hi[0] / c).ceil() as i64 - i0 + 1) as usize, ((hi[1] / c).ceil() as i64 - j0 + 1) as usize);
    // the trees by bucket (a cell of 16 m), for the coverage sums.
    let bucket = 16.0;
    let mut buckets: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (k, t) in trees.iter().enumerate() {
        let reach = t.radius * 1.3;
        for bj in ((t.at[2] - reach) / bucket).floor() as i64..=((t.at[2] + reach) / bucket).floor() as i64 {
            for bi in ((t.at[0] - reach) / bucket).floor() as i64..=((t.at[0] + reach) / bucket).floor() as i64 {
                buckets.entry((bi, bj)).or_default().push(k);
            }
        }
    }
    // per grid vertex: its index (or none) and its coverage.
    let mut idx = vec![u32::MAX; ni * nj];
    for jj in 0..nj {
        for ii in 0..ni {
            let (x, z) = ((i0 + ii as i64) as f64 * c, (j0 + jj as i64) as f64 * c);
            let Some(list) = buckets.get(&((x / bucket).floor() as i64, (z / bucket).floor() as i64)) else { continue };
            let mut cover = 0.0;
            for &k in list {
                let t = &trees[k];
                let d = (x - t.at[0]).hypot(z - t.at[2]);
                let r = t.radius * 1.15;
                if d < r {
                    // the crown's share of the overhead sky: its disc's solid angle seen from the ground, about (r / (r + h))^2 near overhead.
                    let lift = t.height * (t.species.crown_foot4 + 0.5);
                    let k2 = (r / (r + lift * 0.5)).powi(2) * (1.0 - (d / r).powi(2)).sqrt();
                    cover += k2;
                }
            }
            if cover <= 0.0 {
                continue;
            }
            let Some(y) = height_at(x, z) else { continue };
            let vis = (1.0 - cover.min(1.0) * (1.0 - FLOOR_OCCLUSION_MIN)).max(FLOOR_OCCLUSION_MIN);
            let tone = 0.85 + 0.3 * hash(ii as i64 + i0, jj as i64 + j0, 83);
            idx[jj * ni + ii] = m.vertex_count();
            vertex(&mut m, [x, y + FLOOR_LIFT_M, z], [0.0, 1.0, 0.0], DUFF.map(|v| v * vis * tone));
        }
    }
    for jj in 0..nj.saturating_sub(1) {
        for ii in 0..ni.saturating_sub(1) {
            let (a, b, cc, d) = (idx[jj * ni + ii], idx[jj * ni + ii + 1], idx[(jj + 1) * ni + ii + 1], idx[(jj + 1) * ni + ii]);
            // up-facing: (x, z) counter-clockwise seen from above is a, d, c then a, c, b.
            if a != u32::MAX && cc != u32::MAX {
                if d != u32::MAX {
                    m.indices.extend_from_slice(&[a, d, cc]);
                }
                if b != u32::MAX {
                    m.indices.extend_from_slice(&[a, cc, b]);
                }
            }
        }
    }
    // the normals from the drawn ground's slope.
    let n = m.vertex_count() as usize;
    let mut acc = vec![[0.0f64; 3]; n];
    for t in m.indices.chunks_exact(3) {
        let p = |i: u32| [m.positions[3 * i as usize], m.positions[3 * i as usize + 1], m.positions[3 * i as usize + 2]];
        let (a, b, cc) = (p(t[0]), p(t[1]), p(t[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [cc[0] - a[0], cc[1] - a[1], cc[2] - a[2]];
        let nn = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        for &i in t {
            for k in 0..3 {
                acc[i as usize][k] += nn[k];
            }
        }
    }
    for (i, a) in acc.iter().enumerate() {
        let nn = if a[1] > 0.0 { norm(*a) } else { [0.0, 1.0, 0.0] };
        m.normals[3 * i..3 * i + 3].copy_from_slice(&nn);
    }
    m
}

/// The fourth pass's grove landmark: `landmark` with the fourth pass's
/// crowns and trunks, and the litter layer.
pub fn landmark4(site: &Site, centre: [f64; 2], trees: &[Tree], height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Option<Landmark> {
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
        let lumps = if t.species.form == FORM_RAGGED { t.species.lumps + 1 } else { t.species.lumps };
        crown4(&mut crowns, t, &near_sphere, lumps);
        trunk4(&mut trunks, t);
        crown4(&mut far, t, &far_sphere, 1);
    }
    let floor = forest_floor(trees, &format!("{}:floor", site.id), height_at);
    let leaf = plain_material(0.85, 0.0, "facade-masonry", None);
    let bark = plain_material(0.95, 0.0, "facade-masonry", None);
    let duff = plain_material(0.95, 0.0, "facade-masonry", None);
    let mut draws = vec![
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
    if !floor.indices.is_empty() {
        // near and far alike (Role::Always): the floor is the canopy's shade at any range.
        draws.push(Draw { key: "floor".into(), role: Role::Always, material: duff, mesh: floor.freeze() });
    }
    draws.push(Draw { key: "far".into(), role: Role::Far, material: leaf, mesh: far.freeze() });
    let mut species = std::collections::BTreeMap::new();
    for t in trees {
        *species.entry(t.species.name.to_string()).or_insert(0usize) += 1;
    }
    Some(Landmark {
        id: site.id.into(),
        name: site.name.into(),
        version: "grove-v2".into(),
        anchor: centre,
        box_min: mn,
        box_max: mx,
        draws,
        colliders: vec![],
        stats: json!({ "trees": trees.len(), "species": species, "landmark": site.landmark }),
        build_ms: t0.elapsed().as_secs_f64() * 1e3,
    })
}

/// The parking loop's luminaires round Coit Tower: along the edge of the
/// car park nearest the tower (within 120 m), every `LOOP_LAMP_PITCH_M`,
/// `LOOP_LAMP_OUT_M` outside it, the street slice's pool photometry from a
/// `LOOP_LAMP_HEAD_M` head.
pub fn loop_lamps(centre: [f64; 2], lots: &[Vec<[f64; 2]>], height_at: &dyn Fn(f64, f64) -> Option<f64>) -> Vec<LandmarkLight> {
    let Some(lot) = summit_lot(centre, lots) else {
        return vec![];
    };
    let lot = &lot;
    let rc = crate::geometry::ring_centroid(lot);
    let n = lot.len();
    let mut out = Vec::new();
    let mut carry = LOOP_LAMP_PITCH_M / 2.0;
    for i in 0..n {
        let (a, b) = (lot[i], lot[(i + 1) % n]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let mut s = carry;
        while s < len {
            let p = [a[0] + (b[0] - a[0]) * s / len, a[1] + (b[1] - a[1]) * s / len];
            let (dx, dz) = (p[0] - rc[0], p[1] - rc[1]);
            let dd = dx.hypot(dz).max(1e-6);
            let q = [p[0] + dx / dd * LOOP_LAMP_OUT_M, p[1] + dz / dd * LOOP_LAMP_OUT_M];
            if let Some(y) = height_at(q[0], q[1]) {
                out.push(crate::lights::pool_lamp(q, y, LOOP_LAMP_HEAD_M));
            }
            s += LOOP_LAMP_PITCH_M;
        }
        carry = s - len;
    }
    out
}
