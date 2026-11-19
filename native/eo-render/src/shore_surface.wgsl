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
    if (GG2_ASHLAR) {
        gg2_ashlar(s, world, n, faceness, base);
    }
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
    var wrack_band = lm_ss(LM_WRACK_A, LM_WRACK_A + 7.0, shore) * lm_ss(LM_WRACK_B, LM_WRACK_B - 9.0, shore);
    var gh = 0.0; // the foreshore's height over the sea (GG2_SAND), read once
    if (GG2_SAND) {
        gh = gg2_foreshore_h(wp, shore);
        wrack_band = gg2_wrack_band(wp, gh);
        // the prints over the strand line and within GG2_PRINTS_NEAR_M of the eye (a 0.28 m
        // print is under a pixel or two past it): the cell search runs only there
        let prints_k = lm_ss(GG2_SAND_MHHW_M + 0.05, GG2_SAND_MHHW_M + 0.3, gh) * lm_ss(GG2_PRINTS_NEAR_M, 0.75 * GG2_PRINTS_NEAR_M, distance(frame.cam_pos.xyz, wp));
        if (prints_k > 0.0) {
            dry = dry * (1.0 - 0.18 * gg2_footprints(wp) * prints_k);
        }
    }
    let wrack = wrack_band * lm_ss(0.56, 0.74, grain) * lm_ss(0.4, 0.6, macro_n);
    dry = mix(dry, vec3<f32>(0.27, 0.21, 0.14), wrack * 0.62);
    let reach = 0.7 * LM_SWASH_REACH_M;
    var damp = lm_ss(reach + 5.0, reach - 6.0, shore) * 0.62;
    var wet_ratio = 0.58;
    if (GG2_SAND) {
        damp = gg2_damp(gg2_damp_h(wp, shore));
        wet_ratio = GG2_WET_SAND_RATIO;
    }
    let margin = lm_ss(LM_SEA_LEVEL_Y + 0.35, LM_SEA_LEVEL_Y + 0.02, wp.y) * 0.6;
    let wet = clamp(max(damp, margin), 0.0, 1.0);
    let wet_base = dry * wet_ratio;
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

// The shore ground pass's sand (`GG2_SAND`, cc0.rs `shore_ground_wgsl`;
// `--no-sand-albedo` off): bands keyed to the height over the sea, at the
// drawn height scale, from the Golden Gate tide station's datums.
//
// The height is the foreshore's (`gg2_foreshore_h`): the beach ground the
// carpet lays (eo-water `beach_foreshore`, the surf model's slope through the
// sea level at the shoreline), from the shore distance the vertex carries.
// Not the triangle's own height: on the near-flat foreshore its contours
// follow the 8 m grid's diagonals and the blend back to the surveyed ground,
// and a band edge keyed to them is a sawtooth.
fn gg2_foreshore_h(wp: vec3<f32>, shore: f32) -> f32 {
    var slope = GG2_FORESHORE_SLOPE;
    if (GG3_SAND2 && gg3_sand_slope > 0.0) {
        slope = gg3_sand_slope; // the beach's own measured foreshore slope (its vertices carry it)
    }
    return gg2_foreshore_h_at(wp, shore, slope);
}

fn gg2_foreshore_h_at(wp: vec3<f32>, shore: f32, slope: f32) -> f32 {
    // a ragged edge: a slow wander of a few metres along the shore and a finer fray
    let rag = lm_mx_noise(vec3<f32>(wp.x * 0.025, wp.z * 0.025, 6.3)) * 0.12 + lm_mx_noise(vec3<f32>(wp.x * 0.11, wp.z * 0.11, 2.7)) * 0.07;
    return max(shore, 0.0) * slope + rag;
}

// The height the damp band is keyed to: the round-2 1:55 foreshore's
// (GG2_FORESHORE_SLOPE), not the beach's measured slope (the wrack and the
// prints follow that, `gg2_foreshore_h`). On the steep Crissy foreshore
// (1:10) the mean-high-water contour lies about 8 m from the waterline, but
// the damp sand also reaches over the swash's run-up and the capillary
// fringe over the water table (INFERRED), which the tide's height alone does
// not give; until a cited run-up source keys the band, it keeps round 2's width.
fn gg2_damp_h(wp: vec3<f32>, shore: f32) -> f32 {
    return gg2_foreshore_h_at(wp, shore, GG2_FORESHORE_SLOPE);
}

// The beach's measured foreshore slope at the fragment (GG3_SAND2, `--no-sand-detail2`;
// terrain_detail.rs `sand_slope_byte`: the normal's w byte as a share of GG3_SAND_SLOPE_FULL),
// set by draw.wgsl `shade`; 0 where the vertex carries none.
var<private> gg3_sand_slope: f32;

// The damp band: the sand the high water covers twice a day stays damp below
// it (mean high water), darkest where the swash keeps it wet (0.3 m over the
// sea, where `lm_beach_swash_wet` takes over) and drying toward the high
// water line. `h`: the foreshore's height over the sea (`gg2_foreshore_h`).
fn gg2_damp(h: f32) -> f32 {
    return lm_ss(GG2_SAND_MHW_M + 0.05, 0.3, h) * 0.8;
}

// The wrack (the strand line of kelp and eelgrass the higher high water
// leaves): a ragged band round the mean higher high water line, a few
// centimetres of height across (a couple of metres on the foreshore).
fn gg2_wrack_band(wp: vec3<f32>, h: f32) -> f32 {
    let d = abs(h - GG2_SAND_MHHW_M + lm_mx_noise(vec3<f32>(wp.x * 0.6, wp.z * 0.6, 8.9)) * 0.02);
    return lm_ss(0.05, 0.02, d);
}

// How far from the eye the footprints are drawn, metres: at the 1920 px
// capture's 50-60 degree field a 0.28 m print spans about 4 px here (none
// past it worth its 3 x 3 cell search per pixel).
const GG2_PRINTS_NEAR_M: f32 = 80.0;

// Footprints in the dry sand above the strand line: in walkers' trails (a
// slow field), one print per 0.8 m cell where a hash keeps it, each a
// scuffed oval 0.28 x 0.11 m (an adult shoe) turned along the trail; 1 in
// the print's hollow, 0 outside.
fn gg2_footprints(wp: vec3<f32>) -> f32 {
    let trail = lm_ss(0.15, 0.45, lm_mx_noise(vec3<f32>(wp.x * 0.035, wp.z * 0.035, 2.9)));
    if (trail <= 0.0) {
        return 0.0;
    }
    let cs = 0.8;
    let c = floor(wp.xz / cs);
    var best = 0.0;
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let cc = c + vec2<f32>(f32(i), f32(j));
            let h = lm_cell_hash(vec3<f32>(cc, 41.0));
            if (h > 0.55 * trail) {
                continue;
            }
            let h2 = lm_cell_hash(vec3<f32>(cc, 77.0));
            let h3 = lm_cell_hash(vec3<f32>(cc, 113.0));
            let centre = (cc + vec2<f32>(0.2 + 0.6 * h2, 0.2 + 0.6 * h3)) * cs;
            let a = lm_mx_noise(vec3<f32>(centre.x * 0.02, centre.y * 0.02, 1.3)) * 3.1 + (h2 - 0.5) * 0.6;
            let d = wp.xz - centre;
            let u = vec2<f32>(cos(a) * d.x + sin(a) * d.y, -sin(a) * d.x + cos(a) * d.y);
            let e = length(u / vec2<f32>(0.14, 0.055));
            best = max(best, lm_ss(1.0, 0.7, e));
        }
    }
    return best;
}

// The Fort Point seawall's coursed granite ashlar (GG2_ASHLAR, cc0.rs
// `ashlar_wgsl`): on the wall faces inside the revetment's box, courses of
// blocks of varied length, each its own shade of the granite's mean albedo,
// darker recessed joints. The wall's mottle and streaks (`lm_seawall`: its
// tone over the plain concrete's, `base`) carry onto the granite. The joints'
// footprint derivatives are taken in uniform control flow: the box and the
// face gate the result, not the code path.
fn gg2_ashlar(s: ptr<function, Surface>, world: vec3<f32>, n: vec3<f32>, faceness: f32, base: vec3<f32>) {
    let b = GG2_ASHLAR_BOX;
    let inside = world.x >= b.x && world.x <= b.z && world.z >= b.y && world.z <= b.w && faceness >= 0.5;
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let u = dot(world.xz, t);
    let course = floor((world.y - LM_SEA_LEVEL_Y) / GG2_ASHLAR_COURSE_M);
    let cv = fract((world.y - LM_SEA_LEVEL_Y) / GG2_ASHLAR_COURSE_M);
    let blen = mix(GG2_ASHLAR_BLOCK_M.x, GG2_ASHLAR_BLOCK_M.y, lm_cell_hash(vec3<f32>(course, 3.0, 11.0)));
    let uu = u / blen + lm_cell_hash(vec3<f32>(course, 5.0, 7.0));
    let block = floor(uu);
    let cu = fract(uu);
    // joints: about 2 cm, anti-aliased by the footprint
    let jw_v = 0.02 / GG2_ASHLAR_COURSE_M;
    let jw_u = 0.02 / blen;
    let fv = max(fwidth(cv), 1e-4);
    let fu = max(fwidth(uu), 1e-4);
    let jv = 1.0 - smoothstep(jw_v, jw_v + fv, min(cv, 1.0 - cv));
    let ju = 1.0 - smoothstep(jw_u, jw_u + fu, min(cu, 1.0 - cu));
    let joint = max(jv, ju);
    let shade = 0.85 + 0.3 * lm_cell_hash(vec3<f32>(course, block, 29.0));
    // the wall's own variation (mottle, streaks, sky-catch) relative to its plain tone
    let rel = clamp(dot((*s).albedo, vec3<f32>(0.2126, 0.7152, 0.0722)) / max(dot(base, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4), 0.5, 1.6);
    let stone = GG2_GRANITE * shade * (0.9 + 0.2 * lm_mx_noise(world * 1.7)) * rel;
    if (!inside) {
        return;
    }
    (*s).albedo = mix(stone, GG2_GRANITE * 0.35, joint);
    (*s).rough = mix((*s).rough, 0.95, joint);
}
