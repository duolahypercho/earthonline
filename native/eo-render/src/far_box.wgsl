// the far box's facade (appended to the city shader by look.rs; the
// mesh and its words are far_box.rs). A building past its own levels is its
// footprint box; this draws on it, with no texture, the archetype's facade
// as the kit plans it: storeys and window bays, the glass (the room seen
// through by day, the sky reflected, lit rooms at night), frames and
// mullions, spandrels and piers, a shop front, the cornice or a tower's
// crown, a row's canted bay windows, and the roof's measured colour with its
// plant. Every periodic term is box filtered over the pixel's footprint on
// the wall (a pulse train integrated over the pixel), so a window under a
// pixel gives its share of the colour and nothing shimmers; per-window
// variation fades to its mean as the window shrinks under a pixel.

// The box's vertex alpha byte: FB_ALPHA_BASE + the class (far_box.rs ALPHA_BASE).
const FB_ALPHA_BASE: f32 = 128.0;
const FB_PUNCHED: u32 = 0u;
const FB_ROW_BAYS: u32 = 1u;
const FB_RIBBON: u32 = 2u;
const FB_CURTAIN: u32 = 3u;
const FB_MASONRY: u32 = 4u;
const FB_INDUSTRIAL: u32 = 5u;
const FB_SHED: u32 = 6u;
const FB_BLIND: u32 = 7u;
// The roof plant's boxes (far_box.rs ALPHA_PLANT): drawn in their own colour.
const FB_PLANT: u32 = 8u;
// A tower's facade system (the wall's normal byte 0; eo_kit TowerSystem).
const FB_SYS_CURTAIN_FLUSH: u32 = 1u;
const FB_SYS_CURTAIN_MULLION: u32 = 2u;
const FB_SYS_PRECAST_FIN: u32 = 5u;
// Fins of a precast-fin tower stand about 0.3-0.6 m proud (typology.rs).
const FB_FIN_M: f32 = 0.45;
// Coping and the parapet's inner face round a roof's edge: a coping of
// about 0.3 m (a 12 in parapet cap) and the inner face's shade below it.
const FB_COPING_M: f32 = 0.3;
const FB_PARAPET_FACE_M: f32 = 0.25;
// Glass kinds (eo_kit::far_facade::GlassKind).
const FB_DOMESTIC: u32 = 0u;
const FB_OFFICE: u32 = 1u;
const FB_COATED: u32 = 2u;
const FB_TINTED: u32 = 3u;

// The room seen through a pane by day, linear albedo (hlod_bake.rs GLASS_ROOM,
// hlod_draw.wgsl HL_ROOM): the far layer's own value, so both far paths agree.
const FB_ROOM: vec3<f32> = vec3<f32>(0.022, 0.026, 0.030);
// Light blinds and net curtains behind a domestic pane: a white textile
// (reflectance about 0.6) behind clear double glazing (visible transmittance
// about 0.8, through twice): 0.6 x 0.8 x 0.8, about 0.38; drawn half open on
// average. Glass data (manufacturer centre-of-glass, NFRC method): 6 mm clear
// float transmits 88 % of visible light and reflects 8 % outside; 6 mm grey
// tinted 44 % and 6 %; a clear double-glazed unit about 78 %.
const FB_BLIND_ALBEDO: vec3<f32> = vec3<f32>(0.36, 0.35, 0.33);
// The share of domestic windows with blinds or curtains drawn, and of offices.
const FB_BLIND_SHARE_HOME: f32 = 0.45;
const FB_BLIND_SHARE_OFFICE: f32 = 0.2;
// Mullion and frame widths, metres: an aluminium storefront or window frame
// sight line (about 2 in, 50 mm per member) and a painted timber sash's
// surround with its casing (the kit's `surround` 0.12-0.16 m).
const FB_MULLION_M: f32 = 0.06;
const FB_SURROUND_M: f32 = 0.13;
// A shop front: stall riser and the sign band over the glass (the kit's
// storefrontSpec stallRiser 0.42-0.62 m, headClear 0.6-1.1 m), piers of 0.3 m.
const FB_RISER_M: f32 = 0.5;
const FB_SIGN_M: f32 = 0.9;
const FB_SHOP_PIER_M: f32 = 0.3;
// A curtain wall's spandrel: an opaque panel behind the outer pane (a
// painted or fritted back, insulation behind): lighter than the vision glass
// with the dark room behind it, and rougher, so the floor lines read far off.
const FB_SPANDREL: vec3<f32> = vec3<f32>(0.085, 0.092, 0.10);
const FB_SPANDREL_ROUGH: f32 = 0.35;
// A tower's crown screen (louvres in front of the plant), darkened by the plant behind.
const FB_CROWN_SHADE: f32 = 0.55;

// The frame and trim finishes (eo_kit::far_facade::TRIM_FINISHES), linear
// albedo of their kit-material-specs.json colours: painted-white #e6e3dc,
// anodized-aluminium #3a3d40, painted-steel-mid #6e7478, cast-stone #c9bfae,
// painted-steel #2b3236, signal-red #a32820, (the wall's own tone), painted-steel-light #9aa0a3.
fn fb_trim(i: u32, wall: vec3<f32>) -> vec3<f32> {
    var c = wall * 0.72;
    switch i {
        case 0u: { c = vec3<f32>(0.791, 0.768, 0.716); }
        case 1u: { c = vec3<f32>(0.042, 0.047, 0.051); }
        case 2u: { c = vec3<f32>(0.155, 0.175, 0.188); }
        case 3u: { c = vec3<f32>(0.584, 0.521, 0.423); }
        case 4u: { c = vec3<f32>(0.024, 0.031, 0.036); }
        case 5u: { c = vec3<f32>(0.366, 0.021, 0.014); }
        case 7u: { c = vec3<f32>(0.323, 0.352, 0.366); }
        default: {}
    }
    return c;
}

fn fb_u(x: u32) -> u32 {
    let s = x * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// A hash 0..1 of the building, a cell and a salt.
fn fb_h(b: u32, cell: vec2<i32>, salt: u32) -> f32 {
    return f32(fb_u(b ^ fb_u(bitcast<u32>(cell.x) ^ fb_u(bitcast<u32>(cell.y) * 2654435761u ^ salt))) >> 8u) / 16777216.0;
}

// The integral of a pulse train (pulses of `duty` starting at every whole number).
fn fb_pulse_int(t: f32, duty: f32) -> f32 {
    let f = floor(t);
    return f * duty + min(t - f, duty);
}

// The share of [x - w / 2, x + w / 2] that pulses of `duty` (from each whole number) cover.
fn fb_pulse(x: f32, duty: f32, w: f32) -> f32 {
    if (duty <= 0.0) {
        return 0.0;
    }
    if (duty >= 1.0) {
        return 1.0;
    }
    if (w < 1e-4) {
        return select(0.0, 1.0, fract(x) < duty);
    }
    return (fb_pulse_int(x + 0.5 * w, duty) - fb_pulse_int(x - 0.5 * w, duty)) / w;
}

// The share of [x - w / 2, x + w / 2] inside [a, b] (one span, not periodic).
fn fb_span(x: f32, a: f32, b: f32, w: f32) -> f32 {
    let ww = max(w, 1e-4);
    return clamp((min(x + 0.5 * ww, b) - max(x - 0.5 * ww, a)) / ww, 0.0, 1.0);
}

// A facade box's vertex (alpha byte FB_ALPHA_BASE + class) of a building (kind 0).
fn fb_on(in: VOut) -> bool {
    let a = in.aux.x;
    return (objects[in.object].kind & 255u) == 0u && a > FB_ALPHA_BASE - 0.5 && a < FB_ALPHA_BASE + 8.5;
}

// The roof cap: the measured colour (the roof word's rgb565) with its plant
// and hatches as a zero-mean pattern of 4 m cells (the aerial measurement
// already holds them on average), gone to the mean once a cell is under a pixel.
fn fb_roof(s: ptr<function, Surface>, ob: Object, object: u32, world: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, cls: u32, extra: vec2<f32>) {
    let wall = (*s).albedo;
    let w0 = ob.far_first_index;
    let roof = srgb_decode(vec3<f32>(f32(w0 & 31u) / 31.0, f32((w0 >> 5u) & 63u) / 63.0, f32((w0 >> 11u) & 31u) / 31.0));
    let cell_m = 4.0;
    let p = world.xz / cell_m;
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let fw = max(abs(dpx.x) + abs(dpy.x), abs(dpx.z) + abs(dpy.z)) / cell_m;
    let big = cls == FB_RIBBON || cls == FB_CURTAIN || cls == FB_MASONRY || cls == FB_INDUSTRIAL || cls == FB_SHED;
    let share = select(0.06, 0.22, big);
    let h = fb_h(object, cell, 41u);
    // A unit fills the middle of its cell: a dark one (plant, ducts, their shade) or a light one (a white unit, a hatch).
    let inside = fb_span(f.x, 0.2, 0.8, fw) * fb_span(f.y, 0.25, 0.75, fw);
    let k = select(1.32, 0.6, fb_h(object, cell, 43u) < 0.62);
    let unit = select(0.0, inside, h < share);
    // The mean the measurement holds: share x 0.36 of each cell is a unit of mean k.
    let mean_k = 1.0 + share * 0.36 * ((0.62 * 0.6 + 0.38 * 1.32) - 1.0);
    let fade = 1.0 - smoothstep(0.35, 1.0, fw);
    let factor = mix(1.0, (1.0 + unit * (k - 1.0)) / mean_k, fade);
    (*s).albedo = roof * factor;
    (*s).rough = 0.9;
    (*s).metal = 0.0;
    // The coping band (far_box.rs: byte 0 its width in 0.1 m, byte 1 the
    // edge weight, 31 on the roof's edge): the coping in the wall's finish
    // with its flashing, the parapet's inner face in shade, the roof's foot
    // against the parapet a little darker; box filtered over the pixel.
    let band_w = extra.x * 12.7;
    if (band_w > 0.05) {
        let d = (1.0 - clamp(extra.y * 127.0 / 31.0, 0.0, 1.0)) * band_w;
        let fm = max(length(dpx.xz) + length(dpy.xz), 1e-3);
        let coping = fb_span(d, -1.0, FB_COPING_M, fm);
        let face = fb_span(d, FB_COPING_M, FB_COPING_M + FB_PARAPET_FACE_M, fm);
        let foot = fb_span(d, FB_COPING_M + FB_PARAPET_FACE_M, band_w + 0.05, fm) * (1.0 - smoothstep(FB_COPING_M + FB_PARAPET_FACE_M, band_w + 0.05, d));
        let coping_c = mix(wall, vec3<f32>(0.52, 0.52, 0.5), 0.35);
        let rest = max(1.0 - coping - face, 0.0);
        (*s).albedo = (*s).albedo * rest * (1.0 - 0.18 * foot) + coping_c * coping + wall * 0.5 * face;
        (*s).rough = mix(0.9, 0.6, coping);
    }
}

// What a pane reflects beyond the light rig's own 4 %: a pane's Fresnel
// reflection of the sky (Schlick from the glass's F0), the city below the
// horizon at a quarter of the horizon (hlod_draw.wgsl `hl_glass_sky`, `hl_reflect_env`).
fn fb_glass_sky(n: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, cov: f32) -> vec3<f32> {
    if (cov <= 0.0) {
        return vec3<f32>(0.0);
    }
    let nv = max(dot(n, v), 1e-3);
    let fres = pow(1.0 - nv, 5.0);
    let f = f0 + (vec3<f32>(1.0) - f0) * fres;
    let extra = max(f - vec3<f32>(0.04 + 0.96 * fres), vec3<f32>(0.0));
    let r = reflect(-v, n);
    // One sky read where only one side of the horizon shows (most pixels), two in the band between.
    let w = smoothstep(-0.02, 0.2, -r.y);
    var up = vec3<f32>(0.0);
    var horizon = vec3<f32>(0.0);
    if (w < 1.0) {
        up = lk_sky_sample(vec3<f32>(r.x, max(r.y, 0.0), r.z));
    }
    if (w > 0.0) {
        horizon = lk_sky_sample(normalize(vec3<f32>(r.x, 0.0, r.z) + vec3<f32>(1e-5, 0.0, 0.0)));
    }
    let sky = mix(up, horizon * 0.25, w);
    let env_i = look.ambient.w * look.env_class.y;
    return sky * env_i * extra * cov;
}

// Lit windows at night: a window lit by the room drive's occupancy (a tower's
// floors by the office share), warm or cool, its level varied; to the mean as
// the window shrinks under a pixel (sky_light.wgsl `sky_far_windows`'s drive and level).
fn fb_night(object: u32, cell: vec2<i32>, cov: f32, office: bool, a: f32) -> vec3<f32> {
    let dusk = look.rooms_env.x;
    if (!sky_on(skyu.flags.x, SKY_LIGHTS) || dusk <= 0.0 || cov <= 0.0 || frame.copy_step.z > 0.5) {
        return vec3<f32>(0.0);
    }
    let hb = fb_h(object, vec2<i32>(0, 0), 11u);
    var occ = clamp(look.rooms_env.y * (0.35 + 1.3 * hb), 0.0, 0.95);
    var lit = step(fb_h(object, cell, 8u), occ);
    if (office) {
// sf:AAA_BUILDINGS+
        if (SF_AAA_DETAIL) {
            // Irregular multi-scale tenant clusters + varied 2700K-5200K office temperatures (no full-width zebra bars).
            let p_floor = clamp(0.14 + look.rooms_work.x / 0.7 * 1.45 * hb * hb + 0.10 * hb, 0.08, 0.52);
            let fl = step(fb_h(object, vec2<i32>(0, cell.y), 1u), p_floor);
            let run_w = i32(3.0 + floor(fb_h(object, vec2<i32>(2, cell.y), 2u) * 5.0));
            let run_id = i32(floor(f32(cell.x) / f32(run_w) + fb_h(object, vec2<i32>(3, cell.y), 3u) * 4.0));
            let r_lit = fl * step(fb_h(object, vec2<i32>(run_id, cell.y), 4u), mix(0.36, 0.65, hb))
                + (1.0 - fl) * step(fb_h(object, vec2<i32>(run_id, cell.y), 4u), 0.06);
            let wl = step(fb_h(object, cell, 5u), 0.84);
            let v_win = r_lit * wl + (1.0 - r_lit) * step(fb_h(object, cell, 5u), 0.025);
            let v_run = r_lit * 0.84 + (1.0 - r_lit) * 0.025;
            let v_fl = p_floor * 0.42 + 0.03;
            let a_run = smoothstep(0.45, 1.15, a * 0.55);
            let k = mix(mix(v_win, v_run, a), v_fl, a_run);
            let th = fb_h(object, vec2<i32>(run_id, cell.y), 9u);
            let bld_t = fb_h(object, vec2<i32>(0, 0), 71u);
            var col = vec3<f32>(0.97, 0.98, 1.02);
            if (bld_t < 0.45) {
                if (th < 0.35) { col = vec3<f32>(0.54, 0.84, 1.30); }
                else if (th < 0.62) { col = vec3<f32>(0.76, 0.94, 1.18); }
                else if (th < 0.80) { col = vec3<f32>(0.97, 0.98, 1.02); }
                else if (th < 0.94) { col = vec3<f32>(1.16, 0.74, 0.34); }
                else { col = vec3<f32>(0.38, 0.94, 1.14); }
            } else {
                if (th < 0.28) { col = vec3<f32>(0.74, 0.92, 1.20); }
                else if (th < 0.54) { col = vec3<f32>(0.97, 0.98, 1.02); }
                else if (th < 0.80) { col = vec3<f32>(1.16, 0.74, 0.34); }
                else { col = vec3<f32>(1.30, 0.56, 0.16); }
            }
            let mean_col = select(vec3<f32>(1.00, 0.84, 0.66), vec3<f32>(0.78, 0.92, 1.12), bld_t < 0.45);
            let level = mix(0.32 + 0.78 * fb_h(object, vec2<i32>(run_id, cell.y), 10u), 0.62, a_run) * 0.72;
            return mix(col, mean_col, a_run) * (cov * k * level * skyu.bloom_w2.z * 25.0 / LK_PI * dusk);
        }
// sf:end
        // whole floors lit by the office share, a few late windows on a dark floor.
        let p_floor = clamp(look.rooms_work.x / 0.7 * 2.6 * hb * hb, 0.0, 0.92);
        let fl = step(fb_h(object, vec2<i32>(0, cell.y), 1u), p_floor);
        let wl = step(fb_h(object, cell, 5u), 0.88);
        lit = fl * wl + (1.0 - fl) * step(fb_h(object, cell, 5u), 0.03);
        occ = p_floor * 0.88 + (1.0 - p_floor) * 0.03;
    }
    let cool = step(1.0 - look.rooms_env.w, fb_h(object, cell, 9u));
    var col = select(vec3<f32>(1.0, 0.62, 0.34), vec3<f32>(0.86, 0.93, 1.0), cool > 0.5);
    var mean_col = mix(vec3<f32>(1.0, 0.62, 0.34), vec3<f32>(0.86, 0.93, 1.0), look.rooms_env.w);
// sf:AAA_BUILDINGS+
    if (SF_AAA_DETAIL) {
        let rh = fb_h(object, cell, 9u);
        if (rh < 0.42) { col = vec3<f32>(1.28, 0.56, 0.16); }
        else if (rh < 0.68) { col = vec3<f32>(1.12, 0.76, 0.40); }
        else if (rh < 0.82) { col = vec3<f32>(0.98, 0.94, 0.88); }
        else { col = vec3<f32>(0.54, 0.82, 1.28); }
        mean_col = vec3<f32>(1.08, 0.72, 0.44);
    }
// sf:end
    let k = mix(lit, occ, a);
    let level = mix(0.6 + 0.8 * fb_h(object, cell, 10u), 1.0, a);
    return mix(col, mean_col, a) * (cov * k * level * skyu.bloom_w2.z * 25.0 / LK_PI * dusk);
}

// Draw the facade on a far box's wall or roof. `n` faces the camera, `du`
// the bay coordinate's screen derivatives. Returns what the glass emits
// (the sky it reflects past the rig's 4 %, the lit rooms at night).
fn fb_apply(s: ptr<function, Surface>, in: VOut, n: vec3<f32>, v: vec3<f32>, dpx: vec3<f32>, dpy: vec3<f32>, du: vec2<f32>) -> vec3<f32> {
    let ob = objects[in.object];
    let cls = u32(round(in.aux.x - FB_ALPHA_BASE));
    if (cls == FB_PLANT) {
        // Roof plant: its own colour (the vertex's), a painted metal or its walls' finish.
        (*s).rough = 0.55;
        (*s).metal = 0.0;
        return vec3<f32>(0.0);
    }
    // The normal word's first two bytes (far_box.rs): the system and lot on a wall, the coping on the cap.
    let extra = max(in.normal.xy, vec2<f32>(0.0));
    if (n.y > 0.7) {
        fb_roof(s, ob, in.object, in.world, dpx, dpy, cls, extra);
// sf:AAA_BUILDINGS+
        (*s).rough = max((*s).rough, 0.80);
        aaa_bldg_top_y = ob.max.y;
        aaa_bldg_on = SF_AAA_SKYLINE_FIELD;
        aaa_glass_cov = 0.0;
// sf:end
        return vec3<f32>(0.0);
    }
    if (abs(n.y) > 0.3) {
        return vec3<f32>(0.0);
    }
    let w0 = ob.far_first_index;
    let w1 = ob.far_index_count;
    let wall = (*s).albedo;
    let wall_rough = 0.88;
    let floor_y = ob.min.y + f32((w0 >> 16u) & 255u) * 0.1;
    let ground = 2.0 + f32((w0 >> 24u) & 63u) * 0.1;
    let glass_kind = w0 >> 30u;
    let storey = 2.0 + f32(w1 & 255u) * 0.02;
    let top_band = f32((w1 >> 8u) & 63u) * 0.1;
    let ww = (f32((w1 >> 14u) & 15u) + 0.5) / 16.0;
    let wh = (f32((w1 >> 18u) & 15u) + 0.5) / 16.0;
    let sill = f32((w1 >> 22u) & 15u) / 16.0;
    let lights = f32((w1 >> 26u) & 3u) + 1.0;
    let trim = fb_trim((w1 >> 28u) & 7u, wall);
    let shop = (w1 >> 31u) != 0u;
    let office = glass_kind != FB_DOMESTIC;
    let sys = u32(round(extra.x * 127.0));
    let lot_code = u32(round(extra.y * 127.0));

    let t = normalize(vec3<f32>(-n.z, 0.0, n.x) + vec3<f32>(1e-6, 0.0, 0.0));
    // Pixel footprint: bays along the wall, metres along it and up it.
    let fu = abs(du.x) + abs(du.y);
    let along_px = abs(dot(dpx, t)) + abs(dot(dpy, t));
    let fy = abs(dpx.y) + abs(dpy.y);
    let blind = fu < 1e-5 || cls == FB_BLIND;
    let bay_m = select(along_px / max(fu, 1e-6), 3.0, blind);
    let u = in.aux.y * 127.0;
    let y = in.world.y - floor_y;
    let wall_top = ob.max.y - floor_y - top_band;

    var cov = 0.0; // glass
    var frame_cov = 0.0; // frames, surrounds, mullions
    var dark_cov = 0.0; // spandrel panels, sign bands, crown louvres
    var dark = FB_SPANDREL;
    var k = 0.0; // storey index (0 the ground storey)
    var vert_fy = fy / storey;
    var nrm = n;
    var occ = 1.0;
    if (y < 0.0) {
        // The foundation below the floor.
        (*s).albedo = wall * 0.78;
        (*s).rough = wall_rough;
        return vec3<f32>(0.0);
    }
    if (y >= wall_top) {
        // The band over the top storey: a cornice (residential and masonry) or a tower's crown screen.
        let tower = cls == FB_CURTAIN || cls == FB_RIBBON || cls == FB_MASONRY;
        if (tower && top_band > 1.5) {
            let louvre = fb_pulse((y - wall_top) / 0.25, 0.5, fy / 0.25);
            (*s).albedo = mix(wall, wall * FB_CROWN_SHADE, 0.35 + 0.4 * louvre);
// sf:AAA_BUILDINGS+
            if (SF_AAA_DETAIL) {
                (*s).rough = mix(0.62, 0.44, louvre);
                (*s).metal = 0.28 * louvre;
            }
// sf:end
        } else {
            let fascia = fb_span(y, wall_top + 0.15, wall_top + top_band, fy);
            (*s).albedo = mix(wall, mix(trim, wall, 0.4), fascia);
            (*s).rough = wall_rough;
        }
// sf:AAA_BUILDINGS+
        aaa_bldg_on = true;
        aaa_glass_cov = 0.0;
        aaa_bldg_top_y = ob.max.y;
// sf:end
        if (!(tower && top_band > 1.5)) {
            (*s).rough = wall_rough;
        }
        return vec3<f32>(0.0);
    }
    let bay_x = u; // bays from the wall's corner
    let b_cell = i32(floor(bay_x));
    if (!blind) {
        if (y < ground) {
            k = 0.0;
            if (shop) {
                // A shop front: glass from the stall riser to the sign band, piers between.
                let pier = clamp(FB_SHOP_PIER_M / bay_m, 0.04, 0.4);
                let cu = fb_pulse(bay_x + 0.5 * pier, 1.0 - pier, fu);
                let top_glass = max(ground - FB_SIGN_M, FB_RISER_M + 1.0);
                let cv = fb_span(y, FB_RISER_M, top_glass, fy);
                cov = cu * cv;
                frame_cov = cu * cv * min(0.5, 2.0 * FB_MULLION_M / max(bay_m, 0.5));
                cov = cov - frame_cov;
                // The sign band over the glass: a fascia darker than the wall (a sign, an awning's shade).
                let sign = fb_span(y, top_glass, ground - 0.15, fy);
                dark_cov = sign * 0.6;
                dark = wall * 0.35;
            } else {
                let vs = y / ground;
                let cu = fb_pulse(bay_x - 0.5 * (1.0 - ww), ww, fu);
                let cv = fb_pulse(vs - max(sill, 0.28), min(wh, 0.6), fy / ground);
                cov = cu * cv;
            }
        } else {
            let vs = (y - ground) / storey;
            k = 1.0 + floor(vs);
            if (cls == FB_CURTAIN) {
                // Vision glass to the spandrel band, a mullion at every pane.
                let cv = fb_pulse(vs - sill, wh, vert_fy);
                let mull = 1.0 - ww;
                let cm = fb_pulse(bay_x + 0.5 * mull, mull, fu);
                cov = cv * (1.0 - cm);
                frame_cov = cm;
                dark_cov = (1.0 - cv) * (1.0 - cm);
            } else if (cls == FB_RIBBON) {
                // A glass band per floor; piers (the wall) at the bay; mullions in the band.
                let cv = fb_pulse(vs - sill, wh, vert_fy);
                var pier = 1.0 - ww;
                if (sys == FB_SYS_PRECAST_FIN) {
                    // A fin's side hides the glass beside it as the wall turns from the eye.
                    let side = FB_FIN_M * abs(dot(v, t)) / max(abs(dot(v, n)), 0.25) / max(bay_m, 0.5);
                    pier = min(pier + min(side, 0.45), 0.85);
                }
                let cp = fb_pulse(bay_x + 0.5 * pier, pier, fu);
                let lm = min(0.4, (lights - 1.0) * FB_MULLION_M / max(bay_m * ww, 0.3));
                cov = cv * (1.0 - cp) * (1.0 - lm);
                frame_cov = cv * (1.0 - cp) * lm;
            } else if (cls == FB_SHED) {
                // A clerestory band in the top storey only, under the eave.
                let band = wh * storey;
                let cv = fb_span(y, wall_top - 0.35 - band, wall_top - 0.35, fy);
                let cm = fb_pulse(bay_x, 0.08, fu);
                cov = cv * (1.0 - cm);
                frame_cov = cv * cm;
            } else {
                // Punched windows (with a surround on a row, a masonry grid's two lights, a warehouse's sash).
                let cu = fb_pulse(bay_x - 0.5 * (1.0 - ww), ww, fu);
                let cv = fb_pulse(vs - sill, wh, vert_fy);
                let win = cu * cv;
                var sur = 0.0;
                if (cls == FB_ROW_BAYS || cls == FB_PUNCHED) {
                    let ws = min(ww + 2.0 * FB_SURROUND_M / max(bay_m, 0.5), 0.98);
                    let hs = min(wh + 2.0 * FB_SURROUND_M / storey, 0.98);
                    let su = fb_pulse(bay_x - 0.5 * (1.0 - ws), ws, fu);
                    let sv = fb_pulse(vs - sill + FB_SURROUND_M / storey, hs, vert_fy);
                    sur = max(su * sv - win, 0.0);
                }
                // Mullions and muntins: their share of the window, drawn as lines once a light is a few pixels.
                var lm = (lights - 1.0) * FB_MULLION_M / max(bay_m * ww, 0.3);
                if (cls == FB_INDUSTRIAL) {
                    lm = lm + 0.18; // steel sash: a muntin grid of small lights
                }
                lm = min(lm, 0.5);
                let lines = fb_pulse((bay_x - 0.5 * (1.0 - ww)) / ww * lights + 0.5 * lm, lm, fu / ww * lights);
                let mull = mix(lines, lm, smoothstep(0.3, 0.8, fu / ww * lights));
                cov = win * (1.0 - mull);
                frame_cov = win * mull + sur * select(0.45, 0.9, cls == FB_ROW_BAYS);
                if (cls == FB_MASONRY || cls == FB_INDUSTRIAL) {
                    // Deep reveals: the jambs and head in shade round the glass.
                    occ = 1.0 - 0.25 * win;
                }
            }
        }
    }
    // A row's canted bay window: every second bay from the first storey up,
    // its two cheeks turned 45 degrees off the wall (the kit's oriel), faded
    // to the flat wall once a cheek is under a pixel.
    if (cls == FB_ROW_BAYS && !blind && y >= ground && y < wall_top - 0.1) {
        let lot = bay_x * 0.5;
        let fl = fract(lot);
        if (i32(floor(lot)) >= 0 && fl < 0.5) {
            let x = fl * 2.0; // 0..1 across the oriel's bay
            let side = select(0.0, -1.0, x < 0.28) + select(0.0, 1.0, x > 0.72);
            let fade = 1.0 - smoothstep(0.15, 0.45, fu * 2.0);
            if (side != 0.0) {
                nrm = normalize(mix(n, normalize(n * 0.7071 + t * (side * 0.7071)), fade));
            }
        }
    }
    let wall_share = max(1.0 - cov - frame_cov - dark_cov, 0.0);
    // The glass: the room by day, blinds and curtains in some panes (the per-window choice gone to its share under a pixel).
    let cell = vec2<i32>(b_cell, i32(k));
    let a = smoothstep(0.3, 0.9, max(fu, vert_fy));
    let share = select(FB_BLIND_SHARE_HOME, FB_BLIND_SHARE_OFFICE, office);
    // A home's blinds and curtains go window by window; an office's by
    // tenant, a run of 3-8 bays along a floor at one level.
    var bcell = cell;
    if (office) {
        // A tower's tenants (a facade system): a floor's blinds run over 6-20 bays at one level.
        let run_n = select(5.0, 14.0, sys != 0u);
        let run = select(3.0, 6.0, sys != 0u) + floor(run_n * fb_h(in.object, vec2<i32>(0, cell.y), 21u));
        bcell = vec2<i32>(i32(floor(bay_x / run + fb_h(in.object, vec2<i32>(1, cell.y), 22u))), cell.y);
    }
    let hw = fb_h(in.object, bcell, 3u);
    var level = 0.35 + 0.65 * fb_h(in.object, bcell, 4u);
    if (sys != 0u) {
        // ... at one of a few levels a building's blinds share (a floor at a time, not cell by cell).
        level = 0.3 + 0.2 * floor(3.0 * fb_h(in.object, vec2<i32>(2, cell.y / 3), 4u) + 0.5 * fb_h(in.object, bcell, 4u));
    }
    let drawn = mix(select(0.0, level, hw < share), share * 0.675, a);
    var body = mix(FB_ROOM, FB_BLIND_ALBEDO, drawn);
    // Clear float reflects 8 % outside (the data above): the pane's F0.
    var f0 = vec3<f32>(0.08);
    if (glass_kind == FB_COATED) {
        let h = fb_h(in.object, vec2<i32>(1, 2), 7u);
        let h2 = fb_h(in.object, vec2<i32>(3, 4), 7u);
        let tint = mix(vec3<f32>(0.80, 0.92, 1.0), mix(vec3<f32>(0.82, 1.0, 0.92), vec3<f32>(1.0, 0.9, 0.78), step(0.5, h2)), step(0.6, h));
        f0 = (0.08 + 0.14 * h) * tint;
        // Through a coated pane twice (visible transmittance about 0.65, against about 0.78 for clear double glazing: (0.65 / 0.78)^2, about 0.7).
        body = body * 0.7;
    } else if (glass_kind == FB_TINTED) {
        f0 = vec3<f32>(0.06, 0.063, 0.066); // the grey pane's 6 % outside reflectance
        // Through a grey tinted pane twice (visible transmittance 0.44, against about 0.78 for clear double glazing: (0.44 / 0.78)^2, about 0.32).
        body = body * 0.32;
    }
    var wall_c = wall;
    var glass_n = n;
    if (sys == FB_SYS_CURTAIN_FLUSH || sys == FB_SYS_CURTAIN_MULLION) {
        // A curtain wall's panes are not one mirror: each unit sits a little
        // out of plane (a glazed unit's bow and the frame's tolerance, about
        // a degree), its coating a shade off its neighbours'; to the mean
        // once a pane is under a pixel.
        let ph = vec2<f32>(fb_h(in.object, cell, 31u), fb_h(in.object, cell, 32u)) - 0.5;
        let keep = 1.0 - a;
        glass_n = normalize(n + t * (ph.x * 0.035 * keep) + vec3<f32>(0.0, ph.y * 0.03 * keep, 0.0));
        body = body * (1.0 + (fb_h(in.object, cell, 33u) - 0.5) * 0.25 * keep);
    }
    if (lot_code > 0u && !blind) {
        // A footprint over several lots: each lot its own paint (a shade
        // lighter or darker, warmer or cooler) and a downpipe at the lot line.
        let lot_m = 4.0 + f32(lot_code - 1u) * 0.5;
        let x_m = bay_x * bay_m / lot_m;
        let li = i32(floor(x_m));
        let hl = fb_h(in.object, vec2<i32>(li, 77), 51u);
        let hc = fb_h(in.object, vec2<i32>(li, 78), 52u);
        let tone = mix(vec3<f32>(1.0), select(vec3<f32>(1.03, 1.0, 0.95), vec3<f32>(0.96, 0.99, 1.04), hc < 0.5), 0.8);
        let paint = wall * tone * (0.86 + 0.22 * hl);
        let fl = fu * bay_m / lot_m;
        let lot_fade = 1.0 - smoothstep(0.35, 1.0, fl);
        wall_c = mix(wall, paint, lot_fade);
        let pipe = fb_pulse(x_m + 0.012, 0.024, fl);
        wall_c = mix(wall_c, wall_c * 0.6, pipe);
    }
    (*s).albedo = wall_c * wall_share + trim * frame_cov + dark * dark_cov + body * cov;
    (*s).albedo = (*s).albedo * mix(1.0, occ, 0.6);
    var rough = mix(wall_rough, 0.12, cov);
    if (cls == FB_CURTAIN) {
        rough = mix(rough, FB_SPANDREL_ROUGH, dark_cov);
    }
    (*s).rough = rough;
    (*s).metal = 0.0;
    (*s).n = nrm;
    // The street's canyon: the lowest storeys see less sky.
    (*s).ao = (*s).ao * mix(0.78, 1.0, smoothstep(0.0, 12.0, y));
    var emit = fb_night(in.object, cell, cov, office && cls != FB_ROW_BAYS, a);
// sf:AAA_BUILDINGS-
    emit = emit + fb_glass_sky(glass_n, v, f0, cov);
// sf:end
// sf:AAA_BUILDINGS+
    var aaa_cov = cov;
    var aaa_dark_cov = dark_cov;
    var aaa_wall_c = wall_c;
    var aaa_dark_c = dark;
    var aaa_body = body;
    var aaa_f0 = f0;
    var aaa_metal = 0.0;
    if (SF_AAA_DETAIL && !blind) {
        // Macro-frequency distant tower detail (surviving box-filtering out to 4 km):
        // (1) Louvered double-height mechanical/refuge floors every 14-19 storeys on tall towers.
        var mech = 0.0;
        if (wall_top > 42.0 && (cls == FB_CURTAIN || cls == FB_RIBBON || cls == FB_MASONRY)) {
            let m_per = mix(52.0, 72.0, fb_h(in.object, vec2<i32>(7, 13), 61u));
            let m_h = mix(5.0, 7.2, fb_h(in.object, vec2<i32>(8, 14), 62u));
            let m_mid = fb_pulse((y - ground) / m_per + 0.5, m_h / m_per, fy / m_per)
                * smoothstep(22.0, 38.0, y) * smoothstep(wall_top - 3.0, wall_top - 10.0, y);
            let m_top = fb_span(y, wall_top - m_h * 0.9, wall_top, fy) * step(55.0, wall_top);
            mech = clamp(max(m_mid, m_top), 0.0, 1.0);
            if (mech > 0.001) {
                let drop = aaa_cov * (0.88 * mech);
                aaa_cov = aaa_cov - drop;
                aaa_dark_cov = clamp(aaa_dark_cov + drop, 0.0, 1.0);
                let lv_stripe = fb_pulse(y / 0.65, 0.45, fy / 0.65);
                aaa_dark_c = mix(aaa_dark_c, wall * (0.28 + 0.14 * lv_stripe), mech);
                aaa_metal = 0.30 * mech;
            }
        }
        // (2) Podium stone tone break & base plinth grounding + parapet rain-wash drip staining.
        if (wall_top > 24.0) {
            let podium = 1.0 - smoothstep(10.0, 16.0, y);
            aaa_wall_c = aaa_wall_c * mix(vec3<f32>(1.0), vec3<f32>(0.88, 0.87, 0.85), podium * 0.65);
        }
        let base_soot = mix(0.82, 1.0, smoothstep(0.0, 5.0, y));
        let top_drip_n = 0.55 + 0.45 * fb_h(in.object, vec2<i32>(i32(floor(bay_x * 0.25)), 91), 63u);
        let top_drip = 1.0 - 0.14 * top_drip_n * smoothstep(wall_top - 9.0, wall_top, y);
        let broad_w = 0.93 + 0.14 * fb_h(in.object, vec2<i32>(i32(floor(bay_x * 0.18)), i32(floor(y / 28.0))), 64u);
        aaa_wall_c = aaa_wall_c * (base_soot * top_drip * broad_w);
        // (3) Coherent multi-storey tenant suite blind/luminance stratification (survives at 3 km!).
        let suite_h = mix(9.5, 14.5, fb_h(in.object, vec2<i32>(3, 5), 65u));
        let suite_i = i32(floor(y / suite_h));
        let suite_v = fb_h(in.object, vec2<i32>(suite_i, 19), 66u);
        aaa_body = aaa_body * mix(0.55, 1.55, suite_v);
        // Subtle panel-to-panel reflectance/hue shift (distance-faded cleanly).
        let pan_j = (fb_h(in.object, cell, 67u) - 0.5) * (1.0 - a);
        aaa_f0 = aaa_f0 * (1.0 + 0.20 * pan_j);
        // Subtle late-afternoon / golden-hour interior office glow on shadowed facades before dusk lamps take over.
        if (office && aaa_cov > 0.01 && frame.copy_step.z < 0.5 && skyu.atmo.y > 0.0 && skyu.atmo.y < 0.42 && look.rooms_env.x < 0.45) {
            let late_day = smoothstep(0.42, 0.08, skyu.atmo.y) * (1.0 - smoothstep(0.05, 0.42, look.rooms_env.x));
            let shaded = 1.0 - smoothstep(-0.05, 0.35, dot(n, look.key_dir.xyz));
            let suite_on = smoothstep(0.42, 0.78, suite_v) * (0.45 + 0.55 * fb_h(in.object, vec2<i32>(i32(floor(bay_x * 0.15)), suite_i), 68u));
            let tint_o = mix(vec3<f32>(1.0, 0.80, 0.56), vec3<f32>(0.88, 0.94, 1.0), step(0.55, fb_h(in.object, vec2<i32>(suite_i, 41), 69u)));
            emit = emit + tint_o * (aaa_cov * suite_on * late_day * shaded * 0.042);
        }
    }
    if (SF_AAA_BRDF) {
        let op_w = max(wall_share + frame_cov + aaa_dark_cov, 1e-4);
        (*s).albedo = (aaa_wall_c * wall_share + trim * frame_cov + aaa_dark_c * aaa_dark_cov) / op_w;
        (*s).albedo = (*s).albedo * mix(1.0, occ, 0.6);
        var op_rough = wall_rough;
        if (cls == FB_CURTAIN || aaa_metal > 0.0) {
            op_rough = mix(wall_rough, select(FB_SPANDREL_ROUGH, 0.46, aaa_metal > 0.0), clamp(aaa_dark_cov / op_w, 0.0, 1.0));
        }
        let is_curt = cls == FB_CURTAIN || glass_kind == FB_COATED;
        if (is_curt) {
            op_rough = min(op_rough, 0.24);
            aaa_metal = max(aaa_metal, 0.72);
            aaa_f0 = clamp(max(aaa_f0 * 1.75, vec3<f32>(0.26, 0.30, 0.34)), vec3<f32>(0.14), vec3<f32>(0.44));
        } else if (wall_top > 12.0) {
            let plinth_fb = 1.0 - smoothstep(4.0, 9.0, y);
            op_rough = mix(op_rough, 0.34, plinth_fb * 0.65);
        }
        (*s).rough = op_rough;
        (*s).metal = aaa_metal;
        if (SF_AAA_DETAIL && a < 0.95) {
            let cornice_u = smoothstep(2.0, 0.35, wall_top - y) * step(10.0, wall_top) * (1.0 - a);
            let sill_fb = (1.0 - smoothstep(0.0, 0.10, abs(fract((y - ground) / storey) - sill))) * (1.0 - aaa_cov) * (1.0 - a);
            (*s).n = normalize((*s).n + vec3<f32>(0.0, 0.30 * sill_fb - 0.32 * cornice_u, 0.0));
            (*s).ao = clamp((*s).ao * (1.0 - 0.22 * cornice_u), 0.18, 1.0);
        }
        let wp_fb = vec2<f32>(bay_x * 0.24 + f32(in.object & 255u) * 0.63, y * 0.095);
        let pj_fb = (vec2<f32>(fb_h(in.object, cell, 72u), fb_h(in.object, cell, 73u)) - 0.5) * 0.044 * (1.0 - a);
        let jx_fb = (st_noise(wp_fb) - 0.5) * 0.022 + pj_fb.x;
        let jy_fb = (st_noise(wp_fb + vec2<f32>(9.7, 5.3)) - 0.5) * 0.018 + pj_fb.y + select(0.014, 0.044, is_curt);
        let t_fb = normalize(vec2<f32>(-n.z, n.x) + vec2<f32>(1e-5, 0.0));
        let gn_fb = normalize(glass_n + vec3<f32>(t_fb.x * jx_fb, jy_fb, t_fb.y * jx_fb));
        let batch_fb = (st_noise(wp_fb * 1.35 + vec2<f32>(4.1, 7.9)) - 0.5) * 0.16;
        aaa_bldg_on = true;
        aaa_glass_cov = aaa_cov;
        aaa_glass_rough = mix(0.018, 0.044, a);
        aaa_glass_f0 = clamp(aaa_f0 * (1.0 + batch_fb), vec3<f32>(0.09), vec3<f32>(0.46));
        aaa_glass_n = gn_fb;
        aaa_glass_room = aaa_body * mix(1.0, occ, 0.45) * select(0.75, 0.28, is_curt);
        aaa_bldg_top_y = ob.max.y;
    } else {
        aaa_bldg_on = true;
        aaa_glass_cov = 0.0;
        aaa_bldg_top_y = ob.max.y;
        emit = emit + fb_glass_sky(glass_n, v, aaa_f0, aaa_cov);
    }
// sf:end
    return emit;
}
