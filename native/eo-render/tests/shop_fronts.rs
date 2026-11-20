//! One vacancy truth on the real city (`--shop-light`, stage N2;
//! shop_lights.rs `build_with_fronts`): with the facade clutter at level 2
//! (its plywood boards on a few fronts), every shop's lights, its front in
//! the rooms' table (the dark room and W5's papered glass read it there) and
//! the clutter's boards agree: a vacant shop (eo-sky `shops::vacant`) is
//! never lit but by its work light, a boarded one is vacant and never lit at
//! all, an occupied one is lit by its hours; a pane on each shop's glass
//! finds its own front, so a strip is mixed shop by shop. Its own test binary:
//! the clutter level is process-wide.

use eo_assets::kit::KitLibrary;
use eo_kit::city::KitCity;
use eo_render::authored::MaterialTable;
use eo_render::kit::KitSet;
use eo_render::shop_lights;
use eo_sky::shops;
use std::collections::{BTreeMap, HashSet};

#[test]
fn the_lights_the_rooms_and_the_boards_agree_on_which_shops_are_vacant() {
    eo_kit::clutter::set_level(2);
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let elev = eo_data::ElevationGrid::load(&paths).unwrap();
    let lib = KitLibrary::load().unwrap();
    let mut materials = MaterialTable::new();
    let mut excluded: HashSet<String> = eo_assets::load_blender_buildings(&paths.repo_root).unwrap().iter().map(|b| b.record_key().to_string()).collect();
    for id in eo_kit::city::load_exclusions() {
        excluded.insert(id.trim_start_matches("sf-building-").to_string());
    }
    let set = KitSet::prepare(&city, &elev, &excluded, 0, &mut materials, &lib);
    eo_data::roof_colours::install(&paths.repo_root);
    let tenancy = eo_kit::tenancy::Tenancy::load(&paths.repo_root).unwrap();
    let kc = KitCity::build(&city, &tenancy, &eo_kit::city::load_exclusions());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    let (lights, fronts, st) = shop_lights::build_with_fronts(&kc, &set, 1, threads);
    let share = st.vacant as f64 / st.shops as f64;
    println!(
        "{} shops: {} vacant ({:.1} %; the rule {:.0} %), {} of them boarded by the facade clutter, {} with a work light; {} fronts in a {}-row table",
        st.shops,
        st.vacant,
        share * 100.0,
        shops::VACANT_SHARE * 100.0,
        st.boarded,
        st.work_lit,
        fronts.len(),
        shop_lights::fronts_table(&fronts).len()
    );
    assert_eq!(fronts.len(), st.shops, "one front a shop");
    assert!(st.boarded > 0, "level 2 boards some fronts");
    assert!((share - shops::VACANT_SHARE).abs() < 0.02, "vacant share {share}");
    // every bay's lights again, by shop: vacant (the rule, or boarded) exactly when never lit by its hours
    let by_shop: BTreeMap<(u32, u32), &shop_lights::Front> = fronts.iter().map(|f| ((f.building, f.tenancy), f)).collect();
    let table = shop_lights::fronts_table(&fronts);
    let mut again = vec![];
    let (mut found, mut panes, mut mixed_buildings) = (0usize, 0usize, 0usize);
    let buildings: std::collections::BTreeSet<u32> = fronts.iter().map(|f| f.building).collect();
    for &b in &buildings {
        let (bays, boards) = shop_lights::bays_and_boards(&kc, b, set.streets.as_deref(), set.base_y[b as usize] as f64, 2);
        let shut: HashSet<u32> = bays.iter().filter(|bay| shop_lights::boarded(bay, &boards)).map(|bay| bay.tenancy).collect();
        let mut states = HashSet::new();
        for bay in &bays {
            let f = by_shop[&(bay.building, bay.tenancy)];
            let key = shops::shop_key(&kc.records[b as usize].id, bay.tenancy);
            assert_eq!(f.key, key);
            let ls = shop_lights::lights_of_front(bay, key, 1, shut.contains(&bay.tenancy));
            for l in &ls {
                let never = l.span == 0.0;
                assert_eq!(never, f.vacant, "{:?}: lights {l:?}", f);
                if f.boarded {
                    assert!(l.security == 0.0, "a boarded front shows no light: {l:?}");
                }
                if f.vacant {
                    assert!(l.security <= shops::VACANT_LIGHT as f32 && (l.security > 0.0) == f.work);
                }
            }
            again.extend(ls);
            states.insert(f.vacant);
            // the rooms: a pane at the middle of this window's glass finds this shop's front
            let n = [bay.normal[0] as f32, 0.0, bay.normal[1] as f32];
            let p = [bay.centre[0] as f32, bay.centre[1] as f32, bay.centre[2] as f32];
            panes += 1;
            if let Some([_, row]) = shop_lights::front_at(&table, f.group, p, n) {
                if row[3] != f.key {
                    let other = fronts.iter().find(|g| g.key == row[3]).unwrap();
                    panic!("the pane {p:?} of {f:?}\n finds {other:?}");
                }
                assert_eq!((row[2] >> 12) & 1, f.vacant as u32, "with its vacancy");
                assert_eq!((row[2] >> 13) & 1, f.boarded as u32);
                found += 1;
            }
        }
        mixed_buildings += (states.len() > 1) as usize;
    }
    // the same lights the placement handed the sky
    let key = |l: &shop_lights::GpuShopLight| (l.pos.map(f32::to_bits), l.span.to_bits(), l.security.to_bits());
    let mut a: Vec<_> = lights.iter().map(key).collect();
    let mut b: Vec<_> = again.iter().map(key).collect();
    a.sort();
    b.sort();
    assert!(a == b, "the placement's lights are these");
    println!("{panes} shop windows: {found} find their own front from their glass; {mixed_buildings} buildings with a vacant and an occupied shop");
    assert!(found * 1000 >= panes * 995, "{found} of {panes}");
    assert!(mixed_buildings > 50, "a strip is mixed shop by shop");
}
