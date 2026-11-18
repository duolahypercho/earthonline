// scene-scale ground-contact occlusion and one diffuse bounce, the
// JS game's post-chain GTAO (`POST_CHAIN_DEFAULTS.ao`: half resolution, 4 m
// radius, 24 samples, at most 48 half-resolution texels of reach, intensity
// 0.9, multi-bounce albedo 0.7, consumer 'material') and its screen-space
// bounce (src/render/lighting/screen-space-bounce.js: the horizon search's
// hidden arcs charged with the radiance of the surface that hid them, as
// fractions of the cosine-weighted hemisphere). Appended to look_common.wgsl.
//
// Runs after the city pass on its depth and its lit HDR colour; the city pass
// of the NEXT frame reads the result through `ao_fetch` (look_light.wgsl),
// reprojected with the camera this pass ran with, and spends it on the
// indirect light only (diffuse x visibility, specular through the horizon
// term, + bounce x diffuse albedo), never on the sun. One frame of lag, as the
// JS (`occlusion-history.js`) has.

@group(0) @binding(0) var<uniform> look: Look;
@group(0) @binding(1) var ao_depth_in: texture_depth_2d;
@group(0) @binding(2) var ao_hdr_in: texture_2d<f32>;
@group(0) @binding(3) var ao_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var ao_zout: texture_storage_2d<r32float, write>;

const AO_SLICES: i32 = 2;
const AO_STEPS: i32 = 6;
const AO_RADIUS_M: f32 = 4.0;
const AO_MAX_REACH_PX: f32 = 96.0; // 48 half-resolution texels, in full-resolution pixels
const AO_INTENSITY: f32 = 0.9;
// Per-sample radiance ceiling (the JS's exposure-relative clamp): x exposure-1.
const AO_RADIANCE_CAP: f32 = 4.0;

fn ao_pos(px: vec2<i32>, dims: vec2<i32>) -> vec4<f32> {
    let p = clamp(px, vec2<i32>(0), dims - 1);
    let d = textureLoad(ao_depth_in, p, 0);
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(dims);
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let w = look.inv_view_proj * ndc;
    // Camera-relative world position; w = raw depth (0 = sky, reverse-Z).
    return vec4<f32>(w.xyz / w.w, d);
}

// Integral of the cosine-weighted slice from the view vector to horizon angle h.
fn ao_arc(h: f32, n: f32) -> f32 {
    return 0.25 * (-cos(2.0 * h - n) + cos(n) + 2.0 * h * sin(n));
}

@compute @workgroup_size(8, 8)
fn ao_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let half_dims = vec2<i32>(textureDimensions(ao_out));
    let hp = vec2<i32>(gid.xy);
    if (hp.x >= half_dims.x || hp.y >= half_dims.y) {
        return;
    }
    let dims = vec2<i32>(textureDimensions(ao_depth_in));
    let px = hp * 2;
    let c = ao_pos(px, dims);
    let fwd = look.cam_fwd.xyz;
    if (c.w <= 0.0) {
        textureStore(ao_out, hp, vec4<f32>(0.0, 0.0, 0.0, 1.0));
        textureStore(ao_zout, hp, vec4<f32>(-1.0));
        return;
    }
    let P = c.xyz;
    let vd = dot(P, fwd);
    textureStore(ao_zout, hp, vec4<f32>(vd));
    // Normal from the depth neighbours, the smaller step on each axis.
    let r = ao_pos(px + vec2<i32>(1, 0), dims).xyz - P;
    let l = P - ao_pos(px - vec2<i32>(1, 0), dims).xyz;
    let d = ao_pos(px + vec2<i32>(0, 1), dims).xyz - P;
    let u = P - ao_pos(px - vec2<i32>(0, 1), dims).xyz;
    let dx = select(l, r, dot(r, r) < dot(l, l));
    let dy = select(u, d, dot(d, d) < dot(u, u));
    var N = normalize(cross(dy, dx));
    let V = normalize(-P);
    if (dot(N, V) < 0.0) {
        N = -N;
    }
    let right = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, fwd);
    // Reach in pixels: 4 m at this depth, at most 48 half-res texels.
    let focal = 1.0 / look.ao_params.y; // tan(fov/2) inverse x half height in pixels (CPU)
    let reach = min(AO_RADIUS_M * focal / max(vd, 0.1), AO_MAX_REACH_PX);
    if (reach < 2.0) {
        textureStore(ao_out, hp, vec4<f32>(0.0, 0.0, 0.0, 1.0));
        return;
    }
    // Interleaved gradient noise, rotated per frame.
    let fi = look.post2.w;
    let noise = fract(52.9829189 * fract(dot(vec2<f32>(hp) + fi * 5.588238, vec2<f32>(0.06711056, 0.00583715))));
    let jitter = fract(noise * 7.13 + 0.37);
    var vis = 0.0;
    var bounce = vec3<f32>(0.0);
    var norm = 0.0;
    for (var s = 0; s < AO_SLICES; s = s + 1) {
        let phi = (f32(s) + noise) * (LK_PI / f32(AO_SLICES));
        let dir = vec2<f32>(cos(phi), sin(phi));
        // The slice direction in camera-relative world space (screen y is down).
        let D = normalize(right * dir.x - up * dir.y);
        let ortho = D - V * dot(D, V);
        let axis = normalize(cross(ortho, V));
        let pn = N - axis * dot(N, axis);
        let plen = length(pn);
        if (plen < 1e-4) {
            continue;
        }
        let cos_n = clamp(dot(pn / plen, V), -1.0, 1.0);
        let n = sign(dot(ortho, pn)) * acos(cos_n);
        var slice_vis = 0.0;
        var slice_b = vec3<f32>(0.0);
        for (var side = 0; side < 2; side = side + 1) {
            let sg = select(-1.0, 1.0, side == 1);
            // Horizon angle on this side, measured from V; starts fully open.
            var h = n + sg * 0.5 * LK_PI;
            for (var j = 0; j < AO_STEPS; j = j + 1) {
                let t = (f32(j) + jitter) / f32(AO_STEPS);
                let off = dir * sg * (t * t * reach + 1.0);
                let sp = px + vec2<i32>(round(off));
                let S = ao_pos(sp, dims);
                if (S.w <= 0.0) {
                    continue;
                }
                let delta = S.xyz - P;
                let dist = length(delta);
                if (dist < 1e-3) {
                    continue;
                }
                // Coplanar detail (paint lifted 12 mm, a kerb lip seen edge-on)
                // is not an occluder: under 3 cm above the tangent plane is open.
                if (dot(delta, N) < 0.03) {
                    continue;
                }
                let fall = clamp(1.0 - (dist * dist) / (AO_RADIUS_M * AO_RADIUS_M), 0.0, 1.0);
                if (fall <= 0.0) {
                    continue;
                }
                // The sample's angle from V within the slice.
                let cs = dot(delta / dist, V);
                let ang = sg * acos(clamp(cs, -1.0, 1.0));
                // Fade the far samples toward the open horizon.
                let open = n + sg * 0.5 * LK_PI;
                let cand = mix(open, ang, fall);
                let tighter = select(cand > h, cand < h, side == 1);
                if (tighter) {
                    let ch = clamp(cand, n - 0.5 * LK_PI, n + 0.5 * LK_PI);
                    let w = abs(ao_arc(h, n) - ao_arc(ch, n));
                    var rad = min(textureLoad(ao_hdr_in, clamp(sp, vec2<i32>(0), dims - 1), 0).rgb, vec3<f32>(AO_RADIANCE_CAP));
                    if (deck_in_player(S.xyz + look.cam_pos.xyz)) {
                        rad = vec3<f32>(0.0); // the player's body on a landmark deck (landmarks_deck.wgsl)
                    }
                    slice_b = slice_b + w * rad;
                    h = ch;
                }
            }
            // The visible arc on this side (GTAO's IntegrateArc at the horizon).
            slice_vis = slice_vis + ao_arc(h, n);
        }
        // Normalised by the slice's own unoccluded integral, so visibility and
        // the bounce weights are fractions of the cosine-weighted hemisphere.
        let open_total = ao_arc(n + 0.5 * LK_PI, n) + ao_arc(n - 0.5 * LK_PI, n);
        vis = vis + plen * slice_vis;
        bounce = bounce + plen * slice_b;
        norm = norm + plen * open_total;
    }
    if (norm > 1e-4) {
        vis = clamp(vis / norm, 0.0, 1.0);
        bounce = bounce / norm;
    } else {
        vis = 1.0;
    }
    vis = 1.0 - AO_INTENSITY * (1.0 - vis);
    textureStore(ao_out, hp, vec4<f32>(bounce, vis));
}
