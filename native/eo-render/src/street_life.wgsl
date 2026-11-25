// Stage W2b: street life (street_life.rs; `--street-life 1|2`, off by default).
// Litter, butts, leaves and grit on the walks and in the gutters; graffiti,
// buffed patches, posters and (level 2) stickers on ground-floor walls and
// freeway columns; (level 2) utility locate marks. Albedo, roughness and a
// little relief, in the material path of the surface they lie on: the street
// (cc0.wgsl `cc0_street_decals` -> `sl_street_decals`), the kit buildings
// (draw.wgsl after `fa_apply` -> `sl_kit_wall`, glass excepted), the
// structures' concrete (draw.wgsl kind 12 ->
// `sl_structure`). Every function runs only behind `sl_level() != 0u` at its
// call site, so with the level 0 none of it is reached and the frame is as
// before. Constants SL_* come from street_life.rs. Nothing here draws a
// letter, a word, a name or a mark: tags are meandering strokes with curls
// and hooks on no alphabet, their glyphs overlapping; throw-ups are one cloud
// of merged bubbles; pieces one notched band; posters colour fields, bars
// and dots. Stage W2c made it read at normal and mid distance: bold tags in
// two bands, throw-ups with shadows and drips, pieces, column bases covered,
// litter clusters with dark halos (sl_clusters), heaps at the walls' foot.
// Stage W2d put the walk's litter where it collects (sl_walk_zones: the kerb's
// lip and the furnishing zone, the building line and doorways, the trees'
// basins, the bins and the stops), in clumps, the through zone nearly clean,
// and faded the prints (sl_faded).

// The street fragment's distance from the kerb line (m; draw.wgsl
// `street_surface` sets it before the decals, with the level on).
var<private> sl_kd: f32;

// SF_STREET_LIFE false (look.rs `ShaderFeatures`: off at start) compiles every term out.
fn sl_level() -> u32 {
    if (!SF_STREET_LIFE) {
        return 0u;
    }
    return (u32(frame.copy_step.y) >> SL_RUN_SHIFT) & 3u;
}

// 1 while a detail of `size` metres spans a few pixels, 0 once it is under one.
fn sl_keep(size: f32, fw: f32) -> f32 {
    return 1.0 - smoothstep(size * 0.25, size * 0.8, fw);
}

// A signed distance's cover over the pixel footprint (`soft`: at least this soft).
fn sl_cov(d: f32, soft: f32) -> f32 {
    return 1.0 - smoothstep(-0.5 * soft, 0.5 * soft, d);
}

fn sl_rot(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x + s * p.y, -s * p.x + c * p.y);
}

fn sl_seg(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-8), 0.0, 1.0);
    return length(p - a - ab * t);
}

// Distance to the quadratic Bezier a-c-b, as four chords.
fn sl_qbez(p: vec2<f32>, a: vec2<f32>, c: vec2<f32>, b: vec2<f32>) -> f32 {
    var d = 1e9;
    var prev = a;
    for (var i = 1; i <= 4; i = i + 1) {
        let t = f32(i) * 0.25;
        let q = mix(mix(a, c, t), mix(c, b, t), t);
        d = min(d, sl_seg(p, prev, q));
        prev = q;
    }
    return d;
}

fn sl_smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// One painted or dropped thing over the surface: linear colour, cover,
// roughness, metalness, a tilt of the normal in the ground plane (world x, z).
struct SlPaint {
    c: vec3<f32>,
    a: f32,
    rough: f32,
    metal: f32,
    tn: vec2<f32>,
};

fn sl_none() -> SlPaint {
    var o: SlPaint;
    o.c = vec3<f32>(0.0);
    o.a = 0.0;
    o.rough = 0.8;
    o.metal = 0.0;
    o.tn = vec2<f32>(0.0);
    return o;
}

// `src` over `dst`.
fn sl_over(dst: SlPaint, src: SlPaint) -> SlPaint {
    if (src.a <= 0.0) {
        return dst;
    }
    var o: SlPaint;
    o.a = src.a + dst.a * (1.0 - src.a);
    let k = src.a / max(o.a, 1e-5);
    o.c = mix(dst.c, src.c, k);
    o.rough = mix(dst.rough, src.rough, k);
    o.metal = mix(dst.metal, src.metal, k);
    o.tn = mix(dst.tn, src.tn, k);
    return o;
}

// Into the street decals' post terms (cc0_street_post), over what is there.
fn sl_put(p: SlPaint) {
    if (p.a <= 0.0) {
        return;
    }
    let w0 = cc0_p_alb.w;
    let w = p.a + w0 * (1.0 - p.a);
    let k = p.a / max(w, 1e-5);
    cc0_p_alb = vec4<f32>(mix(cc0_p_alb.rgb, p.c, k), w);
    cc0_p_rough = vec2<f32>(select(p.rough, mix(cc0_p_rough.x, p.rough, k), cc0_p_rough.y > 0.0), max(cc0_p_rough.y, p.a));
    cc0_p_metal = max(cc0_p_metal, p.metal * p.a);
    let nk = length(p.tn) * p.a;
    if (nk > 0.0) {
        cc0_p_n = vec3<f32>(mix(cc0_p_n.xy, p.tn, k), max(cc0_p_n.z, min(1.0, nk * 2.0)));
    }
}

// ---- the litter scatter ------------------------------------------------------
//
// Jittered cells keyed on the world, one item a cell at most, inside its cell
// (no neighbour lookups), present with the stretch's probability. An item is
// drawn only by the decal whose own stretch (`own`: its centre and axis,
// `oh`: its half length without the overlap) holds the item's centre.

const SL_BIG_CELL: f32 = 0.42;
const SL_SMALL_CELL: f32 = 0.12;
const SL_COARSE_CELL: f32 = 1.5;
// Mean area (m^2) and linear colour of a paper-size item, a small one and a coarse one,
// the scatter's cover once its items are under a pixel.
const SL_BIG_AREA: f32 = 0.0055;
const SL_BIG_MEAN: vec3<f32> = vec3<f32>(0.50, 0.46, 0.40);
const SL_LEAF_AREA: f32 = 0.0014;
const SL_LEAF_MEAN: vec3<f32> = vec3<f32>(0.25, 0.16, 0.065);
const SL_BUTT_AREA: f32 = 0.0002;
const SL_BUTT_MEAN: vec3<f32> = vec3<f32>(0.58, 0.47, 0.33);
const SL_COARSE_AREA: f32 = 0.09;
const SL_COARSE_MEAN: vec3<f32> = vec3<f32>(0.32, 0.25, 0.17);
// Grit and sand as it dries in SF gutters and against walls (a light warm grey).
const SL_SAND: vec3<f32> = vec3<f32>(0.31, 0.285, 0.235);
// Decayed leaf litter packed into the kerb corner.
const SL_MULCH: vec3<f32> = vec3<f32>(0.085, 0.06, 0.035);

fn sl_owns(c: vec2<f32>, own: vec4<f32>, oh: f32) -> bool {
    return abs(dot(c - own.xy, own.zw)) <= oh;
}

// Stage W2d: a print as it lies on the street weeks on: sun-faded toward its
// own grey and filmed with dust (`k` 0..1). Fresh saturated prints stood out
// on the walk more than real litter does.
fn sl_faded(c: vec3<f32>, k: f32) -> vec3<f32> {
    let l = dot(c, SL_LUMA);
    return mix(c, vec3<f32>(l) * vec3<f32>(1.04, 1.0, 0.9), k) * (1.0 - 0.18 * k) + vec3<f32>(0.035, 0.032, 0.027) * k;
}

// A paper-size item (scraps, receipts, cups, wrappers, caps, napkins, bottles).
fn sl_big(p: vec2<f32>, fw: f32, prob: f32, deck: bool, own: vec4<f32>, oh: f32) -> SlPaint {
    var o = sl_none();
    let cs = SL_BIG_CELL;
    let keep = sl_keep(0.07, fw);
    let mean = clamp(prob, 0.0, 1.0) * SL_BIG_AREA / (cs * cs);
    o.c = SL_BIG_MEAN;
    o.a = mean * (1.0 - keep);
    if (keep <= 0.0) {
        return o;
    }
    let cell = floor(p / cs);
    if (st_hash(cell + vec2<f32>(3.17, 9.71)) >= prob) {
        return o;
    }
    let rmax = 0.13;
    let j = vec2<f32>(st_hash(cell + vec2<f32>(7.3, 1.9)), st_hash(cell + vec2<f32>(2.2, 8.8)));
    let ctr = (cell + 0.5) * cs + (j - 0.5) * (cs - 2.0 * rmax);
    if (!sl_owns(ctr, own, oh)) {
        return o;
    }
    let ang = st_hash(cell + vec2<f32>(5.5, 4.4)) * 6.2832;
    let q = sl_rot(p - ctr, ang);
    let hk = st_hash(cell + vec2<f32>(9.1, 6.6));
    let hs = st_hash(cell + vec2<f32>(1.4, 3.3));
    let hc = st_hash(cell + vec2<f32>(8.2, 2.7));
    var it = sl_none();
    var d = 1.0;
    // the item's own relief in its frame (turned back to the world below)
    var tl = vec2<f32>(0.0);
    let soft = max(fw, 0.0015);
    if (hk < 0.30) {
        // a paper scrap: torn, crumpled, trodden on; white, newsprint, brown or a pastel flyer
        let hx = 0.045 + 0.055 * hs;
        let hy = 0.03 + 0.045 * hc;
        let edge = (st_noise(q * 70.0 + hs * 31.0) - 0.5) * 0.01 + (st_noise(q * 17.0 + hc * 7.0) - 0.5) * 0.014;
        d = max(abs(q.x) - hx, abs(q.y) - hy) + edge;
        var c = vec3<f32>(0.53, 0.515, 0.48);
        if (hc > 0.84) {
            c = sl_faded(select(select(vec3<f32>(0.52, 0.36, 0.37), vec3<f32>(0.52, 0.48, 0.27), hs > 0.4), vec3<f32>(0.31, 0.41, 0.51), hs > 0.75), 0.3);
        } else if (hc > 0.66) {
            c = vec3<f32>(0.30, 0.21, 0.12);
        } else if (hc > 0.46) {
            c = vec3<f32>(0.40, 0.39, 0.36);
        }
        // a fold across it, dirt from shoes
        let fold = 1.0 - 0.18 * (1.0 - smoothstep(0.0, 0.0025 + soft, abs(dot(q, normalize(vec2<f32>(hs - 0.5, 0.6))) - 0.01)));
        it.c = c * fold * (0.62 + 0.38 * st_noise(q * 26.0 + hc * 9.0));
        it.rough = 0.92;
        tl = (vec2<f32>(st_noise(q * 55.0 + 1.7), st_noise(q * 55.0 + 9.3)) - 0.5) * 0.9;
    } else if (hk < 0.42) {
        // a till receipt: a narrow curled strip of thermal paper
        let hx = 0.028 + 0.008 * hs;
        let hy = 0.07 + 0.045 * hc;
        d = max(abs(q.x) - hx, abs(q.y) - hy) + (st_noise(q * 90.0) - 0.5) * 0.004;
        it.c = vec3<f32>(0.6, 0.595, 0.565) * (0.78 + 0.22 * st_noise(q * 30.0));
        it.rough = 0.55;
        tl = vec2<f32>(sin(q.x / hx * 1.5708) * 0.55, 0.0);
    } else if (hk < 0.55) {
        // a cup on its side: a tapered cylinder, its mouth at one end
        let hx = 0.048 + 0.016 * hs;
        let t = clamp(q.x / hx * 0.5 + 0.5, 0.0, 1.0);
        let r = mix(0.026, 0.04, t);
        d = max(abs(q.x) - hx, abs(q.y) - r);
        var c = vec3<f32>(0.66, 0.65, 0.62);
        var rough = 0.5;
        if (hc < 0.38) {
            // a kraft coffee sleeve round a white cup
            c = select(c, vec3<f32>(0.33, 0.22, 0.12), abs(q.x + hx * 0.15) < hx * 0.35);
        } else if (hc < 0.46) {
            c = sl_faded(vec3<f32>(0.40, 0.035, 0.03), 0.35); // a red party cup, scuffed
            rough = 0.35;
        } else if (hc < 0.62) {
            c = vec3<f32>(0.42, 0.44, 0.45); // clear plastic
            rough = 0.12;
        }
        let mouth = 1.0 - smoothstep(0.003, 0.003 + soft, abs(q.x - hx + 0.004));
        it.c = c * (1.0 - 0.75 * mouth);
        it.rough = rough;
        tl = vec2<f32>(0.0, clamp(q.y / r, -1.0, 1.0) * 0.75);
    } else if (hk < 0.68) {
        // a snack wrapper: crimped ends, a print sun-faded and dusty (stage W2d: the
        // fresh saturated squares stood out), foil or clear film for some; crinkled
        let hx = 0.03 + 0.025 * hs;
        let hy = 0.02 + 0.016 * hc;
        let crimp = abs(fract(q.y * 160.0) - 0.5) * 0.004;
        d = max(abs(q.x) - hx + crimp, abs(q.y) - hy);
        let pick = fract(hc * 7.31);
        var c = vec3<f32>(0.30, 0.21, 0.12); // a kraft or brown print
        if (pick > 0.78) {
            c = vec3<f32>(0.50, 0.50, 0.50); // foil-lined, the print worn off
            it.metal = 0.55;
        } else if (pick > 0.62) {
            c = vec3<f32>(0.48, 0.48, 0.46); // clear or white film
        } else if (pick > 0.5) {
            c = sl_faded(vec3<f32>(0.50, 0.04, 0.03), 0.55);
        } else if (pick > 0.4) {
            c = sl_faded(vec3<f32>(0.62, 0.48, 0.04), 0.5);
        } else if (pick > 0.3) {
            c = sl_faded(vec3<f32>(0.03, 0.11, 0.42), 0.55);
        } else if (pick > 0.2) {
            c = sl_faded(vec3<f32>(0.06, 0.33, 0.08), 0.6);
        } else if (pick > 0.1) {
            c = sl_faded(vec3<f32>(0.68, 0.2, 0.02), 0.55);
        }
        // a lighter band across the print (no lettering), the crumples' shading
        let band = 1.0 - smoothstep(0.004, 0.004 + soft, abs(q.x - hx * (hs - 0.5)) - hx * 0.18);
        it.c = mix(c, min(c * 1.5 + 0.08, vec3<f32>(0.7)), band * 0.35) * (0.78 + 0.22 * st_noise(q * 90.0 + hc * 3.0));
        it.rough = 0.38;
        tl = (vec2<f32>(st_noise(q * 120.0 + 4.1), st_noise(q * 120.0 + 7.7)) - 0.5) * 1.3;
    } else if (hk < 0.76) {
        // a bottle cap
        let r = 0.0125 + 0.003 * hs;
        d = length(q) - r;
        let rim = 1.0 - smoothstep(0.002, 0.002 + soft, abs(length(q) - r + 0.002));
        it.c = select(vec3<f32>(0.42, 0.40, 0.37), sl_faded(select(vec3<f32>(0.35, 0.03, 0.03), vec3<f32>(0.03, 0.08, 0.3), hc > 0.7), 0.4), hc > 0.45) * (1.0 - 0.3 * rim);
        it.metal = select(0.0, 0.6, hc <= 0.45);
        it.rough = 0.4;
    } else if (hk < 0.9 || !deck) {
        // a crumpled napkin
        let r = 0.024 + 0.018 * hs;
        let th = atan2(q.y, q.x);
        d = length(q) - r * (1.0 + 0.28 * (st_noise(vec2<f32>(th * 2.0 + 3.0, hs * 9.0)) - 0.5));
        it.c = vec3<f32>(0.66, 0.65, 0.62) * (0.68 + 0.32 * st_noise(q * 140.0 + hc * 5.0));
        it.rough = 0.95;
        tl = (vec2<f32>(st_noise(q * 150.0 + 2.3), st_noise(q * 150.0 + 5.9)) - 0.5) * 1.4 + q / max(r, 1e-3) * 0.5;
    } else {
        // a bottle on its side (under the decks): brown or green glass, a glint along it
        let hx = 0.1;
        let r = select(0.03, 0.013, q.x > 0.035);
        d = max(abs(q.x) - hx, abs(q.y) - r);
        it.c = select(vec3<f32>(0.03, 0.10, 0.035), vec3<f32>(0.09, 0.045, 0.015), hc > 0.5);
        it.rough = 0.08;
        tl = vec2<f32>(0.0, clamp(q.y / r, -1.0, 1.0) * 0.9);
    }
    // the item and its contact shadow just outside it (a crumpled thing stands off the ground)
    let ic = sl_cov(d, soft);
    let sh = (1.0 - smoothstep(0.0, 0.006 + soft, max(d, 0.0))) * (1.0 - ic) * 0.22;
    it.a = min(ic + sh, 1.0);
    it.c = (it.c * ic + vec3<f32>(0.02) * sh) / max(it.a, 1e-5);
    it.tn = sl_rot(tl, -ang) * ic;
    o.c = mix(SL_BIG_MEAN, it.c, keep);
    o.a = mix(o.a, it.a, keep);
    o.rough = it.rough;
    o.metal = it.metal * keep;
    o.tn = it.tn * keep;
    return o;
}

// The small scatter: butts, leaves, torn bits of paper and plastic, glass
// crumbs, grit. `pl`, `pb`, `pg`: the cell's chance of a leaf, a butt or a
// bit (`pf` of them a torn bit), a glass crumb or a grit speck.
fn sl_small(p: vec2<f32>, fw: f32, pl: f32, pb: f32, pg: f32, pf: f32, own: vec4<f32>, oh: f32) -> SlPaint {
    var o = sl_none();
    let cs = SL_SMALL_CELL;
    let keep = sl_keep(0.03, fw);
    let m_area = (pl * SL_LEAF_AREA + pb * SL_BUTT_AREA) / (cs * cs);
    o.c = (pl * SL_LEAF_AREA * SL_LEAF_MEAN + pb * SL_BUTT_AREA * SL_BUTT_MEAN) / max(pl * SL_LEAF_AREA + pb * SL_BUTT_AREA, 1e-6);
    o.a = m_area * (1.0 - keep);
    o.rough = 0.8;
    if (keep <= 0.0) {
        return o;
    }
    let cell = floor(p / cs);
    let h0 = st_hash(cell + vec2<f32>(4.7, 2.9));
    if (h0 >= pl + pb + pg) {
        return o;
    }
    let rmax = 0.05;
    let j = vec2<f32>(st_hash(cell + vec2<f32>(1.1, 6.3)), st_hash(cell + vec2<f32>(8.4, 3.6)));
    let ctr = (cell + 0.5) * cs + (j - 0.5) * (cs - 2.0 * rmax);
    if (!sl_owns(ctr, own, oh)) {
        return o;
    }
    let ang = st_hash(cell + vec2<f32>(6.2, 7.9)) * 6.2832;
    let q = sl_rot(p - ctr, ang);
    let hs = st_hash(cell + vec2<f32>(2.6, 5.1));
    let hc = st_hash(cell + vec2<f32>(9.8, 0.7));
    let soft = max(fw, 0.0008);
    var it = sl_none();
    var d = 1.0;
    var tl = vec2<f32>(0.0);
    if (h0 < pl) {
        // a dead leaf: an oval or a lobed (plane tree) leaf, its midvein, its edges curled up
        let hx = 0.022 + 0.026 * hs;
        if (hc < 0.15) {
            // a plane-tree leaf: a broad blade with five soft lobes
            let r = length(q);
            let th = atan2(q.y, q.x);
            let lobe = cos(th * 2.5);
            d = r - hx * (0.72 + 0.28 * lobe * lobe) * (0.92 + 0.16 * st_noise(vec2<f32>(th * 3.0, hs * 9.0)));
        } else {
            let hy = hx * (0.35 + 0.25 * hc);
            let xn = clamp(q.x / hx, -1.0, 1.0);
            d = max(abs(q.y) - hy * (1.0 - xn * xn), abs(q.x) - hx);
        }
        // its stalk
        d = min(d, max(abs(q.y) - 0.0009, abs(q.x + hx + 0.008) - 0.008));
        // dead brown most, tan, ochre, rust, a few still dull green, some decayed grey-brown
        let pick = fract(hc * 13.7);
        var c = vec3<f32>(0.16, 0.095, 0.042);
        if (pick > 0.92) {
            c = vec3<f32>(0.12, 0.10, 0.08);
        } else if (pick > 0.82) {
            c = vec3<f32>(0.10, 0.12, 0.05);
        } else if (pick > 0.72) {
            c = vec3<f32>(0.25, 0.10, 0.04);
        } else if (pick > 0.58) {
            c = vec3<f32>(0.34, 0.24, 0.075);
        } else if (pick > 0.35) {
            c = vec3<f32>(0.27, 0.185, 0.09);
        }
        let vein = 1.0 - smoothstep(0.0007, 0.0007 + soft, abs(q.y));
        // trodden flat and darkened in places, spotted with decay
        it.c = c * (1.0 - 0.3 * vein) * (0.7 + 0.3 * st_noise(q * 200.0 + hs * 3.0)) * (1.0 - 0.35 * smoothstep(0.62, 0.8, st_noise(q * 90.0 + hc * 7.0)));
        it.rough = 0.72;
        tl = vec2<f32>(0.0, clamp(q.y / (hx * 0.5), -1.0, 1.0) * 0.6);
    } else if (h0 < pl + pb && hs < pf) {
        // a torn bit of paper, a flake of plastic, a sweet wrapper's corner
        let hx = 0.01 + 0.016 * hc;
        let hy = hx * (0.4 + 0.6 * fract(hs * 7.7));
        d = max(abs(q.x) - hx, abs(q.y) - hy) + (st_noise(q * 300.0 + hc * 5.0) - 0.5) * 0.004;
        // mostly dirty, trodden paper (off-white, grey, brown), now and then a dull plastic
        // flake or a clear one
        let pick = fract(hc * 5.3);
        var fc = vec3<f32>(0.44, 0.43, 0.40);
        if (pick > 0.93) {
            fc = mix(sl_spray(0.6 + 0.4 * fract(hc * 11.0)).rgb, vec3<f32>(0.3), 0.55);
        } else if (pick > 0.85) {
            fc = vec3<f32>(0.36, 0.38, 0.38);
        } else if (pick > 0.7) {
            fc = vec3<f32>(0.28, 0.2, 0.12);
        } else if (pick > 0.4) {
            fc = vec3<f32>(0.4, 0.39, 0.37);
        }
        it.c = fc * (0.8 + 0.2 * st_noise(q * 400.0));
        it.rough = select(0.85, 0.2, pick > 0.85);
    } else if (h0 < pl + pb) {
        // a cigarette butt: the filter (tan or white), the paper, the burnt end
        let hx = 0.011 + 0.003 * hs;
        let r = 0.0038;
        d = length(vec2<f32>(max(abs(q.x) - hx, 0.0), q.y)) - r;
        let filt = select(vec3<f32>(0.58, 0.40, 0.19), vec3<f32>(0.64, 0.63, 0.6), hc > 0.7) * (0.85 + 0.15 * st_noise(q * 900.0));
        var c = select(vec3<f32>(0.66, 0.655, 0.63), filt, q.x < hx * 0.1);
        c = select(c, vec3<f32>(0.07, 0.065, 0.06), q.x > hx * 0.85);
        it.c = c;
        it.rough = 0.85;
        tl = vec2<f32>(0.0, clamp(q.y / r, -1.0, 1.0) * 0.7);
    } else {
        // tempered glass crumbs from a car break-in (in clusters), or a grit speck
        let r = 0.003 + 0.003 * hs;
        d = max(abs(q.x), abs(q.y)) - r;
        if (hc < 0.6) {
            it.c = vec3<f32>(0.24, 0.30, 0.29);
            it.rough = 0.06;
            tl = (vec2<f32>(hs, hc) - 0.5) * 1.6;
        } else {
            it.c = vec3<f32>(0.28, 0.26, 0.23) * (0.7 + 0.6 * hs);
            it.rough = 0.9;
        }
    }
    it.a = sl_cov(d, soft);
    o.c = mix(o.c, it.c, keep);
    o.a = mix(o.a, it.a, keep);
    o.rough = it.rough;
    o.tn = sl_rot(tl, -ang) * it.a * keep;
    return o;
}

// The coarse scatter: flattened cardboard, a newspaper sheet, a plastic bag, a clamshell.
fn sl_coarse(p: vec2<f32>, fw: f32, prob: f32, own: vec4<f32>, oh: f32) -> SlPaint {
    var o = sl_none();
    let cs = SL_COARSE_CELL;
    let keep = sl_keep(0.3, fw);
    o.c = SL_COARSE_MEAN;
    o.a = clamp(prob, 0.0, 1.0) * SL_COARSE_AREA / (cs * cs) * (1.0 - keep);
    if (keep <= 0.0) {
        return o;
    }
    let cell = floor(p / cs);
    if (st_hash(cell + vec2<f32>(6.6, 1.3)) >= prob) {
        return o;
    }
    let rmax = 0.36;
    let j = vec2<f32>(st_hash(cell + vec2<f32>(3.9, 4.2)), st_hash(cell + vec2<f32>(0.6, 9.4)));
    let ctr = (cell + 0.5) * cs + (j - 0.5) * (cs - 2.0 * rmax);
    if (!sl_owns(ctr, own, oh)) {
        return o;
    }
    let ang = st_hash(cell + vec2<f32>(2.8, 8.1)) * 6.2832;
    let q = sl_rot(p - ctr, ang);
    let hk = st_hash(cell + vec2<f32>(7.7, 5.2));
    let hs = st_hash(cell + vec2<f32>(4.4, 0.9));
    let soft = max(fw, 0.002);
    var it = sl_none();
    var d = 1.0;
    var tl = vec2<f32>(0.0);
    if (hk < 0.45) {
        // flattened cardboard: corrugated, a fold line, wet patches
        let hx = 0.2 + 0.13 * hs;
        let hy = 0.14 + 0.1 * fract(hs * 7.3);
        d = max(abs(q.x) - hx, abs(q.y) - hy) + (st_noise(q * 40.0) - 0.5) * 0.008;
        let fold = 1.0 - 0.22 * (1.0 - smoothstep(0.003, 0.003 + soft, abs(q.x - hx * 0.3)));
        let wet = smoothstep(0.55, 0.75, st_noise(q * 6.0 + hs * 11.0));
        it.c = vec3<f32>(0.34, 0.255, 0.16) * fold * (1.0 - 0.35 * wet) * (0.9 + 0.1 * st_noise(vec2<f32>(q.x * 300.0, q.y * 6.0)));
        it.rough = 0.92;
        tl = vec2<f32>(sign(q.x - hx * 0.3) * 0.12, 0.0);
    } else if (hk < 0.7) {
        // a newspaper sheet: grey blocks of print and a darker picture block, no type
        let hx = 0.17 + 0.08 * hs;
        let hy = 0.13 + 0.05 * hs;
        d = max(abs(q.x) - hx, abs(q.y) - hy) + (st_noise(q * 60.0) - 0.5) * 0.006;
        let u = q / vec2<f32>(hx, hy);
        let col = abs(fract(u.x * 2.5 + 0.5) - 0.5) > 0.06;
        let pic = abs(u.x - 0.35) < 0.4 && abs(u.y + 0.2) < 0.45;
        var c = vec3<f32>(0.5, 0.49, 0.46);
        c = select(c, c * 0.82, col && abs(u.y) < 0.85);
        c = select(c, vec3<f32>(0.22, 0.21, 0.2) * (0.7 + 0.6 * st_noise(q * 25.0)), pic);
        it.c = c;
        it.rough = 0.9;
        tl = (vec2<f32>(st_noise(q * 18.0), st_noise(q * 18.0 + 3.3)) - 0.5) * 0.5;
    } else if (hk < 0.9) {
        // a plastic bag, flattened: white and thin, or black
        let r = 0.12 + 0.1 * hs;
        let th = atan2(q.y, q.x);
        d = length(q) - r * (1.0 + 0.35 * (st_noise(vec2<f32>(th * 1.6 + 2.0, hs * 13.0)) - 0.5));
        let black = fract(hs * 5.1) > 0.7;
        it.c = select(vec3<f32>(0.56, 0.56, 0.55), vec3<f32>(0.025, 0.025, 0.028), black) * (0.85 + 0.15 * st_noise(q * 70.0));
        it.rough = 0.28;
        tl = (vec2<f32>(st_noise(q * 45.0 + 1.0), st_noise(q * 45.0 + 6.0)) - 0.5) * 1.2;
        it.a = select(0.7, 1.0, black);
    } else {
        // a takeaway clamshell
        let h = 0.09 + 0.02 * hs;
        d = length(max(abs(q) - vec2<f32>(h - 0.015), vec2<f32>(0.0))) - 0.015;
        it.c = vec3<f32>(0.70, 0.70, 0.67) * (1.0 - 0.15 * (1.0 - smoothstep(0.002, 0.002 + soft, abs(q.y))));
        it.rough = 0.55;
        tl = q / h * 0.3;
    }
    let a0 = select(1.0, it.a, it.a > 0.0);
    let ic = sl_cov(d, soft) * a0;
    let sh = (1.0 - smoothstep(0.0, 0.01 + soft, max(d, 0.0))) * (1.0 - sl_cov(d, soft)) * 0.25;
    it.a = min(ic + sh, 1.0);
    it.c = (it.c * ic + vec3<f32>(0.02) * sh) / max(it.a, 1e-5);
    o.c = mix(SL_COARSE_MEAN, it.c, keep);
    o.a = mix(o.a, it.a, keep);
    o.rough = it.rough;
    o.tn = sl_rot(tl, -ang) * it.a * keep;
    return o;
}

// The utility locate marks (level 2): a contractor's spray line and arrow
// beside the kerb, in the APWA colour of the utility (orange telecom, blue
// water, red power, yellow gas, green sewer, white proposed, pink survey).
// `kd`: across the kerb line, + into the walk, - into the gutter.
fn sl_locate(along: f32, kd: f32, oh: f32, seed: f32, fw: f32) -> SlPaint {
    var o = sl_none();
    let h = fract(seed * 13.37);
    if (h > 0.1 || kd < -1.0) {
        return o;
    }
    let len = 0.4 + 0.8 * fract(seed * 7.1);
    let x0 = (fract(seed * 3.3) * 2.0 - 1.0) * max(oh - len - 0.3, 0.0);
    let y0 = select(-0.25, 0.35 + 0.5 * fract(seed * 5.7), fract(seed * 9.1) > 0.4); // in the gutter or on the walk
    let w = 0.017;
    let t = along - x0;
    let dl = max(abs(kd - y0) - w, max(-t, t - len));
    // the arrow head at the far end, or a tick across
    let ah = 0.07;
    let tip = vec2<f32>(t - len, kd - y0);
    let arrow = min(sl_seg(tip, vec2<f32>(0.0, 0.0), vec2<f32>(-ah, ah)), sl_seg(tip, vec2<f32>(0.0, 0.0), vec2<f32>(-ah, -ah))) - w;
    let tick = max(abs(t) - w, abs(kd - y0) - 0.08);
    let d = min(dl, select(tick, arrow, fract(seed * 2.9) > 0.4));
    let pick = fract(seed * 17.9);
    var c = vec3<f32>(0.85, 0.28, 0.02);
    if (pick > 0.85) {
        c = vec3<f32>(0.82, 0.30, 0.5);
    } else if (pick > 0.72) {
        c = vec3<f32>(0.75, 0.74, 0.70);
    } else if (pick > 0.6) {
        c = vec3<f32>(0.08, 0.40, 0.08);
    } else if (pick > 0.48) {
        c = vec3<f32>(0.75, 0.62, 0.03);
    } else if (pick > 0.3) {
        c = vec3<f32>(0.62, 0.04, 0.03);
    } else if (pick > 0.15) {
        c = vec3<f32>(0.03, 0.18, 0.62);
    }
    let soft = max(fw, 0.006);
    let worn = 0.55 + 0.45 * st_noise(vec2<f32>(along * 40.0, kd * 40.0) + seed * 30.0);
    o.c = c;
    o.a = sl_cov(d, soft) * worn * 0.92;
    o.rough = 0.7;
    return o;
}

// A decal's own hash, from its record's centre (the wall and the walk find the same).
fn sl_dseed(d0: vec4<f32>) -> f32 {
    return st_hash(d0.xy * 0.731 + vec2<f32>(0.37, 0.71));
}

// An anchor's place along the stretch (from its middle) from its 4-bit code
// (street_life.rs `anchor_code`: 0 none, 1..15 from the stretch's start to its end).
fn sl_anchor(code: u32, oh: f32) -> f32 {
    return (f32(code - 1u) / 14.0 * 2.0 - 1.0) * oh;
}

// ---- where the walk's litter lies (stage W2d) ----------------------------------
//
// SF litter collects where feet and brooms do not reach and where people
// stop, not evenly over the walk: on the kerb's lip and in the furnishing zone
// beside it (the trees, poles, meters and bins: SL_FURNISH_M), against the
// building line and in the recessed doorways (SL_FRONTAGE_M), in the street
// trees' basins (at their trunks' kerb offset, the seed's code 5), round the
// bin (code 3) and where people wait at a transit stop (code 4). The through
// zone between keeps the odd butt and scrap (its gum is the wear decals').
// Returns the multipliers of the paper-size litter, the leaves and the butts
// at a walk point: `kd` from the kerb line, `dw` from the building line (< 0
// inside a recess), `along` from the stretch's middle (`oh` its half length).

// the through zone's own (a level 2 street is a step less kept)
const SL_OPEN_LITTER: f32 = 0.05;
const SL_OPEN_LEAVES: f32 = 0.2;
const SL_OPEN_BUTTS: f32 = 0.06;
// a street tree's basin: its reach (times the trunk's kerb offset: the basin runs
// back to the kerb's top) and the slack along (the anchor's 1/14 of a stretch, and
// eo-props' kerb shifts); no trunk offset: the furnishing zone's middle. INFERRED.
const SL_BASIN_K: f32 = 0.8;
const SL_BASIN_SLACK_M: f32 = 0.45;
// a bin's spill (along, across from its place 0.6 m off the kerb line) and a
// stop's waiting area (along either way, across from the kerb). INFERRED.
const SL_BIN_SPILL_M: vec2<f32> = vec2<f32>(1.5, 1.1);
const SL_STOP_WAIT_M: vec2<f32> = vec2<f32>(4.0, 2.6);

fn sl_walk_zones(along: f32, kd: f32, dw: f32, oh: f32, codes: u32, wall: bool, lvl: u32) -> vec3<f32> {
    var z = vec3<f32>(SL_OPEN_LITTER, SL_OPEN_LEAVES, SL_OPEN_BUTTS) * select(1.0, 1.6, lvl >= 2u);
    let k = max(kd, 0.0);
    // the kerb's lip: butts flicked at the gutter, scraps caught on the edge
    z += vec3<f32>(0.7, 0.8, 1.4) * exp(-k / 0.2);
    // the furnishing zone beside it
    if (SL_FURNISH_M > 0.0) {
        z += vec3<f32>(0.3, 0.5, 0.5) * (1.0 - smoothstep(0.75 * SL_FURNISH_M, 1.15 * SL_FURNISH_M, k));
    }
    // the building line (a lot's fence or a garden's edge, less): blown against the
    // wall's foot, a little over the frontage zone; and the doorways' recesses
    if (SL_FRONTAGE_M > 0.0) {
        let w = max(dw, 0.0);
        let line = (exp(-w / 0.18) * 2.4 + (1.0 - smoothstep(0.6 * SL_FRONTAGE_M, 1.1 * SL_FRONTAGE_M, w)) * 0.45) * select(0.55, 1.0, wall);
        z += vec3<f32>(1.0, 0.85, 0.75) * line + vec3<f32>(1.2, 1.0, 1.0) * smoothstep(0.0, 0.5, -dw);
    }
    // the trees' basins
    let tkc = (codes >> 20u) & 15u;
    let tkd = select(0.5 * SL_FURNISH_M, f32(tkc) * SL_TREE_KD_STEP_M, tkc > 0u);
    let rb = max(SL_BASIN_K * tkd, 0.4);
    for (var i = 0u; i < 2u; i = i + 1u) {
        let c = (codes >> (4u * i)) & 15u;
        if (c > 0u) {
            let e = length(vec2<f32>(max(abs(along - sl_anchor(c, oh)) - SL_BASIN_SLACK_M, 0.0), kd - tkd)) / rb;
            z += vec3<f32>(1.6, 3.5, 2.6) * (1.0 - smoothstep(0.7, 1.15, e));
        }
    }
    // the bin's spill round it
    let bc = (codes >> 12u) & 15u;
    if (bc > 0u) {
        let e = length(vec2<f32>(along - sl_anchor(bc, oh), kd - 0.6) / SL_BIN_SPILL_M);
        z += vec3<f32>(2.2, 0.6, 2.0) * (1.0 - smoothstep(0.5, 1.0, e));
    }
    // a transit stop: butts and the odd cup where people wait
    let sc = (codes >> 16u) & 15u;
    if (sc > 0u) {
        let w = (1.0 - smoothstep(0.6 * SL_STOP_WAIT_M.x, SL_STOP_WAIT_M.x, abs(along - sl_anchor(sc, oh)))) * (1.0 - smoothstep(0.7 * SL_STOP_WAIT_M.y, SL_STOP_WAIT_M.y, k));
        z += vec3<f32>(0.6, 0.2, 2.2) * w;
    }
    return z;
}

// The litter's clusters at a street point (decal space: `along` from the
// stretch's middle, `oh` its half length; `kd` from the kerb line, `dw` from
// the building line): drifts against the kerb in the gutter, drifts along the
// building line on the walk, a pile at the storm drain's inlet and one by the
// corner's or the stop's bin (the seed's anchors, street_life.rs `place`).
// Returns (core, halo), each 0..1. Clusters are metres long: what still
// reads of the litter at 30-80 m, where its items are under a pixel.
fn sl_clusters(along: f32, kd: f32, dw: f32, oh: f32, codes: u32, hseed: f32, litter: f32, leaves: f32, gutter: bool, road: bool, walk: bool, wall: bool, fw: f32, lvl: u32) -> vec2<f32> {
    let x = along + oh;
    let len = 2.0 * oh;
    let boost = select(1.0, 1.4, lvl >= 2u);
    var e = 9.0;
    if (gutter || (road && kd < 1.5)) {
        // drifts against the kerb face: one in some 5 m cells, 1.2-4.2 m long, 0.25-0.65 m out
        // (over the gutter pan and into the parking lane's edge)
        let cs = 5.0;
        let k = floor(x / cs);
        let hk = st_hash(vec2<f32>(k * 1.93 + 0.3, hseed * 211.0));
        if ((k + 1.0) * cs <= len && hk < (0.15 + 0.08 * litter + 0.03 * leaves) * boost) {
            let hl = 0.6 + 1.5 * fract(hk * 13.1);
            let cx = k * cs + hl + 0.3 + (cs - 2.0 * hl - 0.6) * fract(hk * 7.7);
            let wk = 0.25 + 0.4 * fract(hk * 3.9);
            e = min(e, length(vec2<f32>(max(abs(x - cx) - hl * 0.55, 0.0) / (hl * 0.45 + wk), kd / wk)));
        }
    }
    if (walk && wall) {
        // drifts along the building line: one in some 4.2 m cells, 0.8-3 m long, 0.25-0.55 m out
        let cs = 4.2;
        let xo = x - 1.7;
        let k = floor(xo / cs);
        let hk = st_hash(vec2<f32>(k * 2.71 + 5.3, hseed * 157.0));
        if (xo >= 0.0 && (k + 1.0) * cs + 1.7 <= len && hk < (0.08 + 0.06 * litter) * boost) {
            let hl = 0.4 + 1.1 * fract(hk * 11.3);
            let cx = 1.7 + k * cs + hl + 0.3 + (cs - 2.0 * hl - 0.6) * fract(hk * 5.9);
            let wk = 0.25 + 0.3 * fract(hk * 2.3);
            e = min(e, length(vec2<f32>(max(abs(x - cx) - hl * 0.55, 0.0) / (hl * 0.45 + wk), max(dw, 0.0) / wk)));
        }
    }
    // the anchors: the drain's pile in the gutter at its inlet, the bin's on the walk by the kerb
    let dc = (codes >> 8u) & 15u;
    if (dc > 0u && (gutter || road)) {
        let r = 0.55 + 0.4 * fract(hseed * 7.3);
        e = min(e, length(vec2<f32>((along - sl_anchor(dc, oh)) / r, (kd - 0.1) / (0.5 * r))));
    }
    let bc = (codes >> 12u) & 15u;
    if (bc > 0u && walk) {
        let r = 0.6 + 0.45 * fract(hseed * 11.9);
        e = min(e, length(vec2<f32>((along - sl_anchor(bc, oh)) / r, (kd - 0.6) / (0.75 * r))));
    }
    if (e > 2.0) {
        return vec2<f32>(0.0);
    }
    // a ragged edge (its wander falls to its mean once under a few pixels)
    let wig = mix(0.5, st_noise(vec2<f32>(along / 0.35, (kd + dw) / 0.35) + hseed * 31.0), sl_keep(0.35, fw)) - 0.5;
    let er = e + wig * 0.35;
    return vec2<f32>(1.0 - smoothstep(0.75, 1.0, er), 1.0 - smoothstep(1.0, 1.8, er));
}

// The street fragment inside a life decal: its litter, leaves, butts, grit,
// clusters, heaps and marks. `d0`, `d1`: the decal's record; `pr`: its
// parameters (street_life.rs `Params`).
fn sl_street(world: vec3<f32>, cls: u32, along: f32, across: f32, d0: vec4<f32>, d1: vec4<f32>, pr: u32, dpx: vec3<f32>, dpy: vec3<f32>) {
    let walk = cls == SC_FOOTWAY || cls == SC_RAMP || cls == SC_KERB_TOP;
    let gutter = cls == SC_GUTTER;
    let road = cls == SC_ASPHALT || cls == SC_PAD;
    if (!(walk || gutter || road)) {
        return;
    }
    let lvl = sl_level();
    let lb = select(1.0, 1.3, lvl >= 2u);
    let litter = f32(pr & 7u);
    let leaves = f32((pr >> 3u) & 7u);
    let grit = f32((pr >> 6u) & 3u);
    let deck = ((pr >> 14u) & 1u) == 1u;
    let wall = ((pr >> 15u) & 1u) == 1u;
    let fw = max(length(dpx), length(dpy));
    // the footprint's geometric mean: a cluster's metre-scale shape and its bed's
    // mottle stay across the view where the ground is seen at a grazing angle
    // (the footprint's long axis runs away from the eye; TAA takes the rest)
    let fwc = sqrt(length(dpx) * length(dpy));
    let p = world.xz;
    let own = d0;
    let oh = d1.x - SL_PAD_M;
    let seed = d1.w;
    let codes = u32(floor(seed));
    let rnd = sl_dseed(d0);
    // where in the section: the kerb (sl_kd, from the street's own coordinate) and the building line
    let kd = sl_kd;
    let dw = (d1.y - SL_WALL_REACH_M) - across;
    // Litter collects against the kerb face in the gutter, thins out into the
    // carriageway; on the walk it lies in its zones (stage W2d, sl_walk_zones), the
    // through zone nearly clean; wind and sweeping leave it in drifts.
    // (`acc` the paper-size litter's, `acc_b` the butts', `zl` the walk's leaves')
    var acc = 1.0;
    var acc_b = 1.0;
    var zl = 1.0;
    if (gutter) {
        acc = 1.0 + 5.0 * exp(-max(kd, 0.0) / 0.15);
        acc_b = acc;
    } else if (road) {
        acc = 0.5 * exp(-max(kd - 0.3, 0.0) / 0.5);
        acc_b = acc;
    } else {
        let z = sl_walk_zones(along, kd, dw, oh, codes, wall, lvl);
        acc = z.x;
        zl = z.y;
        acc_b = z.z;
    }
    let drift = 0.3 + 1.4 * smoothstep(0.35, 0.8, st_noise(p / 2.7 + rnd * 17.0));
    // (stage W2d) on the walk the scatter lies in small clumps where an eddy or a broom
    // left it, bare between: no even sprinkle across a zone (a branch, not a select: the
    // gutter's and the lane's fragments skip the noise)
    var clump = 1.0;
    if (walk) {
        clump = 0.06 + 3.2 * smoothstep(0.56, 0.78, st_noise(p / 0.7 + rnd * 23.0));
    }
    // the trees beside the stretch (its seed's low bits): leaves thickest under their crowns
    var tree = 0.0;
    for (var i = 0u; i < 2u; i = i + 1u) {
        let c = (codes >> (4u * i)) & 15u;
        if (c > 0u) {
            tree = max(tree, exp(-pow((along - sl_anchor(c, oh)) / 4.5, 2.0)));
        }
    }
    // the clusters: their cores draw the items in, their halos darken round them
    let cl = sl_clusters(along, max(kd, 0.0), dw, oh, codes, rnd, litter, leaves, gutter, road, walk, wall, fwc, lvl);
    let core = cl.x;
    // chances per cell (street_life.rs densities 0..7)
    let l7 = litter / 7.0;
    let p_big = 0.15 * pow(l7, 1.1) * acc * drift * clump * (1.0 + 5.0 * core) * lb;
    // butts and torn bits (the small grid's share `frag` of them bits)
    let p_butt = (0.06 * pow(l7, 1.5) + 0.12 * l7) * acc_b * drift * clump * (1.0 + 3.0 * core) * lb;
    // (stage W2d: fewer torn bits on the walk, where they read as confetti)
    let frag = 0.1 * l7 / max(0.05 * pow(l7, 1.5) + 0.1 * l7, 1e-4) * select(1.0, 0.6, walk);
    let leaf_acc = select(1.0, 1.0 + 4.0 * exp(-max(kd, 0.0) / 0.2), gutter) * select(1.0, 0.4, road);
    // leaves: fallen under the crowns (thinner where feet kick them on), blown into the
    // walk's zones; the gutter's and the lane's as before
    let leaf_w = select((0.25 + 0.9 * tree) * leaf_acc, 0.9 * tree * (0.45 + 0.55 * min(zl, 1.0)) + 0.25 * zl, walk);
    // (the walk's in drifts: stage W2d; the gutter's and the lane's as before; one noise)
    let ln = st_noise(p / select(1.3, 1.1, walk) + rnd * 5.0);
    let leaf_drift = select(0.4 + 0.9 * smoothstep(0.3, 0.75, ln), 0.15 + 1.6 * smoothstep(0.45, 0.8, ln), walk);
    let p_leaf = 0.5 * pow(leaves / 7.0, 1.5) * leaf_w * leaf_drift * (1.0 + 2.5 * core);
    // glass crumbs: a car broken into here and there, in the gutter and the parking lane
    let glass = select(0.0, 0.55 * smoothstep(0.82, 0.9, st_noise(p / 1.1 + rnd * 41.0)), (gutter || road) && st_hash(floor(p / 6.0) + rnd * 3.0) < 0.06 + 0.05 * l7);
    let p_grit = select(0.0, 0.08 * grit * select(1.0, clamp(acc, 0.25, 1.0), walk), lvl >= 2u) + glass;
    var lit_p = sl_none();
    // Grit and sand drifts: in the kerb corner of the gutter, along the building line;
    // under a deck a dusty film over all, and the kerb corner's packed leaf mulch.
    {
        var sand = 0.0;
        if (gutter) {
            let band = 1.0 - smoothstep(0.05, 0.12 + 0.18 * st_noise(vec2<f32>(along / 1.7, 0.5) + rnd * 9.0), max(kd, 0.0));
            sand = band * (0.35 + 0.65 * st_noise(p / 0.35 + rnd * 3.0)) * (0.3 + 0.23 * grit);
            let mulch = band * smoothstep(0.45, 0.7, st_noise(p / 0.6 + rnd * 7.0)) * leaves / 7.0;
            var m = sl_none();
            m.c = SL_MULCH;
            m.a = mulch * 0.8;
            m.rough = 0.75;
            lit_p = sl_over(lit_p, m);
        } else if (walk) {
            let band = 1.0 - smoothstep(0.05, 0.1 + 0.22 * st_noise(vec2<f32>(along / 2.1, 1.5) + rnd * 5.0), max(dw, 0.0));
            sand = band * (0.3 + 0.7 * st_noise(p / 0.4 + rnd * 2.0)) * (0.25 + 0.2 * grit);
        }
        if (deck) {
            sand = max(sand, 0.22 * (0.5 + 0.5 * st_noise(p / 3.1 + 4.4)));
        }
        var s = sl_none();
        s.c = SL_SAND * (0.85 + 0.3 * st_noise(p / 0.07 + 1.3) * sl_keep(0.07, fw));
        s.a = clamp(sand, 0.0, 0.85);
        s.rough = 0.95;
        lit_p = sl_over(lit_p, s);
    }
    if (cl.y > 0.0) {
        // the grime halo: wet dirt round each cluster
        var hg = sl_none();
        hg.c = vec3<f32>(0.045, 0.04, 0.03);
        hg.a = cl.y * 0.5;
        hg.rough = 0.7;
        lit_p = sl_over(lit_p, hg);
        if (core > 0.0) {
            // the bed: paper, cups, wrappers, leaves, grit and dirt matted together: pale
            // scraps over a dark wet mat, a coloured fleck here and there; its mean holds
            // past a pixel (paper-light where litter rules, leaf-brown under the trees)
            let n1 = mix(0.5, st_noise(p / 0.09 + rnd * 9.0), sl_keep(0.09, fwc));
            let n2 = mix(0.5, st_noise(p / 0.45 + rnd * 5.0), sl_keep(0.45, fwc));
            let n3 = mix(0.5, st_noise(p / 0.05 + rnd * 3.0), sl_keep(0.05, fwc));
            let lf = clamp(leaves / 7.0 * 0.9 + (n2 - 0.5) * 0.7, 0.0, 1.0);
            var bc = mix(vec3<f32>(0.1, 0.085, 0.065), vec3<f32>(0.15, 0.09, 0.045), lf);
            let paper = smoothstep(0.55, 0.66, n1 * 0.55 + n3 * 0.45) * (1.0 - 0.7 * lf);
            bc = mix(bc, vec3<f32>(0.56, 0.54, 0.49) * (0.8 + 0.4 * n3), paper);
            let fleck = step(0.9, n3) * step(0.5, n2) * sl_keep(0.05, fwc);
            bc = mix(bc, sl_faded(sl_spray(0.64 + 0.36 * fract(n1 * 7.3)).rgb, 0.6), fleck * 0.6);
            var b = sl_none();
            b.c = bc;
            // the walk shows through the thin of it (the scatter's items stand on it near)
            b.a = core * (0.55 + 0.35 * max(paper, smoothstep(0.35, 0.6, n2)));
            b.rough = 0.85;
            b.tn = (vec2<f32>(n1, n3) - 0.5) * 0.7 * sl_keep(0.1, fw);
            lit_p = sl_over(lit_p, b);
        }
    }
    // the scatter, the smallest under the larger
    if (p_leaf + p_butt + p_grit > 0.0) {
        lit_p = sl_over(lit_p, sl_small(p, fw, min(p_leaf, 0.85), min(p_butt, 0.7), min(p_grit, 0.6), frag, own, oh));
    }
    if (p_big > 0.0) {
        lit_p = sl_over(lit_p, sl_big(p, fw, min(p_big, 0.8), deck, own, oh));
    }
    // cardboard, newspapers and bags: neglected stretches, under the decks, in the clusters
    let p_coarse = select(0.0, 0.05 * (litter - 2.0) * acc, litter > 2.0) + select(0.0, 0.08, deck) + 0.3 * core;
    if (p_coarse > 0.0 && !road) {
        lit_p = sl_over(lit_p, sl_coarse(p, fw, min(p_coarse, 0.55), own, oh));
    }
    // the foot of a heap against the wall (its upper part is on the wall: sl_wall_pieces)
    if (walk && wall && dw < 1.2 && litter > 0.0) {
        let hp = sl_heap_at(along + oh, 2.0 * oh, litter, deck, rnd, lvl);
        if (hp.ok) {
            lit_p = sl_over(lit_p, sl_heap_ground(along + oh, dw, hp, fw));
        }
    }
    if (lvl >= 2u) {
        if (deck && walk) {
            // pigeon droppings under the deck's ledges: chalky splats in clusters
            let cs = 0.09;
            let cell = floor(p / cs);
            let cl2 = smoothstep(0.7, 0.85, st_noise(p / 2.2 + 7.7));
            if (st_hash(cell + vec2<f32>(5.3, 3.1)) < 0.5 * cl2) {
                let ctr = (cell + 0.5) * cs + (vec2<f32>(st_hash(cell + 1.7), st_hash(cell + 4.9)) - 0.5) * (cs - 0.04);
                let r = 0.008 + 0.01 * st_hash(cell + 8.8);
                let q = p - ctr;
                let d = length(q) - r * (1.0 + 0.4 * (st_noise(q / r * 2.0) - 0.5));
                var sp = sl_none();
                sp.c = vec3<f32>(0.62, 0.61, 0.57);
                sp.a = sl_cov(d, max(fw, 0.001)) * 0.85 * sl_keep(0.02, fw);
                sp.rough = 0.9;
                lit_p = sl_over(lit_p, sp);
            }
        }
        lit_p = sl_over(lit_p, sl_locate(along, select(-across, kd, kd >= 0.0 && !road), oh, rnd + 0.37, fw));
    }
    sl_put(lit_p);
}

// ---- the wall pieces -------------------------------------------------------
//
// In wall metres: `x` along the decal's own stretch from its start, `y` over
// the building's floor (or the street datum at a column). Every piece lies
// inside its cell, every cell inside the stretch: nothing is cut.

const SL_WALL_TOP_M: f32 = 3.4;
// A wall this far in front of the decal's building line (m) still takes its pieces.
const SL_WALL_IN_M: f32 = 2.5;
const SL_WALL_FAR_M: f32 = 170.0;
// The stickers' cells (level 2).
const SL_CELL_C: f32 = 1.3;

// The spray palette: black most, silver (chrome) and white, red, blue, and fluorescent accents.
fn sl_spray(h: f32) -> vec4<f32> {
    // (linear colour, roughness)
    if (h < 0.38) {
        return vec4<f32>(0.022, 0.022, 0.024, 0.42);
    } else if (h < 0.53) {
        return vec4<f32>(0.50, 0.50, 0.51, 0.3);
    } else if (h < 0.64) {
        return vec4<f32>(0.70, 0.70, 0.68, 0.45);
    } else if (h < 0.73) {
        return vec4<f32>(0.42, 0.03, 0.025, 0.42);
    } else if (h < 0.81) {
        return vec4<f32>(0.02, 0.06, 0.30, 0.42);
    } else if (h < 0.87) {
        return vec4<f32>(0.80, 0.10, 0.34, 0.4);
    } else if (h < 0.92) {
        return vec4<f32>(0.24, 0.70, 0.07, 0.4);
    } else if (h < 0.96) {
        return vec4<f32>(0.82, 0.24, 0.02, 0.4);
    }
    return vec4<f32>(0.17, 0.03, 0.26, 0.42);
}

// A tag's path point `i` (3 a glyph), in glyph-height units. Each glyph takes
// one of nine height patterns (none of them a letter's: the line meanders up,
// down and back), jittered; a curl back over the stroke or a backward hook now
// and then, an ascender or a descender: a scribbled handstyle, not lettering.
fn sl_tag_pt(i: i32, n: i32, adv: f32, hs: f32) -> vec2<f32> {
    let ic = clamp(i, 0, 3 * n - 1);
    let j = ic / 3;
    let k = ic - j * 3;
    let r0 = st_hash(vec2<f32>(f32(ic) * 1.731 + 0.5, hs * 97.0));
    let r1 = st_hash(vec2<f32>(f32(ic) * 2.417 + 7.1, hs * 61.0));
    let rt = st_hash(vec2<f32>(f32(j) * 3.137 + 2.3, hs * 83.0));
    let rc = st_hash(vec2<f32>(f32(j) * 5.419 + 4.7, hs * 47.0));
    // nine height patterns (stage W2c: three more, a tall spike, a deep
    // descender and a hump, so no shape repeats into a run of one letter)
    var pat = vec3<f32>(0.1, 1.0, 0.35);
    if (rt > 0.89) {
        pat = vec3<f32>(0.7, 0.2, -0.3);
    } else if (rt > 0.78) {
        pat = vec3<f32>(0.05, 1.35, 0.2);
    } else if (rt > 0.67) {
        pat = vec3<f32>(0.2, 0.85, 0.6);
    } else if (rt > 0.56) {
        pat = vec3<f32>(0.9, 0.1, 0.75);
    } else if (rt > 0.45) {
        pat = vec3<f32>(0.45, 1.05, 0.05);
    } else if (rt > 0.34) {
        pat = vec3<f32>(0.0, 1.6, -0.1);
    } else if (rt > 0.23) {
        pat = vec3<f32>(1.0, -0.45, 0.6);
    } else if (rt > 0.12) {
        pat = vec3<f32>(0.6, 0.55, 1.2);
    }
    var x = (f32(j) + (f32(k) + 0.15 + 0.7 * r0) / 3.0) * adv;
    // each glyph its own size and baseline (no two alike: no run reads as one letter repeated)
    let gs = 0.7 + 0.6 * st_hash(vec2<f32>(f32(j) * 1.913 + 0.7, hs * 29.0));
    let gb = (st_hash(vec2<f32>(f32(j) * 4.271 + 3.1, hs * 53.0)) - 0.5) * 0.4;
    var y = pat[k] * gs + gb + (r1 - 0.5) * 0.45;
    if (k == 2 && rc < 0.55) {
        // a curl back over the stroke
        x = x - adv * (0.5 + 0.4 * r0);
        y = 0.1 + 1.0 * r1;
    } else if (k == 1 && rc > 0.7) {
        // a backward hook at the top
        x = x - adv * (0.3 + 0.3 * r1);
        y = y + 0.25;
    } else if (k == 0 && rc > 0.45 && rc < 0.7) {
        // a lead-in from high up, ahead of the glyph
        x = x + adv * 0.35;
        y = 0.9 + 0.5 * r1;
    }
    return vec2<f32>(x, y);
}

const SL_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const SL_BLACK: vec3<f32> = vec3<f32>(0.022, 0.022, 0.024);
const SL_WHITE: vec3<f32> = vec3<f32>(0.70, 0.70, 0.68);
const SL_SILVER: vec3<f32> = vec3<f32>(0.50, 0.50, 0.51);

// "Over" for the wall pieces' layers (colour, cover): `sc` at cover `sa` over `dst`.
fn sl_ov(dst: vec4<f32>, sc: vec3<f32>, sa: f32) -> vec4<f32> {
    if (sa <= 0.0) {
        return dst;
    }
    let a = sa + dst.a * (1.0 - sa);
    return vec4<f32>((sc * sa + dst.rgb * dst.a * (1.0 - sa)) / max(a, 1e-5), a);
}

// A tag (a handstyle): one flowing meandering line of 4-6 glyph-like
// scribbles with curls and hooks (no alphabet: nine height patterns, jittered),
// an ascender or a descender now and then, a strike through some, an
// underline swoosh on some. Fat-cap spray (strokes 2.5-7 cm wide, soft edged,
// overspray, drips; a third in two tones: the line over a wider outline of a
// paint that stands off it) or a fat marker (1.2-2.4 cm, hard edged), in the
// band `ylo`..`yhi` (m up the wall) of its cell, `cw` wide. Returns (colour,
// cover) and the roughness through `rough`.
fn sl_tag(x: f32, y: f32, cw: f32, hs: f32, sz: f32, ylo: f32, yhi: f32, fw: f32, rough: ptr<function, f32>) -> vec4<f32> {
    let h1 = st_hash(vec2<f32>(hs * 31.1, 1.7));
    let h2 = st_hash(vec2<f32>(hs * 17.3, 4.1));
    let h3 = st_hash(vec2<f32>(hs * 23.9, 7.3));
    let h4 = st_hash(vec2<f32>(hs * 11.7, 2.9));
    let h5 = st_hash(vec2<f32>(hs * 41.3, 5.3));
    let h6 = st_hash(vec2<f32>(hs * 29.1, 8.9));
    let h7 = st_hash(vec2<f32>(hs * 37.7, 3.3));
    let h8 = st_hash(vec2<f32>(hs * 53.1, 6.7));
    let marker = h7 > 0.82;
    let n = 4 + i32(h2 * 2.99);
    // glyphs overlap their neighbours (0.38-0.63 of a glyph's height apart): no letter stands alone
    let adv = 0.38 + 0.25 * h3;
    let wmax = cw - 0.2;
    let hgt = min((0.3 + 0.55 * h1) * sz * select(1.0, 0.7, marker), min(wmax / (f32(n) * adv + 0.3), max(yhi - ylo, 0.2) / 1.9));
    let wid = (f32(n) * adv + 0.3) * hgt;
    let x0 = 0.1 + max(wmax - wid, 0.0) * h5;
    // the line's foot: descenders reach 0.55 of a glyph below it, ascenders 1.35 above
    let y0 = ylo + 0.55 * hgt + max(yhi - ylo - 1.9 * hgt, 0.0) * h4;
    let slant = -0.12 + 0.5 * h6;
    let w = select((0.0125 + 0.022 * h8) * clamp(hgt / 0.45, 0.7, 1.3), 0.006 + 0.006 * h1, marker);
    let two = !marker && h8 > 0.66;
    let wo = select(w, w * 1.8 + 0.006, two);
    let tx = x - x0;
    let ty = y - y0;
    if (tx < -0.15 || tx > wid + 0.15 || ty < -0.6 * hgt - 0.35 || ty > 1.8 * hgt + 0.05) {
        return vec4<f32>(0.0);
    }
    // glyph space: sheared by the slant, in glyph heights
    let g = vec2<f32>((tx - slant * ty) / hgt, ty / hgt);
    let jj = i32(floor(g.x / adv));
    var dmin = 1e9;
    let i0 = 3 * jj - 2;
    for (var i = i0; i <= i0 + 5; i = i + 1) {
        if (i < 0 || i > 3 * n - 2) {
            continue;
        }
        let a = sl_tag_pt(i - 1, n, adv, hs);
        let c = sl_tag_pt(i, n, adv, hs);
        let b = sl_tag_pt(i + 1, n, adv, hs);
        let ma = select((a + c) * 0.5, c, i == 0);
        let mb = select((c + b) * 0.5, b, i == 3 * n - 2);
        dmin = min(dmin, sl_qbez(g, ma, c, mb));
    }
    // a strike through the whole tag on some (no crossbar inside a glyph: that makes letters)
    if (h4 > 0.7) {
        let e = f32(n) * adv;
        dmin = min(dmin, sl_seg(g, vec2<f32>(-0.1, 0.15 + 0.3 * h6), vec2<f32>(e + 0.1, 0.55 + 0.4 * h1)));
    }
    // an underline swoosh, rising at its end
    if (h3 > 0.55) {
        let e = f32(n) * adv;
        dmin = min(dmin, sl_qbez(g, vec2<f32>(-0.05, -0.12), vec2<f32>(e * 0.6, -0.32), vec2<f32>(e + 0.25, 0.35)));
    }
    let dm = dmin * hgt;
    let soft = max(fw, select(w * 0.4, w * 0.12, marker));
    let core = sl_cov(dm - w, soft);
    var cover = sl_cov(dm - wo, soft);
    let col = sl_spray(h5 * 0.6 + h7 * 0.4);
    var c = col.rgb;
    if (two) {
        // the outline in a paint that stands off the line's own
        let dark = dot(col.rgb, SL_LUMA) < 0.12;
        let oc = select(SL_BLACK, select(SL_SILVER, SL_WHITE, h3 > 0.5), dark);
        c = mix(oc, col.rgb, core);
    }
    if (!marker) {
        // overspray: a faint speckled halo
        let ov = (1.0 - smoothstep(wo, wo + 0.04, dm)) * 0.2 * mix(0.5, st_noise(vec2<f32>(x, y) * 420.0 + hs * 9.0), sl_keep(0.004, fw));
        cover = max(cover, ov);
        // drips from the low points of the line
        for (var i = i0; i <= i0 + 5; i = i + 1) {
            if (i < 0 || i > 3 * n - 1) {
                continue;
            }
            let r = st_hash(vec2<f32>(f32(i) * 3.3 + 0.7, hs * 71.0));
            let pt = sl_tag_pt(i, n, adv, hs);
            if (r > 0.3 || pt.y > 0.4) {
                continue;
            }
            let len = 0.03 + 0.25 * r / 0.3;
            let px = (pt.x + slant * pt.y) * hgt;
            let py = pt.y * hgt;
            let dx = tx - px;
            let dy = py - ty;
            let wd = wo * 0.42;
            let dd = min(max(abs(dx) - wd * (1.0 - 0.3 * clamp(dy / len, 0.0, 1.0)), max(-dy, dy - len)), length(vec2<f32>(dx, dy - len)) - wd * 1.4);
            cover = max(cover, sl_cov(dd, max(fw, wd * 0.3)) * 0.95);
        }
    }
    *rough = col.w;
    return vec4<f32>(c, cover * 0.97);
}

// A throw-up's bubble glyphs at `p` (x from its first glyph, y up from its
// foot, metres; `adv` a glyph's advance): (distance, counter distance). Each
// glyph a disc, a slanted capsule or a bean, jittered and merged with its
// neighbours into one cloud (they overlap: no glyph stands alone to read as a
// letter); a short slash inside most as the counter stroke (no ring, no bar).
fn sl_bubble(p: vec2<f32>, n: i32, adv: f32, hgt: f32, hs: f32) -> vec2<f32> {
    let jj = i32(floor(p.x / adv));
    var d = 1e9;
    var counter = 1e9;
    for (var j = max(jj - 1, 0); j <= min(jj + 1, n - 1); j = j + 1) {
        let r = st_hash(vec2<f32>(f32(j) * 4.13 + 0.3, hs * 59.0));
        let r2 = st_hash(vec2<f32>(f32(j) * 6.71 + 2.9, hs * 43.0));
        let r3 = st_hash(vec2<f32>(f32(j) * 2.93 + 5.7, hs * 31.0));
        let c = vec2<f32>((f32(j) + 0.5 + (r3 - 0.5) * 0.15) * adv, (0.5 + (r2 - 0.5) * 0.16) * hgt);
        var dj: f32;
        if (r < 0.45) {
            dj = length(p - c) - (0.44 + 0.1 * r3) * hgt;
        } else if (r < 0.8) {
            dj = sl_seg(p, c + vec2<f32>(-0.15 * adv, -0.2 * hgt), c + vec2<f32>(0.12 * adv, 0.2 * hgt)) - 0.36 * hgt;
        } else {
            dj = min(length(p - c - vec2<f32>(-0.12 * adv, -0.08 * hgt)) - 0.38 * hgt, length(p - c - vec2<f32>(0.14 * adv, 0.1 * hgt)) - 0.34 * hgt);
        }
        d = sl_smin(d, dj, 0.14 * hgt);
        if (r3 < 0.7) {
            counter = min(counter, sl_seg(p, c + vec2<f32>(-0.12 * adv, -0.06 * hgt), c + vec2<f32>(0.08 * adv, (0.16 + 0.08 * r) * hgt)));
        }
    }
    return vec2<f32>(d, counter);
}

// A throw-up: 2-4 fat bubble glyphs merged into one cloud 0.5-1.25 m tall,
// filled quickly (chrome silver or black most, else white or a colour), a hard
// line in a paint that stands off the fill, a counter stroke in each glyph, a
// drop shadow down and to the right, sometimes a second outline, a shine,
// drips from the fill and an overspray halo round it all.
fn sl_throw(x: f32, y: f32, cw: f32, hs: f32, sz: f32, ylo: f32, yhi: f32, fw: f32, rough: ptr<function, f32>) -> vec4<f32> {
    let h1 = st_hash(vec2<f32>(hs * 13.1, 2.3));
    let h2 = st_hash(vec2<f32>(hs * 19.7, 6.1));
    let h3 = st_hash(vec2<f32>(hs * 7.9, 9.9));
    let h4 = st_hash(vec2<f32>(hs * 27.3, 1.1));
    let h5 = st_hash(vec2<f32>(hs * 33.7, 4.7));
    let h6 = st_hash(vec2<f32>(hs * 45.1, 8.3));
    let n = 2 + i32(h2 * 2.99);
    let adv0 = 0.6;
    let hgt = min((0.5 + 0.75 * h1) * sz, min((cw - 0.3) / (f32(n) * adv0 + 0.75), max(yhi - ylo, 0.3) / 1.2));
    let adv = adv0 * hgt;
    let wid = f32(n) * adv + 0.35 * hgt;
    let x0 = 0.15 + max(cw - 0.3 - wid - 0.4 * hgt, 0.0) * h3;
    let y0 = ylo + 0.1 * hgt + max(yhi - ylo - 1.2 * hgt, 0.0) * h4;
    let tx = x - x0 - 0.3 * hgt;
    let ty = y - y0;
    if (tx < -0.4 * hgt || tx > wid + 0.15 * hgt || ty < -0.75 * hgt || ty > 1.3 * hgt) {
        return vec4<f32>(0.0);
    }
    let p = vec2<f32>(tx, ty);
    let bd = sl_bubble(p, n, adv, hgt, hs);
    let d = bd.x;
    let ow = (0.022 + 0.022 * h5) * clamp(hgt / 0.8, 0.7, 1.4);
    let soft = max(fw, 0.004);
    // the scheme: chrome with a black line, black with a white or silver one,
    // white with a dark one, a colour with a black or white one
    let k = fract(h2 * 7.13);
    var fill = vec4<f32>(SL_SILVER, 0.3);
    var line = SL_BLACK;
    if (k > 0.72) {
        fill = sl_spray(0.64 + 0.36 * h3);
        line = select(SL_BLACK, SL_WHITE, h6 > 0.62);
    } else if (k > 0.55) {
        fill = vec4<f32>(SL_WHITE, 0.45);
        line = select(SL_BLACK, sl_spray(0.64 + 0.36 * h6).rgb, h6 > 0.7);
    } else if (k > 0.34) {
        fill = vec4<f32>(SL_BLACK, 0.42);
        line = select(SL_WHITE, SL_SILVER, h6 > 0.5);
    }
    // the drop shadow: black under a light fill, else the line's paint darkened or a colour
    let light = dot(fill.rgb, SL_LUMA) > 0.2;
    let shadow_c = select(select(line * 0.35 + 0.01, sl_spray(0.64 + 0.36 * fract(h5 * 5.3)).rgb * 0.6, h4 > 0.6), vec3<f32>(0.03, 0.03, 0.035), light);
    let ds = sl_bubble(p - vec2<f32>(0.075, -0.06) * hgt, n, adv, hgt, hs).x;
    var o = vec4<f32>(0.0);
    // the overspray halo round it all
    let haze = mix(0.6, st_noise(p * 160.0 + hs * 7.0), sl_keep(0.006, fw));
    o = sl_ov(o, line, (1.0 - smoothstep(0.0, 0.05, min(d - ow, ds - ow))) * 0.18 * haze);
    let fill_cov = sl_cov(d, soft);
    // a second outline round the first, in white or the fill's own paint
    if (h4 > 0.55) {
        o = sl_ov(o, select(SL_WHITE, fill.rgb, h4 > 0.8 || light), sl_cov(d - 3.2 * ow, soft) * (1.0 - fill_cov));
    }
    o = sl_ov(o, shadow_c, sl_cov(ds - ow, soft) * (1.0 - fill_cov));
    // the fill, quick (thin where the can moved fast), the line and the counters over it
    let gaps = mix(0.9, smoothstep(0.18, 0.45, st_noise(p * 9.0 + hs * 5.0)) * 0.25 + 0.75, sl_keep(0.05, fw));
    o = sl_ov(o, fill.rgb * (0.9 + 0.1 * st_noise(p * 30.0)), fill_cov * gaps);
    let ln = max(sl_cov(abs(d) - ow, soft), sl_cov(bd.y - ow * 0.6, soft) * sl_cov(d + ow, soft));
    o = sl_ov(o, line, ln);
    // a shine at the top left of each glyph
    let shine = sl_cov(sl_seg(vec2<f32>(fract(tx / adv) * adv, ty), vec2<f32>(0.25 * adv, 0.78 * hgt), vec2<f32>(0.32 * adv, 0.86 * hgt)) - 0.014, soft) * sl_cov(d + ow * 2.0, soft);
    o = sl_ov(o, vec3<f32>(0.72, 0.72, 0.7), shine * 0.9);
    // drips from the fill's foot
    let dw = 0.1;
    let slot = floor(tx / dw);
    let r = st_hash(vec2<f32>(slot * 2.17 + 1.3, hs * 91.0));
    if (r < 0.32) {
        let xd = (slot + 0.3 + 0.4 * fract(r * 13.7)) * dw;
        let ys = 0.14 * hgt;
        if (sl_bubble(vec2<f32>(xd, ys), n, adv, hgt, hs).x < -ow) {
            let len = 0.06 + 0.3 * fract(r * 7.3);
            let wd = 0.005 + 0.006 * fract(r * 3.1);
            let dy = ys - ty;
            let dd = min(max(abs(tx - xd) - wd * (1.0 - 0.3 * clamp(dy / len, 0.0, 1.0)), max(-dy, dy - len)), length(vec2<f32>(tx - xd, dy - len)) - wd * 1.4);
            o = sl_ov(o, fill.rgb, sl_cov(dd, max(fw, wd * 0.3)) * 0.95);
        }
    }
    *rough = fill.w;
    return o;
}

// A rounded box's distance (centre `c`, half size `h`, corner radius `r`).
fn sl_rbox(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>, r: f32) -> f32 {
    let b = abs(p - c) - h + r;
    return length(max(b, vec2<f32>(0.0))) + min(max(b.x, b.y), 0.0) - r;
}

// A piece's mass at `p` (piece space, metres; `wid` x `hgt`): one slanted
// band, bumps up and down off its edges in some of its segments, wedges cut
// into them in others, an arrow rising off its end. One connected shape by
// construction: no glyph stands alone, so nothing reads as a letter or a word;
// its breaks (sl_piece_breaks), block and colours make it read as a piece.
fn sl_piece_d(p: vec2<f32>, wid: f32, hgt: f32, hs: f32) -> f32 {
    let q = vec2<f32>(p.x - (p.y / hgt - 0.5) * 0.22 * hgt, p.y);
    var d = sl_rbox(q, vec2<f32>(wid * 0.5, hgt * 0.5), vec2<f32>(wid * 0.5, hgt * 0.5), 0.16 * hgt);
    let sw = 0.42 * hgt;
    let js = floor(q.x / sw);
    for (var i = -1; i <= 1; i = i + 1) {
        let j = js + f32(i);
        if (j < 0.0 || (j + 1.0) * sw > wid) {
            continue;
        }
        let r1 = st_hash(vec2<f32>(j * 3.71 + 0.4, hs * 67.0));
        let r2 = st_hash(vec2<f32>(j * 5.13 + 1.9, hs * 29.0));
        let r3 = st_hash(vec2<f32>(j * 2.39 + 3.3, hs * 83.0));
        let r4 = st_hash(vec2<f32>(j * 7.77 + 0.8, hs * 41.0));
        let cx = (j + 0.5) * sw;
        if (r1 < 0.45) {
            // a bump up (a tall part) or down (a descender)
            let y0 = select(-(0.1 + 0.25 * r3) * hgt, 0.55 * hgt, r2 > 0.45);
            let y1 = select(0.45 * hgt, (1.08 + 0.3 * r3) * hgt, r2 > 0.45);
            d = sl_smin(d, sl_rbox(q, vec2<f32>(cx + (r4 - 0.5) * 0.2 * sw, (y0 + y1) * 0.5), vec2<f32>(0.36 * sw, (y1 - y0) * 0.5), 0.1 * hgt), 0.06 * hgt);
        } else if (r1 > 0.68) {
            // a wedge cut into the top or the bottom edge
            let u = select(q.y, hgt - q.y, r2 > 0.5);
            let depth = (0.25 + 0.15 * r4) * hgt;
            let wd = max(abs(q.x - cx) - (depth - u) * 0.45, max(-u - 0.5 * hgt, u - depth));
            d = max(d, -wd);
        }
    }
    // an arrow rising off its end
    let ang = 0.2 + 0.6 * st_hash(vec2<f32>(hs * 17.9, 2.1));
    let dr = vec2<f32>(cos(ang), sin(ang));
    let e = vec2<f32>(wid - 0.1 * hgt, 0.55 * hgt);
    d = sl_smin(d, sl_seg(q, vec2<f32>(wid - 0.4 * hgt, 0.5 * hgt), e) - 0.1 * hgt, 0.05 * hgt);
    let qq = q - e;
    let u = dot(qq, dr);
    let v = abs(dot(qq, vec2<f32>(-dr.y, dr.x)));
    let l = 0.45 * hgt;
    return min(d, max(max(-u, u - l), v - (l - u) * 0.6 - 0.04 * hgt) * 0.85);
}

// A piece's breaks: slanted lines across the band at some of its segments'
// joins (in the line's paint, inside the fill), as lettering's joins read
// from afar.
fn sl_piece_breaks(p: vec2<f32>, hgt: f32, hs: f32) -> f32 {
    let q = vec2<f32>(p.x - (p.y / hgt - 0.5) * 0.22 * hgt, p.y);
    let sw = 0.42 * hgt;
    let j = round(q.x / sw);
    let r = st_hash(vec2<f32>(j * 1.37 + 2.2, hs * 53.0));
    if (r > 0.55 || j <= 0.0) {
        return 1e9;
    }
    let x = j * sw;
    return sl_seg(q, vec2<f32>(x - 0.12 * hgt, (0.15 + 0.2 * r) * hgt), vec2<f32>(x + 0.12 * hgt, (0.6 + 0.3 * r) * hgt));
}

// A piece (a production): one slanted, notched band 0.95-1.25 m tall and 1.6-3.2
// times as long, a two-colour fade fill with bubbles, shines and breaks, a black
// line, a 3D block down and to the right, a white outline round it all,
// sometimes a cloud of a fourth paint behind. On the hottest walls
// (industrial, vacant, under the decks), in the band `ylo`..`yhi` of its cell
// `cw` wide.
fn sl_piece(x: f32, y: f32, cw: f32, hs: f32, ylo: f32, yhi: f32, fw: f32, rough: ptr<function, f32>) -> vec4<f32> {
    let h1 = st_hash(vec2<f32>(hs * 11.3, 5.9));
    let h2 = st_hash(vec2<f32>(hs * 23.1, 0.7));
    let h3 = st_hash(vec2<f32>(hs * 7.1, 3.7));
    let h4 = st_hash(vec2<f32>(hs * 31.9, 9.1));
    let h5 = st_hash(vec2<f32>(hs * 17.9, 2.1));
    let h6 = st_hash(vec2<f32>(hs * 43.3, 6.3));
    let span = 1.6 + 1.6 * h2;
    let hgt = min(0.95 + 0.75 * h1, min(max(yhi - ylo, 0.5) / 2.2, (cw - 0.4) / (span + 1.5)));
    let wid = span * hgt;
    let x0 = 0.2 + 0.7 * hgt + max(cw - 0.4 - wid - 1.5 * hgt, 0.0) * h3;
    let y0 = ylo + 0.6 * hgt + max(yhi - ylo - 2.2 * hgt, 0.0) * h4;
    let p = vec2<f32>(x - x0, y - y0);
    if (p.x < -0.7 * hgt || p.x > wid + 0.8 * hgt || p.y < -0.6 * hgt || p.y > 1.6 * hgt) {
        return vec4<f32>(0.0);
    }
    let soft = max(fw, 0.004);
    let ow = (0.03 + 0.02 * h5) * clamp(hgt / 1.2, 0.8, 1.3);
    // the paints: a fade from c0 (top) to c1, the block c3, the cloud c4
    let c0 = sl_spray(0.64 + 0.36 * h1).rgb;
    let c1 = sl_spray(0.64 + 0.36 * fract(h1 + 0.37 + 0.3 * h6)).rgb;
    let c3 = select(sl_spray(0.64 + 0.36 * fract(h3 * 3.7)).rgb * 0.45, vec3<f32>(0.04, 0.04, 0.05), h6 > 0.5);
    let c4 = mix(sl_spray(0.64 + 0.36 * fract(h4 * 5.1)).rgb, vec3<f32>(0.7), 0.35);
    var o = vec4<f32>(0.0);
    if (h5 > 0.3) {
        // the cloud behind: a bumpy oval round the band, outlined
        let ce = vec2<f32>((p.x - wid * 0.5) / (wid * 0.5 + 0.45 * hgt), (p.y - 0.45 * hgt) / (0.85 * hgt));
        let bump = st_noise(vec2<f32>(atan2(ce.y, ce.x) * 2.5 + hs * 9.0, h5 * 7.0));
        let dcl = (length(ce) - 0.92 - 0.14 * bump) * 0.7 * hgt;
        o = sl_ov(o, c4, sl_cov(dcl, soft) * 0.95);
        o = sl_ov(o, SL_BLACK, sl_cov(abs(dcl) - 0.012, soft));
    }
    let d = sl_piece_d(p, wid, hgt, hs);
    let blk = vec2<f32>(0.085, -0.07) * hgt;
    let d3 = min(sl_piece_d(p - blk * 0.5, wid, hgt, hs), sl_piece_d(p - blk, wid, hgt, hs));
    let dall = min(d, d3);
    // the white outline round the band and its block, the black line inside it
    o = sl_ov(o, SL_WHITE, sl_cov(dall - ow * 2.6, soft));
    o = sl_ov(o, SL_BLACK, sl_cov(dall - ow, soft));
    // the block, darker toward its far edge
    o = sl_ov(o, c3 * (0.8 + 0.4 * clamp(-d3 / (0.12 * hgt), 0.0, 1.0)), sl_cov(d3, soft));
    // the fill: the fade, bubbles of a lighter paint, short shines at the segments' tops
    let t = clamp(p.y / hgt + (st_noise(p * 2.5 + hs * 3.0) - 0.5) * 0.25, 0.0, 1.0);
    var fc = mix(c1, c0, smoothstep(0.3, 0.7, t));
    let bs = 0.17 * hgt;
    let bc = floor(p / bs);
    let rb = st_hash(bc * 1.37 + hs * 19.0);
    let bub = length(p - (bc + 0.5) * bs) - bs * (0.18 + 0.2 * rb);
    fc = mix(fc, min(fc * 1.5 + 0.15, vec3<f32>(0.75)), sl_cov(bub, soft) * step(rb, 0.3) * sl_keep(bs * 0.5, fw));
    let sw = 0.42 * hgt;
    let shine = sl_cov(sl_seg(vec2<f32>(fract(p.x / sw) * sw, p.y), vec2<f32>(0.2 * sw, 0.74 * hgt), vec2<f32>(0.45 * sw, 0.82 * hgt)) - 0.016, soft);
    fc = mix(fc, SL_WHITE, shine * 0.9);
    let inside = sl_cov(d + ow, soft);
    o = sl_ov(o, fc, inside);
    o = sl_ov(o, SL_BLACK, sl_cov(sl_piece_breaks(p, hgt, hs) - ow * 0.5, soft) * inside);
    *rough = 0.42;
    return o;
}

// A heap at the wall's foot (sl_heap_at): what it is, where, how big.
struct SlHeap {
    ok: bool,
    // 0 black and white bags, 1 a leaning sheet of cardboard, 2 a stained mattress
    kind: u32,
    // along the stretch (m): its middle and half length
    cx: f32,
    hl: f32,
    // its height up the wall and its reach out over the walk (m)
    h: f32,
    depth: f32,
    seed: f32,
};

// The heap of the cell (3.4 m) at `x` along a stretch `len` long, if any: more
// on littered stretches, under the decks, at level 2. The wall (sl_wall) and
// the walk (sl_street) find the same one.
fn sl_heap_at(x: f32, len: f32, litter: f32, deck: bool, hseed: f32, lvl: u32) -> SlHeap {
    var o: SlHeap;
    o.ok = false;
    let cs = 3.4;
    let xo = x - 0.3;
    let k = floor(xo / cs);
    if (xo < 0.0 || (k + 1.0) * cs + 0.3 > len) {
        return o;
    }
    let hk = st_hash(vec2<f32>(k * 2.91 + 0.7, hseed * 317.0));
    if (hk >= (0.05 + 0.05 * litter + select(0.0, 0.15, deck)) * select(1.0, 1.4, lvl >= 2u)) {
        return o;
    }
    let hp = st_hash(vec2<f32>(k * 1.77 + 4.1, hseed * 181.0));
    o.kind = select(select(0u, 1u, hp > 0.5), 2u, hp > 0.86);
    o.hl = select(select(0.35 + 0.65 * fract(hp * 7.1), 0.3 + 0.35 * fract(hp * 5.3), o.kind == 1u), 0.45 + 0.25 * fract(hp * 3.3), o.kind == 2u);
    o.cx = 0.3 + k * cs + o.hl + 0.1 + (cs - 2.0 * o.hl - 0.2) * fract(hp * 11.3);
    o.h = select(select(0.3 + 0.45 * fract(hp * 13.7), 0.6 + 0.6 * fract(hp * 17.1), o.kind == 1u), 1.35 + 0.5 * fract(hp * 19.3), o.kind == 2u);
    o.depth = select(select(0.35 + 0.35 * fract(hp * 23.1), 0.25, o.kind == 1u), 0.4, o.kind == 2u);
    o.seed = hp;
    o.ok = true;
    return o;
}

// The bags' distance at (x along, v): three or four lumps standing on the
// ground (`v` up the wall, or out over the walk for the footprint).
fn sl_bags_d(x: f32, v: f32, hp: SlHeap) -> f32 {
    var d = 1e9;
    for (var i = 0; i < 4; i = i + 1) {
        let r = st_hash(vec2<f32>(f32(i) * 1.91 + 0.3, hp.seed * 97.0));
        let rr = min(hp.hl * (0.38 + 0.2 * r), hp.h);
        let cx = hp.cx + (f32(i) / 1.5 - 1.0) * (hp.hl - rr) ;
        d = sl_smin(d, length(vec2<f32>(x - cx, (v - rr * 0.75) * 1.15)) - rr, 0.04);
    }
    return d;
}

// A bag's paint at a lump: black most, some white or grey, now and then a
// green or blue; the lumps' own shading by height (a matte plastic).
fn sl_bag_c(x: f32, v: f32, hp: SlHeap) -> vec3<f32> {
    let i = floor((x - hp.cx + hp.hl) / max(hp.hl * 0.67, 0.1));
    let r = st_hash(vec2<f32>(i * 3.7 + 1.1, hp.seed * 53.0));
    var c = vec3<f32>(0.016, 0.016, 0.018);
    if (r > 0.9) {
        c = vec3<f32>(0.03, 0.12, 0.05);
    } else if (r > 0.82) {
        c = vec3<f32>(0.04, 0.07, 0.2);
    } else if (r > 0.62) {
        c = vec3<f32>(0.5, 0.5, 0.48);
    }
    return c * (0.8 + 0.5 * st_noise(vec2<f32>(x, v) * 14.0 + hp.seed * 7.0));
}

// The heap on the wall at (`x`, `y`): (colour, cover); its roughness through `rough`.
fn sl_heap_wall(x: f32, y: f32, hp: SlHeap, fw: f32, rough: ptr<function, f32>) -> vec4<f32> {
    let soft = max(fw, 0.005);
    if (abs(x - hp.cx) > hp.hl + 0.2 || y > hp.h + 0.1) {
        return vec4<f32>(0.0);
    }
    var o = vec4<f32>(0.0);
    // the damp grime it leaves on the wall's foot
    o = sl_ov(o, vec3<f32>(0.05, 0.045, 0.035), (1.0 - smoothstep(0.0, 0.25, y)) * (1.0 - smoothstep(hp.hl, hp.hl + 0.2, abs(x - hp.cx))) * 0.4);
    if (hp.kind == 0u) {
        let d = sl_bags_d(x, y, hp);
        o = sl_ov(o, sl_bag_c(x, y, hp), sl_cov(d, soft));
        *rough = 0.35;
    } else if (hp.kind == 1u) {
        // a flattened box leaning on the wall: kraft, its flaps' fold lines, wet at the foot
        let lean = (hp.seed - 0.5) * 0.3;
        let u = x - hp.cx - lean * y;
        let d = max(abs(u) - hp.hl, y - hp.h) + (st_noise(vec2<f32>(x, y) * 30.0) - 0.5) * 0.01;
        let fold = 1.0 - 0.25 * (1.0 - smoothstep(0.004, 0.004 + soft, abs(y - hp.h * 0.78)));
        let wet = 1.0 - 0.4 * (1.0 - smoothstep(0.05, 0.2, y));
        let c = vec3<f32>(0.34, 0.24, 0.14) * fold * wet * (0.9 + 0.15 * st_noise(vec2<f32>(u * 200.0, y * 5.0)));
        o = sl_ov(o, c, sl_cov(d, soft));
        *rough = 0.92;
    } else {
        // a mattress on its edge: off-white ticking, a tufted grid, stains, the dirty foot
        let lean = (hp.seed - 0.5) * 0.12;
        let u = x - hp.cx - lean * y;
        let q = vec2<f32>(abs(u) - hp.hl + 0.06, abs(y - hp.h * 0.5) - hp.h * 0.5 + 0.06);
        let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - 0.06;
        let tuft = 1.0 - 0.18 * sl_cov(length(fract(vec2<f32>(u, y) / 0.2) - 0.5) * 0.2 - 0.012, soft) * sl_keep(0.05, fw);
        let stain = smoothstep(0.55, 0.75, st_noise(vec2<f32>(u, y) * 2.2 + hp.seed * 13.0));
        var c = vec3<f32>(0.56, 0.53, 0.46) * tuft;
        c = mix(c, vec3<f32>(0.3, 0.22, 0.12), stain * 0.6);
        c = c * (1.0 - 0.45 * (1.0 - smoothstep(0.0, 0.3, y)));
        o = sl_ov(o, c, sl_cov(d, soft));
        *rough = 0.9;
    }
    return o;
}

// The heap's footprint on the walk at (`x` along, `dw` out from the building
// line), and the dark wet halo round it: the street paint.
fn sl_heap_ground(x: f32, dw: f32, hp: SlHeap, fw: f32) -> SlPaint {
    var o = sl_none();
    let soft = max(fw, 0.006);
    let e = length(vec2<f32>(max(abs(x - hp.cx) - hp.hl * 0.6, 0.0) / (hp.hl * 0.4 + 0.15), max(dw, 0.0) / hp.depth));
    let halo = 1.0 - smoothstep(0.8, 1.7, e + (st_noise(vec2<f32>(x, dw) * 3.0 + hp.seed * 5.0) - 0.5) * 0.3);
    o.c = vec3<f32>(0.05, 0.045, 0.035);
    o.a = halo * 0.45;
    o.rough = 0.6;
    var c = vec3<f32>(0.0);
    var d = 1.0;
    if (hp.kind == 0u) {
        d = sl_bags_d(x, dw * hp.h / hp.depth, hp);
        c = sl_bag_c(x, dw, hp);
    } else if (hp.kind == 1u) {
        d = max(abs(x - hp.cx) - hp.hl, dw - hp.depth);
        c = vec3<f32>(0.34, 0.24, 0.14) * 0.65;
    } else {
        d = max(abs(x - hp.cx) - hp.hl, dw - hp.depth);
        c = vec3<f32>(0.4, 0.37, 0.31);
    }
    var it = sl_none();
    it.c = c;
    it.a = sl_cov(d, soft);
    it.rough = select(0.9, 0.35, hp.kind == 0u);
    return sl_over(o, it);
}

// A painted-over patch (the city's or the owner's buff): one or two roller
// rectangles in a paint that does not quite match the wall.
fn sl_buff(x: f32, y: f32, cw: f32, hs: f32, wall: vec3<f32>, fw: f32) -> vec4<f32> {
    let h1 = st_hash(vec2<f32>(hs * 5.3, 8.1));
    let h2 = st_hash(vec2<f32>(hs * 9.7, 3.9));
    let h3 = st_hash(vec2<f32>(hs * 3.1, 6.7));
    let h4 = st_hash(vec2<f32>(hs * 15.1, 0.9));
    let bw = min(0.8 + 2.2 * h1, cw - 0.2);
    let bh = 0.6 + 1.9 * h2;
    let cx = 0.1 + bw * 0.5 + (cw - 0.2 - bw) * h3;
    let cy = bh * 0.5 + 0.25 * h4 * select(0.0, 1.0, h4 > 0.6);
    let rag = (st_noise(vec2<f32>(y * 9.0, hs * 7.0)) - 0.5) * 0.06 + (st_noise(vec2<f32>(x * 9.0, hs * 3.0)) - 0.5) * 0.05;
    var d = max(abs(x - cx) - bw * 0.5, abs(y - cy) - bh * 0.5) + rag;
    // a second, smaller patch overlapping the first
    if (h1 > 0.5) {
        let d2 = max(abs(x - cx - bw * 0.35) - bw * 0.3, abs(y - cy * 0.8) - bh * 0.35) + rag * 0.7;
        d = min(d, d2);
    }
    let l = dot(wall, vec3<f32>(0.2126, 0.7152, 0.0722));
    var c = vec3<f32>(l) * 1.12 + vec3<f32>(0.02, 0.015, 0.0);
    if (h4 < 0.3) {
        c = vec3<f32>(0.30, 0.30, 0.29);
    } else if (h4 < 0.48) {
        c = vec3<f32>(0.43, 0.39, 0.31);
    } else if (h4 < 0.6) {
        c = vec3<f32>(0.60, 0.58, 0.53);
    } else if (h4 < 0.8) {
        c = mix(wall, vec3<f32>(l), 0.4) * select(0.86, 1.1, h2 > 0.5);
    }
    // roller marks
    c = c * (0.96 + 0.06 * st_noise(vec2<f32>(x * 3.0, y * 40.0) + hs * 3.0));
    return vec4<f32>(c, sl_cov(d, max(fw, 0.006)));
}

// A poster's design at (u, v) in 0..1 (no lettering): a colour field, a big
// shape, solid bars, halftone or a photo-like block.
fn sl_poster_design(u: f32, v: f32, hd: f32, fw_u: f32) -> vec3<f32> {
    let h1 = st_hash(vec2<f32>(hd * 7.7, 1.3));
    let h2 = st_hash(vec2<f32>(hd * 3.9, 5.1));
    let h3 = st_hash(vec2<f32>(hd * 9.1, 2.2));
    var bg = sl_spray(0.6 + 0.4 * h1).rgb;
    if (h2 < 0.3) {
        bg = vec3<f32>(0.68, 0.67, 0.64);
    } else if (h2 < 0.42) {
        bg = vec3<f32>(0.03, 0.03, 0.035);
    }
    let ink = select(vec3<f32>(0.03, 0.03, 0.035), sl_spray(fract(h1 * 5.3 + 0.37) * 0.4 + 0.6).rgb, h2 < 0.42);
    var c = bg;
    let kind = h3;
    let soft = max(fw_u, 0.01);
    if (kind < 0.3) {
        // a big disc or ring over a lower bar
        let r = length(vec2<f32>(u - 0.5, (v - 0.6) * 1.35));
        let ring = select(r - 0.3, abs(r - 0.26) - 0.06, h1 > 0.5);
        c = mix(c, ink, sl_cov(ring, soft));
        c = mix(c, ink, sl_cov(max(abs(u - 0.5) - 0.38, abs(v - 0.16) - 0.05), soft));
    } else if (kind < 0.55) {
        // a diagonal band and blocks
        let band = abs((u - 0.5) * 0.8 + (v - 0.55)) - 0.12;
        c = mix(c, ink, sl_cov(band, soft));
        c = mix(c, mix(ink, bg, 0.5), sl_cov(max(abs(u - 0.3) - 0.2, abs(v - 0.2) - 0.07), soft));
        c = mix(c, ink, sl_cov(max(abs(u - 0.5) - 0.4, abs(v - 0.88) - 0.04), soft));
    } else if (kind < 0.8) {
        // a photo-like block (soft blobs in muted tones) under a top bar
        let inside = sl_cov(max(abs(u - 0.5) - 0.42, abs(v - 0.45) - 0.3), soft);
        let blob = st_noise(vec2<f32>(u, v) * 5.0 + hd * 11.0) * 0.6 + st_noise(vec2<f32>(u, v) * 13.0 + hd * 3.0) * 0.4;
        let photo = mix(vec3<f32>(0.08, 0.07, 0.07), vec3<f32>(0.55, 0.45, 0.38), blob) * mix(vec3<f32>(1.0), bg * 1.4 + 0.2, 0.3);
        c = mix(c, photo, inside);
        c = mix(c, ink, sl_cov(max(abs(u - 0.5) - 0.42, abs(v - 0.86) - 0.06), soft));
        c = mix(c, ink, sl_cov(max(abs(u - 0.35) - 0.27, abs(v - 0.08) - 0.025), soft));
    } else {
        // halftone dots in a gradient, a border
        let g = fract(vec2<f32>(u, v) * 14.0) - 0.5;
        let rr = mix(0.05, 0.45, v);
        let dots = mix(rr * rr * 3.0, sl_cov(length(g) - rr, max(fw_u * 14.0, 0.08)), sl_keep(1.0 / 14.0, fw_u));
        c = mix(c, ink, clamp(dots, 0.0, 1.0) * 0.9);
        let border = min(min(u, 1.0 - u), min(v, 1.0 - v));
        c = mix(c, ink, sl_cov(abs(border - 0.05) - 0.012, soft));
    }
    return c;
}

// A wild posting: one to five copies of the same poster side by side, pasted
// over an older layer, torn (more at the bottom and the edges), sun-faded,
// wrinkled. Returns (colour, cover).
fn sl_posters(x: f32, y: f32, cw: f32, hs: f32, fw: f32) -> vec4<f32> {
    let h1 = st_hash(vec2<f32>(hs * 3.7, 9.3));
    let h2 = st_hash(vec2<f32>(hs * 8.3, 4.6));
    let h3 = st_hash(vec2<f32>(hs * 5.9, 7.2));
    let h4 = st_hash(vec2<f32>(hs * 12.9, 2.5));
    let pw = 0.28 + 0.3 * h2;
    let ph = pw * (1.3 + 0.12 * h3);
    let gap = 0.01 + 0.03 * h4;
    let n = max(min(1 + i32(h1 * 4.99), i32((cw - 0.2) / (pw + gap))), 1);
    let wid = f32(n) * (pw + gap) - gap;
    let x0 = 0.1 + max(cw - 0.2 - wid, 0.0) * fract(h1 * 7.7);
    let y0 = 0.7 + 0.75 * h3;
    let tx = x - x0;
    let ty = y - y0;
    if (tx < -0.05 || tx > wid + 0.05 || ty < -0.05 || ty > ph + 0.05) {
        return vec4<f32>(0.0);
    }
    let k = clamp(i32(floor(tx / (pw + gap))), 0, n - 1);
    let lx = tx - f32(k) * (pw + gap);
    let u = lx / pw;
    let v = ty / ph;
    let soft = max(fw, 0.002);
    let edge = max(abs(lx - pw * 0.5) - pw * 0.5, abs(ty - ph * 0.5) - ph * 0.5);
    // the paste's wet halo
    var a = sl_cov(edge - 0.008, soft) * 0.12;
    var c = vec3<f32>(0.0);
    if (edge < 0.004 + soft) {
        let hk = fract(hs * 17.0 + f32(k) * 0.618);
        // torn away: mostly from the edges and the bottom
        let tear_n = st_noise(vec2<f32>(u * 7.0, v * 7.0) + hk * 31.0) * 0.6 + st_noise(vec2<f32>(u * 23.0, v * 23.0) + hk * 7.0) * 0.4;
        let rim = min(min(u, 1.0 - u), min(v * 1.6, 1.0 - v));
        let torn = smoothstep(0.5, 0.56, tear_n - rim * (1.2 + 2.0 * hk) + 0.35 * h4);
        let design = sl_poster_design(u, v, hs + 0.37, fw / pw);
        // the older layer under it, or the white back of torn paper
        let old = select(sl_poster_design(fract(u + 0.3), fract(v + 0.2), hs + 0.91, fw / pw) * 0.85 + 0.06, vec3<f32>(0.62, 0.61, 0.57), hk > 0.5);
        c = mix(design, old, torn);
        // faded by the sun (greyer, lighter), yellowed, wrinkled
        let fade = 0.15 + 0.55 * h2 * h2;
        let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        c = mix(c, vec3<f32>(l) * 1.1 + vec3<f32>(0.09, 0.08, 0.05), fade);
        c = c * (0.9 + 0.1 * st_noise(vec2<f32>(lx * 45.0, ty * 4.0) + hs * 3.0));
        let cov = sl_cov(edge, soft) * select(1.0, 0.0, torn > 0.98 && hk > 0.75);
        a = max(a, cov);
    }
    return vec4<f32>(c, a);
}

// A sticker cluster (level 2): vinyl slaps at eye height, some peeling.
fn sl_stickers(x: f32, y: f32, cw: f32, hs: f32, fw: f32) -> vec4<f32> {
    let cx = 0.25 + (cw - 0.5) * st_hash(vec2<f32>(hs * 4.3, 1.9));
    let cy = 1.25 + 0.4 * st_hash(vec2<f32>(hs * 6.1, 7.3));
    if (abs(x - cx) > 0.3 || abs(y - cy) > 0.3) {
        return vec4<f32>(0.0);
    }
    var res = vec4<f32>(0.0);
    let n = 2 + i32(st_hash(vec2<f32>(hs * 2.3, 5.5)) * 6.99);
    let soft = max(fw, 0.001);
    for (var i = 0; i < n; i = i + 1) {
        let r1 = st_hash(vec2<f32>(f32(i) * 1.37 + hs * 9.0, 2.1));
        let r2 = st_hash(vec2<f32>(f32(i) * 2.71 + hs * 5.0, 7.7));
        let r3 = st_hash(vec2<f32>(f32(i) * 4.13 + hs * 3.0, 4.4));
        let c = vec2<f32>(cx + (r1 - 0.5) * 0.4, cy + (r2 - 0.5) * 0.35);
        let hsz = vec2<f32>(0.025 + 0.035 * r3, 0.02 + 0.025 * fract(r3 * 7.0));
        let q = sl_rot(vec2<f32>(x, y) - c, (r1 - 0.5) * 0.7);
        let d = length(max(abs(q) - hsz + 0.006, vec2<f32>(0.0))) - 0.006;
        if (d > soft) {
            continue;
        }
        var col = sl_spray(fract(r3 * 3.3)).rgb;
        if (r2 > 0.6) {
            col = vec3<f32>(0.70, 0.70, 0.68);
            // a shape on the white
            let inner = length(q / hsz) - 0.55;
            col = mix(col, sl_spray(fract(r1 * 5.1)).rgb, sl_cov(select(inner, abs(inner) - 0.12, r3 > 0.5), 0.05));
        }
        let peel = select(1.0, 0.45, fract(r1 * 9.7) < 0.2 && q.x > 0.0);
        res = vec4<f32>(mix(res.rgb, col, sl_cov(d, soft)), max(res.a, sl_cov(d, soft) * peel));
    }
    return res;
}

// Every wall piece at (`x` along the stretch from its start, `y` up), on a
// stretch `len` long, by the wall's heat (`tags`, 0..7: street_life.rs
// `Params`) and its posters, back to front: buffed-over patches, a field of
// old tags faded and half buffed (hot walls), pieces (the hottest), throw-ups,
// posters, a low and a high band of tags, stickers (level 2), and at its foot
// the heaps (`litter`, `deck`). Each on its own cell rhythm, every cell inside
// the stretch: nothing is cut. Level 2 draws a third more of each; a freeway
// column's base (`column`, its few metres of circumference) tighter cells and
// more of them, to 2-3 m up.
fn sl_wall_pieces(color: ptr<function, vec3<f32>>, rough: ptr<function, f32>, x: f32, y: f32, len: f32, tags: f32, posters: f32, litter: f32, deck: bool, column: bool, hseed: f32, fw: f32, lvl: u32) {
    if (x < 0.0 || x > len) {
        return;
    }
    let wall = *color;
    let lw = dot(wall, SL_LUMA);
    // a column's base (its circumference a few metres) is covered: tighter cells, more of them
    let cf = select(1.0, 0.7, column);
    let lb = select(1.0, 1.3, lvl >= 2u) * select(1.0, 1.4, column);
    // hot walls (warehouses, decks) carry larger work
    let sz = 1.0 + 0.08 * max(tags - 3.0, 0.0);
    // spray paint takes up a little of the wall's own texture
    let take = clamp(pow(max(lw, 1e-3) / 0.22, 0.25), 0.8, 1.2);
    // buffed-over patches, old tags ghosting through some
    {
        let cs = 4.4;
        let k = floor(x / cs);
        if ((k + 1.0) * cs <= len) {
            let hk = st_hash(vec2<f32>(k * 1.618 + 3.1, hseed * 1013.0));
            let hp = st_hash(vec2<f32>(k * 2.414 + 7.7, hseed * 811.0));
            if (hk < 0.06 + 0.05 * tags) {
                let b = sl_buff(x - k * cs, y, cs, hp, wall, fw);
                *color = mix(*color, b.rgb, b.a);
                *rough = mix(*rough, 0.72, b.a);
                if (hp > 0.6 && b.a > 0.0) {
                    var r0 = 0.4;
                    let g = sl_tag(x - k * cs, y, cs, hp + 0.31, 1.2, 0.4, 2.4, fw, &r0);
                    *color = mix(*color, g.rgb, g.a * 0.14 * b.a);
                }
            }
        }
    }
    // a field of old tags, faded and greyed, half buffed (hot walls)
    if (tags >= 3.0) {
        let cs = 1.25 * cf;
        let xo = x - 0.2;
        let k = floor(xo / cs);
        if (xo >= 0.0 && (k + 1.0) * cs + 0.2 <= len) {
            let hk = st_hash(vec2<f32>(k * 2.017 + 5.3, hseed * 659.0));
            if (hk < 0.12 * (tags - 2.0) * lb) {
                var r = 0.4;
                let t = sl_tag(xo - k * cs, y, cs, st_hash(vec2<f32>(k * 3.31 + 0.9, hseed * 491.0)), sz, 0.3, 2.6, fw, &r);
                let grey = mix(t.rgb, vec3<f32>(dot(t.rgb, SL_LUMA)) * 0.8 + lw * 0.35, 0.6);
                *color = mix(*color, grey * take, t.a * 0.55);
            }
        }
    }
    // pieces: the hottest walls
    if (tags >= 5.0) {
        let cs = 8.6;
        let xo = x - 0.7;
        let k = floor(xo / cs);
        if (xo >= 0.0 && (k + 1.0) * cs + 0.7 <= len) {
            let hk = st_hash(vec2<f32>(k * 1.211 + 2.9, hseed * 743.0));
            if (hk < 0.18 * (tags - 4.0) * lb) {
                var r = 0.4;
                let t = sl_piece(xo - k * cs, y, cs, st_hash(vec2<f32>(k * 2.93 + 6.1, hseed * 367.0)), 0.15, 2.9, fw, &r);
                *color = mix(*color, t.rgb * take, t.a);
                *rough = mix(*rough, r, t.a);
            }
        }
    }
    // throw-ups
    {
        let cs = 2.7 * select(1.0, 0.8, column);
        let xo = x - 1.3;
        let k = floor(xo / cs);
        if (xo >= 0.0 && (k + 1.0) * cs + 1.3 <= len) {
            let hk = st_hash(vec2<f32>(k * 2.236 + 1.3, hseed * 919.0));
            if (hk < (0.06 * tags + 0.06 * max(tags - 3.0, 0.0)) * lb) {
                var r = 0.4;
                let t = sl_throw(xo - k * cs, y, cs, st_hash(vec2<f32>(k * 1.732 + 8.3, hseed * 613.0)), sz, 0.1, select(2.2, 2.9, tags >= 5.0), fw, &r);
                *color = mix(*color, t.rgb * take, t.a);
                *rough = mix(*rough, r, t.a);
            }
        }
    }
    // wheat-pasted posters
    if (posters > 0.0) {
        let cs = 3.6;
        let xo = x - 0.5;
        let k = floor(xo / cs);
        if (xo >= 0.0 && (k + 1.0) * cs + 0.5 <= len) {
            let hk = st_hash(vec2<f32>(k * 1.618 + 3.1, hseed * 1013.0 + 7.0));
            if (hk < 0.08 * posters * lb) {
                let pc = sl_posters(xo - k * cs, y, cs, st_hash(vec2<f32>(k * 2.414 + 7.7, hseed * 811.0 + 3.0)), fw);
                *color = mix(*color, pc.rgb, pc.a);
                *rough = mix(*rough, 0.85, pc.a);
            }
        }
    }
    // tags: a low band at hand height, older ones faded
    if (tags > 0.0) {
        let cs = 1.5 * cf;
        let xo = x - 0.9;
        let k = floor(xo / cs);
        if (xo >= 0.0 && (k + 1.0) * cs + 0.9 <= len) {
            let hk = st_hash(vec2<f32>(k * 3.141 + 1.7, hseed * 577.0));
            if (hk < (0.1 + 0.12 * tags) * lb) {
                let hp = st_hash(vec2<f32>(k * 1.414 + 9.2, hseed * 383.0));
                var r = 0.4;
                let t = sl_tag(xo - k * cs, y, cs, hp, sz, 0.45, 1.9, fw, &r);
                let age = st_hash(vec2<f32>(hp * 7.0, 3.3));
                let tc = mix(t.rgb, vec3<f32>(dot(t.rgb, vec3<f32>(0.33))) * 0.9 + lw * 0.3, 0.45 * age * age);
                *color = mix(*color, tc * take, t.a * (1.0 - 0.35 * age * age));
                *rough = mix(*rough, r, t.a);
            }
        }
    }
    // ... and a high band over it (reached from a box, a ledge, a friend's shoulders)
    if (tags >= 2.0) {
        let cs = 1.75 * cf;
        let xo = x - 0.35;
        let k = floor(xo / cs);
        if (xo >= 0.0 && (k + 1.0) * cs + 0.35 <= len) {
            let hk = st_hash(vec2<f32>(k * 2.718 + 6.1, hseed * 433.0));
            if (hk < 0.1 * (tags - 1.0) * lb) {
                var r = 0.4;
                let t = sl_tag(xo - k * cs, y, cs, st_hash(vec2<f32>(k * 1.123 + 2.4, hseed * 997.0)), sz, 1.25, select(2.5, 3.2, tags >= 6.0 || column), fw, &r);
                *color = mix(*color, t.rgb * take, t.a);
                *rough = mix(*rough, r, t.a);
            }
        }
    }
    if (lvl >= 2u) {
        let cs = SL_CELL_C;
        let xo = x - 0.4;
        let k = floor(xo / cs);
        if (xo >= 0.0 && (k + 1.0) * cs + 0.4 <= len) {
            let hk = st_hash(vec2<f32>(k * 2.236 + 4.4, hseed * 709.0));
            if (hk < 0.04 * (tags + posters)) {
                let s = sl_stickers(xo - k * cs, y, cs, st_hash(vec2<f32>(k * 1.732 + 0.5, hseed * 211.0)), fw);
                *color = mix(*color, s.rgb, s.a);
                *rough = mix(*rough, 0.4, s.a);
            }
        }
    }
    // the heaps at the wall's foot: bags, a leaning sheet of cardboard, a mattress
    if (litter > 0.0 && y < 2.0) {
        let hp = sl_heap_at(x, len, litter, deck, hseed, lvl);
        if (hp.ok) {
            var r = 0.9;
            let h = sl_heap_wall(x, y, hp, fw, &r);
            *color = mix(*color, h.rgb, h.a);
            *rough = mix(*rough, r, h.a);
        }
    }
}

// The street-decal grid cell under a world point (cc0.wgsl's lookup): the
// slot (or none), the cell's eight indices.
struct SlCell {
    ok: bool,
    slot: u32,
    cell: vec4<u32>,
};

fn sl_cell(world: vec3<f32>) -> SlCell {
    var o: SlCell;
    o.ok = false;
    let tk = vec2<i32>(floor(world.xz / CC0_TILE_M));
    let sx = ((tk.x % CC0_SLOT_SIDE) + CC0_SLOT_SIDE) % CC0_SLOT_SIDE;
    let sz = ((tk.y % CC0_SLOT_SIDE) + CC0_SLOT_SIDE) % CC0_SLOT_SIDE;
    o.slot = u32(sx + CC0_SLOT_SIDE * sz);
    let hd = cc0_grid[o.slot];
    if (bitcast<i32>(hd.x) != tk.x || bitcast<i32>(hd.y) != tk.y || hd.w == 0u) {
        return o;
    }
    let local = world.xz - vec2<f32>(tk) * CC0_TILE_M;
    let ci = clamp(vec2<i32>(floor(local / (CC0_TILE_M / f32(CC0_CELLS)))), vec2<i32>(0), vec2<i32>(i32(CC0_CELLS) - 1));
    o.cell = sl_life_cell(o.slot, ci);
    o.ok = true;
    return o;
}

// A slot's street-life cell (cc0.rs `LIFE_GRID_VEC4`: after the decal grid, the same layout).
fn sl_life_cell(slot: u32, ci: vec2<i32>) -> vec4<u32> {
    return cc0_grid[SL_LIFE_BASE + slot * CC0_CELLS * CC0_CELLS + u32(ci.y) * CC0_CELLS + u32(ci.x)];
}

// The street fragment's life decals (cc0.wgsl `cc0_street_decals`, its tile's slot
// and cell already found): each holding it adds its litter, leaves, grit and marks.
fn sl_street_decals(world: vec3<f32>, cls: u32, slot: u32, ci: vec2<i32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let cell = sl_life_cell(slot, ci);
    for (var k = 0u; k < 8u; k = k + 1u) {
        let idx = (cell[k / 2u] >> ((k % 2u) * 16u)) & 0xffffu;
        if (idx == 0xffffu) {
            break;
        }
        let rec = (slot * CC0_MAX_DECALS + idx) * 2u;
        let d1 = cc0_decals[rec + 1u];
        let kind = u32(d1.z); // an exact integer (the f32's spacing reaches 1 at these kinds: no + 0.5)
        if ((kind & 255u) != SL_DK_LIFE) {
            continue;
        }
        let d0 = cc0_decals[rec];
        let q = world.xz - d0.xy;
        let ax = d0.zw;
        let along = dot(q, ax);
        let across = dot(q, vec2<f32>(-ax.y, ax.x));
        if (abs(along) > d1.x || abs(across) > d1.y) {
            continue;
        }
        sl_street(world, cls, along, across, d0, d1, kind >> SL_KIND_SHIFT, dpx, dpy);
    }
}

// A kit building's surface (draw.wgsl, after the facade ageing; every
// material but glass: render, paint, masonry, cladding, shutters, doors).
fn sl_kit_wall(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, object: u32, dpx: vec3<f32>, dpy: vec3<f32>) {
    let d = fa_drivers(fa_ages[objects[object].lod_group]);
    var c = (*s).albedo;
    var r = (*s).rough;
    sl_wall(&c, &r, world, n, world.y - d.base_y, dpx, dpy);
    (*s).albedo = c;
    (*s).rough = r;
}

// A wall: the wall pieces of the life decals whose building line it stands
// near (`SL_WALL_IN_M` in front of it at the most: plinths, recessed shop
// fronts, the footprint's own error) and faces. `local_y`: metres over the
// building's floor.
fn sl_wall(color: ptr<function, vec3<f32>>, rough: ptr<function, f32>, world: vec3<f32>, ng: vec3<f32>, local_y: f32, dpx: vec3<f32>, dpy: vec3<f32>) {
    if (local_y < -0.1 || local_y > SL_WALL_TOP_M || !cc0_on(CC0_RUN_DECALS) || length(frame.cam_pos.xyz - world) > SL_WALL_FAR_M) {
        return;
    }
    let nl = length(ng.xz);
    if (nl < 0.7) {
        return;
    }
    let nx = ng.xz / nl;
    let gc = sl_cell(world);
    if (!gc.ok) {
        return;
    }
    let fw = max(length(dpx), length(dpy));
    let lvl = sl_level();
    for (var k = 0u; k < 8u; k = k + 1u) {
        let idx = (gc.cell[k / 2u] >> ((k % 2u) * 16u)) & 0xffffu;
        if (idx == 0xffffu) {
            break;
        }
        let rec = (gc.slot * CC0_MAX_DECALS + idx) * 2u;
        let d1 = cc0_decals[rec + 1u];
        let kind = u32(d1.z); // an exact integer (the f32's spacing reaches 1 at these kinds: no + 0.5)
        if ((kind & 255u) != SL_DK_LIFE) {
            continue;
        }
        let pr = kind >> SL_KIND_SHIFT;
        let tags = f32((pr >> 8u) & 7u);
        let posters = f32((pr >> 11u) & 7u);
        let litter = f32(pr & 7u);
        if (((pr >> 15u) & 1u) == 0u || tags + posters + litter <= 0.0) {
            continue;
        }
        let d0 = cc0_decals[rec];
        let q = world.xz - d0.xy;
        let ax = d0.zw;
        let px = vec2<f32>(-ax.y, ax.x);
        let along = dot(q, ax);
        let across = dot(q, px);
        if (abs(along) > d1.x || abs(across) > d1.y || across < d1.y - SL_WALL_REACH_M - SL_WALL_IN_M) {
            continue;
        }
        // the wall along the stretch, facing the street
        if (abs(dot(nx, ax)) > 0.3 || dot(nx, px) > -0.7) {
            continue;
        }
        let len = 2.0 * (d1.x - SL_PAD_M);
        sl_wall_pieces(color, rough, along + d1.x - SL_PAD_M, local_y, len, tags, posters, litter, ((pr >> 14u) & 1u) == 1u, false, sl_dseed(d0), fw, lvl);
    }
}

// A freeway column (draw.wgsl kind 12, after sx_concrete): the tags,
// throw-ups and buffs round its base, measured from the street datum beside
// it (the decal's seed), on its own circumference.
fn sl_structure(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    if (!cc0_on(CC0_RUN_DECALS) || length(frame.cam_pos.xyz - world) > SL_WALL_FAR_M || abs(n.y) > 0.4) {
        return;
    }
    let gc = sl_cell(world);
    if (!gc.ok) {
        return;
    }
    let fw = max(length(dpx), length(dpy));
    let lvl = sl_level();
    for (var k = 0u; k < 8u; k = k + 1u) {
        let idx = (gc.cell[k / 2u] >> ((k % 2u) * 16u)) & 0xffffu;
        if (idx == 0xffffu) {
            break;
        }
        let rec = (gc.slot * CC0_MAX_DECALS + idx) * 2u;
        let d1 = cc0_decals[rec + 1u];
        let kind = u32(d1.z); // an exact integer (the f32's spacing reaches 1 at these kinds: no + 0.5)
        if ((kind & 255u) != SL_DK_COLUMN) {
            continue;
        }
        let y = world.y - d1.w;
        if (y < -0.2 || y > SL_WALL_TOP_M) {
            continue;
        }
        let d0 = cc0_decals[rec];
        let q = world.xz - d0.xy;
        let r = length(q);
        if (r > d1.x || r < 0.05 || dot(n.xz, q / r) < 0.6) {
            continue;
        }
        let pr = kind >> SL_KIND_SHIFT;
        let tags = f32((pr >> 8u) & 7u);
        // round the column from its far side (the cells' seam there), in metres
        let circ = 6.2832 * r;
        let x = (atan2(q.y, q.x) + 3.14159) / 6.2832 * circ;
        var c = (*s).albedo;
        var ro = (*s).rough;
        sl_wall_pieces(&c, &ro, x, y, circ, tags, 0.0, 0.0, true, true, sl_dseed(d0), fw, lvl);
        (*s).albedo = c;
        (*s).rough = ro;
    }
}
