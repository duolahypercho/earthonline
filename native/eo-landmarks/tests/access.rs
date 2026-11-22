//! Landmark access (access.rs): every deck is built on its landmark, at the
//! published height, walkable where its view stands, fenced by guards at
//! least the code height, the same bit for bit when built again; and with
//! the switch off the set is built exactly as before.

use eo_landmarks::access::{AccessSet, GUARD_COLLIDE_M, Tri};
use eo_landmarks::set::Options;
use eo_landmarks::{LandmarkSet, LandmarksJs};
use std::sync::OnceLock;

struct World {
    paths: eo_data::DataPaths,
    elevation: eo_data::ElevationGrid,
    ca: eo_data::CaStandards,
}

fn world() -> &'static World {
    static W: OnceLock<World> = OnceLock::new();
    W.get_or_init(|| {
        let paths = eo_data::DataPaths::discover().unwrap();
        let elevation = eo_data::ElevationGrid::load(&paths).unwrap();
        let ca = eo_data::CaStandards::load(&paths).unwrap();
        World { paths, elevation, ca }
    })
}

fn access() -> AccessSet {
    let w = world();
    AccessSet::build(&|x, z| w.elevation.sample(x, z))
}

/// Whether (x, z) lies in a triangle's plan, and the triangle's height there.
fn on_triangle(t: &Tri, x: f64, z: f64) -> Option<f64> {
    let (a, b, c) = (t[0], t[1], t[2]);
    let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if d.abs() < 1e-12 {
        return None;
    }
    let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
    let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
    let l3 = 1.0 - l1 - l2;
    (l1 >= -1e-9 && l2 >= -1e-9 && l3 >= -1e-9).then(|| l1 * a[1] + l2 * b[1] + l3 * c[1])
}

#[test]
fn every_deck_is_walkable_at_its_view_and_fenced() {
    let set = access();
    let ids: Vec<&str> = set.decks.iter().map(|d| d.id.as_str()).collect();
    for want in ["coit-gallery", "salesforce-ohana", "transamerica-48", "golden-gate-east-sidewalk", "golden-gate-south-tower", "bay-bridge-w2", "sutro-platform", "city-hall-lantern"] {
        assert!(ids.contains(&want), "no deck {want} in {ids:?}");
    }
    for d in &set.decks {
        assert!(!d.walk.is_empty() && !d.barrier.is_empty() && !d.structures.is_empty(), "{}: empty", d.id);
        let [x, y, z] = d.view.feet;
        let under = d.walk.iter().filter_map(|t| on_triangle(t, x, z)).fold(f64::NEG_INFINITY, f64::max);
        assert!((under - y).abs() < 0.02, "{}: the view's feet ({y:.3}) are not on the deck's floor ({under:.3})", d.id);
        // Every barrier reaches the collision guard over the floor near it.
        let tops = d.barrier.iter().map(|t| t.iter().map(|p| p[1]).fold(f64::MIN, f64::max)).fold(f64::MIN, f64::max);
        assert!(tops >= d.floor_y + GUARD_COLLIDE_M - 0.25, "{}: the barriers top out at {tops:.2}, the floor is {:.2}", d.id, d.floor_y);
        let tris: usize = d.structures.iter().map(|s| s.mesh.triangles()).sum();
        println!("{}: {tris} drawn triangles, {} walkable and {} barrier collision triangles, {} lifts, {} lamps", d.id, d.walk.len(), d.barrier.len(), d.lifts.len(), d.lights.len());
        for l in &d.lifts {
            assert!(l.top.feet[1] - l.bottom.feet[1] > 40.0, "{}: the lift rises only {:.1} m", d.id, l.top.feet[1] - l.bottom.feet[1]);
            let [tx, ty, tz] = l.top.feet;
            let under = d.walk.iter().filter_map(|t| on_triangle(t, tx, tz)).fold(f64::NEG_INFINITY, f64::max);
            assert!((under - ty).abs() < 0.03, "{}: the lift's top stop is not on the deck ({ty:.3} vs {under:.3})", d.id);
        }
    }
}

#[test]
fn decks_stand_at_the_published_heights() {
    let js = LandmarksJs::get();
    let w = world();
    let full = LandmarkSet::build(&w.paths.repo_root, &|x, z| w.elevation.sample(x, z), &w.ca).unwrap();
    let a = full.access.as_ref().expect("the default set builds the decks");
    let stat = |id: &str, k: &str| full.landmarks.iter().find(|l| l.id == id).unwrap().stats[k].as_f64().unwrap();
    let deck = |id: &str| a.deck(id).unwrap();
    let water = js.sea_level_y;
    // Golden Gate: the tower top 746 ft (227.4 m) over the water (goldengate.org),
    // times the structure scale the bridge is drawn at (golden_gate.rs `data`).
    let gg = deck("golden-gate-south-tower");
    let h = eo_landmarks::golden_gate::structure_scale();
    assert!((gg.floor_y - (water + 746.0 * 0.3048 * h)).abs() < 0.1, "south tower deck at {:.2}", gg.floor_y);
    // Sutro: the platform 751 ft (228.9 m) over the base, the deck on its frame.
    let su = deck("sutro-platform");
    assert!((su.floor_y - stat("sutro-tower", "baseY") - 751.0 * 0.3048).abs() < 1.5, "Sutro deck {:.2} over base {:.2}", su.floor_y, stat("sutro-tower", "baseY"));
    // Salesforce: the 61st floor, its ceiling under the crown (1,070 ft roof).
    let sf = deck("salesforce-ohana");
    let sf_base = stat("salesforce-tower", "baseY");
    let storey = 277.9776 / 61.0;
    assert!((sf.floor_y - sf_base - 60.0 * storey).abs() < 0.1, "the observation level is not floor 61: {:.2}", sf.floor_y - sf_base);
    assert!(stat("salesforce-tower", "crownFromY") - sf.floor_y < 4.6, "the crown starts {:.2} m over the floor", stat("salesforce-tower", "crownFromY") - sf.floor_y);
    // Transamerica: the 48th floor, one storey under the spire's base.
    let ta = deck("transamerica-48");
    let under_spire = stat("transamerica-pyramid", "spireFromY") - ta.floor_y;
    assert!((3.5..4.5).contains(&under_spire), "the room's floor is {under_spire:.2} m under the spire");
    // Coit: the deck in the arcade, 32 ft (9.8 m) under the top as published;
    // the LiDAR arcade puts its sills a guard height over a floor 12 m down.
    let co = deck("coit-gallery");
    let below_top = stat("coit-tower", "topY") - co.floor_y;
    assert!((9.0..13.0).contains(&below_top), "Coit's gallery is {below_top:.2} m under the top");
    // City Hall: the gallery round the lantern's foot, under the 307.5 ft dome top.
    let ch = deck("city-hall-lantern");
    let under_top = stat("city-hall", "domeTopY") - ch.floor_y;
    assert!((8.0..16.0).contains(&under_top), "the lantern gallery is {under_top:.2} m under the dome top");
    println!("decks: GG tower {:.1}, Sutro {:.1}, Salesforce {:.1}, Transamerica {:.1}, Coit {:.1} ({below_top:.1} under the top), City Hall {:.1}", gg.floor_y, su.floor_y, sf.floor_y, ta.floor_y, co.floor_y, ch.floor_y);
}

#[test]
fn decks_are_the_same_bit_for_bit_when_built_again() {
    let (a, b) = (access(), access());
    for (x, y) in a.decks.iter().zip(&b.decks) {
        assert_eq!(x.walk, y.walk, "{}", x.id);
        assert_eq!(x.barrier, y.barrier, "{}", x.id);
        assert_eq!(x.lifts, y.lifts, "{}", x.id);
        assert_eq!(x.cuts, y.cuts, "{}", x.id);
    }
}

#[test]
fn with_access_off_the_set_is_built_as_before() {
    let w = world();
    let h = |x: f64, z: f64| w.elevation.sample(x, z);
    let off = LandmarkSet::build_with(&w.paths.repo_root, &h, &w.ca, Options { access: false, ..Default::default() }).unwrap();
    let on = LandmarkSet::build(&w.paths.repo_root, &h, &w.ca).unwrap();
    assert!(off.access.is_none());
    assert!(!off.structures.iter().any(|s| s.name.starts_with("access:")), "access structures with access off");
    assert!(on.structures.iter().any(|s| s.name.starts_with("access:")));
    assert_eq!(on.lights.len() > off.lights.len(), true, "the decks' lamps");
    // The towers: with access off exactly the generator's meshes (the second
    // generators' with `towers2`, as the set builds them); on, the opened
    // bands only (Coit's painted openings are gone, the rest is equal).
    let js = LandmarksJs::get();
    for id in eo_landmarks::towers::BUILDING_IDS {
        let d = eo_landmarks::set::ground_scale_shift(js, id);
        let shifted = |x: f64, z: f64| h(x + d[0], z + d[1]);
        let g = eo_landmarks::set::ground_of(js, id, &shifted);
        let plain = if Options::default().towers2 { eo_landmarks::towers2::build(id, &js.buildings[id], &g) } else { eo_landmarks::towers::build(id, &js.buildings[id], &g) }.unwrap();
        let built = off.landmarks.iter().find(|l| l.id == id).unwrap();
        for (key, f) in &plain.groups {
            if f.is_empty() {
                continue;
            }
            let draw = built.draws.iter().find(|dr| &dr.key == key || dr.mesh.name.ends_with(&format!(":{key}"))).unwrap_or_else(|| panic!("{id}: no draw {key}"));
            assert_eq!(draw.mesh.indices.len(), f.indices.len(), "{id}:{key} differs with access off");
        }
    }
    let coit_on = on.landmarks.iter().find(|l| l.id == "coit-tower").unwrap();
    let coit_off = off.landmarks.iter().find(|l| l.id == "coit-tower").unwrap();
    let openings = |l: &eo_landmarks::Landmark| l.draws.iter().find(|d| d.key == "openings").map(|d| d.mesh.triangles()).unwrap_or(0);
    // the belvedere's twelve painted arches (two triangles and one more each) go; the lantern's eight stay.
    assert_eq!(openings(coit_off) - openings(coit_on), 12 * 3, "Coit's painted arches with access off only");
}

#[test]
fn every_tower_generator_opens_the_bands_the_decks_replace() {
    // the first generators (towers.rs), the second (towers2.rs), the third
    // and the fourth (towers3.rs): each opened leaves out the same bands.
    let w = world();
    let h = |x: f64, z: f64| w.elevation.sample(x, z);
    let js = LandmarksJs::get();
    type Gen = fn(&str, &serde_json::Value, &eo_landmarks::geometry::GroundGrid, bool) -> Option<eo_landmarks::towers::Out>;
    let gens: [(&str, Gen); 4] = [
        ("towers", |id, s, g, open| eo_landmarks::towers::build_opened(id, s, g, open)),
        ("towers2", |id, s, g, open| eo_landmarks::towers2::build_v_opened(id, s, g, false, open)),
        ("towers3", |id, s, g, open| eo_landmarks::towers3::build_v_opened(id, s, g, false, open)),
        ("towers4", |id, s, g, open| eo_landmarks::towers3::build_v_opened(id, s, g, true, open)),
    ];
    let tris = |o: &eo_landmarks::towers::Out, key: &str| o.groups.iter().find(|(k, _)| k == key).map(|(_, f)| f.triangles()).unwrap_or(0);
    for (name, make) in gens {
        for (id, key) in [("coit-tower", "openings"), ("salesforce-tower", "glass"), ("transamerica-pyramid", "glass"), ("transamerica-pyramid", "precast")] {
            let d = eo_landmarks::set::ground_scale_shift(js, id);
            let shifted = |x: f64, z: f64| h(x + d[0], z + d[1]);
            let g = eo_landmarks::set::ground_of(js, id, &shifted);
            let (shut, open) = (make(id, &js.buildings[id], &g, false).unwrap(), make(id, &js.buildings[id], &g, true).unwrap());
            assert_ne!(tris(&shut, key), tris(&open, key), "{name}: {id}:{key} is not opened");
            assert!(open.far.triangles() > 0, "{name}: {id} has no far item opened");
        }
    }
}

/// Whether the ray from `o` along `d` crosses the planar quad `q` ahead of `o`.
fn crosses(o: [f64; 3], d: [f64; 3], q: &[[f64; 3]; 4]) -> bool {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let n = cross(sub(q[1], q[0]), sub(q[3], q[0]));
    let den = dot(n, d);
    if den.abs() < 1e-12 {
        return false;
    }
    let t = dot(n, sub(q[0], o)) / den;
    if t <= 0.0 {
        return false;
    }
    let p = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
    let s: Vec<f64> = (0..4).map(|i| dot(cross(sub(q[(i + 1) % 4], q[i]), sub(p, q[i])), n)).collect();
    s.iter().all(|v| *v >= -1e-9) || s.iter().all(|v| *v <= 1e-9)
}

#[test]
fn the_enclosed_decks_meter_as_rooms_their_views_leave_by_an_opening() {
    let set = access();
    for id in ["coit-gallery", "salesforce-ohana", "transamerica-48"] {
        let d = set.decks.iter().find(|d| d.id == id).unwrap();
        let room = d.interior.as_ref().unwrap_or_else(|| panic!("{id}: no room for the camera meter"));
        assert!(room.sky > 0.05 && room.sky < 0.6, "{id}: the room's mean sky visibility {:.3}", room.sky);
        assert!(!room.portals.is_empty(), "{id}: no openings");
        // the view's eye stands in the room, and its look leaves through an opening.
        let [x, y, z] = d.view.feet;
        let eye = [x, y + eo_landmarks::access::EYE_M, z];
        assert!(eye[1] > room.y0 && eye[1] < room.y1, "{id}: the eye's height is outside the room");
        let h = d.view.heading_deg.to_radians();
        let dir = [h.sin(), 0.0, -h.cos()];
        assert!(room.portals.iter().any(|q| crosses(eye, dir, q)), "{id}: the view does not leave by an opening");
    }
    // the open-air decks have no room.
    for id in ["golden-gate-east-sidewalk", "golden-gate-south-tower", "bay-bridge-w2", "sutro-platform", "city-hall-lantern"] {
        assert!(set.decks.iter().find(|d| d.id == id).unwrap().interior.is_none(), "{id}: an open deck metered as a room");
    }
}

#[test]
fn interior_surfaces_carry_their_baked_sky_and_coit_is_glazed_but_its_view_arch() {
    let set = access();
    let coit = set.decks.iter().find(|d| d.id == "coit-gallery").unwrap();
    let baked: Vec<&eo_landmarks::set::Structure> = coit.structures.iter().filter(|s| s.mesh.sky.is_some()).collect();
    assert!(!baked.is_empty(), "the gallery's interior is not baked");
    for s in baked {
        let sky = s.mesh.sky.as_ref().unwrap();
        assert_eq!(sky.len() * 3, s.mesh.positions.len(), "{}: one visibility per vertex", s.name);
        assert!(sky.iter().all(|v| (0.0..=1.0).contains(v)), "{}: a visibility outside 0..1", s.name);
    }
    // the acrylic panes: eleven glazed arches of twelve.
    let glass = coit.structures.iter().find(|s| s.material.draw.as_deref() == Some("glass-pane")).expect("Coit's panes");
    assert!(!glass.cast, "glass casts no sun shadow");
    let v = coit.vantage.expect("the view's vantage at the arch's face");
    // the vantage looks out of the open arch: no pane between it and the view.
    let h = coit.view.heading_deg.to_radians();
    let dir = [h.sin() as f32, 0.0f32, -h.cos() as f32];
    let m = &glass.mesh;
    let hit = m.indices.chunks_exact(3).any(|t| {
        let p = |i: u32| [m.positions[3 * i as usize], m.positions[3 * i as usize + 1], m.positions[3 * i as usize + 2]];
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        // Moller-Trumbore, a hit within 3 m.
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let pv = [dir[1] * e2[2] - dir[2] * e2[1], dir[2] * e2[0] - dir[0] * e2[2], dir[0] * e2[1] - dir[1] * e2[0]];
        let det = e1[0] * pv[0] + e1[1] * pv[1] + e1[2] * pv[2];
        if det.abs() < 1e-9 {
            return false;
        }
        let o = [v[0] as f32, v[1] as f32, v[2] as f32];
        let tv = [o[0] - a[0], o[1] - a[1], o[2] - a[2]];
        let u = (tv[0] * pv[0] + tv[1] * pv[1] + tv[2] * pv[2]) / det;
        let qv = [tv[1] * e1[2] - tv[2] * e1[1], tv[2] * e1[0] - tv[0] * e1[2], tv[0] * e1[1] - tv[1] * e1[0]];
        let w = (dir[0] * qv[0] + dir[1] * qv[1] + dir[2] * qv[2]) / det;
        let t = (e2[0] * qv[0] + e2[1] * qv[1] + e2[2] * qv[2]) / det;
        (0.0..=1.0).contains(&u) && w >= 0.0 && u + w <= 1.0 && t > 0.0 && t < 3.0
    });
    assert!(!hit, "a pane stands in the view's arch");
}
