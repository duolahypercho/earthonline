// the room behind kit glazing, a port of night-interiors.js
// `nightRoomFn` (night-interiors-v5-shop-returns) for the desktop tier, by day
// (a diffuse albedo the pane takes, lit by what reaches the pane) and by night
// (radiance on the pane's emissive slot). Ported: the room cells (ribbons and
// curtain walls split into rooms and storeys), the lit decision (home, shop
// switch, office storey and bay), the lamp colours, the box (back, sides,
// floor, ceiling) and its hit point, the ceiling fixture and fill, the
// surfaces (home, shop and office palettes), the ceiling fixtures (disc,
// panel grid, troffers and T-bar), the back-wall features (art, bookshelf,
// kitchen, sofa, table-lamp spot), the office workstations, the TV room, the
// daylight terms, curtains, blinds and sheers, the pane cap, the Fresnel
// transmission and the day level and range fade. Not ported: the occupants
// (clay figures and office partitions) and the shop trade fittings (goods
// shelving and gondola, food, garments, studio, flowers counters): a shop
// draws its room shell; with `--shop-light` (stage N2) a kit shop's pane
// draws its shop's own room by trade instead (shop_rooms.wgsl,
// `rm_shop_pane`). The window frame comes from the vertex (rooms.rs).

// (`--facade-glass`, `fg_on()`; facade.rs) the rooms behind the glass and the
// pane. INFERRED from office practice and glazing physics:
// an office floor's lease depth from the glass to the core, metres;
const FG_OFFICE_DEPTH: vec2<f32> = vec2<f32>(6.5, 11.0);
// a suspended ceiling's white acoustic tile (reflectance ~0.8);
const FG_OFFICE_CEILING: vec3<f32> = vec3<f32>(0.80, 0.80, 0.78);
// the daylight thrown up onto an office ceiling near the glass (added to the room's 0.38);
const FG_OFFICE_CEILING_DAY: f32 = 0.22;
// by night an office's surfaces by what its downlights give them, against the work plane's: its
// walls and its ceiling (the room's inter-reflection only; ~0.3-0.5 for recessed troffers), and
// the gain that keeps a lit floor's mean as it was (INFERRED from office lighting practice);
const FG_OFFICE_WALL_K: f32 = 0.7;
const FG_OFFICE_CEILING_K: f32 = 0.4;
const FG_OFFICE_FILL: f32 = 1.3;
// an office's troffers by day, against the default's 1.1, and the share of offices with them on;
const FG_TROFFER_DAY: f32 = 0.7;
const FG_OFFICE_LIT: f32 = 0.85;
// the share of office windows with a blind or screen, and of those a dark solar screen (its weave's openness);
const FG_OFFICE_BLINDS: f32 = 0.62;
const FG_SOLAR_SCREENS: f32 = 0.35;
const FG_SCREEN_COL: vec3<f32> = vec3<f32>(0.09, 0.09, 0.10);
const FG_SCREEN_OPEN: f32 = 0.14;
// a pane's tilt out of the wall's plane about each axis, and an insulating
// unit's bow at its edges, radians (~0.46 and ~0.4 degrees at most).
const FG_TILT: f32 = 0.008;
const FG_BOW: f32 = 0.007;
// float glass's own roughness, before the dirt film (weathering.wgsl `wx_glass`).
const FG_ROUGH: f32 = 0.04;
// how much of the kit glass's colour (hue only) the room and its light take through the pane.
const FG_GLASS_TINT: f32 = 0.25;
// (`--facade-glass`) a lit room's lamp by its colour temperature, seen by an
// eye adapted to the street's 3000 K lamps and the lit rooms round it (white
// at FG_NIGHT_WHITE_K): homes mostly warm (2300-3400 K, incandescent and warm
// LED), some cool LED (3800-4500 K) and a few daylight tubes (5000-6200 K);
// offices 3400-4300 K. INFERRED shares: most lamps sold for homes are warm
// (2700-3000 K), so a street's lit rooms read amber with a cool one here and
// there (at 22 % and 8 % a street's rooms read grey and cold).
const FG_NIGHT_WHITE_K: f32 = 4300.0;
const FG_HOME_COOL_SHARE: f32 = 0.12;
const FG_HOME_COLD_SHARE: f32 = 0.03;
// glass at the packing's limit (a ribbon or curtain wall): its 1.5 m mullion
// module (rooms of two or three modules, by building), the storeys' vision
// glass from the floor to the ceiling and the opaque spandrel over it to the
// next floor, and their mullions' width (m).
const FG_MODULE_M: f32 = 1.5;
const FG_SILL_M: f32 = 0.85;
const FG_VISION_M: f32 = 2.8;
const FG_MULLION_W: f32 = 0.07;
const FG_PANEL: vec3<f32> = vec3<f32>(0.045, 0.05, 0.055);

fn fg_srgb(c: f32) -> f32 {
    return select(pow((c + 0.055) / 1.055, 2.4), c / 12.92, c <= 0.04045);
}

// a black body's colour (linear, its brightest channel 1; the same fit as rooms.rs `kelvin_to_linear`).
fn fg_kelvin(kelvin: f32) -> vec3<f32> {
    let t = kelvin / 100.0;
    var r = 1.0;
    var g = 0.0;
    if (t <= 66.0) {
        g = clamp((99.4708025861 * log(t) - 161.1195681661) / 255.0, 0.0, 1.0);
    } else {
        r = clamp(329.698727446 * pow(t - 60.0, -0.1332047592) / 255.0, 0.0, 1.0);
        g = clamp(288.1221695283 * pow(t - 60.0, -0.0755148492) / 255.0, 0.0, 1.0);
    }
    var b = 1.0;
    if (t <= 19.0) {
        b = 0.0;
    } else if (t < 66.0) {
        b = clamp((138.5177312231 * log(t - 10.0) - 305.0447927307) / 255.0, 0.0, 1.0);
    }
    let c = vec3<f32>(fg_srgb(r), fg_srgb(g), fg_srgb(b));
    return c / max(c.r, max(c.g, c.b));
}

// a lamp at `kelvin` as the night-adapted eye sees it (FG_NIGHT_WHITE_K white), its brightest channel 1.
fn fg_lamp(kelvin: f32) -> vec3<f32> {
    let c = fg_kelvin(kelvin) / fg_kelvin(FG_NIGHT_WHITE_K);
    return c / max(c.r, max(c.g, c.b));
}

fn rm_h(seed: f32, salt: f32) -> f32 {
    return fract(sin(seed * 91.3458 + salt * 17.1373) * 43758.5453);
}

fn rm_ss(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn rm_filtered(pattern: f32, mean: f32, footprint: f32, period: f32) -> f32 {
    return mix(pattern, mean, rm_ss(period * 0.25, period, footprint));
}

fn rm_filtered3(pattern: vec3<f32>, mean: vec3<f32>, footprint: f32, period: f32) -> vec3<f32> {
    return mix(pattern, mean, rm_ss(period * 0.25, period, footprint));
}

// (`--shop-light`, stage N1; shop_lights.rs) the shop's own light for the
// room `rm_room` draws next (`rm_kit_pane` sets it, then clears it): rgb its
// lit window's radiance, w 0 the rooms' own rule, 1 + fade its light found,
// -(1 + fade) none (a dark shop); `fade` 0..1 over the lights' last stretch
// of reach, where a room gives way to the rooms' own rule at its own distance.
var<private> rm_shop_override: vec4<f32>;

// (`--night-photometry`; `rm_kit_pane` sets it for an office pane, then clears
// it) the office floor's lighting zone the room lies in: x the zone's switch
// roll, y its lamp's roll (colour temperature), z its dimmer roll, w 1 set.
var<private> rm_office_zone: vec4<f32>;

/// (`--night-photometry`) An office zone's lamp: lit offices seen from the
/// street at night are not one neutral white but a tenant's mix: 4000 K
/// fluorescent with its green bias (most), 5000 K LED panels, 3300 K warm LED
/// retrofits, a few 5300 K and 2700 K (IES Lighting Handbook 10th ed., office
/// lighting practice: 3000-5000 K; fluorescent triphosphor lamps sit ~+0.003
/// Duv above the Planckian locus, a green cast).
fn rm_office_lamp(roll: f32) -> vec3<f32> {
    if (roll < 0.34) {
        return RM_COOL_A * vec3<f32>(0.95, 1.03, 0.93);
    } else if (roll < 0.58) {
        return RM_OFFICE_A;
    } else if (roll < 0.82) {
        return RM_WARM_B;
    } else if (roll < 0.92) {
        return RM_COOL_B;
    }
    return RM_WARM_A;
}

// ---- (`--office-floors`) an office storey seen through its glass ----
// Not a 3.6 m box per pane: one open-plan floor behind the whole facade, so
// the street reads floors and tenants, not a checkerboard of cells.
// Dimensions: a 3.6 m storey (the rooms' anchored cells), a 2.75 m finished
// ceiling (9 ft, the US office norm; the slab, plenum and spandrel above it),
// a curtain wall on a 1.5 m (5 ft) module with 65 mm (2.5 in) mullions, the
// core 9-14 m behind the glass (lease depths 30-45 ft), workstation panels
// 1.05-1.2 m high in rows 3.2 m apart with a 1.5 m aisle every 7.2 m. The
// light: a 2x4 troffer grid, continuous linear rows or 2x2 panels per
// tenant; a lit zone at the rooms' office level, a dark one at the egress
// lighting's ~3 % (IBC 1008: 1 fc on the path vs. 300-500 lux on desks),
// a lit zone's light reaching ~2.5 m into its dark neighbour. Lit floors and
// tenants: a storey is lit by the office share (its phase), its zones mostly
// on; a dark storey keeps a few late zones. Blinds on a third of the zones,
// each module's roller at its own drop. INFERRED from office practice
// (BOMA lease depths, IES RP-1 office lighting).
const OF_STOREY: f32 = 3.6;
const OF_CEILING: f32 = 2.75;
const OF_SILL: f32 = 0.85;
const OF_MODULE: f32 = 1.5;
const OF_MULLION: f32 = 0.065;
const OF_ROW_PITCH: f32 = 3.2;
const OF_AISLE_PITCH: f32 = 7.2;
const OF_EGRESS: f32 = 0.03;
const OF_SPILL_M: f32 = 2.5;
const OF_LIT_FLOOR_ZONES: f32 = 0.75;
const OF_DARK_FLOOR_ZONES: f32 = 0.05;
const OF_BLIND_ZONES: f32 = 0.35;

// A box-filtered pulse train: the share of [x - w / 2, x + w / 2] (in
// periods) covered by pulses of `duty` starting at each whole number; past a
// period it tends to the duty, so mullions and spandrels fade to their mean.
fn rm_pulse_int(t: f32, duty: f32) -> f32 {
    let f = floor(t);
    return f * duty + min(t - f, duty);
}

fn rm_pulse(x: f32, duty: f32, w: f32) -> f32 {
    let ww = max(w, 1e-3);
    return (rm_pulse_int(x + 0.5 * ww, duty) - rm_pulse_int(x - 0.5 * ww, duty)) / ww;
}

// a zone's lights on (0..1) on storey `storey` of the face `face`, given its floor lit (0..1)
fn rm_of_zone(face: f32, storey: f32, zone: f32, floor_lit: f32) -> f32 {
    let roll = rm_h(fract(face * 7.13 + storey * 0.6180339 + zone * 0.4142136), 63.0);
    let share = mix(OF_DARK_FLOOR_ZONES, OF_LIT_FLOOR_ZONES, floor_lit);
    return rm_ss(roll - 0.04, roll + 0.04, share);
}

// `pos` the glass fragment, `tangent` the facade's +u, `dir` the view ray in
// the pane frame, `local`/`size` the pane (rm_kit_pane), `anchored` x, y 1
// where the frame did not locate its window (a curtain wall's 3.6 m cells:
// mullions and spandrels are drawn here), `face` the facade plane's hash.
fn rm_office_floor(pos: vec3<f32>, tangent: vec3<f32>, dir: vec3<f32>, local: vec2<f32>, size: vec2<f32>, anchored: vec2<f32>, face: f32, distance: f32, env: vec4<f32>, work: vec4<f32>) -> RmOut {
    var o: RmOut;
    let into = max(dir.z, 0.02);
    let d = vec3<f32>(dir.x, dir.y, into);
    let a0 = dot(pos, tangent);
    // the storey and the eye ray's height over its floor at the glass
    var storey = 0.0;
    var y0 = 0.0;
    var ceil_h = OF_CEILING;
    if (anchored.y > 0.5) {
        storey = floor(pos.y / OF_STOREY);
        y0 = pos.y - storey * OF_STOREY;
    } else {
        storey = floor((pos.y - local.y - OF_SILL) / OF_STOREY + 0.5);
        y0 = local.y + OF_SILL;
        ceil_h = max(OF_CEILING, size.y + OF_SILL + 0.05);
    }
    let fs = fract(face + storey * 0.377);
    // the glass's footprint of a pixel (m), and the frame over it: spandrel, mullions
    let glass_fp = distance * work.y / into;
    var spandrel = 0.0;
    if (anchored.y > 0.5) {
        spandrel = rm_pulse((pos.y - ceil_h) / OF_STOREY, (OF_STOREY - ceil_h) / OF_STOREY, glass_fp / OF_STOREY);
    }
    var mullion = 0.0;
    if (anchored.x > 0.5) {
        mullion = rm_pulse(a0 / OF_MODULE + 0.5 * OF_MULLION / OF_MODULE, OF_MULLION / OF_MODULE, glass_fp / OF_MODULE);
    } else if (size.x > 1.6 * OF_MODULE) {
        // a wide window split into equal modules on its own frame
        let wm = size.x / round(size.x / OF_MODULE);
        mullion = rm_pulse(local.x / wm + 0.5 * OF_MULLION / wm, OF_MULLION / wm, glass_fp / wm) * step(0.5 * OF_MULLION, local.x) * step(local.x, size.x - 0.5 * OF_MULLION);
    }
    let frame_cov = 1.0 - (1.0 - spandrel) * (1.0 - mullion);

    // ---- the ray: the core, the ceiling or the floor, a workstation row ----
    let depth = mix(9.0, 14.0, rm_h(face, 70.0));
    var t = depth / d.z;
    var kind = 0u; // 0 the core, 1 the ceiling, 2 the floor, 3 a panel
    if (d.y > 1e-4) {
        let tc = (ceil_h - y0) / d.y;
        if (tc < t) {
            t = tc;
            kind = 1u;
        }
    } else if (d.y < -1e-4) {
        let tf = -y0 / d.y;
        if (tf < t) {
            t = tf;
            kind = 2u;
        }
    }
    let bench = rm_h(fs, 71.0) > 0.6;
    let panel_h = select(1.2, 1.05, bench);
    let row0 = mix(1.6, 2.6, rm_h(fs, 72.0));
    let aisle_off = rm_h(fs, 73.0) * OF_AISLE_PITCH;
    if (glass_fp < 0.4) {
        for (var k = 0; k < 3; k = k + 1) {
            let tk = (row0 + f32(k) * OF_ROW_PITCH) / d.z;
            if (tk >= t) {
                break;
            }
            let yk = y0 + d.y * tk;
            let ak = a0 + d.x * tk;
            if (yk > 0.0 && yk < panel_h && fract((ak + aisle_off) / OF_AISLE_PITCH) > 0.21) {
                t = tk;
                kind = 3u;
                break;
            }
        }
    }
    let p = vec3<f32>(a0 + d.x * t, y0 + d.y * t, d.z * t);
    let cos_inc = max(select(select(d.z, abs(d.y), kind == 1u || kind == 2u), d.z, kind == 3u), 0.05);
    let fp = (distance + t) * work.y / cos_inc;

    // ---- the zones' light at the hit (its own, a lit neighbour's spill) ----
    let floor_share = min(work.x / 0.7, 1.0);
    let floor_phase = rm_h(fract(face + storey * 0.6180339), 41.0);
    let floor_lit = rm_ss(floor_phase - 0.05, floor_phase + 0.05, floor_share);
    let zone_m = mix(7.2, 21.6, rm_h(fract(face + storey * 0.173), 61.0));
    let zq = (p.x + rm_h(face, 62.0) * 30.0) / zone_m;
    let zi = floor(zq);
    let zf = zq - zi;
    let on = rm_of_zone(face, storey, zi, floor_lit);
    let spill = max(rm_of_zone(face, storey, zi - 1.0, floor_lit) * exp(-zf * zone_m / OF_SPILL_M), rm_of_zone(face, storey, zi + 1.0, floor_lit) * exp(-(1.0 - zf) * zone_m / OF_SPILL_M)) * 0.4;
    let light = max(max(on, spill), OF_EGRESS);
    let zs = fract(face * 3.71 + storey * 0.6180339 + zi * 0.4142136);
    // the tenant's lamps (the floor's, a zone's own now and then) and the zone's dimmer
    let lamp_roll = select(rm_h(fs, 64.0), rm_h(zs, 64.0), rm_h(zs, 66.0) < 0.25);
    let lamp = rm_office_lamp(lamp_roll);
    let level = mix(0.7, 1.0, rm_h(zs, 65.0));
    let light_rad = lamp * level * 0.3 * 0.72;

    // ---- the surfaces and fixtures ----
    var albedo = vec3<f32>(0.5);
    var illum = 1.0;
    var glow = 0.0;
    var day_face = 0.78;
    if (kind == 1u) {
        // the ceiling: 600 mm tiles in a T-bar grid, the tenant's fixtures
        let tb = max(rm_pulse(p.x / 0.6 + 0.012, 0.024, fp / 0.6), rm_pulse(p.z / 0.6 + 0.012, 0.024, fp / 0.6));
        albedo = vec3<f32>(0.78) * (1.0 - 0.35 * tb);
        let fit = rm_h(fs, 74.0);
        var fix = 0.0;
        if (fit < 0.5) {
            // 2x4 troffers (0.6 along x 1.2 deep) on a 2.4 x 3.0 m grid
            fix = rm_pulse(p.x / 2.4, 0.25, fp / 2.4) * rm_pulse(p.z / 3.0 - 0.2, 0.4, fp / 3.0);
        } else if (fit < 0.85) {
            // continuous linear rows along the glass, 2.4 m apart, a break every 3.6 m
            fix = rm_pulse(p.z / 2.4 - 0.3, 0.05, fp / 2.4) * rm_pulse(p.x / 3.6, 0.9, fp / 3.6);
        } else {
            // 2x2 LED panels on a 1.8 m grid
            fix = rm_pulse(p.x / 1.8, 0.333, fp / 1.8) * rm_pulse(p.z / 1.8 - 0.3, 0.333, fp / 1.8);
        }
        glow = fix * 1.8 * on;
        illum = 0.28 * (1.0 - fix);
        day_face = 0.38;
    } else if (kind == 2u) {
        // carpet tile
        albedo = mix(vec3<f32>(0.11, 0.12, 0.14), vec3<f32>(0.16, 0.15, 0.13), rm_h(fs, 75.0)) * (0.9 + 0.2 * rm_h(floor(p.x / 0.5) + floor(p.z / 0.5) * 7.0, 76.0));
        illum = 1.0;
        day_face = 1.0;
    } else if (kind == 3u) {
        // a workstation panel (fabric), its cap lit; benching: the desks' screens over it
        let fabric = mix(mix(vec3<f32>(0.30, 0.30, 0.31), vec3<f32>(0.22, 0.25, 0.30), step(0.4, rm_h(fs, 77.0))), vec3<f32>(0.34, 0.31, 0.27), step(0.75, rm_h(fs, 77.0)));
        let cap = rm_ss(panel_h - 0.06, panel_h - 0.03, p.y);
        albedo = mix(fabric, vec3<f32>(0.62), cap);
        illum = 0.75 * (0.55 + 0.45 * p.y / panel_h);
        if (bench) {
            let cell = floor(p.x / 1.6);
            let scr = rm_ss(panel_h - 0.35, panel_h - 0.33, p.y) * rm_pulse(p.x / 1.6 - 0.3, 0.35, fp / 1.6);
            albedo = mix(albedo, vec3<f32>(0.03), scr);
            glow = scr * step(rm_h(cell + zs * 37.0, 78.0), 0.45) * 0.6 * on;
        }
        day_face = 0.7;
    } else {
        // the core wall: painted, a door every 7-9 m
        let door_p = mix(7.0, 9.0, rm_h(fs, 79.0));
        let door = rm_pulse(p.x / door_p, 1.0 / door_p, fp / door_p) * step(p.y, 2.1);
        albedo = mix(mix(vec3<f32>(0.55, 0.53, 0.49), vec3<f32>(0.48, 0.5, 0.52), rm_h(face, 80.0)), vec3<f32>(0.18, 0.16, 0.14), door);
        illum = mix(0.5, 0.75, clamp(p.y / ceil_h, 0.0, 1.0));
    }
    var rad = (albedo * illum * light + vec3<f32>(glow)) * light_rad;
    var day_room = (albedo * day_face * mix(1.0, 0.24, sqrt(clamp(p.z / depth, 0.0, 1.0))) + vec3<f32>(glow * 1.1)) * RM_DAY_DEPTH;

    // ---- a roller blind at the glass, each module at its own drop ----
    let zg = floor((a0 + rm_h(face, 62.0) * 30.0) / zone_m);
    let zsg = fract(face * 3.71 + storey * 0.6180339 + zg * 0.4142136);
    if (rm_h(zsg, 67.0) < OF_BLIND_ZONES) {
        let m = floor(a0 / OF_MODULE);
        let mr = select(rm_h(zsg, 68.0), rm_h(fract(zsg + m * 0.137), 69.0), rm_h(fract(zsg + m * 0.071), 70.0) < 0.35);
        let drop = floor(mr * 4.0) / 3.0 * 0.95;
        let covered = rm_ss(ceil_h * (1.0 - drop) - glass_fp, ceil_h * (1.0 - drop) + glass_fp, y0) * step(0.01, drop);
        let own = max(rm_of_zone(face, storey, zg, floor_lit), OF_EGRESS);
        rad = mix(rad, vec3<f32>(0.2) * own * light_rad, covered);
        day_room = mix(day_room, vec3<f32>(0.62, 0.6, 0.56) * 0.7, covered);
    }

    let fresnel = 0.04 + 0.96 * pow(1.0 - into, 5.0);
    let far = rm_ss(140.0, 420.0, distance) * 0.55;
    let show = (1.0 - fresnel) * (1.0 - far) * 0.95;
    o.day = vec4<f32>(min(day_room * 1.2, vec3<f32>(1.0)) * RM_DAY_THROUGH, show);
    // the frame: dark anodised mullions, the spandrel's back-painted glass
    let frame_col = mix(vec3<f32>(0.08, 0.085, 0.09), vec3<f32>(0.05, 0.06, 0.07), spandrel / max(frame_cov, 1e-3));
    o.day = mix(o.day, vec4<f32>(frame_col, 1.0), frame_cov);
    o.night = rad * (1.0 - frame_cov) * (1.0 - fresnel) * env.x * lk_practical_gain();
    return o;
}

// ---- (`--room-depth`) a lit home and a lit shop seen from the night street ----
// A home's lamp as a point source (inverse square and cosine) over the room's
// inter-reflection: most homes by a ceiling fixture, the rest by a floor or
// table lamp low by a side wall with the ceiling light off, its shade seen
// lit; the coverings at the glass (curtains, blinds, sheers) glow by the
// light reaching their backs, a hot spot over the lamp; the pane cap on the
// room's level, not each pixel's. `RD_LAMP_K` puts a surface 1.5 m from the
// lamp, facing it at 45 deg, at about 1.0 (the old falloff's near level), 3 m
// off at a third; `RD_BOUNCE` the room's inter-reflection, about an eighth of
// it in a pale room; `RD_FABRIC` an unlined curtain's transmittance (light
// cotton or linen 0.3-0.6) with its colour, `RD_SLATS` a blind's or sheer's,
// each against the light at the glass (about 0.5 half a room's depth from
// the lamp): the covering's middle a little over the old flat level, its
// edges under it; `RD_ROOM_CAP` half the old pane cap (0.24), on the room's
// mean: a bright room's mean comes down to a dim one's, its lit wall by the
// lamp a little over the old cap, its corners well under. A shop's lit surfaces (its walls,
// floor, goods: not its fixtures' glow) at `RD_SHOP_SURFACE` of the
// window-adapted level (shop_lights.rs `ROOM_GAIN`): seen at the street's
// exposure the room sits ~1.5 stops over a lamp's pool, not the 3-4 that
// clipped a pale wall to white; the fixtures stay blown. INFERRED.
const RD_CEILING_SHARE: f32 = 0.6;
const RD_LAMP_K: f32 = 3.5;
const RD_BOUNCE: f32 = 0.12;
const RD_SHADE: f32 = 1.6;
const RD_FABRIC: f32 = 1.2;
const RD_SLATS: f32 = 2.5;
const RD_ROOM_CAP: f32 = 0.12;
const RD_SHOP_SURFACE: f32 = 0.4;

// (stage W5, `--street-life`; weathering.wgsl `wx_window_life_pre` sets it,
// `wx_window` clears it) x 1 on, y the building's wear, z 1 a vacant shop, w
// the share of a flat's windows with sill plants: `rm_room` hands what shows at
// the glass to `wx_room_life` (sill plants, a vacant shop's taped paper).
var<private> rm_w5: vec4<f32>;

// A shop room's lit surfaces for a unit light are about 0.3 (the shell's
// walls, floor and ceiling at the fixture's falloff, rm_room's shop palette);
// they take half the window's mean luminance, which counts the fixtures and
// displays seen through the glass too (the room's own glow terms, brighter).
// INFERRED.
const RM_SHOP_ROOM_MEAN: f32 = 0.6;

struct RmOut {
    // rgb the room's albedo as the street sees it, a its share of the pane (day)
    day: vec4<f32>,
    // the lit room's radiance through the glass (night)
    night: vec3<f32>,
};

// local: metres from the window's lower-left corner; size: window w, h;
// dir: the view ray in the pane frame (along +u, up, into the room > 0);
// seed_kind: (seed, shop, office, distance); floor_key: (shop switch key or
// facade-plane hash, pane bottom); env: (dusk, occupancy, shop occupancy,
// cool share); work: (office share, pixel angle).
fn rm_room(local_in: vec2<f32>, size: vec2<f32>, dir: vec3<f32>, seed_kind: vec4<f32>, floor_key: vec2<f32>, env: vec4<f32>, work: vec4<f32>, day_mode: bool, night_mode: bool) -> RmOut {
    var o: RmOut;
    let shop = seed_kind.y;
    let office = seed_kind.z * (1.0 - shop);
    let distance = seed_kind.w;
    let has_room = step(0.1, size.x) * step(0.1, size.y);
    let win = max(size.x, 0.05);
    let hin = max(size.y, 0.05);
    let cells_x = mix(1.0, max(floor(win / 3.6 + 0.5), 1.0), step(4.8, win));
    let cells_y = mix(1.0, max(floor(hin / 3.6 + 0.5), 1.0), step(4.6, hin));
    let W = win / cells_x;
    let H = hin / cells_y;
    let ix = clamp(floor(local_in.x / W), 0.0, cells_x - 1.0);
    let iy = clamp(floor(local_in.y / H), 0.0, cells_y - 1.0);
    let local = vec2<f32>(local_in.x - ix * W, local_in.y - iy * H);
    let seed = fract(seed_kind.x + ix * 0.6180339 + iy * 0.4142136);
    let split = step(1.5, cells_x * cells_y);

    // ---- lit decision ----
    let band = 0.1;
    let share = mix(env.y, env.z, shop);
    let switched = step(floor_key.x, -1e-3) * shop;
    let threshold = mix(rm_h(seed, 1.0), -floor_key.x, switched);
    let open = rm_ss(threshold - band / 2.0, threshold + band / 2.0, share);
    let lit_home = mix(open, max(open, 0.3), switched);
    let storey = floor((floor_key.y + (iy + 0.5) * H) / 3.6);
    let floor_phase = rm_h(fract(floor_key.x + storey * 0.6180339), 41.0);
    let floor_share = min(work.x / 0.7, 1.0);
    let floor_lit = rm_ss(floor_phase - band / 2.0, floor_phase + band / 2.0, floor_share);
    let bay_lit = step(rm_h(seed, 42.0), 0.7);
    var lit = mix(lit_home, floor_lit * bay_lit, office);
    // (`--night-photometry`) an office floor is lit in zones (a tenant's open plan, a
    // stretch of perimeter offices) that switch with the office share (the storey's
    // phase and the zone's own roll, uniform together), the lit share the bays had
    // (the floors' share x 0.7), with a few dark bays in a lit zone: bands of lit
    // windows, not a per-bay checkerboard.
    let zoned = SF_NIGHT_PHOTOMETRY && look.fog.z > 0.0 && rm_office_zone.w > 0.5 && office > 0.5;
    if (zoned) {
        let zk = fract(floor_phase + rm_office_zone.x);
        let zone_share = floor_share * 0.7 / 0.93;
        lit = rm_ss(zk - band / 2.0, zk + band / 2.0, zone_share) * step(rm_h(seed, 42.0), 0.93);
    }
    // (`--look cinematic`, stage L2: work z the screens' share, w the clock; rooms.rs `drive_look`)
    let cine = SF_CINEMATIC && work.z > 0.0;
    let tv = step(rm_h(seed, 2.0), select(0.12, work.z, cine)) * (1.0 - shop) * (1.0 - office);
    var level = mix(mix(0.35, 1.35, rm_h(seed, 3.0)), mix(0.55, 0.8, rm_h(seed, 3.0)), office);
    if (fg_on()) {
        level = mix(level, mix(0.4, 1.05, rm_h(seed, 3.0)), office); // (`--facade-glass`) offices as varied as homes
    }
    if (zoned) {
        // the zone's dimmer, and a little from bay to bay
        level = mix(0.5, 1.0, rm_office_zone.z) * mix(0.85, 1.0, rm_h(seed, 3.0));
    }
    let cool = step(rm_h(seed, 4.0), env.w - shop * 0.1);
    let temp_mix = rm_h(seed, 5.0);
    let home_color = mix(mix(RM_WARM_A, RM_WARM_B, temp_mix), mix(RM_COOL_A, RM_COOL_B, temp_mix), cool);
    var lamp_color = mix(home_color, mix(RM_OFFICE_A, RM_OFFICE_B, temp_mix), office);
    if (zoned) {
        lamp_color = rm_office_lamp(rm_office_zone.y);
    }
    if (cine) {
        // the look's wider spread of homes: a low amber lamp, a cold daylight LED.
        let roll = rm_h(seed, 44.0);
        let spread = select(select(lamp_color, RM_CINE_DAYLIGHT, roll < RM_CINE_SPREAD.x + RM_CINE_SPREAD.y), RM_CINE_AMBER, roll < RM_CINE_SPREAD.x);
        lamp_color = mix(lamp_color, spread, (1.0 - shop) * (1.0 - office));
    }

    if (fg_on()) {
        // (`--facade-glass`) each room its own lamp's colour temperature, not a white a third desaturated.
        let roll = rm_h(seed, 46.0);
        let tk = rm_h(seed, 47.0);
        var kelvin = mix(2300.0, 3400.0, tk * sqrt(tk));
        if (roll < FG_HOME_COLD_SHARE) {
            kelvin = mix(5000.0, 6200.0, tk);
        } else if (roll < FG_HOME_COLD_SHARE + FG_HOME_COOL_SHARE) {
            kelvin = mix(3800.0, 4500.0, tk);
        }
        // an office floor's tenant fits its own lamps (warm LED to cool fluorescent), so floors differ
        let tenant = rm_h(fract(floor_key.x + floor((floor_key.y + (iy + 0.5) * H) / 3.6) * 0.6180339), 48.0);
        kelvin = mix(kelvin, mix(2900.0, 4800.0, mix(tenant, tk, 0.3)), office);
        lamp_color = mix(fg_lamp(kelvin), lamp_color, shop);
    }

    // ---- the box ----
    let sill = mix(0.85, 0.35, shop);
    let ceil_y = H + mix(0.4, 0.7, shop);
    let floor_y = -sill;
    var depth = mix(mix(2.8, 4.8, rm_h(seed, 6.0)), mix(4.5, 7.5, rm_h(seed, 6.0)), shop);
    // (`--facade-glass`) an office floor runs back to its core: 6.5-11 m of
    // lease depth seen through the glass, not a 3-5 m box.
    let fg = fg_on();
    if (fg) {
        depth = mix(depth, mix(FG_OFFICE_DEPTH.x, FG_OFFICE_DEPTH.y, rm_h(seed, 6.0)), office);
    }
    let pad_x = mix(mix(clamp(W * 0.6, 0.6, 2.2), 0.45, shop), 0.2, split);
    let along = dir.x;
    let up = dir.y;
    let into = max(dir.z, 0.02);
    let to_side = mix(local.x + pad_x, W + pad_x - local.x, step(0.0, along)) / max(abs(along), 1e-3);
    let to_floor_ceil = mix(max(local.y - floor_y, 0.01), max(ceil_y - local.y, 0.01), step(0.0, up)) / max(abs(up), 1e-3);
    let to_back = depth / into;
    let t = min(min(to_side, to_floor_ceil), to_back);
    let back = step(to_back, min(to_side, to_floor_ceil));
    let side = step(to_side, to_floor_ceil) * (1.0 - back);
    let horiz = (1.0 - back) * (1.0 - side);
    let ceiling = horiz * step(0.0, up);
    let floor_face = horiz - ceiling;
    let px_raw = local.x + along * t;
    let px = mix(px_raw, mix(-pad_x, W + pad_x, step(0.0, along)), side);
    let py = mix(local.y + up * t, mix(floor_y, ceil_y, step(0.0, up)), horiz);
    let pz = mix(into * t, depth, back);
    let h_floor = py - floor_y;
    let cos_inc = max(back * into + side * abs(along) + horiz * abs(up), 0.05);
    let footprint = (distance + t) * work.y / cos_inc;

    // ---- light ----
    let lamp_x = W * 0.5 + (rm_h(seed, 7.0) - 0.5) * (W + pad_x);
    let lamp_pos = vec3<f32>(lamp_x, ceil_y - 0.3, depth * mix(0.4, 0.5, shop));
    let to_lamp = lamp_pos - vec3<f32>(px, py, pz);
    let lamp_fall = 1.0 / (1.0 + dot(to_lamp, to_lamp) * mix(0.22, 0.06, shop));
    var illum = mix(lamp_fall * 0.9 + 0.1, lamp_fall * 0.35 + 0.55, office);
    if (fg_on()) {
        // (`--facade-glass`) an office's troffers throw their light down: the desks and floor take
        // it whole, the walls about FG_OFFICE_WALL_K of it, the ceiling only what the room throws
        // back up (FG_OFFICE_CEILING_K), so a lit floor seen from the street is an even glow under
        // its bright fittings, not a white ceiling over a dark room (the hard wedges).
        let face_k = floor_face + (back + side) * FG_OFFICE_WALL_K + ceiling * FG_OFFICE_CEILING_K;
        illum = mix(illum, illum * face_k * FG_OFFICE_FILL, office);
    }
    // (`--room-depth`) a home lit by its lamp as a point source: inverse square
    // and cosine from a ceiling fixture or, in some, a floor or table lamp low
    // by a side wall (the ceiling's light off), over the room's inter-reflection.
    let rd_on = SF_ROOM_DEPTH && (skyu.flags.x & SKY_ROOM_DEPTH) != 0u && shop < 0.5 && office < 0.5;
    var lamp_at = lamp_pos;
    var low_lamp = false;
    var shade_glow = 0.0;
    if (rd_on) {
        low_lamp = rm_h(seed, 81.0) > RD_CEILING_SHARE;
        if (low_lamp) {
            lamp_at = vec3<f32>(select(0.4 - pad_x, W + pad_x - 0.4, rm_h(seed, 82.0) > 0.5), floor_y + mix(1.1, 1.5, rm_h(seed, 83.0)), depth * mix(0.55, 0.85, rm_h(seed, 84.0)));
        }
        let nrm = vec3<f32>(-sign(along) * side, floor_face - ceiling, -back);
        let tl = lamp_at - vec3<f32>(px, py, pz);
        let dl2 = dot(tl, tl);
        let cosl = max(dot(nrm, tl) * inverseSqrt(max(dl2, 1e-4)), 0.0);
        illum = RD_LAMP_K * cosl / (dl2 + 0.25) + RD_BOUNCE;
        // the shade's light up on the ceiling round it
        let r2 = (px - lamp_at.x) * (px - lamp_at.x) + (pz - lamp_at.z) * (pz - lamp_at.z);
        illum = illum + ceiling * select(0.5 / (1.0 + r2 * 3.0), 0.9 / (1.0 + r2 * 1.2), low_lamp);
        if (low_lamp) {
            // the lit shade itself, where the ray passes it before the room's surface
            let o = vec3<f32>(local.x, local.y, 0.0);
            let dv = vec3<f32>(along, up, into);
            let tc = dot(lamp_at - o, dv) / dot(dv, dv);
            if (tc > 0.0 && tc < t) {
                shade_glow = rm_ss(0.24, 0.17, length(o + dv * tc - lamp_at));
            }
        }
    }

    // ---- surfaces ----
    let paint = rm_h(seed, 8.0);
    let wall_col = mix(
        mix(vec3<f32>(0.78, 0.74, 0.66), vec3<f32>(0.70, 0.61, 0.49), step(0.35, paint)),
        mix(vec3<f32>(0.55, 0.61, 0.52), vec3<f32>(0.56, 0.60, 0.67), step(0.82, paint)),
        step(0.62, paint),
    );
    let office_wall = vec3<f32>(0.58, 0.59, 0.60);
    let shop_paint = rm_h(seed, 120.0);
    let shop_wall = mix(
        mix(vec3<f32>(0.24, 0.30, 0.27), vec3<f32>(0.42, 0.25, 0.20), step(0.3, shop_paint)),
        mix(vec3<f32>(0.62, 0.58, 0.50), vec3<f32>(0.27, 0.31, 0.38), step(0.82, shop_paint)),
        step(0.55, shop_paint),
    );
    let wall = mix(mix(wall_col, shop_wall, shop), office_wall, office);
    let floor_col = mix(mix(vec3<f32>(0.36, 0.24, 0.15), vec3<f32>(0.34, 0.33, 0.31), step(0.5, rm_h(seed, 9.0))), vec3<f32>(0.17, 0.18, 0.21), office);
    var ceil_col = mix(vec3<f32>(0.86, 0.85, 0.83), vec3<f32>(0.56, 0.56, 0.55), office);
    // (`--facade-glass`) each office its own ceiling layout (the troffers' and
    // the T-bar grid's offset in the room), its acoustic tile a white (~0.8).
    var t_off = vec2<f32>(0.0);
    if (fg) {
        t_off = vec2<f32>(rm_h(seed, 50.0) * 2.4, rm_h(seed, 51.0) * 1.8);
        ceil_col = mix(ceil_col, FG_OFFICE_CEILING, office);
    }
    var albedo = wall * (back + side * 0.72) + ceil_col * ceiling + floor_col * floor_face;
    let fixture_disc = rm_ss(0.32, 0.12, length(vec2<f32>(px - lamp_pos.x, pz - lamp_pos.z))) * ceiling;
    let grid_u = fract(px / 1.2);
    let grid_v = fract(pz / 1.8);
    let shop_panel = rm_filtered(step(0.32, grid_u) * step(grid_u, 0.68) * step(0.4, grid_v) * step(grid_v, 0.6), 0.072, footprint, 0.36) * ceiling;
    let t_u = fract((px + t_off.x) / 2.4);
    let t_v = fract((pz + t_off.y) / 1.8 + 0.25);
    let troffer = rm_filtered(rm_ss(0.37, 0.385, t_u) * rm_ss(0.63, 0.615, t_u) * rm_ss(0.16, 0.18, t_v) * rm_ss(0.84, 0.82, t_v), 0.167, footprint, 0.6) * ceiling;
    let tbar_u = abs(fract((px + t_off.x) / 0.6 + 0.5) - 0.5);
    let tbar_v = abs(fract((pz + t_off.y) / 0.6 + 0.5) - 0.5);
    let tbar = rm_filtered(1.0 - rm_ss(0.02, 0.035, tbar_u) * rm_ss(0.02, 0.035, tbar_v), 0.1, footprint, 0.12) * ceiling * office;
    albedo = albedo * (1.0 - tbar * 0.45);
    let fixture = mix(mix(fixture_disc, shop_panel, shop), troffer, office);
    var glow = vec3<f32>(fixture * mix(1.2, 1.5, office));
    if (rd_on) {
        // (`--room-depth`) a low lamp's room: the ceiling fixture off, the shade lit
        glow = select(glow, vec3<f32>(shade_glow * RD_SHADE), low_lamp);
    }

    // ---- furniture: back-wall features (homes), workstations (offices) ----
    {
        let home = (1.0 - shop) * (1.0 - office);
        let feature = rm_h(seed, 10.0);
        let fx0 = rm_h(seed, 11.0) * (W + pad_x) - pad_x * 0.5;
        let art_in = step(fx0, px) * step(px, fx0 + 0.7) * step(1.35, h_floor) * step(h_floor, 1.95);
        let art_inner = step(fx0 + 0.05, px) * step(px, fx0 + 0.65) * step(1.4, h_floor) * step(h_floor, 1.9);
        let art_col = mix(vec3<f32>(0.25, 0.32, 0.45), vec3<f32>(0.62, 0.42, 0.28), rm_h(seed, 12.0));
        let art = art_in * step(feature, 0.35) * back;
        let shelf_in = step(fx0, px) * step(px, fx0 + 1.0) * step(h_floor, 2.05) * step(feature, 0.6) * step(0.35, feature) * back;
        let book_cell = floor(px / 0.06);
        let book_row = fract(h_floor / 0.36);
        let book = rm_filtered(step(0.18, book_row) * step(rm_h(book_cell * 0.0137 + seed, 13.0), 0.82), 0.67, footprint, 0.06);
        let book_col = mix(vec3<f32>(0.36, 0.24, 0.18), vec3<f32>(0.24, 0.27, 0.32), rm_h(book_cell * 0.0213 + seed, 14.0)) * (rm_h(book_cell * 0.031 + seed, 19.0) * 0.4 + 0.7);
        let book_tone = rm_filtered3(book_col, vec3<f32>(0.30, 0.255, 0.25) * 0.9, footprint, 0.06);
        let cabinet = step(1.45, h_floor) * step(h_floor, 2.2) * step(0.6, feature) * step(feature, 0.82) * back;
        let cab_groove = rm_filtered(rm_ss(0.47, 0.5, abs(fract(px / 0.5) - 0.5)), 0.03, footprint, 0.1);
        let under_light = step(1.38, h_floor) * step(h_floor, 1.45) * step(0.6, feature) * step(feature, 0.82) * back;
        let counter = step(h_floor, 0.92) * step(0.6, feature) * step(feature, 0.82) * back;
        let sofa = step(h_floor, 0.85) * max(step(0.82, feature), step(feature, 0.35)) * back * step(fx0 - 0.4, px) * step(px, fx0 + 1.6);
        albedo = albedo * (1.0 - art * home) + mix(vec3<f32>(0.08, 0.07, 0.06), art_col, art_inner) * art * home;
        albedo = albedo * (1.0 - shelf_in * home) + mix(vec3<f32>(0.32, 0.22, 0.14), book_tone, book) * shelf_in * home;
        albedo = albedo * (1.0 - cabinet * home) + vec3<f32>(0.80, 0.78, 0.72) * (1.0 - cab_groove * 0.5) * cabinet * home;
        albedo = albedo * (1.0 - counter * home) + vec3<f32>(0.25, 0.22, 0.2) * counter * home;
        albedo = albedo * (1.0 - sofa * home) + mix(vec3<f32>(0.22, 0.24, 0.30), vec3<f32>(0.40, 0.30, 0.22), rm_h(seed, 15.0)) * sofa * home;
        glow = glow + rm_filtered(under_light, 0.0, footprint, 0.07) * home * 2.2;
        let lamp_spot_x = fx0 + rm_h(seed, 16.0) * 1.4;
        let spot = rm_ss(0.55, 0.0, length(vec2<f32>(px - lamp_spot_x, h_floor - 1.05))) * back * home * step(0.4, rm_h(seed, 17.0));
        glow = glow + spot * 1.4;
        let desk = rm_filtered(step(0.70, h_floor) * step(h_floor, 0.76), 0.75, footprint, 0.06) * office * (back + side);
        let pedestal = step(h_floor, 0.7) * office * (back + side);
        let screen_cell = fract((px + pz) / 1.6);
        let monitor = rm_filtered(step(0.78, h_floor) * step(h_floor, 1.12) * step(0.3, screen_cell) * step(screen_cell, 0.62), 0.11, footprint, 0.34) * office * (back + side);
        albedo = albedo * (1.0 - desk * 0.75) * (1.0 - pedestal * 0.35) * (1.0 - monitor * 0.8);
        glow = glow + monitor * step(0.55, rm_h(seed, 43.0)) * 0.5;
    }
    // The TV room: main light off, the screen and its cool spill only.
    let screen_x = W * 0.5;
    let screen = step(abs(px - screen_x), 0.55) * step(0.85, h_floor) * step(h_floor, 1.5) * back;
    illum = mix(illum, rm_ss(2.6, 0.0, length(vec2<f32>(px - screen_x, h_floor - 1.1))) * 0.55 + 0.05, tv);
    var screen_color = RM_SCREEN;
    var flicker = 1.0;
    if (cine) {
        // a screen's picture: its brightness cut to cut, its colour wandering cool to warm.
        let ph = work.w * mix(0.6, 1.4, rm_h(seed, 45.0)) + seed * 97.0;
        flicker = 0.7 + 0.3 * sin(ph * 2.3) * sin(ph * 0.71 + 1.3);
        screen_color = mix(RM_SCREEN, mix(vec3<f32>(0.55, 0.75, 1.0), vec3<f32>(1.0, 0.8, 0.62), 0.5 + 0.5 * sin(ph * 0.37)), 0.45);
    }
    lamp_color = mix(lamp_color, screen_color, tv * 0.85);
    glow = glow * (1.0 - tv) + screen * tv * 2.4 * flicker;
    if (cine) {
        illum = mix(illum, illum * flicker, tv);
    }
    let room_radiance = albedo * illum + glow;

    // ---- daylight ----
    let deep = clamp(pz / depth, 0.0, 1.0);
    let day_face = back * 0.78 + side * 0.55 + ceiling * 0.38 + floor_face * 1.0;
    let day_fall = mix(1.0, 0.24, sqrt(deep));
    let day_illum = day_face * day_fall;
    // the room is lit by the daylight that reaches into it (RM_DAY_DEPTH of the facade's).
    var day_room = (albedo * day_illum + glow * max(shop, office) * 1.1) * RM_DAY_DEPTH;
    if (fg) {
        // (`--facade-glass`) an office ceiling near the glass takes the light
        // the floor and desks throw up; its troffers are on in most rooms
        // (FG_OFFICE_LIT) and read by day as a lit diffuser does against the
        // daylight, a few times the tile round it, not as a lamp (the
        // default's gain drew every office's troffers as white shards).
        let fg_illum = (day_face + ceiling * office * FG_OFFICE_CEILING_DAY) * day_fall;
        let fg_glow = shop * 1.1 + office * FG_TROFFER_DAY * step(rm_h(seed, 52.0), FG_OFFICE_LIT);
        day_room = (albedo * fg_illum + glow * fg_glow) * RM_DAY_DEPTH;
    }

    // ---- at the glass: curtains, blinds, sheers ----
    var light_radiance = lamp_color * level * 0.3 * mix(1.0, 0.72, office);
    // (`--shop-light`) a shop room lit by its own light (its colour and its
    // window's luminance), or dark when its shop is closed; each room keeps the
    // rooms' rule past its own distance in the lights' last stretch.
    var shop_lit = -1.0;
    if (rm_shop_override.w != 0.0 && shop > 0.5 && rm_h(seed, 77.0) >= abs(rm_shop_override.w) - 1.0) {
        shop_lit = select(0.0, 1.0, rm_shop_override.w > 0.0);
        light_radiance = rm_shop_override.rgb / RM_SHOP_ROOM_MEAN;
    }
    var radiance = room_radiance * light_radiance;
    let detail = 1.0 - rm_ss(90.0, 220.0, distance);
    {
        let style = rm_h(seed, 30.0);
        let ux = local.x / W;
        let uy = local.y / H;
        let glass_footprint = distance * work.y / into;
        let curtain_style = step(style, 0.26) * (1.0 - shop) * (1.0 - office);
        let cl = mix(0.1, 0.42, rm_h(seed, 31.0));
        let cr = mix(0.1, 0.42, rm_h(seed, 32.0));
        let in_curtain = max(step(ux, cl), step(1.0 - cr, ux)) * curtain_style;
        let folds = rm_filtered(sin(local.x * 11.0 + seed * 40.0) * 0.1 + sin(local.x * 4.3 + seed * 13.0) * 0.06, 0.0, glass_footprint, 0.3) + 0.84;
        let curtain_col = mix(mix(vec3<f32>(0.85, 0.78, 0.62), vec3<f32>(0.62, 0.22, 0.18), step(0.6, rm_h(seed, 33.0))), vec3<f32>(0.35, 0.45, 0.62), step(0.85, rm_h(seed, 33.0)));
        var curtain = curtain_col * folds * 0.5 * light_radiance * max(illum, 0.5);
        // (`--room-depth`) a covering at the glass glows by the light its lamp
        // sends to its back (inverse square, cosine): a hot spot over the lamp,
        // falling off across the window (a screen's dim light in a TV room).
        var e_glass = 1.0;
        if (rd_on) {
            let tg = lamp_at - vec3<f32>(local.x, local.y, 0.0);
            let dg2 = dot(tg, tg);
            e_glass = mix(RD_LAMP_K * lamp_at.z * inverseSqrt(max(dg2, 1e-4)) / (dg2 + 0.25) + RD_BOUNCE, 0.15, tv);
            curtain = curtain_col * folds * light_radiance * RD_FABRIC * e_glass;
        }
        radiance = mix(radiance, curtain, in_curtain);
        day_room = mix(day_room, curtain_col * folds * 0.62, in_curtain);
        var blind_style = max(step(0.26, style) * step(style, 0.46) * (1.0 - shop), step(style, 0.3) * office);
        var blind_to = mix(0.2, 0.95, rm_h(seed, 34.0));
        var blind_col = vec3<f32>(0.82, 0.8, 0.76);
        var screen = 0.0;
        if (fg) {
            // (`--facade-glass`) most office windows carry a blind or a solar
            // screen, each at its own height, some all the way down: the
            // grid of a tower's windows broken window by window, as a street
            // sees it.
            blind_style = max(blind_style, step(style, FG_OFFICE_BLINDS) * office);
            let b = rm_h(seed, 34.0);
            blind_to = select(mix(0.1, 0.85, b / 0.8), 1.0, b > 0.8);
            screen = step(rm_h(seed, 35.0), FG_SOLAR_SCREENS) * office;
            blind_col = mix(mix(blind_col, vec3<f32>(0.66, 0.66, 0.64), step(0.6, rm_h(seed, 36.0))), FG_SCREEN_COL, screen);
        }
        let covered = step(1.0 - blind_to, uy) * blind_style;
        let slat = fract(local.y / 0.045);
        // a solar screen's open weave lets FG_SCREEN_OPEN of the room through, with no slats.
        let gap = mix(rm_filtered(rm_ss(0.78, 0.86, slat), 0.17, glass_footprint, 0.045) * detail, FG_SCREEN_OPEN, screen);
        let blind = mix(blind_col * 0.42 * light_radiance * select(1.0, RD_SLATS * e_glass, rd_on), radiance * 0.7, gap);
        radiance = mix(radiance, blind, covered);
        day_room = mix(day_room, mix(blind_col * 0.7, day_room * 0.7, gap), covered);
        if (fg) {
            // (`--facade-glass`) a home's roller shade: an opaque fabric pulled
            // part way down, pale by day, glowing in its lamp's colour by night.
            let shade_style = step(0.58, style) * step(style, 0.72) * (1.0 - shop) * (1.0 - office);
            let shade_to = mix(0.15, 0.9, rm_h(seed, 37.0));
            let hem = step(1.0 - shade_to, uy);
            let shade_col = mix(mix(vec3<f32>(0.86, 0.82, 0.72), vec3<f32>(0.78, 0.78, 0.76), step(0.5, rm_h(seed, 38.0))), vec3<f32>(0.42, 0.46, 0.52), step(0.85, rm_h(seed, 38.0)));
            let shaded = hem * shade_style;
            // lit from behind by the room's lamp: brightest across from it, falling off to the edges,
            // the light leaking past the fabric's edges in a thin bright line
            let lamp_u = clamp(lamp_x / W, 0.15, 0.85);
            let glow_at = exp(-pow((ux - lamp_u) / 0.55, 2.0) - pow((uy - 0.75) / 0.9, 2.0));
            let leak = rm_filtered(1.0 - rm_ss(0.012, 0.03, min(local.x, W - local.x)), 0.05, glass_footprint, 0.06);
            radiance = mix(radiance, shade_col * light_radiance * 0.34 * max(illum, 0.6) * (0.55 + 0.75 * glow_at) + light_radiance * leak * 0.25, shaded);
            day_room = mix(day_room, shade_col * 0.72, shaded);
        }
        let sheer_style = step(0.46, style) * step(style, 0.58) * (1.0 - shop) * (1.0 - office);
        radiance = mix(radiance, radiance * 0.45 + light_radiance * 0.18 * select(1.0, RD_SLATS * e_glass, rd_on), sheer_style);
        day_room = mix(day_room, day_room * 0.5 + vec3<f32>(0.25), sheer_style);
    }
    // Soft cap on the max channel toward paneCap 0.24 (not on a shop lit by its own light).
    if (rd_on && shop_lit < 0.0) {
        // (`--room-depth`) the cap on the room's level (its mean, about half its
        // light's), not on each pixel: the lamp's hot spot and the dark corners stay.
        let peak = max(max(light_radiance.x, light_radiance.y), light_radiance.z) * 0.5;
        let over = peak / RD_ROOM_CAP;
        radiance = radiance * pow(1.0 + over * over * over, -1.0 / 3.0);
    } else if (shop_lit < 0.0) {
        let peak = max(max(max(radiance.x, radiance.y), radiance.z), 1e-4);
        let over = peak / 0.24;
        radiance = radiance * pow(1.0 + over * over * over, -1.0 / 3.0);
    }
    let fresnel = 0.04 + 0.96 * pow(1.0 - into, 5.0);
    let day_level = mix(0.62, 1.18, rm_h(seed, 3.0));
    let far = rm_ss(140.0, 420.0, distance) * 0.55;
    let show = (1.0 - fresnel) * has_room * (1.0 - far) * 0.95;
    let kind_gain = mix(mix(1.0, 1.2, office), 1.0, shop);
    // all of it is seen through the pane (RM_DAY_THROUGH, the tinted glass's transmission).
    o.day = vec4<f32>(min(day_room * day_level * kind_gain, vec3<f32>(1.0)) * RM_DAY_THROUGH, show);
    // (`--night-photometry`) a room lit by its own lamp carries the practicals' gain (a shop's own light below has it already).
    o.night = radiance * (1.0 - fresnel) * lit * env.x * has_room * lk_practical_gain();
    if (shop_lit >= 0.0) {
        // the light carries the dusk (sky.rs: the lamps' units times the rooms' dusk).
        o.night = radiance * (1.0 - fresnel) * shop_lit * has_room;
    }
    // (stage W5's hook, `rm_w5`: rm_kit_pane hands the room to `wx_room_life`)
    return o;
}

// (`--shop-light`, stage N1) The light of the shop behind a pane at `world`
// (outward normal `n`) in the frame's lights (sky_lights.wgsl `lights_shops`,
// kind SHOP_KIND), the one nearest across this front: rgb its window's
// radiance (its colour times its candela over the glazed area it stands for,
// in the lamps' units: the window's luminance, sky.rs), w 1; w 0: none (the
// shop is dark, or past the clusters).
fn rm_shop_light(world: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let rel = world - skyu.eye.xyz;
    let vz = dot(rel, skyu.cam_fwd.xyz);
    if (vz < skyu.cluster.x || vz > skyu.cluster.y) {
        return vec4<f32>(0.0);
    }
    let vx = dot(rel, skyu.cam_right.xyz);
    let vy = dot(rel, skyu.cam_up.xyz);
    let ndc = vec2<f32>(vx / (vz * skyu.cluster.z * skyu.cluster.w), vy / (vz * skyu.cluster.z));
    let g = skyu.lights.yzw;
    let tx = u32(clamp((ndc.x * 0.5 + 0.5) * f32(g.x), 0.0, f32(g.x) - 1.0));
    let ty = u32(clamp((0.5 - ndc.y * 0.5) * f32(g.y), 0.0, f32(g.y) - 1.0));
    let tz = u32(clamp(log(vz / skyu.cluster.x) / log(skyu.cluster.y / skyu.cluster.x) * f32(g.z), 0.0, f32(g.z) - 1.0));
    let c = (tz * g.y + ty) * g.x + tx;
    let count = min(sky_clusters[c], SKY_CLUSTER_CAP);
    let base = g.x * g.y * g.z + c * SKY_CLUSTER_CAP;
    let nh = normalize(n.xz + vec2<f32>(1e-6, 0.0));
    var best = 1e9;
    var out = vec4<f32>(0.0);
    for (var k = 0u; k < count; k = k + 1u) {
        let li = sky_lights[sky_clusters[base + k]];
        if (li.kind != SHOP_KIND) {
            continue;
        }
        let lh = normalize(li.dir.xz + vec2<f32>(1e-6, 0.0));
        let d = world - li.pos;
        // the same front (its normal), in front of the light by about its setback (a door's
        // recess may stand behind it), along the front within a bay or two, within the storey.
        let ahead = dot(d.xz, lh);
        let along = abs(dot(d.xz, vec2<f32>(-lh.y, lh.x)));
        if (dot(nh, lh) < 0.7 || ahead < -1.0 || ahead > 1.2 || along > 4.0 || abs(d.y) > 3.0) {
            continue;
        }
        let score = along + abs(ahead - 0.3) + 0.5 * abs(d.y);
        if (score < best) {
            best = score;
            out = vec4<f32>(li.color * li.intensity / max(li.size, 0.25), 1.0);
        }
    }
    return out;
}

// (`--shop-light`, stage N2) A kit shop pane's room by its shop's trade
// (shop_rooms.wgsl `sr_room`, its front `f` from the table): one room behind
// every pane of the shop, lit by the shop's own light (`rm_shop_override`:
// its window's radiance where N1's lookup found it, dark where it did not),
// by day seen through the glass at the day's level; a vacant shop's emptied
// room (W5's paper goes over it, `--street-life`, in rm_kit_pane).
fn rm_shop_pane(f: SrFront, pos: vec3<f32>, n: vec3<f32>, dir: vec3<f32>, distance: f32, work: vec4<f32>) -> RmOut {
    var o: RmOut;
    let into = max(dir.z, 0.02);
    var r: SrRoom;
    let w = max(f.s1 - f.s0, 1.0);
    r.size = vec3<f32>(w, clamp(f.head + mix(0.45, 1.2, sr_r(f.key, 1u, 0)), 2.9, 4.8), mix(7.0, 12.0, sr_r(f.key, 2u, 0)));
    r.fit = f.fit;
    r.goods = f.goods;
    r.key = f.key;
    r.vacant = f.vacant;
    r.work = f.work;
    r.kind = f.kind;
    let ro = vec3<f32>(clamp(f.x, 0.0, w), clamp(pos.y - f.floor, 0.0, r.size.y), max(f.z, 0.0));
    // (`--room-depth`) the lit surfaces at the street's exposure (`RD_SHOP_SURFACE`)
    sr_surface = select(1.0, RD_SHOP_SURFACE, SF_ROOM_DEPTH && (skyu.flags.x & SKY_ROOM_DEPTH) != 0u);
    let room = sr_room(ro, vec3<f32>(dir.x, dir.y, into), r, work.y, distance);
    sr_surface = 1.0;
    // (seen as the eye adapted to this window sees it: `sr_adapt`)
    let light = select(vec3<f32>(0.0), rm_shop_override.rgb, rm_shop_override.w > 0.0) * sr_adapt(f.lum);
    let fresnel = 0.04 + 0.96 * pow(1.0 - into, 5.0);
    let far = rm_ss(140.0, 420.0, distance) * 0.55;
    let show = (1.0 - fresnel) * (1.0 - far) * 0.95;
    let day_level = mix(0.85, 1.1, sr_r(f.key, 3u, 0));
    o.day = vec4<f32>(min(room.day * RM_DAY_DEPTH * day_level, vec3<f32>(1.0)) * RM_DAY_THROUGH, show);
    o.night = room.night * light * (1.0 - fresnel);
    return o;
}

// The pane frame (night-interiors.js `kitRoomArgs`, `paneFrameRay`) from a kit
// glass fragment: `pane` the vertex bytes (rooms.rs), `pos` world, `ng` the
// geometry normal (outward), `kind` the glass's room code (1 auto, 2 shop),
// `group` the building's LOD group (stage N2: its shops' fronts).
fn rm_kit_pane(pane: vec4<f32>, pos: vec3<f32>, ng: vec3<f32>, cam: vec3<f32>, kind: u32, env: vec4<f32>, work: vec4<f32>, group: u32) -> RmOut {
    let b = u32(round(pane.z * 255.0));
    let qw = f32(b & 127u);
    let shop_code = f32(b >> 7u);
    let nwq = round(pane.w * 127.0);
    let office_code = step(nwq, -0.5);
    var size = vec2<f32>(qw, abs(nwq)) / 8.0;
    var local = pane.xy * size;
    let n = normalize(ng + vec3<f32>(0.0, 0.0, 1e-5));
    let tangent = normalize(vec3<f32>(-n.z, 0.0, n.x) + vec3<f32>(1e-5, 0.0, 0.0));
    // a frame at the packing's limit (127 steps) does not locate its
    // window: that axis takes 3.6 m room cells anchored in the world, so the
    // seed below holds over each cell (rooms.rs RoomFixes).
    let fg = fg_on();
    let pix = length(pos - cam) * work.y;
    var fg_block = 0.0;
    if (RM_FRAMES && qw >= 127.0) {
        let a = dot(pos, tangent);
        if (fg) {
            // (`--facade-glass`) rooms of two or three mullion modules (by building),
            // the mullions at the modules' joints, filtered to their share under a pixel
            let rw = FG_MODULE_M * select(2.0, 3.0, fract(f32(group) * 0.6180339) > 0.5);
            local.x = a - floor(a / rw) * rw;
            size.x = rw;
            let mm = abs(fract(a / FG_MODULE_M + 0.5) - 0.5) * FG_MODULE_M;
            fg_block = rm_filtered(1.0 - rm_ss(0.5 * FG_MULLION_W - 0.01, 0.5 * FG_MULLION_W + 0.01, mm), FG_MULLION_W / FG_MODULE_M, pix, FG_MODULE_M * 0.25);
        } else {
            local.x = a - floor(a / 3.6) * 3.6;
            size.x = 3.6;
        }
    }
    if (RM_FRAMES && abs(nwq) >= 127.0) {
        if (fg) {
            // (`--facade-glass`) the building's storeys (its sill rhythm, facade_age.wgsl
            // `fa_drivers`): each storey's room behind its vision glass, floor to ceiling, and
            // the opaque spandrel over it to the next floor, not 3.6 m cells of the world
            let dd = fa_drivers(fa_ages[group]);
            let floor0 = dd.base_y + dd.first_sill - FG_SILL_M;
            let yin = pos.y - floor0 - floor((pos.y - floor0) / dd.pitch) * dd.pitch;
            local.y = yin - FG_SILL_M;
            size.y = max(FG_VISION_M - FG_SILL_M - 0.4, 0.5);
            let span = rm_filtered(rm_ss(FG_VISION_M - 0.02, FG_VISION_M + 0.02, yin), (dd.pitch - FG_VISION_M) / dd.pitch, pix, dd.pitch * 0.5);
            fg_block = max(fg_block, span);
        } else {
            local.y = pos.y - floor(pos.y / 3.6) * 3.6;
            size.y = 3.6;
        }
    }
    let view = normalize(pos - cam);
    let dir = vec3<f32>(dot(view, tangent), view.y, -dot(view, n));
    let corner = pos - tangent * local.x - vec3<f32>(0.0, local.y, 0.0);
    let c = floor(corner * 2.0 + 0.5);
    let seed = fract(sin(fract(c.x * 0.1031 + c.y * 0.11369 + c.z * 0.13787) * 311.7) * 43758.5453);
    let forced_shop = select(0.0, 1.0, kind == 2u);
    let shop = max(forced_shop, shop_code);
    let office = office_code * (1.0 - forced_shop);
    let distance = length(pos - cam);
    let nw = normalize(vec3<f32>(n.x, 0.0, n.z) + vec3<f32>(1e-5, 0.0, 0.0));
    let plane_off = floor(dot(pos.xz, nw.xz) * 0.5 + 0.5);
    let heading = floor(nw.x * 8.0 + 0.5) * 0.37 + floor(nw.z * 8.0 + 0.5) * 0.71;
    let plane_hash = fract(sin(plane_off * 0.1031 + heading) * 43758.5453);
    let pane_bottom = pos.y - local.y;
    // A forced shop glass carries its building's switch class in the kind
    // signs and the size parity (not packed here: one class per window from
    // its own bits), handed in negated.
    let switch_class = office_code + shop_code * 2.0 + (qw % 2.0) * 4.0 + (abs(nwq) % 2.0) * 8.0;
    let shop_key = select(plane_hash, -((switch_class + 0.5) / 16.0), kind == 2u);
    // (`--shop-light`, sky.rs SKY_SHOPS: the shop windows' lights placed and on)
    // a kit shop's glass shows its own light's room within the lights' reach, from dusk on.
    rm_shop_override = vec4<f32>(0.0);
    if (SF_SHOP_LIGHT && (skyu.flags.x & SKY_SHOPS) != 0u && kind == 2u && env.x > 0.0) {
        let reach = select(SHOP_REACH_NEAR, SHOP_REACH_FAR, (skyu.flags.x & SKY_SHOPS_FAR) != 0u);
        let fade = rm_ss(SHOP_ROOM_FADE * reach, reach, length(pos - skyu.eye.xyz));
        if (fade < 1.0) {
            let sl = rm_shop_light(pos, n);
            rm_shop_override = vec4<f32>(sl.rgb, select(-(1.0 + fade), 1.0 + fade, sl.w > 0.5));
        }
    }
    // (`--shop-light`, stage N2; the fronts table bound) the shop's front: the one vacancy
    // rule (the light's, W5's paper and dirt, the clutter's boards) and its trade's room.
    var trade = false;
    var front: SrFront;
    if (SF_SHOP_LIGHT && (skyu.flags.x & SKY_SHOPS) != 0u && kind == 2u && lx_shop_fronts[0].y != 0u) {
        front = sr_front(group, pos, n);
        wx_vacant = select(0.0, 1.0, front.found && front.vacant);
        if (rm_w5.x > 0.5) {
            rm_w5.z = wx_vacant;
        }
        let reach = select(SHOP_REACH_NEAR, SHOP_REACH_FAR, (skyu.flags.x & SKY_SHOPS_FAR) != 0u);
        // by night the shop light's decision holds for this room (N1's fade to the rooms' rule, per shop)
        let lit_rule = rm_shop_override.w != 0.0 && sr_r(front.key, 77u, 0) >= abs(rm_shop_override.w) - 1.0;
        trade = front.found && distance < reach && (env.x <= 0.0 || lit_rule);
    }
    var r: RmOut;
    if (SF_NIGHT_PHOTOMETRY && look.fog.z > 0.0 && office > 0.5) {
        // (`--night-photometry`) the office zone: a stretch of 9-24 m along the
        // facade on each storey (3.6 m storeys anchored in the world), its own rolls.
        let storey = floor(pos.y / 3.6);
        let zone_m = mix(9.0, 24.0, rm_h(plane_hash + storey * 0.173, 61.0));
        let zone = floor((dot(pos, tangent) + rm_h(plane_hash, 62.0) * 30.0) / zone_m);
        let zs = fract(plane_hash * 7.13 + storey * 0.6180339 + zone * 0.4142136);
        rm_office_zone = vec4<f32>(rm_h(zs, 63.0), rm_h(zs, 64.0), rm_h(zs, 65.0), 1.0);
    }
    // (`--office-floors`) the open-plan storey behind the whole facade, with its own
    // mullions and spandrels (so `--facade-glass`'s frame below is not laid over it again)
    let office_floor = SF_OFFICE_FLOORS && (skyu.flags.x & SKY_OFFICE_FLOORS) != 0u && office * (1.0 - shop) > 0.5 && !trade;
    if (office_floor) {
        let anchored = vec2<f32>(select(0.0, 1.0, RM_FRAMES && qw >= 127.0), select(0.0, 1.0, RM_FRAMES && abs(nwq) >= 127.0));
        r = rm_office_floor(pos, tangent, dir, local, size, anchored, plane_hash, distance, env, work);
    } else if (trade) {
        r = rm_shop_pane(front, pos, n, dir, distance, work);
    } else {
        r = rm_room(local, size, dir, vec4<f32>(seed, shop, office, distance), vec2<f32>(shop_key, pane_bottom), env, work, true, true);
    }
    rm_shop_override = vec4<f32>(0.0);
    if (fg && fg_block > 0.0 && !office_floor) {
        // (`--facade-glass`) the mullions and the spandrels' opaque panels behind the glass
        r.night = r.night * (1.0 - fg_block);
        r.day = vec4<f32>(mix(r.day.xyz, FG_PANEL, fg_block), mix(r.day.w, 0.95, fg_block));
    }
    rm_office_zone = vec4<f32>(0.0);
    if (rm_w5.x > 0.5) {
        // stage W5 (`--street-life`, weathering.wgsl): plants on a flat's sill, a vacant shop's taped paper
        wx_room_life(&r, local, max(size, vec2<f32>(0.05)), dir, seed, shop, office * (1.0 - shop), work.y, distance);
    }
    return r;
}

// (`--facade-glass`) a kit pane's own normal. A pane of glass is never quite
// in the wall's plane: glazed in putty or a gasket it sits a fraction of a
// degree off (FG_TILT about each axis), and a sealed unit bows in or out with
// the air in it, a few millimetres at its centre over a metre or so, so its
// slope grows from the centre to about FG_BOW at the edges. The reflections
// break pane by pane and curve within each pane, as a street of real windows
// does. A window wider or taller than a pane is cut into panes of about
// 1.25 x 1.7 m (its sashes or lights); a ribbon (the frame at the packing's
// limit) into panes anchored in the world.
fn rm_pane_normal(pane: vec4<f32>, pos: vec3<f32>, ng: vec3<f32>) -> vec3<f32> {
    let b = u32(round(pane.z * 255.0));
    let qw = f32(b & 127u);
    let nwq = round(pane.w * 127.0);
    var size = max(vec2<f32>(qw, abs(nwq)) / 8.0, vec2<f32>(0.05));
    var local = pane.xy * size;
    let n = normalize(ng + vec3<f32>(0.0, 0.0, 1e-5));
    let tangent = normalize(vec3<f32>(-n.z, 0.0, n.x) + vec3<f32>(1e-5, 0.0, 0.0));
    if (qw >= 127.0) {
        let a = dot(pos, tangent);
        local.x = a - floor(a / 1.5) * 1.5;
        size.x = 1.5;
    }
    if (abs(nwq) >= 127.0) {
        local.y = pos.y - floor(pos.y / 1.8) * 1.8;
        size.y = 1.8;
    }
    let cells = max(vec2<f32>(1.0), floor(size / vec2<f32>(1.25, 1.7) + 0.5));
    let cs = size / cells;
    let ci = min(floor(local / cs), cells - 1.0);
    let q = clamp((local - (ci + 0.5) * cs) / (0.5 * cs), vec2<f32>(-1.0), vec2<f32>(1.0));
    let corner = pos - tangent * local.x - vec3<f32>(0.0, local.y, 0.0);
    let key = floor(corner * 2.0 + 0.5) + vec3<f32>(ci.x * 7.0, ci.y * 13.0, 0.0);
    let h1 = fract(sin(dot(key, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
    let h2 = fract(h1 * 31.7 + 0.13);
    let h3 = fract(h1 * 57.3 + 0.71);
    let tilt = (vec2<f32>(h1, h2) - 0.5) * (2.0 * FG_TILT);
    let bow = (h3 - 0.5) * (2.0 * FG_BOW);
    let d = tilt + q * bow;
    return normalize(n + tangent * d.x + vec3<f32>(0.0, d.y, 0.0));
}

// ---- (`--canyon-glass`, `cg_on()`; facade.rs `canyon_from_args`) the street canyon in kit glass ----
// A window reflects what stands across its street: the sky over the
// opposite roofline, the opposite facades below it (sunlit or in shade,
// their own windows in rows, lit rooms by night) and the street under them.
// The probe grid sees from the street floor, so a pane 20 m up read the
// probe's dark lower street where it faces the opposite upper floors and the
// sky. The canyon is the ambient volume's (the city's own prisms): its height
// the plane's reach (where the sky opens over the street), its width from
// the sky the street floor sees (a long canyon of aspect A = H / W shows its
// floor 1 / sqrt(1 + 4 A^2) of the sky), and the roofline's elevation e from
// the sky the window sees straight out (a wall facing a long wall across a
// street sees (1 - sin e) / 2 of the sky). The reflected ray is cast against
// that canyon and what it meets is shaded as the frame shades it: the sky
// (the dome, as bright as the probe grid sees the sky straight up and greyed
// by the cloud deck's cover; the probe itself where it sees open sky along
// the ray), the
// opposite wall (the sun by the shadow maps a step in front of it, the sky
// and street light by the ambient volume there, an albedo by building, its
// storeys of windows and its cornice), the street (the same at its floor).
// A pane at the street keeps half the probe, which sees the street's own
// cars, kerbs and shop fronts. INFERRED: the facades' and the street's mean
// albedos, the window share, storey and bay; the width's clamp is a guard on
// the estimate, not a street dimension.
const CG_STREET_ALBEDO: f32 = 0.13;
const CG_ROOF_ALBEDO: f32 = 0.22;
const CG_STOREY_M: f32 = 3.7;
const CG_BAY_M: vec2<f32> = vec2<f32>(2.6, 4.4);
const CG_LOT_M: f32 = 19.0;
const CG_AIR_M: f32 = 1.5;
const CG_WIDTH_GUARD_M: vec2<f32> = vec2<f32>(6.0, 160.0);
const CG_CORNICE_M: f32 = 0.9;
// the opposite glass: a dark room behind (its albedo-like share of the light on the wall) and the sky it reflects
const CG_GLASS_ROOM: f32 = 0.05;
const CG_GLASS_SKY: f32 = 0.09;
// the probe's zenith read as broad as a cloud deck's patches (a rough lobe), the brightness it
// may set the dome to (a guard), and how far the deck's cover greys the reflected sky
const CG_ZENITH_ROUGH: f32 = 0.6;
const CG_ZENITH_GAIN: vec2<f32> = vec2<f32>(0.5, 2.0);
const CG_CLOUD_GREY: f32 = 0.6;

// (rgb, cover): the opposite wall or street the pane's ray meets, and how much of the pixel it covers
var<private> cg_env: vec4<f32>;
// (rgb, weight): the sky along the ray, and the canyon's weight over the probe (0: the frame as before)
var<private> cg_sky: vec4<f32>;

// a facade's palette by building: weathered concrete, limestone, render, brick, dark granite and tinted glass
fn cg_facade(k: f32) -> vec3<f32> {
    if (k < 0.24) {
        return vec3<f32>(0.30, 0.29, 0.27);
    } else if (k < 0.44) {
        return vec3<f32>(0.40, 0.36, 0.29);
    } else if (k < 0.62) {
        return vec3<f32>(0.46, 0.41, 0.33);
    } else if (k < 0.80) {
        return vec3<f32>(0.27, 0.15, 0.10);
    }
    return vec3<f32>(0.11, 0.12, 0.13);
}

// the light on a surface with normal n at p (in front of it, in the air):
// the sun by the shadow maps and the clouds, the sky and the street by the
// ambient volume there; `env_w` its environment class.
// the sun on what a pane reflects: the cascades' tent (or the far map past them) as `lk_shadow` takes
// it, without the penumbra's search and filter (`--soft-shadows`) or the cascades' blend: a reflection
// blurred over the panes' waviness has no use for a penumbra, and the search was most of the canyon's cost.
fn cg_shadow(world: vec3<f32>, ng: vec3<f32>) -> f32 {
    if (!(SF_SOFT_SHADOWS && LK_SHADOW_SOFT)) {
        return lk_shadow(world, ng); // no penumbra compiled in: the lookup as before
    }
    let probe = frame.copy_step.z > 0.5;
    var vd = dot(world - look.cam_pos.xyz, look.cam_fwd.xyz);
    if (probe) {
        vd = distance(world, look.cam_pos.xyz);
    }
    let split = look.cascade_split;
    if (look.key_dir.w <= 0.0) {
        return 1.0;
    }
    let far = LK_SHADOW_FAR && lk_far_on();
    if (vd >= split.z || !LK_SHADOW_CASCADES) {
        if (!far) {
            return 1.0;
        }
        return mix(1.0, lk_far_shadow(world, ng), look.key_dir.w);
    }
    var c = 0;
    if (vd > split.x) {
        c = 1;
    }
    if (vd > split.y) {
        c = 2;
    }
    let vis = lk_cascade(c, world, ng, probe);
    if (vis < 0.0) {
        return 1.0;
    }
    let fade = lk_smooth(split.w, split.z, vd);
    if (far && fade > 0.0) {
        return mix(1.0, mix(vis, lk_far_shadow(world, ng), fade), look.key_dir.w);
    }
    return mix(1.0, mix(vis, 1.0, fade), look.key_dir.w);
}

fn cg_light(p: vec3<f32>, n: vec3<f32>, env_w: f32) -> vec3<f32> {
    let l = look.key_dir.xyz;
    var sun = vec3<f32>(0.0);
    let nl = dot(n, l);
    if (nl > 0.0) {
        sun = look.key.xyz * nl * cg_shadow(p, n) * sky_cloud_transmittance(p);
    }
    return sun + amb_irradiance(amb_sample(p, n), n, lk_sh(n) * env_w);
}

// the canyon a pane at `world` (outward normal `ng`) sees along `r`, seen from `cam_d` metres; sets `cg_env`, `cg_sky`.
fn cg_canyon(world: vec3<f32>, ng: vec3<f32>, r: vec3<f32>, cam_d: f32) {
    cg_env = vec4<f32>(0.0);
    cg_sky = vec4<f32>(0.0);
    let nh = vec3<f32>(ng.x, 0.0, ng.z);
    if (!amb_on() || dot(nh, nh) < 0.25) {
        return; // no volume, or not an upright pane: the frame as before
    }
    let nf = normalize(nh);
    let tf = vec3<f32>(-nf.z, 0.0, nf.x);
    let pl = amb_plane(world.xz + nf.xz * 2.0);
    let h = max(world.y - pl.z, 0.3);
    let reach = max(pl.w, 1.0);
    // the canyon's width from the sky its floor sees, and the opposite roofline from the sky the pane sees straight out
    let svf = clamp(pl.x + pl.y, 0.04, 1.0);
    let aspect = 0.5 * sqrt(max(1.0 / (svf * svf) - 1.0, 0.0));
    let width = clamp(reach / max(aspect, 1e-3), CG_WIDTH_GUARD_M.x, CG_WIDTH_GUARD_M.y);
    let sin_e = clamp(1.0 - 2.0 * amb_vis(amb_sample(world, nf), nf), 0.0, 0.995);
    let roof = h + width * sin_e / sqrt(1.0 - sin_e * sin_e);
    let paint_w = look.ambient.w * look.env_class.x;
    var sky = lk_sky_sample(r);
    if (pg_live()) {
        // the dome has no clouds: the sky the probe grid sees straight up (open even in a
        // narrow street) against the dome's there sets its brightness (its luminance only: the
        // dome's and the sky pass's colours differ, so a ratio by channel would tint it), and
        // the cloud deck's cover greys it
        let sel = pg_select(world);
        if (sel.cover > 0.0) {
            let up = vec3<f32>(0.0, 1.0, 0.0);
            let k = clamp(lk_luma(pg_sample(sel, up, pg_lod(CG_ZENITH_ROUGH))) / max(lk_luma(lk_sky_sample(up)), 1e-5), CG_ZENITH_GAIN.x, CG_ZENITH_GAIN.y);
            sky = mix(sky, sky * k, sel.cover);
        }
    }
    sky = mix(sky, vec3<f32>(lk_luma(sky)), clamp(skyu.cloud0.z, 0.0, 1.0) * CG_CLOUD_GREY);
    cg_sky = vec4<f32>(sky, mix(0.5, 1.0, rm_ss(2.5, 7.0, h)));
    let rn = dot(r, nf);
    let t_wall = select(1e6, width / rn, rn > 1e-3);
    let t_ground = select(1e6, h / -r.y, r.y < -1e-4);
    if (t_ground < t_wall && t_ground < 1e5) {
        // the street under the opposite wall
        let p = world + r * t_ground;
        let lg = cg_light(p + vec3<f32>(0.0, 0.2, 0.0), vec3<f32>(0.0, 1.0, 0.0), look.ambient.w * look.env_class.z);
        cg_env = vec4<f32>(CG_STREET_ALBEDO / LK_PI * lg, 1.0);
        return;
    }
    if (t_wall >= 1e5) {
        return; // along the street or up: the sky
    }
    let p = world + r * t_wall;
    let y = h + r.y * t_wall;
    // the reflection's footprint there: the pixel's cone through the mirror, the panes' waviness and the glass's own blur
    let f = look.rooms_work.y * (cam_d + t_wall) + t_wall * 2.0 * (FG_TILT + FG_BOW + FG_ROUGH * FG_ROUGH) + 0.02;
    // the building across, by its lot: its height about the roofline, its palette and its bays
    let plane_key = floor((dot(world, nf) + width) / 7.0);
    let along = dot(p, tf);
    let lot = floor(along / CG_LOT_M + rm_h(plane_key * 0.0731, 60.0));
    let seed = fract(lot * 0.6180339 + plane_key * 0.4142136);
    let top = max(roof * mix(0.72, 1.14, rm_h(seed, 61.0)), 1.0);
    let cover = 1.0 - rm_ss(top - f - 0.1, top + f + 0.1, y);
    let n_o = -nf;
    // the opposite wall's light, a step in front of it (in the street's air: a wall the shadow maps may place a little nearer or further)
    let p_air = world + r * max(t_wall - CG_AIR_M / max(rn, 0.05), 0.0);
    let e_wall = cg_light(p_air, n_o, paint_w);
    let k = rm_h(seed, 62.0);
    let albedo = cg_facade(k) * mix(0.8, 1.15, rm_h(seed, 63.0));
    let glazed = step(0.8, k);
    // its storeys of windows (a curtain wall nearly all glass), the ground storey a shop front
    let bay = mix(CG_BAY_M.x, CG_BAY_M.y, rm_h(seed, 64.0));
    let u = fract(along / bay + rm_h(seed, 65.0));
    let storey = mix(CG_STOREY_M, CG_STOREY_M * 1.2, rm_h(seed, 66.0));
    let vy = fract(y / storey);
    let ground_floor = step(y, 4.5);
    let pane_u = mix(rm_ss(0.16, 0.2, u) * rm_ss(0.84, 0.8, u), rm_ss(0.02, 0.04, u) * rm_ss(0.98, 0.96, u), max(glazed, ground_floor));
    let pane_v = mix(rm_ss(0.26, 0.3, vy) * rm_ss(0.9, 0.86, vy), rm_ss(0.05, 0.08, vy) * rm_ss(0.97, 0.94, vy), glazed);
    let share = mix(0.38, 0.85, glazed);
    let win = rm_filtered(pane_u * mix(pane_v, step(0.6, y), ground_floor), share, f, min(bay, storey));
    let r_o = reflect(r, n_o);
    let glass = CG_GLASS_ROOM / LK_PI * e_wall + CG_GLASS_SKY * lk_sky_sample(r_o);
    // by night its rooms lit by the rooms' own occupancy, each its lamp
    let cell = vec2<f32>(floor(along / bay), floor(y / storey));
    let ch = rm_h(fract(seed + cell.x * 0.0137 + cell.y * 0.1173), 67.0);
    let lit = step(ch, look.rooms_env.y * 0.8) * look.rooms_env.x;
    let lamp = fg_lamp(mix(2400.0, 3800.0, rm_h(ch, 68.0))) * mix(0.04, 0.11, rm_h(ch, 69.0));
    let night = rm_filtered3(lamp * lit, lamp * look.rooms_env.y * 0.8 * look.rooms_env.x, f, min(bay, storey));
    // the cornice's shadow along each roofline, and grime darkening toward the street
    let under_cornice = 1.0 - 0.45 * rm_ss(top - CG_CORNICE_M - f, top - CG_CORNICE_M * 0.4, y) * (1.0 - rm_ss(top - 0.15, top, y));
    let wall = albedo / LK_PI * e_wall * under_cornice * mix(0.85, 1.0, rm_ss(0.0, 6.0, y));
    let opposite = mix(wall, glass + night, win);
    if (r.y < 0.0) {
        // a falling ray past the roofline meets the roofs beyond
        let roofs = CG_ROOF_ALBEDO / LK_PI * cg_light(p + vec3<f32>(0.0, top - y + 0.5, 0.0), vec3<f32>(0.0, 1.0, 0.0), paint_w);
        cg_env = vec4<f32>(mix(roofs, opposite, cover), 1.0);
    } else {
        cg_env = vec4<f32>(opposite, cover); // past the roofline, the sky
    }
}

// the canyon over the reflection `probe` the lighting read along the same ray
// (look_light.wgsl): the opposite wall or street where the ray meets one, the
// sky past them (the probe's own where it sees open sky, with its clouds),
// the probe kept by the canyon's weight (half at the street floor).
fn cg_apply(probe: vec3<f32>) -> vec3<f32> {
    let lp = lk_luma(probe);
    let ls = lk_luma(cg_sky.rgb);
    let sky_like = rm_ss(0.55, 0.85, min(lp, ls) / max(max(lp, ls), 1e-6));
    let open = mix(cg_sky.rgb, probe, sky_like);
    return mix(probe, mix(open, cg_env.rgb, cg_env.w), cg_sky.w);
}
