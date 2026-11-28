//! (`--bikeways`, `StreetOptions::bikeways`) Kerbside bike lanes joined to
//! the city's bike network (eo-data `bikeways`, DataSF ygmz-vaxd): Folsom
//! Street 4th to 5th is a two-way bikeway behind a concrete island with no
//! green paint recorded, Folsom 5th to Falmouth a green lane with safe-hit
//! posts; the drawn street carries the island (a kerb-high top in the
//! buffer), the posts, the two-way centre line and the symbols. Its own test
//! binary: the switch is process-wide (eo-street `options::set_bikeways`).

use eo_data::{City, DataPaths, ElevationGrid};
use eo_street::{BuildingIndex, Layer, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;

#[test]
fn folsom_bikeways_follow_the_network() {
    eo_street::options::set_bikeways(true);
    let paths = DataPaths::discover().unwrap();
    if !paths.sf(eo_data::bikeways::FILE).is_file() {
        return;
    }
    let city = City::load(&paths).unwrap();
    let elev = ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    assert!(o.bikeways);
    let lift = o.open_ground_lift;
    let curb = o.curb_face_height;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let w = StreetWorld::build(o, StreetCity::load().unwrap(), Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    assert!(w.bike_dims.is_some());
    let joined = w.bikeways.iter().filter(|b| b.is_some()).count();
    assert!(joined > 1000, "{joined} runs joined");
    let s = eo_data::groundscale::scale();
    let near = |x: f64, z: f64| {
        let (x, z) = (x * s, z * s);
        (0..w.plan.entries.len())
            .filter(|&ei| w.bikeway(ei).is_some())
            .min_by(|&a, &b| {
                let d = |ei: usize| w.plan.entries[ei].points.iter().map(|p| (p[0] - x).hypot(p[1] - z)).fold(f64::MAX, f64::min);
                d(a).total_cmp(&d(b))
            })
            .unwrap()
    };
    // Folsom 4th to 5th (city metres): two-way, concrete island, no green
    let f = near(1500.0, -347.0);
    let st = *w.bikeway(f).unwrap();
    assert_eq!(st.cnn, 5664000);
    assert!(st.two_way && st.island && !st.green && !st.posts, "{st:?}");
    assert!(!w.lines[f].iter().any(|l| l.paint == eo_street::paint::Paint::Green), "no green on the 4th-5th block");
    // its drawn island: a kerb-top surface standing a kerb's height over the carriageway beside it
    let key = eo_street::tile_key(1500.0 * s, -347.0 * s);
    let mesh = w.tile_mesh(key);
    let tops = mesh.layers.iter().filter(|l| **l == Layer::CurbTop).count();
    assert!(tops > 0);
    // Folsom 5th to Falmouth: green with posts
    let g = near(1360.0, -205.0);
    let sg = *w.bikeway(g).unwrap();
    assert_eq!(sg.cnn, 5665000);
    assert!(sg.green && sg.posts && !sg.island, "{sg:?}");
    assert!(w.lines[g].iter().any(|l| l.paint == eo_street::paint::Paint::Green));
    let _ = curb;
}
