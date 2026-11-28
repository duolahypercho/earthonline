use super::*;
use crate::{BuildingIndex, Layer, StreetCity, Terrain};
use std::sync::Arc;

#[test]
fn fitted_driveway_mouths_meet_the_receiving_pavement_across_both_wheel_tracks() {
    let paths = eo_data::DataPaths::discover().unwrap();
    let city = eo_data::City::load(&paths).unwrap();
    let grid = Arc::new(eo_data::ElevationGrid::load(&paths).unwrap());
    let opts = StreetOptions::load().unwrap();
    let carpet_lift = opts.open_ground_lift;
    let buildings = city
        .buildings
        .iter()
        .map(|b| {
            b.ring
                .iter()
                .map(|p| [f64::from(p[0]), f64::from(p[1])])
                .collect()
        })
        .collect();
    let w = StreetWorld::build(
        opts,
        StreetCity::load().unwrap(),
        Terrain { grid, carpet_lift },
        BuildingIndex::new(buildings),
    );
    let mut mouths = 0;
    let mut probes = 0;
    let mut gaps = Vec::new();
    let mut maximum = 0.0_f64;

    for (ei, g) in w.geoms.iter().enumerate() {
        let Some(aprons) = &g.driveways else {
            continue;
        };
        let e = &w.plan.entries[ei];
        for apron in &aprons.aprons {
            let end = usize::from(apron.approach < apron.mouth);
            let owner = e.driveway_owners[end].unwrap();
            let i = g.stations.iter().position(|s| s.s == apron.mouth).unwrap();
            let span = i - end;
            let f = g.stations[i];
            let key = tile_key(f.x, f.z);
            let mut source = TileMesh::default();
            w.emit_span(&mut source, ei, span, false);
            let source = SurfaceTile::build(&[&source], key);
            let mut receiving = TileMesh::default();
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let Some(refs) = w.tiles.get(&(key.0 + dx, key.1 + dz)) else {
                        continue;
                    };
                    for &(r, j) in &refs.spans {
                        if r as usize == owner {
                            w.emit_span(&mut receiving, owner, j as usize, false);
                        }
                    }
                }
            }
            let receiving = SurfaceTile::build(&[&receiving], key);
            mouths += 1;
            for k in -10..=10 {
                let u = 0.95 * e.half * f64::from(k) / 10.0;
                // Probe both sides of the mouth, inside the source triangle
                // by more than the precision of its stored map coordinates.
                let p = f.at(u);
                let sign = if end == 0 { 1.0 } else { -1.0 };
                let from = [p[0] + sign * 0.01 * f.tx, p[1] + sign * 0.01 * f.tz];
                let to = [p[0] - sign * 0.01 * f.tx, p[1] - sign * 0.01 * f.tz];
                let a = source
                    .heights_on(from[0], from[1], Some(Layer::Carriageway))
                    .into_iter()
                    .reduce(f64::max);
                let b = receiving.height(to[0], to[1]);
                let gap = a.zip(b).map_or(f64::INFINITY, |(a, b)| (a - b).abs());
                probes += 1;
                maximum = maximum.max(gap);
                if gap > w.o.marking_lift {
                    gaps.push((ei, e.id.as_str(), k, p, gap));
                }
            }
        }
    }
    println!(
        "{mouths} terminal driveways, {probes} lateral probes, {} discontinuities, max {maximum:.5} m, samples {:?}",
        gaps.len(),
        &gaps[..gaps.len().min(12)]
    );
    assert!(mouths > 0 && gaps.is_empty());
}
