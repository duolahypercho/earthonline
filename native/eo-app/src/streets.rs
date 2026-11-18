//! Slice 4 at load time: the street world (eo-street, built on its own
//! thread beside the rest of the load), the street tiles' groups and static
//! levels, and the foundation skirts of the Blender-built buildings.

use anyhow::Result;
use eo_data::{City, ElevationGrid};
use eo_render::authored::AuthoredSet;
use eo_render::scene::Skirt;
use eo_render::street::{StreetPalette, StreetSet, StreetStatic};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;
use std::thread::JoinHandle;

/// The street plan needs only the street data: start it first thing.
pub fn spawn_plan() -> JoinHandle<Result<(StreetOptions, StreetCity, eo_street::Plan)>> {
    std::thread::spawn(|| {
        let t = std::time::Instant::now();
        let o = StreetOptions::load()?;
        let sc = StreetCity::load()?;
        let plan = eo_street::plan::build_plan(&sc, &o);
        eprintln!("startup: street plan {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        Ok((o, sc, plan))
    })
}

pub fn spawn(plan: JoinHandle<Result<(StreetOptions, StreetCity, eo_street::Plan)>>, city: &Arc<City>, elevation: &ElevationGrid) -> JoinHandle<Result<Arc<StreetWorld>>> {
    let city = city.clone();
    let grid = Arc::new(elevation.clone());
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let (o, sc, plan) = plan.join().map_err(|_| anyhow::anyhow!("street plan thread panicked"))??;
        let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
        let lift = o.open_ground_lift;
        let w = StreetWorld::from_plan(o, sc, plan, Terrain { grid, carpet_lift: lift }, BuildingIndex::new(rings));
        eprintln!("startup: street world {:.0} ms after its plan", t.elapsed().as_secs_f64() * 1e3);
        Ok(Arc::new(w))
    })
}

/// the survey the street wear decals place by (eo-render cc0_decals.rs):
/// shopfronts (the ring points of buildings whose OSM record names a walk-in
/// use: a shop or food `amenity`, or a retail / commercial `building`) and
/// the surveyed storm-drain inlets (DataSF `sf-storm-drains.json.gz`, the
/// SFPUC's catch basins and curb inlets in the street gutter), in world
/// metres. Missing data leaves the matching rule without points.
pub fn cc0_survey(city: &City) {
    // Not walk-in shopfronts: worship, schools, services behind a lobby, parking.
    const NOT_SHOPS: &[&str] = &["place_of_worship", "school", "university", "kindergarten", "college", "parking", "fire_station", "police", "company", "social_facility", "clinic", "hospital", "doctors", "dentist", "toilets", "grave_yard", "prison", "townhall", "courthouse"];
    const SHOP_BUILDINGS: &[&str] = &["retail", "commercial", "supermarket", "kiosk"];
    let mut shops = vec![];
    for b in &city.buildings {
        let am = b.tags.amenity.as_str();
        let walk_in = (!am.is_empty() && !NOT_SHOPS.contains(&am)) || SHOP_BUILDINGS.contains(&b.tags.building.as_str());
        if walk_in {
            shops.extend(b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]));
        }
    }
    let frame = eo_data::Frame::from_meta(&city.meta);
    let drains: Vec<[f64; 2]> = eo_data::DataPaths::discover()
        .ok()
        .and_then(|p| eo_data::read_maybe_gzip(&p.sf("sf-storm-drains.json.gz")).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|v| v["drains"].as_array().cloned())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let (lat, lon) = (r.get(0)?.as_f64()?, r.get(1)?.as_f64()?);
                    // `projectLatLon` (z is south): the inverse of `Frame::unproject`;
                    // on the scaled plan, as the streets are.
                    Some(eo_data::groundscale::plan([(lon - frame.center_lon) * frame.m_per_deg_lon, -(lat - frame.center_lat) * frame.m_per_deg_lat]))
                })
                .collect()
        })
        .unwrap_or_default();
    eprintln!("cc0: decal survey: {} shopfront points, {} storm-drain inlets", shops.len(), drains.len());
    eo_render::cc0_decals::set_points(shops, drains);
}

/// Street tiles (groups after `group_base`), their static far/coarse levels,
/// and the skirts of the Blender-built buildings.
pub fn prepare(world: Arc<StreetWorld>, group_base: u32, authored: &mut AuthoredSet, city: &City, palette: Option<StreetPalette>) -> (StreetSet, StreetStatic, Vec<Skirt>) {
    cc0_survey(city); // before any street tile places its decals
    let t = std::time::Instant::now();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let (set, st) = StreetSet::prepare(world.clone(), group_base, palette, threads);
    eprintln!("startup: street tiles {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = std::time::Instant::now();
    let by_id: std::collections::HashMap<&str, &eo_data::Building> = city.buildings.iter().map(|b| (b.id.as_str(), b)).collect();
    let list: Vec<_> = authored.buildings.iter().filter_map(|b| by_id.get(b.record_key.as_str()).map(|r| (*r, b.placement.base_y, b.placement.frame.edge_index))).collect();
    let chunk = list.len().div_ceil(threads).max(1);
    let skirts: Vec<Skirt> = std::thread::scope(|s| {
        let hs: Vec<_> = list
            .chunks(chunk)
            .map(|c| {
                let world = &world;
                s.spawn(move || {
                    c.iter()
                        .map(|(rec, js_base, edge)| {
                            let ring: Vec<[f64; 2]> = rec.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
                            let record: Vec<[f64; 2]> = rec.record_polygon().iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
                            let (base, _) = world.authored_floor(&record, *edge, *js_base);
                            let f = world.foundation(&ring, base, world.o.archetype_foundation_skirt_m);
                            Skirt { ring: rec.ring.clone(), base: base as f32, bottom: f.bottom_y as f32 }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    // Raise each model to its floor: one translation in front of its placement.
    let mut raised = 0;
    for (b, s) in authored.buildings.iter_mut().zip(&skirts) {
        let dy = s.base as f64 - b.placement.base_y;
        if dy.abs() > 1e-6 {
            raised += 1;
            b.placement.world_from_model = glam::DMat4::from_translation(glam::DVec3::new(0.0, dy, 0.0)) * b.placement.world_from_model;
            b.placement.lod_center.y += dy;
            b.placement.base_y += dy;
            if let Some(g) = authored.groups.get_mut(b.group as usize) {
                g.center[1] += dy as f32;
            }
        }
    }
    eprintln!("startup: {raised} Blender-built buildings raised to the walk");
    eprintln!("startup: authored skirts {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    (set, st, skirts)
}
