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
            (*s).ao = (*s).ao * lm_gg_deck_contact(world);
            lm_key_vis = lm_deck_key(world);
            lm_rim_vis = lm_bridge_rim();
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
        }
    } else if (kind == 2u) {
        let tone = lm_noise(local / LM_CO_PATCH_M) * LM_CO_PATCH_TONE;
        let wet = mix(LM_CO_WET_TONE, 1.0, lm_ss(0.0, LM_CO_WET_BAND_M, world.y));
        let streak = lm_noise(vec3<f32>(uv.x * 2.3, uv.y * 0.11, 4.2)) * LM_CO_STREAK_TONE * vertical;
        (*s).albedo = (*s).albedo * vec3<f32>((1.0 + tone - abs(streak)) * wet);
        if (bridge) {
            lm_key_vis = lm_deck_key(world);
            lm_rim_vis = lm_bridge_rim();
            lm_bridge_tide(s, world, vertical);
            rm_emit = rm_emit + lm_bridge_bounce(world, n, (*s).albedo, (*s).ao);
        }
        (*s).rough = clamp(((*s).rough + lm_noise(local / 3.1 + vec3<f32>(1.7, 8.2, 4.4)) * LM_CO_ROUGH_SPREAD)
            * mix(0.75, 1.0, lm_ss(0.0, LM_CO_WET_BAND_M, world.y)), 0.4, 1.0);
        let fj = lm_fade(footprint, LM_CO_LIFT_M);
        if (fj > 0.0) {
            (*s).n = lm_bump(n, dpx, dpy, lm_concrete_height(uv) * fj, lm_concrete_height(uv + du) * fj, lm_concrete_height(uv + dv) * fj);
        }
    }
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
    let sky = look.hemi_sky.xyz + look.ambient.xyz + lk_sh(up) * look.ambient.w * look.env_class.x;
    let share = clamp((1.0 - n.y) * 0.5, 0.0, 1.0);
    return (LM_SEA_ALBEDO * (key + sky) - look.hemi_ground.xyz) * share;
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

// renderer.js's seawall colour and roughness nodes: pour mottle, vertical
// run-off streaks, the algae-green tide stain over the bottom half metre, the
// wet-dark band at the waterline, the stack floored at 0.38 of the albedo, the
// sky-catch on the upper face; wet concrete glossier through the stain.
fn lm_seawall(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>) {
    let base = (*s).albedo;
    let sea = LM_SEA_LEVEL_Y;
    let mottle = lm_mx_noise(vec3<f32>(world.x * 0.11, world.y * 0.45, world.z * 0.11)) * 0.09 + 1.0;
    let streak = 1.0 - (lm_mx_noise(vec3<f32>(world.x * 1.35, world.y * 0.07, world.z * 1.35)) * 0.5 + 0.5) * 0.14 + 0.07;
    var tone = base * mottle * streak;
    let stain_t = lm_ss(sea + 0.5, sea + 0.08, world.y);
    let stain_noise = lm_mx_noise(vec3<f32>(world.x * 0.9, world.y * 1.8, world.z * 0.9)) * 0.14 + 0.86;
    tone = mix(tone, tone * vec3<f32>(0.62, 0.67, 0.55) * stain_noise, stain_t);
    let wet = 1.0 - lm_ss(sea + 0.55, sea + 0.02, world.y) * 0.36;
    let floored = max(tone * wet, base * 0.38);
    let faceness = clamp(1.0 - abs(n.y), 0.0, 1.0);
    let crest_band = lm_ss(sea + 0.55, sea + 1.0, world.y);
    (*s).albedo = floored * (faceness * crest_band * 0.38 + 1.0);
    (*s).rough = clamp((*s).rough - lm_ss(sea + 0.55, sea + 0.08, world.y) * 0.4, 0.05, 1.0);
    // the intertidal band the bay's 1.7 m tide leaves on a wall: a
    // ragged tide line about 1.2 m over the still sea, dark and brown-green
    // below it (weed and silt), black-green and glossy in the splash a few
    // decimetres over the water.
    let ragged = lm_mx_noise(vec3<f32>(world.x * 0.7, 0.3, world.z * 0.7)) * 0.18 + lm_mx_noise(vec3<f32>(world.x * 3.1, 1.7, world.z * 3.1)) * 0.06;
    let tide = lm_ss(sea + 1.25 + ragged, sea + 1.05 + ragged, world.y) * faceness;
    let splash = lm_ss(sea + 0.45, sea + 0.05, world.y) * faceness;
    (*s).albedo = mix((*s).albedo, (*s).albedo * vec3<f32>(0.42, 0.44, 0.36), tide);
    (*s).albedo = mix((*s).albedo, (*s).albedo * vec3<f32>(0.45, 0.5, 0.42), splash * 0.7); // (sea r9: darkened, not black: it read as a gap at the wall foot)
    (*s).rough = mix((*s).rough, 0.22, splash);
}

// beach-surface.js's wet margin on the dry side of the waterline (ground
// within a few decimetres of the datum stays saturated from below: darkest at
// the water, drying up the face; 0.6 of full wetness where no shore
// simulation runs) and its wet look: darker, more saturated, cooled, glossier.
fn lm_wet_ground(s: ptr<function, Surface>, world: vec3<f32>) {
    let wet = lm_ss(LM_SEA_LEVEL_Y + 0.35, LM_SEA_LEVEL_Y + 0.02, world.y) * 0.6;
    if (wet <= 0.0) {
        return;
    }
    let dry = (*s).albedo;
    let wet_base = dry * 0.58;
    let luma = dot(wet_base, vec3<f32>(0.2126, 0.7152, 0.0722));
    let wet_sand = mix(vec3<f32>(luma), wet_base, 1.15) * vec3<f32>(0.97, 0.98, 1.0);
    (*s).albedo = mix(dry, wet_sand, wet);
    (*s).rough = mix((*s).rough, mix(0.45, 0.28, wet), wet);
}

// beach-surface.js `createBeachSurfaceMaterial` on the terrain's
// beach sand (`shore` metres inland of the shoreline, negative offshore, from
// the vertex): the dry sand's drifts and two grain scales, the 0.42 m wind
// ripples above the swash, the wrack line, the damp band the swash leaves
// (its reach at the sets' mean, 0.7 of SWASH_REACH_M: the moving sheet and
// its foam are not drawn), the wet margin at the waterline, and the
// submerged sand's wave ripples and fade to the deep bed. Three's MaterialX
// noise (`lm_mx_noise`) as the seawall's.
const LM_SWASH_REACH_M: f32 = 24.0;
const LM_WRACK_A: f32 = 38.0;
const LM_WRACK_B: f32 = 58.0;
const LM_SEABED_FADE_A: f32 = 60.0;
const LM_SEABED_FADE_B: f32 = 180.0;

fn lm_beach(s: ptr<function, Surface>, wp: vec3<f32>, shore: f32) {
    let coarse = lm_mx_noise(vec3<f32>(wp.x * 0.12, wp.z * 0.12, 0.35)) * 0.5 + 0.5;
    let macro_n = lm_mx_noise(vec3<f32>(wp.x * 0.031, wp.z * 0.031, 1.1)) * 0.5 + 0.5;
    let grain = lm_mx_noise(vec3<f32>(wp.x * 1.3, wp.z * 1.3, 2.2)) * 0.5 + 0.5;
    let grain_fine = lm_mx_noise(vec3<f32>(wp.x * 5.4, wp.z * 5.4, 4.4)) * 0.5 + 0.5;
    let ripples = sin(wp.x * 2.4 + wp.z * 0.9 + (macro_n - 0.5) * 2.4) * 0.5 + 0.5;
    let base = (*s).albedo;
    var dry = mix(base * vec3<f32>(1.06, 1.05, 1.03), base * vec3<f32>(1.0, 0.96, 0.89), lm_ss(0.34, 0.76, macro_n));
    dry = dry * ((coarse - 0.5) * 0.13 + 1.0) * ((grain - 0.45) * 0.26 + 0.99) * ((grain_fine - 0.45) * 0.16 + 1.0);
    let ripple_k = lm_ss(LM_SWASH_REACH_M + 4.0, LM_SWASH_REACH_M + 18.0, shore);
    dry = dry * ((ripples - 0.5) * 0.1 * ripple_k + 1.0);
    let wrack_band = lm_ss(LM_WRACK_A, LM_WRACK_A + 7.0, shore) * lm_ss(LM_WRACK_B, LM_WRACK_B - 9.0, shore);
    let wrack = wrack_band * lm_ss(0.56, 0.74, grain) * lm_ss(0.4, 0.6, macro_n);
    dry = mix(dry, vec3<f32>(0.27, 0.21, 0.14), wrack * 0.62);
    let reach = 0.7 * LM_SWASH_REACH_M;
    let damp = lm_ss(reach + 5.0, reach - 6.0, shore) * 0.62;
    let margin = lm_ss(LM_SEA_LEVEL_Y + 0.35, LM_SEA_LEVEL_Y + 0.02, wp.y) * 0.6;
    let wet = clamp(max(damp, margin), 0.0, 1.0);
    let wet_base = dry * 0.58;
    let luma = dot(wet_base, vec3<f32>(0.2126, 0.7152, 0.0722));
    let wet_sand = mix(vec3<f32>(luma), wet_base, 1.15) * vec3<f32>(0.97, 0.98, 1.0);
    var colour = mix(dry, wet_sand, wet);
    let submerged = lm_ss(LM_SEA_LEVEL_Y + 0.03, LM_SEA_LEVEL_Y - 0.3, wp.y) * lm_ss(60.0, 45.0, shore);
    let wave = sin(shore * 6.2 + macro_n * 3.0) * 0.5 + 0.5;
    var under = wet_sand * ((wave - 0.5) * 0.12 + 0.95);
    let depth_below = max(LM_SEA_LEVEL_Y - wp.y, 0.0);
    under = mix(under, vec3<f32>(0.05, 0.075, 0.08), lm_ss(LM_SEABED_FADE_A, LM_SEABED_FADE_B, max(-shore, 0.0)) * lm_ss(0.6, 1.8, depth_below));
    colour = mix(colour, under, submerged);
    (*s).albedo = colour;
    (*s).rough = clamp(mix(0.94, mix(0.45, 0.28, wet), wet), 0.06, 1.0);
    lm_beach_swash_wet(s, wp);
}

// the sand the swash wets (the water pass runs each bore up the beach
// to about half a metre over the sea): below that reach it is dark and glossy
// (water in its pores, about half the dry albedo), nearly a mirror within a
// few centimetres of the sea, drying over the 0.35 m above. Keyed to the sea
// level, not to the ground's height scale, as the swash itself is.
fn lm_beach_swash_wet(s: ptr<function, Surface>, wp: vec3<f32>) {
    let wet = lm_ss(LM_SEA_LEVEL_Y + 0.65, LM_SEA_LEVEL_Y + 0.3, wp.y);
    if (wet <= 0.0) {
        return;
    }
    let soak = lm_ss(LM_SEA_LEVEL_Y + 0.3, LM_SEA_LEVEL_Y + 0.03, wp.y);
    (*s).albedo = mix((*s).albedo, (*s).albedo * vec3<f32>(0.5, 0.52, 0.56), wet * 0.9);
    (*s).rough = mix((*s).rough, mix(0.3, 0.08, soak), wet);
    // The band the swash keeps running over lies under a film of water a few millimetres thick,
    // in patches (drier between): the film half fills the sand's relief (the normal halfway to flat),
    // is nearly smooth (0.07), reflects the environment at 0.65 of its radiance (ENV_WET_FILM,
    // look_light.wgsl), and the sand under it is a tenth darker.
    let patchy = lm_mx_noise(vec3<f32>(wp.x * 0.06, wp.z * 0.06, 5.1));
    let film = lm_ss(LM_SEA_LEVEL_Y + 0.42, LM_SEA_LEVEL_Y + 0.12, wp.y + 0.12 * patchy) * lm_ss(0.0, 0.35, patchy + 0.1 * soak) * (0.55 + 0.45 * lm_ss(-0.3, 0.3, lm_mx_noise(vec3<f32>(wp.x * 0.7, wp.z * 0.7, 2.6))));
    (*s).n = normalize(mix((*s).n, vec3<f32>(0.0, 1.0, 0.0), film * 0.5));
    (*s).rough = mix((*s).rough, 0.07, film);
    (*s).albedo = (*s).albedo * (1.0 - 0.1 * film);
    (*s).env = ENV_WET_FILM;
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
