//! the room behind kit glazing (src/render/materials/night-interiors.js,
//! `night-interiors-v5-shop-returns`): the window frame each glass vertex
//! needs (`bakeNightWindowUv`), packed into the bytes a kit glass vertex does
//! not use, the per-glass room kind (`NIGHT_INTERIOR_KIT_GLASS`), the
//! building's night use (`nightInteriorUseOf` over frontage-activity.js
//! `frontageUseOf`), the practical drive (`updateNightInteriors` over
//! environment-ibl.js `nightPracticalProfile`) and the WGSL constants of the
//! room (`rooms.wgsl`, a port of `nightRoomFn`).
//!
//! Packing (kit glass vertices only; their paint is unused, the material
//! carries the glass colour): colour r, g = the vertex's place in its window
//! as a fraction of the window's width and height (unorm8), colour b = the
//! window width in 1/8 m (`uvQuant`, 7 bits, so at most 15.875 m where the JS
//! packs 31.875 m) with bit 7 set for a shop window, normal w = the window
//! height in 1/8 m (snorm8 0..127), negative for an office window. A vertex
//! with no window (a face that is not vertical, a window under 0.25 m) packs
//! width 0: no room.

/// NIGHT_INTERIORS constants this port reads (night-interiors.js).
pub const UV_QUANT: f64 = 8.0;
pub const GAP_M: f64 = 0.14;
pub const SHOP_MIN_W: f64 = 1.5;
pub const SHOP_MIN_H: f64 = 2.1;
pub const SHOP_MAX_W: f64 = 12.0;
pub const SHOP_GROUND_BAND_M: f64 = 2.5;
pub const SPLIT_ABOVE_M: f64 = 4.8;
pub const SPLIT_TALL_M: f64 = 4.6;

/// Glass kind code in the material flags (bits 8..10 of a glass material).
pub const ROOM_AUTO: u32 = 1;
pub const ROOM_SHOP: u32 = 2;
pub const ROOM_SHIFT: u32 = 8;

/// A kit glass's visible transmittance (centre of glass, the manufacturers'
/// published performance data), for what is seen through it by day
/// (`--no-pane-transmittance`: one transmission for every pane):
///
/// * `glass-clear`: a clear insulating unit, two 6 mm clear float lites of
///   about 0.89 each (Viracon, 1/4 in monolithic clear), 0.89^2, about 0.79;
/// * `glass-sky`: the blue-grey and neutral low-e units of the 2000s, Tv
///   0.64 (Vitro, Solarban 70 on clear in a 1 in unit); `glass-shell` (a
///   hollow building's stand-in for its clear, door and film glass) as one;
/// * `glass-dark`: the grey and bronze body tints of the 1960s-80s, Tv 0.40
///   in a 1 in unit with clear (Vitro, Solargray; Viracon's 1 in grey unit
///   0.39; Solargray monolithic 0.44, Solarbronze 1/4 in 0.53);
/// * `glass-film`: a dark tint behind a solar film, Tv about 0.30 (an estimate);
/// * `glass-reflective`: the silver and grey reflective coatings of the
///   1970s-80s, about 0.25 (an estimate inside the coatings' range of about
///   0.1-0.35 by coating and substrate).
///
/// A room is lit through the pane and seen back through it, so what shows
/// goes as Tv squared; `PANE_TV_REFERENCE` (the clear unit) is what
/// `DAY_THROUGH` was set for, so a clear pane reads as before. The far box
/// already scales its coated and tinted panes this way (far_box.wgsl).
pub fn pane_transmittance(name: &str) -> Option<f32> {
    Some(match name {
        "glass-clear" => 0.79,
        "glass-sky" => 0.64,
        "glass-shell" => 0.64,
        "glass-dark" => 0.4,
        "glass-film" => 0.3,
        "glass-reflective" => 0.25,
        _ => return None,
    })
}

/// The clear insulating unit's Tv (`pane_transmittance`): a pane at it reads as before.
pub const PANE_TV_REFERENCE: f32 = 0.79;
/// The material flag bits a glass's Tv is stored in (4 bits, Tv x 15; 0 none).
pub const PANE_TV_SHIFT: u32 = 20;

/// The flag bits for a kit glass's Tv (none with `--no-pane-transmittance`).
pub fn pane_tv_flags(name: &str) -> u32 {
    if !RoomFixes::from_args().pane_tv {
        return 0;
    }
    pane_transmittance(name).map(|t| ((t * 15.0).round().clamp(1.0, 15.0) as u32) << PANE_TV_SHIFT).unwrap_or(0)
}

/// `NIGHT_INTERIOR_KIT_GLASS`: which kit glass draws a room, and how. The shop
/// families (`glass-shop-*`) draw the shop room; their trade fittings are not
/// ported (the room shell only).
pub fn kit_glass_room(name: &str) -> u32 {
    match name {
        "glass-clear" | "glass-dark" | "glass-film" | "glass-shell" | "glass-reflective" | "glass-sky" => ROOM_AUTO,
        "glass-door" | "glass-storefront" => ROOM_SHOP,
        n if n == "glass-shop" || n.starts_with("glass-shop-") => ROOM_SHOP,
        _ => 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    Retail,
    Office,
    Residential,
}

/// `nightInteriorUseOf(building)` on the default world's record (no v3 civic
/// record, so `civic.use` is absent; the transit-station test is not ported).
pub fn night_use(amenity: &str, program: &str, usage: &str) -> Option<Use> {
    const RETAIL: [&str; 16] = [
        "restaurant", "cafe", "bar", "pub", "fast_food", "food_court", "ice_cream", "marketplace", "bank", "pharmacy", "shop", "retail", "nightclub", "cinema", "theatre", "biergarten",
    ];
    const CIVIC: [&str; 14] = [
        "library", "school", "college", "university", "townhall", "courthouse", "police", "fire_station", "hospital", "clinic", "community_centre", "place_of_worship", "post_office", "arts_centre",
    ];
    let a = amenity.to_lowercase();
    let frontage = if !a.is_empty() && RETAIL.contains(&a.as_str()) {
        "trade"
    } else if !a.is_empty() && CIVIC.contains(&a.as_str()) {
        "civic"
    } else if a == "parking" {
        "industrial"
    } else {
        match program.to_lowercase().as_str() {
            "parking" | "warehouse" => "industrial",
            "big-box" => "trade",
            "hall" => "civic",
            "residential" => "residential",
            _ => match usage.to_lowercase().as_str() {
                "retail" => "trade",
                "office" => "office",
                "civic" => "civic",
                "industrial" => "industrial",
                "residential" => "residential",
                _ => "unknown",
            },
        }
    };
    match frontage {
        "trade" | "retail" => Some(Use::Retail),
        "office" | "civic" | "industrial" => Some(Use::Office),
        "residential" => Some(Use::Residential),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Shop,
    Office,
    Residential,
}

/// `nightWindowKind({ w, h, bottomM, use })`.
pub fn window_kind(w: f64, h: f64, bottom: Option<f64>, use_: Option<Use>) -> Kind {
    let tall_wide = h >= SHOP_MIN_H && w >= SHOP_MIN_W;
    let ground = bottom.is_none_or(|b| b <= SHOP_GROUND_BAND_M);
    let ribbon = w > SHOP_MAX_W || (h > SPLIT_TALL_M && w > SPLIT_ABOVE_M);
    match use_ {
        Some(Use::Residential) => return Kind::Residential,
        Some(Use::Office) => return Kind::Office,
        Some(Use::Retail) if ground && tall_wide => return Kind::Shop,
        _ => {}
    }
    if use_ != Some(Use::Retail) && ground && tall_wide && w <= SHOP_MAX_W {
        return Kind::Shop;
    }
    if ribbon { Kind::Office } else { Kind::Residential }
}

/// `bakeNightWindowUv` on one glass piece (positions, normals, indices in
/// world space): faces (connected triangles), vertical faces grouped per
/// plane (heading to 2 degrees, offset to 0.25 m), faces in one plane whose
/// rectangles touch within `GAP_M` merged into one window. Returns per vertex
/// (colour rgb, normal w byte) as packed above. `ground`: the building's
/// ground (its floor), else the lowest window bottom.
pub fn bake(positions: &[[f64; 3]], normals: &[[f32; 3]], indices: &[u32], ground: Option<f64>, use_: Option<Use>) -> Vec<([u8; 3], u8)> {
    let n = positions.len();
    let mut out = vec![([0u8; 3], 0u8); n];
    if n == 0 {
        return out;
    }
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for t in indices.chunks_exact(3) {
        let a = find(&mut parent, t[0] as usize);
        let b = find(&mut parent, t[1] as usize);
        parent[b] = a;
        let c = find(&mut parent, t[2] as usize);
        parent[c] = a;
    }
    let mut faces: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for v in 0..n {
        let r = find(&mut parent, v);
        faces.entry(r).or_default().push(v);
    }
    struct Rect {
        a0: f64,
        a1: f64,
        y0: f64,
        y1: f64,
        tx: f64,
        tz: f64,
        verts: Vec<usize>,
    }
    let mut planes: std::collections::BTreeMap<(i64, i64), Vec<Rect>> = Default::default();
    for verts in faces.into_values() {
        let v0 = verts[0];
        let [nx, ny, nz] = normals[v0].map(|c| c as f64);
        if ny.abs() > 0.5 {
            continue;
        }
        let len = nx.hypot(nz).max(1e-12);
        let (ux, uz) = (nx / len, nz / len);
        let (tx, tz) = (-uz, ux);
        let (mut a0, mut a1, mut y0, mut y1, mut off) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, 0.0);
        for &v in &verts {
            let [x, y, z] = positions[v];
            let s = x * tx + z * tz;
            a0 = a0.min(s);
            a1 = a1.max(s);
            y0 = y0.min(y);
            y1 = y1.max(y);
            off += x * ux + z * uz;
        }
        off /= verts.len() as f64;
        let angle = (uz.atan2(ux) / (std::f64::consts::PI / 90.0)).round() as i64;
        let key = (angle, (off / 0.25).round() as i64);
        planes.entry(key).or_default().push(Rect { a0, a1, y0, y1, tx, tz, verts });
    }
    let ground = ground.unwrap_or_else(|| planes.values().flatten().map(|r| r.y0).fold(f64::INFINITY, f64::min));
    for list in planes.values() {
        let m = list.len();
        let mut wp: Vec<usize> = (0..m).collect();
        let mut order: Vec<usize> = (0..m).collect();
        order.sort_by(|&i, &j| list[i].a0.partial_cmp(&list[j].a0).unwrap());
        for oi in 0..m {
            let i = order[oi];
            for &j in &order[oi + 1..] {
                if list[j].a0 > list[i].a1 + GAP_M {
                    break;
                }
                if list[j].y0 <= list[i].y1 + GAP_M && list[i].y0 <= list[j].y1 + GAP_M {
                    let (ri, rj) = (find(&mut wp, i), find(&mut wp, j));
                    wp[rj] = ri;
                }
            }
        }
        let mut groups: std::collections::BTreeMap<usize, (f64, f64, f64, f64, Vec<usize>)> = Default::default();
        for i in 0..m {
            let r = find(&mut wp, i);
            let g = groups.entry(r).or_insert((f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY, vec![]));
            let rc = &list[i];
            g.0 = g.0.min(rc.a0);
            g.1 = g.1.max(rc.a1);
            g.2 = g.2.min(rc.y0);
            g.3 = g.3.max(rc.y1);
            g.4.push(i);
        }
        for (a0, a1, y0, y1, members) in groups.into_values() {
            let (w, h) = (a1 - a0, y1 - y0);
            if w < 0.25 || h < 0.25 {
                continue;
            }
            let kind = window_kind(w, h, ground.is_finite().then_some(y0 - ground), use_);
            let qw = ((w * UV_QUANT).round() as i64).clamp(1, 127) as u8;
            let qh = ((h * UV_QUANT).round() as i64).clamp(1, 127) as i8;
            let (w_q, h_q) = (qw as f64 / UV_QUANT, qh as f64 / UV_QUANT);
            let b = qw | if kind == Kind::Shop { 128 } else { 0 };
            let nw = if kind == Kind::Office { -qh } else { qh } as u8;
            for i in members {
                let rc = &list[i];
                for &v in &rc.verts {
                    let [x, y, z] = positions[v];
                    let s = x * rc.tx + z * rc.tz;
                    let fx = ((s - a0) / w_q).clamp(0.0, 1.0);
                    let fy = ((y - y0) / h_q).clamp(0.0, 1.0);
                    out[v] = ([(fx * 255.0).round() as u8, (fy * 255.0).round() as u8, b], nw);
                }
            }
        }
    }
    out
}

/// `kelvinToLinear` (Tanner Helland fit, linearised, max channel 1).
pub fn kelvin_to_linear(kelvin: f64) -> [f64; 3] {
    let t = kelvin / 100.0;
    let c = |v: f64| v.clamp(0.0, 255.0) / 255.0;
    let r = if t <= 66.0 { 1.0 } else { c(329.698727446 * (t - 60.0).powf(-0.1332047592)) };
    let g = if t <= 66.0 { c(99.4708025861 * t.ln() - 161.1195681661) } else { c(288.1221695283 * (t - 60.0).powf(-0.0755148492)) };
    let b = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        c(138.5177312231 * (t - 10.0).ln() - 305.0447927307)
    };
    let lin = |v: f64| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let out = [lin(r), lin(g), lin(b)];
    let m = out[0].max(out[1]).max(out[2]);
    out.map(|v| v / m)
}

fn balanced(k: f64, white: f64) -> [f64; 3] {
    kelvin_to_linear(k).map(|c| c + (1.0 - c) * white)
}

/// The room's WGSL constants (NIGHT_INTERIORS; the balanced lamp colours).
pub fn wgsl_constants() -> String {
    let v3 = |c: [f64; 3]| format!("vec3<f32>({:.6}, {:.6}, {:.6})", c[0], c[1], c[2]);
    let mut s = String::new();
    s.push_str(&format!("const RM_WARM_A: vec3<f32> = {};\n", v3(balanced(2700.0, 0.32))));
    s.push_str(&format!("const RM_WARM_B: vec3<f32> = {};\n", v3(balanced(3300.0, 0.32))));
    s.push_str(&format!("const RM_COOL_A: vec3<f32> = {};\n", v3(balanced(4000.0, 0.32))));
    s.push_str(&format!("const RM_COOL_B: vec3<f32> = {};\n", v3(balanced(5300.0, 0.32))));
    s.push_str(&format!("const RM_SCREEN: vec3<f32> = {};\n", v3(balanced(9000.0, 0.32))));
    s.push_str(&format!("const RM_OFFICE_A: vec3<f32> = {};\n", v3(balanced(5000.0, 0.5))));
    s.push_str(&format!("const RM_OFFICE_B: vec3<f32> = {};\n", v3(balanced(6000.0, 0.5))));
    // (`--look cinematic`, stage L2) the homes' wider spread: a low amber lamp, a cold daylight LED.
    s.push_str(&format!("const RM_CINE_AMBER: vec3<f32> = {};\n", v3(balanced(CINE_AMBER_K, 0.2))));
    s.push_str(&format!("const RM_CINE_DAYLIGHT: vec3<f32> = {};\n", v3(balanced(CINE_DAYLIGHT_K, 0.32))));
    s.push_str(&format!("const RM_CINE_SPREAD: vec2<f32> = vec2<f32>({CINE_AMBER_SHARE:?}, {CINE_DAYLIGHT_SHARE:?});\n"));
    // (`--shop-light`) the shop windows' lights the shop rooms read (rooms.wgsl `rm_shop_light`).
    s.push_str(&crate::shop_lights::wgsl());
    // (`--shop-light`, stage N2) the shops' rooms by trade (shop_rooms.wgsl).
    s.push_str(&crate::shop_lights::rooms_wgsl());
    // the switches of the room fixes (see `RoomFixes`).
    let f = RoomFixes::from_args();
    s.push_str(&format!(
        "const RM_FRAMES: bool = {};\nconst RM_DAY_DEPTH: f32 = {:?};\nconst RM_DAY_THROUGH: f32 = {:?};\nconst RM_BLIND_RUNS: bool = {};\nconst RM_PANE_TV: bool = {};\nconst RM_PANE_TV_SHIFT: u32 = {}u;\nconst RM_PANE_TV_REFERENCE: f32 = {:?};\n",
        f.frames,
        if f.dim { DAY_DEPTH } else { 1.0 },
        if f.dim { DAY_THROUGH } else { 1.0 },
        f.blind_runs,
        f.pane_tv,
        PANE_TV_SHIFT,
        PANE_TV_REFERENCE
    ));
    s
}

/// a window frame that hit the packing's range (`UV_QUANT` 7 bits:
/// 15.875 m) no longer locates its window, so the room seed changed every
/// half metre across it (the block noise in ribbon and curtain-wall glass);
/// such a frame's axis now takes 3.6 m room cells anchored in the world
/// (`--no-room-frames`). By day the room behind the glass is lit by the
/// light that reaches into it, not the full light on the facade: everything
/// past the glass plane takes `DAY_DEPTH` of it, and all of it, the curtains
/// and blinds at the glass too, is seen through the tinted pane
/// (`DAY_THROUGH`) (`--no-room-dim`). An office's blinds go by tenant, a
/// run of 3-7 room cells along a floor at one of a few levels, not cell by
/// cell (the scatter of pale cells over a dark tower) (`--no-room-blind-runs`).
/// What a pane lets through goes by its glass (`pane_transmittance`), so a
/// tinted or reflective tower reads dark and its blinds faint, as through
/// real tinted glass (`--no-pane-transmittance`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomFixes {
    pub frames: bool,
    pub dim: bool,
    pub blind_runs: bool,
    pub pane_tv: bool,
}

pub const DAY_DEPTH: f32 = 0.5;
pub const DAY_THROUGH: f32 = 0.6;
/// The largest frame the packing holds, in its 1/8 m steps (rooms.wgsl reads a full count as "clamped").
pub const FRAME_MAX_Q: u8 = 127;

impl RoomFixes {
    pub fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        Self {
            frames: !args.iter().any(|a| a == "--no-room-frames"),
            dim: !args.iter().any(|a| a == "--no-room-dim"),
            blind_runs: !args.iter().any(|a| a == "--no-room-blind-runs"),
            pane_tv: !args.iter().any(|a| a == "--no-pane-transmittance"),
        }
    }
}

/// `--smooth-time`: the offices' night share comes back to full over this
/// long before 07:00 (`drive_smooth`), where `drive` steps from its night
/// decay (about 0.06) to the full share at 07:00, the hour the JS reopens them.
pub const OFFICE_MORNING_RAMP_H: f64 = 0.5;

// ---- (`--look cinematic`, stage L2) the homes at night under the look ----

/// More homes lit in the evening: the occupancy x this (about 15 % more lit
/// windows at 21 h; the JS profile's 0.62 there, SF's denser evening than the
/// JS default's). INFERRED, set against the reference's "a few lit upper
/// windows in mixed warm tones" (sheet 7.1).
pub const CINE_OCCUPANCY_GAIN: f64 = 1.15;
/// The share of lit homes watching a screen (the default's 0.12).
pub const CINE_TV_SHARE: f64 = 0.18;
/// The homes' colour spread: a share at a low amber lamp, a share under a cold daylight LED.
pub const CINE_AMBER_K: f64 = 2200.0;
pub const CINE_DAYLIGHT_K: f64 = 6500.0;
pub const CINE_AMBER_SHARE: f64 = 0.12;
pub const CINE_DAYLIGHT_SHARE: f64 = 0.08;

/// The rooms' drive the look's uniform takes: `drive` (`drive_smooth` with
/// `smooth`), and under the cinematic look (`cinematic`) the occupancy raised
/// by `CINE_OCCUPANCY_GAIN` (to 0.95), work z the screens' share and w the
/// clock in seconds (the screens' flicker); work z 0 marks the default.
pub fn drive_look(sun_altitude_deg: f64, hour: f64, smooth: bool, cinematic: bool) -> ([f32; 4], [f32; 4]) {
    let (mut env, mut work) = if smooth { drive_smooth(sun_altitude_deg, hour) } else { drive(sun_altitude_deg, hour) };
    if cinematic {
        env[1] = (env[1] as f64 * CINE_OCCUPANCY_GAIN).min(0.95) as f32;
        work[2] = CINE_TV_SHARE as f32;
        work[3] = (hour.rem_euclid(24.0) * 3600.0) as f32;
    }
    (env, work)
}

/// `drive` without the 07:00 step (`--smooth-time`): the office share ramps
/// to its full value over `OFFICE_MORNING_RAMP_H` before it; everything else
/// is `drive`'s.
pub fn drive_smooth(sun_altitude_deg: f64, hour: f64) -> ([f32; 4], [f32; 4]) {
    let (env, mut work) = drive(sun_altitude_deg, hour);
    let dusk = env[0] as f64;
    if dusk > 0.0 {
        let since = (((hour - 18.0) % 24.0) + 24.0) % 24.0;
        if since < 13.0 {
            let t = ((since - (13.0 - OFFICE_MORNING_RAMP_H)) / OFFICE_MORNING_RAMP_H).clamp(0.0, 1.0);
            let decay = (-since / 2.6).exp();
            let decay = decay + (1.0 - decay) * t * t * (3.0 - 2.0 * t);
            work[0] = ((0.06 + (0.62 - 0.06) * decay) * dusk) as f32;
        }
    }
    (env, work)
}

/// `updateNightInteriors(nightPracticalProfile(model))` for a clear, dry
/// model: env = (dusk, occupancy, shop occupancy, cool share), work =
/// (office share, pixel angle).
pub fn drive(sun_altitude_deg: f64, hour: f64) -> ([f32; 4], [f32; 4]) {
    let ss = |a: f64, b: f64, x: f64| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let dusk = 1.0 - ss(2.0, 16.0, sun_altitude_deg);
    let wrap = |x: f64| ((x % 24.0) + 24.0) % 24.0;
    let bell = |centre: f64, width: f64| {
        let mut d = wrap(hour - centre).abs();
        if d > 12.0 {
            d = 24.0 - d;
        }
        (-(d * d) / (2.0 * width * width)).exp()
    };
    let r4 = |v: f64| (v * 10000.0).round() / 10000.0;
    let occupancy = r4((0.08 + 0.56 * bell(20.5, 3.0) + 0.20 * bell(7.0, 1.5)).clamp(0.0, 1.0) * dusk);
    let shop = r4((occupancy * 1.25).clamp(0.0, 0.95));
    let cool = r4((0.18 + 0.42 * bell(21.0, 2.4)).clamp(0.0, 0.7));
    let dusk = r4(dusk);
    // nightInteriorOfficeShare
    let office = if dusk > 0.0 {
        let since = wrap(hour - 18.0);
        let decay = if since < 13.0 { (-since / 2.6).exp() } else { 1.0 };
        (0.06 + (0.62 - 0.06) * decay) * dusk
    } else {
        0.0
    };
    // nightInteriorLitShare('shop'): shopSpill.occupancy, or min(0.95, occ x 1.25) when 0.
    let shop = if shop > 0.0 { shop } else { (occupancy * 1.25).min(0.95) };
    ([dusk as f32, occupancy as f32, shop as f32, cool as f32], [office as f32, 0.00123, 0.0, 0.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_sash_window_is_one_room() {
        // Two 0.6 x 1.4 m sashes 0.1 m apart in the plane z = 0, facing +z.
        let mut p = vec![];
        let mut n = vec![];
        let mut ix = vec![];
        for x0 in [0.0, 0.7] {
            let b = p.len() as u32;
            for (x, y) in [(x0, 3.0), (x0 + 0.6, 3.0), (x0 + 0.6, 4.4), (x0, 4.4)] {
                p.push([x, y, 0.0]);
                n.push([0.0, 0.0, 1.0]);
            }
            ix.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        let out = bake(&p, &n, &ix, Some(0.0), None);
        // t = (-n.z, 0, n.x) = (-1, 0, 0): the window runs along -x.
        let w = (out[0].0[2] & 127) as f64 / UV_QUANT;
        let h = out[0].1 as i8 as f64 / UV_QUANT;
        assert!((w - 1.3).abs() < 0.07 && (h - 1.4).abs() < 0.07, "{w} x {h}");
        assert_eq!(out[1].0[2], out[4].0[2], "both sashes share one window");
        assert_eq!(out[0].0[2] & 128, 0, "a 1.3 m window 3 m up is not a shop");
        assert!(pane_transmittance("glass-reflective").unwrap() < pane_transmittance("glass-dark").unwrap() && pane_transmittance("glass-dark").unwrap() < PANE_TV_REFERENCE);
        assert!(pane_transmittance("glass-shop").is_none());
        assert!(kit_glass_room("glass-shell") == ROOM_AUTO && kit_glass_room("glass-shop-food") == ROOM_SHOP && kit_glass_room("glass-low-iron") == 0);
    }

    #[test]
    fn a_ribbon_past_the_packing_range_is_marked_clamped() {
        // a 22 m ribbon 18 m tall packs the full count on both axes,
        // which rooms.wgsl reads as "no frame" (world-anchored room cells).
        let p = vec![[0.0, 3.0, 0.0], [22.0, 3.0, 0.0], [22.0, 21.0, 0.0], [0.0, 21.0, 0.0]];
        let n = vec![[0.0, 0.0, 1.0]; 4];
        let out = bake(&p, &n, &[0, 1, 2, 0, 2, 3], Some(0.0), Some(Use::Office));
        assert_eq!(out[0].0[2] & 127, FRAME_MAX_Q, "width at the range");
        assert_eq!((out[0].1 as i8).unsigned_abs(), FRAME_MAX_Q, "height at the range");
        // Inside the range a frame locates its window: a 6 m x 3 m window is not clamped.
        let p = vec![[0.0, 3.0, 0.0], [6.0, 3.0, 0.0], [6.0, 6.0, 0.0], [0.0, 6.0, 0.0]];
        let out = bake(&p, &n, &[0, 1, 2, 0, 2, 3], Some(0.0), Some(Use::Office));
        assert!(out[0].0[2] & 127 < FRAME_MAX_Q && (out[0].1 as i8).unsigned_abs() < FRAME_MAX_Q);
        assert!(DAY_DEPTH > 0.0 && DAY_DEPTH < 1.0 && DAY_THROUGH > 0.0 && DAY_THROUGH < 1.0);
    }

    #[test]
    fn the_drive_is_zero_by_day_and_up_at_dusk() {
        let (day, _) = drive(55.0, 12.0);
        assert_eq!(day[0], 0.0);
        let (dusk, work) = drive(-2.0, 19.5);
        assert!(dusk[0] > 0.99 && dusk[1] > 0.3 && work[0] > 0.3);
    }

    /// `--smooth-time`: no step in the office share at 07:00, and `drive`'s values away from the ramp.
    #[test]
    fn the_smooth_drive_has_no_morning_step() {
        let alt = 1.0; // sunrise: the interiors still at their dusk share
        let (_, before) = drive(alt, 7.0 - 1e-6);
        let (_, after) = drive(alt, 7.0);
        assert!((after[0] - before[0]).abs() > 0.3, "drive steps at 07:00 ({} -> {})", before[0], after[0]);
        let mut worst = 0.0f32;
        let mut prev = drive_smooth(alt, 6.0).1[0];
        for k in 1..=7200 {
            let w = drive_smooth(alt, 6.0 + k as f64 / 3600.0).1[0];
            worst = worst.max((w - prev).abs());
            prev = w;
        }
        assert!(worst < 2e-3, "the smooth office share steps by {worst} in a clock second");
        for h in [2.0, 6.4, 7.0, 7.5, 12.0, 18.0, 19.5, 23.0] {
            let (a, b) = (drive(alt, h), drive_smooth(alt, h));
            assert_eq!(a.0, b.0, "{h} h");
            assert!((a.1[0] - b.1[0]).abs() < 1e-6, "{h} h: {} vs {}", a.1[0], b.1[0]);
        }
    }
}
