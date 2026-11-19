// stage 4, distant trees (hlod_trees.rs), appended to draw.wgsl and the
// look's WGSL. One camera-facing card per tree (turned about the vertical),
// its canopy an ellipsoid silhouette with a lobed, world-keyed edge, its
// normal the ellipsoid's (so the sun and sky light the crown as a volume), lit
// by the city's `look_light`. Drawn from where the street props' trees end
// (faded in by the far layer's screen-door dither) to the far plane.

// Interleaved gradient noise: the screen-door dither of the far layer's cross-fades.
fn hlod_dither(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

struct FarTree {
    pos: vec3<f32>,
    height: f32,
    crown: f32,
    tint: u32,
    seed: u32,
    pad: u32,
};

struct FtParams {
    // x near (none closer), y full (faded in), z on
    range: vec4<f32>,
};

@group(2) @binding(0) var<storage, read> ft_trees: array<FarTree>;
@group(2) @binding(1) var<uniform> ft: FtParams;

struct FtOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) tint: vec3<f32>,
    @location(3) @interpolate(flat) fade: f32,
    @location(4) @interpolate(flat) seed: f32,
    @location(5) @interpolate(flat) right: vec3<f32>,
};

@vertex
fn far_tree_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FtOut {
    var out: FtOut;
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    if (ft.range.z < 0.5) {
        return out;
    }
    let t = ft_trees[ft_index(ii)];
    let rel = t.pos - frame.cam_pos.xyz;
    let d = length(rel.xz);
    if (d < ft.range.x || dot(rel, look.cam_fwd.xyz) < 0.0) {
        return out;
    }
    var k = array<vec2<f32>, 6>(vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = k[vi % 6u];
    let to_cam = normalize(vec2<f32>(-rel.x, -rel.z) + vec2<f32>(1e-5, 0.0));
    let right = vec3<f32>(-to_cam.y, 0.0, to_cam.x);
    let p = t.pos + right * (c.x * t.crown) + vec3<f32>(0.0, c.y * t.height, 0.0);
    out.clip = frame.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    out.uv = c;
    out.tint = srgb_decode(unpack4x8unorm(t.tint).rgb);
    out.fade = smoothstep(ft.range.x, ft.range.y, d);
    out.seed = f32(t.seed & 1023u);
    out.right = right;
    return out;
}

fn ft_shade(in: FtOut) -> vec4<f32> {
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    // The crown: an ellipse over the upper 70 % of the card, a lobed edge.
    let cy = (in.uv.y - 0.62) / 0.38;
    let r = length(vec2<f32>(in.uv.x, cy));
    let ang = atan2(cy, in.uv.x);
    let lobes = 0.08 * sin(ang * 5.0 + in.seed) + 0.05 * sin(ang * 11.0 + in.seed * 1.7);
    let crown = r < 0.92 + lobes;
    // The trunk: the lower part, a narrow column.
    let trunk = in.uv.y < 0.3 && abs(in.uv.x) < 0.07;
    // the card's outline and the fade's screen door
    let cut = !(crown || trunk) || (in.fade < 0.999 && hlod_dither(floor(in.clip.xy)) >= in.fade);
    if (!FT_LATE_DISCARD && cut) {
        discard;
    }
    let v = normalize(frame.cam_pos.xyz - in.world);
    var n = vec3<f32>(0.0, 1.0, 0.0);
    var albedo = in.tint;
    if (crown) {
        let z = sqrt(max(0.0, 1.0 - min(r * r, 1.0)));
        let toward = normalize(vec3<f32>(v.x, 0.0, v.z) + vec3<f32>(1e-5, 0.0, 0.0));
        n = normalize(in.right * in.uv.x + vec3<f32>(0.0, cy, 0.0) + toward * z);
        albedo = in.tint * (0.75 + 0.25 * st_hash(floor(in.world.xz * 0.7) + in.seed));
    } else {
        n = normalize(vec3<f32>(v.x, 0.0, v.z) + vec3<f32>(1e-5, 0.0, 0.0));
        albedo = vec3<f32>(0.09, 0.07, 0.05);
    }
    var s: Surface;
    s.albedo = albedo;
    s.rough = 0.92;
    s.metal = 0.0;
    s.ao = select(1.0, 0.7 + 0.3 * in.uv.y, crown);
    s.n = n;
    s.alpha = 1.0;
    s.env = -1;
    var m: Material;
    m.base = vec4<f32>(1.0);
    m.emissive = vec4<f32>(0.0);
    m.params = vec4<f32>(0.92, 0.0, 0.0, 0.0);
    m.layer = -1;
    m.flags = 0u;
    m.rough_flatten = 0.0;
    m.thin_absorption = 0.0;
    rm_emit = vec3<f32>(0.0);
    let face = normalize(cross(dpx, dpy));
    let lit = look_light(s, m, v, in.world, select(-face, face, dot(face, v) > 0.0), 0u);
    // FT_LATE_DISCARD (the fallback path, set by hlod_trees.rs): the discards
    // after the lighting. The light reads screen derivatives, which are
    // undefined after a discard on backends where discard ends the invocation
    // (Metal), and the whole dithered band then shaded black.
    if (FT_LATE_DISCARD && cut) {
        discard;
    }
    return lit;
}

@fragment
fn far_tree_fs(in: FtOut) -> @location(0) vec4<f32> {
    return ft_shade(in);
}

@fragment
fn far_tree_fs_ids(in: FtOut) -> IdsOut {
    var out: IdsOut;
    out.color = ft_shade(in);
    out.ids = vec2<u32>(0xfffffffeu, 0u);
    return out;
}
