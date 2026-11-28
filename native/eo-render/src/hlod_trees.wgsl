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
    // the card's form (FT_FORM_MASK: 0 the round park crown, 1 cypress, 2 pine,
    // 3 blue gum), FT_BAKED with the baked Black Point canopy, and that canopy's
    // one-based model from FT_MODEL_SHIFT up (hlod_trees.rs `FarTree::species`)
    species: u32,
};

struct FtParams {
    // x near (none closer), y full (faded in), z on, w 0 (round 2's light and
    // dither) or 1 + the frame's dither phase (the leaf-volume light)
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
    // the baked canopy's two nearest views (layers) and their blend; x < 0 none
    @location(6) @interpolate(flat) view: vec3<f32>,
    // crown half width, height (m), form
    @location(7) @interpolate(flat) dims: vec3<f32>,
// sf:CROWN|FAR_PALMS+
    // (`--crown-light`, `--far-palms`) the crown's half width and the tree's height (m), its species.
    @location(8) @interpolate(flat) size: vec3<f32>,
// sf:end
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
// sf:FAR_PALMS-
    if (d < ft.range.x || dot(rel, look.cam_fwd.xyz) < 0.0) {
        return out;
    }
// sf:end
// sf:FAR_PALMS+
    // (`--far-palms`) a palm's card takes over where the street props' palm fades out.
    var near = ft.range.xy;
    if ((t.species & FT_FORM_MASK) == FT_PALM) {
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
    let raw_tint = unpack4x8unorm(t.tint).rgb;
    let sp0 = t.species & FT_FORM_MASK;
    let is_baked = (t.species & FT_BAKED) != 0u;
    let aaa_palm_vs = SF_AAA_DETAIL && sp0 == 0u && !is_baked && raw_tint.r > 0.29 && raw_tint.r < 0.32 && abs(raw_tint.g - 96.0 / 255.0) < 0.016 && abs(raw_tint.b - 52.0 / 255.0) < 0.016;
    var crown_w = t.crown;
    var tree_h = t.height;
    if (aaa_palm_vs) {
        // Widen Embarcadero & street palm cards to natural Canary palm frond reach when `--far-palms` is off.
        crown_w = max(t.crown, 0.44 * t.height);
    } else if (SF_AAA_DETAIL && !is_baked) {
        // Widen narrow vertical tree quads (park pines, columnar/tall street trees) and cap skinny vertical
        // aspect ratios so canopies read as broad spreading California trees rather than vertical eggs.
        crown_w = max(t.crown * 1.28, min(t.height * 0.56, t.crown * 1.85));
        tree_h = min(t.height, crown_w * 2.10);
    }
// sf:WEATHER_FX+
    var sway = vec3<f32>(0.0);
    if (sky_weather_fx_on(skyu.cirrus) && c.y > 0.0) {
        let wx_spd = length(skyu.wind.xy);
        let wd = select(vec2<f32>(0.94, 0.34), skyu.wind.xy / max(wx_spd, 1e-4), wx_spd > 0.25);
        let eff_spd = select(1.8, wx_spd, wx_spd > 0.25) * clamp(skyu.wind.z, 0.5, 1.6);
        let ws = clamp((eff_spd - 1.2) / 11.8, 0.0, 1.0);
        let amp = (0.07 + 0.50 * pow(ws, 1.15)) * (tree_h / 8.0);
        let wt = skyu.wet.w;
        let adv = dot(t.pos.xz, wd) * 0.042;
        let ph = f32(t.seed & 1023u) * 0.37;
        let squall = 0.5 + 0.5 * sin(adv * 0.45 - wt * (0.42 + 0.65 * ws));
        let wave = (0.30 + 0.58 * squall) * (0.55 * ws) + sin(adv - wt * (1.15 + 2.35 * ws) + ph * 0.35) * 0.72;
        let d_xz = wd * (wave * amp * c.y * c.y);
        sway = vec3<f32>(d_xz.x, -dot(d_xz, d_xz) / max(2.0 * tree_h, 2.0), d_xz.y);
    }
    let p = t.pos + right * (c.x * crown_w) + vec3<f32>(0.0, c.y * tree_h, 0.0) + sway;
// sf:end
// sf:WEATHER_FX-
    let p = t.pos + right * (c.x * crown_w) + vec3<f32>(0.0, c.y * tree_h, 0.0);
// sf:end
    out.clip = frame.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    out.uv = c;
    out.tint = srgb_decode(raw_tint);
// sf:FAR_PALMS-
    out.fade = smoothstep(ft.range.x, ft.range.y, d);
// sf:end
// sf:FAR_PALMS+
    out.fade = smoothstep(near.x, near.y, d);
// sf:end
// sf:CROWN|FAR_PALMS+
    out.size = vec3<f32>(crown_w, tree_h, f32(t.species & FT_FORM_MASK));
// sf:end
    out.seed = f32(t.seed & 1023u);
    out.right = right;
    out.view = vec3<f32>(-1.0);
    let model = t.species >> FT_MODEL_SHIFT;
    if ((t.species & FT_BAKED) != 0u && model != 0u) {
        // A stable rotation per stem. The projected view blends continuously
        // through north when a swimmer moves sideways along the shore.
        let turns = atan2(to_cam.y, to_cam.x) / 6.2831853 + f32(t.seed & 65535u) / 65536.0;
        let azimuth = fract(turns) * f32(FT_VIEWS);
        let base = (model - 1u) * FT_VIEWS;
        let view = u32(azimuth);
        out.view = vec3<f32>(f32(base + view), f32(base + ((view + 1u) % FT_VIEWS)), fract(azimuth));
    }
    out.dims = vec3<f32>(crown_w, tree_h, f32(t.species & FT_FORM_MASK));
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
const FT_VOL_LEAF_T_OVER_R: f32 = 0.5;
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

// Coastal California foliage calibration (`SF_AAA_DETAIL`): physically measured
// LOPEX93-range daylight reflectances (linear lum 0.068..0.115, muted coastal
// G/R 1.14..1.34) so trees read as natural olive/cypress/eucalyptus/plane in
// bright sun AND keep layered green volume inside building shadows (`08` park).
fn ft_ca_palette(seed: f32, tint: vec3<f32>, world_xz: vec2<f32>) -> vec3<f32> {
    let h0 = fract(sin(seed * 12.9898 + dot(floor(world_xz * 0.08), vec2<f32>(78.233, 37.719))) * 43758.5453);
    let h1 = fract(seed * 0.6180339 + 0.19);
    let h2 = fract(seed * 1.3247179 + 0.41);
    var col: vec3<f32>;
    if (h0 < 0.22) {
        col = vec3<f32>(0.088, 0.106, 0.066); // Mediterranean olive / holm oak
    } else if (h0 < 0.44) {
        col = vec3<f32>(0.066, 0.088, 0.056); // Monterey cypress / coast live oak
    } else if (h0 < 0.62) {
        col = vec3<f32>(0.084, 0.106, 0.090); // silvery blue-gum eucalyptus
    } else if (h0 < 0.80) {
        col = vec3<f32>(0.098, 0.118, 0.064); // London plane / street sycamore
    } else if (h0 < 0.90) {
        col = vec3<f32>(0.074, 0.094, 0.060); // Monterey pine / coastal conifer
    } else {
        col = vec3<f32>(0.114, 0.110, 0.062); // late-summer sun-warmed olive accent
    }
    let warm = mix(vec3<f32>(0.94, 0.99, 1.05), vec3<f32>(1.07, 1.00, 0.90), h2);
    return col * warm * (0.88 + 0.26 * h1);
}

// Identify far palm cards (`hlod_trees.rs:94` sets `rgb(78, 96, 52)` exclusively on palms)
// even when `--far-palms` is disabled, so Embarcadero palms render as feathered crowns.
fn ft_is_aaa_palm(tint: vec3<f32>, sp: u32, baked: bool) -> bool {
    return SF_AAA_DETAIL && sp == 0u && !baked
        && tint.r > 0.068 && tint.r < 0.085
        && tint.g > 0.105 && tint.g < 0.130
        && tint.b > 0.028 && tint.b < 0.042;
}

struct FtAaaPalm {
    frond: bool,
    trunk: bool,
    n: vec3<f32>,
    albedo: vec3<f32>,
    ao: f32,
    open: f32,
};

// Compact feathered Canary Island date palm for far cards under `SF_AAA_DETAIL`.
fn ft_aaa_palm(uv: vec2<f32>, dims: vec2<f32>, seed: f32, right: vec3<f32>, toward: vec3<f32>, px: f32) -> FtAaaPalm {
    var o: FtAaaPalm;
    let rc = max(dims.x, 1.5);
    let h = max(dims.y, 4.0);
    let x = uv.x * rc;
    let y = uv.y * h;
    let yh = h - 0.50 * rc;
    let rt = clamp(0.030 * h, 0.20, 0.38) * (1.0 + 0.25 * exp(-y / 0.7));
    o.trunk = y < yh + 0.08 * rc && abs(x) < max(rt, 0.45 * px);
    let q = vec2<f32>(x, y - yh) / rc;
    let w_min = 0.45 * px / rc;
    var best = 1e9;
    var bt = vec2<f32>(0.0, 1.0);
    var bh = 0.0;
    var hang = 0.0;
    for (var i = 0u; i < 18u; i = i + 1u) {
        let fi = f32(i);
        let hs = fract(sin(fi * 12.9898 + seed * 0.731) * 43758.5453);
        let th = (fi + 0.55 * hs) / 18.0 * 6.2831853 + seed;
        let dir = vec2<f32>(cos(th), 0.78 * sin(th) + 0.26);
        let len = 0.96 * (0.54 + 0.46 * abs(cos(th * 1.37 + seed * 2.1)));
        let s0 = clamp(dot(q, dir) / max(dot(dir, dir), 1e-4), 0.0, len);
        let p0 = s0 * dir - vec2<f32>(0.0, 0.52 * s0 * s0);
        let tg0 = dir - vec2<f32>(0.0, 1.04 * s0);
        let s1 = clamp(s0 + dot(q - p0, tg0) / max(dot(tg0, tg0), 1e-4), 0.0, len);
        let p1 = s1 * dir - vec2<f32>(0.0, 0.52 * s1 * s1);
        let f = s1 / max(len, 1e-4);
        let serrate = 0.76 + 0.28 * sin(f * 26.0 + fi * 1.9);
        let w = max(0.155 * sin(3.14159 * clamp(f * 0.9 + 0.06, 0.0, 1.0)) * (0.45 + 0.55 * f) * serrate, w_min);
        let d = length(q - p1) / w;
        if (d < best) {
            best = d;
            bt = normalize(dir - vec2<f32>(0.0, 1.04 * s1));
            bh = hs;
            hang = smoothstep(0.0, -0.45, dir.y);
        }
    }
    o.frond = best < 1.0 || length(q * vec2<f32>(1.0, 1.5)) < 0.18;
    let tone = 0.90 + 0.22 * fract(seed * 0.377);
    if (o.frond) {
        let up = vec3<f32>(0.0, 1.0, 0.0);
        o.n = normalize(right * (0.42 * bt.x) + up * 0.86 + toward * (0.26 + 0.25 * bh));
        let dead = hang * step(0.50, bh);
        o.albedo = mix(vec3<f32>(0.086, 0.112, 0.058), vec3<f32>(0.18, 0.145, 0.082), dead) * tone;
        o.open = smoothstep(0.08, 0.75, length(q));
        o.ao = mix(0.72, 1.0, o.open);
    } else {
        let xn = clamp(x / max(rt, 0.45 * px), -1.0, 1.0);
        o.n = normalize(right * xn + toward * sqrt(max(1.0 - xn * xn, 0.0)));
        o.albedo = vec3<f32>(0.16, 0.125, 0.092) * tone;
        o.open = 0.4;
        o.ao = mix(0.94, 0.68, smoothstep(yh - 1.8, yh, y));
    }
    return o;
}

// Multi-bough organic crown silhouette for generic far trees (`sp == 0u`) when
// `SF_AAA_DETAIL` is on: returns (nx, ny, nz, open_ao) with open_ao >= 0 inside crown.
// Uses separated asymmetric sub-boughs, ragged leaf-sprig edges, and porous outer-rim
// sky peep-holes so silhouettes read as airy California trees instead of eggs or balls.
fn ft_aaa_crown(uv: vec2<f32>, seed: f32) -> vec4<f32> {
    let habit = u32(floor(fract(seed * 0.371 + 0.13) * 4.0));
    let flip = select(-1.0, 1.0, fract(seed * 0.831 + 0.07) > 0.5);
    var best_z = -1.0;
    var best_n = vec3<f32>(0.0, 0.4, 0.9);
    var best_open = 0.5;
    let p = vec2<f32>(uv.x, uv.y);
    let edge_n = st_noise(uv * vec2<f32>(8.5, 10.5) + vec2<f32>(seed * 3.1, seed * 1.9)) - 0.5;
    let fine_n = st_noise(uv * vec2<f32>(21.0, 25.0) + vec2<f32>(seed * 7.3, 2.1)) - 0.5;
    let peep_n = st_noise(uv * vec2<f32>(28.0, 32.0) + vec2<f32>(seed * 4.3, 9.7));
    for (var k = 0; k < 6; k = k + 1) {
        let kf = f32(k);
        let jx = ftv_hash(seed, kf * 3.1 + 1.0) - 0.5;
        let jy = ftv_hash(seed, kf * 3.1 + 2.0) - 0.5;
        let jr = ftv_hash(seed, kf * 3.1 + 3.0);
        var c: vec2<f32>;
        var rad: vec2<f32>;
        if (habit == 0u) {
            // Wide spreading Coast Live Oak / London Plane: flat-topped asymmetric broad umbrella
            var ox = array<f32, 6>(-0.62, -0.32, 0.14, 0.54, 0.66, -0.10);
            var oy = array<f32, 6>(0.55, 0.72, 0.75, 0.67, 0.53, 0.56);
            c = vec2<f32>(ox[k] * flip + jx * 0.14, oy[k] + jy * 0.06);
            rad = vec2<f32>(0.30 + 0.10 * jr, 0.14 + 0.045 * jr);
        } else if (habit == 1u) {
            // Wind-sculpted Monterey Cypress / Coastal Pine: flat separated horizontal plates
            let tier = floor(kf * 0.5) / 2.0; // 3 horizontal tiers, 2 plates each
            let side = select(-0.38, 0.38, (k & 1) == 1);
            let shear = flip * (tier * 0.24 - 0.10);
            c = vec2<f32>(side + shear + jx * 0.18, 0.46 + tier * 0.34 + jy * 0.04);
            rad = vec2<f32>(0.36 + 0.11 * jr, 0.09 + 0.025 * jr);
        } else if (habit == 2u) {
            // Open airy Blue-Gum Eucalyptus: wide separated billowing tufts high on a pale bole
            var ox = array<f32, 6>(-0.54, 0.48, -0.26, 0.30, -0.02, -0.52);
            var oy = array<f32, 6>(0.54, 0.58, 0.74, 0.78, 0.64, 0.69);
            c = vec2<f32>(ox[k] * flip + jx * 0.16, oy[k] + jy * 0.06);
            rad = vec2<f32>(0.26 + 0.09 * jr, 0.125 + 0.04 * jr);
        } else {
            // Asymmetric stepped Mediterranean broadleaf: twin offset shoulders + sprig tufts
            var ox = array<f32, 6>(-0.56, -0.22, 0.36, 0.60, 0.16, -0.36);
            var oy = array<f32, 6>(0.52, 0.74, 0.64, 0.54, 0.77, 0.63);
            c = vec2<f32>(ox[k] * flip + jx * 0.14, oy[k] + jy * 0.06);
            rad = vec2<f32>(0.29 + 0.09 * jr, 0.135 + 0.04 * jr);
        }
        let q = (p - c) / rad;
        let rr = length(q) + edge_n * 0.34 + fine_n * 0.22;
        let pore = select(0.0, smoothstep(0.36, 0.14, peep_n) * 0.28, rr > 0.68);
        if (rr + pore < 0.94) {
            let z = sqrt(max(0.0, 1.0 - min(rr * rr, 1.0)));
            let open = clamp(0.42 * clamp(q.y * 0.5 + 0.5, 0.0, 1.0) + 0.38 * (1.0 - z) + 0.20 * clamp((c.y - 0.38) / 0.48, 0.0, 1.0), 0.0, 1.0);
            let depth_score = z + c.y * 0.24;
            if (depth_score > best_z) {
                best_z = depth_score;
                best_n = normalize(vec3<f32>(q.x * 0.78 + fine_n * 0.50, q.y * 0.78 + 0.32 + edge_n * 0.38, max(z, 0.22)));
                best_open = open;
            }
        }
    }
    if (best_z < 0.0) {
        return vec4<f32>(0.0, 1.0, 0.0, -1.0);
    }
    return vec4<f32>(best_n, best_open);
}

fn ft_shade(in: FtOut) -> vec4<f32> {
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let atlas_uv = vec2<f32>(in.uv.x * 0.5 + 0.5, 1.0 - in.uv.y);
    let gx = dpdx(atlas_uv);
    let gy = dpdy(atlas_uv);
    let light3 = ft.range.w >= 1.0;
    let sp = u32(in.dims.z + 0.5);
    let v = normalize(frame.cam_pos.xyz - in.world);
    var crown = false;
    var trunk = false;
    var n = vec3<f32>(0.0, 1.0, 0.0);
    var cy = 0.0;
    var r = 0.0;
    var aaa_open = 0.5;
    let baked = in.view.x >= 0.0;
    let toward_h = normalize(vec3<f32>(v.x, 0.0, v.z) + vec3<f32>(1e-5, 0.0, 0.0));
    let aaa_palm = ft_is_aaa_palm(in.tint, sp, baked);
    var aaa_ph: FtAaaPalm;
    if (aaa_palm) {
        aaa_ph = ft_aaa_palm(in.uv, in.dims.xy, in.seed, in.right, toward_h, max(length(dpx), length(dpy)));
        crown = aaa_ph.frond;
        trunk = aaa_ph.trunk;
        n = aaa_ph.n;
        aaa_open = aaa_ph.open;
    } else if (sp == 0u) {
        if (SF_AAA_DETAIL) {
            let ac = ft_aaa_crown(in.uv, in.seed);
            crown = ac.w >= 0.0;
            aaa_open = max(ac.w, 0.0);
            cy = ac.y;
            r = length(ac.xy);
            if (crown) {
                let up = vec3<f32>(0.0, 1.0, 0.0);
                let toward = select(toward_h, v, light3);
                let upp = normalize(up - toward * dot(up, toward));
                n = normalize(in.right * ac.x + upp * ac.y + toward * ac.z);
            }
            let trunk_w = mix(0.056, 0.030, clamp(in.uv.y / 0.48, 0.0, 1.0));
            let main_bole = in.uv.y < 0.46 && abs(in.uv.x) < trunk_w;
            let fork_t = clamp((in.uv.y - 0.30) / 0.24, 0.0, 1.0);
            let limb_l = in.uv.y >= 0.30 && in.uv.y < 0.54 && abs(in.uv.x + fork_t * 0.22) < mix(0.028, 0.014, fork_t);
            let limb_r = in.uv.y >= 0.32 && in.uv.y < 0.56 && abs(in.uv.x - fork_t * 0.20) < mix(0.026, 0.013, fork_t);
            trunk = main_bole || limb_l || limb_r;
        } else {
            // The crown: an ellipse over the upper 70 % of the card, a lobed edge.
            cy = (in.uv.y - 0.62) / 0.38;
            r = length(vec2<f32>(in.uv.x, cy));
            let ang = atan2(cy, in.uv.x);
            let lobes = 0.08 * sin(ang * 5.0 + in.seed) + 0.05 * sin(ang * 11.0 + in.seed * 1.7);
            crown = r < 0.92 + lobes;
            // The trunk: the lower part, a narrow column.
            trunk = in.uv.y < 0.3 && abs(in.uv.x) < 0.07;
        }
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
        aaa_open = clamp(0.45 * (b.y * 0.5 + 0.5) + 0.35 * (1.0 - b.z) + 0.20 * in.uv.y, 0.0, 1.0);
        if (crown) {
            // the clump's sphere, seen from the eye
            let up = vec3<f32>(0.0, 1.0, 0.0);
            let toward = select(toward_h, v, light3);
            let upp = normalize(up - toward * dot(up, toward));
            n = normalize(in.right * b.x + upp * b.y + toward * b.z);
        }
    }
    // the baked Black Point canopy (`FarTree::species` FT_BAKED): its two nearest
    // views blended, their coverage the card's outline in place of the form's.
    var leaf = vec4<f32>(0.0);
    let view_a = max(i32(in.view.x), 0);
    let view_b = max(i32(in.view.y), 0);
    let view_blend = smoothstep(0.0, 1.0, in.view.z);
    var covered = crown || trunk;
    if (baked) {
        leaf = mix(textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, view_a, gx, gy),
                   textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, view_b, gx, gy), view_blend);
        covered = leaf.a >= 0.34;
    }
    // the card's outline and the fade's screen door (with the leaf-volume light the
    // door's phase steps each frame, so the temporal resolve averages it).
// sf:LOD_FADE-
    let door = select(hlod_dither(floor(in.clip.xy)), fract(hlod_dither(floor(in.clip.xy)) + fract(ft.range.w)), light3);
// sf:end
// sf:LOD_FADE+
    // (`--temporal-lod-fade`) the pattern moves every frame, so the temporal
    // anti-aliasing resolves the fade-in to a blend.
    let door = hlod_dither(floor(in.clip.xy) + vec2<f32>(5.588238 * look.post2.w));
// sf:end
    let cut = !covered || (in.fade < 0.999 && door >= in.fade);
    if (!FT_LATE_DISCARD && cut) {
        discard;
    }
    var albedo = in.tint;
    if (baked) {
        let surface = mix(textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, view_a + FT_NORMALS, gx, gy),
                          textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, view_b + FT_NORMALS, gx, gy), view_blend);
        let local = surface.xyz / max(surface.a, 0.01) * 2.0 - 1.0;
        let toward = vec3<f32>(in.right.z, 0.0, -in.right.x);
        n = normalize(in.right * local.x + vec3<f32>(0.0, local.y, 0.0) + toward * local.z + vec3<f32>(0.0, 1e-4, 0.0));
        albedo = srgb_decode(leaf.rgb / max(leaf.a, 0.01));
    } else if (aaa_palm) {
        albedo = aaa_ph.albedo;
    } else if (crown) {
        if (sp == 0u && !SF_AAA_DETAIL) {
            let z = sqrt(max(0.0, 1.0 - min(r * r, 1.0)));
            if (light3) {
                // the crown's sphere seen from the eye (its view from above shows the lit top)
                let up = vec3<f32>(0.0, 1.0, 0.0);
                let upp = normalize(up - v * dot(up, v));
                n = normalize(in.right * in.uv.x + upp * cy + v * z);
            } else {
                n = normalize(in.right * in.uv.x + vec3<f32>(0.0, cy, 0.0) + toward_h * z);
            }
        }
        if (SF_AAA_DETAIL) {
            let base_c = ft_ca_palette(in.seed, in.tint, in.world.xz);
            let cluster = st_noise(in.uv * vec2<f32>(9.5, 11.5) + vec2<f32>(in.seed * 2.3, in.seed * 4.1));
            let sprig = st_noise(in.uv * vec2<f32>(24.0, 28.0) + vec2<f32>(in.seed * 5.7, 1.3));
            let dapple = 0.84 + 0.26 * cluster + 0.14 * (sprig - 0.5);
            // Sunlit outer crown shoots read slightly warmer yellow-olive; deep crevices retain cool green depth
            let depth_tint = mix(vec3<f32>(0.84, 0.92, 0.98), vec3<f32>(1.08, 1.04, 0.92), aaa_open);
            albedo = base_c * dapple * depth_tint;
        } else {
            albedo = in.tint * (0.75 + 0.25 * st_hash(floor(in.world.xz * 0.7) + in.seed));
        }
    } else {
        n = toward_h;
        // bark: the park trees' dark brown; the blue gum's smooth pale trunk (Cal-IPC: peels to a pale trunk; INFERRED albedo)
        albedo = select(vec3<f32>(0.09, 0.07, 0.05), vec3<f32>(0.22, 0.20, 0.16), sp == 3u);
    }
    var s: Surface;
    s.albedo = albedo;
    s.rough = 0.92;
    s.metal = 0.0;
    s.ao = select(
        select(1.0, aaa_ph.ao, aaa_palm),
        select(0.7 + 0.3 * in.uv.y, select(mix(0.72, 1.0, clamp(0.42 * in.uv.y + 0.58 * aaa_open, 0.0, 1.0)), aaa_ph.ao, aaa_palm), SF_AAA_DETAIL && !baked),
        crown
    );
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
    let vol_light = light3 && crown && !baked && !aaa_palm;
    let ng = select(fn_, -l, vol_light);
    var lit = look_light(s, m, v, in.world, ng, 0u);
    if (vol_light) {
        // the leaf volume's light through it: the sun on the far side of the clump,
        // passed by its leaves (a diffuse transmission, FT_VOL_LEAF_T_OVER_R of the
        // reflectance), the term a sphere of opaque leaves leaves out. Shadowed
        // and fogged as the key light, the props' back-lit leaves' form (props_draw.wgsl).
        let through = max(dot(-n, l), 0.0);
        let base = in.world - in.right * (in.uv.x * in.dims.x) - vec3<f32>(0.0, in.uv.y * in.dims.y, 0.0);
        let centre = base + vec3<f32>(0.0, 0.68 * in.dims.y, 0.0);
        let sh = lk_shadow(centre + l * in.dims.x, l) * sky_cloud_transmittance(in.world);
        let vd = dot(in.world - look.cam_pos.xyz, look.cam_fwd.xyz);
        let fog = select(lk_smooth(look.fog.x, look.fog.y, vd), lk_fog_out, SF_AAA_DETAIL);
        let trans = look.key.xyz * s.albedo * FT_VOL_LEAF_T_OVER_R * through * s.ao * sh / LK_PI;
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
    } else if (SF_AAA_DETAIL && crown && !baked && look.sun.y > 0.0) {
        // Warm olive-yellow subsurface sun translucency along the outer sunlit/backlit crown rim.
        let rim = clamp(aaa_open * 0.72 + 0.28 * (1.0 - max(dot(n, v), 0.0)), 0.0, 1.0);
        let through = max(dot(-n, l), 0.0) * 0.45 + pow(max(dot(-v, l), 0.0), 3.0) * 0.55;
        let sh = lk_shadow(in.world + l * 0.5, l) * sky_cloud_transmittance(in.world);
        let sss = look.key.xyz * (s.albedo * vec3<f32>(0.86, 0.96, 0.38)) * (through * rim * 0.46 * sh / LK_PI);
        lit = vec4<f32>(lit.rgb + sss * (1.0 - lk_fog_out), lit.a);
    }
    if (SF_AAA_DETAIL && crown && !baked) {
        // Porous-canopy hemispheric sky-dome + sunlit-urban-plaza bounce fill (`look.sun.y` so
        // night stays dark): lifts building-shadowed & backlit park crowns (`08` Sue Bierman Park,
        // `01`/`07`) from crushed black (`12/255`) to natural layered coastal green (`42..75/255`)
        // while preserving internal sub-bough depth modulation via `aaa_open`.
        let day_f = smoothstep(-0.04, 0.08, look.sun.y);
        if (day_f > 0.001) {
            let env_f = mix(look.ambient.w * look.env_class.x, 1.0, clamp(look.hemi_sky.w, 0.0, 1.0));
            let sky_dome = lk_sh(vec3<f32>(0.0, 1.0, 0.0)) * env_f + look.hemi_sky.xyz * 0.35 + look.ambient.xyz;
            let ghi_f = look.key.xyz * max(look.sun.y, 0.06);
            let urban_ind = sky_dome * 1.8 + ghi_f * 0.55;
            let sky_wrap = urban_ind * (s.albedo * vec3<f32>(0.90, 1.06, 0.74)) * (mix(0.45, 1.05, aaa_open) / LK_PI) * day_f;
            lit = vec4<f32>(lit.rgb + sky_wrap * (1.0 - lk_fog_out), lit.a);
        }
    }
// sf:FOLIAGE+
    if ((crown || baked) && !vol_light) {
        // (`--foliage-light`) the crown passes light: what reaches its far side
        // comes through the leaves toward the eye, most where the view's chord
        // through the crown is short (its edge: the backlit rim), little through its depth.
        lit = vec4<f32>(lit.rgb + ft_foliage(albedo, n, v, in.world), lit.a);
    }
// sf:end
    // FT_LATE_DISCARD (the fallback path, set by hlod_trees.rs): the discards
    // after the lighting. The light reads screen derivatives, which are
    // undefined after a discard on backends where discard ends the invocation
    // (Metal), and the whole dithered band then shaded black.
    if (FT_LATE_DISCARD && cut) {
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
// sf:end
// sf:FOLIAGE&!LEAF_SHADE+
    let t = albedo * vec3<f32>(0.55, 0.85, 0.3);
// sf:end
// sf:FOLIAGE&LEAF_SHADE+
    let t = albedo * vec3<f32>(0.8, 0.95, 0.35); // (`--leaf-shade`) over each primary's band (props_draw.wgsl PR_LEAF_T_OVER_R)
// sf:end
// sf:FOLIAGE+
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
//   Gamma(beta) = omega / (3 pi) (sin beta - beta cos beta) + tau / 3 cos beta
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
// (r2i3) A city tree's crown is open-grown: its leaf area density is about
// 1 m2/m3 (0.6-1.2 for street and park trees, Peper and McPherson 2003), and
// its leaves gather on shoots and branches with gaps between the clumps, so
// the light goes further in (Nilson's clumping index, 0.5-0.8 for broadleaf
// crowns, Chen and Black 1992: 0.6 here, on every path). The leaf
// reflectance is a measured leaf's (LOPEX93 broadleaf and conifer: luminance
// 0.07-0.11; the tint only sets the tree's own shade of it, where it had run
// down to the tints' 0.045: the park pines read black from the air), and the
// sky's light takes the same multiple scattering as the sun's.
const FT_SIGMA: f32 = 0.3; // 1/m: clumping 0.6 x G 0.5 x leaf area density 1.0 m2/m3
// FT_LEAF_LUM (0.07, 0.11): hlod_trees.rs `LEAF_LUM`
// sf:end
// sf:CROWN&!LEAF_SHADE+
const FT_LEAF_T_OVER_R: vec3<f32> = vec3<f32>(0.55, 0.85, 0.3); // props_draw.wgsl PR_LEAF_T_OVER_R
// sf:end
// sf:CROWN&LEAF_SHADE+
const FT_LEAF_T_OVER_R: vec3<f32> = vec3<f32>(0.8, 0.95, 0.35); // (`--leaf-shade`) props_draw.wgsl PR_LEAF_T_OVER_R
// sf:end
// sf:CROWN+
// FT_LEAF_CHROMA, FT_LEAF_PULL: a mature broadleaf leaf's reflectance over its
// luminance, and how far a tint's own chroma is pulled to it (hlod_trees.rs `LEAF_CHROMA`, `LEAF_PULL`)
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
// measured leaf's chroma, its luminance held in a leaf's range
// (FT_LEAF_LUM), each tree a little its own.
fn ft_leaf_r(tint: vec3<f32>, seed: f32) -> vec3<f32> {
    let lum = max(dot(tint, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4);
    let c = mix(tint / lum, FT_LEAF_CHROMA, FT_LEAF_PULL);
    return c * clamp(lum, FT_LEAF_LUM.x, FT_LEAF_LUM.y) * (0.85 + 0.3 * fract(seed * 0.6180339));
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
    // (r2i3: tau / 3, Shultis and Myneni's; it read tau / pi, a few per cent low forward)
    let gamma = max(omega / (3.0 * LK_PI) * (sin(beta) - beta * cb) + tau / 3.0 * cb, vec3<f32>(0.0));
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
// (r2i3) a Canary Island date palm carries 50-80 live fronds; drawn as 24
// (each a few of them overlapping at this distance), their leaflets' spread
// 0.17 of the reach (leaflets 40-45 cm each side of a 5 m rachis), round a
// heart dense with the fronds' bases (13 thin fronds read as a sapling's).
const FT_PALM_FRONDS: u32 = 24u;
const FT_PALM_SPREAD: f32 = 0.17;
const FT_PALM_HEART: f32 = 0.2;

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
// foot, and FT_PALM_FRONDS fronds from the heart, each a feather that rises and bends
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
        let w = max(FT_PALM_SPREAD * sin(3.14159 * clamp(f * 0.9 + 0.06, 0.0, 1.0)) * (0.45 + 0.55 * f), w_min);
        let d = length(q - p1) / w;
        if (d < best) {
            best = d;
            bt = normalize(dir - vec2<f32>(0.0, 1.1 * s1));
            bh = hs;
            hang = smoothstep(0.0, -0.5, dir.y);
        }
    }
    o.frond = best < 1.0 || length(q * vec2<f32>(1.0, 1.6)) < FT_PALM_HEART;
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
    var aaa_c = vec4<f32>(0.0, 1.0, 0.0, -1.0);
    let baked = in.view.x >= 0.0;
    let v = normalize(frame.cam_pos.xyz - in.world);
    let toward = normalize(vec3<f32>(v.x, 0.0, v.z) + vec3<f32>(1e-5, 0.0, 0.0));
    let aaa_palm2 = ft_is_aaa_palm(in.tint, u32(in.dims.z + 0.5), baked);
    var aaa_ph2: FtAaaPalm;
    if (aaa_palm2) {
        aaa_ph2 = ft_aaa_palm(in.uv, in.dims.xy, in.seed, in.right, toward, max(length(dpx), length(dpy)));
        crown = aaa_ph2.frond;
        trunk = aaa_ph2.trunk;
    } else if (SF_AAA_DETAIL) {
        aaa_c = ft_aaa_crown(in.uv, in.seed);
        crown = aaa_c.w >= 0.0;
        let tw = mix(0.056, 0.030, clamp(in.uv.y / 0.48, 0.0, 1.0));
        let fork_t = clamp((in.uv.y - 0.30) / 0.24, 0.0, 1.0);
        let limb_l = in.uv.y >= 0.30 && in.uv.y < 0.54 && abs(in.uv.x + fork_t * 0.22) < mix(0.028, 0.014, fork_t);
        let limb_r = in.uv.y >= 0.32 && in.uv.y < 0.56 && abs(in.uv.x - fork_t * 0.20) < mix(0.026, 0.013, fork_t);
        trunk = (in.uv.y < 0.46 && abs(in.uv.x) < tw) || limb_l || limb_r;
    }
    var leaf = vec4<f32>(0.0);
    let a = max(i32(in.view.x), 0);
    let b = max(i32(in.view.y), 0);
    let blend = smoothstep(0.0, 1.0, in.view.z);
    var covered = crown || trunk;
    if (baked) {
        leaf = mix(textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, a, gx, gy),
                   textureSampleGrad(ft_canopy, kit_sampler, atlas_uv, b, gx, gy), blend);
        covered = leaf.a >= 0.34;
    }
    var palm = aaa_palm2;
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
    } else if (aaa_palm2) {
        n = aaa_ph2.n;
        albedo = aaa_ph2.albedo;
        ao = aaa_ph2.ao;
    } else if (crown) {
        if (SF_AAA_DETAIL) {
            let up = vec3<f32>(0.0, 1.0, 0.0);
            let upp = normalize(up - toward * dot(up, toward));
            n = normalize(in.right * aaa_c.x + upp * aaa_c.y + toward * aaa_c.z);
            let cluster = st_noise(in.uv * vec2<f32>(9.5, 11.5) + vec2<f32>(in.seed * 2.3, in.seed * 4.1));
            albedo = ft_ca_palette(in.seed, in.tint, in.world.xz) * (0.86 + 0.26 * cluster);
            ao = mix(0.72, 1.0, clamp(0.42 * in.uv.y + 0.58 * max(aaa_c.w, 0.0), 0.0, 1.0));
        } else {
            let z = sqrt(max(0.0, 1.0 - min(r * r, 1.0)));
            n = normalize(in.right * in.uv.x + vec3<f32>(0.0, cy, 0.0) + toward * z);
            albedo = in.tint * (0.75 + 0.25 * st_hash(floor(in.world.xz * 0.7) + in.seed));
        }
    } else {
        n = toward;
        albedo = vec3<f32>(0.09, 0.07, 0.05);
    }
    var vol = false;
    var sun = vec3<f32>(0.0);
// sf:end
// sf:FAR_PALMS+
    if (u32(in.size.z + 0.5) == FT_PALM && !baked) {
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
            let rho = select(ft_leaf_r(in.tint, in.seed), ft_ca_palette(in.seed, in.tint, in.world.xz) * 1.15, SF_AAA_DETAIL);
            let cv = ft_crown(in.uv, in.size.xy, in.right, v, rho);
            let om = rho * (vec3<f32>(1.0) + FT_LEAF_T_OVER_R);
            albedo = 0.5 * om * (vec3<f32>(1.0) + 0.5 * om / (vec3<f32>(1.0) - om)); // omega / 2, and its multiple scattering
            ao = select(cv.w, cv.w * mix(0.75, 1.08, max(aaa_c.w, 0.0)), SF_AAA_DETAIL);
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
    if (SF_AAA_DETAIL && crown && !baked) {
        let day_f = smoothstep(-0.04, 0.08, look.sun.y);
        if (day_f > 0.001) {
            let open2 = select(max(aaa_c.w, 0.0), aaa_ph2.open, aaa_palm2);
            let env_f = mix(look.ambient.w * look.env_class.x, 1.0, clamp(look.hemi_sky.w, 0.0, 1.0));
            let sky_dome = lk_sh(vec3<f32>(0.0, 1.0, 0.0)) * env_f + look.hemi_sky.xyz * 0.35 + look.ambient.xyz;
            let ghi_f = look.key.xyz * max(look.sun.y, 0.06);
            let urban_ind = sky_dome * 1.8 + ghi_f * 0.55;
            let sky_wrap = urban_ind * (s.albedo * vec3<f32>(0.90, 1.06, 0.74)) * (mix(0.45, 1.05, open2) / LK_PI) * day_f;
            lit = vec4<f32>(lit.rgb + sky_wrap * (1.0 - lk_fog_out), lit.a);
        }
    }
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
