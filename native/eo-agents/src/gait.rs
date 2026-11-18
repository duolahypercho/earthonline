//! The walk cycle's odometer (`advanceGaitPhase` and the `GAIT` cadence and
//! stride model in src/simulation/pedestrians/pedestrian-presentation.js):
//! the clip phase advances by distance walked over the stride, so feet do not
//! slide whatever the speed. The simulated walkers carry their own gait phase
//! (eo-sim); this is for the player body, which eo-player does not phase.

/// Leg length as a share of body height, and the reference body height.
pub const LEG_OF_HEIGHT: f64 = 0.526;
pub const REFERENCE_HEIGHT_M: f64 = 1.75;

/// Steps a second at ground speed `v` for a body `height` tall.
pub fn cadence(v: f64, height: f64) -> f64 {
    let leg = LEG_OF_HEIGHT * height;
    let leg_ref = LEG_OF_HEIGHT * REFERENCE_HEIGHT_M;
    ((1.02 + 0.62 * v) * (leg_ref / leg)).clamp(0.70, 3.20)
}

/// Metres per full cycle (two steps).
pub fn stride(v: f64, height: f64) -> f64 {
    let leg = LEG_OF_HEIGHT * height;
    (2.0 * v / cadence(v, height)).min(3.0 * leg)
}

/// Advance a 0..1 cycle phase by `dt` seconds at ground speed `v`.
pub fn advance_phase(phase: f64, v: f64, dt: f64, height: f64) -> f64 {
    if v <= 1e-4 {
        return phase;
    }
    (phase + v * dt / stride(v, height).max(1e-3)).rem_euclid(1.0)
}

/// `locomotionBlend`'s walk ramp: 0 standing, 1 walking (smoothstep 0.15..0.60 m/s).
pub fn walk_weight(v: f64) -> f64 {
    let t = ((v - 0.15) / (0.60 - 0.15)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stride_at_walking_pace() {
        // 1.4 m/s on the reference body: cadence 1.888 steps/s, 1.48 m a cycle.
        assert!((cadence(1.4, 1.75) - 1.888).abs() < 1e-9);
        assert!((stride(1.4, 1.75) - 1.4830508).abs() < 1e-6);
    }
}
