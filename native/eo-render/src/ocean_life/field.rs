//! Stable world-space populations. The visible window moves, the organisms do not reseed.
use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use std::sync::Arc;

pub const CORE_KINDS: usize = 14;
pub const KINDS: [&str; 21] = [
    "perch",
    "topsmelt",
    "sculpin",
    "eelgrass",
    "sea-lettuce",
    "red-algae",
    "rockweed",
    "stone-ledge",
    "stone-block",
    "stone-cobble",
    "shell",
    "bull-kelp",
    "sanddab",
    "sea-pen",
    "reef-block",
    "reef-outcrop",
    "cup-sponge",
    "tube-sponge",
    "branching-colony",
    "kelp-column",
    "baitfish",
];
pub const TILE: f32 = 6.0;
pub const RING: i32 = 2;
pub const SEA: f32 = eo_water::SEA_LEVEL_Y as f32;
pub type Sampler = Arc<dyn Fn(Vec2) -> Option<Site> + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub floor: f32,
    pub shore: f32,
    pub beach: f32,
    pub pacific: f32,
    pub offshore: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct Instance {
    pub at: [f32; 4],     // x,y,z,heading
    pub shape: [f32; 4],  // x,y,z scale and motion phase
    pub traits: [f32; 4], // mesh kind, school phase, travel radius, pigment variation
}

fn hash(x: i32, z: i32) -> u32 {
    let mut n = (x as u32).wrapping_mul(0x9e37_79b1) ^ (z as u32).wrapping_mul(0x85eb_ca77);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846c_a68b);
    n ^ (n >> 16)
}
struct Rng(u32);
impl Rng {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 8) as f32 / 16777216.
    }
    fn between(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }
    fn offset(&mut self, r: f32) -> Vec2 {
        Vec2::new(self.between(-r, r), self.between(-r, r))
    }
}
fn patch(p: Vec2) -> f32 {
    let q = p / 23.;
    let c = q.floor().as_ivec2();
    let u = q - q.floor();
    let t = u * u * (3. - 2. * u);
    let v = |x, z| (hash(x, z) >> 8) as f32 / 16777216.;
    let south = v(c.x, c.y) * (1. - t.x) + v(c.x + 1, c.y) * t.x;
    let north = v(c.x, c.y + 1) * (1. - t.x) + v(c.x + 1, c.y + 1) * t.x;
    south * (1. - t.y) + north * t.y
}
fn put(
    out: &mut [Vec<Instance>; KINDS.len()],
    kind: usize,
    p: Vec2,
    y: f32,
    scale: [f32; 3],
    heading: f32,
    motion: [f32; 3],
    rng: &mut Rng,
) {
    out[kind].push(Instance {
        at: [p.x, y, p.y, heading],
        shape: [scale[0], scale[1], scale[2], motion[0]],
        traits: [kind as f32, motion[1], motion[2], rng.between(0.80, 1.18)],
    });
}
fn seabed(
    out: &mut [Vec<Instance>; KINDS.len()],
    p: Vec2,
    site: Site,
    rng: &mut Rng,
    sample: &Sampler,
) {
    let d = SEA - site.floor;
    let slope = sample(p + Vec2::new(0.9, 0.35)).map_or(0., |s| (s.floor - site.floor).abs());
    let zonation = patch(p);
    let rocky = site.beach < 0.45
        && (slope > 0.13 || zonation > 0.53)
        && (site.shore < 210. || slope > 0.15);
    let density = if rocky {
        8
    } else if zonation > 0.57 {
        5
    } else {
        2
    };
    for _ in 0..density {
        let q = p + rng.offset(TILE * 0.43);
        let Some(b) = sample(q).filter(|b| b.shore > 0.20 && b.floor < SEA - 0.17) else {
            continue;
        };
        let depth = SEA - b.floor;
        let phase = rng.unit() * 6.28;
        let yaw = rng.unit() * std::f32::consts::TAU;
        let s = if rocky {
            rng.between(0.09, 0.68)
        } else {
            rng.between(0.024, 0.14)
        };
        let kind = if rocky {
            7 + (rng.unit() * 3.) as usize
        } else {
            9
        };
        let sy = s * rng.between(0.37, 0.76);
        let sz = s * rng.between(0.51, 0.89);
        put(
            out,
            kind,
            q,
            b.floor + sy * 0.10,
            [s, sy, sz],
            yaw,
            [0.; 3],
            rng,
        );
        if rocky && depth < 16. && rng.unit() < 0.78 {
            let plant = if site.pacific > 0.60
                && (3.5..15.).contains(&depth)
                && zonation > 0.62
                && rng.unit() < 0.55
            {
                11
            } else if depth < 2.5 && rng.unit() < 0.46 {
                6
            } else if rng.unit() < 0.52 {
                4
            } else {
                5
            };
            let h = if plant == 11 {
                rng.between(1.9, 9.).min(depth * 0.80)
            } else {
                rng.between(0.09, 0.34).min(depth * 0.45)
            };
            // Root attaches to the crown of the same irregular stone, slightly inside the mesh.
            put(
                out,
                plant,
                q,
                b.floor + sy * 0.81,
                [h, h, h],
                yaw,
                [phase, 0., 0.],
                rng,
            );
        } else if !rocky
            && site.pacific < 0.48
            && (0.46..4.2).contains(&depth)
            && zonation > 0.30
            && site.shore < 320.
        {
            for _ in 0..4 {
                let root = q + rng.offset(0.37);
                if let Some(ground) = sample(root).filter(|g| g.shore > 0.2 && SEA - g.floor > 0.36)
                {
                    let h = rng.between(0.24, 0.68).min((SEA - ground.floor) * 0.72);
                    let angle = rng.between(0., 6.28);
                    put(
                        out,
                        3,
                        root,
                        ground.floor - 0.009,
                        [h, h, h],
                        angle,
                        [phase, 0., 0.],
                        rng,
                    );
                }
            }
        }
        if depth < 55. && rng.unit() < 0.62 {
            let shell = q + rng.offset(0.39);
            if let Some(g) = sample(shell).filter(|s| s.shore > 0.05 && s.floor < SEA - 0.15) {
                let radius = rng.between(0.014, 0.055);
                put(
                    out,
                    10,
                    shell,
                    g.floor,
                    [radius; 3],
                    yaw,
                    [phase, 0., 0.],
                    rng,
                );
            }
        }
    }
    if !rocky && (6.0..250.0).contains(&d) && rng.unit() < 0.31 {
        let l = rng.between(0.16, 0.33);
        let yaw = rng.between(0., std::f32::consts::TAU);
        put(
            out,
            12,
            p,
            site.floor + 0.015,
            [l; 3],
            yaw,
            [rng.between(0., 6.28), 0., 0.],
            rng,
        );
    }
    if !rocky && site.pacific > 0.5 && d > 18. && zonation > 0.48 && rng.unit() < 0.35 {
        let h = rng.between(0.12, 0.41);
        let yaw = rng.between(0., std::f32::consts::TAU);
        put(
            out,
            13,
            p,
            site.floor - 0.014,
            [h; 3],
            yaw,
            [rng.between(0., 6.28), 0., 0.],
            rng,
        );
    }
    if (0.7..42.).contains(&d) && rng.unit() < 0.37 {
        let l = rng.between(0.12, 0.24);
        let yaw = rng.between(0., 6.28);
        put(
            out,
            2,
            p,
            site.floor + l * 0.12,
            [l; 3],
            yaw,
            [rng.between(0., 6.28), 0., 0.],
            rng,
        );
    }
}
pub fn populate(x: i32, z: i32, sample: &Sampler, out: &mut [Vec<Instance>; KINDS.len()]) {
    let mut rng = Rng(hash(x, z));
    let centre = (Vec2::new(x as f32, z as f32) + 0.5) * TILE;
    let p = centre + rng.offset(TILE * 0.20);
    let Some(site) = sample(p).filter(|b| b.shore > 0.15 && b.floor < SEA - 0.18) else {
        return;
    };
    seabed(out, p, site, &mut rng, sample);
    let weight = rng.unit();
    if weight > 0.71 || site.shore < 1.5 {
        return;
    }
    let kind = usize::from(site.pacific > 0.6 || site.shore > 190. || weight > 0.32);
    let span = if kind == 0 { 0.86 } else { 1.28 };
    // Reserve room for the whole orbit and group at the steepest edge.
    let mut bed = site.floor;
    for offset in [
        Vec2::X,
        Vec2::NEG_X,
        Vec2::Y,
        Vec2::NEG_Y,
        Vec2::ONE,
        Vec2::NEG_ONE,
        Vec2::new(1., -1.),
        Vec2::new(-1., 1.),
    ] {
        let Some(n) = sample(p + offset * span * 1.7).filter(|v| v.shore > 0.35) else {
            return;
        };
        bed = bed.max(n.floor);
    }
    let depth = SEA - bed;
    if depth < 0.78 {
        return;
    }
    let phase = rng.between(0., 6.28);
    let heading = rng.between(0., 6.28);
    let y = if kind == 0 {
        bed.max(SEA - 6.) + rng.between(0.31, 1.15).min(depth * 0.42)
    } else {
        SEA - rng.between(0.55, 4.2).min(depth * 0.47)
    };
    for _ in 0..if kind == 0 {
        3 + (rng.unit() * 4.) as usize
    } else {
        6 + (rng.unit() * 8.) as usize
    } {
        let q = p + rng.offset(span * 0.65);
        let l = rng.between(0.115, 0.205);
        let height = (y + rng.between(-0.16, 0.16)).clamp(bed + 0.19, SEA - 0.32);
        let a = heading + rng.between(-0.14, 0.14);
        let motion = [rng.between(0., 6.28), phase, span * rng.between(0.74, 0.93)];
        put(out, kind, q, height, [l; 3], a, motion, &mut rng);
    }
}

/// A second stream leaves the Blender populations stable on both sides of the switch.
pub fn reef(x: i32, z: i32, sample: &Sampler, out: &mut [Vec<Instance>; KINDS.len()]) {
    let mut rng = Rng(hash(x, z) ^ 0xe41859a1);
    let centre = (Vec2::new(x as f32, z as f32) + 0.5) * TILE;
    let p = centre + rng.offset(1.6);
    let Some(site) = sample(p).filter(|s| s.shore > 1.5 && s.floor < SEA - 0.9) else {
        return;
    };
    let d = SEA - site.floor;
    let slope = |delta| sample(p + delta).map_or(0., |s| (s.floor - site.floor).abs());
    let relief = slope(Vec2::new(2.4, 0.)).max(slope(Vec2::new(0., 2.4)));
    let patch = patch(p * 0.76);
    let exposed = site.pacific > 0.55;
    // Rocky, wave-exposed headland. The sand across Ocean Beach stays open.
    let rocky = site.beach < 0.42 && (relief > 0.20 || (site.shore < 120. && patch > 0.50));
    if rocky && rng.unit() < 0.48 + 0.44 * patch {
        let scale = rng.between(0.52, 1.46) * (0.7 + patch);
        let h = scale * rng.between(0.54, 0.91);
        let width = scale * rng.between(0.67, 1.24);
        let yaw = rng.between(0., std::f32::consts::TAU);
        let shape = if rng.unit() < 0.52 { 14 } else { 15 };
        put(
            out,
            shape,
            p,
            site.floor + h * 0.10,
            [scale, h, width],
            yaw,
            [0.; 3],
            &mut rng,
        );
        // Vertical ray at the two baked chunks' local origins. Sink holdfasts into the crown.
        let crown = site.floor + h * (0.10 + if shape == 14 { 0.83528 } else { 0.79038 });
        let remaining = SEA - crown;
        if exposed && (2.5..19.).contains(&remaining) && rng.unit() < 0.65 + 0.24 * patch {
            let height = remaining * rng.between(0.51, 0.87);
            let spread = height * rng.between(0.63, 1.05);
            put(
                out,
                19,
                p,
                crown - 0.06,
                [spread, height, spread],
                yaw,
                [rng.between(0., 6.28), 0., 0.],
                &mut rng,
            );
        } else if remaining > 0.5 && rng.unit() < 0.83 {
            let kind = 16 + (rng.unit() * 3.) as usize;
            let h = rng.between(0.18, 0.53);
            put(
                out,
                kind,
                p,
                crown - 0.035,
                [h; 3],
                yaw,
                [rng.between(0., 6.28), 0., 0.],
                &mut rng,
            );
        }
    }
    if !exposed || d < 3. || rng.unit() > 0.25 || patch < 0.34 {
        return;
    }
    let heading = rng.between(0., std::f32::consts::TAU);
    let phase = rng.between(0., std::f32::consts::TAU);
    let clearance = (site.floor + 1.2).max(SEA - 8.);
    if clearance > SEA - 0.8 {
        return;
    }
    let y = rng.between(clearance, SEA - 0.8);
    for _ in 0..8 + (rng.unit() * 11.) as usize {
        let q = p + rng.offset(1.);
        if !sample(q).is_some_and(|s| s.shore > 1.5 && s.floor < y - 0.65) {
            continue;
        }
        let l = rng.between(0.095, 0.16);
        let angle = heading + rng.between(-0.10, 0.10);
        let motion = [rng.between(0., 6.28), phase, 0.61];
        put(
            out,
            20,
            q,
            y + rng.between(-0.19, 0.19),
            [l, l * 0.7, l],
            angle,
            motion,
            &mut rng,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(floor: f32, shore: f32, pacific: f32) -> Sampler {
        Arc::new(move |_| {
            Some(Site {
                floor,
                shore,
                beach: 0.,
                pacific,
                offshore: false,
            })
        })
    }

    fn census(x: i32, z: i32, sample: &Sampler) -> [Vec<Instance>; KINDS.len()] {
        let mut life = Default::default();
        populate(x, z, sample, &mut life);
        life
    }

    #[test]
    fn return_to_the_same_tile_restores_its_population() {
        let bay = samples(SEA - 3., 28., 0.);
        let a = census(-27, 51, &bay);
        assert_eq!(a, census(-27, 51, &bay));
        assert_ne!(a, census(-26, 51, &bay));
        assert!(a.iter().any(|group| !group.is_empty()));
        assert!(
            census(0, 0, &samples(SEA + 3., -12., 0.))
                .iter()
                .all(Vec::is_empty)
        );
    }

    #[test]
    fn reef_life_respects_substrate_depth_and_the_original_population() {
        let mut observed = [false; KINDS.len()];
        for (floor, beach, shore, pacific, expect_reef) in [
            (SEA - 7., 0., 24., 1., true),
            (SEA - 7., 1., 24., 1., false),
            (SEA - 48., 0., 24., 1., true),
            (SEA + 1., 0., -5., 0., false),
        ] {
            let s: Sampler = Arc::new(move |_| {
                Some(Site {
                    floor,
                    beach,
                    shore,
                    pacific,
                    offshore: false,
                })
            });
            for x in -5..5 {
                for z in -5..5 {
                    let core = census(x, z, &s);
                    let mut a = core.clone();
                    reef(x, z, &s, &mut a);
                    let mut b = core.clone();
                    reef(x, z, &s, &mut b);
                    assert_eq!(a, b);
                    assert_eq!(core[..CORE_KINDS], a[..CORE_KINDS]);
                    if !expect_reef {
                        assert!(a[14..20].iter().all(Vec::is_empty));
                    }
                    if floor < SEA - 25. {
                        assert!(a[19].is_empty());
                    }
                    if shore < 0. {
                        assert!(a[20].is_empty());
                    }
                    for (k, g) in a.iter().enumerate().skip(CORE_KINDS) {
                        observed[k] |= !g.is_empty();
                        for v in g {
                            assert!(v.at[1] > floor && v.at[1] < SEA);
                        }
                    }
                }
            }
        }
        assert!(observed[CORE_KINDS..].iter().all(|&found| found));
    }

    #[test]
    fn shelf_floor_supports_benthic_animals_below_the_photic_zone() {
        let shelf = samples(SEA - 48., 400., 1.);
        let mut species = [false; KINDS.len()];
        for z in -6..6 {
            for x in -6..6 {
                let tile = census(x, z, &shelf);
                for (i, group) in tile.iter().enumerate() {
                    species[i] |= !group.is_empty();
                }
                for fish in tile[1].iter() {
                    assert!(fish.at[1] < SEA && fish.at[1] > SEA - 6.);
                }
                for fish in tile[12].iter() {
                    assert!((fish.at[1] - (SEA - 48.)).abs() < 0.05);
                }
            }
        }
        assert!(species[12] && species[13]);
        assert!(!species[3..7].contains(&true) && !species[11]);
    }
}
