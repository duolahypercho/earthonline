//! the landmark buildings' second generators (towers2.rs) and the named
//! skyline towers (skyline.rs), on the repo's data.

use eo_landmarks::LandmarksJs;
use eo_landmarks::skyline;

fn elevation() -> (eo_data::ElevationGrid, std::path::PathBuf) {
    let paths = eo_data::DataPaths::discover().unwrap();
    (eo_data::ElevationGrid::load(&paths).unwrap(), paths.repo_root)
}

/// Every skyline entry names a facade style of the table, every style is a
/// drawable grid, and the WGSL rows come out in the table's order.
#[test]
fn the_skyline_table_is_whole() {
    let d = skyline::data();
    assert!(!d.buildings.is_empty());
    for b in &d.buildings {
        assert!(skyline::facade_index(&b.facade).is_some(), "{}: no facade style {}", b.id, b.facade);
        assert!(b.height_m > 50.0 && b.floors > 0.0, "{}", b.id);
        let mut last = 0.0;
        for t in &b.tiers {
            assert!(t.to > last && t.to <= 1.0 + 1e-9 && t.along_frac > 0.0 && t.along_frac <= 1.0, "{}: tiers", b.id);
            last = t.to;
        }
    }
    let rows = skyline::facade_rows();
    assert_eq!(rows.len(), d.facades.len());
    for (r, (name, f)) in rows.iter().zip(d.facades.iter()) {
        assert!(f.module_m > 0.0 && f.storey_m > 0.0, "{name}");
        assert!((0.0..=1.0).contains(&f.glaze_x) && (0.0..=1.0).contains(&f.glaze_y), "{name}");
        assert_eq!(r[0], f.module_m as f32, "{name}");
    }
}

/// Every skyline record is in sf-city.json, and each tower stands its
/// published height over its own lowest ground, its crown on top.
#[test]
fn the_skyline_towers_stand_their_published_heights() {
    eo_data::groundscale::pin(1.0);
    let (g, root) = elevation();
    let recs = skyline::load_records(&root).unwrap();
    for b in &skyline::data().buildings {
        let points = recs.get(&b.record).unwrap_or_else(|| panic!("{}: record {} not in sf-city.json", b.id, b.record));
        let ground = |x: f64, z: f64| g.sample(x, z).unwrap_or(0.0);
        let t = skyline::build_tower(b, points, &ground).unwrap();
        let s = &t.landmark.stats;
        let (base, roof, tip) = (s["baseY"].as_f64().unwrap(), s["roofY"].as_f64().unwrap(), s["tipY"].as_f64().unwrap());
        let top_tier = b.tiers.last().unwrap().to;
        assert!((roof - base - b.height_m * top_tier).abs() < 0.01, "{}: roof {} over {}", b.id, roof - base, b.height_m * top_tier);
        assert!(tip >= roof, "{}", b.id);
        assert!((t.landmark.box_max[1] as f64) >= roof - 0.01, "{}: box", b.id);
        assert!(t.landmark.draws.iter().any(|d| d.key == "far" && !d.mesh.is_empty()), "{}: far item", b.id);
        assert_eq!(t.lights.len(), usize::from(b.aviation), "{}: aviation light", b.id);
    }
}

/// The skyline's records are claimed (the kit and the far boxes skip them) only while it is on.
#[test]
fn the_skyline_claims_its_records_only_when_on() {
    let on = eo_landmarks::set::Options { skyline: true, ..eo_landmarks::set::Options::off() };
    let off = eo_landmarks::set::Options { buildings: true, ..eo_landmarks::set::Options::off() };
    let c_on = eo_landmarks::set::claimed_records_with(on);
    let c_off = eo_landmarks::set::claimed_records_with(off);
    for r in skyline::claims() {
        assert!(c_on.contains(&r));
        assert!(!c_off.contains(&r));
    }
    // the registry's claims are the previous set's, unchanged.
    assert_eq!(c_off, eo_landmarks::set::claimed_records());
}

/// The second generators build every landmark building; Salesforce's crown
/// narrows from the top floor's scale to the top's with its walls rounding
/// in (each band narrower than the last, the narrowing growing toward the
/// top); Sutro's legs are tied only at its levels; the new groups carry
/// their own materials and the towers their aviation lights.
#[test]
fn the_second_generators_build_the_five() {
    eo_data::groundscale::pin(1.0);
    let (g, _) = elevation();
    let js = LandmarksJs::get();
    for id in eo_landmarks::towers::BUILDING_IDS {
        let ground = js.registry[id].ground.sampled(&|x, z| g.sample(x, z));
        let v1 = eo_landmarks::towers::build(id, &js.buildings[id], &ground).unwrap();
        let v2 = eo_landmarks::towers2::build(id, &js.buildings[id], &ground).unwrap();
        assert!(v1.materials.is_empty() && v1.lights.is_empty(), "{id}: the previous generator is unchanged");
        assert!(v2.version.ends_with("+towers2"), "{id}");
        for (key, _) in &v2.groups {
            assert!(v1.groups.iter().any(|(k, _)| k == key), "{id}: group {key} is the registry's");
        }
        let tris: usize = v2.groups.iter().map(|(_, f)| f.triangles()).sum();
        let budget = js.registry[id].budget["triangles"].as_f64().unwrap() as usize;
        assert!(tris <= budget, "{id}: {tris} triangles over the registry's budget {budget}");
        match id {
            "transamerica-pyramid" | "salesforce-tower" => {
                assert!(v2.materials.iter().any(|(k, m)| k == "glass" && m.surface.as_deref() == Some("authored")), "{id}: glazing");
                assert_eq!(v2.lights.len(), 1, "{id}: aviation light");
            }
            "sutro-tower" => assert_eq!(v2.lights.len(), 6, "{id}: aviation lights"),
            _ => assert!(v2.materials.iter().any(|(_, m)| m.surface_options.as_ref().is_some_and(|o| o["flood"].is_array())), "{id}: floodlit"),
        }
    }
    // Salesforce: the crown's half-widths, bottom to top.
    let id = "salesforce-tower";
    let ground = js.registry[id].ground.sampled(&|x, z| g.sample(x, z));
    let o = eo_landmarks::towers2::build(id, &js.buildings[id], &ground).unwrap();
    let crown = &o.groups.iter().find(|(k, _)| k == "crown").unwrap().1;
    let flat: Vec<f64> = js.buildings[id]["record"]["points"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    let c = eo_landmarks::geometry::ring_centroid(&eo_landmarks::geometry::pairs(&flat));
    // the crown's walls (the pearl), not its lip or the well inside.
    let pearl = eo_landmarks::geometry::srgb_hex(eo_landmarks::towers2::SALESFORCE_PEARL_SRGB);
    let mut by_y: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for (p, col) in crown.positions.chunks_exact(3).zip(crown.colors.chunks_exact(3)) {
        if (col[0] as f64 - pearl[0]).abs() > 1e-4 {
            continue;
        }
        let r = (p[0] as f64 - c[0]).hypot(p[2] as f64 - c[1]);
        let e = by_y.entry((p[1] as f64 * 10.0).round() as i64).or_insert(0.0);
        *e = e.max(r);
    }
    let widths: Vec<f64> = by_y.values().copied().collect();
    assert!(widths.len() > 10);
    let shrink: Vec<f64> = widths.windows(2).map(|w| w[0] - w[1]).collect();
    assert!(shrink.iter().take(shrink.len() - 3).all(|d| *d > -1e-3), "the crown narrows upward");
    let n = shrink.len();
    assert!(shrink[n / 2 - 1] < shrink[n - 5], "the crown rounds in toward its top");
}
