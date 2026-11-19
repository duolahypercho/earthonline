// the landmark surfaces (src/render/materials/landmark-surface.js,
// `paintedSteel` and `concrete`) and the set's fog cap (landmark-set.js
// `landmarkFog`), appended to the city shader after draw.wgsl, look_light
// and facade_age (look.rs `city_shader_source`). Constants LM_* are
// generated from the JS tables (landmarks.rs `landmarks_constants_wgsl`).
//
// Every term is a function of the fragment's world position and face frame,
// with the landmark's anchor (material params.zw) as the noise origin. The
// JS takes the bump's height derivatives with dFdx / dFdy; the branch here is
// not in uniform control flow, so the height is differenced at the pixel's
// neighbours (world + dpdx, world + dpdy, taken by the caller).

fn lm_pcg(u: u32) -> u32 {
    let state = u * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn lm_cell_hash(c: vec3<f32>) -> f32 {
    let x = u32(c.x + 32768.0);
    let y = u32(c.y + 32768.0);
    let z = u32(c.z + 32768.0);
    return f32(lm_pcg(x ^ lm_pcg(y ^ lm_pcg(z)))) * (1.0 / 4294967296.0);
}

// Value noise in [-1, 1] over unit cells, cubic between the corner hashes.
fn lm_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x00 = mix(lm_cell_hash(i + vec3<f32>(0.0, 0.0, 0.0)), lm_cell_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let x10 = mix(lm_cell_hash(i + vec3<f32>(0.0, 1.0, 0.0)), lm_cell_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let x01 = mix(lm_cell_hash(i + vec3<f32>(0.0, 0.0, 1.0)), lm_cell_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let x11 = mix(lm_cell_hash(i + vec3<f32>(0.0, 1.0, 1.0)), lm_cell_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z) * 2.0 - 1.0;
}

fn lm_ss(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn lm_line(c: f32, pitch: f32) -> f32 {
    let f = fract(c / pitch);
    return min(f, 1.0 - f) * pitch;
}

// `fadeByFootprint`: full strength until a quarter of the pitch per pixel, gone at 0.08.
fn lm_fade(footprint: f32, pitch: f32) -> f32 {
    return lm_ss(pitch * 0.25, pitch * 0.08, footprint);
}

fn lm_steel_height(uv: vec2<f32>) -> vec2<f32> {
    let seam_x = lm_line(uv.x, LM_PS_PLATE_U);
    let seam_y = lm_line(uv.y, LM_PS_PLATE_V);
    let seam = (1.0 - lm_ss(0.0, 0.02, min(seam_x, seam_y))) * -LM_PS_SEAM_DEPTH;
    let along = lm_line(uv.x, LM_PS_RIVET_PITCH);
    let across = abs(seam_y - 0.06);
    let rivet = lm_ss(LM_PS_RIVET_RADIUS, 0.0, length(vec2<f32>(along, across))) * LM_PS_RIVET_HEIGHT;
    return vec2<f32>(seam, rivet);
}

fn lm_concrete_height(uv: vec2<f32>) -> f32 {
    let joint = min(lm_line(uv.y, LM_CO_LIFT_M), lm_line(uv.x, LM_CO_FORM_M));
    return (1.0 - lm_ss(0.0, 0.025, joint)) * -LM_CO_JOINT_DEPTH;
}

// Mikkelsen's surface-gradient bump in world space, from the height at the
// pixel and its two neighbours.
fn lm_bump(n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, h: f32, hx: f32, hy: f32) -> vec3<f32> {
    let r1 = cross(dpy, n);
    let r2 = cross(n, dpx);
    let det = dot(dpx, r1);
    let grad = sign(det) * ((hx - h) * r1 + (hy - h) * r2);
    let b = abs(det) * n - grad;
    if (dot(b, b) < 1e-24) {
        return n;
    }
    return normalize(b);
}

// The surface of a landmark material (bits 25-27 of its flags): 1 painted
// steel, 2 cast concrete. `n` faces the camera; `world` is the fragment.
fn lm_surface(s: ptr<function, Surface>, m: Material, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let kind = (m.flags >> LM_SURFACE_SHIFT) & 7u;
    if (kind == 1u) {
        (*s).env = ENV_PAINTED_METAL;
    }
    if (kind == 0u) {
        return;
    }
    if (kind == 3u) {
        lm_seawall(s, world, n);
        return;
    }
    if (kind == 7u) {
        // the Golden Gate's far item: only the towers' floodlighting (and, with
        // `finish`, the sea's light on its faces turned down).
        rm_emit = rm_emit + lm_floodlit(world, n, (*s).albedo);
        if (LM_FINISH_ON) {
            (*s).ao = (*s).ao * lm_bridge_sky_vis(world, n);
            rm_emit = rm_emit + lm_bridge_bounce(world, n, (*s).albedo, (*s).ao);
            lm_rim_vis = lm_bridge_rim();
        }
        return;
    }
    if (kind >= 4u) {
        lm_building_surface(s, m, kind, world, n, dpx, dpy);
        return;
    }
    let local = world - vec3<f32>(m.params.z, 0.0, m.params.w);
    let fr = fa_frame(n);
    let uv = vec2<f32>(dot(world, fr.t), dot(world, fr.b));
    let du = vec2<f32>(dot(dpx, fr.t), dot(dpx, fr.b));
    let dv = vec2<f32>(dot(dpy, fr.t), dot(dpy, fr.b));
    let footprint = ((abs(du.x) + abs(dv.x)) + (abs(du.y) + abs(dv.y))) * 0.5;
    let up = n.y;
    let vertical = clamp(1.0 - abs(up), 0.0, 1.0);
    // a bridge's own coat (eo-landmarks suspension.rs `finish_material`).
    let bridge = m.rough_flatten > 0.5;
    if (kind == 1u) {
        // the bridges' paint is renewed panel by panel and touched up: its
        // tone wanders little over a member (a fifth of the landmark steel's
        // 9 m patches, half its 2.6 m ones), where the landmark mottle drew blotches.
        let kp = select(1.0, 0.2, bridge);
        let kf = select(1.0, 0.5, bridge);
        let tone = lm_noise(local / LM_PS_PATCH_M) * LM_PS_PATCH_TONE * kp
            + lm_noise(local / LM_PS_FINE_PATCH_M + vec3<f32>(13.1, 7.7, 3.3)) * LM_PS_FINE_PATCH_TONE * kf;
        // Salt grime: darkest at the water, gone by grimeHeightM over it.
        let grime = mix(LM_PS_GRIME_TONE, 1.0, lm_ss(0.0, LM_PS_GRIME_HEIGHT_M, world.y));
        let streak = lm_noise(vec3<f32>(uv.x * 1.7, uv.y * 0.07, 0.0)) * LM_PS_STREAK_TONE * vertical;
        let chalk = lm_ss(0.4, 0.95, up) * LM_PS_CHALK_TONE;
        (*s).albedo = (*s).albedo * (vec3<f32>((1.0 + tone - abs(streak)) * grime) + chalk);
        if (bridge) {
            (*s).albedo = (*s).albedo * lm_bridge_plating(uv, du, dv, vertical);
            (*s).ao = (*s).ao * lm_gg_deck_contact(world) * lm_bridge_sky_vis(world, n);
            lm_key_vis = lm_deck_key(world);
            lm_rim_vis = lm_bridge_rim();
            if (LM_WEATHER_ON && lm_is_gg(world)) {
                lm_weather(s, world, local, n, uv, du, dv, footprint, vertical);
            }
            if (LM_LEGCELLS_ON && lm_is_gg(world)) {
                let sh = lm_gg_self_shade(world, n, footprint);
                lm_key_vis = lm_key_vis * sh.x;
                (*s).ao = (*s).ao * sh.y;
            }
        }
        // (the bridges' coat a quarter of the landmark steel's roughness spread:
        // one film, renewed as one; its patches under 0.5 took the sky's sharp
        // reflection and drew the clouds on the struts.)
        let spread = LM_PS_ROUGH_SPREAD * select(1.0, 0.25, bridge);
        (*s).rough = clamp((*s).rough + lm_noise(local / (LM_PS_PATCH_M * 0.7) + vec3<f32>(5.1, 2.3, 9.4)) * spread
            + chalk / max(1e-3, LM_PS_CHALK_TONE) * LM_PS_CHALK_ROUGH, 0.2, 0.95);
        let fs = lm_fade(footprint, LM_PS_PLATE_U);
        let fr2 = lm_fade(footprint, LM_PS_RIVET_PITCH);
        if (fs + fr2 > 0.0) {
            let h0 = lm_steel_height(uv);
            let hx = lm_steel_height(uv + du);
            let hy = lm_steel_height(uv + dv);
            (*s).n = lm_bump(n, dpx, dpy, h0.x * fs + h0.y * fr2, hx.x * fs + hx.y * fr2, hy.x * fs + hy.y * fr2);
        }
        rm_emit = rm_emit + lm_floodlit(world, (*s).n, (*s).albedo);
        if (bridge) {
            rm_emit = rm_emit + lm_bridge_bounce(world, (*s).n, (*s).albedo, (*s).ao);
            if (LM_PROBE != 0u && lm_is_gg(world)) {
                lm_probe(s, world, n);
            }
        }
    } else if (kind == 2u) {
        let tone = lm_noise(local / LM_CO_PATCH_M) * LM_CO_PATCH_TONE;
        let wet = mix(LM_CO_WET_TONE, 1.0, lm_ss(0.0, LM_CO_WET_BAND_M, world.y));
        let streak = lm_noise(vec3<f32>(uv.x * 2.3, uv.y * 0.11, 4.2)) * LM_CO_STREAK_TONE * vertical;
        (*s).albedo = (*s).albedo * vec3<f32>((1.0 + tone - abs(streak)) * wet);
        if (bridge && LM_CONCRETE2_ON && lm_is_gg(world)) {
            lm_concrete_age(s, world, n, uv, du, dv, footprint, vertical);
        }
        if (bridge) {
            (*s).ao = (*s).ao * lm_bridge_sky_vis(world, n);
            lm_key_vis = lm_deck_key(world);
            lm_rim_vis = lm_bridge_rim();
            lm_bridge_tide(s, world, vertical);
            rm_emit = rm_emit + lm_bridge_bounce(world, n, (*s).albedo, (*s).ao);
        }
        (*s).rough = clamp(((*s).rough + lm_noise(local / 3.1 + vec3<f32>(1.7, 8.2, 4.4)) * LM_CO_ROUGH_SPREAD)
            * mix(0.75, 1.0, lm_ss(0.0, LM_CO_WET_BAND_M, world.y)), 0.4, 1.0);
        let fj = lm_fade(footprint, LM_CO_LIFT_M);
        // (`pylons`) the Golden Gate's rough-board-formed concrete on its walls.
        let board = bridge && LM_BOARDFORM_ON && vertical > 0.7 && lm_is_gg(world);
        let fb = select(0.0, lm_fade(footprint, LM_GG_BOARD_M[0]), board);
        if (fb > 0.0) {
            (*s).albedo = (*s).albedo * (1.0 + lm_board(world, n).x * fb);
        }
        if (fj > 0.0 || fb > 0.0) {
            let h0 = lm_concrete_height(uv) * fj + lm_board(world, n).y * fb;
            let hx = lm_concrete_height(uv + du) * fj + lm_board(world + dpx, n).y * fb;
            let hy = lm_concrete_height(uv + dv) * fj + lm_board(world + dpy, n).y * fb;
            (*s).n = lm_bump(n, dpx, dpy, h0, hx, hy);
        }
    }
}

// The Golden Gate's concrete cast in "form lumber with the rough side
// against the concrete" (Strauss 1937; `pylons`): horizontal boards
// LM_GG_BOARD_M[0] wide and [1] long, their ends staggered row by row, each
// its own tone (the lumber's grain and its water take-up), the joints
// between rows a shallow ridge. Returns (tone, height) at a point of a wall.
fn lm_board(p: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let side = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let h = dot(p.xz, side);
    let row = floor(p.y / LM_GG_BOARD_M[0]);
    let seg = floor(h / LM_GG_BOARD_M[1] + fract(row * 0.618));
    let tone = (lm_cell_hash(vec3<f32>(row, seg, 11.0)) - 0.5) * 2.0 * LM_GG_BOARD_TONE;
    let joint = 1.0 - lm_ss(0.0, 0.008, lm_line(p.y, LM_GG_BOARD_M[0]));
    return vec2<f32>(tone, joint * LM_GG_BOARD_RIDGE_M);
}

// the Golden Gate towers' floodlighting at night (eo-landmarks lights.rs
// `gg_floodlit`): on the towers above the deck (the legs and the portal
// struts between them, inside the tower's plan box), the floods' mean
// illuminance LM_FLOOD_LUX from 1.4 x at the deck to 0.6 x at the top, on the
// vertical faces and the undersides (the floods stand below), none on the
// tops; diffuse, in the lamps' scale, with the lamps' switch-on.
fn lm_floodlit(p: vec3<f32>, n: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    if (LM_FLOODS2_ON) {
        return lm_floodlit2(p, n, albedo);
    }
    let lamps = skyu.night.x;
    if (!LM_FLOOD_ON || lamps <= 0.0 || p.y < LM_GG_DECK_Y || p.y > LM_GG_TOP_Y + 1.0) {
        return vec3<f32>(0.0);
    }
    let rel = p.xz - LM_GG_ORIGIN;
    let c = dot(rel, LM_GG_AXIS);
    let sl = dot(rel, vec2<f32>(-LM_GG_AXIS.y, LM_GG_AXIS.x));
    let ct = select(0.0, LM_GG_SPAN, c > LM_GG_SPAN * 0.5);
    if (abs(c - ct) > LM_GG_HALF_ALONG + 1.0 || abs(sl) > LM_GG_HP + LM_GG_HALF_ACROSS + 1.0) {
        return vec3<f32>(0.0);
    }
    let h = clamp((p.y - LM_GG_DECK_Y) / max(LM_GG_TOP_Y - LM_GG_DECK_Y, 1.0), 0.0, 1.0);
    let facing = max(1.0 - abs(n.y), -n.y);
    let e = LM_FLOOD_LUX * (1.4 - 0.8 * h) * facing;
    return albedo * (e / 3.14159265) * skyu.bloom_w2.z * lamps * LM_FLOOD_RGB;
}

// The box-filtered share of [x - f/2, x + f/2] that lines `w` wide at a
// `pitch` (centred on its multiples) cover: exact at any footprint, so a seam
// pattern goes to its mean (w / pitch) instead of aliasing as it recedes.
fn lm_lines_int(x: f32, pitch: f32, w: f32) -> f32 {
    let y = x + 0.5 * w;
    let k = floor(y / pitch);
    return k * w + clamp(y - k * pitch, 0.0, w);
}

fn lm_lines(x: f32, f: f32, pitch: f32, w: f32) -> f32 {
    let ff = max(f, 1e-4);
    return clamp((lm_lines_int(x + 0.5 * ff, pitch, w) - lm_lines_int(x - 0.5 * ff, pitch, w)) / ff, 0.0, 1.0);
}

// the bridges' plating (`finish`): each plate seam a dark line (the paint
// shadowed and dirt-filled in the joint, 4 cm), the rows of rivet heads 6 cm
// either side of the horizontal seams a lighter and a darker line (the heads'
// lit tops and their shadows, 3 cm each), on the landmark steel's plate grid
// (LM_PS_PLATE_U x LM_PS_PLATE_V; landmark-surface.js); filtered by the
// pixel's footprint. INFERRED: the line widths and tones.
fn lm_bridge_plating(uv: vec2<f32>, du: vec2<f32>, dv: vec2<f32>, vertical: f32) -> f32 {
    let fx = abs(du.x) + abs(dv.x);
    let fy = abs(du.y) + abs(dv.y);
    let seam = max(lm_lines(uv.x, fx, LM_PS_PLATE_U, 0.04), lm_lines(uv.y, fy, LM_PS_PLATE_V, 0.04));
    let lit_row = lm_lines(uv.y - 0.06, fy, LM_PS_PLATE_V, 0.03) + lm_lines(uv.y + 0.06, fy, LM_PS_PLATE_V, 0.03);
    let shade_row = lm_lines(uv.y - 0.085, fy, LM_PS_PLATE_V, 0.025) + lm_lines(uv.y + 0.035, fy, LM_PS_PLATE_V, 0.025);
    return 1.0 - 0.38 * seam + (0.10 * lit_row - 0.16 * shade_row) * vertical;
}

// The key light's visibility a bridge's surface sets for look_light (1 for
// every other surface: the initialiser runs per invocation).
var<private> lm_key_vis: f32 = 1.0;
// The rig's rim light's share a bridge's surface takes (`finish`; 1 for every
// other surface): none at night. The rig's night rim (0.3-0.4 from the
// west-north-west, four times its daytime value) stands for no source on the
// bay: it lit the west span's anchorage and towers a daytime grey over the
// dark water. The bridges at night take their lamps, floods and the sky.
var<private> lm_rim_vis: f32 = 1.0;

fn lm_bridge_rim() -> f32 {
    return 1.0 - clamp(skyu.night.x, 0.0, 1.0);
}

fn lm_gg_deck_top(c: f32) -> f32 {
    let cs = LM_GG_DECK[0];
    let ys = LM_GG_DECK[1];
    let l = LM_GG_DECK[2];
    let yt = LM_GG_DECK[3];
    let ym = LM_GG_DECK[4];
    let cn = LM_GG_DECK[5];
    let yn = LM_GG_DECK[6];
    if (c <= 0.0) {
        return ys + (yt - ys) * clamp((c - cs) / max(-cs, 1e-3), 0.0, 1.0);
    }
    if (c <= l) {
        let t = (c - 0.5 * l) / (0.5 * l);
        return yt + (ym - yt) * (1.0 - t * t);
    }
    return yt + (yn - yt) * clamp((c - l) / max(cn - l, 1e-3), 0.0, 1.0);
}

fn lm_smin(a: f32, b: f32, k: f32) -> f32 {
    return (a + b - sqrt((a - b) * (a - b) + k * k)) * 0.5;
}

fn lm_bay_deck_top(c: f32) -> f32 {
    let g = LM_BAY_DECK[5];
    return lm_smin(lm_smin(LM_BAY_DECK[4], LM_BAY_DECK[2] + g * (c - LM_BAY_DECK[0]), LM_BAY_ROUND_M), LM_BAY_DECK[3] + g * (LM_BAY_DECK[1] - c), LM_BAY_ROUND_M);
}

// The deck's shadow on what stands under it (`finish`): the sun's visibility
// at a point of a bridge, 0 where the ray toward the sun meets the deck
// plate above it (the Golden Gate's roadway; on the west span the lower
// deck for what is under it, else the upper) inside the deck's width and
// length, with a 0.8 m soft edge (the sun's half degree and the railings).
// The decks' lines are the builds' own (eo-landmarks suspension.rs
// `deck_line`), on each bridge's straight frame (the Golden Gate's
// approaches' curve is left out). The shadow maps reach 160 m from the eye:
// past them this is the only shadow the members under a deck receive.
fn lm_deck_key(p: vec3<f32>) -> f32 {
    let l = look.key_dir.xyz;
    if (!LM_DECKSHADE_ON || l.y < 0.02) {
        return 1.0;
    }
    let gg = distance(p.xz, LM_GG_ORIGIN) < distance(p.xz, LM_BAY_ORIGIN);
    let o = select(LM_BAY_ORIGIN, LM_GG_ORIGIN, gg);
    let ax = select(LM_BAY_AXIS, LM_GG_AXIS, gg);
    let across = vec2<f32>(-ax.y, ax.x);
    let rel = p.xz - o;
    let c = dot(rel, ax);
    let hw = select(LM_BAY_DECK[7], LM_GG_DECK[7], gg);
    if (abs(dot(rel, across)) > hw + 1.0) {
        return 1.0;
    }
    var c0 = LM_BAY_DECK[0];
    var c1 = LM_BAY_DECK[1];
    var top = lm_bay_deck_top(c);
    if (gg) {
        c0 = LM_GG_DECK[0];
        c1 = LM_GG_DECK[5];
        top = lm_gg_deck_top(c);
    } else if (p.y < top - LM_BAY_DECK[6] - 0.25) {
        top = top - LM_BAY_DECK[6];
    }
    let dh = top - 0.3 - p.y;
    if (c < c0 || c > c1 || dh < 0.05) {
        return 1.0;
    }
    let hit = rel + l.xz * (dh / l.y);
    let ch = dot(hit, ax);
    let inside = (1.0 - lm_ss(hw - 0.4, hw + 0.4, abs(dot(hit, across)))) * step(c0, ch) * step(ch, c1);
    return 1.0 - inside;
}

// The Golden Gate legs' occlusion where they pass through the deck
// (`finish`): the sky a leg's face sees within a couple of metres over the
// roadway is cut by the deck's railings, the sidewalk round the leg and the
// leg's own collar; the frame's screen-space occlusion misses it at a
// distance. A band from half the sky at the deck to all of it 2.5 m up.
// INFERRED: the band's height and depth.
fn lm_gg_deck_contact(p: vec3<f32>) -> f32 {
    let dy = p.y - LM_GG_DECK_Y;
    if (dy < -0.5 || dy > 2.5) {
        return 1.0;
    }
    let across = vec2<f32>(-LM_GG_AXIS.y, LM_GG_AXIS.x);
    let rel = p.xz - LM_GG_ORIGIN;
    let c0 = dot(rel, LM_GG_AXIS);
    let c = c0 - select(0.0, LM_GG_SPAN, c0 > LM_GG_SPAN * 0.5);
    let sl = abs(dot(rel, across));
    if (abs(c) > LM_GG_HALF_ALONG + 1.0 || abs(sl - LM_GG_HP) > LM_GG_HALF_ACROSS + 1.0) {
        return 1.0;
    }
    return mix(0.5, 1.0, lm_ss(0.0, 2.5, dy));
}

// The light the sea reflects onto a bridge's faces turned down (`finish`;
// eo-landmarks suspension.rs `SEA_ALBEDO`), irradiance in the rig's units:
// the sea as an isotropic reflector (the isotropic ground term of Liu and
// Jordan, Solar Energy 4, 1960), its albedo times the global horizontal
// irradiance (the key on the level times its cloud transmittance, and the
// sky's irradiance on the level: the rig's hemisphere, ambient and
// environment terms at n = up), over the share (1 - n.y) / 2 of the face's
// hemisphere the sea fills. The rig's hemisphere delivers its own ground
// colour over that share, so what is returned is the sea's light less it:
// by day the sea's reflection over the rig's ground, at night (no sun for the
// sea to reflect, the rig's ground lobe near its daytime value) the rig's
// unsourced ground light taken off again, so a bridge's concrete and steel
// stand dark over the dark bay. The deck's shadow on the water is left out
// (a few per cent of what a strut's underside sees).
fn lm_bridge_bounce_e(world: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let key = look.key.xyz * max(look.key_dir.y, 0.0) * sky_cloud_transmittance(world);
    var env_i = look.ambient.w * look.env_class.x;
    var ground = look.hemi_ground.xyz;
    if (LM_DIELECTRIC_ON && lm_gg_over_water(world)) {
        // (`dielectric`, the Golden Gate) the physical daylight fades the rig's
        // ground out (eo-look phys.rs `apply`) and delivers the ground's light
        // in the environment's lower hemisphere instead (its sky light over the
        // land's albedo): the irradiance a face turned straight down takes from
        // it is lk_sh(-up), the share a face sees (1 - n.y) / 2 of it. That is
        // the light the sea's replaces, so it is what comes off, else the
        // ground was counted twice. The sky's irradiance on the level is the
        // environment's at its own intensity under the daylight (look_light's
        // `env_i`), as the paint's diffuse takes it.
        let w = lk_phys();
        env_i = mix(env_i, 1.0, w);
        ground = ground + lk_sh(-up) * w;
    }
    let sky = look.hemi_sky.xyz + look.ambient.xyz + lk_sh(up) * env_i;
    let share = clamp((1.0 - n.y) * 0.5, 0.0, 1.0);
    return (LM_SEA_ALBEDO * (key + sky) - ground) * share;
}

// A point belongs to the Golden Gate (not the west span): the nearer of the two frames' origins.
fn lm_is_gg(p: vec3<f32>) -> bool {
    return distance(p.xz, LM_GG_ORIGIN) < distance(p.xz, LM_BAY_ORIGIN);
}

// A Golden Gate point over the strait (between the shore-end pylon and the
// Marin anchorage): the sea under it is what its faces turned down see.
fn lm_gg_over_water(p: vec3<f32>) -> bool {
    let c = dot(p.xz - LM_GG_ORIGIN, LM_GG_AXIS);
    return lm_is_gg(p) && c > LM_GG_WATER_C.x && c < LM_GG_WATER_C.y;
}

// The view factor from a point to a rectangle in a plane parallel to the
// point's face, `c` away, with one corner on the face's normal and sides a
// and b (Howell, "A Catalog of Radiation Heat Transfer Configuration
// Factors", B-3: differential planar element to a parallel rectangle).
fn lm_vf_rect(a: f32, b: f32, c: f32) -> f32 {
    let x = max(a, 0.0) / max(c, 1e-3);
    let y = max(b, 0.0) / max(c, 1e-3);
    let sx = sqrt(1.0 + x * x);
    let sy = sqrt(1.0 + y * y);
    return (x / sx * atan(y / sx) + y / sy * atan(x / sy)) / (2.0 * 3.14159265);
}

// The share of the sky a Golden Gate face still sees (`shade2`): what the
// screen-space occlusion misses at the bridge's distances (it is near 1 past
// a few hundred metres), from the structure's own two large occluders, taken
// off the indirect light only (the albedo bake it replaces dimmed the sun on
// the face as well, and a face turned down twice: its environment irradiance
// holds no sky).
//  * The deck plate over a member under it (the truss, the floor beams, the
//    legs under the roadway): an infinitely long strip of the deck's half
//    width at the height over the point (Howell B-14 and B-15 for a
//    differential strip under, beside and facing a parallel strip); a face
//    turned up loses 0.5 (sin a + sin b) of its sky (a and b the edges'
//    angles off the vertical), a face turned across 0.5 (1 - sin e) (e the
//    near edge's elevation), a face turned along the span half the up face's.
//  * The opposite leg over a leg's inner face between the legs: a rectangle
//    the opposite leg's width along the span and the tower's height, facing
//    the face across the gap (lm_vf_rect over its four quadrants).
// The occluders' own reflected light (paint of a 0.14 albedo) is left out.
fn lm_bridge_sky_vis(p: vec3<f32>, n: vec3<f32>) -> f32 {
    if (!LM_SHADE2_ON || !lm_is_gg(p)) {
        return 1.0;
    }
    let ax = LM_GG_AXIS;
    let across = vec2<f32>(-ax.y, ax.x);
    let rel = p.xz - LM_GG_ORIGIN;
    let c = dot(rel, ax);
    let u = dot(rel, across);
    let na = dot(n.xz, across);
    let nc = dot(n.xz, ax);
    let horiz = max(abs(na) + abs(nc), 1e-4);
    var f = 0.0;
    // the deck.
    let hw = LM_GG_DECK[7];
    let dh = lm_gg_deck_top(c) - 0.3 - p.y;
    if (c > LM_GG_DECK[0] && c < LM_GG_DECK[5] && dh > 0.05 && abs(u) < hw + 40.0) {
        let ea = hw - u;
        let eb = hw + u;
        let f_up = 0.5 * (ea / sqrt(ea * ea + dh * dh) + eb / sqrt(eb * eb + dh * dh));
        let e = max(select(eb, ea, na > 0.0), 0.0);
        let f_side = 0.5 * (1.0 - dh / sqrt(dh * dh + e * e));
        let f_vert = (abs(na) * f_side + abs(nc) * 0.5 * f_up) / horiz;
        f = select((1.0 + n.y) * f_vert, n.y * f_up + (1.0 - n.y) * f_vert, n.y > 0.0);
    }
    // the opposite leg, over a leg's inner face.
    let ct = select(0.0, LM_GG_SPAN, c > LM_GG_SPAN * 0.5);
    let inward = -sign(u) * na;
    if (abs(c - ct) < LM_GG_HALF_ALONG + 1.0 && abs(u) < LM_GG_HP && inward > 0.0) {
        var ha = LM_GG_STAGES[0].w;
        var hc = LM_GG_STAGES[0].z;
        for (var i = 0u; i < LM_GG_STAGE_N; i = i + 1u) {
            if (p.y >= LM_GG_STAGES[i].x) {
                ha = LM_GG_STAGES[i].w;
                hc = LM_GG_STAGES[i].z;
            }
        }
        let gap = abs(u) + LM_GG_HP - hc;
        let up_b = LM_GG_STAGES[LM_GG_STAGE_N - 1u].y - p.y;
        let down_b = p.y - LM_GG_STAGES[0].x;
        let a0 = ha + (c - ct);
        let a1 = ha - (c - ct);
        let leg = lm_vf_rect(a0, up_b, gap) + lm_vf_rect(a1, up_b, gap) + lm_vf_rect(a0, down_b, gap) + lm_vf_rect(a1, down_b, gap);
        f = max(f, leg * inward);
    }
    return clamp(1.0 - f, 0.0, 1.0);
}

// The sea's light as outgoing Lambert radiance (added on the emission slot,
// outside look_light's occlusion multiply): what it takes off is weighted by
// the occlusion the rig's ground term was (`ao`: the bake's and the contact
// band's; the screen-space term is near 1 at a bridge's distances).
fn lm_bridge_bounce(world: vec3<f32>, n: vec3<f32>, albedo: vec3<f32>, ao: f32) -> vec3<f32> {
    let e = lm_bridge_bounce_e(world, n);
    return albedo / 3.14159265 * (max(e, vec3<f32>(0.0)) + min(e, vec3<f32>(0.0)) * ao * 0.95);
}

// the intertidal band on the bridges' concrete (`finish`): the bay's tide
// (mean diurnal range 1.78 m at the Golden Gate, NOAA station 9414290) and
// its splash leave a ragged dark, brown-green band from the still sea to
// about 2 m over it, glossy in the splash (lm_seawall's band, taller).
fn lm_bridge_tide(s: ptr<function, Surface>, world: vec3<f32>, faceness: f32) {
    let sea = LM_SEA_LEVEL_Y;
    let ragged = lm_mx_noise(vec3<f32>(world.x * 0.35, 0.3, world.z * 0.35)) * 0.35 + lm_mx_noise(vec3<f32>(world.x * 2.1, 1.7, world.z * 2.1)) * 0.1;
    let tide = lm_ss(sea + 2.1 + ragged, sea + 1.7 + ragged, world.y) * faceness;
    let splash = lm_ss(sea + 0.9, sea + 0.1, world.y) * faceness;
    (*s).albedo = mix((*s).albedo, (*s).albedo * vec3<f32>(0.46, 0.47, 0.40), tide);
    (*s).albedo = mix((*s).albedo, (*s).albedo * vec3<f32>(0.55, 0.6, 0.5), splash * 0.7);
    (*s).rough = mix((*s).rough, 0.3, splash);
}

// A segment from `o` along unit `d` for `tmax` metres crosses the box
// [bmin, bmax] (slab test; the start outside it).
fn lm_ray_box(o: vec3<f32>, d: vec3<f32>, tmax: f32, bmin: vec3<f32>, bmax: vec3<f32>) -> bool {
    let dd = select(d, vec3<f32>(1e-6), abs(d) < vec3<f32>(1e-6));
    let inv = 1.0 / dd;
    let t0 = (bmin - o) * inv;
    let t1 = (bmax - o) * inv;
    let tn = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let tf = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    return tf > max(tn, 0.0) && tn < tmax;
}

// the Golden Gate towers' floods as placed sources (`BridgeFeatures::floods`,
// eo-landmarks lights.rs `gg_tower_floods`): the nearer tower's 12 floods on
// the sidewalk round its legs, each lighting what faces it inside its beam
// (`LM_FLOOD_COS_*`) and what the leg it stands at and the portal struts
// above the deck leave in its view; the face illuminance laid by the
// floods' combined aim (`LM_FLOOD_PROFILE`, about half at the top), times
// the share of the face's two floods that reach it. So the faces the
// sidewalk passes are lit, the inner faces (toward the roadway) and the
// setbacks' tops stay dark, and the struts throw their shadows up the legs.
// Under the deck the 12 lights below the roadway (the list's last 12, on the
// pier) light the legs the same way, up to the deck's underside (the deck
// stands between the two sets: each lights only its own side of it).
fn lm_floodlit2(p: vec3<f32>, n: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    let lamps = skyu.night.x;
    let above = p.y > LM_GG_DECK_Y - 1.0;
    let below = p.y < LM_GG_UNDER_Y + 0.5 && p.y > LM_GG_BASE_Y - 1.0;
    if (!LM_FLOOD_ON || lamps <= 0.0 || !(above || below) || p.y > LM_GG_TOP_Y + 1.0) {
        return vec3<f32>(0.0);
    }
    let across = vec2<f32>(-LM_GG_AXIS.y, LM_GG_AXIS.x);
    let rel = p.xz - LM_GG_ORIGIN;
    let c0 = dot(rel, LM_GG_AXIS);
    let sl = dot(rel, across);
    let ct = select(0.0, LM_GG_SPAN, c0 > LM_GG_SPAN * 0.5);
    let c = c0 - ct;
    if (abs(c) > LM_GG_HALF_ALONG + 8.0 || abs(sl) > LM_GG_HP + LM_GG_HALF_ACROSS + 8.0) {
        return vec3<f32>(0.0);
    }
    let nl = vec3<f32>(dot(n.xz, LM_GG_AXIS), dot(n.xz, across), n.y);
    let q = vec3<f32>(c, sl, p.y) + nl * 0.05;
    var fpos = LM_GG_FLOOD_POS;
    var faim = LM_GG_FLOOD_AIM;
    var struts = LM_GG_STRUTS;
    var e = 0.0;
    let i0 = select(LM_GG_FLOOD_NA, 0u, above);
    let i1 = select(LM_GG_FLOOD_N, LM_GG_FLOOD_NA, above);
    for (var i = i0; i < i1; i = i + 1u) {
        let fp = fpos[i];
        let to = fp - q;
        let d = length(to);
        let l = to / max(d, 1e-3);
        let facing = dot(nl, l);
        let beam = lm_ss(LM_FLOOD_COS_OUT, LM_FLOOD_COS_IN, dot(-l, faim[i]));
        if (facing <= 0.0 || beam <= 0.0) {
            continue;
        }
        // the leg the flood stands at (unless the point is on it: its own faces turn from what they cannot see).
        let leg_s = select(-LM_GG_HP, LM_GG_HP, fp.y > 0.0);
        var vis = 1.0;
        let on_leg = abs(sl - leg_s) < LM_GG_HALF_ACROSS + 0.2 && abs(c) < LM_GG_HALF_ALONG + 0.2;
        if (!on_leg && lm_ray_box(q, l, d, vec3<f32>(-LM_GG_HALF_ALONG, leg_s - LM_GG_HALF_ACROSS, LM_GG_BASE_Y), vec3<f32>(LM_GG_HALF_ALONG, leg_s + LM_GG_HALF_ACROSS, LM_GG_TOP_Y))) {
            vis = 0.0;
        }
        for (var k = 0u; k < LM_GG_STRUT_N && vis > 0.0; k = k + 1u) {
            let st = struts[k];
            if (lm_ray_box(q, l, d, vec3<f32>(-st.z, -st.w, st.x), vec3<f32>(st.z, st.w, st.y))) {
                vis = 0.0;
            }
        }
        e = e + beam * lm_ss(0.0, 0.3, facing) * vis;
    }
    var h = clamp((p.y - LM_GG_DECK_Y) / max(LM_GG_TOP_Y - LM_GG_DECK_Y, 1.0), 0.0, 1.0);
    var lux0 = LM_FLOOD_LUX;
    if (!above) {
        h = clamp((p.y - LM_GG_BASE_Y) / max(LM_GG_UNDER_Y - LM_GG_BASE_Y, 1.0), 0.0, 1.0);
        lux0 = LM_FLOOD_LUX_B;
    }
    let lux = lux0 * (LM_FLOOD_PROFILE.x + LM_FLOOD_PROFILE.y * h) * 0.5 * e;
    return albedo * (lux / 3.14159265) * skyu.bloom_w2.z * lamps * LM_FLOOD_RGB;
}

// the landmark buildings' surfaces (landmark-surface.js):
// 4 `precast` (panel joints at the bay and storey pitch, a per-panel tone,
// drip staining under each panel's top edge; the bay and storey ride in the
// material's `rough_flatten` and `thin_absorption`), 5 `finCurtain` (pale
// vertical fins over glass, box-filtered by the pixel footprint), 6
// `gildedCladding` (the gilt told from the cladding by the vertex colour's warmth).
fn lm_fin_integral(t: f32) -> f32 {
    return floor(t) * LM_FC_FIN_SHARE + max(fract(t) - (1.0 - LM_FC_FIN_SHARE), 0.0);
}

fn lm_building_surface(s: ptr<function, Surface>, m: Material, kind: u32, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let local = world - vec3<f32>(m.params.z, 0.0, m.params.w);
    let fr = fa_frame(n);
    let uv = vec2<f32>(dot(world, fr.t), dot(world, fr.b));
    let du = vec2<f32>(dot(dpx, fr.t), dot(dpx, fr.b));
    let dv = vec2<f32>(dot(dpy, fr.t), dot(dpy, fr.b));
    let vertical = clamp(1.0 - abs(n.y), 0.0, 1.0);
    if (kind == 4u) {
        let cellm = vec2<f32>(max(m.rough_flatten, 0.1), max(m.thin_absorption, 0.1));
        let cell = floor(uv / cellm);
        let panel_tone = lm_noise(vec3<f32>(cell.x * 0.71, cell.y * 0.53, 0.3)) * LM_PC_PATCH_TONE;
        let drip = pow(1.0 - fract(uv.y / cellm.y), 6.0) * LM_PC_STREAK_TONE * vertical;
        let tone = lm_noise(local / LM_PC_PATCH_M) * LM_PC_PATCH_TONE;
        (*s).albedo = (*s).albedo * (1.0 + panel_tone + tone - drip);
        (*s).rough = clamp((*s).rough + tone * (LM_PC_ROUGH_SPREAD / max(1e-3, LM_PC_PATCH_TONE)), 0.4, 0.95);
        let footprint = ((abs(du.x) + abs(dv.x)) + (abs(du.y) + abs(dv.y))) * 0.5;
        let fj = lm_fade(footprint, cellm.x);
        if (fj > 0.0) {
            let h0 = lm_precast_height(uv, cellm);
            let hx = lm_precast_height(uv + du, cellm);
            let hy = lm_precast_height(uv + dv, cellm);
            (*s).n = lm_bump(n, dpx, dpy, h0 * fj, hx * fj, hy * fj);
        }
    } else if (kind == 5u) {
        let x = uv.x / LM_FC_MODULE_M;
        let fw = max(abs(du.x) + abs(dv.x), 1e-4 * LM_FC_MODULE_M) / LM_FC_MODULE_M;
        let filtered = (lm_fin_integral(x + fw * 0.5) - lm_fin_integral(x - fw * 0.5)) / fw;
        let fin = select(0.0, clamp(filtered, 0.0, 1.0), abs(n.y) < 0.5);
        (*s).albedo = mix((*s).albedo, vec3<f32>(LM_FC_FIN_R, LM_FC_FIN_G, LM_FC_FIN_B), fin);
        (*s).rough = mix(LM_FC_GLASS_ROUGH, LM_FC_FIN_ROUGH, fin);
        (*s).metal = mix(LM_FC_GLASS_METAL, 0.0, fin);
    } else if (kind == 6u) {
        let c = (*s).albedo;
        let warmth = (c.x - c.z) / max(c.x, 1e-3);
        let gilt = lm_ss(LM_GC_WARM_FROM, LM_GC_WARM_TO, warmth);
        let tone = lm_noise(local / LM_GC_PATCH_M) * LM_GC_PATCH_TONE * (1.0 - gilt);
        (*s).albedo = (*s).albedo * (1.0 + tone);
        let gilt_scale = LM_GC_GILT_ROUGH / max(1e-3, m.params.x);
        (*s).rough = clamp((*s).rough * mix(1.0, gilt_scale, gilt), 0.05, 1.0);
        (*s).metal = mix((*s).metal, LM_GC_GILT_METAL, gilt);
    }
}

fn lm_precast_height(uv: vec2<f32>, cellm: vec2<f32>) -> f32 {
    let joint = min(lm_line(uv.x, cellm.x), lm_line(uv.y, cellm.y));
    return (1.0 - lm_ss(0.0, 0.02, joint)) * -LM_PC_JOINT_DEPTH;
}

// ---- three's MaterialX Perlin noise (mx_noise_float, the TSL `noise` the
// seawall's colour node samples), as water.wgsl ports it ----
fn lm_mx_rotl(x: u32, k: u32) -> u32 {
    return (x << k) | (x >> (32u - k));
}

fn lm_mx_hash3(x: i32, y: i32, z: i32) -> u32 {
    let seed = 0xdeadbeefu + (3u << 2u) + 13u;
    var a = seed + bitcast<u32>(x);
    var b = seed + bitcast<u32>(y);
    var c = seed + bitcast<u32>(z);
    c = c ^ b;
    c = c - lm_mx_rotl(b, 14u);
    a = a ^ c;
    a = a - lm_mx_rotl(c, 11u);
    b = b ^ a;
    b = b - lm_mx_rotl(a, 25u);
    c = c ^ b;
    c = c - lm_mx_rotl(b, 16u);
    a = a ^ c;
    a = a - lm_mx_rotl(c, 4u);
    b = b ^ a;
    b = b - lm_mx_rotl(a, 14u);
    c = c ^ b;
    c = c - lm_mx_rotl(b, 24u);
    return c;
}

fn lm_mx_grad(hash: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = hash & 15u;
    let u = select(y, x, h < 8u);
    let v = select(select(z, x, h == 12u || h == 14u), y, h < 4u);
    return select(u, -u, (h & 1u) != 0u) + select(v, -v, (h & 2u) != 0u);
}

fn lm_mx_fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn lm_mx_noise(p: vec3<f32>) -> f32 {
    let fl = floor(p);
    let i = vec3<i32>(fl);
    let f = p - fl;
    let u = lm_mx_fade(f.x);
    let v = lm_mx_fade(f.y);
    let w = lm_mx_fade(f.z);
    let g000 = lm_mx_grad(lm_mx_hash3(i.x, i.y, i.z), f.x, f.y, f.z);
    let g100 = lm_mx_grad(lm_mx_hash3(i.x + 1, i.y, i.z), f.x - 1.0, f.y, f.z);
    let g010 = lm_mx_grad(lm_mx_hash3(i.x, i.y + 1, i.z), f.x, f.y - 1.0, f.z);
    let g110 = lm_mx_grad(lm_mx_hash3(i.x + 1, i.y + 1, i.z), f.x - 1.0, f.y - 1.0, f.z);
    let g001 = lm_mx_grad(lm_mx_hash3(i.x, i.y, i.z + 1), f.x, f.y, f.z - 1.0);
    let g101 = lm_mx_grad(lm_mx_hash3(i.x + 1, i.y, i.z + 1), f.x - 1.0, f.y, f.z - 1.0);
    let g011 = lm_mx_grad(lm_mx_hash3(i.x, i.y + 1, i.z + 1), f.x, f.y - 1.0, f.z - 1.0);
    let g111 = lm_mx_grad(lm_mx_hash3(i.x + 1, i.y + 1, i.z + 1), f.x - 1.0, f.y - 1.0, f.z - 1.0);
    let s1 = 1.0 - u;
    let t1 = 1.0 - v;
    let r1 = 1.0 - w;
    let res = r1 * (t1 * (g000 * s1 + g100 * u) + v * (g010 * s1 + g110 * u)) + w * (t1 * (g001 * s1 + g101 * u) + v * (g011 * s1 + g111 * u));
    return 0.982 * res;
}

// landmark-set.js `landmarkFog`: a landmark takes the scene's range fog with
// its sight distance capped at fogCapM, so it never clears with distance and
// stands on the skyline past the fog's end.
fn lm_fog_depth(m: Material, vd: f32) -> f32 {
    if ((m.flags & LM_F_LANDMARK) != 0u) {
        return min(vd, LM_FOG_CAP_M);
    }
    return vd;
}

// ---- the fifth pass (round 3's bridge finish) ----

// The tower frame at a Golden Gate point: (chainage from the nearer tower's
// centre line, lateral offset, height).
fn lm_gg_tower_local(p: vec3<f32>) -> vec3<f32> {
    let rel = p.xz - LM_GG_ORIGIN;
    let c0 = dot(rel, LM_GG_AXIS);
    let ct = select(0.0, LM_GG_SPAN, c0 > LM_GG_SPAN * 0.5);
    return vec3<f32>(c0 - ct, dot(rel, vec2<f32>(-LM_GG_AXIS.y, LM_GG_AXIS.x)), p.y);
}

// The paint's weathering (`BridgeFeatures::weathering`). Facts (goldengate.org,
// "Painting the Bridge"): the coat is renewed continuously, "routine touch up
// painting is_on an is_on-going basis" where inspections find corrosion; salt-laden
// fog attacks the film; rust "attacked the rivet heads and surfaces" from the
// bases of the towers; failures start under loosened mill scale. Acrylic
// topcoats chalk under sunlight (the degree rated by ASTM D4214). INFERRED,
// every form and amount: touch-up patches over a few plates in fresher,
// glossier, more saturated paint (`LM_WX_TOUCH` of the plate groups); the rest
// chalked by its own age, more is_on faces turned up; rust bloom along the rivet
// rows beside the plate seams, densest at the towers' feet and in the splash
// zone; dirt is_on the ledges (faces turned up); run-off streaks under the legs'
// setb-backs and under the deck's edge. Each is filtered by the pixel's
// footprint to its mean, so a far member takes the mean, not noise.
const LM_WX_TOUCH: f32 = 0.12;
const LM_WX_CHALK: f32 = 0.22;
const LM_WX_RUST_P: vec2<f32> = vec2<f32>(0.04, 0.35); // rivets blooming: mean, and at the towers' feet
const LM_WX_RUST_RGB: vec3<f32> = vec3<f32>(0.115, 0.048, 0.022); // iron oxide brown (linear)
const LM_WX_LEDGE: f32 = 0.45;
const LM_WX_RUN_M: f32 = 7.0;
const LM_WX_RUN_TONE: f32 = 0.22;

fn lm_weather(s: ptr<function, Surface>, world: vec3<f32>, local: vec3<f32>, n: vec3<f32>, uv: vec2<f32>, du: vec2<f32>, dv: vec2<f32>, footprint: f32, vertical: f32) {
    let c0 = (*s).albedo;
    let up = n.y;
    let plate = vec2<f32>(LM_PS_PLATE_U, LM_PS_PLATE_V);
    let fp = lm_fade(footprint, LM_PS_PLATE_U * 3.0);
    let fid = floor(dot(n, vec3<f32>(1.0, 3.0, 7.0)) * 2.0 + 0.5);
    let cell = floor(uv / plate);
    let age = mix(0.5, lm_cell_hash(vec3<f32>(cell.x, cell.y, fid + 17.0)), fp);
    let grp = floor(vec2<f32>(cell.x / 3.0, cell.y / 2.0));
    let touched = select(0.0, 1.0, lm_cell_hash(vec3<f32>(grp.x, grp.y, fid + 41.0)) < LM_WX_TOUCH);
    let touch = mix(LM_WX_TOUCH, touched, fp);
    // chalk: a lighter tint of the paint's own colour and a little white (the binder gone, the pigment and extender loose).
    let chalk = LM_WX_CHALK * (0.35 + 0.65 * age) * (0.65 + 0.35 * max(up, 0.0)) * (1.0 - touch);
    var c = mix(c0, c0 * 1.3 + vec3<f32>(0.010, 0.008, 0.007), chalk);
    c = mix(c, c0 * vec3<f32>(0.95, 0.86, 0.82), touch);
    // (a fresher patch keeps the coat's roughness: a glossier one reflected the bright sky at grazing
    // views as pale blotches, and no gloss value for the touch-up coat was found)
    (*s).rough = clamp((*s).rough + 0.15 * chalk, 0.3, 0.95);
    // rust bloom is_on the rivet rows (6 cm either side of the horizontal seams), as stain bands 5 cm high.
    let fy = abs(du.y) + abs(dv.y);
    let fx = abs(du.x) + abs(dv.x);
    let rows = lm_lines(uv.y - 0.06, fy, LM_PS_PLATE_V, 0.05) + lm_lines(uv.y + 0.06, fy, LM_PS_PLATE_V, 0.05);
    let foot = 1.0 - lm_ss(LM_SEA_LEVEL_Y + 6.0, LM_SEA_LEVEL_Y + 40.0, world.y);
    let p_rust = mix(LM_WX_RUST_P.x, LM_WX_RUST_P.y, foot) * (1.0 - touch);
    let rivet = floor(uv.x / LM_PS_RIVET_PITCH);
    let is_on = select(0.0, 1.0, lm_cell_hash(vec3<f32>(rivet, floor(uv.y / LM_PS_PLATE_V * 2.0 + 0.5), fid + 5.0)) < p_rust);
    let spot = mix(p_rust, is_on, lm_fade(fx, LM_PS_RIVET_PITCH));
    c = mix(c, LM_WX_RUST_RGB, clamp(rows * spot * 0.85 * vertical, 0.0, 1.0));
    // dirt is_on the ledges.
    let ledge = LM_WX_LEDGE * lm_ss(0.55, 0.95, up) * (0.65 + 0.35 * lm_noise(local / 1.7 + vec3<f32>(2.2, 0.0, 9.1)));
    c = mix(c, c * 0.5 + vec3<f32>(0.012, 0.010, 0.008), ledge);
    // run-off under the ledge above: a leg's setb-back top (in a tower's plan box) or the deck's edge.
    let t = lm_gg_tower_local(world);
    var drop = 1e4;
    if (abs(t.x) < LM_GG_HALF_ALONG + 1.0 && abs(abs(t.y) - LM_GG_HP) < LM_GG_HALF_ACROSS + 1.0) {
        for (var i = 0u; i + 1u < LM_GG_STAGE_N; i = i + 1u) {
            let top = LM_GG_STAGES[i].y;
            if (top > world.y) {
                drop = min(drop, top - world.y);
            }
        }
    } else {
        let deck = lm_gg_deck_top(dot(world.xz - LM_GG_ORIGIN, LM_GG_AXIS));
        if (deck > world.y) {
            drop = deck - world.y;
        }
    }
    let fr = lm_fade(footprint, 1.2);
    let stripes = mix(0.5, lm_ss(-0.1, 0.6, lm_noise(vec3<f32>(uv.x * 0.9, world.y * 0.025, 6.3))), fr);
    let run = LM_WX_RUN_TONE * exp(-drop / LM_WX_RUN_M) * stripes * vertical;
    c = c * (1.0 - run);
    (*s).albedo = c;
}

// The concrete's age (`BridgeFeatures::concrete2`): rough-board-formed
// (Strauss 1937) and cast in lifts, each lift's own tone (each pour's mix and
// its curing), lime leached at the lift joints and run down the face as pale
// streaks (efflorescence), and dark weathering streaks down the faces where
// the run-off from the tops carries dirt and the salt fog's growth.
// INFERRED: the lift height (`LM_CO_LIFT_M`, the export's), the tones and lengths.
fn lm_concrete_age(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, uv: vec2<f32>, du: vec2<f32>, dv: vec2<f32>, footprint: f32, vertical: f32) {
    let lift = floor(world.y / LM_CO_LIFT_M);
    let fid = floor(dot(n, vec3<f32>(1.0, 3.0, 7.0)) * 2.0 + 0.5);
    let fl = lm_fade(footprint, LM_CO_LIFT_M);
    let lift_tone = mix(0.0, (lm_cell_hash(vec3<f32>(lift, fid, 23.0)) - 0.5) * 0.10, fl) * vertical;
    let below = fract(world.y / LM_CO_LIFT_M); // 1 just under a joint, 0 at the next
    let fs = lm_fade(footprint, 0.6);
    let leach = mix(0.25, lm_ss(0.35, 0.9, lm_noise(vec3<f32>(uv.x * 2.1, lift * 1.7, 12.5)) * 0.5 + 0.5), fs) * pow(below, 3.0) * 0.10 * vertical;
    let dirt = mix(0.4, lm_ss(-0.2, 0.7, lm_noise(vec3<f32>(uv.x * 0.7, world.y * 0.018, 31.0))), lm_fade(footprint, 1.5)) * 0.22 * vertical;
    // the same weathering at the wall's scale (bands some 5-10 m across, faded with height over 40 m), so a
    // housing's wall seen from hundreds of metres keeps its staining where the fine streaks have gone to their mean.
    let broad = lm_ss(-0.3, 0.8, lm_noise(vec3<f32>(uv.x * 0.12, world.y * 0.012, 47.0))) * 0.16 * vertical;
    let c = (*s).albedo;
    (*s).albedo = c * (1.0 + lift_tone) * (1.0 - dirt) * (1.0 - broad) + vec3<f32>(0.05, 0.05, 0.048) * leach;
}

// The share of a face's sky that two parallel fins `d` deep, standing `x0`
// and `x1` to either side of the point and as tall as the face, take
// (Howell's differential element to a perpendicular semi-infinite plate,
// each 0.5 (1 - x / sqrt(x^2 + d^2))).
fn lm_fin_occ(x: f32, d: f32) -> f32 {
    let xx = max(x, 0.0);
    return 0.5 * (1.0 - xx / sqrt(xx * xx + d * d));
}

// The light through a recess: a point `t` across a channel [a, b] is_on a face
// `depth` deep (the channel's walls standing out of the face), the key
// direction's components (along the face's normal `ln`, across the channel
// `lt`); a soft edge `ws` wide. 1 where the ray to the sun clears the walls.
fn lm_channel_key(t: f32, a: f32, b: f32, depth: f32, ln: f32, lt: f32, ws: f32) -> f32 {
    if (ln <= 1e-3) {
        return 1.0;
    }
    let te = t + lt / ln * depth;
    return lm_ss(a - ws, a + ws, te) * (1.0 - lm_ss(b - ws, b + ws, te));
}

// The legs' recessed panels (golden_gate/legs.rs `face_runs`, mirrored): the
// run of a face `hw` either side that holds `t` as (from, to, recessed).
fn lm_leg_run(t: f32, hw: f32) -> vec3<f32> {
    let w = 2.0 * hw;
    let n = max(round(w / LM_GG_CELL_M), 3.0);
    let m = w / n;
    let panels = max(ceil((n - 1.0) / (LM_GG_PANEL_MAX + 1.0)), 1.0);
    let wp = (n - (panels + 1.0)) / panels;
    let x = t + hw; // from the face's start
    let unitw = m + wp * m;
    let k = floor(x / unitw);
    let r = x - k * unitw;
    if (k >= panels || r < m) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    let a = -hw + k * unitw + m;
    return vec3<f32>(a, a + wp * m, 1.0);
}

// A Golden Gate tower face's own sun shadow and sky occlusion past the
// shadow maps (`BridgeFeatures::leg_cells`): is_on a leg's recessed panel the
// panel's two pilasters; is_on a portal casing's face the legs standing proud of
// it, the lip over it and its ribs. (key visibility, sky visibility).
fn lm_gg_self_shade(p: vec3<f32>, n: vec3<f32>, footprint: f32) -> vec2<f32> {
    let t = lm_gg_tower_local(p);
    let ax = vec3<f32>(LM_GG_AXIS.x, 0.0, LM_GG_AXIS.y);
    let ac = vec3<f32>(-LM_GG_AXIS.y, 0.0, LM_GG_AXIS.x);
    let nc = dot(n, ax);
    let ns = dot(n, ac);
    let l = look.key_dir.xyz;
    let ws = max(footprint, 0.03);
    if (abs(t.x) > LM_GG_HALF_ALONG + 1.0 || abs(t.y) > LM_GG_HP + LM_GG_HALF_ACROSS + 1.0 || t.z < LM_GG_STAGES[0].x || t.z > LM_GG_STAGES[LM_GG_STAGE_N - 1u].y + 0.5) {
        return vec2<f32>(1.0);
    }
    // the portal casings (between the legs' inner faces, a face turned along the span).
    if (abs(nc) > 0.9) {
        for (var i = 0u; i < LM_GG_CASING_N; i = i + 1u) {
            let cs = LM_GG_CASINGS[i];
            if (t.z < cs.x || t.z > cs.y || abs(t.y) > cs.w || abs(t.x) > cs.z + LM_GG_RIB.x + 0.05) {
                continue;
            }
            let on_rib = abs(t.x) > cs.z + 0.5 * LM_GG_RIB.x;
            let ln = dot(l, ax) * sign(nc);
            let lt = dot(l, ac);
            let setb = select(LM_GG_CASE_SET, LM_GG_CASE_SET - LM_GG_RIB.x, on_rib);
            // the legs, either side.
            var key = lm_channel_key(t.y, -cs.w, cs.w, setb, ln, lt, ws);
            var occ = lm_fin_occ(cs.w - t.y, setb) + lm_fin_occ(cs.w + t.y, setb);
            // the lip over the face (its underside at cs.y - lip height, standing out lip proud).
            let under = cs.y - LM_GG_LIP.x;
            let lo = select(LM_GG_LIP.y + LM_GG_RIB.x, LM_GG_LIP.y, on_rib);
            if (t.z < under) {
                if (ln > 1e-3) {
                    key = key * (1.0 - lm_ss(under - ws, under + ws, t.z + l.y / ln * lo));
                }
                occ = occ + lm_fin_occ(under - t.z, lo);
            }
            if (!on_rib) {
                // the ribs (centred is_on the casing; laid as golden_gate.rs `portal_ribs`).
                let nr = max(floor((2.0 * cs.w - LM_GG_RIB.y) / LM_GG_RIB.z), 1.0);
                let first = -nr * LM_GG_RIB.z * 0.5;
                let k = clamp(floor((t.y - first) / LM_GG_RIB.z), 0.0, nr - 1.0);
                let a = first + k * LM_GG_RIB.z + 0.5 * LM_GG_RIB.y;
                let b = first + (k + 1.0) * LM_GG_RIB.z - 0.5 * LM_GG_RIB.y;
                if (t.y > a && t.y < b) {
                    key = key * lm_channel_key(t.y, a, b, LM_GG_RIB.x, ln, lt, ws);
                    occ = occ + lm_fin_occ(t.y - a, LM_GG_RIB.x) + lm_fin_occ(b - t.y, LM_GG_RIB.x);
                }
            }
            return vec2<f32>(key, clamp(1.0 - occ, 0.0, 1.0));
        }
    }
    // a leg's face: the stage the point is in.
    var st = LM_GG_STAGES[0];
    for (var i = 0u; i < LM_GG_STAGE_N; i = i + 1u) {
        if (t.z >= LM_GG_STAGES[i].x) {
            st = LM_GG_STAGES[i];
        }
    }
    let leg_s = select(-LM_GG_HP, LM_GG_HP, t.y > 0.0);
    let ls = t.y - leg_s;
    var tf = 0.0;
    var hw = 0.0;
    var depth = 0.0;
    var ln = 0.0;
    var lt = 0.0;
    if (abs(nc) > 0.9) {
        tf = ls;
        hw = st.z;
        depth = st.w - abs(t.x);
        ln = dot(l, ax) * sign(nc);
        lt = dot(l, ac);
    } else if (abs(ns) > 0.9) {
        tf = t.x;
        hw = st.w;
        depth = st.z - abs(ls);
        ln = dot(l, ac) * sign(ns);
        lt = dot(l, ax);
    } else {
        return vec2<f32>(1.0);
    }
    if (abs(tf) > hw + 0.05 || depth < 0.5 * LM_GG_RECESS_M || depth > LM_GG_RECESS_M + 0.05) {
        return vec2<f32>(1.0); // a pilaster's face, a wall of a recess, or not is_on the leg
    }
    let run = lm_leg_run(tf, hw);
    if (run.z < 0.5) {
        return vec2<f32>(1.0);
    }
    let key = lm_channel_key(tf, run.x, run.y, LM_GG_RECESS_M, ln, lt, ws);
    let occ = lm_fin_occ(tf - run.x, LM_GG_RECESS_M) + lm_fin_occ(run.y - tf, LM_GG_RECESS_M);
    return vec2<f32>(key, clamp(1.0 - occ, 0.0, 1.0));
}

// The radiance probe (EO_BRIDGE_PROBE = a mask; a debugging view, never is_on
// by default): a Golden Gate painted surface keeps only the terms its mask
// names, recomputed as look_light computes them (1 the sun's diffuse and
// rim, 2 the sun's specular, 4 the sky's diffuse (hemisphere, ambient,
// environment), 8 the sky's specular, 16 the sea's bounce, 32 the scene's
// bounce), the rest of the frame unchanged (the atmosphere still laid over
// it). Mask 63 redraws the surface as it is. look_light's own terms are
// zeroed (no albedo, no occlusion, no key or rim) and the kept ones ride is_on
// the emission slot.
fn lm_probe(s: ptr<function, Surface>, world: vec3<f32>, ng: vec3<f32>) {
    let mask = LM_PROBE;
    let sf = *s;
    let n = sf.n;
    let v = normalize(look.cam_pos.xyz - world);
    let diffuse = sf.albedo;
    let f0 = vec3<f32>(0.04);
    let rough = sf.rough;
    let env_i = mix(look.ambient.w * ST_ENV_PAINTED_METAL, 1.0, lk_phys());
    let l = look.key_dir.xyz;
    let nl = max(dot(n, l), 0.0);
    var key = vec3<f32>(0.0);
    if (nl > 0.0 && dot(ng, l) > -0.05) {
        key = look.key.xyz * nl * lk_shadow(world, ng) * sky_cloud_transmittance(world) * sky_contact_shadow(world, ng) * lm_key_vis;
    }
    let rim = look.rim.xyz * max(dot(n, look.rim_dir.xyz), 0.0) * lm_rim_vis;
    var out = vec3<f32>(0.0);
    if ((mask & 1u) != 0u) {
        out = out + (key + rim) * diffuse / LK_PI;
    }
    if ((mask & 2u) != 0u) {
        out = out + key * lk_ggx(n, v, l, f0, rough) + rim * lk_ggx(n, v, look.rim_dir.xyz, f0, rough);
    }
    let nv = max(dot(n, v), 1e-4);
    let dfg = lk_dfg(nv, rough);
    let fss = f0 * dfg.x + vec3<f32>(dfg.y);
    var ao4 = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (frame.copy_step.z < 0.5) {
        ao4 = ao_fetch(world);
    }
    let ao = sf.ao * ao_multi_bounce(ao4.a);
    if ((mask & 4u) != 0u) {
        let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5);
        var env_e = lk_sh(n);
        if (fa_probe_on() && (fa_run() & 16u) != 0u) {
            env_e = fa_probe_irradiance(n);
        }
        out = out + (hemi + look.ambient.xyz + env_e * env_i) * diffuse / LK_PI * (vec3<f32>(1.0) - fss) * ao;
    }
    if ((mask & 8u) != 0u) {
        let r = reflect(-v, n);
        var rad = lk_sh(r) / LK_PI;
        let sharp = 1.0 - lk_smooth(0.15, 0.5, rough);
        if (fa_probe_on()) {
            rad = fa_probe_sample(r, fa_probe_lod(rough));
        } else if (sharp > 0.0) {
            rad = mix(rad, lk_sky_spec(r, lk_sky_sample(r)), sharp);
        }
        let spec_occ = clamp(pow(nv + ao, exp2(-16.0 * rough - 1.0)) - 1.0 + ao, 0.0, 1.0);
        let ssr = sky_ssr(world, n, v, rough);
        out = out + mix(rad * env_i, ssr.rgb, ssr.a) * fss * spec_occ;
    }
    if ((mask & 32u) != 0u) {
        out = out + ao4.rgb * diffuse;
    }
    // the sea's bounce already sits is_on the emission slot (lm_surface): take it off unless kept.
    if ((mask & 16u) == 0u) {
        rm_emit = rm_emit - lm_bridge_bounce(world, n, diffuse, sf.ao);
    }
    rm_emit = rm_emit + out;
    (*s).albedo = vec3<f32>(0.0);
    (*s).ao = 0.0;
    lm_key_vis = 0.0;
    lm_rim_vis = 0.0;
}
