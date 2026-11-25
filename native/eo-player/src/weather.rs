//! Weather physics for vehicles and on-foot movement: wet-pavement tire grip,
//! standing-puddle hydroplaning and splash drag, and aerodynamic wind / crosswind
//! forces.
//!
//! Pure and deterministic (f64, zero randomness, no wall-clock reads). The
//! default state (`WeatherPhysics::default()`) is dry and calm and is an exact
//! no-op, leaving baseline physics tests and replays unchanged.

use glam::DVec3;

/// Standard sea-level air density, kg/m^3.
const AIR_DENSITY_KG_M3: f64 = 1.225;
/// Speed where standing water starts building a hydrodynamic wedge under a tire, m/s (~22 mph).
const HYDROPLANE_START_MPS: f64 = 10.0;
/// Speed where hydroplaning reaches its plateau on a deep puddle, m/s (~63 mph).
const HYDROPLANE_FULL_MPS: f64 = 28.0;

/// Weather state consumed by `eo-player` physics steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherPhysics {
    /// Road-surface water film wetness, 0 (dry) .. 1 (saturated).
    pub wetness: f64,
    /// Standing-water puddle accumulation, 0 (none) .. 1 (flooded).
    pub puddle_fill: f64,
    /// Rainfall rate, mm/h (0 clear, 0.5 drizzle, 1.8 light rain, 3.5 sun-rain,
    /// 6 medium rain, 16 heavy rain, 22 storm).
    pub rain_mm_h: f64,
    /// Horizontal wind velocity vector (m/s, x east, z south).
    pub wind: [f64; 2],
    /// Gust factor (~0.65..1.35; 1.0 steady).
    pub gust: f64,
}

impl Default for WeatherPhysics {
    fn default() -> Self {
        Self { wetness: 0.0, puddle_fill: 0.0, rain_mm_h: 0.0, wind: [0.0, 0.0], gust: 1.0 }
    }
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash2(x: f64, z: f64) -> f64 {
    let s = (x * 127.1 + z * 311.7).sin() * 43758.5453123;
    s - s.floor()
}

fn value_noise(x: f64, z: f64) -> f64 {
    let ix = x.floor();
    let iz = z.floor();
    let fx = x - ix;
    let fz = z - iz;
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uz = fz * fz * (3.0 - 2.0 * fz);
    let a = hash2(ix, iz);
    let b = hash2(ix + 1.0, iz);
    let c = hash2(ix, iz + 1.0);
    let d = hash2(ix + 1.0, iz + 1.0);
    (a + (b - a) * ux) + ((c + (d - c) * ux) - (a + (b - a) * ux)) * uz
}

impl WeatherPhysics {
    /// True when the state has no wet surface, rain or wind effect.
    pub fn is_dry_calm(&self) -> bool {
        self.wetness <= 1e-4 && self.puddle_fill <= 1e-4 && self.rain_mm_h <= 1e-4 && self.wind[0].hypot(self.wind[1]) <= 1e-4
    }

    /// Effective wind vector (m/s, x east, z south), including gust factor.
    pub fn effective_wind(&self) -> [f64; 2] {
        let g = self.gust.clamp(0.5, 1.6);
        [self.wind[0] * g, self.wind[1] * g]
    }

    /// Deterministic standing-water puddle depth `0..1` at world `(x, z)`,
    /// matching the renderer's multi-scale basin scales (`4.2 m`, `1.55 m`, `0.52 m`).
    pub fn puddle_depth_at(&self, x: f64, z: f64) -> f64 {
        let w = self.wetness.clamp(0.0, 1.0);
        if w <= 0.02 {
            return 0.0;
        }
        let fill = (self.puddle_fill + smoothstep(2.5, 20.0, self.rain_mm_h) * 0.14).clamp(0.0, 1.0);
        if fill <= 0.005 {
            return 0.0;
        }
        let n_lg = value_noise(x / 4.2 + 7.3, z / 4.2 + 19.1);
        let n_md = value_noise(x / 1.55 + 31.7, z / 1.55 + 4.9);
        let n_sm = value_noise(x / 0.52 + 11.4, z / 0.52 + 27.3);
        let topo = 0.50 * n_lg + 0.35 * n_md + 0.15 * n_sm;
        let thr = 0.92 + (0.45 - 0.92) * fill.powf(0.52) + 0.02;
        let mask = smoothstep(thr - 0.035, thr + 0.035, topo);
        (((topo - thr) / 0.28).clamp(0.0, 1.0) * mask * w).clamp(0.0, 1.0)
    }

    /// Per-wheel `(friction_slip_scale, side_stiffness_scale)` at `(x, z)` and
    /// travel speed `speed_mps`. Dry pavement returns `(1.0, 1.0)`.
    pub fn tyre_grip_scale(&self, x: f64, z: f64, speed_mps: f64) -> (f64, f64) {
        let w = self.wetness.clamp(0.0, 1.0);
        if w <= 0.01 {
            return (1.0, 1.0);
        }
        // Wet asphalt film lowers rubber friction ~22% and cornering stiffness ~16%.
        let rain_film = smoothstep(1.0, 22.0, self.rain_mm_h) * 0.06;
        let mut slip = 1.0 - (0.20 * w + rain_film);
        let mut lat = 1.0 - (0.14 * w + rain_film * 0.8);
        // Standing water hydroplaning at higher road speeds.
        let pud = self.puddle_depth_at(x, z);
        if pud > 0.02 {
            let hydro = smoothstep(HYDROPLANE_START_MPS, HYDROPLANE_FULL_MPS, speed_mps.abs()) * pud;
            slip -= 0.34 * hydro;
            lat -= 0.40 * hydro;
        }
        (slip.clamp(0.40, 1.0), lat.clamp(0.42, 1.0))
    }

    /// Standing-water splash resistance force (N) on one wheel moving at `speed_mps`
    /// through puddle depth `puddle_depth` (`0..1`, up to ~12 mm standing water).
    pub fn wheel_puddle_drag_n(&self, speed_mps: f64, puddle_depth: f64) -> f64 {
        if puddle_depth <= 0.02 || speed_mps.abs() < 0.2 {
            return 0.0;
        }
        // Water displacement drag ~ 0.5 * rho_w * C_d * (tyre_width * h_water) * v^2.
        let h_water_m = 0.012 * puddle_depth.clamp(0.0, 1.0);
        let tyre_width_m = 0.22;
        let c_splash = 0.65;
        0.5 * 1000.0 * c_splash * tyre_width_m * h_water_m * speed_mps * speed_mps
    }

    /// Extra aerodynamic wind & crosswind force vector (N, world XZ) on a vehicle
    /// chassis relative to still air (still-air longitudinal drag is already in
    /// `VehicleSpec::resistance_force`).
    pub fn vehicle_wind_force_n(&self, vel: DVec3, forward: DVec3, width_m: f64, height_m: f64, length_m: f64) -> DVec3 {
        let [wx, wz] = self.effective_wind();
        let w_speed = wx.hypot(wz);
        if w_speed <= 0.25 {
            return DVec3::ZERO;
        }
        let fwd = DVec3::new(forward.x, 0.0, forward.z).normalize_or(DVec3::Z);
        let right = DVec3::new(fwd.z, 0.0, -fwd.x);
        let wind_v = DVec3::new(wx, 0.0, wz);
        let rel_air = wind_v - DVec3::new(vel.x, 0.0, vel.z);
        let still_air = -DVec3::new(vel.x, 0.0, vel.z);
        // Longitudinal (headwind / tailwind) delta over still-air drag.
        let area_front = width_m * height_m * 0.82;
        let cd_front = 0.34;
        let r_fwd = rel_air.dot(fwd);
        let s_fwd = still_air.dot(fwd);
        let f_fwd = 0.5 * AIR_DENSITY_KG_M3 * cd_front * area_front * (r_fwd * r_fwd.abs() - s_fwd * s_fwd.abs());
        // Crosswind push on the car's broadside profile.
        let area_side = length_m * height_m * 0.78;
        let cd_side = 0.85;
        let w_side = wind_v.dot(right);
        let f_side = 0.5 * AIR_DENSITY_KG_M3 * cd_side * area_side * w_side * w_side.abs();
        fwd * f_fwd + right * f_side
    }

    /// On-foot movement modifiers `(speed_scale, wind_drift_x_mps, wind_drift_z_mps)`
    /// for walking/running through puddles and strong storm winds.
    pub fn on_foot_modifiers(&self, x: f64, z: f64, move_dir_x: f64, move_dir_z: f64) -> (f64, f64, f64) {
        if self.is_dry_calm() {
            return (1.0, 0.0, 0.0);
        }
        let [wx, wz] = self.effective_wind();
        let w_speed = wx.hypot(wz);
        // Slight wading drag in deep flooded puddles (up to ~8% slower in heavy rain/storm).
        let pud = self.puddle_depth_at(x, z);
        let mut speed_scale = 1.0 - 0.08 * pud;
        // Headwind slows movement gently in gales (> 6 m/s); tailwind helps slightly.
        let m_len = move_dir_x.hypot(move_dir_z);
        if m_len > 1e-4 && w_speed > 3.5 {
            let along = (wx * move_dir_x + wz * move_dir_z) / (m_len * w_speed);
            let gale = smoothstep(3.5, 14.0, w_speed);
            speed_scale *= (1.0 + 0.07 * along * gale).clamp(0.86, 1.04);
        }
        // Subtle lateral wind push in heavy rain / coastal storms (> 7 m/s).
        let push = smoothstep(7.0, 15.0, w_speed) * 0.22;
        let (dx, dz) = if w_speed > 1e-4 { (wx / w_speed * push, wz / w_speed * push) } else { (0.0, 0.0) };
        (speed_scale.clamp(0.82, 1.04), dx, dz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_calm_default_is_an_exact_no_op() {
        let dry = WeatherPhysics::default();
        assert!(dry.is_dry_calm());
        assert_eq!(dry.puddle_depth_at(12.3, -45.6), 0.0);
        assert_eq!(dry.tyre_grip_scale(12.3, -45.6, 25.0), (1.0, 1.0));
        assert_eq!(dry.wheel_puddle_drag_n(25.0, 0.0), 0.0);
        assert_eq!(dry.vehicle_wind_force_n(DVec3::new(0.0, 0.0, 15.0), DVec3::Z, 1.85, 1.45, 4.6), DVec3::ZERO);
        assert_eq!(dry.on_foot_modifiers(12.3, -45.6, 0.0, 1.0), (1.0, 0.0, 0.0));
    }

    #[test]
    fn wet_grip_hydroplaning_puddle_drag_and_storm_wind_scale_physically() {
        let light = WeatherPhysics { wetness: 0.70, puddle_fill: 0.25, rain_mm_h: 1.8, wind: [2.8, 0.7], gust: 1.0 };
        let heavy = WeatherPhysics { wetness: 0.99, puddle_fill: 0.85, rain_mm_h: 16.0, wind: [7.8, 2.5], gust: 1.0 };
        let storm = WeatherPhysics { wetness: 0.99, puddle_fill: 0.90, rain_mm_h: 22.0, wind: [11.5, 4.2], gust: 1.2 };

        // Find a flooded puddle coordinate under storm conditions.
        let mut p_coord = None;
        for ix in 0..40 {
            for iz in 0..40 {
                let (x, z) = (ix as f64 * 0.7, iz as f64 * 0.7);
                if storm.puddle_depth_at(x, z) > 0.5 {
                    p_coord = Some((x, z));
                    break;
                }
            }
            if p_coord.is_some() {
                break;
            }
        }
        let (px, pz) = p_coord.expect("storm should pool standing puddles across the street");
        assert!(storm.puddle_depth_at(px, pz) >= heavy.puddle_depth_at(px, pz));
        assert!(heavy.puddle_depth_at(px, pz) > light.puddle_depth_at(px, pz));

        let g_slow = storm.tyre_grip_scale(px, pz, 5.0);
        let g_fast = storm.tyre_grip_scale(px, pz, 26.0);
        assert!(g_slow.0 < 1.0 && g_fast.0 < g_slow.0, "hydroplaning lowers grip: slow={g_slow:?} fast={g_fast:?}");
        assert!(g_fast.1 < g_slow.1, "hydroplaning lowers lateral stiffness");

        let drag = storm.wheel_puddle_drag_n(20.0, storm.puddle_depth_at(px, pz));
        assert!(drag > 150.0, "puddle splash drag at 20 m/s: {drag} N");

        // Crosswind from +X on a car driving +Z produces lateral +X force.
        let f_wind = storm.vehicle_wind_force_n(DVec3::new(0.0, 0.0, 15.0), DVec3::Z, 1.85, 1.45, 4.6);
        assert!(f_wind.x > 100.0, "storm crosswind push: {f_wind:?}");

        // Headwind slows on-foot walk into the storm compared to tailwind.
        let head = storm.on_foot_modifiers(px, pz, -11.5, -4.2);
        let tail = storm.on_foot_modifiers(px, pz, 11.5, 4.2);
        assert!(head.0 < tail.0, "headwind slower than tailwind: head={head:?} tail={tail:?}");
    }
}
