//! Find where the drawn street surface misbehaves: a verge bank drawn above
//! the paving it borders (the tan wedge at building corners), and frontage
//! paving that steps against the walk beside it.
//!   cargo run --release -p eo-street --example surface_defects [step] [near_x near_z]

use eo_data::{City, DataPaths, ElevationGrid};
use eo_street::{BuildingIndex, Layer, StreetCity, StreetOptions, StreetWorld, Terrain, TileMesh};
use std::sync::Arc;

fn tri_h(m: &TileMesh, t: usize, x: f64, z: f64) -> Option<f64> {
    let [a, b, c] = [m.indices[3 * t], m.indices[3 * t + 1], m.indices[3 * t + 2]].map(|i| m.positions[i as usize]);
    let (ax, az, bx, bz, cx, cz) = (a[0] as f64, a[2] as f64, b[0] as f64, b[2] as f64, c[0] as f64, c[2] as f64);
    let det = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
    if det.abs() < 1e-12 {
        return None;
    }
    let wa = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / det;
    let wb = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / det;
    let wc = 1.0 - wa - wb;
    if wa < -1e-6 || wb < -1e-6 || wc < -1e-6 {
        return None;
    }
    Some(wa * a[1] as f64 + wb * b[1] as f64 + wc * c[1] as f64)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let step: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let near: Option<[f64; 2]> = match (args.get(2).and_then(|s| s.parse().ok()), args.get(3).and_then(|s| s.parse().ok())) {
        (Some(x), Some(z)) => Some([x, z]),
        _ => None,
    };
    let paths = DataPaths::discover()?;
    let city = City::load(&paths)?;
    let elevation = ElevationGrid::load(&paths)?;
    let o = StreetOptions::load()?;
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let w = StreetWorld::build(o, StreetCity::load()?, Terrain { grid: Arc::new(elevation), carpet_lift: lift }, BuildingIndex::new(rings));
    let mut keys: Vec<_> = w.tile_keys().into_iter().step_by(step).collect();
    if let Some(p) = near {
        let k = eo_street::tile_key(p[0], p[1]);
        keys = (-1..=1).flat_map(|j| (-1..=1).map(move |i| (k.0 + i, k.1 + j))).collect();
    }
    let (mut samples, mut pokes, mut pokes_bldg) = (0usize, 0usize, 0usize);
    let mut found: Vec<(f64, [f64; 3], f64, bool)> = vec![];
    for k in &keys {
        let center = w.tile_mesh(*k);
        let around: Vec<TileMesh> = (-1..=1).flat_map(|j| (-1..=1).map(move |i| (k.0 + i, k.1 + j))).map(|kk| w.tile_mesh(kk)).collect();
        for (t, layer) in center.layers.iter().enumerate() {
            if *layer != Layer::Verge {
                continue;
            }
            let [a, b, c] = [center.indices[3 * t], center.indices[3 * t + 1], center.indices[3 * t + 2]].map(|i| center.positions[i as usize]);
            for (wa, wb) in [(1.0 / 3.0, 1.0 / 3.0), (0.6, 0.2), (0.2, 0.6), (0.2, 0.2)] {
                let wc = 1.0 - wa - wb;
                let x = a[0] as f64 * wa + b[0] as f64 * wb + c[0] as f64 * wc;
                let z = a[2] as f64 * wa + b[2] as f64 * wb + c[2] as f64 * wc;
                let vy = a[1] as f64 * wa + b[1] as f64 * wb + c[1] as f64 * wc;
                let mut paved: Option<f64> = None;
                let mut which = Layer::Verge;
                for m in &around {
                    for (u, l2) in m.layers.iter().enumerate() {
                        if matches!(l2, Layer::Sidewalk | Layer::CurbTop | Layer::Ramp | Layer::Carriageway) {
                            if let Some(h) = tri_h(m, u, x, z) {
                                if paved.is_none_or(|p| h > p) {
                                    which = *l2;
                                }
                                paved = Some(paved.map_or(h, |p: f64| p.max(h)));
                            }
                        }
                    }
                }
                if std::env::var("EO_DEFECT_TRACE").is_ok() && paved.is_some_and(|p| vy > p + 1.0) {
                    eprintln!(
                        "verge tri {:?} {:?} {:?} over {:?} at {:.3}; at the sample: paving beside {:?}, walk beside {:?}, carriageway of another run {}",
                        a,
                        b,
                        c,
                        which,
                        paved.unwrap(),
                        w.paving_level_beside(x, z, None),
                        w.walk_level_beside(x, z, None),
                        w.on_other_carriageway(x, z, usize::MAX)
                    );
                }
                let Some(p) = paved else { continue };
                samples += 1;
                if vy > p + 0.01 {
                    pokes += 1;
                    let bl = (0..8).any(|q| {
                        let ang = q as f64 * std::f64::consts::FRAC_PI_4;
                        w.buildings.contains(x + 2.0 * ang.cos(), z + 2.0 * ang.sin())
                    });
                    if bl {
                        pokes_bldg += 1;
                    }
                    found.push((vy - p, [x, vy, z], p, bl));
                }
            }
        }
    }
    println!("{} tiles: verge samples over paving {samples}, verge above paving by > 1 cm {pokes} ({pokes_bldg} within 2 m of a building)", keys.len());
    found.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    for f in found.iter().take(25) {
        println!("  +{:.3} m at ({:.2}, {:.3}, {:.2}) paving {:.3} building {}", f.0, f.1[0], f.1[1], f.1[2], f.2, f.3);
    }
    Ok(())
}
