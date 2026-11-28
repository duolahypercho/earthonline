//! The street trees' switches (`furniture::set_tree_keep`, `set_tree_sizes`,
//! `set_tree_species`; eo-app `--tree-keep`, `--tree-sizes`, `--tree-species`)
//! on the real city: the Folsom Street (4th-5th) list trees the walk dropped
//! are placed; every tree `--tree-keep` adds on a citywide sample stands clear
//! of kerb ramps, driveways, carriageway and buildings; `--tree-sizes` joins
//! the list to the current DataSF records and sizes a tree from its trunk;
//! `--tree-species` draws a tree as its genus's street form. Its own test
//! binary: the modes are process-wide.
//!
//!   cargo test --release -p eo-props --test street_trees -- --nocapture

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use eo_props::furniture::{TreeFate, set_tree_keep, set_tree_sizes, set_tree_species};
use eo_props::{Model, PropsWorld};

fn world(keep: bool, sizes: bool, species: bool) -> PropsWorld {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let street = eo_props::load_street_world(&paths).unwrap();
    let std = eo_props::Standards::load(&eo_data::CaStandards::load(&paths).unwrap()).unwrap();
    set_tree_keep(keep);
    set_tree_sizes(sizes);
    set_tree_species(species);
    let w = PropsWorld::new(street, Arc::new(eo_props::JsProps::load().unwrap()), std);
    w.install_furniture(&city.buildings).unwrap();
    set_tree_keep(false);
    set_tree_sizes(false);
    set_tree_species(false);
    w
}

/// What became of list tree `id` on the ribbon that owns it.
fn fate(w: &PropsWorld, id: u32) -> Option<TreeFate> {
    let f = w.furniture();
    let ei = f.tree_owner(w, id)?;
    f.entry(w, ei as usize).tree_fates.iter().find(|t| t.id == id).copied()
}

/// The ribbons of every 60th street tile of the city, in key order: a citywide spread.
fn sample_entries(w: &PropsWorld) -> Vec<u32> {
    let mut keys: Vec<(i32, i32)> = w.street.tiles.keys().copied().collect();
    keys.sort();
    let mut out: Vec<u32> = keys.into_iter().step_by(60).flat_map(|k| w.street.tiles[&k].spans.iter().map(|s| s.0).collect::<Vec<_>>()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[test]
fn street_tree_switches() {
    let off = world(false, false, false);
    let on = world(true, true, true);
    assert!(!off.furniture().tree_keep && on.furniture().tree_keep && on.furniture().tree_sizes && on.furniture().tree_species);

    // Folsom Street, 4th to 5th: the list trees the walk dropped (an observed overlay's absent
    // windows, the joint between two pieces at the bus island) are placed; the one in the 5th
    // Street corner's kerb ramp is not.
    for id in [22995u32, 31698, 31697, 31696, 19744, 28597, 18901] {
        let (a, b) = (fate(&off, id).map(|t| t.why), fate(&on, id).map(|t| t.why));
        assert_ne!(a, Some("placed"), "list tree {id} was placed before");
        assert_eq!(b, Some("placed"), "list tree {id}: {b:?}");
    }
    assert_eq!(fate(&on, 14958).map(|t| t.why), Some("in-kerb-ramp"), "list tree 14958 stands in the 5th Street ramp");

    // Every tree --tree-keep adds on a citywide sample stands clear of kerb ramps (with their
    // clear zone), driveways, carriageway and buildings.
    let entries = sample_entries(&off);
    assert!(entries.len() > 300, "{} ribbons", entries.len());
    let (fo, fn_) = (off.furniture().clone(), on.furniture().clone());
    let clear = fo.js.options.kerb_ramp_clear_m;
    let mouths = eo_props::parking::kerb_mouths(&on);
    let rules = eo_props::parking::rules(&on);
    let (mut placed_off, mut placed_on, mut added) = (0usize, 0usize, 0usize);
    let mut problems: BTreeMap<&str, usize> = BTreeMap::new();
    for &ei in &entries {
        let before: HashMap<u32, TreeFate> = fo.entry(&off, ei as usize).tree_fates.iter().map(|t| (t.id, *t)).collect();
        let e = &on.street.plan.entries[ei as usize];
        let ramps = eo_street::plan::segment_kerb_ramps(e, &on.street.o);
        let plan = fn_.entry(&on, ei as usize);
        placed_off += before.values().filter(|t| t.why == "placed").count();
        for t in plan.tree_fates.iter().filter(|t| t.why == "placed") {
            placed_on += 1;
            if before.get(&t.id).is_some_and(|b| b.why == "placed") {
                continue;
            }
            added += 1;
            let it = plan.items.iter().find(|it| (it.x - t.at[0]).abs() < 1e-9 && (it.z - t.at[1]).abs() < 1e-9).expect("the placed item");
            let r = fn_.kinds[it.kind as usize].radius;
            let wins = &ramps[if it.side > 0 { 0 } else { 1 }];
            let in_ramp = wins.iter().any(|w| {
                let lo = (if w.flare_lo.is_finite() { w.flare_lo.min(w.lo) } else { w.lo }) - clear;
                let hi = (if w.flare_hi.is_finite() { w.flare_hi.max(w.hi) } else { w.hi }) + clear;
                it.station > lo && it.station < hi
            });
            let drive = on.std.no_parking_driveway;
            let in_mouth = mouths.get(&(ei, it.side)).is_some_and(|v| v.iter().any(|m| it.station + r > m[0] + drive && it.station - r < m[1] - drive));
            let checks = [
                ("in a kerb ramp's span", in_ramp),
                ("in a driveway", in_mouth || rules.on_crossover(ei as usize, it.side, it.station, r)),
                ("on the carriageway", on.roadway.hit(it.x, it.z, 0.0).is_some()),
                ("in a building", on.street.buildings.contains(it.x, it.z)),
            ];
            for (what, bad) in checks {
                if bad {
                    *problems.entry(what).or_insert(0) += 1;
                }
            }
        }
    }
    println!("{} ribbons: list trees placed {placed_off} -> {placed_on}, {added} added; problems {problems:?}", entries.len());
    assert!(placed_on > placed_off && added > 0, "{placed_off} -> {placed_on}");
    assert!(problems.is_empty(), "added trees: {problems:?}");

    // --tree-sizes: the list joins the current records; a Folsom elm (DataSF 85744, Ulmus
    // parvifolia, 3 in) takes the Berkeley Chinese elm's crown and height for 3 in.
    let j = fn_.tree_join.as_ref().expect("the join");
    assert!(j.joined * 100 > j.record.len() * 99, "joined {} of {}", j.joined, j.record.len());
    assert!(j.sized * 100 > j.record.len() * 85, "sized {} of {}", j.sized, j.record.len());
    let elm = 26461u32;
    let k = j.record[elm as usize].expect("joined") as usize;
    assert_eq!(j.table.trees[k].treeid, 85744);
    let [crown, height] = j.size[elm as usize].unwrap();
    let (c, h) = j.table.size(k).unwrap();
    assert_eq!((crown, height), (c as f32, h as f32));
    // --tree-species: drawn as the street elm, its leaves as wide as the allometry's crown.
    let t = fate(&on, elm).unwrap();
    let ids = eo_props::eztree::street_tree_species().unwrap();
    let widths = eo_props::eztree::street_crown_widths();
    assert_eq!(ids[t.variant as usize], "street-elm");
    let drawn = widths[t.variant as usize] * t.scale[0] as f64;
    assert!((drawn - c).abs() < 0.05, "drawn crown {drawn:.2} m, allometry {c:.2} m");
    // and before, the survey's form and size (an overlay's "full", 7.6 m), drawn wider.
    let b = fate(&off, elm).unwrap();
    assert_eq!(ids[b.variant as usize], "street-full");
    assert!(drawn < widths[b.variant as usize] * b.scale[0] as f64);
    // palms stay palms.
    let palms = |w: &PropsWorld| entries.iter().map(|&ei| w.furniture().entry(w, ei as usize).props.iter().filter(|p| p.model == Model::Palm).count()).sum::<usize>();
    assert!(palms(&on) >= palms(&off) && palms(&off) > 0);
}
