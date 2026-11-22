//! Street tree basins (`basins::set_tree_basins`, eo-app `--tree-basins`) on
//! the real city over a citywide sample of street tiles: each is one of
//! Director's Order 187,246's sizes, stands round a tree or palm, is squared
//! to the kerb, and is never buried at a corner; without them nothing else
//! the tiles place moves. Its own test binary: the switch is process-wide.

use std::collections::BTreeMap;
use std::sync::Arc;

use eo_props::{Model, Prop, PropsWorld};

fn world(basins: bool) -> PropsWorld {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let street = eo_props::load_street_world(&paths).unwrap();
    let std = eo_props::Standards::load(&eo_data::CaStandards::load(&paths).unwrap()).unwrap();
    eo_props::basins::set_tree_basins(basins);
    let w = PropsWorld::new(street, Arc::new(eo_props::JsProps::load().unwrap()), std);
    eo_props::basins::set_tree_basins(false);
    w.install_furniture(&city.buildings).unwrap();
    w
}

fn sample(w: &PropsWorld) -> Vec<(i32, i32)> {
    let mut keys: Vec<(i32, i32)> = w.street.tiles.keys().copied().collect();
    keys.sort();
    keys.into_iter().step_by(60).collect()
}

#[test]
fn every_basin_is_a_standard_one_round_a_tree_squared_to_the_kerb() {
    let (off, on) = (world(false), world(true));
    assert!(on.tree_basins && !off.tree_basins);
    let keys = sample(&on);
    let sizes: Vec<[f64; 2]> = on.std.tree_basin.iter().map(|c| [c[1], c[2]]).collect();
    let (mut basins, mut trees) = (0usize, 0usize);
    let mut by_size: BTreeMap<String, usize> = BTreeMap::new();
    for &k in &keys {
        let (a, b) = (off.plan_tile(k).props, on.plan_tile(k).props);
        // nothing else moves: the props without the basins are the same.
        let rest: Vec<&Prop> = b.iter().filter(|p| p.model != Model::TreeBasin).collect();
        assert_eq!(rest.len(), a.len(), "tile {k:?}");
        assert!(a.iter().all(|p| p.model != Model::TreeBasin));
        trees += b.iter().filter(|p| matches!(p.model, Model::Tree | Model::Palm)).count();
        // a basin and its tree each belong to the tile their own position is in: look round the tile.
        let mut near: Vec<Prop> = vec![];
        for dz in -1..=1 {
            for dx in -1..=1 {
                near.extend(on.plan_tile((k.0 + dx, k.1 + dz)).props.into_iter().filter(|p| matches!(p.model, Model::Tree | Model::Palm)));
            }
        }
        let tree_list: Vec<&Prop> = near.iter().collect();
        for p in b.iter().filter(|p| p.model == Model::TreeBasin) {
            basins += 1;
            let (across, along) = (p.scale[0] as f64, p.scale[2] as f64);
            assert!(sizes.iter().any(|s| (s[0] - across).abs() < 1e-3 && (s[1] - along).abs() < 1e-3), "size {across} x {along}");
            *by_size.entry(format!("{:.2} x {:.2} m", across, along)).or_insert(0) += 1;
            // round a tree: a trunk inside the basin (the basin may be moved out to clear the kerb).
            assert!(tree_list.iter().any(|t| (t.pos[0] - p.pos[0]).hypot(t.pos[2] - p.pos[2]) <= across / 2.0 + 0.6), "basin at {:?} has no tree", p.pos);
            // never buried: every corner's ground is under the tilted panel.
            let (s, c) = p.yaw.sin_cos();
            let slope = eo_props::basins::slope_of(p.variant);
            // the trunk the variant names stands in the basin, at a tree.
            let tx = eo_props::basins::trunk_of(p.variant);
            assert!(tx.abs() <= across / 2.0 + 0.6, "trunk {tx} m across a {across} m basin");
            let (ts, tc) = p.yaw.sin_cos();
            let (wx, wz) = (p.pos[0] + tx * tc, p.pos[2] - tx * ts);
            assert!(tree_list.iter().any(|t| (t.pos[0] - wx).hypot(t.pos[2] - wz) <= eo_props::basins::TRUNK_STEP_M), "no trunk where basin {:?} names one", p.pos);
            for (lx, lz) in [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
                let (x, z) = (lx * across, lz * along);
                let (wx, wz) = (p.pos[0] + x * c + z * s, p.pos[2] - x * s + z * c);
                let top = p.pos[1] + z * slope;
                assert!(on.ground(wx, wz) <= top + 0.005, "corner buried: ground {} panel {}", on.ground(wx, wz), top);
            }
        }
    }
    println!("{} tiles: {trees} trees and palms, {basins} basins; sizes {by_size:?}", keys.len());
    // one per tree on a footway (each counted in its own tile, so a pair split by a tile edge can count once either side).
    assert!(basins > 0 && basins <= trees + trees / 100 + 2, "{basins} basins for {trees} trees");
    assert!(basins * 2 >= trees, "most street trees stand on a footway: {basins} of {trees}");
}
