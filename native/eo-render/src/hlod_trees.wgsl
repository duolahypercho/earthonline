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
    // the card's form: 0 the round park crown, 1 cypress, 2 pine, 3 blue gum (hlod_trees.rs)
    species: u32,
};

struct FtParams {
    // x near (none closer), y full (faded in), z on, w 0 (round 2's light and
    // dither) or 1 + the frame's dither phase (the leaf-volume light)
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
    // crown half width, height (m), form
    @location(6) @interpolate(flat) dims: vec3<f32>,
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
    out.dims = vec3<f32>(t.crown, t.height, f32(t.species));
    return out;
}

// The prevailing wind's downwind direction in the strait (from the west-north-west,
// toward 110 degrees; x east, z south): the coastal cypress crowns are sheared
// that way (INFERRED from the westerly sea breeze through the Golden Gate).
const FT_DOWNWIND: vec2<f32> = vec2<f32>(0.940, 0.342);
// A leaf's diffuse transmittance over its reflectance in the visible: green
// leaves pass light of the same order as they reflect (Jacquemoud and Ustin,
// Leaf Optical Properties, 2019, ch. 1: the PROSPECT spectra), needles
// less; one half, INFERRED.
const FT_LEAF_T_OVER_R: f32 = 0.5;
// Ross's G-function of a spherical leaf-angle distribution (see ft_shade).
const FT_ROSS_G: f32 = 0.5;
// Leaf area per crown volume, m2 per m3 (INFERRED: a dense coastal conifer or
// eucalypt crown; with the crown radius it sets how far sun reaches into the shade side).
const FT_LAD: f32 = 1.0;

fn ftv_hash(seed: f32, k: f32) -> f32 {
    return fract(sin(seed * 12.9898 + k * 78.233) * 43758.5453);
}

// One ellipsoidal clump of the crown at c (m) with radii r (m), an edge lobed by
// `lobe`: keeps the clump nearest the eye (largest depth z) in `best`
// (q.x, q.y, z in the clump's unit sphere, 1 when hit).
fn ftv_clump(p: vec2<f32>, c: vec2<f32>, r: vec2<f32>, lobe: f32, flat_top: f32, best: ptr<function, vec4<f32>>) {
    var q = (p - c) / max(r, vec2<f32>(0.05));
    if (q.y > 0.0) {
        q.y = q.y * (1.0 + flat_top);
    }
    let ang = atan2(q.y, q.x);
    let edge = 1.0 + lobe * (0.6 * sin(ang * 6.0 + c.x * 3.1) + 0.4 * sin(ang * 13.0 + c.y * 1.7));
    let d2 = dot(q, q) / (edge * edge);
    if (d2 < 1.0) {
        let z = sqrt(1.0 - d2);
        if ((*best).w < 0.5 || z > (*best).z) {
            *best = vec4<f32>(q.x / edge, q.y / edge, z, 1.0);
        }
    }
}

// A species' crown on the card (p in metres from the trunk base, card plane):
// (q.x, q.y, z, hit) of the clump the pixel shows, and in .w of the second
// result the trunk (1) or nothing.
fn ftv_species_crown(p: vec2<f32>, crown: f32, height: f32, sp: u32, seed: f32, lean: f32) -> vec4<f32> {
    var best = vec4<f32>(0.0);
    if (sp == 1u) {
        // Monterey cypress: broad, flat-topped and layered, sheared downwind.
        for (var k = 0; k < 4; k = k + 1) {
            let kf = f32(k);
            let y = height * (0.42 + 0.15 * kf);
            let x = lean * crown * (y / height - 0.3) + (ftv_hash(seed, kf) - 0.5) * 0.4 * crown;
            let rx = min(crown * (0.95 - 0.15 * kf) * (0.85 + 0.25 * ftv_hash(seed, kf + 7.0)), crown - abs(x));
            ftv_clump(p, vec2<f32>(x, y), vec2<f32>(rx, height * (0.11 + 0.05 * ftv_hash(seed, kf + 2.0))), 0.14, 0.3, &best);
        }
    } else if (sp == 2u) {
        // Monterey pine: a rounded, irregular dome of dense clumps on a long bole.
        for (var k = 0; k < 6; k = k + 1) {
            let kf = f32(k);
            let a = mix(-1.1, 1.1, (kf + 0.5) / 6.0) + (ftv_hash(seed, kf) - 0.5) * 0.35;
            let r = crown * 0.42 * (0.8 + 0.4 * ftv_hash(seed, kf + 3.0));
            let c = vec2<f32>(sin(a) * 0.55 * crown, height * (0.6 + 0.22 * cos(a)));
            let rx = min(r, crown - abs(c.x));
            ftv_clump(p, c, vec2<f32>(rx, min(r * 0.9, height - c.y)), 0.12, 0.0, &best);
        }
    } else {
        // blue gum: tall and open, a few drooping clumps high on a pale trunk, sky between.
        for (var k = 0; k < 5; k = k + 1) {
            let kf = f32(k);
            let c = vec2<f32>((ftv_hash(seed, kf) - 0.5) * 1.1 * crown, height * (0.5 + 0.34 * ftv_hash(seed, kf + 5.0)));
            let rx = min(crown * 0.36 * (0.8 + 0.4 * ftv_hash(seed, kf + 9.0)), crown - abs(c.x));
            let ry = min(height * 0.14 * (0.8 + 0.4 * ftv_hash(seed, kf + 11.0)), height - c.y);
            ftv_clump(p, c, vec2<f32>(rx, ry), 0.14, -0.3, &best);
        }
    }
    return best;
}

// Where a species' trunk ends (m over its base): at its lowest clump's centre,
// so no bare bole shows in the gaps between the clumps above it.
fn ftv_trunk_top(height: f32, sp: u32, seed: f32) -> f32 {
    if (sp == 1u) {
        return height * 0.42;
    }
    var lo = height;
    if (sp == 2u) {
        for (var k = 0; k < 6; k = k + 1) {
            let kf = f32(k);
            let a = mix(-1.1, 1.1, (kf + 0.5) / 6.0) + (ftv_hash(seed, kf) - 0.5) * 0.35;
            lo = min(lo, height * (0.6 + 0.22 * cos(a)));
        }
    } else {
        for (var k = 0; k < 5; k = k + 1) {
            lo = min(lo, height * (0.5 + 0.34 * ftv_hash(seed, f32(k) + 5.0)));
        }
    }
    return lo;
}

// A clump's radius over the crown's half width, for the chord the sun crosses
// in it (the species forms above: about 0.36-0.5 for pine and gum clumps, the
// cypress's flat layers wider but as thin; INFERRED).
const FT_CLUMP_R: f32 = 0.4;

fn ft_shade(in: FtOut) -> vec4<f32> {
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let light3 = ft.range.w >= 1.0;
    let sp = u32(in.dims.z + 0.5);
    let v = normalize(frame.cam_pos.xyz - in.world);
    var crown = false;
    var trunk = false;
    var n = vec3<f32>(0.0, 1.0, 0.0);
    var cy = 0.0;
    var r = 0.0;
    if (sp == 0u) {
        // The crown: an ellipse over the upper 70 % of the card, a lobed edge.
        cy = (in.uv.y - 0.62) / 0.38;
        r = length(vec2<f32>(in.uv.x, cy));
        let ang = atan2(cy, in.uv.x);
        let lobes = 0.08 * sin(ang * 5.0 + in.seed) + 0.05 * sin(ang * 11.0 + in.seed * 1.7);
        crown = r < 0.92 + lobes;
        // The trunk: the lower part, a narrow column.
        trunk = in.uv.y < 0.3 && abs(in.uv.x) < 0.07;
    } else {
        let p = vec2<f32>(in.uv.x * in.dims.x, in.uv.y * in.dims.y);
        let lean = select(0.0, dot(vec2<f32>(in.right.x, in.right.z), FT_DOWNWIND) * 0.35, sp == 1u);
        let b = ftv_species_crown(p, in.dims.x, in.dims.y, sp, in.seed, lean);
        crown = b.w > 0.5;
        let tx = lean * in.dims.x * (in.uv.y - 0.3) * select(0.0, 1.0, in.uv.y > 0.3);
        let top = select(select(0.62, 0.5, sp == 3u) * in.dims.y, ftv_trunk_top(in.dims.y, sp, in.seed), light3);
        trunk = p.y < top && abs(p.x - tx) < max(0.25, 0.012 * in.dims.y);
        cy = b.y;
        r = length(b.xy);
        if (crown) {
            // the clump's sphere, seen from the eye
            let up = vec3<f32>(0.0, 1.0, 0.0);
            let toward = select(normalize(vec3<f32>(v.x, 0.0, v.z) + vec3<f32>(1e-5, 0.0, 0.0)), v, light3);
            let upp = normalize(up - toward * dot(up, toward));
            n = normalize(in.right * b.x + upp * b.y + toward * b.z);
        }
    }
    // the card's outline and the fade's screen door (with the leaf-volume light the
    // door's phase steps each frame, so the temporal resolve averages it).
    let door = select(hlod_dither(floor(in.clip.xy)), fract(hlod_dither(floor(in.clip.xy)) + fract(ft.range.w)), light3);
    let cut = !(crown || trunk) || (in.fade < 0.999 && door >= in.fade);
    if (!FT_LATE_DISCARD && cut) {
        discard;
    }
    var albedo = in.tint;
    if (crown) {
        if (sp == 0u) {
            let z = sqrt(max(0.0, 1.0 - min(r * r, 1.0)));
            if (light3) {
                // the crown's sphere seen from the eye (its view from above shows the lit top)
                let up = vec3<f32>(0.0, 1.0, 0.0);
                let upp = normalize(up - v * dot(up, v));
                n = normalize(in.right * in.uv.x + upp * cy + v * z);
            } else {
                let toward = normalize(vec3<f32>(v.x, 0.0, v.z) + vec3<f32>(1e-5, 0.0, 0.0));
                n = normalize(in.right * in.uv.x + vec3<f32>(0.0, cy, 0.0) + toward * z);
            }
        }
        albedo = in.tint * (0.75 + 0.25 * st_hash(floor(in.world.xz * 0.7) + in.seed));
    } else {
        n = normalize(vec3<f32>(v.x, 0.0, v.z) + vec3<f32>(1e-5, 0.0, 0.0));
        // bark: the park trees' dark brown; the blue gum's smooth pale trunk (Cal-IPC: peels to a pale trunk; INFERRED albedo)
        albedo = select(vec3<f32>(0.09, 0.07, 0.05), vec3<f32>(0.22, 0.20, 0.16), sp == 3u);
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
    let fn_ = select(-face, face, dot(face, v) > 0.0);
    // the leaf-volume light reads the shadow once per tree: at the crown's centre
    // (its base recovered from the card point), moved a crown's half width toward
    // the sun. A card is a flat stand-in for the crown, and the shadow read at each
    // of its points drew the shadow map's texel rows across it as stripes. So the
    // key light is left out of look_light (a geometric normal facing from the sun
    // turns it off there) and added below with that one shadow.
    let l = look.key_dir.xyz;
    let vol_light = light3 && crown;
    let ng = select(fn_, -l, vol_light);
    var lit = look_light(s, m, v, in.world, ng, 0u);
    if (vol_light) {
        // the leaf volume's light through it: the sun on the far side of the clump,
        // passed by its leaves (a diffuse transmission, FT_LEAF_T_OVER_R of the
        // reflectance), the term a sphere of opaque leaves leaves out. Shadowed
        // and fogged as the key light, the props' back-lit leaves' form (props_draw.wgsl).
        let through = max(dot(-n, l), 0.0);
        let base = in.world - in.right * (in.uv.x * in.dims.x) - vec3<f32>(0.0, in.uv.y * in.dims.y, 0.0);
        let centre = base + vec3<f32>(0.0, 0.68 * in.dims.y, 0.0);
        let sh = lk_shadow(centre + l * in.dims.x, l) * sky_cloud_transmittance(in.world);
        let vd = dot(in.world - look.cam_pos.xyz, look.cam_fwd.xyz);
        let fog = lk_smooth(look.fog.x, look.fog.y, vd);
        let trans = look.key.xyz * s.albedo * FT_LEAF_T_OVER_R * through * s.ao * sh / LK_PI;
        // a crown is a leaf volume, not an opaque sphere: with leaves of every
        // orientation the sun reaches the leaves at the surface with a mean
        // projection G (Ross's G-function, 0.5 for a spherical leaf-angle
        // distribution: J. Ross, The Radiation Regime and Architecture of Plant
        // Stands, 1981), on the far side of the crown too, thinned by the leaf
        // area on the way in (Beer's law over the chord 2 R cos to the sun, leaf
        // area density FT_LAD). The opaque sphere's n.l (look_light) becomes
        // G P + (1 - G) n.l on the lit side and G P on the far side.
        let nl = dot(n, l);
        let chord = 2.0 * FT_CLUMP_R * in.dims.x * max(-nl, 0.0);
        let pen = exp(-FT_ROSS_G * FT_LAD * chord);
        let vol = look.key.xyz * s.albedo * FT_ROSS_G * (pen - max(nl, 0.0)) * s.ao * sh / LK_PI;
        // the key light look_light left out: the opaque sphere's n.l (no specular on
        // leaves this rough), with the tree's one shadow.
        let key = look.key.xyz * s.albedo * max(nl, 0.0) * sh / LK_PI;
        lit = vec4<f32>(lit.rgb + key * (1.0 - fog) + (trans + vol) * (1.0 - fog), lit.a);
    }
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
