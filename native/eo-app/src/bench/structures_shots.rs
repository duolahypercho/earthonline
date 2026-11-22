//! The grade-separated roads' capture card (`--bench --structures-shots --out
//! DIR --label NAME`): four places, each from the street (eye 1.7 m) and from
//! above, at 15:00 under the clear preset with weather seed 7:
//!
//! * `i80`: the I-80 approach (the built Route 80 deck nearest 5th Street at
//!   Harrison Street);
//! * `central`: the Central Freeway (its built deck nearest Mission Street);
//! * `i280`: an I-280 interchange: Selby Street under I-280 (the Southern
//!   Embarcadero Freeway) at its middle column standing in Selby Street, the
//!   lower ramp on T bents beside it;
//! * `presidio`: the Presidio Parkway.
//!
//! A place whose chains are all held takes the longest built deck no other
//! place uses, and its card says so. The street view stands on a street the
//! deck crosses, 28 m along it from the deck's centreline and looking back
//! under it (on Selby Street: on the street at the column, looking along it);
//! the elevated view stands 40 m over the deck, 70 m to its side (the lower
//! ground's) and 70 m back along it, looking at the deck.
//!
//! The poses are placed on the structures (built here for the poses alone
//! when the run has no `--structures`), so a run with and without them takes
//! the very same cameras. Output `capture-structures-<label>-<name>.png`
//! and `capture-structures-<label>.json` (every card's eye, place, chain and
//! what it shows); `EO_STRUCTURES_SHOTS=a,b` keeps the cards whose name
//! contains one of them; `EO_STRUCTURES_SHOTS_HOUR=H` takes the cards at
//! another hour than 15:00 (the JSON records the hour);
//! `EO_STRUCTURES_SHOTS_HDR=1` also writes each card's scene luminance before
//! the exposure and the tone curve (`.lum`, `write_luminance`), for ratios.
//!
//! Stage A8's cards are taken only when `EO_STRUCTURES_SHOTS` names them
//! (`a8_cards`): `join-leg-214`, `join-leg-215`, `join-leg-service` (a
//! junction where a structure end became a leg, from the street and from
//! above), `join-raised-306`, `-132`, `-203` (a raised stub's approach),
//! `thirteenth` (the Central Freeway over 13th Street) and `tunnel-macarthur-
//! south`, `-north`, `tunnel-broadway-west`, `-east` (`-out` 45 m out on the
//! approach looking at the portal, `-in` 40 % into the bore looking back at
//! it, `-above` 30 m over the portal and 45 m out). They stand on the street datum and on a tunnel world built for the
//! poses when the run has none, so runs with and without `--structures` /
//! `--tunnels` take the same cameras.

use super::structures_bench::{at, crossed_name, drawn_deck, junction, named, poses_world, street_point, surface};
use super::{EYE_HEIGHT_M, capture, settle_p, yaw_of};
use crate::{Args, World};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::sky::{SkyOptions, WeatherMode};
use eo_render::{Renderer, Targets, wgpu};
use eo_sky::Kind;
use eo_structures::{Chain, StructureWorld};
use glam::Vec3;
use std::collections::HashSet;
use std::sync::Arc;

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--structures-shots")
}

/// 15:00, clear, weather seed 7.
const HOUR: f64 = 15.0;
const SEED: u32 = 7;

/// The cards' hour: `HOUR`, or `EO_STRUCTURES_SHOTS_HOUR` (0-24) when given.
fn hour() -> f64 {
    std::env::var("EO_STRUCTURES_SHOTS_HOUR").ok().and_then(|v| v.trim().parse::<f64>().ok()).filter(|h| (0.0..=24.0).contains(h)).unwrap_or(HOUR)
}
/// The street view's distance along the crossed street from the deck's centreline.
const STREET_BACK_M: f32 = 28.0;
/// The elevated view: over the deck, to its side, back along it.
const ELEVATED_UP_M: f32 = 40.0;
const ELEVATED_SIDE_M: f32 = 70.0;
const ELEVATED_BACK_M: f32 = 70.0;

struct Place {
    name: &'static str,
    label: &'static str,
    /// Chain names that make it.
    names: &'static [&'static str],
}

const PLACES: [Place; 4] = [
    Place { name: "i80", label: "the I-80 approach", names: &["Route 80"] },
    Place { name: "central", label: "the Central Freeway", names: &["Central Freeway"] },
    Place { name: "i280", label: "an I-280 interchange (Selby Street)", names: &["Southern Embarcadero Freeway", "John F Foran Freeway"] },
    Place { name: "presidio", label: "the Presidio Parkway", names: &["Presidio Parkway"] },
];

fn look_at(eye: Vec3, at: Vec3, aspect: f32, far: f32) -> Camera {
    let to = at - eye;
    Camera { far, ..Camera::new(eye, yaw_of(to), (to.y / Vec3::new(to.x, 0.0, to.z).length().max(1e-3)).atan(), aspect) }
}

/// The card's chain and station: the place's built deck nearest its anchor
/// (else its longest), else the longest built deck no place uses.
fn pick<'a>(world: &World, sw: &'a StructureWorld, p: &Place, taken: &HashSet<u32>) -> Option<(&'a Chain, f64, Option<Vec3>, bool)> {
    let mine: Vec<&Chain> = sw.chains().iter().filter(|c| drawn_deck(c) && p.names.iter().any(|n| named(c, n))).collect();
    let anchor: Option<Vec3> = match p.name {
        "i80" => junction(world, "5th Street", "Harrison Street"),
        "central" => {
            // where a built Central Freeway deck crosses Mission Street.
            mine.iter().find_map(|c| c.crossings.iter().find(|x| x.chain.is_none() && crossed_name(&x.over) == "Mission Street").map(|x| at(c, (x.s0 + x.s1) / 2.0).0))
        }
        "i280" => {
            // the middle of the columns standing in Selby Street.
            let mut cols: Vec<_> = sw.all_column_footprints().into_iter().filter(|f| f.street.as_ref().is_some_and(|s| s.name == "Selby Street")).collect();
            cols.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.z.total_cmp(&b.z)));
            cols.get(cols.len() / 2).map(|f| Vec3::new(f.x as f32, 0.0, f.z as f32))
        }
        _ => None,
    };
    let longest = |cs: &mut dyn Iterator<Item = &'a Chain>| cs.max_by(|a, b| a.length().total_cmp(&b.length()).then(b.id.cmp(&a.id)));
    let chosen = match anchor {
        Some(a) => mine.iter().min_by(|x, y| x.project(a.x as f64, a.z as f64).1.total_cmp(&y.project(a.x as f64, a.z as f64).1)).copied(),
        None => longest(&mut mine.iter().copied()),
    };
    if let Some(c) = chosen {
        let s = match anchor {
            Some(a) => c.project(a.x as f64, a.z as f64).0,
            None => c.length() / 2.0,
        };
        return Some((c, s.clamp(c.length().min(20.0), (c.length() - 20.0).max(c.length().min(20.0))), anchor, false));
    }
    let sub = longest(&mut sw.chains().iter().filter(|c| drawn_deck(c) && !taken.contains(&c.id) && !PLACES.iter().any(|q| q.names.iter().any(|n| named(c, n)))))?;
    Some((sub, sub.length() / 2.0, None, true))
}

/// The cards: (name, camera, what), in capture order.
fn cards(world: &World, sw: &StructureWorld, aspect: f32, far: f32) -> Vec<(String, Camera, String)> {
    let mut out = vec![];
    let mut taken = HashSet::new();
    for p in &PLACES {
        let Some((c, s, anchor, substitute)) = pick(world, sw, p, &taken) else {
            eprintln!("structures-shots: no built deck for {}; its cards are left out", p.label);
            continue;
        };
        taken.insert(c.id);
        let place = if substitute { format!("{} is held; standing in: {} (chain {}, the longest built deck no other card uses)", p.label, c.name(), c.id) } else { format!("{} ({}, chain {})", p.label, c.name(), c.id) };
        // From the street.
        let selby = p.name == "i280" && anchor.is_some();
        let street = if selby {
            let a = anchor.unwrap();
            street_point(world, "Selby Street", a).map(|(q, d)| {
                let (_, t, _) = at(c, s);
                let d = if d.dot(t) < 0.0 { -d } else { d };
                let eye = Vec3::new(q.x, surface(world, q.x, q.z) + EYE_HEIGHT_M, q.z);
                (look_at(eye, eye + d * 40.0 + Vec3::Y * 4.0, aspect, far), format!("on Selby Street at its middle column, looking along it under the deck"))
            })
        } else {
            // the paved crossing nearest the station, on the crossed street.
            let cross = c.crossings.iter().filter(|x| x.chain.is_none()).min_by(|a, b| ((a.s0 + a.s1) / 2.0 - s).abs().total_cmp(&((b.s0 + b.s1) / 2.0 - s).abs()));
            cross.and_then(|x| {
                let sc = (x.s0 + x.s1) / 2.0;
                let (pc, _, _) = at(c, sc);
                let name = crossed_name(&x.over);
                street_point(world, name, pc).map(|(q, d)| {
                    let e = q + d * STREET_BACK_M;
                    let eye = Vec3::new(e.x, surface(world, e.x, e.z) + EYE_HEIGHT_M, e.z);
                    let soffit = pc.y - c.depth_at(sc) as f32;
                    (look_at(eye, Vec3::new(q.x, (soffit + eye.y) * 0.5, q.z), aspect, far), format!("on {name}, {STREET_BACK_M} m from the deck's centreline, looking back under it"))
                })
            })
        }
        .unwrap_or_else(|| {
            let (pc, _, n) = at(c, s);
            let half = c.half_at(s) as f32;
            let e = pc + n * (half + 25.0);
            let eye = Vec3::new(e.x, surface(world, e.x, e.z) + EYE_HEIGHT_M, e.z);
            (look_at(eye, Vec3::new(pc.x, pc.y - 2.0, pc.z), aspect, far), format!("{:.0} m to the side of the deck, on the ground", half + 25.0))
        });
        out.push((format!("{}-street", p.name), street.0, format!("{place}: {}", street.1)));
        // From above.
        let (pc, t, n) = at(c, s);
        let side = if surface(world, (pc + n * ELEVATED_SIDE_M).x, (pc + n * ELEVATED_SIDE_M).z) <= surface(world, (pc - n * ELEVATED_SIDE_M).x, (pc - n * ELEVATED_SIDE_M).z) { n } else { -n };
        let e = pc + side * ELEVATED_SIDE_M - t * ELEVATED_BACK_M;
        let eye = Vec3::new(e.x, pc.y + ELEVATED_UP_M, e.z);
        out.push((format!("{}-elevated", p.name), look_at(eye, Vec3::new(pc.x, pc.y - 3.0, pc.z), aspect, far), format!("{place}: {ELEVATED_UP_M} m over the deck, {ELEVATED_SIDE_M} m to its side and {ELEVATED_BACK_M} m back along it, at station {s:.0}")));
    }
    out
}

/// Stage A8's places (A6A7 hand-off): the junctions where a structure end
/// became a leg (no kerb across its mouth), the Central Freeway over 13th
/// Street, and the MacArthur and Broadway tunnels' portals (each from 45 m
/// out and from 40 % into the bore, looking back at that portal).
const JOIN_LEGS: [(&str, [f64; 2]); 3] = [("join-leg-214", [2986.6, 3407.4]), ("join-leg-215", [2260.4, 8680.9]), ("join-leg-service", [-2913.7, 662.9])];
const THIRTEENTH: [f64; 2] = [450.0, 1165.0];
const PORTALS: [(&str, [f64; 2]); 4] = [("macarthur-south", [-5518.3, -2016.3]), ("macarthur-north", [-5510.9, -2509.9]), ("broadway-west", [171.0, -2534.0]), ("broadway-east", [894.0, -2653.0])];
/// The joins' raised stubs (stage A6's rescued chains, by the OSM way they
/// carry): I-80 eastbound's lower approach over Beale Street (chain 306 in
/// the A6A7 census), the James Lick Freeway (132), the Presidio Parkway (203).
const RAISED: [(&str, &str); 3] = [("join-raised-306", "661905446"), ("join-raised-132", "27183377"), ("join-raised-203", "349663310")];
/// How far out the portal and join views stand, and how far into a bore.
const PORTAL_OUT_M: f64 = 45.0;
const PORTAL_ABOVE_M: f32 = 30.0;
const BORE_IN: f64 = 0.4;
const JOIN_BACK_M: f32 = 32.0;

/// The street datum under (x, z) (the bare ground plus the street's road
/// lift): the same with and without the joins (which raise and lower stubs),
/// so their cards stand at one height in both runs.
fn datum(world: &World, x: f32, z: f32) -> f32 {
    match &world.street {
        Some(s) => (s.world.terrain.ground(x as f64, z as f64) + s.world.o.road_lift) as f32,
        None => world.elevation.ground(x as f64, z as f64) as f32,
    }
}

/// The tunnels' poses' world: the run's own when it has bores, else one
/// built with `--tunnels` for the poses alone (so a run without tunnels
/// takes the very same cameras).
fn tunnel_poses(world: &World) -> Option<Arc<StructureWorld>> {
    if let Some(s) = world.structures.as_ref().filter(|s| !s.bores().is_empty()) {
        return Some(s.clone());
    }
    let streets = world.street.as_ref()?;
    let standards = eo_data::standards::CaStandards::load(&world.paths).ok()?;
    let w = StructureWorld::build(&streets.world, &world.city, &standards, eo_landmarks::LandmarksJs::get(), eo_structures::StructureOptions { tunnels: true }).ok()?;
    eprintln!("structures-shots: tunnels built for the poses only (not drawn)");
    Some(Arc::new(w))
}

/// Stage A8's cards (named `join-*`, `thirteenth-*`, `tunnel-*`): only
/// when `EO_STRUCTURES_SHOTS` names them, so the card's default set is the
/// eight above.
fn a8_cards(world: &World, sw: &StructureWorld, aspect: f32, far: f32, want: &dyn Fn(&str) -> bool) -> Vec<(String, Camera, String)> {
    let mut out = vec![];
    let eye_at = |p: Vec3| Vec3::new(p.x, datum(world, p.x, p.z) + EYE_HEIGHT_M, p.z);
    // a leg's junction: from 32 m out along the paved street opposite the
    // structure, looking at the mouth; and from 30 m over it.
    for (name, p) in JOIN_LEGS {
        if !want(name) {
            continue;
        }
        let c = sw.chains().iter().filter(|c| c.is_built()).min_by(|a, b| a.project(p[0], p[1]).1.total_cmp(&b.project(p[0], p[1]).1));
        let Some(c) = c else { continue };
        let (s, _) = c.project(p[0], p[1]);
        let (_, t, _) = at(c, s.clamp(0.0, c.length()));
        // the chain runs away from the junction: the street side is behind it.
        let away = if s < c.length() / 2.0 { t } else { -t };
        let mouth = Vec3::new(p[0] as f32, 0.0, p[1] as f32);
        let eye = eye_at(mouth - away * JOIN_BACK_M);
        let tgt = Vec3::new(mouth.x, datum(world, mouth.x, mouth.z) + 1.0, mouth.z);
        out.push((format!("{name}-street"), look_at(eye, tgt + away * 10.0, aspect, far), format!("the junction at ({:.0}, {:.0}) where chain {} ({}) is a leg: {JOIN_BACK_M} m out along the street, looking at its mouth", p[0], p[1], c.id, c.name())));
        let up = Vec3::new(mouth.x, tgt.y + 30.0, mouth.z) - away * 30.0;
        out.push((format!("{name}-above"), look_at(up, tgt, aspect, far), format!("the same junction from 30 m over the street, 30 m back")));
    }
    // a raised stub: from its street, 32 m back from the chain's paved end, beside the stub.
    for (name, way) in RAISED {
        if !want(name) {
            continue;
        }
        // (held without the joins: its stub is the same street either way.)
        let Some(c) = sw.chains().iter().find(|c| c.ways().contains(&way)) else { continue };
        let Some(k) = (0..2).find(|&k| c.ends[k].kind == eo_structures::EndKind::Paved) else { continue };
        let s = if k == 0 { 0.0 } else { c.length() };
        let (pc, t, n) = at(c, s);
        let out_dir = if k == 0 { -t } else { t };
        let eye = eye_at(pc + out_dir * JOIN_BACK_M + n * 6.0);
        out.push((format!("{name}-street"), look_at(eye, pc - out_dir * 20.0 + Vec3::Y * 2.0, aspect, far), format!("chain {} ({}): its paved end's stub, from {JOIN_BACK_M} m out, looking up the approach", c.id, c.name())));
    }
    // the Central Freeway's columns in 13th Street's parking lanes.
    if want("thirteenth") {
        let p = Vec3::new(THIRTEENTH[0] as f32, 0.0, THIRTEENTH[1] as f32);
        if let Some((q, d)) = street_point(world, "13th Street", p) {
            let eye = eye_at(q - d * 40.0);
            out.push(("thirteenth-street".to_string(), look_at(eye, Vec3::new(q.x, eye.y + 3.0, q.z), aspect, far), "13th Street under the Central Freeway, 40 m along it from (450, 1165), looking along it".to_string()));
            let up = Vec3::new(q.x, eye.y + 35.0, q.z) - d * 50.0 + Vec3::new(-d.z, 0.0, d.x) * 25.0;
            out.push(("thirteenth-above".to_string(), look_at(up, Vec3::new(q.x, eye.y, q.z), aspect, far), "the Central Freeway over 13th Street from 35 m up".to_string()));
        }
    }
    // the portals: from 45 m out along the approach, and from 40 % into the bore looking back at them.
    let portal_wanted = |name: &str| ["out", "in", "above"].iter().any(|k| want(&format!("{name}-{k}")));
    if PORTALS.iter().any(|(n, _)| portal_wanted(&format!("tunnel-{n}"))) {
        if let Some(tw) = tunnel_poses(world) {
            for (name, p) in PORTALS {
                let name = format!("tunnel-{name}");
                if !portal_wanted(&name) {
                    continue;
                }
                // the bore end nearest the portal.
                let mut best: Option<(&eo_structures::Bore, bool, f64)> = None;
                for b in tw.bores() {
                    for (end, q) in [(false, b.points.first()), (true, b.points.last())] {
                        let Some(q) = q else { continue };
                        let d = (q.x - p[0]).hypot(q.z - p[1]);
                        if (if end { b.portal[1] } else { b.portal[0] }) && best.is_none_or(|bb| d < bb.2) {
                            best = Some((b, end, d));
                        }
                    }
                }
                let Some((b, end, d)) = best.filter(|b| b.2 < 60.0) else {
                    eprintln!("structures-shots: no portal near {name}; its cards are left out");
                    continue;
                };
                let Some(c) = tw.chain(b.chain) else { continue };
                let face = if end { b.s1 } else { b.s0 };
                let into = if end { -1.0 } else { 1.0 };
                let (pf, t, _) = at(c, face);
                // on the approach's roadway, or past the chain's end on along its last direction on the street.
                let s_out = face - into * PORTAL_OUT_M;
                let eye = if (0.0..=c.length()).contains(&s_out) {
                    let (po, _, _) = at(c, s_out);
                    Vec3::new(po.x, po.y + EYE_HEIGHT_M, po.z)
                } else {
                    let s_end = s_out.clamp(0.0, c.length());
                    let (pe, te, _) = at(c, s_end);
                    let q = pe - te * (into as f32) * (s_out - s_end).abs() as f32;
                    Vec3::new(q.x, surface(world, q.x, q.z) + EYE_HEIGHT_M, q.z)
                };
                out.push((format!("{name}-out"), look_at(eye, Vec3::new(pf.x, pf.y + 3.0, pf.z), aspect, far), format!("{} (chain {}): {PORTAL_OUT_M} m out from its portal ({d:.0} m from the hand-off's point), on the approach's roadway", c.name(), c.id)));
                let s_in = face + into * BORE_IN * (b.s1 - b.s0);
                let (pi, _, _) = at(c, s_in);
                let eye = Vec3::new(pi.x, pi.y + EYE_HEIGHT_M, pi.z);
                let back = -t * into as f32;
                out.push((format!("{name}-in"), look_at(eye, eye + back * 40.0 + Vec3::Y * 0.6, aspect, far), format!("{} (chain {}): {:.0} % into the bore ({:.0} m), looking back at the portal", c.name(), c.id, BORE_IN * 100.0, (s_in - face).abs())));
                // from above: the portal, its cut and the hillside round it (holes, buried mouths).
                let outward = Vec3::new(-t.x, 0.0, -t.z) * into as f32;
                let up = Vec3::new(pf.x, pf.y + PORTAL_ABOVE_M, pf.z) + outward * PORTAL_ABOVE_M * 1.5 + Vec3::new(t.z, 0.0, -t.x) * 12.0;
                out.push((format!("{name}-above"), look_at(up, Vec3::new(pf.x, pf.y + 2.0, pf.z), aspect, far), format!("{} (chain {}): its portal from {PORTAL_ABOVE_M} m over its roadway and {:.0} m out", c.name(), c.id, PORTAL_ABOVE_M * 1.5)));
            }
        }
    }
    out
}

/// The scene's linear luminance (the HDR target the capture's last frame
/// was lit into, before the exposure and the tone curve) as `EOLM`, width,
/// height (u32 LE), then one f32 LE a pixel, row by row: for luminance
/// ratios between surfaces of one card (`EO_STRUCTURES_SHOTS_HDR=1`).
fn write_luminance(ctx: &eo_render::GpuContext, renderer: &Renderer, w: u32, h: u32, path: &std::path::Path) -> Result<()> {
    let view = renderer.look.hdr_target(w, h);
    let row = (w * 8).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("capture-hdr"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        view.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    eo_render::pace::submit(&ctx.queue, [enc.finish()]); // counted
    let raw = eo_render::read_mapped(ctx, &buf)?;
    let half = |o: usize| eo_render::ambient_bake::f16_value(u16::from_le_bytes([raw[o], raw[o + 1]]));
    let mut out = Vec::with_capacity(12 + (w * h * 4) as usize);
    out.extend_from_slice(b"EOLM");
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    for y in 0..h as usize {
        for x in 0..w as usize {
            let o = y * row as usize + x * 8;
            let l = 0.2126 * half(o) + 0.7152 * half(o + 2) + 0.0722 * half(o + 4);
            out.extend_from_slice(&l.to_le_bytes());
        }
    }
    std::fs::write(path, out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    ctx: &eo_render::GpuContext,
    renderer: &Renderer,
    world: &World,
    models: &mut crate::models::ModelRuntime,
    props: &mut crate::props::PropsRuntime,
    agents: &mut crate::agents::AgentRuntime,
    args: &Args,
    color: &wgpu::Texture,
    attach: &wgpu::TextureView,
    resolve: Option<&wgpu::TextureView>,
    targets: &Targets,
    dir: &std::path::Path,
    label: &str,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let base = args.look.sky;
    let legacy = base == SkyOptions::off();
    let aspect = color.width() as f32 / color.height() as f32;
    let Some(sw) = poses_world(world) else {
        anyhow::bail!("structures-shots: no street world to place the structures on");
    };
    let only = std::env::var("EO_STRUCTURES_SHOTS").ok();
    let want = |n: &str| only.as_deref().is_none_or(|o| o.split(',').any(|k| n.contains(k.trim())));
    let mut written = vec![];
    let hour = hour();
    let mut all = cards(world, &sw, aspect, args.far);
    if only.is_some() {
        all.extend(a8_cards(world, &sw, aspect, args.far, &want)); // stage A8's places, when named
    }
    for (name, cam, what) in all {
        if !want(&name) {
            continue;
        }
        let t = std::time::Instant::now();
        eprintln!("structures-shots: {name}: settling at eye ({:.0}, {:.0}, {:.0})", cam.eye.x, cam.eye.y, cam.eye.z);
        renderer.look.set_time_of_day(hour);
        renderer.look.set_sky_options(if legacy { SkyOptions::off() } else { SkyOptions { weather: WeatherMode::Preset(Kind::Clear), seed: SEED, ..base } });
        settle_p(ctx, renderer, world, models, Some(&mut *props), &cam, 600.0)?;
        // (stage N1) `EO_STRUCTURES_SHOTS_BORE_CAR=1`: a car on the road 14 m ahead of a tunnel card's
        // eye (the evidence of a car in a bore lit by the bore; marked as the player's car is).
        if std::env::var_os("EO_STRUCTURES_SHOTS_BORE_CAR").is_some() && name.starts_with("tunnel-") {
            let f = cam.forward();
            let ahead = glam::Vec3::new(f.x, 0.0, f.z).normalize_or_zero() * 14.0;
            agents.show_car([(cam.eye.x + ahead.x) as f64, (cam.eye.y - super::EYE_HEIGHT_M) as f64, (cam.eye.z + ahead.z) as f64], f.x.atan2(f.z) + std::f32::consts::PI);
        }
        crate::props::sync_parked(ctx, renderer, props, agents, &cam);
        let p = dir.join(format!("capture-structures-{label}-{name}.png"));
        capture(ctx, renderer, color, attach, resolve, &targets.depth, &cam, &p)?;
        if std::env::var_os("EO_STRUCTURES_SHOTS_HDR").is_some() {
            write_luminance(ctx, renderer, color.width(), color.height(), &p.with_extension("lum"))?;
        }
        println!("wrote {} (eye {:.1} {:.1} {:.1}; {:.1} s)", p.display(), cam.eye.x, cam.eye.y, cam.eye.z, t.elapsed().as_secs_f64());
        written.push(serde_json::json!({ "name": name, "file": p.file_name().map(|f| f.to_string_lossy().to_string()), "eye": [cam.eye.x, cam.eye.y, cam.eye.z], "yaw": cam.yaw, "pitch": cam.pitch, "hour": hour, "weather": if legacy { "none (--no-sky)".to_string() } else { format!("clear, seed {SEED}") }, "what": what }));
    }
    let manifest = dir.join(format!("capture-structures-{label}.json"));
    std::fs::write(&manifest, serde_json::to_vec_pretty(&serde_json::json!({ "label": label, "cards": written, "structures": crate::structures::json(args, world), "lighting": crate::quality::lighting_json(args) }))?)?;
    println!("wrote {}", manifest.display());
    renderer.look.set_time_of_day(args.time_of_day);
    renderer.look.set_sky_options(base);
    Ok(())
}
