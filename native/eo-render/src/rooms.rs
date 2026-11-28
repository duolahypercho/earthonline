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
    // (`--office-floors`, `--room-depth`) their sky flags.
    s.push_str(&crate::sky::interiors_wgsl());
    // the switches of the room fixes (see `RoomFixes`).
    let f = RoomFixes::from_args();
    s.push_str(&format!(
        "const RM_FRAMES: bool = {};\nconst RM_DAY_DEPTH: f32 = {:?};\nconst RM_DAY_THROUGH: f32 = {:?};\nconst RM_BLIND_RUNS: bool = {};\nconst RM_PANE_TV: bool = {};\nconst RM_PANE_TV_SHIFT: u32 = {}u;\nconst RM_PANE_TV_REFERENCE: f32 = {:?};\nconst RM_LIT_RUNS: bool = {};\nconst RM_RUN_LIT: f32 = {:?};\nconst RM_LIT_IN_RUN: f32 = {:?};\nconst RM_ATLAS: bool = {};\nconst RM_ATLAS_FORCE: bool = {};\nconst RM_ATLAS_ROOMS: u32 = {}u;\n",
        f.frames,
        if f.dim { DAY_DEPTH } else { 1.0 },
        if f.dim { DAY_THROUGH } else { 1.0 },
        f.blind_runs,
        f.pane_tv,
        PANE_TV_SHIFT,
        PANE_TV_REFERENCE,
        f.lit_runs,
        RUN_LIT,
        LIT_IN_RUN,
        f.atlas,
        f.atlas_force,
        ATLAS_ROOMS
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
/// real tinted glass (`--no-pane-transmittance`). Lit windows come on through
/// civil twilight, not from 16 degrees of sun (`LIT_DUSK_RAMP`), and an office
/// lights by tenant: a run of room cells along a floor is lit or dark
/// together, at the run's own lamp colour and level, instead of a coin per
/// window at one near-white level (the chequerboard of lit and black cells
/// over a tower at dusk); a dark pane keeps its glass's reflection as the
/// light fades (`--no-room-lit-runs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomFixes {
    pub frames: bool,
    pub dim: bool,
    pub blind_runs: bool,
    pub pane_tv: bool,
    pub lit_runs: bool,
    pub atlas: bool,
    pub atlas_force: bool,
}

pub const DAY_DEPTH: f32 = 0.5;
pub const DAY_THROUGH: f32 = 0.6;
/// The largest frame the packing holds, in its 1/8 m steps (rooms.wgsl reads a full count as "clamped").
pub const FRAME_MAX_Q: u8 = 127;

impl RoomFixes {
    pub fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let no_atlas = args.iter().any(|a| a == "--no-room-atlas")
            || matches!(std::env::var("EO_ROOM_ATLAS").as_deref(), Ok("off") | Ok("0"));
        let force_atlas = args.iter().any(|a| a == "--room-atlas")
            || matches!(std::env::var("EO_ROOM_ATLAS").as_deref(), Ok("on") | Ok("1"));
        Self {
            frames: !args.iter().any(|a| a == "--no-room-frames"),
            dim: !args.iter().any(|a| a == "--no-room-dim"),
            blind_runs: !args.iter().any(|a| a == "--no-room-blind-runs"),
            pane_tv: !args.iter().any(|a| a == "--no-pane-transmittance"),
            lit_runs: !args.iter().any(|a| a == "--no-room-lit-runs"),
            atlas: !no_atlas,
            atlas_force: force_atlas && !no_atlas,
        }
    }

    /// `from_args`, read once.
    pub fn get() -> Self {
        static F: std::sync::OnceLock<RoomFixes> = std::sync::OnceLock::new();
        *F.get_or_init(Self::from_args)
    }
}

/// The sun altitudes (degrees) over which the windows' dusk goes from 0 to 1:
/// as ported (`nightInteriors`, from 16 degrees down to 2), and under
/// `--room-lit-runs` from 6 degrees down to -4 (the lamps behind the glass
/// read only once the sunlit street is in shadow and the sky dims; civil
/// twilight ends at -6) (INFERRED).
pub const JS_DUSK_RAMP: (f64, f64) = (2.0, 16.0);
pub const LIT_DUSK_RAMP: (f64, f64) = (-4.0, 6.0);

/// The windows' dusk ramp in use (`LIT_DUSK_RAMP` under `--room-lit-runs`); the
/// shop lights' gather takes it too (sky.rs), so a shop lights its walk and its
/// window together.
pub fn dusk_ramp() -> (f64, f64) {
    if RoomFixes::get().lit_runs { LIT_DUSK_RAMP } else { JS_DUSK_RAMP }
}
/// `--room-lit-runs`: an office floor's tenant run is lit with this chance, and
/// a window in a lit run with this one (rooms.wgsl `RM_RUN_LIT`, `RM_LIT_IN_RUN`) (INFERRED).
pub const RUN_LIT: f32 = 0.7;
pub const LIT_IN_RUN: f32 = 0.94;

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
/// Under `--room-lit-runs` (`RoomFixes::lit_runs`) the dusk goes by `LIT_DUSK_RAMP`.
pub fn drive_look(sun_altitude_deg: f64, hour: f64, smooth: bool, cinematic: bool) -> ([f32; 4], [f32; 4]) {
    let ramp = dusk_ramp();
    let (mut env, mut work) = if smooth { drive_smooth_ramp(sun_altitude_deg, hour, ramp) } else { drive_ramp(sun_altitude_deg, hour, ramp) };
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
    drive_smooth_ramp(sun_altitude_deg, hour, JS_DUSK_RAMP)
}

/// `drive_smooth` with the dusk over `ramp` (sun altitudes, degrees).
pub fn drive_smooth_ramp(sun_altitude_deg: f64, hour: f64, ramp: (f64, f64)) -> ([f32; 4], [f32; 4]) {
    let (env, mut work) = drive_ramp(sun_altitude_deg, hour, ramp);
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
    drive_ramp(sun_altitude_deg, hour, JS_DUSK_RAMP)
}

/// `drive` with the dusk over `ramp` (sun altitudes, degrees: full at the first, none from the second).
pub fn drive_ramp(sun_altitude_deg: f64, hour: f64, ramp: (f64, f64)) -> ([f32; 4], [f32; 4]) {
    let ss = |a: f64, b: f64, x: f64| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let dusk = 1.0 - ss(ramp.0, ramp.1, sun_altitude_deg);
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

// ===========================================================================
// 24-Room High-Detail Interior Parallax Texture Array (`--room-atlas`,
// `--no-room-atlas`; cached in `native/cache/rooms/room-atlas-v1.bin`).
// ===========================================================================

/// Number of distinct synthesized architectural room themes:
///   0..8   Commercial / Office suites
///   8..16  San Francisco Residential (Victorian, Edwardian, modern apartments)
///   16..24 Ground-floor Tower Lobbies & Retail storefronts
pub const ATLAS_ROOMS: u32 = 24;
/// Resolution of each packed face quadrant inside a layer.
pub const ATLAS_FACE: u32 = 256;
/// Full layer resolution (2 x 2 quadrants of `ATLAS_FACE`).
pub const ATLAS_RES: u32 = ATLAS_FACE * 2;
/// Two layers per room: `2*i` = shell albedo+emissive, `2*i+1` = mid-plane card + baked lightmaps.
pub const ATLAS_LAYERS: u32 = ATLAS_ROOMS * 2;
/// Mip levels per layer (`512 -> 256 -> 128 -> 64 -> 32 -> 16`, downsampled per quadrant).
pub const ATLAS_MIPS: u32 = 6;
pub const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const ATLAS_CACHE_MAGIC: &[u8; 8] = b"EORM0001";
pub const ATLAS_CACHE_VERSION: u32 = 1;

/// Synthesized room atlas (all layers and per-quadrant mip levels).
pub struct RoomAtlas {
    /// `[layer][mip]` tightly packed RGBA8 bytes (`(ATLAS_RES >> mip)^2 * 4`).
    pub layers: Vec<Vec<Vec<u8>>>,
    pub from_cache: bool,
    pub ms: f64,
}

impl RoomAtlas {
    pub fn total_bytes(&self) -> u64 {
        self.layers.iter().flatten().map(|m| m.len() as u64).sum()
    }
}

/// Interior room atlas bound at `@group(0) @binding(30)` (`rm_atlas`), or a 1x1 stand-in.
pub struct RoomsGpu {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Bytes allocated on the GPU (0 for the stand-in).
    pub bytes: u64,
    pub layers: u32,
}

static ATLAS_JOB: std::sync::Mutex<Option<std::thread::JoinHandle<RoomAtlas>>> = std::sync::Mutex::new(None);
static ATLAS_DONE: std::sync::OnceLock<std::sync::Arc<RoomAtlas>> = std::sync::OnceLock::new();

/// Start synthesizing or loading the interior room atlas on a worker thread.
pub fn start_atlas() {
    if !RoomFixes::from_args().atlas || ATLAS_DONE.get().is_some() {
        return;
    }
    let mut job = ATLAS_JOB.lock().unwrap();
    if job.is_none() {
        *job = Some(std::thread::spawn(|| {
            crate::hitch::worker_started();
            atlas::load_or_build(Some(&eo_assets::default_cache_dir()))
        }));
    }
}

/// Wait for (or build) the cached interior room atlas.
pub fn built_atlas() -> Option<std::sync::Arc<RoomAtlas>> {
    if !RoomFixes::from_args().atlas {
        return None;
    }
    if let Some(a) = ATLAS_DONE.get() {
        return Some(a.clone());
    }
    let job = ATLAS_JOB.lock().unwrap().take();
    let a = match job {
        Some(h) => h.join().unwrap_or_else(|_| atlas::load_or_build(Some(&eo_assets::default_cache_dir()))),
        None => atlas::load_or_build(Some(&eo_assets::default_cache_dir())),
    };
    Some(ATLAS_DONE.get_or_init(|| std::sync::Arc::new(a)).clone())
}

impl RoomsGpu {
    /// Upload the 24-room interior texture array (or a 1x1 stand-in when `--no-room-atlas`).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        start_atlas();
        match built_atlas() {
            Some(a) => {
                eprintln!(
                    "rooms atlas: {} rooms ({} layers x {} px, {} mips, {:.1} MB) {} in {:.0} ms",
                    ATLAS_ROOMS,
                    ATLAS_LAYERS,
                    ATLAS_RES,
                    ATLAS_MIPS,
                    a.total_bytes() as f64 / (1024.0 * 1024.0),
                    if a.from_cache { "loaded from cache" } else { "synthesized" },
                    a.ms
                );
                Self::upload(device, queue, &a)
            }
            None => Self::stand_in(device, queue),
        }
    }

    pub fn stand_in(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rooms-atlas-stand-in"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ATLAS_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[0, 0, 0, 0],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { texture, view, bytes: 0, layers: 1 }
    }

    fn upload(device: &wgpu::Device, queue: &wgpu::Queue, a: &RoomAtlas) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rooms-atlas"),
            size: wgpu::Extent3d { width: ATLAS_RES, height: ATLAS_RES, depth_or_array_layers: ATLAS_LAYERS },
            mip_level_count: ATLAS_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ATLAS_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, mips) in a.layers.iter().enumerate() {
            for (mip, bytes) in mips.iter().enumerate() {
                let side = ATLAS_RES >> mip;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: mip as u32,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 4), rows_per_image: Some(side) },
                    wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                );
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { texture, view, bytes: a.total_bytes(), layers: ATLAS_LAYERS }
    }
}

/// Deterministic architectural interior atlas renderer + disk cache.
pub mod atlas {
    use super::{ATLAS_CACHE_MAGIC, ATLAS_CACHE_VERSION, ATLAS_FACE, ATLAS_LAYERS, ATLAS_MIPS, ATLAS_RES, ATLAS_ROOMS, RoomAtlas};
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    const F: usize = ATLAS_FACE as usize;
    const R: usize = ATLAS_RES as usize;

    #[inline]
    fn lin_to_srgb(c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
    }

    #[inline]
    fn srgb_to_lin(c: f32) -> f32 {
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    }

    #[inline]
    fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    #[inline]
    fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
        let t = t.clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    }

    #[inline]
    fn mul3(a: [f32; 3], k: f32) -> [f32; 3] {
        [a[0] * k, a[1] * k, a[2] * k]
    }

    #[inline]
    fn hash_u32(mut x: u32) -> u32 {
        x = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let w = ((x >> ((x >> 28).wrapping_add(4))) ^ x).wrapping_mul(277_803_737);
        (w >> 22) ^ w
    }

    #[inline]
    fn hash2(ix: i32, iy: i32, seed: u32) -> f32 {
        let h = hash_u32((ix as u32).wrapping_mul(374_761_393) ^ (iy as u32).wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_654_435_761));
        (h >> 8) as f32 / 16_777_216.0
    }

    #[inline]
    fn vnoise(x: f32, y: f32, seed: u32) -> f32 {
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        let fx = x - ix as f32;
        let fy = y - iy as f32;
        let ux = fx * fx * (3.0 - 2.0 * fx);
        let uy = fy * fy * (3.0 - 2.0 * fy);
        let a = hash2(ix, iy, seed);
        let b = hash2(ix + 1, iy, seed);
        let c = hash2(ix, iy + 1, seed);
        let d = hash2(ix + 1, iy + 1, seed);
        let ab = a + (b - a) * ux;
        let cd = c + (d - c) * ux;
        ab + (cd - ab) * uy
    }

    #[inline]
    fn fbm(x: f32, y: f32, seed: u32) -> f32 {
        vnoise(x, y, seed) * 0.55 + vnoise(x * 2.13 + 7.1, y * 2.07 + 3.4, seed + 17) * 0.30 + vnoise(x * 4.31 + 2.9, y * 4.19 + 9.2, seed + 53) * 0.15
    }

    /// 256x256 linear RGBA canvas for one face quadrant.
    struct Face {
        px: Vec<[f32; 4]>,
    }

    impl Face {
        fn new(fill: [f32; 4]) -> Self {
            Self { px: vec![fill; F * F] }
        }

        #[inline]
        fn blend_px(&mut self, x: usize, y: usize, col: [f32; 3], a_out: f32, cov: f32) {
            if cov <= 1e-4 || x >= F || y >= F {
                return;
            }
            let p = &mut self.px[y * F + x];
            let c = cov.clamp(0.0, 1.0);
            p[0] = p[0] + (col[0] - p[0]) * c;
            p[1] = p[1] + (col[1] - p[1]) * c;
            p[2] = p[2] + (col[2] - p[2]) * c;
            p[3] = p[3] + (a_out - p[3]) * c;
        }

        /// Over-composite RGB without touching alpha (useful for shell faces where alpha = emissive).
        #[inline]
        fn paint_rgb(&mut self, x: usize, y: usize, col: [f32; 3], cov: f32) {
            if cov <= 1e-4 || x >= F || y >= F {
                return;
            }
            let p = &mut self.px[y * F + x];
            let c = cov.clamp(0.0, 1.0);
            p[0] = p[0] + (col[0] - p[0]) * c;
            p[1] = p[1] + (col[1] - p[1]) * c;
            p[2] = p[2] + (col[2] - p[2]) * c;
        }

        /// Fill normalized rectangle `[u0..u1, v0..v1]` with smooth antialiased edges.
        fn rect(&mut self, u0: f32, v0: f32, u1: f32, v1: f32, col: [f32; 3], a: Option<f32>) {
            let px = 1.0 / F as f32;
            let x0 = ((u0 - px).max(0.0) * F as f32).floor() as usize;
            let x1 = ((u1 + px).min(1.0) * F as f32).ceil() as usize;
            let y0 = ((v0 - px).max(0.0) * F as f32).floor() as usize;
            let y1 = ((v1 + px).min(1.0) * F as f32).ceil() as usize;
            for y in y0..y1.min(F) {
                let v = (y as f32 + 0.5) * px;
                let cy = smoothstep(v0 - px, v0 + px, v) * (1.0 - smoothstep(v1 - px, v1 + px, v));
                if cy <= 0.0 {
                    continue;
                }
                for x in x0..x1.min(F) {
                    let u = (x as f32 + 0.5) * px;
                    let cx = smoothstep(u0 - px, u0 + px, u) * (1.0 - smoothstep(u1 - px, u1 + px, u));
                    let cov = cx * cy;
                    if let Some(av) = a {
                        self.blend_px(x, y, col, av, cov);
                    } else {
                        self.paint_rgb(x, y, col, cov);
                    }
                }
            }
        }

        /// Fill rectangle with vertical gradient top->bottom.
        fn rect_grad_v(&mut self, u0: f32, v0: f32, u1: f32, v1: f32, top: [f32; 3], bot: [f32; 3], a: Option<f32>) {
            let px = 1.0 / F as f32;
            let x0 = ((u0 - px).max(0.0) * F as f32).floor() as usize;
            let x1 = ((u1 + px).min(1.0) * F as f32).ceil() as usize;
            let y0 = ((v0 - px).max(0.0) * F as f32).floor() as usize;
            let y1 = ((v1 + px).min(1.0) * F as f32).ceil() as usize;
            let dv = (v1 - v0).max(1e-4);
            for y in y0..y1.min(F) {
                let v = (y as f32 + 0.5) * px;
                let cy = smoothstep(v0 - px, v0 + px, v) * (1.0 - smoothstep(v1 - px, v1 + px, v));
                if cy <= 0.0 {
                    continue;
                }
                let t = ((v - v0) / dv).clamp(0.0, 1.0);
                let col = mix3(top, bot, t);
                for x in x0..x1.min(F) {
                    let u = (x as f32 + 0.5) * px;
                    let cx = smoothstep(u0 - px, u0 + px, u) * (1.0 - smoothstep(u1 - px, u1 + px, u));
                    let cov = cx * cy;
                    if let Some(av) = a {
                        self.blend_px(x, y, col, av, cov);
                    } else {
                        self.paint_rgb(x, y, col, cov);
                    }
                }
            }
        }

        /// Soft drop shadow (multiplicative darkening in RGB).
        fn shadow_rect(&mut self, u0: f32, v0: f32, u1: f32, v1: f32, blur: f32, strength: f32) {
            let px = 1.0 / F as f32;
            let b = blur.max(px);
            let x0 = ((u0 - b).max(0.0) * F as f32).floor() as usize;
            let x1 = ((u1 + b).min(1.0) * F as f32).ceil() as usize;
            let y0 = ((v0 - b).max(0.0) * F as f32).floor() as usize;
            let y1 = ((v1 + b).min(1.0) * F as f32).ceil() as usize;
            for y in y0..y1.min(F) {
                let v = (y as f32 + 0.5) * px;
                let dy = (v0 - v).max(0.0).max(v - v1);
                for x in x0..x1.min(F) {
                    let u = (x as f32 + 0.5) * px;
                    let dx = (u0 - u).max(0.0).max(u - u1);
                    let d = (dx * dx + dy * dy).sqrt();
                    let k = (1.0 - smoothstep(0.0, b, d)) * strength;
                    if k > 1e-4 {
                        let p = &mut self.px[y * F + x];
                        let s = 1.0 - k;
                        p[0] *= s;
                        p[1] *= s;
                        p[2] *= s;
                    }
                }
            }
        }

        /// Filled ellipse (soft edge).
        fn ellipse(&mut self, cu: f32, cv: f32, ru: f32, rv: f32, col: [f32; 3], a: Option<f32>, softness: f32) {
            let px = 1.0 / F as f32;
            let pad = softness.max(px);
            let x0 = ((cu - ru - pad).max(0.0) * F as f32).floor() as usize;
            let x1 = ((cu + ru + pad).min(1.0) * F as f32).ceil() as usize;
            let y0 = ((cv - rv - pad).max(0.0) * F as f32).floor() as usize;
            let y1 = ((cv + rv + pad).min(1.0) * F as f32).ceil() as usize;
            let edge = (softness / ru.min(rv).max(px)).clamp(px * 2.0, 0.99);
            for y in y0..y1.min(F) {
                let v = (y as f32 + 0.5) * px;
                let dy = (v - cv) / rv.max(1e-4);
                for x in x0..x1.min(F) {
                    let u = (x as f32 + 0.5) * px;
                    let dx = (u - cu) / ru.max(1e-4);
                    let r = (dx * dx + dy * dy).sqrt();
                    let cov = 1.0 - smoothstep(1.0 - edge, 1.0 + edge * 0.25, r);
                    if cov > 1e-4 {
                        if let Some(av) = a {
                            self.blend_px(x, y, col, av, cov);
                        } else {
                            self.paint_rgb(x, y, col, cov);
                        }
                    }
                }
            }
        }

        /// Additive radial glow (for lightmaps and emissive halos).
        fn add_glow(&mut self, cu: f32, cv: f32, ru: f32, rv: f32, col: [f32; 3], alpha_add: f32) {
            let px = 1.0 / F as f32;
            let x0 = ((cu - ru * 2.2).max(0.0) * F as f32).floor() as usize;
            let x1 = ((cu + ru * 2.2).min(1.0) * F as f32).ceil() as usize;
            let y0 = ((cv - rv * 2.2).max(0.0) * F as f32).floor() as usize;
            let y1 = ((cv + rv * 2.2).min(1.0) * F as f32).ceil() as usize;
            for y in y0..y1.min(F) {
                let v = (y as f32 + 0.5) * px;
                let dy = (v - cv) / rv.max(1e-4);
                for x in x0..x1.min(F) {
                    let u = (x as f32 + 0.5) * px;
                    let dx = (u - cu) / ru.max(1e-4);
                    let w = (-(dx * dx + dy * dy)).exp();
                    if w > 1e-4 {
                        let p = &mut self.px[y * F + x];
                        p[0] += col[0] * w;
                        p[1] += col[1] * w;
                        p[2] += col[2] * w;
                        p[3] = (p[3] + alpha_add * w).clamp(0.0, 1.0);
                    }
                }
            }
        }

        /// Bake soft room corner ambient occlusion along all four perimeter edges.
        fn apply_corner_ao(&mut self, edge_w: f32, strength: f32) {
            let px = 1.0 / F as f32;
            for y in 0..F {
                let v = (y as f32 + 0.5) * px;
                let vy = smoothstep(0.0, edge_w, v) * smoothstep(0.0, edge_w, 1.0 - v);
                for x in 0..F {
                    let u = (x as f32 + 0.5) * px;
                    let vx = smoothstep(0.0, edge_w, u) * smoothstep(0.0, edge_w, 1.0 - u);
                    let ao = 1.0 - strength * (1.0 - vx * vy);
                    let p = &mut self.px[y * F + x];
                    p[0] *= ao;
                    p[1] *= ao;
                    p[2] *= ao;
                }
            }
        }
    }

    // ---- Architectural surface fills ----------------------------------------

    fn fill_plaster(f: &mut Face, base: [f32; 3], seed: u32, amp: f32) {
        let px = 1.0 / F as f32;
        for y in 0..F {
            let v = (y as f32 + 0.5) * px;
            for x in 0..F {
                let u = (x as f32 + 0.5) * px;
                let n = fbm(u * 6.0, v * 6.0, seed) - 0.5;
                let fine = vnoise(u * 28.0, v * 28.0, seed + 91) - 0.5;
                // Subtle height shade: slightly darker near floor, gently brighter near ceiling bounce
                let grad = 0.94 + 0.08 * (1.0 - v);
                let k = (grad + n * amp + fine * amp * 0.35).clamp(0.2, 1.3);
                f.px[y * F + x] = [base[0] * k, base[1] * k, base[2] * k, 0.0];
            }
        }
    }

    fn fill_brick(f: &mut Face, seed: u32) {
        let px = 1.0 / F as f32;
        let rows = 26.0f32;
        let cols = 9.0f32;
        for y in 0..F {
            let v = (y as f32 + 0.5) * px;
            let row = (v * rows).floor() as i32;
            let rv = (v * rows).fract();
            let shift = if row & 1 != 0 { 0.5 } else { 0.0 };
            for x in 0..F {
                let u = (x as f32 + 0.5) * px;
                let cu = u * cols + shift;
                let col = cu.floor() as i32;
                let ru = cu.fract();
                let mortar = (rv < 0.13 || ru < 0.055) as u8;
                let h = hash2(col, row, seed);
                let tex = fbm(u * 24.0, v * 24.0, seed + 31);
                let c = if mortar != 0 {
                    mul3([0.36, 0.33, 0.29], 0.85 + 0.25 * tex)
                } else {
                    let b = mix3([0.24, 0.10, 0.065], [0.38, 0.17, 0.11], h);
                    mul3(b, 0.8 + 0.35 * tex)
                };
                f.px[y * F + x] = [c[0], c[1], c[2], 0.0];
            }
        }
    }

    fn fill_marble(f: &mut Face, base: [f32; 3], vein: [f32; 3], seed: u32, panels_x: f32, panels_y: f32) {
        let px = 1.0 / F as f32;
        for y in 0..F {
            let v = (y as f32 + 0.5) * px;
            let py = (v * panels_y).floor() as i32;
            let fy = (v * panels_y).fract();
            for x in 0..F {
                let u = (x as f32 + 0.5) * px;
                let px_i = (u * panels_x).floor() as i32;
                let fx = (u * panels_x).fract();
                // Bookmatch odd panels horizontally for authentic architectural stone slab look
                let bu = if px_i & 1 != 0 { 1.0 - fx } else { fx };
                let warp = fbm(bu * 4.5 + px_i as f32 * 0.3, fy * 4.5 + py as f32 * 0.7, seed);
                let wave = ((bu * 5.0 + fy * 3.0 + warp * 4.8) * std::f32::consts::PI).sin().abs();
                let v_str = (1.0 - wave).powf(4.5) * 0.72 + warp * 0.18;
                let slab_tint = 0.94 + 0.12 * hash2(px_i, py, seed + 7);
                let mut c = mul3(mix3(base, vein, v_str), slab_tint);
                // Crisp dark architectural reveal joints between marble slabs
                let joint = fy < 0.018 || fy > 0.985 || fx < 0.015 || fx > 0.988;
                if joint {
                    c = mul3(c, 0.42);
                }
                f.px[y * F + x] = [c[0], c[1], c[2], 0.0];
            }
        }
    }

    fn fill_wood_slats(f: &mut Face, u0: f32, v0: f32, u1: f32, v1: f32, wood_a: [f32; 3], wood_b: [f32; 3], slats: f32, seed: u32) {
        let px = 1.0 / F as f32;
        let x0 = (u0 * F as f32).max(0.0) as usize;
        let x1 = (u1 * F as f32).min(F as f32) as usize;
        let y0 = (v0 * F as f32).max(0.0) as usize;
        let y1 = (v1 * F as f32).min(F as f32) as usize;
        for y in y0..y1 {
            let v = (y as f32 + 0.5) * px;
            for x in x0..x1 {
                let u = (x as f32 + 0.5) * px;
                let su = (u - u0) / (u1 - u0).max(1e-4) * slats;
                let si = su.floor() as i32;
                let sf = su.fract();
                let c = if sf < 0.26 {
                    [0.028, 0.022, 0.018] // dark acoustic felt recess behind slats
                } else {
                    let tone = hash2(si, 1, seed);
                    let grain = vnoise(sf * 3.0, v * 28.0 + tone * 9.0, seed + 5);
                    let bevel = 0.84 + 0.24 * smoothstep(0.26, 0.55, sf) * (1.0 - 0.18 * smoothstep(0.82, 1.0, sf));
                    mul3(mix3(wood_a, wood_b, tone), bevel * (0.9 + 0.2 * grain))
                };
                f.paint_rgb(x, y, c, 1.0);
            }
        }
    }

    fn fill_wood_floor(f: &mut Face, wood_a: [f32; 3], wood_b: [f32; 3], seed: u32) {
        let px = 1.0 / F as f32;
        let planks = 18.0f32;
        for y in 0..F {
            let v = (y as f32 + 0.5) * px;
            for x in 0..F {
                let u = (x as f32 + 0.5) * px;
                let p = u * planks;
                let pi = p.floor() as i32;
                let pf = p.fract();
                let stagger = hash2(pi, 0, seed) * 2.5;
                let row = (v * 4.0 + stagger).floor() as i32;
                let rf = (v * 4.0 + stagger).fract();
                let tone = hash2(pi, row, seed + 13);
                let grain = vnoise(pf * 4.0, v * 36.0 + tone * 11.0, seed + 29);
                let seam = if pf < 0.06 || rf < 0.02 { 0.45 } else { 1.0 };
                let c = mul3(mix3(wood_a, wood_b, tone), (0.86 + 0.26 * grain) * seam);
                f.px[y * F + x] = [c[0], c[1], c[2], 0.35];
            }
        }
    }

    fn fill_carpet_tiles(f: &mut Face, col_a: [f32; 3], col_b: [f32; 3], seed: u32) {
        let px = 1.0 / F as f32;
        let n = 8.0f32;
        for y in 0..F {
            let v = (y as f32 + 0.5) * px;
            let ty = (v * n).floor() as i32;
            let fy = (v * n).fract();
            for x in 0..F {
                let u = (x as f32 + 0.5) * px;
                let tx = (u * n).floor() as i32;
                let fx = (u * n).fract();
                let chk = ((tx + ty) & 1) as f32;
                let weave = if chk > 0.5 { vnoise(u * 64.0, v * 16.0, seed) } else { vnoise(u * 16.0, v * 64.0, seed) };
                let seam = if fx < 0.03 || fy < 0.03 { 0.82 } else { 1.0 };
                let base = mix3(col_a, col_b, chk * 0.65 + 0.35 * hash2(tx, ty, seed + 3));
                let c = mul3(base, (0.88 + 0.22 * weave) * seam);
                f.px[y * F + x] = [c[0], c[1], c[2], 0.22];
            }
        }
    }

    fn draw_rug(f: &mut Face, u0: f32, v0: f32, u1: f32, v1: f32, primary: [f32; 3], border: [f32; 3], accent: [f32; 3], seed: u32) {
        f.shadow_rect(u0, v0, u1, v1, 0.025, 0.35);
        let px = 1.0 / F as f32;
        let x0 = (u0 * F as f32).max(0.0) as usize;
        let x1 = (u1 * F as f32).min(F as f32) as usize;
        let y0 = (v0 * F as f32).max(0.0) as usize;
        let y1 = (v1 * F as f32).min(F as f32) as usize;
        for y in y0..y1 {
            let lv = ((y as f32 + 0.5) * px - v0) / (v1 - v0).max(1e-4);
            for x in x0..x1 {
                let lu = ((x as f32 + 0.5) * px - u0) / (u1 - u0).max(1e-4);
                let d_edge = lu.min(1.0 - lu).min(lv).min(1.0 - lv);
                let weave = 0.88 + 0.22 * vnoise(lu * 48.0, lv * 48.0, seed);
                let pat = ((lu * 10.0).sin() * (lv * 10.0).sin()).abs();
                let col = if d_edge < 0.09 {
                    border
                } else if d_edge < 0.13 {
                    accent
                } else if ((lu - 0.5).abs() + (lv - 0.5).abs()) < 0.22 {
                    mix3(accent, primary, 0.45)
                } else {
                    mix3(primary, border, smoothstep(0.6, 0.85, pat) * 0.45)
                };
                f.paint_rgb(x, y, mul3(col, weave), 1.0);
            }
        }
    }

    fn draw_wainscot_and_trim(f: &mut Face, trim_col: [f32; 3], dado_v: f32, panels: usize) {
        // Crown molding at top
        f.rect_grad_v(0.0, 0.0, 1.0, 0.042, mul3(trim_col, 1.06), mul3(trim_col, 0.82), None);
        f.rect(0.0, 0.042, 1.0, 0.052, mul3(trim_col, 0.55), None);
        // Chair rail & paneled wainscoting
        f.shadow_rect(0.0, dado_v, 1.0, dado_v + 0.022, 0.015, 0.28);
        f.rect_grad_v(0.0, dado_v - 0.016, 1.0, dado_v + 0.012, mul3(trim_col, 1.08), mul3(trim_col, 0.78), None);
        f.rect(0.0, dado_v + 0.012, 1.0, 0.935, mul3(trim_col, 0.92), None);
        let pw = 1.0 / panels.max(1) as f32;
        for i in 0..panels {
            let p0 = i as f32 * pw + 0.025;
            let p1 = (i + 1) as f32 * pw - 0.025;
            let pv0 = dado_v + 0.045;
            let pv1 = 0.89;
            f.rect(p0, pv0, p1, pv1, mul3(trim_col, 0.72), None);
            f.rect(p0 + 0.012, pv0 + 0.014, p1 - 0.012, pv1 - 0.014, mul3(trim_col, 0.96), None);
        }
        // Baseboard
        f.rect_grad_v(0.0, 0.925, 1.0, 1.0, mul3(trim_col, 0.96), mul3(trim_col, 0.65), None);
    }

    fn draw_framed_art(f: &mut Face, u0: f32, v0: f32, u1: f32, v1: f32, frame_col: [f32; 3], seed: u32) {
        f.shadow_rect(u0 + 0.008, v0 + 0.012, u1 + 0.012, v1 + 0.016, 0.022, 0.45);
        f.rect(u0, v0, u1, v1, frame_col, None);
        let m = 0.014;
        let i0 = u0 + m;
        let j0 = v0 + m;
        let i1 = u1 - m;
        let j1 = v1 - m;
        // Pale archival mat border
        f.rect(i0, j0, i1, j1, [0.80, 0.77, 0.71], None);
        let a0 = i0 + 0.016;
        let b0 = j0 + 0.016;
        let a1 = i1 - 0.016;
        let b1 = j1 - 0.016;
        let h1 = hash2(seed as i32, 1, 77);
        let h2 = hash2(seed as i32, 2, 77);
        let palettes: [([f32; 3], [f32; 3], [f32; 3]); 5] = [
            ([0.14, 0.24, 0.38], [0.74, 0.48, 0.24], [0.86, 0.78, 0.62]), // coastal golden sunset
            ([0.18, 0.28, 0.22], [0.46, 0.36, 0.24], [0.78, 0.74, 0.64]), // landscape sage/ochre
            ([0.52, 0.14, 0.12], [0.10, 0.14, 0.22], [0.85, 0.72, 0.34]), // bold modern abstract
            ([0.22, 0.26, 0.30], [0.58, 0.55, 0.49], [0.09, 0.11, 0.14]), // architectural charcoal
            ([0.12, 0.32, 0.38], [0.78, 0.38, 0.22], [0.90, 0.85, 0.74]), // mid-century colorblock
        ];
        let (c0, c1, c2) = palettes[(seed as usize) % palettes.len()];
        let px = 1.0 / F as f32;
        let x0 = (a0 * F as f32).max(0.0) as usize;
        let x1 = (a1 * F as f32).min(F as f32) as usize;
        let y0 = (b0 * F as f32).max(0.0) as usize;
        let y1 = (b1 * F as f32).min(F as f32) as usize;
        for y in y0..y1 {
            let lv = ((y as f32 + 0.5) * px - b0) / (b1 - b0).max(1e-4);
            for x in x0..x1 {
                let lu = ((x as f32 + 0.5) * px - a0) / (a1 - a0).max(1e-4);
                let brush = fbm(lu * 7.0 + h1 * 5.0, lv * 7.0 + h2 * 5.0, seed + 19);
                let col = if seed % 2 == 0 {
                    // Horizon + hills + glowing accent
                    let split = 0.52 + 0.12 * (lu * 4.0 + h1 * 6.0).sin();
                    let base = if lv < split { mix3(c0, c2, lv / split) } else { mix3(c1, mul3(c0, 0.5), (lv - split) / (1.0 - split).max(0.1)) };
                    mul3(base, 0.85 + 0.3 * brush)
                } else {
                    // Abstract geometric + painterly field
                    let d = ((lu - 0.35 - h1 * 0.3).powi(2) + (lv - 0.45).powi(2)).sqrt();
                    let base = if d < 0.24 { c2 } else if lu + lv * 0.4 > 0.72 { c1 } else { c0 };
                    mul3(base, 0.86 + 0.28 * brush)
                };
                f.paint_rgb(x, y, col, 1.0);
            }
        }
    }

    fn draw_bookshelf(f: &mut Face, u0: f32, v0: f32, u1: f32, v1: f32, wood: [f32; 3], rows: usize, seed: u32, backlit: bool) {
        f.shadow_rect(u0, v0, u1, v1, 0.025, 0.45);
        // Outer carcass & dark recess
        f.rect(u0, v0, u1, v1, wood, None);
        let pad = 0.016;
        f.rect(u0 + pad, v0 + pad, u1 - pad, v1 - pad, mul3(wood, 0.24), None);
        let book_cols: [[f32; 3]; 10] = [
            [0.48, 0.12, 0.10],
            [0.12, 0.22, 0.38],
            [0.14, 0.30, 0.20],
            [0.72, 0.58, 0.26],
            [0.76, 0.72, 0.64],
            [0.24, 0.22, 0.26],
            [0.54, 0.28, 0.16],
            [0.22, 0.36, 0.42],
            [0.64, 0.46, 0.34],
            [0.36, 0.16, 0.26],
        ];
        let inner_h = (v1 - v0 - 2.0 * pad).max(0.05);
        let row_h = inner_h / rows.max(1) as f32;
        for r in 0..rows {
            let sy0 = v0 + pad + r as f32 * row_h;
            let sy1 = sy0 + row_h;
            if backlit {
                // Warm LED strip glowing under top of each shelf bay
                f.rect(u0 + pad + 0.006, sy0 + 0.006, u1 - pad - 0.006, sy0 + 0.020, [0.95, 0.80, 0.52], Some(0.75));
            }
            // Pack varied individual books across the shelf
            let mut u = u0 + pad + 0.008;
            let mut b = 0i32;
            while u < u1 - pad - 0.014 {
                let bw = 0.011 + 0.015 * hash2(b, r as i32, seed + 11);
                let gap = hash2(b, r as i32, seed + 23);
                if gap > 0.14 {
                    let bh = row_h * (0.52 + 0.32 * hash2(b, r as i32, seed + 37));
                    let bc = book_cols[(hash_u32((b as u32) ^ (r as u32 * 31) ^ seed) as usize) % book_cols.len()];
                    let bu1 = (u + bw).min(u1 - pad - 0.006);
                    f.rect_grad_v(u, sy1 - bh - 0.014, bu1, sy1 - 0.014, mul3(bc, 1.08), mul3(bc, 0.76), None);
                    // Thin spine highlight band
                    f.rect(u + 0.002, sy1 - bh * 0.7 - 0.014, bu1 - 0.002, sy1 - bh * 0.62 - 0.014, mul3(bc, 1.35), None);
                }
                u += bw + 0.003;
                b += 1;
            }
            // Shelf board + cast shadow beneath it
            f.shadow_rect(u0 + pad, sy1 - 0.010, u1 - pad, sy1 + 0.016, 0.012, 0.42);
            f.rect_grad_v(u0 + pad * 0.5, sy1 - 0.014, u1 - pad * 0.5, sy1, mul3(wood, 1.12), mul3(wood, 0.68), None);
        }
    }

    fn draw_acoustic_ceiling(f: &mut Face, style: u32, seed: u32) {
        let px = 1.0 / F as f32;
        let tile = 8.0f32;
        for y in 0..F {
            let v = (y as f32 + 0.5) * px;
            let fy = (v * tile).fract();
            for x in 0..F {
                let u = (x as f32 + 0.5) * px;
                let fx = (u * tile).fract();
                let tbar = fx < 0.045 || fy < 0.045;
                let grain = 0.94 + 0.10 * vnoise(u * 42.0, v * 42.0, seed);
                let base = if tbar { [0.46, 0.47, 0.48] } else { mul3([0.78, 0.78, 0.76], grain) };
                let fix = match style {
                    0 => {
                        // Continuous architectural recessed linear LED bars
                        let on_x = (u > 0.14 && u < 0.86) && ((v - 0.25).abs() < 0.032 || (v - 0.68).abs() < 0.032);
                        if on_x { 1.0 } else { 0.0 }
                    }
                    1 => {
                        // 2x4 recessed office troffers in 2x2 grid
                        let gx = (u - 0.27).abs() < 0.13 || (u - 0.73).abs() < 0.13;
                        let gy = (v - 0.28).abs() < 0.075 || (v - 0.72).abs() < 0.075;
                        if gx && gy { 0.95 } else { 0.0 }
                    }
                    2 => {
                        // Dense trading floor 2x2 LED panel grid
                        let gx = (u * 3.0).fract();
                        let gy = (v * 3.0).fract();
                        if gx > 0.24 && gx < 0.76 && gy > 0.28 && gy < 0.72 { 0.95 } else { 0.0 }
                    }
                    _ => {
                        // Perimeter cove + center recessed downlights
                        let d1 = ((u - 0.30).powi(2) + (v - 0.42).powi(2)).sqrt();
                        let d2 = ((u - 0.70).powi(2) + (v - 0.42).powi(2)).sqrt();
                        let d3 = ((u - 0.50).powi(2) + (v - 0.74).powi(2)).sqrt();
                        let spot = (1.0 - smoothstep(0.035, 0.055, d1.min(d2).min(d3))) * 1.0;
                        let cove = if v < 0.06 || v > 0.94 { 0.55 } else { 0.0 };
                        spot.max(cove)
                    }
                };
                let c = mix3(base, [1.0, 0.99, 0.96], fix);
                f.px[y * F + x] = [c[0], c[1], c[2], fix];
            }
        }
        f.apply_corner_ao(0.08, 0.28);
    }

    fn draw_plaster_ceiling(f: &mut Face, col: [f32; 3], spots: &[(f32, f32, f32)], coffer: bool, cove: bool, seed: u32) {
        fill_plaster(f, col, seed, 0.07);
        if coffer {
            f.rect(0.08, 0.08, 0.92, 0.92, mul3(col, 0.80), None);
            f.rect(0.12, 0.12, 0.88, 0.88, mul3(col, 1.04), None);
            f.rect(0.48, 0.12, 0.52, 0.88, mul3(col, 0.82), None);
        }
        if cove {
            // Warm recessed architectural perimeter cove strip
            f.rect(0.09, 0.09, 0.91, 0.125, [1.0, 0.88, 0.68], Some(0.85));
            f.rect(0.09, 0.875, 0.91, 0.91, [1.0, 0.88, 0.68], Some(0.85));
            f.rect(0.09, 0.09, 0.125, 0.91, [1.0, 0.88, 0.68], Some(0.75));
            f.rect(0.875, 0.09, 0.91, 0.91, [1.0, 0.88, 0.68], Some(0.75));
        }
        for &(cu, cv, r) in spots {
            f.ellipse(cu, cv, r * 1.4, r * 1.4, mul3(col, 0.35), None, 0.008);
            f.ellipse(cu, cv, r, r, [1.0, 0.96, 0.86], Some(1.0), 0.01);
        }
        f.apply_corner_ao(0.09, 0.32);
    }

    /// Draw a stylized indoor ficus / monstera potted plant on a canvas (works on mid-plane or wall).
    fn draw_plant(f: &mut Face, cu: f32, base_v: f32, scale: f32, pot_col: [f32; 3], alpha: Option<f32>, seed: u32) {
        let pw = 0.055 * scale;
        let ph = 0.11 * scale;
        // Ceramic planter pot
        f.rect_grad_v(cu - pw, base_v - ph, cu + pw, base_v, mul3(pot_col, 1.12), mul3(pot_col, 0.65), alpha);
        f.rect(cu - pw * 1.08, base_v - ph, cu + pw * 1.08, base_v - ph + 0.018 * scale, mul3(pot_col, 1.18), alpha);
        // Stems & layered dark/bright botanical fronds
        for i in 0..9 {
            let fi = i as f32;
            let ang = -1.15 + fi * 0.28 + (hash2(i, 1, seed) - 0.5) * 0.18;
            let reach = (0.07 + 0.09 * hash2(i, 2, seed)) * scale;
            let lu = cu + ang.sin() * reach;
            let lv = base_v - ph - (0.04 + 0.16 * hash2(i, 3, seed)) * scale;
            let leaf_col = mix3([0.045, 0.14, 0.055], [0.11, 0.26, 0.09], hash2(i, 4, seed));
            f.ellipse(lu, lv, 0.032 * scale, 0.022 * scale, leaf_col, alpha, 0.008);
        }
    }

    /// Draw a workstation desk with chair back + dual glowing monitors on `mid` & `mid_emit`.
    fn draw_workstation(mid: &mut Face, emit: &mut Face, u0: f32, u1: f32, desk_v: f32, wood: [f32; 3], lit_monitors: bool, seed: u32) {
        let w = u1 - u0;
        // Desk legs & modesty panel
        mid.rect(u0 + 0.015, desk_v + 0.022, u0 + 0.035, 1.0, [0.10, 0.11, 0.12], Some(1.0));
        mid.rect(u1 - 0.035, desk_v + 0.022, u1 - 0.015, 1.0, [0.10, 0.11, 0.12], Some(1.0));
        mid.rect_grad_v(u0 + 0.03, desk_v + 0.024, u1 - 0.03, desk_v + 0.15, mul3(wood, 0.55), mul3(wood, 0.35), Some(1.0));
        // Tabletop edge
        mid.rect_grad_v(u0, desk_v, u1, desk_v + 0.024, mul3(wood, 1.15), mul3(wood, 0.72), Some(1.0));
        // Dual monitors on stands
        let mw = (w * 0.34).min(0.15);
        let mh = 0.095;
        for m in 0..2 {
            let mu0 = u0 + w * (0.14 + m as f32 * 0.39);
            let mu1 = mu0 + mw;
            let mv0 = desk_v - mh - 0.028;
            let mv1 = desk_v - 0.028;
            let cx = 0.5 * (mu0 + mu1);
            mid.rect(cx - 0.010, mv1, cx + 0.010, desk_v, [0.08, 0.085, 0.09], Some(1.0));
            mid.rect(mu0, mv0, mu1, mv1, [0.035, 0.04, 0.045], Some(1.0));
            if lit_monitors && hash2(m, 3, seed) > 0.22 {
                // Realistic cool-blue/white workspace UI content (not a solid neon rectangle)
                let ui = mix3([0.28, 0.42, 0.62], [0.58, 0.64, 0.72], hash2(m, 7, seed));
                mid.rect(mu0 + 0.008, mv0 + 0.008, mu1 - 0.008, mv1 - 0.008, mul3(ui, 0.38), Some(1.0));
                emit.rect_grad_v(mu0 + 0.008, mv0 + 0.008, mu1 - 0.008, mv1 - 0.008, mul3(ui, 0.85), mul3(ui, 0.45), Some(0.70));
                // Sidebar & code/chart lines
                emit.rect(mu0 + 0.014, mv0 + 0.014, mu0 + 0.036, mv1 - 0.014, mul3(ui, 0.25), Some(0.35));
            }
        }
        // Ergonomic chair back silhouette
        let ch_u = u0 + w * 0.52;
        mid.ellipse(ch_u, desk_v + 0.015, 0.055, 0.095, [0.055, 0.06, 0.068], Some(1.0), 0.006);
        mid.rect(ch_u - 0.014, desk_v + 0.09, ch_u + 0.014, 1.0, [0.07, 0.075, 0.08], Some(1.0));
    }

    /// Draw a hanging pendant fixture (cord + shade + glowing bulb/aperture) on `mid` & `emit`.
    fn draw_pendant(mid: &mut Face, emit: &mut Face, cu: f32, drop_v: f32, w: f32, h: f32, shade_col: [f32; 3], lamp_col: [f32; 3], warm_globe: bool) {
        // Suspension cord from ceiling
        mid.rect(cu - 0.005, 0.0, cu + 0.005, drop_v - h * 0.4, [0.04, 0.04, 0.045], Some(1.0));
        if warm_globe {
            // Translucent warm paper/glass globe
            mid.ellipse(cu, drop_v, w * 0.5, h * 0.55, mul3(lamp_col, 0.85), Some(0.95), 0.008);
            emit.ellipse(cu, drop_v, w * 0.48, h * 0.52, lamp_col, Some(0.95), 0.015);
            emit.add_glow(cu, drop_v, w * 0.9, h * 0.9, mul3(lamp_col, 0.45), 0.35);
        } else {
            // Architectural metal dome/drum shade + glowing lower aperture
            mid.ellipse(cu, drop_v - h * 0.1, w * 0.52, h * 0.45, shade_col, Some(1.0), 0.006);
            mid.rect(cu - w * 0.52, drop_v - h * 0.1, cu + w * 0.52, drop_v + h * 0.25, shade_col, Some(1.0));
            mid.ellipse(cu, drop_v + h * 0.28, w * 0.44, h * 0.14, [0.98, 0.92, 0.78], Some(1.0), 0.005);
            emit.ellipse(cu, drop_v + h * 0.28, w * 0.46, h * 0.18, lamp_col, Some(1.0), 0.010);
            emit.add_glow(cu, drop_v + h * 0.36, w * 0.85, h * 0.45, mul3(lamp_col, 0.40), 0.30);
        }
    }

    /// Synthesize one complete architectural room (`id` in `0..24`), returning the two 512x512 layers (with mip chains).
    fn synthesize_room(id: u32) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let seed = id.wrapping_mul(1013).wrapping_add(137);
        let mut back = Face::new([0.5, 0.5, 0.5, 0.0]);
        let mut side = Face::new([0.5, 0.5, 0.5, 0.0]);
        let mut ceil = Face::new([0.75, 0.75, 0.75, 0.0]);
        let mut floor = Face::new([0.2, 0.2, 0.2, 0.25]);
        let mut mid = Face::new([0.0, 0.0, 0.0, 0.0]);
        let mut mid_emit = Face::new([0.0, 0.0, 0.0, 0.0]);
        let mut lm_back: Face;
        let mut lm_side: Face;

        match id {
            // ===============================================================
            // 0..8: COMMERCIAL & OFFICE SUITES
            // ===============================================================
            0 => {
                // Modern open-plan SF tech office: cool-neutral linear LEDs, whiteboards, dual monitors, glass column
                fill_plaster(&mut back, [0.68, 0.69, 0.70], seed, 0.06);
                fill_plaster(&mut side, [0.60, 0.62, 0.65], seed + 1, 0.06);
                draw_acoustic_ceiling(&mut ceil, 0, seed);
                fill_carpet_tiles(&mut floor, [0.13, 0.14, 0.17], [0.17, 0.18, 0.21], seed);
                // Glass whiteboard & acoustic felt pinboard on back wall
                back.shadow_rect(0.10, 0.24, 0.54, 0.56, 0.02, 0.25);
                back.rect(0.10, 0.24, 0.54, 0.56, [0.82, 0.85, 0.87], None);
                back.rect(0.13, 0.30, 0.34, 0.315, [0.22, 0.38, 0.58], None);
                back.rect(0.13, 0.35, 0.45, 0.362, [0.42, 0.45, 0.48], None);
                back.rect(0.13, 0.41, 0.38, 0.422, [0.62, 0.28, 0.22], None);
                back.rect(0.60, 0.22, 0.90, 0.58, [0.24, 0.28, 0.32], None);
                back.rect(0.64, 0.27, 0.75, 0.41, [0.85, 0.82, 0.75], None);
                back.rect(0.77, 0.29, 0.87, 0.45, [0.78, 0.84, 0.86], None);
                // Low credenza along back wall
                back.rect_grad_v(0.05, 0.70, 0.95, 0.96, [0.56, 0.44, 0.30], [0.36, 0.27, 0.18], None);
                // Side wall glass office door + frame
                side.rect(0.32, 0.18, 0.68, 0.98, [0.10, 0.11, 0.12], None);
                side.rect(0.35, 0.21, 0.65, 0.96, [0.32, 0.38, 0.42], Some(0.28));
                // Mid-plane: two desks with glowing dual monitors + structural column + ficus
                draw_workstation(&mut mid, &mut mid_emit, 0.04, 0.46, 0.68, [0.66, 0.56, 0.42], true, seed);
                draw_workstation(&mut mid, &mut mid_emit, 0.52, 0.92, 0.68, [0.66, 0.56, 0.42], true, seed + 5);
                mid.rect_grad_v(0.91, 0.0, 1.0, 1.0, [0.38, 0.39, 0.41], [0.24, 0.25, 0.27], Some(1.0));
                draw_plant(&mut mid, 0.49, 0.98, 0.85, [0.76, 0.74, 0.70], Some(1.0), seed);
                // Baked 4300K office lighting
                lm_back = Face::new([0.36, 0.38, 0.40, 1.0]);
                lm_side = Face::new([0.30, 0.32, 0.34, 1.0]);
                lm_back.add_glow(0.30, 0.22, 0.34, 0.48, [0.58, 0.62, 0.66], 0.0);
                lm_back.add_glow(0.72, 0.22, 0.34, 0.48, [0.58, 0.62, 0.66], 0.0);
                lm_side.add_glow(0.50, 0.24, 0.40, 0.52, [0.52, 0.56, 0.60], 0.0);
            }
            1 => {
                // Corporate perimeter office: warm wood paneling, 2x4 troffers, bookshelves, glass partition
                fill_plaster(&mut back, [0.72, 0.69, 0.63], seed, 0.06);
                fill_plaster(&mut side, [0.64, 0.61, 0.56], seed + 1, 0.06);
                draw_acoustic_ceiling(&mut ceil, 1, seed);
                fill_carpet_tiles(&mut floor, [0.16, 0.15, 0.14], [0.20, 0.19, 0.17], seed);
                draw_bookshelf(&mut back, 0.06, 0.24, 0.38, 0.96, [0.26, 0.16, 0.10], 5, seed, false);
                draw_framed_art(&mut back, 0.48, 0.22, 0.86, 0.56, [0.14, 0.10, 0.07], seed);
                back.rect_grad_v(0.42, 0.68, 0.94, 0.97, [0.28, 0.17, 0.10], [0.18, 0.10, 0.06], None);
                draw_framed_art(&mut side, 0.25, 0.24, 0.72, 0.56, [0.14, 0.10, 0.07], seed + 3);
                // Mid-plane: executive desk + glass conference partition on right
                draw_workstation(&mut mid, &mut mid_emit, 0.12, 0.62, 0.66, [0.34, 0.20, 0.12], true, seed + 11);
                // Translucent glass partition wall with dark aluminum mullions
                mid.rect(0.70, 0.0, 1.0, 1.0, [0.32, 0.38, 0.40], Some(0.24));
                mid.rect(0.70, 0.42, 1.0, 0.58, [0.76, 0.79, 0.80], Some(0.38)); // frosted privacy band
                mid.rect(0.695, 0.0, 0.715, 1.0, [0.08, 0.085, 0.09], Some(1.0));
                mid.rect(0.70, 0.0, 1.0, 0.03, [0.08, 0.085, 0.09], Some(1.0));
                lm_back = Face::new([0.34, 0.32, 0.28, 1.0]);
                lm_side = Face::new([0.28, 0.26, 0.23, 1.0]);
                lm_back.add_glow(0.27, 0.24, 0.32, 0.46, [0.64, 0.58, 0.48], 0.0);
                lm_back.add_glow(0.72, 0.24, 0.32, 0.46, [0.64, 0.58, 0.48], 0.0);
                lm_side.add_glow(0.48, 0.26, 0.36, 0.48, [0.56, 0.50, 0.42], 0.0);
            }
            2 => {
                // Glass conference room: walnut wood-slat accent wall, large display screen, long table, linear pendant
                fill_plaster(&mut back, [0.64, 0.63, 0.60], seed, 0.05);
                fill_wood_slats(&mut back, 0.06, 0.05, 0.94, 0.96, [0.34, 0.22, 0.13], [0.26, 0.16, 0.09], 36.0, seed);
                // Large mounted conference display with subtle chart glow
                back.shadow_rect(0.22, 0.24, 0.78, 0.58, 0.025, 0.55);
                back.rect(0.22, 0.24, 0.78, 0.58, [0.04, 0.045, 0.05], None);
                back.rect_grad_v(0.24, 0.26, 0.76, 0.56, [0.16, 0.26, 0.36], [0.09, 0.14, 0.22], Some(0.55));
                fill_plaster(&mut side, [0.58, 0.60, 0.62], seed + 2, 0.05);
                // Side glass wall mullions
                side.rect(0.0, 0.0, 1.0, 0.04, [0.07, 0.075, 0.08], None);
                side.rect(0.48, 0.0, 0.51, 1.0, [0.07, 0.075, 0.08], None);
                draw_acoustic_ceiling(&mut ceil, 0, seed + 2);
                fill_carpet_tiles(&mut floor, [0.12, 0.13, 0.15], [0.15, 0.16, 0.18], seed);
                // Mid-plane: suspended linear pendant bar + long conference table + chairs
                mid.rect(0.22, 0.0, 0.228, 0.22, [0.06, 0.06, 0.06], Some(1.0));
                mid.rect(0.772, 0.0, 0.78, 0.22, [0.06, 0.06, 0.06], Some(1.0));
                mid.rect(0.18, 0.21, 0.82, 0.245, [0.12, 0.12, 0.13], Some(1.0));
                mid_emit.rect(0.19, 0.232, 0.81, 0.252, [1.0, 0.94, 0.82], Some(1.0));
                mid_emit.add_glow(0.50, 0.26, 0.42, 0.12, [0.65, 0.60, 0.48], 0.35);
                // Conference table & chairs
                for i in 0..4 {
                    let cu = 0.22 + i as f32 * 0.185;
                    mid.ellipse(cu, 0.68, 0.052, 0.088, [0.08, 0.09, 0.10], Some(1.0), 0.006);
                }
                mid.rect_grad_v(0.10, 0.69, 0.90, 0.73, [0.46, 0.32, 0.20], [0.28, 0.18, 0.11], Some(1.0));
                mid.rect(0.22, 0.73, 0.28, 1.0, [0.08, 0.085, 0.09], Some(1.0));
                mid.rect(0.72, 0.73, 0.78, 1.0, [0.08, 0.085, 0.09], Some(1.0));
                lm_back = Face::new([0.28, 0.26, 0.22, 1.0]);
                lm_side = Face::new([0.26, 0.25, 0.22, 1.0]);
                lm_back.add_glow(0.50, 0.32, 0.48, 0.42, [0.76, 0.68, 0.54], 0.0);
                lm_side.add_glow(0.50, 0.34, 0.46, 0.48, [0.62, 0.56, 0.46], 0.0);
            }
            3 => {
                // Executive corner suite: warm walnut bookshelves, leather lounge chairs, warm 3000K downlights
                fill_plaster(&mut back, [0.66, 0.60, 0.52], seed, 0.08);
                fill_plaster(&mut side, [0.58, 0.52, 0.45], seed + 3, 0.08);
                draw_plaster_ceiling(&mut ceil, [0.76, 0.74, 0.70], &[(0.28, 0.35, 0.042), (0.72, 0.35, 0.042), (0.50, 0.72, 0.042)], true, false, seed);
                fill_wood_floor(&mut floor, [0.24, 0.14, 0.08], [0.32, 0.20, 0.11], seed);
                draw_rug(&mut floor, 0.16, 0.20, 0.84, 0.84, [0.22, 0.25, 0.30], [0.58, 0.50, 0.38], [0.46, 0.24, 0.16], seed);
                draw_bookshelf(&mut back, 0.04, 0.10, 0.42, 0.96, [0.20, 0.11, 0.06], 6, seed + 7, true);
                draw_framed_art(&mut back, 0.50, 0.20, 0.90, 0.58, [0.55, 0.42, 0.20], seed + 1);
                back.rect_grad_v(0.48, 0.68, 0.94, 0.96, [0.24, 0.14, 0.08], [0.15, 0.08, 0.04], None);
                draw_wainscot_and_trim(&mut side, [0.22, 0.13, 0.07], 0.62, 3);
                // Mid-plane: warm desk lamp + leather armchairs + plant
                mid.rect_grad_v(0.18, 0.66, 0.72, 0.71, [0.28, 0.16, 0.09], [0.18, 0.10, 0.05], Some(1.0));
                mid.rect(0.22, 0.71, 0.68, 1.0, [0.14, 0.08, 0.05], Some(1.0));
                // Glowing warm brass desk lamp on right of desk
                mid.rect(0.61, 0.57, 0.625, 0.66, [0.55, 0.42, 0.18], Some(1.0));
                mid.ellipse(0.618, 0.54, 0.052, 0.036, [0.95, 0.80, 0.50], Some(1.0), 0.006);
                mid_emit.ellipse(0.618, 0.54, 0.052, 0.038, [1.0, 0.78, 0.42], Some(0.95), 0.012);
                mid_emit.add_glow(0.618, 0.56, 0.14, 0.14, [0.85, 0.58, 0.24], 0.35);
                draw_plant(&mut mid, 0.86, 0.98, 1.05, [0.20, 0.18, 0.17], Some(1.0), seed + 9);
                lm_back = Face::new([0.22, 0.18, 0.13, 1.0]);
                lm_side = Face::new([0.18, 0.15, 0.11, 1.0]);
                lm_back.add_glow(0.24, 0.32, 0.30, 0.45, [0.82, 0.60, 0.34], 0.0);
                lm_back.add_glow(0.70, 0.34, 0.34, 0.42, [0.88, 0.66, 0.38], 0.0);
                lm_side.add_glow(0.50, 0.36, 0.42, 0.45, [0.72, 0.52, 0.30], 0.0);
            }
            4 => {
                // SoMa creative design studio loft: exposed brick wall, concrete ceiling, industrial pendants, pinup boards
                fill_brick(&mut back, seed);
                fill_brick(&mut side, seed + 5);
                fill_plaster(&mut ceil, [0.42, 0.41, 0.40], seed + 7, 0.12);
                // Exposed galvanized HVAC duct across loft ceiling
                ceil.rect_grad_v(0.0, 0.38, 1.0, 0.54, [0.56, 0.58, 0.60], [0.34, 0.36, 0.38], None);
                ceil.ellipse(0.28, 0.72, 0.055, 0.055, [1.0, 0.90, 0.70], Some(1.0), 0.01);
                ceil.ellipse(0.72, 0.72, 0.055, 0.055, [1.0, 0.90, 0.70], Some(1.0), 0.01);
                fill_wood_floor(&mut floor, [0.36, 0.24, 0.14], [0.46, 0.32, 0.19], seed + 2);
                // Architectural pinup drawings on brick wall
                back.shadow_rect(0.14, 0.22, 0.86, 0.58, 0.02, 0.35);
                back.rect(0.14, 0.22, 0.46, 0.56, [0.86, 0.84, 0.79], None);
                back.rect(0.18, 0.28, 0.42, 0.48, [0.22, 0.32, 0.42], None);
                back.rect(0.52, 0.24, 0.86, 0.56, [0.88, 0.86, 0.82], None);
                back.rect(0.56, 0.30, 0.82, 0.42, [0.72, 0.38, 0.22], None);
                // Mid-plane: two industrial enamel pendants + tall drafting workstations + hanging fern
                draw_pendant(&mut mid, &mut mid_emit, 0.28, 0.26, 0.14, 0.09, [0.10, 0.11, 0.12], [1.0, 0.86, 0.58], false);
                draw_pendant(&mut mid, &mut mid_emit, 0.72, 0.26, 0.14, 0.09, [0.10, 0.11, 0.12], [1.0, 0.86, 0.58], false);
                draw_workstation(&mut mid, &mut mid_emit, 0.08, 0.48, 0.66, [0.58, 0.44, 0.28], true, seed + 17);
                draw_workstation(&mut mid, &mut mid_emit, 0.52, 0.92, 0.66, [0.58, 0.44, 0.28], true, seed + 19);
                lm_back = Face::new([0.24, 0.20, 0.16, 1.0]);
                lm_side = Face::new([0.20, 0.17, 0.14, 1.0]);
                lm_back.add_glow(0.28, 0.36, 0.34, 0.42, [0.85, 0.68, 0.44], 0.0);
                lm_back.add_glow(0.72, 0.36, 0.34, 0.42, [0.85, 0.68, 0.44], 0.0);
                lm_side.add_glow(0.45, 0.40, 0.38, 0.46, [0.70, 0.56, 0.38], 0.0);
            }
            5 => {
                // High-density FiDi finance / trading floor: crisp 4800K 2x2 LED grid, steel column, multi-monitor rows
                fill_plaster(&mut back, [0.64, 0.66, 0.69], seed, 0.04);
                fill_plaster(&mut side, [0.56, 0.59, 0.62], seed + 4, 0.05);
                draw_acoustic_ceiling(&mut ceil, 2, seed + 1);
                fill_carpet_tiles(&mut floor, [0.10, 0.12, 0.15], [0.14, 0.16, 0.20], seed);
                // Glass interior office doors across back core
                for i in 0..3 {
                    let bx = 0.06 + i as f32 * 0.31;
                    back.rect(bx, 0.14, bx + 0.27, 0.96, [0.10, 0.11, 0.13], None);
                    back.rect_grad_v(bx + 0.02, 0.17, bx + 0.25, 0.94, [0.34, 0.40, 0.46], [0.18, 0.22, 0.26], Some(0.25));
                }
                // Mid-plane: dense row of workstations + central structural column
                draw_workstation(&mut mid, &mut mid_emit, 0.02, 0.44, 0.67, [0.72, 0.72, 0.70], true, seed + 21);
                draw_workstation(&mut mid, &mut mid_emit, 0.56, 0.98, 0.67, [0.72, 0.72, 0.70], true, seed + 23);
                mid.rect_grad_v(0.45, 0.0, 0.55, 1.0, [0.58, 0.59, 0.61], [0.36, 0.38, 0.40], Some(1.0));
                lm_back = Face::new([0.42, 0.45, 0.48, 1.0]);
                lm_side = Face::new([0.36, 0.39, 0.42, 1.0]);
                lm_back.add_glow(0.25, 0.20, 0.36, 0.48, [0.54, 0.60, 0.68], 0.0);
                lm_back.add_glow(0.75, 0.20, 0.36, 0.48, [0.54, 0.60, 0.68], 0.0);
                lm_side.add_glow(0.50, 0.22, 0.42, 0.52, [0.50, 0.56, 0.64], 0.0);
            }
            6 => {
                // Late evening quiet office: overhead dimmed, glowing monitors + one warm task lamp + glowing hallway door
                fill_plaster(&mut back, [0.56, 0.57, 0.59], seed, 0.06);
                fill_plaster(&mut side, [0.50, 0.51, 0.54], seed + 6, 0.06);
                draw_acoustic_ceiling(&mut ceil, 3, seed + 3);
                fill_carpet_tiles(&mut floor, [0.11, 0.12, 0.14], [0.14, 0.15, 0.17], seed);
                // Bright hallway glass sidelight on back left (classic night office silhouette)
                back.rect(0.08, 0.14, 0.32, 0.96, [0.09, 0.095, 0.10], None);
                back.rect_grad_v(0.10, 0.17, 0.30, 0.94, [0.88, 0.80, 0.62], [0.55, 0.48, 0.36], Some(0.85));
                draw_bookshelf(&mut back, 0.40, 0.34, 0.92, 0.96, [0.22, 0.20, 0.18], 4, seed + 29, false);
                draw_workstation(&mut mid, &mut mid_emit, 0.38, 0.88, 0.66, [0.42, 0.36, 0.30], true, seed + 31);
                // Warm task lamp on left corner of desk
                mid.ellipse(0.42, 0.55, 0.042, 0.028, [0.95, 0.82, 0.52], Some(1.0), 0.005);
                mid_emit.ellipse(0.42, 0.55, 0.045, 0.030, [1.0, 0.82, 0.48], Some(0.95), 0.012);
                mid_emit.add_glow(0.42, 0.58, 0.16, 0.16, [0.80, 0.58, 0.28], 0.35);
                lm_back = Face::new([0.11, 0.12, 0.15, 1.0]);
                lm_side = Face::new([0.10, 0.11, 0.14, 1.0]);
                lm_back.add_glow(0.20, 0.52, 0.28, 0.45, [0.85, 0.72, 0.46], 0.0);
                lm_back.add_glow(0.62, 0.62, 0.26, 0.32, [0.32, 0.46, 0.68], 0.0);
                lm_side.add_glow(0.32, 0.48, 0.34, 0.40, [0.52, 0.44, 0.30], 0.0);
            }
            7 => {
                // Architectural office break lounge: kitchenette cabinets, pendant Edison bulbs, wood slat wall, sofa
                fill_plaster(&mut back, [0.74, 0.72, 0.68], seed, 0.06);
                fill_wood_slats(&mut side, 0.0, 0.0, 1.0, 1.0, [0.42, 0.29, 0.17], [0.34, 0.22, 0.12], 28.0, seed);
                draw_plaster_ceiling(&mut ceil, [0.78, 0.76, 0.73], &[(0.25, 0.40, 0.036), (0.50, 0.40, 0.036), (0.75, 0.40, 0.036)], false, true, seed);
                fill_wood_floor(&mut floor, [0.42, 0.30, 0.18], [0.52, 0.38, 0.24], seed + 4);
                // Kitchenette upper & lower cabinets + tile backsplash + under-cabinet LED
                back.rect(0.06, 0.14, 0.66, 0.42, [0.22, 0.25, 0.28], None);
                back.rect(0.06, 0.42, 0.66, 0.442, [1.0, 0.88, 0.64], Some(0.92));
                back.rect(0.06, 0.442, 0.66, 0.62, [0.78, 0.77, 0.74], None);
                back.rect(0.06, 0.62, 0.66, 0.97, [0.18, 0.20, 0.23], None);
                // Stainless fridge on right
                back.rect_grad_v(0.70, 0.18, 0.94, 0.97, [0.58, 0.60, 0.62], [0.42, 0.44, 0.46], None);
                // Mid-plane: 3 warm globe pendants + island counter + plant
                draw_pendant(&mut mid, &mut mid_emit, 0.24, 0.28, 0.085, 0.085, [0.95, 0.86, 0.65], [1.0, 0.84, 0.52], true);
                draw_pendant(&mut mid, &mut mid_emit, 0.50, 0.28, 0.085, 0.085, [0.95, 0.86, 0.65], [1.0, 0.84, 0.52], true);
                draw_pendant(&mut mid, &mut mid_emit, 0.76, 0.28, 0.085, 0.085, [0.95, 0.86, 0.65], [1.0, 0.84, 0.52], true);
                mid.rect_grad_v(0.14, 0.66, 0.78, 0.70, [0.80, 0.78, 0.74], [0.62, 0.60, 0.56], Some(1.0));
                mid.rect(0.16, 0.70, 0.76, 1.0, [0.34, 0.23, 0.14], Some(1.0));
                draw_plant(&mut mid, 0.88, 0.98, 0.95, [0.74, 0.42, 0.28], Some(1.0), seed + 12);
                lm_back = Face::new([0.26, 0.22, 0.17, 1.0]);
                lm_side = Face::new([0.24, 0.20, 0.15, 1.0]);
                lm_back.add_glow(0.36, 0.48, 0.38, 0.34, [0.92, 0.72, 0.42], 0.0);
                lm_back.add_glow(0.50, 0.28, 0.44, 0.38, [0.78, 0.62, 0.38], 0.0);
                lm_side.add_glow(0.50, 0.34, 0.42, 0.44, [0.74, 0.58, 0.36], 0.0);
            }

            // ===============================================================
            // 8..16: SAN FRANCISCO RESIDENTIAL (VICTORIAN, EDWARDIAN, APTS)
            // ===============================================================
            8 => {
                // Classic Victorian parlor: sage plaster + cream wainscoting, tall bookcase, gilt oil art, brass chandelier, Persian rug
                fill_plaster(&mut back, [0.54, 0.60, 0.52], seed, 0.08);
                fill_plaster(&mut side, [0.48, 0.54, 0.47], seed + 1, 0.08);
                draw_wainscot_and_trim(&mut back, [0.82, 0.79, 0.72], 0.64, 4);
                draw_wainscot_and_trim(&mut side, [0.80, 0.77, 0.70], 0.64, 3);
                draw_plaster_ceiling(&mut ceil, [0.85, 0.83, 0.78], &[(0.50, 0.48, 0.055)], true, false, seed);
                fill_wood_floor(&mut floor, [0.30, 0.17, 0.09], [0.40, 0.24, 0.12], seed);
                draw_rug(&mut floor, 0.14, 0.16, 0.86, 0.86, [0.46, 0.13, 0.12], [0.16, 0.22, 0.32], [0.78, 0.62, 0.30], seed);
                draw_bookshelf(&mut back, 0.05, 0.16, 0.35, 0.94, [0.22, 0.12, 0.07], 5, seed + 3, false);
                draw_framed_art(&mut back, 0.46, 0.18, 0.86, 0.54, [0.64, 0.50, 0.20], seed + 2);
                draw_framed_art(&mut side, 0.28, 0.20, 0.72, 0.54, [0.64, 0.50, 0.20], seed + 5);
                // Mid-plane: brass chandelier + tufted velvet sofa + glowing warm floor lamp
                draw_pendant(&mut mid, &mut mid_emit, 0.50, 0.22, 0.18, 0.10, [0.62, 0.48, 0.20], [1.0, 0.82, 0.48], true);
                mid.rect_grad_v(0.24, 0.68, 0.76, 0.98, [0.34, 0.16, 0.16], [0.20, 0.09, 0.09], Some(1.0));
                mid.ellipse(0.32, 0.72, 0.06, 0.05, [0.76, 0.64, 0.44], Some(1.0), 0.008); // throw pillow
                // Warm linen floor lamp on right
                mid.rect(0.835, 0.48, 0.848, 0.98, [0.46, 0.36, 0.18], Some(1.0));
                mid.rect_grad_v(0.78, 0.36, 0.90, 0.49, [0.96, 0.84, 0.56], [0.88, 0.72, 0.42], Some(1.0));
                mid_emit.rect_grad_v(0.78, 0.36, 0.90, 0.49, [1.0, 0.80, 0.42], [0.95, 0.68, 0.32], Some(0.95));
                mid_emit.add_glow(0.84, 0.43, 0.22, 0.22, [0.90, 0.62, 0.26], 0.35);
                lm_back = Face::new([0.22, 0.17, 0.11, 1.0]);
                lm_side = Face::new([0.20, 0.15, 0.10, 1.0]);
                lm_back.add_glow(0.50, 0.26, 0.42, 0.42, [0.74, 0.54, 0.30], 0.0);
                lm_back.add_glow(0.84, 0.44, 0.32, 0.38, [0.92, 0.66, 0.32], 0.0);
                lm_side.add_glow(0.55, 0.42, 0.44, 0.44, [0.78, 0.56, 0.28], 0.0);
            }
            9 => {
                // Edwardian living room: warm cream plaster, fireplace mantel + arched mirror, warm oak floor, sofa
                fill_plaster(&mut back, [0.78, 0.74, 0.65], seed, 0.08);
                fill_plaster(&mut side, [0.72, 0.68, 0.60], seed + 2, 0.08);
                draw_wainscot_and_trim(&mut side, [0.84, 0.82, 0.77], 0.70, 3);
                draw_plaster_ceiling(&mut ceil, [0.86, 0.85, 0.82], &[(0.50, 0.45, 0.050)], false, false, seed);
                fill_wood_floor(&mut floor, [0.38, 0.24, 0.13], [0.48, 0.32, 0.18], seed + 1);
                draw_rug(&mut floor, 0.18, 0.22, 0.82, 0.82, [0.68, 0.64, 0.56], [0.30, 0.34, 0.38], [0.64, 0.46, 0.28], seed);
                // Classical fireplace mantel + dark hearth + mirror over mantel
                back.shadow_rect(0.28, 0.56, 0.72, 0.98, 0.025, 0.35);
                back.rect(0.26, 0.54, 0.74, 0.59, [0.84, 0.82, 0.78], None);
                back.rect(0.30, 0.59, 0.70, 0.98, [0.82, 0.80, 0.76], None);
                back.rect(0.39, 0.67, 0.61, 0.96, [0.06, 0.055, 0.05], None); // hearth
                back.ellipse(0.50, 0.34, 0.14, 0.17, [0.58, 0.45, 0.20], None, 0.008); // gilt mirror frame
                back.ellipse(0.50, 0.34, 0.12, 0.15, [0.44, 0.46, 0.48], None, 0.006); // mirror glass
                draw_bookshelf(&mut back, 0.04, 0.28, 0.24, 0.96, [0.76, 0.74, 0.70], 4, seed + 8, false);
                draw_bookshelf(&mut back, 0.76, 0.28, 0.96, 0.96, [0.76, 0.74, 0.70], 4, seed + 9, false);
                // Mid-plane: table lamp on left side table + sofa back + monstera plant
                mid.rect(0.08, 0.68, 0.24, 0.98, [0.34, 0.22, 0.13], Some(1.0));
                mid.rect(0.11, 0.52, 0.21, 0.63, [0.95, 0.88, 0.70], Some(1.0));
                mid_emit.rect(0.11, 0.52, 0.21, 0.63, [1.0, 0.82, 0.48], Some(0.95));
                mid_emit.add_glow(0.16, 0.57, 0.20, 0.20, [0.88, 0.64, 0.30], 0.35);
                mid.rect_grad_v(0.30, 0.72, 0.82, 0.98, [0.24, 0.30, 0.36], [0.14, 0.18, 0.22], Some(1.0));
                draw_plant(&mut mid, 0.88, 0.98, 0.95, [0.78, 0.76, 0.72], Some(1.0), seed + 4);
                lm_back = Face::new([0.24, 0.20, 0.14, 1.0]);
                lm_side = Face::new([0.22, 0.18, 0.13, 1.0]);
                lm_back.add_glow(0.18, 0.56, 0.34, 0.38, [0.92, 0.68, 0.34], 0.0);
                lm_back.add_glow(0.50, 0.24, 0.44, 0.42, [0.68, 0.54, 0.34], 0.0);
                lm_side.add_glow(0.42, 0.46, 0.42, 0.44, [0.74, 0.56, 0.32], 0.0);
            }
            10 => {
                // Modern SF apartment living room: gallery wall, warm oak floor, pendant, monstera, credenza
                fill_plaster(&mut back, [0.82, 0.80, 0.76], seed, 0.05);
                fill_plaster(&mut side, [0.76, 0.75, 0.72], seed + 3, 0.05);
                draw_plaster_ceiling(&mut ceil, [0.86, 0.86, 0.84], &[(0.32, 0.40, 0.035), (0.68, 0.40, 0.035)], false, false, seed);
                fill_wood_floor(&mut floor, [0.46, 0.34, 0.21], [0.56, 0.42, 0.27], seed + 2);
                draw_rug(&mut floor, 0.16, 0.18, 0.84, 0.84, [0.76, 0.73, 0.68], [0.56, 0.52, 0.46], [0.22, 0.24, 0.26], seed);
                // Gallery wall of 3 framed artworks
                draw_framed_art(&mut back, 0.12, 0.18, 0.38, 0.50, [0.10, 0.10, 0.10], seed + 1);
                draw_framed_art(&mut back, 0.42, 0.16, 0.66, 0.48, [0.46, 0.34, 0.20], seed + 4);
                draw_framed_art(&mut back, 0.70, 0.20, 0.90, 0.52, [0.10, 0.10, 0.10], seed + 7);
                // Mid-century credenza
                back.rect_grad_v(0.14, 0.66, 0.86, 0.90, [0.44, 0.28, 0.15], [0.30, 0.18, 0.09], None);
                back.rect(0.18, 0.90, 0.21, 0.98, [0.18, 0.12, 0.07], None);
                back.rect(0.79, 0.90, 0.82, 0.98, [0.18, 0.12, 0.07], None);
                draw_bookshelf(&mut side, 0.20, 0.24, 0.78, 0.96, [0.46, 0.32, 0.19], 4, seed + 10, false);
                // Mid-plane: sculptural paper pendant + sofa + large monstera
                draw_pendant(&mut mid, &mut mid_emit, 0.48, 0.24, 0.15, 0.13, [0.96, 0.92, 0.82], [1.0, 0.88, 0.64], true);
                mid.rect_grad_v(0.18, 0.70, 0.72, 0.98, [0.64, 0.58, 0.48], [0.44, 0.40, 0.33], Some(1.0));
                draw_plant(&mut mid, 0.82, 0.98, 1.15, [0.76, 0.44, 0.30], Some(1.0), seed + 6);
                lm_back = Face::new([0.26, 0.23, 0.18, 1.0]);
                lm_side = Face::new([0.24, 0.21, 0.16, 1.0]);
                lm_back.add_glow(0.48, 0.28, 0.48, 0.44, [0.84, 0.70, 0.46], 0.0);
                lm_side.add_glow(0.48, 0.34, 0.44, 0.48, [0.74, 0.62, 0.40], 0.0);
            }
            11 => {
                // Warm kitchen & dining: shaker cabinets, subway tile backsplash, glowing under-cabinet LED, brass pendant over dining table
                fill_plaster(&mut back, [0.82, 0.80, 0.76], seed, 0.05);
                fill_plaster(&mut side, [0.76, 0.74, 0.70], seed + 4, 0.05);
                draw_plaster_ceiling(&mut ceil, [0.86, 0.85, 0.82], &[(0.30, 0.36, 0.036), (0.70, 0.36, 0.036)], false, false, seed);
                fill_wood_floor(&mut floor, [0.42, 0.29, 0.17], [0.50, 0.36, 0.22], seed + 3);
                // Upper shaker cabinets (warm white/sage)
                back.rect(0.04, 0.08, 0.96, 0.40, [0.76, 0.76, 0.72], None);
                for i in 0..5 {
                    let c0 = 0.055 + i as f32 * 0.18;
                    back.rect(c0, 0.10, c0 + 0.165, 0.38, [0.68, 0.68, 0.64], None);
                    back.rect(c0 + 0.015, 0.12, c0 + 0.15, 0.36, [0.80, 0.80, 0.76], None);
                }
                // Warm under-cabinet LED strip!
                back.rect(0.05, 0.40, 0.95, 0.422, [1.0, 0.88, 0.60], Some(0.95));
                // Subway tile backsplash + stone counter + lower cabinets
                back.rect(0.04, 0.422, 0.96, 0.62, [0.82, 0.81, 0.78], None);
                back.rect(0.04, 0.62, 0.96, 0.65, [0.22, 0.23, 0.24], None);
                back.rect(0.04, 0.65, 0.96, 0.98, [0.20, 0.28, 0.26], None);
                draw_framed_art(&mut side, 0.30, 0.24, 0.70, 0.56, [0.36, 0.24, 0.14], seed + 8);
                // Mid-plane: brass dome pendant over warm oak dining table + chairs
                draw_pendant(&mut mid, &mut mid_emit, 0.50, 0.28, 0.18, 0.10, [0.64, 0.48, 0.20], [1.0, 0.84, 0.50], false);
                mid.rect_grad_v(0.18, 0.70, 0.82, 0.74, [0.48, 0.32, 0.18], [0.32, 0.20, 0.10], Some(1.0));
                mid.rect(0.24, 0.74, 0.28, 1.0, [0.28, 0.18, 0.10], Some(1.0));
                mid.rect(0.72, 0.74, 0.76, 1.0, [0.28, 0.18, 0.10], Some(1.0));
                lm_back = Face::new([0.26, 0.22, 0.16, 1.0]);
                lm_side = Face::new([0.24, 0.20, 0.15, 1.0]);
                lm_back.add_glow(0.50, 0.46, 0.52, 0.26, [0.98, 0.78, 0.44], 0.0);
                lm_back.add_glow(0.50, 0.28, 0.44, 0.38, [0.76, 0.60, 0.36], 0.0);
                lm_side.add_glow(0.50, 0.38, 0.44, 0.44, [0.76, 0.60, 0.36], 0.0);
            }
            12 => {
                // Cozy bedroom / warm bedside lamps: dusty blue-grey plaster, headboard, twin amber lamps
                fill_plaster(&mut back, [0.46, 0.50, 0.54], seed, 0.08);
                fill_plaster(&mut side, [0.42, 0.46, 0.50], seed + 5, 0.08);
                draw_plaster_ceiling(&mut ceil, [0.80, 0.79, 0.76], &[(0.50, 0.50, 0.040)], false, false, seed);
                fill_wood_floor(&mut floor, [0.34, 0.22, 0.12], [0.42, 0.28, 0.16], seed + 5);
                draw_rug(&mut floor, 0.16, 0.24, 0.84, 0.88, [0.74, 0.70, 0.64], [0.42, 0.44, 0.48], [0.68, 0.54, 0.34], seed);
                // Upholstered headboard + framed art above
                back.shadow_rect(0.24, 0.58, 0.76, 0.96, 0.025, 0.4);
                back.rect_grad_v(0.24, 0.58, 0.76, 0.96, [0.58, 0.54, 0.48], [0.42, 0.39, 0.34], None);
                draw_framed_art(&mut back, 0.36, 0.18, 0.64, 0.48, [0.22, 0.16, 0.10], seed + 9);
                // Twin warm glowing bedside lamps on nightstands
                for &lu in &[0.14f32, 0.86f32] {
                    back.rect(lu - 0.08, 0.72, lu + 0.08, 0.96, [0.28, 0.18, 0.10], None);
                    back.rect(lu - 0.055, 0.56, lu + 0.055, 0.68, [0.98, 0.84, 0.56], Some(0.92));
                }
                // Mid-plane: bed foot / draped quilt edge + dresser silhouette on left
                mid.rect_grad_v(0.20, 0.76, 0.80, 1.0, [0.72, 0.68, 0.62], [0.48, 0.44, 0.40], Some(1.0));
                mid.rect(0.22, 0.82, 0.78, 0.90, [0.24, 0.30, 0.36], Some(1.0));
                lm_back = Face::new([0.14, 0.11, 0.08, 1.0]);
                lm_side = Face::new([0.12, 0.10, 0.07, 1.0]);
                lm_back.add_glow(0.14, 0.60, 0.28, 0.34, [0.96, 0.68, 0.30], 0.0);
                lm_back.add_glow(0.86, 0.60, 0.28, 0.34, [0.96, 0.68, 0.30], 0.0);
                lm_side.add_glow(0.65, 0.56, 0.40, 0.42, [0.76, 0.52, 0.24], 0.0);
            }
            13 => {
                // Sunlit artist studio flat: bright white plaster, oversized canvases, paper lantern, plants, books
                fill_plaster(&mut back, [0.85, 0.84, 0.80], seed, 0.06);
                fill_plaster(&mut side, [0.80, 0.79, 0.75], seed + 6, 0.06);
                draw_plaster_ceiling(&mut ceil, [0.88, 0.88, 0.86], &[(0.50, 0.45, 0.060)], false, false, seed);
                fill_wood_floor(&mut floor, [0.48, 0.36, 0.23], [0.58, 0.44, 0.29], seed + 6);
                draw_framed_art(&mut back, 0.08, 0.14, 0.56, 0.66, [0.56, 0.42, 0.24], seed + 13);
                draw_bookshelf(&mut back, 0.62, 0.24, 0.94, 0.96, [0.74, 0.70, 0.64], 5, seed + 14, false);
                draw_framed_art(&mut side, 0.18, 0.20, 0.78, 0.64, [0.12, 0.12, 0.12], seed + 15);
                draw_pendant(&mut mid, &mut mid_emit, 0.48, 0.25, 0.18, 0.17, [0.98, 0.94, 0.84], [1.0, 0.90, 0.68], true);
                draw_plant(&mut mid, 0.18, 0.98, 1.15, [0.72, 0.40, 0.26], Some(1.0), seed + 16);
                mid.rect_grad_v(0.36, 0.70, 0.86, 0.75, [0.62, 0.48, 0.32], [0.46, 0.34, 0.22], Some(1.0));
                mid.rect(0.40, 0.75, 0.43, 1.0, [0.40, 0.30, 0.20], Some(1.0));
                mid.rect(0.80, 0.75, 0.83, 1.0, [0.40, 0.30, 0.20], Some(1.0));
                lm_back = Face::new([0.28, 0.25, 0.20, 1.0]);
                lm_side = Face::new([0.25, 0.22, 0.18, 1.0]);
                lm_back.add_glow(0.48, 0.30, 0.50, 0.46, [0.84, 0.72, 0.50], 0.0);
                lm_side.add_glow(0.48, 0.36, 0.46, 0.46, [0.76, 0.64, 0.44], 0.0);
            }
            14 => {
                // Dim evening TV / media living room: cool-blue screen glow + one warm corner accent lamp (rich night contrast!)
                fill_plaster(&mut back, [0.44, 0.43, 0.42], seed, 0.07);
                fill_plaster(&mut side, [0.40, 0.40, 0.41], seed + 7, 0.07);
                draw_plaster_ceiling(&mut ceil, [0.70, 0.70, 0.68], &[], false, false, seed);
                fill_wood_floor(&mut floor, [0.26, 0.17, 0.10], [0.34, 0.23, 0.14], seed + 7);
                // Wall-mounted flatscreen TV emitting cool cinematic glow
                back.rect(0.24, 0.28, 0.76, 0.60, [0.03, 0.035, 0.04], None);
                back.rect_grad_v(0.26, 0.30, 0.74, 0.58, [0.28, 0.46, 0.72], [0.16, 0.26, 0.44], Some(0.90));
                back.rect(0.18, 0.68, 0.82, 0.94, [0.18, 0.14, 0.11], None);
                // Mid-plane: sofa silhouette + warm reading floor lamp on far left
                mid.rect_grad_v(0.22, 0.70, 0.88, 1.0, [0.16, 0.18, 0.21], [0.09, 0.10, 0.12], Some(1.0));
                mid.rect(0.11, 0.48, 0.122, 0.98, [0.16, 0.15, 0.14], Some(1.0));
                mid.rect(0.06, 0.36, 0.17, 0.48, [0.95, 0.78, 0.46], Some(1.0));
                mid_emit.rect(0.06, 0.36, 0.17, 0.48, [1.0, 0.72, 0.34], Some(0.90));
                mid_emit.add_glow(0.115, 0.42, 0.18, 0.18, [0.85, 0.52, 0.20], 0.30);
                lm_back = Face::new([0.07, 0.08, 0.12, 1.0]);
                lm_side = Face::new([0.07, 0.08, 0.11, 1.0]);
                lm_back.add_glow(0.50, 0.44, 0.38, 0.36, [0.34, 0.50, 0.78], 0.0);
                lm_back.add_glow(0.12, 0.46, 0.26, 0.32, [0.78, 0.48, 0.18], 0.0);
                lm_side.add_glow(0.35, 0.46, 0.42, 0.42, [0.40, 0.48, 0.66], 0.0);
            }
            15 => {
                // Bay-window study / library: dark wood built-in bookcases, warm emerald banker's lamp, framed vintage maps
                fill_plaster(&mut back, [0.58, 0.52, 0.42], seed, 0.08);
                fill_plaster(&mut side, [0.52, 0.46, 0.38], seed + 8, 0.08);
                draw_wainscot_and_trim(&mut side, [0.22, 0.13, 0.07], 0.60, 3);
                draw_plaster_ceiling(&mut ceil, [0.80, 0.77, 0.72], &[(0.50, 0.48, 0.044)], true, false, seed);
                fill_wood_floor(&mut floor, [0.22, 0.13, 0.07], [0.30, 0.18, 0.10], seed + 8);
                draw_rug(&mut floor, 0.14, 0.16, 0.86, 0.86, [0.16, 0.28, 0.22], [0.52, 0.20, 0.16], [0.74, 0.62, 0.32], seed);
                draw_bookshelf(&mut back, 0.04, 0.10, 0.96, 0.96, [0.20, 0.11, 0.06], 6, seed + 19, true);
                draw_framed_art(&mut side, 0.22, 0.18, 0.76, 0.54, [0.38, 0.26, 0.12], seed + 20);
                // Mid-plane: study desk + warm green-glass banker's lamp + armchair
                mid.rect_grad_v(0.16, 0.68, 0.84, 0.73, [0.28, 0.16, 0.09], [0.18, 0.10, 0.05], Some(1.0));
                mid.rect(0.20, 0.73, 0.80, 1.0, [0.14, 0.08, 0.05], Some(1.0));
                mid.rect(0.32, 0.60, 0.335, 0.68, [0.56, 0.44, 0.18], Some(1.0));
                mid.ellipse(0.328, 0.57, 0.055, 0.028, [0.10, 0.46, 0.22], Some(1.0), 0.005);
                mid_emit.ellipse(0.328, 0.58, 0.054, 0.024, [0.46, 0.92, 0.46], Some(0.85), 0.010);
                mid_emit.add_glow(0.328, 0.63, 0.15, 0.12, [0.95, 0.76, 0.38], 0.35);
                lm_back = Face::new([0.20, 0.15, 0.10, 1.0]);
                lm_side = Face::new([0.18, 0.14, 0.09, 1.0]);
                lm_back.add_glow(0.50, 0.40, 0.46, 0.48, [0.84, 0.62, 0.32], 0.0);
                lm_side.add_glow(0.50, 0.42, 0.44, 0.46, [0.76, 0.56, 0.28], 0.0);
            }

            // ===============================================================
            // 16..24: GROUND-FLOOR TOWER LOBBIES & SF RETAIL STOREFRONTS
            // ===============================================================
            16 => {
                // Grand FiDi marble tower lobby: bookmatched travertine walls, coffered ceiling cove uplighting, concierge desk, marble columns
                fill_marble(&mut back, [0.74, 0.70, 0.63], [0.46, 0.41, 0.35], seed, 4.0, 3.0);
                fill_marble(&mut side, [0.70, 0.66, 0.59], [0.44, 0.39, 0.33], seed + 2, 3.0, 3.0);
                draw_plaster_ceiling(&mut ceil, [0.76, 0.73, 0.68], &[(0.28, 0.32, 0.036), (0.72, 0.32, 0.036), (0.28, 0.68, 0.036), (0.72, 0.68, 0.036)], true, true, seed);
                fill_marble(&mut floor, [0.54, 0.50, 0.45], [0.30, 0.27, 0.24], seed + 5, 4.0, 4.0);
                // Elevator core bronze portals on back wall
                for &ex in &[0.16f32, 0.44f32, 0.72f32] {
                    back.rect(ex, 0.32, ex + 0.14, 0.98, [0.34, 0.26, 0.16], None);
                    back.rect_grad_v(ex + 0.016, 0.35, ex + 0.124, 0.98, [0.22, 0.19, 0.16], [0.14, 0.12, 0.10], None);
                    back.rect(ex + 0.068, 0.35, ex + 0.072, 0.98, [0.05, 0.04, 0.03], None);
                    // Glowing brass floor indicator above elevator
                    back.rect(ex + 0.052, 0.26, ex + 0.088, 0.29, [1.0, 0.86, 0.52], Some(0.90));
                }
                // Warm architectural ceiling cove wash at top of marble walls
                back.rect(0.0, 0.0, 1.0, 0.035, [1.0, 0.88, 0.64], Some(0.88));
                side.rect(0.0, 0.0, 1.0, 0.035, [1.0, 0.88, 0.64], Some(0.85));
                // Mid-plane: twin monumental fluted marble columns + stone concierge desk with warm LED reveal
                mid.rect_grad_v(0.05, 0.0, 0.16, 1.0, [0.74, 0.70, 0.62], [0.54, 0.50, 0.44], Some(1.0));
                mid.rect_grad_v(0.84, 0.0, 0.95, 1.0, [0.74, 0.70, 0.62], [0.54, 0.50, 0.44], Some(1.0));
                mid.rect_grad_v(0.26, 0.66, 0.74, 0.98, [0.30, 0.24, 0.18], [0.16, 0.12, 0.09], Some(1.0));
                mid.rect(0.25, 0.64, 0.75, 0.67, [0.80, 0.77, 0.72], Some(1.0));
                // Glowing warm LED strip under reception counter overhang & toe-kick
                mid_emit.rect(0.27, 0.67, 0.73, 0.695, [1.0, 0.84, 0.52], Some(0.95));
                mid_emit.rect(0.28, 0.95, 0.72, 0.975, [1.0, 0.82, 0.48], Some(0.80));
                mid_emit.add_glow(0.50, 0.69, 0.28, 0.08, [0.88, 0.66, 0.34], 0.30);
                lm_back = Face::new([0.32, 0.27, 0.20, 1.0]);
                lm_side = Face::new([0.28, 0.24, 0.18, 1.0]);
                lm_back.add_glow(0.50, 0.12, 0.62, 0.32, [0.95, 0.76, 0.46], 0.0);
                lm_back.add_glow(0.50, 0.55, 0.48, 0.42, [0.68, 0.54, 0.34], 0.0);
                lm_side.add_glow(0.50, 0.16, 0.58, 0.34, [0.86, 0.68, 0.42], 0.0);
            }
            17 => {
                // Contemporary wood-slat & stone tower lobby: vertical oak wall, monumental lobby feature art, linear slots
                fill_plaster(&mut back, [0.76, 0.75, 0.72], seed, 0.04);
                fill_wood_slats(&mut back, 0.0, 0.0, 1.0, 1.0, [0.46, 0.32, 0.18], [0.36, 0.24, 0.13], 44.0, seed);
                draw_framed_art(&mut back, 0.22, 0.16, 0.78, 0.64, [0.08, 0.08, 0.09], seed + 33);
                fill_marble(&mut side, [0.68, 0.68, 0.67], [0.46, 0.47, 0.48], seed + 9, 3.0, 3.0);
                draw_acoustic_ceiling(&mut ceil, 0, seed + 8);
                fill_marble(&mut floor, [0.36, 0.36, 0.37], [0.22, 0.22, 0.24], seed + 10, 4.0, 4.0);
                // Mid-plane: sculptural stone reception desk + tall architectural planter + dark steel column
                mid.rect_grad_v(0.22, 0.68, 0.70, 0.98, [0.74, 0.73, 0.70], [0.56, 0.55, 0.52], Some(1.0));
                mid_emit.rect(0.24, 0.94, 0.68, 0.975, [1.0, 0.88, 0.64], Some(0.90));
                draw_plant(&mut mid, 0.82, 0.98, 1.35, [0.18, 0.19, 0.20], Some(1.0), seed + 22);
                mid.rect_grad_v(0.02, 0.0, 0.11, 1.0, [0.14, 0.15, 0.16], [0.08, 0.09, 0.10], Some(1.0));
                lm_back = Face::new([0.30, 0.27, 0.22, 1.0]);
                lm_side = Face::new([0.28, 0.26, 0.22, 1.0]);
                lm_back.add_glow(0.50, 0.36, 0.46, 0.42, [0.95, 0.82, 0.58], 0.0);
                lm_side.add_glow(0.50, 0.26, 0.48, 0.44, [0.78, 0.70, 0.56], 0.0);
            }
            18 => {
                // SF Artisan Cafe / Bakery: subway tile back wall, espresso bar, chalkboard menu, hanging brass pendants, glowing pastry display
                fill_plaster(&mut back, [0.78, 0.75, 0.69], seed, 0.06);
                fill_brick(&mut side, seed + 11);
                draw_plaster_ceiling(&mut ceil, [0.24, 0.23, 0.22], &[(0.25, 0.45, 0.042), (0.50, 0.45, 0.042), (0.75, 0.45, 0.042)], false, false, seed);
                fill_wood_floor(&mut floor, [0.36, 0.24, 0.14], [0.46, 0.32, 0.19], seed + 12);
                // Menu board + warm lit pastry/coffee shelves on back wall
                back.rect(0.14, 0.14, 0.52, 0.48, [0.05, 0.055, 0.06], None);
                for r in 0..6 {
                    let rv = 0.18 + r as f32 * 0.045;
                    back.rect(0.18, rv, 0.42 + (r as f32 % 2.0) * 0.05, rv + 0.012, [0.84, 0.80, 0.72], None);
                }
                draw_bookshelf(&mut back, 0.56, 0.16, 0.92, 0.62, [0.34, 0.22, 0.12], 3, seed + 34, true);
                back.rect_grad_v(0.06, 0.62, 0.94, 0.98, [0.32, 0.20, 0.12], [0.20, 0.12, 0.07], None);
                // Mid-plane: 3 brass cafe pendants + illuminated glass pastry display counter
                draw_pendant(&mut mid, &mut mid_emit, 0.24, 0.26, 0.12, 0.09, [0.64, 0.48, 0.20], [1.0, 0.84, 0.48], false);
                draw_pendant(&mut mid, &mut mid_emit, 0.50, 0.26, 0.12, 0.09, [0.64, 0.48, 0.20], [1.0, 0.84, 0.48], false);
                draw_pendant(&mut mid, &mut mid_emit, 0.76, 0.26, 0.12, 0.09, [0.64, 0.48, 0.20], [1.0, 0.84, 0.48], false);
                mid.rect_grad_v(0.10, 0.64, 0.90, 0.98, [0.36, 0.24, 0.14], [0.22, 0.14, 0.08], Some(1.0));
                // Glowing pastry vitrine inside counter
                mid.rect(0.16, 0.52, 0.54, 0.67, [0.82, 0.72, 0.54], Some(0.85));
                mid_emit.rect(0.17, 0.53, 0.53, 0.66, [1.0, 0.84, 0.52], Some(0.88));
                lm_back = Face::new([0.30, 0.24, 0.17, 1.0]);
                lm_side = Face::new([0.26, 0.20, 0.14, 1.0]);
                lm_back.add_glow(0.30, 0.34, 0.36, 0.42, [0.95, 0.72, 0.38], 0.0);
                lm_back.add_glow(0.72, 0.34, 0.36, 0.42, [0.95, 0.72, 0.38], 0.0);
                lm_side.add_glow(0.50, 0.38, 0.46, 0.46, [0.82, 0.62, 0.34], 0.0);
            }
            19 => {
                // Neighborhood bookstore: floor-to-ceiling warm backlit timber bookcases on back & side walls, reading table
                fill_plaster(&mut back, [0.70, 0.64, 0.54], seed, 0.06);
                fill_plaster(&mut side, [0.66, 0.60, 0.50], seed + 12, 0.06);
                draw_plaster_ceiling(&mut ceil, [0.82, 0.79, 0.74], &[(0.28, 0.35, 0.044), (0.72, 0.35, 0.044), (0.50, 0.72, 0.044)], false, false, seed);
                fill_wood_floor(&mut floor, [0.34, 0.21, 0.11], [0.44, 0.28, 0.15], seed + 13);
                draw_bookshelf(&mut back, 0.03, 0.08, 0.97, 0.96, [0.34, 0.21, 0.11], 7, seed + 40, true);
                draw_bookshelf(&mut side, 0.05, 0.08, 0.95, 0.96, [0.34, 0.21, 0.11], 7, seed + 41, true);
                // Mid-plane: hanging library pendants + stacked display table
                draw_pendant(&mut mid, &mut mid_emit, 0.34, 0.24, 0.14, 0.09, [0.16, 0.28, 0.20], [1.0, 0.86, 0.54], false);
                draw_pendant(&mut mid, &mut mid_emit, 0.66, 0.24, 0.14, 0.09, [0.16, 0.28, 0.20], [1.0, 0.86, 0.54], false);
                mid.rect_grad_v(0.20, 0.68, 0.80, 0.98, [0.38, 0.24, 0.13], [0.22, 0.13, 0.07], Some(1.0));
                // Book stacks on display table
                for i in 0..5 {
                    let bx = 0.25 + i as f32 * 0.11;
                    mid.rect(bx, 0.60, bx + 0.075, 0.68, [0.68 - i as f32 * 0.08, 0.32 + i as f32 * 0.07, 0.24 + i as f32 * 0.09], Some(1.0));
                }
                lm_back = Face::new([0.32, 0.26, 0.18, 1.0]);
                lm_side = Face::new([0.30, 0.24, 0.17, 1.0]);
                lm_back.add_glow(0.32, 0.42, 0.42, 0.48, [0.92, 0.72, 0.40], 0.0);
                lm_back.add_glow(0.68, 0.42, 0.42, 0.48, [0.92, 0.72, 0.40], 0.0);
                lm_side.add_glow(0.50, 0.44, 0.48, 0.48, [0.84, 0.66, 0.38], 0.0);
            }
            20 => {
                // Contemporary art gallery: crisp gallery white walls, bold spotlight pools on large framed canvases, polished concrete floor
                fill_plaster(&mut back, [0.86, 0.86, 0.85], seed, 0.03);
                fill_plaster(&mut side, [0.82, 0.82, 0.81], seed + 13, 0.03);
                draw_plaster_ceiling(&mut ceil, [0.86, 0.86, 0.86], &[(0.24, 0.35, 0.032), (0.50, 0.35, 0.032), (0.76, 0.35, 0.032), (0.24, 0.70, 0.032), (0.76, 0.70, 0.032)], false, false, seed);
                fill_marble(&mut floor, [0.56, 0.56, 0.55], [0.46, 0.46, 0.45], seed + 14, 3.0, 3.0);
                draw_framed_art(&mut back, 0.08, 0.18, 0.44, 0.66, [0.06, 0.06, 0.06], seed + 50);
                draw_framed_art(&mut back, 0.54, 0.18, 0.92, 0.66, [0.06, 0.06, 0.06], seed + 51);
                draw_framed_art(&mut side, 0.20, 0.20, 0.80, 0.64, [0.06, 0.06, 0.06], seed + 52);
                // Mid-plane: museum gallery plinth + minimalist sculpture + bench
                mid.rect_grad_v(0.42, 0.56, 0.58, 0.98, [0.86, 0.85, 0.83], [0.68, 0.67, 0.65], Some(1.0));
                mid.ellipse(0.50, 0.44, 0.055, 0.105, [0.24, 0.20, 0.16], Some(1.0), 0.008); // bronze sculpture
                // Crisp dramatic gallery track spotlight cones!
                lm_back = Face::new([0.24, 0.24, 0.24, 1.0]);
                lm_side = Face::new([0.22, 0.22, 0.22, 1.0]);
                lm_back.add_glow(0.26, 0.42, 0.22, 0.32, [1.0, 0.96, 0.88], 0.0);
                lm_back.add_glow(0.73, 0.42, 0.22, 0.32, [1.0, 0.96, 0.88], 0.0);
                lm_side.add_glow(0.50, 0.42, 0.30, 0.34, [0.96, 0.92, 0.84], 0.0);
            }
            21 => {
                // Design & apparel boutique: backlit recessed display niches, brass garment racks, warm travertine floor
                fill_plaster(&mut back, [0.76, 0.72, 0.66], seed, 0.05);
                fill_plaster(&mut side, [0.72, 0.68, 0.62], seed + 14, 0.05);
                draw_plaster_ceiling(&mut ceil, [0.84, 0.82, 0.78], &[(0.24, 0.38, 0.036), (0.50, 0.38, 0.036), (0.76, 0.38, 0.036)], false, true, seed);
                fill_wood_floor(&mut floor, [0.54, 0.44, 0.32], [0.62, 0.52, 0.39], seed + 15);
                // Three backlit architectural wall display niches
                for i in 0..3 {
                    let nx0 = 0.07 + i as f32 * 0.31;
                    let nx1 = nx0 + 0.24;
                    back.rect(nx0, 0.16, nx1, 0.86, [0.60, 0.56, 0.50], None);
                    back.rect(nx0 + 0.015, 0.175, nx1 - 0.015, 0.20, [1.0, 0.90, 0.68], Some(0.90));
                    for s in 0..3 {
                        let sv = 0.34 + s as f32 * 0.17;
                        back.rect(nx0 + 0.02, sv, nx1 - 0.02, sv + 0.02, [0.28, 0.20, 0.14], None);
                        back.rect(nx0 + 0.04, sv - 0.07, nx0 + 0.11, sv, [0.76, 0.70, 0.60], None);
                        back.rect(nx0 + 0.13, sv - 0.06, nx1 - 0.04, sv, [0.24, 0.28, 0.34], None);
                    }
                }
                // Mid-plane: brass garment rack + hanging apparel + sculptural plant
                mid.rect(0.14, 0.44, 0.66, 0.46, [0.68, 0.54, 0.26], Some(1.0));
                mid.rect(0.14, 0.44, 0.155, 0.98, [0.68, 0.54, 0.26], Some(1.0));
                mid.rect(0.645, 0.44, 0.66, 0.98, [0.68, 0.54, 0.26], Some(1.0));
                let gcols = [[0.78, 0.74, 0.68], [0.22, 0.26, 0.34], [0.62, 0.34, 0.24], [0.16, 0.17, 0.19], [0.58, 0.64, 0.56]];
                for g in 0..8 {
                    let gu = 0.18 + g as f32 * 0.056;
                    mid.rect_grad_v(gu, 0.46, gu + 0.044, 0.82, gcols[g % gcols.len()], mul3(gcols[g % gcols.len()], 0.7), Some(1.0));
                }
                draw_plant(&mut mid, 0.82, 0.98, 1.05, [0.80, 0.76, 0.70], Some(1.0), seed + 25);
                lm_back = Face::new([0.34, 0.30, 0.24, 1.0]);
                lm_side = Face::new([0.30, 0.26, 0.22, 1.0]);
                lm_back.add_glow(0.19, 0.42, 0.22, 0.42, [0.96, 0.82, 0.58], 0.0);
                lm_back.add_glow(0.50, 0.42, 0.22, 0.42, [0.96, 0.82, 0.58], 0.0);
                lm_back.add_glow(0.81, 0.42, 0.22, 0.42, [0.96, 0.82, 0.58], 0.0);
                lm_side.add_glow(0.50, 0.38, 0.48, 0.48, [0.82, 0.72, 0.54], 0.0);
            }
            22 => {
                // Corner market / wine shop: illuminated glass beverage coolers, stocked wood wine racks, warm strips
                fill_plaster(&mut back, [0.74, 0.72, 0.68], seed, 0.05);
                fill_plaster(&mut side, [0.68, 0.66, 0.62], seed + 15, 0.05);
                draw_acoustic_ceiling(&mut ceil, 0, seed + 10);
                fill_marble(&mut floor, [0.52, 0.50, 0.47], [0.40, 0.38, 0.36], seed + 16, 6.0, 6.0);
                // Illuminated beverage cooler bank on left + wine shelves on right
                for d in 0..3 {
                    let dx0 = 0.04 + d as f32 * 0.19;
                    let dx1 = dx0 + 0.175;
                    back.rect(dx0, 0.16, dx1, 0.94, [0.12, 0.13, 0.14], None);
                    back.rect(dx0 + 0.012, 0.18, dx1 - 0.012, 0.92, [0.78, 0.84, 0.88], Some(0.75));
                    for s in 0..5 {
                        let sv = 0.26 + s as f32 * 0.13;
                        back.rect(dx0 + 0.016, sv, dx1 - 0.016, sv + 0.016, [0.18, 0.19, 0.20], None);
                        back.rect(dx0 + 0.022, sv - 0.06, dx1 - 0.022, sv, [0.36 + (s as f32 * 0.11) % 0.4, 0.48, 0.32], None);
                    }
                }
                draw_bookshelf(&mut back, 0.63, 0.18, 0.96, 0.95, [0.36, 0.24, 0.14], 6, seed + 61, true);
                draw_bookshelf(&mut side, 0.06, 0.22, 0.94, 0.95, [0.36, 0.24, 0.14], 5, seed + 62, false);
                // Mid-plane: checkout counter + produce/wine island + hanging pendants
                draw_pendant(&mut mid, &mut mid_emit, 0.30, 0.24, 0.12, 0.08, [0.12, 0.14, 0.16], [1.0, 0.92, 0.74], false);
                draw_pendant(&mut mid, &mut mid_emit, 0.70, 0.24, 0.12, 0.08, [0.12, 0.14, 0.16], [1.0, 0.92, 0.74], false);
                mid.rect_grad_v(0.16, 0.64, 0.84, 0.98, [0.44, 0.30, 0.18], [0.28, 0.18, 0.10], Some(1.0));
                lm_back = Face::new([0.38, 0.38, 0.36, 1.0]);
                lm_side = Face::new([0.34, 0.33, 0.30, 1.0]);
                lm_back.add_glow(0.32, 0.50, 0.36, 0.48, [0.80, 0.88, 0.96], 0.0);
                lm_back.add_glow(0.78, 0.48, 0.30, 0.46, [0.92, 0.76, 0.46], 0.0);
                lm_side.add_glow(0.50, 0.40, 0.48, 0.48, [0.80, 0.74, 0.60], 0.0);
            }
            _ => {
                // 23: Moody SF evening bar & bistro: dark mahogany paneling, glowing amber backlit bottle shelves, intimate brass pendants
                fill_plaster(&mut back, [0.18, 0.14, 0.12], seed, 0.07);
                fill_plaster(&mut side, [0.16, 0.13, 0.11], seed + 16, 0.07);
                draw_wainscot_and_trim(&mut side, [0.16, 0.10, 0.06], 0.58, 3);
                draw_plaster_ceiling(&mut ceil, [0.12, 0.10, 0.09], &[(0.28, 0.42, 0.032), (0.50, 0.42, 0.032), (0.72, 0.42, 0.032)], false, false, seed);
                fill_wood_floor(&mut floor, [0.18, 0.11, 0.06], [0.25, 0.15, 0.08], seed + 17);
                // Dramatic amber-backlit back-bar bottle wall!
                back.rect(0.10, 0.16, 0.90, 0.72, [0.14, 0.08, 0.05], None);
                for s in 0..4 {
                    let sv = 0.24 + s as f32 * 0.12;
                    back.rect(0.13, sv - 0.018, 0.87, sv + 0.008, [1.0, 0.70, 0.26], Some(0.95));
                    // Individual amber/green/clear glass bottles along each backlit tier
                    for b in 0..14 {
                        let bu = 0.15 + b as f32 * 0.051;
                        let bh = 0.065 + 0.025 * hash2(b, s, seed + 70);
                        let bc = mix3([0.52, 0.24, 0.06], [0.14, 0.30, 0.14], hash2(b, s + 9, seed));
                        back.rect(bu, sv - bh, bu + 0.026, sv - 0.008, bc, Some(0.45));
                    }
                    back.rect(0.12, sv, 0.88, sv + 0.018, [0.22, 0.13, 0.07], None);
                }
                // Mid-plane: warm amber glass pendant row + dark bar counter + barstools
                draw_pendant(&mut mid, &mut mid_emit, 0.25, 0.30, 0.095, 0.095, [0.76, 0.44, 0.12], [1.0, 0.68, 0.24], true);
                draw_pendant(&mut mid, &mut mid_emit, 0.50, 0.30, 0.095, 0.095, [0.76, 0.44, 0.12], [1.0, 0.68, 0.24], true);
                draw_pendant(&mut mid, &mut mid_emit, 0.75, 0.30, 0.095, 0.095, [0.76, 0.44, 0.12], [1.0, 0.68, 0.24], true);
                mid.rect_grad_v(0.08, 0.64, 0.92, 0.69, [0.28, 0.16, 0.09], [0.16, 0.09, 0.05], Some(1.0));
                mid.rect(0.10, 0.69, 0.90, 1.0, [0.11, 0.07, 0.045], Some(1.0));
                mid_emit.rect(0.12, 0.695, 0.88, 0.715, [1.0, 0.68, 0.24], Some(0.85));
                lm_back = Face::new([0.10, 0.07, 0.04, 1.0]);
                lm_side = Face::new([0.09, 0.06, 0.04, 1.0]);
                lm_back.add_glow(0.50, 0.44, 0.48, 0.38, [1.0, 0.64, 0.22], 0.0);
                lm_side.add_glow(0.55, 0.46, 0.44, 0.40, [0.85, 0.52, 0.18], 0.0);
            }
        }

        // Bake architectural corner & perimeter AO into all shell faces and lightmaps
        back.apply_corner_ao(0.08, 0.34);
        side.apply_corner_ao(0.08, 0.34);
        floor.apply_corner_ao(0.09, 0.36);
        lm_back.apply_corner_ao(0.10, 0.42);
        lm_side.apply_corner_ao(0.10, 0.42);

        // Pack per-quadrant linear mip chains so faces NEVER bleed across quadrant borders
        let layer0 = pack_quad_mips([&back, &side, &ceil, &floor]);
        let layer1 = pack_quad_mips([&mid, &mid_emit, &lm_back, &lm_side]);
        (layer0, layer1)
    }

    fn downsample_face(src: &[[f32; 4]], side: usize) -> Vec<[f32; 4]> {
        let h = side / 2;
        let mut out = vec![[0.0f32; 4]; h * h];
        for y in 0..h {
            for x in 0..h {
                let mut acc = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = src[(2 * y + dy) * side + (2 * x + dx)];
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                    acc[3] += p[3];
                }
                out[y * h + x] = [acc[0] * 0.25, acc[1] * 0.25, acc[2] * 0.25, acc[3] * 0.25];
            }
        }
        out
    }

    fn pack_quad_mips(quads: [&Face; 4]) -> Vec<Vec<u8>> {
        let mut cur: [Vec<[f32; 4]>; 4] = [quads[0].px.clone(), quads[1].px.clone(), quads[2].px.clone(), quads[3].px.clone()];
        let mut qside = F;
        let mut mips = Vec::with_capacity(ATLAS_MIPS as usize);
        for m in 0..ATLAS_MIPS {
            let side = qside * 2;
            let mut bytes = vec![0u8; side * side * 4];
            let offsets = [(0, 0), (qside, 0), (0, qside), (qside, qside)];
            for (qi, &(ox, oy)) in offsets.iter().enumerate() {
                let q = &cur[qi];
                for y in 0..qside {
                    for x in 0..qside {
                        let p = q[y * qside + x];
                        let dst = ((oy + y) * side + (ox + x)) * 4;
                        bytes[dst] = (lin_to_srgb(p[0]) * 255.0).round() as u8;
                        bytes[dst + 1] = (lin_to_srgb(p[1]) * 255.0).round() as u8;
                        bytes[dst + 2] = (lin_to_srgb(p[2]) * 255.0).round() as u8;
                        bytes[dst + 3] = (p[3].clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                }
            }
            mips.push(bytes);
            if m + 1 < ATLAS_MIPS {
                for q in &mut cur {
                    *q = downsample_face(q, qside);
                }
                qside /= 2;
            }
        }
        mips
    }

    /// Synthesize all 24 rooms in parallel across available CPU cores.
    pub fn build_atlas() -> RoomAtlas {
        let t = Instant::now();
        let n_rooms = ATLAS_ROOMS as usize;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(n_rooms);
        let mut results: Vec<Option<(Vec<Vec<u8>>, Vec<Vec<u8>>)>> = (0..n_rooms).map(|_| None).collect();
        let chunk = n_rooms.div_ceil(threads);
        std::thread::scope(|s| {
            for (ci, slice) in results.chunks_mut(chunk).enumerate() {
                let start = ci * chunk;
                s.spawn(move || {
                    for (off, slot) in slice.iter_mut().enumerate() {
                        *slot = Some(synthesize_room((start + off) as u32));
                    }
                });
            }
        });
        let mut layers = Vec::with_capacity(ATLAS_LAYERS as usize);
        for item in results {
            let (l0, l1) = item.unwrap();
            layers.push(l0);
            layers.push(l1);
        }
        RoomAtlas { layers, from_cache: false, ms: t.elapsed().as_secs_f64() * 1e3 }
    }

    fn cache_path(native_cache: &Path) -> PathBuf {
        native_cache.join("rooms").join(format!("room-atlas-v{ATLAS_CACHE_VERSION}.bin"))
    }

    pub fn encode_cache(a: &RoomAtlas) -> Vec<u8> {
        let mut out = Vec::with_capacity(a.total_bytes() as usize + 64);
        out.extend_from_slice(ATLAS_CACHE_MAGIC);
        for v in [ATLAS_CACHE_VERSION, ATLAS_ROOMS, ATLAS_RES, ATLAS_MIPS] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for mips in &a.layers {
            for bytes in mips {
                out.extend_from_slice(bytes);
            }
        }
        let digest = Sha256::digest(&out);
        out.extend_from_slice(&digest);
        out
    }

    pub fn decode_cache(bytes: &[u8]) -> Option<RoomAtlas> {
        let header = 8 + 16;
        if bytes.len() < header + 32 || &bytes[..8] != ATLAS_CACHE_MAGIC {
            return None;
        }
        let (body, digest) = bytes.split_at(bytes.len() - 32);
        if Sha256::digest(body).as_slice() != digest {
            return None;
        }
        let u32_at = |at: usize| -> Option<u32> { Some(u32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?)) };
        if u32_at(8)? != ATLAS_CACHE_VERSION || u32_at(12)? != ATLAS_ROOMS || u32_at(16)? != ATLAS_RES || u32_at(20)? != ATLAS_MIPS {
            return None;
        }
        let mut at = header;
        let mut layers = Vec::with_capacity(ATLAS_LAYERS as usize);
        for _ in 0..ATLAS_LAYERS {
            let mut mips = Vec::with_capacity(ATLAS_MIPS as usize);
            for mip in 0..ATLAS_MIPS {
                let side = (ATLAS_RES >> mip) as usize;
                let len = side * side * 4;
                mips.push(body.get(at..at + len)?.to_vec());
                at += len;
            }
            layers.push(mips);
        }
        (at == body.len()).then(|| RoomAtlas { layers, from_cache: true, ms: 0.0 })
    }

    /// Load the cached interior atlas from `cache_dir/rooms/`, or synthesize and cache it.
    pub fn load_or_build(cache_dir: Option<&Path>) -> RoomAtlas {
        let t = Instant::now();
        let path = cache_dir.map(cache_path);
        if let Some(p) = &path {
            if let Ok(bytes) = std::fs::read(p) {
                if let Some(mut a) = decode_cache(&bytes) {
                    a.ms = t.elapsed().as_secs_f64() * 1e3;
                    return a;
                }
            }
        }
        let a = build_atlas();
        if let Some(p) = &path {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, encode_cache(&a)).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
        }
        a
    }

    /// Mean linear RGB luminance of quadrant `(qx, qy)` in mip 0 of a layer (for tests).
    pub fn quadrant_stats(layer_mip0: &[u8], qx: usize, qy: usize) -> (f32, f32, f32) {
        let ox = qx * F;
        let oy = qy * F;
        let (mut sum, mut sum2, mut max_a) = (0.0f64, 0.0f64, 0.0f32);
        for y in 0..F {
            for x in 0..F {
                let i = ((oy + y) * R + (ox + x)) * 4;
                let l = 0.2126 * srgb_to_lin(layer_mip0[i] as f32 / 255.0)
                    + 0.7152 * srgb_to_lin(layer_mip0[i + 1] as f32 / 255.0)
                    + 0.0722 * srgb_to_lin(layer_mip0[i + 2] as f32 / 255.0);
                sum += l as f64;
                sum2 += (l * l) as f64;
                max_a = max_a.max(layer_mip0[i + 3] as f32 / 255.0);
            }
        }
        let n = (F * F) as f64;
        let mean = (sum / n) as f32;
        let std = ((sum2 / n - (mean as f64).powi(2)).max(0.0).sqrt()) as f32;
        (mean, std, max_a)
    }
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

    #[test]
    fn room_atlas_is_deterministic_varied_and_roundtrips_cache() {
        let a = atlas::build_atlas();
        assert_eq!(a.layers.len(), ATLAS_LAYERS as usize);
        for (li, mips) in a.layers.iter().enumerate() {
            assert_eq!(mips.len(), ATLAS_MIPS as usize, "layer {li}");
            for (mi, bytes) in mips.iter().enumerate() {
                let side = (ATLAS_RES >> mi) as usize;
                assert_eq!(bytes.len(), side * side * 4, "layer {li} mip {mi}");
            }
        }
        // Every room has real architectural detail, mid-plane foreground elements, and lightmaps.
        for room in 0..ATLAS_ROOMS as usize {
            let l0 = &a.layers[room * 2][0];
            let l1 = &a.layers[room * 2 + 1][0];
            let (back_mean, back_std, _) = atlas::quadrant_stats(l0, 0, 0);
            let (_, _, mid_cov) = atlas::quadrant_stats(l1, 0, 0);
            let (lm_mean, lm_std, _) = atlas::quadrant_stats(l1, 0, 1);
            assert!(back_mean > 0.04 && back_mean < 0.90, "room {room}: back_mean={back_mean}");
            assert!(back_std > 0.02, "room {room}: back wall too flat (std={back_std})");
            assert!(mid_cov > 0.5, "room {room}: mid-plane missing foreground elements (max_a={mid_cov})");
            assert!(lm_mean > 0.05 && lm_std > 0.015, "room {room}: lightmap flat ({lm_mean}, {lm_std})");
        }
        // Cache encode/decode preserves exact bytes and rejects corruption.
        let mut encoded = atlas::encode_cache(&a);
        let decoded = atlas::decode_cache(&encoded).expect("valid cache decodes");
        assert_eq!(decoded.layers, a.layers);
        encoded[64] ^= 0x55;
        assert!(atlas::decode_cache(&encoded).is_none(), "corrupted cache rejected");
    }
}
