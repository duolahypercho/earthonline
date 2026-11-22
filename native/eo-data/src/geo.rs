//! Geodetic positions (WGS84 latitude and longitude in degrees, height in
//! metres above mean sea level) to game coordinates and back, and the
//! look-at pose of the common web-globe URL convention.
//!
//! **Plan.** The city data is projected by `sf-city.json` `meta` (`PlanFrame`):
//! x east, z south, metres about `center`, `metersPerDegree*` per degree.
//! The drawn ground plan is that times `groundscale::scale()` about the map
//! origin (the approved ground-plan deviation, 1.25 by default), so
//! `plan_xz(lat, lon) = PlanFrame::to_plan(lon, lat) * scale` and its inverse
//! is the existing `PlanFrame::drawn_to_lonlat`.
//!
//! **Height.** The elevation grid is NAVD88 (`sf-elevation.bin.gz` header
//! `verticalDatum`), stored as game y with `NAVD88_TO_GAME_Y`; mean sea level
//! is 0.969 m above NAVD88 there, so a height `alt` above mean sea level is
//! surveyed game y `alt + SEA_LEVEL_Y`. The drawn ground lifts every height
//! above mean sea level by `groundscale::height_scale()` (`lift`); buildings,
//! landmarks and models stand on that ground at their real size. A point is
//! therefore placed at its height above the surveyed ground, kept over the
//! drawn ground: `y = alt + SEA_LEVEL_Y + (lift(g) - g)`, `g` the surveyed
//! ground there (`ElevationGrid::sample_surveyed`). A point on the ground
//! lands on the drawn ground, a point 40 m up a tower lands 40 m up the drawn
//! tower, and over the sea (`g` at or below mean sea level) nothing is lifted.
//!
//! **Look-at pose.** `(lat, lon, alt)` is the target; the camera stands
//! `range` metres from it; `heading` is the compass bearing the camera faces
//! (0 north, 90 east); `tilt` is the angle from straight down (0 looks
//! straight down, 90 at the horizon). `camera = target - range * forward`,
//! `forward = (sin(tilt) sin(heading) east, sin(tilt) cos(heading) north,
//! -cos(tilt) up)` in the target's local east-north-up frame, evaluated on
//! the WGS84 ellipsoid (`look_at_camera`). Both points are then converted to
//! the game, where the camera looks at the target: the bearing is kept (the
//! plan scale is the same in x and z), and the tilt and range change with
//! the plan and height scales.

use crate::roof_colours::PlanFrame;
use crate::{CityMeta, ElevationGrid, SEA_LEVEL_Y, groundscale};

/// WGS84 semi-major axis, metres.
pub const WGS84_A: f64 = 6_378_137.0;
/// WGS84 flattening.
pub const WGS84_F: f64 = 1.0 / 298.257_223_563;

fn e2() -> f64 {
    WGS84_F * (2.0 - WGS84_F)
}

/// Geodetic (degrees, metres above the ellipsoid) to earth-centred, earth-fixed metres.
pub fn ecef(lat_deg: f64, lon_deg: f64, h: f64) -> [f64; 3] {
    let (sp, cp) = lat_deg.to_radians().sin_cos();
    let (sl, cl) = lon_deg.to_radians().sin_cos();
    let n = WGS84_A / (1.0 - e2() * sp * sp).sqrt();
    [(n + h) * cp * cl, (n + h) * cp * sl, (n * (1.0 - e2()) + h) * sp]
}

/// Earth-centred, earth-fixed metres to geodetic (degrees, metres above the ellipsoid).
pub fn geodetic(p: [f64; 3]) -> (f64, f64, f64) {
    let lon = p[1].atan2(p[0]);
    let r = p[0].hypot(p[1]);
    let mut lat = p[2].atan2(r * (1.0 - e2()));
    let mut h = 0.0;
    for _ in 0..12 {
        let s = lat.sin();
        let n = WGS84_A / (1.0 - e2() * s * s).sqrt();
        h = r / lat.cos() - n;
        lat = p[2].atan2(r * (1.0 - e2() * n / (n + h)));
    }
    (lat.to_degrees(), lon.to_degrees(), h)
}

/// The local east, north and up unit vectors (earth-fixed) at a geodetic point.
pub fn enu_basis(lat_deg: f64, lon_deg: f64) -> [[f64; 3]; 3] {
    let (sp, cp) = lat_deg.to_radians().sin_cos();
    let (sl, cl) = lon_deg.to_radians().sin_cos();
    [[-sl, cl, 0.0], [-sp * cl, -sp * sl, cp], [cp * cl, cp * sl, sp]]
}

/// The look direction of a (heading, tilt) pose in east-north-up components.
pub fn look_forward_enu(heading_deg: f64, tilt_deg: f64) -> [f64; 3] {
    let (sh, ch) = heading_deg.to_radians().sin_cos();
    let (st, ct) = tilt_deg.to_radians().sin_cos();
    [st * sh, st * ch, -ct]
}

/// The camera of a look-at pose: `target - range * forward` on the WGS84
/// ellipsoid, as (lat, lon, height). Heights above mean sea level are used as
/// heights above the ellipsoid: the geoid's change over a few kilometres
/// (decimetres) is left out.
pub fn look_at_camera(lat: f64, lon: f64, alt: f64, range: f64, heading_deg: f64, tilt_deg: f64) -> (f64, f64, f64) {
    let t = ecef(lat, lon, alt);
    let [e, n, u] = enu_basis(lat, lon);
    let f = look_forward_enu(heading_deg, tilt_deg);
    let c = [0, 1, 2].map(|k| t[k] - range * (f[0] * e[k] + f[1] * n[k] + f[2] * u[k]));
    geodetic(c)
}

/// The game's geodetic frame: the survey projection, the ground-plan scale
/// and the height scale.
#[derive(Clone, Copy, Debug)]
pub struct GeoFrame {
    pub plan: PlanFrame,
    /// `groundscale::scale()`: the drawn plan about the map origin.
    pub scale: f64,
    /// `groundscale::height_scale()`: heights above mean sea level on the drawn ground.
    pub height_scale: f64,
}

/// A point converted, with the ground under it.
#[derive(Clone, Copy, Debug)]
pub struct GamePoint {
    /// Game metres: x east, y up, z south.
    pub p: [f64; 3],
    /// The surveyed ground there, metres above mean sea level (None outside the elevation grid).
    pub ground_msl: Option<f64>,
    /// The drawn ground there, game y (None outside the elevation grid).
    pub ground_y: Option<f64>,
}

impl GeoFrame {
    pub fn new(plan: PlanFrame, scale: f64, height_scale: f64) -> Self {
        GeoFrame { plan, scale, height_scale }
    }

    /// This process's frame: the city's projection with the resolved plan and height scales.
    pub fn of_city(meta: &CityMeta) -> Self {
        Self::new(PlanFrame::from_meta(meta), groundscale::scale(), groundscale::height_scale())
    }

    /// (lat, lon) to the drawn ground plan, game metres (x east, z south).
    pub fn plan_xz(&self, lat: f64, lon: f64) -> [f64; 2] {
        let (x, z) = self.plan.to_plan(lon, lat);
        [x * self.scale, z * self.scale]
    }

    /// A drawn plan point (game x, z) to (lat, lon): the existing `PlanFrame::drawn_to_lonlat`.
    pub fn lat_lon(&self, x: f64, z: f64) -> (f64, f64) {
        let (lon, lat) = self.plan.drawn_to_lonlat(x, z, self.scale);
        (lat, lon)
    }

    /// What the drawn ground adds to a surveyed height (game y) there.
    fn lift_delta(&self, surveyed_ground_y: Option<f64>) -> f64 {
        surveyed_ground_y.map(|g| groundscale::lift_by(g, self.height_scale) - g).unwrap_or(0.0)
    }

    /// Game y of a height `alt` metres above mean sea level, over a point whose
    /// surveyed ground is `surveyed_ground_y` (game y, `ElevationGrid::sample_surveyed`).
    pub fn game_y(&self, alt_msl: f64, surveyed_ground_y: Option<f64>) -> f64 {
        alt_msl + SEA_LEVEL_Y + self.lift_delta(surveyed_ground_y)
    }

    /// The inverse of `game_y`.
    pub fn alt_msl(&self, y: f64, surveyed_ground_y: Option<f64>) -> f64 {
        y - SEA_LEVEL_Y - self.lift_delta(surveyed_ground_y)
    }

    /// (lat, lon, metres above mean sea level) to game metres. `grid` must be
    /// loaded at this frame's scales (`ElevationGrid::load` in this process);
    /// without it the height is the plain datum shift.
    pub fn to_game(&self, lat: f64, lon: f64, alt_msl: f64, grid: Option<&ElevationGrid>) -> GamePoint {
        let [x, z] = self.plan_xz(lat, lon);
        let g = grid.and_then(|g| g.sample_surveyed(x, z));
        GamePoint { p: [x, self.game_y(alt_msl, g), z], ground_msl: g.map(|g| g - SEA_LEVEL_Y), ground_y: g.map(|g| groundscale::lift_by(g, self.height_scale)) }
    }

    /// Game metres to (lat, lon, metres above mean sea level).
    pub fn from_game(&self, p: [f64; 3], grid: Option<&ElevationGrid>) -> (f64, f64, f64) {
        let (lat, lon) = self.lat_lon(p[0], p[2]);
        let g = grid.and_then(|g| g.sample_surveyed(p[0], p[2]));
        (lat, lon, self.alt_msl(p[1], g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(scale: f64, h: f64) -> GeoFrame {
        // sf-city.json meta (the elevation grid's header names the same frame).
        GeoFrame::new(PlanFrame { center_lat: 37.778, center_lon: -122.4194, m_per_deg_lat: 110574.0, m_per_deg_lon: 87986.24747640654 }, scale, h)
    }

    #[test]
    fn ecef_round_trips() {
        for (lat, lon, h) in [(37.778, -122.4194, 0.0), (37.8199, -122.4783, 227.0), (-33.9, 151.2, -40.0), (89.9, 10.0, 5000.0)] {
            let (a, b, c) = geodetic(ecef(lat, lon, h));
            assert!((a - lat).abs() < 1e-11 && (b - lon).abs() < 1e-11 && (c - h).abs() < 1e-6, "{lat} {lon} {h} -> {a} {b} {c}");
        }
    }

    #[test]
    fn look_at_follows_the_url_convention() {
        let (lat, lon, alt) = (37.7955, -122.3937, 20.0);
        // tilt 0 looks straight down: the camera is `range` above the target.
        let (a, b, c) = look_at_camera(lat, lon, alt, 500.0, 37.0, 0.0);
        assert!((a - lat).abs() < 1e-9 && (b - lon).abs() < 1e-9 && (c - (alt + 500.0)).abs() < 1e-6);
        // heading 0, tilt 90: the camera faces north, so it stands south of the target.
        let (a, b, _) = look_at_camera(lat, lon, alt, 1000.0, 0.0, 90.0);
        assert!(a < lat && (b - lon).abs() < 1e-9);
        // heading 90: it faces east and stands west.
        let (a, b, c) = look_at_camera(lat, lon, alt, 1000.0, 90.0, 90.0);
        assert!(b < lon && (a - lat).abs() < 1e-4);
        // at the horizon the camera stands 1 km out on the tangent plane: the
        // ellipsoid has fallen away below it by about d^2 / 2R.
        assert!((c - alt - 1000.0f64.powi(2) / (2.0 * 6_371_000.0)).abs() < 0.01, "{c}");
        // heading 45, tilt 60: the horizontal and vertical parts of the range.
        let f = look_forward_enu(45.0, 60.0);
        assert!((f[0] - f[1]).abs() < 1e-12 && (f[2] + 0.5).abs() < 1e-12);
        assert!(((f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn lat_lon_round_trips_through_the_existing_unprojections() {
        for s in [1.0, 1.25] {
            let f = frame(s, s);
            for (lat, lon) in [(37.778, -122.4194), (37.7952, -122.4028), (37.8024, -122.4058), (37.7897, -122.3972), (37.8199, -122.4783), (37.7083, -122.5150)] {
                let [x, z] = f.plan_xz(lat, lon);
                // PlanFrame::drawn_to_lonlat (roof colours) and lib::Frame::unproject (land cover) on the surveyed plan.
                let (lon2, lat2) = f.plan.drawn_to_lonlat(x, z, s);
                assert!((lat2 - lat).abs() < 1e-12 && (lon2 - lon).abs() < 1e-12, "scale {s}: {lat} {lon} -> {lat2} {lon2}");
                let (lat3, lon3) = crate::Frame::sf().unproject(x / s, z / s);
                assert!((lat3 - lat).abs() < 1e-12 && (lon3 - lon).abs() < 1e-12, "scale {s}: Frame::unproject {lat3} {lon3}");
                let p = f.to_game(lat, lon, 12.5, None);
                let (lat4, lon4, alt4) = f.from_game(p.p, None);
                assert!((lat4 - lat).abs() < 1e-12 && (lon4 - lon).abs() < 1e-12 && (alt4 - 12.5).abs() < 1e-9);
            }
            // The centre is the map origin; one degree of latitude north is
            // metersPerDegreeLat x scale toward -z.
            let o = f.plan_xz(37.778, -122.4194);
            assert!(o[0].abs() < 1e-9 && o[1].abs() < 1e-9);
            let n = f.plan_xz(37.778 + 1e-3, -122.4194);
            assert!((n[1] + 110.574 * s).abs() < 1e-6 && n[0].abs() < 1e-9);
        }
    }

    #[test]
    fn heights_keep_their_height_over_the_drawn_ground() {
        let f = frame(1.25, 1.25);
        // Mean sea level is game y SEA_LEVEL_Y; over the sea nothing is lifted.
        assert!((f.game_y(0.0, None) - SEA_LEVEL_Y).abs() < 1e-12);
        assert!((f.game_y(30.0, Some(SEA_LEVEL_Y - 4.0)) - (30.0 + SEA_LEVEL_Y)).abs() < 1e-12);
        // A point on ground 80 m above the sea lands on the drawn ground (80 x 1.25),
        // and a point 40 m above that ground 40 m above the drawn ground.
        let g = 80.0 + SEA_LEVEL_Y;
        assert!((f.game_y(80.0, Some(g)) - (SEA_LEVEL_Y + 100.0)).abs() < 1e-9);
        assert!((f.game_y(120.0, Some(g)) - (SEA_LEVEL_Y + 140.0)).abs() < 1e-9);
        assert!((f.alt_msl(SEA_LEVEL_Y + 140.0, Some(g)) - 120.0).abs() < 1e-9);
        // With heights at 1.0 it is the plain NAVD88 datum shift.
        let f1 = frame(1.25, 1.0);
        assert!((f1.game_y(120.0, Some(g)) - (120.0 + SEA_LEVEL_Y)).abs() < 1e-12);
        // NAVD88 zero is 0.969 m below mean sea level.
        assert!((crate::NAVD88_TO_GAME_Y - (SEA_LEVEL_Y - 0.969)).abs() < 1e-12);
    }
}
