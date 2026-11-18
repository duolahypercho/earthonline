//! the shipped street size on the real city. The default scale
//! is read through the approved ground-plan deviations (1.25); the city at
//! that scale is compared with the same city as surveyed (1:1) in this one
//! process (`City::load_at`).
//!
//!   cargo test --release -p eo-data --test ground_plan -- --nocapture

use eo_data::groundscale::{self, PULL_FULL_M};
use eo_data::standards::{self, CaStandards, DeviationStatus};
use eo_data::*;
use std::collections::HashMap;

fn paths() -> DataPaths {
    DataPaths::discover().expect("repo data")
}

fn poly(b: &Building) -> Vec<[f64; 2]> {
    b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 1e-12 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (a[0] + dx * t - p[0]).hypot(a[1] + dz * t - p[1])
}

fn boundary_dist(p: [f64; 2], q: &[[f64; 2]]) -> f64 {
    (0..q.len()).map(|i| seg_dist(p, q[i], q[(i + 1) % q.len()])).fold(f64::MAX, f64::min)
}

#[test]
fn the_default_is_the_approved_deviation_and_every_entry_names_a_known_standard() {
    if std::env::var("EO_GROUND_SCALE").is_ok() {
        eprintln!("EO_GROUND_SCALE is set; the default is not under test here");
        return;
    }
    assert_eq!(groundscale::scale(), 1.25, "the default comes from the approved ground-plan deviations");
    assert_eq!(groundscale::scale(), standards::ground_plan_scale().unwrap());
    let ca = CaStandards::load(&paths()).unwrap();
    ca.check_deviations(standards::CALIFORNIA_STREET_DEVIATIONS).expect("every deviation names a standard of the table");
    // A planted entry naming a standard the table does not have is refused,
    // and so is one approved without its approval record.
    let mut bad = standards::CALIFORNIA_STREET_DEVIATIONS.to_vec();
    bad[0].standard = "no-such-standard";
    assert!(ca.check_deviations(&bad).is_err());
    let mut bad = standards::CALIFORNIA_STREET_DEVIATIONS.to_vec();
    bad[1].approved_on = None;
    assert!(ca.check_deviations(&bad).is_err());
    bad[1].status = DeviationStatus::Pending;
    assert!(ca.check_deviations(&bad).is_ok(), "a pending entry is well formed");
    assert!(standards::ground_plan_scale_in(&bad).is_err(), "and the factor is refused while it is pending");
}

#[test]
fn hand_built_buildings_move_whole_and_their_neighbours_close_the_strip() {
    let s = groundscale::scale();
    if s == 1.0 {
        eprintln!("scale 1: nothing to compare");
        return;
    }
    let p = paths();
    let surveyed = City::load_at(&p, 1.0).unwrap();
    let scaled = City::load_at(&p, s).unwrap();
    let report = scaled.ground_plan.clone().expect("a ground-plan report at scale != 1");
    // the same records in the same order, then one infill record per filled seam.
    assert_eq!(surveyed.buildings.len() + report.seams_filled, scaled.buildings.len());
    assert!(scaled.buildings[surveyed.buildings.len()..].iter().all(|b| b.id.starts_with("seam-")));
    assert!(surveyed.buildings.iter().zip(&scaled.buildings).all(|(a, b)| a.id == b.id));
    let moved = groundscale::hand_built_records(&p.repo_root).unwrap();
    let blender: Vec<usize> = (0..surveyed.buildings.len()).filter(|&i| matches!(moved.get(&surveyed.buildings[i].id), Some(groundscale::Anchor::FrontEdge(_)))).collect();
    assert_eq!(blender.len(), 343, "every Blender-built building has its record");
    assert!(report.moved >= 343);
    // 1. Every hand-built record is moved whole: the same shape, one translation.
    let mut shifts: HashMap<&str, [f64; 2]> = HashMap::new();
    for &i in &blender {
        let (a, b) = (&surveyed.buildings[i].points, &scaled.buildings[i].points);
        let d = [b[0] - a[0], b[1] - a[1]];
        for (u, v) in a.chunks_exact(2).zip(b.chunks_exact(2)) {
            assert!((v[0] - u[0] - d[0]).abs() < 1e-6 && (v[1] - u[1] - d[1]).abs() < 1e-6, "{} is not a pure translation", surveyed.buildings[i].id);
        }
        shifts.insert(surveyed.buildings[i].id.as_str(), d);
    }
    // 2. The neighbours: every vertex of another record that touched a
    //    hand-built record as surveyed (within PULL_FULL_M) touches it as
    //    drawn by the same distance (the party wall stays shut), where the
    //    plain scale would have opened a strip.
    let polys1: Vec<Vec<[f64; 2]>> = blender.iter().map(|&i| poly(&surveyed.buildings[i])).collect();
    let polys2: Vec<Vec<[f64; 2]>> = blender.iter().map(|&i| poly(&scaled.buildings[i])).collect();
    let boxes: Vec<[f64; 4]> = polys1.iter().map(|q| q.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])).collect();
    let is_moved: std::collections::HashSet<usize> = blender.iter().copied().collect();
    let (mut touching, mut worst_change, mut plain_gaps) = (0usize, 0.0f64, vec![]);
    // Per hand-built building: touching neighbour vertices and the widest strip the plain scale would leave.
    let mut per: Vec<(usize, f64)> = vec![(0, 0.0); blender.len()];
    for (i, b) in surveyed.buildings.iter().enumerate() {
        if is_moved.contains(&i) || moved.contains_key(&b.id) {
            continue;
        }
        for (k, v) in poly(b).into_iter().enumerate() {
            // The hand-built records this vertex touched as surveyed; it follows one
            // of them (the nearest, the pull's choice), so its distance to that one holds.
            let mut near: Vec<(usize, f64)> = vec![];
            for (j, q) in polys1.iter().enumerate() {
                let bx = boxes[j];
                if v[0] < bx[0] - 1.0 || v[0] > bx[2] + 1.0 || v[1] < bx[1] - 1.0 || v[1] > bx[3] + 1.0 {
                    continue;
                }
                let d1 = boundary_dist(v, q);
                if d1 <= PULL_FULL_M {
                    near.push((j, d1));
                }
            }
            if near.is_empty() {
                continue;
            }
            touching += 1;
            let w = [scaled.buildings[i].ring[k][0] as f64, scaled.buildings[i].ring[k][1] as f64];
            let change = near.iter().map(|&(j, d1)| (boundary_dist(w, &polys2[j]) - d1).abs()).fold(f64::MAX, f64::min);
            worst_change = worst_change.max(change);
            let &(j, _) = near.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            let g = boundary_dist([v[0] * s, v[1] * s], &polys2[j]);
            plain_gaps.push(g);
            per[j].0 += 1;
            per[j].1 = per[j].1.max(g);
        }
    }
    plain_gaps.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| plain_gaps[((plain_gaps.len() - 1) as f64 * p) as usize];
    println!(
        "ground plan x{s}: {} records moved whole, groups {:?} ({} touching pairs left apart), {} street edges held (worst {:.3} m off their scaled line); {} neighbour vertices touched a hand-built building as surveyed: as drawn their distance changed by at most {:.4} m (f32 rings); the plain scale would have left them {:.2} / {:.2} / {:.2} m away (p50 / p90 / max)",
        report.moved,
        report.clusters,
        report.open_seams,
        report.street_edges,
        report.worst_street_edge_m,
        touching,
        worst_change,
        pct(0.5),
        pct(0.9),
        pct(1.0)
    );
    let mut rank: Vec<usize> = (0..blender.len()).filter(|&j| per[j].0 >= 4).collect();
    rank.sort_by(|&a, &b| (per[b].1 * per[b].0 as f64).total_cmp(&(per[a].1 * per[a].0 as f64)));
    for &j in rank.iter().take(16) {
        let b = &scaled.buildings[blender[j]];
        let c = b.ring.iter().fold([0.0f64; 2], |a, p| [a[0] + p[0] as f64 / b.ring.len() as f64, a[1] + p[1] as f64 / b.ring.len() as f64]);
        println!("  gap closed: {} at ({:.0}, {:.0}) drawn, {} neighbour vertices, plain-scale strip up to {:.2} m, shift ({:.1}, {:.1})", b.id, c[0], c[1], per[j].0, per[j].1, shifts[b.id.as_str()][0], shifts[b.id.as_str()][1]);
    }
    println!("  off their line: {:?}", report.off_line);
    assert!(touching > 300, "the hand-built buildings have neighbours ({touching})");
    assert!(worst_change < 0.01, "a party wall opened by {worst_change} m");
    assert!(pct(0.5) > 0.5, "the plain scale opens a strip the pull closes");
    // 3. A front held on its street line: within 0.75 m of where the scaled
    //    plan puts its line (a group of buildings shares one shift). A side
    //    face on a second street is held too, but a building with faces on
    //    opposite streets keeps neither exactly (reported, not asserted).
    assert!(report.worst_front_m < 0.75, "a hand-built front is {:.2} m off its scaled street line", report.worst_front_m);
    println!("  worst front {:.3} m off its scaled line; worst other street face {:.2} m (set back inside its scaled lot)", report.worst_front_m, report.worst_street_edge_m);
    // 4. Everything else scales: a record far from any hand-built one is s x itself.
    let far = surveyed.buildings.iter().zip(&scaled.buildings).filter(|(a, _)| !moved.contains_key(&a.id)).take(2000).filter(|(a, b)| a.points.iter().zip(&b.points).all(|(u, v)| (u * s - v).abs() < 1e-6)).count();
    assert!(far > 1900, "records away from the hand-built ones are plainly scaled ({far} of 2000)");
    // Roads are scaled about the origin.
    let (r1, r2) = (&surveyed.roads[100], &scaled.roads[100]);
    assert!((r1.points64[0][0] * s - r2.points64[0][0]).abs() < 1e-9);
}
