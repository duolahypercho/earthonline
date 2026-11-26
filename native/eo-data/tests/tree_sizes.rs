//! The street trees' trunk sizes and their allometry, as shipped
//! (`public/data/sf/sf-tree-sizes.json.gz`, `sf-tree-allometry.json`).
//!
//!   cargo test --release -p eo-data --test tree_sizes -- --nocapture
use eo_data::tree_sizes::{Sizing, TreeSizes};

fn load() -> TreeSizes {
    TreeSizes::load(&eo_data::DataPaths::discover().unwrap()).unwrap()
}

#[test]
fn the_list_loads_whole_and_in_the_city() {
    let t = load();
    assert!(t.trees.len() > 120_000, "{} trees", t.trees.len());
    let with_dbh = t.trees.iter().filter(|r| r.dbh_in.is_some()).count();
    assert!(with_dbh * 10 > t.trees.len() * 8, "DBH on {with_dbh} of {}", t.trees.len());
    // the city's local frame: San Francisco is about 12 km across round the centre.
    assert!(t.trees.iter().all(|r| r.x.abs() < 12_000.0 && r.z.abs() < 12_000.0));
    // ids are unique and sorted (the builder's order).
    assert!(t.trees.windows(2).all(|w| w[0].treeid < w[1].treeid));
}

#[test]
fn a_folsom_street_elm_sizes_from_its_record() {
    let t = load();
    // DataSF tree 100568: Ulmus parvifolia, 3 in, 3x3 plot, planted 2007-05-07, 868 Folsom St.
    let i = t.trees.iter().position(|r| r.treeid == 100568).expect("tree 100568");
    let r = t.trees[i];
    assert!(t.species[r.species as usize].starts_with("Ulmus parvifolia"));
    assert_eq!((r.dbh_in, r.planted), (Some(3.0), Some(20070507)));
    assert_eq!(t.plots[r.plot.unwrap() as usize], "3x3");
    // found again by its own position, and sized by the NoCalC Chinese elm record.
    assert_eq!(t.nearest(r.x + 0.3, r.z - 0.3, 1.0), Some(i));
    let Sizing::Record(e) = t.sizing[r.species as usize] else { panic!("not sized by a record") };
    assert_eq!(t.allometry[e].key, "NoCalC/Ulmus parvifolia");
    let (crown, height) = t.size(i).unwrap();
    assert_eq!((crown, height), t.allometry[e].size(3.0 * 2.54));
    // a 7.6 cm elm: about a 4 m crown on a 5 m tree (PSW-GTR-253's Berkeley curves).
    assert!((3.5..4.5).contains(&crown) && (4.8..6.0).contains(&height), "{crown} {height}");
}

#[test]
fn every_equation_is_finite_and_grows_over_the_measured_range() {
    let t = load();
    for a in &t.allometry {
        let lo = a.dbh_cm[0].unwrap_or(2.5).max(2.5);
        let hi = a.dbh_cm[1].unwrap_or(100.0);
        let (c0, h0) = a.size(lo);
        let (c1, h1) = a.size(hi);
        assert!(c0.is_finite() && h0.is_finite() && c0 > 0.0 && h0 > 0.0, "{} at {lo} cm: {c0} {h0}", a.key);
        assert!(c1 >= c0 * 0.95 && h1 >= h0 * 0.95, "{}: {c0} -> {c1} m crown, {h0} -> {h1} m tall over {lo}..{hi} cm", a.key);
        assert!(c1 < 40.0 && h1 < 60.0, "{}: {c1} {h1}", a.key);
    }
}

#[test]
fn species_match_as_documented() {
    let t = load();
    let m = |name: &str| {
        let i = t.species.iter().position(|s| s.starts_with(name)).unwrap_or_else(|| panic!("{name}"));
        (t.species_match[i].clone(), t.sizing[i])
    };
    let key = |s: Sizing| match s {
        Sizing::Record(i) => t.allometry[i].key.clone(),
        other => format!("{other:?}"),
    };
    let (lvl, s) = m("Magnolia grandiflora :: ");
    assert_eq!((lvl.as_str(), key(s).as_str()), ("species/NoCalC", "NoCalC/Magnolia grandiflora"));
    let (lvl, s) = m("Platanus x hispanica");
    assert_eq!((lvl.as_str(), key(s).as_str()), ("species/NoCalC", "NoCalC/Platanus x acerifolia"));
    let (lvl, s) = m("Lophostemon confertus");
    assert_eq!((lvl.as_str(), key(s).as_str()), ("species/SoCalC", "SoCalC/Tristaniopsis conferta"));
    let (lvl, s) = m("Tristaniopsis laurina :: ");
    assert_eq!((lvl.as_str(), key(s).as_str()), ("family-myrtaceae", "SoCalC/Melaleuca quinquenervia"));
    let (lvl, s) = m("Washingtonia robusta");
    assert_eq!((lvl.as_str(), s), ("palm", Sizing::None));
    let (lvl, s) = m("Ligustrum lucidum");
    assert_eq!((lvl.as_str(), s), ("broadleaf-median", Sizing::BroadleafMedian));
    // the broadleaf median lies within its records' spread.
    let lig = t.species.iter().position(|s| s.starts_with("Ligustrum lucidum")).unwrap() as u32;
    let (c, h) = t.species_size(lig, 8.0).unwrap();
    assert!((2.0..12.0).contains(&c) && (3.0..14.0).contains(&h), "{c} {h}");
}
