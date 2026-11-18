//! the far city's fixed card set (`--bench --far-shots --label NAME`):
//! the six `--hlod-shots` views (the hill view lifted clear of the buildings
//! round it), street-level views where the far city fills the frame, and three
//! of them again at golden hour. Every card is settled (cells round the camera
//! and every district resident) before it is drawn; the camera, hour and
//! settings of every card go to `capture-far-<label>-manifest.json`.
//! `EO_FAR_SHOTS=a,b` keeps only the cards whose name contains a or b.
//! `--far-diag` also writes `far-diag-<label>.json`: the distribution of wall,
//! glass and roof colours over the kit buildings as the palette, rung 2, the
//! cell atlas and the district hold them (the far-colour diagnosis).

use super::{Segment, capture, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::{Renderer, Targets, wgpu};
use glam::{Vec2, Vec3};

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--far-shots")
}

fn diag_requested() -> bool {
    std::env::args().any(|a| a == "--far-diag")
}

fn look_at(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    Camera { far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect) }
}

fn ground(world: &World, p: Vec3) -> f32 {
    world.elevation.ground(p.x as f64, p.z as f64) as f32
}

/// The highest building top (ground at its centroid + its height) whose
/// footprint box comes within `r` metres of `p` (ground plane).
fn tallest_near(world: &World, p: Vec3, r: f32) -> f32 {
    let mut top = f32::MIN;
    for b in &world.city.buildings {
        let Some(first) = b.ring.first() else { continue };
        let (mut mn, mut mx) = (Vec2::from(*first), Vec2::from(*first));
        for q in &b.ring {
            mn = mn.min(Vec2::from(*q));
            mx = mx.max(Vec2::from(*q));
        }
        let d = (mn - Vec2::new(p.x, p.z)).max(Vec2::new(p.x, p.z) - mx).max(Vec2::ZERO).length();
        if d <= r {
            let c = (mn + mx) * 0.5;
            top = top.max(world.elevation.ground(c.x as f64, c.y as f64) as f32 + b.height);
        }
    }
    top
}

/// An elevated eye lifted until it clears every building within 60 m by 15 m.
fn clear_above(world: &World, eye: Vec3) -> Vec3 {
    Vec3::new(eye.x, eye.y.max(tallest_near(world, eye, 60.0) + 15.0), eye.z)
}

/// The points of every road of that name, as one list of segments.
fn road(world: &World, name: &str) -> Vec<(Vec2, Vec2)> {
    let mut out = vec![];
    for r in world.city.roads.iter().filter(|r| r.name == name && !r.bridge && !r.tunnel) {
        for w in r.points.windows(2) {
            out.push((Vec2::from(w[0]), Vec2::from(w[1])));
        }
    }
    out
}

/// The road point nearest `want` metres from `from` (ground plane), on a
/// segment's vertex or midpoint, with the segment's direction.
fn road_point_at(segs: &[(Vec2, Vec2)], from: Vec2, want: f32) -> Option<(Vec2, Vec2)> {
    let mut best: Option<(f32, Vec2, Vec2)> = None;
    for &(a, b) in segs {
        let dir = (b - a).normalize_or_zero();
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let p = a.lerp(b, t);
            let e = (p.distance(from) - want).abs();
            if best.is_none_or(|(be, _, _)| e < be) {
                best = Some((e, p, dir));
            }
        }
    }
    best.map(|(_, p, d)| (p, d))
}

/// A street-level eye on a road: 2.2 m over the ground, moved sideways off a
/// footprint if it stands in one.
fn street_eye(world: &World, p: Vec2, side: Vec2) -> Vec3 {
    let mut q = p;
    for k in 0..12 {
        let e = Vec3::new(q.x, 0.0, q.y);
        if super::eye_inside_building(Vec3::new(e.x, ground(world, e) + 2.2, e.z)).is_none() {
            break;
        }
        q = p + side * (1.5 * (k + 1) as f32) * if k % 2 == 0 { 1.0 } else { -1.0 };
    }
    let e = Vec3::new(q.x, 0.0, q.y);
    Vec3::new(e.x, ground(world, e) + 2.2, e.z)
}

/// The cards: (name, camera, what). Golden hour repeats are added by `run`.
pub fn cameras(world: &World, segs: &[Segment], args: &Args, aspect: f32) -> Vec<(String, Camera, String)> {
    let far = args.far;
    let mut out = vec![];
    // The six far-layer views; the hill view lifted clear of the buildings round it.
    for (n, cam, what) in super::hlod_shots::cameras(world, segs, args, aspect) {
        if n == "hill-view" {
            let eye = clear_above(world, cam.eye);
            let moved = eye.y - cam.eye.y;
            out.push((n, Camera { eye, ..cam }, format!("{what}; eye lifted {moved:.0} m to clear the buildings within 60 m by 15 m")));
        } else {
            out.push((n, cam, what));
        }
    }
    let dt = super::hlod_shots::downtown(world);
    let dt2 = Vec2::new(dt.x, dt.z);
    // The far end of a long street in the western row-house grid: Noriega
    // Street eastward from 300 m in from its western end.
    let nor = road(world, "Noriega Street");
    if let Some(&(w0, _)) = nor.iter().min_by(|a, b| a.0.x.total_cmp(&b.0.x)) {
        let e0 = nor.iter().map(|s| s.1).max_by(|a, b| a.x.total_cmp(&b.x)).unwrap_or(w0 + Vec2::X);
        let dir = (e0 - w0).normalize_or(Vec2::X);
        let (p, _) = road_point_at(&nor, w0, 300.0).unwrap_or((w0, dir));
        let eye = street_eye(world, p, Vec2::new(-dir.y, dir.x));
        let at = Vec3::new(eye.x + dir.x * 1000.0, eye.y + 4.0, eye.z + dir.y * 1000.0);
        out.push(("sunset-street".into(), look_at(eye, at, aspect, far), format!("street level on Noriega Street 300 m in from its western end, looking east along it ({:.0}, {:.0})", eye.x, eye.z)));
    }
    // Up a major street toward downtown: Market Street 2.4 km from downtown, street level.
    let market = road(world, "Market Street");
    if let Some((p, d)) = road_point_at(&market, dt2, 2400.0) {
        let eye = street_eye(world, p, Vec2::new(-d.y, d.x));
        out.push(("market-up".into(), look_at(eye, Vec3::new(dt.x, eye.y + 30.0, dt.z), aspect, far), format!("street level on Market Street 2.4 km from downtown, looking at downtown ({:.0}, {:.0})", eye.x, eye.z)));
    }
    // 150 m over a long avenue about 2 km from downtown, looking down it toward downtown.
    if let Some((p, _)) = road_point_at(&market, dt2, 2000.0) {
        let g = ground(world, Vec3::new(p.x, 0.0, p.y));
        let eye = clear_above(world, Vec3::new(p.x, g + 150.0, p.y));
        out.push(("avenue-150m".into(), look_at(eye, Vec3::new(dt.x, ground(world, dt) + 20.0, dt.z), aspect, far), format!("150 m over Market Street 2 km from downtown, looking down it at downtown ({:.0}, {:.0})", eye.x, eye.z)));
    }
    // A hilltop over the western row-house grid: the highest ground within 1.5 km
    // of Noriega Street's middle, 20 m up (clear of buildings), looking at that middle.
    if !nor.is_empty() {
        let mid = nor.iter().fold(Vec2::ZERO, |a, s| a + s.0) / nor.len() as f32;
        let h = super::hlod_shots::hill_within(world, Vec3::new(mid.x, 0.0, mid.y), 1500.0);
        let eye = clear_above(world, h + Vec3::new(0.0, 20.0, 0.0));
        let at = Vec3::new(mid.x, ground(world, Vec3::new(mid.x, 0.0, mid.y)), mid.y);
        out.push(("western-hilltop".into(), look_at(eye, at, aspect, far), format!("20 m over the highest ground within 1.5 km of Noriega Street's middle ({:.0}, {:.0}, ground {:.0} m), looking at that middle", h.x, h.z, h.y)));
    }
    // Downtown towers from a few blocks away at street level: Market Street 700 m out.
    if let Some((p, d)) = road_point_at(&market, dt2, 700.0) {
        let eye = street_eye(world, p, Vec2::new(-d.y, d.x));
        out.push(("towers-street".into(), look_at(eye, Vec3::new(dt.x, eye.y + 90.0, dt.z), aspect, far), format!("street level on Market Street 700 m from downtown, looking up at the towers ({:.0}, {:.0})", eye.x, eye.z)));
    }
    // From a mid-rise roof: 35 m over Market Street 1.1 km from downtown.
    if let Some((p, _)) = road_point_at(&market, dt2, 1100.0) {
        let g = ground(world, Vec3::new(p.x, 0.0, p.y));
        let eye = Vec3::new(p.x, g + 35.0, p.y);
        out.push(("towers-roof".into(), look_at(eye, Vec3::new(dt.x, g + 70.0, dt.z), aspect, far), format!("35 m over Market Street 1.1 km from downtown (a mid-rise roof's height), looking at the towers ({:.0}, {:.0})", eye.x, eye.z)));
    }
    // 500 m over Twin Peaks looking northeast to downtown.
    {
        let h = super::hlod_shots::hill_within(world, dt, 4000.0);
        let eye = h + Vec3::new(0.0, 500.0, 0.0);
        out.push(("twin-peaks-500m".into(), look_at(eye, Vec3::new(dt.x, ground(world, dt), dt.z), aspect, far.max(6000.0)), format!("500 m over the highest ground within 4 km of downtown ({:.0}, {:.0}), looking at downtown", h.x, h.z)));
    }
    out
}

/// Golden hour is drawn for these cards as well as noon.
const GOLDEN: [&str; 3] = ["bay-bridge-skyline", "elevated-sweep-f0", "sunset-street"];

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
    let only = std::env::var("EO_FAR_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k)));
    let cams = cameras(world, segs, args, aspect);
    let mut cards = vec![];
    for (hname, hour) in [("noon", 12.0), ("golden", 18.5)] {
        for (name, cam, what) in &cams {
            if hname == "golden" && !GOLDEN.contains(&name.as_str()) {
                continue;
            }
            cards.push((format!("{name}-{hname}"), name.clone(), hour, *cam, what.clone()));
        }
    }
    // A still frame must not depend on how long the settle ran: the water clock is fixed and,
    // per card, the cloud clock is pinned and held still (as in so_shots), so on/off pairs match.
    if let Some(w) = renderer.water.get() {
        w.set_time(20.0);
        w.set_always(true);
    }
    let base_clock = args.look.sky.world_time_s;
    let mut manifest = vec![];
    for (n, name, hour, cam, what) in cards {
        if !want(&n) {
            continue;
        }
        let t = std::time::Instant::now();
        renderer.look.set_time_of_day(hour);
        renderer.look.sky.set_clock(base_clock, 0.0);
        let settle_ms = settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 900.0)?;
        crate::props::sync_parked(ctx, renderer, props, agents, &cam);
        let p = dir.join(format!("capture-far-{label}-{n}.png"));
        let blocked = super::blocked_view(&cam, super::CAPTURE_WALL_WARN_M);
        capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
        let hl = renderer.hlod.get().map(|h| h.readback(&ctx.queue));
        println!("wrote {} (eye {:.1} {:.1} {:.1}, {hour} h; settle {settle_ms:.0} ms, {:.1} s; far entries drawn/triangles {:?})", p.display(), cam.eye.x, cam.eye.y, cam.eye.z, t.elapsed().as_secs_f64(), hl.map(|c| (c[0], c[1])));
        manifest.push(serde_json::json!({
            "file": p.file_name().map(|f| f.to_string_lossy().to_string()),
            "name": name, "hour": hour, "what": what,
            "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "yaw": cam.yaw, "pitch": cam.pitch, "fov_y_deg": cam.fov_y_deg, "near": cam.near, "far": cam.far, "aspect": cam.aspect,
            "settle_ms": settle_ms,
            "blocked_view": blocked,
            "far_layer_entries_drawn_triangles_in_frustum_occluded": hl,
        }));
    }
    renderer.look.sky.set_clock(renderer.look.sky.world_time(), 1.0 / 60.0);
    let settings = serde_json::json!({
        "label": label,
        "args": std::env::args().collect::<Vec<_>>(),
        "width": color.width(), "height": color.height(), "msaa": args.msaa,
        "far_layer_on": renderer.hlod.get().is_some(),
        "far_layer": models.hlod.as_ref().map(|h| h.stats_json()),
        "look": format!("{:?}", args.look),
        "shots": manifest,
    });
    let p = dir.join(format!("capture-far-{label}-manifest.json"));
    std::fs::write(&p, serde_json::to_string_pretty(&settings)?)?;
    println!("wrote {}", p.display());
    if diag_requested() {
        diag(models, dir, label)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The far-colour diagnosis.
// ---------------------------------------------------------------------------

fn srgb(c: f32) -> f32 {
    let v = c.clamp(0.0, 1.0);
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Statistics of a set of linear colours: mean linear, mean sRGB, the sRGB
/// HSV saturation (mean, p10/p50/p90), luminance (mean, sd, p10/p50/p90),
/// warmth (sRGB R - B: mean) and the share of colours whose saturation is under 0.12.
fn stats(cs: &[[f32; 3]]) -> serde_json::Value {
    if cs.is_empty() {
        return serde_json::json!({ "n": 0 });
    }
    let n = cs.len() as f64;
    let mut mean = [0f64; 3];
    let mut ms = [0f64; 3];
    let mut sat = vec![];
    let mut lum = vec![];
    let mut warm = 0f64;
    for c in cs {
        let s = [srgb(c[0]), srgb(c[1]), srgb(c[2])];
        for k in 0..3 {
            mean[k] += c[k] as f64 / n;
            ms[k] += s[k] as f64 / n;
        }
        let mx = s[0].max(s[1]).max(s[2]);
        let mn = s[0].min(s[1]).min(s[2]);
        sat.push(if mx > 1e-4 { (mx - mn) / mx } else { 0.0 });
        lum.push(0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]);
        warm += (s[0] - s[2]) as f64 / n;
    }
    let pct = |v: &mut Vec<f32>, p: f64| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[((v.len() - 1) as f64 * p).round() as usize]
    };
    let lm = lum.iter().map(|&x| x as f64).sum::<f64>() / n;
    let lsd = (lum.iter().map(|&x| (x as f64 - lm).powi(2)).sum::<f64>() / n).sqrt();
    let low = sat.iter().filter(|&&s| s < 0.12).count() as f64 / n;
    let sm = sat.iter().map(|&x| x as f64).sum::<f64>() / n;
    serde_json::json!({
        "n": cs.len(),
        "mean_linear": mean, "mean_srgb_255": [ms[0] * 255.0, ms[1] * 255.0, ms[2] * 255.0],
        "saturation": { "mean": sm, "p10": pct(&mut sat, 0.1), "p50": pct(&mut sat, 0.5), "p90": pct(&mut sat, 0.9), "share_under_0.12": low },
        "luminance": { "mean": lm, "sd": lsd, "p10": pct(&mut lum, 0.1), "p50": pct(&mut lum, 0.5), "p90": pct(&mut lum, 0.9) },
        "warmth_srgb_r_minus_b": warm,
    })
}

/// Every eighth cell: the kit buildings' palette, rung 2 and atlas colours,
/// overall and per archetype.
fn diag(models: &mut crate::models::ModelRuntime, dir: &std::path::Path, label: &str) -> Result<()> {
    let Some(h) = models.hlod.as_ref() else { return Ok(()) };
    let Some(bctx) = h.bake_context() else {
        println!("far-diag: the bake's inputs are not in");
        return Ok(());
    };
    let t = std::time::Instant::now();
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Acc {
        wall_palette: Vec<[f32; 3]>,
        roof_palette: Vec<[f32; 3]>,
        rung_wall: Vec<[f32; 3]>,
        rung_glass: Vec<[f32; 3]>,
        rung_glass_share: Vec<f32>,
        atlas_facade: Vec<[f32; 3]>,
        atlas_glass: Vec<f32>,
        atlas_roof: Vec<[f32; 3]>,
        rhythm: Vec<[f32; 5]>,
        towers: usize,
    }
    let mut by: BTreeMap<String, Acc> = BTreeMap::new();
    let cells: Vec<u32> = (0..bctx.set.cells.len() as u32).step_by(8).collect();
    let rows: Vec<(Vec<eo_render::hlod_bake::DiagBuilding>, Vec<eo_render::hlod_bake::BuildingSummary>)> = std::thread::scope(|s| {
        let chunks: Vec<&[u32]> = cells.chunks(cells.len().div_ceil(6).max(1)).collect();
        let hs: Vec<_> = chunks
            .into_iter()
            .map(|ch| {
                let b = bctx.clone();
                s.spawn(move || ch.iter().map(|&c| (eo_render::hlod_bake::diag_cell(&b, c), eo_render::hlod_bake::load_or_bake(&b, c).0.summaries)).collect::<Vec<_>>())
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    for (ds, sums) in rows {
        let of: std::collections::HashMap<u32, &eo_render::hlod_bake::BuildingSummary> = sums.iter().map(|s| (s.group, s)).collect();
        for d in ds {
            for key in [d.archetype.clone(), "all".to_string()] {
                let a = by.entry(key).or_default();
                a.wall_palette.push(d.wall_palette);
                a.roof_palette.push(d.roof_palette);
                a.rung_wall.push(d.rung_wall);
                if d.rung_glass_share > 0.0 {
                    a.rung_glass.push(d.rung_glass);
                }
                a.rung_glass_share.push(d.rung_glass_share);
                if d.rhythm[0] > 0.0 {
                    a.rhythm.push(d.rhythm);
                }
                a.towers += d.tower as usize;
                if let Some(s) = of.get(&d.group) {
                    a.atlas_facade.push(s.facade);
                    a.atlas_glass.push(s.glass);
                    a.atlas_roof.push(s.roof);
                }
            }
        }
    }
    let mean = |v: &[f32]| if v.is_empty() { 0.0 } else { v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64 };
    let mut out = serde_json::Map::new();
    for (k, a) in &by {
        out.insert(
            k.clone(),
            serde_json::json!({
                "buildings": a.wall_palette.len(),
                "wall_palette": stats(&a.wall_palette),
                "rung2_wall_opaque": stats(&a.rung_wall),
                "rung2_glass_base": stats(&a.rung_glass),
                "rung2_glass_share_mean": mean(&a.rung_glass_share),
                "atlas_facade_mean": stats(&a.atlas_facade),
                "atlas_glass_share_mean": mean(&a.atlas_glass),
                "roof_palette": stats(&a.roof_palette),
                "atlas_roof": stats(&a.atlas_roof),
                "towers": a.towers,
                "with_rhythm": a.rhythm.len(),
                "rhythm_median_bay_storey_w_h_phase": (0..5).map(|k| { let mut v: Vec<f32> = a.rhythm.iter().map(|r| r[k]).collect(); v.sort_by(|x, y| x.total_cmp(y)); v.get(v.len() / 2).copied().unwrap_or(0.0) }).collect::<Vec<f32>>(),
            }),
        );
    }
    let p = dir.join(format!("far-diag-{label}.json"));
    std::fs::write(&p, serde_json::to_string_pretty(&serde_json::Value::Object(out))?)?;
    println!("wrote {} ({} cells sampled, {:.1} s)", p.display(), cells.len(), t.elapsed().as_secs_f64());
    Ok(())
}
