//! the street size's matched captures (`--bench --gs-shots
//! --label NAME`; the before side adds `--ground-scale 1.0 --default-camera`,
//! each side a fresh process). Every camera is anchored to what the plan
//! draws, not to a world point, so both sides frame the same place:
//!
//! * the street-size trial's two bench-path frames (kit_street frame 260, the
//!   residential street; sidewalk_walk frame 195);
//! * one view of each hand-built building whose lot strip the ground plan
//!   closes (`GAP_RECORDS`): from the far side of the street it fronts, eye
//!   1.7 m, looking square at its authored front (the neighbours either side
//!   in the frame);
//! * the bridges from a shore (the Golden Gate from Crissy Field, the Bay
//!   Bridge from the Embarcadero) and each bridge's landing on its San
//!   Francisco shore; the Ferry Building, the Ferry Plaza pier and the
//!   Embarcadero quay wall;
//! * one high view of the city (the sky slice's `above` card), so the far
//!   layer shows;
//! * hills: the hill cards (`hill_cameras`): eye-level looks up and down the
//!   steepest street (Mount Vernon Avenue) and up two well-known steep
//!   blocks (Filbert Street at Russian Hill, 22nd Street at Noe Valley), a
//!   hill skyline from the bay, and Twin Peaks from Dolores Park. Their
//!   anchors are plan points, so `--height-scale 1.0` frames the same place.
//!
//! The street / walk / drive views come from `--agents-capture` with
//! `EO_GS_CAPTURE=1` (agents_bench.rs). `EO_GS_SHOTS=a,b` keeps the shots
//! whose name contains a or b. Not a quality judgement: the visual gate's
//! recorded verdict for the AAA-level claim is REJECT.

use super::{EYE_HEIGHT_M, Segment, capture, gsf, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::{Renderer, Targets, wgpu};
use glam::Vec3;

/// The hand-built buildings whose lot strip the ground plan closes at the
/// street: the four with the widest strip the plain scale would open between
/// a front corner and a kit neighbour flush with it (4.3, 3.5, 3.5 and 2.8 m;
/// 48 Blender buildings have such a neighbour), with that corner (0: the
/// front edge's start, 1: its end); and the largest building, whose street
/// faces on opposite sides stand set back. Two views each: square at the
/// front, and at that corner.
pub const GAP_RECORDS: &[(&str, usize, &str)] = &[
    ("173886703", 0, "the spawn's block, a 34.6 m front"),
    ("283656332", 1, "the Sunset, a 28.2 m front"),
    ("260361486", 0, "a 27.9 m front"),
    ("32609888", 0, "downtown, a 22.4 m front"),
    ("165846552", 1, "the largest: its party walls closed, its faces on two opposite streets set back"),
];

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--gs-shots")
}

fn look_at(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    Camera { far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect) }
}

fn surface(world: &World, x: f32, z: f32) -> f32 {
    match &world.street {
        Some(s) => s.world.surface_at(x as f64, z as f64, None) as f32,
        None => world.elevation.ground(x as f64, z as f64) as f32,
    }
}

/// From across the street a hand-built building fronts: (eye, the front's
/// middle 5 m up, its `corner` (0: start, 1: end) 5 m up): square at it,
/// and along the street at that corner (the neighbour beside it in view).
fn gap_camera(world: &World, key: &str, corner: usize) -> Option<(Vec3, Vec3, Vec3)> {
    let manifest = eo_assets::load_blender_buildings(&world.paths.repo_root).ok()?;
    let b = manifest.iter().find(|b| b.record_key() == key)?;
    let rec = world.city.buildings.iter().find(|r| r.id == key)?;
    let poly = rec.record_polygon();
    let m = poly.len();
    let e = b.frame.edge_index % m;
    let (pa, pb) = (poly[e], poly[(e + 1) % m]);
    let ring = &rec.ring;
    let area2: f32 = (0..ring.len()).map(|i| ring[i][0] * ring[(i + 1) % ring.len()][1] - ring[(i + 1) % ring.len()][0] * ring[i][1]).sum();
    let orient = if area2 >= 0.0 { 1.0 } else { -1.0 };
    let along = glam::Vec2::new(pb[0] - pa[0], pb[1] - pa[1]).normalize_or_zero();
    let n = glam::Vec2::new(along.y * orient, -along.x * orient);
    let mid = glam::Vec2::new((pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0);
    // The nearest vehicle way's centreline straight out from the front.
    let mut best = f32::MAX;
    for r in world.city.roads.iter().filter(|r| eo_data::standards::is_vehicle_way(&r.highway)) {
        for w in r.points.windows(2) {
            let (p, q) = (glam::Vec2::from(w[0]), glam::Vec2::from(w[1]));
            let s = q - p;
            let den = n.x * s.y - n.y * s.x;
            if den.abs() < 1e-6 {
                continue;
            }
            let d = p - mid;
            let t = (d.x * s.y - d.y * s.x) / den;
            let u = (d.x * n.y - d.y * n.x) / den;
            if t > 1.0 && t < 60.0 * gsf() && (0.0..=1.0).contains(&u) {
                best = best.min(t);
            }
        }
    }
    let across = if best < f32::MAX { 2.0 * best - 2.5 * gsf() } else { 30.0 * gsf() };
    let e2 = mid + n * across;
    let eye = Vec3::new(e2.x, surface(world, e2.x, e2.y) + EYE_HEIGHT_M, e2.y);
    let at = Vec3::new(mid.x, surface(world, mid.x, mid.y) + 5.0, mid.y);
    let c = if corner == 0 { pa } else { pb };
    let end = Vec3::new(c[0], surface(world, c[0], c[1]) + 5.0, c[1]);
    Some((eye, at, end))
}

/// The shots: (name, camera, what).
pub fn cameras(world: &World, segs: &[Segment], args: &Args, aspect: f32) -> Vec<(String, Camera, String)> {
    let far = args.far;
    let mut out = vec![];
    // The trial's two bench-path frames (as `--capture-props2` writes them).
    for (seg, frame, what) in [("kit_street", 260usize, "residential street (Castro), bench path kit_street frame 260"), ("sidewalk_walk", 195, "sidewalk (Castro at the four-leg junction), bench path sidewalk_walk frame 195")] {
        if let Some(s) = segs.iter().find(|s| s.name == seg) {
            out.push((format!("{seg}-f{frame}"), (s.camera)(frame, args.warmup + s.frames), what.to_string()));
        }
    }
    for (key, corner, what) in GAP_RECORDS {
        if let Some((eye, at, end)) = gap_camera(world, key, *corner) {
            out.push((format!("gap-{key}"), look_at(eye, at, aspect, far), format!("hand-built sf-building-{key} ({what}) from across its street, eye 1.7 m, square at its front")));
            out.push((format!("gap-{key}-corner"), look_at(eye, end, aspect, far), format!("hand-built sf-building-{key} ({what}) from across its street, eye 1.7 m, at the corner beside its kit neighbour")));
        }
    }
    // The bridges from a shore, the shore pieces, from the landmarks slice's cards.
    for (n, cam, what) in super::landmarks_bench::capture_cameras(world, aspect, far) {
        if matches!(n, "golden-gate" | "bay-bridge" | "embarcadero-seawall" | "pier" | "pier-deck") {
            out.push((format!("lm-{n}"), cam, what));
        }
    }
    // Each bridge's landing on its San Francisco shore.
    let js = eo_landmarks::LandmarksJs::get();
    let b = &js.bay_bridge_west;
    let sf = Vec3::new(b.deck_ends["sf"].x as f32, 0.0, b.deck_ends["sf"].z as f32);
    let w2 = Vec3::new(b.towers[0].centre[0] as f32, 0.0, b.towers[0].centre[1] as f32);
    let along = (w2 - sf).normalize_or(Vec3::X);
    let side = Vec3::new(-along.z, 0.0, along.x);
    let north = if side.z < 0.0 { side } else { -side };
    let e = sf + (along * 250.0 + north * 220.0) * gsf();
    let eye = Vec3::new(e.x, 45.0, e.z);
    out.push(("bb-sf-landing".to_string(), look_at(eye, sf + Vec3::new(0.0, 15.0, 0.0), aspect, far), "the Bay Bridge's San Francisco landing from 45 m over the bay north of the span, looking back at where the deck meets the shore".to_string()));
    let g = &js.golden_gate;
    let ws = Vec3::new(g.way_ends["south"][0] as f32, 0.0, g.way_ends["south"][1] as f32);
    let mid = |a: [f64; 2], b: [f64; 2]| Vec3::new(((a[0] + b[0]) / 2.0) as f32, 0.0, ((a[1] + b[1]) / 2.0) as f32);
    let st = mid(g.tower_legs["south"].east, g.tower_legs["south"].west);
    let along = (st - ws).normalize_or(Vec3::NEG_Z);
    let side = Vec3::new(-along.z, 0.0, along.x);
    let e = ws - along * 60.0 * gsf() + side * 70.0 * gsf();
    let eye = Vec3::new(e.x, surface(world, e.x, e.z).max(-0.55) + 25.0, e.z);
    out.push(("gg-south-landing".to_string(), look_at(eye, st + Vec3::new(0.0, 60.0, 0.0), aspect, far), "the Golden Gate's south approach and landing from 25 m over the ground beside it, looking at the south tower".to_string()));
    // The Ferry Building from the Embarcadero roadway west of it.
    let fb = super::water_bench::ferry_building();
    let fbv = Vec3::new(fb[0], 0.0, fb[1]);
    let e = fbv + Vec3::new(-170.0, 0.0, 60.0) * gsf();
    let eye = Vec3::new(e.x, surface(world, e.x, e.z) + EYE_HEIGHT_M, e.z);
    out.push(("ferry-building".to_string(), look_at(eye, fbv + Vec3::new(0.0, 15.0, 0.0), aspect, far), "the Ferry Building from the Embarcadero west of it, eye 1.7 m".to_string()));
    // One high view: the city from above over the western hills.
    for (n, cam, what) in super::hlod_shots::cameras(world, segs, args, aspect) {
        if n == "city-above" {
            out.push((n, cam, what));
        }
    }
    out.extend(hill_cameras(world, aspect, far)); // hills
    out
}

/// a surveyed lat/lon on the scaled ground plan (`Frame::sf`'s projection, then the plan).
fn latlon(lat: f64, lon: f64) -> Vec3 {
    let f = eo_data::Frame::sf();
    let (x, z) = ((lon - f.center_lon) * f.m_per_deg_lon, -(lat - f.center_lat) * f.m_per_deg_lat);
    Vec3::new(x as f32 * gsf(), 0.0, z as f32 * gsf())
}

/// the steepest 20 m (on the plan) of a named street, within `radius`
/// m of `near` when given: (low end, high end) on the plan, and the grade drawn.
fn steepest_stretch(world: &World, name: &str, near: Option<(Vec3, f32)>) -> Option<(Vec3, Vec3, f64)> {
    let mut p = eo_player::routes::street_polyline(&world.city, name, 2.0);
    if let Some((c, r)) = near {
        p.retain(|q| (q[0] as f32 - c.x).hypot(q[1] as f32 - c.z) <= r);
    }
    // Off its bridges (Mount Vernon Avenue crosses the freeway: the ground under a
    // bridge is the cut it spans) and their abutments, 60 m (plan) each way.
    let bridges: Vec<[f64; 2]> = world.city.roads.iter().filter(|r| r.bridge && eo_data::standards::is_vehicle_way(&r.highway)).flat_map(|r| r.points64.iter().copied()).collect();
    let clear = 60.0 * gsf() as f64;
    p.retain(|q| !bridges.iter().any(|b| (b[0] - q[0]).hypot(b[1] - q[1]) < clear));
    if p.len() < 3 {
        return None;
    }
    let (g, i) = eo_player::routes::steepest_grade(&p, &world.elevation, 20.0);
    let mut j = i;
    while j + 1 < p.len() && eo_player::routes::polyline_length(&p[i..=j]) < 20.0 {
        j += 1;
    }
    let (a, b) = (Vec3::new(p[i][0] as f32, 0.0, p[i][1] as f32), Vec3::new(p[j][0] as f32, 0.0, p[j][1] as f32));
    let (ya, yb) = (world.elevation.ground(a.x as f64, a.z as f64), world.elevation.ground(b.x as f64, b.z as f64));
    Some(if ya <= yb { (a, b, g) } else { (b, a, g) })
}

/// the hill cards (see the header).
pub fn hill_cameras(world: &World, aspect: f32, far: f32) -> Vec<(String, Camera, String)> {
    let mut out = vec![];
    let on_ground = |p: Vec3, up: f32| Vec3::new(p.x, surface(world, p.x, p.z) + up, p.z);
    let streets: [(&str, &str, Option<(Vec3, f32)>, bool); 4] = [
        ("hill-mount-vernon-up", "Mount Vernon Avenue", None, true),
        ("hill-mount-vernon-down", "Mount Vernon Avenue", None, false),
        ("hill-filbert-up", "Filbert Street", Some((latlon(37.8003, -122.4183), 140.0 * gsf())), true),
        ("hill-22nd-street-up", "22nd Street", Some((latlon(37.7553, -122.4278), 140.0 * gsf())), true),
    ];
    for (n, street, near, up) in streets {
        let Some((lo, hi, g)) = steepest_stretch(world, street, near) else { continue };
        let dir = (hi - lo).normalize_or(Vec3::X);
        // 35 m (plan) below / above the steepest 20 m, looking 40 m past its other end, eye 1.7 m.
        let (eye, at) = if up { (lo - dir * 35.0, hi + dir * 40.0) } else { (hi + dir * 35.0, lo - dir * 40.0) };
        let (eye, at) = (on_ground(eye, EYE_HEIGHT_M), on_ground(at, EYE_HEIGHT_M));
        out.push((n.to_string(), look_at(eye, at, aspect, far), format!("{street}: eye 1.7 m on the street {} its steepest 20 m (drawn {:.1} % on this process's heights), looking {}", if up { "below" } else { "above" }, g * 100.0, if up { "up" } else { "down" })));
    }
    let twin = latlon(37.7546, -122.4474);
    let twin = Vec3::new(twin.x, world.elevation.ground(twin.x as f64, twin.z as f64) as f32, twin.z);
    let bay = latlon(37.8150, -122.4250);
    let eye = Vec3::new(bay.x, eo_data::SEA_LEVEL_Y as f32 + 30.0, bay.z);
    out.push(("hill-skyline-from-bay".to_string(), look_at(eye, twin, aspect, far.max(14_000.0)), "the city's hills from the bay north of the city (by Alcatraz), eye 30 m over the water, looking south at Twin Peaks' north summit over Russian and Nob Hills".to_string()));
    let dolores = on_ground(latlon(37.7590, -122.4275), 25.0);
    out.push(("hill-twin-peaks-from-dolores".to_string(), look_at(dolores, twin, aspect, far.max(6_000.0)), "Twin Peaks from 25 m over the top of Dolores Park (20th and Church; at eye level the park's own slope fills the view)".to_string()));
    let castro = on_ground(latlon(37.7609, -122.4350), EYE_HEIGHT_M);
    out.push(("hill-twin-peaks-from-castro".to_string(), look_at(castro, twin, aspect, far.max(6_000.0)), "Twin Peaks from Castro and Market, eye 1.7 m".to_string()));
    out
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    ctx: &eo_render::GpuContext,
    renderer: &Renderer,
    world: &World,
    models: &mut crate::models::ModelRuntime,
    props: &mut crate::props::PropsRuntime,
    agents: &mut crate::agents::AgentRuntime,
    segs: &[Segment],
    args: &Args,
    color: &wgpu::Texture,
    attach: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    targets: &Targets,
    dir: &std::path::Path,
    label: &str,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let aspect = color.width() as f32 / color.height() as f32;
    let only = std::env::var("EO_GS_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k)));
    let mut manifest = vec![];
    renderer.look.set_time_of_day(args.time_of_day);
    for (name, cam, what) in cameras(world, segs, args, aspect) {
        if !want(&name) {
            continue;
        }
        let t = std::time::Instant::now();
        let settle_ms = settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 900.0)?;
        crate::props::sync_parked(ctx, renderer, props, agents, &cam);
        let p = dir.join(format!("capture-gs-{label}-{name}.png"));
        capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
        println!("wrote {} (eye {:.1} {:.1} {:.1}; settle {settle_ms:.0} ms, {:.1} s)", p.display(), cam.eye.x, cam.eye.y, cam.eye.z, t.elapsed().as_secs_f64());
        manifest.push(serde_json::json!({
            "file": p.file_name().map(|f| f.to_string_lossy().to_string()),
            "name": name, "what": what,
            "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "yaw": cam.yaw, "pitch": cam.pitch, "fov_y_deg": cam.fov_y_deg, "far": cam.far, "aspect": cam.aspect,
            "settle_ms": settle_ms,
        }));
    }
    let settings = serde_json::json!({
        "label": label,
        "args": std::env::args().collect::<Vec<_>>(),
        "ground_scale": eo_data::groundscale::scale(),
        "height_scale": eo_data::groundscale::height_scale(), // hills
        "low_camera": eo_data::groundscale::low_camera(),
        "width": color.width(), "height": color.height(), "msaa": args.msaa,
        "shots": manifest,
    });
    let p = dir.join(format!("capture-gs-{label}-manifest.json"));
    std::fs::write(&p, serde_json::to_string_pretty(&settings)?)?;
    println!("wrote {}", p.display());
    Ok(())
}
