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
    // the previous table's rows are the v2 data's, whatever the switch.
    assert_eq!(skyline::facade_rows_for(false).len(), skyline::data_for(false).facades.len());
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
        let v2 = eo_landmarks::towers2::build_v(id, &js.buildings[id], &ground, false).unwrap();
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
    // Salesforce: the crown's half-widths, bottom to top (the third pass's lofted crown; the fourth's fins below).
    let id = "salesforce-tower";
    let ground = js.registry[id].ground.sampled(&|x, z| g.sample(x, z));
    let o = eo_landmarks::towers3::build_v(id, &js.buildings[id], &ground, false).unwrap();
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
    // the fourth pass: its fins stand their depth proud of that crown at every height, its panels stop under the top.
    let o4 = eo_landmarks::towers3::build_v(id, &js.buildings[id], &ground, true).unwrap();
    assert!(o4.version.ends_with("+towers4"));
    let fins = &o4.groups.iter().find(|(k, _)| k == "crown").unwrap().1;
    let screen = &o4.groups.iter().find(|(k, _)| k == "screen").unwrap().1;
    let base = o4.stats["baseY"].as_f64().unwrap();
    let (top_floor, h) = (js.buildings[id]["dimensions"]["topFloorM"].as_f64().unwrap(), js.buildings[id]["dimensions"]["heightM"].as_f64().unwrap());
    let r_max = |m: &eo_landmarks::Frozen, y0: f64, y1: f64| m.positions.chunks_exact(3).filter(|p| (p[1] as f64) >= y0 && (p[1] as f64) <= y1).map(|p| (p[0] as f64 - c[0]).hypot(p[2] as f64 - c[1])).fold(0.0, f64::max);
    for f in [0.1, 0.4, 0.7] {
        let y = base + top_floor + f * (h - top_floor);
        let (a, b) = (r_max(crown, y - 1.5, y + 1.5), r_max(fins, y - 1.5, y + 1.5));
        assert!(b > a + 0.3 && b < a + eo_landmarks::towers3::CROWN_FIN_DEPTH_M + 0.6, "fins {b} beside the crown {a} at {f}");
    }
    let screen_top = screen.positions.chunks_exact(3).map(|p| p[1] as f64).fold(f64::MIN, f64::max);
    assert!((screen_top - (base + top_floor + eo_landmarks::towers3::CROWN_SCREEN_TOP * (h - top_floor))).abs() < 0.5, "the panels stop under the top");
    assert!(o4.stats["crownFins"].as_u64().unwrap() > 80, "a fin a module round the crown");
}

/// The towers' aviation lights stand beside the bridges' (`tower_lights`):
/// one on each tall skyline tower, Transamerica's apex, Salesforce's top and
/// six on Sutro; none with both off.
#[test]
fn the_towers_carry_their_aviation_lights() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let elevation = eo_data::ElevationGrid::load(&paths).unwrap();
    let ca = eo_data::CaStandards::load(&paths).unwrap();
    let on = eo_landmarks::set::Options { towers2: true, skyline: true, ..eo_landmarks::set::Options::default() };
    let set = eo_landmarks::LandmarkSet::build_with(&paths.repo_root, &|x, z| elevation.sample(x, z), &ca, on).unwrap();
    let sky = skyline::data().buildings.iter().filter(|b| b.aviation).count();
    // the aviation lights by their red (the fourth pass adds the grove's white loop lamps round Coit Tower).
    let (red, white): (Vec<&eo_landmarks::LandmarkLight>, Vec<&eo_landmarks::LandmarkLight>) = set.tower_lights.iter().partition(|l| l.colour_linear[1] < 0.3 * l.colour_linear[0]);
    assert_eq!(red.len(), sky + 1 + 1 + 6);
    for l in &red {
        assert!(l.pos[1] > 100.0, "an aviation light on a tower top, not at {}", l.pos[1]);
    }
    assert_eq!(white.is_empty(), !eo_landmarks::towers4_on(), "{} loop lamps", white.len());
    let off = eo_landmarks::set::Options { towers2: false, skyline: false, ..eo_landmarks::set::Options::default() };
    let set = eo_landmarks::LandmarkSet::build_with(&paths.repo_root, &|x, z| elevation.sample(x, z), &ca, off).unwrap();
    // no aviation light (the groves are their own switch: their loop lamps may stay).
    assert!(set.tower_lights.iter().all(|l| l.colour_linear[1] >= 0.3 * l.colour_linear[0]));
}

/// The third pass (towers3.rs, `eo-app --no-towers3` off) builds the five
/// within their budgets, in the registry's groups; the floodlit ones carry
/// a projector rig (not the lit band) and the rest keep their materials.
#[test]
fn the_third_pass_builds_the_five() {
    eo_data::groundscale::pin(1.0);
    let (g, _) = elevation();
    let js = LandmarksJs::get();
    let (_, ranges) = eo_landmarks::towers3::projector_rows();
    for id in eo_landmarks::towers::BUILDING_IDS {
        let ground = js.registry[id].ground.sampled(&|x, z| g.sample(x, z));
        let v1 = eo_landmarks::towers::build(id, &js.buildings[id], &ground).unwrap();
        let v3 = eo_landmarks::towers2::build_v(id, &js.buildings[id], &ground, true).unwrap();
        assert!(v3.version.ends_with("+towers3"), "{id}: {}", v3.version);
        for (key, _) in &v3.groups {
            assert!(v1.groups.iter().any(|(k, _)| k == key), "{id}: group {key} is the registry's");
        }
        let tris: usize = v3.groups.iter().map(|(_, f)| f.triangles()).sum();
        let budget = js.registry[id].budget["triangles"].as_f64().unwrap() as usize;
        assert!(tris <= budget, "{id}: {tris} triangles over the registry's budget {budget}");
        let rigged: Vec<&str> = v3.materials.iter().filter_map(|(k, m)| m.surface_options.as_ref().filter(|o| o["rig"].is_array()).map(|_| k.as_str())).collect();
        assert!(v3.materials.iter().all(|(_, m)| m.surface_options.as_ref().is_none_or(|o| o["flood"].is_null())), "{id}: no lit band in the third pass");
        match id {
            "coit-tower" => assert!(rigged.contains(&"concrete"), "{id}: floodlit"),
            "city-hall" => assert!(rigged.contains(&"stone") && rigged.contains(&"dome"), "{id}: floodlit"),
            "transamerica-pyramid" => assert!(rigged.contains(&"metal"), "{id}: the spire lit from within"),
            _ => assert!(rigged.is_empty(), "{id}"),
        }
        for (_, m) in &v3.materials {
            if let Some(r) = m.surface_options.as_ref().and_then(|o| o["rig"].as_array()) {
                let (first, count) = (r[0].as_u64().unwrap() as u32, r[2].as_u64().unwrap() as u32);
                assert!(count > 0 && ranges.iter().any(|(_, f, c)| *f == first && *c == count), "{id}: the rig is in the table");
            }
        }
    }
    // Coit: the third pass's flutes cut deeper than the export's.
    let id = "coit-tower";
    let ground = js.registry[id].ground.sampled(&|x, z| g.sample(x, z));
    let o = eo_landmarks::towers2::build_v(id, &js.buildings[id], &ground, true).unwrap();
    let export_depth = js.buildings[id]["dimensions"]["fluteDepth"].as_f64().unwrap() * js.buildings[id]["dimensions"]["shaftRadiusM"].as_f64().unwrap();
    assert!(o.stats["fluteDepthM"].as_f64().unwrap() > 2.0 * export_depth);
}

/// Every projector stands outside the surface it lights, aims at it, and
/// gives its aim illuminance there (E = I cos / d^2): the rows the shader
/// reads carry the cone's cosines in order (field <= beam).
#[test]
fn the_projector_rigs_are_physical() {
    let (rows, ranges) = eo_landmarks::towers3::projector_rows();
    assert_eq!(ranges.len(), 3);
    let total: u32 = ranges.iter().map(|r| r.2).sum();
    assert_eq!(rows.len() as u32, total);
    for r in &rows {
        assert!(r[3] > 0.0, "candela");
        let aim = (r[4] * r[4] + r[5] * r[5] + r[6] * r[6]).sqrt();
        assert!((aim - 1.0).abs() < 1e-3, "unit aim");
        assert!(r[7] <= r[8] && r[8] <= 1.0 && r[7] >= 0.0, "field {} beam {}", r[7], r[8]);
        assert!((0.0..=1.0).contains(&r[9]), "spill");
    }
    // the ground and roof floods aim up (none down onto a roof).
    for (id, first, count) in &ranges {
        for r in &rows[*first as usize..(*first + *count) as usize] {
            assert!(r[5] > 0.0, "{id}: a flood aimed up");
        }
    }
}

/// The pane's albedo and metalness reproduce its reflectance at normal
/// incidence and the room's diffuse in the city shader's metalness model;
/// the third pass's table carries them and the previous table its tints.
#[test]
fn the_glazing_is_a_dielectric_pane() {
    for (r, d) in [(0.08, 0.02), (0.12, 0.03), (0.15, 0.025)] {
        let (a, m) = skyline::pane_albedo_metal(r, d);
        let f0 = 0.04 * (1.0 - m) + a * m;
        assert!((f0 - r).abs() < 1e-3, "F0 {f0} for {r}");
        assert!((a * (1.0 - m) - d).abs() < 1e-4, "diffuse for {d}");
    }
    let v2 = skyline::data_for(false);
    let v3 = skyline::data_for(true);
    assert!(v2.facades.values().all(|f| f.reflectance == 0.0), "the previous table has no reflectance");
    assert!(v3.facades.values().filter(|f| f.pattern == "grid" && f.glaze_x > 0.0).all(|f| f.reflectance > 0.0));
    // 555 California's granite: darker and less saturated than the previous tone.
    let b2 = v2.buildings.iter().find(|b| b.id == "skyline-555-california").unwrap();
    let b3 = v3.buildings.iter().find(|b| b.id == "skyline-555-california").unwrap();
    let (c2, c3) = (skyline::hex_linear(&b2.stone_srgb), skyline::hex_linear(&b3.stone_srgb));
    let sat = |c: [f64; 3]| (c[0] - c[2]) / c[0];
    assert!(sat(c3) < 0.5 * sat(c2), "desaturated");
    assert!(b3.stone_roughness <= 0.25, "polished");
    let rows = skyline::facade_rows_for(true);
    let i = skyline::data_for(true).facades.keys().position(|k| k == "fremont-181").unwrap();
    assert!(rows[i][12] > 0.5 && rows[i][15] > 0.0, "pane metalness and window luminance");
    // Phase 3: v4 LM_FACADES row 5 carries diverse cool/warm nightTemp (r5.z) and nightAccent (r5.w) while v2/v3 stay 0.
    let r2 = skyline::facade_rows_at(2);
    let r3 = skyline::facade_rows_at(3);
    let r4 = skyline::facade_rows_at(4);
    assert!(r2.iter().all(|r| r[22] == 0.0 && r[23] == 0.0), "v2 keeps row 5 z/w zero");
    assert!(r3.iter().all(|r| r[22] == 0.0 && r[23] == 0.0), "v3 keeps row 5 z/w zero");
    assert!(r4.iter().any(|r| r[22] <= -0.8), "v4 includes cool 6000K tech glass towers");
    assert!(r4.iter().any(|r| r[22] >= 0.75), "v4 includes warm 2500K residential/historic towers");
    assert!(r4.iter().any(|r| (r[23] - 2.0).abs() < 1e-3), "v4 includes Embarcadero champagne outline accent");
    assert!(r4.iter().any(|r| (r[23] - 3.0).abs() < 1e-3) && r4.iter().any(|r| (r[23] - 4.0).abs() < 1e-3), "v4 includes red/green pierhead beacons");
}

/// The expanded 70-tower San Francisco skyline builds cleanly within lean Near/Far
/// triangle budgets, preserves crowns/spires/X-braces in `Role::Far` with separate
/// roof/metal draws, claims overlapping extra records (e.g. MIRA's 120 Folsom), and
/// keeps every vertex/normal finite.
#[test]
fn the_expanded_sf_skyline_has_rich_silhouettes_and_lean_budgets() {
    eo_data::groundscale::pin(1.0);
    let (g, root) = elevation();
    let d = skyline::data();
    assert!(d.buildings.len() >= 60, "expected >= 60 SF skyline towers, got {}", d.buildings.len());
    let recs = skyline::load_records(&root).unwrap();
    let ground = |x: f64, z: f64| g.sample(x, z).unwrap_or(0.0);
    let (mut near_tris, mut far_tris, mut total_ms) = (0usize, 0usize, 0.0f64);
    for b in &d.buildings {
        let points = recs.get(&b.record).unwrap_or_else(|| panic!("{}: missing {}", b.id, b.record));
        let built = skyline::build_tower(b, points, &ground).unwrap_or_else(|| panic!("{}: build_tower failed", b.id));
        total_ms += built.landmark.build_ms;
        for dr in &built.landmark.draws {
            assert!(dr.mesh.positions.iter().all(|v| v.is_finite()), "{}: non-finite pos in {}", b.id, dr.key);
            assert!(dr.mesh.normals.iter().all(|v| v.is_finite()), "{}: non-finite nrm in {}", b.id, dr.key);
            match dr.role {
                eo_landmarks::Role::Near => near_tris += dr.mesh.triangles(),
                eo_landmarks::Role::Far => far_tris += dr.mesh.triangles(),
                eo_landmarks::Role::Always => {}
            }
        }
        // Key metal-crown / braced towers preserve metal silhouettes in Role::Far.
        if matches!(
            b.id.as_str(),
            "skyline-181-fremont" | "skyline-345-california" | "skyline-millennium-tower" | "skyline-one-rincon-hill" | "skyline-100-first" | "skyline-505-montgomery" | "skyline-one-maritime"
        ) {
            assert!(
                built.landmark.draws.iter().any(|dr| dr.key == "far-metal" && dr.role == eo_landmarks::Role::Far && !dr.mesh.is_empty()),
                "{}: missing far-metal silhouette draw",
                b.id
            );
        }
        // Far facade wall vertices carry `stone` (not double-darkened `far_c`).
        let stone = skyline::hex_linear(&b.stone_srgb);
        let far_draw = built.landmark.draws.iter().find(|dr| dr.key == "far").unwrap();
        let has_stone = far_draw.mesh.colors.chunks_exact(3).any(|c| (c[0] as f64 - stone[0]).abs() < 0.05 && (c[1] as f64 - stone[1]).abs() < 0.05);
        assert!(has_stone, "{}: far draw does not preserve stone tone for lm_authored", b.id);
        // Round 9 regression guard: night accent-edge ribbons strictly hug the real tier parapet (`<= roofY + 0.75 m`, never floating at `tipY`!).
        let roof_y = built.landmark.stats["roofY"].as_f64().unwrap();
        for dr in &built.landmark.draws {
            if dr.key.contains("crown-edge") || dr.key.contains("embarcadero-edge") {
                assert!(
                    dr.mesh.max_y <= roof_y + 0.75,
                    "{}: accent draw {} floats above roofY ({:.1} > {:.1})",
                    b.id,
                    dr.key,
                    dr.mesh.max_y,
                    roof_y + 0.75
                );
            }
        }
        // Embarcadero 4 tight fixed-metre cornice & accent bounds (no multiplicative slab-end ears).
        if b.id == "skyline-embarcadero-4" {
            let far_max_r = far_draw
                .mesh
                .positions
                .chunks_exact(3)
                .map(|p| ((p[0] as f64) - built.landmark.anchor[0]).hypot((p[2] as f64) - built.landmark.anchor[1]))
                .fold(0.0f64, f64::max);
            for dr in &built.landmark.draws {
                if dr.key == "roof" || dr.key == "metal" || dr.key.contains("embarcadero-edge") {
                    let dr_max_r = dr
                        .mesh
                        .positions
                        .chunks_exact(3)
                        .map(|p| ((p[0] as f64) - built.landmark.anchor[0]).hypot((p[2] as f64) - built.landmark.anchor[1]))
                        .fold(0.0f64, f64::max);
                    assert!(
                        dr_max_r <= far_max_r + 1.10,
                        "skyline-embarcadero-4: draw {} protrudes {:.2} m past facade ({:.2} vs {:.2})",
                        dr.key,
                        dr_max_r - far_max_r,
                        dr_max_r,
                        far_max_r
                    );
                }
            }
        }
    }
    println!("skyline ({} towers): near {} tris, far {} tris, build {:.1} ms", d.buildings.len(), near_tris, far_tris, total_ms);
    assert!(near_tris < 250_000, "skyline near triangle budget exceeded: {near_tris}");
    assert!(far_tris < 65_000, "skyline far triangle budget exceeded: {far_tris}");
    // Extra overlapping record (MIRA's 120 Folsom) is claimed only while skyline is on.
    assert!(skyline::claims().iter().any(|r| r == "693630796"));
    // Salesforce v4 far mesh stays within registry's 600-triangle far budget.
    let js = LandmarksJs::get();
    let sf_ground = js.registry["salesforce-tower"].ground.sampled(&|x, z| g.sample(x, z));
    let sf4 = eo_landmarks::towers3::build_v("salesforce-tower", &js.buildings["salesforce-tower"], &sf_ground, true).unwrap();
    let sf_far_budget = js.registry["salesforce-tower"].budget["farTriangles"].as_f64().unwrap() as usize;
    assert!(sf4.far.triangles() > 0 && sf4.far.triangles() <= sf_far_budget, "salesforce v4 far {} > {sf_far_budget}", sf4.far.triangles());
}

/// Historic Embarcadero waterfront pier bulkheads & low-rise landmarks (`waterfront`)
/// build cleanly with finite geometry, pitched roof silhouettes in Near and Far, no
/// aviation lights, lean triangle budgets, and clean `--no-skyline` record claiming.
#[test]
fn the_embarcadero_waterfront_landmarks_build_cleanly() {
    eo_data::groundscale::pin(1.0);
    let (g, root) = elevation();
    let d = skyline::data();
    assert!(d.waterfront.len() >= 15, "expected >= 15 waterfront landmarks, got {}", d.waterfront.len());
    let recs = skyline::load_records(&root).unwrap();
    let claims = skyline::claims();
    let ground = |x: f64, z: f64| g.sample(x, z).unwrap_or(0.0);
    let (mut near_tris, mut far_tris) = (0usize, 0usize);
    for w in &d.waterfront {
        assert!(skyline::facade_index(&w.facade).is_some(), "{}: unknown facade {}", w.id, w.facade);
        assert!(w.height_m > 0.0 && w.height_m <= 50.0, "{}: invalid waterfront height {}", w.id, w.height_m);
        assert!(!w.aviation, "{}: waterfront landmark must not carry aviation light", w.id);
        assert!(claims.iter().any(|r| r == &w.record), "{}: record {} not claimed", w.id, w.record);
        let points = recs.get(&w.record).unwrap_or_else(|| panic!("{}: missing {}", w.id, w.record));
        let built = skyline::build_tower(w, points, &ground).unwrap_or_else(|| panic!("{}: build_tower failed", w.id));
        assert!(built.lights.is_empty(), "{}: unexpected lights", w.id);
        assert!(built.landmark.draws.iter().any(|dr| dr.key == "far" && !dr.mesh.is_empty()), "{}: missing far draw", w.id);
        for dr in &built.landmark.draws {
            assert!(dr.mesh.positions.iter().all(|v| v.is_finite()), "{}: non-finite pos in {}", w.id, dr.key);
            assert!(dr.mesh.normals.iter().all(|v| v.is_finite()), "{}: non-finite nrm in {}", w.id, dr.key);
            match dr.role {
                eo_landmarks::Role::Near => near_tris += dr.mesh.triangles(),
                eo_landmarks::Role::Far => far_tris += dr.mesh.triangles(),
                eo_landmarks::Role::Always => {}
            }
        }
    }
    println!("waterfront ({} landmarks): near {} tris, far {} tris", d.waterfront.len(), near_tris, far_tris);
    assert!(near_tris < 35_000, "waterfront near triangle budget exceeded: {near_tris}");
    assert!(far_tris < 15_000, "waterfront far triangle budget exceeded: {far_tris}");
    // Bogus DataSF ferry-gangway/promenade polygons beside Ferry Building & Pier 1 are claimed only while skyline is on.
    for bogus in ["ds8789", "ds9288", "ds5521", "ds855"] {
        assert!(claims.iter().any(|r| r == bogus), "expected bogus ferry-gangway footprint {bogus} to be claimed");
    }
    // Roof/trim draws carry the `precast` (bayM=0) weathering contract for landmarks_surface.wgsl.
    let ag = d.waterfront.iter().find(|w| w.id == "waterfront-agriculture-bldg").unwrap();
    let ag_built = skyline::build_tower(ag, recs.get(&ag.record).unwrap(), &ground).unwrap();
    let ag_roof = ag_built.landmark.draws.iter().find(|dr| dr.key == "roof").unwrap();
    assert_eq!(ag_roof.material.surface.as_deref(), Some("precast"));
    // Moored Bay vessels (incl. glossy glass cabin/pilothouse windows) and Ferry Building night illumination build cleanly.
    let vessels = skyline::build_moored_bay_vessels(-0.55);
    assert_eq!(vessels.len(), 3);
    for v in &vessels {
        assert!(v.draws.iter().any(|dr| dr.role == eo_landmarks::Role::Near && !dr.mesh.is_empty()), "{}: near", v.id);
        assert!(v.draws.iter().any(|dr| dr.role == eo_landmarks::Role::Far && !dr.mesh.is_empty()), "{}: far", v.id);
        assert!(v.draws.iter().any(|dr| dr.key == "glass" && !dr.mesh.is_empty()), "{}: glossy glass", v.id);
        for dr in &v.draws {
            assert!(dr.mesh.positions.iter().all(|p| p.is_finite()), "{}: non-finite", v.id);
            assert!(dr.mesh.normals.iter().all(|n| n.is_finite()), "{}: non-finite nrm", v.id);
        }
    }
    let fb_night = skyline::build_ferry_night_illumination(&|x, z| g.sample(x, z)).expect("ferry night glow");
    assert!(fb_night.draws.iter().any(|dr| dr.role == eo_landmarks::Role::Near && !dr.mesh.is_empty()));
    assert!(fb_night.draws.iter().any(|dr| dr.role == eo_landmarks::Role::Far && !dr.mesh.is_empty()));
    for dr in &fb_night.draws {
        assert!(dr.mesh.positions.iter().all(|p| p.is_finite()), "ferry-night: non-finite");
        assert!(dr.mesh.normals.iter().all(|n| n.is_finite()), "ferry-night: non-finite nrm");
    }
}
