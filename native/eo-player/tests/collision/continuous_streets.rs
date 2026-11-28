//! A single mapped street may be split into several records on a bend.
use super::streets;
use eo_player::world::{self, BuildingSet, CollisionWorld};
use glam::DVec3;
use std::sync::Arc;

#[test]
fn tires_and_feet_have_paving_on_both_sides_of_continuous_bends() {
    let ground = streets();
    let street = &ground.world;
    let mut world = CollisionWorld::new(ground.clone(), Arc::new(BuildingSet::empty()), 1);
    for (way, incoming, outgoing) in [
        ("sf-street-419755793", "sf-seg-577", "sf-seg-578"), // Terry A. Francois Boulevard
        ("sf-street-254466694", "sf-seg-422", "sf-seg-423"), // Masonic Avenue
        ("sf-street-27044025", "sf-seg-135", "sf-seg-136"),  // Indiana Street
        ("sf-street-945101895", "sf-seg-4237", "sf-seg-4238"), // Mariposa entrance
    ] {
        let entry = |id| {
            street
                .plan
                .entries
                .iter()
                .position(|e| {
                    e.id == id && street.city.segments[e.road].street_id.as_deref() == Some(way)
                })
                .expect("original street record and traffic identity")
        };
        let (i, j) = (entry(incoming), entry(outgoing));
        let (a, b) = (&street.plan.entries[i], &street.plan.entries[j]);
        let (f, g) = (a.frame(a.length, false), b.frame(0., false));
        let (last, first) = (
            street.geoms[i].stations.last().unwrap(),
            &street.geoms[j].stations[0],
        );
        let y = *street.geoms[i].datums.last().unwrap();
        world.load_blocking(DVec3::new(f.x, y, f.z), world::LOAD_RADIUS_M);
        // Both ends must meet across the full carriageway and both footways.
        for (side, sign) in [(0, 1.), (1, -1.)] {
            let u = sign * (a.half + a.walks[side].min(b.walks[side]));
            let (p, q) = (last.at(u), first.at(u));
            assert!(
                (p[0] - q[0]).hypot(p[1] - q[1]) < 0.0001,
                "open edge on {way}"
            );
        }
        // Probe the former open wedge and the inside of the turn. An earth
        // heightfield under the missing strip cannot satisfy the street ray.
        for u in [-0.9, -0.6, -0.25, 0.0, 0.25, 0.6, 0.9] {
            let (p, q) = (f.at(u * a.half), g.at(u * b.half));
            for t in [0.1, 0.5, 0.9] {
                let (x, z) = (p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t);
                let drawn = street.carriageway_near(x, z, y).unwrap_or_else(|| {
                    panic!("missing drawn asphalt on {way} at {x}, {z}, u={u}, t={t}")
                });
                let hit = world.raycast(
                    DVec3::new(x, drawn + 0.03, z),
                    DVec3::NEG_Y,
                    0.06,
                    world::layer::STREET,
                    None,
                );
                assert!(
                    hit.is_some(),
                    "missing street collider on {way} at ({x}, {drawn}, {z})"
                );
            }
        }
    }
}

#[test]
fn mapped_bends_keep_the_outside_wheel_track() {
    use std::collections::HashMap;
    let ground = streets();
    let world = &ground.world;
    let mut ends = HashMap::new();
    for (i, e) in world.plan.entries.iter().enumerate() {
        for (p, start) in [(e.points[0], true), (*e.points.last().unwrap(), false)] {
            ends.entry(p.map(f64::to_bits))
                .or_insert_with(Vec::new)
                .push((i, start));
        }
    }
    let mut sites = Vec::new();
    for pair in ends.values() {
        let &[(i, a), (j, b)] = pair.as_slice() else {
            continue;
        };
        if a == b || i == j {
            continue;
        }
        let (i, j) = if a { (j, i) } else { (i, j) };
        let (e, n) = (&world.plan.entries[i], &world.plan.entries[j]);
        let way = &world.city.segments[e.road].street_id;
        let (ge, gn) = (&world.geoms[i], &world.geoms[j]);
        if way.is_none()
            || *way != world.city.segments[n.road].street_id
            || e.half != n.half
            || e.trim_end != 0.
            || n.trim_start != 0.
            || e.structure_ends[1].is_some()
            || n.structure_ends[0].is_some()
            || ge.keep.last() != Some(&true)
            || gn.keep.first() != Some(&true)
        {
            continue;
        }
        let (f, g) = (e.frame(e.length, false), n.frame(0., false));
        let y = *ge.datums.last().unwrap();
        if (y - gn.datums[0]).abs() > 0.05 || f.tx * g.tx + f.tz * g.tz < 0.5 {
            continue;
        }
        // Halfway between the two independent, unmitred end sections on
        // the outside of the bend, where an outer wheel used to lose contact.
        let u = -(f.tx * g.tz - f.tz * g.tx).signum() * 0.75 * e.half;
        let (p, q) = (f.at(u), g.at(u));
        if (p[0] - q[0]).hypot(p[1] - q[1]) <= 0.02 {
            continue;
        }
        let (x, z) = ((p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5);
        sites.push((eo_street::tile_key(x, z), i, x, z, y));
    }
    sites.sort_by_key(|&(key, i, ..)| (key, i));
    let all = std::env::var_os("EO_AUDIT_ALL_STREET_TILES").is_some();
    let stride = if all {
        1
    } else {
        sites.len().div_ceil(96).max(1)
    };
    let mut checked = 0;
    for &(_, i, x, z, y) in sites.iter().step_by(stride) {
        assert!(
            world
                .carriageway_near(x, z, y)
                .is_some_and(|h| (h - y).abs() < 0.5),
            "missing outside wheel track on {} at ({x}, {y}, {z})",
            world.plan.entries[i].id
        );
        checked += 1;
    }
    assert!(checked >= 96, "only {checked} bends sampled");
    println!(
        "{checked} of {} eligible mapped bends with asphalt under the outside wheel track",
        sites.len()
    );
}
