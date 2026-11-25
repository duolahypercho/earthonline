//! The JS game's look, as pure functions: sun position, the analytic sky and
//! its dome, distance fog, the solved light rig, exposure, the tone curve and
//! grade, and the sun shadow cascades. No GPU. Every module names the JS file
//! it was ported from; `tests/js_oracle.rs` compares the ports with numbers
//! the JS computed (the earlier browser version's exporter ->
//! data/look-oracle.json).
//!
//! The renderer asks for one `LookFrame` per (hour, camera heading) and one
//! set of cascades per camera.

pub mod aerial;
pub mod blend;
pub mod bounce;
pub mod cinematic;
pub mod golden;
pub mod ground_field;
pub mod look;
pub mod math;
pub mod occluder;
pub mod penumbra;
pub mod rig;
pub mod screen_gi;
pub mod shadow;
pub mod sky;
pub mod sun;
pub mod sun_bounce;
pub mod tone;

use math::*;

/// Everything the renderer needs for one time of day (camera-independent
/// except the fog colour, which follows the camera's heading as in the JS).
#[derive(Clone, Copy, Debug)]
pub struct LookFrame {
    pub hour: f64,
    pub model: sky::SkyModel,
    pub rig: rig::Rig,
    /// Toward the key light (the sun while it is up).
    pub key_dir: [f64; 3],
    /// Key colour x intensity (three's directional light radiance).
    pub key: Rgb,
    pub rim_dir: [f64; 3],
    pub rim: Rgb,
    /// Hemisphere light: sky and ground lobes x intensity.
    pub hemi_sky: Rgb,
    pub hemi_ground: Rgb,
    pub ambient: Rgb,
    /// Irradiance of the analytic environment (sky + ground) as 9 SH
    /// coefficients, already cosine-convolved: E(n) = sum c_i Y_i(n).
    pub env_sh: [Rgb; 9],
    pub exposure: f64,
    pub aerial: aerial::Aerial,
    /// 0..1: how much of the key the shadow may take (`lightRig.shadow`
    /// intensity x the key envelope, so 0 with the sun down).
    pub shadow_strength: f64,
    /// The drawn solar disc: radiance (rgb) and angular radius (radians).
    pub disc: Rgb,
    pub disc_radius: f64,
    /// the open sky's irradiance luminance on an upward normal
    /// (`probeDiffuseIrradiance(model, { normal: up })`, intensity 1): the
    /// reference a scene bake's transfer is measured against.
    pub sky_irradiance_up: f64,
}

/// Map span the fog's depth pair is graded from (the loaded city's larger
/// extent); past 3.4 km the pair is the same.
pub const DEFAULT_MAP_SPAN_M: f64 = 13784.0;

/// SH basis (real, l <= 2) in the usual order.
pub(crate) fn sh_basis(d: [f64; 3]) -> [f64; 9] {
    let [x, y, z] = d;
    [
        0.282095,
        0.488603 * y,
        0.488603 * z,
        0.488603 * x,
        1.092548 * x * y,
        1.092548 * y * z,
        0.315392 * (3.0 * z * z - 1.0),
        1.092548 * x * z,
        0.546274 * (x * x - y * y),
    ]
}

/// Cosine-convolved SH9 of the environment radiance `sample`.
fn irradiance_sh(model: &sky::SkyModel) -> [Rgb; 9] {
    let rings = 32;
    let sectors = 64;
    let mut c = [[0.0; 3]; 9];
    let mut wsum = 0.0;
    for i in 0..rings {
        let theta = (i as f64 + 0.5) / rings as f64 * std::f64::consts::PI;
        let (st, ct) = theta.sin_cos();
        for j in 0..sectors {
            let phi = (j as f64 + 0.5) / sectors as f64 * TAU;
            let d = [st * phi.cos(), ct, st * phi.sin()];
            let l = model.sample(d);
            let y = sh_basis(d);
            for k in 0..9 {
                for ch in 0..3 {
                    c[k][ch] += l[ch] * y[k] * st;
                }
            }
            wsum += st;
        }
    }
    let norm = 4.0 * std::f64::consts::PI / wsum;
    let band = [std::f64::consts::PI, 2.0 * std::f64::consts::PI / 3.0, std::f64::consts::PI / 4.0];
    for k in 0..9 {
        let a = band[if k == 0 { 0 } else if k < 4 { 1 } else { 2 }];
        for ch in 0..3 {
            c[k][ch] *= norm * a;
        }
    }
    c
}

/// (`--dome-env`, eo-render look.rs) the environment's colour from the sky the
/// frame paints. The diffuse environment and the rough reflections (the
/// shaders' `lk_sh`: the cosine-convolved SH9 of the environment) take the
/// analytic dome (`SkyModel::sample`), while the sky pass paints it graded
/// (`SkyModel::dome`: San Francisco's haze desaturating and lifting the low
/// sky, the horizon's haze band). Noon's horizon is painted b/r 1.1 and
/// integrated b/r 2.5 (the analytic dome's 9 at the zenith): a rough street
/// at a grazing view reflected a deep blue sky the frame never shows. Here
/// each direction of the upper hemisphere keeps the analytic dome's luminance
/// (the energy the rig's yield is solved against) and takes the painted
/// dome's chromaticity; the lower hemisphere (the street field) is as it was.
/// Returns (the analytic SH, this SH) on the same `DOME_ENV_RINGS` x
/// `DOME_ENV_SECTORS` grid of the sphere, cosine-convolved, so a caller scales
/// the second by its own yield over the first.
pub const DOME_ENV_RINGS: usize = 16;
pub const DOME_ENV_SECTORS: usize = 32;

pub fn dome_env_sh(model: &sky::SkyModel) -> ([Rgb; 9], [Rgb; 9]) {
    let (rings, sectors) = (DOME_ENV_RINGS, DOME_ENV_SECTORS);
    let mut a = [[0.0; 3]; 9];
    let mut c = [[0.0; 3]; 9];
    let mut wsum = 0.0;
    for i in 0..rings {
        let theta = (i as f64 + 0.5) / rings as f64 * std::f64::consts::PI;
        let (st, ct) = theta.sin_cos();
        for j in 0..sectors {
            let phi = (j as f64 + 0.5) / sectors as f64 * TAU;
            let d = [st * phi.cos(), ct, st * phi.sin()];
            let l = model.sample(d);
            let m = if d[1] >= 0.0 {
                let p = model.dome(d);
                let (lp, ll) = (luminance(p), luminance(l));
                if lp > 1e-12 { scale3(p, ll / lp) } else { l }
            } else {
                l
            };
            let y = sh_basis(d);
            for k in 0..9 {
                for ch in 0..3 {
                    a[k][ch] += l[ch] * y[k] * st;
                    c[k][ch] += m[ch] * y[k] * st;
                }
            }
            wsum += st;
        }
    }
    let norm = 4.0 * std::f64::consts::PI / wsum;
    let band = [std::f64::consts::PI, 2.0 * std::f64::consts::PI / 3.0, std::f64::consts::PI / 4.0];
    for k in 0..9 {
        let w = band[if k == 0 { 0 } else if k < 4 { 1 } else { 2 }] * norm;
        for ch in 0..3 {
            a[k][ch] *= w;
            c[k][ch] *= w;
        }
    }
    (a, c)
}

/// (`--dome-env`) a frame's environment SH `env` (the analytic dome at the
/// frame's yield and weather) with the painted dome's colour: `dome_env_sh`'s
/// second SH, scaled by the frame's own SH over the analytic one facing up
/// (luminance; the frame's yield). Unchanged where the analytic dome is dark.
pub fn env_sh_with_dome_colour(model: &sky::SkyModel, env: &[Rgb; 9]) -> [Rgb; 9] {
    let (a, c) = dome_env_sh(model);
    let up = [0.0, 1.0, 0.0];
    let la = luminance(sh_eval(&a, up));
    if la <= 1e-12 {
        return *env;
    }
    let k = luminance(sh_eval(env, up)) / la;
    c.map(|v| scale3(v, k))
}

/// Evaluate a cosine-convolved SH9 (CPU reference of the shader).
pub fn sh_eval(c: &[Rgb; 9], n: [f64; 3]) -> Rgb {
    let y = sh_basis(n);
    let mut out = [0.0; 3];
    for k in 0..9 {
        for ch in 0..3 {
            out[ch] += c[k][ch] * y[k];
        }
    }
    out
}

impl LookFrame {
    pub fn new(hour: f64) -> Self {
        Self::with_transfer(hour, None)
    }

    /// the frame with the rig solved for a scene bake whose transfer
    /// was measured (`rig::rig_at_transfer`); None: the JS fallback yield.
    pub fn with_transfer(hour: f64, transfer: Option<f64>) -> Self {
        Self::with_sky_profile(hour, transfer, sky::Profile::clear_with_look(&look::LOOK)) // sky
    }

    /// the frame under another atmosphere profile (`weatherProfile` of a
    /// weather bucket: overcast, isotropy, desaturation, brightness, tint,
    /// urban glow, turbidity); the clear profile gives `with_transfer`'s frame.
    pub fn with_sky_profile(hour: f64, transfer: Option<f64>, profile: sky::Profile) -> Self {
        Self::with_sky_profile_interp(hour, transfer, profile, rig::Interp::Linear)
    }

    /// `with_sky_profile` with the rig rows (and the key's intensity between
    /// them) interpolated by `interp`; `Linear` is `with_sky_profile`.
    pub fn with_sky_profile_interp(hour: f64, transfer: Option<f64>, profile: sky::Profile, interp: rig::Interp) -> Self {
        let model = sky::SkyModel::with_profile(hour, profile, true); // was SkyModel::new(hour)
        let rig = match transfer {
            Some(t) => rig::rig_at_transfer_interp(hour, t, interp),
            None => rig::rig_at_interp(hour, interp),
        };
        let key_dir = rig::key_direction(hour);
        let alt = model.sun.altitude_deg;
        // The table's intensity carries the envelope and the transmittance of
        // its own rows; re-evaluate both at the exact hour.
        let row_env = |h: f64| {
            let s = sun::sun_direction(h);
            rig::key_envelope(s.altitude_deg) * rig::direct_beam_transmittance(s.altitude_deg)
        };
        let t = rig::table();
        let i = ((sun::wrap_hour(hour) / 0.25).floor() as usize).min(t.len() - 2);
        let w = clamp((sun::wrap_hour(hour) - t[i].hour) / 0.25, 0.0, 1.0);
        let base = |r: &rig::RigRow| {
            let e = row_env(r.hour);
            if e > 1e-9 { r.sun_intensity / e } else { 0.0 }
        };
        let (ba, bb) = (base(&t[i]), base(&t[i + 1]));
        let unenveloped = match interp {
            rig::Interp::Linear => {
                if ba > 0.0 && bb > 0.0 { mix(ba, bb, w) } else { ba.max(bb) }
            }
            // the cubic through the rows' unenveloped keys; a row with the sun down
            // (no key to unenvelope) is left out as the linear rule leaves it out.
            rig::Interp::MonotoneCubic => {
                if ba > 0.0 && bb > 0.0 {
                    let (rows, wc) = rig::cubic_rows(t.len(), |k| t[k].hour, hour);
                    let y = rows.map(|k| base(&t[k]));
                    let y0 = if y[0] > 0.0 { y[0] } else { y[1] };
                    let y3 = if y[3] > 0.0 { y[3] } else { y[2] };
                    rig::monotone_cubic([y0, y[1], y[2], y3], wc)
                } else {
                    ba.max(bb)
                }
            }
        };
        let reference = rig::direct_beam_transmittance(sun::sun_direction(rig::EXPOSURE_REFERENCE_HOUR).altitude_deg);
        let key_i = unenveloped * rig::key_envelope(alt) * rig::direct_beam_transmittance(alt);
        let key = scale3(rig.sun_color, key_i);
        let rim_dir = match interp {
            rig::Interp::Linear => rig::rim_direction(key_dir),
            rig::Interp::MonotoneCubic => rig::rim_direction_smooth(hour), // no half turn as the key passes overhead
        };
        let exposure = rig::recommended_exposure(&model, rig.light_rig_sun_scale);
        let aerial = aerial::aerial_perspective(&model, DEFAULT_MAP_SPAN_M);
        // the analytic sky's irradiance delivered at the bound
        // probe's yield (see `rig::table`): the scene bake the JS binds
        // delivers ~2-3 % of it by day, the rest of the fill being the
        // topped-up hemisphere and ambient the rig rows carry.
        let sky_sh = irradiance_sh(&model);
        let sky_irradiance_up = luminance(sh_eval(&sky_sh, [0.0, 1.0, 0.0]));
        let env_sh = sky_sh.map(|c| scale3(c, rig.env_diffuse_scale));
        // Disc (sky-atmosphere.js placeSkyBodies): 0.5334 deg across.
        let beam_ratio = clamp(rig::direct_beam_transmittance(alt) / reference, 0.0, 1.0);
        let disc_rad = (26.0 + 46.0 * model.daylight) * beam_ratio.max(0.25);
        let sw = model.sunward_horizon_radiance;
        let swl = luminance(sw).max(1e-6);
        let redden = clamp(1.0 - beam_ratio, 0.0, 1.0).powf(0.8);
        let tint = [mix(1.0, sw[0] / swl, redden), mix(0.94, sw[1] / swl, redden), mix(0.84, sw[2] / swl, redden)];
        let tl = luminance(tint).max(1e-6);
        let above = smoothstep(-0.6, 0.3, alt);
        let disc = scale3(tint, disc_rad * above / tl * luminance([1.0, 0.94, 0.84]));
        Self {
            hour,
            model,
            rig,
            key_dir,
            key,
            rim_dir,
            rim: scale3(rig.rim_color, rig.rim_intensity),
            hemi_sky: scale3(rig.hemi_sky, rig.hemi_intensity),
            hemi_ground: scale3(rig.hemi_ground, rig.hemi_intensity),
            ambient: scale3(rig.ambient_color, rig.ambient_intensity),
            env_sh,
            exposure,
            aerial,
            shadow_strength: clamp(rig.shadow_intensity * rig::key_envelope(alt), 0.0, 1.0),
            disc,
            disc_radius: 0.5334 * 0.5 * DEG,
            sky_irradiance_up,
        }
    }

    /// this frame with the rig re-solved for another measured transfer
    /// (None: the fallback yield), without re-integrating the sky: only the
    /// fill (hemisphere, ambient, rim, environment intensity, the ground lobe)
    /// and the diffuse environment's scale move. Equal to `with_transfer`.
    pub fn retarget(&self, transfer: Option<f64>) -> Self {
        self.retarget_interp(transfer, rig::Interp::Linear)
    }

    /// `retarget` with the rig rows interpolated by `interp` (`Linear` is
    /// `retarget`); equal to `with_sky_profile_interp` with the same `interp`.
    pub fn retarget_interp(&self, transfer: Option<f64>, interp: rig::Interp) -> Self {
        let rig = match transfer {
            Some(t) => rig::rig_at_transfer_interp(self.hour, t, interp),
            None => rig::rig_at_interp(self.hour, interp),
        };
        let k = if self.rig.env_diffuse_scale > 1e-12 { rig.env_diffuse_scale / self.rig.env_diffuse_scale } else { 0.0 };
        let mut f = *self;
        f.env_sh = if self.rig.env_diffuse_scale > 1e-12 { self.env_sh.map(|c| scale3(c, k)) } else { irradiance_sh(&self.model).map(|c| scale3(c, rig.env_diffuse_scale)) };
        f.rim = scale3(rig.rim_color, rig.rim_intensity);
        f.hemi_sky = scale3(rig.hemi_sky, rig.hemi_intensity);
        f.hemi_ground = scale3(rig.hemi_ground, rig.hemi_intensity);
        f.ambient = scale3(rig.ambient_color, rig.ambient_intensity);
        f.rig = rig;
        f
    }

    pub fn fog_color(&self, forward_x: f64, forward_z: f64) -> Rgb {
        aerial::fog_color(&self.model, forward_x, forward_z)
    }
}

#[cfg(test)]
mod dome_env_tests {
    use super::*;

    #[test]
    fn the_dome_env_keeps_the_light_and_takes_the_painted_colour() {
        for h in [9.0, 12.0, 15.0, 18.5] {
            let lf = LookFrame::new(h);
            let (a, c) = dome_env_sh(&lf.model);
            // the same energy every way (each direction keeps its luminance): within 2 %.
            for n in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, -1.0, 0.0], [0.6, 0.3, 0.74]] {
                let (ea, ec) = (luminance(sh_eval(&a, n)), luminance(sh_eval(&c, n)));
                assert!((ec / ea - 1.0).abs() < 0.02, "{h} {n:?}: {ea} {ec}");
            }
            // the frame's yield carried over.
            let e = env_sh_with_dome_colour(&lf.model, &lf.env_sh);
            let up = [0.0, 1.0, 0.0];
            assert!((luminance(sh_eval(&e, up)) / luminance(sh_eval(&lf.env_sh, up)) - 1.0).abs() < 0.02, "{h}");
        }
        // noon: the low sky as painted (hazy, near grey) instead of the analytic deep blue.
        let lf = LookFrame::new(12.0);
        let (a, c) = dome_env_sh(&lf.model);
        let low = [0.0, 0.15, 0.988];
        let (ea, ec) = (sh_eval(&a, low), sh_eval(&c, low));
        println!("noon low sky b/r: analytic {:.2}, painted colour {:.2}", ea[2] / ea[0], ec[2] / ec[0]);
        assert!(ec[2] / ec[0] < 0.7 * (ea[2] / ea[0]));
        // the street below is as it was.
        let down = [0.0, -1.0, 0.0];
        let (da, dc) = (sh_eval(&a, down), sh_eval(&c, down));
        assert!((dc[2] / dc[0]) / (da[2] / da[0]) > 0.9);
    }
}
