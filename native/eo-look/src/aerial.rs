//! Distance fog: `aerialPerspective` (src/render/environment-ibl.js) with the
//! depth pair the sky pass hands it (`passAerial` in
//! src/render/passes/sky-atmosphere.js), and the fog colour the sky pass
//! writes into `scene.fog` every frame (`skylineRadiance` + `applyFogVeil`):
//! the dome's own colour 2 degrees above the horizon in the camera's heading,
//! clamped to the graded horizon, then re-graded toward the dome at the look's
//! veil elevation. three's range fog is `smoothstep(near, far, viewDepth)`.
//!
//! Not ported: the night skyglow/depth terms the sky pass adds to the skyline
//! sample (they follow the night practicals), the ground-haze shells, and the
//! post chain's own aerial-path stage.

use crate::look::LOOK;
use crate::math::*;
use crate::sky::SkyModel;
use crate::sun::wrap_hour;

/// `FAR_FIELD_REACH_M` in sky-atmosphere.js.
pub const FAR_FIELD_REACH_M: f64 = 1600.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aerial {
    pub near: f64,
    pub far: f64,
    pub haze_density: f64,
    pub low_sun: f64,
}

pub fn morning_inversion(hour: f64) -> f64 {
    let h = wrap_hour(hour);
    smoothstep(3.8, 6.2, h) * (1.0 - smoothstep(8.0, 11.0, h))
}

fn round_to(v: f64, k: f64) -> f64 {
    // JS Math.round: half up.
    (v * k + 0.5).floor() / k
}

/// `passAerial(model, mapSpan)` -> `aerialPerspective`, clear weather.
pub fn aerial_perspective(m: &SkyModel, map_span: f64) -> Aerial {
    let span = clamp(if map_span > 0.0 { map_span } else { 2000.0 }, 120.0, 40000.0);
    let near0 = clamp(span * 0.10, 240.0, 340.0);
    let far0 = clamp((span * 1.55).max(FAR_FIELD_REACH_M * 1.03), 520.0, 2600.0);
    let (grade_near, grade_far) = (1.0, 1.18);
    let low_sun = m.daylight * (1.0 - smoothstep(0.0, 30.0, m.sun.altitude_deg));
    let inversion = morning_inversion(m.hour);
    let night_pull = m.night * 0.22;
    let vis = LOOK.aerial_visibility_scale;
    let near_scale = clamp(grade_near * vis * (1.0 - 0.30 * low_sun) * (1.0 - 0.28 * inversion) * (1.0 - night_pull), 0.08, 3.5);
    let far_scale = clamp(grade_far * vis * (1.0 - 0.22 * low_sun) * (1.0 - 0.16 * inversion) * (1.0 - night_pull), 0.10, 4.0);
    let near = near0 * near_scale;
    let mut far = far0 * far_scale;
    if !(far > near) {
        far = near * 1.35;
    }
    let min_sep = 24f64.max(span * 0.05);
    if far - near < min_sep {
        far = near + min_sep;
    }
    let haze = clamp((0.10 * (1.0 + 1.15 * inversion) + 0.06 * low_sun) * LOOK.aerial_haze_density_scale, 0.0, 0.85);
    Aerial { near: round_to(near, 1000.0), far: round_to(far, 1000.0), haze_density: round_to(haze, 10000.0), low_sun }
}

const SKYLINE_ELEVATION_DEG: f64 = 2.0;

/// The fog colour for a camera heading (`applyFogColor`, day terms only).
pub fn fog_color(m: &SkyModel, forward_x: f64, forward_z: f64) -> Rgb {
    let h = forward_x.hypot(forward_z);
    let (nx, nz) = if h > 1e-6 { (forward_x / h, forward_z / h) } else { (0.0, -1.0) };
    let (ss, sc) = ((SKYLINE_ELEVATION_DEG * DEG).sin(), (SKYLINE_ELEVATION_DEG * DEG).cos());
    let mut out = m.dome([nx * sc, ss, nz * sc]);
    let base = m.dome_horizon(m.horizon_radiance, 1.0);
    let level = luminance(out);
    let ceiling = luminance(base);
    if level > ceiling && level > 1e-9 {
        out = scale3(out, ceiling / level);
    }
    for c in 0..3 {
        out[c] = clamp(out[c], 0.0, base[c]);
    }
    // applyFogVeil (clear weather, the look's veil).
    let share = clamp(LOOK.veil_share, 0.0, 1.0);
    if share > 0.0 {
        let e = LOOK.veil_elevation_deg * DEG;
        let sample = m.dome([nx * e.cos(), e.sin(), nz * e.cos()]);
        let sl = luminance(sample);
        let target = luminance(out) * LOOK.veil_luminance_scale;
        if sl > 1e-9 {
            let k = target / sl;
            for c in 0..3 {
                out[c] += (sample[c] * k - out[c]) * share;
            }
        }
    }
    out
}
