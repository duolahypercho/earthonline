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
// draws its room shell. The window frame comes from the vertex (rooms.rs).

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
    let lit = mix(lit_home, floor_lit * bay_lit, office);
    let tv = step(rm_h(seed, 2.0), 0.12) * (1.0 - shop) * (1.0 - office);
    let level = mix(mix(0.35, 1.35, rm_h(seed, 3.0)), mix(0.55, 0.8, rm_h(seed, 3.0)), office);
    let cool = step(rm_h(seed, 4.0), env.w - shop * 0.1);
    let temp_mix = rm_h(seed, 5.0);
    let home_color = mix(mix(RM_WARM_A, RM_WARM_B, temp_mix), mix(RM_COOL_A, RM_COOL_B, temp_mix), cool);
    var lamp_color = mix(home_color, mix(RM_OFFICE_A, RM_OFFICE_B, temp_mix), office);

    // ---- the box ----
    let sill = mix(0.85, 0.35, shop);
    let ceil_y = H + mix(0.4, 0.7, shop);
    let floor_y = -sill;
    let depth = mix(mix(2.8, 4.8, rm_h(seed, 6.0)), mix(4.5, 7.5, rm_h(seed, 6.0)), shop);
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
    let ceil_col = mix(vec3<f32>(0.86, 0.85, 0.83), vec3<f32>(0.56, 0.56, 0.55), office);
    var albedo = wall * (back + side * 0.72) + ceil_col * ceiling + floor_col * floor_face;
    let fixture_disc = rm_ss(0.32, 0.12, length(vec2<f32>(px - lamp_pos.x, pz - lamp_pos.z))) * ceiling;
    let grid_u = fract(px / 1.2);
    let grid_v = fract(pz / 1.8);
    let shop_panel = rm_filtered(step(0.32, grid_u) * step(grid_u, 0.68) * step(0.4, grid_v) * step(grid_v, 0.6), 0.072, footprint, 0.36) * ceiling;
    let t_u = fract(px / 2.4);
    let t_v = fract(pz / 1.8 + 0.25);
    let troffer = rm_filtered(rm_ss(0.37, 0.385, t_u) * rm_ss(0.63, 0.615, t_u) * rm_ss(0.16, 0.18, t_v) * rm_ss(0.84, 0.82, t_v), 0.167, footprint, 0.6) * ceiling;
    let tbar_u = abs(fract(px / 0.6 + 0.5) - 0.5);
    let tbar_v = abs(fract(pz / 0.6 + 0.5) - 0.5);
    let tbar = rm_filtered(1.0 - rm_ss(0.02, 0.035, tbar_u) * rm_ss(0.02, 0.035, tbar_v), 0.1, footprint, 0.12) * ceiling * office;
    albedo = albedo * (1.0 - tbar * 0.45);
    let fixture = mix(mix(fixture_disc, shop_panel, shop), troffer, office);
    var glow = vec3<f32>(fixture * mix(1.2, 1.5, office));

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
    lamp_color = mix(lamp_color, RM_SCREEN, tv * 0.85);
    glow = glow * (1.0 - tv) + screen * tv * 2.4;
    let room_radiance = albedo * illum + glow;

    // ---- daylight ----
    let deep = clamp(pz / depth, 0.0, 1.0);
    let day_face = back * 0.78 + side * 0.55 + ceiling * 0.38 + floor_face * 1.0;
    let day_fall = mix(1.0, 0.24, sqrt(deep));
    let day_illum = day_face * day_fall;
    // the room is lit by the daylight that reaches into it (RM_DAY_DEPTH of the facade's).
    var day_room = (albedo * day_illum + glow * max(shop, office) * 1.1) * RM_DAY_DEPTH;

    // ---- at the glass: curtains, blinds, sheers ----
    let light_radiance = lamp_color * level * 0.3 * mix(1.0, 0.72, office);
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
        let curtain = curtain_col * folds * 0.5 * light_radiance * max(illum, 0.5);
        radiance = mix(radiance, curtain, in_curtain);
        day_room = mix(day_room, curtain_col * folds * 0.62, in_curtain);
        let blind_style = max(step(0.26, style) * step(style, 0.46) * (1.0 - shop), step(style, 0.3) * office);
        let blind_to = mix(0.2, 0.95, rm_h(seed, 34.0));
        let covered = step(1.0 - blind_to, uy) * blind_style;
        let slat = fract(local.y / 0.045);
        let gap = rm_filtered(rm_ss(0.78, 0.86, slat), 0.17, glass_footprint, 0.045) * detail;
        let blind = mix(vec3<f32>(0.82, 0.8, 0.76) * 0.42 * light_radiance, radiance * 0.7, gap);
        radiance = mix(radiance, blind, covered);
        day_room = mix(day_room, mix(vec3<f32>(0.82, 0.8, 0.76) * 0.7, day_room * 0.7, gap), covered);
        let sheer_style = step(0.46, style) * step(style, 0.58) * (1.0 - shop) * (1.0 - office);
        radiance = mix(radiance, radiance * 0.45 + light_radiance * 0.18, sheer_style);
        day_room = mix(day_room, day_room * 0.5 + vec3<f32>(0.25), sheer_style);
    }
    // Soft cap on the max channel toward paneCap 0.24.
    let peak = max(max(max(radiance.x, radiance.y), radiance.z), 1e-4);
    let over = peak / 0.24;
    radiance = radiance * pow(1.0 + over * over * over, -1.0 / 3.0);
    let fresnel = 0.04 + 0.96 * pow(1.0 - into, 5.0);
    let day_level = mix(0.62, 1.18, rm_h(seed, 3.0));
    let far = rm_ss(140.0, 420.0, distance) * 0.55;
    let show = (1.0 - fresnel) * has_room * (1.0 - far) * 0.95;
    let kind_gain = mix(mix(1.0, 1.2, office), 1.0, shop);
    // all of it is seen through the pane (RM_DAY_THROUGH, the tinted glass's transmission).
    o.day = vec4<f32>(min(day_room * day_level * kind_gain, vec3<f32>(1.0)) * RM_DAY_THROUGH, show);
    o.night = radiance * (1.0 - fresnel) * lit * env.x * has_room;
    return o;
}

// The pane frame (night-interiors.js `kitRoomArgs`, `paneFrameRay`) from a kit
// glass fragment: `pane` the vertex bytes (rooms.rs), `pos` world, `ng` the
// geometry normal (outward), `kind` the glass's room code (1 auto, 2 shop).
fn rm_kit_pane(pane: vec4<f32>, pos: vec3<f32>, ng: vec3<f32>, cam: vec3<f32>, kind: u32, env: vec4<f32>, work: vec4<f32>) -> RmOut {
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
    if (RM_FRAMES && qw >= 127.0) {
        let a = dot(pos, tangent);
        local.x = a - floor(a / 3.6) * 3.6;
        size.x = 3.6;
    }
    if (RM_FRAMES && abs(nwq) >= 127.0) {
        local.y = pos.y - floor(pos.y / 3.6) * 3.6;
        size.y = 3.6;
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
    return rm_room(local, size, dir, vec4<f32>(seed, shop, office, distance), vec2<f32>(shop_key, pane_bottom), env, work, true, true);
}
