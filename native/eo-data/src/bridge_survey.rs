//! City bridge road elevations. Source, datum and extraction are recorded in
//! data/bridge-roads.NOTICE.txt. Each profile follows one identified OSM way.
pub use crate::bridge_readings::*;

/// A surveyed roadway in the same vertical datum and scale as the terrain.
pub fn game_y(navd88_m: f64) -> f64 {
    crate::groundscale::lift(navd88_m + crate::NAVD88_TO_GAME_Y)
}

/// Mariposa Street crosses the railway below I-280. The southbound entrance
/// shares its elevation at the fork and then climbs under a separate off-ramp.
/// At the open ends, blend the last measured interval into the adjoining road.
pub fn mariposa_grade(way: &str, at: [f64; 2], terrain: f64) -> Option<f64> {
    let (p, axis) = match way.strip_prefix("sf-street-").unwrap_or(way) {
        "396965550" | "544431982" | "544431983" | "623399444" => (MARIPOSA_STREET, 0),
        "8915249" | "1111444346" | "945101895" => (MARIPOSA_RAMP, 1),
        _ => return None,
    };
    let s = at[axis] / crate::groundscale::scale();
    let last = p.len() - 1;
    let end = (p[last][axis] - s) / (p[last][axis] - p[last - 1][axis]);
    let start = if axis == 0 { (s - p[0][axis]) / (p[1][axis] - p[0][axis]) } else { 1.0 };
    let weight = start.min(end).clamp(0.0, 1.0);
    (weight > 0.0).then(|| terrain + weight * (interpolate(p, s, axis) - terrain))
}

/// The street and service drive at the Saint Charles bridge run on
/// measured paving, including where the terrain grid follows the bank below.
pub fn driveway_grade(way: &str, at: [f64; 2], terrain: f64) -> Option<f64> {
    let (profile, axis) = match way.strip_prefix("sf-street-").unwrap_or(way) {
        "8917011" => (SAINT_CHARLES, 1),
        "459121915" => (SAINT_CHARLES_DRIVE, 0),
        _ => return None,
    };
    let s = at[axis] / crate::groundscale::scale();
    let weight = ((s - profile[0][axis]) / (profile[1][axis] - profile[0][axis])).clamp(0.0, 1.0);
    Some(terrain + weight * (interpolate(profile, s, axis) - terrain))
}

/// Road datum at a registered approach. Blend surveyed pavement into
/// unchanged street at the open edge of a survey. The overlapping plaza
/// lanes share their transition. The underpass follows northing as it curves.
pub fn approach(way: &str, at: [f64; 2], terrain: f64) -> Option<f64> {
    let way = way.strip_prefix("sf-street-").unwrap_or(way);
    let p = match way {
        "50690291" => RINCON_LOWER,
        "739661211" => RINCON_MAIN,
        "179235237" => RINCON_RAMP,
        "60319196" => RINCON_UNDERPASS,
        "1034687991" => ISLAND_UNDERPASS,
        "123741452" => FEEDER_123741452,
        "468201421" => FEEDER_468201421,
        "27772070" => FEEDER_27772070,
        "397094807" => FEEDER_397094807,
        "839972871" => FEEDER_839972871,
        "1212176124" => ISLAND_LANDING,
        "26968357" => PLAZA_26968357,
        "920648684" => PLAZA_920648684,
        "920648685" => PLAZA_920648685,
        "920648686" => PLAZA_920648686,
        "920648689" => PLAZA_920648689,
        _ => return None,
    };
    let scale = crate::groundscale::scale();
    let x = at[0] / scale;
    let blend = |v: f64, a: f64, b: f64| ((v - a) / (b - a)).clamp(0.0, 1.0);
    let weight = match way {
        "60319196" | "1034687991" => {
            let z = at[1] / scale;
            let last = p.len() - 1;
            let w = blend(z, p[0][1], p[1][1]).min(blend(z, p[last][1], p[last - 1][1]));
            return Some(terrain + w * (interpolate(p, z, 1) - terrain));
        }
        "1212176124" | "123741452" | "27772070" | "468201421" | "397094807" => {
            blend(x, p[0][0], p[p.len() - 1][0])
        }
        "26968357" | "920648684" | "920648685" | "920648686" | "920648689" => {
            blend(x, PLAZA_26968357.last()?[0], PLAZA_26968357[0][0])
        }
        _ => 1.0,
    };
    Some(terrain + weight * (interpolate(p, x, 0) - terrain))
}

/// The two parallel east-span ways. All sampled roads increase monotonically
/// in source easting, including through the curve off Yerba Buena Island.
pub fn east_span(way: &str) -> Option<&'static [[f64; 3]]> {
    match way {
        "236348361" => Some(EAST_WESTBOUND),
        "237731428" => Some(EAST_EASTBOUND),
        _ => None,
    }
}

/// Interpolate along an identified way. The street export snaps its ends
/// to neighbouring ways, so the nearest endpoint also owns that short seam.
pub fn height_at(readings: &[[f64; 3]], game_x: f64) -> f64 {
    interpolate(readings, game_x / crate::groundscale::scale(), 0)
}

/// Closed movable leaves at Islais Creek run north–south. The survey samples
/// the deck at both hinges and midspan, including the road on the rail bridge.
pub fn movable_height(way: &str, game_z: f64) -> Option<f64> {
    let profile = match way {
        "667768321" => ISLAIS_667768321,
        "667768323" => ISLAIS_667768323,
        "675892536" => ISLAIS_675892536,
        _ => return None,
    };
    Some(interpolate(profile, game_z / crate::groundscale::scale(), 1))
}

fn interpolate(readings: &[[f64; 3]], x: f64, axis: usize) -> f64 {
    let i = readings
        .partition_point(|p| p[axis] < x)
        .clamp(1, readings.len() - 1);
    let (a, b) = (readings[i - 1], readings[i]);
    let t = ((x - a[axis]) / (b[axis] - a[axis])).clamp(0.0, 1.0);
    game_y(a[2] + (b[2] - a[2]) * t)
}
