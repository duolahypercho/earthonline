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
    if (kind == 1u) {
        let tone = lm_noise(local / LM_PS_PATCH_M) * LM_PS_PATCH_TONE
            + lm_noise(local / LM_PS_FINE_PATCH_M + vec3<f32>(13.1, 7.7, 3.3)) * LM_PS_FINE_PATCH_TONE;
        // Salt grime: darkest at the water, gone by grimeHeightM over it.
        let grime = mix(LM_PS_GRIME_TONE, 1.0, lm_ss(0.0, LM_PS_GRIME_HEIGHT_M, world.y));
        let streak = lm_noise(vec3<f32>(uv.x * 1.7, uv.y * 0.07, 0.0)) * LM_PS_STREAK_TONE * vertical;
        let chalk = lm_ss(0.4, 0.95, up) * LM_PS_CHALK_TONE;
        (*s).albedo = (*s).albedo * (vec3<f32>((1.0 + tone - abs(streak)) * grime) + chalk);
        (*s).rough = clamp((*s).rough + lm_noise(local / (LM_PS_PATCH_M * 0.7) + vec3<f32>(5.1, 2.3, 9.4)) * LM_PS_ROUGH_SPREAD
            + chalk / max(1e-3, LM_PS_CHALK_TONE) * LM_PS_CHALK_ROUGH, 0.2, 0.95);
        let fs = lm_fade(footprint, LM_PS_PLATE_U);
        let fr2 = lm_fade(footprint, LM_PS_RIVET_PITCH);
        if (fs + fr2 > 0.0) {
            let h0 = lm_steel_height(uv);
            let hx = lm_steel_height(uv + du);
            let hy = lm_steel_height(uv + dv);
            (*s).n = lm_bump(n, dpx, dpy, h0.x * fs + h0.y * fr2, hx.x * fs + hx.y * fr2, hy.x * fs + hy.y * fr2);
        }
    } else if (kind == 2u) {
        let tone = lm_noise(local / LM_CO_PATCH_M) * LM_CO_PATCH_TONE;
        let wet = mix(LM_CO_WET_TONE, 1.0, lm_ss(0.0, LM_CO_WET_BAND_M, world.y));
        let streak = lm_noise(vec3<f32>(uv.x * 2.3, uv.y * 0.11, 4.2)) * LM_CO_STREAK_TONE * vertical;
        (*s).albedo = (*s).albedo * vec3<f32>((1.0 + tone - abs(streak)) * wet);
        (*s).rough = clamp(((*s).rough + lm_noise(local / 3.1 + vec3<f32>(1.7, 8.2, 4.4)) * LM_CO_ROUGH_SPREAD)
            * mix(0.75, 1.0, lm_ss(0.0, LM_CO_WET_BAND_M, world.y)), 0.4, 1.0);
        let fj = lm_fade(footprint, LM_CO_LIFT_M);
        if (fj > 0.0) {
            (*s).n = lm_bump(n, dpx, dpy, lm_concrete_height(uv) * fj, lm_concrete_height(uv + du) * fj, lm_concrete_height(uv + dv) * fj);
        }
    }
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

// How much of a ground tone is grass (x mown lawn: bright green; y any grass:
// green or the dry summer straw), from the tone before the scans (cc0_ground_sets' kinds).
fn nt_turf_weight(c: vec3<f32>) -> vec2<f32> {
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let green = smoothstep(1.03, 1.12, c.g / max(c.r, 1e-4)) * smoothstep(1.12, 1.3, c.g / max(c.b, 1e-4));
    let lawn = green * smoothstep(0.035, 0.06, l);
    let warm = c.r / max(c.b, 1e-4);
    let straw = (1.0 - smoothstep(0.1, 0.16, abs(c.r - c.g) / max(c.g, 1e-4))) * smoothstep(1.2, 1.35, warm);
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
    // the sun's share of the light here (its shadow taken), for the shadows the clumps and stones cast
    let l = look.key_dir.xyz;
    var key = dot(look.key.xyz, NB_LUMA) * max(dot(n, l), 0.0);
    if (key > 0.0) {
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
        let rk = nb_rock(p);
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
