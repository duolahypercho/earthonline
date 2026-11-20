//! The hill rock: rock outcrops on mapped bare rock and the cliffs over the
//! beaches, single blocks on steep slopes and a few stones on bare earth,
//! placed on the drawn ground's sampler (`vegetation::Site`) by the same
//! world-anchored lattice as the plants (`vegetation::lattice`, salt 3), so a
//! tile planned twice is the same and the plants' streams are untouched.
//! `vegetation::plant_coarse` calls `plant` after its trees and scrub when
//! `--no-hill-rock` is not given; the mid ring past the props tiles calls it
//! alone (eo-app props/hill_rock.rs). Sources and colours: the module notes of
//! `vegetation` (Rock).

use crate::vegetation::{Cover, Kind, Lithology, Plant, Site, field, hash, lattice, smoothstep, srgb_to_linear};

/// The scanned-rock switch names (the roll-up first) and `--no-hill-rock`
/// (read by `vegetation::Switches`); eo-app accepts them. The scanned rock's
/// are read by `rocks::scan::Switches` (`--no-gg3` turns them all off too).
pub const SWITCHES: &[&str] = &["--no-gg3-rock", "--no-scan-rocks", "--no-cliff-dressing", "--no-surf-rocks", "--no-scan-riprap", "--no-rock-geology", "--no-hill-rock"];

/// The rock lattice (m).
pub const ROCK_PITCH_M: f64 = 5.0;

/// Rock colours, sRGB (see `vegetation`'s module notes, Rock). The chert is 10R 3/2, the USGS
/// "dull brownish red" at a lower chroma than the 10R 3/3 first used, which drew
/// salmon-pink under the frame's light: 10R 3/3 (98, 64, 57) taken two thirds of
/// the way to the neutral of the same value in linear light (INFERRED).
pub const CHERT_SRGB: [u8; 3] = [90, 67, 62];
pub const SERPENTINITE_SRGB: [u8; 3] = [88, 104, 95];
pub const SANDSTONE_SRGB: [u8; 3] = [128, 122, 101];
/// The rock mesh's mean vertex shade (vegetation_models.rs `rock`: 0.78 + up light,
/// cavities darker; about 0.8): the tint divides it out so the rock's mean is its colour.
pub const ROCK_SHADE: f32 = 0.8;

/// The rock outcrops and blocks over [x0, x1) x [z0, z1) (the coarse
/// layer's third part).
///
/// Where a scanned rock piece stands (`rocks::scan::Placed::claims`: the
/// footprints of the slope pieces the landmark build placed from the DEM
/// and the imagery) nothing is planted; between them, and everywhere when
/// no slope piece is placed, the blocks plant as before.
pub fn plant(x0: f64, z0: f64, x1: f64, z1: f64, site: &dyn Fn(f64, f64) -> Option<Site>, out: &mut Vec<Plant>) {
    let placed = crate::rocks::scan::placed();
    plant_with(placed.as_ref().map(|p| &p.claims), x0, z0, x1, z1, site, out);
}

/// `plant` with the scans' footprints given (None: nothing claimed).
pub fn plant_with(claims: Option<&crate::rocks::scan::Claims>, x0: f64, z0: f64, x1: f64, z1: f64, site: &dyn Fn(f64, f64) -> Option<Site>, out: &mut Vec<Plant>) {
    lattice(x0, z0, x1, z1, ROCK_PITCH_M, 3, &mut |x, z, ix, iz| {
        if claims.is_some_and(|c| c.covers(x, z)) {
            return;
        }
        let Some(s) = site(x, z) else { return };
        let Some(cover) = s.cover else { return };
        let slope = s.slope_deg();
        let steep = smoothstep(30.0, 50.0, slope);
        let outcrop = matches!(cover, Cover::Rock | Cover::Cliff);
        // outcrops (mapped bare rock, the cliffs over the beaches): whole patches of the
        // 22 m field filled with large overlapping blocks, a few loose blocks between;
        // elsewhere single blocks on steep slopes (grouped by the same field) and a few
        // stones on bare earth.
        let group = field(x, z, 22.0, 31);
        // the flora pass (`--no-chert-ground`): ribbon-chert outcrops on the steep Marin faces.
        let ribbon = crate::flora::flora().chert && s.lithology == Lithology::Chert && slope >= crate::flora::RIBBON_SLOPE_DEG && z < crate::vegetation::marin_z();
        let chance = match cover {
            Cover::Rock | Cover::Cliff => 0.12 + 0.86 * outcrop_patch(x, z, cover),
            Cover::Bare => 0.06 + 0.5 * steep,
            Cover::Lawn => 0.0,
            _ => steep * steep * smoothstep(0.35, 0.75, group) * 0.9,
        } + if ribbon { 0.3 * smoothstep(0.3, 0.6, group) } else { 0.0 };
        if hash(ix, iz, 3) >= chance {
            return;
        }
        // outcrop blocks of 4.5-9.5 m on the 5 m lattice, so they overlap into rock faces;
        // elsewhere 0.8-3.4 m (sizes INFERRED).
        // The flora pass (`--no-serpentinite`): the serpentinite stands as hard knobs
        // 0.5-3 m (a few to 4.5) out of its sheared matrix (game m: x1.25).
        let serp = crate::flora::flora().serpentinite && s.lithology == Lithology::Serpentinite;
        let size = if serp {
            let (a, b) = crate::flora::BOULDER_M;
            let big = hash(ix, iz, 11) < crate::flora::BOULDER_BIG_SHARE;
            (if big { crate::flora::BOULDER_BIG_M } else { a + (b - a) * hash(ix, iz, 4).powf(1.4) } * 1.25) as f32
        } else if outcrop { (4.5 + 5.0 * hash(ix, iz, 4).powf(1.2)) as f32 } else { (0.8 + 2.6 * hash(ix, iz, 4).powf(1.6) * (0.6 + 0.8 * steep)) as f32 };
        let shape = (hash(ix, iz, 5) * 4.0) as u8 % 4;
        let shape = match s.lithology {
            Lithology::Chert => shape % 2,
            Lithology::Serpentinite => 2 + shape % 2,
            Lithology::Sandstone => shape,
        };
        let base = match s.lithology {
            Lithology::Serpentinite if serp => crate::flora::SERPENTINITE_SRGB,
            Lithology::Chert if ribbon => crate::flora::CHERT_RIBBON_SRGB,
            Lithology::Chert => CHERT_SRGB,
            Lithology::Serpentinite => SERPENTINITE_SRGB,
            Lithology::Sandstone => SANDSTONE_SRGB,
        };
        let v = 0.85 + 0.3 * hash(ix, iz, 6) as f32;
        let tint = base.map(|c| srgb_to_linear(c) / ROCK_SHADE * v);
        // a block stands proud of its slope (the mesh is about 0.6 as tall as wide at
        // scale 1): a loose block 0.5-0.7 as tall as wide, an outcrop's 0.65-0.95, so the
        // overlapping blocks stand as a broken face rather than a bed of flat pillows; the
        // bedded chert a fifth taller so its slabs break the skyline rather than lying
        // as one smooth slab.
        let tall = if outcrop { 1.1 + 0.5 * hash(ix, iz, 9) } else { 0.85 + 0.35 * hash(ix, iz, 9) } as f32 * if s.lithology == Lithology::Chert { 1.2 } else { 1.0 };
        let (grad, sink) = rock_seat(s.grad(), size, tall);
        out.push(Plant {
            kind: Kind::Rock(shape),
            pos: [x, s.y - sink, z],
            yaw: hash(ix, iz, 7) * std::f64::consts::TAU,
            scale: [size * (0.9 + 0.4 * hash(ix, iz, 10) as f32), size * tall, size],
            rank: hash(ix, iz, 8) as f32,
            tint,
            grad,
            tip: if ribbon { crate::flora::TIP_RIBBON } else if serp { crate::flora::TIP_SCALY } else { crate::vegetation::TIP_NONE },
        });
        if serp && outcrop {
            // the sheared bluff: more knobs round the first, half buried in the matrix
            // (blocky, broken faces; the knobs per outcrop cell INFERRED).
            for k in 0..crate::flora::SERP_KNOBS {
                let h = |salt: u32| hash(ix * 7 + k as i64, iz, salt);
                let (ox, oz) = ((h(41) - 0.5) * ROCK_PITCH_M, (h(42) - 0.5) * ROCK_PITCH_M);
                let Some(s2) = site(x + ox, z + oz) else { continue };
                let (a, b) = crate::flora::BOULDER_M;
                let size = ((a + (b - a) * h(43).powf(1.6)) * 1.25) as f32;
                let tall = (0.6 + 0.4 * h(44)) as f32;
                let (grad, sink) = rock_seat(s2.grad(), size, tall);
                out.push(Plant {
                    kind: Kind::Rock(2 + (h(45) * 2.0) as u8 % 2),
                    pos: [x + ox, s2.y - sink - 0.15 * size as f64, z + oz],
                    yaw: h(46) * std::f64::consts::TAU,
                    scale: [size * (0.9 + 0.4 * h(47) as f32), size * tall, size],
                    rank: h(48) as f32,
                    tint: crate::flora::SERPENTINITE_SRGB.map(|c| srgb_to_linear(c) / ROCK_SHADE * (0.8 + 0.35 * h(49) as f32)),
                    grad,
                    tip: crate::flora::TIP_SCALY,
                });
            }
        }
    });
}

/// How much of a rock block follows its slope (the shear): a block sheared
/// fully onto a steep face lies flat on it like a disc; one standing plumb
/// stands out of the face. A third of the slope (INFERRED).
pub const ROCK_SHEAR: f32 = 0.35;

/// The block's half width over its scale (the rock mesh: unit sphere cut by
/// joint planes at 0.62-0.92, times its stretch 0.95-1.35, times a half).
const ROCK_HALF_WIDTH: f32 = 0.42;

/// The seat of a rock block on its slope (gradient `g`, block `size` m across,
/// `tall` its height scale): the shear it takes (`ROCK_SHEAR` of the slope)
/// and how far its origin sinks below the ground so its downhill edge meets
/// the ground (the rest of the slope across its half width, less the fifth of
/// its height the mesh already sets below its origin), plus a twentieth of its
/// size so no edge floats.
pub fn rock_seat(g: [f32; 2], size: f32, tall: f32) -> ([f32; 2], f64) {
    let steep = (g[0] * g[0] + g[1] * g[1]).sqrt();
    let rest = steep * (1.0 - ROCK_SHEAR);
    let height = size * tall * 0.6;
    let drop = ROCK_HALF_WIDTH * size * rest - 0.2 * height;
    ([g[0] * ROCK_SHEAR, g[1] * ROCK_SHEAR], (drop.max(0.0) + 0.05 * size) as f64)
}

/// The outcrop field's share at (x, z) on mapped rock and cliff ground: 1 in
/// a patch of the 22 m field, 0 between, over a narrow band so a patch fills
/// whole. About half a cliff face is outcrop and mapped rock three quarters
/// (INFERRED: the Baker and Marshall's faces are rock with scrub between).
pub fn outcrop_patch(x: f64, z: f64, cover: Cover) -> f64 {
    let t = if cover == Cover::Rock { 0.32 } else { 0.48 };
    smoothstep(t, t + 0.06, field(x, z, 22.0, 31))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vegetation::{Switches, plant_coarse};

    /// The coarse layer with only the rock on plants exactly what `plant`
    /// does, and with everything on it ends with those rocks in order.
    #[test]
    fn the_coarse_layer_plants_these_rocks_last() {
        let site = |x: f64, z: f64| {
            let a = (20.0 + 30.0 * ((x * 0.01).sin() * (z * 0.013).cos()).abs()).to_radians();
            let cover = if x < 120.0 { Cover::Cliff } else { Cover::Scrub };
            Some(Site { y: 15.0, normal: [a.sin(), a.cos(), 0.0], ground: [0.2; 3], cover: Some(cover), lithology: Lithology::Sandstone, shore_m: 80.0, hollow_m: 0.0 })
        };
        let key = |p: &Plant| (p.pos.map(|v| (v * 1e6) as i64), p.scale.map(|v| (v * 1e6) as i64), (p.yaw * 1e9) as i64);
        let mut alone = vec![];
        plant_with(None, 0.0, 0.0, 256.0, 256.0, &site, &mut alone);
        assert!(alone.len() > 50, "{}", alone.len());
        let mut rock_only = vec![];
        plant_coarse(0.0, 0.0, 256.0, 256.0, &site, Switches { coastal_trees: false, ground_cover: false, hill_rock: true }, &mut rock_only);
        assert_eq!(alone.iter().map(key).collect::<Vec<_>>(), rock_only.iter().map(key).collect::<Vec<_>>());
        let mut all = vec![];
        plant_coarse(0.0, 0.0, 256.0, 256.0, &site, Switches::ALL, &mut all);
        let tail = &all[all.len() - alone.len()..];
        assert_eq!(tail.iter().map(key).collect::<Vec<_>>(), alone.iter().map(key).collect::<Vec<_>>());
        assert!(SWITCHES.contains(&"--no-hill-rock") && SWITCHES[0] == "--no-gg3-rock");
    }

    /// The scans claim only where a piece stands: with two footprints the
    /// blocks inside them go and every other block stays exactly as it was;
    /// with no slope piece placed (the cliff band under `--no-cliff-dressing`)
    /// nothing goes, inside the scans' regions or out.
    #[test]
    fn the_scans_claim_only_their_footprints() {
        let site = |_x: f64, _z: f64| Some(Site { y: 15.0, normal: [0.6, 0.8, 0.0], ground: [0.2; 3], cover: Some(Cover::Cliff), lithology: Lithology::Serpentinite, shore_m: 40.0, hollow_m: 0.0 });
        let key = |p: &Plant| (p.pos.map(|v| (v * 1e6) as i64), (p.yaw * 1e9) as i64);
        let mut all = vec![];
        plant_with(None, 0.0, 0.0, 200.0, 200.0, &site, &mut all);
        assert!(all.len() > 200, "{}", all.len());
        let discs = vec![[50.0, 50.0, 12.0], [150.0, 120.0, 8.0]];
        let claims = crate::rocks::scan::Claims::new(ROCK_PITCH_M, discs.clone());
        let mut some = vec![];
        plant_with(Some(&claims), 0.0, 0.0, 200.0, 200.0, &site, &mut some);
        let inside = |p: &Plant| discs.iter().any(|d| (p.pos[0] - d[0]).hypot(p.pos[2] - d[1]) < d[2]);
        assert!(some.iter().all(|p| !inside(p)));
        let kept: Vec<_> = all.iter().filter(|p| !inside(p)).map(key).collect();
        assert_eq!(some.iter().map(key).collect::<Vec<_>>(), kept);
        assert!(kept.len() < all.len() && kept.len() > all.len() * 3 / 4);
        let mut none = vec![];
        plant_with(Some(&crate::rocks::scan::Claims::default()), 0.0, 0.0, 200.0, 200.0, &site, &mut none);
        assert_eq!(none.iter().map(key).collect::<Vec<_>>(), all.iter().map(key).collect::<Vec<_>>());
    }
}
