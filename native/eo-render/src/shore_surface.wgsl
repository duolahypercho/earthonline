// The shore's ground and wall surfaces, appended to the city shader right
// after landmarks_surface.wgsl (landmarks.rs `city_wgsl`): the seawall's
// concrete and tide band (`lm_seawall`), the wet margin at the waterline
// (`lm_wet_ground`), the beach sand (`lm_beach`) and the band the swash wets
// (`lm_beach_swash_wet`). They use landmarks_surface.wgsl's helpers
// (`lm_ss`, `lm_mx_noise`) and the generated LM_* constants; draw.wgsl and
// cc0.wgsl call them.

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
