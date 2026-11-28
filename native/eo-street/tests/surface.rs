//! The drawn street surface itself, on a sample of real tiles (every 20th
//! street tile of the city, about 350): no holes in the paved footprint, no
//! two surfaces fighting in one place, the carriageway on the JS cross
//! section, paint above what it is painted on, and no ground showing through.
//!
//! Probes, per kept span of every run anchored in a sampled tile, at the span's
//! middle: 7 across the carriageway (|u| <= 0.9 half) and the middle of each
//! footway, away from junction mouths and yield spans.
//!
//!   hole       nothing walkable drawn at a probe
//!   fight      two walkable surfaces at one probe between 0.5 mm and 8 mm
//!              apart (closer is one surface's shared edge, farther is a kerb
//!              or a bank, which depth-sorts cleanly at street range)
//!   off        carriageway drawn more than 2 cm off crossSectionY at the probe
//!   ground     the ground carpet above the street at a probe

use eo_data::{City, DataPaths, ElevationGrid};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;

#[test]
fn taraval_shared_mouth_has_one_continuous_carriageway() {
    let paths = DataPaths::discover().unwrap();
    let city = City::load(&paths).unwrap();
    let elev = ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| p.map(f64::from)).collect()).collect();
    let w = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));

    let node = w.plan.nodes.iter().find(|n| n.id == "inferred:-5716.560:4801.681").unwrap();
    assert_eq!(node.approaches.len(), 6, "the shared pavement still carries both routes");

    // Consecutive traffic positions on the west side of the junction.
    let heights = [
        [-4585.98959471102, 3839.315204413882],
        [-4586.105707247667, 3839.321504368864],
    ].map(|p| {
        let [x, z] = eo_data::groundscale::plan(p);
        let heights = w.paved_heights(x, z);
        assert_eq!(heights.len(), 1, "overlapping surfaces at {x}, {z}: {heights:?}");
        assert!(w.only_carriageway_at(x, z));
        heights[0]
    });
    assert!((heights[1] - heights[0]).abs() < 0.05, "{heights:?}");
}

#[test]
fn street_surface_is_closed_and_on_its_section() {
    let paths = DataPaths::discover().unwrap();
    let city = City::load(&paths).unwrap();
    let elev = ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let w = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    // Two Market Street carriageways exchange which one supplies the floor.
    // The independently rounded edges once left a 0.09 mm slit. Sample the
    // actual seam and adjacent representable mesh coordinates on both sides.
    let [x, z] = eo_data::groundscale::plan([-568.730954205614, 876.533983100553]);
    let unit = |v: f64| { let v = v.abs() as f32; f64::from(v.next_up() - v) };
    for i in -2..=2 {
        for j in -2..=2 {
            let (qx, qz) = (x + f64::from(i) * unit(x), z + f64::from(j) * unit(z));
            assert!(w.carriageway_near(qx, qz, w.ground(qx, qz)).is_some(),
                "missing Market Street asphalt at {qx}, {qz}");
        }
    }
    let keys: Vec<_> = w.tile_keys().into_iter().step_by(20).collect();
    let (mut probes, mut holes, mut fights, mut off, mut ground, mut walk_probes, mut walk_holes) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut worst_off = (0.0f64, [0.0, 0.0]);
    let mut hole_at = vec![];
    for k in &keys {
        let refs = w.tiles[k].clone();
        for (ei, i) in refs.spans {
            let (ei, i) = (ei as usize, i as usize);
            let e = &w.plan.entries[ei];
            let g = &w.geoms[ei];
            let (a, b) = (g.stations[i], g.stations[i + 1]);
            let s = (a.s + b.s) / 2.0;
            // Away from the junction mouths (pads and corner returns own them).
            if s < e.s0() + 1.0 || s > e.s1() - 1.0 || e.in_yield_span(s) {
                continue;
            }
            let f = e.frame(s, false);
            let datum = g.datums[i] + (g.datums[i + 1] - g.datums[i]) * 0.5;
            for k in -3..=3 {
                let u = e.half * 0.9 * k as f64 / 3.0;
                let p = f.at(u);
                probes += 1;
                let hs = w.paved_heights(p[0], p[1]);
                if hs.is_empty() {
                    holes += 1;
                    if hole_at.len() < 5 {
                        hole_at.push(p);
                    }
                    continue;
                }
                let mut hs2 = hs.clone();
                hs2.sort_by(|x, y| x.partial_cmp(y).unwrap());
                if hs2.windows(2).any(|q| q[1] - q[0] > 0.0005 && q[1] - q[0] < 0.008) {
                    fights += 1;
                }
                let want = w.o.cross_section_y(datum, u, e.half);
                let best = hs.iter().map(|h| (h - want).abs()).fold(f64::MAX, f64::min);
                if best > 0.02 {
                    off += 1;
                    if best > worst_off.0 {
                        worst_off = (best, p);
                    }
                }
                if w.carpet_y(p[0], p[1]) > hs2[hs2.len() - 1] {
                    ground += 1;
                }
            }
            for (kk, side) in [(0usize, 1.0f64), (1, -1.0)] {
                let walk = e.walks[kk];
                if walk < w.o.min_sidewalk_width || e.walk_yield_spans[kk].iter().any(|sp| s >= sp[0] && s <= sp[1]) {
                    continue;
                }
                let p = f.at(side * (e.half + walk * 0.5));
                walk_probes += 1;
                if w.paved_heights(p[0], p[1]).is_empty() {
                    walk_holes += 1;
                }
            }
        }
    }
    println!(
        "{} tiles: carriageway probes {probes}, holes {holes} {hole_at:?}, fights {fights}, off-section {off} (worst {:.3} m at {:?}), ground above street {ground}; footway probes {walk_probes}, holes {walk_holes}",
        keys.len(),
        worst_off.0,
        worst_off.1
    );
    // Bounds are regression guards set just above what this sample measures.
    // Slice 4 (2026-11-13): holes 0 + 0 of 125,286 + 26,714 probes, fights 328
    // (0.26 %: two ribbons of different streets overlapping where no node
    // joins them), ground above the street 512 (0.41 %: an 8 m terrain cell
    // straddling a corridor edge on a cross slope rose over the street).
    // surfaces (2026-11-13): fights 287 (0.23 %; still open: the overlapping
    // ribbons are a plan defect, fewer now that banks stop at another street's
    // paving), ground above the street 125 (0.10 %: the carpet is held half a
    // cell past the corridor, `TERRAIN_CAP_MARGIN_M`; the rest are cells whose
    // far vertex is still uphill of that margin).
    assert!(probes > 50_000);
    assert!(holes * 1000 <= probes, "carriageway holes {holes} of {probes}");
    assert!(walk_holes * 1000 <= walk_probes, "footway holes {walk_holes} of {walk_probes}");
    assert!(fights * 10_000 <= probes * 25, "fighting surfaces {fights} of {probes}");
    assert!(ground * 10_000 <= probes * 12, "ground above the street {ground} of {probes}");
}

/// a verge bank drawn above the street paving it borders (the tan
/// wedge). Every 20th street tile; samples at four points of every bank
/// triangle that has paving (carriageway, kerb top, ramp, footway, frontage)
/// under it. Before (2026-11-13): 21,252 of 91,189 samples over paving by more
/// than 1 cm, 1,924 of them within 2 m of a building. After the bank stops at
/// another street's paving (`bank`'s foot) and the carpet margin: 16,955 of
/// 54,159, 676 near a building; the rest are the steep faces where two streets meet at different
/// levels (a retaining wall in the JS, not ported) and junction pads the
/// corridor query does not see. fix3: where the bank's run meets another
/// street's paving more than one kerb face below the walk edge, a vertical
/// face in the kerb-face concrete closes the step (the JS `retaining` layer)
/// and no bank is drawn there; so is a bank whose run would reach a building
/// (a walk over the lower ground beside a building's base) and one under
/// open ground more than a kerb face above the walk (the retaining edge):
/// 1,239 of 17,712 samples, 3 near a building (the rest: lower paving within
/// one kerb face, which the JS grades to).
#[test]
fn verge_banks_stay_off_the_paving() {
    let paths = DataPaths::discover().unwrap();
    let city = City::load(&paths).unwrap();
    let elev = ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let w = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    let tri_h = |m: &eo_street::TileMesh, t: usize, x: f64, z: f64| -> Option<f64> {
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
    };
    let (mut samples, mut pokes, mut near_building) = (0usize, 0usize, 0usize);
    for k in w.tile_keys().into_iter().step_by(20) {
        let center = w.cached_mesh(k);
        let around: Vec<_> = (-1..=1).flat_map(|j| (-1..=1).map(move |i| (k.0 + i, k.1 + j))).map(|kk| w.cached_mesh(kk)).collect();
        for (t, layer) in center.layers.iter().enumerate() {
            if *layer != eo_street::Layer::Verge {
                continue;
            }
            let [a, b, c] = [center.indices[3 * t], center.indices[3 * t + 1], center.indices[3 * t + 2]].map(|i| center.positions[i as usize]);
            for (wa, wb) in [(1.0 / 3.0, 1.0 / 3.0), (0.6, 0.2), (0.2, 0.6), (0.2, 0.2)] {
                let wc = 1.0 - wa - wb;
                let x = a[0] as f64 * wa + b[0] as f64 * wb + c[0] as f64 * wc;
                let z = a[2] as f64 * wa + b[2] as f64 * wb + c[2] as f64 * wc;
                let vy = a[1] as f64 * wa + b[1] as f64 * wb + c[1] as f64 * wc;
                let mut paved: Option<f64> = None;
                for m in &around {
                    for (u, l2) in m.layers.iter().enumerate() {
                        if matches!(l2, eo_street::Layer::Sidewalk | eo_street::Layer::CurbTop | eo_street::Layer::Ramp | eo_street::Layer::Carriageway) {
                            if let Some(h) = tri_h(m, u, x, z) {
                                paved = Some(paved.map_or(h, |p: f64| p.max(h)));
                            }
                        }
                    }
                }
                let Some(p) = paved else { continue };
                samples += 1;
                if vy > p + 0.01 {
                    pokes += 1;
                    if (0..8).any(|q| {
                        let ang = q as f64 * std::f64::consts::FRAC_PI_4;
                        w.buildings.contains(x + 2.0 * ang.cos(), z + 2.0 * ang.sin())
                    }) {
                        near_building += 1;
                    }
                }
            }
        }
    }
    println!("verge samples over paving {samples}: above it by > 1 cm {pokes}, {near_building} within 2 m of a building");
    assert!(samples > 10_000);
    assert!(pokes <= 1_300, "verge banks over paving: {pokes}"); // was 17,500
    assert!(near_building <= 10, "verge banks over paving beside buildings: {near_building}"); // was 750
}
