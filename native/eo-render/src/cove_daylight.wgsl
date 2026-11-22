// Kd is measured per vertical metre of downwelling PAR. The relative RGB
// shape is estimated from the cove model. Camera contrast keeps its own full
// beam extinction. At zero depth an unfocused bed receives unit irradiance.
fn wo_daylight_ratio(ext: vec3<f32>, kd: f32) -> f32 {
    return kd / max(dot(ext, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-5);
}

fn wo_daylight_bed(depth: f32, ext: vec3<f32>, sun_mu: f32, sun_share: f32, focus: f32, kd: f32) -> vec3<f32> {
    let total = exp(-ext * (wo_daylight_ratio(ext, kd) * depth));
    // Only the part of the downward flux still travelling in a direct beam
    // can be concentrated by waves. The dispersed remainder stays diffuse.
    let beam = min(total, exp(-ext * (depth / max(sun_mu, 0.1))));
    return total + beam * (clamp(sun_share, 0.0, 1.0) * (focus - 1.0));
}
