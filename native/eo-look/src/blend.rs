//! Blending two look frames a short clock interval apart (`--smooth-time`:
//! the renderer keeps keyframes a clock minute apart and draws the hour
//! between them). Every plain number is linear between the two; directions
//! go through a normalised slerp; the sun is placed exactly at the hour.
//! What is not a number switches at the keyframe boundary: `SkyExt::on`
//! (the first frame's). A dome grade present on one side only blends against
//! the grade that changes nothing (weight 0, no veil, gain 1, no skyline).

use crate::math::*;
use crate::sky::{Basis, DomeGrade, Field, Preetham, Profile, SkyExt, SkyModel};
use crate::sun::{sun_direction, wrap_hour};
use crate::{LookFrame, aerial::Aerial, rig::Rig};

/// The unit direction `t` of the way from `a` to `b` along their great
/// circle (both unit or near it); exactly `a` at 0 and `b` at 1. Opposite
/// directions turn through the horizontal plane's normal (any circle is as
/// short), nearly equal ones fall back to a normalised lerp.
pub fn slerp_dir(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let norm = |v: [f64; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if l > 1e-12 { [v[0] / l, v[1] / l, v[2] / l] } else { v }
    };
    let (a, b) = (norm(a), norm(b));
    let c = clamp(a[0] * b[0] + a[1] * b[1] + a[2] * b[2], -1.0, 1.0);
    if c > 0.9995 {
        return norm([mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]);
    }
    // nearly opposite: the half turn as two quarter turns through a direction
    // at right angles to `a` (toward the zenith, or x when `a` is vertical).
    if c < -0.9995 {
        let up = if a[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
        let d = up[0] * a[0] + up[1] * a[1] + up[2] * a[2];
        let mid = norm([up[0] - d * a[0], up[1] - d * a[1], up[2] - d * a[2]]);
        return if t < 0.5 { slerp_dir(a, mid, 2.0 * t) } else { slerp_dir(mid, b, 2.0 * t - 1.0) };
    }
    let theta = c.acos();
    let s = theta.sin();
    let (wa, wb) = (((1.0 - t) * theta).sin() / s, (t * theta).sin() / s);
    norm([a[0] * wa + b[0] * wb, a[1] * wa + b[1] * wb, a[2] * wa + b[2] * wb])
}

fn l3(a: Rgb, b: Rgb, t: f64) -> Rgb {
    mix3(a, b, t)
}

fn profile(a: &Profile, b: &Profile, t: f64) -> Profile {
    Profile {
        turbidity: mix(a.turbidity, b.turbidity, t),
        rayleigh: mix(a.rayleigh, b.rayleigh, t),
        mie_coefficient: mix(a.mie_coefficient, b.mie_coefficient, t),
        mie_directional_g: mix(a.mie_directional_g, b.mie_directional_g, t),
        overcast: mix(a.overcast, b.overcast, t),
        isotropy: mix(a.isotropy, b.isotropy, t),
        desaturation: mix(a.desaturation, b.desaturation, t),
        brightness: mix(a.brightness, b.brightness, t),
        tint: l3(a.tint, b.tint, t),
        ground_albedo: l3(a.ground_albedo, b.ground_albedo, t),
        urban_glow: mix(a.urban_glow, b.urban_glow, t),
    }
}

fn preetham(a: &Preetham, b: &Preetham, t: f64) -> Preetham {
    Preetham {
        sun_e: mix(a.sun_e, b.sun_e, t),
        beta_r: l3(a.beta_r, b.beta_r, t),
        beta_m: l3(a.beta_m, b.beta_m, t),
        beta_sum: l3(a.beta_sum, b.beta_sum, t),
        horizon_up: mix(a.horizon_up, b.horizon_up, t),
        g: mix(a.g, b.g, t),
    }
}

fn field(a: &Field, b: &Field, t: f64, sun: crate::sun::Sun) -> Field {
    Field {
        sun,
        state: preetham(&a.state, &b.state, t),
        profile: profile(&a.profile, &b.profile, t),
        mean: l3(a.mean, b.mean, t),
        grade: mix(a.grade, b.grade, t),
        twilight: mix(a.twilight, b.twilight, t),
        night: mix(a.night, b.night, t),
        ozone: mix(a.ozone, b.ozone, t),
        belt: mix(a.belt, b.belt, t),
        shadow_top_sin: mix(a.shadow_top_sin, b.shadow_top_sin, t),
        deep_twilight: mix(a.deep_twilight, b.deep_twilight, t),
        airglow: mix(a.airglow, b.airglow, t),
    }
}

fn basis(a: &Basis, b: &Basis, t: f64) -> Basis {
    Basis { hour: mix(a.hour, b.hour, t), mean: l3(a.mean, b.mean, t), energy_scale: mix(a.energy_scale, b.energy_scale, t), sky_irradiance: l3(a.sky_irradiance, b.sky_irradiance, t) }
}

/// The grade that leaves the dome as it is, shaped like `g` (its weight, veil and skyline 0, its gain 1).
fn neutral_grade(g: &DomeGrade) -> DomeGrade {
    DomeGrade { weight: 0.0, veil_luminance: 0.0, dome_gain: 1.0, skyline_share: 0.0, ..*g }
}

fn grade(a: &Option<DomeGrade>, b: &Option<DomeGrade>, t: f64) -> Option<DomeGrade> {
    let (a, b) = match (a, b) {
        (None, None) => return None,
        (Some(a), Some(b)) => (*a, *b),
        (Some(a), None) => (*a, neutral_grade(a)),
        (None, Some(b)) => (neutral_grade(b), *b),
    };
    Some(DomeGrade {
        weight: mix(a.weight, b.weight, t),
        veil_luminance: mix(a.veil_luminance, b.veil_luminance, t),
        zenith_desaturation: mix(a.zenith_desaturation, b.zenith_desaturation, t),
        horizon_desaturation: mix(a.horizon_desaturation, b.horizon_desaturation, t),
        horizon_lift: mix(a.horizon_lift, b.horizon_lift, t),
        dome_gain: mix(a.dome_gain, b.dome_gain, t),
        skyline_share: mix(a.skyline_share, b.skyline_share, t),
        low_sun_share: mix(a.low_sun_share, b.low_sun_share, t),
        sunward_horizon_desaturation: mix(a.sunward_horizon_desaturation, b.sunward_horizon_desaturation, t),
    })
}

/// Two sky models blended, with the sun placed at `hour`.
pub fn sky_model(a: &SkyModel, b: &SkyModel, t: f64, hour: f64) -> SkyModel {
    let sun = sun_direction(hour);
    SkyModel {
        hour: wrap_hour(hour),
        sun,
        profile: profile(&a.profile, &b.profile, t),
        daylight: mix(a.daylight, b.daylight, t),
        night: mix(a.night, b.night, t),
        field: field(&a.field, &b.field, t, sun),
        basis: basis(&a.basis, &b.basis, t),
        zenith_radiance: l3(a.zenith_radiance, b.zenith_radiance, t),
        horizon_radiance: l3(a.horizon_radiance, b.horizon_radiance, t),
        sunward_horizon_radiance: l3(a.sunward_horizon_radiance, b.sunward_horizon_radiance, t),
        antisun_horizon_radiance: l3(a.antisun_horizon_radiance, b.antisun_horizon_radiance, t),
        ground_radiance: l3(a.ground_radiance, b.ground_radiance, t),
        grade: grade(&a.grade, &b.grade, t),
        ext: SkyExt { on: a.ext.on, afterglow: mix(a.ext.afterglow, b.ext.afterglow, t) },
    }
}

/// The frame `t` (0..1) of the way from `a` to `b`, drawn at `hour` (the
/// clock time between the two frames' hours). `t` 0 is `a`'s numbers, 1 `b`'s.
pub fn frame(a: &LookFrame, b: &LookFrame, t: f64, hour: f64) -> LookFrame {
    let t = clamp(t, 0.0, 1.0);
    let sh: [Rgb; 9] = std::array::from_fn(|k| l3(a.env_sh[k], b.env_sh[k], t));
    LookFrame {
        hour,
        model: sky_model(&a.model, &b.model, t, hour),
        rig: Rig::lerp(&a.rig, &b.rig, t),
        key_dir: slerp_dir(a.key_dir, b.key_dir, t),
        key: l3(a.key, b.key, t),
        rim_dir: slerp_dir(a.rim_dir, b.rim_dir, t),
        rim: l3(a.rim, b.rim, t),
        hemi_sky: l3(a.hemi_sky, b.hemi_sky, t),
        hemi_ground: l3(a.hemi_ground, b.hemi_ground, t),
        ambient: l3(a.ambient, b.ambient, t),
        env_sh: sh,
        exposure: mix(a.exposure, b.exposure, t),
        aerial: Aerial { near: mix(a.aerial.near, b.aerial.near, t), far: mix(a.aerial.far, b.aerial.far, t), haze_density: mix(a.aerial.haze_density, b.aerial.haze_density, t), low_sun: mix(a.aerial.low_sun, b.aerial.low_sun, t) },
        shadow_strength: mix(a.shadow_strength, b.shadow_strength, t),
        disc: l3(a.disc, b.disc, t),
        disc_radius: mix(a.disc_radius, b.disc_radius, t),
        sky_irradiance_up: mix(a.sky_irradiance_up, b.sky_irradiance_up, t),
    }
}

/// Every number of a frame that the renderer's look uniform reads from it,
/// in a fixed order (for checks that the blend has no step).
pub fn frame_numbers(f: &LookFrame) -> Vec<f64> {
    let m = &f.model;
    let mut v = vec![];
    v.extend(f.key_dir);
    v.extend(f.key);
    v.extend(f.rim_dir);
    v.extend(f.rim);
    v.extend(f.hemi_sky);
    v.extend(f.hemi_ground);
    v.extend(f.ambient);
    for c in &f.env_sh {
        v.extend(c);
    }
    v.extend([f.exposure, f.aerial.near, f.aerial.far, f.shadow_strength, f.sky_irradiance_up, f.disc_radius]);
    v.extend(f.disc);
    v.extend(f.rig.pack());
    v.extend(m.sun.dir());
    v.extend([m.field.state.sun_e, m.field.state.horizon_up, m.field.state.g, m.field.twilight, m.field.night, m.field.ozone, m.field.belt, m.field.shadow_top_sin, m.field.deep_twilight, m.field.airglow, m.field.grade]);
    v.extend(m.field.state.beta_r);
    v.extend(m.field.state.beta_m);
    v.extend(m.field.mean);
    v.extend(m.horizon_radiance);
    v.extend(m.zenith_radiance);
    v.extend(m.ground_radiance);
    v.extend([m.profile.urban_glow, m.profile.overcast, m.profile.isotropy, m.profile.desaturation, m.profile.turbidity, m.ext.afterglow]);
    v.extend(m.profile.tint);
    let g = m.grade.map(|g| [g.weight, g.veil_luminance, g.zenith_desaturation * g.weight, g.horizon_desaturation * g.weight, g.horizon_lift * g.weight, g.dome_gain, g.skyline_share, g.sunward_horizon_desaturation * g.weight]).unwrap_or([0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    v.extend(g);
    v
}
