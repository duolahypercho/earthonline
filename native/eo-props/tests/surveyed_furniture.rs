//! The surveyed-first furniture walk (`furniture::set_surveyed_first`, eo-app
//! `--surveyed-furniture`), on the real city over a citywide sample of street
//! tiles: with it a walk slot given to a kind rare on SF footways stays
//! empty (every bollard, planter and newsrack left is observed kit), and one
//! given to a street-commerce kind does away from a trading frontage; the
//! surveyed and forced items (trees, meters, signs, lights, parked cars) are
//! as before and the other hashed kinds thin a little; without it the walk
//! is the JS walk as ported. Its own test binary: the mode is process-wide.

use std::collections::BTreeMap;
use std::sync::Arc;

use eo_props::{Model, PropsWorld};

fn world(surveyed_first: bool) -> PropsWorld {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let street = eo_props::load_street_world(&paths).unwrap();
    let std = eo_props::Standards::load(&eo_data::CaStandards::load(&paths).unwrap()).unwrap();
    eo_props::furniture::set_surveyed_first(surveyed_first);
    let w = PropsWorld::new(street, Arc::new(eo_props::JsProps::load().unwrap()), std);
    w.install_furniture(&city.buildings).unwrap();
    eo_props::furniture::set_surveyed_first(false);
    w
}

/// Every 40th street tile of the city, in key order: a citywide spread.
fn sample(w: &PropsWorld) -> Vec<(i32, i32)> {
    let mut keys: Vec<(i32, i32)> = w.street.tiles.keys().copied().collect();
    keys.sort();
    keys.into_iter().step_by(40).collect()
}

fn census(w: &PropsWorld, keys: &[(i32, i32)]) -> BTreeMap<String, usize> {
    let mut c = BTreeMap::new();
    for &k in keys {
        for p in w.plan_tile(k).props {
            *c.entry(format!("{:?}", p.model)).or_insert(0) += 1;
        }
    }
    c
}

#[test]
fn the_surveyed_first_walk_invents_no_rare_or_surveyed_kinds() {
    let off = world(false);
    let on = world(true);
    assert!(!off.furniture().surveyed_first && on.furniture().surveyed_first);
    let keys = sample(&off);
    assert!(keys.len() > 100, "{} tiles", keys.len());
    let (a, b) = (census(&off, &keys), census(&on, &keys));
    let n = |c: &BTreeMap<String, usize>, m: Model| c.get(&format!("{m:?}")).copied().unwrap_or(0);
    println!("{} tiles\n  as ported: {a:?}\n  surveyed first: {b:?}", keys.len());
    // the rare kinds: what is left is observed kit; never more.
    for m in [Model::Bollard, Model::Planter, Model::NewsBox] {
        assert!(n(&b, m) * 4 <= n(&a, m), "{m:?}: {} -> {}", n(&a, m), n(&b, m));
    }
    // street-commerce kinds: only at trading frontages, so fewer; still some.
    for m in [Model::Bench, Model::WasteBin, Model::Mailbox] {
        assert!(n(&b, m) < n(&a, m) && n(&b, m) > 0, "{m:?}: {} -> {}", n(&a, m), n(&b, m));
    }
    // everything else is still the walk's own rule; an emptied slot takes a placed item's step, where the
    // JS walk stepped 1.6 m past a failed placement, so the hashed kinds thin a little (hydrants about a
    // sixth on this sample) and never grow; the surveyed and forced ones stay.
    for m in [Model::Tree, Model::Hydrant, Model::ParkingMeter, Model::SignPost, Model::LampCobra, Model::UtilityBox] {
        let (x, y) = (n(&a, m) as f64, n(&b, m) as f64);
        assert!(x > 0.0 && y >= 0.75 * x && y <= 1.1 * x, "{m:?}: {x} -> {y}");
    }
    for m in [Model::Tree, Model::ParkingMeter, Model::SignPost] {
        assert!((n(&b, m) as f64 - n(&a, m) as f64).abs() <= 0.01 * n(&a, m) as f64, "{m:?}: surveyed or forced, as before");
    }
    // the street lights and the parked cars are not the walk's, but share its kerb.
    let lights = |w: &PropsWorld| keys.iter().map(|&k| w.plan_tile(k).lights.len()).sum::<usize>();
    let parked = |w: &PropsWorld| keys.iter().map(|&k| w.plan_tile(k).parked.len()).sum::<usize>();
    // (a corner without its hashed bollard can leave room for a lamp the corner's occupancy refused)
    let (l0, l1) = (lights(&off) as f64, lights(&on) as f64);
    assert!(l1 >= l0 && l1 <= l0 * 1.01, "street lights {l0} -> {l1}");
    // parking keeps clear of kerbside furniture, so fewer bollards and hydrants free a few spaces.
    let (p0, p1) = (parked(&off) as f64, parked(&on) as f64);
    assert!(p1 >= p0 && p1 <= p0 * 1.1, "parked cars {p0} -> {p1}");
}
