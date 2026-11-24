// Shop rooms by trade (`--shop-light`, stage N2; shop_lights.rs `Front`,
// eo-sky shops.rs `Fit`): what a lit shop shows through its glass. Each shop
// is one room behind every pane of its tenancy (its door and its windows look
// into the same room), filled by its trade: gondola runs and wall shelving
// stacked with goods (markets, liquor, pharmacies, hardware, books, flowers),
// clothing racks and a display table, tables and chairs under pendants with a
// counter and a lit kitchen pass, a bar with stools and a back bar of
// bottles, salon chairs facing a mirror wall, washers and stacked dryers, a
// gallery's framed pieces, a clinic's reception, a cleaner's bagged rails, a
// gym's machines; a back wall in the trade's colour; ceiling fixtures (strips,
// troffers, downlights, track spots, pendants) and the light falling off
// toward the back and the corners; now and then a person. Goods, garments and
// pictures are colour fields and plain shapes: nothing here draws a letter,
// a word, a number or a mark.
//
// Everything below is a pure function of the room and the ray (no binding
// but `lx_shop_fronts` in `sr_front`), so a test can draw it alone
// (tests/shop_rooms.rs). The room's frame: x along the front from the shop's
// start (0..W), y up from its floor (0..H), z into the room from its glass
// (0..D). Radiance comes out per unit of the window's own (rooms.wgsl
// multiplies by its light's), its mean about the window's luminance with the
// fixtures well over it and the corners under it; by day an albedo the pane
// takes (rooms.wgsl `RmOut.day`).

const SR_M_NONE: u32 = 0u;
const SR_M_SHELF: u32 = 1u;   // goods on its faces
const SR_M_RACK: u32 = 2u;    // garments on a rail
const SR_M_WOOD: u32 = 3u;    // counters, tables
const SR_M_DARK: u32 = 4u;    // chairs, stools, posts
const SR_M_WHITE: u32 = 5u;   // washers and dryers: enamel, round doors on their room faces
const SR_M_LAMP: u32 = 6u;    // a pendant: its underside glows
const SR_M_TILL: u32 = 7u;    // a till: its screen glows
const SR_M_CASE: u32 = 8u;    // a counter with a lit display case in its front
const SR_M_BAGS: u32 = 9u;    // garments in clear bags
const SR_M_CARTON: u32 = 10u; // cardboard (a vacant shop)
const SR_M_MACHINE: u32 = 11u;
const SR_M_SOFT: u32 = 12u;   // upholstery, plinths

struct SrRoom {
    // W, H, D (m)
    size: vec3<f32>,
    fit: u32,
    goods: u32,
    key: u32,
    vacant: bool,
    // a vacant shop's work light
    work: bool,
    // the shop's kind (eo-sky `ShopKind` as u32: 1 a cafe, 3 liquor, 4 a market...)
    kind: u32,
};

// (`--room-depth`, rooms.wgsl `rm_shop_pane` sets it for one room, then
// restores it) the share of the window-adapted level the lit surfaces show at
// by night; 1 draws them as before.
var<private> sr_surface: f32 = 1.0;

struct SrOut {
    // per unit of the window's radiance (night), and the albedo the pane takes (day)
    night: vec3<f32>,
    day: vec3<f32>,
};

struct SrBox {
    lo: vec3<f32>,
    hi: vec3<f32>,
    mat: u32,
    id: u32,
};

struct SrFront {
    found: bool,
    // the room's span along the front (m, from the shop's anchor on its glass)
    s0: f32,
    s1: f32,
    // the pane in the room's frame: along the front from the room's start, behind the glass
    x: f32,
    z: f32,
    floor: f32,
    head: f32,
    fit: u32,
    goods: u32,
    kind: u32,
    vacant: bool,
    boarded: bool,
    work: bool,
    key: u32,
    // its lit window's luminance (cd/m2)
    lum: f32,
};

// The factor a shop's room is seen at for its window's luminance `lum`
// (shop_lights.rs `room_adapt`: the eye adapts to each lit window).
fn sr_adapt(lum: f32) -> f32 {
    return pow(SR_ADAPT_REF / clamp(lum, 1.0, 255.0), 1.0 - SR_ADAPT_POWER);
}

// The shop front a pane at `pos` (outward normal `n`) of LOD group `group`
// belongs to (shop_lights.rs `fronts_table`, `front_at`: the same steps): in
// the front's own frame (its anchor on its glass, its normal), so the pane's
// place along the front holds to the centimetre anywhere in the city.
fn sr_front(group: u32, pos: vec3<f32>, n: vec3<f32>) -> SrFront {
    var f: SrFront;
    f.found = false;
    let h = lx_shop_fronts[0];
    if (h.y == 0u || group < h.x || group >= h.x + h.y) {
        return f;
    }
    let k = group - h.x;
    let e = lx_shop_fronts[1u + k / 4u][k % 4u];
    let first = e >> 8u;
    let count = e & 255u;
    let nh = n.xz / max(length(n.xz), 1e-6);
    for (var r = 0u; r < count; r = r + 1u) {
        let a = lx_shop_fronts[h.z + 2u * (first + r)];
        let b = lx_shop_fronts[h.z + 2u * (first + r) + 1u];
        let turn = f32(a.w & 0xffffu) / SR_HEADING_STEPS * 6.2831853;
        let rn = vec2<f32>(cos(turn), sin(turn));
        if (dot(rn, nh) < SR_FACING) {
            continue;
        }
        let q = pos.xz - vec2<f32>(bitcast<f32>(a.x), bitcast<f32>(a.y));
        let s = dot(q, vec2<f32>(-rn.y, rn.x));
        let off = dot(q, rn);
        let s0 = bitcast<f32>(b.x);
        let s1 = bitcast<f32>(b.y);
        if (abs(off) > SR_PLANE || s < s0 || s > s1) {
            continue;
        }
        f.found = true;
        f.s0 = s0;
        f.s1 = s1;
        f.x = s - s0;
        f.z = -off;
        f.floor = bitcast<f32>(a.z);
        f.head = f32(a.w >> 16u) / 100.0;
        f.fit = b.z & 15u;
        f.goods = (b.z >> 4u) & 15u;
        f.kind = (b.z >> 8u) & 15u;
        f.vacant = ((b.z >> 12u) & 1u) != 0u;
        f.boarded = ((b.z >> 13u) & 1u) != 0u;
        f.work = ((b.z >> 14u) & 1u) != 0u;
        f.key = b.w;
        f.lum = f32((b.z >> 16u) & 255u);
        return f;
    }
    return f;
}

// ---- hashing ----

fn sr_u(x: u32) -> u32 {
    var v = x * 747796405u + 2891336453u;
    v = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    return (v >> 22u) ^ v;
}

// A number in [0, 1) for the shop `key`, a `salt` and an index.
fn sr_r(key: u32, salt: u32, i: i32) -> f32 {
    return f32(sr_u(key ^ sr_u(salt * 0x9e3779b9u + sr_u(bitcast<u32>(i)))) >> 8u) / 16777216.0;
}

fn sr_noise(p: vec2<f32>, key: u32, salt: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let ix = i32(i.x);
    let iy = i32(i.y);
    let a = sr_r(key, salt, ix * 7919 + iy * 104729);
    let b = sr_r(key, salt, (ix + 1) * 7919 + iy * 104729);
    let c = sr_r(key, salt, ix * 7919 + (iy + 1) * 104729);
    let d = sr_r(key, salt, (ix + 1) * 7919 + (iy + 1) * 104729);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// A pattern of `period` (m) seen at a pixel footprint `fp` (m): itself up close, its mean beyond.
fn sr_keep(period: f32, fp: f32) -> f32 {
    return 1.0 - smoothstep(period * 0.3, period * 1.2, fp);
}

// A step at `edge` antialiased over the footprint.
fn sr_step(edge: f32, x: f32, fp: f32) -> f32 {
    return clamp((x - edge) / max(fp, 1e-4) + 0.5, 0.0, 1.0);
}

// ---- palettes (linear albedo) ----

// Packaged goods: printed card and film, mostly mid tones with saturated runs.
fn sr_goods_col(goods: u32, h: f32, g: f32) -> vec3<f32> {
    if (goods == 1u) {
        // bottles: amber, green, clear, dark red wine, a few bright labels' glass
        let c = select(select(select(vec3<f32>(0.32, 0.16, 0.04), vec3<f32>(0.08, 0.20, 0.08), h > 0.35), vec3<f32>(0.42, 0.44, 0.42), h > 0.6), vec3<f32>(0.16, 0.03, 0.04), h > 0.82);
        return c * (0.75 + 0.5 * g);
    }
    if (goods == 2u) {
        // a pharmacy: white boxes, pastel and blue packs
        let c = select(select(vec3<f32>(0.80, 0.80, 0.78), vec3<f32>(0.42, 0.55, 0.75), h > 0.5), vec3<f32>(0.78, 0.55, 0.62), h > 0.8);
        return c * (0.8 + 0.3 * g);
    }
    if (goods == 3u) {
        // hardware: orange, grey, red, black, yellow, galvanised
        let c = select(select(select(select(vec3<f32>(0.70, 0.28, 0.05), vec3<f32>(0.36, 0.37, 0.38), h > 0.25), vec3<f32>(0.55, 0.08, 0.06), h > 0.5), vec3<f32>(0.05, 0.05, 0.05), h > 0.65), vec3<f32>(0.75, 0.62, 0.08), h > 0.85);
        return c * (0.75 + 0.4 * g);
    }
    if (goods == 4u) {
        // books and records: muted spines
        let c = mix(vec3<f32>(0.35, 0.10, 0.08), vec3<f32>(0.10, 0.16, 0.30), h);
        return mix(c, vec3<f32>(0.62, 0.58, 0.48), step(0.7, g)) * (0.6 + 0.6 * g);
    }
    if (goods == 5u) {
        // flowers in buckets: leaves and blooms
        let c = select(select(select(vec3<f32>(0.06, 0.20, 0.05), vec3<f32>(0.75, 0.16, 0.35), h > 0.55), vec3<f32>(0.80, 0.70, 0.12), h > 0.72), vec3<f32>(0.85, 0.82, 0.78), h > 0.88);
        return c * (0.7 + 0.5 * g);
    }
    // groceries: red, yellow, blue, green, white, brown, orange, black runs, each its own shade
    let c = select(select(select(select(select(select(select(vec3<f32>(0.55, 0.09, 0.07), vec3<f32>(0.72, 0.56, 0.12), h > 0.14), vec3<f32>(0.12, 0.22, 0.48), h > 0.27), vec3<f32>(0.14, 0.36, 0.15), h > 0.4), vec3<f32>(0.76, 0.74, 0.68), h > 0.52), vec3<f32>(0.36, 0.22, 0.11), h > 0.68), vec3<f32>(0.70, 0.32, 0.08), h > 0.8), vec3<f32>(0.06, 0.06, 0.07), h > 0.9);
    let tint = vec3<f32>(fract(h * 7.31), fract(h * 3.17), fract(h * 5.71)) - 0.5;
    return max(c + tint * 0.12, vec3<f32>(0.02)) * (0.65 + 0.5 * g);
}

// Clothes: blacks, navy, greys, denim, white, beige, now and then a colour.
fn sr_cloth_col(h: f32, g: f32) -> vec3<f32> {
    let c = select(select(select(select(select(vec3<f32>(0.03, 0.03, 0.035), vec3<f32>(0.05, 0.07, 0.16), h > 0.22), vec3<f32>(0.25, 0.25, 0.26), h > 0.38), vec3<f32>(0.13, 0.20, 0.34), h > 0.52), vec3<f32>(0.72, 0.70, 0.66), h > 0.66), vec3<f32>(0.48, 0.38, 0.26), h > 0.8);
    let accent = mix(vec3<f32>(0.55, 0.07, 0.08), vec3<f32>(0.20, 0.40, 0.25), g);
    return select(c, accent, h > 0.93) * (0.8 + 0.4 * g);
}

// The shop's own wall colour (its trade's palette, a share of shops white).
fn sr_wall_col(fit: u32, key: u32) -> vec3<f32> {
    let h = sr_r(key, 20u, 0);
    let g = sr_r(key, 21u, 0);
    if (fit == 3u) {
        // a bar: dark wood panelling, oxblood, deep green, brick
        return select(select(select(vec3<f32>(0.20, 0.11, 0.06), vec3<f32>(0.34, 0.09, 0.07), h > 0.35), vec3<f32>(0.10, 0.24, 0.14), h > 0.6), vec3<f32>(0.36, 0.16, 0.09), h > 0.8) * (0.85 + 0.3 * g);
    }
    if (fit == 2u) {
        // dining: warm plaster, terracotta, sage, white tile, dark wood
        return select(select(select(vec3<f32>(0.62, 0.50, 0.36), vec3<f32>(0.48, 0.20, 0.10), h > 0.35), vec3<f32>(0.34, 0.40, 0.28), h > 0.55), vec3<f32>(0.78, 0.76, 0.70), h > 0.78);
    }
    if (fit == 6u || fit == 7u || fit == 5u) {
        return vec3<f32>(0.74, 0.74, 0.72) * (0.92 + 0.1 * g);
    }
    // shops: white mostly, a pale tint, a trade colour (blue, green, yellow, red) on a few
    let trade = select(select(select(vec3<f32>(0.10, 0.25, 0.55), vec3<f32>(0.15, 0.42, 0.20), h > 0.85), vec3<f32>(0.80, 0.60, 0.12), h > 0.9), vec3<f32>(0.55, 0.10, 0.08), h > 0.95);
    let pale = mix(vec3<f32>(0.78, 0.77, 0.74), trade, 0.25);
    return select(select(vec3<f32>(0.72, 0.71, 0.68), pale, h > 0.6), trade, h > 0.8) * (0.9 + 0.15 * g);
}

// ---- the room's fittings ----

fn sr_box(lo: vec3<f32>, hi: vec3<f32>, mat: u32, id: u32) -> SrBox {
    var b: SrBox;
    b.lo = lo;
    b.hi = hi;
    b.mat = mat;
    b.id = id;
    return b;
}

// How many fittings the room has (`sr_obj` draws them).
fn sr_count(r: SrRoom) -> u32 {
    if (r.vacant) {
        return 2u;
    }
    switch r.fit {
        case 0u: { return 6u; }   // counter, till, up to four gondolas
        case 1u: { return 7u; }   // table, counter, till, up to four racks
        case 2u: { return 38u; }  // counter, till, four tables of nine parts (top, post, two chairs of three, a pendant)
        case 3u: { return 23u; }  // counter, six stools, two high tables, three pendants and their cords
        case 4u: { return 15u; }  // shelf, desk, till, four chairs of three parts
        case 5u: { return 4u; }   // washers, island, dryers, folding table
        case 6u: { return 3u; }
        case 7u: { return 4u; }
        case 8u: { return 4u; }
        default: { return 17u; }  // a gym: four treadmills of four parts, a bench
    }
    return 0u;
}

// Fitting `i` of the room (empty: lo > hi).
fn sr_obj(r: SrRoom, i: u32) -> SrBox {
    let W = r.size.x;
    let H = r.size.y;
    let D = r.size.z;
    let k = r.key;
    let none = sr_box(vec3<f32>(1.0), vec3<f32>(0.0), SR_M_NONE, i);
    let right = sr_r(k, 30u, 0) < 0.5;
    // mirror x to the room's other side
    let mx = select(0.0, W, right);
    let sx = select(1.0, -1.0, right);
    if (r.vacant) {
        // a couple of cartons left on the floor
        let x = W * (0.25 + 0.5 * sr_r(k, 31u, i32(i)));
        let z = D * (0.3 + 0.4 * sr_r(k, 32u, i32(i)));
        let s = vec3<f32>(0.45, 0.35, 0.4) * (0.7 + 0.6 * sr_r(k, 33u, i32(i)));
        return sr_box(vec3<f32>(x - s.x, 0.0, z - s.z), vec3<f32>(x + s.x, s.y * 2.0, z + s.z), SR_M_CARTON, i);
    }
    switch r.fit {
        case 0u: {
            // shelves: a checkout counter along one side near the door, gondola runs into the room
            let cz1 = min(0.9 + 1.4 + 0.6 * sr_r(k, 34u, 0), D * 0.45);
            if (i == 0u) {
                let a = mx + sx * 0.85;
                let b = mx + sx * 1.45;
                return sr_box(vec3<f32>(min(a, b), 0.0, 0.9), vec3<f32>(max(a, b), 0.98, cz1), SR_M_WOOD, i);
            }
            if (i == 1u) {
                let cx = mx + sx * 1.15;
                return sr_box(vec3<f32>(cx - 0.17, 0.98, 1.05), vec3<f32>(cx + 0.17, 1.24, 1.36), SR_M_TILL, i);
            }
            let a = select(0.25, 1.75, !right);
            let b = select(W - 0.25, W - 1.75, right);
            let span = b - a;
            let pitch = 1.85 + 0.35 * sr_r(k, 35u, 0);
            let n = u32(clamp(floor((span + 0.6) / pitch), 0.0, 4.0));
            let g = i - 2u;
            let z0 = 2.0 + 0.6 * sr_r(k, 36u, 0);
            if (g >= n || span < 1.0 || D - 1.4 - z0 < 1.0) {
                return none;
            }
            let xc = (a + b) * 0.5 + (f32(g) - (f32(n) - 1.0) * 0.5) * pitch;
            let hg = 1.4 + 0.35 * sr_r(k, 37u, 0);
            return sr_box(vec3<f32>(xc - 0.45, 0.0, z0), vec3<f32>(xc + 0.45, hg, D - 1.4), SR_M_SHELF, i);
        }
        case 1u: {
            // racks: a display table by the window, a counter at the back, racks in rows
            if (i == 0u) {
                let tw = min(1.5, W - 1.4);
                let tx = W * (0.35 + 0.3 * sr_r(k, 40u, 0));
                if (tw < 0.8) {
                    return none;
                }
                return sr_box(vec3<f32>(tx - tw * 0.5, 0.0, 1.0), vec3<f32>(tx + tw * 0.5, 0.78, 1.85), SR_M_WOOD, i);
            }
            if (i == 1u) {
                let a = mx + sx * 0.4;
                let b = mx + sx * 2.1;
                return sr_box(vec3<f32>(min(a, b), 0.0, D - 2.3), vec3<f32>(max(a, b), 1.0, D - 1.65), SR_M_WOOD, i);
            }
            if (i == 2u) {
                let cx = mx + sx * 1.2;
                return sr_box(vec3<f32>(cx - 0.17, 1.0, D - 2.15), vec3<f32>(cx + 0.17, 1.26, D - 1.85), SR_M_TILL, i);
            }
            let g = i - 3u;
            let row = g / 2u;
            let col = g % 2u;
            let zr = 2.6 + f32(row) * 1.9;
            let two = W > 3.6;
            if (zr + 0.3 > D - 2.6 || (col == 1u && !two)) {
                return none;
            }
            let len = select(min(1.7, W - 1.2), min(1.6, W * 0.5 - 0.5), two);
            let xc = select(W * 0.5, W * (0.27 + 0.46 * f32(col)), two) + (sr_r(k, 41u, i32(g)) - 0.5) * 0.3;
            let hem = 0.5 + 0.35 * sr_r(k, 42u, i32(g));
            return sr_box(vec3<f32>(xc - len * 0.5, hem, zr - 0.28), vec3<f32>(xc + len * 0.5, 1.5, zr + 0.28), SR_M_RACK, i);
        }
        case 2u: {
            // dining: the counter at the back (a lit case in a cafe's), tables with two chairs and a pendant
            if (i == 0u) {
                return sr_box(vec3<f32>(0.5, 0.0, D - 2.0), vec3<f32>(W - 0.5, 1.05, D - 1.35), select(SR_M_WOOD, SR_M_CASE, r.kind == 1u), i);
            }
            if (i == 1u) {
                let cx = W * (0.25 + 0.5 * sr_r(k, 50u, 0));
                return sr_box(vec3<f32>(cx - 0.17, 1.05, D - 1.85), vec3<f32>(cx + 0.17, 1.31, D - 1.55), SR_M_TILL, i);
            }
            let g = (i - 2u) / 9u;
            let part = (i - 2u) % 9u;
            let cols = u32(clamp(floor((W - 0.4) / 1.75), 1.0, 2.0));
            let rows = u32(clamp(floor((D - 3.4) / 1.9), 1.0, 2.0));
            let c = g % 2u;
            let rr = g / 2u;
            if (c >= cols || rr >= rows) {
                return none;
            }
            let xc = W * (f32(c) + 0.5) / f32(cols) + (sr_r(k, 51u, i32(g)) - 0.5) * 0.25;
            let zc = 1.6 + f32(rr) * 1.95 + (sr_r(k, 52u, i32(g)) - 0.5) * 0.2;
            let ts = 0.33 + 0.07 * sr_r(k, 53u, 0);
            switch part {
                case 0u: { return sr_box(vec3<f32>(xc - ts, 0.72, zc - ts), vec3<f32>(xc + ts, 0.76, zc + ts), SR_M_WOOD, i); }
                case 1u: { return sr_box(vec3<f32>(xc - 0.04, 0.0, zc - 0.04), vec3<f32>(xc + 0.04, 0.72, zc + 0.04), SR_M_DARK, i); }
                // two chairs facing the table: a seat, a back rest over it, a leg frame under it
                case 2u: { return sr_box(vec3<f32>(xc - ts - 0.42, 0.43, zc - 0.2), vec3<f32>(xc - ts - 0.04, 0.47, zc + 0.2), SR_M_DARK, i); }
                case 3u: { return sr_box(vec3<f32>(xc - ts - 0.46, 0.6, zc - 0.19), vec3<f32>(xc - ts - 0.42, 0.9, zc + 0.19), SR_M_DARK, i); }
                case 4u: { return sr_box(vec3<f32>(xc - ts - 0.4, 0.0, zc - 0.17), vec3<f32>(xc - ts - 0.06, 0.43, zc - 0.15), SR_M_DARK, i); }
                case 5u: { return sr_box(vec3<f32>(xc + ts + 0.04, 0.43, zc - 0.2), vec3<f32>(xc + ts + 0.42, 0.47, zc + 0.2), SR_M_DARK, i); }
                case 6u: { return sr_box(vec3<f32>(xc + ts + 0.42, 0.6, zc - 0.19), vec3<f32>(xc + ts + 0.46, 0.9, zc + 0.19), SR_M_DARK, i); }
                case 7u: { return sr_box(vec3<f32>(xc + ts + 0.06, 0.0, zc - 0.17), vec3<f32>(xc + ts + 0.4, 0.43, zc - 0.15), SR_M_DARK, i); }
                default: { return sr_box(vec3<f32>(xc - 0.12, H - 1.2, zc - 0.12), vec3<f32>(xc + 0.12, H - 1.04, zc + 0.12), SR_M_LAMP, i); }
            }
        }
        case 3u: {
            // a bar: the counter along one side (or across the back of a narrow room), stools, high tables, pendants
            let along = W >= 3.6;
            if (i == 0u) {
                if (along) {
                    let a = mx + sx * (W - 1.55);
                    let b = mx + sx * (W - 0.95);
                    return sr_box(vec3<f32>(min(a, b), 0.0, 1.6), vec3<f32>(max(a, b), 1.08, D - 1.0), SR_M_WOOD, i);
                }
                return sr_box(vec3<f32>(0.4, 0.0, D - 2.2), vec3<f32>(W - 0.4, 1.08, D - 1.6), SR_M_WOOD, i);
            }
            if (i <= 12u) {
                let s = (i - 1u) / 2u;
                let n = u32(clamp(floor((D - 2.8) / 0.75), 2.0, 6.0));
                if (s >= n) {
                    return none;
                }
                var p = vec2<f32>(mx + sx * (W - 1.95), 2.0 + f32(s) * 0.75);
                if (!along) {
                    p = vec2<f32>(0.7 + f32(s) * 0.75, D - 2.6);
                    if (p.x > W - 0.6) {
                        return none;
                    }
                }
                if ((i - 1u) % 2u == 0u) {
                    return sr_box(vec3<f32>(p.x - 0.18, 0.74, p.y - 0.18), vec3<f32>(p.x + 0.18, 0.79, p.y + 0.18), SR_M_DARK, i);
                }
                return sr_box(vec3<f32>(p.x - 0.03, 0.0, p.y - 0.03), vec3<f32>(p.x + 0.03, 0.74, p.y + 0.03), SR_M_DARK, i);
            }
            if (i <= 16u) {
                let t = (i - 13u) / 2u;
                let tp = vec2<f32>(mx + sx * 0.75, 2.0 + f32(t) * 1.7);
                if (!along || tp.y > D - 1.8) {
                    return none;
                }
                if ((i - 13u) % 2u == 0u) {
                    return sr_box(vec3<f32>(tp.x - 0.3, 1.0, tp.y - 0.3), vec3<f32>(tp.x + 0.3, 1.04, tp.y + 0.3), SR_M_WOOD, i);
                }
                return sr_box(vec3<f32>(tp.x - 0.04, 0.0, tp.y - 0.04), vec3<f32>(tp.x + 0.04, 1.0, tp.y + 0.04), SR_M_DARK, i);
            }
            let q = f32((i - 17u) / 2u);
            var pc = vec2<f32>(mx + sx * (W - 1.25), 2.2 + q * 1.3);
            if (!along) {
                pc = vec2<f32>(W * (q + 0.5) / 3.0, D - 1.9);
            }
            if (pc.y > D - 1.2) {
                return none;
            }
            if ((i - 17u) % 2u == 1u) {
                return sr_box(vec3<f32>(pc.x - 0.007, H - 1.0, pc.y - 0.007), vec3<f32>(pc.x + 0.007, H, pc.y + 0.007), SR_M_DARK, i);
            }
            return sr_box(vec3<f32>(pc.x - 0.1, H - 1.15, pc.y - 0.1), vec3<f32>(pc.x + 0.1, H - 1.0, pc.y + 0.1), SR_M_LAMP, i);
        }
        case 4u: {
            // a salon: the station shelf along the mirror wall, chairs facing it, a desk by the door
            if (i == 0u) {
                let a = mx;
                let b = mx + sx * 0.38;
                return sr_box(vec3<f32>(min(a, b), 0.82, 1.4), vec3<f32>(max(a, b), 0.88, D - 1.0), SR_M_WOOD, i);
            }
            if (i == 1u) {
                let a = W - mx - sx * 0.4;
                let b = W - mx - sx * 1.3;
                return sr_box(vec3<f32>(min(a, b), 0.0, 0.9), vec3<f32>(max(a, b), 1.05, 1.6), SR_M_WOOD, i);
            }
            if (i == 2u) {
                let cx = W - mx - sx * 0.85;
                return sr_box(vec3<f32>(cx - 0.15, 1.05, 1.25), vec3<f32>(cx + 0.15, 1.28, 1.5), SR_M_TILL, i);
            }
            let st = (i - 3u) / 3u;
            let part = (i - 3u) % 3u;
            let n = u32(clamp(floor((D - 2.4) / 1.55), 1.0, 4.0));
            if (st >= n) {
                return none;
            }
            let zc = 1.95 + f32(st) * 1.55;
            let xs = mx + sx * 1.05;
            if (part == 0u) {
                return sr_box(vec3<f32>(xs - 0.28, 0.46, zc - 0.28), vec3<f32>(xs + 0.28, 0.62, zc + 0.28), SR_M_DARK, i);
            }
            if (part == 1u) {
                let a = xs + sx * 0.2;
                let b = xs + sx * 0.32;
                return sr_box(vec3<f32>(min(a, b), 0.62, zc - 0.26), vec3<f32>(max(a, b), 1.25, zc + 0.26), SR_M_DARK, i);
            }
            return sr_box(vec3<f32>(xs - 0.07, 0.0, zc - 0.07), vec3<f32>(xs + 0.07, 0.46, zc + 0.07), SR_M_DARK, i);
        }
        case 5u: {
            // a laundromat: front-loaders along one wall, an island, dryers stacked across the back
            if (i == 0u) {
                let a = mx;
                let b = mx + sx * 0.72;
                return sr_box(vec3<f32>(min(a, b), 0.0, 1.2), vec3<f32>(max(a, b), 0.92, D - 1.0), SR_M_WHITE, i);
            }
            if (i == 1u) {
                if (W < 3.6) {
                    return none;
                }
                let xc = W * 0.5 + sx * 0.3;
                return sr_box(vec3<f32>(xc - 0.7, 0.0, 2.1), vec3<f32>(xc + 0.7, 0.95, D - 1.9), SR_M_WHITE, i);
            }
            if (i == 2u) {
                return sr_box(vec3<f32>(0.3, 0.0, D - 0.82), vec3<f32>(W - 0.3, 1.95, D), SR_M_WHITE, i);
            }
            let a = W - mx - sx * 0.3;
            let b = W - mx - sx * 1.0;
            return sr_box(vec3<f32>(min(a, b), 0.0, 1.5), vec3<f32>(max(a, b), 0.9, 3.4), SR_M_WOOD, i);
        }
        case 6u: {
            // a gallery or a furniture shop: a plinth, a sofa or a long table, a desk
            let x = W * (0.25 + 0.5 * sr_r(k, 60u, i32(i)));
            let z = 1.8 + (D - 3.6) * sr_r(k, 61u, i32(i));
            if (i == 0u) {
                return sr_box(vec3<f32>(x - 0.25, 0.0, z - 0.25), vec3<f32>(x + 0.25, 1.0, z + 0.25), SR_M_SOFT, i);
            }
            if (i == 1u) {
                let w = min(1.0, W * 0.25);
                return sr_box(vec3<f32>(W * 0.5 - w, 0.0, D * 0.55 - 0.45), vec3<f32>(W * 0.5 + w, 0.8, D * 0.55 + 0.45), SR_M_SOFT, i);
            }
            let a = mx + sx * 0.6;
            let b = mx + sx * 2.0;
            return sr_box(vec3<f32>(min(a, b), 0.0, D - 2.2), vec3<f32>(max(a, b), 0.75, D - 1.5), SR_M_WOOD, i);
        }
        case 7u: {
            // a clinic: the reception counter across the room, a bench along one wall
            if (i == 0u) {
                return sr_box(vec3<f32>(0.5, 0.0, D - 2.8), vec3<f32>(W - 0.5, 1.1, D - 2.2), SR_M_WOOD, i);
            }
            if (i == 1u) {
                let cx = W * 0.5;
                return sr_box(vec3<f32>(cx - 0.2, 1.1, D - 2.7), vec3<f32>(cx + 0.2, 1.36, D - 2.45), SR_M_TILL, i);
            }
            let a = mx;
            if (i == 2u) {
                let b = mx + sx * 0.5;
                return sr_box(vec3<f32>(min(a, b), 0.42, 1.2), vec3<f32>(max(a, b), 0.47, min(3.8, D - 3.2)), SR_M_SOFT, i);
            }
            let b = mx + sx * 0.08;
            return sr_box(vec3<f32>(min(a, b), 0.0, 1.2), vec3<f32>(max(a, b), 0.92, min(3.8, D - 3.2)), SR_M_SOFT, i);
        }
        case 8u: {
            // dry cleaners: the counter across the front, rails of bagged garments behind
            if (i == 0u) {
                return sr_box(vec3<f32>(0.3, 0.0, 1.3), vec3<f32>(W - 0.3, 1.05, 1.85), SR_M_WOOD, i);
            }
            if (i == 1u) {
                let cx = mx + sx * 0.8;
                return sr_box(vec3<f32>(cx - 0.17, 1.05, 1.45), vec3<f32>(cx + 0.17, 1.3, 1.75), SR_M_TILL, i);
            }
            let z = select(3.0, 4.4, i == 3u);
            if (z + 0.4 > D - 0.4) {
                return none;
            }
            return sr_box(vec3<f32>(0.6, 0.3, z), vec3<f32>(W - 0.6, 1.95, z + 0.4), SR_M_BAGS, i);
        }
        default: {
            // a gym: treadmills facing the window (deck, a post each side, the console), a bench
            if (i == 16u) {
                return sr_box(vec3<f32>(W * 0.5 - 0.6, 0.0, D - 2.6), vec3<f32>(W * 0.5 + 0.6, 0.45, D - 2.25), SR_M_SOFT, i);
            }
            let t = i / 4u;
            let n = u32(clamp(floor((W - 0.6) / 1.1), 1.0, 4.0));
            if (t >= n) {
                return none;
            }
            let xc = W * 0.5 + (f32(t) - (f32(n) - 1.0) * 0.5) * 1.1;
            switch i % 4u {
                case 0u: { return sr_box(vec3<f32>(xc - 0.36, 0.0, 2.2), vec3<f32>(xc + 0.36, 0.22, 3.9), SR_M_MACHINE, i); }
                case 1u: { return sr_box(vec3<f32>(xc - 0.36, 0.0, 2.22), vec3<f32>(xc - 0.31, 1.1, 2.3), SR_M_MACHINE, i); }
                case 2u: { return sr_box(vec3<f32>(xc + 0.31, 0.0, 2.22), vec3<f32>(xc + 0.36, 1.1, 2.3), SR_M_MACHINE, i); }
                default: { return sr_box(vec3<f32>(xc - 0.36, 1.1, 2.12), vec3<f32>(xc + 0.36, 1.32, 2.3), SR_M_TILL, i); }
            }
        }
    }
    return none;
}

// ---- people ----

// A person standing (or sitting) on a card at depth z facing the window:
// (x, y top of the head, z, 1 seated); w < 0: nobody. Slot 0 is the staff
// (behind the counter), slot 1 a customer.
fn sr_person_at(r: SrRoom, slot: u32) -> vec4<f32> {
    let W = r.size.x;
    let D = r.size.z;
    let k = r.key;
    let right = sr_r(k, 30u, 0) < 0.5;
    let mx = select(0.0, W, right);
    let sx = select(1.0, -1.0, right);
    let here = sr_r(k, 70u, i32(slot)) < select(0.62, 0.42, slot == 1u);
    if (r.vacant || !here) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    let tall = 1.6 + 0.25 * sr_r(k, 71u, i32(slot));
    let u = sr_r(k, 72u, i32(slot));
    switch r.fit {
        case 0u: {
            if (slot == 0u) {
                return vec4<f32>(mx + sx * 0.45, tall, 1.3 + 0.6 * u, 0.0);
            }
            // a customer at the head of the aisles, on the side away from the counter
            return vec4<f32>(select(W * 0.35, W * 0.65, !right) + (u - 0.5) * 0.6, tall, 1.45 + 0.4 * sr_r(k, 73u, 1), 0.0);
        }
        case 1u: {
            if (slot == 0u) {
                return vec4<f32>(mx + sx * 1.2, tall, D - 1.3, 0.0);
            }
            // browsing behind the display table
            return vec4<f32>(W * (0.3 + 0.4 * u), tall, 1.95 + 0.25 * sr_r(k, 73u, 1), 0.0);
        }
        case 2u: {
            if (slot == 0u) {
                return vec4<f32>(W * (0.2 + 0.6 * u), tall, D - 1.0, 0.0);
            }
            // a diner on a chair at the first table
            let cols = clamp(floor((W - 0.4) / 1.75), 1.0, 2.0);
            return vec4<f32>(W * 0.5 / cols - 0.62, 1.32, 1.6, 1.0);
        }
        case 3u: {
            if (slot == 0u) {
                return vec4<f32>(select(W * 0.5, mx + sx * (W - 0.45), W >= 3.6), tall, select(D - 1.25, 2.4 + 2.0 * u, W >= 3.6), 0.0);
            }
            return vec4<f32>(select(W * 0.4, mx + sx * (W - 1.95), W >= 3.6), 1.45 + 0.1 * u, select(D - 2.6, 2.75, W >= 3.6), 1.0);
        }
        case 4u: {
            if (slot == 0u) {
                return vec4<f32>(mx + sx * 1.55, tall, 1.95 + 0.2 * u, 0.0);
            }
            return vec4<f32>(mx + sx * 1.05, 1.32, 1.95, 1.0);
        }
        case 5u: {
            // at the folding table
            return vec4<f32>(W - mx - sx * (1.3 + 0.2 * f32(slot)), tall, 2.0 + 1.2 * f32(slot), 0.0);
        }
        case 7u: {
            if (slot == 0u) {
                return vec4<f32>(W * (0.3 + 0.4 * u), tall, D - 1.8, 0.0);
            }
            return vec4<f32>(mx + sx * 0.3, 1.3, 2.0, 1.0);
        }
        case 8u: {
            if (slot == 0u) {
                return vec4<f32>(W * (0.3 + 0.4 * u), tall, 2.3, 0.0);
            }
            return vec4<f32>(W * (0.3 + 0.4 * u), tall, 0.95, 0.0);
        }
        case 9u: {
            // on the first treadmill
            let n = clamp(floor((W - 0.6) / 1.1), 1.0, 4.0);
            return vec4<f32>(W * 0.5 - (n - 1.0) * 0.55, tall + 0.24, 3.0, 0.0);
        }
        default: {
            return vec4<f32>(W * (0.25 + 0.5 * u), tall, 1.8 + (D - 3.8) * sr_r(k, 73u, i32(slot)), 0.0);
        }
    }
    return vec4<f32>(0.0, 0.0, 0.0, -1.0);
}

// The person's (albedo, cover) at (x, y) on its card (x from its middle).
fn sr_person(q: vec2<f32>, top: f32, seated: bool, k: u32, slot: u32, fp: f32) -> vec4<f32> {
    let hr = 0.1;
    let hc = vec2<f32>(0.0, top - 0.12);
    let head = length((q - hc) * vec2<f32>(1.08, 0.92)) - hr;
    let sh = top - 0.29;
    let waist = select(top * 0.56, 0.62, seated);
    // the torso: shoulders 0.22 either side, narrowing to the waist, rounded at the shoulders
    let ty = clamp((sh - q.y) / max(sh - waist, 0.1), 0.0, 1.0);
    let half = mix(0.215, 0.175, ty) - 0.06 * smoothstep(sh - 0.08, sh + 0.02, q.y);
    let torso = max(abs(q.x) - half, max(q.y - (sh + 0.03), waist - q.y));
    let neck = max(abs(q.x) - 0.055, max(q.y - hc.y, sh - q.y));
    var legs = 1.0;
    if (!seated) {
        legs = max(abs(abs(q.x) - 0.085) - 0.075, max(q.y - waist - 0.02, -q.y));
    } else {
        // seated: the thighs forward (seen end on, under the waist), the shins down to the floor
        legs = min(max(abs(q.x) - 0.18, max(q.y - waist, 0.42 - q.y)), max(abs(abs(q.x) - 0.08) - 0.06, max(q.y - 0.45, -q.y)));
    }
    let d = min(min(head, torso), min(neck, legs));
    let cover = 1.0 - sr_step(0.0, d, fp);
    if (cover <= 0.0) {
        return vec4<f32>(0.0);
    }
    let skin_h = sr_r(k, 74u, i32(slot));
    let skin = select(select(select(vec3<f32>(0.55, 0.38, 0.30), vec3<f32>(0.33, 0.20, 0.13), skin_h > 0.3), vec3<f32>(0.16, 0.09, 0.06), skin_h > 0.6), vec3<f32>(0.62, 0.48, 0.38), skin_h > 0.85);
    let hair = mix(vec3<f32>(0.02, 0.015, 0.01), vec3<f32>(0.30, 0.22, 0.12), step(0.75, sr_r(k, 75u, i32(slot))));
    let top_c = sr_cloth_col(sr_r(k, 76u, i32(slot)), sr_r(k, 77u, i32(slot)));
    let leg_c = select(vec3<f32>(0.04, 0.04, 0.05), vec3<f32>(0.10, 0.15, 0.28), sr_r(k, 78u, i32(slot)) > 0.5);
    var c = top_c;
    if (head <= 0.0 || neck <= 0.0) {
        c = select(skin, hair, q.y > hc.y + 0.03 || (head <= 0.0 && abs(q.x) > 0.075 && q.y > hc.y - 0.02));
    } else if (q.y < waist) {
        c = leg_c;
    }
    return vec4<f32>(c, cover);
}

// ---- the surfaces ----

// Rows of goods on a shelf face: (u along the shelf, v up from its foot)
// over `height`; (albedo, glow) where a bottle's glass catches the light.
fn sr_goods(u: f32, v: f32, height: f32, goods: u32, k: u32, fp: f32) -> vec4<f32> {
    let pitch = select(select(0.36, 0.42, goods == 1u), 0.3, goods == 4u);
    let rowf = v / pitch;
    let row = floor(rowf);
    let fv = rowf - row;
    let lip = 0.035 / pitch;
    // above the top shelf: the unit's top, bare
    if (v > height - 0.04 || v < 0.1) {
        return vec4<f32>(vec3<f32>(0.08), 0.0);
    }
    let rk = i32(row) * 131 + i32(floor(u * 0.25)) * 17;
    let wcell = select(select(mix(0.07, 0.24, sr_r(k, 80u, rk)), 0.085, goods == 1u), mix(0.022, 0.05, sr_r(k, 80u, rk)), goods == 4u);
    let cu = u / wcell + sr_r(k, 81u, i32(row));
    let cell = floor(cu);
    let fu = cu - cell;
    let run = floor(cell / (2.0 + floor(3.0 * sr_r(k, 82u, rk))));
    let h = sr_r(k, 83u, i32(run) * 977 + i32(row) * 31);
    let g = sr_r(k, 84u, i32(cell) * 613 + i32(row) * 29);
    let ph = select(mix(0.5, 0.88, sr_r(k, 85u, i32(run) * 53 + i32(row))), mix(0.7, 0.92, g), goods == 4u);
    var col = sr_goods_col(goods, h, g);
    // the product's outline: its top below the next shelf, a gap between facings
    let in_v = fv - lip;
    var mask = (1.0 - sr_step(ph * (1.0 - lip), in_v, fp / pitch)) * sr_step(0.0, in_v, fp / pitch);
    mask = mask * sr_step(0.06, fu, fp / wcell) * (1.0 - sr_step(0.94, fu, fp / wcell));
    var glow = 0.0;
    if (goods == 1u) {
        // a bottle: shoulders at 60 %, a neck a third as wide
        let by = in_v / max(ph * (1.0 - lip), 1e-3);
        let bw = select(0.42, mix(0.42, 0.14, smoothstep(0.6, 0.75, by)), by > 0.6);
        mask = mask * (1.0 - sr_step(bw, abs(fu - 0.5), fp / wcell));
        // the light caught down one side of the glass
        glow = mask * (1.0 - smoothstep(0.0, 0.08, abs(fu - 0.36))) * step(by, 0.6) * sr_keep(wcell, fp);
    }
    if (goods == 5u) {
        // buckets of stems: leaves low, blooms in a mound
        let mound = 0.55 + 0.35 * sin(fu * 3.14159);
        mask = (1.0 - sr_step(mound, fv, fp / pitch)) * sr_step(0.05, fv, fp / pitch);
        col = select(sr_goods_col(5u, 0.1, g), col, fv > 0.35 && sr_noise(vec2<f32>(u, v) * 18.0, k, 86u) > 0.45);
    }
    if (goods == 0u || goods == 2u || goods == 3u) {
        // packaging: a label field across some, a darker or lighter cap, each facing a little its own;
        // the top of each product in the shadow of the shelf over it
        let pv = in_v / max(ph * (1.0 - lip), 1e-3);
        let label = sr_step(0.3, pv, fp / pitch) * (1.0 - sr_step(0.62, pv, fp / pitch)) * step(0.45, sr_r(k, 87u, i32(run) * 7 + i32(row)));
        let label_c = select(vec3<f32>(0.78, 0.76, 0.7), col * 0.45, sr_r(k, 88u, i32(run)) > 0.6);
        col = mix(col, label_c, label * sr_keep(0.12, fp));
        col = mix(col, col * 0.55, step(0.85, pv) * step(0.5, sr_r(k, 89u, i32(run) + 3)));
        col = col * (0.85 + 0.3 * g) * mix(1.0, 0.72, smoothstep(0.5, 1.0, fv));
    }
    let shelf = vec3<f32>(0.30, 0.30, 0.31) * select(1.0, 2.4, goods == 2u);
    var c = mix(vec3<f32>(0.035), col, mask);
    c = mix(c, shelf, 1.0 - sr_step(lip, fv, fp / pitch));
    // the mean at a distance: goods over most of each row, the shelf edge, the shadow above
    let mean = mix(vec3<f32>(0.035), sr_goods_col(goods, 0.5, 0.5) * 0.85, 0.62) * 0.9 + shelf * lip;
    c = mix(mean, c, sr_keep(min(wcell, pitch), fp));
    return vec4<f32>(c, glow);
}

// Garments hanging from a rail: (u along the rail, v up), the rail at `top`.
fn sr_garments(u: f32, v: f32, top: f32, k: u32, fp: f32) -> vec3<f32> {
    let w = 0.055;
    let cu = u / w;
    let cell = floor(cu);
    let fu = cu - cell;
    let h = sr_r(k, 90u, i32(cell));
    let g = sr_r(k, 91u, i32(cell));
    let hem = top - mix(0.55, 1.05, sr_r(k, 92u, i32(cell)));
    let col = sr_cloth_col(h, g) * (0.75 + 0.35 * sin(fu * 3.14159));
    let below = sr_step(hem, v, fp);
    let shoulder = 1.0 - sr_step(top - 0.06 - 0.03 * (1.0 - sin(fu * 3.14159)), v, fp);
    var c = mix(vec3<f32>(0.03), col, below * shoulder);
    // the rail
    c = mix(c, vec3<f32>(0.5), (1.0 - sr_step(0.015, abs(v - top + 0.02), fp)));
    return mix(vec3<f32>(0.11, 0.11, 0.12), c, sr_keep(w, fp));
}

// Round doors on enamel (a washer's, a dryer's): (u, v) on the face, rows of `rows`.
fn sr_doors(u: f32, v: f32, height: f32, rows: f32, fp: f32) -> vec4<f32> {
    let cw = 0.72;
    let rh = height / rows;
    let c = vec2<f32>((floor(u / cw) + 0.5) * cw, (floor(v / rh) + 0.55) * rh);
    let d = length(vec2<f32>(u, v) - c);
    let rad = min(0.21, rh * 0.36);
    let glass = 1.0 - sr_step(rad - 0.035, d, fp);
    let rim = (1.0 - sr_step(rad, d, fp)) - glass;
    let seam = 1.0 - sr_step(0.008, abs(fract(u / cw + 0.5) - 0.5) * cw, fp);
    var a = vec3<f32>(0.78, 0.79, 0.80) * (1.0 - 0.5 * seam);
    a = mix(a, vec3<f32>(0.55, 0.57, 0.6), rim);
    a = mix(a, vec3<f32>(0.04, 0.05, 0.06), glass);
    // the lit panel over each door, a little
    let panel = (1.0 - sr_step(0.012, abs(v - c.y - rad - 0.07), fp)) * (1.0 - sr_step(0.1, abs(u - c.x), fp));
    let mean = vec3<f32>(0.62, 0.63, 0.64);
    return vec4<f32>(mix(mean, a, sr_keep(0.2, fp)), panel * sr_keep(0.05, fp));
}

// Frames on a wall: (u along the wall, v up) -> (albedo, cover).
fn sr_frames(u: f32, v: f32, k: u32, salt: u32, fp: f32) -> vec4<f32> {
    let pitch = 1.6;
    let cell = floor(u / pitch);
    let h = sr_r(k, salt, i32(cell));
    if (h < 0.35) {
        return vec4<f32>(0.0);
    }
    let w = mix(0.3, 0.6, sr_r(k, salt + 1u, i32(cell)));
    let hh = w * mix(0.7, 1.35, sr_r(k, salt + 2u, i32(cell)));
    let c = vec2<f32>((cell + 0.5) * pitch, 1.55);
    let q = abs(vec2<f32>(u, v) - c);
    let outer = max(q.x - w * 0.5, q.y - hh * 0.5);
    let inner = max(q.x - w * 0.5 + 0.035, q.y - hh * 0.5 + 0.035);
    let cover = 1.0 - sr_step(0.0, outer, fp);
    // the picture: two or three colour fields
    let split = sr_step(c.y + (sr_r(k, salt + 3u, i32(cell)) - 0.5) * hh * 0.6, v, fp);
    let f0 = sr_goods_col(0u, sr_r(k, salt + 4u, i32(cell)), 0.4);
    let f1 = sr_goods_col(4u, sr_r(k, salt + 5u, i32(cell)), 0.6);
    let pic = mix(f0, f1, split);
    let frame = select(vec3<f32>(0.04), vec3<f32>(0.7, 0.62, 0.45), h > 0.8);
    return vec4<f32>(mix(frame, pic, 1.0 - sr_step(0.0, inner, fp)), cover);
}

// The ceiling's fixtures at (x, z): its glow (in units of the window's
// radiance), its albedo; and the kind of light the shop has (0 strips and
// troffers, 1 downlights and pendants, 2 track spots).
fn sr_light_kind(r: SrRoom) -> u32 {
    if (r.fit == 2u || r.fit == 3u) {
        return 1u;
    }
    if (r.fit == 6u || (r.fit == 1u && sr_r(r.key, 100u, 0) < 0.5)) {
        return 2u;
    }
    return 0u;
}

fn sr_ceiling(x: f32, z: f32, r: SrRoom, fp: f32) -> vec4<f32> {
    let lk = sr_light_kind(r);
    if (r.vacant) {
        // bare: a bulb on a cord over the middle (lit only while the work light is on)
        let d = length(vec2<f32>(x - r.size.x * 0.5, z - r.size.z * 0.45));
        return vec4<f32>(vec3<f32>(0.45), select(0.0, 30.0, r.work) * (1.0 - sr_step(0.05, d, fp)));
    }
    if (lk == 1u) {
        // dark (exposed or painted out), recessed downlights
        let p = vec2<f32>(fract(x / 1.8 + 0.5) - 0.5, fract(z / 1.8) - 0.5) * 1.8;
        let disc = 1.0 - sr_step(0.06, length(p), fp);
        return vec4<f32>(vec3<f32>(0.06, 0.055, 0.05), 12.0 * disc * sr_keep(0.12, fp) + 12.0 * 0.0035 * (1.0 - sr_keep(0.12, fp)));
    }
    if (lk == 2u) {
        // white, two track rails with spot heads
        let rail = min(abs(x - r.size.x * 0.33), abs(x - r.size.x * 0.67));
        let line = 1.0 - sr_step(0.02, rail, fp);
        let head = (1.0 - sr_step(0.05, rail, fp)) * (1.0 - sr_step(0.06, abs(fract(z / 0.9) - 0.5) * 0.9, fp));
        let c = mix(vec3<f32>(0.72), vec3<f32>(0.05), max(line, head));
        return vec4<f32>(c, 9.0 * head * (1.0 - sr_step(0.03, abs(fract(z / 0.9) - 0.5) * 0.9, fp)));
    }
    // white; continuous strips along the room, or troffers in a grid
    let strips = sr_r(r.key, 101u, 0) < 0.55;
    let pitch = select(2.4, 1.8, strips);
    let px = abs(fract(x / pitch + 0.5) - 0.5) * pitch;
    var glow = 0.0;
    if (strips) {
        glow = (1.0 - sr_step(0.06, px, fp)) * step(0.6, z);
        glow = mix(0.12 / pitch, glow, sr_keep(0.12, fp));
    } else {
        let pz = abs(fract(z / 2.4) - 0.5) * 2.4;
        glow = (1.0 - sr_step(0.3, px, fp)) * (1.0 - sr_step(0.6, pz, fp));
        glow = mix(0.6 * 1.2 / (2.4 * 2.4), glow, sr_keep(0.6, fp)) * 0.45;
    }
    let tile = 1.0 - 0.18 * (1.0 - sr_step(0.012, min(abs(fract(x / 0.6 + 0.5) - 0.5), abs(fract(z / 0.6 + 0.5) - 0.5)) * 0.6, fp)) * sr_keep(0.6, fp);
    return vec4<f32>(vec3<f32>(0.78) * tile, SR_FIXTURE * glow);
}

// The light a surface at p (normal nrm) receives, relative (about 1 under
// the fixtures on a floor or a top, less on the walls, falling to the back
// and the corners).
fn sr_light(p: vec3<f32>, nrm: vec3<f32>, r: SrRoom) -> f32 {
    let W = r.size.x;
    let H = r.size.y;
    let D = r.size.z;
    let lk = sr_light_kind(r);
    if (r.vacant) {
        // the work light's pool round the room's middle
        let d = length(p - vec3<f32>(W * 0.5, H - 0.6, D * 0.45));
        return select(0.35, 1.2, r.work) / (1.0 + d * d * 0.35);
    }
    var e = mix(0.62, 1.0, max(nrm.y, 0.0));
    if (nrm.y < -0.5) {
        e = 0.3;
    }
    e = e * mix(1.0, SR_BACK_FALL, smoothstep(1.0, D, p.z));
    let wx = min(p.x, W - p.x);
    e = e * (1.0 - 0.4 * (1.0 - smoothstep(0.0, 0.9, wx)));
    e = e * (1.0 - 0.3 * (1.0 - smoothstep(0.0, 0.8, D - p.z)));
    if (abs(nrm.y) < 0.5) {
        // contact: darker at the foot of a wall or a fitting, more light up high
        e = e * mix(0.5, 1.0, smoothstep(0.0, 0.5, p.y)) * mix(0.9, 1.1, smoothstep(1.2, H, p.y));
    }
    if (lk == 1u) {
        // pendants and downlights: pools on the tables and the floor, scallops on the walls, the rest dimmer
        e = e * select(0.8, 1.0, r.fit == 3u);
        let along = select(p.x, p.z, abs(nrm.x) > 0.5);
        let su = (fract(along / 1.8) - 0.5) * 1.8;
        let at_wall = select(0.0, 1.0, abs(nrm.y) < 0.5 && (wx < 0.05 || D - p.z < 0.05));
        e = e + at_wall * 0.55 * exp(-su * su / 0.1) * smoothstep(0.4, 2.4, p.y) * (1.0 - smoothstep(H - 0.5, H, p.y));
        var pc = vec2<f32>(0.0);
        if (r.fit == 2u) {
            let cols = clamp(floor((W - 0.4) / 1.75), 1.0, 2.0);
            pc = vec2<f32>((floor(p.x / W * cols) + 0.5) * W / cols, 1.6 + floor(clamp((p.z - 0.6) / 1.95, 0.0, 1.0)) * 1.95);
        } else {
            let right = sr_r(r.key, 30u, 0) < 0.5;
            pc = vec2<f32>(select(W - 1.25, 1.25, right), clamp(round((p.z - 2.2) / 1.3), 0.0, 2.0) * 1.3 + 2.2);
            if (W < 3.6) {
                pc = vec2<f32>((floor(p.x / W * 3.0) + 0.5) * W / 3.0, D - 1.9);
            }
        }
        let dd = p.xz - pc;
        let pool = exp(-dot(dd, dd) / 0.5) * (0.6 + 0.4 * max(nrm.y, 0.0)) * smoothstep(H - 0.9, 0.6, p.y);
        e = e + 1.6 * pool;
    }
    if (lk == 2u && abs(nrm.y) < 0.5) {
        // track spots washing the walls and the racks
        let u = select(p.x, p.z, abs(nrm.x) > 0.5);
        let su = (fract(u / 1.6) - 0.5) * 1.6;
        let sy = p.y - 1.5;
        let spot = exp(-su * su / 0.18 - sy * sy / 0.5);
        e = e * 0.8 + 0.7 * spot;
    }
    return e;
}

// The room's walls, floor and ceiling at p (the face `face`: 0 a side wall, 1
// the back, 2 the floor, 3 the ceiling): (albedo, glow).
fn sr_wall(p: vec3<f32>, face: u32, r: SrRoom, fp: f32) -> vec4<f32> {
    let W = r.size.x;
    let D = r.size.z;
    let k = r.key;
    if (face == 3u) {
        return sr_ceiling(p.x, p.z, r, fp);
    }
    if (face == 2u) {
        // the floor: tile, planks or concrete by trade
        let fk = sr_r(k, 110u, 0);
        let wood = (r.fit == 2u || r.fit == 3u || r.fit == 1u) && fk < 0.7;
        let tile = !wood && (r.fit == 0u || r.fit == 5u || r.fit == 7u) && fk < 0.8;
        var c = vec3<f32>(0.32, 0.31, 0.30) * (0.85 + 0.3 * sr_noise(p.xz * 1.3, k, 111u));
        if (r.vacant) {
            return vec4<f32>(c * 0.8, 0.0);
        }
        if (wood) {
            let plank = floor(p.x / 0.13);
            let tone = sr_r(k, 112u, i32(plank) * 7 + i32(floor(p.z / 1.8 + sr_r(k, 113u, i32(plank)))));
            c = mix(vec3<f32>(0.32, 0.18, 0.09), vec3<f32>(0.42, 0.27, 0.14), tone);
            c = mix(vec3<f32>(0.36, 0.22, 0.11), c, sr_keep(0.13, fp));
        }
        if (tile) {
            let q = floor(p.xz / 0.3);
            let chk = (i32(q.x) + i32(q.y)) % 2;
            let pale = select(vec3<f32>(0.62, 0.62, 0.6), vec3<f32>(0.5, 0.5, 0.49), chk != 0);
            let grout = 1.0 - (1.0 - sr_step(0.006, min(abs(fract(p.x / 0.3 + 0.5) - 0.5), abs(fract(p.z / 0.3 + 0.5) - 0.5)) * 0.3, fp));
            c = mix(vec3<f32>(0.55), pale * mix(0.75, 1.0, grout), sr_keep(0.3, fp));
        }
        return vec4<f32>(c, 0.0);
    }
    let u = select(p.x, p.z, face == 0u);
    let wall = sr_wall_col(r.fit, k);
    var c = wall * (0.93 + 0.12 * sr_noise(vec2<f32>(u, p.y) * 0.8, k, 120u));
    if (r.vacant) {
        // emptied: paint patched where the fittings came off, a dado of scuffs
        let patchy = sr_noise(vec2<f32>(u, p.y) * 2.3, k, 121u) * 0.6 + sr_noise(vec2<f32>(u, p.y) * 7.0, k, 122u) * 0.4;
        let scuff = (1.0 - smoothstep(0.3, 1.0, p.y)) * 0.15;
        return vec4<f32>(mix(vec3<f32>(0.62, 0.61, 0.58), vec3<f32>(0.52, 0.51, 0.48), smoothstep(0.6, 0.75, patchy)) * (1.0 - scuff), 0.0);
    }
    var glow = 0.0;
    // a skirting at the foot
    c = mix(c, vec3<f32>(0.06), 1.0 - sr_step(0.1, p.y, fp));
    let back = face == 1u;
    switch r.fit {
        case 0u: {
            // wall shelving to 2.1 m (coolers across the back of a market or a liquor store)
            if (back && (r.goods == 0u || r.goods == 1u) && p.y < 2.05) {
                let door = 0.76;
                let fu = fract(u / door);
                let mull = 1.0 - sr_step(0.035, min(fu, 1.0 - fu) * door, fp);
                let gd = sr_goods(u, p.y, 2.0, r.goods, k + 7u, fp);
                // inside the glass: the goods lit from the cooler's own strips
                c = mix(gd.rgb * 0.9, vec3<f32>(0.12), mull);
                glow = (1.0 - mull) * (0.35 + 0.65 * smoothstep(0.1, 0.0, abs(fract(u / door) - 0.06))) * SR_COOLER;
                glow = glow + (1.0 - mull) * dot(gd.rgb, vec3<f32>(0.33)) * SR_COOLER * 0.6;
            } else if (p.y < 2.1 && p.y > 0.1) {
                let gd = sr_goods(u, p.y, 2.1, r.goods, k + u32(face) * 3u, fp);
                c = gd.rgb;
                glow = gd.a * SR_GLINT;
            } else if (p.y > 2.3 && p.y < 2.75 && sr_r(k, 123u, 0) < 0.6) {
                // a band of the trade colour over the shelving, plain blocks on it
                let band = mix(wall, sr_wall_col(9u, k + 3u), 0.5);
                let blk = sr_step(0.55, sr_noise(vec2<f32>(u * 0.35, 0.5), k, 122u), 0.05);
                c = mix(band, band * 0.75, blk);
            }
        }
        case 1u: {
            // wall bays of 1.8 m: garments on a rail (or two), folded stacks on shelves, a bare panel
            let bay = floor(u / 1.8);
            let bu = u - bay * 1.8;
            let bk = sr_r(k, 128u, i32(bay) + i32(face) * 97);
            let in_bay = sr_step(0.12, bu, fp) * (1.0 - sr_step(1.68, bu, fp));
            if (bk < 0.55 && p.y > 0.5 && p.y < 1.95) {
                let two = bk < 0.25;
                let top = select(1.9, select(1.9, 1.2, p.y < 1.2), two);
                c = mix(c, sr_garments(u + 7.0 * f32(face), p.y, top, k + select(0u, 11u, p.y < 1.2), fp), in_bay);
            } else if (bk < 0.85 && p.y > 0.4 && p.y < 2.0) {
                c = mix(c, sr_goods(u, p.y - 0.4, 1.6, 4u, k + 5u, fp).rgb * 1.4, in_bay);
            }
        }
        case 2u: {
            // dining: wainscot, framed pictures; the kitchen pass lit through the back wall
            c = mix(c, c * 0.55, 1.0 - sr_step(1.0, p.y, fp));
            let fr = sr_frames(u, p.y, k, 124u + u32(face) * 8u, fp);
            c = mix(c, fr.rgb, fr.a);
            if (back) {
                let pass_c = W * (0.3 + 0.4 * sr_r(k, 125u, 0));
                let in_pass = (1.0 - sr_step(0.9, abs(p.x - pass_c), fp)) * sr_step(1.15, p.y, fp) * (1.0 - sr_step(1.85, p.y, fp));
                c = mix(c, vec3<f32>(0.55, 0.55, 0.5), in_pass);
                glow = in_pass * SR_KITCHEN * (0.75 + 0.25 * sr_noise(vec2<f32>(p.x * 3.0, p.y * 3.0), k, 126u));
                // a board of plain lines beside it
                let board_c = select(pass_c + 1.8, pass_c - 1.8, pass_c > W * 0.5);
                let in_board = (1.0 - sr_step(0.55, abs(p.x - board_c), fp)) * (1.0 - sr_step(0.4, abs(p.y - 2.05), fp));
                let lines = (1.0 - sr_step(0.012, abs(fract(p.y / 0.09) - 0.5) * 0.09, fp)) * step(0.3, sr_noise(vec2<f32>(p.x * 4.0, floor(p.y / 0.09)), k, 127u));
                c = mix(c, mix(vec3<f32>(0.03), vec3<f32>(0.6), lines * sr_keep(0.09, fp) + 0.15 * (1.0 - sr_keep(0.09, fp))), in_board);
            }
        }
        case 3u: {
            // a bar: the back bar of bottles on shelves over a lit strip, a mirror behind them
            let right = sr_r(k, 30u, 0) < 0.5;
            // the counter stands by the wall away from the room's `right` side (sr_obj)
            let behind = select(back, face == 0u && (p.x > W * 0.5) != right, W >= 3.6);
            if (behind && p.y > 0.95 && p.y < 2.35) {
                let gd = sr_goods(u, p.y - 0.95, 1.4, 1u, k + 13u, fp);
                c = mix(vec3<f32>(0.12, 0.11, 0.1), gd.rgb * 1.6, step(0.04, dot(gd.rgb, vec3<f32>(1.0))));
                let strip = 1.0 - sr_step(0.02, abs(fract((p.y - 0.95) / 0.42) - 0.03) * 0.42, fp);
                glow = gd.a * SR_GLINT * 1.6 + strip * SR_STRIP + 0.02;
            } else {
                let fr = sr_frames(u, p.y, k, 130u, fp);
                c = mix(c, fr.rgb * 0.7, fr.a);
            }
        }
        case 4u: {
            // a salon: mirrors over the stations, shelves of products at the back
            let right = sr_r(k, 30u, 0) < 0.5;
            let mirror_wall = face == 0u && (p.x > W * 0.5) == right;
            if (mirror_wall && p.y > 0.9 && p.y < 2.1 && p.z > 1.3 && p.z < D - 1.0) {
                let pz = fract((p.z - 1.95) / 1.55 + 0.5);
                let in_m = (1.0 - sr_step(0.36, abs(pz - 0.5) * 1.55, fp));
                // the room reflected: a soft light grey, its fixtures as a brighter band up high
                c = mix(c, vec3<f32>(0.5, 0.52, 0.54), in_m);
                glow = in_m * SR_GAIN * (0.25 + 0.6 * smoothstep(1.6, 2.1, p.y));
            } else if (back && p.y > 1.0 && p.y < 2.0) {
                c = sr_goods(u, p.y - 1.0, 1.0, 2u, k + 17u, fp).rgb;
            }
        }
        case 6u, 7u: {
            let fr = sr_frames(u, p.y, k, 140u + u32(face) * 8u, fp);
            c = mix(c, fr.rgb, fr.a);
        }
        case 9u: {
            // a gym: the mirror wall across the back
            if (back && p.y > 0.3 && p.y < 2.2) {
                c = vec3<f32>(0.45, 0.47, 0.5);
                glow = SR_GAIN * (0.25 + 0.6 * smoothstep(1.7, 2.2, p.y));
            }
        }
        default: {}
    }
    return vec4<f32>(c, glow);
}

// A fitting's (albedo, glow) at p on its face of normal nrm.
fn sr_fitting(b: SrBox, p: vec3<f32>, nrm: vec3<f32>, r: SrRoom, fp: f32) -> vec4<f32> {
    let k = r.key;
    let side = abs(nrm.x) > 0.5;
    let u = select(p.x - b.lo.x, p.z - b.lo.z, side);
    let v = p.y - b.lo.y;
    let top = nrm.y > 0.5;
    switch b.mat {
        case 1u: {
            if (top) {
                return vec4<f32>(vec3<f32>(0.12), 0.0);
            }
            let gd = sr_goods(u, v, b.hi.y - b.lo.y, r.goods, k + b.id * 5u + select(0u, 2u, nrm.x > 0.5 || nrm.z > 0.5), fp);
            return vec4<f32>(gd.rgb, gd.a * SR_GLINT);
        }
        case 2u: {
            return vec4<f32>(sr_garments(u + f32(b.id) * 3.1, p.y, b.hi.y, k + b.id, fp), 0.0);
        }
        case 3u, 8u: {
            // wood or laminate; a darker kick; a cafe's lit case in its front
            // wood, a pale laminate, or a dark one
            let wk = sr_r(k, 150u, 0);
            let wood = select(select(vec3<f32>(0.40, 0.24, 0.12), vec3<f32>(0.62, 0.60, 0.56), wk > 0.55), vec3<f32>(0.10, 0.10, 0.11), wk > 0.8);
            var c = wood * (0.85 + 0.25 * sr_noise(vec2<f32>(u * 0.6, v * 6.0), k, 151u));
            if (top) {
                c = c * 1.15;
                return vec4<f32>(c, 0.0);
            }
            c = mix(c, vec3<f32>(0.03), 1.0 - sr_step(0.12, v, fp));
            var glow = 0.0;
            if (b.mat == 8u && !side && v > 0.5 && v < (b.hi.y - b.lo.y) - 0.05) {
                // the case: pastries and cakes on two lit shelves
                let gd = sr_goods(u, v - 0.5, 0.5, 0u, k + 19u, fp);
                c = mix(gd.rgb * 1.2, vec3<f32>(0.8, 0.65, 0.45), 0.35);
                glow = SR_CASE_GLOW * (0.5 + 0.5 * dot(gd.rgb, vec3<f32>(0.5)));
            }
            return vec4<f32>(c, glow);
        }
        case 5u: {
            if (top) {
                return vec4<f32>(vec3<f32>(0.72), 0.0);
            }
            // the room faces carry doors (dryers two rows); the backs plain
            let toward_room = (nrm.z < -0.5) || (side && b.lo.x > 0.3 && b.hi.x < r.size.x - 0.3) || (side && ((nrm.x > 0.5 && b.lo.x < 0.3) || (nrm.x < -0.5 && b.hi.x > r.size.x - 0.3)));
            if (!toward_room) {
                return vec4<f32>(vec3<f32>(0.7), 0.0);
            }
            let rows = select(1.0, 2.0, b.hi.y > 1.5);
            let dd = sr_doors(u, v, b.hi.y - b.lo.y, rows, fp);
            return vec4<f32>(dd.rgb, dd.a * 0.8);
        }
        case 6u: {
            // a pendant: its shade (dark metal or coloured glass), its mouth bright
            let shade = select(vec3<f32>(0.05), vec3<f32>(0.6, 0.35, 0.12), sr_r(k, 152u, 0) > 0.6);
            if (nrm.y < -0.5) {
                return vec4<f32>(vec3<f32>(0.9), SR_PENDANT);
            }
            return vec4<f32>(shade, select(0.0, SR_PENDANT * 0.12, sr_r(k, 152u, 0) > 0.6));
        }
        case 7u: {
            // a till: dark, its screen lit toward the room or the door
            let screen = (1.0 - sr_step(0.1, abs(u - 0.17), fp)) * sr_step(0.08, v, fp) * (1.0 - sr_step(0.22, v, fp));
            return vec4<f32>(mix(vec3<f32>(0.04), vec3<f32>(0.3, 0.4, 0.5), screen), screen * 0.22);
        }
        case 9u: {
            // bagged garments: pale, glossy plastic over them
            let g = sr_garments(u, p.y, b.hi.y, k + 23u, fp);
            return vec4<f32>(mix(g, vec3<f32>(0.75, 0.76, 0.78), 0.3), 0.03);
        }
        case 10u: {
            return vec4<f32>(vec3<f32>(0.42, 0.30, 0.17) * (0.85 + 0.3 * sr_r(k, 153u, i32(b.id))), 0.0);
        }
        case 11u: {
            return vec4<f32>(vec3<f32>(0.07, 0.07, 0.075), 0.0);
        }
        case 12u: {
            let soft = mix(vec3<f32>(0.78, 0.76, 0.72), vec3<f32>(0.25, 0.30, 0.38), step(0.5, sr_r(k, 154u, i32(b.id))));
            return vec4<f32>(soft * (0.9 + 0.15 * sr_noise(vec2<f32>(u, v) * 4.0, k, 155u)), 0.0);
        }
        default: {
            // chairs, stools, posts: black, dark wood or a colour
            let h = sr_r(k, 156u, 0);
            let c = select(select(vec3<f32>(0.03), vec3<f32>(0.22, 0.12, 0.06), h > 0.45), vec3<f32>(0.45, 0.08, 0.06), h > 0.85);
            return vec4<f32>(c * select(1.0, 1.3, top), 0.0);
        }
    }
    return vec4<f32>(0.0);
}

// ---- the ray ----

// The room seen along `d` (x along the front, y up, z into the room; d.z >
// 0) from `o` on its glass, `pix` the pixel angle, `dist` the eye's distance.
fn sr_room(o: vec3<f32>, d_in: vec3<f32>, r: SrRoom, pix: f32, dist: f32) -> SrOut {
    var out: SrOut;
    let W = r.size.x;
    let H = r.size.y;
    let D = r.size.z;
    let d0 = vec3<f32>(d_in.x, d_in.y, max(d_in.z, 0.02));
    let d = select(d0, select(vec3<f32>(-1e-5), vec3<f32>(1e-5), d0 >= vec3<f32>(0.0)), abs(d0) < vec3<f32>(1e-5));
    let inv = 1.0 / d;
    // the box: its side walls, floor and ceiling, its back
    let tx = select(-o.x, W - o.x, d.x > 0.0) * inv.x;
    let ty = select(-o.y, H - o.y, d.y > 0.0) * inv.y;
    let tz = (D - o.z) * inv.z;
    var t = max(min(min(tx, ty), tz), 0.0);
    var face = 1u;
    var nrm = vec3<f32>(0.0, 0.0, -1.0);
    if (tx < ty && tx < tz) {
        face = 0u;
        nrm = vec3<f32>(-sign(d.x), 0.0, 0.0);
    } else if (ty < tz) {
        face = select(2u, 3u, d.y > 0.0);
        nrm = vec3<f32>(0.0, -sign(d.y), 0.0);
    }
    var mat = 0u;
    var hit = sr_box(vec3<f32>(0.0), vec3<f32>(0.0), 0u, 0u);
    // the fittings, near the eye (they give way to the room's mean past the detail range)
    let detail = 1.0 - smoothstep(SR_DETAIL_M * 0.7, SR_DETAIL_M, dist);
    if (detail > 0.0) {
        let n = sr_count(r);
        for (var i = 0u; i < n; i = i + 1u) {
            let b = sr_obj(r, i);
            if (b.mat == SR_M_NONE) {
                continue;
            }
            let t0 = (b.lo - o) * inv;
            let t1 = (b.hi - o) * inv;
            let tn = min(t0, t1);
            let tf = max(t0, t1);
            let enter = max(max(tn.x, tn.y), tn.z);
            let leave = min(min(tf.x, tf.y), tf.z);
            if (enter <= leave && enter > 0.0 && enter < t) {
                t = enter;
                mat = b.mat;
                hit = b;
                if (enter == tn.x) {
                    nrm = vec3<f32>(-sign(d.x), 0.0, 0.0);
                } else if (enter == tn.y) {
                    nrm = vec3<f32>(0.0, -sign(d.y), 0.0);
                } else {
                    nrm = vec3<f32>(0.0, 0.0, -1.0);
                }
            }
        }
    }
    var p = o + d * t;
    let cos_inc = max(abs(dot(normalize(d), nrm)), 0.08);
    let fp = (dist + t * length(d)) * pix / cos_inc;
    var sc = vec4<f32>(0.0);
    if (mat == 0u) {
        sc = sr_wall(p, face, r, fp);
    } else {
        sc = sr_fitting(hit, p, nrm, r, fp);
    }
    var e = sr_light(p, nrm, r);
    // people: a card each, nearer than what the ray hit
    if (detail > 0.0) {
        for (var s = 0u; s < 2u; s = s + 1u) {
            let pp = sr_person_at(r, s);
            if (pp.w < 0.0) {
                continue;
            }
            let tp = (pp.z - o.z) * inv.z;
            if (tp <= 0.0 || tp >= t) {
                continue;
            }
            let q = o + d * tp;
            let ps = sr_person(vec2<f32>(q.x - pp.x, q.y), pp.y, pp.w > 0.5, r.key, s, (dist + tp * length(d)) * pix);
            if (ps.a > 0.0) {
                // lit from above, its own shadow side toward the floor
                let pe = mix(0.35, 1.0, smoothstep(0.2, pp.y, q.y)) * sr_light(vec3<f32>(q.x, q.y, pp.z), vec3<f32>(0.0, 0.0, -1.0), r) * 0.9;
                sc = mix(sc, vec4<f32>(ps.rgb, 0.0), ps.a);
                e = mix(e, pe, ps.a);
            }
        }
    }
    // a glossy floor carries the fixtures' streaks
    var sheen = 0.0;
    if (mat == 0u && face == 2u && (r.fit == 0u || r.fit == 5u || r.fit == 6u || r.fit == 7u) && !r.vacant) {
        let up = vec3<f32>(d.x, -d.y, d.z);
        let tc = (H - p.y) / max(-d.y, 1e-3);
        let pc = p + up * tc;
        if (pc.z < D && pc.x > 0.0 && pc.x < W) {
            sheen = sr_ceiling(pc.x, pc.z, r, fp * 3.0).a * 0.035 * detail;
        }
    }
    // far off: the fittings' and walls' mean
    let mean_alb = select(vec3<f32>(0.24, 0.22, 0.2), vec3<f32>(0.4, 0.39, 0.37), r.vacant);
    let alb = mix(mean_alb, sc.rgb, mix(0.55, 1.0, detail));
    out.night = alb * e * SR_GAIN * sr_surface + vec3<f32>(sc.a + sheen);
    // by day: the same room by its own light and the daylight near the window, its fixtures a little
    let near_win = 1.0 - smoothstep(0.0, 3.0, p.z);
    let lit_day = select(1.0, 0.25, r.vacant);
    out.day = alb * (e * 0.75 * lit_day + 0.3 * near_win) + vec3<f32>(sc.a * 0.04 * lit_day);
    return out;
}
