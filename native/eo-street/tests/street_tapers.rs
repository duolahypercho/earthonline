//! (`--street-tapers`, `StreetOptions::tapers`) Two runs meeting end to end at
//! a plain joint (no junction, neither trimmed there) with different widths:
//! the wider narrows to the other's width over the lane-reduction taper, so
//! the drawn carriageway meets with no kerb jog and no lip across the road.
//! Without tapers the crown of the wider run stands higher at the joint (the
//! crown rises with the half-width at the cross slope): the Folsom Street
//! joint in SoMa between its 8.77 m and 10.29 m halves read a 3.1 cm lip.
//!
//! Its own test binary: the switch is process-wide (eo-street `options::set_tapers`).

use eo_data::{City, DataPaths, ElevationGrid};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::collections::HashMap;
use std::sync::Arc;

#[test]
fn plain_joints_taper_with_no_lip() {
    eo_street::options::set_tapers(true);
    let paths = DataPaths::discover().unwrap();
    let city = City::load(&paths).unwrap();
    let elev = ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    assert!(o.tapers);
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let w = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    let s = eo_data::groundscale::scale();
    // Folsom Street, 800 block (5th to 4th), the joint 195.3 m from 5th St (city metres x the plan scale)
    let g = |st: f64, n: f64| [s * (1405.1 + 0.7099 * st + 0.7041 * n), s * (-253.0 - 0.7041 * st + 0.7099 * n)];
    let top = |p: [f64; 2]| w.paved_heights(p[0], p[1]).into_iter().fold(f64::MIN, f64::max);
    for n in [-3.0, 0.0, 3.0] {
        let step = top(g(195.4, n)) - top(g(195.2, n));
        assert!(step.abs() < 0.005, "Folsom joint at n {n}: a {step:.4} m step across the road");
    }
    // every taper: drawn at the narrower width at the joint, its own past the taper
    let mut ends: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for (ei, e) in w.plan.entries.iter().enumerate() {
        if e.structure_leg.is_some() || e.points.len() < 2 {
            continue;
        }
        for (end, p) in [(0usize, e.points[0]), (1, e.points[e.points.len() - 1])] {
            ends.entry(((p[0] * 4.0).round() as i64, (p[1] * 4.0).round() as i64)).or_default().push((ei, end));
        }
    }
    let (mut tapered, mut checked) = (0usize, 0usize);
    for v in ends.values() {
        if v.len() != 2 {
            continue;
        }
        for (k, &(ei, end)) in v.iter().enumerate() {
            let (delta, len) = w.tapers[ei][end];
            if delta <= 0.0 {
                continue;
            }
            tapered += 1;
            let e = &w.plan.entries[ei];
            let other = &w.plan.entries[v[1 - k].0];
            let (at_joint, past) = if end == 0 { (e.s0(), e.s0() + len) } else { (e.s1(), e.s1() - len) };
            assert!((w.half_at(ei, at_joint) - other.half).abs() < 1e-6, "run {} at its joint", e.id);
            assert!((w.half_at(ei, past) - e.half).abs() < 1e-6 || w.tapers[ei][1 - end].0 > 0.0, "run {} past its taper", e.id);
            assert!(len >= 0.5 && len <= 0.45 * (e.s1() - e.s0()) + 1e-6, "run {}: taper {len} m", e.id);
            checked += 1;
        }
    }
    assert!(tapered > 500, "{tapered} tapered joints");
    assert_eq!(tapered, checked);
}
