// the city's lights each frame, GPU-driven (appended to look_common.wgsl
// and sky_light's SkyLight record): (1) gather the street lamps near the
// camera and two low-beam headlamps per lit vehicle from the agent instances
// into one frame list (one invocation per lamp and per agent, an atomic
// append), (2) assign them to a froxel grid (16 x 9 tiles x 24 log-depth
// slices, one workgroup per cluster, at most 32 lights each), which the city
// shader loops over (sky_light.wgsl `sky_local_lights`). No CPU work per
// light per frame. The street lamps are the props slice's real ones (the
// JS district lights with the JS lamp-pool optics, `PropsGpu::lights_buffer`)
// where its tiles are resident, and the JS fallback placement elsewhere.

struct SkyLight {
    pos: vec3<f32>,
    intensity: f32,
    color: vec3<f32>,
    range: f32,
    dir: vec3<f32>,
    cos_outer: f32,
    cos_inner: f32,
    kind: u32,
    size: f32,
    pad: f32,
};

struct LtAgent {
    pos: vec3<f32>,
    model: u32,
    rot: vec4<f32>,
    color: u32,
    flags: u32,
    anim: f32,
    steer: f32,
    speed: f32,
    scale: f32,
    clip_phase: f32,
    clip: u32,
};

struct LtModel {
    meshes: vec4<u32>,
    wheel: array<vec4<f32>, 4>,
    center: vec3<f32>,
    radius: f32,
    model_class: u32,
    skeleton: u32,
    joints: u32,
    wheel_radius: f32,
    pad: vec4<u32>,
};

struct LtParams {
    // x street lamps, y agents, z frame cap, w agents valid (1/0)
    counts: vec4<u32>,
    // x units per candela (lamps on), y headlamp units per candela (lamps on), z lamp reach m, w headlamp reach m
    scale: vec4<f32>,
    // props lamps: x units per JS candela (lamps on), y any props tile resident (1/0),
    // z the RP-8 pool peak in units (lamps on), w a tunnel bore's lamps' pool
    // peak in units (`--tunnels`: lit round the clock; 0 without bores)
    props: vec4<f32>,
    // the waterside cells: x, z of the grid's corner, cell m, unused
    water: vec4<f32>,
    // the waterside cells: columns, rows
    water_n: vec4<u32>,
    // (lighting, `--light-shadows`) SF's 3000 K LED lamps: rgb their colour at unit
    // luminance, w 1 on (0: the colours as given); `led_key` the JS street lamp role's
    // chromaticity key (largest channel 1), w the tolerance (sky.rs `led_params`).
    led: vec4<f32>,
    led_key: vec4<f32>,
    // (`--shop-light`, stage N1; shop_lights.rs) the shop windows' lights: x how
    // many, y units per candela (the lamps' scale times the rooms' dusk; 0 by
    // day or without them), z the drawn hour, w the eye distance they are gathered within.
    shops: vec4<f32>,
};

/// eo_render::props::GpuLampLight (64 bytes): a district street light's
/// luminaire with the JS lamp practical (a spot aimed down from a virtual
/// source `source_lift` m above the head; `lampPoolOptics`).
struct PropLamp {
    pos: vec3<f32>,
    range: f32,
    colour: vec3<f32>,
    intensity: f32,
    angle: f32,
    penumbra: f32,
    decay: f32,
    source_lift: f32,
    kind: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};

@group(0) @binding(0) var<uniform> skyu: Sky;
@group(0) @binding(1) var<storage, read> lt_lamps: array<SkyLight>;
@group(0) @binding(2) var<storage, read_write> lt_frame: array<SkyLight>;
@group(0) @binding(3) var<storage, read_write> lt_count: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> lt_clusters: array<u32>;
@group(0) @binding(5) var<storage, read> lt_agents: array<LtAgent>;
@group(0) @binding(6) var<storage, read> lt_models: array<LtModel>;
@group(0) @binding(7) var<uniform> lt: LtParams;
// The luminaires drawn this frame (near and far, to SPRITE_REACH): the fx
// pass draws each one's lens and air light, and under the far ones (past the
// clusters' reach, not shaded) the pool it throws on the street as a decal.
@group(0) @binding(8) var<storage, read_write> lt_sprites: array<SkyLight>;
// The props slice's lamp lights and `[slots in use, capacity, 0, 0]`.
@group(0) @binding(9) var<storage, read> lt_props: array<PropLamp>;
@group(0) @binding(10) var<storage, read> lt_props_n: array<u32>;
// Bit masks: the props tiles resident (STREET_TILE_M keys -128..127 on each
// axis, LT_TILE_WORDS words), then the waterside cells.
@group(0) @binding(11) var<storage, read> lt_mask: array<u32>;

/// eo_render::shop_lights::GpuShopLight (64 bytes): a shop window's light
/// just inside its glass, its trade's colour at unit luminance, its candela
/// while open, its axis out of the window, the glazed area it stands for,
/// its hours (open, hours lit from then, its level while closed).
struct ShopLamp {
    pos: vec3<f32>,
    range: f32,
    colour: vec3<f32>,
    candela: f32,
    dir: vec3<f32>,
    area: f32,
    open: f32,
    span: f32,
    security: f32,
    kind: u32,
};
@group(0) @binding(12) var<storage, read> lt_shops: array<ShopLamp>;

const LT_TILE_WORDS: u32 = 2048u;

const LT_SPRITE_REACH: f32 = 4000.0;
const LT_SPRITE_CAP: u32 = 32768u;

const LT_CAP: u32 = 32u;
const LT_PERSON: u32 = 1u;

fn lt_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

fn lt_view(p: vec3<f32>) -> vec3<f32> {
    let r = p - skyu.eye.xyz;
    return vec3<f32>(dot(r, skyu.cam_right.xyz), dot(r, skyu.cam_up.xyz), dot(r, skyu.cam_fwd.xyz));
}

/// In front of the camera, inside the cluster range and the widened frustum.
fn lt_visible(p: vec3<f32>, r: f32) -> bool {
    let v = lt_view(p);
    if (v.z < -r || v.z - r > skyu.cluster.y) {
        return false;
    }
    let z = max(v.z, 0.1);
    let hx = z * skyu.cluster.z * skyu.cluster.w + r * 1.5;
    let hy = z * skyu.cluster.z + r * 1.5;
    return abs(v.x) <= hx && abs(v.y) <= hy;
}

fn lt_push(l: SkyLight) {
    let i = atomicAdd(&lt_count[0], 1u);
    if (i < lt.counts.z) {
        lt_frame[i] = l;
    }
}

/// (lighting, `--light-shadows`) A street lamp of the JS role (its colour's
/// chromaticity is the role's 0xffc46a) in SF's 3000 K LED colour at the same
/// luminance; any other light, and everything with the option off, as given.
fn lt_led(c: vec3<f32>) -> vec3<f32> {
    if (lt.led.w <= 0.0) {
        return c;
    }
    let k = c / max(max(c.r, max(c.g, c.b)), 1e-6);
    if (any(abs(k - lt.led_key.xyz) > vec3<f32>(lt.led_key.w))) {
        return c;
    }
    return lt.led.xyz * dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn lt_bit(word: u32, bit: u32) -> bool {
    return ((lt_mask[word + bit / 32u] >> (bit % 32u)) & 1u) != 0u;
}

/// A props tile is resident where `p` stands: its real lamps are drawn there.
fn lt_props_tile(p: vec3<f32>) -> bool {
    if (lt.props.y < 0.5) {
        return false;
    }
    let k = vec2<i32>(floor(p.xz / LT_TILE_M)) + vec2<i32>(128);
    if (any(k < vec2<i32>(0)) || any(k > vec2<i32>(255))) {
        return false;
    }
    return lt_bit(0u, u32(k.y * 256 + k.x));
}

/// `p` is in a cell within the waterside reach of the shore (eo-water's shore field).
fn lt_waterside(p: vec3<f32>) -> bool {
    let c = vec2<i32>(floor((p.xz - lt.water.xy) / max(lt.water.z, 1.0)));
    if (any(c < vec2<i32>(0)) || c.x >= i32(lt.water_n.x) || c.y >= i32(lt.water_n.y)) {
        return false;
    }
    return lt_bit(LT_TILE_WORDS, u32(c.y) * lt.water_n.x + u32(c.x));
}

// ---- (`--look cinematic`, stage L2; eo-sky `cine_lamps`, sky.rs `cine_lamps_wgsl`) ----
// The look's street lamps: each takes its street's colour from the palette
// grid (`lt.water.w` its cell, 0 off; `lt.water_n.zw` its columns and rows;
// its corner the waterside grid's) at the light's own luminance, its pool's
// axis along the street (a lean of `dir`, sky_light.wgsl `sky_cine_cobra`),
// and the cobra head's longer reach. Compiled out without the look (LT_CINE).

fn lt_cine_on() -> bool {
    return LT_CINE && lt.water.w > 0.0;
}

/// The street at `p`: x its tint (0-3), y 1 where a street is, zw its axis.
fn lt_cine_street(p: vec3<f32>) -> vec4<f32> {
    let c = vec2<i32>(floor((p.xz - lt.water.xy) / lt.water.w));
    if (any(c < vec2<i32>(0)) || c.x >= i32(lt.water_n.z) || c.y >= i32(lt.water_n.w)) {
        return vec4<f32>(0.0);
    }
    let i = u32(c.y) * lt.water_n.z + u32(c.x);
    let b = (lt_mask[LT_PALETTE_WORD + i / 4u] >> ((i % 4u) * 8u)) & 255u;
    if ((b & 128u) == 0u) {
        return vec4<f32>(0.0);
    }
    let a = f32((b >> 2u) & 15u) / 16.0 * 3.14159265;
    return vec4<f32>(f32(b & 3u), 1.0, cos(a), sin(a));
}

/// A colour of the JS street lamp role (0xffc46a), whatever its luminance.
fn lt_cine_key(c: vec3<f32>) -> bool {
    let k = c / max(max(c.r, max(c.g, c.b)), 1e-6);
    return all(abs(k - LT_CINE_KEY) <= vec3<f32>(LT_CINE_KEY_TOL));
}

/// A street luminaire under the look, its head `h` m over the street:
/// `tint` 4 its street's (else that tint), `round` no axis (a globe).
fn lt_cine_lamp(l: ptr<function, SkyLight>, head: vec3<f32>, h: f32, tint: u32, round: bool) {
    let st = lt_cine_street(head);
    let t = select(tint, u32(st.x), tint > 3u);
    (*l).color = LT_CINE_TINTS[t] * dot((*l).color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let ax = select(vec2<f32>(0.0), st.zw * LT_CINE_AXIS_TILT, st.y > 0.5 && !round);
    (*l).dir = normalize(vec3<f32>(ax.x, -1.0, ax.y));
    (*l).range = h * LT_CINE_RANGE_MH;
}

/// A street lamp into the frame's lights (near) and the luminaires (to the
/// sprite reach). `head` is the luminaire (the lens; the source may stand
/// above it).
fn lt_emit(l: SkyLight, head: vec3<f32>) {
    let near = lt_visible(l.pos, l.range);
    if (near) {
        lt_push(l);
    }
    // The luminaire (and, far, its pool) for the fx pass.
    let v = lt_view(head);
    if (v.z > 0.3 && v.z < LT_SPRITE_REACH) {
        let z = v.z;
        let by_water = lt_waterside(head);
        var shown = abs(v.x) <= z * skyu.cluster.z * skyu.cluster.w + l.range && abs(v.y) <= z * skyu.cluster.z + l.range;
        if (!shown && by_water) {
            // A waterside lamp over the frame's top whose image in the sea is in it.
            let m = lt_view(vec3<f32>(head.x, 2.0 * SKY_SEA_Y - head.y, head.z));
            shown = m.z > 0.3 && abs(m.x) <= m.z * skyu.cluster.z * skyu.cluster.w + l.range && abs(m.y) <= m.z * skyu.cluster.z + l.range;
        }
        if (shown) {
            let k = atomicAdd(&lt_count[1], 1u);
            if (k < LT_SPRITE_CAP) {
                var s = l;
                // bit 0: shaded by the clusters (no pool decal); bit 1: by the water (its glitter in the sea).
                s.pad = select(0.0, 1.0, near && v.z - l.range <= skyu.cluster.y) + select(0.0, 2.0, by_water);
                lt_sprites[k] = s;
            }
        }
    }
}

@compute @workgroup_size(64)
fn lights_lamps(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= lt.counts.x || lt.scale.x <= 0.0) {
        return;
    }
    var l = lt_lamps[i];
    // The fallback placement stands down where the props slice's real lamps are resident.
    if (lt_props_tile(l.pos)) {
        return;
    }
    l.intensity = l.intensity * lt.scale.x;
    if (LT_LAMP_SPREAD && l.kind == 0u) {
        // (`--lamp-spread`) the cobra head's reach (`size`: the head's height).
        l.range = l.size * LT_CINE_RANGE_MH;
    }
    if (lt_cine_on() && l.kind == 0u && lt_cine_key(l.color)) {
        // (`--look cinematic`) its street's colour, axis and reach (`size`: the head's height).
        lt_cine_lamp(&l, l.pos, l.size, 4u, false);
    } else {
        l.color = lt_led(l.color);
    }
    lt_emit(l, l.pos);
}

/// `lampPracticals.sourceLiftMH`: the JS lamp practical's virtual source
/// stands this many head heights over the head.
const LT_JS_SOURCE_LIFT_MH: f32 = 0.57;

/// The props slice's district lights: the JS lamp practical as three.js
/// draws it (a spot from the virtual source `source_lift` over the head,
/// aimed down, cone `angle`, `penumbra`, inverse square to `range`). Its
/// intensity is set so the pool's nadir is the RP-8 peak the fallback's
/// lamps give (`LAMP_POOL_PEAK_LUX`; the JS's own 150 cd in its units would
/// give 36-107 lux by head height); a lamp without a lift keeps the JS's.
@compute @workgroup_size(64)
fn lights_props(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= min(lt_props_n[0], arrayLength(&lt_props)) || (lt.props.x <= 0.0 && lt.props.w <= 0.0)) {
        return;
    }
    let p = lt_props[i];
    // a tunnel bore's lamp (pad0 1, `--tunnels`) is lit round the clock at the bore's own pool.
    let tunnel = p.pad0 == 1u;
    if (p.intensity <= 0.0 || select(lt.props.x, lt.props.w, tunnel) <= 0.0) {
        return;
    }
    var l: SkyLight;
    l.pos = p.pos + vec3<f32>(0.0, p.source_lift, 0.0);
    let hs = p.source_lift / LT_JS_SOURCE_LIFT_MH + p.source_lift;
    l.intensity = select(p.intensity * lt.props.x, select(lt.props.z, lt.props.w, tunnel) * hs * hs, p.source_lift > 0.0);
    l.color = select(lt_led(p.colour), p.colour, tunnel);
    l.range = p.range;
    l.dir = vec3<f32>(0.0, -1.0, 0.0);
    l.cos_outer = cos(p.angle);
    l.cos_inner = cos(p.angle * (1.0 - p.penumbra));
    l.kind = 3u;
    l.size = p.source_lift;
    l.pad = 0.0;
    if (LT_LAMP_SPREAD && !lt_cine_on() && !tunnel && p.kind != LT_LANDMARK && p.source_lift > 0.0 && lt_cine_key(p.colour)) {
        // (`--lamp-spread`) the fallback's type III cobra head (kind 0) at its own
        // head: the batwing over the street, its nadir the RP-8 peak as before.
        let h = p.source_lift / LT_JS_SOURCE_LIFT_MH;
        l.pos = p.pos;
        l.intensity = lt.props.z * h * h;
        l.kind = 0u;
        l.size = h;
        l.cos_outer = LT_CINE_COS_CUT;
        l.cos_inner = LT_CINE_COS_PEAK;
        l.range = h * LT_CINE_RANGE_MH;
    }
    if (lt_cine_on() && !tunnel && p.kind != LT_LANDMARK && p.source_lift > 0.0 && lt_cine_key(p.colour)) {
        // (`--look cinematic`) a cobra head (kind 0) at its own head, its
        // nadir the RP-8 peak as before; the historic and banner heads warm
        // white, a historic globe's pool round.
        let h = p.source_lift / LT_JS_SOURCE_LIFT_MH;
        l.pos = p.pos;
        l.intensity = lt.props.z * h * h;
        l.color = p.colour;
        l.kind = 0u;
        l.size = h;
        l.cos_outer = LT_CINE_COS_CUT;
        l.cos_inner = LT_CINE_COS_PEAK;
        let warm = p.kind == LT_MODEL_HISTORIC || p.kind == LT_MODEL_BANNER;
        lt_cine_lamp(&l, p.pos, h, select(4u, 3u, warm), p.kind == LT_MODEL_HISTORIC);
    }
    lt_emit(l, p.pos);
}

@compute @workgroup_size(64)
fn lights_agents(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (lt.counts.w == 0u || i >= lt.counts.y || lt.scale.y <= 0.0) {
        return;
    }
    let a = lt_agents[i];
    let m = lt_models[a.model];
    if (m.model_class == LT_PERSON) {
        return;
    }
    // A parked car (eo-app props: stopped, speed 0, wheels never turned) has its lamps off.
    if (a.flags == 2u && a.speed == 0.0 && a.anim == 0.0) {
        return;
    }
    if (distance(a.pos, skyu.eye.xyz) > lt.scale.w + 20.0) {
        return;
    }
    // The headlamps: above the front wheels' pivots, at the nose; aimed 14 m
    // ahead and 1.15 m down (vehicle-presentation.js VEHICLE_LIGHTS), cone
    // 0.42 rad, penumbra 0.55.
    let fl = m.wheel[0].xyz;
    var lat = abs(fl.x) * 0.78;
    var up = fl.y + 0.28;
    var fwd = fl.z + 0.8;
    if (lat < 0.2) {
        lat = 0.62;
        up = 0.7;
        fwd = m.center.z + m.radius * 0.85;
    }
    let axis = normalize(lt_rotate(a.rot, vec3<f32>(0.0, -0.0819, 1.0)));
    let outer = 0.42;
    for (var s = 0; s < 2; s = s + 1) {
        let side = select(-1.0, 1.0, s == 0);
        var l: SkyLight;
        l.pos = a.pos + lt_rotate(a.rot, vec3<f32>(side * lat, up, fwd) * a.scale);
        l.intensity = lt.scale.y;
        l.color = vec3<f32>(1.0, 0.96, 0.88) / 0.9667;
        l.range = lt.scale.w;
        l.dir = axis;
        l.cos_outer = cos(outer);
        l.cos_inner = cos(outer * 0.45);
        l.kind = 1u;
        l.size = 0.0;
        l.pad = 0.0;
        if (lt_visible(l.pos + axis * l.range * 0.5, l.range * 0.6)) {
            lt_push(l);
        }
    }
}

/// (`--shop-light`, stage N1) The shop windows' lights near the eye: lit
/// by their hours at the drawn hour (`Hours::level_at`), appended after every
/// other light (the frame list's cap drops them first), each a spot of its
/// window's lobe (sky_light.wgsl kinds 3 and up: full out along its axis, none
/// behind its glass), `size` its glazed area (rooms.wgsl reads the window's
/// luminance from it). No lens or pool is drawn for it (no sprite).
@compute @workgroup_size(64)
fn lights_shops(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= min(u32(lt.shops.x), arrayLength(&lt_shops)) || lt.shops.y <= 0.0) {
        return;
    }
    let p = lt_shops[i];
    let since = (lt.shops.z - p.open) - floor((lt.shops.z - p.open) / 24.0) * 24.0;
    let level = select(p.security, 1.0, p.span >= 24.0 || since < p.span);
    if (level <= 0.0 || distance(p.pos, skyu.eye.xyz) > lt.shops.w) {
        return;
    }
    var l: SkyLight;
    l.pos = p.pos;
    l.intensity = p.candela * level * lt.shops.y;
    l.color = p.colour;
    l.range = p.range;
    l.dir = p.dir;
    l.cos_outer = 0.0;
    l.cos_inner = 1.0;
    l.kind = SHOP_KIND;
    l.size = p.area;
    l.pad = 0.0;
    if (lt_visible(l.pos, l.range)) {
        lt_push(l);
    }
}

var<workgroup> lt_wcount: atomic<u32>;

@compute @workgroup_size(64)
fn lights_cluster(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let g = skyu.lights.yzw;
    let c = wid.x;
    if (lid == 0u) {
        atomicStore(&lt_wcount, 0u);
    }
    workgroupBarrier();
    let tx = c % g.x;
    let ty = (c / g.x) % g.y;
    let tz = c / (g.x * g.y);
    let near = skyu.cluster.x;
    let far = skyu.cluster.y;
    let z0 = near * pow(far / near, f32(tz) / f32(g.z));
    let z1 = near * pow(far / near, f32(tz + 1u) / f32(g.z));
    let x0 = -1.0 + 2.0 * f32(tx) / f32(g.x);
    let x1 = -1.0 + 2.0 * f32(tx + 1u) / f32(g.x);
    let y1 = 1.0 - 2.0 * f32(ty) / f32(g.y);
    let y0 = 1.0 - 2.0 * f32(ty + 1u) / f32(g.y);
    let sx = skyu.cluster.z * skyu.cluster.w;
    let sy = skyu.cluster.z;
    let lo = vec3<f32>(min(x0 * z0, x0 * z1) * sx, min(y0 * z0, y0 * z1) * sy, z0);
    let hi = vec3<f32>(max(x1 * z0, x1 * z1) * sx, max(y1 * z0, y1 * z1) * sy, z1);
    let n = min(atomicLoad(&lt_count[0]), lt.counts.z);
    let total = g.x * g.y * g.z;
    // (`--shop-light`) the street lamps and vehicle lamps first, then the shop
    // windows' lights in what room is left (none without them: one pass).
    let phases = select(1u, 2u, lt.shops.x > 0.0);
    for (var phase = 0u; phase < phases; phase = phase + 1u) {
        for (var i = lid; i < n; i = i + 64u) {
            let l = lt_frame[i];
            if (phases == 2u && (l.kind == SHOP_KIND) != (phase == 1u)) {
                continue;
            }
            let v = lt_view(l.pos);
            let q = clamp(v, lo, hi);
            let dd = v - q;
            if (dot(dd, dd) <= l.range * l.range) {
                let k = atomicAdd(&lt_wcount, 1u);
                if (k < LT_CAP) {
                    lt_clusters[total + c * LT_CAP + k] = i;
                }
            }
        }
        workgroupBarrier();
    }
    if (lid == 0u) {
        lt_clusters[c] = min(atomicLoad(&lt_wcount), LT_CAP);
        if (c == 0u) {
            // The luminaires' indirect draw: 12 vertices (lens + pool decal) x the frame's sprites.
            atomicStore(&lt_count[4], 12u);
            atomicStore(&lt_count[5], min(atomicLoad(&lt_count[1]), LT_SPRITE_CAP));
            atomicStore(&lt_count[6], 0u);
            atomicStore(&lt_count[7], 0u);
        }
    }
}
