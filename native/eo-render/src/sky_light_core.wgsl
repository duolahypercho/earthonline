// the part of the sky slice every surface shader on the look's group 1
// takes (the city's, through sky_light.wgsl, and the water's, water.rs): the
// group's sky bindings and the cloud decks' shadow on the key.

@group(1) @binding(6) var<uniform> skyu: Sky;
// the cloud-shadow map (clouds.rs; the coverage sheet was bound here before)
@group(1) @binding(7) var sky_cshadow: texture_2d<f32>;
@group(1) @binding(8) var<storage, read> sky_lights: array<SkyLight>;
@group(1) @binding(9) var<storage, read> sky_clusters: array<u32>;
@group(1) @binding(10) var sky_prev: texture_2d<f32>;
@group(1) @binding(11) var sky_rep: sampler;

struct SkyLight {
    pos: vec3<f32>,
    // irradiance (units) at 1 m along the axis
    intensity: f32,
    color: vec3<f32>,
    range: f32,
    dir: vec3<f32>,
    cos_outer: f32,
    cos_inner: f32,
    kind: u32,
    size: f32,
    pad: f32,
};

const SKY_CLUSTER_CAP: u32 = 32u;

/// Transmittance of the cloud decks toward the key at a world point: (clouds)
/// the cloud-shadow map (62.5 m texels, filtered), from 1 (open) to 0.1 under a
/// cumulus core; under a continuous deck the weather's mean beam
/// (environment-ibl.js `cloudBeamTransmission`).
fn sky_cloud_transmittance(world: vec3<f32>) -> f32 {
    if (!sky_on(skyu.flags.x, SKY_CLOUD_SHADOWS) || look.key_dir.y <= 0.02) {
        return mix(1.0, skyu.beam.x, clamp((skyu.cloud0.z - 0.75) / 0.2, 0.0, 1.0));
    }
    // the cloud-shadow map (the low deck's transmittance along the key through the
    // shape the volume draws, clouds.rs), where the sheet at the deck's mid height was read.
    let tc = sky_cshadow_at(sky_cshadow, sky_rep, world, look.key_dir.xyz, look.cam_pos.xyz, SKY_SEA_Y);
    // under a continuous deck (cover over 0.75: overcast, fog, rain,
    // storm) the sun every pass takes is the weather's mean beam, with no
    // sunlit gaps (the sheet's last few percent let the full sun onto a pier in
    // the rain and glints onto a storm sea).
    let stratus = clamp((skyu.cloud0.z - 0.75) / 0.2, 0.0, 1.0);
    return mix(tc, skyu.beam.x, stratus);
}

/// The range fog's view depth at a world point `dh` metres above or below the
/// eye, with the aerial path density (sky_common `sky_path_density`); off
/// (the renderer before the slice) it is the view depth.
fn sky_fog_depth(vd: f32, dh: f32) -> f32 {
    if (skyu.flags.x == 0u) {
        return vd;
    }
    return vd * sky_path_density(dh);
}
