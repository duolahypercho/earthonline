// Submerged radiance is composed from the opaque scene before the sea, so
// air fog, rain sprites and the above-water mirror cannot obscure the view.
// Use the ordinary display transform after transport through the water.
struct UwView {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) level: f32,
    @location(1) @interpolate(flat) tilt: vec2<f32>,
    @location(2) @interpolate(flat) atmo_gain: f32,
};

// Invert the horizontal choppiness for the point whose displaced position
// lies over the eye. Include the beach's breaking face and swash.
fn uw_height(xz: vec2<f32>, spacing: f32) -> f32 {
    let disp = w_displacement(xz, spacing, w_field_at(xz));
    let base = xz - disp.xz;
    return wu.grid.z + w_displacement(base, spacing, w_field_at(base)).y;
}

@vertex
fn uw_vs(@builtin(vertex_index) vi: u32) -> UwView {
    let xz = wu.cam.xz;
    let spacing = wg_spacing(0.0, wu.fwd.w);
    w_expo_set(xz); // (iteration 2, exposure) the waves over the eye, as the surface's vertex
    let y = uw_height(xz, spacing);
    let slope = (vec2<f32>(uw_height(xz + vec2<f32>(spacing, 0.0), spacing),
                           uw_height(xz + vec2<f32>(0.0, spacing), spacing)) - y) / spacing;
    var out: UwView;
    out.clip = vec4<f32>(f32(i32(vi & 1u) * 4 - 1), f32(i32(vi >> 1u) * 4 - 1), 0.0, 1.0);
    out.level = y;
    out.tilt = slope;
    out.atmo_gain = w_atmo_gain();
    return out;
}

// Integral along the sightline of attenuated downwelling light, with
// depth varying from d0 to d1. Use the limit for parallel optical paths.
fn uw_integral(ext: vec3<f32>, path: f32, d0: f32, d1: f32, mu: f32) -> vec3<f32> {
    let a = exp(-ext * d0 / mu);
    let b = exp(-ext * (path + d1 / mu));
    let k = 1.0 + (d1 - d0) / max(mu * path, 1e-5);
    if (abs(k) < 0.005) {
        return (a + b) * (path * 0.5);
    }
    return (a - b) / (ext * k);
}

// Use the actual frame along the bent ray where it projects on screen.
// Off screen the atmosphere supplies the sky and the scene probe the skyline.
fn uw_air_light(p: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    var radiance = w_drawn_sky(dir, w_atmo_dome(dir));
    if (wu.flags.y > 0.5 && dir.y < 0.25) {
        radiance = mix(w_probe_sample(dir, 0.0), radiance, smoothstep(0.0, 0.25, dir.y));
    }
    let s = w_to_screen(p + dir * 1000.0);
    if (s.z > 0.0 && all(s.xy > vec2<f32>(0.0)) && all(s.xy < vec2<f32>(1.0))) {
        let view_depth = w_scene_depth(s.xy);
        let behind_surface = view_depth < 1e8 && view_depth > w_view_depth(p);
        if (behind_surface) {
            radiance = mix(radiance, textureSampleLevel(w_scene, w_clamp, s.xy, 0.0).rgb, w_edge(s.xy));
        }
    }
    return radiance;
}

// Suspended grains advect with the water. Traverse only the nearby cells
// that the ray actually enters. Turbidity hides distant grains naturally.
fn uw_grains(origin: vec3<f32>, ray: vec3<f32>, path: f32, ext: f32) -> f32 {
    let cell_m = 0.16;
    let q = (origin - vec3<f32>(0.018, -0.006, 0.009) * wu.cam.w) / cell_m;
    var cell = floor(q);
    let step = sign(ray);
    let stride = 1.0 / max(abs(ray), vec3<f32>(1e-6));
    var next = (step * (cell - q) + step * 0.5 + 0.5) * stride;
    var light = 0.0;
    for (var k = 0; k < 10; k++) {
        let seed = w_hash(cell.xz, cell.y);
        let offsets = vec3<f32>(seed, fract(seed * 83.31), fract(seed * 147.73));
        let v = (cell + 0.07 + 0.86 * offsets - q) * cell_m;
        let t = dot(v, ray);
        if (t > 0.08 && t < path) {
            let r = (0.00045 + 0.00065 * seed) + 0.0006 * t;
            let dist = length(v - ray * t);
            light += (1.0 - smoothstep(r * 0.2, r, dist)) * exp(-ext * t) * smoothstep(0.08, 0.18, t)
                * (1.0 - smoothstep(0.65, 0.9, t));
        }
        let t_next = min(next.x, min(next.y, next.z));
        if (t_next * cell_m >= min(path, 0.9)) { break; }
        let axis = vec3<f32>(next <= vec3<f32>(t_next));
        cell += axis * step;
        next += axis * stride;
    }
    return light;
}

@fragment
fn uw_fs(i: UwView) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(w_depth));
    let uv = i.clip.xy / dims;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let near_h = wu.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let to_near = near_h.xyz / near_h.w - wu.cam.xyz;
    let ray = normalize(to_near);
    let d0 = i.level - wu.cam.y;
    let rise = ray.y - dot(i.tilt, ray.xz);
    let d_near = d0 - rise * length(to_near);
    // Rasterization clips the sea inside the near plane. Continue the
    // wave across that plane so a crossing can show both media in a frame.
    if (d0 <= 0.0 && d_near <= 0.0) { discard; }
    let start = select(0.0, max(d0 / min(rise, -1e-5), 0.0), d0 < 0.0);
    let origin = wu.cam.xyz + ray * start;
    w_expo_set(origin.xz); // (iteration 2, exposure) the place's swell share and waves, as the surface's
    let ocean = wg_ocean(origin.x);
    let scatter = mix(WO_BAY_S, WO_COAST_S, ocean);
    let ext = mix(WO_BAY_A, WO_COAST_A, ocean) + scatter;
    let limit = 10.0 / min(ext.r, min(ext.g, ext.b));
    let hit = w_scene_depth(uv) / max(dot(ray, wu.fwd.xyz), 1e-4);
    var path = min(max(hit - start, 0.0), limit);
    var surface = false;
    if (d0 > 0.0 && rise > 0.001) {
        let t = d0 / rise;
        if (t < path) {
            path = t;
            surface = true;
        }
    }
    let end = origin + ray * path;
    let down0 = max(d0, 0.0);
    let down1 = select(max(down0 - ray.y * path, 0.0), 0.0, surface);
    let sun = normalize(look.key_dir.xyz);
    let sun_up = wo_refracted_sun_up(sun);
    let entry = origin + sun_up * (down0 / max(sun_up.y, 0.1));
    let direct = look.key.xyz * sky_cloud_transmittance(entry) * w_shadow(entry, vec3<f32>(0.0, 1.0, 0.0))
        * (1.0 - wo_fresnel(sun.y, WO_IOR)) * smoothstep(0.0, 0.08, sun.y);
    // The entry hemisphere illuminates the suspended volume even in
    // bridge shadow. The diffuse irradiance includes sky and ambient fill.
    let ambient = w_sh(vec3<f32>(0.0, 1.0, 0.0)) * look.ambient.w * look.env_class.w + look.hemi_sky.xyz + look.ambient.xyz;
    let back = scatter * mix(WO_BAY_BB, WO_COAST_BB, ocean);
    let multiple = 1.32 * back / (ext - scatter + back);
    // The same converging beam crosses the suspended material before it
    // lights the bed. Sample it within one optical mean free path of the eye.
    let scatter_t = min(0.5 * path, 1.0 / lk_luma(ext));
    let scatter_depth = mix(down0, down1, scatter_t / max(path, 1e-5));
    let beam_focus = w_caustic(origin + ray * scatter_t, scatter_depth, sun_up);
    let vol = ambient * (scatter * 0.25 + ext * multiple) * uw_integral(ext, path, down0, down1, 0.75)
        + direct * (scatter * wo_phase(dot(ray, sun_up)) * beam_focus + ext * multiple / WG_PI)
          * uw_integral(ext, path, down0, down1, max(sun_up.y, 0.1));
    var boundary = vec3<f32>(0.0);
    if (surface) {
        var slope = i.tilt;
        let amp = wg_cascade_amp(end.x);
        var foam = 0.0;
        for (var c = 1u; c < 4u; c++) {
            let d = textureSampleLevel(w_deriv, w_repeat, w_casc_uv(end.xz, c), c, 0.0);
            slope += d.xy * amp[c];
            foam = max(foam, mix(d.w, d.z, ocean));
        }
        // The same moving capillary chop bends both views of the interface.
        let rough = wv_ripple(end.x);
        slope += w_chop(end.xz, wu.cam.w, path / dims.y, 1.0, rough) + w_capillary(end.xz) * rough;
        let n = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
        // Above the critical angle the underside reflects only the water.
        let f = wo_fresnel(max(dot(ray, n), 0.0), 1.0 / WO_IOR);
        let mirror = reflect(ray, n);
        boundary = ambient * (0.25 * scatter / ext + multiple)
            + direct * (scatter / ext * wo_phase(dot(mirror, sun_up)) + multiple / WG_PI);
        // A grazing facet can mirror a nearby submerged piling or seabed.
        // Use the opaque depth to reject samples in air and in front of the facet.
        if (f > 0.02 && mirror.y < -0.03) {
            for (var j = 0u; j < 5u; j++) {
                let distance = exp2(f32(j) - 2.0);
                let q = end + mirror * distance;
                let projected = w_to_screen(q);
                if (projected.z <= 0.0 || any(projected.xy <= vec2<f32>(0.0)) || any(projected.xy >= vec2<f32>(1.0))) { break; }
                let scene_z = w_scene_depth(projected.xy);
                let error = projected.z - scene_z;
                if (error >= 0.0 && error < distance * 0.45 && scene_z > w_view_depth(end)) {
                    let seen = textureSampleLevel(w_scene, w_clamp, projected.xy, 0.0).rgb;
                    boundary = mix(boundary, seen, exp(-ext * distance) * w_edge(projected.xy));
                    break;
                }
            }
        }
        if (f < 1.0) {
            w_atmo_g = i.atmo_gain;
            let air = refract(ray, -n, WO_IOR);
            // Radiance / n² is invariant across a lossless interface.
            boundary = mix(uw_air_light(end, air) * (WO_IOR * WO_IOR), boundary, f);
        }
        // Whitecap bubbles diffuse incident light on the underside.
        boundary = mix(boundary, (ambient + direct * max(sun.y, 0.0)) / WG_PI, smoothstep(0.08, 0.65, foam));
    } else if (hit - start < limit) {
        // Only the incident sunlight is focused. Keep ambient in open shade.
        let direct_share = lk_luma(direct) / max(lk_luma(direct + ambient), 1e-5);
        let focus = w_caustic(end, down1, sun_up);
        let incident = mix(exp(-ext * down1 / 0.75),
            exp(-ext * down1 / max(sun_up.y, 0.1)) * focus, direct_share);
        boundary = textureLoad(w_scene, vec2<i32>(i.clip.xy), 0).rgb * incident;
    }
    let grains = uw_grains(origin, ray, path, lk_luma(ext));
    let radiance = boundary * exp(-ext * path) + vol
        + grains * scatter * (ambient + direct * 0.25) * exp(-ext * down0 / 0.75);
    return finish_color(max(radiance, vec3<f32>(0.0)), i.clip, dims);
}
