//! The facade clutter (stage W3, `--facade-clutter`): off draws what was
//! drawn before; on, the same bytes on every thread and in any order;
//! nothing through another element, the plan's own parts or past what may
//! stand out over the sidewalk; roof pieces inside the roof; lit faces only
//! on signs; fire escapes where the city has them.

use eo_kit::city::KitCity;
use eo_kit::clutter::{self, Bx};
use eo_kit::pipeline::{Planned, RungMesh, build_rung, build_rung_with, plan_building};
use std::collections::BTreeMap;
use std::sync::OnceLock;

fn city() -> &'static KitCity {
    static C: OnceLock<KitCity> = OnceLock::new();
    C.get_or_init(|| {
        let root = eo_kit::repo_root().expect("repo root");
        KitCity::load(&root).expect("city").1
    })
}

fn bytes(m: &RungMesh) -> Vec<u8> {
    let mut out = vec![];
    for (k, (name, tint, sink)) in m.groups.iter().enumerate() {
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&tint.unwrap_or(u32::MAX).to_le_bytes());
        out.push(m.glow.get(k).copied().unwrap_or(0));
        for v in &sink.positions {
            for c in v {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        for n in &sink.normals {
            for c in n {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        for i in &sink.indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
    }
    out
}

fn planned(i: usize) -> Option<Planned> {
    let kc = city();
    if kc.excluded.contains(&i) {
        return None;
    }
    plan_building(&kc.records[i], &kc.world, i as u32, 3.25)
}

fn district(p: &Planned) -> String {
    p.classified.civic.as_ref().and_then(|c| c.district.clone()).unwrap_or_default()
}

#[test]
fn off_draws_what_was_drawn_before() {
    assert_eq!(clutter::level(), 0, "the game's level is off unless eo-app sets it");
    assert!(clutter::cache_tag().is_empty() && clutter::tag_for(0).is_empty(), "off: the kit tile cache key is today's");
    assert!(!clutter::tag_for(1).is_empty() && clutter::tag_for(1) != clutter::tag_for(2));
    let kc = city();
    let mut n = 0;
    for i in (0..kc.records.len()).step_by(1499) {
        let Some(p) = planned(i) else { continue };
        for rung in [0, 1, 2, 3] {
            let a = build_rung(&p, rung);
            let b = build_rung_with(&p, rung, 0);
            assert!(a.glow.is_empty() && b.glow.is_empty(), "#{i}: no glow without the clutter");
            assert_eq!(bytes(&a), bytes(&b), "#{i} rung {rung}");
        }
        let cp = clutter::plan(&p, 0);
        assert!(cp.boxes.is_empty() && cp.roof_boxes.is_empty() && cp.replaced.is_empty());
        n += 1;
    }
    assert!(n > 80, "{n}");
}

#[test]
fn the_same_clutter_on_every_thread_and_in_any_order() {
    let kc = city();
    let picks: Vec<usize> = (0..kc.records.len()).step_by(613).filter(|i| !kc.excluded.contains(i)).collect();
    let mesh = |i: usize, level: u8| -> Vec<u8> {
        let Some(p) = planned(i) else { return vec![] };
        let mut out = bytes(&build_rung_with(&p, 0, level));
        out.extend(bytes(&build_rung_with(&p, 2, level)));
        out
    };
    for level in [1u8, 2] {
        let first: Vec<Vec<u8>> = picks.iter().map(|&i| mesh(i, level)).collect();
        let second: Vec<(usize, Vec<u8>)> = std::thread::scope(|s| {
            let chunks: Vec<Vec<usize>> = picks.iter().rev().copied().collect::<Vec<_>>().chunks(picks.len().div_ceil(8)).map(|c| c.to_vec()).collect();
            let hs: Vec<_> = chunks.into_iter().map(|c| s.spawn(move || c.into_iter().map(|i| (i, mesh(i, level))).collect::<Vec<_>>())).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let differing = second.iter().filter(|(i, b)| &first[picks.iter().position(|p| p == i).unwrap()] != b).count();
        assert!(picks.len() > 250);
        assert_eq!(differing, 0, "level {level}");
    }
}

#[test]
fn nothing_through_another_nor_past_the_sidewalk() {
    let kc = city();
    let l = clutter::limits();
    assert!(l.read, "the street standards table is read");
    // (sidewalk-width-sf-neighborhood-residential minimum 10 ft) - (edge zone offset 18 in).
    assert!((l.street_reach_m - (3.048 - 0.457)).abs() < 1e-9 && (l.alley_reach_m - (1.829 - 0.457)).abs() < 1e-9, "{l:?}");
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    let mut roofs = 0;
    for i in (0..kc.records.len()).step_by(37) {
        let Some(p) = planned(i) else { continue };
        let cp = clutter::plan(&p, 2);
        let faces = clutter::face_obstacles(&p);
        let floor = p.plan.base_y + p.plan.storeys[0].y0;
        for (k, (kind, edge, id, b)) in cp.boxes.iter().enumerate() {
            *by_kind.entry(kind).or_default() += 1;
            let (_, len, reach, obstacles) = faces.iter().find(|f| f.0 == *edge).unwrap_or_else(|| panic!("#{i}: {kind} on edge {edge}, not a street face"));
            assert!(b.w1 <= reach + 1e-9 && b.s0 >= -1e-9 && b.s1 <= len + 1e-9, "#{i} {kind} {b:?}: past the face or the sidewalk (reach {reach}, length {len})");
            // the port's fire escape a new one replaces is not an obstacle to it.
            for o in obstacles {
                assert!(!b.hits(o, 0.004), "#{i} {kind} {b:?} goes through a part of the plan {o:?}");
            }
            for (k2, (kind2, edge2, id2, b2)) in cp.boxes.iter().enumerate() {
                if k2 != k && edge2 == edge && id2 != id {
                    assert!(!b.hits(b2, 0.004), "#{i} {kind} {b:?} goes through {kind2} {b2:?}");
                }
            }
            if matches!(*kind, "awning" | "blade-sign") {
                assert!(b.y0 >= floor + clutter::UNDER_CLEAR_M - 0.13, "#{i} {kind} {b:?}: under the clearance over the walk (floor {floor})");
            }
            if *kind == "awning" {
                assert!(b.w1 <= clutter::AWNING_MAX_PROJECTION_M + 0.021, "#{i} awning {b:?}");
            }
        }
        // roof pieces inside the roof, apart, under the plant allowance.
        let cap = p.plan.base_y + p.plan.envelope.roof_top + p.plan.envelope.roof_plant_allowance;
        for (k, (kind, a, b)) in cp.roof_boxes.iter().enumerate() {
            roofs += 1;
            for (x, z) in [(a[0], a[2]), (b[0], a[2]), (b[0], b[2]), (a[0], b[2])] {
                assert!(p.frame.signed_distance(x, z) < 0.0, "#{i} {kind} pokes off the roof");
            }
            assert!(b[1] <= cap + 0.5 + 1e-9, "#{i} {kind} above the roof plant allowance");
            for (kind2, a2, b2) in cp.roof_boxes.iter().skip(k + 1) {
                let apart = b[0] <= a2[0] || b2[0] <= a[0] || b[2] <= a2[2] || b2[2] <= a[2];
                assert!(apart, "#{i} {kind} through {kind2}");
            }
        }
    }
    eprintln!("checked: {by_kind:?}, roof pieces {roofs}");
    for kind in ["fire-escape", "awning", "sign-face", "blade-sign", "shutter", "grille", "downspout", "meters"] {
        assert!(by_kind.get(kind).copied().unwrap_or(0) > 0, "no {kind} in the sample");
    }
    assert!(roofs > 100);
}

#[test]
fn only_sign_faces_glow() {
    let kc = city();
    let mut lit = 0;
    for i in (0..kc.records.len()).step_by(97) {
        let Some(p) = planned(i) else { continue };
        for rung in [0, 2] {
            let m = build_rung_with(&p, rung, 1);
            if m.glow.is_empty() {
                continue;
            }
            assert_eq!(m.glow.len(), m.groups.len(), "#{i}: a glow code per group");
            for (g, (name, _, _)) in m.glow.iter().zip(&m.groups) {
                if *g != 0 {
                    assert!((clutter::GLOW_BOX..=127).contains(g), "#{i}: glow {g}");
                    assert!(matches!(name.as_str(), "painted-white" | "signage"), "#{i}: {name} glows");
                    lit += 1;
                }
            }
        }
    }
    assert!(lit > 20, "{lit}");
}

#[test]
fn fire_escapes_where_the_city_has_them() {
    let kc = city();
    // (escapes, qualifying older multi-unit blocks of 4 to 9 storeys) by district.
    let mut by: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut landings = 0;
    let mut escapes = 0;
    for i in (0..kc.records.len()).step_by(3) {
        let Some(p) = planned(i) else { continue };
        if p.classified.archetype != "bay-window-apartment" || !(4..=9).contains(&p.plan.storeys.len()) {
            continue;
        }
        let cp = clutter::plan(&p, 1);
        let e = by.entry(district(&p)).or_default();
        e.1 += 1;
        if cp.census.fire_escapes > 0 {
            e.0 += 1;
        }
        escapes += cp.census.fire_escapes;
        landings += cp.census.landings;
    }
    let share = |d: &str| by.get(d).map(|e| e.0 as f64 / e.1.max(1) as f64).unwrap_or(0.0);
    eprintln!("{by:?}");
    assert!(escapes > 30 && landings >= 2 * escapes, "{escapes} escapes, {landings} landings");
    assert!(share("Tenderloin") > share("Marina") && share("Chinatown") > share("Marina") && share("Nob Hill") > share("Pacific Heights"), "{by:?}");
}

#[test]
fn the_box_test_is_strict() {
    let a = Bx::new(0.0, 1.0, 0.0, 1.0, 0.0, 1.0);
    assert!(a.hits(&Bx::new(0.5, 1.5, 0.5, 1.5, 0.5, 1.5), 0.004));
    assert!(!a.hits(&Bx::new(1.0, 2.0, 0.0, 1.0, 0.0, 1.0), 0.004), "touching is not through");
}
