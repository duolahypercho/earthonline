// ---- rear-yard fences (eo-props `yards.rs`, `--yard-detail`; props_yards.rs) ----
// Appended to the props' colour shader. A fence piece is a unit box scaled to
// (thickness, height from its sunk foot, length) and sheared along its length
// by the ground's grade. Its broad faces are vertical boards: the board width
// is a nominal 6 in board's dressed 5-1/2 in (American Softwood Lumber
// Standard, NIST PS 20), each board its own shade of the vertex colour (the
// kit's unpainted cedar), the gaps between them dark; where the CC0 sets are
// on, the `Planks010` scan stood on end gives the grain, its relief and
// roughness round that tone. The top is the cap's end grain, the foot darker
// with splash and damp. Boards and gaps fade out where a pixel spans a board.

const PR_FENCE_BOARD_M: f32 = 0.1397;
// A framing piece (a post, a rail, the cap: eo-app yards.rs `timber`): plain timber, no boards.
const PR_FENCE_TIMBER: u32 = 1073741824u;
// A weathered cedar fence's silver grey (sRGB about 125, 118, 108; INFERRED
// from photographs of old unstained fences): the boards go to it board by board.
const PR_FENCE_WEATHERED: vec3<f32> = vec3<f32>(0.205, 0.181, 0.150);

fn pr_fence(s: ptr<function, Surface>, inst: PrInstance, local: vec3<f32>, dlx: vec3<f32>, dly: vec3<f32>) {
    // the piece's own metres: x across, y up from the foot, z along.
    let q = local * inst.scale;
    let dqx = dlx * inst.scale;
    let dqy = dly * inst.scale;
    let nl = pr_rot(-inst.yaw, (*s).n);
    // a stable seed per fence line: its heading and where its line passes the origin.
    let along = pr_rot(inst.yaw, vec3<f32>(0.0, 0.0, 1.0));
    let off = dot(inst.pos.xz, vec2<f32>(-along.z, along.x));
    let seed = vec2<f32>(round(off * 4.0), round(inst.yaw * 40.0));
    // the station along the whole line (pieces of one fence share their boards).
    let s_line = dot(inst.pos.xz, along.xz) + q.z;
    let timber = (inst.variant & PR_FENCE_TIMBER) != 0u;
    // weathering (with the yards' plants, PR_FENCE_AGED): each board its own share of
    // the silver grey, more toward the sun-bleached top; framing a little less.
    var tone = (*s).albedo;
    if (PR_FENCE_AGED) {
        let bw = floor(s_line / PR_FENCE_BOARD_M);
        let grey = 0.35 + 0.45 * st_hash(vec2<f32>(bw, 7.0) + seed) + 0.25 * smoothstep(0.4, 1.0, q.y / max(inst.scale.y, 0.5));
        tone = mix(tone, PR_FENCE_WEATHERED, clamp(grey * select(1.0, 0.8, timber), 0.0, 0.95));
        (*s).albedo = tone;
    }
    if (timber) {
        // plain timber: one shade a piece, the scan's grain along its length.
        let sh = 0.85 + 0.25 * st_hash(vec2<f32>(round(s_line * 2.0), 21.0) + seed);
        var a = tone * sh;
        if (cc0_on(CC0_RUN_WALLS)) {
            let l = CC0_L_PLANKS010;
            let inv = cc0_linv(l);
            let long_y = inst.scale.y > inst.scale.z;
            let uv = select(vec2<f32>(q.x + q.y, s_line), vec2<f32>(q.x + q.z, q.y), long_y) * inv;
            let t = cc0_tap(uv, vec2<f32>(dqx.y, dqx.z) * inv, vec2<f32>(dqy.y, dqy.z) * inv, l);
            let rel = t.alb.rgb / max(cc0_lmean(l).rgb, vec3<f32>(1e-3));
            a = a * max(mix(vec3<f32>(1.0), mix(vec3<f32>(dot(rel, CC0_LUMA)), rel, 0.5), 0.7), vec3<f32>(0.0));
        }
        (*s).albedo = a * mix(0.7, 1.0, smoothstep(0.05, 0.4, q.y));
        return;
    }
    if (abs(nl.y) > 0.7) {
        // the top: the cap's end grain, a little darker, rougher.
        (*s).albedo = tone * (0.78 + 0.12 * st_noise(vec2<f32>(s_line * 3.1, seed.x)));
        (*s).rough = min((*s).rough + 0.08, 1.0);
        return;
    }
    let board = s_line / PR_FENCE_BOARD_M;
    let bi = floor(board);
    let fb = fract(board);
    let fw = max(length(vec2<f32>(dqx.z, dqy.z)), 1e-5) / PR_FENCE_BOARD_M; // boards a pixel spans
    let detail = 1.0 - smoothstep(0.25, 0.7, fw);
    // each board its own shade (a weathered fence is patched board by board), and a gap between.
    let shade = mix(1.0, 0.82 + 0.3 * st_hash(vec2<f32>(bi, 13.0) + seed), detail);
    let edge = max(1.0 - smoothstep(0.0, 0.06, fb), 1.0 - smoothstep(1.0, 0.94, fb));
    var alb = tone * shade * mix(1.0, 0.35, edge * detail * 0.85);
    var rough = (*s).rough;
    var n = (*s).n;
    if (cc0_on(CC0_RUN_WALLS)) {
        // the scan stood on end: its u up the board, v along the fence, each board a fresh patch of it.
        let l = CC0_L_PLANKS010;
        let inv = cc0_linv(l);
        let uv = vec2<f32>(q.y, s_line) * inv + vec2<f32>(st_hash(vec2<f32>(bi, 3.0) + seed) * 7.0, 0.0);
        let t = cc0_tap(uv, vec2<f32>(dqx.y, dqx.z) * inv, vec2<f32>(dqy.y, dqy.z) * inv, l);
        let rel = t.alb.rgb / max(cc0_lmean(l).rgb, vec3<f32>(1e-3));
        alb = alb * max(mix(vec3<f32>(1.0), mix(vec3<f32>(dot(rel, CC0_LUMA)), rel, 0.5), 0.8), vec3<f32>(0.0));
        rough = clamp(rough * mix(1.0, t.rough / max(cc0_lmisc(l).x, 0.05), 0.7), 0.3, 1.0);
        // its relief: u up the board, v along the fence, on the face's own side.
        let up = vec3<f32>(0.0, 1.0, 0.0);
        let side = along * sign(dot(along, cross(up, n)) + 1e-4);
        let nm = normalize(n + up * t.n.x * 0.5 + side * t.n.y * 0.5);
        if (all(nm == nm)) {
            n = nm;
        }
    }
    // splash and damp at the foot (the foot is sunk; the ground meets it about 0.1 m up), grey weathering up the boards.
    let foot = smoothstep(0.1, 0.45, q.y);
    alb = alb * mix(0.62, 1.0, foot);
    (*s).albedo = alb;
    (*s).rough = rough;
    (*s).n = n;
}

// ---- the yards' objects and the houses' backs (props_lib2 yard_items.rs, yard_backs.rs) ----
// The materials' marker (params.w): 1 painted wood (the back stairs, the doors),
// 2 bare wood (a patio set, a raised bed, a barrel, a shed), 3 painted metal (the
// downspouts); 0 as built. Paint is the instance's, the lamp word's sRGB x 4
// (eo-app yards/backs.rs, `pack_tint`; a zero word keeps the model's colour).
// Wood has a grain: along the piece's direction its builder wrote in the UV
// (yard_items.rs `grain`), the CC0 `Planks010` scan where the scanned sets are
// on, each piece its own patch of it; paint on wood wears through to the grey of
// weathered bare wood where it is walked and rained on most (the treads, the deck,
// the rail's cap: the faces turned up), in patches along the grain
// (`--no-yard-wood`, PR_YARD_WOOD false: neither).
const PR_YARD_BARE: vec3<f32> = vec3<f32>(0.19, 0.17, 0.145);

fn pr_yard_item(s: ptr<function, Surface>, inst: PrInstance, kind: f32, local: vec3<f32>, uv: vec2<f32>, dlx: vec3<f32>, dly: vec3<f32>) {
    if (kind < 0.5) {
        return;
    }
    var alb = (*s).albedo;
    let painted = kind < 1.5 || kind > 2.5;
    if (painted && inst.lamp != 0u) {
        alb = alb * srgb_decode(unpack4x8unorm(inst.lamp).rgb) * 4.0;
    }
    // (`--no-yard-wood`: the flat colours, the paint only.)
    if (kind > 2.5 || !PR_YARD_WOOD) {
        (*s).albedo = alb;
        return;
    }
    // the grain's direction in model space and the face's direction across it.
    let gx = uv.x * 2.0 - 1.0;
    let gz = uv.y * 2.0 - 1.0;
    let d = vec3<f32>(gx, sqrt(max(1.0 - gx * gx - gz * gz, 0.0)), gz);
    let nl = pr_rot(-inst.yaw, (*s).n);
    let q = local * inst.scale;
    let dqx = dlx * inst.scale;
    let dqy = dly * inst.scale;
    let c = cross(nl, d);
    let cl = length(c);
    let seed = st_hash(vec2<f32>(round(inst.pos.x * 4.0 + dot(round(q * 2.0), vec3<f32>(1.0, 7.0, 13.0))), round(inst.pos.z * 4.0))) * 40.0;
    if (cl < 0.35) {
        // end grain: darker, rougher.
        (*s).albedo = alb * 0.78;
        (*s).rough = min((*s).rough + 0.08, 1.0);
        return;
    }
    let t = c / cl;
    let along = dot(q, d);
    let across = dot(q, t);
    var n = (*s).n;
    var rough = (*s).rough;
    if (painted) {
        let up = max(nl.y, 0.0);
        let n1 = st_noise(vec2<f32>(along * 1.1 + seed, across * 7.0));
        let n2 = st_noise(vec2<f32>(along * 6.0 + seed * 1.7, across * 31.0));
        let wear = smoothstep(0.58, 0.8, 0.65 * n1 + 0.35 * n2 + 0.32 * up * up - 0.14);
        alb = mix(alb, PR_YARD_BARE * (0.8 + 0.4 * n2), wear * 0.9);
        rough = mix(rough, 0.92, wear);
    }
    if (cc0_on(CC0_RUN_WALLS)) {
        let l = CC0_L_PLANKS010;
        let inv = cc0_linv(l);
        let tuv = vec2<f32>(along, across) * inv + vec2<f32>(seed * 0.37, seed * 0.11);
        let tap = cc0_tap(tuv, vec2<f32>(dot(dqx, d), dot(dqx, t)) * inv, vec2<f32>(dot(dqy, d), dot(dqy, t)) * inv, l);
        let rel = tap.alb.rgb / max(cc0_lmean(l).rgb, vec3<f32>(1e-3));
        // (under paint the grain shows as relief and a little value; bare, its colour too.)
        let k = select(0.85, 0.3, painted);
        alb = alb * max(mix(vec3<f32>(1.0), mix(vec3<f32>(dot(rel, CC0_LUMA)), rel, 0.5), k), vec3<f32>(0.0));
        let dw = pr_rot(inst.yaw, d);
        let tw = pr_rot(inst.yaw, t);
        let nm = normalize(n + dw * tap.n.x * 0.45 + tw * tap.n.y * 0.45);
        if (all(nm == nm)) {
            n = nm;
        }
        rough = clamp(rough * mix(1.0, tap.rough / max(cc0_lmisc(l).x, 0.05), 0.5), 0.3, 1.0);
    }
    (*s).albedo = alb;
    (*s).rough = rough;
    (*s).n = n;
}
