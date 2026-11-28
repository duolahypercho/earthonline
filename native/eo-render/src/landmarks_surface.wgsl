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

// The surface of a landmark material (`lm_kind`), then its floodlights at
// night (LM_F_FLOOD).
fn lm_surface(s: ptr<function, Surface>, m: Material, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    lm_surface_kind(s, m, world, n, dpx, dpy);
    if (LM_V3) {
        lm_flood3(s, m, world); // projector rigs (eo-landmarks towers3.rs)
    } else {
        lm_flood(s, m, world, n);
    }
}

// A landmark material's surface kind: flag bits 25-27, and bit 31 its
// fourth (eo-render landmarks.rs `surface_flags`).
fn lm_kind(flags: u32) -> u32 {
    return ((flags >> LM_SURFACE_SHIFT) & 7u) | ((flags >> LM_SURFACE_HIGH_SHIFT) & 8u);
}

// The surface kinds: 1 painted steel, 2 cast concrete, 3 the seawall, 4-6
// the landmark buildings' (below), 7 the authored facade (`lm_authored`), 8
// the Golden Gate's far item. `n` faces the camera; `world` is the fragment.
fn lm_surface_kind(s: ptr<function, Surface>, m: Material, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let kind = lm_kind(m.flags);
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
        if (LM_V4) {
            lm_authored4(s, m, world, n, dpx, dpy); // the fourth pass (eo-app --no-towers4: the third)
        } else if (LM_V3) {
            lm_authored3(s, m, world, n, dpx, dpy); // the third pass (eo-app --no-towers3: the previous)
        } else {
            lm_authored(s, m, world, n, dpx, dpy);
        }
        return;
    }
    if (kind == 8u) {
        // the Golden Gate's far item: only the towers' floodlighting (and, with
        // `finish`, the sea's light on its faces turned down).
        if (LM_AGE_MATTE_ON || LM_AGE_FADE_ON) {
            lm_gg_age_far(s, n);
        }
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
            if ((LM_AGE_FADE_ON || LM_AGE_GRIME_ON || LM_AGE_UNDER_ON) && lm_is_gg(world)) {
                let ud = lm_gg_under(world, n);
                if (LM_AGE_UNDER_ON) {
                    lm_key_vis = lm_key_vis * ud.y;
                    (*s).ao = (*s).ao * ud.z;
                }
                lm_gg_age_paint(s, world, n, uv, footprint, vertical, ud.x);
            }
        }
        // (the bridges' coat a quarter of the landmark steel's roughness spread:
        // one film, renewed as one; its patches under 0.5 took the sky's sharp
        // reflection and drew the clouds on the struts.)
        let spread = LM_PS_ROUGH_SPREAD * select(1.0, 0.25, bridge);
        (*s).rough = clamp((*s).rough + lm_noise(local / (LM_PS_PATCH_M * 0.7) + vec3<f32>(5.1, 2.3, 9.4)) * spread
            + chalk / max(1e-3, LM_PS_CHALK_TONE) * LM_PS_CHALK_ROUGH, 0.2, 0.95);
        if (bridge && LM_AGE_MATTE_ON && lm_is_gg(world)) {
            // (`matte`) the chalked coat's flat finish.
            (*s).rough = max((*s).rough, LM_AGE_ROUGH);
        }
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
            if (LM_AGE_CONCRETE_ON) {
                lm_gg_age_concrete(s, world, n, uv, footprint, vertical);
            }
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
    if (!LM_BRIDGE_SCALE_ON) {
        return (a + b - sqrt((a - b) * (a - b) + k * k)) * 0.5;
    }
    let overlap = max(0.0, k - abs(a - b));
    return min(a, b) - overlap * overlap / (4.0 * k);
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

// sf:AAA_BUILDINGS+
// Procedural PBR weathering for all 131 landmarks (`--aaa-buildings` / `SF_AAA_DETAIL`):
//  * `lm_aaa_roof`: corrugated Spanish/Mediterranean barrel clay-tile courses + kiln variation + bump
//    on warm pitched terracotta roofs; rolled tar/gravel membrane seams, ballast/ponding stains &
//    strict matte roughness guard (`>= 0.80`, metal `>= 0.64`) on flat/cool landmark roofs.
//  * `lm_aaa_masonry`: weathered Colusa sandstone, Beaux-Arts limestone/stucco, granite & brick
//    micro-grain, ashlar/brick course joints, cornice/sill rain-drip streaks, base plinth soot, and
//    distance-filtered `lm_bump` normal micro-relief.
fn lm_aaa_roof(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, seed: vec2<f32>) {
    let c = (*s).albedo;
    let fp = max(length(dpx.xz), length(dpy.xz));
    let is_terracotta = n.y < 0.965 && c.r > c.b * 1.32 && c.r > 0.05;
    if (is_terracotta) {
        // Slope-aligned coordinates: `u_eave` along eaves, `v_slope` down roof pitch.
        let horiz = normalize(vec2<f32>(n.x, n.z) + vec2<f32>(1e-5, 0.0));
        let eave = vec2<f32>(-horiz.y, horiz.x);
        let u_eave = dot(world.xz, eave);
        let slope_len = max(length(n.xz), 0.12);
        let v_slope = dot(world.xz, horiz) / max(n.y, 0.25) + world.y * slope_len;
        let barrel_p = 0.24;
        let lap_p = 0.44;
        let keep_tile = 1.0 - lm_ss(0.05, 0.30, fp);
        let keep_course = 1.0 - lm_ss(0.10, 0.55, fp);
        let b_phase = fract(u_eave / barrel_p + seed.x * 0.37);
        let c_idx = floor(v_slope / lap_p);
        let b_idx = floor(u_eave / barrel_p + seed.x * 0.37);
        let lap_f = fract(v_slope / lap_p);
        // Barrel rib profile (convex semicircular clay tile) + stepped course-lap overhang shadow.
        let rib = sin(b_phase * 3.14159265);
        let trough = 1.0 - lm_ss(0.0, 0.24, min(b_phase, 1.0 - b_phase));
        let lap_sh = 1.0 - lm_ss(0.0, 0.16, lap_f);
        // Kiln-fired terracotta tile-to-tile hue & value diversity (sienna, ochre, dusty coral, umber).
        let th = st_hash(vec2<f32>(b_idx * 1.73, c_idx * 2.19) + seed);
        let kiln_tint = mix(vec3<f32>(0.82, 0.72, 0.66), vec3<f32>(1.16, 1.04, 0.90), th);
        let broad = 0.88 + 0.24 * (lm_noise(world / 4.2 + vec3<f32>(seed, 1.3)) * 0.5 + 0.5);
        let lichen = lm_ss(0.58, 0.88, lm_noise(world / 1.9 + vec3<f32>(seed, 5.7)) * 0.5 + 0.5);
        var alb = c * mix(vec3<f32>(1.0), kiln_tint, 0.62 * keep_course + 0.22) * broad;
        alb = alb * (1.0 - (0.24 * trough + 0.22 * lap_sh) * keep_tile);
        alb = mix(alb, alb * vec3<f32>(0.68, 0.67, 0.58), lichen * 0.26);
        (*s).albedo = alb;
        (*s).rough = clamp(max((*s).rough, 0.78) + 0.08 * trough * keep_tile - 0.04 * rib * keep_tile, 0.74, 0.94);
        (*s).ao = clamp((*s).ao * (1.0 - (0.22 * trough + 0.18 * lap_sh) * keep_tile), 0.25, 1.0);
        if (keep_tile > 0.02) {
            let h0 = (0.045 * rib - 0.024 * lap_sh) * keep_tile;
            let du_e = dot(dpx.xz, eave);
            let dv_e = dot(dpy.xz, eave);
            let du_s = dot(dpx.xz, horiz) / max(n.y, 0.25) + dpx.y * slope_len;
            let dv_s = dot(dpy.xz, horiz) / max(n.y, 0.25) + dpy.y * slope_len;
            let bx = fract((u_eave + du_e) / barrel_p + seed.x * 0.37);
            let by = fract((u_eave + dv_e) / barrel_p + seed.x * 0.37);
            let lx = 1.0 - lm_ss(0.0, 0.16, fract((v_slope + du_s) / lap_p));
            let ly = 1.0 - lm_ss(0.0, 0.16, fract((v_slope + dv_s) / lap_p));
            let hx = (0.045 * sin(bx * 3.14159265) - 0.024 * lx) * keep_tile;
            let hy = (0.045 * sin(by * 3.14159265) - 0.024 * ly) * keep_tile;
            (*s).n = lm_bump(n, dpx, dpy, h0, hx, hy);
        }
    } else {
        // Flat or cool/slate/metal landmark roof: weathered built-up tar/gravel membrane seams,
        // aggregate ballast drift, ponding stains & strict matte roughness guard (zero blue pool roofs).
        let macro_p = st_noise(world.xz / 6.2 + seed * 1.7);
        let stain = lm_ss(0.56, 0.86, macro_p);
        if (fp > 0.85) {
            (*s).albedo = c * (1.0 - 0.18 * stain);
            (*s).rough = clamp(max((*s).rough, select(0.80, 0.64, (*s).metal >= 0.25)), 0.62, 0.96);
        } else {
            let keep_seam = 1.0 - lm_ss(0.04, 0.28, fp);
            let strip_d = lm_line(world.x + seed.x * 3.1, 1.15);
            let cross_d = lm_line(world.z + seed.y * 5.3, 6.4);
            let seam = max(1.0 - lm_ss(0.012, 0.045, strip_d), 1.0 - lm_ss(0.012, 0.040, cross_d)) * keep_seam;
            let drift_p = st_noise(world.xz / 2.4 + seed * 2.9);
            let grain_p = st_noise(world.xz * 2.8 + vec2<f32>(3.7, 1.9));
            let gravel = lm_ss(0.50, 0.84, drift_p);
            let roll_tone = (st_hash(vec2<f32>(floor((world.x + seed.x * 3.1) / 1.15), floor(seed.y * 31.0))) - 0.5) * 0.08 * keep_seam;
            (*s).albedo = c * (0.94 + 0.12 * grain_p * (1.0 - lm_ss(0.06, 0.35, fp)) + roll_tone)
                * (1.0 - 0.22 * stain) * (1.0 + 0.12 * gravel) * (1.0 - 0.24 * seam);
            (*s).rough = clamp(max((*s).rough, select(0.80, 0.64, (*s).metal >= 0.25)) + 0.05 * gravel - 0.06 * stain, 0.62, 0.96);
            (*s).ao = clamp((*s).ao * (1.0 - 0.20 * seam), 0.25, 1.0);
        }
    }
    aaa_bldg_on = SF_AAA_SKYLINE_FIELD;
    aaa_glass_cov = 0.0;
}

fn lm_aaa_masonry(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, seed: vec2<f32>, storey_m: f32, is_trim: bool) {
    let fr = fa_frame(n);
    let u = dot(world, fr.t);
    let v = dot(world, fr.b);
    let fp = max(length(dpx), length(dpy));
    let c = (*s).albedo;
    let is_polished = (*s).rough < 0.36;
    let broad = st_noise(vec2<f32>(u, v) / 9.0 + seed * 2.1) * 2.0 - 1.0;
    if (fp > 1.6) {
        // Distant skyline fast path (1.5-3+ km): keep subtle broad patina & plinth grounding at minimal ALU.
        let base_soot = (1.0 - lm_ss(0.5, 5.2, world.y)) * 0.14;
        (*s).albedo = mix(c * (1.0 + broad * 0.06), c * vec3<f32>(0.46, 0.44, 0.40), base_soot);
        return;
    }
    let keep_fine = 1.0 - lm_ss(0.015, 0.11, fp);
    let keep_block = 1.0 - lm_ss(0.05, 0.34, fp);
    let keep_med = 1.0 - lm_ss(0.18, 1.25, fp);
    let is_brick = c.r > c.b * 1.48 && c.r > c.g * 1.12 && c.r < 0.45;

    // Multi-scale mineral grain & broad weathered patina (breaks flat CAD polygons at 10 m - 1.5 km).
    let med = (st_noise(vec2<f32>(u, v) / 2.2 + seed * 5.3) * 2.0 - 1.0) * keep_med;
    let fine = select(0.0, (st_noise(vec2<f32>(u, v) * 5.2 + seed * 9.7) * 2.0 - 1.0) * keep_fine, keep_fine > 0.01);

    // Ashlar stone blocks (0.92 m x 0.46 m) on limestone/sandstone/stucco or running-bond brick courses.
    let bw = select(select(0.92, 0.68, is_trim), 0.24, is_brick);
    let bh = select(select(0.46, 0.34, is_trim), 0.08, is_brick);
    let row_i = floor(v / bh);
    let stagger = fract(row_i * 0.5) * bw;
    let col_i = floor((u + stagger) / bw);
    let block_h = st_hash(vec2<f32>(col_i * 1.31, row_i * 2.17) + seed);
    let block_var = (block_h - 0.5) * select(0.085, 0.145, is_brick) * (0.35 + 0.65 * keep_block);
    let block_hue = mix(vec3<f32>(1.03, 1.00, 0.96), vec3<f32>(0.96, 0.99, 1.03), fract(block_h * 7.3));

    // Distance-filtered ashlar / mortar joints (muted on smooth stucco/polished granite, crisp on stone/brick).
    let du_b = lm_line(u + stagger, bw);
    let dv_b = lm_line(v, bh);
    let j_w = select(0.014, 0.008, is_brick);
    let j_str = select(select(0.24, 0.16, is_trim), 0.30, is_brick) * select(1.0, 0.32, is_polished);
    let joint = max(1.0 - lm_ss(0.003, j_w, du_b), 1.0 - lm_ss(0.003, j_w, dv_b)) * keep_block * j_str;

    // Subtle vertical rain-drip streaks under cornices, stringcourses & storey window sills,
    // plus beveled architectural stringcourse ledge highlights & honed/polished ground plinths.
    let sp = max(storey_m, 3.6);
    let sy = fract(world.y / sp);
    let below_sill = (1.0 - sy) * sp;
    let streak_n = lm_ss(0.32, 0.86, st_noise(vec2<f32>(u / 0.58, floor(world.y / sp) * 1.9) + seed * 3.7));
    let sill_drip = streak_n * exp(-below_sill / 0.95) * (0.095 + 0.065 * keep_med) * select(1.0, 0.45, is_polished);
    // Beveled horizontal storey stringcourse / spandrel lip catching top sun and shadow crevice below.
    let ledge_top = (1.0 - lm_ss(0.0, 0.14, sy * sp)) * keep_med * select(1.0, 0.35, is_brick);
    let ledge_under = (1.0 - lm_ss(0.0, 0.18, below_sill)) * keep_med * select(1.0, 0.35, is_brick);
    // Subtle dark splash-back & urban plinth grounding near ground level.
    let base_soot = (1.0 - lm_ss(0.5, 5.2, world.y)) * 0.16;
    let patina = 1.0 + broad * 0.065 + med * 0.055 + fine * 0.045 + block_var;
    var alb = c * mix(vec3<f32>(1.0), block_hue, 0.55 * keep_block) * patina;
    alb = mix(alb, alb * vec3<f32>(0.46, 0.44, 0.40), clamp(sill_drip + base_soot, 0.0, 0.42));
    alb = alb * (1.0 - joint) * (1.0 + 0.10 * ledge_top - 0.18 * ledge_under);
    (*s).albedo = alb;
    if (!is_polished) {
        // High-contrast PBR micro-roughness breakup: honed/polished stone podium plinth (`world.y < 9 m`)
        // vs weathered matte stone/precast upper shafts.
        let plinth_w = (1.0 - lm_ss(4.2, 9.2, world.y)) * select(1.0, 0.25, is_brick);
        let base_r = clamp((*s).rough + broad * 0.065 + fine * 0.045 + joint * 0.16 - sill_drip * 0.06, 0.58, 0.95);
        (*s).rough = mix(base_r, 0.30, plinth_w * 0.72);
    }
    (*s).ao = clamp((*s).ao * (1.0 - 0.32 * joint - 0.14 * base_soot - 0.24 * ledge_under), 0.18, 1.0);
    if (ledge_top > 0.01 || ledge_under > 0.01) {
        (*s).n = normalize((*s).n + vec3<f32>(0.0, 0.34 * ledge_top - 0.28 * ledge_under, 0.0));
    }
    if (keep_fine > 0.02 && !is_polished) {
        let j_amp = select(0.009, 0.005, is_brick) * keep_block * select(1.0, 0.65, is_trim);
        let g_amp = 0.0035 * keep_fine;
        let h0 = -j_amp * joint + g_amp * fine;
        let ux = dot(world + dpx, fr.t);
        let vx = dot(world + dpx, fr.b);
        let uy = dot(world + dpy, fr.t);
        let vy = dot(world + dpy, fr.b);
        let jx = max(1.0 - lm_ss(0.003, j_w, lm_line(ux + fract(floor(vx / bh) * 0.5) * bw, bw)), 1.0 - lm_ss(0.003, j_w, lm_line(vx, bh))) * keep_block * j_str;
        let jy = max(1.0 - lm_ss(0.003, j_w, lm_line(uy + fract(floor(vy / bh) * 0.5) * bw, bw)), 1.0 - lm_ss(0.003, j_w, lm_line(vy, bh))) * keep_block * j_str;
        let fx = (st_noise(vec2<f32>(ux, vx) * 5.2 + seed * 9.7) * 2.0 - 1.0) * keep_fine;
        let fy = (st_noise(vec2<f32>(uy, vy) * 5.2 + seed * 9.7) * 2.0 - 1.0) * keep_fine;
        (*s).n = lm_bump(n, dpx, dpy, h0, -j_amp * jx + g_amp * fx, -j_amp * jy + g_amp * fy);
    }
}
// sf:end

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
// sf:AAA_BUILDINGS-
        let cellm = vec2<f32>(max(m.rough_flatten, 0.1), max(m.thin_absorption, 0.1));
        let cell = floor(uv / cellm);
        let panel_tone = lm_noise(vec3<f32>(cell.x * 0.71, cell.y * 0.53, 0.3)) * LM_PC_PATCH_TONE;
        let drip = pow(1.0 - fract(uv.y / cellm.y), 6.0) * LM_PC_STREAK_TONE * vertical;
        let tone = lm_noise(local / LM_PC_PATCH_M) * LM_PC_PATCH_TONE;
        (*s).albedo = (*s).albedo * (1.0 + panel_tone + tone - drip);
        (*s).rough = clamp((*s).rough + tone * (LM_PC_ROUGH_SPREAD / max(1e-3, LM_PC_PATCH_TONE)), 0.4, 0.95);
        let footprint = ((abs(du.x) + abs(dv.x)) + (abs(du.y) + abs(dv.y))) * 0.5;
        if (LM_V3) {
            // the panels' chamfered joints (a 40 mm V-groove, its sealant and
            // the groove's own shadow reading about half the panel's tone,
            // INFERRED), box-filtered over the pixel so a far wall takes their share.
            let fu = max(abs(du.x) + abs(dv.x), 1e-4) / cellm.x;
            let fv = max(abs(du.y) + abs(dv.y), 1e-4) / cellm.y;
            let ju = 1.0 - mix(lm_pulse(uv.x / cellm.x, fu, 1.0 - 0.04 / cellm.x), 1.0 - 0.04 / cellm.x, lm_ss(0.5, 1.0, fu));
            let jv = 1.0 - mix(lm_pulse(uv.y / cellm.y, fv, 1.0 - 0.04 / cellm.y), 1.0 - 0.04 / cellm.y, lm_ss(0.5, 1.0, fv));
            let joint = clamp(ju + jv - ju * jv, 0.0, 1.0) * vertical;
            (*s).albedo = (*s).albedo * (1.0 - 0.5 * joint);
        }
        let fj = lm_fade(footprint, cellm.x);
        if (fj > 0.0) {
            let h0 = lm_precast_height(uv, cellm);
            let hx = lm_precast_height(uv + du, cellm);
            let hy = lm_precast_height(uv + dv, cellm);
            (*s).n = lm_bump(n, dpx, dpy, h0 * fj, hx * fj, hy * fj);
        }
// sf:end
// sf:AAA_BUILDINGS+
        let lm_seed = vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173);
        let is_trim_or_roof = m.rough_flatten < 0.05;
        if (SF_AAA_DETAIL && n.y > 0.35) {
            lm_aaa_roof(s, world, n, dpx, dpy, lm_seed);
        } else if (SF_AAA_DETAIL && is_trim_or_roof) {
            // Contract with skyline.rs: `precast` with `bayM=0, storeyM=0` denotes landmark masonry
            // trim, cornices, entablatures & pier walls (skip the 40 mm tower V-groove grid).
            lm_aaa_masonry(s, world, n, dpx, dpy, lm_seed, 4.2, true);
            aaa_bldg_on = true;
            aaa_glass_cov = 0.0;
        } else {
            let cellm = vec2<f32>(max(m.rough_flatten, 0.1), max(m.thin_absorption, 0.1));
            let cell = floor(uv / cellm);
            let panel_tone = lm_noise(vec3<f32>(cell.x * 0.71, cell.y * 0.53, 0.3)) * LM_PC_PATCH_TONE;
            let drip = pow(1.0 - fract(uv.y / cellm.y), 6.0) * LM_PC_STREAK_TONE * vertical;
            let tone = lm_noise(local / LM_PC_PATCH_M) * LM_PC_PATCH_TONE;
            (*s).albedo = (*s).albedo * (1.0 + panel_tone + tone - drip);
            (*s).rough = clamp((*s).rough + tone * (LM_PC_ROUGH_SPREAD / max(1e-3, LM_PC_PATCH_TONE)), 0.4, 0.95);
            let footprint = ((abs(du.x) + abs(dv.x)) + (abs(du.y) + abs(dv.y))) * 0.5;
            if (LM_V3) {
                let fu = max(abs(du.x) + abs(dv.x), 1e-4) / cellm.x;
                let fv = max(abs(du.y) + abs(dv.y), 1e-4) / cellm.y;
                let ju = 1.0 - mix(lm_pulse(uv.x / cellm.x, fu, 1.0 - 0.04 / cellm.x), 1.0 - 0.04 / cellm.x, lm_ss(0.5, 1.0, fu));
                let jv = 1.0 - mix(lm_pulse(uv.y / cellm.y, fv, 1.0 - 0.04 / cellm.y), 1.0 - 0.04 / cellm.y, lm_ss(0.5, 1.0, fv));
                let joint = clamp(ju + jv - ju * jv, 0.0, 1.0) * vertical;
                (*s).albedo = (*s).albedo * (1.0 - 0.5 * joint);
            }
            let fj = lm_fade(footprint, cellm.x);
            if (fj > 0.0) {
                let h0 = lm_precast_height(uv, cellm);
                let hx = lm_precast_height(uv + du, cellm);
                let hy = lm_precast_height(uv + dv, cellm);
                (*s).n = lm_bump(n, dpx, dpy, h0 * fj, hx * fj, hy * fj);
            }
            if (SF_AAA_DETAIL) {
                let fine_g = lm_noise(world * 4.5 + vec3<f32>(lm_seed, 3.1)) * (1.0 - lm_ss(0.02, 0.14, footprint));
                (*s).albedo = (*s).albedo * (1.0 + fine_g * 0.045);
            }
            aaa_bldg_on = true;
            aaa_glass_cov = 0.0;
        }
// sf:end
    } else if (kind == 5u) {
        let x = uv.x / LM_FC_MODULE_M;
        let fw = max(abs(du.x) + abs(dv.x), 1e-4 * LM_FC_MODULE_M) / LM_FC_MODULE_M;
        let filtered = (lm_fin_integral(x + fw * 0.5) - lm_fin_integral(x - fw * 0.5)) / fw;
        let fin = select(0.0, clamp(filtered, 0.0, 1.0), abs(n.y) < 0.5);
// sf:AAA_BUILDINGS-
        (*s).albedo = mix((*s).albedo, vec3<f32>(LM_FC_FIN_R, LM_FC_FIN_G, LM_FC_FIN_B), fin);
        (*s).rough = mix(LM_FC_GLASS_ROUGH, LM_FC_FIN_ROUGH, fin);
        (*s).metal = mix(LM_FC_GLASS_METAL, 0.0, fin);
// sf:end
// sf:AAA_BUILDINGS+
        if (SF_AAA_BRDF && abs(n.y) < 0.5) {
            // Fin curtain wall (e.g. Salesforce Tower): optically crisp mirror glass (`0.016-0.042`)
            // with distinct pane-by-pane fabrication bow/tilt faceting (`±1.5 deg`), layered interior
            // floor slabs/blinds behind deep dark glass, and bright anodized metallic aluminum fins.
            let base_c = (*s).albedo;
            let g_lum = max(dot(base_c, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-3);
            let g_hue = mix(vec3<f32>(1.0), clamp(base_c / g_lum, vec3<f32>(0.72), vec3<f32>(1.26)), 0.58);
            let raw_f0 = mix(vec3<f32>(0.06), base_c, LM_FC_GLASS_METAL);
            let seed_fc = vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173);
            let fy_fc = max(abs(du.y) + abs(dv.y), 1e-3) / 4.1;
            let c_near = floor(vec2<f32>(x, world.y / 4.1));
            let near_w = 1.0 - lm_ss(0.40, 1.35, max(fw, fy_fc));
            let jn = (vec2<f32>(st_hash(c_near * vec2<f32>(1.37, 2.11) + seed_fc), st_hash(c_near * vec2<f32>(2.71, 1.13) + seed_fc + 4.1)) * 2.0 - 1.0) * (0.028 * near_w);
            let wave_p = vec2<f32>(x * 0.24 + seed_fc.x * 5.0, world.y * 0.085);
            let jm = (vec2<f32>(st_noise(wave_p), st_noise(wave_p + vec2<f32>(7.3, 3.1))) * 2.0 - 1.0) * 0.011;
            let jt = jn + jm;
            let gn_fc = normalize((*s).n + fr.t * jt.x + vec3<f32>(0.0, jt.y + 0.044, 0.0));
            let batch_v = (st_noise(wave_p * 1.4 + vec2<f32>(2.1, 5.9)) * 2.0 - 1.0) * 0.10;
            let suite_v = st_hash(vec2<f32>(floor(world.y / 12.3) * 1.71, 5.3) + seed_fc);
            // Layered interior floor slab shadow & varied-height blinds visible inside crisp glass.
            let view_fc = normalize(frame.cam_pos.xyz - world);
            let vn_fc = max(dot(view_fc.xz, normalize(n.xz + vec2<f32>(1e-5, 0.0))), 0.14);
            let vy_int = fract((world.y / 4.1) + clamp(view_fc.y / vn_fc * 0.20, -0.35, 0.35));
            let slab_dark = 1.0 - 0.78 * (1.0 - lm_ss(0.06, 0.14, vy_int) * lm_ss(0.94, 0.84, vy_int)) * near_w;
            let blind_h = mix(0.25, 0.65, st_hash(c_near * vec2<f32>(0.33, 1.91) + seed_fc + 7.1));
            let blind_on = step(0.54, st_hash(c_near * vec2<f32>(0.25, 1.43) + seed_fc + 2.3)) * step(1.0 - blind_h, vy_int) * near_w;
            let room_fc = mix(base_c * (1.0 - LM_FC_GLASS_METAL) * mix(0.08, 0.22, suite_v) * slab_dark, vec3<f32>(0.24, 0.25, 0.26), blind_on * 0.42);
            (*s).albedo = vec3<f32>(LM_FC_FIN_R, LM_FC_FIN_G, LM_FC_FIN_B) * (0.94 + 0.08 * suite_v);
            (*s).rough = 0.22;
            (*s).metal = 0.76;
            aaa_bldg_on = true;
            aaa_glass_cov = 1.0 - fin;
            aaa_glass_rough = clamp(mix(0.016, 0.042, lm_ss(0.35, 1.6, fy_fc)), 0.015, 0.048);
            aaa_glass_f0 = clamp(max(raw_f0 * 1.68, vec3<f32>(0.31) * g_hue) * (1.0 + batch_v), vec3<f32>(0.18), vec3<f32>(0.46));
            aaa_glass_n = gn_fc;
            aaa_glass_room = room_fc;
        } else {
            (*s).albedo = mix((*s).albedo, vec3<f32>(LM_FC_FIN_R, LM_FC_FIN_G, LM_FC_FIN_B), fin);
            (*s).rough = mix(LM_FC_GLASS_ROUGH, LM_FC_FIN_ROUGH, fin);
            (*s).metal = mix(LM_FC_GLASS_METAL, 0.0, fin);
            if (SF_AAA_DETAIL && n.y > 0.35) {
                lm_aaa_roof(s, world, n, dpx, dpy, vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173));
            } else {
                aaa_bldg_on = true;
                aaa_glass_cov = 0.0;
            }
        }
// sf:end
    } else if (kind == 6u) {
        let c = (*s).albedo;
        let warmth = (c.x - c.z) / max(c.x, 1e-3);
        let gilt = lm_ss(LM_GC_WARM_FROM, LM_GC_WARM_TO, warmth);
        let tone = lm_noise(local / LM_GC_PATCH_M) * LM_GC_PATCH_TONE * (1.0 - gilt);
        (*s).albedo = (*s).albedo * (1.0 + tone);
        let gilt_scale = LM_GC_GILT_ROUGH / max(1e-3, m.params.x);
        (*s).rough = clamp((*s).rough * mix(1.0, gilt_scale, gilt), 0.05, 1.0);
        (*s).metal = mix((*s).metal, LM_GC_GILT_METAL, gilt);
        if (LM_V4) {
            // gold leaf a metal (eo-landmarks towers3.rs GILT4_*: its albedo gold's F0, its roughness).
            (*s).rough = mix((*s).rough, LM_GILT4_ROUGH, gilt);
            (*s).metal = mix((*s).metal, 1.0, gilt);
        }
    }
}

// ---- the authored facade (surface kind 7; eo-landmarks skyline.rs styles,
// LM_FACADES: three rows a style, the index in `rough_flatten`, the storey
// origin in `thin_absorption`) ----
//
// A window grid of `moduleM` x `storeyM`, the glazing `glazeX` x `glazeY` of
// each cell, box-filtered over the pixel's footprint (the exact integral of
// the periodic pulse, so a far wall takes the glass's share of its tone with
// no moire). The glass is a coated pane drawn as a near-mirror of its tint
// (metalness 0.85, its roughness): in the painted facades' environment class
// that reflects about a fifth of the sky, a coated curtain wall's visible
// reflectance. At night a window is lit as the far boxes' are
// (sky_far_windows: the city's occupancy, warm or cool, the same luminance),
// times the style's `nightLit`. The `screen` pattern is an open lattice over a
// dark interior; `led` the Salesforce crown's light sculpture.

// The integral of a unit-period pulse of width `duty` centred in each period.
fn lm_pulse_int(t: f32, duty: f32) -> f32 {
    return floor(t) * duty + clamp(fract(t) - 0.5 * (1.0 - duty), 0.0, duty);
}

// The share of [x - w/2, x + w/2] (periods) that lies in the pulses.
fn lm_pulse(x: f32, w: f32, duty: f32) -> f32 {
    let ww = max(w, 1e-3);
    return clamp((lm_pulse_int(x + 0.5 * ww, duty) - lm_pulse_int(x - 0.5 * ww, duty)) / ww, 0.0, 1.0);
}

// Day for Night (Jim Campbell, 2018): low-resolution moving figures on the
// top six of the crown's nine storeys, a colour wash on the three below.
// Luminance INFERRED: 14 cd/m2 at a figure's brightest, 2 between, the wash
// 4 cd/m2 (an LED field behind a perforated screen, read against the lit
// windows' 8 cd/m2).
// INFERRED: an office floor's lighting zone (a bay of the ceiling grid), 6 m.
const LM_ZONE_M: f32 = 6.0;

// The crown's height over the top floor: 'the top 150 feet (46 m) above the
// 61st floor' (Wikipedia, Salesforce Tower).
const LM_LED_CROWN_M: f32 = 45.72;

fn lm_crown_led(u_m: f32, y_rel: f32, crown_m: f32) -> vec3<f32> {
    let px = floor(vec2<f32>(u_m, y_rel) / 1.2);
    let top = lm_ss(crown_m * 0.33 - 0.6, crown_m * 0.33 + 0.6, y_rel);
    let figure = lm_ss(0.18, 0.52, lm_noise(vec3<f32>(px.x * 0.13, px.y * 0.09, 3.7)))
        * (0.55 + 0.45 * lm_ss(-0.2, 0.4, lm_noise(vec3<f32>(px.x * 0.5, px.y * 0.5, 9.1))));
    let white = vec3<f32>(0.93, 0.96, 1.0) * (2.0 + 12.0 * figure);
    let wash = vec3<f32>(0.42, 0.55, 1.0) * 4.0;
    return mix(wash, white, top);
}

fn lm_authored(s: ptr<function, Surface>, m: Material, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let i = min(u32(round(max(m.rough_flatten, 0.0))), max(LM_FACADE_COUNT, 1u) - 1u) * LM_FACADE_ROWS;
    let r0 = LM_FACADES[i];
    let r1 = LM_FACADES[i + 1u];
    let r2 = LM_FACADES[i + 2u];
    // walls only: a roof or a soffit keeps its own finish.
    let vertical = lm_ss(0.45, 0.25, abs(n.y));
    if (vertical <= 0.0) {
// sf:AAA_BUILDINGS+
        if (SF_AAA_DETAIL && n.y > 0.35) {
            lm_aaa_roof(s, world, n, dpx, dpy, vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173));
        }
// sf:end
        return;
    }
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let module_m = max(r0.x, 0.05);
    let storey_m = max(r0.y, 0.05);
    let u = dot(world.xz, t) / module_m;
    let v = (world.y - m.thin_absorption) / storey_m;
    let fu = (abs(dot(dpx.xz, t)) + abs(dot(dpy.xz, t))) / module_m;
    let fv = (abs(dpx.y) + abs(dpy.y)) / storey_m;
    // The frame (fins, sunshades, reveals) stands `frame depth` proud of the
    // glass: seen at an angle it hides depth x tan(angle) of each pane.
    let view = normalize(frame.cam_pos.xyz - world);
    let nh = normalize(n.xz + vec2<f32>(1e-5, 0.0));
    let vn = max(dot(view.xz, nh), 0.08);
    let gx = max(r0.z - r2.w * abs(dot(view.xz, t)) / vn / module_m, 0.0);
    let gy = max(r0.w - r2.w * abs(view.y) / vn / storey_m, 0.0);
    let glass = lm_pulse(u, fu, gx) * lm_pulse(v, fv, gy) * vertical;
    (*s).albedo = mix((*s).albedo, r1.xyz, glass);
    (*s).rough = mix((*s).rough, r1.w, glass);
    if (r2.x < 0.5) {
        (*s).metal = mix((*s).metal, 0.85, glass);
    }
    let dusk = look.rooms_env.x;
    if (dusk <= 0.0 || !sky_on(skyu.flags.x, SKY_LIGHTS) || frame.copy_step.z > 0.5) {
        return;
    }
    let level = skyu.bloom_w2.z * dusk;
    if (r2.z > 0.5) {
        rm_emit = rm_emit + lm_crown_led(u * module_m, world.y - m.thin_absorption, LM_LED_CROWN_M) * level * vertical;
    }
    if (r2.y > 0.0) {
        // An office's lighting zone: whole modules at least LM_ZONE_M wide, a storey high.
        let zone = max(1.0, ceil(LM_ZONE_M / module_m));
        let cell = floor(vec2<f32>(u / zone, v));
        let seed = vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173);
        let occ = clamp(look.rooms_env.y * r2.y, 0.0, 1.0);
        // Floors light together: each floor's own share about the occupancy (mean kept), then its zones.
        let floor_share = clamp(occ * (0.35 + 1.3 * st_hash(vec2<f32>(cell.y * 3.17, 7.9) + seed)), 0.0, 1.0);
        let lit = step(st_hash(cell * vec2<f32>(1.37, 2.11) + seed), floor_share) * (0.6 + 0.8 * st_hash(cell + 5.1 + seed));
        let cool = step(1.0 - look.rooms_env.w, st_hash(cell + 17.3 + seed));
        let col = select(vec3<f32>(1.0, 0.62, 0.34), vec3<f32>(0.86, 0.93, 1.0), cool > 0.5);
        let k = mix(lit, occ, lm_ss(0.25, 0.6, max(fu / zone, fv)));
        // The far boxes' window luminance: 25 lux on a diffuser (sky_far_windows).
        rm_emit = rm_emit + col * k * glass * level * 25.0 / LK_PI;
    }
}

// A floodlit landmark material at night (LM_F_FLOOD): `base.x` lux at the lit
// band's foot (`base.y`, world y), falling to a quarter of it at its head
// (`base.z`: the beams spread with height), on the walls (ground-mounted
// floodlights aim up: a third on faces turned up, none on faces turned
// down); the lit surface's diffuse radiance, E x albedo / pi, in the
// floodlights' colour, in the night lighting's units (lamp_k a lux).
fn lm_flood(s: ptr<function, Surface>, m: Material, world: vec3<f32>, n: vec3<f32>) {
    if ((m.flags & LM_F_FLOOD) == 0u) {
        return;
    }
    let dusk = look.rooms_env.x;
    if (dusk <= 0.0 || !sky_on(skyu.flags.x, SKY_LIGHTS) || frame.copy_step.z > 0.5) {
        return;
    }
    let y0 = m.base.y;
    let y1 = max(m.base.z, y0 + 1.0);
    let t = clamp((world.y - y0) / (y1 - y0), 0.0, 1.0);
    let band = lm_ss(y1 + 2.0, y1 - 1.0, world.y) * lm_ss(y0 - 1.0, y0 + 0.5, world.y);
    let facing = clamp(1.0 - abs(n.y), 0.0, 1.0) + max(n.y, 0.0) * 0.35;
    let e = m.base.x * mix(1.0, 0.25, pow(t, 0.7)) * band * facing;
    rm_emit = rm_emit + (*s).albedo * (1.0 - 0.6 * (*s).metal) * LM_TOWER_FLOOD_RGB * e * skyu.bloom_w2.z * dusk / LK_PI;
}


// ---- the third pass (LM_V3; eo-app --no-towers3 draws the previous above) ----
//
// `lm_authored3`: the authored facade as `lm_authored` with
//  * the window grid's box filter faded to its mean once a period is under
//    two pixels (past Nyquist a box filter's ringing is the stripe moire);
//  * the glass a dielectric pane of the style's visible reflectance over a
//    dark room (skyline.rs `pane_albedo_metal`: albedo and metalness that
//    give F0 = the reflectance, diffuse = the room), drawn in the glazing's
//    environment class (look_light `lm_glaze`), each pane tilted by up to the
//    style's set-out angle (a curtain wall's broken reflections), the tilt
//    turning into roughness where panes fall under a pixel;
//  * at night, office bays (`bayModules` modules a storey) lit by floor and
//    by tenant run (floors cluster: some dark, some mostly lit, the mean the
//    occupancy x `nightLit`), each at a lit window's luminance (`windowCd`,
//    under the night exposure's clip) times a per-bay dimming;
//  * the crown's sculpture as a blue colour wash over the whole crown and
//    moving low-resolution figures over its top six storeys, seen through
//    the screen's openings (the members lit a third as much by the field behind).

const LM_CROWN_LED_PITCH_M: f32 = 1.6;

fn lm_led3(u_m: f32, y_rel: f32, crown_m: f32, open: f32) -> vec3<f32> {
    let t = skyu.wet.w;
    let p = vec2<f32>(u_m, y_rel) / LM_CROWN_LED_PITCH_M;
    // the figures: soft blobs drifting sideways and fading (Day for Night's walkers, INFERRED motion).
    let f0 = lm_noise(vec3<f32>(p.x * 0.11 - t * 0.09, p.y * 0.07, t * 0.05 + 3.7));
    let f1 = lm_noise(vec3<f32>(floor(p.x) * 0.31 + t * 0.13, floor(p.y) * 0.23, 9.1));
    let figure = lm_ss(0.15, 0.55, f0) * (0.7 + 0.3 * f1);
    let upper = lm_ss(crown_m * 0.33 - 1.5, crown_m * 0.33 + 1.5, y_rel);
    let wash = vec3<f32>(0.32, 0.5, 1.0) / dot(vec3<f32>(0.32, 0.5, 1.0), vec3<f32>(0.2126, 0.7152, 0.0722));
    let white = vec3<f32>(0.9, 0.95, 1.0);
    let l = wash * LM_LED_WASH_CD * (0.8 + 0.2 * f1) + white * LM_LED_FIGURE_CD * figure * upper;
    return l * (open + 0.33 * (1.0 - open));
}

fn lm_authored3(s: ptr<function, Surface>, m: Material, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let i = min(u32(round(max(m.rough_flatten, 0.0))), max(LM_FACADE_COUNT, 1u) - 1u) * LM_FACADE_ROWS;
    let r0 = LM_FACADES[i];
    let r1 = LM_FACADES[i + 1u];
    let r2 = LM_FACADES[i + 2u];
    let r3 = LM_FACADES[i + 3u];
    let vertical = lm_ss(0.45, 0.25, abs(n.y));
    if (vertical <= 0.0) {
// sf:AAA_BUILDINGS+
        if (SF_AAA_DETAIL && n.y > 0.35) {
            lm_aaa_roof(s, world, n, dpx, dpy, vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173));
        }
// sf:end
        return;
    }
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let module_m = max(r0.x, 0.05);
    let storey_m = max(r0.y, 0.05);
    let u = dot(world.xz, t) / module_m;
    let v = (world.y - m.thin_absorption) / storey_m;
    let fu = (abs(dot(dpx.xz, t)) + abs(dot(dpy.xz, t))) / module_m;
    let fv = (abs(dpx.y) + abs(dpy.y)) / storey_m;
    let view = normalize(frame.cam_pos.xyz - world);
    let nh = normalize(n.xz + vec2<f32>(1e-5, 0.0));
    let vn = max(dot(view.xz, nh), 0.08);
    let gx = max(r0.z - r2.w * abs(dot(view.xz, t)) / vn / module_m, 0.0);
    let gy = max(r0.w - r2.w * abs(view.y) / vn / storey_m, 0.0);
    let pu = mix(lm_pulse(u, fu, gx), gx, lm_ss(0.5, 1.0, fu));
    let pv = mix(lm_pulse(v, fv, gy), gy, lm_ss(0.5, 1.0, fv));
    let glass = pu * pv * vertical;
    let seed = vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173);
    let cell = floor(vec2<f32>(u, v));
    // polished stone (the frame's own roughness under 0.35) reflects as the glazing does.
    let polish = lm_ss(0.35, 0.2, (*s).rough);
// sf:AAA_BUILDINGS-
    (*s).albedo = mix((*s).albedo, r1.xyz, glass);
    (*s).rough = mix((*s).rough, r1.w, glass);
    if (r2.x < 0.5) {
        (*s).metal = mix((*s).metal, r3.x, glass);
        lm_glaze = max(glass, polish * vertical);
        if (r3.y > 0.0) {
            let j = (vec2<f32>(st_hash(cell * vec2<f32>(1.37, 2.11) + seed + 3.3), st_hash(cell * vec2<f32>(2.71, 1.13) + seed + 8.1)) * 2.0 - 1.0) * r3.y;
            let far_k = lm_ss(0.3, 1.0, max(fu, fv));
            let tt = vec3<f32>(t.x, 0.0, t.y);
            let nj = normalize((*s).n + (tt * j.x + vec3<f32>(0.0, j.y, 0.0)) * (1.0 - far_k));
            (*s).n = normalize(mix((*s).n, nj, glass));
            let r = (*s).rough;
            (*s).rough = mix(r, sqrt(r * r + 4.0 * r3.y * far_k), glass);
        }
    }
// sf:end
// sf:AAA_BUILDINGS+
    if (SF_AAA_BRDF && r2.x < 0.5) {
        lm_glaze = polish * vertical;
        let curtain_w = lm_ss(0.35, 0.68, r0.z * r0.w);
        if (SF_AAA_DETAIL && curtain_w < 0.58) {
            lm_aaa_masonry(s, world, n, dpx, dpy, seed, storey_m, false);
        }
        // Crisp anodized metallic mullions on glass curtain walls vs honed stone piers.
        if (curtain_w > 0.42) {
            (*s).rough = mix((*s).rough, 0.24, curtain_w * 0.85);
            (*s).metal = mix((*s).metal, 0.74, curtain_w * 0.85);
        }
        let tilt_amp = max(r3.y, mix(0.016, 0.028, curtain_w));
        let far_k = lm_ss(0.40, 1.35, max(fu, fv));
        let j = (vec2<f32>(st_hash(cell * vec2<f32>(1.37, 2.11) + seed + 3.3), st_hash(cell * vec2<f32>(2.71, 1.13) + seed + 8.1)) * 2.0 - 1.0) * (tilt_amp * (1.0 - far_k));
        let wave_a = vec2<f32>(u * 0.22 + seed.x * 5.0, v * 0.30);
        let jm = (vec2<f32>(st_noise(wave_a), st_noise(wave_a + vec2<f32>(7.1, 2.9))) * 2.0 - 1.0) * mix(0.005, 0.012, curtain_w);
        let tt = vec3<f32>(t.x, 0.0, t.y);
        let gn = normalize(n + tt * (j.x + jm.x) + vec3<f32>(0.0, j.y + jm.y + 0.044 * curtain_w, 0.0));
        let g_lum = max(dot(r1.xyz, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-3);
        let g_hue = mix(vec3<f32>(1.0), clamp(r1.xyz / g_lum, vec3<f32>(0.66), vec3<f32>(1.30)), 0.58);
        let raw_f0 = mix(vec3<f32>(0.04), r1.xyz, r3.x);
        let min_f0 = mix(0.13, 0.32, curtain_w) * g_hue;
        let mac_batch = (st_noise(wave_a * 1.35 + vec2<f32>(3.3, 1.9)) * 2.0 - 1.0) * 0.095;
        let pane_f0 = clamp(max(raw_f0 * mix(1.12, 1.65, curtain_w), min_f0) * (1.0 + mac_batch), vec3<f32>(0.09), vec3<f32>(0.46));
        let vy_int = fract(v + clamp(view.y / vn * 0.22, -0.32, 0.32));
        let slab_d = 1.0 - 0.72 * (1.0 - lm_ss(0.07, 0.16, vy_int) * lm_ss(0.93, 0.82, vy_int)) * (1.0 - far_k);
        let blind_on = step(0.56, st_hash(floor(vec2<f32>(u * 0.35, v)) + seed + 2.1)) * step(0.52, vy_int) * (1.0 - far_k);
        let room_c3 = mix(r1.xyz * (1.0 - r3.x) * mix(0.24, 0.11, curtain_w) * slab_d, vec3<f32>(0.24, 0.24, 0.25), blind_on * 0.36);
        aaa_bldg_on = true;
        aaa_glass_cov = glass;
        aaa_glass_rough = clamp(sqrt(min(r1.w, 0.036) * min(r1.w, 0.036) + 0.14 * tilt_amp * tilt_amp * far_k), 0.016, 0.046);
        aaa_glass_f0 = pane_f0;
        aaa_glass_n = gn;
        aaa_glass_room = room_c3;
    } else {
        (*s).albedo = mix((*s).albedo, r1.xyz, glass);
        (*s).rough = mix((*s).rough, r1.w, glass);
        if (r2.x < 0.5) {
            (*s).metal = mix((*s).metal, r3.x, glass);
            lm_glaze = max(glass, polish * vertical);
        }
        aaa_bldg_on = true;
        aaa_glass_cov = 0.0;
    }
// sf:end
    let dusk = look.rooms_env.x;
    if (dusk <= 0.0 || !sky_on(skyu.flags.x, SKY_LIGHTS) || frame.copy_step.z > 0.5) {
        return;
    }
    let level = skyu.bloom_w2.z * dusk;
    if (r2.z > 0.5) {
        rm_emit = rm_emit + lm_led3(u * module_m, world.y - m.thin_absorption, LM_LED_CROWN_M, glass) * level * vertical;
    }
    if (r2.y > 0.0) {
        let bay = max(r3.z, 1.0);
        let bc = floor(vec2<f32>(u / bay, v));
        let occ = clamp(look.rooms_env.y * r2.y, 0.0, 1.0);
        // the floor's share (uniform over occ -/+ 0.8, clamped: about a quarter of floors dark, the mean about occ).
        let fh = st_hash(vec2<f32>(bc.y * 3.17, 7.9) + seed);
        let floor_share = clamp(occ + (fh - 0.5) * 1.6, 0.0, 1.0);
        // tenant runs of two to five bays along the floor, each lit or not.
        let run_len = 2.0 + floor(st_hash(vec2<f32>(bc.y * 1.91, 2.3) + seed) * 4.0);
        let run = floor((bc.x + 13.0 * st_hash(vec2<f32>(bc.y, 4.4) + seed)) / run_len);
        let lit = step(st_hash(vec2<f32>(run * 1.37, bc.y * 2.11) + seed), floor_share);
        // per bay: a lamp's spread and a few blinds down.
        let dim = (0.7 + 0.3 * st_hash(bc + 5.1 + seed)) * mix(1.0, 0.35, step(0.86, st_hash(bc + 11.7 + seed)));
        let cool = step(1.0 - look.rooms_env.w, st_hash(vec2<f32>(run, bc.y) + 17.3 + seed));
        let warm_c = vec3<f32>(1.0, 0.62, 0.34) / 0.6779;
        let cool_c = vec3<f32>(0.86, 0.93, 1.0) / 0.9200;
        let col = select(warm_c, cool_c, cool > 0.5);
        // past a bay a pixel, the mean (the floors' mean share x the mean dimming).
        let k = mix(lit * dim, occ * 0.78, lm_ss(0.25, 0.6, max(fu / bay, fv)));
        rm_emit = rm_emit + col * k * glass * level * max(r3.w, 0.1);
    }
}

// ---- the fourth pass (LM_V4; eo-app --no-towers4 draws the third above) ----
//
// `lm_authored4`: `lm_authored3` with
//  * the frame split into its vertical piers (between the glass columns) and
//    the spandrel band (under the glass columns, between one floor's glass and
//    the next) at the style's `spandrelTone` of the frame, so floor lines read
//    apart from fins and ribs;
//  * a pier's polished centre (`polishShare` of its width, `polishRoughness`)
//    and its honed rest (`honedTone`, `honedRoughness`): 555 California's
//    polished and rough carnelian granite;
//  * with `revealShadow`, the frame's depth (`finDepthM`) shading the glass
//    and its own reveals from the sun: of each pane, the share the fins and
//    the sunshade (or lintel) over it shadow at the sun's angles to the wall
//    (depth x tan of each), and of the reveals the view shows (the share the
//    depth hides of the glass), the side reveal turned from the sun and the
//    soffit seen from below: all exact shares of the cell, so a far wall takes
//    their mean (no moire); the key light is cut by that share
//    (look_light `lm_unlit`);
//  * at night the office bays lit by runs of one to three bays (tenants'
//    zones), and the crown's sculpture as figures on its LED grid (`lm_led4`).

// Day for Night (Jim Campbell, 2018): about 11,000 LEDs on the crown's top
// six floors (Wikipedia, Salesforce Tower; KQED, 2018: they point back at
// the building, their light coming out through the perforated aluminium
// panels). Over the six floors' face (about 170 m round, 27 m high) that is
// an LED every 0.69 m (INFERRED from the count and the face).
const LM_LED4_PITCH_M: f32 = 0.69;
// City Hall's gilding's roughness (eo-landmarks towers3.rs GILT4_ROUGHNESS).
const LM_GILT4_ROUGH: f32 = 0.38;
// The figures: people walking, about 19 m tall on the screen (most of the
// LED floors' height), one slot every 15 m, drifting 1.6 m/s; their LEDs
// read through the panels as soft dots (INFERRED: the work shows the city's
// walkers, filmed by day, at night).
const LM_LED4_FIGURE_M: f32 = 19.0;
const LM_LED4_SLOT_M: f32 = 15.0;
const LM_LED4_DRIFT: f32 = 1.6;

fn lm_seg_sdf(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

// A walker's silhouette (figure units: height 1, feet at y 0), its stride phase `ph`.
fn lm_walker_sdf(p: vec2<f32>, ph: f32) -> f32 {
    let sw = sin(ph);
    var d = length(p - vec2<f32>(0.0, 0.925)) - 0.068;
    d = min(d, lm_seg_sdf(p, vec2<f32>(0.0, 0.53), vec2<f32>(0.0, 0.81), 0.085));
    d = min(d, lm_seg_sdf(p, vec2<f32>(0.0, 0.53), vec2<f32>(0.24 * sw, 0.02), 0.05));
    d = min(d, lm_seg_sdf(p, vec2<f32>(0.0, 0.53), vec2<f32>(-0.24 * sw, 0.02), 0.05));
    d = min(d, lm_seg_sdf(p, vec2<f32>(0.0, 0.79), vec2<f32>(-0.15 * sw, 0.49), 0.035));
    d = min(d, lm_seg_sdf(p, vec2<f32>(0.0, 0.79), vec2<f32>(0.15 * sw, 0.49), 0.035));
    return d;
}

// The sculpture's luminance (cd/m2) at (u along the face, y over the top floor), the pixel's footprint `fp` (m).
fn lm_led4(u_m: f32, y_rel: f32, crown_m: f32, fp: f32) -> vec3<f32> {
    let t = skyu.wet.w;
    // the LED grid's dots while a dot is over a pixel; the image itself past it.
    let q = select(vec2<f32>(u_m, y_rel), (floor(vec2<f32>(u_m, y_rel) / LM_LED4_PITCH_M) + 0.5) * LM_LED4_PITCH_M, fp < LM_LED4_PITCH_M);
    let lower = crown_m / 3.0;
    var lum = 0.0;
    for (var lane = 0; lane < 2; lane = lane + 1) {
        let dir = select(-1.0, 1.0, lane == 0);
        let x = q.x - dir * t * LM_LED4_DRIFT + f32(lane) * LM_LED4_SLOT_M * 0.5;
        let k = floor(x / LM_LED4_SLOT_M);
        let present = step(st_hash(vec2<f32>(k * 1.73, f32(lane) * 7.1 + 3.3)), 0.85);
        let hscale = LM_LED4_FIGURE_M * (0.85 + 0.25 * st_hash(vec2<f32>(k, 11.0 + f32(lane))));
        let cx = (k + 0.5) * LM_LED4_SLOT_M + (st_hash(vec2<f32>(k, 5.0 + f32(lane))) - 0.5) * 6.0;
        let foot = lower + 0.5 + st_hash(vec2<f32>(k, 9.0 + f32(lane))) * 1.5;
        let p = vec2<f32>((x - cx) * dir, q.y - foot) / hscale;
        let ph = (t * LM_LED4_DRIFT / hscale) * 9.0 + st_hash(vec2<f32>(k, 2.0)) * 6.28;
        let d = lm_walker_sdf(p, ph) * hscale;
        // soft by about a dot (the LEDs' light reflected out through the panels), wider than a pixel.
        let w = max(LM_LED4_PITCH_M * 0.6, fp);
        let bright = 0.65 + 0.35 * st_hash(vec2<f32>(k, 13.0 + f32(lane)));
        lum = max(lum, present * bright * lm_ss(w, -w, d));
    }
    let upper = lm_ss(lower - 1.5, lower + 1.5, y_rel);
    // the three floors below: a slow colour field (blue to violet to teal); the upper field's ground a dim grey-blue.
    let hue = 0.5 + 0.5 * sin(t * 0.05 + u_m * 0.01);
    let field = mix(vec3<f32>(0.32, 0.5, 1.0), mix(vec3<f32>(0.62, 0.42, 1.0), vec3<f32>(0.3, 0.85, 0.9), hue), 0.5);
    let fl = field / dot(field, vec3<f32>(0.2126, 0.7152, 0.0722));
    let ground = vec3<f32>(0.7, 0.8, 1.0) / 0.8;
    let white = vec3<f32>(0.95, 0.97, 1.0);
    // the figures on a dim ground, the colour field below.
    // the scene the walkers cross, dimmed (a filmed street at night's playback): about 0.4 of the wash.
    let scene = 0.3 + 0.2 * lm_noise(vec3<f32>(q.x * 0.05 - t * 0.02, q.y * 0.08, 4.1));
    let top = mix(ground * LM_LED_WASH_CD * scene, white * LM_LED_FIGURE_CD, lum);
    let base_led = mix(fl * LM_LED_WASH_CD, top, upper);
// sf:AAA_BUILDINGS+
    if (SF_AAA_DETAIL) {
        // Refined luminous twilight-sapphire / deep-cobalt / soft-cyan / subtle-gold animated SF
        // art tapestry on Salesforce Tower (`06a_night_skyline`): avoids blown-out neon-magenta
        // luma division and softens distant blocky walker silhouettes into fine glowing video art.
        let y_norm = clamp(y_rel / max(crown_m, 1.0), 0.0, 1.0);
        let wave1 = 0.5 + 0.5 * sin(u_m * 0.048 - t * 0.07 + y_norm * 3.8);
        let wave2 = 0.5 + 0.5 * sin(u_m * 0.092 + t * 0.05 - y_norm * 5.2 + 1.4);
        let c_cobalt = vec3<f32>(0.14, 0.34, 0.96);
        let c_saph = vec3<f32>(0.18, 0.54, 1.05);
        let c_cyan = vec3<f32>(0.24, 0.78, 0.98);
        let c_gold = vec3<f32>(1.04, 0.76, 0.36);
        var art = mix(c_cobalt, c_saph, lm_ss(0.15, 0.75, wave1));
        art = mix(art, c_cyan, lm_ss(0.45, 0.92, wave2 * 0.72 + (1.0 - y_norm) * 0.35) * 0.62);
        art = mix(art, c_gold, lm_ss(0.68, 0.98, wave1 * wave2) * (0.28 + 0.16 * (1.0 - y_norm)));
        // Cap luminance normalization denominator (>= 0.55) so deep sapphire/cobalt never clips into neon magenta.
        let art_n = art / max(dot(art, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.55);
        // Smooth subtle shimmer tapestry replacing blocky high-contrast white walker blobs at 1-3 km.
        let far_w = lm_ss(0.22, 0.75, fp);
        let soft_lum = lum * mix(0.68, 0.24, far_w);
        let shimmer = 0.78 + 0.22 * sin(u_m * 0.14 + y_norm * 9.0 - t * 0.12);
        let band_lo = art_n * (LM_LED_WASH_CD * 0.76 * shimmer);
        let fig_col = mix(vec3<f32>(0.78, 0.90, 1.04), vec3<f32>(1.06, 0.90, 0.66), wave1);
        let band_hi = mix(art_n * (LM_LED_WASH_CD * (0.44 + 0.26 * scene) * shimmer), fig_col * (LM_LED_FIGURE_CD * 0.56), soft_lum);
        return mix(band_lo, band_hi, upper);
    }
// sf:end
    return base_led;
}

// sf:AAA_BUILDINGS+
/// Multi-temperature SF night window color palette (`06a_night_skyline`): honors optional
/// `nightTemp` (`r5.z` in `[-1..+1]` from `skyline.rs`) and breaks monochrome pale-yellow
/// uniformity into authentic cool 5500K-6500K tech/glass towers, 4000K clean white corporate
/// offices, rich 2350K-2800K golden-amber residential/hotels, and subtle cyan/teal accents.
fn lm_aaa_night_col(bld_h: f32, suite_h: f32, curtain_w: f32, night_temp: f32) -> vec3<f32> {
    let c_ice = vec3<f32>(0.54, 0.84, 1.32) / 0.8105;   // 6500K crisp ice-blue LED office / tech
    let c_cool = vec3<f32>(0.76, 0.94, 1.18) / 0.9187;  // 5200K clean cool-white LED
    let c_neut = vec3<f32>(0.97, 0.98, 1.02) / 0.9807;  // 4000K neutral white commercial
    let c_warm = vec3<f32>(1.14, 0.74, 0.36) / 0.7975;  // 2900K warm office / lounge
    let c_amber = vec3<f32>(1.30, 0.56, 0.16) / 0.6884; // 2350K rich golden-amber residential
    let c_teal = vec3<f32>(0.36, 0.94, 1.16) / 0.8325;  // creative / lobby cyan-teal accent
    let is_tech = (night_temp < -0.25) || (abs(night_temp) <= 0.25 && (curtain_w > 0.52 || bld_h < 0.38));
    let is_res = (night_temp > 0.25) || (abs(night_temp) <= 0.25 && !is_tech && bld_h > 0.72);
    if (is_tech) {
        let sh = clamp(suite_h + min(night_temp, 0.0) * 0.18, 0.0, 1.0);
        if (sh < 0.34) { return c_ice; }
        if (sh < 0.60) { return c_cool; }
        if (sh < 0.78) { return c_neut; }
        if (sh < 0.92) { return c_warm; }
        if (sh < 0.97) { return c_amber; }
        return c_teal;
    }
    if (is_res) {
        let sh = clamp(suite_h - max(night_temp, 0.0) * 0.14, 0.0, 1.0);
        if (sh < 0.38) { return c_amber; }
        if (sh < 0.64) { return c_warm; }
        if (sh < 0.80) { return c_neut; }
        return c_ice;
    }
    // Classic corporate / stone / mixed towers: bold balance of cool white, clean white & rich gold.
    if (suite_h < 0.24) { return c_cool; }
    if (suite_h < 0.48) { return c_neut; }
    if (suite_h < 0.74) { return c_warm; }
    if (suite_h < 0.92) { return c_amber; }
    return c_ice;
}

/// Architectural night accent palette (`r5.w` 0..5 from `skyline.rs`, `#110`).
fn lm_aaa_night_accent(code: u32) -> vec3<f32> {
    if (code == 1u) { return vec3<f32>(1.30, 0.62, 0.18) / 0.7330; } // 2500K golden-amber uplight/arcade (Ferry belfry/arches)
    if (code == 2u) { return vec3<f32>(1.02, 0.98, 0.90) / 0.9827; } // 4800K champagne-white outline strip (Embarcadero 1-4)
    if (code == 3u) { return vec3<f32>(1.65, 0.14, 0.10) / 0.4581; } // maritime ruby-red pier beacon
    if (code == 4u) { return vec3<f32>(0.12, 1.38, 0.42) / 1.0428; } // maritime emerald-green pier beacon
    if (code == 5u) { return vec3<f32>(0.22, 0.68, 1.48) / 0.6401; } // luminous sapphire-cyan crown wash
    return vec3<f32>(1.04, 0.96, 0.82) / 0.9669;                     // 0u: warm-ivory clock dials (Ferry Building)
}
// sf:end

fn lm_authored4(s: ptr<function, Surface>, m: Material, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let i = min(u32(round(max(m.rough_flatten, 0.0))), max(LM_FACADE_COUNT, 1u) - 1u) * LM_FACADE_ROWS;
    let r0 = LM_FACADES[i];
    let r1 = LM_FACADES[i + 1u];
    let r2 = LM_FACADES[i + 2u];
    let r3 = LM_FACADES[i + 3u];
    let r4 = LM_FACADES[i + 4u];
    let r5 = LM_FACADES[i + 5u];
    let vertical = lm_ss(0.45, 0.25, abs(n.y));
    if (vertical <= 0.0) {
// sf:AAA_BUILDINGS+
        if (SF_AAA_DETAIL && n.y > 0.35) {
            lm_aaa_roof(s, world, n, dpx, dpy, vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173));
        }
// sf:end
        return;
    }
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let module_m = max(r0.x, 0.05);
    let storey_m = max(r0.y, 0.05);
    let u = dot(world.xz, t) / module_m;
    let v = (world.y - m.thin_absorption) / storey_m;
    let fu = (abs(dot(dpx.xz, t)) + abs(dot(dpy.xz, t))) / module_m;
    let fv = (abs(dpx.y) + abs(dpy.y)) / storey_m;
    let view = normalize(frame.cam_pos.xyz - world);
    let nh = normalize(n.xz + vec2<f32>(1e-5, 0.0));
    let vn = max(dot(view.xz, nh), 0.08);
    let depth = r2.w;
    let gx = max(r0.z - depth * abs(dot(view.xz, t)) / vn / module_m, 0.0);
    let gy = max(r0.w - depth * abs(view.y) / vn / storey_m, 0.0);
    let pu = mix(lm_pulse(u, fu, gx), gx, lm_ss(0.5, 1.0, fu));
    let pv = mix(lm_pulse(v, fv, gy), gy, lm_ss(0.5, 1.0, fv));
    let glass = pu * pv * vertical;
    let seed = vec2<f32>(m.params.z * 0.0131, m.params.w * 0.0173);
    let cell = floor(vec2<f32>(u, v));
    let frame_c = (*s).albedo;
    var frame_r = (*s).rough;
    // the piers' polished centre and honed rest (the piers centred on whole modules).
    var polished_share = 1.0;
    if (r4.z > 0.0) {
        let pw = (1.0 - r0.z) * r4.z;
        let pp = mix(lm_pulse(u + 0.5, fu, pw), pw, lm_ss(0.5, 1.0, fu)) / max(1.0 - pu, 1e-3);
        polished_share = clamp(pp, 0.0, 1.0);
    }
    let pier_c = frame_c * mix(r5.x, 1.0, polished_share);
    let pier_r = mix(select(frame_r, r5.y, r5.y > 0.0), select(frame_r, r4.w, r4.w > 0.0), polished_share);
    // piers, spandrels, glass by their shares of the pixel.
    let pier = (1.0 - pu) * vertical;
    let spandrel = pu * (1.0 - pv) * vertical;
    let rest = 1.0 - vertical;
// sf:AAA_BUILDINGS-
    (*s).albedo = frame_c * rest + pier_c * pier + frame_c * r4.x * spandrel + r1.xyz * glass;
    (*s).rough = frame_r * (rest + spandrel) + pier_r * pier + r1.w * glass;
    let polish = lm_ss(0.35, 0.2, (*s).rough);
    if (r2.x < 0.5) {
        (*s).metal = mix((*s).metal, r3.x, glass);
        lm_glaze = max(glass, polish * vertical);
        if (r3.y > 0.0) {
            let j = (vec2<f32>(st_hash(cell * vec2<f32>(1.37, 2.11) + seed + 3.3), st_hash(cell * vec2<f32>(2.71, 1.13) + seed + 8.1)) * 2.0 - 1.0) * r3.y;
            let far_k = lm_ss(0.3, 1.0, max(fu, fv));
            let tt = vec3<f32>(t.x, 0.0, t.y);
            let nj = normalize((*s).n + (tt * j.x + vec3<f32>(0.0, j.y, 0.0)) * (1.0 - far_k));
            (*s).n = normalize(mix((*s).n, nj, glass));
            let r = (*s).rough;
            (*s).rough = mix(r, sqrt(r * r + 4.0 * r3.y * far_k), glass);
        }
    }
// sf:end
// sf:AAA_BUILDINGS+
    if (SF_AAA_BRDF && r2.x < 0.5) {
        // True dual-lobe facade response on hero landmark towers & Beaux-Arts waterfront buildings:
        // optically razor-sharp mirror glass (`0.016-0.044`), distinct pane-by-pane bow/tilt faceting
        // (`±1.5 deg`), parallax-shifted interior floor slabs & blinds, anodized metallic mullions,
        // and deep 0.26-0.36 m recessed 3D window jamb/soffit/sill normal & AO catches.
        let op_w = max(rest + pier + spandrel, 1e-4);
        (*s).albedo = (frame_c * rest + pier_c * pier + frame_c * r4.x * spandrel) / op_w;
        (*s).rough = (frame_r * (rest + spandrel) + pier_r * pier) / op_w;
        let polish = lm_ss(0.35, 0.2, (*s).rough);
        lm_glaze = polish * vertical;
        let curtain_w = lm_ss(0.35, 0.68, r0.z * r0.w);
        if (SF_AAA_DETAIL && curtain_w < 0.58) {
            lm_aaa_masonry(s, world, n, dpx, dpy, seed, storey_m, false);
        }
        if (curtain_w > 0.42) {
            (*s).rough = mix((*s).rough, 0.22, curtain_w * 0.85);
            (*s).metal = mix((*s).metal, 0.78, curtain_w * 0.85);
        }
        // Recessed 3D window reveal jambs, top shadowed soffit & sunlit bottom sill normal catches
        // (breaks flat 2D painted rectangles on stone/precast/Beaux-Arts landmark towers!).
        let far_k = lm_ss(0.40, 1.35, max(fu, fv));
        let eff_d = max(depth, mix(0.32, 0.14, curtain_w));
        let tt = vec3<f32>(t.x, 0.0, t.y);
        if (SF_AAA_DETAIL && far_k < 0.98 && eff_d > 0.05) {
            let vt = dot(view.xz, t);
            let gx_r = max(r0.z - eff_d * abs(vt) / vn / module_m, 0.0);
            let gy_r = max(r0.w - eff_d * abs(view.y) / vn / storey_m, 0.0);
            let side_rev = clamp((r0.z - gx_r) * pv * (1.0 - far_k) * vertical, 0.0, 1.0);
            let vert_rev = clamp((r0.w - gy_r) * pu * (1.0 - far_k) * vertical, 0.0, 1.0);
            // Only tilt normal on visible jamb/soffit/sill band near window border (not solid pier centers).
            let u_edge = abs(fract(u) - 0.5);
            let v_edge = abs(fract(v) - 0.5);
            let near_side = (1.0 - lm_ss(0.5 * r0.z, 0.5 * r0.z + 0.14, u_edge)) * pv * (1.0 - pu);
            let near_vert = (1.0 - lm_ss(0.5 * r0.w, 0.5 * r0.w + 0.14, v_edge)) * pu * (1.0 - pv);
            let jamb_s = select(-1.0, 1.0, vt > 0.0);
            let sill_s = select(-1.0, 1.0, view.y > 0.0);
            let rev_w = clamp(near_side * (0.45 + 0.55 * side_rev) + near_vert * (0.45 + 0.55 * vert_rev), 0.0, 1.0) * (1.0 - far_k);
            if (rev_w > 0.01) {
                (*s).n = normalize((*s).n + tt * (jamb_s * 0.62 * near_side * (1.0 - far_k)) + vec3<f32>(0.0, sill_s * 0.58 * near_vert * (1.0 - far_k), 0.0));
                (*s).ao = clamp((*s).ao * mix(1.0, select(0.52, 0.82, view.y > 0.0), rev_w * 0.65), 0.18, 1.0);
            }
        }
        let tilt_amp = max(r3.y, mix(0.016, 0.028, curtain_w));
        let j = (vec2<f32>(st_hash(cell * vec2<f32>(1.37, 2.11) + seed + 3.3), st_hash(cell * vec2<f32>(2.71, 1.13) + seed + 8.1)) * 2.0 - 1.0) * (tilt_amp * (1.0 - far_k));
        let wave_a = vec2<f32>(u * 0.22 + seed.x * 5.0, v * 0.30);
        let jm = (vec2<f32>(st_noise(wave_a), st_noise(wave_a + vec2<f32>(7.1, 2.9))) * 2.0 - 1.0) * mix(0.005, 0.012, curtain_w);
        let gn = normalize(n + tt * (j.x + jm.x) + vec3<f32>(0.0, j.y + jm.y + 0.044 * curtain_w, 0.0));
        let g_lum = max(dot(r1.xyz, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-3);
        let g_hue = mix(vec3<f32>(1.0), clamp(r1.xyz / g_lum, vec3<f32>(0.66), vec3<f32>(1.30)), 0.58);
        let raw_f0 = mix(vec3<f32>(0.04), r1.xyz, r3.x);
        let min_f0 = mix(0.13, 0.32, curtain_w) * g_hue;
        let mac_batch = (st_noise(wave_a * 1.35 + vec2<f32>(3.3, 1.9)) * 2.0 - 1.0) * 0.095;
        let pane_f0 = clamp(max(raw_f0 * mix(1.12, 1.65, curtain_w), min_f0), vec3<f32>(0.09), vec3<f32>(0.46));
        let suite_i = floor(v / 3.0);
        let suite_v = st_hash(vec2<f32>(suite_i * 1.93, 4.7) + seed);
        // Parallax-shifted interior floor slab dark band & varied-height translucent roller blinds.
        let vy_int = fract(v + clamp(view.y / vn * 0.22, -0.32, 0.32));
        let slab_d = 1.0 - 0.75 * (1.0 - lm_ss(0.07, 0.16, vy_int) * lm_ss(0.93, 0.82, vy_int)) * (1.0 - far_k);
        let blind_h = mix(0.24, 0.66, st_hash(floor(vec2<f32>(u * 0.35, v)) + seed + 6.7));
        let blind_on = step(0.54, st_hash(floor(vec2<f32>(u * 0.35, v)) + seed + 2.1)) * step(1.0 - blind_h, vy_int) * (1.0 - far_k);
        let base_room = r1.xyz * (1.0 - r3.x) * mix(0.12, 0.32, suite_v) * mix(1.0, 0.44, curtain_w) * slab_d;
        let pane_room = mix(base_room, vec3<f32>(0.25, 0.25, 0.26), blind_on * 0.38);
        aaa_bldg_on = true;
        aaa_glass_cov = glass;
        aaa_glass_rough = clamp(sqrt(min(r1.w, 0.036) * min(r1.w, 0.036) + 0.14 * tilt_amp * tilt_amp * far_k), 0.016, 0.046);
        aaa_glass_f0 = clamp(pane_f0 * (1.0 + (st_hash(cell * vec2<f32>(3.1, 1.7) + seed) - 0.5) * 0.16 * (1.0 - far_k) + mac_batch + (suite_v - 0.5) * 0.12), vec3<f32>(0.08), vec3<f32>(0.48));
        aaa_glass_n = gn;
        aaa_glass_room = pane_room;
    } else {
        (*s).albedo = frame_c * rest + pier_c * pier + frame_c * r4.x * spandrel + r1.xyz * glass;
        (*s).rough = frame_r * (rest + spandrel) + pier_r * pier + r1.w * glass;
        let polish = lm_ss(0.35, 0.2, (*s).rough);
        if (r2.x < 0.5) {
            (*s).metal = mix((*s).metal, r3.x, glass);
            lm_glaze = max(glass, polish * vertical);
        }
        if (SF_AAA_DETAIL) {
            lm_aaa_masonry(s, world, n, dpx, dpy, seed, storey_m, false);
        }
        aaa_bldg_on = true;
        aaa_glass_cov = 0.0;
    }
// sf:end
    // the frame's depth shading the glass and its reveals from the sun.
    let eff_sh_depth = select(depth, max(depth, mix(0.32, 0.14, lm_ss(0.35, 0.68, r0.z * r0.w))), SF_AAA_DETAIL);
    if ((r4.y > 0.5 || SF_AAA_DETAIL) && eff_sh_depth > 0.0) {
        let l = look.key_dir.xyz;
        let ln = dot(l.xz, nh);
        if (ln > 0.02) {
            let lt = dot(l.xz, t) / ln;
            let lv = max(l.y, 0.0) / ln;
            let pane_w = max(r0.z * module_m, 0.05);
            let pane_h = max(r0.w * storey_m, 0.05);
            let su = clamp(eff_sh_depth * abs(lt) / pane_w, 0.0, 1.0);
            let sv = clamp(eff_sh_depth * lv / pane_h, 0.0, 1.0);
            let glass_shade = 1.0 - (1.0 - su) * (1.0 - sv);
            // the reveals the view shows: the side one lit only from the sun's side, the soffit never.
            let vt = dot(view.xz, t);
            let gx_sh = max(r0.z - eff_sh_depth * abs(vt) / vn / module_m, 0.0);
            let gy_sh = max(r0.w - eff_sh_depth * abs(view.y) / vn / storey_m, 0.0);
            let side = max(r0.z - gx_sh, 0.0) * pv;
            let side_dark = select(1.0, 0.0, vt * dot(l.xz, t) > 0.0);
            let head = max(r0.w - gy_sh, 0.0) * pu;
            let head_dark = select(0.0, 1.0, view.y < 0.0);
            let sh_scale = select(1.0, 0.85, r4.y <= 0.5);
            lm_unlit = clamp((glass * glass_shade + (side * side_dark + head * head_dark) * vertical) * sh_scale, 0.0, 1.0);
        }
    }
    let dusk = look.rooms_env.x;
    if (dusk <= 0.0 || !sky_on(skyu.flags.x, SKY_LIGHTS) || frame.copy_step.z > 0.5) {
        return;
    }
    let level = skyu.bloom_w2.z * dusk;
    var in_far_crown = false;
    if (r2.z > 1.5) {
        // Salesforce Tower Role::Far (originY = base): Day for Night LED crown above 277.98 m.
        let y_crown = (world.y - m.thin_absorption) - 277.98;
        if (y_crown >= 0.0) {
            in_far_crown = true;
            let fp = max(fu * module_m, fv * storey_m);
            rm_emit = rm_emit + lm_led4(u * module_m, y_crown, LM_LED_CROWN_M, fp) * (glass + 0.6 * (1.0 - glass)) * level * vertical;
        }
    } else if (r2.z > 0.5) {
        let fp = max(fu * module_m, fv * storey_m);
        // through the panels' perforation (the glass share here), the panels' faces lit 0.6 as much (the light reflected out through them).
        rm_emit = rm_emit + lm_led4(u * module_m, world.y - m.thin_absorption, LM_LED_CROWN_M, fp) * (glass + 0.6 * (1.0 - glass)) * level * vertical;
    }
    if (r2.y > 0.0 && !in_far_crown) {
        let bay = max(r3.z, 1.0);
        let bc = floor(vec2<f32>(u / bay, v));
        let occ = clamp(look.rooms_env.y * r2.y, 0.0, 1.0);
        let fh = st_hash(vec2<f32>(bc.y * 3.17, 7.9) + seed);
        let floor_share = clamp(occ + (fh - 0.5) * 1.6, 0.0, 1.0);
        // tenants' zones of one to three bays.
        let run_len = 1.0 + floor(st_hash(vec2<f32>(bc.y * 1.91, 2.3) + seed) * 3.0);
        let run = floor((bc.x + 13.0 * st_hash(vec2<f32>(bc.y, 4.4) + seed)) / run_len);
        let lit = step(st_hash(vec2<f32>(run * 1.37, bc.y * 2.11) + seed), floor_share);
        let dim = (0.7 + 0.3 * st_hash(bc + 5.1 + seed)) * mix(1.0, 0.35, step(0.86, st_hash(bc + 11.7 + seed)));
        let cool = step(1.0 - look.rooms_env.w, st_hash(vec2<f32>(run, bc.y) + 17.3 + seed));
        let warm_c = vec3<f32>(1.0, 0.62, 0.34) / 0.6779;
        let cool_c = vec3<f32>(0.86, 0.93, 1.0) / 0.9200;
        let col = select(warm_c, cool_c, cool > 0.5);
// sf:AAA_BUILDINGS-
        let k = mix(lit * dim, occ * 0.78, lm_ss(0.25, 0.6, max(fu / bay, fv)));
        rm_emit = rm_emit + col * k * glass * level * max(r3.w, 0.1);
// sf:end
// sf:AAA_BUILDINGS+
        if (SF_AAA_DETAIL) {
            // Architectural screen/beacon/crown accents (`r2.x >= 0.5` or `r5.w >= 0.5`, `#110`):
            // emit clean continuous architectural illumination (Ferry dials/arches, Embarcadero
            // outline strips, maritime pier beacons, glowing glass tower crowns).
            let accent_code = u32(round(max(r5.w, 0.0)));
            if (r2.x >= 0.5 || accent_code > 0u) {
                let col_acc = lm_aaa_night_accent(accent_code);
                let acc_gain = select(1.0, 1.45, accent_code == 1u);
                rm_emit = rm_emit + col_acc * glass * level * max(r3.w, 0.1) * acc_gain;
            } else {
                // Multi-scale irregular office/residential tenant suites surviving at 1.5-4 km:
                // never collapses to a uniform per-floor neon blind at Bay Bridge distance.
                let face_s = seed + vec2<f32>(floor(atan2(n.z, n.x) * 2.546 + 0.5) * 1.31, 0.0);
                let wing_m = mix(14.0, 26.0, st_hash(vec2<f32>(bc.y * 2.31, 6.1) + face_s));
                let u_m = u * module_m;
                let fu_m = fu * module_m;
                let wing = floor(u_m / wing_m + st_hash(vec2<f32>(bc.y * 1.73, 9.7) + face_s) * 5.0);
                let wing_h = st_hash(vec2<f32>(wing * 1.49, bc.y * 2.77) + face_s);
                let f_act = smoothstep(0.18, 0.88, floor_share);
                let wing_lit = step(wing_h, clamp(f_act * 0.62 + 0.06, 0.04, 0.68));
                let sub_lit = wing_lit * step(st_hash(vec2<f32>(run * 1.37, bc.y * 2.11) + face_s), 0.76)
                    + (1.0 - wing_lit) * step(st_hash(vec2<f32>(run * 1.37, bc.y * 2.11) + face_s), 0.055 * f_act + 0.015);
                let b_dim = mix(0.32, 1.08, st_hash(bc + 5.1 + face_s) * (0.5 + 0.5 * st_hash(vec2<f32>(wing, bc.y) + 12.3 + face_s)))
                    * mix(1.0, 0.34, step(0.78, st_hash(bc + 11.7 + face_s)));
                let a_bay = lm_ss(0.25, 0.65, fu / bay);
                let a_run = lm_ss(0.35, 0.90, fu / (bay * run_len));
                let a_wing = lm_ss(0.75, 1.55, fu_m / wing_m);
                let a_fl = lm_ss(0.65, 1.35, fv);
                let v_run = sub_lit * 0.74;
                let v_wing = wing_lit * 0.56 + (1.0 - wing_lit) * (0.04 * f_act + 0.012);
                let v_fl = floor_share * 0.36 + 0.015;
                var ka = mix(mix(mix(sub_lit * b_dim, v_run, a_bay), v_wing, a_run), v_fl, a_wing);
                ka = mix(ka, occ * 0.38, a_fl);
                let bld_h = st_hash(seed * vec2<f32>(3.17, 5.71) + vec2<f32>(1.9, 4.3));
                let suite_h = mix(st_hash(vec2<f32>(run, bc.y) + 17.3 + face_s), st_hash(vec2<f32>(wing, bc.y) + 21.7 + face_s), a_run);
                let cw_n = lm_ss(0.35, 0.68, r0.z * r0.w);
                let col_a = lm_aaa_night_col(bld_h, suite_h, cw_n, r5.z);
                rm_emit = rm_emit + col_a * ka * glass * level * max(r3.w, 0.1) * 0.92;
            }
        } else {
            let k = mix(lit * dim, occ * 0.78, lm_ss(0.25, 0.6, max(fu / bay, fv)));
            rm_emit = rm_emit + col * k * glass * level * max(r3.w, 0.1);
        }
// sf:end
    }
}

// `lm_flood3`: a floodlit material lit by its projector rig (`base`: first
// projector, the rig's origin y, count; LM_PROJ three rows a projector:
// offset from (anchor x, origin y, anchor z) and axis cd, aim and cos of
// the field angle, cos of the beam angle, spill share, interior flag). Each
// projector gives E = I(theta) N.L / d^2 lux, I full inside the beam,
// smoothstepped to none at the field, plus its spill over the forward
// hemisphere; the lit surface's radiance is E (albedo (1 - metal) / pi +
// GGX), in the floodlights' colour (3,500 K), in the night lighting's units
// (lamp_k a lux), times the scene AO. An interior projector lights a
// translucent skin from inside: |N.L|, E albedo / pi.
fn lm_flood3(s: ptr<function, Surface>, m: Material, world: vec3<f32>) {
    if ((m.flags & LM_F_FLOOD) == 0u) {
        return;
    }
    let dusk = look.rooms_env.x;
    if (dusk <= 0.0 || !sky_on(skyu.flags.x, SKY_LIGHTS) || frame.copy_step.z > 0.5) {
        return;
    }
    let first = u32(max(m.base.x, 0.0) + 0.5);
    let count = u32(max(m.base.z, 0.0) + 0.5);
    let origin = vec3<f32>(m.params.z, m.base.y, m.params.w);
    let nn = (*s).n;
    let v = normalize(frame.cam_pos.xyz - world);
    let diffuse = (*s).albedo * (1.0 - (*s).metal);
    let f0 = mix(vec3<f32>(0.04), (*s).albedo, (*s).metal);
    let rough = clamp((*s).rough, 0.05, 1.0);
    var lo = vec3<f32>(0.0);
    for (var k = first; k < min(first + count, LM_PROJ_COUNT); k = k + 1u) {
        let a = LM_PROJ[3u * k];
        let b = LM_PROJ[3u * k + 1u];
        let c = LM_PROJ[3u * k + 2u];
        let d = origin + a.xyz - world;
        let d2 = max(dot(d, d), 0.25);
        let l = d * inverseSqrt(d2);
        let ct = dot(-l, b.xyz);
        let beam = lm_ss(b.w, c.x, ct) + c.y * step(0.0, ct);
        if (beam <= 0.0) {
            continue;
        }
        let e = a.w * beam / d2;
        if (c.z > 0.5) {
            lo = lo + e * abs(dot(nn, l)) * (*s).albedo / LK_PI;
            continue;
        }
        let nl = dot(nn, l);
        if (nl <= 0.0) {
            continue;
        }
        lo = lo + e * nl * (diffuse / LK_PI + lk_ggx(nn, v, l, f0, rough));
    }
    let ao = ao_multi_bounce(ao_fetch(world).a);
    rm_emit = rm_emit + lo * ao * LM_TOWER_FLOOD_RGB * skyu.bloom_w2.z * dusk;
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

// Submerged grains have a mineral-water boundary. The air-facing specular
// film on the wet beach belongs on the free water surface once the tide covers it.
fn lm_cove_sediment(s: ptr<function, Surface>, wp: vec3<f32>, normal: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>) {
    let depth = LM_SEA_LEVEL_Y - wp.y;
    let cover = lk_cove_weight(wp.xz * look.cove.x) * smoothstep(-0.035, 0.08, depth);
    if (cover <= 0.0) { return; }
    let p = wp.xz;
    let footprint = max(length(dx.xz), length(dy.xz));
    let deposit = st_noise(p * 0.72);
    let strand = st_noise(p * 1.8 + vec2<f32>(deposit, 0.0));
    let ripple = dot(p, vec2<f32>(2.3, 16.0)) + 3.0 * deposit + strand * 1.6;
    let crest = sin(ripple) * (1.0 - smoothstep(0.035, 0.16, footprint));
    let fine = (st_noise(p * 68.0) - 0.5) * (1.0 - smoothstep(0.004, 0.018, footprint));
    // Fines settle in the low, sheltered hollows between sand ridges.
    let silt = smoothstep(0.05, 1.45, depth + 0.54 * (deposit - 0.5) - 0.18 * crest);
    let pocket = smoothstep(0.46, 0.79, deposit * 0.58 + strand * 0.42);
    let sand = mix(vec3<f32>(0.175, 0.142, 0.099), vec3<f32>(0.087, 0.080, 0.060), silt);
    (*s).albedo = mix((*s).albedo, sand * (1.04 - 0.31 * pocket + fine * 0.20 + crest * 0.055), cover);
    (*s).rough = mix((*s).rough, 0.82 + 0.12 * silt, cover);
    let slope = cos(ripple) * 0.065 * (1.0 - silt) * (1.0 - smoothstep(0.035, 0.16, footprint));
    (*s).n = normalize(mix((*s).n, normal + vec3<f32>(-0.14 * slope, 0.0, -slope), cover));
    if (cover > 0.5) { (*s).env = ENV_SUBMERGED; }
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
// painting on an on-going basis" where inspections find corrosion; salt-laden
// fog attacks the film; rust "attacked the rivet heads and surfaces" from the
// bases of the towers; failures start under loosened mill scale. Acrylic
// topcoats chalk under sunlight (the degree rated by ASTM D4214). INFERRED,
// every form and amount: touch-up patches over a few plates in fresher,
// glossier, more saturated paint (`LM_WX_TOUCH` of the plate groups); the rest
// chalked by its own age, more on faces turned up; rust bloom along the rivet
// rows beside the plate seams, densest at the towers' feet and in the splash
// zone; dirt on the ledges (faces turned up); run-off streaks under the legs'
// set-backs and under the deck's edge. Each is filtered by the pixel's
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
    // rust bloom on the rivet rows (6 cm either side of the horizontal seams), as stain bands 5 cm high.
    let fy = abs(du.y) + abs(dv.y);
    let fx = abs(du.x) + abs(dv.x);
    let rows = lm_lines(uv.y - 0.06, fy, LM_PS_PLATE_V, 0.05) + lm_lines(uv.y + 0.06, fy, LM_PS_PLATE_V, 0.05);
    let foot = 1.0 - lm_ss(LM_SEA_LEVEL_Y + 6.0, LM_SEA_LEVEL_Y + 40.0, world.y);
    let p_rust = mix(LM_WX_RUST_P.x, LM_WX_RUST_P.y, foot) * (1.0 - touch);
    let rivet = floor(uv.x / LM_PS_RIVET_PITCH);
    let is_on = select(0.0, 1.0, lm_cell_hash(vec3<f32>(rivet, floor(uv.y / LM_PS_PLATE_V * 2.0 + 0.5), fid + 5.0)) < p_rust);
    let spot = mix(p_rust, is_on, lm_fade(fx, LM_PS_RIVET_PITCH));
    c = mix(c, LM_WX_RUST_RGB, clamp(rows * spot * 0.85 * vertical, 0.0, 1.0));
    // dirt on the ledges.
    let ledge = LM_WX_LEDGE * lm_ss(0.55, 0.95, up) * (0.65 + 0.35 * lm_noise(local / 1.7 + vec3<f32>(2.2, 0.0, 9.1)));
    c = mix(c, c * 0.5 + vec3<f32>(0.012, 0.010, 0.008), ledge);
    // run-off under the ledge above: a leg's set-back top (in a tower's plan box) or the deck's edge.
    let drop = lm_gg_drop(world);
    let fr = lm_fade(footprint, 1.2);
    let stripes = mix(0.5, lm_ss(-0.1, 0.6, lm_noise(vec3<f32>(uv.x * 0.9, world.y * 0.025, 6.3))), fr);
    let run = LM_WX_RUN_TONE * exp(-drop / LM_WX_RUN_M) * stripes * vertical;
    c = c * (1.0 - run);
    (*s).albedo = c;
}

// The height of a Golden Gate steel point under the ledge above it: a leg's
// set-back top (in a tower's plan box) or the deck's edge (1e4 under neither).
fn lm_gg_drop(world: vec3<f32>) -> f32 {
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
    return drop;
}

// The concrete's age (`BridgeFeatures::concrete2`): rough-board-formed
// (Strauss 1937) and cast in lifts, each lift's own tone (each pour's mix and
// its curing), lime leached at the lift joints and run down the face as pale
// streaks (efflorescence), and dark weathering streaks down the faces where
// the run-off from the tops carries dirt and the salt fog's growth.
// INFERRED: the lift height (`LM_CO_LIFT_M`, the export's), the tones and lengths.
// The cited albedo (golden_gate.rs `CONCRETE2_LUMINANCE`) is already a
// weathered concrete's, so every term here is zero-mean: each stain divides
// by its own mean (the value noise's, measured over 600,000 samples:
// `LM_CO_MEAN_*`), and so does the round-2 streak's darkening the caller laid
// down (its mean |noise| x `LM_CO_STREAK_TONE`); a face's mean albedo, near
// or far, is the cited one (the wet band at the tide stays the tide's).
const LM_CO_MEAN_DIRT: f32 = 0.286; // E[ss(-0.2, 0.7, noise)]
const LM_CO_MEAN_BROAD: f32 = 0.295; // E[ss(-0.3, 0.8, noise)]
const LM_CO_MEAN_LEACH: f32 = 0.295; // E[ss(0.35, 0.9, noise / 2 + 1 / 2)]
const LM_CO_MEAN_ABS_NOISE: f32 = 0.303; // E[|noise|]
fn lm_concrete_age(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, uv: vec2<f32>, du: vec2<f32>, dv: vec2<f32>, footprint: f32, vertical: f32) {
    let lift = floor(world.y / LM_CO_LIFT_M);
    let fid = floor(dot(n, vec3<f32>(1.0, 3.0, 7.0)) * 2.0 + 0.5);
    let fl = lm_fade(footprint, LM_CO_LIFT_M);
    let lift_tone = mix(0.0, (lm_cell_hash(vec3<f32>(lift, fid, 23.0)) - 0.5) * 0.10, fl) * vertical;
    let below = mix(0.25, pow(fract(world.y / LM_CO_LIFT_M), 3.0), fl); // 1 just under a joint, 0 at the next; E = 1/4
    let fs = lm_fade(footprint, 0.6);
    let leach_k = 0.10 * vertical;
    let leach = (mix(LM_CO_MEAN_LEACH, lm_ss(0.35, 0.9, lm_noise(vec3<f32>(uv.x * 2.1, lift * 1.7, 12.5)) * 0.5 + 0.5), fs) * below - LM_CO_MEAN_LEACH * 0.25) * leach_k;
    let dirt_k = 0.22 * vertical;
    let dirt = mix(LM_CO_MEAN_DIRT, lm_ss(-0.2, 0.7, lm_noise(vec3<f32>(uv.x * 0.7, world.y * 0.018, 31.0))), lm_fade(footprint, 1.5)) * dirt_k;
    // the same weathering at the wall's scale (bands some 5-10 m across, faded with height over 40 m), so a
    // housing's wall seen from hundreds of metres keeps its staining where the fine streaks have gone to their mean.
    let broad_k = 0.16 * vertical;
    let broad = mix(LM_CO_MEAN_BROAD, lm_ss(-0.3, 0.8, lm_noise(vec3<f32>(uv.x * 0.12, world.y * 0.012, 47.0))), lm_fade(footprint, 8.0)) * broad_k;
    let norm = (1.0 - LM_CO_MEAN_DIRT * dirt_k) * (1.0 - LM_CO_MEAN_BROAD * broad_k) * (1.0 - LM_CO_MEAN_ABS_NOISE * LM_CO_STREAK_TONE * vertical);
    let c = (*s).albedo;
    (*s).albedo = max(c * (1.0 + lift_tone) * (1.0 - dirt) * (1.0 - broad) / norm + vec3<f32>(0.05, 0.05, 0.048) * leach, vec3<f32>(0.0));
}

// The share of a face's sky that two parallel fins `d` deep, standing `x0`
// and `x1` to either side of the point and as tall as the face, take
// (Howell's differential element to a perpendicular semi-infinite plate,
// each 0.5 (1 - x / sqrt(x^2 + d^2))).
fn lm_fin_occ(x: f32, d: f32) -> f32 {
    let xx = max(x, 0.0);
    return 0.5 * (1.0 - xx / sqrt(xx * xx + d * d));
}

// The light through a recess: a point `t` across a channel [a, b] on a face
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
// shadow maps (`BridgeFeatures::leg_cells`): on a leg's recessed panel the
// panel's two pilasters; on a portal casing's face the legs standing proud of
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
                // the ribs (centred on the casing; laid as golden_gate.rs `portal_ribs`).
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
        return vec2<f32>(1.0); // a pilaster's face, a wall of a recess, or not on the leg
    }
    let run = lm_leg_run(tf, hw);
    if (run.z < 0.5) {
        return vec2<f32>(1.0);
    }
    let key = lm_channel_key(tf, run.x, run.y, LM_GG_RECESS_M, ln, lt, ws);
    let occ = lm_fin_occ(tf - run.x, LM_GG_RECESS_M) + lm_fin_occ(run.y - tf, LM_GG_RECESS_M);
    return vec2<f32>(key, clamp(1.0 - occ, 0.0, 1.0));
}

// The radiance probe (EO_BRIDGE_PROBE = a mask; a debugging view, never on
// by default): a Golden Gate painted surface keeps only the terms its mask
// names, recomputed as look_light computes them (1 the sun's diffuse and
// rim, 2 the sun's specular, 4 the sky's diffuse (hemisphere, ambient,
// environment), 8 the sky's specular, 16 the sea's bounce, 32 the scene's
// bounce), the rest of the frame unchanged (the atmosphere still laid over
// it). Mask 63 redraws the surface as it is. look_light's own terms are
// zeroed (no albedo, no occlusion, no key or rim) and the kept ones ride on
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
    // the sea's bounce already sits on the emission slot (lm_surface): take it off unless kept.
    if ((mask & 16u) == 0u) {
        rm_emit = rm_emit - lm_bridge_bounce(world, n, diffuse, sf.ao);
    }
    rm_emit = rm_emit + out;
    (*s).albedo = vec3<f32>(0.0);
    (*s).ao = 0.0;
    lm_key_vis = 0.0;
    lm_rim_vis = 0.0;
}

// ---- the sixth pass: the Golden Gate's aging (eo-landmarks golden_gate/aging.rs,
// where every value's source is given; `BridgeFeatures::matte` .. `concrete_age`,
// `--no-gg-aging`). Every pattern is anchored in the world and filtered by the
// pixel's footprint to its mean before it could alias (`lm_fade`), so a member
// far off takes the mean, not noise; the sizes are a few metres, to read at
// 300-800 m. ----

// The means of the value noise's smoothsteps the patterns use (2,000,000 samples).
const LM_AGE_MEAN_SS_0_06: f32 = 0.2351; // E[ss(0.0, 0.6, noise)]
const LM_AGE_MEAN_SS_03_08: f32 = 0.0823; // E[ss(0.3, 0.8, noise)]

// Where a Golden Gate steel point stands under the roadway (`underside`):
// (x) how far it is inside the truss or sheltered under the deck (1 inside
// the two truss planes, on a plane's inner face or turned down under the
// deck; 0 in the open); (y) the sun's share the near truss plane's lattice
// lets through to it, where the ray to the sun leaves the truss through a
// side plane under the deck (the deck itself is `lm_deck_key`'s); (z) the
// sky's share the two planes leave it. The towers' legs, which pass through
// the deck, are in the open.
// The deck's centre line's offset across the straight frame at chainage c (the approaches' curve).
fn lm_gg_offset(c: f32) -> f32 {
    let x = clamp((c - LM_GG_OFFSET_C.x) / max(LM_GG_OFFSET_C.y - LM_GG_OFFSET_C.x, 1e-3), 0.0, 1.0) * 95.0;
    let i = min(u32(floor(x)), 94u);
    var t = LM_GG_OFFSET;
    return mix(t[i], t[i + 1u], x - f32(i));
}

fn lm_gg_under(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let open = vec3<f32>(0.0, 1.0, 1.0);
    let across = vec2<f32>(-LM_GG_AXIS.y, LM_GG_AXIS.x);
    let rel = p.xz - LM_GG_ORIGIN;
    let c = dot(rel, LM_GG_AXIS);
    let u = dot(rel, across) - lm_gg_offset(c);
    // (the members built off the deck's edge, the arch's ribs and the bents' columns, stand a few metres out)
    if (c < LM_GG_DECK[0] || c > LM_GG_DECK[5] || abs(u) > max(LM_GG_DECK[7], LM_GG_HP) + 4.0) {
        return open;
    }
    let deck = lm_gg_deck_top(c) - 0.3;
    if (deck - p.y < 0.3) {
        return open;
    }
    // (a leg's widest stage, its base, and the struts between the legs)
    let t = lm_gg_tower_local(p);
    let leg = max(LM_GG_STAGES[0].zw, vec2<f32>(LM_GG_HALF_ACROSS, LM_GG_HALF_ALONG)) + 0.5;
    if (abs(t.x) < leg.y && abs(t.y) < LM_GG_HP + leg.x) {
        return open;
    }
    let hp = LM_GG_HP;
    let inward = -sign(u) * dot(n.xz, across);
    let inside = 1.0 - lm_ss(hp - 0.9, hp - 0.2, abs(u));
    let inner_face = select(0.0, 1.0, inward > 0.3 && abs(u) < hp + 1.0);
    let down = lm_ss(-0.2, -0.7, n.y);
    // the outer faces under the deck's edge take part of it (the roadway's run-off drains over them).
    let shelter = max(max(max(inside, inner_face), down), LM_AGE_UNDER_OUTER);
    // the sun through a side plane.
    let l = look.key_dir.xyz;
    let lu = dot(l.xz, across);
    var key = 1.0;
    if (abs(lu) > 1e-3 && l.y > 0.02) {
        let tt = (sign(lu) * hp - u) / lu;
        if (tt * abs(lu) > 0.4 && p.y + l.y * tt < deck) {
            key = LM_AGE_LATTICE_T;
        }
    }
    let sky = mix(1.0, LM_AGE_UNDER_SKY, max(inside, inner_face));
    return vec3<f32>(shelter, key, sky);
}

// The Golden Gate coat's age (`fade`, `grime`, `underside`): the chalk by the
// coat's age and exposure, the age's wander over sections and its repainted
// stretches, the run-off streaks, the salt at the towers' feet and the dirt
// under the deck. `shelter` is lm_gg_under's (x).
fn lm_gg_age_paint(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, uv: vec2<f32>, footprint: f32, vertical: f32, shelter_in: f32) {
    var c = (*s).albedo;
    let fid = floor(dot(n, vec3<f32>(1.0, 3.0, 7.0)) * 2.0 + 0.5);
    var age = 0.5;
    var tone = 0.0;
    if (LM_AGE_GRIME_ON) {
        // the age's wander, a section a few metres across; a stretch repainted (fresh, darker, fuller).
        let fa = lm_fade(footprint, LM_AGE_PATCH_M.x * 2.0);
        let na = lm_noise(vec3<f32>(uv.x / LM_AGE_PATCH_M.x, uv.y / LM_AGE_PATCH_M.y, fid * 1.37 + 3.1));
        age = 0.5 + 0.5 * clamp(na / 0.6, -1.0, 1.0) * fa;
        tone = lm_noise(vec3<f32>(uv.x / (LM_AGE_PATCH_M.x * 1.6), uv.y / (LM_AGE_PATCH_M.y * 0.7), fid * 2.11 + 9.7)) / 0.6 * LM_AGE_TONE * fa;
        let cell = floor(uv / LM_AGE_REPAINT_M);
        let fresh = select(0.0, 1.0, lm_cell_hash(vec3<f32>(cell.x, cell.y, fid + 71.0)) < LM_AGE_REPAINT_SHARE);
        let fr = mix(LM_AGE_REPAINT_SHARE, fresh, lm_fade(footprint, LM_AGE_REPAINT_M.x));
        age = mix(age, 0.05, fr);
        tone = tone + fr * LM_AGE_REPAINT_TONE;
    }
    if (LM_AGE_FADE_ON) {
        // chalk: most on faces in the open and turned up, least where sheltered; by the coat's age.
        let shelter = max(shelter_in, lm_ss(-0.2, -0.7, n.y));
        let expo = mix(1.0, LM_AGE_CHALK_SHELTER, shelter) * (0.85 + 0.3 * max(n.y, 0.0));
        let k = LM_AGE_CHALK * expo * (0.3 + 1.4 * age);
        c = mix(c, LM_AGE_CHALK_RGB, clamp(k, 0.0, 1.0));
    }
    if (LM_AGE_GRIME_ON) {
        c = c * (1.0 + tone);
        // run-off streaks down the vertical faces, strongest under the ledge above, some rust-brown.
        let drop = lm_gg_drop(world);
        let fs = lm_fade(footprint, LM_AGE_STREAK_M.x);
        let sn = lm_noise(vec3<f32>(uv.x / LM_AGE_STREAK_M.x, world.y / LM_AGE_STREAK_M.y, fid + 23.0));
        let st = mix(LM_AGE_MEAN_SS_0_06, lm_ss(0.0, 0.6, sn), fs);
        let k = LM_AGE_STREAK_TONE * st * (0.5 + 0.5 * exp(-drop / LM_AGE_STREAK_DECAY_M)) * vertical;
        let rn = lm_noise(vec3<f32>(uv.x / (LM_AGE_STREAK_M.x * 3.0), world.y / LM_AGE_STREAK_M.y, fid + 41.0));
        let rust = LM_AGE_STREAK_RUST * mix(0.5, lm_ss(-0.2, 0.4, rn), fs);
        c = mix(c * (1.0 - k), LM_WX_RUST_RGB, k * rust);
        // salt and soot at the feet.
        c = c * mix(LM_AGE_BASE_TONE, 1.0, lm_ss(LM_SEA_LEVEL_Y, LM_SEA_LEVEL_Y + LM_AGE_BASE_M, world.y));
    }
    if (LM_AGE_UNDER_ON) {
        // the dirt under the deck that no rain washes off.
        c = c * mix(1.0, LM_AGE_UNDER_DIRT, shelter_in);
    }
    (*s).albedo = c;
}

// The far item (the towers past the near items' range) at the coat's mean
// age: its paint (told from its concrete by the red) chalked, its finish flat.
fn lm_gg_age_far(s: ptr<function, Surface>, n: vec3<f32>) {
    let a = (*s).albedo;
    let paint = a.x > 2.0 * a.y;
    if (LM_AGE_MATTE_ON && paint) {
        (*s).rough = max((*s).rough, LM_AGE_ROUGH);
    }
    if (LM_AGE_FADE_ON && paint) {
        let k = LM_AGE_CHALK * (0.85 + 0.3 * max(n.y, 0.0));
        (*s).albedo = mix(a, LM_AGE_CHALK_RGB, k);
    }
}

// The Golden Gate concrete's age (`concrete_age`): run-off from the coping
// above (a pylon's top in its plan along the span, else the deck's edge over
// the point), a wall-scale tone, efflorescence and a dark foot (a pylon's own,
// else the sea's). Normalised so a face's mean stays the cited albedo (the
// foot, like the tide's band, is the foot's own).
fn lm_gg_age_concrete(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, uv: vec2<f32>, footprint: f32, vertical: f32) {
    let fid = floor(dot(n, vec3<f32>(1.0, 3.0, 7.0)) * 2.0 + 0.5);
    let c0 = dot(world.xz - LM_GG_ORIGIN, LM_GG_AXIS);
    var drop = 1e4;
    var foot = LM_SEA_LEVEL_Y;
    for (var i = 0u; i < LM_GG_PYLON_N; i = i + 1u) {
        let py = LM_GG_PYLONS[i];
        if (abs(c0 - py.x) < py.z + 0.6) {
            drop = py.y - world.y;
            foot = py.w;
        }
    }
    if (drop > 1e3 && c0 > LM_GG_DECK[0] && c0 < LM_GG_DECK[5]) {
        let deck = lm_gg_deck_top(c0);
        if (deck > world.y) {
            drop = deck - world.y;
        }
    }
    drop = max(drop, 0.0);
    // run-off: dark grey-brown streaks, strongest under the coping.
    let fr = lm_fade(footprint, LM_AGE_RUNOFF.x);
    let rn = lm_noise(vec3<f32>(uv.x / LM_AGE_RUNOFF.x, world.y / 14.0, fid + 61.0));
    let run = mix(LM_AGE_MEAN_SS_0_06, lm_ss(0.0, 0.6, rn), fr);
    let k_run = LM_AGE_RUNOFF.y * (0.35 + 0.65 * exp(-drop / LM_AGE_RUNOFF.z)) * vertical;
    // the wall's own tone, a few metres across, zero-mean.
    let fw = lm_fade(footprint, LM_AGE_WALL.x);
    let wall = lm_noise(vec3<f32>(uv.x / LM_AGE_WALL.x, world.y / 30.0, fid + 67.0)) / 0.6 * LM_AGE_WALL.y * fw * vertical;
    // efflorescence: pale streaks.
    let fe = lm_fade(footprint, 0.8);
    let ef = mix(LM_AGE_MEAN_SS_03_08, lm_ss(0.3, 0.8, lm_noise(vec3<f32>(uv.x / 0.8, world.y / 6.0, fid + 71.0))), fe) * LM_AGE_EFFLOR * vertical;
    let c = (*s).albedo;
    let dirt = vec3<f32>(0.92, 1.0, 1.12); // the run-off's dirt is brown: the blue goes first
    var a = c * (1.0 + wall) * max(vec3<f32>(1.0) - dirt * (k_run * run), vec3<f32>(0.0));
    a = mix(a, LM_AGE_EFFLOR_RGB, ef);
    // the mean the staining moves a face by, taken off again (the run-off at its mean strength, the efflorescence at its mean).
    let km = LM_AGE_RUNOFF.y * 0.5 * vertical * LM_AGE_MEAN_SS_0_06;
    let em = LM_AGE_MEAN_SS_03_08 * LM_AGE_EFFLOR * vertical;
    let mean = (c * (1.0 - km) * (1.0 - em) + LM_AGE_EFFLOR_RGB * em) / max(c, vec3<f32>(1e-4));
    a = a / mean;
    // the foot: splash and soil at a pylon's, salt and the marine growth at the sea's.
    a = a * mix(LM_AGE_FOOT.y, 1.0, lm_ss(foot, foot + LM_AGE_FOOT.x, world.y));
    if (foot == LM_SEA_LEVEL_Y) {
        a = a * mix(LM_AGE_SEA_FOOT_RGB, vec3<f32>(1.0), lm_ss(LM_SEA_LEVEL_Y, LM_SEA_LEVEL_Y + LM_AGE_SEA_FOOT_M, world.y));
    }
    (*s).albedo = max(a, vec3<f32>(0.0));
}
// sf:TURF+

// ---- turf (`--turf`; this block is in the shader only with it): grass as a canopy ----
// A lawn or a field of grass is not a painted plane: it is a layer of blades over
// soil, and what the eye sees of it depends on where the sun and the eye stand.
// Its reflectance follows the RPV model fitted to grass canopies (Rahman, Pinty
// and Verstraete 1993): a Minnaert-like term (k < 1: brighter toward grazing, where
// only lit blade sides show), a backward-peaked Henyey-Greenstein phase (theta < 0:
// darker looking into the sun, where the shaded sides face the eye) and the hotspot
// (the sun at the eye's back hides every shadow). It scales the ground's albedo by
// its ratio to a Lambertian plane (1 at 45 degrees of sun and view, 90 degrees
// apart), by the sun's share of the light for the direct part. Over it a mown
// lawn's stripes (the mower's passes, the blades leaning along its path: light where
// they lean away from the eye, dark where they lean toward it) and its tones at
// every distance: irrigation patches, clumps, dry and worn patches.
const NT_TURF_K: f32 = 0.75;
const NT_TURF_THETA: f32 = -0.15;
const NT_TURF_RHO0: f32 = 0.1;
const NT_TURF_STRIPE_M: f32 = 1.6;
const NT_TURF_STRIPE: f32 = 0.14;
// sf:end
// sf:TURF&LAWN_ALBEDO+
// (`--lawn-albedo`) a mown lawn's luminance: Kentucky bluegrass and tall fescue
// turf reflect 0.10-0.13 at 550 nm, a luminance of about 0.06-0.10 (Trenholm,
// Carrow and Duncan 1999; Bell et al. 2002); the mapped lawn tones ran to 0.15
// (the imagery's haze over a lawn reads as its own), twice a tree crown's, so the
// parks from the air were lime sheets round black crowns. The tone (before the
// scans) is held to that range, its hue and the scans' detail kept.
const NT_TURF_LUM: vec2<f32> = vec2<f32>(0.06, 0.10);
var<private> nt_lawn_k: f32 = 1.0; // (set by `nt_turf_weight` from the tone) the lawn's scale to that range
// sf:end
// sf:TURF+

// How much of a ground tone is grass (x mown lawn: bright green; y any grass:
// green or the dry summer straw), from the tone before the scans (cc0_ground_sets' kinds).
fn nt_turf_weight(c: vec3<f32>) -> vec2<f32> {
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
// sf:end
// sf:TURF&LAWN_ALBEDO+
    nt_lawn_k = clamp(l, NT_TURF_LUM.x, NT_TURF_LUM.y) / max(l, 1e-4); // (`--lawn-albedo`)
// sf:end
// sf:TURF+
    let green = smoothstep(1.03, 1.12, c.g / max(c.r, 1e-4)) * smoothstep(1.12, 1.3, c.g / max(c.b, 1e-4));
    let lawn = green * smoothstep(0.035, 0.06, l);
    let warm = c.r / max(c.b, 1e-4);
    let straw = max((1.0 - smoothstep(0.1, 0.16, abs(c.r - c.g) / max(c.g, 1e-4))) * smoothstep(1.2, 1.35, warm), hm_cured(c)); // (+ the aerial survey's cured grass)
    return vec2<f32>(lawn, max(green, straw) * smoothstep(0.012, 0.02, l));
}

// The RPV reflectance (up to rho0) for sun cosine mi, view cosine mv, phase cosine cg and relative azimuth cosine cp.
fn nt_rpv(mi: f32, mv: f32, cg: f32, cp: f32) -> f32 {
    let k = NT_TURF_K;
    let th = NT_TURF_THETA;
    let m = pow(mi * mv * (mi + mv), k - 1.0);
    let f = (1.0 - th * th) / pow(max(1.0 + 2.0 * th * cg + th * th, 1e-4), 1.5);
    let ti = sqrt(max(1.0 - mi * mi, 0.0)) / mi;
    let tv = sqrt(max(1.0 - mv * mv, 0.0)) / mv;
    let g = sqrt(max(ti * ti + tv * tv - 2.0 * ti * tv * cp, 0.0));
    return m * f * (1.0 + (1.0 - NT_TURF_RHO0) / (1.0 + g));
}

fn nt_turf(s: ptr<function, Surface>, w: vec2<f32>, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    if (w.y <= 0.0 || n.y < 0.5) {
        return;
    }
    let v = normalize(frame.cam_pos.xyz - world);
    let l = look.key_dir.xyz;
    // (the fits hold to about 78 degrees of view and 84 of sun: no further)
    let mv = clamp(dot(n, v), 0.2, 1.0);
    let mi = clamp(dot(n, l), 0.1, 1.0);
    let lt = l - n * dot(n, l);
    let vt = v - n * dot(n, v);
    let cp = dot(lt, vt) / max(length(lt) * length(vt), 1e-4);
    // reference: sun and view at 45 degrees, 90 degrees of azimuth apart (cos g 0.5, G sqrt 2).
    let ref_rpv = nt_rpv(0.70710678, 0.70710678, 0.5, 0.0);
    let sun = nt_rpv(mi, mv, clamp(dot(l, v), -1.0, 1.0), cp) / ref_rpv;
    // the sky (an extended source): the view's part alone, the sky at 41 degrees, across.
    let sky = nt_rpv(0.75, mv, 0.75 * mv, 0.0) / ref_rpv;
    // the sun's share of the light where it reaches (its shadow: in the shade of a
    // building the grass takes the sky's ratio alone, not the hotspot's brightening)
    var key = dot(look.key.xyz, vec3<f32>(0.2126, 0.7152, 0.0722)) * max(dot(n, l), 0.0);
    if (key > 0.0) {
        key = key * lk_shadow(world, n) * sky_cloud_transmittance(world);
    }
    let amb = dot(look.hemi_sky.xyz + look.ambient.xyz, vec3<f32>(0.2126, 0.7152, 0.0722));
    let share = key / max(key + amb, 1e-6);
    let canopy = clamp(mix(sky, sun, share), 0.55, 1.8);
    var alb = (*s).albedo;
// sf:end
// sf:TURF&LAWN_ALBEDO+
    alb = alb * mix(1.0, nt_lawn_k, w.x); // (`--lawn-albedo`) the tone's scale, the scans' detail kept
// sf:end
// sf:TURF+
    // The pixel's footprint (m): what is finer than it averages out.
    let fw = max(length(dpx), length(dpy));
    let p = world.xz;
    // irrigation and soil patches (40 m and 13 m): lusher and bluer, or thinner and yellower.
    let patchy = st_noise(p / 41.0 + vec2<f32>(7.1, 3.9)) * 0.6 + st_noise(p / 13.0 + vec2<f32>(1.3, 8.4)) * 0.4;
    let lush = (patchy - 0.5) * 2.0;
    alb = alb * mix(vec3<f32>(1.0), select(vec3<f32>(1.1, 1.02, 0.78), vec3<f32>(0.9, 1.04, 1.06), lush > 0.0), abs(lush) * 0.5);
    alb = alb * (1.0 + 0.14 * lush);
    // clumps and tufts (2.2 m and 0.7 m), gone where a pixel spans them.
    let ck = 1.0 - smoothstep(0.4, 1.4, fw);
    let tk = 1.0 - smoothstep(0.12, 0.45, fw);
    let clump = (st_noise(p / 2.2 + vec2<f32>(4.4, 0.7)) - 0.5) * ck + (st_noise(p / 0.7 + vec2<f32>(9.2, 5.5)) - 0.5) * tk * 0.6;
    alb = alb * (1.0 + 0.26 * clump);
    // dry and worn patches (a mown lawn's under-watered spots, the trodden ground), ~15 %.
    let dry_f = st_noise(p / 23.0 + vec2<f32>(2.7, 6.1)) * 0.65 + st_noise(p / 6.5 + vec2<f32>(5.3, 1.9)) * 0.35;
    let dry = smoothstep(0.64, 0.8, dry_f) * w.x;
    let straw = vec3<f32>(dot(alb, vec3<f32>(0.2126, 0.7152, 0.0722))) * vec3<f32>(1.5, 1.12, 0.62);
    alb = mix(alb, straw, dry * 0.75);
    // the mower's stripes along the street grid's axis.
    let d = GF_AXIS;
    let across = dot(p, vec2<f32>(-d.y, d.x)) / NT_TURF_STRIPE_M;
    let sk = (1.0 - smoothstep(0.25, 0.6, fw / NT_TURF_STRIPE_M)) * w.x * (1.0 - dry);
    if (sk > 0.0) {
        let lean = clamp(sin(across * 3.14159265) * 4.0, -1.0, 1.0);
        let toward = dot(vec2<f32>(v.x, v.z), d); // + : the eye stands where the blades lean
        alb = alb * (1.0 - NT_TURF_STRIPE * lean * toward * sk);
    }
    (*s).albedo = mix((*s).albedo, alb * canopy, w.y);
}
// sf:end
// sf:COVER+

// ---- ground cover (`--ground-cover`; this block is in the shader only with it) ----
// The city's unbuilt hills and wild parks (ground2.rs `hill_tone`: dry summer
// grass through coyote brush to groves, by the measured vegetation share) are a
// mosaic with edges, not a smear of tones: golden annual grass, coyote brush and
// chaparral in clumps and up the steep faces, eucalyptus, cypress and oak groves
// (more on the north faces), and the Franciscan chert breaking through on the
// steep faces and the ridges (thin red-brown beds, weathered dull). The mosaic is
// fractal noise from 48 m down to the pixel round the tone's level (each octave
// faded where a pixel spans it), cut with a crisp edge as wide as the pixel.
//
// Brush and groves are crowns over grass. Where a pixel resolves them each crown
// is drawn: a spheroid in a jittered cell (groves: cells of 7 m, crowns 3.2-4.6 m
// across the radius, their base 2.5 m up; brush: cells of 2.4 m, 0.8-1.4 m, on the
// ground), its normal bent over the dome so the light finds its sunlit and shaded
// sides, leaf clumps in it, and the shadows the crowns cast along the sun's path
// (a ray from the pixel toward the sun tested against the crowns round where it
// crosses their height). Where a pixel spans them, Li and Strahler's
// geometric-optical shares (1992, as the far hills' `--hill-cover`): sunlit and
// shaded crown and grass seen in the proportions the sun's and the eye's gap
// probabilities give, with the hotspot. Direct and sky light are weighed by the
// sun's share of the light at the pixel (the building shadow taken).
const NC_WOOD_CELL: f32 = 7.0;
const NC_WOOD_BASE: f32 = 2.5;
const NC_BRUSH_CELL: f32 = 2.4;
// straw on the hills (ground2.rs DRY_A / DRY_B), coyote brush, a grove's crowns, chert
const NC_BRUSH: vec3<f32> = vec3<f32>(0.050, 0.056, 0.032);
const NC_WOOD: vec3<f32> = vec3<f32>(0.030, 0.042, 0.024);
const NC_CHERT: vec3<f32> = vec3<f32>(0.150, 0.092, 0.066);

fn nc_fbm(p: vec2<f32>, s0: f32, octaves: i32, fp: f32) -> f32 {
    var sum = 0.0;
    var norm = 0.0;
    var a = 1.0;
    var sc = s0;
    for (var i = 0; i < octaves; i = i + 1) {
        let k = 1.0 - smoothstep(0.6 * sc, 1.8 * sc, fp);
        sum = sum + a * mix(0.5, st_noise(p / sc + vec2<f32>(f32(i) * 7.31, f32(i) * 3.17)), k);
        norm = norm + a;
        a = a * 0.6;
        sc = sc * 0.37;
    }
    return sum / norm;
}

// How much of a ground tone (before the scans) is the hills' cover, and its level along
// ground2.rs's line: 0 straw, 1 coyote brush, 2 grove (x weight, y level).
fn nt_cover_weight(c: vec3<f32>) -> vec2<f32> {
    let lum = max(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-5);
    let gr = c.g / max(c.r, 1e-5);
    let br = c.b / max(c.r, 1e-5);
    let x = log(lum);
    // the line in (ln luminance, g/r): straw (-1.24, 0.91), brush (-2.94, 1.29), grove (-3.44, 1.76)
    var t = clamp((-1.24 - x) / 1.70, 0.0, 1.0);
    var line = mix(0.91, 1.29, t);
    if (x < -2.94) {
        let u = clamp((-2.94 - x) / 0.5, 0.0, 1.0);
        t = 1.0 + u;
        line = mix(1.29, 1.76, u);
    }
    let br_max = mix(0.64, 0.8, clamp(t - 0.8, 0.0, 1.0));
    // (not the paler sand, bare earth and paving tones mapped on the flat: straw is ln 0.26-0.32)
    let w = (1.0 - smoothstep(0.12, 0.24, abs(gr - line))) * (1.0 - smoothstep(br_max, br_max + 0.06, br)) * smoothstep(-4.3, -3.9, x) * (1.0 - smoothstep(-1.08, -0.95, x));
    return vec2<f32>(w, t);
}

// sec and tan of the equivalent sphere's angle for a spheroid of half height b over radius r.
fn nc_sec_eq(tan_t: f32, br: f32) -> vec2<f32> {
    let t = br * tan_t;
    return vec2<f32>(sqrt(1.0 + t * t), t);
}

// The geometric-optical shares (farshore.wgsl `fs_go`): x the ground sunlit and seen,
// y the ground seen, z the crowns seen, w the crowns seen sunlit.
fn nc_go(lam: f32, tan_i: f32, tan_v: f32, cos_phi: f32, sin_phi: f32, br: f32, hb: f32) -> vec4<f32> {
    let i = nc_sec_eq(tan_i, br);
    let v = nc_sec_eq(tan_v, br);
    let d2 = max(i.y * i.y + v.y * v.y - 2.0 * i.y * v.y * cos_phi, 0.0);
    let x = i.y * v.y * sin_phi;
    let cos_t = clamp(hb * sqrt(d2 + x * x) / (i.x + v.x), -1.0, 1.0);
    let t = acos(cos_t);
    let overlap = (t - sin(t) * cos_t) * (i.x + v.x) / 3.14159265;
    let pv = exp(-lam * v.x);
    let kg = exp(-lam * (i.x + v.x - overlap));
    let ci = 1.0 / i.x;
    let cv = 1.0 / v.x;
    let cos_xi = ci * cv + i.y * ci * v.y * cv * cos_phi;
    let lit = 0.5 * (1.0 + cos_xi) * mix(1.0, 0.55, 1.0 - exp(-lam * i.x));
    return vec4<f32>(kg, pv, 1.0 - pv, (1.0 - pv) * lit);
}

// One crown layer at p (m): the topmost crown over p in the 3 x 3 cells round it.
// x its cover (antialiased by fw), y its surface's height over its base (m), zw the
// offset from its centre (m); `r` the crown's radius out of its cell's.
struct NcCrown {
    cover: f32,
    z: f32,
    d: vec2<f32>,
    r: f32,
    seed: f32,
};

fn nc_crown_at(p: vec2<f32>, cell: f32, density: f32, rmin: f32, rmax: f32, hr: f32, salt: f32, fw: f32) -> NcCrown {
    var o: NcCrown;
    o.cover = 0.0;
    o.z = -1.0;
    o.d = vec2<f32>(0.0);
    o.r = 1.0;
    o.seed = 0.0;
    let g = p / cell;
    let i0 = floor(g);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let id = i0 + vec2<f32>(f32(x), f32(y));
            if (st_hash(id + vec2<f32>(salt, 1.7)) >= density) {
                continue;
            }
            let c = (id + vec2<f32>(0.15) + 0.7 * vec2<f32>(st_hash(id + vec2<f32>(salt, 5.3)), st_hash(id + vec2<f32>(9.1, salt)))) * cell;
            let d = p - c;
            let seed = st_hash(id + vec2<f32>(salt, 13.0));
            let a = atan2(d.y, d.x);
            // a leafy outline: lobes round the crown
            let r = cell * mix(rmin, rmax, seed) * (1.0 + 0.1 * sin(a * 5.0 + seed * 40.0) + 0.06 * sin(a * 9.0 + seed * 71.0));
            let q = length(d) / r;
            let cov = clamp((1.0 - q) * r / max(fw, 1e-3) + 0.5, 0.0, 1.0);
            if (cov <= 0.0) {
                continue;
            }
            let z = r * hr * sqrt(max(1.0 - q * q, 0.0));
            if (z > o.z) {
                o.cover = max(o.cover, cov);
                o.z = z;
                o.d = d;
                o.r = r;
                o.seed = seed;
            }
        }
    }
    return o;
}

// The light the crowns of one layer leave on a point `p` at height `h` (m) on its
// way from the sun `l`: 1 open, down to `floor` through a crown.
fn nc_crown_shade(p: vec2<f32>, h: f32, l: vec3<f32>, cell: f32, density: f32, rmin: f32, rmax: f32, hr: f32, base: f32, salt: f32, floor_t: f32) -> f32 {
    let ly = max(l.y, 0.08);
    let lxz = l.xz / ly;
    var vis = 1.0;
    // where the sun's ray crosses the crowns' middle height, and a crown's height above and below it
    for (var k = -1; k <= 1; k = k + 1) {
        let hc = base + cell * rmax * hr * (0.5 + 0.45 * f32(k));
        if (hc <= h) {
            continue;
        }
        let q = p + lxz * (hc - h);
        let i0 = floor(q / cell);
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let id = i0 + vec2<f32>(f32(x), f32(y));
                if (st_hash(id + vec2<f32>(salt, 1.7)) >= density) {
                    continue;
                }
                let c = (id + vec2<f32>(0.15) + 0.7 * vec2<f32>(st_hash(id + vec2<f32>(salt, 5.3)), st_hash(id + vec2<f32>(9.1, salt)))) * cell;
                let r = cell * mix(rmin, rmax, st_hash(id + vec2<f32>(salt, 13.0)));
                // the spheroid as a sphere: heights scaled by 1 / hr
                let cy = base + r * hr * 0.35;
                let rel = vec3<f32>(c.x - p.x, (cy - h) / hr, c.y - p.y);
                let ld = normalize(vec3<f32>(l.x, l.y / hr, l.z));
                let t = dot(rel, ld);
                if (t <= 0.0) {
                    continue;
                }
                let d = sqrt(max(dot(rel, rel) - t * t, 0.0));
                vis = min(vis, mix(floor_t, 1.0, smoothstep(0.8 * r, 1.05 * r, d)));
            }
        }
    }
    return vis;
}

fn nt_cover(s: ptr<function, Surface>, wt: vec2<f32>, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let w = wt.x;
    if (w <= 0.0 || n.y < 0.3) {
        return;
    }
    let p = world.xz;
    let fw = max(length(dpx), length(dpy));
    let v = normalize(frame.cam_pos.xyz - world);
    let l = look.key_dir.xyz;
    let flat_n = max(length(n.xz), 1e-4);
    let slope = flat_n / max(n.y, 0.05); // rise over run
    let north = clamp(-n.z / flat_n, -1.0, 1.0) * smoothstep(0.05, 0.3, slope);
    // the mosaic round the tone's level: groves in the folds and on the north faces,
    // brush on the steep faces, chert on the steepest and the ridges
    let edge = clamp(fw / 8.0, 0.02, 0.3);
    let f1 = nc_fbm(p, 48.0, 5, fw);
    let f2 = nc_fbm(p + vec2<f32>(37.0, 91.0), 22.0, 4, fw);
    let lvl = wt.y + (f1 - 0.5) * 1.4 + 0.3 * north + 0.35 * smoothstep(0.25, 0.6, slope);
    let wood = smoothstep(1.45 - edge, 1.45 + edge, lvl + 0.25 * (f2 - 0.5));
    let brush = smoothstep(0.62 - edge, 0.62 + edge, lvl + 0.3 * (f2 - 0.5)) * (1.0 - wood);
    let rk = nc_fbm(p + vec2<f32>(13.0, 57.0), 30.0, 4, fw);
    // (the chert crops out where the slope sheds its soil: none on the flat)
    let rock = smoothstep(0.66 - edge, 0.66 + edge, rk + 0.45 * smoothstep(0.35, 0.9, slope)) * smoothstep(0.2, 0.45, slope) * (1.0 - wood);
    // the grass: its tone as the scans left it, drier on the south faces, greener in the swales
    let south = clamp(-north, 0.0, 1.0);
    var grass = (*s).albedo * (0.86 + 0.28 * nc_fbm(p + vec2<f32>(41.0, 5.0), 9.0, 3, fw));
    grass = grass * mix(vec3<f32>(1.0), vec3<f32>(1.08, 1.0, 0.86), south * 0.6);
    // chert: thin beds (4-12 cm) folded along the strike, red-brown to grey, dull
    let strike = vec2<f32>(0.83, 0.56);
    let beds = st_noise(vec2<f32>(dot(p, strike) * 9.0 + 2.0 * st_noise(p * 0.6), 0.5));
    let bk = 1.0 - smoothstep(0.03, 0.1, fw);
    var chert = NC_CHERT * mix(0.8, 1.15, rk) * mix(1.0, mix(0.7, 1.2, beds), bk);
    chert = mix(chert, vec3<f32>(0.11, 0.105, 0.095), smoothstep(0.55, 0.8, f2) * 0.6); // the greywacke and the lichen-grey faces
    let bg = mix(grass, chert, rock);
    // cover inside the patches: a closed grove ~0.85 with gaps, brush ~0.7
    let thin = nc_fbm(p + vec2<f32>(7.0, 29.0), 12.0, 3, fw);
    let f_wood = clamp(wood * (0.68 + 0.35 * thin), 0.0, 0.95);
    let f_brush = clamp(brush * (0.5 + 0.4 * thin), 0.0, 0.9);
    // the sun's share of the light here (its shadow taken), luminance-weighed
    let luma = vec3<f32>(0.2126, 0.7152, 0.0722);
    let nl_g = max(dot(n, l), 0.0);
    var key = dot(look.key.xyz, luma) * nl_g;
    if (key > 0.0) {
        key = key * lk_shadow(world, n) * sky_cloud_transmittance(world);
    }
    let amb = dot(look.hemi_sky.xyz + look.ambient.xyz, luma);
    let share = key / max(key + amb, 1e-6);
    // the aggregate (a pixel spanning the crowns): geometric-optical shares in the ground's frame
    let ci = clamp(dot(n, l), 0.02, 1.0);
    let cv = clamp(dot(n, v), 0.02, 1.0);
    let tan_i = sqrt(1.0 - ci * ci) / ci;
    let tan_v = sqrt(1.0 - cv * cv) / cv;
    let lt = l - n * dot(n, l);
    let vt = v - n * dot(n, v);
    let cos_phi = clamp(dot(lt, vt) / max(length(lt) * length(vt), 1e-4), -1.0, 1.0);
    let sin_phi = sqrt(max(1.0 - cos_phi * cos_phi, 0.0));
    let lam_t = -log(1.0 - f_wood);
    let lam_s = -log(1.0 - f_brush);
    let go_t = nc_go(lam_t, tan_i, tan_v, cos_phi, sin_phi, 0.8, (NC_WOOD_BASE + 3.0) / 3.0);
    let go_s = nc_go(lam_s, tan_i, tan_v, cos_phi, sin_phi, 0.7, 1.0);
    let tone = 0.85 + 0.3 * f2;
    let brush_c = NC_BRUSH * tone;
    let wood_c = NC_WOOD * tone * mix(1.0, 0.8, clamp(north, 0.0, 1.0));
    // the brush over the grass (x sun, y sky as albedos), then the groves over that
    let lit_s = go_s.w / max(go_s.z, 1e-4);
    let b_sun = go_s.z * lit_s * brush_c + go_s.x * bg;
    let b_sky = go_s.z * brush_c + go_s.y * bg * (0.5 + 0.5 * exp(-lam_s));
    let lit_t = go_t.w / max(go_t.z, 1e-4);
    let t_sun = go_t.z * lit_t * wood_c + go_t.x * b_sun;
    let t_sky = go_t.z * wood_c + go_t.y * b_sky * (0.5 + 0.5 * exp(-lam_t));
    var alb = mix(t_sky, t_sun, share);
    // resolved crowns (groves to ~2.5 m a pixel, brush to ~0.8 m), drawn over the aggregate
    let k_wood = (1.0 - smoothstep(0.9, 2.6, fw)) * step(1e-3, f_wood);
    let k_brush = (1.0 - smoothstep(0.3, 0.8, fw)) * step(1e-3, f_brush);
    if (k_wood > 0.0 || k_brush > 0.0) {
        // the brush layer at the pixel: its crown, or the grass among them with their shadows
        var lb_sun = b_sun;
        var lb_sky = b_sky;
        var h = 0.0;
        if (k_brush > 0.0) {
            let tb = nc_crown_at(p, NC_BRUSH_CELL, f_brush * 1.3, 0.34, 0.6, 0.7, 5.0, fw);
            var r_sun = bg;
            var r_sky = bg * (0.5 + 0.5 * exp(-lam_s));
            if (tb.cover > 0.0) {
                let hz = tb.r * 0.7;
                let nn = normalize(vec3<f32>(tb.d.x / (tb.r * tb.r), max(tb.z, 0.05 * hz) / (hz * hz), tb.d.y / (tb.r * tb.r)));
                let clump = st_noise(p / 0.3 + vec2<f32>(tb.seed * 50.0)) - 0.5;
                let a = brush_c * mix(0.85, 1.2, tb.seed) * (1.0 + 0.4 * clump * (1.0 - smoothstep(0.1, 0.3, fw)));
                let c_sun = a * max(dot(nn, l), 0.0) / max(nl_g, 0.05);
                let c_sky = a * mix(0.6, 1.0, tb.z / max(hz, 1e-3));
                r_sun = mix(r_sun, c_sun, tb.cover);
                r_sky = mix(r_sky, c_sky, tb.cover);
                h = tb.z * tb.cover;
            }
            if (key > 0.0) {
                r_sun = r_sun * nc_crown_shade(p, h, l, NC_BRUSH_CELL, f_brush * 1.3, 0.34, 0.6, 0.7, 0.0, 5.0, 0.2);
            }
            lb_sun = mix(b_sun, r_sun, k_brush);
            lb_sky = mix(b_sky, r_sky, k_brush);
        }
        // the groves over it
        var lt_sun = go_t.z * lit_t * wood_c + go_t.x * lb_sun;
        var lt_sky = go_t.z * wood_c + go_t.y * lb_sky * (0.5 + 0.5 * exp(-lam_t));
        if (k_wood > 0.0) {
            let tw = nc_crown_at(p, NC_WOOD_CELL, f_wood * 1.25, 0.46, 0.66, 0.8, 3.0, fw);
            var r_sun = lb_sun;
            var r_sky = lb_sky * (0.5 + 0.5 * exp(-lam_t));
            var hw = h;
            if (tw.cover > 0.0) {
                let hz = tw.r * 0.8;
                let nn = normalize(vec3<f32>(tw.d.x / (tw.r * tw.r), max(tw.z, 0.05 * hz) / (hz * hz), tw.d.y / (tw.r * tw.r)));
                let clump = st_noise(p / 0.7 + vec2<f32>(tw.seed * 50.0)) - 0.5;
                let a = wood_c * mix(0.85, 1.2, tw.seed) * (1.0 + 0.5 * clump * (1.0 - smoothstep(0.2, 0.6, fw)));
                let c_sun = a * max(dot(nn, l), 0.0) / max(nl_g, 0.05);
                let c_sky = a * mix(0.55, 1.0, tw.z / max(hz, 1e-3));
                r_sun = mix(r_sun, c_sun, tw.cover);
                r_sky = mix(r_sky, c_sky, tw.cover);
                hw = mix(h, NC_WOOD_BASE + tw.z, tw.cover);
            }
            if (key > 0.0) {
                r_sun = r_sun * nc_crown_shade(p, hw, l, NC_WOOD_CELL, f_wood * 1.25, 0.46, 0.66, 0.8, NC_WOOD_BASE, 3.0, 0.1);
            }
            lt_sun = mix(lt_sun, r_sun, k_wood);
            lt_sky = mix(lt_sky, r_sky, k_wood);
        }
        alb = mix(lt_sky, lt_sun, share);
    }
    (*s).albedo = mix((*s).albedo, alb, w);
    (*s).rough = mix((*s).rough, 0.9, w);
}
// sf:end
// sf:BARE+

// ---- bare ground (`--bare-ground`; this block is in the shader only with it) ----
// The ground WorldCover calls bare or sparsely vegetated (never over a tenth
// green in a year: the city's lots, fills and works yards, the strips the
// paving and the landmarks leave, the waterfront's made land) and the bay's
// revetments, which the land cover's placeholder tones drew as flat beige and
// mint (ground2.rs `bare_ground` marks their vertices with alpha 254 and tells
// the two apart by the tone: rock under 1.35 in r/b; bare ground 0.71 in g/r,
// 0.93 at its weediest).
//
// Bare ground: compacted fines (silty fill and decomposed granite, dry, about
// 0.17 in luminance: Post et al. 2000 measured dry soils at 0.1-0.3) with gravel
// worked into it in drifts, each stone its own (Franciscan greywacke, sandstone,
// red chert, serpentine, a little quartz), darker humus in the low spots, and the
// ruderal weeds of the city's waste ground in clumps (wild oats and foxtail dried
// to straw by late summer, a few green mallow and dock), denser along the margins
// where the ground meets the paving (the run-off collects there and nothing
// treads them), where the fines also spill a little onto the paving. A clump is
// dark at its heart and casts its shadow along the sun's path. Every scale fades
// to its mean where a pixel spans it, so from the air the ground keeps its tone.
//
// A revetment: quarry rock of about a metre laid at the shore, blocky with
// rounded arrises and dark voids between the stones, in the bay's zones by
// height over the sea (Ricketts and Calvin's zonation for the outer coast, the
// bay's own species): wet and dark with green sea lettuce and brown rockweed up to
// about high water, white with acorn barnacles a little above it, a band of black
// tar lichen (Verrucaria) where the spray reaches, then dry grey stone with
// orange Xanthoria on the tops and the gulls' white streaks along the crest.
const NB_FINES: vec3<f32> = vec3<f32>(0.205, 0.168, 0.128);
const NB_HUMUS: vec3<f32> = vec3<f32>(0.098, 0.080, 0.060);
const NB_STONES: vec3<f32> = vec3<f32>(0.180, 0.152, 0.124); // the stones' mean (nb_stone_tone's mix)
const NB_STRAW: vec3<f32> = vec3<f32>(0.300, 0.245, 0.135);
const NB_WEED: vec3<f32> = vec3<f32>(0.055, 0.085, 0.032);
const NB_PEBBLE_CELL: f32 = 0.055;
const NB_TUFT_CELL: f32 = 0.9;
const NB_TUFT_H: f32 = 0.2; // a clump's mid height (m): where its shadow is cast from
const NB_TUFT_TALL: f32 = 0.35; // a clump's height (m), 0.6-1.4 of it
const NB_ROCK_CELL: f32 = 1.15;
const NB_ROCK_MEAN: vec3<f32> = vec3<f32>(0.165, 0.160, 0.148);
const NB_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// x how much of the pixel's vertex tone is marked (the vertex alpha 254 against 255),
// y rock (1) or bare ground (0), z how weedy the bare ground is; zero unmarked.
fn nt_bare_weight(c: vec3<f32>, a: f32) -> vec3<f32> {
    if (a < 253.5) {
        return vec3<f32>(0.0);
    }
    let w = clamp(255.0 - a, 0.0, 1.0);
    let rb = c.r / max(c.b, 1e-4);
    let gr = c.g / max(c.r, 1e-4);
    let rock = (1.0 - smoothstep(1.3, 1.45, rb)) * smoothstep(0.86, 0.9, gr);
    let weedy = clamp((gr - 0.71) / 0.22, 0.0, 1.0);
    return vec3<f32>(w, rock, weedy);
}

// a surface tilt `t` in the ground's plane (world x, z) bent onto the normal `n`
fn nb_bend(n: vec3<f32>, t: vec2<f32>, up: f32) -> vec3<f32> {
    let w = vec3<f32>(t.x, 0.0, t.y);
    return normalize(n * up + (w - n * dot(n, w)));
}

struct NbBlob {
    cover: f32, // 0..1, antialiased by the footprint
    q: f32, // distance from its centre in radii
    d: vec2<f32>, // offset from its centre (radii)
    seed: f32,
};

// The blob of a grid's cell at p (m): a stone or a clump, `density` of the cells
// holding one, each kept inside its own cell (so one cell is looked at), elongated
// by its own seed; `rmin`..`rmax` of the cell.
fn nb_blob(p: vec2<f32>, cell: f32, density: f32, rmin: f32, rmax: f32, salt: f32, fw: f32) -> NbBlob {
    var o: NbBlob;
    o.cover = 0.0;
    o.q = 9.0;
    o.d = vec2<f32>(0.0);
    o.seed = 0.0;
    let g = p / cell;
    let id = floor(g);
    if (st_hash(id + vec2<f32>(salt, 3.7)) >= density) {
        return o;
    }
    let seed = st_hash(id + vec2<f32>(salt * 2.0, 5.5));
    let r = mix(rmin, rmax, seed);
    let j = (vec2<f32>(st_hash(id + vec2<f32>(1.3, salt)), st_hash(id + vec2<f32>(salt, 8.1))) - 0.5) * (1.0 - 2.0 * r);
    let a = seed * 6.2831853;
    let ax = vec2<f32>(cos(a), sin(a));
    let rel = (g - id - vec2<f32>(0.5) - j) / r;
    let d = vec2<f32>(dot(rel, ax), dot(rel, vec2<f32>(-ax.y, ax.x)) * mix(1.0, 1.6, fract(seed * 13.7)));
    o.q = length(d);
    o.d = d;
    o.seed = seed;
    o.cover = clamp((1.0 - o.q) * r * cell / max(fw, 1e-4) + 0.5, 0.0, 1.0);
    return o;
}

// a stone's tone by its seed: greywacke, sandstone, red chert, serpentine, quartz
fn nb_stone_tone(h: f32) -> vec3<f32> {
    if (h < 0.45) {
        return vec3<f32>(0.170, 0.158, 0.140);
    }
    if (h < 0.7) {
        return vec3<f32>(0.235, 0.190, 0.140);
    }
    if (h < 0.85) {
        return vec3<f32>(0.190, 0.108, 0.078);
    }
    if (h < 0.98) {
        return vec3<f32>(0.080, 0.090, 0.078);
    }
    return vec3<f32>(0.330, 0.320, 0.300);
}

// the weed clumps at p: the two grids' (0.9 m and 1.3 m, the second turned) nearer clump
fn nb_tufts(p: vec2<f32>, density: f32, fw: f32) -> NbBlob {
    let a = nb_blob(p, NB_TUFT_CELL, density, 0.16, 0.3, 11.0, fw);
    let q = vec2<f32>(p.x * 0.8 - p.y * 0.6, p.x * 0.6 + p.y * 0.8) + vec2<f32>(17.3, 4.1);
    let b = nb_blob(q, NB_TUFT_CELL * 1.46, density * 0.7, 0.14, 0.26, 23.0, fw);
    if (b.cover > a.cover) {
        return b;
    }
    return a;
}

// A revetment's stones round p (m): x the distance to the nearest stone's edge
// (m), y its seed, zw the way to that edge (unit, world x z).
fn nb_rock(p: vec2<f32>) -> vec4<f32> {
    let g = p / NB_ROCK_CELL;
    let i0 = floor(g);
    var f1 = 1e9;
    var c1 = vec2<f32>(0.0);
    var id1 = vec2<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let id = i0 + vec2<f32>(f32(x), f32(y));
            let c = id + vec2<f32>(0.2) + 0.6 * vec2<f32>(st_hash(id + vec2<f32>(4.1, 7.3)), st_hash(id + vec2<f32>(9.7, 1.9)));
            let d = dot(g - c, g - c);
            if (d < f1) {
                f1 = d;
                c1 = c;
                id1 = id;
            }
        }
    }
    // the edge: the nearest bisector with the stones round it
    var e = 1e9;
    var dir = vec2<f32>(1.0, 0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let id = id1 + vec2<f32>(f32(x), f32(y));
            if (x == 0 && y == 0) {
                continue;
            }
            let c = id + vec2<f32>(0.2) + 0.6 * vec2<f32>(st_hash(id + vec2<f32>(4.1, 7.3)), st_hash(id + vec2<f32>(9.7, 1.9)));
            let k = c - c1;
            let kl = length(k);
            if (kl < 1e-4) {
                continue;
            }
            let u = k / kl;
            let de = dot((c1 + c) * 0.5 - g, u);
            if (de < e) {
                e = de;
                dir = u;
            }
        }
    }
    return vec4<f32>(max(e, 0.0) * NB_ROCK_CELL, st_hash(id1 + vec2<f32>(2.9, 6.1)), dir);
}

fn nt_bare(s: ptr<function, Surface>, wt: vec3<f32>, world: vec3<f32>, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) {
    let p = world.xz;
    let fw = max(length(dpx), length(dpy));
    // the margin: an irregular edge where the bare ground meets the paving or the grass
    // (the vertices' 8 m blend broken by noise at 3 m and 0.8 m, cut a pixel wide)
    let en = 0.62 * st_noise(p / 3.1 + vec2<f32>(5.1, 2.3)) + 0.38 * st_noise(p / 0.83 + vec2<f32>(1.7, 9.2));
    let wv = wt.x + 0.5 * (en - 0.5);
    let ew = clamp(fw / 8.0, 0.01, 0.3);
    let w = smoothstep(0.5 - ew, 0.5 + ew, wv);
    let margin = 1.0 - smoothstep(0.55, 1.0, wv);
    // the sun's share of the light here (its shadow taken), for the shadows the clumps and
    // stones cast: only where a pixel resolves them (past that the sun's and the sky's albedos
    // are one, and the shadow lookup is saved)
    let l = look.key_dir.xyz;
    var key = dot(look.key.xyz, NB_LUMA) * max(dot(n, l), 0.0);
    if (key > 0.0 && fw < 0.52) {
        key = key * lk_shadow(world, n) * sky_cloud_transmittance(world);
    }
    let amb = dot(look.hemi_sky.xyz + look.ambient.xyz, NB_LUMA);
    let share = key / max(key + amb, 1e-6);
    // the fines spilled onto the paving beyond the edge
    let stain = (1.0 - w) * smoothstep(0.2, 0.5, wv);
    if (w <= 0.0) {
        (*s).albedo = mix((*s).albedo, (*s).albedo * 0.8 + NB_FINES * 0.15, stain * 0.5);
        return;
    }
    var alb_sky: vec3<f32>;
    var alb_sun: vec3<f32>;
    var nn = n;
    var ao = 1.0;
    var rough = 0.95;
    let rock = smoothstep(0.4, 0.6, wt.y + 0.3 * (st_noise(p / 2.3 + vec2<f32>(3.0, 3.0)) - 0.5));
    if (rock >= 0.5) {
        // ---- a revetment ----
        let kr = 1.0 - smoothstep(0.12, 0.45, fw / NB_ROCK_CELL); // its stones resolved
        var rk = vec4<f32>(1.0, 0.5, 1.0, 0.0); // (where they are not: a stone's top, its mean)
        if (kr > 0.0) {
            rk = nb_rock(p);
        }
        let edge = rk.x;
        let seed = rk.y;
        // each stone: a block tilted its own way, its shoulders rounded over 35 cm down
        // into the voids between the stones (dark: the light does not reach them)
        let tilt = (vec2<f32>(st_hash(vec2<f32>(seed * 91.0, 2.0)), st_hash(vec2<f32>(5.0, seed * 77.0))) - 0.5) * 0.9;
        let bev = 0.35;
        let u = clamp(edge / bev, 0.0, 1.0);
        let fall = 1.0 - u; // 1 at the edge, 0 on the top
        // the shoulder's slope: a quarter circle (steep at the void, flat on the top)
        let sl = fall / max(sqrt(max(1.0 - fall * fall, 0.0)), 0.25);
        let slope = tilt * u + rk.zw * sl * 0.9;
        nn = normalize(mix(n, nb_bend(n, slope, 1.0), kr));
        let void_k = 1.0 - smoothstep(0.03, 0.1, edge);
        var tone: vec3<f32>;
        if (seed < 0.5) {
            tone = vec3<f32>(0.195, 0.192, 0.180); // granite
        } else if (seed < 0.8) {
            tone = vec3<f32>(0.150, 0.150, 0.128); // greywacke
        } else if (seed < 0.92) {
            tone = vec3<f32>(0.185, 0.155, 0.122); // sandstone
        } else {
            tone = vec3<f32>(0.075, 0.075, 0.072); // basalt
        }
        // weathered faces: blotched at 30 cm and 8 cm
        let kb = 1.0 - smoothstep(0.03, 0.12, fw);
        tone = tone * (0.85 + 0.3 * st_noise(p / 0.3 + vec2<f32>(seed * 40.0, 1.0))) * mix(1.0, 0.9 + 0.2 * st_noise(p / 0.08 + vec2<f32>(seed * 17.0, 3.0)), kb);
        var a = mix(tone, tone * 0.08, void_k);
        a = mix(NB_ROCK_MEAN * 0.8, a, kr);
        ao = mix(0.7, mix(0.15, 1.0, smoothstep(0.0, 0.25, edge)), kr);
        rough = 0.86;
        // the zones by height over the sea (game m), their edges broken over each stone
        let z = world.y - LM_SEA_LEVEL_Y + 0.35 * (st_noise(p / 1.7 + vec2<f32>(8.0, 2.0)) - 0.5) + 0.15 * (1.0 - fall);
        let top = smoothstep(0.6, 0.9, nn.y) * (1.0 - fall);
        // wet up to about high water, with sea lettuce and below it rockweed
        let wet = 1.0 - smoothstep(0.8, 1.2, z);
        let algae = (1.0 - smoothstep(0.5, 0.9, z)) * smoothstep(0.45, 0.65, st_noise(p / 0.6 + vec2<f32>(4.0, 9.0)));
        let weed = (1.0 - smoothstep(0.15, 0.45, z));
        a = mix(a, mix(vec3<f32>(0.045, 0.072, 0.030), vec3<f32>(0.062, 0.046, 0.022), weed), algae);
        // acorn barnacles a little above it: white specks on the stones (1.5 cm), their mean beyond
        let barn = smoothstep(0.5, 0.75, z) * (1.0 - smoothstep(1.2, 1.5, z)) * (1.0 - void_k);
        let kbn = 1.0 - smoothstep(0.006, 0.02, fw);
        let speck = step(0.62, st_hash(floor(p / 0.015) + vec2<f32>(seed * 31.0, 0.0)));
        a = mix(a, vec3<f32>(0.42, 0.41, 0.38), barn * mix(0.3, speck, kbn));
        a = a * mix(1.0, 0.55, wet);
        rough = mix(rough, 0.4, wet);
        // black tar lichen where the spray reaches
        let tar = smoothstep(1.2, 1.5, z) * (1.0 - smoothstep(2.2, 2.7, z)) * smoothstep(0.35, 0.55, st_noise(p / 0.45 + vec2<f32>(seed * 9.0, 7.0)));
        a = mix(a, vec3<f32>(0.040, 0.040, 0.037), tar * 0.85);
        // above it the dry stone's lichens on the tops: orange Xanthoria, grey-green crusts; the gulls' streaks on the crest
        let dry_z = smoothstep(2.1, 2.6, z);
        let lich = smoothstep(0.7, 0.78, st_noise(p / 0.05 + vec2<f32>(seed * 23.0, 4.0))) * smoothstep(0.5, 0.7, st_noise(p / 0.4 + vec2<f32>(1.0, seed * 5.0))) * top * dry_z;
        let crust = smoothstep(0.5, 0.7, st_noise(p / 0.35 + vec2<f32>(2.0, seed * 29.0))) * dry_z * (1.0 - void_k);
        a = mix(a, vec3<f32>(0.24, 0.25, 0.21), crust * 0.5);
        a = mix(a, vec3<f32>(0.30, 0.15, 0.045), lich * mix(0.15, 0.8, kb));
        let guano = smoothstep(3.0, 3.6, z) * top * smoothstep(0.7, 0.8, st_noise(vec2<f32>(p.x / 0.25, p.y / 0.9) + vec2<f32>(seed * 13.0, 0.0)));
        a = mix(a, vec3<f32>(0.55, 0.54, 0.50), guano * 0.7);
        alb_sky = a;
        // a low sun casts the next stone's shadow over the arris on the far side from it
        let toward = dot(normalize(l.xz + vec2<f32>(1e-5, 0.0)), rk.zw);
        let cast_sh = fall * smoothstep(0.0, 0.5, toward) * (1.0 - smoothstep(0.25, 0.7, l.y));
        alb_sun = a * (1.0 - 0.75 * cast_sh * kr);
    } else {
        // ---- bare ground ----
        // drifts of gravel and fines (6 m and 1.4 m); damp humus in the low spots
        let drift = 0.65 * st_noise(p / 6.0 + vec2<f32>(3.3, 8.8)) + 0.35 * st_noise(p / 1.4 + vec2<f32>(7.7, 0.4));
        let gravel = smoothstep(0.3, 0.75, drift);
        let hollow = smoothstep(0.55, 0.85, st_noise(p / 2.6 + vec2<f32>(4.2, 6.6)));
        let kg = 1.0 - smoothstep(0.03, 0.12, fw);
        let grit = mix(1.0, 0.82 + 0.36 * st_noise(p / 0.11 + vec2<f32>(2.0, 1.0)), kg);
        var a = mix(NB_FINES, NB_HUMUS, hollow * 0.55 + margin * 0.15) * grit;
        // the stones: two grids (5.5 cm and 3.7 cm, the second turned), their mean where a pixel spans them
        let dens = mix(0.2, 0.8, gravel) * (1.0 - 0.6 * hollow);
        let kp = 1.0 - smoothstep(0.012, 0.04, fw);
        let mean_cover = dens * 0.3;
        var stone_k = 0.0;
        if (kp > 0.0) {
            let s1 = nb_blob(p, NB_PEBBLE_CELL, dens, 0.26, 0.42, 1.0, fw);
            let q2 = vec2<f32>(p.x * 0.87 - p.y * 0.5, p.x * 0.5 + p.y * 0.87);
            let s2 = nb_blob(q2, NB_PEBBLE_CELL * 0.68, dens * 0.8, 0.26, 0.42, 2.0, fw);
            var pb = s1;
            if (s2.cover > s1.cover) {
                pb = s2;
            }
            let tone = nb_stone_tone(pb.seed) * (0.9 + 0.2 * fract(pb.seed * 7.3));
            a = mix(a, tone, pb.cover * kp);
            stone_k = pb.cover * kp;
            // a stone's dome, and the fines darkened round its foot
            let dome = sqrt(max(1.0 - pb.q * pb.q, 0.0));
            nn = nb_bend(n, pb.d * 0.9 * pb.cover * kp, max(dome, 0.35));
            ao = 1.0 - 0.35 * (1.0 - smoothstep(1.0, 1.35, pb.q)) * (1.0 - pb.cover) * kp;
        }
        a = mix(a, mix(a, NB_STONES, mean_cover), 1.0 - kp);
        rough = mix(0.95, 0.8, stone_k);
        // the weeds: sparse clumps, weedier with the measured share and along the margins
        let dens_t = clamp(mix(0.14, 0.42, wt.z) + 0.4 * margin, 0.0, 0.75);
        let green_f = mix(0.15, 0.4, wt.z);
        let kt = 1.0 - smoothstep(0.08, 0.3, fw);
        let mean_t = dens_t * 0.17;
        let mean_c = mix(NB_STRAW, NB_WEED, green_f) * 0.75;
        alb_sky = mix(a, mean_c, mean_t * (1.0 - kt));
        alb_sun = alb_sky;
        if (kt > 0.0) {
            // The clumps stand up: the eye's ray is marched down through their height
            // (relief mapping, six steps from 0.5 m to the ground, jittered per point for
            // the temporal resolve), so near the eye a clump shows its side and hides the
            // ground behind it instead of lying flat on it.
            let v = normalize(frame.cam_pos.xyz - world);
            let off = v.xz / max(dot(v, n), 0.03);
            let jit = st_hash(p * 731.0 + vec2<f32>(frame.cam_pos.x * 3.1, 0.0));
            var hit = vec4<f32>(0.0);
            var hit_d = vec2<f32>(0.0);
            for (var k = 0; k < 6; k = k + 1) {
                let h = NB_TUFT_TALL * 1.4 * (1.0 - (f32(k) + jit) / 6.0);
                let pk = p + off * h;
                let t = nb_tufts(pk, dens_t, fw);
                if (t.cover > 0.0) {
                    // a tussock: an ellipsoid of blades, ragged and gappy toward its rim and top
                    let tall = NB_TUFT_TALL * mix(0.6, 1.4, fract(t.seed * 3.3));
                    let rel = h / tall;
                    let ang = atan2(t.d.y, t.d.x);
                    let blade = st_noise(vec2<f32>(ang * 16.0 + t.seed * 40.0, rel * 3.0 + t.q));
                    let reach = sqrt(max(1.0 - rel * rel, 0.0)) * (0.55 + 0.55 * blade);
                    let inside = (1.0 - smoothstep(reach - 0.08, reach, t.q)) * mix(1.0, smoothstep(0.3, 0.55, blade), smoothstep(0.3, 0.8, max(rel, t.q)));
                    if (t.cover * inside * kt > 0.5) {
                        hit = vec4<f32>(1.0, rel, t.q, t.seed);
                        hit_d = t.d;
                        break;
                    }
                }
            }
            var t_sun = a;
            if (hit.x > 0.0) {
                // its blades: dark at the heart and the foot, lit toward the tips, straw or green
                let green = step(1.0 - green_f, fract(hit.w * 5.31));
                var c = mix(NB_STRAW * (0.8 + 0.4 * fract(hit.w * 3.7)), NB_WEED * (0.85 + 0.3 * fract(hit.w * 9.1)), green);
                let lit = smoothstep(0.0, 0.9, hit.y) * 0.6 + smoothstep(0.2, 0.9, hit.z) * 0.4;
                c = c * mix(0.3, 1.2, lit);
                alb_sky = c;
                t_sun = c;
                nn = nb_bend(n, hit_d * (0.4 + 0.6 * hit.y), 0.8);
                ao = mix(0.45, 1.0, lit);
                rough = 0.8;
            } else {
                t_sun = alb_sky;
            }
            // the clumps' shadows on the ground, cast along the sun's path from their mid height
            var sh = 1.0;
            if (key > 0.0 && hit.x <= 0.0) {
                let ly = max(l.y, 0.08);
                let ct = nb_tufts(p + l.xz / ly * NB_TUFT_H, dens_t, fw);
                sh = 1.0 - 0.65 * ct.cover * (1.0 - smoothstep(0.4, 0.75, ct.q)) * kt;
            }
            alb_sun = mix(alb_sky, t_sun * sh, kt);
            alb_sky = mix(mix(a, mean_c, mean_t), alb_sky, kt);
        }
    }
    let alb = mix(alb_sky, alb_sun, share);
    (*s).albedo = mix((*s).albedo * mix(1.0, 0.8, stain) + NB_FINES * 0.15 * stain, alb, w);
    (*s).n = normalize(mix((*s).n, nn, w));
    (*s).ao = (*s).ao * mix(1.0, ao, w);
    (*s).rough = mix((*s).rough, rough, w);
    (*s).metal = mix((*s).metal, 0.0, w);
}
// sf:end
