//! Lit shop windows (`--shop-light`, stage N1; shop_lights.rs), on the real
//! city: the kit planner's shop bays give lights that stand just inside
//! their building's front, behind its glass, along the commercial streets
//! (Castro at 18th, Mission at 24th), in the trades' mix; the same twice.
//! The GPU part (the gather, the ranking, the rooms) is in tests/sky.rs's
//! neighbour `shop_lights_gpu` below.

use eo_assets::kit::KitLibrary;
use eo_kit::city::KitCity;
use eo_render::authored::MaterialTable;
use eo_render::kit::KitSet;
use eo_render::shop_lights;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

struct Fixture {
    city: Arc<eo_data::City>,
    kc: KitCity,
    set: KitSet,
}

fn fixture() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| {
        let paths = eo_data::DataPaths::discover().unwrap();
        let city = Arc::new(eo_data::City::load(&paths).unwrap());
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
        Fixture { city, kc, set }
    })
}

/// A surveyed (lat, lon) on the scaled plan.
fn at(city: &eo_data::City, lat: f64, lon: f64) -> [f64; 2] {
    let f = eo_data::Frame::from_meta(&city.meta);
    eo_data::groundscale::plan([(lon - f.center_lon) * f.m_per_deg_lon, -(lat - f.center_lat) * f.m_per_deg_lat])
}

/// Distance from (x, z) to the ring's nearest edge, and whether it is inside.
fn ring_distance(ring: &[[f32; 2]], x: f64, z: f64) -> (f64, bool) {
    let n = ring.len();
    let (mut best, mut inside) = (f64::INFINITY, false);
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (ax, az, bx, bz) = (a[0] as f64, a[1] as f64, b[0] as f64, b[1] as f64);
        let (dx, dz) = (bx - ax, bz - az);
        let t = (((x - ax) * dx + (z - az) * dz) / (dx * dx + dz * dz).max(1e-12)).clamp(0.0, 1.0);
        best = best.min((x - ax - dx * t).hypot(z - az - dz * t));
        if (az > z) != (bz > z) && x < ax + (z - az) / (bz - az) * dx {
            inside = !inside;
        }
    }
    (best, inside)
}

#[test]
fn the_kit_shop_bays_light_the_commercial_streets_from_behind_their_glass() {
    let f = fixture();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    let (lights, st) = shop_lights::build(&f.kc, &f.set, 1, threads);
    println!(
        "{} kit buildings with a retail ground planned, {} drew {} shop bays of {} shops: {} lights at level 1 in {:.0} ms on {threads} threads; kinds {:?}",
        st.planned, st.with_bays, st.bays, st.shops, st.lights, st.ms, st.by_kind
    );
    assert!(st.shops > 1000 && st.lights == st.bays, "{st:?}");
    // level 2: two or more a bay, the same bays.
    let (two, st2) = shop_lights::build(&f.kc, &f.set, 2, threads);
    assert_eq!((st2.bays, st2.shops), (st.bays, st.shops));
    assert!(two.len() >= 2 * lights.len(), "{} vs {}", two.len(), lights.len());
    // deterministic: the same lights on another thread count.
    let (again, _) = shop_lights::build(&f.kc, &f.set, 1, 3);
    assert_eq!(again, lights, "the same lights whatever the threads");
    // every kind of trade is present; most shops are retail, food is next.
    let kinds: std::collections::BTreeMap<&str, usize> = st.by_kind.iter().cloned().collect();
    for k in ["restaurant", "cafe", "bar", "market", "retail", "salon"] {
        assert!(kinds.get(k).copied().unwrap_or(0) > 20, "{k}: {kinds:?}");
    }
    // the commercial blocks: Castro at 18th (the reference photo's corner), Mission at 24th.
    for (name, lat, lon) in [("Castro and 18th", 37.760841, -122.434922), ("Mission and 24th", 37.752427, -122.418415)] {
        let c = at(&f.city, lat, lon);
        let near: Vec<&shop_lights::GpuShopLight> = lights.iter().filter(|l| (l.pos[0] as f64 - c[0]).hypot(l.pos[2] as f64 - c[1]) < 150.0).collect();
        let mut by_kind: std::collections::BTreeMap<u32, usize> = Default::default();
        for l in &near {
            *by_kind.entry(l.kind).or_default() += 1;
        }
        println!("{name} ({:.0}, {:.0}): {} shop lights within 150 m, by kind {by_kind:?}", c[0], c[1], near.len());
        assert!(near.len() >= 40, "{name}: {} lights", near.len());
        assert!(by_kind.len() >= 3, "{name}: a mix of trades {by_kind:?}");
    }
    // each bay's glass lies on its building's front (within the glass's setback and the reveal), its light inside the footprint.
    let (mut checked, mut worst, mut outside) = (0usize, 0.0f64, 0usize);
    for b in 0..f.kc.records.len() as u32 {
        if checked >= 600 || f.set.building_group[b as usize] == eo_render::scene::NO_GROUP {
            continue;
        }
        let bays = shop_lights::bays_of(&f.kc, b, None, f.set.base_y[b as usize] as f64);
        let ring = &f.city.buildings[b as usize].ring;
        for bay in &bays {
            let (d, _) = ring_distance(ring, bay.centre[0], bay.centre[2]);
            worst = worst.max(d);
            let l = shop_lights::lights_of(bay, 1, 1)[0];
            let (_, inside) = ring_distance(ring, l.pos[0] as f64, l.pos[2] as f64);
            if !inside {
                outside += 1;
            }
            checked += 1;
            // the glass's middle stands between the sill and the head over the kit's floor.
            assert!(bay.height > 0.3 && bay.centre[1] > f.set.base_y[b as usize] as f64, "{bay:?}");
        }
    }
    println!("{checked} bays checked: the glass at most {worst:.2} m off its front, {outside} lights outside their footprint");
    // the planner lifts its shop bays by the base it is given and decides nothing else by it (bays_of plans at 0).
    let mut lifted = 0;
    for b in 0..f.kc.records.len() as u32 {
        if lifted >= 40 || f.set.building_group[b as usize] == eo_render::scene::NO_GROUP {
            continue;
        }
        let rec = &f.kc.records[b as usize];
        if !eo_kit::pipeline::classify_loaded(rec).is_some_and(|c| c.has_retail_ground) {
            continue;
        }
        let bays_at = |base: f64| -> Vec<(f64, f64, f64, f64)> {
            eo_kit::pipeline::plan_building(rec, &f.kc.world, b, base)
                .and_then(|p| p.plan.extra.get("shopBays").and_then(|v| v.as_array()).cloned())
                .unwrap_or_default()
                .iter()
                .map(|v| (v["s0"].as_f64().unwrap_or(0.0), v["s1"].as_f64().unwrap_or(0.0), v["y0"].as_f64().unwrap_or(0.0), v["y1"].as_f64().unwrap_or(0.0)))
                .collect()
        };
        let (a, z) = (bays_at(37.25), bays_at(0.0));
        if a.is_empty() {
            continue;
        }
        assert_eq!(a.len(), z.len());
        for (p, q) in a.iter().zip(&z) {
            assert!((p.0 - q.0).abs() < 1e-9 && (p.1 - q.1).abs() < 1e-9 && (p.2 - q.2 - 37.25).abs() < 1e-6 && (p.3 - q.3 - 37.25).abs() < 1e-6, "{p:?} vs {q:?}");
        }
        lifted += 1;
    }
    assert!(lifted >= 40, "{lifted} buildings' bays compared at two bases");
    assert!(checked >= 600 && worst < 0.6 && outside * 50 <= checked, "{checked} bays, {worst:.2} m, {outside} outside");
}

/// On the street world (slice 4), a shop's lights stand on the floor the kit
/// cooks its building on (`kit::kit_floor`, the street front's walk), and the
/// placement's cost a building (the plan, the floor).
#[test]
fn the_lights_stand_on_the_kits_street_floors() {
    let f = fixture();
    let paths = eo_data::DataPaths::discover().unwrap();
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let o = eo_street::StreetOptions::load().unwrap();
    let sc = eo_street::StreetCity::load().unwrap();
    let plan = eo_street::plan::build_plan(&sc, &o);
    let rings = f.city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let lift = o.open_ground_lift;
    let sw = eo_street::StreetWorld::from_plan(o, sc, plan, eo_street::Terrain { grid, carpet_lift: lift }, eo_street::BuildingIndex::new(rings));
    let c = at(&f.city, 37.760841, -122.434922);
    let near: Vec<u32> = (0..f.kc.records.len() as u32)
        .filter(|&b| f.set.building_group[b as usize] != eo_render::scene::NO_GROUP)
        .filter(|&b| f.city.buildings[b as usize].ring.first().is_some_and(|p| (p[0] as f64 - c[0]).hypot(p[1] as f64 - c[1]) < 400.0))
        .filter(|&b| eo_kit::pipeline::classify_loaded(&f.kc.records[b as usize]).is_some_and(|c| c.has_retail_ground))
        .collect();
    let t = std::time::Instant::now();
    let mut with_streets = vec![];
    for &b in &near {
        with_streets.push(shop_lights::bays_of(&f.kc, b, Some(&sw), 0.0));
    }
    let placed_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = std::time::Instant::now();
    let (mut checked, mut worst) = (0, 0.0f64);
    for (&b, bays) in near.iter().zip(&with_streets) {
        if bays.is_empty() {
            continue;
        }
        let (floor, _, _) = eo_render::kit::kit_floor(&f.kc, &sw, b);
        let flat = shop_lights::bays_of(&f.kc, b, None, floor.y);
        assert_eq!(flat.len(), bays.len());
        for (p, q) in bays.iter().zip(&flat) {
            worst = worst.max((p.centre[1] - q.centre[1]).abs());
            checked += 1;
        }
    }
    let kit_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "{} retail buildings within 400 m of Castro and 18th: bays placed on the street floors in {placed_ms:.0} ms ({:.1} ms a building); {checked} bays within {worst:.4} m of the kit's own floor (kit_floor and a second plan: {kit_ms:.0} ms)",
        near.len(),
        placed_ms / near.len().max(1) as f64
    );
    assert!(checked > 100 && worst < 1e-6, "{checked} bays, {worst} m");
}
