//! Stable world-space populations. The visible window moves, the organisms do not reseed.
use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use std::sync::Arc;

pub const CORE_KINDS: usize = 14;
pub const CROWN_KINDS: [usize; 4] = [11, 21, 22, 23];
pub const LEDGE: usize = 24;
pub const BOULDER: usize = 25;
pub const KINDS: [&str; 26] = [
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
    "streaming-crown",
    "open-crown",
    "clustered-crown",
    "bedrock-ledge",
    "fractured-boulder",
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

/// Follow established eelgrass patches out along the sediment. Sample every
/// new shoot location so the roots stay on the bed as the bottom slopes.
pub fn coastal_grass(x: i32, z: i32, sample: &Sampler, shoots: &mut Vec<Instance>) {
    let mut rng = Rng(hash(x, z) ^ 0xb5ea_6a51);
    let founders = shoots.len();
    for i in 0..founders {
        let parent = shoots[i];
        let axis = Vec2::new(parent.at[3].cos(), parent.at[3].sin());
        let across = Vec2::new(-axis.y, axis.x);
        for _ in 0..3 {
            let p = Vec2::new(parent.at[0], parent.at[2])
                + axis * rng.between(-0.95, 0.95)
                + across * rng.between(-0.31, 0.31);
            let Some(site) = sample(p) else {
                continue;
            };
            let depth = SEA - site.floor;
            if site.shore < 0.35
                || site.pacific > 0.48
                || !(0.46..4.2).contains(&depth)
                || (site.floor - parent.at[1]).abs() > 0.23
            {
                continue;
            }
            let h = (parent.shape[1] * rng.between(0.47, 1.18)).min(depth * 0.72);
            let angle = parent.at[3] + rng.between(-0.52, 0.52);
            let phase = rng.between(0., std::f32::consts::TAU);
            let mut shoot = parent;
            shoot.at = [p.x, site.floor - 0.009, p.y, angle];
            shoot.shape = [h, h, h, phase];
            shoots.push(shoot);
        }
    }
}

/// Mature bull kelp holds its crown near the surface. Some holdfast sites
/// also retain a shorter, blade-bearing understory in the shelter below.
pub fn coastal_canopy(groups: &mut [Vec<Instance>; KINDS.len()]) {
    let (core, reef) = groups.split_at_mut(19);
    reef[0].retain_mut(|plant| {
        let column = SEA - plant.at[1];
        let seed = plant.shape[3];
        let variation = 0.5 + 0.5 * seed.sin();
        let gap = 0.46 + 0.44 * variation;
        let h = (column - gap).max(plant.shape[1]) / 1.035;
        let mut crown = *plant;
        crown.traits[0] = 11.;
        let spread = h * (0.78 + 0.22 * variation);
        crown.shape = [spread, h, spread, seed];
        core[11].push(crown);
        if variation < 0.37 {
            return false;
        }
        plant.shape[1] *= 0.67 + 0.19 * variation;
        plant.shape[0] *= 0.74;
        plant.shape[2] *= 0.74;
        plant.at[0] += 0.075 * plant.at[3].cos();
        plant.at[2] += 0.075 * plant.at[3].sin();
        true
    });
}

/// Keep a holdfast in place while varying the number, age and spread of its stipes.
pub fn vary_crowns(x: i32, z: i32, groups: &mut [Vec<Instance>; KINDS.len()]) {
    let mut rng = Rng(hash(x, z) ^ 0xa13e_5d7c);
    let plants = std::mem::take(&mut groups[11]);
    for mut plant in plants {
        let kind = CROWN_KINDS[(rng.unit() * CROWN_KINDS.len() as f32) as usize];
        // Young or storm-abraded individuals form a broken subcanopy.
        let height = if kind == 22 {
            rng.between(0.48, 0.79)
        } else {
            rng.between(0.83, 1.0)
        };
        plant.shape[1] *= height;
        plant.shape[0] *= height * rng.between(0.86, 1.21);
        plant.shape[2] *= height * rng.between(0.77, 1.14);
        groups[kind].push(plant); // All four meshes share the bull-kelp shader.
    }
}
/// Extend existing reef rock into low overlapping ledges. Smaller fragments
/// sit in the sediment at the foot of each outcrop, with gaps between outcrops.
pub fn rock_aprons(x: i32, z: i32, sample: &Sampler, groups: &mut [Vec<Instance>; KINDS.len()]) {
    let mut rng = Rng(hash(x, z) ^ 0x56d4_bf09);
    let (anchors, stones) = groups.split_at_mut(LEDGE);
    for rock in anchors[14].iter().chain(&anchors[15]) {
        let centre = Vec2::new(rock.at[0], rock.at[2]);
        let axis = Vec2::new(rock.at[3].cos(), rock.at[3].sin());
        let cross = Vec2::new(-axis.y, axis.x);
        let mass = rock.shape[0];
        let count = 6 + (rng.unit() * 9.) as usize;
        for j in 0..count {
            let ledge = j < 2;
            let sign = if j % 2 == 0 { 1. } else { -1. };
            let p = centre
                + mass * (sign * rng.between(0.65, 1.85) * axis + rng.between(-0.86, 0.86) * cross);
            let Some(site) = sample(p).filter(|s| {
                s.pacific > 0.55 && s.beach < 0.45 && s.shore > 1.5 && s.floor < SEA - 1.2
            }) else {
                continue;
            };
            let radius = mass
                * if ledge {
                    rng.between(0.78, 1.46)
                } else {
                    rng.between(0.13, 0.62)
                };
            let height = radius * rng.between(0.40, 1.04);
            let yaw = rock.at[3]
                + rng.between(-0.34, 0.34)
                + if ledge { 0. } else { rng.between(-1.8, 1.8) };
            let buried = height * rng.between(-0.12, 0.16);
            let id = if ledge { LEDGE } else { BOULDER };
            stones[id - LEDGE].push(Instance {
                at: [p.x, site.floor + buried, p.y, yaw],
                shape: [
                    radius,
                    height,
                    radius * rng.between(0.68, 1.19),
                    rng.between(0., 6.28),
                ],
                traits: [id as f32, 0., 0., rng.between(0.75, 1.25)],
            });
        }
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
    fn reef_rock_and_kelp_forms_follow_world_sites() {
        let sample = samples(SEA - 8., 18., 0.86);
        let mut forms = [0; 4];
        let mut stones = 0;
        for x in -8..8 {
            for z in -8..8 {
                let mut groups = Default::default();
                reef(x, z, &sample, &mut groups);
                coastal_canopy(&mut groups);
                let mut replay = groups.clone();
                for g in [&mut groups, &mut replay] {
                    vary_crowns(x, z, g);
                    rock_aprons(x, z, &sample, g);
                }
                assert_eq!(groups, replay);
                for (i, kind) in CROWN_KINDS.into_iter().enumerate() {
                    forms[i] += groups[kind].len();
                }
                for stone in groups[LEDGE].iter().chain(&groups[BOULDER]) {
                    assert!((stone.at[1] - (SEA - 8.)).abs() < stone.shape[1] * 0.17);
                    stones += 1;
                }
            }
        }
        assert!(forms.into_iter().all(|count| count > 0));
        assert!(stones > 0);
    }

    #[test]
    fn crowns_remain_submerged_and_rhizome_shoots_follow_the_sediment() {
        let mut root = Instance {
            at: [0., SEA - 2.2, 0., 0.4],
            shape: [0.5, 0.5, 0.5, 1.],
            traits: [3., 0., 0., 1.],
        };
        let sample: Sampler = Arc::new(|p| Some(Site {
            floor: SEA - 2.2 + 0.08*p.x, shore: if p.y < 0. { 0. } else { 50. },
            beach: 0.8, pacific: 0.1, offshore: false,
        }));
        let mut shoots = vec![root; 6];
        coastal_grass(1, 3, &sample, &mut shoots);
        assert!(shoots.len() > 6);
        for shoot in &shoots[6..] {
            let p = Vec2::new(shoot.at[0], shoot.at[2]);
            assert!(p.y >= 0.);
            assert!((shoot.at[1] + 0.009 - sample(p).unwrap().floor).abs() < 1e-5);
        }
        let mut groups: [Vec<Instance>; KINDS.len()] = Default::default();
        root.at[1] = SEA - 9.;
        root.traits[0] = 19.;
        groups[19].push(root);
        coastal_canopy(&mut groups);
        let crown = &groups[11][0];
        assert!(crown.at[1] + 1.035*crown.shape[1] < SEA);
        assert_eq!(crown.at, root.at);
        assert!(groups[19][0].shape[1] < crown.shape[1]);
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
