//! `--bench --occlusion-check`, the ground-truth test of the
//! occlusion cull on every bench path.
//!
//! For sampled cameras on each path (after streaming settles there):
//!
//! - **Ground truth**: occlusion OFF, the frame rendered with an object-id
//!   target; every object owning at least one final pixel is "visible".
//! - **Occlusion ON**, three histories of the per-object visibility flags:
//!   `warm` (the path's previous frame rendered first, as in play), `cold`
//!   (flags cleared: everything goes through the late test) and `stale`
//!   (the flags of the opposite view from the same spot). Each frame's drawn
//!   set (early + late + glass lists, read back) must contain every visible
//!   object: the drawn set is a superset of the ground truth.
//! - **Image**: each ON frame against the OFF frame, pixel by pixel. A pixel
//!   whose object id differs must be a depth tie (the two surfaces at the same
//!   depth: draw order decides, and the order of the GPU-appended lists is not
//!   fixed even with occlusion off; the OFF-vs-OFF repeat measures that floor);
//!   a pixel with the same object must match in colour within 2/255 per
//!   channel (the display dither, as the look test's tolerance).
//!
//! Every frame is preceded by a forced refit of all three shadow cascades, so
//! the compared frames have identical shadow maps; the AO pass is off (bench.rs:
//! it differs between two identical frames on its own). Exit code 1 on any failure.

use super::{Segment, settle};
use crate::World;
use crate::models::ModelRuntime;
use anyhow::{Result, anyhow};
use eo_render::camera::Camera;
use eo_render::{GpuContext, IDS_FORMAT, Renderer, read_mapped, wgpu};
use serde_json::json;
use std::collections::HashSet;

struct Frame {
    ids: Vec<u32>,
    color: Vec<u8>,
    depth: Vec<f32>,
    /// The HDR scene before post (Rgba16Float as raw bits): no dither.
    hdr: Vec<u64>,
}

struct Targets {
    w: u32,
    h: u32,
    color: wgpu::Texture,
    ids: wgpu::Texture,
    depth: wgpu::Texture,
    cv: wgpu::TextureView,
    iv: wgpu::TextureView,
    dv: wgpu::TextureView,
}

fn targets(ctx: &GpuContext, w: u32, h: u32) -> Targets {
    let mk = |format, usage| {
        ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("occlusion-check"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let rc = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    let color = mk(wgpu::TextureFormat::Rgba8UnormSrgb, rc);
    let ids = mk(IDS_FORMAT, rc);
    let depth = mk(eo_render::DEPTH_FORMAT, rc | wgpu::TextureUsages::TEXTURE_BINDING);
    let (cv, iv, dv) = (color.create_view(&Default::default()), ids.create_view(&Default::default()), depth.create_view(&Default::default()));
    Targets { w, h, color, ids, depth, cv, iv, dv }
}

/// Refit every shadow cascade on the next update (no change to the clock).
fn force_shadow_refit(r: &Renderer) {
    let h = r.look.time_of_day();
    r.look.set_time_of_day(h + 1e-3);
    r.look.set_time_of_day(h);
}

fn render(ctx: &GpuContext, r: &Renderer, t: &Targets, cam: &Camera) {
    r.update(&ctx.queue, cam);
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    r.encode_with_ids(&mut enc, &t.cv, None, &t.dv, Some(&t.iv), None);
    ctx.queue.submit([enc.finish()]);
}

fn read(ctx: &GpuContext, r: &Renderer, t: &Targets) -> Result<Frame> {
    let (w, h) = (t.w, t.h);
    let hdr_view = r.look.hdr_target(w, h);
    let hdr = hdr_view.texture();
    let mut out = vec![];
    for (tex, bpp, aspect) in [
        (&t.ids, 8u32, wgpu::TextureAspect::All),
        (&t.color, 4, wgpu::TextureAspect::All),
        (&t.depth, 4, wgpu::TextureAspect::DepthOnly),
        (hdr, 8, wgpu::TextureAspect::All),
    ] {
        let row = (w * bpp).div_ceil(256) * 256;
        let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion-check-read"),
            size: row as u64 * h as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        ctx.queue.submit([enc.finish()]);
        let bytes = read_mapped(ctx, &buf)?;
        let mut tight = Vec::with_capacity((w * h * bpp) as usize);
        for y in 0..h as usize {
            tight.extend_from_slice(&bytes[y * row as usize..y * row as usize + (w * bpp) as usize]);
        }
        out.push(tight);
    }
    let ids: Vec<u32> = bytemuck::pod_collect_to_vec::<u8, [u32; 2]>(&out[0]).into_iter().map(|p| p[0]).collect();
    let depth: Vec<f32> = bytemuck::pod_collect_to_vec(&out[2]);
    let hdr: Vec<u64> = bytemuck::pod_collect_to_vec(&out[3]);
    Ok(Frame { ids, color: std::mem::take(&mut out[1]), depth, hdr })
}

/// Output colour tolerance per channel between two frames: the display dither
/// is +-1 step of 8-bit (triangular) with a per-frame phase, so two frames of
/// the same scene differ by up to 2 steps of dither and 1 of rounding. The
/// HDR scene (before post, no dither) is compared exactly.
const COLOR_TOLERANCE: u8 = 3;

#[derive(Default, Clone, Copy)]
struct Diff {
    /// Pixels whose object differs.
    id_pixels: usize,
    /// ... of which not a depth tie (a different surface won: a failure).
    id_pixels_not_tie: usize,
    /// Pixels of the same object whose output colour differs by more than COLOR_TOLERANCE.
    color_pixels: usize,
    /// Pixels of the same object whose HDR scene value (before post) differs at all.
    hdr_pixels: usize,
    max_channel: u8,
    /// The first offending pixel: (pixel index, object, colour a, colour b).
    first: Option<(usize, u32, [u8; 4], [u8; 4])>,
}

fn diff(a: &Frame, b: &Frame) -> Diff {
    let mut d = Diff::default();
    for p in 0..a.ids.len() {
        if a.ids[p] != b.ids[p] {
            d.id_pixels += 1;
            let (za, zb) = (a.depth[p], b.depth[p]);
            // A tie: the same depth to float precision (reverse Z, relative).
            if (za - zb).abs() > 1e-6 * za.abs().max(zb.abs()).max(1e-30) {
                d.id_pixels_not_tie += 1;
            }
            continue;
        }
        if a.hdr[p] != b.hdr[p] {
            d.hdr_pixels += 1;
        }
        let m = (0..4).map(|c| a.color[p * 4 + c].abs_diff(b.color[p * 4 + c])).max().unwrap();
        d.max_channel = d.max_channel.max(m);
        if m > COLOR_TOLERANCE {
            d.color_pixels += 1;
            if d.first.is_none() {
                let c = |f: &Frame| [f.color[p * 4], f.color[p * 4 + 1], f.color[p * 4 + 2], f.color[p * 4 + 3]];
                d.first = Some((p, a.ids[p], c(a), c(b)));
            }
        }
    }
    d
}

fn visible_set(f: &Frame) -> HashSet<u32> {
    // u32::MAX: nothing; 0xFFFF_FFFE: a moving agent (not in the static object table).
    // FAR_ID_BASE | group (below 0xFFFF_FFFD) is a far-layer building, not in the table either.
    f.ids.iter().copied().filter(|&i| i < 0xFFFF_FFFD && i < eo_render::hlod::FAR_ID_BASE).collect() // 0xFFFF_FFFD a street prop instance (not in the table either)
}

fn diff_json(d: &Diff) -> serde_json::Value {
    json!({ "id_pixels": d.id_pixels, "id_pixels_not_depth_tie": d.id_pixels_not_tie, "same_object_pixels_over_tolerance": d.color_pixels, "color_tolerance_of_255": COLOR_TOLERANCE, "max_channel_diff_same_object": d.max_channel, "same_object_hdr_pixels_not_bit_equal": d.hdr_pixels })
}

/// Runs the check; returns the report and whether it passed.
pub fn run(ctx: &GpuContext, renderer: &Renderer, world: &World, models: &mut ModelRuntime, segs: &[Segment], warmup: usize, w: u32, h: u32) -> Result<(serde_json::Value, bool)> {
    if !renderer.debug_ids {
        return Err(anyhow!("--occlusion-check needs a renderer made with debug ids"));
    }
    if !renderer.compact {
        return Err(anyhow!("--occlusion-check needs MULTI_DRAW_INDIRECT_COUNT (occlusion is off without it)"));
    }
    let t = targets(ctx, w, h);
    let mut cameras = vec![];
    let mut all_ok = true;
    // cameras, visible objects, missing, not-tie pixels, colour pixels, HDR pixels, tie pixels
    let mut totals = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for seg in segs {
        let total = warmup + seg.frames;
        for k in 0..6 {
            let i = (k * (total - 1)) / 5;
            let cam = (seg.camera)(i, total);
            let prev = (seg.camera)(i.saturating_sub(1), total);
            let mut opposite = cam;
            opposite.yaw += std::f32::consts::PI;
            renderer.probe.set_frozen(false); // facade
            settle(ctx, renderer, world, models, &cam, 600.0)?;
            // finish any scene-probe capture at this camera, then hold it,
            // so every render below sees the same environment.
            for _ in 0..8 {
                if !renderer.probe.capturing() {
                    break;
                }
                render(ctx, renderer, &t, &cam);
            }
            renderer.probe.set_frozen(true);
            // Ground truth, twice (the second measures draw-order ties alone).
            renderer.set_occlusion(false);
            render(ctx, renderer, &t, &cam);
            force_shadow_refit(renderer);
            render(ctx, renderer, &t, &cam);
            let off = read(ctx, renderer, &t)?;
            force_shadow_refit(renderer);
            render(ctx, renderer, &t, &cam);
            let off2 = read(ctx, renderer, &t)?;
            let visible = visible_set(&off);
            let floor = diff(&off, &off2);
            // the far cull's district-chunk skip changes no pixel: the
            // same OFF frame with every chunk drawn and tested.
            let chunk_skip = match renderer.hlod.get() {
                Some(h) => {
                    h.set_chunk_skip(false);
                    force_shadow_refit(renderer);
                    render(ctx, renderer, &t, &cam);
                    let all = read(ctx, renderer, &t);
                    h.set_chunk_skip(true);
                    let d = diff(&off, &all?);
                    let ok = d.id_pixels_not_tie == 0 && d.color_pixels == 0 && d.hdr_pixels == 0;
                    all_ok &= ok;
                    if !ok {
                        println!("  FAIL {} frame {i} (hlod chunk skip): {} id pixels not a depth tie, {} same-object pixels over {COLOR_TOLERANCE}/255, {} HDR not bit-equal", seg.name, d.id_pixels_not_tie, d.color_pixels, d.hdr_pixels);
                    }
                    Some(json!({ "ok": ok, "image_vs_skip_off": diff_json(&d) }))
                }
                None => None,
            };
            let mut variants = serde_json::Map::new();
            renderer.set_occlusion(true);
            for (name, history) in [("warm", Some(prev)), ("cold", None), ("stale", Some(opposite))] {
                match history {
                    Some(c) => render(ctx, renderer, &t, &c),
                    None => renderer.occlusion.reset(&ctx.queue),
                }
                force_shadow_refit(renderer);
                render(ctx, renderer, &t, &cam);
                let (counters, drawn) = renderer.drawn_readback(ctx)?;
                let on = read(ctx, renderer, &t)?;
                let drawn: HashSet<u32> = drawn.into_iter().collect();
                let missing: Vec<u32> = visible.iter().copied().filter(|o| !drawn.contains(o)).collect();
                let d = diff(&off, &on);
                let ok = missing.is_empty() && d.id_pixels_not_tie == 0 && d.color_pixels == 0 && d.hdr_pixels == 0;
                all_ok &= ok;
                totals.2 += missing.len();
                totals.3 += d.id_pixels_not_tie;
                totals.4 += d.color_pixels;
                totals.5 += d.hdr_pixels;
                totals.6 += d.id_pixels;
                if !ok {
                    // what each missed object is (static scene objects: kind and bounds).
                    for o in missing.iter().take(4) {
                        if let Some(g) = world.scene.objects.get(*o as usize) {
                            println!("    missed object {o}: kind {} lod_group {} role {} bounds {:?} .. {:?}", g.kind & 0xff, g.lod_group, g.lod_role, g.min, g.max);
                        } else {
                            println!("    missed object {o}: streamed (past the {} static objects)", world.scene.objects.len());
                        }
                    }
                    println!(
                        "  FAIL {} frame {i} ({name}): {} visible objects not drawn {:?}, {} id pixels not a depth tie, {} same-object pixels over {COLOR_TOLERANCE}/255 (first: {:?}), {} same-object HDR pixels not bit-equal",
                        seg.name,
                        missing.len(),
                        &missing[..missing.len().min(8)],
                        d.id_pixels_not_tie,
                        d.color_pixels,
                        d.first.map(|(p, o, ca, cb)| (p as u32 % w, p as u32 / w, o, ca, cb)),
                        d.hdr_pixels
                    );
                }
                variants.insert(
                    name.into(),
                    json!({
                        "ok": ok,
                        "objects_in_frustum": counters[11],
                        "objects_drawn": drawn.len(),
                        "drawn_early": counters[13],
                        "occluded": counters[14],
                        "visible_not_drawn": missing.len(),
                        "image_vs_off": diff_json(&d),
                    }),
                );
            }
            totals.0 += 1;
            totals.1 += visible.len();
            println!(
                "  {} frame {i}: {} objects visible (ground truth); drawn warm {} / cold {} / stale {} of {} in the frustum; OFF-vs-OFF: tie pixels {}, colour over tolerance {}, HDR not bit-equal {}",
                seg.name,
                visible.len(),
                variants["warm"]["objects_drawn"],
                variants["cold"]["objects_drawn"],
                variants["stale"]["objects_drawn"],
                variants["warm"]["objects_in_frustum"],
                floor.id_pixels,
                floor.color_pixels,
                floor.hdr_pixels,
            );
            cameras.push(json!({
                "path": seg.name,
                "frame": i,
                "eye": [cam.eye.x, cam.eye.y, cam.eye.z],
                "yaw": cam.yaw,
                "pitch": cam.pitch,
                "visible_objects_ground_truth": visible.len(),
                "off_vs_off": diff_json(&floor),
                "on": variants,
                "hlod_chunk_skip": chunk_skip, // hlod
            }));
        }
    }
    renderer.probe.set_frozen(false); // facade
    renderer.set_occlusion(true);
    println!(
        "\nocclusion check: {} cameras x 3 histories, {} visible objects in all; visible but not drawn {}; id pixels that differ {} (not depth ties {}); same-object pixels: output colour over {COLOR_TOLERANCE}/255 {}, HDR not bit-equal {}: {}",
        totals.0,
        totals.1,
        totals.2,
        totals.6,
        totals.3,
        totals.4,
        totals.5,
        if all_ok { "PASS" } else { "FAIL" }
    );
    Ok((
        json!({
            "rule": "for every sampled camera and three visibility histories (warm: the path's previous frame; cold: flags cleared; stale: the opposite view), every object with at least one pixel in the occlusion-OFF id buffer is in the occlusion-ON drawn lists; where the pixel's object is the same the HDR scene is bit-equal and the output within the dither tolerance; a pixel whose object differs must be a depth tie (draw order, as between two OFF frames)",
            "size": [w, h],
            "cameras": cameras,
            "pass": all_ok,
        }),
        all_ok,
    ))
}
