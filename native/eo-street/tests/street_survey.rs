//! (`--survey-parking`, `--midblock-crossings`) The city's parking and
//! mid-block crossings on Folsom Street 4th to 5th: the piece the street
//! data drew with no parking beside its bike lane gets the parking lane the
//! city's meters show, so it is as wide as the rest of the block (no kerb jog
//! at the joint), and the mid-block signal (DataSF Traffic Signals, CNN
//! 5664000 MIDBLOCK) is a crossing on the run with kerb ramps both sides. Its
//! own test binary: the switches are process-wide.

use eo_data::{City, DataPaths, ElevationGrid};
use eo_street::{BuildingIndex, StreetCity, StreetOptions, StreetWorld, Terrain};
use std::sync::Arc;

#[test]
fn folsom_parking_and_midblock_crossing_from_the_survey() {
    let paths = DataPaths::discover().unwrap();
    if !paths.sf(eo_data::street_survey::FILE).is_file() {
        return;
    }
    let before = StreetCity::load().unwrap();
    eo_street::options::set_survey_parking(true);
    eo_street::options::set_midblock(true);
    let sc = StreetCity::load().unwrap();
    let id = |c: &StreetCity, sid: &str| c.segments.iter().position(|s| s.street_id.as_deref() == Some(sid)).unwrap();
    // the narrow piece (OSM way 617990571) and its neighbour (1052634341)
    let (narrow0, narrow1) = (&before.segments[id(&before, "sf-street-617990571")], &sc.segments[id(&sc, "sf-street-617990571")]);
    let wide = &sc.segments[id(&sc, "sf-street-1052634341")];
    assert!(narrow0.parking[0] <= 0.0 && narrow1.parking[0] > 0.0, "parking given on the bike lane's side");
    assert!((narrow1.width - wide.width).abs() < 0.05, "the block one width: {} vs {}", narrow1.width, wide.width);
    // the building line held: carriageway half plus footway unchanged on both sides
    for (w0, w1) in [(narrow0.sidewalk_left, narrow1.sidewalk_left), (narrow0.sidewalk_right, narrow1.sidewalk_right)] {
        let (w0, w1) = (w0.unwrap(), w1.unwrap());
        assert!((narrow0.width / 2.0 + w0 - (narrow1.width / 2.0 + w1)).abs() < 1e-6);
    }
    let city = City::load(&paths).unwrap();
    let elev = ElevationGrid::load(&paths).unwrap();
    let o = StreetOptions::load().unwrap();
    assert!(o.survey_parking && o.midblock);
    assert!(o.structures_tag().contains("midblock"), "kit tiles keyed on the moved kerbs: {}", o.structures_tag());
    let lift = o.open_ground_lift;
    let rings = city.buildings.iter().map(|b| b.ring.iter().map(|p| [p[0] as f64, p[1] as f64]).collect()).collect();
    let w = StreetWorld::build(o, sc, Terrain { grid: Arc::new(elev), carpet_lift: lift }, BuildingIndex::new(rings));
    let n: usize = w.midblock.iter().map(|v| v.len()).sum();
    assert!(n >= 30, "{n} mid-block crossings");
    // Folsom's: on a Folsom run, with a kerb ramp on each kerb at it
    let s = eo_data::groundscale::scale();
    let at = [1509.59 * s, -348.68 * s];
    let (ei, c) = w
        .midblock
        .iter()
        .enumerate()
        .flat_map(|(ei, v)| v.iter().map(move |c| (ei, *c)))
        .min_by(|a, b| {
            let d = |(ei, c): &(usize, eo_street::MidCrossing)| {
                let f = w.plan.entries[*ei].frame(c.s, false);
                (f.x - at[0]).hypot(f.z - at[1])
            };
            d(a).total_cmp(&d(b))
        })
        .unwrap();
    let e = &w.plan.entries[ei];
    let f = e.frame(c.s, false);
    assert!((f.x - at[0]).hypot(f.z - at[1]) < e.half + 8.0 * s);
    for k in 0..2 {
        assert!(e.kerb_ramps[k].iter().any(|r| r.lo <= c.s && r.hi >= c.s), "a ramp on kerb {k}");
    }
    assert!(!w.cut_midblock(ei, c.s - 20.0, c.s + 20.0).iter().any(|p| p[0] <= c.s && p[1] >= c.s), "the lane lines broken across it");
}
