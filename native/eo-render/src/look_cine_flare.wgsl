// (`--lens-flare`, camera track) the lens's diffraction spikes from the frame's
// point sources. Composed into the camera model's post pass only with the
// switch (cine_post.rs `shader_source`; a stub `cam_spikes` stands in without
// it), after the meter's list (`cam_stars`, cine_camera.wgsl `cam_find_source`)
// and cine_post.rs's `CAM_FLARE_*` constants.

/// The spikes at `pos` (exposed scene units, added before the curve): for each
/// of the meter's sources (its centre, the spikes' intensity at the core, their
/// drawn length, its colour at unit luminance), six spikes along three axes
/// (a hexagonal aperture's three pairs of edges, turned `CAM_FLARE_ANGLE`), a
/// little over a pixel wide and widening slowly outward (their energy along a
/// unit of length kept as they widen), falling as (r0 / (r0 + r))^p; each colour
/// at its own wavelength's scale (`CAM_FLARE_DISPERSION`: the long red reaches
/// furthest, a spike's tip warms), tapered over the last 30 % of the drawn
/// length. A source further than its length is skipped at the cost of one test.
fn cam_spikes(pos: vec2<f32>, dims: vec2<f32>) -> vec3<f32> {
    let n = min(cam_stars.count, CAM_FLARE_SOURCES);
    if (n == 0u) {
        return vec3<f32>(0.0);
    }
    let px = dims.y / 1080.0;
    let r0 = CAM_FLARE_SPIKE.z;
    var acc = vec3<f32>(0.0);
    for (var i = 0u; i < n; i = i + 1u) {
        let a = cam_stars.s[2u * i];
        let d = pos - a.xy;
        if (max(abs(d.x), abs(d.y)) > a.w + 2.0 * px) {
            continue;
        }
        var sp = vec3<f32>(0.0);
        for (var k = 0; k < 3; k = k + 1) {
            let ang = CAM_FLARE_ANGLE + f32(k) * 1.0471976;
            let dir = vec2<f32>(cos(ang), sin(ang));
            let along = abs(dot(d, dir)) / px;
            let across = abs(dot(d, vec2<f32>(-dir.y, dir.x))) / px;
            let w = 0.75 + 0.004 * along;
            let prof = exp(-(across * across) / (w * w)) / w;
            let r = vec3<f32>(along) / CAM_FLARE_DISPERSION;
            let f = pow(vec3<f32>(r0) / (vec3<f32>(r0) + r), vec3<f32>(CAM_FLARE_SPIKE.y)) / CAM_FLARE_DISPERSION;
            let taper = 1.0 - smoothstep(0.7, 1.0, along * px / max(a.w, 1e-3));
            sp = sp + f * (prof * taper);
        }
        acc = acc + cam_stars.s[2u * i + 1u].rgb * sp * a.z;
    }
    return select(acc, vec3<f32>(0.0), any(acc != acc));
}
