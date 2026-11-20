//! Latitude, longitude and height above mean sea level to game coordinates
//! (`eo_data::geo`) on the real city: the frame is the city's own, and a
//! point on the surveyed ground lands on the drawn ground.
//!
//!   cargo test --release -p eo-data --test geo -- --nocapture

use eo_data::geo::GeoFrame;
use eo_data::roof_colours::PlanFrame;
use eo_data::*;

fn paths() -> DataPaths {
    DataPaths::discover().expect("repo data")
}

#[test]
fn the_frame_is_the_city_projection_and_ground_points_land_on_the_drawn_ground() {
    let p = paths();
    let city = City::load(&p).expect("city loads");
    let grid = ElevationGrid::load(&p).expect("elevation loads");
    let f = GeoFrame::of_city(&city.meta);
    assert_eq!(f.scale, groundscale::scale());
    assert_eq!(f.height_scale, groundscale::height_scale());
    assert_eq!(grid.height_scale, f.height_scale);
    // The land-cover frame (`Frame::sf`, SF_FRAME) is the same projection.
    let sf = Frame::sf();
    let pf = PlanFrame::from_meta(&city.meta);
    assert_eq!((pf.center_lat, pf.center_lon, pf.m_per_deg_lat), (sf.center_lat, sf.center_lon, sf.m_per_deg_lat));
    assert!((pf.m_per_deg_lon - sf.m_per_deg_lon).abs() < 1e-6, "{} {}", pf.m_per_deg_lon, sf.m_per_deg_lon);
    // Points on land (Twin Peaks, Telegraph Hill, the Ferry Building, Golden
    // Gate Park, Mount Sutro): the surveyed ground's height above mean sea
    // level lands on the drawn ground, and back.
    let mut worst: f64 = 0.0;
    for (lat, lon) in [(37.7544, -122.4477), (37.8024, -122.4058), (37.7955, -122.3937), (37.7694, -122.4862), (37.7580, -122.4580)] {
        let [x, z] = f.plan_xz(lat, lon);
        let g = grid.sample_surveyed(x, z).expect("inside the grid");
        let alt = g - SEA_LEVEL_Y;
        let q = f.to_game(lat, lon, alt, Some(&grid));
        let drawn = grid.sample(x, z).unwrap();
        worst = worst.max((q.p[1] - drawn).abs());
        assert!((q.p[1] - drawn).abs() < 1e-9, "({lat}, {lon}): {} vs the drawn ground {drawn}", q.p[1]);
        assert!((q.ground_y.unwrap() - drawn).abs() < 1e-9);
        // 50 m above that ground is 50 m above the drawn ground.
        let up = f.to_game(lat, lon, alt + 50.0, Some(&grid));
        assert!((up.p[1] - drawn - 50.0).abs() < 1e-9);
        let (lat2, lon2, alt2) = f.from_game(up.p, Some(&grid));
        assert!((lat2 - lat).abs() < 1e-12 && (lon2 - lon).abs() < 1e-12 && (alt2 - alt - 50.0).abs() < 1e-9);
        eprintln!("geo: ({lat}, {lon}) -> game ({:.2}, {:.2}, {:.2}); surveyed ground {alt:.2} m above mean sea level, drawn ground y {drawn:.2}", q.p[0], q.p[1], q.p[2]);
    }
    eprintln!("geo: worst ground mismatch {worst:.2e} m at scale {} / heights {}", f.scale, f.height_scale);
}
