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
// sf:CROWN|FAR_PALMS+
    // (`--crown-light`, `--far-palms`) the crown's half width and the tree's height (m), its species.
    @location(7) @interpolate(flat) size: vec3<f32>,
// sf:end
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
// sf:FAR_PALMS-
    if (d < ft.range.x || dot(rel, look.cam_fwd.xyz) < 0.0) {
        return out;
    }
// sf:end
// sf:FAR_PALMS+
    // (`--far-palms`) a palm's card takes over where the street props' palm fades out.
    var near = ft.range.xy;
    if (t.species == FT_PALM) {
        near = vec2<f32>(FT_PALM_NEAR_M, FT_PALM_FULL_M);
    }
    if (d < near.x || dot(rel, look.cam_fwd.xyz) < 0.0) {
        return out;
    }
// sf:end
    var k = array<vec2<f32>, 6>(vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = k[vi % 6u];
    let to_cam = normalize(vec2<f32>(-rel.x, -rel.z) + vec2<f32>(1e-5, 0.0));
    let right = vec3<f32>(-to_cam.y, 0.0, to_cam.x);
    let p = t.pos + right * (c.x * t.crown) + vec3<f32>(0.0, c.y * t.height, 0.0);
    out.clip = frame.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    out.uv = c;
    out.tint = srgb_decode(unpack4x8unorm(t.tint).rgb);
// sf:FAR_PALMS-
    out.fade = smoothstep(ft.range.x, ft.range.y, d);
// sf:end
// sf:FAR_PALMS+
    out.fade = smoothstep(near.x, near.y, d);
// sf:end
// sf:CROWN|FAR_PALMS+
    out.size = vec3<f32>(t.crown, t.height, f32(t.species));
// sf:end
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
// sf:LOD_FADE-
    let faded_out = in.fade < 0.999 && hlod_dither(floor(in.clip.xy)) >= in.fade;
// sf:end
// sf:LOD_FADE+
    // (`--temporal-lod-fade`) the pattern moves every frame, so the temporal
    // anti-aliasing resolves the fade-in to a blend.
    let faded_out = in.fade < 0.999 && hlod_dither(floor(in.clip.xy) + vec2<f32>(5.588238 * look.post2.w)) >= in.fade;
// sf:end
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
// sf:FOLIAGE-
    let lit = look_light(s, m, v, in.world, select(-face, face, dot(face, v) > 0.0), 0u);
// sf:end
// sf:FOLIAGE+
    var lit = look_light(s, m, v, in.world, select(-face, face, dot(face, v) > 0.0), 0u);
    if (crown || baked) {
        // (`--foliage-light`) the crown passes light: what reaches its far side
        // comes through the leaves toward the eye, most where the view's chord
        // through the crown is short (its edge: the backlit rim), little through its depth.
        lit = vec4<f32>(lit.rgb + ft_foliage(albedo, n, v, in.world), lit.a);
    }
// sf:end
    if (faded_out) {
        discard;
    }
    return lit;
}
// sf:FOLIAGE+

// (`--foliage-light`) light through a far crown toward the eye (radiance, fogged
// as look_light fogs): the leaves' transmittance (props_draw.wgsl
// `PR_LEAF_T_OVER_R`, the same mesophytic ratio) of the sun on the far side (a
// forward lobe, Henyey-Greenstein g 0.5) and of the sky and ground light there,
// times the crown's own transmittance along the view's chord, exp(-2 z) with z
// the crown normal's share toward the eye (1 at its centre, 0 at its edge).
fn ft_foliage(albedo: vec3<f32>, n: vec3<f32>, v: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let t = albedo * vec3<f32>(0.55, 0.85, 0.3);
    let b = -n;
    let chord = exp(-2.0 * max(dot(n, v), 0.0));
    let l = look.key_dir.xyz;
    var e = vec3<f32>(0.0);
    let bl = dot(b, l);
    if (bl > 0.0) {
        let g = 0.5;
        let cs = dot(-v, l);
        let hg = (1.0 - g * g) / (4.0 * LK_PI * pow(max(1.0 + g * g - 2.0 * g * cs, 1e-4), 1.5));
        e = look.key.xyz * bl * lk_shadow(world, b) * sky_cloud_transmittance(world) * (0.7 / LK_PI + 0.3 * hg) * LK_PI;
    }
    let irr = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, b.y * 0.5 + 0.5) + look.ambient.xyz + lk_sh(b) * look.ambient.w * look.env_class.x;
    var vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    if (skyu.flags.x != 0u) {
        let u = max(abs(world.y - look.cam_pos.y) - 20.0, 0.0) / 400.0;
        if (u > 1e-2) {
            vd = vd * (1.0 - exp(-u)) / u;
        }
    }
    let fog = lk_smooth(look.fog.x, look.fog.y, vd);
    return t * (e + irr) / LK_PI * chord * (1.0 - fog);
}
// sf:end

// sf:CROWN+

// ---- (`--crown-light`) a far crown as a volume of leaves ----
// Seen from afar a tree's crown is a cloud of leaves, not a shell: the light
// leaving it toward the eye is scattered by the leaves along the eye's ray
// inside it, each lit by the sun through the leaves between it and the sun
// and by the sky through the leaves above it. The crown is the card's
// spheroid (its half width across, 0.38 of the height up, centred at 0.62 of
// it), a turbid medium of leaves (Ross 1981): extinction sigma = G u, G 0.5
// (leaves facing every way), u the leaf area density. Single scattering of
// the sun along the eye's ray, s metres in:
//   L = E Gamma(beta) / (pi G) x integral of sigma e^(-sigma s) T_sun(s) ds,
// Gamma Ross's area scattering phase function of bi-Lambertian leaves
// (reflectance rho, transmittance tau, omega = rho + tau) facing every way,
// beta the angle between the sun's photons (-l) and the ray to the eye (v):
//   Gamma(beta) = omega / (3 pi) (sin beta - beta cos beta) + tau / pi cos beta
// (Shultis and Myneni 1988): bright with the sun behind the eye (the lit
// faces), dark side-lit, and with the sun behind the crown only what the
// leaves pass, a thin rim where the sun's path through the crown is short.
// T_sun = e^(-sigma d), d the path to the crown's surface toward the sun,
// less what it shares with the eye's path near the hotspot (Kuusk). The
// integral is a three-point quadrature (0.3, 1 and 2.2 free paths in). The
// sky's light is the same medium's under a uniform sky: a Lambertian albedo
// of omega / 2 (look_light) times the sky's visibility through the leaves
// above (the zenith and four directions 35 degrees up, uniform over the
// hemisphere as leaves facing every way see it), at the same depths.
// Multiple scattering adds about omega / (2 (1 - omega)) in the visible.
const FT_SIGMA: f32 = 0.6; // 1/m: G 0.5 x leaf area density 1.2 m2/m3 (street and park broadleaf crowns, 0.8-1.6)
const FT_LEAF_T_OVER_R: vec3<f32> = vec3<f32>(0.55, 0.85, 0.3); // props_draw.wgsl PR_LEAF_T_OVER_R
// a mature broadleaf leaf's reflectance over its luminance (linear, about 0.055 / 0.10 / 0.043)
const FT_LEAF_CHROMA: vec3<f32> = vec3<f32>(0.64, 1.17, 0.50);
// the hotspot's angular width (rad): a leaf's size over the crown's depth
const FT_HOTSPOT_RAD: f32 = 0.08;

// The path (m) from q (m from the crown's centre, inside it) along the unit
// w to the spheroid's surface (semi-axes 1 / inv).
fn ft_exit(q: vec3<f32>, w: vec3<f32>, inv: vec3<f32>) -> f32 {
    let qn = q * inv;
    let wn = w * inv;
    let a = max(dot(wn, wn), 1e-6);
    let b = dot(qn, wn);
    let c = dot(qn, qn) - 1.0;
    return max((-b + sqrt(max(b * b - a * c, 0.0))) / a, 0.0);
}

// A crown's leaf reflectance: the tree's tint pulled most of the way to a
// measured leaf's chroma, its luminance held in a leaf's range (0.045 to
// 0.095), each tree a little its own.
fn ft_leaf_r(tint: vec3<f32>, seed: f32) -> vec3<f32> {
    let lum = max(dot(tint, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4);
    let c = mix(tint / lum, FT_LEAF_CHROMA, 0.6);
    return c * clamp(lum, 0.045, 0.095) * (0.85 + 0.3 * fract(seed * 0.6180339));
}

// The crown's light at a card pixel: xyz the sun's scattered radiance per unit
// of the key's irradiance (before its shadow), w the sky's visibility.
fn ft_crown(uv: vec2<f32>, size: vec2<f32>, right: vec3<f32>, v: vec3<f32>, rho: vec3<f32>) -> vec4<f32> {
    let ax = max(vec3<f32>(size.x, 0.38 * size.y, size.x), vec3<f32>(0.3));
    let inv = vec3<f32>(1.0) / ax;
    // the pixel's point on the card, in the crown
    var pn = vec2<f32>(uv.x, (uv.y - 0.62) / 0.38);
    let lp = length(pn);
    if (lp > 0.96) {
        pn = pn * (0.96 / lp);
    }
    let p = right * (pn.x * ax.x) + vec3<f32>(0.0, pn.y * ax.y, 0.0);
    // where the eye's ray enters the crown, and its chord through it
    let t_in = ft_exit(p, v, inv);
    let e = p + v * t_in;
    let chord = t_in + ft_exit(p, -v, inv);
    let l = look.key_dir.xyz;
    let cb = clamp(dot(-l, v), -1.0, 1.0);
    let hot = exp(-acos(clamp(dot(l, v), -1.0, 1.0)) / FT_HOTSPOT_RAD);
    let depth = vec3<f32>(0.3, 1.0, 2.2);
    let wt = vec3<f32>(0.45, 0.35, 0.2);
    var sun = 0.0;
    var sky = 0.0;
    for (var k = 0; k < 3; k = k + 1) {
        let s = min(depth[k] / FT_SIGMA, chord * 0.98);
        let q = e - v * s;
        let d = max(ft_exit(q, l, inv) - hot * s, 0.0);
        sun = sun + wt[k] * exp(-FT_SIGMA * d);
        var vs = exp(-FT_SIGMA * ft_exit(q, vec3<f32>(0.0, 1.0, 0.0), inv));
        for (var j = 0; j < 4; j = j + 1) {
            let az = f32(j) * 1.5707963 + 0.4;
            vs = vs + exp(-FT_SIGMA * ft_exit(q, vec3<f32>(0.819 * cos(az), 0.574, 0.819 * sin(az)), inv));
        }
        sky = sky + wt[k] * vs * 0.2;
    }
    let tau = rho * FT_LEAF_T_OVER_R;
    let omega = rho + tau;
    let beta = acos(cb);
    let gamma = max(omega / (3.0 * LK_PI) * (sin(beta) - beta * cb) + tau / LK_PI * cb, vec3<f32>(0.0));
    let ms = vec3<f32>(1.0) + 0.5 * omega / (vec3<f32>(1.0) - omega);
    return vec4<f32>(gamma / (LK_PI * 0.5) * sun * ms, sky);
}
// sf:end
// sf:FAR_PALMS+

// ---- (`--far-palms`) a far palm ----
const FT_PALM: u32 = 7u; // hlod_trees.rs `FAR_PALM`: the survey's palms
// The street props draw their palms to 450 m (props_lib.rs `Model::Palm`),
// fading out over the last PR_FADE_BAND (0.12) of it: the card fades in there.
const FT_PALM_FULL_M: f32 = 450.0;
const FT_PALM_NEAR_M: f32 = 396.0;
const FT_PALM_FRONDS: u32 = 13u;

struct FtPalm {
    frond: bool,
    trunk: bool,
    n: vec3<f32>,
    albedo: vec3<f32>,
    ao: f32,
};

// A palm on its card (x across in crown reaches, y up the height), the
// survey's palms being mostly Canary Island date palms: a column to the
// crown's heart (0.55 of the reach under the top), a little swollen at the
// foot, and 13 fronds from the heart, each a feather that rises and bends
// under its own weight (s along its bearing, 0.55 s^2 down), its leaflets'
// spread widest past mid length; a frond turned toward or away from the eye
// is seen shorter, the oldest hang. The fronds take the palm leaflets' tones
// (props_draw.wgsl `pr_palm`: a mature leaflet about 0.062 / 0.098 / 0.046),
// some hanging ones dying straw; the trunk the weathered leaf bases' mean.
// Every width is at least half a pixel, so a palm 450 m off keeps its star.
fn ft_palm(uv: vec2<f32>, size: vec2<f32>, seed: f32, right: vec3<f32>, toward: vec3<f32>, px: f32) -> FtPalm {
    var o: FtPalm;
    let rc = max(size.x, 1.2);
    let h = max(size.y, 3.0);
    let x = uv.x * rc;
    let y = uv.y * h;
    let yh = h - 0.55 * rc;
    let rt = clamp(0.035 * h, 0.22, 0.42) * (1.0 + 0.3 * exp(-y / 0.6));
    o.trunk = y < yh + 0.1 * rc && abs(x) < max(rt, 0.5 * px);
    let q = vec2<f32>(x, y - yh) / rc;
    let w_min = 0.5 * px / rc;
    var best = 1e9;
    var bt = vec2<f32>(0.0, 1.0);
    var bh = 0.0;
    var hang = 0.0;
    for (var i = 0u; i < FT_PALM_FRONDS; i = i + 1u) {
        let fi = f32(i);
        let hs = fract(sin(fi * 12.9898 + seed * 0.731) * 43758.5453);
        let th = (fi + 0.6 * hs) / f32(FT_PALM_FRONDS) * 6.2831853 + seed;
        let dir = vec2<f32>(cos(th), 0.8 * sin(th) + 0.25);
        let len = 0.95 * (0.5 + 0.5 * abs(cos(th * 1.37 + seed * 2.1)));
        let s0 = clamp(dot(q, dir) / dot(dir, dir), 0.0, len);
        let p0 = s0 * dir - vec2<f32>(0.0, 0.55 * s0 * s0);
        let tg0 = dir - vec2<f32>(0.0, 1.1 * s0);
        let s1 = clamp(s0 + dot(q - p0, tg0) / max(dot(tg0, tg0), 1e-4), 0.0, len);
        let p1 = s1 * dir - vec2<f32>(0.0, 0.55 * s1 * s1);
        let f = s1 / len;
        let w = max(0.13 * sin(3.14159 * clamp(f * 0.9 + 0.06, 0.0, 1.0)) * (0.45 + 0.55 * f), w_min);
        let d = length(q - p1) / w;
        if (d < best) {
            best = d;
            bt = normalize(dir - vec2<f32>(0.0, 1.1 * s1));
            bh = hs;
            hang = smoothstep(0.0, -0.5, dir.y);
        }
    }
    o.frond = best < 1.0;
    let tone = 0.88 + 0.24 * fract(seed * 0.377);
    if (o.frond) {
        let up = vec3<f32>(0.0, 1.0, 0.0);
        o.n = normalize(right * (0.45 * bt.x) + up * 0.85 + toward * (0.25 + 0.3 * bh));
        let dead = hang * step(0.45, bh);
        o.albedo = mix(vec3<f32>(0.062, 0.098, 0.046), vec3<f32>(0.17, 0.13, 0.07), dead) * tone;
        o.ao = 0.5 + 0.5 * smoothstep(0.05, 0.6, length(q));
    } else {
        let xn = clamp(x / max(rt, 0.5 * px), -1.0, 1.0);
        o.n = normalize(right * xn + toward * sqrt(max(1.0 - xn * xn, 0.0)));
        o.albedo = vec3<f32>(0.15, 0.115, 0.082) * tone;
        o.ao = mix(0.92, 0.55, smoothstep(yh - 1.8, yh, y));
    }
    return o;
}
// sf:end
// sf:CROWN|FAR_PALMS+

// (`--crown-light`, `--far-palms`) The far card's shading with these bits in:
// `ft_shade`'s, with the crown lit as a volume of leaves (`--crown-light`,
// `ft_crown`) and the palms drawn as palms (`--far-palms`, `ft_palm`).
fn ft_shade2(in: FtOut) -> vec4<f32> {
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let atlas_uv = vec2<f32>(in.uv.x * 0.5 + 0.5, 1.0 - in.uv.y);
    let gx = dpdx(atlas_uv);
    let gy = dpdy(atlas_uv);
    let cy = (in.uv.y - 0.62) / 0.38;
    let r = length(vec2<f32>(in.uv.x, cy));
    let ang = atan2(cy, in.uv.x);
    let lobes = 0.08 * sin(ang * 5.0 + in.seed) + 0.05 * sin(ang * 11.0 + in.seed * 1.7);
    var crown = r < 0.92 + lobes;
    var trunk = in.uv.y < 0.3 && abs(in.uv.x) < 0.07;
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
    let v = normalize(frame.cam_pos.xyz - in.world);
    let toward = normalize(vec3<f32>(v.x, 0.0, v.z) + vec3<f32>(1e-5, 0.0, 0.0));
    var palm = false;
// sf:end
// sf:FAR_PALMS+
    var ph: FtPalm;
    if (u32(in.size.z + 0.5) == FT_PALM && !baked) {
        palm = true;
        ph = ft_palm(in.uv, in.size.xy, in.seed, in.right, toward, max(length(dpx), length(dpy)));
        crown = ph.frond;
        trunk = ph.trunk;
        covered = crown || trunk;
    }
// sf:end
// sf:CROWN|FAR_PALMS+
    if (!covered) {
        discard;
    }
// sf:end
// sf:CROWN&!LOD_FADE|FAR_PALMS&!LOD_FADE+
    let shift = vec2<f32>(0.0);
// sf:end
// sf:CROWN&LOD_FADE|FAR_PALMS&LOD_FADE+
    let shift = vec2<f32>(5.588238 * look.post2.w); // (`--temporal-lod-fade`) the pattern moves every frame
// sf:end
// sf:CROWN|FAR_PALMS+
    // discarded after the lighting (see `ft_shade`)
    let faded_out = in.fade < 0.999 && hlod_dither(floor(in.clip.xy) + shift) >= in.fade;
    var n = vec3<f32>(0.0, 1.0, 0.0);
    var albedo = in.tint;
    var ao = select(1.0, 0.7 + 0.3 * in.uv.y, crown);
    if (baked) {
        let surface = mix(textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, a + FT_NORMALS, gx, gy),
                          textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, b + FT_NORMALS, gx, gy), blend);
        let local = surface.xyz / max(surface.a, 0.01) * 2.0 - 1.0;
        n = normalize(in.right * local.x + vec3<f32>(0.0, local.y, 0.0) + toward * local.z + vec3<f32>(0.0, 1e-4, 0.0));
        albedo = srgb_decode(leaf.rgb / max(leaf.a, 0.01));
    } else if (crown) {
        let z = sqrt(max(0.0, 1.0 - min(r * r, 1.0)));
        n = normalize(in.right * in.uv.x + vec3<f32>(0.0, cy, 0.0) + toward * z);
        albedo = in.tint * (0.75 + 0.25 * st_hash(floor(in.world.xz * 0.7) + in.seed));
    } else {
        n = toward;
        albedo = vec3<f32>(0.09, 0.07, 0.05);
    }
    var vol = false;
    var sun = vec3<f32>(0.0);
// sf:end
// sf:FAR_PALMS+
    if (palm) {
        n = ph.n;
        albedo = ph.albedo;
        ao = ph.ao;
    }
// sf:end
// sf:CROWN+
    if (!baked && !palm) {
        if (crown) {
            // (`--crown-light`) the crown as a volume of leaves: its sky light at
            // a leaf medium's albedo (omega / 2) through the leaves above, its sun
            // added after `look_light` (which lays none on it: `lk_leaf_sun`).
            vol = true;
            let rho = ft_leaf_r(in.tint, in.seed);
            let cv = ft_crown(in.uv, in.size.xy, in.right, v, rho);
            albedo = 0.5 * rho * (vec3<f32>(1.0) + FT_LEAF_T_OVER_R);
            ao = cv.w;
            sun = cv.xyz;
            lk_leaf_sun = 0.0;
            lk_vis_wrap = 2.0;
        } else {
            ao = 0.65; // the trunk under the crown
        }
    }
// sf:end
// sf:CROWN|FAR_PALMS+
    var s: Surface;
    s.albedo = albedo;
    s.rough = 0.92;
    s.metal = 0.0;
    s.ao = ao;
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
    var lit = look_light(s, m, v, in.world, select(-face, face, dot(face, v) > 0.0), 0u);
// sf:end
// sf:CROWN+
    if (vol) {
        // the sun through the crown, at the key's shadow and clouds where look_light looked, fogged as it fogged
        lit = vec4<f32>(lit.rgb + look.key.xyz * sun * lk_key_vis * (1.0 - lk_fog_out), lit.a);
        lk_leaf_sun = 1.0;
        lk_vis_wrap = 0.0;
    }
// sf:end
// sf:FOLIAGE&CROWN|FOLIAGE&FAR_PALMS+
    if ((crown || baked) && !vol) {
        // (`--foliage-light`) the light through the leaves (a volume's crown has it in its phase function)
        lit = vec4<f32>(lit.rgb + ft_foliage(albedo, n, v, in.world), lit.a);
    }
// sf:end
// sf:CROWN|FAR_PALMS+
    if (faded_out) {
        discard;
    }
    return lit;
}
// sf:end
@fragment
fn far_tree_fs(in: FtOut) -> @location(0) vec4<f32> {
// sf:CROWN|FAR_PALMS-
    return ft_shade(in);
// sf:end
// sf:CROWN|FAR_PALMS+
    return ft_shade2(in);
// sf:end
}

@fragment
fn far_tree_fs_ids(in: FtOut) -> IdsOut {
    var out: IdsOut;
// sf:CROWN|FAR_PALMS-
    out.color = ft_shade(in);
// sf:end
// sf:CROWN|FAR_PALMS+
    out.color = ft_shade2(in);
// sf:end
    out.ids = vec2<u32>(0xfffffffeu, 0u);
    return out;
}
