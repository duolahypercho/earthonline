// (`--probe-sky`, the reflect track; probe_grid.rs) The probe grid's sky and
// levels, appended to probe_fill.wgsl (after look_common.wgsl) and to the
// volume's sky function (sky_volume.wgsl `vol_atmo_gain`, `vol_atmo_sky`,
// cut out of it by probe_grid.rs `atmo_source`, so the two never part).
// Compiled only with SF_PROBE_SKY; the single probe keeps probe_fill.wgsl.
//
//  ps_fill: level 0 of the cube from the six captured faces; where nothing was
//           drawn, the sky the frame draws behind everything (the volume pass:
//           sky_volume.wgsl), not the analytic dome alone: the atmosphere
//           tables' sky by their share (`atmo2.y`: day and twilight whole,
//           deep night none) over the drawn dome (`lk_sky_dome`, its night
//           blue as look_sky.wgsl `sky_color` casts it), plus the city's glow
//           (`sky_city_glow`, as the volume adds it to sky pixels). The cloud
//           deck is not marched here: its cover greys the sky toward its own
//           luminance (clouds scatter every colour alike; the facades track's
//           canyon glass greys its sky the same way, rooms.wgsl
//           `CG_CLOUD_GREY`). The dome alone was the probes' sky before: a
//           golden or blue-hour frame's glass reflected the dome's saturated
//           cyan beside a gold or violet sky.
//  ps_prefilter: level m (1..) of each face, the GGX lobe of the roughness the
//           lookups read it at (probe_grid.wgsl `pg_lod`: rough =
//           2^((m - log2 face) / 2) / 1.16, alpha its square), by filtered
//           importance sampling (Krivanek & Colbert 2008: each sample read at
//           the level whose texel matches its share of the lobe's solid angle,
//           from the levels already made) with n = v = r (Karis 2013's split
//           sum), weighted by n.l. The 2 x 2 box levels before it drew a
//           hard-edged, blocky blur with no GGX tail: a lamp or the low sun's
//           glow in glass was a soft square, not a highlight with a halo.

@group(0) @binding(4) var vol_at_view: texture_2d<f32>;
@group(0) @binding(5) var vol_lin: sampler;
@group(0) @binding(6) var<uniform> skyu: Sky;
// ps_prefilter: every level made so far (0 .. m - 1) of the slot's six faces.
@group(0) @binding(7) var ps_src: texture_2d_array<f32>;

/// The cloud cover's grey (rooms.wgsl `CG_CLOUD_GREY`'s model): the share of
/// the sky's colour a full deck takes to its luminance.
const PS_CLOUD_GREY: f32 = 0.6;
/// The night's blue cast on the dark dome (look_sky.wgsl `sky_color`).
const PS_NIGHT_CAST: vec3<f32> = vec3<f32>(0.78, 0.9, 1.35);
/// GGX samples a prefiltered texel.
const PS_SAMPLES: u32 = 32u;

/// The sky the frame draws along `d` where nothing stands (see above).
fn ps_sky(d: vec3<f32>) -> vec3<f32> {
    let up = vec3<f32>(d.x, max(d.y, 0.0), d.z);
    let du = normalize(up + vec3<f32>(0.0, 1e-4, 0.0));
    var dome = lk_sky_dome(du);
    dome = dome * mix(vec3<f32>(1.0), PS_NIGHT_CAST, smoothstep(0.85, 1.0, skyu.night.z) * clamp(1.0 - skyu.cloud0.z, 0.0, 1.0));
    var sky = dome;
    if (skyu.atmo2.x > 0.5 && skyu.atmo2.y > 0.0) {
        sky = mix(dome, vol_atmo_sky(du), skyu.atmo2.y);
    }
    sky = sky + sky_city_glow(du, look.cam_pos.xyz, skyu.amb_top.w);
    sky = mix(sky, vec3<f32>(lk_luma(sky)), clamp(skyu.cloud0.z, 0.0, 1.0) * PS_CLOUD_GREY);
    if (d.y >= 0.0) {
        return sky;
    }
    // below the horizon where nothing was drawn: the ground field, as the dome's lookup has it
    return mix(sky, lk_ground_field(d), lk_smooth(0.0, 0.0157074, -d.y));
}

@compute @workgroup_size(8, 8, 1)
fn ps_fill(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(src).x;
    if (gid.x >= size || gid.y >= size) {
        return;
    }
    var c = textureLoad(src, vec2<u32>(gid.xy), gid.z, 0);
    if (c.a < 0.5) {
        c = vec4<f32>(ps_sky(texel_dir(gid.z, gid.xy, size)), 1.0);
    }
    textureStore(dst, vec2<u32>(gid.xy), gid.z, vec4<f32>(max(c.rgb, vec3<f32>(0.0)), 1.0));
}

/// The levels made so far along `d` at level `lod` (the faces' basis as
/// probe_grid.wgsl `pg_sample`; held half a texel of the level inside the face).
fn ps_read(d: vec3<f32>, lod: f32) -> vec3<f32> {
    let a = abs(d);
    var f = 0u;
    var fw = vec3<f32>(1.0, 0.0, 0.0);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (a.x >= a.y && a.x >= a.z) {
        f = select(1u, 0u, d.x > 0.0);
        fw = vec3<f32>(sign(d.x), 0.0, 0.0);
    } else if (a.y >= a.z) {
        f = select(3u, 2u, d.y > 0.0);
        fw = vec3<f32>(0.0, sign(d.y), 0.0);
        up = vec3<f32>(0.0, 0.0, -sign(d.y));
    } else {
        f = select(5u, 4u, d.z > 0.0);
        fw = vec3<f32>(0.0, 0.0, sign(d.z));
    }
    let right = cross(fw, up);
    let k = 1.0 / max(dot(d, fw), 1e-6);
    let size0 = f32(textureDimensions(ps_src).x);
    let h = 0.5 * exp2(ceil(lod)) / size0;
    let uv = clamp(vec2<f32>(dot(d, right) * k * 0.5 + 0.5, 0.5 - dot(d, up) * k * 0.5), vec2<f32>(h), vec2<f32>(1.0 - h));
    return textureSampleLevel(ps_src, vol_lin, uv, i32(f), lod).rgb;
}

/// Hammersley point `i` of `n`.
fn ps_hammersley(i: u32, n: u32) -> vec2<f32> {
    var b = i;
    b = (b << 16u) | (b >> 16u);
    b = ((b & 0x55555555u) << 1u) | ((b & 0xAAAAAAAAu) >> 1u);
    b = ((b & 0x33333333u) << 2u) | ((b & 0xCCCCCCCCu) >> 2u);
    b = ((b & 0x0F0F0F0Fu) << 4u) | ((b & 0xF0F0F0F0u) >> 4u);
    b = ((b & 0x00FF00FFu) << 8u) | ((b & 0xFF00FF00u) >> 8u);
    return vec2<f32>(f32(i) / f32(n), f32(b) * 2.3283064e-10);
}

@compute @workgroup_size(8, 8, 1)
fn ps_prefilter(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(dst).x;
    if (gid.x >= size || gid.y >= size) {
        return;
    }
    let size0 = f32(textureDimensions(ps_src).x);
    let made = f32(textureNumLevels(ps_src));
    // this level's index and its lobe (`pg_lod`'s roughness, alpha its square)
    let m = log2(size0 / f32(size));
    let rough = exp2(0.5 * (m - log2(size0))) / 1.16;
    let alpha = rough * rough;
    let a2 = alpha * alpha;
    let n = texel_dir(gid.z, gid.xy, size);
    // a basis about the texel's direction
    let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.z) > 0.999);
    let tx = normalize(cross(up, n));
    let ty = cross(n, tx);
    // a level-0 texel's solid angle (at the face's centre)
    let omega_p = 4.0 / (size0 * size0);
    var c = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0u; i < PS_SAMPLES; i = i + 1u) {
        let xi = ps_hammersley(i, PS_SAMPLES);
        // GGX half-vector about n
        let ct = sqrt((1.0 - xi.y) / (1.0 + (a2 - 1.0) * xi.y));
        let st = sqrt(max(1.0 - ct * ct, 0.0));
        let ph = 6.2831853 * xi.x;
        let hv = tx * (st * cos(ph)) + ty * (st * sin(ph)) + n * ct;
        let l = 2.0 * dot(n, hv) * hv - n;
        let nl = dot(n, l);
        if (nl <= 0.0) {
            continue;
        }
        // the sample's share of the lobe's solid angle against a texel's: its level
        let dd = ct * ct * (a2 - 1.0) + 1.0;
        let pdf = a2 / (3.14159265 * dd * dd) * 0.25; // D (n.h) / (4 v.h), n = v
        let omega_s = 1.0 / (f32(PS_SAMPLES) * max(pdf, 1e-6));
        let lod = clamp(0.5 * log2(omega_s / omega_p) + 1.0, 0.0, made - 1.0);
        c = c + ps_read(l, lod) * nl;
        wsum = wsum + nl;
    }
    textureStore(dst, vec2<u32>(gid.xy), gid.z, vec4<f32>(c / max(wsum, 1e-6), 1.0));
}
