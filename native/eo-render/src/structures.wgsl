// The grade-separated roads' concrete (structures.rs, `ObjectKind::Structure`;
// appended to the city shader by look.rs after the SX_* constants).
//
// A concrete vertex carries its formwork coordinates (metres) and its wear
// (0 clean .. 1 stained) instead of a colour (structures.rs
// `encode_concrete`); its tint and finish are its material's (base colour,
// params.z). The vertex shader decodes the coordinates (`sx_decode`) into
// `ft` (u along the boards, v across them) and the wear into `aux.x`.
//
// What it draws, by face (reference sheet 5.10, Selby St under I-280; 1.5,
// 2.4, 6.A.8):
//  * undersides (soffit, overhang, cap undersides; normal down): form panel
//    joints along and across the boards, each panel's own tone, the boards'
//    grain; efflorescence and damp along the joints where the wear is high
//    (near deck joints);
//  * webs (sloped, facing down and out): the same panels, boards along;
//  * the edge band (the barrier's faces and the fascia flush below its back,
//    v within a metre and a half of the deck top): slip-formed, smooth, a faint
//    drag; grime toward the drip edge;
//  * walls (cap sides and ends, fill and abutment walls, end faces): panels;
//  * tops (barrier tops, cap tops): dirt;
//  * columns (`v` past SX_COLUMN_V: v - SX_COLUMN_V is the height above the
//    base, u the distance round): a smooth steel-formed shaft, lighter than
//    the rest, its form seams and lift joints, the splashed base and the
//    stained top under the cap; the Central Viaduct's banded rough/smooth
//    finish (SX_FINISH_BANDED).
// Stain streaks hang down every face but the undersides, as dark as the wear
// there says, broken by noise; a mottle in the surface's own coordinates
// varies the whole (and gives back the bents' brightness jitter). With the
// CC0 sets the board-formed concrete scan (wood_textured_concrete, 1.8 m)
// gives the fine texture and relief, its boards along the formwork; without
// them (or past SX_SCAN_FAR) a procedural board grain stands in.

// Form panels: 2.4 m along the boards, 1.2 m across, rows staggered by half
// a panel; their joints 12 mm wide. Boards 0.15 m. Column form segments 1.5 m
// round, lifts 3.66 m (12 ft). Banded finish bands 0.9 m. INFERRED (common
// US formwork sizes; the 597.6 m wrap of `u` is a multiple of each repeat
// along it).
const SX_PANEL_L: f32 = 2.4;
const SX_PANEL_W: f32 = 1.2;
const SX_JOINT_W: f32 = 0.012;
const SX_BOARD_W: f32 = 0.15;
const SX_SEAM_PITCH: f32 = 1.5;
const SX_LIFT_M: f32 = 3.66;
const SX_BAND_M: f32 = 0.9;
// the scan's detail fades out over this distance (a 1.8 m repeat is a few
// pixels past it and the mips average it to its mean).
const SX_SCAN_NEAR: f32 = 70.0;
const SX_SCAN_FAR: f32 = 120.0;
// efflorescence's colour (a white salt bloom over grey concrete).
const SX_BLOOM: vec3<f32> = vec3<f32>(0.62, 0.61, 0.58);

// The formwork coordinates of a concrete vertex (structures.rs `encode_concrete`).
fn sx_decode(color: vec4<f32>, w: f32) -> vec2<f32> {
    let b = round(color.rgb * 255.0);
    let hi = round(w * 127.0) + 127.0;
    return vec2<f32>((b.x + b.y * 256.0) * SX_QUANT, (b.z + hi * 256.0) * SX_QUANT - SX_V_BIAS);
}

// Coverage of a line `w` wide at distance `d`, box-filtered over a pixel `fw` wide.
fn sx_line(d: f32, w: f32, fw: f32) -> f32 {
    let f = max(fw, 1e-4);
    return clamp((0.5 * (w + f) - d) / f, 0.0, 1.0) * min(1.0, w / f);
}

// Distance from `x` to the nearest multiple of `pitch`.
fn sx_dist(x: f32, pitch: f32) -> f32 {
    return abs(fract(x / pitch + 0.5) - 0.5) * pitch;
}

fn sx_fbm(p: vec2<f32>) -> f32 {
    return st_noise(p) * 0.5 + st_noise(p * 2.03 + vec2<f32>(5.2, 1.3)) * 0.3 + st_noise(p * 4.11 + vec2<f32>(2.7, 9.1)) * 0.2;
}

struct SxFrame {
    t: vec3<f32>,
    b: vec3<f32>,
};

// The world directions of increasing (a, b) on the surface, from their
// screen derivatives (`da` = (d/dx, d/dy) of a): a cotangent frame.
fn sx_frame(n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, da: vec2<f32>, db: vec2<f32>) -> SxFrame {
    let p2 = cross(dpy, n);
    let p1 = cross(n, dpx);
    let t = p2 * da.x + p1 * da.y;
    let b = p2 * db.x + p1 * db.y;
    let k = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-24));
    return SxFrame(t * k, b * k);
}

// ---- weathering (stage A9, `--grime`; structures.rs SX_WX_*) -----------------
// Every term below runs only behind `wx_level() != 0u` at its call site
// (weathering.wgsl, the frame's grime bits): without `--grime` nothing here
// is reached. Albedo and roughness only.

// How weathered a structure's concrete is drawn: its era (structures.rs
// `ERA_SHIFT`: old 1955-1971 heavy, rebuilt 2005-2008 medium, new 2012-15
// nearly clean), a little more at the hyper level.
fn sx_era_k(object: u32) -> f32 {
    let e = (objects[object].kind >> SX_ERA_SHIFT) & 3u;
    let k = select(select(SX_ERA_OLD, SX_ERA_REBUILT, e == 1u), SX_ERA_NEW, e == 2u);
    return k * select(1.0, SX_WX_HYPER, wx_level() >= 2u);
}

// Run-off streaks hanging from a source line on a vertical face: `x` along
// the face (m), `dn` how far below the source (m), `len` their mean reach,
// `dens` the share of the face's columns streaked; once a column is under a
// few pixels (`fw`) their mean. Returns (cover, the streak's own hash).
fn sx_streaks(x: f32, dn: f32, len: f32, dens: f32, fw: f32, seed: f32) -> vec2<f32> {
    let cw = SX_WX_STREAK_COL_M;
    let col = floor(x / cw);
    let h = st_hash(vec2<f32>(col, seed));
    let cx = 0.5 + (st_hash(vec2<f32>(col, seed + 3.1)) - 0.5) * 0.5;
    let wdt = 0.12 + 0.3 * st_hash(vec2<f32>(col, seed + 5.7));
    let l = len * (0.3 + 1.0 * st_hash(vec2<f32>(col, seed + 7.3)));
    let wob = st_noise(vec2<f32>(x * 2.3 + seed, dn * 0.6)) - 0.5;
    let core = 1.0 - smoothstep(0.3, 1.0, abs(x / cw - col - cx + wob * 0.2) / wdt);
    let fall = (1.0 - smoothstep(0.15 * l, l, dn)) * smoothstep(-0.05, 0.1, dn);
    let body = 0.55 + 0.45 * st_noise(vec2<f32>(x * 6.0 + seed, dn * 1.7));
    let one = select(0.0, 1.0, h < dens) * core * fall * body;
    let mean = dens * 0.3 * (1.0 - smoothstep(0.0, len, dn)) * smoothstep(-0.05, 0.1, dn);
    return vec2<f32>(mix(mean, one, wx_keep(cw * 0.6, fw)), st_hash(vec2<f32>(col, seed + 11.9)));
}

// The freeway concrete aged (reference sheet 5.10: the Selby St viaduct's
// "darker vertical stain streaks on cap beam faces and column tops,
// efflorescence blotches on the soffit", Mariposa St's "grime streaks
// running down from deck joints and drains", heavy at the 1960s ramps, the
// Presidio Parkway's "very light, clean concrete"; sheet 1.5 AASHTO defect
// 1120: white efflorescence, build-up, then rust staining), driven by A2's
// wear (high at joints, drains, column tops and bases, cap faces under
// joints) and the face it is on: soiling in water-stained blotches; run-off
// streaks down the fascia from the barrier top and on down the girder from
// the drip edge, down cap faces and walls, down columns from under the caps
// (some rust-coloured under joints and bearings); efflorescence leaching
// white out of the soffit's joints and down the caps under the deck joints;
// splash-back and soil at column feet and wall feet; grime along the
// barrier's toe; dirt on what faces up.
struct SxFace {
    column: bool,
    under: bool,
    web: bool,
    top: bool,
    band: bool,
    wall: bool,
    along: f32,
    across: f32,
    f_al: f32,
    f_ac: f32,
    joint: f32,
};

fn sx_weather(albedo: ptr<function, vec3<f32>>, rough: ptr<function, f32>, in: VOut, n: vec3<f32>, f: SxFace, k: f32) {
    let wear = clamp(in.aux.x / 255.0, 0.0, 1.0);
    let v = in.ft.y;
    var a = *albedo;
    var r = *rough;
    // soiling, in broad water-stained blotches (a few metres), deeper with the wear.
    let blot = smoothstep(0.25, 0.75, sx_fbm(in.world.xz * 0.19 + vec2<f32>(in.world.y * 0.27, 1.7)));
    // (an underside, sheltered from the rain and already in the rig's shade, a third of it:
    // the soffit "is not black", reference sheet 5.10.)
    let soil = SX_WX_SOIL * k * (0.35 + 0.6 * blot + 0.35 * wear) * select(1.0, 0.33, f.under);
    a = a * (1.0 - min(soil, 0.6));
    a = mix(a, vec3<f32>(dot(a, CC0_LUMA)) * SX_WX_DIRT_TINT, 0.3 * k);
    var streak = vec2<f32>(0.0);
    var stain = vec3<f32>(0.0);
    var bloom = 0.0;
    if (f.band) {
        if (n.y > 0.08) {
            // the barrier's traffic face: road spray, tyre and brake dust along its toe.
            let toe = 1.0 - smoothstep(0.0, 0.3 + 0.15 * st_noise(vec2<f32>(f.along * 0.9, 2.1)), v);
            a = a * (1.0 - SX_WX_TOE * k * toe * (0.7 + 0.3 * st_noise(vec2<f32>(f.along * 4.0, v * 9.0))));
            r = min(r + 0.06 * toe, 1.0);
        } else {
            // the fascia and the barrier's back: run-off from the barrier's top
            // (more and longer where the wear says a drain or a joint is), grime
            // gathering toward the drip edge.
            streak = sx_streaks(f.along, SX_WX_BAND_TOP - v, 1.3 + 1.5 * wear, (0.35 + 0.7 * wear) * k, f.f_al, 3.7);
            let drip = 1.0 - smoothstep(-0.3, 0.35, v);
            a = a * (1.0 - SX_WX_DRIP * k * drip * (0.6 + 0.4 * st_noise(vec2<f32>(f.along * 1.3, 4.4))));
        }
    } else if (f.web) {
        // the girder's face under the drip edge: the run-off that got past it.
        streak = sx_streaks(f.along, -0.3 - v, 1.6 + 2.4 * wear, (0.25 + 0.65 * wear) * k, f.f_al, 9.1) * vec2<f32>(0.85, 1.0);
    } else if (f.wall) {
        // cap sides, walls, abutments: run-off from above, heavier under joints (wear).
        if (v < 0.0) {
            streak = sx_streaks(f.along, -v - 0.2, 2.5 + 3.5 * wear, (0.3 + 0.7 * wear) * k, f.f_al, 5.3);
        } else {
            // (a face whose `v` climbs from the road: a headwall, a cut's wall. Its
            // run-off comes from far above: long streaks broken into runs down it.)
            let runs = smoothstep(0.35, 0.6, st_noise(vec2<f32>(f.along * 0.8 + 2.0, in.world.y * 0.16)));
            streak = sx_streaks(f.along, 0.6 + 2.0 * fract(in.world.y * 0.071 + st_hash(vec2<f32>(floor(f.along / SX_WX_STREAK_COL_M), 4.0))), 3.0, (0.35 + 0.6 * wear) * k, f.f_al, 5.9) * vec2<f32>(runs, 1.0);
        }
        // splash and soil at a wall's foot (the wear the walls carry there).
        let sp = smoothstep(0.42, 0.62, wear) * (0.6 + 0.4 * st_noise(vec2<f32>(f.along * 1.4, in.world.y * 3.0)));
        a = a * mix(vec3<f32>(1.0), WX_SPLASH_TINT * (1.0 - WX_SPLASH_DARK), sp * k);
    } else if (f.column) {
        // from under the cap down the shaft: each streak reaches as far down as
        // the wear (rising toward the top) is past its own threshold.
        let cw = SX_WX_STREAK_COL_M * 0.6;
        let col = floor(f.across / cw);
        let t = 0.3 + 0.38 * st_hash(vec2<f32>(col, 2.9));
        let fx = f.across / cw - col - 0.5 - (st_hash(vec2<f32>(col, 4.3)) - 0.5) * 0.4;
        let wdt = 0.12 + 0.25 * st_hash(vec2<f32>(col, 6.1));
        let core = 1.0 - smoothstep(0.3, 1.0, abs(fx) / wdt);
        let one = core * smoothstep(t, t + 0.12, wear) * (0.55 + 0.45 * st_noise(vec2<f32>(f.across * 5.0, f.along * 1.3)));
        let dens = 0.55 + 0.4 * k;
        let on = select(0.0, 1.0, st_hash(vec2<f32>(col, 8.8)) < dens);
        streak = vec2<f32>(mix(dens * 0.35 * smoothstep(0.3, 0.7, wear), on * one, wx_keep(cw * 0.6, f.f_ac)) * k, st_hash(vec2<f32>(col, 1.3)));
        // splash-back over the ground (the column's base is buried 0.5 m): 0.3-1 m, soil-tinted, ragged.
        let h = f.along - 0.5;
        let reach = mix(WX_SPLASH_LO, WX_SPLASH_HI, k) * (0.75 + 0.5 * st_noise(vec2<f32>(f.across * 1.9, 5.5)));
        let sp = (1.0 - smoothstep(0.0, reach, h)) * smoothstep(-0.6, 0.0, h) * (0.65 + 0.35 * st_noise(vec2<f32>(f.across * 6.0, h * 8.0)));
        a = a * mix(vec3<f32>(1.0), WX_SPLASH_TINT * (1.0 - WX_SPLASH_DARK), sp * (0.5 + 0.5 * k));
        r = min(r + 0.06 * sp, 1.0);
    } else if (f.under) {
        // the soffit: salts leach white out of the form joints and round the
        // deck joints (the wear), damp darker patches between.
        let leach = f.joint * smoothstep(0.3, 0.6, wear) * (0.4 + 0.6 * st_noise(vec2<f32>(f.along * 1.7, f.across * 1.7)));
        let damp = smoothstep(0.5, 0.75, sx_fbm(vec2<f32>(f.along, f.across) * 0.35 + vec2<f32>(4.1, 0.3))) * smoothstep(0.35, 0.65, wear);
        bloom = max(bloom, (leach * 0.9 + damp * 0.5) * k);
        a = a * (1.0 - 0.1 * k * smoothstep(0.4, 0.7, sx_fbm(vec2<f32>(f.along, f.across) * 0.21 + 2.0)));
    } else if (f.top) {
        // dirt and windblown grit on what faces up, greener on old copings.
        let d = 0.6 + 0.4 * st_noise(in.world.xz * 1.3);
        a = a * (1.0 - SX_WX_TOP * k * d);
        a = mix(a, a * SX_WX_MOSS, 0.35 * k * smoothstep(0.55, 0.8, st_noise(in.world.xz * 0.7 + 9.0)));
    }
    // the streaks: grime, rust where a joint or a bearing bleeds (the wear high
    // there and the structure old), white efflorescence runs below joints.
    if (streak.x > 0.0) {
        let rusty = streak.y < SX_WX_RUST * k * smoothstep(0.55, 0.8, wear);
        let white = !rusty && streak.y > 1.0 - SX_WX_LEACH * k * smoothstep(0.6, 0.85, wear);
        let c = clamp(streak.x, 0.0, 1.0) * SX_WX_STREAK * (0.6 + 0.4 * min(k, 1.2));
        if (white) {
            bloom = max(bloom, c * 1.2);
        } else {
            a = a * mix(vec3<f32>(1.0), select(SX_WX_STREAK_TINT, WX_RUST_TINT, rusty), c);
            r = min(r + 0.05 * c, 1.0);
        }
    }
    a = mix(a, SX_BLOOM, clamp(bloom, 0.0, 0.75) * 0.65);
    r = mix(r, 0.97, clamp(bloom, 0.0, 1.0) * 0.5);
    *albedo = a;
    *rough = r;
}

// A tunnel bore's lining aged (`--grime` with `--tunnels`): soot in the
// grout, over the vault and round the luminaire (the exhaust rises and the
// lamps' warmth draws it), road spray dirtying the bottom metre of the walls,
// seepage streaks down from the vault, an uneven glaze (some tiles crazed and
// dull, a few stained or replaced). `v` the height over the roadway on the
// walls; `grout` the grout's cover; `cell` the tile's.
fn sx_tile_weather(albedo: ptr<function, vec3<f32>>, rough: ptr<function, f32>, in: VOut, n: vec3<f32>, grout: f32, cell: vec2<f32>, f_u: f32, k: f32) {
    let u = in.ft.x;
    let v = in.ft.y;
    var a = *albedo;
    var r = *rough;
    let wall = abs(n.y) < 0.6;
    let h3 = st_hash(cell + vec2<f32>(17.3, 2.2));
    let h4 = st_hash(cell * vec2<f32>(0.7, 1.9) + vec2<f32>(5.5, 13.1));
    // the glaze: crazed and dulled on some tiles, a few stained, a few replaced
    // (each tile its own only while tiles are a few pixels across).
    let res = clamp(2.0 - 6.0 * f_u / SX_TILE_M, 0.0, 1.0);
    r = r * (1.0 + (0.2 * h4 - 0.05) * res) + select(0.0, 0.08 * res, h4 > 1.0 - SX_WX_TILE_DULL * k);
    if (h3 < SX_WX_TILE_STAIN * k) {
        a = mix(a, a * SX_WX_TILE_STAIN_TINT, res);
    } else if (h3 > 1.0 - SX_WX_TILE_NEW) {
        a = mix(a, vec3<f32>(dot(a, CC0_LUMA)) * 1.04, 0.4 * res);
    }
    // soot: the grout first, then the vault and round the luminaire, streaky along the bore.
    let up = smoothstep(0.2, 0.95, -n.y);
    let along = 0.55 + 0.45 * st_noise(vec2<f32>(u * 0.07, v * 2.5 + 1.0));
    let soot = k * (SX_WX_SOOT * up * along + SX_WX_SOOT * 0.6 * smoothstep(0.96, 0.995, -n.y));
    a = a * (1.0 - min(soot, 0.7));
    a = mix(a, a * (1.0 - SX_WX_GROUT_SOOT * k * (0.6 + 0.4 * up)), grout);
    if (wall) {
        // road spray: the bottom metre, densest low, a ragged top and fine spatter.
        let top = 1.0 + 0.35 * st_noise(vec2<f32>(u * 0.45, 3.3));
        let spray = (1.0 - smoothstep(0.15, top, v)) * (0.6 + 0.4 * st_noise(vec2<f32>(u * 3.1, v * 5.0)));
        a = mix(a, a * SX_WX_SPRAY_TINT, clamp(spray * k, 0.0, 0.85));
        r = mix(r, 0.75, spray * k * 0.7);
        // seepage from the vault's joints down the walls (rust or salt).
        let s = sx_streaks(u, SX_WX_SPRING_M - v, 2.2, 0.12 * k, f_u, 21.7);
        let rusty = s.y < 0.5;
        a = mix(a, select(a * 1.2 + vec3<f32>(0.04), a * WX_RUST_TINT, rusty), clamp(s.x * 0.6, 0.0, 0.6));
    }
    *albedo = clamp(a, vec3<f32>(0.02), vec3<f32>(0.85));
    *rough = clamp(r, 0.08, 1.0);
}

// A tunnel bore's lining (stage A8, `--tunnels`; the material's finish
// SX_FINISH_TILE): glossy small cream-yellow glazed tiles over the whole wall
// and vault, slightly irregular (reference sheet 5.10, Broadway Tunnel
// interior; 1.7 "tiled walls and ceiling"). `u` runs along the bore (its
// station), `v` up the walls (height over the roadway) and across the vault's
// flatter part (its lateral offset), so the courses run along the bore. A
// stack-bonded grid of SX_TILE_M tiles (US 4-1/4 in wall tile, fitted to a
// whole number of tiles in SX's u period) in recessed grout lines, each tile's
// own glaze tone and a slight tilt (the glaze's waviness, so the luminaire's
// streak breaks up from tile to tile), a cushion edge into the grout, soot and
// road grime toward the walkway (the vertex wear) and over the luminaire.
// the luminaire's image in the lining's glaze (sx_tile; unit luminance x Fresnel x cover).
var<private> sx_glint: vec3<f32>;

fn sx_tile(in: VOut, m: Material, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, du: vec2<f32>, dv: vec2<f32>) -> Surface {
    var s: Surface;
    let u = in.ft.x;
    let v = in.ft.y;
    let wear = clamp(in.aux.x / 255.0, 0.0, 1.0);
    let f_u = abs(du.x) + abs(du.y);
    let f_v = abs(dv.x) + abs(dv.y);
    // the grout: lines SX_GROUT_M wide between the tiles, box-filtered over the pixel.
    let gu = sx_line(sx_dist(u, SX_TILE_M), SX_GROUT_M, f_u);
    let gv = sx_line(sx_dist(v, SX_TILE_M), SX_GROUT_M, f_v);
    let grout = max(gu, gv);
    // a tile's own glaze tone and tilt (resolved only while its tiles are a few pixels across).
    let cell = vec2<f32>(floor(u / SX_TILE_M), floor(v / SX_TILE_M));
    let resolved = clamp(2.0 - 6.0 * max(f_u, f_v) / SX_TILE_M, 0.0, 1.0);
    let h1 = st_hash(cell + vec2<f32>(3.7, 1.9));
    let h2 = st_hash(cell * vec2<f32>(1.3, 2.1) + vec2<f32>(7.1, 4.4));
    var tone = 1.0 + (h1 - 0.5) * 0.07 * resolved;
    // slow variation over the lining (replacements, the glaze's batches) and grime with the wear.
    let mott = sx_fbm(vec2<f32>(u * 0.21, v * 0.9)) - 0.5;
    tone = tone * (1.0 + 0.06 * mott) * (1.0 - 0.32 * wear * (0.7 + 0.6 * st_noise(vec2<f32>(u * 0.37, v * 1.7))));
    // soot over the lamps: the vault's flat crown (facing down) darkens either side of the luminaire.
    let over = smoothstep(0.8, 0.97, -n.y);
    tone = tone * (1.0 - 0.12 * over);
    var albedo = m.base.rgb * tone;
    albedo = mix(albedo, SX_GROUT_RGB * (1.0 - 0.4 * wear), grout);
    // the glaze: glossy; the grout matte.
    var rough = mix(m.params.x * (0.85 + 0.3 * h2), 0.9, grout);
    rough = mix(rough, min(rough + 0.2, 1.0), wear * 0.6);
    if (wx_level() != 0u) {
        sx_tile_weather(&albedo, &rough, in, n, grout, cell, f_u, sx_era_k(in.object)); // stage A9, `--grime`
    }
    // each tile tilted a little, its cushion edge rolling into the grout.
    let ru = fract(u / SX_TILE_M) - 0.5;
    let rv = fract(v / SX_TILE_M) - 0.5;
    let edge = SX_TILE_EDGE_M / SX_TILE_M;
    let cu = sign(ru) * smoothstep(0.5 - edge, 0.5, abs(ru)) * clamp(1.0 - 2.0 * f_u / SX_TILE_M, 0.0, 1.0);
    let cv = sign(rv) * smoothstep(0.5 - edge, 0.5, abs(rv)) * clamp(1.0 - 2.0 * f_v / SX_TILE_M, 0.0, 1.0);
    let tn = vec2<f32>((h1 - 0.5) * 0.006 * resolved + cu * 0.12, (h2 - 0.5) * 0.006 * resolved + cv * 0.12);
    s.albedo = clamp(albedo, vec3<f32>(0.02), vec3<f32>(0.85));
    s.rough = clamp(rough, 0.08, 1.0);
    s.metal = 0.0;
    s.ao = 1.0 - 0.5 * grout;
    s.alpha = 1.0;
    s.env = -1;
    s.n = n;
    let tl = dot(tn, tn);
    if (tl > 1e-6) {
        let fr = sx_frame(n, dpx, dpy, du, dv);
        let tz = sqrt(max(0.0, 1.0 - min(tl, 0.9)));
        s.n = normalize(n * tz + fr.t * tn.x + fr.b * tn.y);
    }
    // the luminaire's image in the glaze: on a wall (`v` its height over the
    // roadway) where the reflected ray climbs to the crown's line,
    // SX_BORE_REACH_M across the bore, at the luminaire's face (sheet 5.10:
    // "its specular streak repeats down the glossy walls"); broken from tile to
    // tile by their tilt, none in the grout. sx_bore_light lights it.
    sx_glint = vec3<f32>(0.0);
    if (abs(n.y) < 0.6) {
        let vw = normalize(frame.cam_pos.xyz - in.world);
        let r = reflect(-vw, s.n);
        let rh = dot(r, normalize(vec3<f32>(n.x, 0.0, n.z)));
        if (rh > 0.02 && r.y > 0.0) {
            let t = SX_BORE_REACH_M / rh;
            let path = t + length(frame.cam_pos.xyz - in.world);
            let miss = (v + r.y * t - SX_LAMP_FACE_M) / path;
            // the diffuser's angular half size, widened by the glaze's lobe (GGX alpha):
            // a sharp line where the glaze is the smoother, spread and dimmer where not.
            let a = max(s.rough * s.rough, 1e-3);
            let size = SX_LAMP_HALF_M / path;
            let cover = min(1.0, size / a) * (1.0 - smoothstep(size, size + 2.0 * a, abs(miss)));
            let fr = 0.04 + 0.96 * pow(1.0 - clamp(dot(s.n, vw), 0.0, 1.0), 5.0);
            sx_glint = SX_LAMP_RGB * (fr * cover * (1.0 - grout));
        }
    }
    return s;
}

// `du`, `dv`: the screen derivatives of in.ft.x and in.ft.y (taken in `shade`).
fn sx_concrete(in: VOut, m: Material, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, du: vec2<f32>, dv: vec2<f32>) -> Surface {
    var s: Surface;
    let u = in.ft.x;
    let v = in.ft.y;
    let wear = clamp(in.aux.x / 255.0, 0.0, 1.0);
    let finish = u32(m.params.z + 0.5);
    if (finish == SX_FINISH_TILE) {
        return sx_tile(in, m, n, dpx, dpy, du, dv); // a tunnel bore's lining
    }
    let stain = select(1.0, 0.4, finish == SX_FINISH_CLEAN);
    let dist = length(in.rel);
    let column = v > SX_COLUMN_V - 50.0;
    let under = !column && n.y < -0.6;
    let web = !column && n.y >= -0.6 && n.y < -0.15;
    let top = !column && n.y >= 0.6;
    let side = !column && !under && !web && !top;
    let band = side && v > -0.35 && v < 1.4;
    let wall = side && !band;
    // the surface's coordinates: `along` the boards and `across` them, and their pixel footprints.
    var along = u;
    var across = v;
    var d_al = du;
    var d_ac = dv;
    if (column) {
        along = v - SX_COLUMN_V;
        across = u;
        d_al = dv;
        d_ac = du;
    }
    let f_al = abs(d_al.x) + abs(d_al.y);
    let f_ac = abs(d_ac.x) + abs(d_ac.y);
    var tone = 1.0;
    var rough = m.params.x;
    var ao = 1.0;
    var tn = vec2<f32>(0.0);

    // Mottle (the pour's own variation), and a slow one across a few bents.
    let mott = sx_fbm(vec2<f32>(along, across) * 0.37) - 0.5;
    let slow = st_noise(in.world.xz * 0.021 + vec2<f32>(in.world.y * 0.013, 3.7)) - 0.5;
    tone = tone * (1.0 + 0.13 * mott + 0.08 * slow);
    // General grime with the wear (an underside is sheltered from the rain: half of it).
    tone = tone * (1.0 - select(0.24, 0.12, under) * wear * stain);

    // Form panels on the undersides, webs and walls.
    var joint = 0.0; // (its cover is kept for the weathering, stage A9)
    if (under || web || wall) {
        let row = floor(across / SX_PANEL_W);
        let stagger = select(0.0, 0.5 * SX_PANEL_L, (i32(row) & 1) == 1);
        let pa = along + stagger;
        joint = max(sx_line(sx_dist(across, SX_PANEL_W), SX_JOINT_W, f_ac), sx_line(sx_dist(pa, SX_PANEL_L), SX_JOINT_W, f_al));
        let cell = vec2<f32>(floor(pa / SX_PANEL_L), row);
        let resolved = clamp(2.0 - 4.0 * max(f_al / SX_PANEL_L, f_ac / SX_PANEL_W), 0.0, 1.0);
        tone = tone * (1.0 + (st_hash(cell * vec2<f32>(1.7, 3.1) + vec2<f32>(11.3, 4.9)) - 0.5) * 0.09 * resolved);
        tone = tone * (1.0 - joint * (0.16 + 0.3 * wear * stain));
    }
    // Columns: lighter smooth shafts (steel forms), their seams and lift joints.
    if (column) {
        let h = along;
        tone = tone * 1.07;
        rough = rough * 0.9;
        let seam = sx_line(sx_dist(across, SX_SEAM_PITCH), 0.008, f_ac);
        let lift = sx_line(sx_dist(h - 0.5, SX_LIFT_M), 0.015, f_al);
        let pour = st_hash(vec2<f32>(floor((h - 0.5) / SX_LIFT_M), 7.0) + in.world.xz * 0.01) - 0.5;
        tone = tone * (1.0 - 0.07 * seam - 0.12 * lift) * (1.0 + 0.05 * pour * clamp(2.0 - 2.0 * f_al, 0.0, 1.0));
        if (finish == SX_FINISH_BANDED) {
            // the Central Viaduct's bands: rough (fractured-fin) bands between smooth ones.
            let b = fract((h - 0.5) / SX_BAND_M);
            let cover = clamp(f_al / (0.25 * SX_BAND_M), 0.0, 1.0);
            let rough_band = mix(smoothstep(0.46, 0.5, b) * (1.0 - smoothstep(0.96, 1.0, b)), 0.5, cover);
            let flute = mix(0.5 + 0.5 * sin(across * 6.2831853 / 0.05), 0.5, clamp(f_ac / 0.03, 0.0, 1.0));
            tone = tone * (1.0 - rough_band * (0.07 + 0.06 * flute));
            rough = mix(rough, 0.96, rough_band);
            tn = tn + vec2<f32>(cos(across * 6.2831853 / 0.05) * 0.35 * rough_band * (1.0 - clamp(f_ac / 0.03, 0.0, 1.0)), 0.0);
        }
        // the splash zone just above the ground (the base is 0.5 m under it).
        let splash = (1.0 - smoothstep(0.5, 1.4, h)) * (0.6 + 0.4 * st_noise(vec2<f32>(across * 3.1, h * 5.0)));
        tone = tone * (1.0 - 0.22 * splash * stain);
    }
    // The edge band: slip-formed and smooth, a faint drag; darker toward the
    // drip edge below; the barrier's construction joint on the deck (v 0).
    if (band) {
        let drag = st_noise(vec2<f32>(along * 0.4, across * 23.0)) - 0.5;
        tone = tone * (1.0 + 0.035 * drag * clamp(1.0 - f_ac / 0.05, 0.0, 1.0));
        tone = tone * (1.0 - 0.14 * stain * (1.0 - smoothstep(-0.3, 0.6, v)));
        tone = tone * (1.0 - 0.18 * sx_line(abs(v + 0.01), 0.012, f_ac));
    }
    if (top) {
        tone = tone * (1.0 - 0.12 * stain); // dirt on what faces up
    }
    // Streaks down every face but the undersides: long, broken, as dark as the wear.
    if (!under) {
        let hz = select(u, across, column);
        let fh = select(f_al, f_ac, column);
        let y = in.world.y;
        // sparse where the face is clean, crowding in where the wear is high
        // (a drain's streak, a joint's run-off), never a regular stripe.
        let n1 = st_noise(vec2<f32>(hz * 0.9, y * 0.09));
        let n2 = st_noise(vec2<f32>(hz * 4.3 + 3.1, y * 0.3));
        var streak = smoothstep(0.6, 0.92, n1 * 0.6 + n2 * 0.4 + 0.45 * (wear - 0.45));
        streak = mix(streak, 0.12, clamp(fh / 0.3, 0.0, 1.0));
        // faint where the face is clean (a column's shaft), dark under joints, drains and caps.
        tone = tone * (1.0 - streak * (0.04 + 0.6 * max(wear - 0.15, 0.0)) * stain);
        rough = mix(rough, min(rough + 0.04, 1.0), streak);
    }
    var albedo = m.base.rgb * tone;
    // Undersides: efflorescence blooms and damp along the joints, where the wear is high.
    if (under) {
        let bloom = smoothstep(0.56, 0.84, sx_fbm(vec2<f32>(along, across) * 0.42 + vec2<f32>(7.0, 2.0))) * smoothstep(0.3, 0.75, wear) * stain;
        albedo = mix(albedo, SX_BLOOM, bloom * 0.42);
    }

    // The board-formed scan (with the CC0 sets), else a procedural board grain.
    let fade = 1.0 - smoothstep(SX_SCAN_NEAR, SX_SCAN_FAR, dist);
    // how much of the boards shows: most on the formed faces, little on the
    // slip-formed band and the steel-formed columns.
    var contrast = select(select(0.45, 0.5, under || web), 0.25, band || top);
    if (column) {
        contrast = 0.12;
    }
    contrast = contrast * select(1.0, 0.6, finish == SX_FINISH_CLEAN);
    if (fade > 0.0 && cc0_on(CC0_RUN_WALLS)) {
        let la = CC0_L_WOOD_TEXTURED_CONCRETE;
        let inv = cc0_linv(la);
        // scan x across the boards, y along them (its boards run up the image).
        let q = vec2<f32>(across, along) * inv;
        let gx = vec2<f32>(d_ac.x, d_al.x) * inv;
        let gy = vec2<f32>(d_ac.y, d_al.y) * inv;
        let t = cc0_tap(q, gx, gy, la);
        let mean = cc0_lmean(la).rgb;
        // its luminance only (the scan is browner than SF's grey concrete), its bright flecks held in.
        let rl = dot(t.alb.rgb, CC0_LUMA) / max(dot(mean, CC0_LUMA), 1e-3);
        let detail = vec3<f32>(clamp(rl, 0.7, 1.2));
        let k = contrast * fade;
        albedo = albedo * mix(vec3<f32>(1.0), detail, k);
        rough = rough * mix(1.0, clamp(t.rough / max(cc0_lmisc(la).x, 0.05), 0.75, 1.25), k);
        ao = mix(1.0, clamp(t.ao / max(cc0_lmisc(la).y, 0.05), 0.5, 1.0), k);
        tn = tn + vec2<f32>(t.n.x, -t.n.y) * (0.8 * k);
    } else if (fade > 0.0) {
        // boards: their seams, each board's tone, a grain along it.
        let bi = floor(across / SX_BOARD_W);
        let seam = sx_line(sx_dist(across, SX_BOARD_W), 0.004, f_ac);
        let grain = st_noise(vec2<f32>(along * 1.3, across * 40.0)) - 0.5;
        let k = contrast * fade * clamp(1.5 - 6.0 * f_ac / SX_BOARD_W, 0.0, 1.0);
        albedo = albedo * (1.0 + k * ((st_hash(vec2<f32>(bi, 3.3)) - 0.5) * 0.07 + grain * 0.05 - seam * 0.1));
    }
    if (wx_level() != 0u) {
        // stage A9 (`--grime`): aged as its era says.
        sx_weather(&albedo, &rough, in, n, SxFace(column, under, web, top, band, wall, along, across, f_al, f_ac, joint), sx_era_k(in.object));
    }
    s.albedo = clamp(albedo, vec3<f32>(0.02), vec3<f32>(0.85));
    s.rough = clamp(rough, 0.3, 1.0);
    s.metal = 0.0;
    s.ao = ao;
    s.alpha = 1.0;
    s.env = -1;
    s.n = n;
    let tl = dot(tn, tn);
    if (tl > 1e-6) {
        let fr = sx_frame(n, dpx, dpy, d_ac, d_al);
        let tz = sqrt(max(0.0, 1.0 - min(tl, 0.9)));
        s.n = normalize(n * tz + fr.t * tn.x + fr.b * tn.y);
    }
    return s;
}

// ---- inside a tunnel's bore (stage A8, `--tunnels`) ------------------------
// A bore's parts (structures.rs `Part::Bore*`, `Part::Luminaire`: the object
// kind's SX_BORE_BIT) stand under the hill: neither the sun nor the open sky
// reach them, so `shade` (draw.wgsl) lights them here instead of with
// `look_light`. What lights them: the bore's own lamps, on round the clock
// (the clustered lights, sky_lights.wgsl, with any headlamps), the light the
// lined bore throws back of them (an even interreflection, and its image in
// the glaze), and the daylight its portal lets in by the depth in (the sky's
// share of the open fill; never the sun). The luminaire strip's housing
// glows. With the lights off (`--no-lights`) only the portal's daylight is left.

// SF_TUNNELS false (look.rs `ShaderFeatures`: no `--tunnels`, so no bore) compiles the bore's lighting out.
fn sx_in_bore(object: u32) -> bool {
    return SF_TUNNELS && (objects[object].kind & SX_BORE_BIT) != 0u;
}

// The share of the open sky's fill the bore's portal lets in at `world`
// (structures.rs `encode_portal`: the object's terrain-only words carry its
// portal; none: 0): half the open sky's at the face, falling off with the
// depth in as a mouth of radius SX_PORTAL_R2's root seen from its axis.
fn sx_portal_sky(object: u32, world: vec3<f32>) -> f32 {
    return sx_portal_sky_at(sx_portal_depth(object, world));
}

fn sx_portal_sky_at(d: f32) -> f32 {
    if (d < 0.0) {
        return 0.0;
    }
    return SX_PORTAL_SKY * SX_PORTAL_R2 / (SX_PORTAL_R2 + d * d) * (1.0 - smoothstep(0.7 * SX_PORTAL_DAY_M, SX_PORTAL_DAY_M, d));
}

// How far in from its portal `world` lies (the object's terrain-only words,
// structures.rs `encode_portal`); -1 without a portal.
fn sx_portal_depth(object: u32, world: vec3<f32>) -> f32 {
    let o = objects[object];
    if ((o.far_index_count & SX_PORTAL_VALID) == 0u) {
        return -1.0;
    }
    let p = o.min.xz - vec2<f32>(SX_PORTAL_MARGIN_M) + vec2<f32>(f32(o.far_first_index & 0xffffu), f32(o.far_first_index >> 16u)) / SX_PORTAL_QUANT;
    let a = f32(o.far_index_count & 0xffffu) / 65536.0 * 6.2831853;
    return max(dot(world.xz - p, vec2<f32>(cos(a), sin(a))), 0.0);
}

// Stage A9: the threshold zone's lighting over the interior's at `d` in from
// the portal (structures.rs `threshold_share`, `threshold_over_interior`):
// by day the threshold and transition zones' level (CIE 88), by night 1.
fn sx_threshold(d: f32) -> f32 {
    if (d < 0.0 || SX_TH_OVER <= 1.0) {
        return 1.0;
    }
    let z = SX_TH_ZONE_M;
    var share = 1.0;
    if (d > z) {
        share = select(0.0, pow(1.9 + (d - z) / max(SX_TH_SPEED, 1e-3), -1.4), SX_TH_SPEED > 0.0);
    } else if (d > 0.5 * z) {
        share = 1.0 - 0.6 * (d - 0.5 * z) / (0.5 * z);
    }
    return mix(max(1.0, SX_TH_OVER * share), 1.0, clamp(skyu.night.x, 0.0, 1.0));
}

fn sx_bore_light(s: Surface, m: Material, v: vec3<f32>, world: vec3<f32>, object: u32) -> vec4<f32> {
    return sx_bore_light_at(s, m, v, world, objects[object].kind, sx_portal_depth(object, world));
}

// `sx_bore_light` for a surface of object kind word `kind` (its
// `SX_LUMINAIRE_BIT`) `d` in from its portal (-1: none in reach): what a
// pipeline without the city's object table calls (stage A9; for a car or a
// person in a bore, the agents' shader, with `d` from its feed).
fn sx_bore_light_at(s: Surface, m: Material, v: vec3<f32>, world: vec3<f32>, kind: u32, d: f32) -> vec4<f32> {
    let n = s.n;
    let rough = clamp(s.rough, 0.03, 1.0);
    let diffuse = s.albedo * (1.0 - s.metal);
    let f0 = mix(vec3<f32>(0.04), s.albedo, s.metal);
    let nv = max(dot(n, v), 1e-4);
    let dfg = lk_dfg(nv, rough);
    let fss = f0 * dfg.x + vec3<f32>(dfg.y);
    // the bore's lamps (and headlamps), direct.
    var lit = sky_local_lights(world, n, v, diffuse, f0, rough);
    // the interior's roadway irradiance in the rig's units (sky.rs `Sky.twi.w`:
    // structures.rs `bore_interior_units`; 0 with the lights off).
    let on = sky_on(skyu.flags.x, SKY_LIGHTS) && skyu.lights.x != 0u && frame.copy_step.z < 0.5;
    let e_int = select(0.0, skyu.twi.w, on);
    // stage A9: by day the threshold zone's added luminaires near the portal
    // (an even light over the lamps' pools, most on the roadway), the
    // luminaire and its image in the glaze brighter with them.
    let th = sx_threshold(d);
    let extra = (th - 1.0) * e_int * (0.6 + 0.4 * max(n.y, 0.0));
    lit = lit + extra * (diffuse / LK_PI * (vec3<f32>(1.0) - fss) + fss / LK_PI) * s.ao;
    // the lined bore's interreflection of the lamps: even, and mirrored in the glaze;
    // the luminaire's own image in the glaze (sx_tile's streak on the walls).
    let e = SX_BORE_BOUNCE * e_int * th;
    lit = lit + e * diffuse / LK_PI * (vec3<f32>(1.0) - fss) * s.ao + vec3<f32>(e / LK_PI) * fss * s.ao;
    lit = lit + sx_glint * (SX_LUMINAIRE_OVER * e_int * th);
    // the daylight through the portal: the sky's share of the open fill.
    let day = sx_portal_sky_at(d);
    if (day > 0.0) {
        let hemi = mix(look.hemi_ground.xyz, look.hemi_sky.xyz, n.y * 0.5 + 0.5) + look.ambient.xyz;
        lit = lit + day * hemi * diffuse / LK_PI * (vec3<f32>(1.0) - fss) * s.ao;
    }
    // the luminaire's diffuser.
    if ((kind & SX_LUMINAIRE_BIT) != 0u) {
        lit = lit + SX_LAMP_RGB * (SX_LUMINAIRE_OVER * e_int * th);
    }
    // three's range fog on view depth, as look_light.
    var vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    if (frame.copy_step.z > 0.5) {
        vd = distance(world, frame.cam_pos.xyz);
    }
    if (frame.copy_step.z < 0.5 && skyu.flags.x != 0u) {
        let k = max(abs(world.y - look.cam_pos.y) - 20.0, 0.0) / 400.0;
        if (k > 1e-2) {
            vd = vd * (1.0 - exp(-k)) / k;
        }
    }
    // (the air in the bore is lit by its lamps, not by the day: it fades to the lined bore's own glow.)
    let fog = lk_smooth(look.fog.x, look.fog.y, lm_fog_depth(m, vd));
    lit = mix(lit, vec3<f32>(e_int * th * (1.0 + SX_BORE_BOUNCE) * 0.5 / LK_PI) * SX_LAMP_RGB, fog);
    return vec4<f32>(lit, s.alpha);
}
