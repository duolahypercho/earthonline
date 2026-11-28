//! Round 3 of the signal poles on real junctions (its own binary: the switches are process-wide
//! and each world caches what it planned under them).
//!
//! The arm rise only where the data says the junction is on a State route (Caltrans RSP ES-7E);
//! every other arm level (`signal-mast-arm-sf-city-street`). The combination poles: a street
//! light within the SFPUC like-for-like distance of a mast pole stands on it, keeping its
//! fixture, mounting height and orientation (its light's photometry the walk's), one per pole,
//! no corner with two lights, and the city's lamps and lights the same in number.
use eo_props::signal_lamps::{self, LAMP_POST_TOP_M};
use eo_props::signals::{self, Fitting};
use eo_props::{Model, PropsWorld};
use std::sync::Arc;

fn world() -> Arc<PropsWorld> {
    let paths = eo_data::DataPaths::discover().unwrap();
    let w = Arc::new(eo_props::load_world(&paths).unwrap());
    let city = eo_data::City::load(&paths).unwrap();
    w.install_furniture(&city.buildings).unwrap();
    eo_props::overhead::install(&w, eo_props::overhead::DropBuilding::from_city(&city.buildings)).unwrap();
    w
}

fn is_lamp(m: Model) -> bool {
    matches!(m, Model::LampCobra | Model::LampBanner | Model::LampHistoric)
}

#[derive(Debug, Default)]
struct Census {
    lamps: usize,
    lights: usize,
    on_poles: usize,
    /// Street lights within 9 m (the lamp kinds' gap) of another.
    close_pairs: usize,
}

fn census(w: &PropsWorld, keys: &[(i32, i32)]) -> Census {
    let mut c = Census::default();
    let mut feet = vec![];
    for &k in keys {
        let t = w.plan_tile(k);
        let lamps: Vec<[f64; 3]> = t.props.iter().filter(|p| is_lamp(p.model)).map(|p| p.pos).collect();
        for l in &t.lights {
            assert!(lamps.contains(&l.base), "a light at {:?} without its post", l.base);
        }
        for f in &t.fittings {
            if let Fitting::Luminaire { pole } = f {
                c.on_poles += 1;
                assert!(lamps.contains(pole), "a pole's light stands on its foot");
                let mast = t.props.iter().find(|p| p.model == Model::SignalPole && p.variant == 1 && p.pos == *pole).expect("the mast pole");
                let lamp = t.props.iter().find(|p| is_lamp(p.model) && p.pos == *pole).unwrap();
                assert!(mast.scale[1] as f64 >= LAMP_POST_TOP_M * lamp.scale[1] as f64 - 1e-4, "the pole reaches the light's post top");
            }
        }
        c.lamps += lamps.len();
        c.lights += t.lights.len();
        feet.extend(lamps);
    }
    for (i, a) in feet.iter().enumerate() {
        c.close_pairs += feet[i + 1..].iter().filter(|b| (a[0] - b[0]).hypot(a[2] - b[2]) < 9.0).count();
    }
    c
}

#[test]
fn round3_rise_on_state_routes_and_combination_poles() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let std = eo_props::Standards::load(&eo_data::CaStandards::load(&paths).unwrap()).unwrap();
    assert_eq!(std.signal_arm_city_rise, 0.0, "a city street's arm is level");
    assert!((std.lamp_share_move_max - 3.048).abs() < 1e-3, "10 ft");
    eo_props::overhead::set_lamp_standards(true);
    eo_props::overhead::set_joint_dedupe(true);
    signals::set_arm_rise(true);
    signals::set_arm_rise_routes(true);
    let w = world();

    // the rise only on a State route
    let (mut state_rising, mut city_arms, mut state_nodes) = (0, 0, 0);
    let mut a_city_node = None;
    for (ni, n) in w.street.plan.nodes.iter().enumerate() {
        if !n.signalised() {
            continue;
        }
        let np = signals::plan_node_parts(&w, ni);
        if signals::on_state_route(&w.street, n) {
            state_nodes += 1;
            state_rising += np.arms.iter().filter(|a| a.class.is_some()).count();
        } else {
            assert!(np.arms.iter().all(|a| a.class.is_none()), "node {ni}: a city street's arm rises");
            city_arms += np.arms.len();
            if a_city_node.is_none() && np.arms.iter().any(|a| a.length >= std.signal_arms[0].length) {
                a_city_node = Some(ni);
            }
        }
    }
    println!("State-route junctions {state_nodes}: {state_rising} rising arms; city arms {city_arms}, level");
    assert!(state_nodes > 50 && state_rising > 50 && city_arms > 500);
    // off (`--no-arm-rise-routes`): the city's arms rise as in round 2
    signals::set_arm_rise_routes(false);
    let ni = a_city_node.unwrap();
    assert!(signals::plan_node_parts(&w, ni).arms.iter().any(|a| a.class.is_some()), "round 2: every arm the table covers rises");
    signals::set_arm_rise_routes(true);

    // the combination poles
    let w_off = world();
    signal_lamps::set_signal_luminaires(true);
    let f = w.furniture();
    let mut moves = vec![];
    for (ni, n) in w.street.plan.nodes.iter().enumerate() {
        if !n.signalised() {
            continue;
        }
        let masts: Vec<[f64; 3]> = w.signal_node_props(ni).iter().filter(|p| p.model == Model::SignalPole && p.variant == 1).map(|p| p.pos).collect();
        let lamps = signal_lamps::node_lamps(&w, ni);
        for (i, l) in lamps.iter().enumerate() {
            assert!(masts.contains(&l.pole), "on one of the junction's mast poles");
            assert!(lamps[i + 1..].iter().all(|o| o.pole != l.pole), "one light per pole");
            let d = (l.from[0] - l.pole[0]).hypot(l.from[2] - l.pole[2]);
            assert!(d <= std.lamp_share_move_max + 1e-9, "moved {d:.2} m");
            assert!(matches!(l.prop.model, Model::LampCobra | Model::LampBanner), "a cobra or banner light");
            // like for like: the walk's light, from the pole's spot
            let it = &f.entry(&w, l.entry as usize).items[l.item];
            let (p0, l0) = f.moved_lamp(it, [it.x, it.y, it.z]).unwrap();
            let (l0, l1) = (l0.unwrap(), l.light.unwrap());
            assert_eq!((p0.yaw, p0.scale, p0.model), (l.prop.yaw, l.prop.scale, l.prop.model), "the same fixture, orientation and size");
            assert_eq!((l0.intensity, l0.angle, l0.penumbra, l0.range_m, l0.source_lift_m, l0.decay), (l1.intensity, l1.angle, l1.penumbra, l1.range_m, l1.source_lift_m, l1.decay), "the same photometry");
            for k in 0..3 {
                assert!(((l1.pos[k] - l1.base[k]) - (l0.pos[k] - l0.base[k])).abs() < 1e-9, "the head where it was over its post's foot");
            }
            assert_eq!(l1.base, l.pole);
            moves.push((ni, l.pole));
        }
    }
    println!("street lights on mast poles: {}", moves.len());
    assert!(moves.len() >= 10, "{} combination poles", moves.len());
    let mut keys: Vec<(i32, i32)> = moves
        .iter()
        .flat_map(|m| {
            let k = eo_street::tile_key(m.1[0], m.1[2]);
            (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (k.0 + dx, k.1 + dz)))
        })
        .collect();
    keys.sort_unstable();
    keys.dedup();
    signal_lamps::set_signal_luminaires(false);
    let off = census(&w_off, &keys);
    signal_lamps::set_signal_luminaires(true);
    let on = census(&w, &keys);
    signal_lamps::set_signal_luminaires(false);
    println!("off {off:?}\non {on:?}");
    assert_eq!(off.on_poles, 0, "off: none");
    assert_eq!(on.on_poles, moves.len(), "every combination pole drawn in its junction's tile");
    assert_eq!((on.lamps, on.lights), (off.lamps, off.lights), "the lamps and lights the same in number");
    assert!(on.close_pairs <= off.close_pairs, "a move stood two street lights at one corner");
}
