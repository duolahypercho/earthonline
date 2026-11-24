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
    species: u32,
};

struct FtParams {
    // x near (none closer), y full (faded in), z on
    range: vec4<f32>,
};

@group(2) @binding(0) var<storage, read> ft_trees: array<FarTree>;
@group(2) @binding(1) var<uniform> ft: FtParams;
@group(2) @binding(2) var ft_canopy: texture_2d_array<f32>;

struct FtOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) tint: vec3<f32>,
    @location(3) @interpolate(flat) fade: f32,
    @location(4) @interpolate(flat) seed: f32,
    @location(5) @interpolate(flat) right: vec3<f32>,
    @location(6) @interpolate(flat) view: vec3<f32>,
};

@vertex
fn far_tree_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> FtOut {
    var out: FtOut;
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    if (ft.range.z < 0.5) {
        return out;
    }
    let t = ft_trees[ii];
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
    out.view = vec3<f32>(-1.0);
    if (t.species > 16u) {
        // A stable rotation per stem. The projected view blends continuously
        // through north when a swimmer moves sideways along the shore.
        let turns = atan2(to_cam.y, to_cam.x) / 6.2831853 + f32(t.seed & 65535u) / 65536.0;
        let azimuth = fract(turns) * f32(FT_VIEWS);
        let base = (t.species - 17u) * FT_VIEWS;
        let view = u32(azimuth);
        out.view = vec3<f32>(f32(base + view), f32(base + ((view + 1u) % FT_VIEWS)), fract(azimuth));
    }
    return out;
}

fn ft_shade(in: FtOut) -> vec4<f32> {
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let atlas_uv = vec2<f32>(in.uv.x * 0.5 + 0.5, 1.0 - in.uv.y);
    let gx = dpdx(atlas_uv);
    let gy = dpdy(atlas_uv);
    // The crown: an ellipse over the upper 70 % of the card, a lobed edge.
    let cy = (in.uv.y - 0.62) / 0.38;
    let r = length(vec2<f32>(in.uv.x, cy));
    let ang = atan2(cy, in.uv.x);
    let lobes = 0.08 * sin(ang * 5.0 + in.seed) + 0.05 * sin(ang * 11.0 + in.seed * 1.7);
    let crown = r < 0.92 + lobes;
    // The trunk: the lower part, a narrow column.
    let trunk = in.uv.y < 0.3 && abs(in.uv.x) < 0.07;
    var leaf = vec4<f32>(0.0);
    let baked = in.view.x >= 0.0;
    let a = max(i32(in.view.x), 0);
    let b = max(i32(in.view.y), 0);
    let blend = smoothstep(0.0, 1.0, in.view.z);
    var covered = crown || trunk;
    if (baked) {
        leaf = mix(textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, a, gx, gy),
                   textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, b, gx, gy), blend);
        covered = leaf.a >= 0.34;
    }
    if (!covered) { discard; }
    // The screen-door fade-in is decided here but discarded only after the
    // lighting: discarded before it, every card in its fade band drew black on
    // Metal (the dither splits nearly every 2x2 quad, which likely leaves the
    // lighting's derivatives undefined for the pixels it keeps).
    let faded_out = in.fade < 0.999 && hlod_dither(floor(in.clip.xy)) >= in.fade;
    let v = normalize(frame.cam_pos.xyz - in.world);
    var n = vec3<f32>(0.0, 1.0, 0.0);
    var albedo = in.tint;
    if (baked) {
        let surface = mix(textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, a + FT_NORMALS, gx, gy),
                          textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, b + FT_NORMALS, gx, gy), blend);
        let local = surface.xyz / max(surface.a, 0.01) * 2.0 - 1.0;
        let toward = vec3<f32>(in.right.z, 0.0, -in.right.x);
        n = normalize(in.right * local.x + vec3<f32>(0.0, local.y, 0.0) + toward * local.z + vec3<f32>(0.0, 1e-4, 0.0));
        albedo = srgb_decode(leaf.rgb / max(leaf.a, 0.01));
    } else if (crown) {
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
    if (faded_out) {
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
