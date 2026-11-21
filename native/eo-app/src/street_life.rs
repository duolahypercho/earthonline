//! Street life (`--street-life 1|2`, stage W2b; eo-render street_life.rs):
//! the level and the survey its decals place by. The survey is each
//! building's land use, from the building-metadata packs
//! (public/data/sf/building-metadata/v1, then v1-pack: the first record of an
//! id wins, as eo-kit's tenancy reads them) and, for a building with no
//! record, its OSM tags; and the freeway decks (`--structures`). It is read on
//! a thread of its own beside the load; a street tile that places before it
//! is ready waits for it. Nothing runs without the switch.

use eo_render::street_life::LandUse;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Deserialize)]
struct Index {
    tiles: HashMap<String, TileRef>,
}

#[derive(Deserialize)]
struct TileRef {
    file: String,
}

#[derive(Deserialize)]
struct Tile {
    #[serde(default)]
    buildings: HashMap<String, Record>,
}

#[derive(Deserialize)]
struct Record {
    #[serde(rename = "use", default)]
    use_: Option<String>,
}

/// Building id -> the packs' `use`; the first pack that records an id wins.
pub fn load_uses(repo_root: &Path) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for base in ["public/data/sf/building-metadata/v1", "public/data/sf/building-metadata/v1-pack"] {
        let dir = repo_root.join(base);
        let Ok(text) = std::fs::read(dir.join("index.json")) else { continue };
        let Ok(index) = serde_json::from_slice::<Index>(&text) else {
            eprintln!("street life: {base}/index.json not read");
            continue;
        };
        let mut files: Vec<PathBuf> = index.tiles.values().map(|t| dir.join(&t.file)).collect();
        files.sort();
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
        let chunk = files.len().div_ceil(threads).max(1);
        let parts: Vec<Vec<(String, String)>> = std::thread::scope(|s| {
            let hs: Vec<_> = files
                .chunks(chunk)
                .map(|c| {
                    s.spawn(move || {
                        eo_render::hitch::worker_started();
                        let mut v = vec![];
                        for f in c {
                            let Ok(bytes) = std::fs::read(f) else { continue };
                            let Ok(tile) = serde_json::from_slice::<Tile>(&bytes) else { continue };
                            let mut recs: Vec<(String, String)> = tile.buildings.into_iter().filter_map(|(id, r)| r.use_.map(|u| (id, u))).collect();
                            recs.sort();
                            v.extend(recs);
                        }
                        v
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        for (id, u) in parts.into_iter().flatten() {
            out.entry(id).or_insert(u);
        }
    }
    out
}

/// Each building's land use, in `City::buildings` order (the street world's building index).
pub fn classes(city: &eo_data::City, uses: &HashMap<String, String>) -> Vec<u8> {
    city.buildings
        .iter()
        .map(|b| {
            let rec = uses.get(&b.id).or_else(|| uses.get(b.id.trim_start_matches("sf-building-")));
            rec.and_then(|u| LandUse::from_metadata(u)).unwrap_or_else(|| LandUse::from_osm(&b.tags.building, &b.tags.amenity)) as u8
        })
        .collect()
}

/// With `--street-life`: the placement level, and the survey read on a
/// thread of its own (the street tiles wait for it). Without: nothing.
pub fn start(level: u8, city: Arc<eo_data::City>, repo_root: PathBuf, structures: Option<Arc<eo_structures::StructureWorld>>) {
    eo_render::street_life::set_level(level);
    if level == 0 {
        return;
    }
    let spawned = std::thread::Builder::new().name("eo-street-life".into()).spawn(move || {
        eo_render::hitch::worker_started();
        let t = Instant::now();
        let uses = load_uses(&repo_root);
        let cls = classes(&city, &uses);
        let mut count = [0usize; 8];
        for &c in &cls {
            count[c as usize & 7] += 1;
        }
        eprintln!(
            "street life: level {level}; {} building uses read, {} buildings: unknown {}, house {}, flats {}, office {}, retail {}, industrial {}, vacant {}, civic {}; decks {} ({:.0} ms)",
            uses.len(),
            cls.len(),
            count[0],
            count[1],
            count[2],
            count[3],
            count[4],
            count[5],
            count[6],
            count[7],
            if structures.is_some() { "on" } else { "off" },
            t.elapsed().as_secs_f64() * 1e3
        );
        eo_render::street_life::set_survey(cls, structures);
    });
    if let Err(e) = spawned {
        eprintln!("street life: survey thread not started ({e}); placing without it");
        eo_render::street_life::set_survey(vec![], None);
    }
}
