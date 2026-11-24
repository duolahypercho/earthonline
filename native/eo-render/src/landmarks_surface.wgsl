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
    let key = dot(look.key.xyz, vec3<f32>(0.2126, 0.7152, 0.0722)) * max(dot(n, l), 0.0);
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
