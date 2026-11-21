// stage 3, the far city's street lamps by night (hlod_lights.rs),
// appended to draw.wgsl and the look's WGSL. One point sprite per lamp of the
// citywide set (the sky slice's lamp placement), past the range where the
// frame's own lights draw their luminaires: a few pixels, the lamp's
// intensity spread over them so the energy reaching the eye is the lamp's
// (I / d^2 over the pixels it covers), times the fog's transmittance (three's
// range fog, the aerial path density as look_light has it), additive in the
// HDR target, tested against the city's depth.

struct FarLamp {
    pos: vec3<f32>,
    candela: f32,
    color: vec3<f32>,
    pad: f32,
};

struct FlParams {
    // x near (no sprite closer), y full (fade-in done), z sprite radius px, w on
    range: vec4<f32>,
    // x pixel metres per metre of depth (2 tan(fov/2) / height), y lamps
    px: vec4<f32>,
};

@group(2) @binding(0) var<storage, read> fl_lamps: array<FarLamp>;
@group(2) @binding(1) var<uniform> fl: FlParams;

struct FlOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) rad: vec3<f32>,
};

fn fl_corner(vi: u32) -> vec2<f32> {
    var k = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    return k[vi % 6u];
}

@vertex
fn far_lamp_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FlOut {
    var out: FlOut;
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    let lamps_on = skyu.night.x;
    if (fl.range.w < 0.5 || lamps_on <= 0.0 || !sky_on(skyu.flags.x, SKY_SPRITES)) {
        return out;
    }
    let l = fl_lamps[ii];
    let rel = l.pos - look.cam_pos.xyz;
    let d = length(rel);
    let z = dot(rel, look.cam_fwd.xyz);
    if (d < fl.range.x || z < 1.0) {
        return out;
    }
    let fade = smoothstep(fl.range.x, fl.range.y, d);
    // The metres one pixel spans at this depth; the sprite's radius in metres.
    let px_m = z * fl.px.x;
    let r_m = fl.range.z * px_m;
    let right = normalize(cross(look.cam_fwd.xyz, vec3<f32>(0.0, 1.0, 0.0)) + vec3<f32>(1e-6, 0.0, 0.0));
    let up = normalize(cross(right, look.cam_fwd.xyz));
    let c = fl_corner(vi);
    let p = l.pos + (right * c.x + up * c.y) * r_m;
    out.clip = frame.view_proj * vec4<f32>(p, 1.0);
    out.uv = c;
    // Radiance of the sprite: the lamp's intensity in render units over the
    // area its pixels cover (the Gaussian below, exp(-3 r^2), integrates to
    // pi (1 - e^-3) / 3 = 0.317 pi over the unit disc), with the fog's transmittance.
    // candela -> the rig's units (sky.rs: the lights' scale is lamp_k x lamps_on; lamp_k rides bloom_w2.z).
    let intensity = l.candela * skyu.bloom_w2.z * lamps_on;
    var vd = z;
    if (skyu.flags.x != 0u) {
        let u = max(abs(l.pos.y - look.cam_pos.y) - 20.0, 0.0) / 400.0;
        if (u > 1e-2) {
            vd = vd * (1.0 - exp(-u)) / u;
        }
    }
    let trans = 1.0 - lk_smooth(look.fog.x, look.fog.y, vd);
    let area = 3.14159265 * r_m * r_m * 0.317;
    out.rad = ls_led(l.color) * (intensity / max(area, 1e-4)) * trans * fade; // (lighting, `--light-shadows`: SF's LED lamps; as given otherwise)
    return out;
}

@fragment
fn far_lamp_fs(in: FlOut) -> @location(0) vec4<f32> {
    let r2 = dot(in.uv, in.uv);
    if (r2 > 1.0) {
        discard;
    }
    let g = exp(-3.0 * r2);
    return vec4<f32>(in.rad * g, 0.0);
}
