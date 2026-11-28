// ---- the rear yards' ground (`--yard-detail`; yards.rs, eo-props yards.rs) ----
// The yard survey's classes (1 lawn, 2 planting, 3 paving, 4 deck) on the
// drawn plan, read where the terrain draws the open ground. Each class draws
// its tone (`YD_TONES`, yards.rs; after the aerial's colour ratio with the
// canopy map, `YdAt::ratio`) with its CC0 scans' detail round it, two
// scans height-blended under a slow field where a class has two, so a lawn
// has leafier patches and a bed its litter between the green. The lookup is
// rounded between the four nearest cells and nudged by a slow noise, so the
// classes meet along wandering edges, not the survey's 0.5 m squares.

@group(0) @binding(31) var<storage, read> yd_map: array<u32>;

const YD_HEAD: u32 = 4u;
const YD_MAP: u32 = 8u;

// What the yards' maps hold at a point: the class drawn (`c`, 0 none), the
// class it blends toward along an edge (`c2`) and its share (`t`, 0 to 0.5),
// the aerial's colour ratio (`ratio`, 1 without the canopy map) and the
// vegetation's height (`h`, m), both filtered between the four nearest cells.
struct YdAt {
    c: u32,
    c2: u32,
    t: f32,
    ratio: vec3<f32>,
    h: f32,
    // the drawn class's filtered share (1 inside it, about a half at its edge).
    own: f32,
};

// The four nearest cells' classes, each weighted bilinearly; the heaviest wins
// after `nudge` (0..1, a slow noise) shifts the balance between them, so two
// classes meet along a wandering line, and the runner-up blends in near it.
fn yd_at(p: vec2<f32>, nudge: f32) -> YdAt {
    var r: YdAt;
    r.c = 0u;
    r.c2 = 0u;
    r.t = 0.0;
    r.ratio = vec3<f32>(1.0);
    r.h = 0.0;
    r.own = 1.0;
    let maps = yd_map[0];
    for (var m = 0u; m < maps; m = m + 1u) {
        let h = YD_HEAD + m * YD_MAP;
        let o = vec2<f32>(bitcast<f32>(yd_map[h]), bitcast<f32>(yd_map[h + 1u]));
        let q = (p - o) * bitcast<f32>(yd_map[h + 2u]) - vec2<f32>(0.5);
        let nx = i32(yd_map[h + 4u]);
        let nz = i32(yd_map[h + 5u]);
        if (q.x < -0.5 || q.y < -0.5 || q.x >= f32(nx) - 0.5 || q.y >= f32(nz) - 0.5) {
            continue;
        }
        let canopy = yd_map[h + 7u];
        let b = floor(q);
        let f = q - b;
        var w = array<f32, 5>(0.0, 0.0, 0.0, 0.0, 0.0);
        var ratio = vec3<f32>(0.0);
        var rw = 0.0;
        var hv = 0.0;
        for (var corner = 0u; corner < 4u; corner = corner + 1u) {
            let dx = i32(corner & 1u);
            let dz = i32(corner >> 1u);
            let i = i32(b.x) + dx;
            let j = i32(b.y) + dz;
            let wc = select(1.0 - f.x, f.x, dx == 1) * select(1.0 - f.y, f.y, dz == 1);
            var c = 0u;
            if (i >= 0 && j >= 0 && i < nx && j < nz) {
                let k = u32(j * nx + i);
                c = min((yd_map[yd_map[h + 6u] + k / 8u] >> ((k % 8u) * 4u)) & 15u, 4u);
                if (canopy != 0u && c != 0u) {
                    let cw = yd_map[canopy + k];
                    ratio = ratio + wc * vec3<f32>(f32((cw >> 8u) & 255u), f32((cw >> 16u) & 255u), f32(cw >> 24u)) * (YD_RATIO_MAX / 255.0);
                    rw = rw + wc;
                    hv = hv + wc * f32(cw & 255u) * 0.1;
                }
            }
            w[c] = w[c] + wc;
        }
        // a laid surface (paving, a deck) keeps the edge its cells draw: the
        // filtered share's half-way line, straight along the survey's rows and
        // chamfered at its steps (a patio's edge is straight, not a wandering
        // line); planted edges wander (`nudge`).
        var hard = false;
        if (YD_CANOPY) {
            var pure = 0u;
            var pw = -1.0;
            var alt = 0u;
            var aw = -1.0;
            for (var c = 0u; c < 5u; c = c + 1u) {
                if (w[c] > pw) {
                    alt = pure;
                    aw = pw;
                    pure = c;
                    pw = w[c];
                } else if (w[c] > aw) {
                    alt = c;
                    aw = w[c];
                }
            }
            hard = pure >= 3u || (aw > 0.0 && alt >= 3u);
        }
        let wander = select(select(0.6, 0.45, YD_CANOPY), 0.0, hard);
        var best = 0u;
        var top = -1.0;
        var second = 0u;
        var next = -1.0;
        for (var c = 0u; c < 5u; c = c + 1u) {
            let v = w[c] + (fract(nudge + f32(c) * 0.37) - 0.5) * wander * step(1e-4, w[c]);
            if (w[c] > 0.0 && v > top) {
                second = best;
                next = top;
                top = v;
                best = c;
            } else if (w[c] > 0.0 && v > next) {
                second = c;
                next = v;
            }
        }
        r.c = best;
        r.own = w[best];
        if (YD_CANOPY && next > -0.5 && second != 0u && best != 0u) {
            r.c2 = second;
            r.t = 0.5 * (1.0 - smoothstep(0.0, select(0.35, 0.08, hard), top - next));
        }
        if (rw > 1e-4) {
            r.ratio = ratio / rw;
            r.h = hv / rw;
        }
        return r;
    }
    return r;
}

// A paved yard's scan: worn concrete with stained concrete through it, and a
// share of yards (cells of an 8 m jittered grid) laid in pavers or brick.
fn yd_paving(world: vec2<f32>) -> vec2<i32> {
    let g = world / 8.0;
    let cell = floor(g);
    var best = 1e9;
    var pick = 0.0;
    for (var dj = -1; dj <= 1; dj = dj + 1) {
        for (var di = -1; di <= 1; di = di + 1) {
            let c = cell + vec2<f32>(f32(di), f32(dj));
            let jit = vec2<f32>(st_hash(c + vec2<f32>(3.1, 7.7)), st_hash(c + vec2<f32>(9.2, 1.3)));
            let d = length(g - c - jit);
            if (d < best) {
                best = d;
                pick = st_hash(c + vec2<f32>(5.5, 2.2));
            }
        }
    }
    if (pick < 0.18) {
        return vec2<i32>(CC0_L_PATTERNED_CONCRETE_PAVERS, CC0_L_PATTERNED_CONCRETE_PAVERS);
    } else if (pick < 0.28) {
        return vec2<i32>(CC0_L_HERRINGBONE_PAVEMENT, CC0_L_HERRINGBONE_PAVEMENT);
    }
    return vec2<i32>(CC0_L_CONCRETE_FLOOR_WORN_001, CC0_L_DIRTY_CONCRETE);
}

// A bed's ground under its plants, by yard (cells of a 10 m jittered grid,
// as the paving's): two in five leaf litter over soil, three in ten dark bark
// mulch, a fifth an unkempt bed's dead grass and thatch through the litter, the
// rest gravel (shares INFERRED from the city's rear yards). Two cells meet
// along a wandering line (the lookup warped up to 2 m by a slow noise, half a
// metre by a fine one), not a straight one. The scans (x, y), the second's
// share (z) and the tone's factor on the litter's (the struct's `tint`).
struct YdBed {
    la: i32,
    lb: i32,
    share: f32,
    tint: vec3<f32>,
};

fn yd_bed(world: vec2<f32>) -> YdBed {
    let warp = vec2<f32>(st_noise(world * 0.3 + vec2<f32>(2.2, 6.6)), st_noise(world * 0.3 + vec2<f32>(8.1, 3.4))) * 4.0
        + vec2<f32>(st_noise(world * 1.9 + vec2<f32>(9.4, 0.8)), st_noise(world * 1.9 + vec2<f32>(5.2, 7.7))) * 1.0;
    let g = (world + warp - vec2<f32>(2.5)) / 10.0;
    let cell = floor(g);
    var best = 1e9;
    var pick = 0.0;
    for (var dj = -1; dj <= 1; dj = dj + 1) {
        for (var di = -1; di <= 1; di = di + 1) {
            let c = cell + vec2<f32>(f32(di), f32(dj));
            let jit = vec2<f32>(st_hash(c + vec2<f32>(6.3, 2.9)), st_hash(c + vec2<f32>(1.7, 8.4)));
            let d = length(g - c - jit);
            if (d < best) {
                best = d;
                pick = st_hash(c + vec2<f32>(4.6, 7.1));
            }
        }
    }
    var b: YdBed;
    if (pick < 0.4) {
        b = YdBed(CC0_L_FOREST_FLOOR, CC0_L_PARK_DIRT, 0.35, vec3<f32>(1.0));
    } else if (pick < 0.7) {
        // shredded bark: dark, a little red.
        b = YdBed(CC0_L_PARK_DIRT, CC0_L_FOREST_FLOOR, 0.25, vec3<f32>(0.78, 0.7, 0.64));
    } else if (pick < 0.9) {
        b = YdBed(CC0_L_FOREST_FLOOR, CC0_L_WITHERED_GRASS, 0.45, vec3<f32>(1.08, 1.08, 0.9));
    } else {
        b = YdBed(CC0_L_GRAVEL_FLOOR_03, CC0_L_PARK_DIRT, 0.25, vec3<f32>(1.35, 1.45, 1.6));
    }
    return b;
}

// A paved yard's wear, a factor on its tone: soil and leaf stain washed onto
// it along its edge (its class's share under 0.85), broad stains of damp and
// spills (a 2-6 m field), and on poured concrete (`poured`) the control joints
// sawn every 1.5 m both ways (a residential slab's usual spacing, INFERRED
// from the ACI rule of 24-36 times a 4 in slab's thickness), faded where a
// pixel spans a joint.
fn yd_slab(p: vec2<f32>, own: f32, dpx: vec3<f32>, dpy: vec3<f32>, poured: bool) -> f32 {
    let edge = mix(0.68, 1.0, smoothstep(0.4, 0.9, own));
    let stain = 0.84 + 0.2 * st_noise(p * 0.35 + vec2<f32>(4.2, 9.1)) + 0.08 * (st_noise(p * 1.7 + vec2<f32>(1.3, 2.7)) - 0.5);
    var joint = 1.0;
    if (poured) {
        let g = p / 1.5;
        let d = min(abs(fract(g.x + 0.5) - 0.5), abs(fract(g.y + 0.5) - 0.5)) * 1.5;
        let fw = length(dpx.xz) + length(dpy.xz);
        let line = (1.0 - smoothstep(0.004, 0.004 + fw, d)) * (1.0 - smoothstep(0.02, 0.06, fw));
        joint = mix(1.0, 0.5, line);
    }
    return edge * stain * joint * 0.9;
}

// Draw the yard's ground at `world` and say so; false: not a yard (or off), the terrain as before.
fn yd_ground(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) -> bool {
    if (!YD_ON || !cc0_on(CC0_RUN_GROUND)) {
        return false;
    }
    let nudge = st_noise(world.xz * 0.9 + vec2<f32>(7.1, 1.7)) * 0.7 + st_noise(world.xz * 2.7 + vec2<f32>(2.9, 5.3)) * 0.3;
    let at = yd_at(world.xz, nudge);
    let c = at.c;
    if (c == 0u || c > 4u) {
        return false;
    }
    let dist = length(frame.cam_pos.xyz - world);
    let near = 1.0 - smoothstep(130.0, 190.0, dist);
    // nearer than YD_PLANTS_M (yards.rs `plants_m`) the planting under the yards'
    // shrubs and trees is litter; past it, the canopy's shade (the planting tone).
    let plants = 1.0 - smoothstep(YD_PLANTS_M.x, YD_PLANTS_M.y, dist);
    let under = select(0.0, smoothstep(0.4, 0.9, at.h), c == 2u) * plants;
    var la = CC0_L_GRASS004;
    var lb = CC0_L_LEAFY_GRASS;
    var share = 0.25; // of the second scan
    var scale = 9.0; // its patches' size, m
    var bed = YdBed(CC0_L_FOREST_FLOOR, CC0_L_PARK_DIRT, 0.35, vec3<f32>(1.0));
    if (c == 2u && YD_CANOPY) {
        bed = yd_bed(world.xz);
    }
    if (c == 2u) {
        if (under > 0.5) {
            la = bed.la;
            lb = bed.lb;
            share = bed.share;
            scale = 2.4;
        } else {
            la = CC0_L_LEAFY_GRASS;
            lb = CC0_L_FOREST_FLOOR;
            share = 0.3;
            scale = 1.6;
        }
    } else if (c == 3u && !YD_CANOPY) {
        la = CC0_L_CONCRETE_FLOOR_WORN_001;
        lb = la;
        share = 0.0;
    } else if (c == 3u) {
        let pv = yd_paving(world.xz);
        la = pv.x;
        lb = pv.y;
        share = select(0.0, 0.35, pv.x != pv.y);
        scale = 4.0;
    } else if (c == 4u) {
        la = CC0_L_PLANKS010;
        lb = la;
        share = 0.0;
    }
    let field = st_noise(world.xz / scale + vec2<f32>(3.3, 8.1)) * 0.7 + st_noise(world.xz / (scale * 0.37) + vec2<f32>(1.9, 4.4)) * 0.3;
    let w0 = (field - (1.0 - share)) * 10.0 + 0.5;
    var ta: Cc0Tap;
    var tb: Cc0Tap;
    var w = 0.0;
    if (share <= 0.0 || w0 <= -1.0) {
        ta = cc0_ground_tap(world, dpx, dpy, la, 0.0, false);
        tb = ta;
    } else if (w0 >= 2.0) {
        tb = cc0_ground_tap(world, dpx, dpy, lb, 0.37, false);
        ta = tb;
        w = 1.0;
    } else {
        ta = cc0_ground_tap(world, dpx, dpy, la, 0.0, false);
        tb = cc0_ground_tap(world, dpx, dpy, lb, 0.37, false);
        w = clamp(w0 + (tb.h - ta.h), 0.0, 1.0);
    }
    // the class's tone after the aerial's ratio, blended toward the runner-up's
    // along an edge; litter under the plants near the camera.
    var tones = YD_TONES;
    if (!YD_CANOPY) {
        (*s).albedo = tones[c - 1u];
        cc0_ground_apply(s, n, mix(ta.alb.rgb, tb.alb.rgb, w), mix(ta.rough, tb.rough, w), mix(ta.ao, tb.ao, w), mix(ta.n, tb.n, w) * near, 0.9);
        return true;
    }
    // the ratio's brightness, and a third of its hue (the aerial's colour is
    // greyed and shifted by haze and compression; its brightness is the yard's).
    let rl = clamp(dot(at.ratio, CC0_LUMA), 0.55, 1.7);
    let ratio = rl * clamp(mix(vec3<f32>(1.0), at.ratio / max(dot(at.ratio, CC0_LUMA), 1e-3), 0.35), vec3<f32>(0.8), vec3<f32>(1.25));
    // (a laid surface keeps nearer its own tone: the aerial's brightest paving is glare.)
    var tone = tones[c - 1u] * select(ratio, clamp(ratio, vec3<f32>(0.6), vec3<f32>(1.2)), c >= 3u);
    if (at.c2 != 0u) {
        tone = mix(tone, tones[at.c2 - 1u] * ratio, at.t);
    }
    tone = mix(tone, YD_LITTER * bed.tint * mix(vec3<f32>(1.0), ratio, 0.3), under);
    if (c == 3u) {
        tone = tone * yd_slab(world.xz, at.own, dpx, dpy, la == CC0_L_CONCRETE_FLOOR_WORN_001);
    }
    (*s).albedo = tone;
    var rel = mix(ta.alb.rgb, tb.alb.rgb, w);
    if (c == 2u) {
        // (the litter scans' fallen leaves are an autumn wood's orange: half their chroma
        // under a garden's evergreens.)
        rel = mix(vec3<f32>(dot(rel, CC0_LUMA)), rel, mix(1.0, 0.5, under));
    }
    cc0_ground_apply(s, n, rel, mix(ta.rough, tb.rough, w), mix(ta.ao, tb.ao, w), mix(ta.n, tb.n, w) * near, 0.9);
    return true;
}
