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
pub mod atmosphere;
pub mod features;
pub mod ground_field;
pub mod look;
pub mod math;
pub mod phys;
pub mod rig;
pub mod season; // the grass's dryness by the day of the year (one curve for every land layer)
pub mod shadow;
pub mod sky;
pub mod spectral; // the direct sun's colour from its spectrum (`--no-sun-spectrum2`)
pub mod sun;
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
fn sh_basis(d: [f64; 3]) -> [f64; 9] {
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
        let model = sky::SkyModel::with_profile(hour, profile, true); // was SkyModel::new(hour)
        let rig = match transfer {
            Some(t) => rig::rig_at_transfer(hour, t),
            None => rig::rig_at(hour),
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
        let unenveloped = if ba > 0.0 && bb > 0.0 { mix(ba, bb, w) } else { ba.max(bb) };
        let reference = rig::direct_beam_transmittance(sun::sun_direction(rig::EXPOSURE_REFERENCE_HOUR).altitude_deg);
        let key_i = unenveloped * rig::key_envelope(alt) * rig::direct_beam_transmittance(alt);
        let key = scale3(rig.sun_color, key_i);
        let rim_dir = rig::rim_direction(key_dir);
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
        let rig = match transfer {
            Some(t) => rig::rig_at_transfer(self.hour, t),
            None => rig::rig_at(self.hour),
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
