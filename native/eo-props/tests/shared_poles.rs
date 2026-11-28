//! Street lights share the Muni trolley support poles, and two pieces of a
//! street meeting at a plain joint stand one support station there
//! (`overhead::set_lamp_standards`, `overhead::set_joint_dedupe`; eo-app
//! `--no-shared-poles`, `--no-joint-dedupe`). Its own binary: the switches are
//! process-wide, and each world caches what it planned under them.
use eo_props::overhead::{self, PoleKind};
use eo_props::{Model, PropsWorld};
use std::sync::Arc;

/// Downtown and Mission sites where, before the switches, a street light stood
/// 0.3-2.3 m from a trolley support pole, or two support poles stood on one spot.
const SITES: [[f64; 2]; 6] = [[509.5, 780.0], [-1454.0, 4018.0], [-1629.0, 3621.0], [2274.0, -2379.0], [2286.0, -1879.0], [2151.0, -2356.0]];

fn world() -> Arc<PropsWorld> {
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = Arc::new(eo_props::load_world(&paths).unwrap());
    let city = eo_data::City::load(&paths).unwrap();
    w.install_furniture(&city.buildings).unwrap();
    overhead::install(&w, overhead::DropBuilding::from_city(&city.buildings)).unwrap();
    w
}

fn is_lamp(m: Model) -> bool {
    matches!(m, Model::LampCobra | Model::LampBanner | Model::LampHistoric)
}

#[derive(Debug, Default, PartialEq)]
struct Census {
    /// Street lights within 2.5 m of a drawn (not lamp-standard) support pole.
    lamp_beside_pole: usize,
    /// Drawn support poles within 0.5 m of another.
    stacked_supports: usize,
    lamp_standards: usize,
    /// Street lights within 0.5 m of another.
    stacked_lamps: usize,
}

fn census(w: &PropsWorld, keys: &[(i32, i32)]) -> Census {
    let mut c = Census::default();
    let (mut lamps, mut supports) = (vec![], vec![]);
    for &k in keys {
        let t = w.plan_tile(k);
        let o = overhead::plan_tile(w, k);
        let tile_lamps: Vec<[f64; 3]> = t.props.iter().filter(|p| is_lamp(p.model)).map(|p| p.pos).collect();
        for p in o.poles.iter().filter(|p| p.kind == PoleKind::Support) {
            if p.lamp_standard {
                // a lamp standard has its street light standing on its foot, in the same tile.
                assert!(tile_lamps.contains(&p.pos), "lamp standard {} at {:?} has no street light on its spot", p.id, p.pos);
                c.lamp_standards += 1;
            } else {
                supports.push(p.pos);
            }
        }
        // each light moved with its post: every lamp light's base is a lamp's foot.
        for l in &t.lights {
            assert!(tile_lamps.contains(&l.base), "a lamp light at {:?} has no post at its base", l.base);
        }
        lamps.extend(tile_lamps);
    }
    let near = |a: [f64; 3], b: [f64; 3], r: f64| (a[0] - b[0]).hypot(a[2] - b[2]) < r;
    c.lamp_beside_pole = lamps.iter().filter(|&&l| supports.iter().any(|&s| near(l, s, 2.5))).count();
    for (i, &a) in supports.iter().enumerate() {
        c.stacked_supports += supports[i + 1..].iter().filter(|&&b| near(a, b, 0.5)).count();
    }
    for (i, &a) in lamps.iter().enumerate() {
        c.stacked_lamps += lamps[i + 1..].iter().filter(|&&b| near(a, b, 0.5)).count();
    }
    c
}

#[test]
fn lamps_share_trolley_poles_and_joints_stand_one_station() {
    let keys: Vec<(i32, i32)> = {
        let mut v: Vec<(i32, i32)> = SITES.iter().flat_map(|s| {
            let k = eo_street::tile_key(s[0], s[1]);
            (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (k.0 + dx, k.1 + dz)))
        }).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    // Off (the library default): as before.
    let off = census(&world(), &keys);
    println!("switches off: {off:?}");
    assert_eq!(off.lamp_standards, 0);
    assert!(off.lamp_beside_pole > 0 && off.stacked_supports > 0, "the sample no longer shows the problem: {off:?}");
    // On (the game's default).
    overhead::set_lamp_standards(true);
    overhead::set_joint_dedupe(true);
    let w = world();
    let on = census(&w, &keys);
    println!("switches on: {on:?}");
    // (A lamp moves at most its walk's nudge reach, so across the sample's edge the count may
    // differ by one or two; city-wide it is unchanged: eo-props example `pole_gaps`.)
    assert!(on.stacked_lamps <= off.stacked_lamps, "sharing stood two street lights on one spot");
    assert!(on.lamp_standards > 0);
    assert!(on.lamp_beside_pole * 4 <= off.lamp_beside_pole, "street lights still beside trolley poles: {} of {}", on.lamp_beside_pole, off.lamp_beside_pole);
    assert_eq!(on.stacked_supports, 0, "two support poles on one spot");
    // At 509.5, 780: a cobra light stood 1.9 m from a support pole; it now stands on the pole's spot.
    let k = eo_street::tile_key(SITES[0][0], SITES[0][1]);
    let o = overhead::plan_tile(&w, k);
    let std = o.poles.iter().find(|p| p.lamp_standard && (p.pos[0] - SITES[0][0]).hypot(p.pos[2] - SITES[0][1]) < 3.0).expect("a lamp standard at the first site");
    assert!(w.plan_tile(k).props.iter().any(|p| p.model == Model::LampCobra && p.pos == std.pos));
}
