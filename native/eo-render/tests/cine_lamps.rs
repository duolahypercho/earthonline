//! (`--look cinematic`, stage L2) The cinematic night's street lamps on the
//! real city: the palette grid (eo-sky `cine_lamps`) gives every lamp street
//! one colour from end to end, clustered by street (not lamp by lamp), with
//! sodium on 15-25 % of the lamps; the props slice's district lights find
//! their own street in it; the same grid twice; and the shaders' constants
//! are eo-sky's.

use eo_sky::cine_lamps::{self, LampTint, PaletteGrid, StreetIn};
use std::collections::HashMap;
use std::sync::OnceLock;

struct Fixture {
    sc: eo_street::StreetCity,
    elev: eo_data::ElevationGrid,
}

fn fixture() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| {
        let paths = eo_data::DataPaths::discover().unwrap();
        let elev = eo_data::ElevationGrid::load(&paths).unwrap();
        Fixture { sc: eo_street::StreetCity::load().unwrap(), elev }
    })
}

/// The grid as eo-app builds it (models.rs: 16 m cells from the elevation grid's corner).
fn grid(f: &Fixture) -> PaletteGrid {
    let g = &f.elev;
    let corner = [g.meta.origin_x, g.meta.origin_z];
    let span = (g.max_x() - corner[0]).max(g.max_z() - corner[1]);
    let cell = 16f64.max((span / eo_render::sky::PALETTE_CELLS_MAX as f64).ceil());
    let cols = (((g.max_x() - corner[0]) / cell).ceil() as usize).min(eo_render::sky::PALETTE_CELLS_MAX);
    let rows = (((g.max_z() - corner[1]) / cell).ceil() as usize).min(eo_render::sky::PALETTE_CELLS_MAX);
    let streets: Vec<StreetIn> = f
        .sc
        .segments
        .iter()
        .map(|s| StreetIn { name: if s.street_name.is_empty() { s.id.as_str() } else { s.street_name.as_str() }, highway: s.highway.as_str(), width: s.width, points: &s.points })
        .collect();
    PaletteGrid::build(corner, cell, cols, rows, &streets)
}

#[test]
fn the_city_s_lamps_take_their_street_s_colour_with_a_sodium_share() {
    let f = fixture();
    let t0 = std::time::Instant::now();
    let g = grid(f);
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    assert_eq!(g, grid(f), "deterministic");
    assert!(g.cols <= eo_render::sky::PALETTE_CELLS_MAX && g.rows <= eo_render::sky::PALETTE_CELLS_MAX);
    // the fallback lamps (eo-app models.rs) on their segments; a lamp's tint by the grid at its head.
    let mut tally: HashMap<LampTint, usize> = HashMap::new();
    let mut found = 0usize;
    let mut own = 0usize;
    let mut total = 0usize;
    let mut by_street: HashMap<&str, HashMap<LampTint, usize>> = HashMap::new();
    for s in &f.sc.segments {
        let seg = [eo_sky::lights::SegmentIn {
            highway: s.highway.as_str(),
            width: s.width,
            sidewalk_left: s.sidewalk_left.or(s.sidewalk_w).unwrap_or(0.0),
            sidewalk_right: s.sidewalk_right.or(s.sidewalk_w).unwrap_or(0.0),
            points: &s.points,
        }];
        let name = if s.street_name.is_empty() { s.id.as_str() } else { s.street_name.as_str() };
        let want = cine_lamps::street_tint(name, &s.highway);
        for l in eo_sky::lights::place_street_lamps(&seg, |_, _| 0.0) {
            total += 1;
            let t = match g.at(l.pos[0], l.pos[2]) {
                Some((t, _)) => {
                    found += 1;
                    t
                }
                None => LampTint::Led3000,
            };
            if Some(t) == want {
                own += 1;
            }
            *tally.entry(t).or_default() += 1;
            *by_street.entry(name).or_default().entry(t).or_default() += 1;
        }
    }
    let share = |t: LampTint| *tally.get(&t).unwrap_or(&0) as f64 / total as f64;
    // clustered by street: a named street's lamps mostly one colour (the majority's share over every lamp).
    let majority: usize = by_street.values().map(|m| m.values().copied().max().unwrap_or(0)).sum();
    println!(
        "{total} lamps, grid {}x{} of {} m in {ms:.0} ms: found {:.3}, own street {:.3}, majority {:.3}; 3000 K {:.3}, 4000 K {:.3}, sodium {:.3}",
        g.cols,
        g.rows,
        g.cell,
        found as f64 / total as f64,
        own as f64 / total as f64,
        majority as f64 / total as f64,
        share(LampTint::Led3000),
        share(LampTint::Led4000),
        share(LampTint::Sodium)
    );
    assert!(total > 5000, "{total}");
    assert!(found as f64 > 0.99 * total as f64, "every lamp stands on a palette street: {found} / {total}");
    assert!(own as f64 > 0.9 * total as f64, "a lamp takes its own street's tint: {own} / {total}");
    assert!(majority as f64 > 0.92 * total as f64, "clustered by street: {majority} / {total}");
    let na = share(LampTint::Sodium);
    assert!((0.15..=0.25).contains(&na), "sodium share {na}");
    assert!(share(LampTint::Led3000) > 0.6, "mostly 3000 K");
    assert!(share(LampTint::Led4000) > 0.02 && share(LampTint::Led4000) < 0.15);
    for name in ["Castro Street", "Mission Street", "The Embarcadero", "18th Street", "24th Street"] {
        println!("{name}: {:?}", by_street.get(name));
    }
}

#[test]
fn the_district_lights_find_their_street() {
    let f = fixture();
    let g = grid(f);
    let paths = eo_data::DataPaths::discover().unwrap();
    let world = eo_props::load_world(&paths).unwrap();
    // Castro at 18th, Mission at 24th and a downtown tile: every district light's head on a street of the grid.
    let mut n = 0;
    let mut on = 0;
    let mut tiles = 0;
    for key in world.tile_keys().into_iter().step_by(37).take(60) {
        let p = world.plan_tile(key);
        if !p.lights.is_empty() {
            tiles += 1;
        }
        for l in &p.lights {
            n += 1;
            if g.at(l.pos[0], l.pos[2]).is_some() {
                on += 1;
            }
        }
    }
    println!("{n} district lights in {tiles} tiles: {on} on a palette street");
    assert!(n > 50, "{n}");
    assert!(on as f64 > 0.97 * n as f64, "{on} / {n}");
}

#[test]
fn the_shaders_cinematic_lamp_constants_are_eo_skys() {
    use eo_sky::cine_lamps::*;
    let src = include_str!("../src/sky_light.wgsl");
    let has = |line: String| assert!(src.contains(&line), "sky_light.wgsl lacks `{line}`");
    let c = |d: f64| d.to_radians().cos();
    has(format!("const SKY_CINE_COBRA_GAIN: f32 = {COBRA_ALONG_GAIN:?};"));
    has(format!("const SKY_CINE_COBRA_ALONG_COS: vec2<f32> = vec2<f32>({:.7}, {:.7});", c(COBRA_ALONG_ROLL_DEG[0]), c(COBRA_ALONG_ROLL_DEG[1])));
    has(format!("const SKY_CINE_COBRA_ACROSS_COS: vec2<f32> = vec2<f32>({:.7}, {:.7});", c(COBRA_ACROSS_ROLL_DEG[0]), c(COBRA_ACROSS_ROLL_DEG[1])));
    has(format!("const SKY_CINE_COBRA_ROUND: f32 = {COBRA_ROUND_ALONG2:?};"));
    has(format!("const SKY_CINE_BEAM_CUT: vec2<f32> = vec2<f32>({LOW_BEAM_CUT_LEFT_DEG:?}, {LOW_BEAM_CUT_RIGHT_DEG:?});"));
    has(format!("const SKY_CINE_BEAM_STEP: f32 = {LOW_BEAM_STEP_DEG:?};"));
    has(format!("const SKY_CINE_BEAM_EDGE: f32 = {LOW_BEAM_EDGE_DEG:?};"));
    has(format!("const SKY_CINE_BEAM_GLARE: f32 = {LOW_BEAM_GLARE:?};"));
    has(format!("const SKY_CINE_BEAM_HOT: vec2<f32> = vec2<f32>({:?}, {:?});", LOW_BEAM_HOT_DEG[0], LOW_BEAM_HOT_DEG[1]));
    has(format!("const SKY_CINE_BEAM_SPREAD: f32 = {LOW_BEAM_SPREAD:?};"));
    has(format!("const SKY_CINE_SRC_STREET: f32 = {STREET_SOURCE_RADIUS_M:?};"));
    has(format!("const SKY_CINE_SRC_HEADLAMP: f32 = {HEADLAMP_SOURCE_RADIUS_M:?};"));
    has(format!("const SKY_CINE_SPEC_CAP: f32 = {SPEC_CAP:?};"));
    // the gather's: compiled in and out, every tint's colour, and it validates both ways.
    let on = eo_render::sky::cine_lamps_wgsl(true);
    assert!(on.contains("const LT_CINE: bool = true;") && eo_render::sky::cine_lamps_wgsl(false).contains("const LT_CINE: bool = false;"));
    for t in LampTint::ALL {
        let k = t.colour();
        assert!(on.contains(&format!("vec3<f32>({:.6}, {:.6}, {:.6})", k[0], k[1], k[2])), "{t:?}");
    }
    for flag in [true, false] {
        let src = format!(
            "{}\n{}\nconst LT_TILE_M: f32 = 1.0;\n{}{}{}",
            include_str!("../src/sky_common.wgsl"),
            eo_render::sky::sea_wgsl(),
            eo_render::shop_lights::wgsl(),
            eo_render::sky::cine_lamps_wgsl(flag) + &eo_render::sky::night_photometry_wgsl() + &eo_render::sky::lamp_bounce_wgsl() + &eo_render::sky::lamp_units_wgsl(),
            include_str!("../src/sky_lights.wgsl")
        );
        let m = wgpu::naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all()).validate(&m).unwrap();
    }
}

#[test]
fn the_cinematic_rooms_light_more_homes_and_only_under_the_look() {
    use eo_render::rooms::{CINE_OCCUPANCY_GAIN, CINE_TV_SHARE, drive, drive_look};
    for (alt, hour) in [(-20.0, 21.0), (-5.0, 19.5), (30.0, 13.0)] {
        assert_eq!(drive_look(alt, hour, false, false), drive(alt, hour), "default unchanged at {hour}");
        let (env, work) = drive_look(alt, hour, false, true);
        let base = drive(alt, hour).0;
        assert!((env[1] as f64 - (base[1] as f64 * CINE_OCCUPANCY_GAIN).min(0.95)).abs() < 1e-6);
        assert_eq!(work[2], CINE_TV_SHARE as f32);
    }
    let (b, c) = (drive(-20.0, 21.0).0[1], drive_look(-20.0, 21.0, false, true).0[1]);
    println!("lit homes at 21 h: {b} -> {c}");
    assert!(c > b * 1.14 && c < b * 1.16);
}
