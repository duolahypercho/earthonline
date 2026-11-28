// the far layer's draw in the city pass (appended to draw.wgsl and the
// look's WGSL by hlod.rs). Group 0 is the city's (frame, objects, ...),
// group 1 the look's (light rig, shadows, fog), group 2 the far layer's: the
// cell table, the per-group state the LOD pass wrote (cull.wgsl `lod`), the
// facade atlas pages and their sampler.
//
// Every vertex names its building's LOD group. A building the state does not
// give to this entry's level (bit 0 a neighbourhood cell, bit 1 a district)
// is dropped in the vertex shader (all its vertices leave the clip volume);
// in a cross-fade the pixels are shared with the other level by a screen-door
// dither: between a cell and its district each keeps its complementary half;
// against a near level (a kit mid rung, a Blender model), which draws whole
// and unchanged (the city shader has no cut), the far level's kept pixels
// are drawn `HL_FADE_BIAS_M` nearer the eye along the view ray, so they win
// the depth test over the rung there and nowhere else.
// Lighting, the cascaded sun shadow, fog and aerial perspective are the
// city's own `look_light`.

struct HlCellD {
    min: vec3<f32>,
    layer_base: u32,
    max: vec3<f32>,
    flags: u32,
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
    pad: u32,
};

@group(2) @binding(0) var<storage, read> hl_cells_d: array<HlCellD>;
@group(2) @binding(1) var<storage, read> hl_group_state: array<u32>;
// per LOD group its building's lowest and highest drawn point (world y; hlod.rs `top_buf`, `--tower-crowns`).
@group(2) @binding(4) var<storage, read> hl_group_top: array<vec2<f32>>;
@group(2) @binding(2) var hl_pages: texture_2d_array<f32>;
@group(2) @binding(3) var hl_sampler: sampler;

// Vertex colour flags (hlod_bake.rs VF_*).
const HL_TEXTURED: u32 = 1u;
const HL_ROOF: u32 = 2u;
const HL_DISTRICT: u32 = 4u;
// a tower's wall (hlod_bake.rs VF_TOWER).
const HL_TOWER: u32 = 8u;
// a residential building's wall (hlod_bake.rs VF_RESIDENTIAL, `#109`).
const HL_RESIDENTIAL: u32 = 128u;
// a district's glass by day, the room seen through the pane (hlod_bake.rs GLASS_ROOM, glass_body).
const HL_ROOM: vec3<f32> = vec3<f32>(0.022, 0.026, 0.030);
// How much nearer the eye the far level draws in a cross-fade with a near
// level: more than a rung's relief out of the wall plane (cornices, bays).
const HL_FADE_BIAS_M: f32 = 2.5;

// Interleaved gradient noise: the screen-door dither of the far layer's cross-fades.
fn hlod_dither(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

struct HlOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec3<f32>,
    @location(4) roof: vec3<f32>,
    @location(5) @interpolate(flat) layer: i32,
    @location(6) @interpolate(flat) flags: u32,
    // x the far side's share, y 0 whole, 1 keep the pixels below it, 2 keep those at or above it
    @location(7) @interpolate(flat) fade: vec2<f32>,
    // district: the facade's window share
    @location(8) @interpolate(flat) glass: f32,
    // the building's LOD group (the debug id)
    @location(9) @interpolate(flat) group: u32,
    // a district wall's window rhythm (hlod_district.rs pack_rhythm) and its phase up a storey
    @location(10) @interpolate(flat) rhythm: u32,
    @location(11) @interpolate(flat) phase: f32,
    // @ff-corner-field
    // @fr-roof-field
};

@vertex
fn hlod_vs(
    @location(0) pos: vec3<f32>,
    @location(1) nrm: u32,
    @location(2) uvw: u32,
    @location(3) col: u32,
    @location(4) grp: u32,
    @builtin(instance_index) entry: u32,
) -> HlOut {
    var out: HlOut;
    let st = hl_group_state[grp];
    let district = (hl_cells_d[entry].flags & 2u) != 0u;
    let mine = select(1u, 2u, district);
    if ((st & mine) == 0u) {
        // Drawn by another level this frame.
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let t = f32((st >> 8u) & 255u) / 255.0;
    var mode = 0.0;
    if ((st & 4u) != 0u) {
        mode = 1.0; // the near level fades out: the far one keeps the pixels below the share
    } else if (!district && (st & 2u) != 0u) {
        mode = 2.0; // the cell fades into the district: the cell keeps the others
    } else if (district && (st & 1u) != 0u) {
        mode = 1.0;
    }
    out.fade = vec2<f32>(t, mode);
    out.group = grp;
    var p = pos;
    if ((st & 4u) != 0u) {
        // The near level draws whole in this band: the far pixels kept here win
        // the depth test, drawn a little nearer the eye (the same screen point).
        let to_eye = frame.cam_pos.xyz - pos;
// sf:AAA_BUILDINGS-
        p = pos + to_eye * min(HL_FADE_BIAS_M / max(length(to_eye), 1.0), 0.5);
// sf:end
// sf:AAA_BUILDINGS+
        // Cut near-fade pull from 2.5 m -> 0.38 m so fading HLOD cells clear Kit R2 trims
        // without punching through touching front row houses or stamping 2.5 m SSAO depth cliffs.
        let fade_bias = select(HL_FADE_BIAS_M, 0.38, SF_AAA_BRDF);
        p = pos + to_eye * min(fade_bias / max(length(to_eye), 1.0), 0.5);
// sf:end
    }
    out.clip = frame.view_proj * vec4<f32>(p, 1.0);
    out.world = pos;
    out.normal = unpack4x8snorm(nrm).xyz;
    let c = unpack4x8unorm(col);
    out.color = srgb_decode(c.rgb);
    let flags = (col >> 24u) & 255u;
    out.flags = flags;
    out.uv = unpack2x16unorm(uvw);
    out.layer = select(-1, i32(hl_cells_d[entry].layer_base + ((nrm >> 24u) & 255u)), (flags & HL_TEXTURED) != 0u);
    // A district vertex carries its roof colour (rgb565) and the window share in the uv word.
    out.roof = srgb_decode(vec3<f32>(f32(uvw & 31u) / 31.0, f32((uvw >> 5u) & 63u) / 63.0, f32((uvw >> 11u) & 31u) / 31.0));
    out.glass = f32((uvw >> 16u) & 255u) / 255.0;
    out.rhythm = select(0u, nrm, district);
    out.phase = f32(uvw >> 24u) / 255.0;
    // @ff-corner-out
    // @fr-roof-out
    return out;
}

fn hl_keep(in: HlOut) {
    if (in.fade.y > 0.5) {
// sf:AAA_BUILDINGS-
        let n = hlod_dither(floor(in.clip.xy));
// sf:end
// sf:AAA_BUILDINGS+
        // Temporally jitter cross-fade dither (matching hlod_trees.wgsl) so TAA resolves
        // Kit R2 <-> HLOD Cell and Cell <-> District transitions smoothly with zero static zebra stipple.
        let dither_p = floor(in.clip.xy) + select(vec2<f32>(0.0), vec2<f32>(5.588238 * look.post2.w), SF_AAA_BRDF);
        let n = hlod_dither(dither_p);
// sf:end
        if (in.fade.y < 1.5) {
            if (n >= in.fade.x) {
                discard;
            }
        } else if (n < in.fade.x) {
            discard;
        }
    }
}

// an integer hash (PCG output step) and three-key form, mirrored in
// hlod.rs (`night_hash3`) where the lit shares are tested.
fn hl_u(x: u32) -> u32 {
    let s = x * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn hl_h3(a: u32, b: i32, c: u32) -> f32 {
    return f32(hl_u(a ^ hl_u(bitcast<u32>(b) ^ hl_u(c))) >> 8u) / 16777216.0;
}

// Stage W2c (`--grime`, weathering.wgsl): each far building its own age and
// paint, as the kit buildings nearer have (wx_building_k, the paint blocks):
// a tone and a warm or cool cast a little off its neighbours', a little
// darker on the whole as the weathered near ones. No base band: the far
// layer's vertices carry no height over the building's floor, and its bake
// (hlod_bake.rs, hashed into the far cache's key) is left as it is.
fn hl_age(g: u32) -> vec3<f32> {
    return vec3<f32>(0.8 + 0.3 * hl_h3(g, 7, 101u)) * mix(vec3<f32>(1.04, 1.0, 0.94), vec3<f32>(0.95, 1.0, 1.05), hl_h3(g, 11, 211u));
}

// the night's office structure (hlod.rs NIGHT_*): within a lit floor a
// tenant's run of bays is lit with this chance, and in it a window with this one;
// a dark floor keeps a few late windows.
const HL_SEG_LIT: f32 = 0.7;
const HL_LIT_IN_SEG: f32 = 0.88;
const HL_SCATTER: f32 = 0.03;
// office lamps (about 3,000 K and 4,300 K) and the cool share of an office floor.
const HL_OFFICE_WARM: vec3<f32> = vec3<f32>(1.0, 0.76, 0.50);
const HL_OFFICE_COOL: vec3<f32> = vec3<f32>(0.80, 0.90, 1.0);
const HL_OFFICE_COOL_SHARE: f32 = 0.72;

// the facade's heading in eighths (so two faces of a tower draw different tenants).
fn hl_face(n: vec3<f32>) -> u32 {
    return u32(i32(floor(atan2(n.z, n.x) * 4.0 / LK_PI + 0.5)) & 7);
}

// the building's bay and storey (a district's rhythm, else a tower's
// or a low building's defaults) and the storey phase.
fn hl_grid(in: HlOut) -> vec3<f32> {
    let tower = (in.flags & HL_TOWER) != 0u;
    let w = in.rhythm;
    if (w != 0u) {
        return vec3<f32>(max(f32(w & 255u) * 0.06, 0.8), max(f32((w >> 8u) & 255u) * 0.03, 2.4), in.phase);
    }
    return vec3<f32>(select(2.6, 1.5, tower), select(3.2, 3.9, tower), 0.0);
}

// lit windows by night with a building's own activity. A tower (an
// office) lights whole floors, by the room drive's office share times the
// tower's own activity (2.6 h^2: many towers mostly dark, a few busy); in a
// lit floor runs of 6-18 m (a tenant) are lit or dark, cool or warm; a dark
// floor keeps 3 % late windows. A low building keeps the occupancy, scaled
// per building (0.35 - 1.65, mean 1). Every level of the structure gives way
// to its mean as it shrinks under a pixel (window, run, storey), so the far
// tower holds its tone without shimmer.
fn hl_night_windows(in: HlOut, world: vec3<f32>, n: vec3<f32>, glass: f32, dpx: vec3<f32>, dpy: vec3<f32>, dusk: f32) -> vec3<f32> {
    let tower = (in.flags & HL_TOWER) != 0u;
    let grid = hl_grid(in);
    let bay = grid.x;
    let storey = grid.y;
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let along = dot(world.xz, t);
    let fl = i32(floor(world.y / storey - grid.z));
    let fx = abs(dot(dpx.xz, t)) + abs(dot(dpy.xz, t));
    let fy = abs(dpx.y) + abs(dpy.y);
    let g = in.group;
    let gf = g ^ (hl_face(n) << 24u);
    let cell = i32(floor(along / bay));
    let hb = hl_h3(g, 0, 11u);
    let base = skyu.bloom_w2.z * 25.0 / LK_PI * dusk;
// sf:AAA_BUILDINGS+
    if (tower && SF_AAA_DETAIL) {
        return hl_aaa_tower_night(in, world, n, glass, fx, fy, bay, storey, grid.z, along, fl, cell, g, gf, hb, base);
    }
// sf:end
    if (tower && FL_TOWER_NIGHT) {
        return hl_tower_night(in, world, n, glass, fx, fy, bay, storey, grid.z, along, fl, cell, g, gf, hb, base);
    }
    if (tower) {
        let p_floor = clamp(look.rooms_work.x / 0.7 * 2.6 * hb * hb, 0.0, 0.92);
        let floor_lit = step(hl_h3(g, fl, 1u), p_floor);
        let seg_len = mix(6.0, 18.0, hl_h3(g, fl, 2u));
        let seg = i32(floor(along / seg_len + hl_h3(g, fl, 3u)));
        let seg_lit = step(hl_h3(gf, fl * 131 + seg, 4u), HL_SEG_LIT);
        let wh = hl_h3(gf, fl * 977 + cell, 5u);
        let v_win = floor_lit * seg_lit * step(wh, HL_LIT_IN_SEG) + (1.0 - floor_lit) * step(wh, HL_SCATTER);
        let v_seg = floor_lit * seg_lit * HL_LIT_IN_SEG + (1.0 - floor_lit) * HL_SCATTER;
        let v_floor = floor_lit * HL_SEG_LIT * HL_LIT_IN_SEG + (1.0 - floor_lit) * HL_SCATTER;
        let v_bld = p_floor * HL_SEG_LIT * HL_LIT_IN_SEG + (1.0 - p_floor) * HL_SCATTER;
        let a1 = smoothstep(0.25, 0.6, fx / bay);
        let a2 = smoothstep(0.25, 0.6, fx / seg_len);
        let a3 = smoothstep(0.45, 1.0, fy / storey); // a lit floor holds as a line until a storey is a pixel
        let k = mix(mix(mix(v_win, v_seg, a1), v_floor, a2), v_bld, a3);
        let level = mix(0.55 + 0.7 * hl_h3(g, fl, 6u), 0.9, a3);
        let cool = step(hl_h3(gf, fl * 131 + seg, 7u), HL_OFFICE_COOL_SHARE);
        let mean_col = mix(HL_OFFICE_WARM, HL_OFFICE_COOL, HL_OFFICE_COOL_SHARE);
        let col = mix(mix(HL_OFFICE_WARM, HL_OFFICE_COOL, cool), mean_col, max(a2, a3));
        return col * (glass * k * level * base);
    }
    let occ = clamp(look.rooms_env.y * (0.35 + 1.3 * hb), 0.0, 0.95);
    let h = hl_h3(gf, fl * 977 + cell, 8u);
    let lit = step(h, occ);
    let cool = step(1.0 - look.rooms_env.w, hl_h3(gf, fl * 977 + cell, 9u));
    var col = select(vec3<f32>(1.0, 0.62, 0.34), vec3<f32>(0.86, 0.93, 1.0), cool > 0.5);
// sf:AAA_BUILDINGS+
    if (SF_AAA_DETAIL) {
        let is_res = (in.flags & HL_RESIDENTIAL) != 0u;
        let rh = hl_h3(gf, fl * 977 + cell, 9u);
        if (is_res) {
            // SF hillside rowhouses & apartments (`05a`/`06a`/`09`/`10`): rich cozy 2300K-2800K golden amber
            // + 3400K soft white + ~20% crisp 6200K-7200K cool blue-cyan TV/study rooms.
            if (rh < 0.42) {
                col = vec3<f32>(1.28, 0.56, 0.16);
            } else if (rh < 0.66) {
                col = vec3<f32>(1.12, 0.76, 0.40);
            } else if (rh < 0.81) {
                col = vec3<f32>(0.98, 0.94, 0.88);
            } else {
                col = vec3<f32>(0.54, 0.82, 1.28);
            }
        } else {
            // Low/mid-rise commercial, retail & civic: crisp 5200K-6200K clean white/mint + warm cafés/lobbies.
            if (rh < 0.38) {
                col = vec3<f32>(0.68, 0.90, 1.24);
            } else if (rh < 0.66) {
                col = vec3<f32>(0.96, 0.99, 1.04);
            } else if (rh < 0.90) {
                col = vec3<f32>(1.20, 0.68, 0.24);
            } else {
                col = vec3<f32>(0.42, 0.96, 1.12);
            }
        }
    }
// sf:end
    let a = smoothstep(0.25, 0.6, max(fx / bay, fy / storey));
    let k = mix(lit, occ, a);
    let level = 0.6 + 0.8 * hl_h3(gf, fl * 977 + cell, 10u);
    return col * (glass * k * mix(level, 1.0, a) * base);
}

// (`--tower-night`, hlod.rs `tower_night`) a tower's lit office floors seen as
// offices. A floor is lit (by the office share and the tower's own activity,
// as above), half lit for the cleaners (a dim, warm few runs) or dark; in a lit
// floor a tenant's run (6-18 m) is lit or dark at its own lamp colour (a
// continuous 3,000-5,000 K, not two) and level. Behind the vision glass the
// room reads as a room: the ceiling's glow and its fixture rows near the head
// of the glass, the desks', screens' and partitions' dark silhouettes along
// the sill, the light falling off toward the run's ends; some runs with their
// blinds part down, glowing evenly. Each level of detail gives way to its mean
// as it shrinks under a pixel (the fixtures, the profile, the window, the run,
// the storey), so a far tower holds its tone without shimmer.
const HL_TN_PROFILE_MEAN: f32 = 0.62;
// the lit floor's light on the inside of its spandrel and frames, as a share of the glass's.
const HL_TN_SPANDREL: f32 = 0.07;
const HL_TN_WARM: vec3<f32> = vec3<f32>(1.0, 0.66, 0.38);
const HL_TN_NEUTRAL: vec3<f32> = vec3<f32>(1.0, 0.86, 0.70);
const HL_TN_COOL: vec3<f32> = vec3<f32>(0.84, 0.91, 1.0);
fn hl_tower_night(in: HlOut, world: vec3<f32>, n: vec3<f32>, glass: f32, fx: f32, fy: f32, bay: f32, storey: f32, phase: f32, along: f32, fl: i32, cell: i32, g: u32, gf: u32, hb: f32, base: f32) -> vec3<f32> {
    let share = look.rooms_work.x / 0.7;
    // the floor: lit, half lit (cleaners: a few warm runs at a quarter), or dark.
    let p_lit = clamp(share * 2.6 * hb * hb + 0.1 + 0.12 * hb, 0.0, 0.9);
    let p_dim = clamp(0.22 * (1.0 - p_lit) * (0.4 + share), 0.0, 0.3);
    let fh = hl_h3(g, fl, 1u);
    let floor_lit = step(fh, p_lit);
    let floor_dim = step(p_lit, fh) * step(fh, p_lit + p_dim);
    let floor_level = mix(0.65, 1.0, hl_h3(g, fl, 6u));
    // the tenant's run.
    let seg_len = mix(6.0, 18.0, hl_h3(g, fl, 2u));
    let seg_pos = along / seg_len + hl_h3(g, fl, 3u);
    let seg = i32(floor(seg_pos));
    let sk = fl * 131 + seg;
    let seg_lit = floor_lit * step(hl_h3(gf, sk, 4u), 0.8) + floor_dim * step(hl_h3(gf, sk, 4u), 0.3);
    let seg_level = mix(0.6, 1.0, hl_h3(gf, sk, 12u)) * mix(floor_level, 0.24, floor_dim);
    let t = hl_h3(gf, sk, 7u) * hl_h3(gf, sk, 7u); // most office lamps warm (3,000-3,500 K), a few cool
    let warmth = select(t, 0.15 * t, floor_dim > 0.5); // the cleaners' few lamps warm
    var col = mix(mix(HL_TN_WARM, HL_TN_NEUTRAL, smoothstep(0.0, 0.45, warmth)), HL_TN_COOL, smoothstep(0.45, 1.0, warmth));
    // a window in a lit run: nearly all, a few dark (a meeting room, a vacant desk).
    let wh = hl_h3(gf, fl * 977 + cell, 5u);
    let win_on = seg_lit * step(wh, 0.9) + (1.0 - floor_lit - floor_dim) * step(wh, 0.02);
    // the room behind the glass, by height within the storey (0 the sill, 1 the head).
    let up = fract(world.y / storey - phase);
    let v = clamp(up / 0.62, 0.0, 1.0);
    let ceiling = 0.6 + 0.5 * smoothstep(0.5, 0.97, v);
    let desks = mix(0.5 + 0.25 * hl_h3(gf, fl * 977 + cell, 13u), 1.0, smoothstep(0.18, 0.4, v));
    let fixtures = 1.0 + 0.9 * hl_pulse(along / 1.8 + 0.3, 0.32, fx / 1.8) * smoothstep(0.82, 0.97, v);
    // the run's ends a little dimmer (its lamps stop at the partition).
    let run_edge = mix(0.75, 1.0, smoothstep(0.0, 0.18, min(fract(seg_pos), 1.0 - fract(seg_pos))));
    var profile = ceiling * desks * mix(fixtures, 1.0 + 0.9 * 0.32 * 0.4, smoothstep(0.2, 0.5, fx / 1.8)) * run_edge;
    // blinds part down over some runs: an even, dimmer glow over their share of the glass.
    let blind_to = select(0.0, mix(0.25, 0.8, hl_h3(gf, sk, 14u)), hl_h3(gf, sk, 15u) < 0.35);
    profile = mix(profile, 0.38, step(1.0 - blind_to, v));
    // every level to its mean as it shrinks under a pixel.
    // the slab and spandrel between the head of one floor's glass and the next sill stay dark
    // (a curtain wall's glass covers them too: its atlas has no wall there).
    let slab = smoothstep(0.62, 0.68, up) * (1.0 - smoothstep(0.97, 1.0, up));
    profile = profile * mix(1.0, 0.1, slab);
    profile = mix(profile, HL_TN_PROFILE_MEAN, smoothstep(0.12, 0.35, fy / storey));
    let v_win = win_on;
    let v_seg = seg_lit * 0.9 + (1.0 - floor_lit - floor_dim) * 0.02;
    let v_floor = floor_lit * 0.8 * 0.9 + floor_dim * 0.3 * 0.9 + (1.0 - floor_lit - floor_dim) * 0.02;
    let v_bld = p_lit * 0.8 * 0.9 + p_dim * 0.3 * 0.9 + (1.0 - p_lit - p_dim) * 0.02;
    let a1 = smoothstep(0.25, 0.6, fx / bay);
    let a2 = smoothstep(0.25, 0.6, fx / seg_len);
    let a3 = smoothstep(0.45, 1.0, fy / storey);
    let k = mix(mix(mix(v_win, v_seg, a1), v_floor, a2), v_bld, a3);
    let level = mix(seg_level, 0.8, max(a2, a3));
    let mean_col = mix(HL_TN_NEUTRAL, HL_TN_COOL, 0.35);
    col = mix(col, mean_col, max(a2, a3));
    // the lit run's light on the spandrel and frames between its panes (a lit floor reads as one band).
    let ks = mix(mix(v_seg, v_floor, a2), v_bld, a3);
    let spandrel = select(0.0, (1.0 - glass) * HL_TN_SPANDREL * ks, abs(n.y) < 0.3);
    return col * ((glass * k * profile + spandrel) * level * base);
}

// sf:AAA_BUILDINGS+
// (`--aaa-buildings` / `SF_AAA_DETAIL`) Photoreal SF night tower office lighting:
// replaces the full-width zebra stripes produced when `hl_tower_night` collapses
// `v_seg -> v_floor` at 1.5-3 km with three coherent horizontal frequencies (macro
// tenant wings 14-28 m, mid office clusters 5.5-11.5 m, individual bays + dark
// structural column/core breaks), dark mechanical floors matching daytime louvers,
// crisp dark floor spandrels, warm lobby/podium ground glows, and rich 2700K-5200K
// color temperature & brightness diversity.
fn hl_aaa_tower_night(in: HlOut, world: vec3<f32>, n: vec3<f32>, glass: f32, fx: f32, fy: f32, bay: f32, storey: f32, phase: f32, along: f32, fl: i32, cell: i32, g: u32, gf: u32, hb: f32, base: f32) -> vec3<f32> {
    let bt = hl_group_top[in.group];
    let b_h = max(bt.y - bt.x, 0.0);
    let y_rel = max(world.y - bt.x, 0.0);
    let depth_top = max(bt.y - world.y, 0.0);
    // Keep mechanical/refuge floors & crown screens dark (aligned with daytime louvers).
    if (b_h > 40.0) {
        let m_per = mix(52.0, 74.0, hl_h3(g, 0, 41u));
        let m_h = mix(5.2, 7.6, hl_h3(g, 0, 42u));
        let m_mid = hl_pulse(y_rel / m_per + 0.35, m_h / m_per, fy / m_per)
            * smoothstep(16.0, 30.0, y_rel) * smoothstep(5.0, 14.0, depth_top);
        let crown_h = clamp(b_h * mix(0.035, 0.072, hl_h3(g, 0, 43u)), 4.2, 10.5);
        let m_crown = smoothstep(crown_h + fy, crown_h - fy, depth_top) * step(46.0, b_h);
        if (max(m_mid, m_crown) > 0.55) {
            return vec3<f32>(0.0);
        }
    }
    let share = clamp(look.rooms_work.x / 0.7, 0.0, 1.0);
    let act = 0.25 + 0.75 * hb;
    let p_lit = clamp(0.14 + share * 0.52 * act + 0.14 * hb * hb, 0.08, 0.50);
    let p_dim = clamp(0.24 * (1.0 - p_lit), 0.10, 0.28);
    let fh = hl_h3(g, fl, 1u);
    let floor_lit = step(fh, p_lit);
    let floor_dim = step(p_lit, fh) * step(fh, p_lit + p_dim);
    // Macro tenant wings (14-28 m): survive out to ~4 km so towers never turn into solid zebra bars.
    let wing_len = mix(14.0, 28.0, hl_h3(g, fl, 61u));
    let wing_pos = along / wing_len + hl_h3(gf, fl, 62u) * 3.7;
    let wing_id = i32(floor(wing_pos));
    let wk = fl * 89 + wing_id;
    let wing_h = hl_h3(gf, wk, 63u);
    let wing_lit = floor_lit * step(wing_h, mix(0.36, 0.62, hb)) + floor_dim * step(wing_h, 0.26);
    // Mid office suite clusters (5.5-11.5 m) inside each wing.
    let seg_len = mix(5.5, 11.5, hl_h3(g, fl, 2u));
    let seg_pos = along / seg_len + hl_h3(gf, fl, 3u) * 5.3;
    let seg_id = i32(floor(seg_pos));
    let sk = fl * 131 + seg_id;
    let seg_h = hl_h3(gf, sk, 4u);
    let seg_lit = wing_lit * step(seg_h, 0.72)
        + (1.0 - wing_lit) * (floor_lit * step(seg_h, 0.14) + (1.0 - floor_lit) * step(seg_h, 0.045));
    // Structural dark vertical core/column breaks every 7-11 bays + per-bay occupancy.
    let core_pitch = mix(7.0, 11.0, hl_h3(g, 0, 64u));
    let core_u = abs(fract(along / (bay * core_pitch) + hl_h3(gf, 0, 65u)) - 0.5) * 2.0;
    let core_dark = smoothstep(0.06, 0.16, core_u);
    let wh = hl_h3(gf, fl * 977 + cell, 5u);
    let win_on = (seg_lit * step(wh, 0.84) + (1.0 - seg_lit) * step(wh, 0.025)) * mix(0.28, 1.0, core_dark);
    let a_win = smoothstep(0.25, 0.65, fx / bay);
    let a_seg = smoothstep(0.35, 0.90, fx / seg_len);
    let a_wing = smoothstep(0.75, 1.55, fx / wing_len);
    let a_fl = smoothstep(0.65, 1.35, fy / storey);
    let v_seg = seg_lit * 0.84 + (1.0 - seg_lit) * 0.025;
    let v_wing = wing_lit * 0.54 + (1.0 - wing_lit) * (floor_lit * 0.11 + 0.032);
    let v_floor = (floor_lit * 0.32 + floor_dim * 0.11 + 0.028) * mix(0.55, 1.15, hl_h3(gf, fl, 66u));
    let v_bld = p_lit * 0.32 + p_dim * 0.11 + 0.028;
    var k = mix(mix(mix(win_on, v_seg, a_win), v_wing, a_seg), v_floor, a_wing);
    k = mix(k, v_bld, a_fl);
    // Warm inviting ground-floor lobby & podium glow (bottom 9 m).
    let lobby = (1.0 - smoothstep(4.5, 10.5, y_rel)) * step(22.0, b_h);
    // Interior ceiling/desk luminance profile & crisp dark floor spandrels (zero wall-wash smearing).
    let up = fract(world.y / storey - phase);
    let v = clamp(up / 0.58, 0.0, 1.0);
    let ceiling = 0.55 + 0.55 * smoothstep(0.48, 0.96, v);
    let desks = mix(0.42 + 0.28 * hl_h3(gf, fl * 977 + cell, 13u), 1.0, smoothstep(0.18, 0.42, v));
    let blind_to = select(0.0, mix(0.25, 0.78, hl_h3(gf, sk, 14u)), hl_h3(gf, sk, 15u) < 0.42);
    var profile = mix(ceiling * desks, 0.34, step(1.0 - blind_to, v));
    let slab = smoothstep(0.58, 0.66, up) * (1.0 - smoothstep(0.96, 1.0, up));
    let slab_fade = 1.0 - smoothstep(0.30, 0.85, fy / storey);
    profile = profile * mix(1.0, 0.06, slab * slab_fade);
    profile = mix(profile, 0.54, smoothstep(0.18, 0.55, fy / storey));
    // Authentic multi-temperature SF night tower palette using `HL_RESIDENTIAL` (`#109`) + `mat_cls`:
    // FiDi glass/tech towers glow crisp 5500K-6500K ice-white/cyan + 4000K corporate + warm corners;
    // stone/precast corporate towers balance clean white & warm gold; residential high-rises glow
    // cozy 2400K-2900K golden amber + cool 6400K TV/study rooms.
    let is_res = (in.flags & HL_RESIDENTIAL) != 0u;
    let mcls_n = (in.flags >> 4u) & 7u;
    let bld_type = hl_h3(g, 0, 71u);
    let temp_h = mix(hl_h3(gf, sk, 7u), hl_h3(gf, wk, 67u), a_seg);
    let c_ice = vec3<f32>(0.54, 0.84, 1.30);
    let c_cool = vec3<f32>(0.76, 0.94, 1.18);
    let c_neut = vec3<f32>(0.97, 0.98, 1.02);
    let c_warm = vec3<f32>(1.14, 0.74, 0.36);
    let c_amber = vec3<f32>(1.30, 0.56, 0.16);
    let c_teal = vec3<f32>(0.38, 0.94, 1.14);
    var col = c_neut;
    if (floor_dim > 0.5) {
        col = select(c_warm, c_amber, temp_h < 0.55);
    } else if (is_res) {
        if (temp_h < 0.38) { col = c_amber; }
        else if (temp_h < 0.64) { col = c_warm; }
        else if (temp_h < 0.80) { col = c_neut; }
        else { col = c_ice; }
    } else if (mcls_n == 1u || mcls_n == 2u || bld_type < 0.42) {
        if (temp_h < 0.34) { col = c_ice; }
        else if (temp_h < 0.60) { col = c_cool; }
        else if (temp_h < 0.78) { col = c_neut; }
        else if (temp_h < 0.93) { col = c_warm; }
        else { col = c_teal; }
    } else {
        if (temp_h < 0.26) { col = c_cool; }
        else if (temp_h < 0.52) { col = c_neut; }
        else if (temp_h < 0.78) { col = c_warm; }
        else if (temp_h < 0.92) { col = c_amber; }
        else { col = c_ice; }
    }
    let lobby_col = select(vec3<f32>(1.18, 0.74, 0.34), vec3<f32>(0.68, 0.92, 1.24), !is_res && bld_type < 0.35);
    col = mix(col, lobby_col, lobby * 0.75);
    let s_lvl = mix(0.28, 1.08, hl_h3(gf, sk, 12u) * (0.55 + 0.45 * hl_h3(gf, wk, 68u)))
        * mix(1.0, 0.34, floor_dim);
    let level = mix(s_lvl, 0.52, a_wing) * mix(0.62, 0.92, lobby);
    let k_tot = max(k * profile, lobby * 0.40 * (0.65 + 0.35 * hl_h3(gf, cell, 69u)));
    return col * (glass * k_tot * level * base);
}
// sf:end

// (`--tower-crowns`, hlod.rs `tower_crowns`) a tall tower's crown at night:
// most towers over `HL_CROWN_MIN_M` floodlight their top storeys from a setback
// (brightest low in the band, fading up it), in warm or cool white, a few in
// colour, and some draw an LED line along the parapet. By the building's own
// base and top (`hl_group_top`). INFERRED from night photographs of the
// skyline (lit crowns on most of its tall towers).
const HL_CROWN_MIN_M: f32 = 70.0;
const HL_CROWN_SHARE: f32 = 0.6;
fn hl_tower_crown(in: HlOut, world: vec3<f32>, n: vec3<f32>, fy: f32, base: f32) -> vec3<f32> {
    let bt = hl_group_top[in.group];
    let height = bt.y - bt.x;
    if (height < HL_CROWN_MIN_M || abs(n.y) > 0.3 || base <= 0.0) {
        return vec3<f32>(0.0);
    }
    let g = in.group;
    if (hl_h3(g, 0, 21u) > HL_CROWN_SHARE) {
        return vec3<f32>(0.0);
    }
    let depth = bt.y - world.y;
    if (depth < -0.25) {
        return vec3<f32>(0.0); // over the building's known top: not its crown
    }
    let band = clamp(height * mix(0.02, 0.05, hl_h3(g, 0, 22u)), 3.0, 9.0);
    // the floodlit band: from its foot (the lamps' setback) up to the parapet, brightest low.
    let foot = smoothstep(band, band * 0.75, depth);
    let wash = foot * mix(1.0, 0.55, clamp(1.0 - depth / band, 0.0, 1.0));
    let roll = hl_h3(g, 0, 23u);
    var col = vec3<f32>(1.0, 0.82, 0.62); // warm white
    if (roll > 0.55) { col = vec3<f32>(0.86, 0.92, 1.0); } // cool white
    if (roll > 0.8) { col = vec3<f32>(0.35, 0.62, 1.0); } // blue
    if (roll > 0.9) { col = vec3<f32>(0.3, 1.0, 0.72); } // teal green
    // an LED line along the parapet on some, and a second a storey down on some: each the
    // share of the pixel's vertical footprint the line covers (a box filter: sharp near, its
    // area spread over the pixel far, nothing a pixel away from it).
    let led_on = step(hl_h3(g, 0, 24u), 0.75);
    let led_w = 0.35;
    let lo = depth - 0.5 * fy;
    let hi = depth + 0.5 * fy;
    let cover1 = max(0.0, min(led_w, hi) - max(0.0, lo)) / max(fy, 1e-3);
    let cover2 = max(0.0, min(4.0 + 0.5 * led_w, hi) - max(4.0 - 0.5 * led_w, lo)) / max(fy, 1e-3);
    let led = led_on * min(cover1, 1.0);
    let led2 = led_on * step(hl_h3(g, 0, 25u), 0.4) * min(cover2, 1.0);
// sf:AAA_BUILDINGS+
    if (SF_AAA_DETAIL) {
        var col_a = vec3<f32>(1.18, 0.76, 0.36); // warm golden crown
        if (roll > 0.36) { col_a = vec3<f32>(0.82, 0.93, 1.12); } // crisp 5500K white
        if (roll > 0.64) { col_a = vec3<f32>(0.24, 0.58, 1.32); } // electric sapphire
        if (roll > 0.82) { col_a = vec3<f32>(0.22, 1.05, 0.86); } // vivid cyan-teal
        if (roll > 0.92) { col_a = vec3<f32>(0.92, 0.38, 1.08); } // subtle violet accent
        return col_a * base * (0.065 * wash + 1.15 * (led + 0.62 * led2));
    }
// sf:end
    return col * base * (0.05 * wash + 1.6 * (led + 0.7 * led2));
}

// a tower's panes are not one mirror. Each storey (and a little each
// run of three bays) tilts by up to about 0.7 degrees, so the reflected sky and the sun's
// glint break up floor by floor and panel by panel; faded to the plain
// normal as a storey shrinks under a pixel (the mean reflection).
fn hl_panel_normal(in: HlOut, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) -> vec3<f32> {
    if (!FL_PANELS || (in.flags & HL_TOWER) == 0u || abs(n.y) > 0.3) {
        return n;
    }
    let grid = hl_grid(in);
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let fl = i32(floor(in.world.y / grid.y - grid.z));
    let run = i32(floor(dot(in.world.xz, t) / (grid.x * 3.0)));
    let gf = in.group ^ (hl_face(n) << 24u);
    // Mostly a floor's own tilt (a storey's band of panes), a little per run.
    let jx = 0.7 * (hl_h3(gf, fl, 12u) - 0.5) + 0.3 * (hl_h3(gf, fl * 389 + run, 14u) - 0.5);
    let jy = 0.7 * (hl_h3(gf, fl, 13u) - 0.5) + 0.3 * (hl_h3(gf, fl * 389 + run, 15u) - 0.5);
    let fade = 1.0 - smoothstep(0.3, 0.8, (abs(dpx.y) + abs(dpy.y)) / grid.y);
    return normalize(n + (vec3<f32>(t.x, 0.0, t.y) * jx + vec3<f32>(0.0, jy, 0.0)) * (HL_PANEL_TILT * fade));
}

// what a far pane reflects: the sky above the horizon, below it the
// city and the water seen from above (a quarter of the horizon's radiance),
// not the horizon sky held under it.
fn hl_reflect_env(r: vec3<f32>) -> vec3<f32> {
    if (!FL_PANELS) {
        return lk_sky_sample(vec3<f32>(r.x, max(r.y, 0.02), r.z));
    }
    let up = lk_sky_sample(vec3<f32>(r.x, max(r.y, 0.0), r.z));
    let horizon = lk_sky_sample(normalize(vec3<f32>(r.x, 0.0, r.z) + vec3<f32>(1e-5, 0.0, 0.0)));
    return mix(up, horizon * HL_CITY_REFL, smoothstep(-0.02, 0.2, -r.y));
}

const HL_CITY_REFL: f32 = 0.25;
// the panes' tilt (radians at most, before the 0.5 of the hash): about 0.7 degrees.
const HL_PANEL_TILT: f32 = 0.025;

// Lit windows by night: one lit or dark choice per window-sized cell of the
// wall (2.6 m x 3.2 m), the occupancy, warm/cool share and level of the far
// boxes' windows (sky_light.wgsl `sky_far_windows`), weighted by the glass
// the bake saw there; averaged to the mean once a cell is under a pixel.
fn hl_windows(in: HlOut, world: vec3<f32>, n: vec3<f32>, glass: f32, dpx: vec3<f32>, dpy: vec3<f32>) -> vec3<f32> {
    let dusk = look.rooms_env.x;
    if (!sky_on(skyu.flags.x, SKY_LIGHTS) || dusk <= 0.0 || glass <= 0.0 || abs(n.y) > 0.3) {
        return vec3<f32>(0.0);
    }
    if (FL_NIGHT) {
        return hl_night_windows(in, world, n, glass, dpx, dpy, dusk);
    }
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let cell = floor(vec2<f32>(dot(world.xz, t) / 2.6, world.y / 3.2));
    let h = st_hash(cell * vec2<f32>(1.37, 2.11) + 0.731);
    let occ = look.rooms_env.y;
    let lit = step(h, occ);
    let cool = step(1.0 - look.rooms_env.w, st_hash(cell + 17.3));
    let col = select(vec3<f32>(1.0, 0.62, 0.34), vec3<f32>(0.86, 0.93, 1.0), cool > 0.5);
    let px = max(length(dpx), length(dpy)) / 2.6;
    let k = mix(lit, occ, smoothstep(0.25, 0.6, px));
    let level = skyu.bloom_w2.z * 25.0 / LK_PI * dusk * (0.6 + 0.8 * st_hash(cell + 5.1));
    return col * (glass * k * level);
}

// a box-filtered pulse train (hlod_district.rs `pulse_filtered`, tested):
// the share of [x - w / 2, x + w / 2] covered by pulses of `duty` starting at
// each whole number (x and w in periods). Wider than a period it tends to the
// duty, so the grid fades to its mean instead of shimmering.
fn hl_pulse_int(t: f32, duty: f32) -> f32 {
    let f = floor(t);
    return f * duty + min(t - f, duty);
}

fn hl_pulse(x: f32, duty: f32, w: f32) -> f32 {
    if (w < 1e-4) {
        return select(0.0, 1.0, fract(x) < duty);
    }
    return (hl_pulse_int(x + 0.5 * w, duty) - hl_pulse_int(x - 0.5 * w, duty)) / w;
}

// a district wall's glass coverage at this pixel: windows of the
// building's rhythm (bay and storey periods, window shares; a tower without
// one a curtain wall), scaled about their centre so their mean is the
// window share the cell bake saw, box filtered over the pixel's footprint
// on the wall.
fn hl_window_grid(in: HlOut, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) -> f32 {
    let g = in.glass;
    if (g <= 0.0 || abs(n.y) > 0.3) {
        return 0.0;
    }
    let tower = (in.flags & HL_TOWER) != 0u;
    let w = in.rhythm;
    var bay = f32(w & 255u) * 0.06;
    var storey = f32((w >> 8u) & 255u) * 0.03;
    var ww = f32((w >> 16u) & 255u) / 255.0;
    var wh = f32(w >> 24u) / 255.0;
    if (w == 0u) {
        bay = select(3.0, 1.5, tower);
        storey = select(3.2, 3.9, tower);
        ww = select(0.45, 0.88, tower);
        wh = select(0.5, 0.62, tower);
    }
    let s = sqrt(g / max(ww * wh, 1e-3));
    ww = clamp(ww * s, 0.0, 0.95);
    wh = clamp(wh * s, 0.0, 0.95);
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let along = dot(in.world.xz, t) / bay;
    let up = in.world.y / storey - in.phase;
    let fx = (abs(dot(dpx.xz, t)) + abs(dot(dpy.xz, t))) / bay;
    let fy = (abs(dpx.y) + abs(dpy.y)) / storey;
    // Centre the window in its bay; the storey's sill is where the phase puts it.
    return hl_pulse(along - 0.5 * (1.0 - ww), ww, fx) * hl_pulse(up, wh, fy);
}

// what the glass adds by day beyond the plain 4 % the light rig
// gives every glossy surface: a pane's Fresnel reflection of the sky
// (Schlick, F0 0.04), and on a tower a coated pane's (F0 0.08 - 0.22 and a
// faint tint, varied per tower). Emitted before the fog, so it hazes with the wall.
fn hl_glass_sky(in: HlOut, n: vec3<f32>, v: vec3<f32>, cov: f32) -> vec3<f32> {
    if (cov <= 0.0 || abs(n.y) > 0.3) {
        return vec3<f32>(0.0);
    }
    let nv = max(dot(n, v), 1e-3);
    let fres = pow(1.0 - nv, 5.0);
    var f0 = vec3<f32>(0.04);
    if (FL_TOWER && (in.flags & HL_TOWER) != 0u) {
        let h = st_hash(vec2<f32>(f32(in.group) * 0.618, 7.31));
        let h2 = st_hash(vec2<f32>(f32(in.group) * 1.37, 2.9));
        let tint = mix(vec3<f32>(0.80, 0.92, 1.0), mix(vec3<f32>(0.82, 1.0, 0.92), vec3<f32>(1.0, 0.9, 0.78), step(0.5, h2)), step(0.6, h));
        f0 = (0.08 + 0.14 * h) * tint;
    }
    let f = f0 + (vec3<f32>(1.0) - f0) * fres;
    // The light rig already reflects the sky at its plain 0.04 (look_light, fss): add the rest.
    let extra = max(f - vec3<f32>(0.04 + 0.96 * fres), vec3<f32>(0.0));
    let r = reflect(-v, n);
    let sky = hl_reflect_env(r); // the city below the horizon
    let env_i = look.ambient.w * look.env_class.y;
    return sky * env_i * extra * cov;
}

// ---- (`--far-facades`, facade_age.wgsl `ff_on()`) the far roofs and glass ----
// A district's roof is one flat polygon in its measured colour and a wall one
// colour with its window rhythm: from the air the far city read as massing.
// A flat roof carries its building's plant (fans, condensers, air handlers,
// penthouse boxes) on a membrane patched and ponded over the years, with
// hatches and skylights; a wall's windows are each a room, some with a blind
// down. All of it is filtered over the pixel's footprint and gives way to its
// mean as a cell shrinks under a pixel, so the far city holds its tone
// without shimmer. INFERRED sizes and shares (rooftop units 1-3.6 m by 1-3.2 m,
// 0.8-2.4 m tall; one at most per FF_CELL_M square).
const FF_CELL_M: f32 = 7.0;
const FF_UNIT_SHARE: vec2<f32> = vec2<f32>(0.14, 0.62);
const FF_UNIT_TONE: vec3<f32> = vec3<f32>(0.47, 0.48, 0.47);
// a stair or lift penthouse (3-5.5 x 3.5-7 m, 2.4-3.6 m tall) in this share of FF_PENT_M squares, by building.
const FF_PENT_M: f32 = 22.0;
const FF_PENT_SHARE: vec2<f32> = vec2<f32>(0.12, 0.55);
const FF_HATCH_TONE: vec3<f32> = vec3<f32>(0.06, 0.07, 0.08);
const FF_SHADOW: f32 = 0.6;
// a far window's room: a blind or a lit ceiling (pale) in this share, else the dark room.
const FF_BLIND_SHARE: f32 = 0.36;
const FF_BLIND_TONE: vec3<f32> = vec3<f32>(0.20, 0.19, 0.17);

fn ff_h(c: vec2<f32>, g: u32, salt: u32) -> f32 {
    let ci = bitcast<vec2<u32>>(vec2<i32>(c));
    return f32(hl_u(g ^ hl_u(ci.x ^ hl_u(ci.y ^ (salt * 2654435769u)))) >> 8u) / 16777216.0;
}

// value noise on a lattice of whole metres / the scale (0..1).
fn ff_noise(p: vec2<f32>, g: u32) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = ff_h(i, g, 11u);
    let b = ff_h(i + vec2<f32>(1.0, 0.0), g, 11u);
    let c = ff_h(i + vec2<f32>(0.0, 1.0), g, 11u);
    let d = ff_h(i + vec2<f32>(1.0, 1.0), g, 11u);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// the share of [a, b] a pixel `w` wide centred at x covers (a box filter).
fn ff_cov1(x: f32, a: f32, b: f32, w: f32) -> f32 {
    let hw = 0.5 * max(w, 1e-4);
    return clamp((min(x + hw, b) - max(x - hw, a)) / (2.0 * hw), 0.0, 1.0);
}

// a unit's box in its FF_CELL_M square: (x, z) its corner in the square, (z, w) its size; size 0 none.
fn ff_unit_box(cell: vec2<f32>, g: u32, share: f32) -> vec4<f32> {
    let size = vec2<f32>(1.0 + 2.6 * ff_h(cell, g, 2u), 1.0 + 2.2 * ff_h(cell, g, 3u));
    let o = vec2<f32>(ff_h(cell, g, 4u), ff_h(cell, g, 5u)) * (FF_CELL_M - size - 1.0) + 0.5;
    return select(vec4<f32>(0.0, 0.0, -1.0, -1.0), vec4<f32>(o, size), ff_h(cell, g, 1u) < share);
}

fn ff_unit_height(cell: vec2<f32>, g: u32) -> f32 {
    return 0.8 + 1.6 * ff_h(cell, g, 6u);
}

// a stair or lift penthouse's box in its FF_PENT_M square (as `ff_unit_box`).
fn ff_pent_box(cell: vec2<f32>, g: u32, share: f32) -> vec4<f32> {
    let size = vec2<f32>(3.0 + 2.5 * ff_h(cell, g, 41u), 3.5 + 3.5 * ff_h(cell, g, 42u));
    let sz = select(size, size.yx, ff_h(cell, g, 43u) < 0.5);
    let o = vec2<f32>(ff_h(cell, g, 44u), ff_h(cell, g, 45u)) * (FF_PENT_M - sz - 2.0) + 1.0;
    return select(vec4<f32>(0.0, 0.0, -1.0, -1.0), vec4<f32>(o, sz), ff_h(cell, g, 40u) < share);
}

fn ff_pent_height(cell: vec2<f32>, g: u32) -> f32 {
    return 2.4 + 1.2 * ff_h(cell, g, 46u);
}

// the tallest a unit (`ff_unit_height`, 2.4 m) or a penthouse (`ff_pent_height`, 3.6 m) stands: the shade's march stops once its ray is over it.
const FF_PLANT_TOP_M: f32 = 3.6;

// a far flat roof (`--far-facades`): rgb its albedo over `base` (the
// measured roof colour stays its mean), a its roughness.
fn ff_roof(world: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, g: u32, base: vec3<f32>, plant: bool) -> vec4<f32> {
    let p = world.xz;
    let fw = max(abs(dpx.xz) + abs(dpy.xz), vec2<f32>(1e-3));
    let fwm = max(fw.x, fw.y);
    // the membrane: re-roofed patches (12-15 m) and ponding stains (4 m), each faded to its mean
    // (and not looked up at all once it is: the same frame).
    let keep_patch = 1.0 - smoothstep(9.0, 26.0, fwm);
    let keep_stain = 1.0 - smoothstep(2.5, 8.0, fwm);
    var k = 1.0;
    if (keep_patch > 0.0) {
        let patch_n = ff_noise(p / 13.0, g ^ 0x51u);
        k = 1.0 + (patch_n - 0.5) * 0.26 * keep_patch;
    }
    if (keep_stain > 0.0) {
        let stain = smoothstep(0.58, 0.82, ff_noise(p / 4.3, g ^ 0xa3u));
        k = k * (1.0 - 0.2 * (stain - 0.12) * keep_stain);
    }
    var col = base * k;
    var rough = 0.9;
    if (!plant) {
        return vec4<f32>(col, rough);
    }
    let busy = hl_h3(g, 31, 401u);
    let share = mix(FF_UNIT_SHARE.x, FF_UNIT_SHARE.y, busy);
    let pshare = mix(FF_PENT_SHARE.x, FF_PENT_SHARE.y, hl_h3(g, 37, 409u));
    let cell = floor(p / FF_CELL_M);
    let q = p - cell * FF_CELL_M;
    // once a cell is under a pixel only the means below are kept: its units, penthouse, hatch and the
    // shade march are not looked up there (the same frame)
    let fade = smoothstep(0.8, 1.6, fwm / FF_CELL_M);
    var ub = vec4<f32>(0.0, 0.0, -1.0, -1.0);
    var unit = 0.0;
    // a stair or lift penthouse (one at most in each FF_PENT_M square)
    let pcell = floor(p / FF_PENT_M);
    var pent = 0.0;
    if (fade < 1.0) {
        ub = ff_unit_box(cell, g, share);
        unit = ff_cov1(q.x, ub.x, ub.x + ub.z, fw.x) * ff_cov1(q.y, ub.y, ub.y + ub.w, fw.y);
        let pq = p - pcell * FF_PENT_M;
        let pb = ff_pent_box(pcell, g, pshare);
        pent = ff_cov1(pq.x, pb.x, pb.x + pb.z, fw.x) * ff_cov1(pq.y, pb.y, pb.y + pb.w, fw.y);
    }
    // the shade the units and penthouses cast down the roof from the key light: a march toward the
    // sun over their tops (a low sun throws a unit's shade many metres, over the next squares)
    let l = look.key_dir.xyz;
    let sun = smoothstep(0.03, 0.15, l.y);
    let lh = max(length(l.xz), 1e-3);
    let toward = l.xz / lh;
    let rise = max(l.y, 0.02) / lh;
    var shade = 0.0;
    if (sun > 0.0 && fade < 1.0) {
        for (var i = 0; i < 10; i = i + 1) {
            let sd = 0.4 + 0.22 * f32(i * i) + 0.6 * f32(i);
            let ray = sd * rise;
            if (ray > FF_PLANT_TOP_M) {
                break; // the ray has risen over the tallest unit or penthouse: nothing further shades
            }
            let at = p + toward * sd;
            let cu = floor(at / FF_CELL_M);
            let bu = ff_unit_box(cu, g, share);
            let ru = at - cu * FF_CELL_M;
            let in_u = step(bu.x, ru.x) * step(ru.x, bu.x + bu.z) * step(bu.y, ru.y) * step(ru.y, bu.y + bu.w) * step(ray, ff_unit_height(cu, g));
            let cp = floor(at / FF_PENT_M);
            let bp = ff_pent_box(cp, g, pshare);
            let rp = at - cp * FF_PENT_M;
            let in_p = step(bp.x, rp.x) * step(rp.x, bp.x + bp.z) * step(bp.y, rp.y) * step(rp.y, bp.y + bp.w) * step(ray, ff_pent_height(cp, g));
            shade = max(shade, max(in_u, in_p));
        }
        shade = shade * (1.0 - max(unit, pent)) * sun;
    }
    // a hatch or skylight in a cell without a unit
    var hatch = 0.0;
    if (fade < 1.0) {
        let has = step(0.01, ub.z);
        let hatch_on = (1.0 - has) * step(ff_h(cell, g, 8u), 0.22);
        let hs = vec2<f32>(0.9, 0.9) + 1.4 * ff_h(cell, g, 9u);
        let ho = vec2<f32>(ff_h(cell, g, 10u), ff_h(cell, g, 12u)) * (FF_CELL_M - hs - 1.0) + 0.5;
        hatch = ff_cov1(q.x, ho.x, ho.x + hs.x, fw.x) * ff_cov1(q.y, ho.y, ho.y + hs.y, fw.y) * hatch_on;
    }
    // once a cell is under a pixel: the cell's mean cover and shade (a unit's shade about 2.3 m
    // wide and its height over the sun's rise long; a penthouse's 4.5 m wide)
    let mean_unit = share * 5.9 / (FF_CELL_M * FF_CELL_M);
    let mean_pent = pshare * 24.0 / (FF_PENT_M * FF_PENT_M);
    let reach_u = min(1.6 / max(rise, 0.05), 16.0);
    let reach_p = min(3.0 / max(rise, 0.05), 16.0);
    let mean_shade = min(share * 2.3 * reach_u / (FF_CELL_M * FF_CELL_M) + pshare * 4.5 * reach_p / (FF_PENT_M * FF_PENT_M), 0.6) * sun;
    let mean_hatch = (1.0 - share) * 0.22 * 3.2 / (FF_CELL_M * FF_CELL_M);
    let u = mix(unit, mean_unit, fade);
    let pu = mix(pent, mean_pent, fade);
    let sh = mix(shade, mean_shade, fade);
    let ha = mix(hatch, mean_hatch, fade);
    let tone = FF_UNIT_TONE * mix(0.8, 1.2, ff_h(cell, g, 7u));
    let ptone = base * mix(0.85, 1.25, ff_h(pcell, g, 47u));
    col = mix(mix(mix(col, tone, u), ptone, pu), FF_HATCH_TONE, ha) * (1.0 - FF_SHADOW * sh);
// sf:AAA_BUILDINGS-
    rough = mix(mix(rough, 0.5, u), 0.25, ha);
// sf:end
// sf:AAA_BUILDINGS+
    rough = mix(mix(rough, 0.68, u), select(0.25, 0.72, SF_AAA_BRDF), ha);
// sf:end
    return vec4<f32>(col, rough);
}

// a far window's room (`--far-facades`): each window its own (a blind down, a
// lit ceiling, or the dark room), filtered to the mean as a window shrinks
// under a pixel.
fn ff_window_room(in: HlOut, n: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) -> vec3<f32> {
    let grid = hl_grid(in);
    let t = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let along = dot(in.world.xz, t) / grid.x;
    let up = in.world.y / grid.y - grid.z;
    let fx = (abs(dot(dpx.xz, t)) + abs(dot(dpy.xz, t))) / grid.x;
    let fy = (abs(dpx.y) + abs(dpy.y)) / grid.y;
    let mean = mix(HL_ROOM * 1.2, FF_BLIND_TONE * 0.9, FF_BLIND_SHARE);
    let a = smoothstep(0.3, 0.8, max(fx, fy));
    if (a >= 1.0) {
        return mean; // a window under a pixel: its mean, no room looked up (the same frame)
    }
    let gf = in.group ^ (hl_face(n) << 24u);
    let w = ff_h(vec2<f32>(floor(along), floor(up)), gf, 21u);
    let pale = step(w, FF_BLIND_SHARE);
    let room = mix(HL_ROOM * mix(0.6, 1.8, ff_h(vec2<f32>(floor(along), floor(up)), gf, 22u)), FF_BLIND_TONE * mix(0.6, 1.2, fract(w * 7.3)), pale);
    return mix(room, mean, a);
}

// ---- (`--far-facades`) an untextured far wall's facade ----
// A building the far layer draws as its plain prism (one with no kit plan and
// no model here, as the Ferry Building is) was a bare extrusion in one far
// tone: the far cell draws it at any distance while it is resident. Its walls
// are drawn here by the shader alone, from the top down (the prism's foot is
// its footprint's lowest ground, under a quay or a pier deck, not the street
// it stands on): a cornice with dentils under its coping, an attic storey of
// small windows over a belt course, then storeys of tall round-headed windows
// in stone surrounds with keystones and sills between pilasters, a belt course
// at each floor; each opening a recess its reveal shades and the low sun
// reaches only where the ray clears it; the stone aged (mottled, washed down
// below each sill, sooted in plumes from the cornice). The wall's top and its
// first corner come flat from the quad's first vertex (hlod_bake.rs: an
// untextured wall quad starts at its top corner). INFERRED proportions.
const FB_BAY_M: f32 = 4.6;
// (`--far-relief`) the stone front's sandstone (linear albedo of a weathered buff freestone, sRGB
// about 182/175/161: a light grey-buff), the soot crust where the rain never washes it (its
// multiplier at full shelter) and the rain-washed upper faces of its projections (their
// multiplier). INFERRED from photographs of the waterfront's sandstone fronts.
const FB_SANDSTONE: vec3<f32> = vec3<f32>(0.47, 0.43, 0.355);
const FB_SOOT: vec3<f32> = vec3<f32>(0.42, 0.40, 0.38);
const FB_WASHED: f32 = 1.12;
const FB_PIER_M: f32 = 1.4;
const FB_STOREY_M: f32 = 5.0;
const FB_CORNICE_M: f32 = 1.15;
const FB_ATTIC_M: f32 = 2.5;

fn fb_cov(d: f32, w: f32) -> f32 {
    return clamp(0.5 - d / max(w, 1e-4), 0.0, 1.0);
}

fn fb_rect(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>) -> f32 {
    let q = abs(p - c) - h;
    return max(q.x, q.y);
}

// a round-headed opening `w` wide from `y0`: a rectangle to its springing, a half circle over it.
fn fb_arch(p: vec2<f32>, y0: f32, w: f32, spring: f32) -> f32 {
    let r = 0.5 * w;
    let rect = fb_rect(p, vec2<f32>(0.0, 0.5 * (y0 + spring)), vec2<f32>(r, 0.5 * (spring - y0)));
    let circ = select(1e3, length(p - vec2<f32>(0.0, spring)) - r, p.y >= spring);
    return min(rect, circ);
}

// an opening's signed distance: round-headed (`arch`) or square.
fn fb_open(p: vec2<f32>, arch: bool, hw: f32, y0: f32, y1: f32) -> f32 {
    if (arch) {
        return fb_arch(p, y0, 2.0 * hw, y1 - hw);
    }
    return fb_rect(p, vec2<f32>(0.0, 0.5 * (y0 + y1)), vec2<f32>(hw, 0.5 * (y1 - y0)));
}

struct FbOut {
    albedo: vec3<f32>,
    glass: f32,
    n: vec3<f32>,
    rough: f32,
    // (`--far-relief`) the sky's visibility under the projections (the ambient terms only); 1 without it
    ao: f32,
    // (`--far-relief`) the lit window's light across its glass (1: even); 1 without it
    lit: f32,
};

// (`--far-relief`) the stone front's projections (metres out of the wall
// face) and the arcade's depth: the cornice's corona over its drip, a belt
// course, a sill, a pilaster, a pavilion's front over the wall between them.
// INFERRED proportions (a Beaux-Arts front of dressed stone).
const FB_CORONA_P_M: f32 = 0.85;
const FB_BELT_P_M: f32 = 0.22;
const FB_SILL_P_M: f32 = 0.12;
const FB_PILASTER_P_M: f32 = 0.12;
const FB_PAV_P_M: f32 = 0.45;
// the ground floor's arcade (the third storey under the attic): a corridor
// this deep behind round-headed openings this wide, to this springing.
const FB_ARCADE_DEPTH_M: f32 = 2.6;
const FB_ARCADE_HW_M: f32 = 1.55;
const FB_ARCADE_SPRING_M: f32 = 3.0;
// a pavilion of three bays at the wall's first corner and every FB_PAV_PERIOD bays along it.
const FB_PAV_PERIOD: f32 = 13.0;
// a waterfront quay's deck over the sea (the Embarcadero's apron stands about this high).
const FB_QUAY_DECK_M: f32 = 2.5;

// the share of a pixel `w` wide in the shade of an edge that throws it `reach` (0 none) past `x` (the distance from the edge).
fn fb_shade(x: f32, reach: f32, w: f32) -> f32 {
    return select(0.0, clamp((reach - x) / max(w, 1e-4) + 0.5, 0.0, 1.0), reach > 0.0 && x > -0.5 * w);
}

// the sky a wall point `y` under a projection `p` deep still sees: the projection hides the part of the
// upper half of its view hemisphere within the angle atan(p / y) of the wall (half the sky at its root).
fn fb_sky(y: f32, p: f32) -> f32 {
    let yy = max(y, 0.0);
    return 1.0 - 0.5 * (1.0 - yy / sqrt(yy * yy + p * p));
}

fn fb_facade(in: HlOut, n: vec3<f32>, tone: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>) -> FbOut {
    var o: FbOut;
    let corner = ff_corner_of(in);
    let top = corner.y;
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let t2 = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
    let t = vec3<f32>(t2.x, 0.0, t2.y);
    let along = abs(dot(in.world.xz - corner.xz, t2));
    let dt = top - in.world.y;
    let fw = vec2<f32>(abs(dot(dpx, t)) + abs(dot(dpy, t)), abs(dpx.y) + abs(dpy.y)) + vec2<f32>(1e-3);
    let fwm = max(fw.x, fw.y);
    let g = in.group;
    // the stone: the far tone desaturated and warmed toward a weathered sandstone, kept matte.
    let lum = dot(tone, vec3<f32>(0.2126, 0.7152, 0.0722));
    var stone = mix(vec3<f32>(lum) * vec3<f32>(1.08, 1.0, 0.86), tone, 0.25);
    stone = clamp(stone * mix(0.78, 0.98, hl_h3(g, 3, 501u)), vec3<f32>(0.1), vec3<f32>(0.46));
    if (SF_FAR_RELIEF) {
        // (`--far-relief`) a buff sandstone, not the prism's far tone (a plain prism's palette tone is
        // its glass's pale blue): warm, matte, each building a little its own
        stone = FB_SANDSTONE * mix(0.86, 1.04, hl_h3(g, 3, 501u));
    }
    let trim = min(stone * 1.18, vec3<f32>(0.56));
    var col = stone;
    var nn = n;
    var occ = 1.0;
    var glass = 0.0;
    var rough = 0.88;
    var sun_cut = 0.0;
    // the key light and the view in the wall's frame (along, up, out of the wall)
    let l = look.key_dir.xyz;
    let sl = vec3<f32>(dot(l, t), l.y, dot(l, n));
    let vdir = normalize(in.world - frame.cam_pos.xyz);
    let dir = vec3<f32>(dot(vdir, t), vdir.y, -dot(vdir, n));
    let dz = max(dir.z, 0.06);
    // bays from the wall's first corner (a pier at it), pilasters between them
    let in_bays = along > FB_PIER_M;
    let bay = floor(max(along - FB_PIER_M, 0.0) / FB_BAY_M);
    let x = along - FB_PIER_M - (bay + 0.5) * FB_BAY_M;
    // the storey: the attic under the cornice, then the floors below it
    let body = dt - FB_CORNICE_M - FB_ATTIC_M - 0.35;
    let attic = dt > FB_CORNICE_M && body < 0.0;
    let k = floor(max(body, 0.0) / FB_STOREY_M);
    // height over the storey's floor (the attic's over its belt course)
    var py = FB_STOREY_M - (body - k * FB_STOREY_M);
    if (attic) {
        py = FB_ATTIC_M - (dt - FB_CORNICE_M);
    }
    let arch = !attic;
    var hw = select(0.95, 0.55, attic);
    var y0 = select(0.95, 0.55, attic);
    var y1 = select(4.15, 1.75, attic);
    var depth = select(0.4, 0.25, attic);
    // (`--far-relief`) the sun's reach across the wall's frame: how far down (per metre out) and sideways it throws a shade
    let relief = SF_FAR_RELIEF;
    let lit_face = sl.z > 0.02 && sl.y > 0.0;
    let drop = select(0.0, sl.y / max(sl.z, 0.02), lit_face);
    // `along` runs away from the first corner, with or against t: the sun's sideways reach along it
    let sgn = select(-1.0, 1.0, dot(in.world.xz - corner.xz, t2) >= 0.0);
    let side = select(0.0, sgn * sl.x / max(sl.z, 0.02), lit_face);
    var vis = 1.0;
    var sky = 1.0;
    var lit = 1.0; // (`--far-relief`) the window light's share across a shop front (its shelves dark, its lights bright)
    // (`--far-relief`) the ground floor's arcade: the storey 1.5 m over the ground it stands on (the cell's lowest
    // point, or on the waterfront the quay's deck over the sea, whose ground the prism's foot reaches under it),
    // and any under it, below a storey of windows at least; and the pavilions
    let ground = max(corner.w, LM_SEA_LEVEL_Y + FB_QUAY_DECK_M) + 1.5;
    let ka = floor((top - ground - FB_CORNICE_M - FB_ATTIC_M - 0.35) / FB_STOREY_M);
    let arcade = relief && !attic && ka >= 1.0 && k >= ka;
    if (relief && !arcade) {
        depth = depth + 0.15; // a deeper reveal: the stone's full bed, the frame set back in it
    }
    var pav = 0.0;
    if (relief) {
        if (arcade) {
            hw = FB_ARCADE_HW_M;
            y0 = 0.0;
            y1 = FB_ARCADE_SPRING_M + FB_ARCADE_HW_M;
            depth = FB_ARCADE_DEPTH_M;
        }
        let m = bay - FB_PAV_PERIOD * floor(bay / FB_PAV_PERIOD);
        pav = select(0.0, 1.0, in_bays && (bay < 3.0 || (m >= 6.0 && m < 9.0)));
    }
    var opening = 1e3;
    if (in_bays && dt > FB_CORNICE_M && (attic || body > 0.35)) {
        opening = fb_open(vec2<f32>(x, py), arch, hw, y0, y1);
    }
    let pq = vec2<f32>(x, py);
    let key = bay * 3.0 + select(k * 17.0 + 1.0, 0.0, attic);
    if (opening < 0.0) {
        // inside the opening: the ray runs into the recess, to the glass or a reveal
        let hit = pq + dir.xy / dz * depth;
        let inside = fb_open(hit, arch, hw - 0.07, y0 + 0.07, y1 - 0.07) < 0.0;
        var tau = depth;
        if (arcade) {
            // (`--far-relief`) the arcade: the back wall's shop fronts, or the vault and the paving in its shade
            if (hit.y > y1 - 0.3) {
                col = trim * 0.62; // the vault's soffit
                nn = -up;
                occ = 0.55;
                sky = 0.35;
                if (relief) {
                    col = trim * FB_SOOT * 0.9; // (`--far-relief`) sooted: the rain never reaches it
                    occ = 0.42;
                    sky = 0.22;
                }
                tau = depth * clamp((y1 - 0.3 - py) / max(hit.y - py, 1e-3), 0.0, 1.0);
            } else if (hit.y < 0.05) {
                col = stone * 0.7; // the arcade's paving
                nn = up;
                occ = 0.6;
                sky = 0.4;
                tau = depth * clamp(py / max(py - hit.y, 1e-3), 0.0, 1.0);
            } else {
                // the back wall: a shop front between stone piers, each bay its own: a kick plate, a
                // bronze frame of three lights under a transom, and through it the shop's shelves
                // under its lit ceiling (darker between the shelves, brightest under the lights)
                let sx = fract((hit.x + FB_BAY_M * 0.5) / FB_BAY_M) * FB_BAY_M - FB_BAY_M * 0.5;
                let shop = fb_cov(abs(sx) - 1.35, fw.x) * fb_cov(hit.y - 3.2, fw.y);
                let hs = ff_h(vec2<f32>(bay, 7.0), g, 61u);
                let ax = abs(sx);
                let frame = max(max(fb_cov(abs(ax - 0.45) - 0.03, fw.x), fb_cov(abs(hit.y - 2.5) - 0.04, fw.y)), max(fb_cov(hit.y - 0.45, fw.y), fb_cov(1.27 - ax, fw.x)));
                let row = floor(hit.y / 0.42);
                let shelf = fb_cov(abs(fract(hit.y / 0.42) - 0.08) - 0.06, fw.y) * step(hit.y, 2.2);
                let item = ff_noise(vec2<f32>(hit.x * 4.3 + bay * 7.0, row * 3.1), g ^ 0x3bu);
                let hue = ff_h(vec2<f32>(bay, row), g, 63u);
                let goods = mix(mix(vec3<f32>(0.30, 0.20, 0.12), vec3<f32>(0.16, 0.22, 0.30), step(0.55, hue)), vec3<f32>(0.42, 0.36, 0.26), step(0.85, hue)) * (0.45 + 0.8 * item);
                let ceiling = smoothstep(2.0, 2.45, hit.y);
                let interior = mix(mix(goods, vec3<f32>(0.08, 0.07, 0.06), shelf), vec3<f32>(0.55, 0.50, 0.42), ceiling * select(0.0, 1.0, hit.y < 2.46));
                let pane = shop * (1.0 - frame);
                col = mix(mix(stone * select(0.8, 0.62, relief), vec3<f32>(0.06, 0.05, 0.04), shop * frame), interior * mix(0.75, 1.15, hs), pane);
                glass = pane;
                lit = mix(0.35 + 0.5 * item * (1.0 - shelf), 1.6, ceiling);
                rough = mix(0.85, 0.2, pane);
                nn = n;
                occ = 0.7;
                sky = 0.3; // the back wall sees the sky only through the opening
            }
        } else if (inside) {
            // the glass: each window's room (a blind, a lit ceiling or the dark), its sash bars
            let hr = ff_h(vec2<f32>(key, f32(g % 4093u)), g, 23u);
            let room = select(HL_ROOM * mix(0.7, 1.6, hr), FF_BLIND_TONE * mix(0.7, 1.15, fract(hr * 9.1)), hr < FF_BLIND_SHARE);
            let bars = max(fb_cov(abs(fract(hit.x / 0.64 + 0.5) - 0.5) * 0.64 - 0.025, fw.x), fb_cov(abs(fract((hit.y - y0) / 0.8) - 0.5) * 0.8 - 0.025, fw.y));
            col = mix(room, stone * 0.3, bars);
            glass = 1.0 - bars;
            rough = mix(0.88, 0.15, glass);
            nn = n;
        } else {
            // the reveal: a jamb or the soffit of the head
            col = trim * 0.88;
            tau = depth * 0.5;
            occ = 0.62;
            nn = normalize(n * 0.3 + t * (-sign(dir.x)) * 0.95);
            sky = 0.6;
        }
        if (relief && !arcade && inside) {
            sky = 0.75; // the glass at the back of its reveal
        }
        if (sl.z > 0.02) {
            let q = pq + dir.xy / dz * tau + sl.xy / sl.z * tau;
            sun_cut = step(0.0, fb_open(q, arch, hw, y0, y1));
        }
    } else {
        // the wall: dressed stone in courses, the surrounds, keystones and sills, the pilasters
        let course = 0.6;
        let joint_y = abs(fract(dt / course) - 0.5) * course;
        let row = floor(dt / course);
        let joint_x = abs(fract(along / 1.3 + 0.5 * (row % 2.0)) - 0.5) * 1.3;
        let joints = max(fb_cov(joint_y - 0.014, fw.y), fb_cov(joint_x - 0.014, fw.x) * 0.7);
        let keep = 1.0 - smoothstep(0.03, 0.12, fwm);
        col = col * (1.0 - 0.3 * joints * keep);
        if (relief) {
            // (`--far-relief`) each block its own tone; the ground floor and the pavilions rusticated
            // (channelled joints, deep enough to hold their shade from a pixel away)
            let block = floor(along / 1.3 + 0.5 * (row % 2.0));
            col = col * (1.0 + 0.09 * (ff_h(vec2<f32>(block, row), g, 71u) - 0.5) * (1.0 - smoothstep(0.4, 1.4, fwm)));
            if (arcade || pav > 0.5) {
                let chan = fb_cov(joint_y - 0.05, fw.y);
                col = col * (1.0 - 0.38 * chan);
                vis = vis * (1.0 - chan * 0.5 * select(0.0, 1.0, lit_face));
            }
        }
        // a pilaster at each bay's edge: a stone strip standing out of the wall, lit on its side
        let edge = 0.5 * FB_BAY_M - abs(x);
        let pil = fb_cov(edge - 0.42, fw.x) * select(0.0, 1.0, in_bays && dt > FB_CORNICE_M);
        col = mix(col, trim * 0.97, pil);
        nn = normalize(mix(nn, n + t * sign(x) * 0.8 * smoothstep(0.32, 0.42, edge), pil));
        if (relief && in_bays && dt > FB_CORNICE_M) {
            // (`--far-relief`) the pilaster's shade on the wall beside it, on the side away from the sun
            let toward = select(-1.0, 1.0, x > 0.0); // the nearer pilaster's side
            let reach = FB_PILASTER_P_M * max(side * toward, 0.0);
            vis = vis * (1.0 - fb_shade(edge - 0.42, reach, fw.x) * (1.0 - pil));
        }
        if (opening < 0.26 && opening < 1e2) {
            let band = fb_cov(opening - 0.24, fwm);
            col = mix(col, trim, band);
            if (arch) {
                let ks = fb_cov(fb_rect(pq, vec2<f32>(0.0, y1 + 0.1), vec2<f32>(0.2, 0.28)), fwm);
                col = mix(col, trim * 1.06, ks);
            }
        }
        if (in_bays && opening < 1e2 && !arcade) {
            // the sill: a projecting slab, its shade, and the wash below its ends
            let sill = fb_cov(fb_rect(pq, vec2<f32>(0.0, y0 - 0.08), vec2<f32>(hw + 0.2, 0.08)), fwm);
            col = mix(col, trim * select(1.05, 1.05 * FB_WASHED, relief), sill); // (`--far-relief`: its top washed by the rain)
            nn = normalize(mix(nn, n + up * 1.0, sill));
            let below = y0 - 0.16 - py;
            if (below > 0.0 && abs(x) < hw + 0.22) {
                occ = occ * mix(0.55, 1.0, smoothstep(0.0, 0.2, below));
                let streak = exp(-below / 1.6) * (0.5 + 0.5 * ff_noise(vec2<f32>(along * 6.0, dt * 0.7), g));
                col = col * (1.0 - 0.3 * streak);
            }
            if (relief && abs(x + side * min(max(below, 0.0) / max(drop, 1e-3), FB_SILL_P_M)) < hw + 0.2) {
                // (`--far-relief`) the sill's shade down the wall under it
                vis = vis * (1.0 - fb_shade(below, FB_SILL_P_M * drop, fw.y) * (1.0 - sill));
            }
            if (relief && abs(x) < hw + 0.2 && below > -0.02) {
                sky = sky * mix(fb_sky(below, FB_SILL_P_M), 1.0, sill);
            }
        }
        // the belt course at each floor and over the attic's foot, and its shade
        let bb = select(py - FB_STOREY_M + 0.15, py + 0.18, attic);
        let belt = fb_cov(abs(bb) - 0.16, fw.y) * select(0.0, 1.0, dt > FB_CORNICE_M);
        col = mix(col, trim * select(1.0, FB_WASHED, relief), belt); // (`--far-relief`: its top washed by the rain)
        nn = normalize(mix(nn, n + up * 0.7, belt));
        if (relief && !attic && dt > FB_CORNICE_M) {
            // (`--far-relief`) the belt's shade down the storey under it, and the sky it hides
            vis = vis * (1.0 - fb_shade(FB_STOREY_M - 0.31 - py, FB_BELT_P_M * drop, fw.y) * (1.0 - belt));
            sky = sky * mix(fb_sky(FB_STOREY_M - 0.31 - py, FB_BELT_P_M), 1.0, belt);
        }
    }
    if (relief && in_bays && dt > FB_CORNICE_M) {
        // (`--far-relief`) a pavilion stands FB_PAV_P_M proud of the wall: its stone a little cleaner, and at its
        // edges the wall beside it in the shade of its side on the side away from the sun
        col = col * mix(1.0, 1.07, pav);
        let pb = bay - FB_PAV_PERIOD * floor(bay / FB_PAV_PERIOD);
        let first = bay >= 3.0 && pb == 6.0;
        let last = bay == 2.0 || pb == 8.0;
        let xb = x + 0.5 * FB_BAY_M; // from the bay's first edge
        // just outside a pavilion: the bay before its first, the bay after its last
        let before = pav < 0.5 && (pb == 5.0);
        let after = pav < 0.5 && (pb == 9.0 || bay == 3.0);
        let dist_after = xb;
        let dist_before = FB_BAY_M - xb;
        var pshade = 0.0;
        if (after) {
            pshade = fb_shade(dist_after, FB_PAV_P_M * max(-side, 0.0), fw.x);
        } else if (before) {
            pshade = fb_shade(dist_before, FB_PAV_P_M * max(side, 0.0), fw.x);
        }
        vis = vis * (1.0 - pshade);
        // the pavilion's own returns, seen edge on: a thin side face at its first and last edge
        let ret = FB_PAV_P_M * abs(dir.x) / dz;
        if (first || last) {
            let e = select(FB_BAY_M - xb, xb, first);
            let rc = fb_cov(e - ret, fw.x) * pav;
            col = mix(col, stone * 0.9, rc);
            nn = normalize(mix(nn, t * select(1.0, -1.0, first), rc));
        }
    }
    // the cornice: coping, corona over dentils, frieze, its shade and soot below
    if (dt < FB_CORNICE_M + 0.7) {
        if (dt < 0.22) {
            col = trim * 0.9;
            nn = normalize(n * 0.4 + up);
        } else if (dt < 0.52) {
            col = trim;
            nn = normalize(n + up * 0.3);
        } else if (dt < 0.72) {
            let dent = fb_cov(abs(fract(along / 0.34) - 0.5) * 0.34 - 0.09, fw.x);
            col = mix(trim * 0.32, trim, dent);
            occ = occ * mix(0.55, 1.0, dent);
        } else if (dt < FB_CORNICE_M) {
            col = mix(col, trim * 0.97, 0.7);
        } else {
            let below = dt - FB_CORNICE_M;
            occ = occ * mix(0.5, 1.0, smoothstep(0.0, 0.7, below));
        }
    }
    if (relief && dt > 0.52) {
        // (`--far-relief`) the corona's shade: from its drip down the frieze, the attic and as far as the sun throws it
        vis = vis * (1.0 - fb_shade(dt - 0.52, FB_CORONA_P_M * drop, fw.y));
        sky = sky * fb_sky(dt - 0.52, FB_CORONA_P_M);
        // each bay weathered its own: a wash down the pier from its cornice, soot pooled where the rain does not reach
        let hb = ff_h(vec2<f32>(bay, 3.0), g, 83u);
        col = col * (0.95 + 0.1 * hb);
        let wash = smoothstep(0.55, 0.95, ff_noise(vec2<f32>(along / 0.9, dt / 6.0), g ^ 0x5du)) * (1.0 - smoothstep(1.0, 9.0, dt));
        col = col * (1.0 - 0.22 * wash * (1.0 - smoothstep(0.5, 2.0, fwm)));
    }
    // the stone's age: mottled, soot plumes from the cornice, darker low down the wall
    let mottle = ff_noise(vec2<f32>(along / 2.3, dt / 1.7), g ^ 0x77u);
    col = col * (0.84 + 0.3 * mottle);
    let plume = smoothstep(0.5, 0.9, ff_noise(vec2<f32>(along / 3.1, 0.0), g ^ 0x99u)) * (1.0 - smoothstep(2.0, 12.0, dt));
    col = col * (1.0 - 0.3 * plume);
    col = col * mix(1.0, 0.82, smoothstep(9.0, 16.0, dt));
    // the recess's sun shadow where the wall faces the key light
    let facing = smoothstep(0.0, 0.2, sl.z);
    if (relief) {
        // (`--far-relief`) the reveal's shade over the key light only (the sky still reaches into it)
        vis = vis * (1.0 - sun_cut * facing);
        fr_sun_vis = vis;
    } else {
        col = col * mix(1.0, 0.35, sun_cut * facing);
    }
    if (relief) {
        // (`--far-relief`) the soot crust where the rain never washes the stone: under the cornice, the
        // belt courses and the sills, in the reveals and the arcade (the shelter the sky cut measures)
        col = col * mix(vec3<f32>(1.0), FB_SOOT, clamp((1.0 - sky) * 1.4, 0.0, 1.0) * (1.0 - glass));
    }
    o.albedo = col * mix(1.0, occ, 0.6);
    o.glass = glass;
    o.n = nn;
    o.rough = rough;
    o.ao = select(1.0, sky, relief);
    o.lit = select(1.0, lit, relief);
    return o;
}

// the wall's first corner (xyz, the top in y) and the cell's lowest point (w), flat from the quad's first vertex.
fn ff_corner_of(in: HlOut) -> vec4<f32> {
    return vec4<f32>(in.world, in.world.y - 10.0); // @ff-corner-read
}

// ---- (`--far-relief`) the far layer's construction depth ----
// A district roof's distances to its footprint's box (hlod_district.rs
// `roof_frame_words`: along -u, +u, -v, +v, in metres), interpolated over the
// roof; off (compiled out) the reader returns none.
fn fr_d_of(in: HlOut) -> vec4<f32> {
    return vec4<f32>(-1.0); // @fr-roof-read
}

// A district roof under its parapet (hlod_district.rs `roof_frame_words`):
// the roof lies the parapet's height under the prism's top, so the eye's ray
// meets it that much further on (away from the eye), or meets the inner face of
// the parapet across from the eye first; on top, the parapet's coping. The
// parapet on the sun's side shades the roof as far as the sun's height throws
// it (the key light only: `fr_sun_vis`, look_light). Every band is a coverage
// over the pixel's footprint along the box's axes, so it holds its mean as it
// shrinks. INFERRED: the parapet's height (hlod_district.rs ROOF_PARAPET_*_M)
// and its thickness.
const FR_CAP_M: f32 = 0.3;
// the longest parapet shade drawn (a sun 1 degree up off a 1.5 m wall): beyond it the cascades' own shade holds.
const FR_SHADE_MAX_M: f32 = 60.0;

struct FrRoof {
    // the roof point the eye's ray reaches (xz) and the share of the pixel on the roof itself
    at: vec2<f32>,
    roof: f32,
    // the coping's share and the parapet's inner face's share, and that face's normal
    cap: f32,
    face: f32,
    face_n: vec3<f32>,
    // the key light's visibility on the roof's share (the parapet's shade)
    sun: f32,
};

fn fr_roof(in: HlOut, dpx: vec3<f32>, dpy: vec3<f32>) -> FrRoof {
    var o: FrRoof;
    o.at = in.world.xz;
    o.roof = 1.0;
    o.cap = 0.0;
    o.face = 0.0;
    o.face_n = vec3<f32>(0.0, 1.0, 0.0);
    o.sun = 1.0;
    let d0 = fr_d_of(in);
    // the heading, height and scale ride in the roof ring's glass and phase bytes (flat)
    let upper = u32(round(in.glass * 255.0)) | (u32(round(in.phase * 255.0)) << 8u);
    let hp = f32((upper >> 7u) & 15u) * 0.1;
    if (d0.x < 0.0 || hp <= 0.0) {
        return o;
    }
    let th = f32(upper & 127u) * (LK_PI / 128.0);
    let eu = vec2<f32>(cos(th), sin(th));
    let ev = vec2<f32>(-eu.y, eu.x);
    let fu = abs(dot(dpx.xz, eu)) + abs(dot(dpy.xz, eu)) + 1e-3;
    let fv = abs(dot(dpx.xz, ev)) + abs(dot(dpy.xz, ev)) + 1e-3;
    let fw = vec4<f32>(fu, fu, fv, fv);
    // the coping: the first FR_CAP_M in from each side at the top
    let capv = clamp((vec4<f32>(FR_CAP_M) - d0) / fw + 0.5, vec4<f32>(0.0), vec4<f32>(1.0));
    o.cap = max(max(capv.x, capv.y), max(capv.z, capv.w));
    // the ray on down to the roof, hp under the top: displaced away from the eye
    let te = normalize(frame.cam_pos.xyz - in.world);
    let shift = -te.xz * (hp / max(te.y, 0.05));
    let su = dot(shift, eu);
    let sv = dot(shift, ev);
    let d = d0 + vec4<f32>(su, -su, sv, -sv);
    o.at = in.world.xz + shift;
    // past a side's line the ray met that side's inner face (it faces back into the roof)
    let past = clamp(0.5 - d / fw, vec4<f32>(0.0), vec4<f32>(1.0));
    o.face = max(max(past.x, past.y), max(past.z, past.w));
    var fn2 = eu;
    var best = past.x;
    if (past.y > best) {
        best = past.y;
        fn2 = -eu;
    }
    if (past.z > best) {
        best = past.z;
        fn2 = ev;
    }
    if (past.w > best) {
        fn2 = -ev;
    }
    o.face_n = vec3<f32>(fn2.x, 0.0, fn2.y);
    // the parapet's shade toward the sun: within reach of the side between the point and the sun
    let l = look.key_dir.xyz;
    let lh = length(l.xz);
    if (l.y > 0.0 && lh > 1e-3) {
        let reach = min(hp * lh / max(l.y, 0.017), FR_SHADE_MAX_M);
        let tu = dot(l.xz, eu) / lh;
        let tv = dot(l.xz, ev) / lh;
        let r = reach * vec4<f32>(max(-tu, 0.0), max(tu, 0.0), max(-tv, 0.0), max(tv, 0.0));
        let sh = clamp((r - d) / fw + 0.5, vec4<f32>(0.0), vec4<f32>(1.0)) * step(vec4<f32>(1e-3), r);
        o.sun = (1.0 - sh.x) * (1.0 - sh.y) * (1.0 - sh.z) * (1.0 - sh.w);
    }
    o.face = o.face * (1.0 - o.cap);
    o.roof = max(1.0 - o.cap - o.face, 0.0);
    return o;
}

fn hl_shade(in: HlOut) -> vec4<f32> {
    if (SF_FAR_RELIEF) {
        fr_sun_vis = 1.0; // (`--far-relief`) the relief's shade over the key light, set below
    }
    // Derivatives and the atlas first, in uniform control flow.
    let dpx = dpdx(in.world);
    let dpy = dpdy(in.world);
    let duvx = dpdx(in.uv);
    let duvy = dpdy(in.uv);
    let tex = textureSampleGrad(hl_pages, hl_sampler, in.uv, max(in.layer, 0), duvx, duvy);
    let face = normalize(cross(dpx, dpy));
    let v = normalize(frame.cam_pos.xyz - in.world);
    var n = face;
    if (dot(in.normal, in.normal) > 0.25 && (in.flags & HL_DISTRICT) == 0u) { // a district's normal word is its window rhythm
        n = normalize(in.normal);
    }
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    let district = (in.flags & HL_DISTRICT) != 0u;
    var albedo = in.color;
    var glass = 0.0;
    var roofish = (in.flags & HL_ROOF) != 0u;
    var aaa_room = HL_ROOM;
    var aaa_wall_raw = in.color;
    if (in.layer >= 0) {
        albedo = tex.rgb;
        glass = tex.a;
        aaa_wall_raw = tex.rgb;
    }
    if (district) {
        roofish = abs(n.y) > 0.7;
        if (roofish) {
            albedo = in.roof;
        } else {
            glass = in.glass;
            if (FL_GLASS) {
                // the wall is the opaque wall; the glass, by its rhythm or its share, is the room through the pane.
                var cov = glass;
                if (FL_WINDOWS) {
                    cov = hl_window_grid(in, n, dpx, dpy);
                }
                var room = HL_ROOM;
                if (ff_on()) {
                    room = ff_window_room(in, n, dpx, dpy); // (`--far-facades`) each window its own room
                }
                aaa_room = room;
                aaa_wall_raw = in.color;
                albedo = mix(in.color, room, cov);
                glass = cov;
            }
        }
    }
    var ff_rough = -1.0;
    var ff_n = vec3<f32>(0.0);
    var fr_ao = 1.0; // (`--far-relief`) the stone front's sky under its projections
    var fr_lit = 1.0; // (`--far-relief`) its window light across a shop front
    if (ff_on() && !district && !roofish && in.layer < 0 && abs(n.y) < 0.3) {
        // (`--far-facades`) an untextured far wall (a plain prism): its stone facade
        let fb = fb_facade(in, n, albedo, dpx, dpy);
        albedo = fb.albedo;
        glass = fb.glass;
        aaa_wall_raw = fb.albedo;
        ff_rough = fb.rough;
        ff_n = fb.n;
        if (SF_FAR_RELIEF) {
            fr_ao = fb.ao;
            fr_lit = fb.lit;
        }
    }
    if (wx_level() != 0u && !roofish) {
        let age_c = hl_age(in.group);
        albedo = albedo * mix(vec3<f32>(1.0), age_c, 1.0 - glass); // weathering (`--grime`): each building its own tone
        aaa_wall_raw = aaa_wall_raw * age_c;
    }
    // (`--far-relief`) a district roof under its parapet: where the eye's ray meets the roof, the parapet's face or its coping
    let relief_roof = SF_FAR_RELIEF && district && roofish && n.y > 0.7;
    var rr: FrRoof;
    rr.at = in.world.xz;
    rr.roof = 1.0;
    rr.cap = 0.0;
    rr.face = 0.0;
    rr.face_n = vec3<f32>(0.0, 1.0, 0.0);
    rr.sun = 1.0;
    if (relief_roof) {
        rr = fr_roof(in, dpx, dpy);
    }
    if (ff_on() && roofish && n.y > 0.7) {
        // (`--far-facades`) a flat roof's membrane, and on a district's its plant, hatches and their shade
        let fr = ff_roof(vec3<f32>(rr.at.x, in.world.y, rr.at.y), dpx, dpy, in.group, albedo, district);
        albedo = fr.rgb;
        ff_rough = fr.a;
    }
    if (relief_roof && rr.roof < 1.0) {
        // the parapet is the wall carried up (the wall's colour word, aged as the wall), the coping a little paler
        var wall = in.color;
        if (wx_level() != 0u) {
            wall = wall * hl_age(in.group);
        }
        albedo = albedo * rr.roof + wall * rr.face + min(wall * 1.1, vec3<f32>(0.75)) * rr.cap;
        ff_rough = mix(select(0.92, ff_rough, ff_rough >= 0.0), 0.88, rr.face + rr.cap);
        ff_n = normalize(vec3<f32>(0.0, rr.roof + rr.cap, 0.0) + rr.face_n * rr.face);
    }
    if (relief_roof) {
        fr_sun_vis = rr.roof * rr.sun + rr.face + rr.cap; // the parapet's shade over the key light
    }
    var s: Surface;
    s.albedo = albedo;
    s.rough = mix(select(0.88, 0.92, roofish), select(0.15, 0.45, district && !FL_GLASS), glass); // a district pane as glossy as a cell's
    if (ff_rough >= 0.0) {
        s.rough = ff_rough;
    }
    s.metal = 0.0;
    s.ao = 1.0;
    if (SF_FAR_RELIEF) {
        s.ao = fr_ao;
    }
    // a tower's panes tilt panel by panel (by the glass share at this pixel).
    var n_pane = n;
    if (glass > 0.0 && !roofish) {
        n_pane = normalize(mix(n, hl_panel_normal(in, n, dpx, dpy), glass));
    }
    if (dot(ff_n, ff_n) > 0.5) {
        n_pane = ff_n; // (`--far-facades`) the stone's relief
    }
    s.n = n_pane;
    s.alpha = 1.0;
    s.env = -1;
    var m: Material;
    m.base = vec4<f32>(1.0);
    m.emissive = vec4<f32>(0.0);
    m.params = vec4<f32>(s.rough, 0.0, 0.0, 0.0);
    m.layer = -1;
    m.flags = 0u;
    m.rough_flatten = 0.0;
    m.thin_absorption = 0.0;
    rm_emit = hl_windows(in, in.world, n, glass, dpx, dpy);
    if (SF_FAR_RELIEF) {
        rm_emit = rm_emit * fr_lit;
    }
    // (`--tower-crowns`) a tall tower's crown, on its glass and its opaque wall alike.
    if (FL_TOWER_CROWNS && (in.flags & HL_TOWER) != 0u && !roofish && sky_on(skyu.flags.x, SKY_LIGHTS) && look.rooms_env.x > 0.0) {
        let crown_base = skyu.bloom_w2.z * 25.0 / LK_PI * look.rooms_env.x;
        rm_emit = rm_emit + hl_tower_crown(in, in.world, n, abs(dpx.y) + abs(dpy.y), crown_base);
    }
// sf:AAA_BUILDINGS-
    if (FL_GLASS && !roofish) {
        rm_emit = rm_emit + hl_glass_sky(in, n_pane, v, glass); // the tilted pane
    }
// sf:end
// sf:AAA_BUILDINGS+
    aaa_bldg_top_y = hl_group_top[in.group].y;
    if (roofish || abs(n.y) >= 0.40) {
        // Keep HLOD flat & pitched roofs strictly matte and apply skyline-field canyon AO.
        s.rough = max(s.rough, 0.78);
        aaa_bldg_on = SF_AAA_SKYLINE_FIELD;
        aaa_glass_cov = 0.0;
    } else if (!roofish && abs(n.y) < 0.40) {
        let is_tower = (in.flags & HL_TOWER) != 0u;
        let mat_cls = (in.flags >> 4u) & 7u;
        let grid = hl_grid(in);
        let t_ax = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
        let along_m = dot(in.world.xz, t_ax);
        let fx_m = max(abs(dot(dpx.xz, t_ax)) + abs(dot(dpy.xz, t_ax)), 1e-3);
        let fy_m = max(abs(dpx.y) + abs(dpy.y), 1e-3);
        let gf = in.group ^ (hl_face(n) << 24u);

        // Recover pure opaque wall albedo (for cells & untextured prisms, unmix baked dark room so
        // dual-lobe weighting by (1 - glass) never double-darkens mullions & spandrels).
        var wall_albedo = aaa_wall_raw;
        if (!district && glass > 0.01) {
            wall_albedo = clamp((aaa_wall_raw - aaa_room * glass) / max(1.0 - glass, 0.14), vec3<f32>(0.045), vec3<f32>(0.82));
        }
        var wall_rough = select(0.88, ff_rough, ff_rough >= 0.0);
        var wall_metal = 0.0;
        var glass_cov = select(0.0, glass, FL_GLASS);
        var room_albedo = aaa_room;

        // Glazing F0 & architectural coating tint from explicit material class (bits 4..6) or per-tower palette.
        var glass_f0 = vec3<f32>(0.090);
        if (mat_cls == 1u) {
            glass_f0 = vec3<f32>(0.235, 0.285, 0.340); // curtain-blue low-E
            wall_rough = min(wall_rough, 0.54);
        } else if (mat_cls == 2u) {
            glass_f0 = vec3<f32>(0.215, 0.310, 0.275); // curtain-emerald
            wall_rough = min(wall_rough, 0.54);
        } else if (mat_cls == 3u) {
            glass_f0 = vec3<f32>(0.320, 0.260, 0.195); // curtain-bronze / champagne
            wall_rough = min(wall_rough, 0.50);
        } else if (mat_cls == 4u) {
            glass_f0 = vec3<f32>(0.125, 0.135, 0.148); // granite / stone
            wall_rough = min(wall_rough, 0.72);
        } else if (mat_cls == 5u) {
            glass_f0 = vec3<f32>(0.125, 0.138, 0.152); // precast concrete
            wall_rough = max(wall_rough, 0.86);
        } else if (mat_cls == 6u) {
            glass_f0 = vec3<f32>(0.088, 0.094, 0.100); // brick
            wall_rough = max(wall_rough, 0.88);
        } else if (mat_cls == 7u) {
            glass_f0 = vec3<f32>(0.080, 0.084, 0.088); // wood / Victorian
            wall_rough = max(wall_rough, 0.85);
        } else if (is_tower && FL_TOWER) {
            let h = st_hash(vec2<f32>(f32(in.group) * 0.618, 7.31));
            let h2 = st_hash(vec2<f32>(f32(in.group) * 1.37, 2.9));
            let tint = mix(
                vec3<f32>(0.78, 0.92, 1.06),
                mix(vec3<f32>(0.78, 1.04, 0.92), vec3<f32>(1.06, 0.90, 0.72), step(0.48, h2)),
                step(0.54, h),
            );
            glass_f0 = (0.190 + 0.180 * h) * tint;
            wall_rough = min(wall_rough, mix(0.84, 0.56, step(0.45, glass_cov)));
        } else {
            let h = hl_h3(in.group, 1, 55u);
            glass_f0 = vec3<f32>(0.088 + 0.052 * h) * mix(vec3<f32>(0.94, 0.98, 1.02), vec3<f32>(1.00, 0.97, 0.93), h);
        }

        if (SF_AAA_DETAIL) {
            let bt = hl_group_top[in.group];
            let b_h = max(bt.y - bt.x, 0.0);
            let y_rel = max(in.world.y - bt.x, 0.0);
            let depth_top = max(bt.y - in.world.y, 0.0);

            // (1) Louvered double-height mechanical/refuge floors & crown screen on tall towers (survives at 3+ km).
            if (b_h > 40.0 && (is_tower || b_h > 55.0)) {
                let m_per = mix(52.0, 74.0, hl_h3(in.group, 0, 41u));
                let m_h = mix(5.2, 7.6, hl_h3(in.group, 0, 42u));
                let m_mid = hl_pulse(y_rel / m_per + 0.35, m_h / m_per, fy_m / m_per)
                    * smoothstep(16.0, 30.0, y_rel) * smoothstep(5.0, 14.0, depth_top);
                let crown_h = clamp(b_h * mix(0.035, 0.072, hl_h3(in.group, 0, 43u)), 4.2, 10.5);
                let m_crown = smoothstep(crown_h + fy_m, crown_h - fy_m, depth_top) * step(46.0, b_h);
                let mech = clamp(max(m_mid, m_crown * 0.85), 0.0, 1.0);
                if (mech > 0.001) {
                    glass_cov = glass_cov * (1.0 - 0.88 * mech);
                    let lv = hl_pulse(y_rel / 0.70, 0.45, fy_m / 0.70);
                    wall_albedo = mix(wall_albedo, wall_albedo * (0.28 + 0.16 * lv), mech);
                    wall_rough = mix(wall_rough, 0.46, mech);
                    wall_metal = mix(wall_metal, 0.34, mech);
                }
            }

            // (2) Architectural podium tone break, parapet cap, base plinth soot & rain-wash weathering.
            if (b_h > 10.0) {
                if (b_h > 26.0) {
                    let podium = 1.0 - smoothstep(9.0, 16.0, y_rel);
                    wall_albedo = wall_albedo * mix(vec3<f32>(1.0), vec3<f32>(0.87, 0.86, 0.84), podium * 0.65);
                }
                let coping = smoothstep(1.8 + fy_m, 0.4, depth_top) * step(14.0, b_h);
                wall_albedo = mix(wall_albedo, min(wall_albedo * 1.14, vec3<f32>(0.76)), coping * 0.55);
                let base_g = mix(0.80, 1.0, smoothstep(0.0, 6.0, y_rel));
                let drip_n = 0.55 + 0.45 * hl_h3(gf, i32(floor(along_m / 12.0)), 51u);
                let top_g = 1.0 - 0.14 * drip_n * smoothstep(10.0, 0.0, depth_top);
                wall_albedo = wall_albedo * (base_g * top_g);
            }
            let broad_g = 0.92 + 0.16 * hl_h3(gf, i32(floor(along_m / 18.0)) * 73 + i32(floor(in.world.y / 38.0)), 52u);
            let bld_val = 0.90 + 0.22 * hl_h3(in.group, 3, 53u);
            let bld_hue = mix(vec3<f32>(1.04, 1.00, 0.94), vec3<f32>(0.94, 0.99, 1.05), hl_h3(in.group, 5, 54u));
            wall_albedo = wall_albedo * (broad_g * bld_val) * mix(vec3<f32>(1.0), bld_hue, 0.65);

            // (3) Coherent multi-floor tenant suite blind/interior brightness bands & panel reflectance jitter.
            let suite_h = mix(8.5, 14.0, hl_h3(in.group, 0, 44u));
            let s_idx = i32(floor(in.world.y / suite_h));
            let s_val = hl_h3(gf, s_idx, 45u);
            let fl_i = i32(floor(in.world.y / grid.y - grid.z));
            let fl_v = mix(hl_h3(gf, fl_i, 46u), 0.5, smoothstep(0.4, 1.1, fy_m / grid.y));
            room_albedo = room_albedo * mix(0.45, 1.75, 0.65 * s_val + 0.35 * fl_v);
            let run_i = i32(floor(along_m / (grid.x * 3.0)));
            let pan_fade = 1.0 - smoothstep(0.35, 1.0, max(fx_m / (grid.x * 3.0), fy_m / grid.y));
            let pan_var = (hl_h3(gf, fl_i * 389 + run_i, 47u) - 0.5) * pan_fade;
            glass_f0 = glass_f0 * (1.0 + 0.24 * pan_var);

            // (4) Subtle late-afternoon / golden-hour interior office glow on shadowed tower facades.
            if (is_tower && glass_cov > 0.04 && frame.copy_step.z < 0.5 && skyu.atmo.y > -0.02 && skyu.atmo.y < 0.42 && look.rooms_env.x < 0.45) {
                let late_day = smoothstep(0.42, 0.06, skyu.atmo.y) * smoothstep(-0.02, 0.03, skyu.atmo.y) * (1.0 - smoothstep(0.05, 0.45, look.rooms_env.x));
                let shaded = 1.0 - smoothstep(-0.05, 0.35, dot(n, look.key_dir.xyz));
                let bay_run = i32(floor(along_m / mix(12.0, 22.0, hl_h3(in.group, s_idx, 48u))));
                let lit_suite = smoothstep(0.38, 0.78, s_val) * (0.40 + 0.60 * hl_h3(gf, s_idx * 97 + bay_run, 49u));
                let o_col = mix(vec3<f32>(1.0, 0.80, 0.55), vec3<f32>(0.86, 0.93, 1.0), step(0.58, hl_h3(gf, s_idx, 50u)));
                rm_emit = rm_emit + o_col * (glass_cov * lit_suite * late_day * shaded * 0.048);
            }
        }

        if (SF_AAA_BRDF) {
            let cw_t = select(0.0, 1.0, is_tower && glass_cov > 0.36);
            // High-contrast PBR micro-roughness breakup on HLOD walls: honed podium plinths,
            // crisp anodized metallic mullions on glass curtain walls, and beveled cornice/stringcourse catches.
            let bt_b = hl_group_top[in.group];
            let y_rel_b = max(in.world.y - bt_b.x, 0.0);
            let d_top_b = max(bt_b.y - in.world.y, 0.0);
            if (cw_t > 0.5) {
                wall_rough = min(wall_rough, 0.24);
                wall_metal = max(wall_metal, 0.72);
            } else if (bt_b.y - bt_b.x > 12.0) {
                let plinth_hl = 1.0 - smoothstep(4.0, 9.0, y_rel_b);
                wall_rough = mix(wall_rough, 0.34, plinth_hl * 0.65);
            }
            var n_wall_hl = select(n, ff_n, dot(ff_n, ff_n) > 0.5);
            let keep_rev = 1.0 - smoothstep(0.35, 1.25, fy_m / grid.y);
            if (SF_AAA_DETAIL && keep_rev > 0.01) {
                let up_fl = fract(in.world.y / grid.y - grid.z);
                let ledge_t = (1.0 - smoothstep(0.0, 0.08, up_fl)) * keep_rev;
                let cornice_u = smoothstep(2.2, 0.5, d_top_b) * (1.0 - smoothstep(0.5, 0.0, d_top_b)) * step(10.0, bt_b.y - bt_b.x) * keep_rev;
                let sill_catch = (1.0 - smoothstep(0.0, 0.09, abs(up_fl - 0.14))) * (1.0 - glass_cov) * keep_rev;
                n_wall_hl = normalize(n_wall_hl + vec3<f32>(0.0, 0.32 * (ledge_t + sill_catch) - 0.34 * cornice_u, 0.0));
                s.ao = clamp(s.ao * (1.0 - 0.24 * cornice_u), 0.18, 1.0);
            }
            s.albedo = wall_albedo;
            s.rough = wall_rough;
            s.metal = wall_metal;
            s.n = n_wall_hl;
            // Distinct pane-by-pane fabrication bow/tilt faceting (`±1.4 deg`) + smooth structural
            // curtain-wall wave + parallax interior floor slab shadow & blind stratification.
            let bay_c = floor(vec2<f32>(along_m / grid.x, in.world.y / grid.y - grid.z));
            let near_p = 1.0 - smoothstep(0.35, 1.25, max(fx_m / grid.x, fy_m / grid.y));
            let pj_x = (hl_h3(gf, i32(bay_c.y) * 977 + i32(bay_c.x), 72u) - 0.5) * 0.044 * near_p;
            let pj_y = (hl_h3(gf, i32(bay_c.y) * 977 + i32(bay_c.x), 73u) - 0.5) * 0.036 * near_p;
            let wp_h = vec2<f32>(along_m * 0.085 + f32(in.group & 255u) * 0.71, in.world.y * 0.095);
            let mjx = (st_noise(wp_h) - 0.5) * 0.022 + pj_x;
            let mjy = (st_noise(wp_h + vec2<f32>(11.3, 4.7)) - 0.5) * 0.018 + pj_y + 0.044 * cw_t;
            let gn_hl = normalize(n_pane + vec3<f32>(t_ax.x * mjx, mjy, t_ax.y * mjx));
            let batch_h = (st_noise(wp_h * 1.4 + vec2<f32>(3.7, 8.9)) - 0.5) * 0.16;
            let vn_hl = max(dot(v.xz, normalize(n.xz + vec2<f32>(1e-5, 0.0))), 0.14);
            let vy_hl = fract(in.world.y / grid.y - grid.z + clamp(v.y / vn_hl * 0.20, -0.32, 0.32));
            let slab_hl = 1.0 - 0.70 * (1.0 - smoothstep(0.06, 0.16, vy_hl) * smoothstep(0.94, 0.82, vy_hl)) * near_p;
            aaa_bldg_on = true;
            aaa_glass_cov = glass_cov;
            aaa_glass_rough = mix(0.018, 0.044, smoothstep(0.45, 1.8, fy_m / grid.y));
            aaa_glass_f0 = clamp(glass_f0 * (1.0 + batch_h), vec3<f32>(0.09), vec3<f32>(0.46));
            aaa_glass_n = gn_hl;
            aaa_glass_room = room_albedo * select(0.75, 0.28, is_tower && glass_cov > 0.36) * slab_hl;
        } else {
            aaa_bldg_on = true;
            aaa_glass_cov = 0.0;
            if (FL_GLASS) {
                rm_emit = rm_emit + hl_glass_sky(in, n_pane, v, glass_cov);
            }
        }
    }
// sf:end
    // Object 0 is a building: the painted-facade environment class.
    return look_light(s, m, v, in.world, n, 0u);
}

@fragment
fn hlod_fs(in: HlOut) -> @location(0) vec4<f32> {
    hl_keep(in);
    return hl_shade(in);
}

@fragment
fn hlod_fs_ids(in: HlOut) -> IdsOut {
    hl_keep(in);
    var out: IdsOut;
    out.color = hl_shade(in);
    // hlod.rs FAR_ID_BASE | the building: not an object-table index.
    out.ids = vec2<u32>(0xc0000000u | in.group, 0u);
    return out;
}
