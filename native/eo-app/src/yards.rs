//! `--yard-detail` (off by default; `--no-yard-detail`): the rear-yard fences
//! the yard survey measured on the lot lines (eo-props `yards.rs`), as one
//! block of the props' instance arena, uploaded once like the moored boats
//! (no street tile holds them: the props' tiles stop at `props::LOAD_M`, the
//! fences are drawn to eo-render `props_yards::FENCE_FAR_M`, so an aerial view
//! has them). Each fence is cut into pieces that follow the drawn ground,
//! left out where the drawn street, a drawn building or the sea holds them.
//! The yards' shrubs and bed plants (eo-props `yards::plants`, with the survey's
//! canopy map; `--no-yard-plants` leaves them out) join the same block; their
//! trees are planted with the measured trees (props.rs `cook`, main.rs's far trees).
//! The yards' ground is the terrain shader's (eo-render `yards.rs`).

use eo_props::PropsWorld;
use eo_render::props::GpuPropInstance;
use eo_render::props_lib2 as lib2;

mod backs; // the houses' backs: back stairs, doors, downspouts (`--no-yard-stairs`)

/// The measured fences' pieces and the yards' shrubs and bed plants as prop
/// instances (none without the switch or the survey).
pub fn fence_instances(world: &PropsWorld) -> Vec<GpuPropInstance> {
    let fences = eo_props::yards::fences();
    if fences.is_empty() && eo_props::yards::plants().is_empty() {
        return vec![];
    }
    let sw = &world.street;
    let sea = eo_water::SEA_LEVEL_Y;
    let ground = |x: f64, z: f64| world.ground(x, z);
    let blocked = |x: f64, z: f64| sw.buildings.contains(x, z) || sw.paved_at(x, z).is_some() || world.ground(x, z) < sea + 0.3;
    let (mut out, mut drawn, mut measured) = (vec![], 0.0, 0.0);
    for (i, f) in fences.iter().enumerate() {
        measured += (f.b[0] - f.a[0]).hypot(f.b[1] - f.a[1]);
        // the framing's side (the posts and rails face one neighbour; the other sees the boards).
        let side = if (i as u32).wrapping_mul(0x9e37_79b9) >> 31 == 0 { 1.0 } else { -1.0 };
        let pieces = eo_props::yards::pieces(f, &ground, &blocked);
        for (k, p) in pieces.iter().enumerate() {
            drawn += p.length;
            out.push(GpuPropInstance {
                pos: [p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32],
                yaw: p.yaw as f32,
                scale: [eo_props::yards::THICKNESS_M as f32, p.height as f32, p.length as f32],
                model: lib2::YARD_FENCE,
                variant: i as u32,
                lamp: 0,
                slope: p.slope as f32,
                pad: 0,
            });
            if eo_props::yards::plants_enabled() {
                // the posts at each piece's ends, the rails and the cap (`timber`).
                let last = k + 1 == pieces.len() || (pieces[k + 1].pos[0] - p.pos[0]).hypot(pieces[k + 1].pos[2] - p.pos[2]) > 0.5 * (p.length + pieces[k + 1].length) + 0.05;
                timber(&mut out, p, i as u32, side, last);
            }
        }
    }
    eprintln!("yards: {} fences in {} pieces, {drawn:.0} m drawn of {measured:.0} m measured (drawn plan; the rest under a drawn building or street)", fences.len(), out.len());
    // the houses' backs first: the plants and objects keep off their stairs.
    let mut backs = vec![];
    let clear = backs::back_instances(world, &mut backs);
    let n = out.len();
    plant_instances(world, &clear, &mut out);
    if out.len() > n {
        eprintln!("yards: {} shrubs and bed plants drawn", out.len() - n);
    }
    let n = out.len();
    item_instances(world, &clear, &mut out);
    if out.len() > n {
        eprintln!("yards: {} objects drawn (with their pots' and beds' plants)", out.len() - n);
    }
    out.append(&mut backs);
    out
}

/// The yards' objects (eo-props `yards::items`; `--no-yard-objects` none) as the
/// library's yard models (eo-render props_lib2 `yard_items`): a cart row as two
/// or three carts side by side in a shuffled order, a pot and a half barrel each
/// at its own size with a garden shrub in it, a raised bed with a few small
/// plants on its soil; each on the drawn ground (the lowest of its corners, so
/// none floats on a slope), left out where the drawn street, a building or the
/// sea holds it.
pub fn item_instances(world: &PropsWorld, clear: &[backs::Clear], out: &mut Vec<GpuPropInstance>) {
    use eo_props::yards::ItemKind;
    use eo_render::props_lib2::{YARD_ITEM_BASE, yard_items as yi};
    let sw = &world.street;
    let sea = eo_water::SEA_LEVEL_Y;
    let blocked = |x: f64, z: f64| sw.buildings.contains(x, z) || sw.paved_at(x, z).is_some() || world.ground(x, z) < sea + 0.3;
    // whether `hit` holds within r of a point (12 headings, every metre out).
    let within = |x: f64, z: f64, r: f64, hit: &dyn Fn(f64, f64) -> bool| {
        (1..=(r.ceil() as i32)).any(|k| {
            let d = (k as f64).min(r);
            (0..12).any(|a| {
                let t = a as f64 * std::f64::consts::TAU / 12.0;
                hit(x + d * t.cos(), z + d * t.sin())
            })
        })
    };
    // a rear yard: a house within 12 m and no drawn street within 6 m (the survey's
    // yards are every lot's open ground: the front setbacks by the walk too).
    let rear = |x: f64, z: f64| within(x, z, 12.0, &|u, v| sw.buildings.contains(u, v)) && !within(x, z, 6.0, &|u, v| sw.paved_at(u, v).is_some());
    // the lowest drawn ground under a footprint (half sizes along the item's own x and z).
    let foot = |x: f64, z: f64, yaw: f64, hx: f64, hz: f64| {
        let (s, c) = yaw.sin_cos();
        [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (0.0, 0.0)]
            .iter()
            .map(|&(a, b)| world.ground(x + a * hx * c + b * hz * s, z - a * hx * s + b * hz * c))
            .fold(f64::MAX, f64::min)
    };
    let push = |out: &mut Vec<GpuPropInstance>, model: u32, x: f64, y: f64, z: f64, yaw: f64, scale: f32, seed: u32| {
        out.push(GpuPropInstance { pos: [x as f32, y as f32, z as f32], yaw: yaw as f32, scale: [scale; 3], model, variant: seed & 255, lamp: 0, slope: 0.0, pad: 0 });
    };
    // a small garden shrub (boxwood or rosemary; a sage now and then) standing on soil at y.
    let plant = |out: &mut Vec<GpuPropInstance>, x: f64, y: f64, z: f64, height: f64, seed: u32| {
        let p = eo_props::yards::YardPlant { kind: eo_props::yards::PlantKind::Bed, x, z, height_m: height, crown_r_m: 0.6 * height, tint: [1.0; 3], seed };
        let Some((model, fh, fr)) = model_for(&p) else { return };
        let u = |k: u32| ((seed.rotate_left(k * 7) >> 8) as f32) / (1u32 << 24) as f32;
        let sy = (height / fh as f64) as f32;
        let sx = ((0.6 * height / fr as f64) as f32).clamp(0.7 * sy, 1.3 * sy);
        let v = 0.85 + 0.3 * u(7);
        out.push(GpuPropInstance {
            pos: [x as f32, (y - 0.04) as f32, z as f32],
            yaw: u(1) * std::f32::consts::TAU,
            scale: [sx, sy, sx],
            model,
            variant: (u(3) * 255.0) as u32,
            lamp: (crate::props::vegetation::pack_tint([v, v, v]) & 0x00ff_ffff) | ((eo_landmarks::vegetation::TIP_NONE as u32) << 24),
            slope: u(3),
            pad: 0,
        });
    };
    for it in eo_props::yards::items() {
        let (x, z, yaw, s) = (it.x, it.z, it.yaw, it.seed);
        if blocked(x, z) || !rear(x, z) || clear.iter().any(|c| c.holds(x, z, 1.0)) {
            continue;
        }
        // (carts stand against a drawn house: within 2 m of one.)
        if it.kind == ItemKind::Carts && !within(x, z, 2.0, &|u, v| sw.buildings.contains(u, v)) {
            continue;
        }
        if std::env::var("EO_YARD_ITEMS_TRACE").is_ok_and(|v| v == "1") {
            eprintln!("yards: drawn object {:?} at {x:.1} {z:.1} yaw {yaw:.2}", it.kind);
        }
        let u = |k: u32| ((s.rotate_left(k * 5) >> 8) as f64) / (1u32 << 24) as f64;
        match it.kind {
            ItemKind::Carts => {
                // local x along the wall (three.js: x -> (cos yaw, -sin yaw)).
                let (sn, cs) = yaw.sin_cos();
                let n = 2 + (u(1) < 0.7) as usize;
                let order = [[0u32, 1, 2], [1, 0, 2], [0, 2, 1], [2, 1, 0], [1, 2, 0], [2, 0, 1]][(u(2) * 6.0) as usize % 6];
                for k in 0..n {
                    let off = (k as f64 - (n as f64 - 1.0) / 2.0) * 0.62;
                    let (px, pz) = (x + off * cs, z - off * sn);
                    if blocked(px, pz) {
                        continue;
                    }
                    let y = foot(px, pz, yaw, 0.25, 0.3);
                    let jit = (u(3 + k as u32) - 0.5) * 0.25;
                    push(out, YARD_ITEM_BASE + yi::CART + order[k], px, y, pz, yaw + jit, 1.0, s >> (k * 3));
                }
            }
            ItemKind::Patio { wood } => {
                let y = foot(x, z, yaw, 0.9, 0.9);
                push(out, YARD_ITEM_BASE + if wood { yi::PATIO_WOOD } else { yi::PATIO_METAL }, x, y, z, yaw, 1.0, s);
            }
            ItemKind::Grill => {
                let y = foot(x, z, yaw, 0.3, 0.3);
                push(out, YARD_ITEM_BASE + yi::GRILL, x, y, z, yaw, 1.0, s);
            }
            ItemKind::Pot(k) => {
                let scale = (0.75 + 0.5 * u(1)) as f32;
                let r = [0.22, 0.25, 0.33][k as usize] * scale as f64;
                let y = foot(x, z, yaw, r, r);
                push(out, YARD_ITEM_BASE + [yi::POT_TERRACOTTA, yi::POT_GLAZED, yi::BARREL][k as usize], x, y, z, yaw, scale, s);
                let soil = y + yi::SOIL_TOP_M[k as usize] as f64 * scale as f64;
                plant(out, x, soil, z, [0.45, 0.6, 0.8][k as usize] * (0.6 + 0.8 * u(2)), s ^ 0x1234_5678);
            }
            ItemKind::RaisedBed => {
                let y = foot(x, z, yaw, 1.2, 0.6);
                push(out, YARD_ITEM_BASE + yi::RAISED_BED, x, y, z, yaw, 1.0, s);
                // its crop: two rows of small plants along the bed, a few gaps.
                let (sn, cs) = yaw.sin_cos();
                let soil = y + yi::SOIL_TOP_M[3] as f64;
                for row in [-0.28f64, 0.28] {
                    for k in 0..4 {
                        if u(4 + k + (row > 0.0) as u32 * 4) < 0.25 {
                            continue;
                        }
                        // the bed's long side is its local z (2.4 m along the fence's run).
                        let (lx, lz) = (row, -0.85 + k as f64 * 0.57);
                        let (px, pz) = (x + lx * cs + lz * sn, z - lx * sn + lz * cs);
                        plant(out, px, soil, pz, 0.25 + 0.25 * u(12 + k), s.rotate_left(k + 1) ^ 0x0bad_5eed ^ (row > 0.0) as u32);
                    }
                }
            }
            ItemKind::Shed => {
                let y = foot(x, z, yaw, 0.9, 0.6);
                push(out, YARD_ITEM_BASE + yi::SHED, x, y - 0.03, z, yaw, 1.0, s);
            }
        }
    }
}

/// The model a yard plant takes and its height and half crown at scale 1: an
/// ez-tree garden shrub (eo-props `eztree::GARDEN_SHRUBS`: pittosporum,
/// boxwood, rosemary) or one of the coastal shrubs (coyote brush, sage) for
/// variety; the lawn's turf patch. Tall shrubs are mostly pittosporum, middling
/// ones boxwood and pittosporum, the beds rosemary, boxwood balls and sage.
fn model_for(p: &eo_props::yards::YardPlant) -> Option<(u32, f32, f32)> {
    use eo_props::yards::PlantKind;
    use eo_render::props_lib2::vegetation_models as vm;
    let r = (p.seed >> 8) % 100;
    let ez = |model: u32, id: &str| eo_props::eztree::ez_species(id).ok().map(|s| (model, s.height_m as f32, s.max_half_crown_m as f32));
    let garden = |i: usize| ez(vm::V_GARDEN + i as u32, eo_props::eztree::GARDEN_SHRUBS[i]);
    let coastal = |i: usize| ez(vm::V_SHRUB + i as u32, eo_props::eztree::COASTAL_SHRUBS[i]);
    match p.kind {
        PlantKind::Lawn => Some((vm::V_LAWN, 0.08, 0.7)),
        // a bed's weeds: a quarter rough turf (the lawn's blades, taller: a dense
        // mesh, so the fewest), the rest the headlands' short tufts and taller wild
        // grass bunches (crossed cards, a tenth of the turf's triangles).
        PlantKind::Weeds => Some(match r {
            0..=24 => (vm::V_LAWN, 0.08, 0.7),
            25..=79 => (vm::V_TUFT, 0.3, 0.45),
            _ => (vm::V_GRASS, 0.65, 0.65),
        }),
        PlantKind::Tree => tree_model(p.height_m, p.crown_r_m, p.seed),
        PlantKind::Bed => match r {
            0..=39 => garden(2),
            40..=74 => garden(1),
            _ => coastal(1),
        },
        // (a tall one is as often a big shrub pruned up into a small tree, a bay, a
        // ceanothus or a loquat, as a screen leafy to the ground: the dense
        // evergreen's crown on a short trunk, so a garden's tall shrubs are not one shape.)
        _ if p.height_m >= 2.2 => match r {
            0..=44 => garden(0),
            45..=64 => coastal(0),
            _ => ez(vm::V_GARDEN_TREE + 1, eo_props::eztree::GARDEN_TREES[1]),
        },
        _ if p.height_m >= 1.8 => match r {
            0..=64 => garden(0),
            _ => coastal(0),
        },
        _ => match r {
            0..=44 => garden(1),
            45..=74 => garden(0),
            _ => coastal(0),
        },
    }
}

/// A yard tree's drawn crown over its measured radius (the lidar survey's area-equivalent
/// radius of the crown's segment, which the leaves' tips reach past; INFERRED).
const TREE_CROWN_REACH: f64 = 1.2;

/// A yard tree's model (eo-props `eztree::GARDEN_TREES`) and its height and half
/// crown at scale 1: a conifer where the crown is narrow for its height (a
/// half crown under a fifth of it, a cypress's or juniper's; the measured
/// yards' median is 0.29), else a broadleaf or
/// a dense evergreen by the tree's hash (a quarter of the broadleaves grey:
/// olive, acacia, `tree_tint`).
fn tree_model(height_m: f64, crown_r_m: f64, seed: u32) -> Option<(u32, f32, f32)> {
    use eo_render::props_lib2::vegetation_models as vm;
    let r = (seed >> 8) % 100;
    let i = if crown_r_m < 0.2 * height_m {
        2
    } else if r < 55 {
        0
    } else {
        1
    };
    let s = eo_props::eztree::ez_species(eo_props::eztree::GARDEN_TREES[i]).ok()?;
    Some((vm::V_GARDEN_TREE + i as u32, s.height_m as f32, s.max_half_crown_m as f32))
}

/// The yards' shrubs, bed plants and lawn turf (eo-props yards.rs `plants`)
/// as the garden and coastal ez-tree shrubs and the lawn patch (`model_for`),
/// each at its measured height and crown, tinted by the aerial's colour over
/// it (half way to it: the aerial is greyed by haze and blur; a lawn patch
/// mottled round that), sheared onto the drawn ground; left out where the
/// drawn street, a drawn building or the sea holds it.
pub fn plant_instances(world: &PropsWorld, clear: &[backs::Clear], out: &mut Vec<GpuPropInstance>) {
    use eo_props::yards::PlantKind;
    let sw = &world.street;
    let sea = eo_water::SEA_LEVEL_Y;
    // the measured trees standing in the yards join the canopy's (the yards draw them, not the street trees' rings).
    let measured: Vec<eo_props::yards::YardPlant> = eo_props::lidar_trees::all()
        .iter()
        .enumerate()
        .filter(|(_, t)| eo_props::yards::in_yard(t.x, t.z))
        .map(|(i, t)| eo_props::yards::YardPlant { kind: PlantKind::Tree, x: t.x, z: t.z, height_m: t.height_m, crown_r_m: t.crown_r_m, tint: [1.0; 3], seed: (i as u32).wrapping_mul(0x9e37_79b9) ^ 0x5bd1_e995 })
        .collect();
    let mut trees = 0;
    for p in eo_props::yards::plants().iter().chain(measured.iter()) {
        // (the lawn's turf: its tint the aerial's, nearer the measured one than a shrub's.)
        let (x, z) = (p.x, p.z);
        if sw.buildings.contains(x, z) || sw.paved_at(x, z).is_some() || world.ground(x, z) < sea + 0.3 {
            continue;
        }
        let Some((model, fh, fr)) = model_for(p) else { continue };
        // (a back stair's footprint: no shrub, tree or grass bunch grows through it; the lawn under it stays.)
        // (a shrub by it: its crown would grow through the landings.)
        let pad = match p.kind {
            PlantKind::Tree => 0.6,
            PlantKind::Shrub => 0.3 + p.crown_r_m.min(1.2),
            _ => 0.4,
        };
        if p.kind != PlantKind::Lawn && clear.iter().any(|c| c.holds(x, z, pad)) {
            continue;
        }
        let weeds = p.kind == PlantKind::Weeds;
        let lawn = p.kind == PlantKind::Lawn || weeds;
        // (a weed drawn as a grass bunch or flowers rather than turf: its own blades' tint.)
        let bunch = weeds && model != eo_render::props_lib2::vegetation_models::V_LAWN;
        let tree = p.kind == PlantKind::Tree;
        trees += tree as usize;
        // (a bunch stands taller than the turf's tufts: 1.3 x a tuft, 2.4 x the grass.)
        let stand = if !bunch {
            1.0
        } else if model == eo_render::props_lib2::vegetation_models::V_TUFT {
            1.3
        } else {
            2.4
        };
        let sy = (stand * p.height_m / fh as f64) as f32;
        // (a tree's crown is drawn a fifth wider than measured: the survey's radius is its
        // segment's area-equivalent, inside the leaves' reach.)
        let crown = p.crown_r_m * if tree { TREE_CROWN_REACH } else { 1.0 };
        let sx = if lawn { 1.0 } else { ((crown / fr as f64) as f32).clamp(if tree { 0.6 } else { 0.7 } * sy, if tree { 1.6 } else { 1.5 } * sy) };
        // a lawn's mowing height varies a little patch to patch.
        let sy = if weeds { sy } else if lawn { 0.85 + 0.3 * ((p.seed >> 4) & 255) as f32 / 255.0 } else { sy };
        // (a weed tuft is its own width: the patch's 0.7 m radius scaled to it.)
        // (a bunch's cards are sparser than the turf's blades: drawn wider, so a drift reads as cover.)
        let sx = if weeds { (p.crown_r_m / fr as f64 * if bunch { 1.9 } else { 1.0 }) as f32 } else { sx };
        let u = |k: u32| ((p.seed.rotate_left(k * 7) >> 8) as f32) / (1u32 << 24) as f32;
        // the aerial's colour half way; a lawn patch its own value and hue round it
        // (the turf's mottle a patch across: water, wear and the mower).
        let mut tint = p.tint.map(|t| 1.0 + 0.5 * (t - 1.0));
        // a grey-leaved tree (olive, acacia): a quarter of the broadleaves, greyed and lighter.
        if tree && model == eo_render::props_lib2::vegetation_models::V_GARDEN_TREE && (p.seed >> 8) % 100 < 14 {
            tint = [tint[0] * 1.35, tint[1] * 1.15, tint[2] * 1.5];
        }
        if !lawn && !tree {
            // a garden's shrubs are not one green: each its own value and cast, and some
            // the cultivars' colours: a fifth yellow-green (golden privet, euonymus), a
            // tenth blue-grey (lavender, westringia, senecio), one in twenty bronze
            // (purple pittosporum, phormium, loropetalum) (INFERRED from the city's
            // garden plantings; the leaf shading halves a cast's chroma with `--crown-light`).
            let (v, h, c) = (0.78 + 0.44 * u(7), u(8) - 0.5, u(9));
            tint = [tint[0] * v * (1.0 + 0.35 * h), tint[1] * v, tint[2] * v * (1.0 - 0.35 * h)];
            let cast = if c < 0.2 {
                [1.35, 1.18, 0.7]
            } else if c < 0.3 {
                [1.2, 1.12, 1.55]
            } else if c < 0.35 {
                [1.6, 0.72, 0.95]
            } else {
                [1.0; 3]
            };
            tint = [tint[0] * cast[0], tint[1] * cast[1], tint[2] * cast[2]];
        }
        if lawn {
            let (v, h) = (0.9 + 0.2 * u(5), u(6) - 0.5);
            tint = [tint[0] * v * (1.0 + 0.25 * h), tint[1] * v, tint[2] * v * (1.0 - 0.25 * h)];
        }
        // the shaded beds' weeds: darker, a little yellower than the turf.
        if weeds && !bunch {
            tint = [tint[0] * 0.85, tint[1] * 0.78, tint[2] * 0.6];
        }
        if bunch {
            // the season's grass (the headlands' green and cured albedos), a clump its own value.
            // (blue raised to 0.45 of green, as the lawn's `LAWN_LIVE`: the headlands' grass
            // albedo's 0.34 drew a lime under the play look's camera.)
            let g = crate::props::vegetation::season_grass(x, z);
            let v = 0.75 + 0.4 * u(5);
            tint = [g[0] * v * 1.05, g[1] * v, (g[2] * 0.9).max(0.45 * g[1]) * v];
        }
        let d = 0.5;
        let grad = [((world.ground(x + d, z) - world.ground(x - d, z)) / (2.0 * d)) as f32, ((world.ground(x, z + d) - world.ground(x, z - d)) / (2.0 * d)) as f32];
        out.push(GpuPropInstance {
            pos: [x as f32, (world.ground(x, z) - if lawn { eo_landmarks::vegetation::GRASS_SINK_M } else if tree { 0.08 } else { 0.12 }) as f32, z as f32],
            yaw: u(1) * std::f32::consts::TAU,
            // (a weed tuft is an oval, up to twice as long as wide.)
            scale: [sx, sy, sx * if weeds { 0.5 + 1.0 * u(2) } else { 0.85 + 0.3 * u(2) }],
            model,
            // (the yard reach is the shrubs': a tree is drawn and casts as the coastal trees
            // do, a bed's plant and the lawn's turf at their own reach.)
            variant: (u(3) * 255.0) as u32 | if p.kind == PlantKind::Shrub { eo_render::props_lib2::vegetation_models::YARD_PLANT_BIT } else { 0 },
            lamp: (crate::props::vegetation::pack_tint(tint) & 0x00ff_ffff) | ((eo_landmarks::vegetation::TIP_NONE as u32) << 24),
            slope: u(3),
            pad: crate::props::vegetation::pack_grad(if tree { [0.0; 2] } else { grad }),
        });
    }
    if trees > 0 {
        eprintln!("yards: {trees} trees drawn as the garden species ({} of them measured)", measured.len());
    }
}

/// A wood fence's framing, as pieces of the fence's own box (eo-render
/// props_yards.wgsl draws them as plain timber, `TIMBER_BIT`): a 4x4 post
/// (dressed 3-1/2 in) at the piece's start, and at its end where the fence
/// stops; two 2x4 rails (3-1/2 x 1-1/2 in dressed) on the framing side, 0.35 m
/// over the ground and 0.3 m under the top, following the grade; a 2x6 cap
/// laid flat on top (dressed sizes: American Softwood Lumber Standard, NIST
/// PS 20; the layout is a common board-on-rail privacy fence, INFERRED).
fn timber(out: &mut Vec<GpuPropInstance>, p: &eo_props::yards::FencePiece, line: u32, side: f64, last: bool) {
    const IN: f64 = 0.0254;
    let t = eo_props::yards::THICKNESS_M;
    let (sy, cy) = p.yaw.sin_cos();
    let along = [sy, cy];
    let across = [cy, -sy];
    let foot = p.pos[1];
    let top = foot + p.height;
    let at = |s: f64, x: f64| [p.pos[0] + along[0] * s + across[0] * x, p.pos[2] + along[1] * s + across[1] * x];
    let mut push = |c: [f64; 2], y: f64, scale: [f64; 3], slope: f64| {
        out.push(GpuPropInstance {
            pos: [c[0] as f32, y as f32, c[1] as f32],
            yaw: p.yaw as f32,
            scale: scale.map(|v| v as f32),
            model: lib2::YARD_FENCE,
            variant: line | TIMBER_BIT,
            lamp: 0,
            slope: slope as f32,
            pad: 0,
        });
    };
    let post = 3.5 * IN;
    let rail = (1.5 * IN, 3.5 * IN);
    let off = side * (0.5 * t + 0.5 * post);
    let half = 0.5 * p.length;
    for (s, here) in [(-half, true), (half, last)] {
        if here {
            let y = foot + p.slope * s;
            push(at(s, off), y, [post, top - foot + 0.06, post], 0.0);
        }
    }
    let roff = side * (0.5 * t + 0.5 * rail.0);
    for h in [0.35 + eo_props::yards::SINK_M, p.height - 0.3] {
        push(at(0.0, roff), foot + h - 0.5 * rail.1, [rail.0, rail.1, p.length], p.slope);
    }
    push(at(0.0, side * 0.5 * (post - t).max(0.0) * 0.5), top, [5.5 * IN, 1.5 * IN, p.length], p.slope);
}

/// The fence pieces' `variant` bit for its framing (posts, rails, the cap): plain timber.
pub const TIMBER_BIT: u32 = 1 << 30;

/// The fences' block of the arena: waiting for the props renderer, then (at, n).
#[derive(Default)]
pub struct FenceBlock {
    pub pending: Vec<GpuPropInstance>,
    pub block: (u32, u32),
}
