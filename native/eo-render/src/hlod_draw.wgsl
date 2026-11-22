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
@group(2) @binding(2) var hl_pages: texture_2d_array<f32>;
@group(2) @binding(3) var hl_sampler: sampler;

// Vertex colour flags (hlod_bake.rs VF_*).
const HL_TEXTURED: u32 = 1u;
const HL_ROOF: u32 = 2u;
const HL_DISTRICT: u32 = 4u;
// a tower's wall (hlod_bake.rs VF_TOWER).
const HL_TOWER: u32 = 8u;
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
        p = pos + to_eye * min(HL_FADE_BIAS_M / max(length(to_eye), 1.0), 0.5);
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
    return out;
}

fn hl_keep(in: HlOut) {
    if (in.fade.y > 0.5) {
        let n = hlod_dither(floor(in.clip.xy));
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
    let col = select(vec3<f32>(1.0, 0.62, 0.34), vec3<f32>(0.86, 0.93, 1.0), cool > 0.5);
    let a = smoothstep(0.25, 0.6, max(fx / bay, fy / storey));
    let k = mix(lit, occ, a);
    let level = 0.6 + 0.8 * hl_h3(gf, fl * 977 + cell, 10u);
    return col * (glass * k * mix(level, 1.0, a) * base);
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

fn hl_shade(in: HlOut) -> vec4<f32> {
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
    if (in.layer >= 0) {
        albedo = tex.rgb;
        glass = tex.a;
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
                albedo = mix(in.color, HL_ROOM, cov);
                glass = cov;
            }
        }
    }
    if (wx_level() != 0u && !roofish) {
        albedo = albedo * mix(vec3<f32>(1.0), hl_age(in.group), 1.0 - glass); // weathering (`--grime`): each building its own tone
    }
    var s: Surface;
    s.albedo = albedo;
    s.rough = mix(select(0.88, 0.92, roofish), select(0.15, 0.45, district && !FL_GLASS), glass); // a district pane as glossy as a cell's
    s.metal = 0.0;
    s.ao = 1.0;
    // a tower's panes tilt panel by panel (by the glass share at this pixel).
    var n_pane = n;
    if (glass > 0.0 && !roofish) {
        n_pane = normalize(mix(n, hl_panel_normal(in, n, dpx, dpy), glass));
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
    if (FL_GLASS && !roofish) {
        rm_emit = rm_emit + hl_glass_sky(in, n_pane, v, glass); // the tilted pane
    }
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
