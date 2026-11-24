//! The interface (`eo-ui`) in the app, behind its own flags so the default
//! paths and `--bench` are untouched:
//!
//!   eo-app --ui                      windowed: title, boot progress, HUD over the fly-through, pause
//!   eo-app --capture-ui SCREEN       offscreen 1920x1080 PNG, no window:
//!                                    title | loading | pause | settings | hud | map | all,
//!                                    and (not in all) settings-hyper | settings-world | pause-settings
//!                                    | photo | photo-look
//!                                    (photo: photo mode's panel over a frame through its
//!                                    pipeline, the Camera page with focus peaking on; photo-look:
//!                                    its Look page, monochrome through its contrast filter; with
//!                                    `--no-photo-mode`, the HUD frame it replaces)
//!   eo-app --ui-bench                HUD on over the street walk, plus each full-screen
//!                                    menu; prints the UI's own cost with PASS/FAIL
//!
//! Also read: --out DIR, --frames N, --warmup N, --no-cache, --cache DIR,
//! --no-models, --no-kit, --no-shadows, `--quality` and every switch it sets
//! (quality.rs; the rest keep the app's defaults).
//!
//! The settings screen drives the window (`--ui` only; the captures and the
//! bench never read the player's file). At start the stored "Lighting and
//! detail" tier, "Sun shadows" off and "Grade-separated roads" apply as if
//! `--quality TIER`, `--no-shadows` and `--structures [--tunnels]` were written
//! first on the line, so every switch given on the line wins (the last
//! `--quality`, a feature's own switch wherever it stands, the last of
//! `--structures` / `--no-structures`). A fresh install's record changes
//! nothing. A change in the menu reaches the running renderer where the
//! feature allows (`live_look`); the rest waits for a restart, as the rows'
//! notes say. `--no-settings-apply` leaves the renderer and the world as the
//! line alone sets them (the frame before the settings drove it).

use crate::models::{ModelRuntime, make_renderer};
use crate::{Args, World, load_world};
use anyhow::{Result, anyhow};
use eo_render::camera::Camera;
use eo_render::{GpuContext, GpuOptions, QUERIES_PER_FRAME, Renderer, Targets, read_mapped, wgpu};
use eo_ui::{Event, HudMode, HudState, Input, PauseTab, Screen, TitleView, Ui, Weapon, WeaponKind};
use glam::Vec3;
use std::path::PathBuf;
use std::time::Instant;

/// The HUD's own budget and the full-screen menus' (owner's numbers).
pub const HUD_BUDGET_MS: f64 = 0.3;
pub const MENU_BUDGET_MS: f64 = 1.0;
const W: u32 = 1920;
const H: u32 = 1080;
const EYE_M: f32 = 1.7;
/// `--capture-ui settings-world`: the title panel's scroll that shows the World block.
const SETTINGS_WORLD_SCROLL: f32 = 1100.0;
/// `--capture-ui settings-hyper`: the title panel's scroll that shows the presets, the
/// "Lighting and detail" row and its note (the panel ends above the credit line, stage T2).
const SETTINGS_HYPER_SCROLL: f32 = 140.0;
/// `--capture-ui pause-settings`: the pause sheet's scroll that shows the graphics block's first rows.
const PAUSE_SETTINGS_SCROLL: f32 = 560.0;

#[derive(Clone, Debug)]
pub enum Mode {
    Window,
    Capture(Vec<String>),
    Bench,
}

pub struct Flags {
    pub mode: Mode,
    args: Args,
    /// The command line, to expand the settings over again when they change.
    raw: Vec<String>,
    /// Whether the stored settings drive the renderer and the world (`settings_apply`).
    apply: bool,
}

/// What the stored settings set, as the switches they stand for, written
/// before the line's own (so the line's win): the tier, and the roads.
fn stored_switches(s: &eo_ui::settings::Settings) -> Vec<(&'static str, Option<&str>)> {
    let mut v = vec![("--quality", Some(s.performance.lighting.as_str()))];
    match s.world.structures.as_str() {
        "on" => v.push(("--structures", None)),
        "tunnels" => v.extend([("--structures", None), ("--tunnels", None)]),
        _ => {}
    }
    v
}

impl Flags {
    /// The UI flags if any are present (then the app's own parser is not run).
    pub fn from_env() -> Option<Result<Flags>> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let has = |f: &str| raw.iter().any(|a| a == f);
        if !(has("--ui") || has("--capture-ui") || has("--ui-bench")) {
            return None;
        }
        Some(Self::parse(&raw))
    }

    /// The line, and in the `--ui` window the player's stored settings under it.
    fn parse(raw: &[String]) -> Result<Flags> {
        let flags = Self::parse_with(raw, None)?;
        if !flags.apply {
            return Ok(flags);
        }
        let stored = eo_ui::settings::default_path().map(|p| eo_ui::settings::Settings::load(&p)).unwrap_or_default();
        Self::parse_with(raw, Some(&stored))
    }

    /// Only the `--ui` window applies the stored settings (the captures and
    /// the bench never read them), and not with `--no-settings-apply`.
    fn settings_apply(mode: &Mode, raw: &[String]) -> bool {
        matches!(mode, Mode::Window) && !raw.iter().any(|a| a == "--no-settings-apply")
    }

    /// The line with `stored` (the settings screen's record) applied under it:
    /// its tier, "Sun shadows" and roads as switches written before the line's.
    fn parse_with(raw: &[String], stored: Option<&eo_ui::settings::Settings>) -> Result<Flags> {
        let mut args = default_args();
        let mut mode = Mode::Window;
        let mut tier = crate::quality::Switches::default(); // `--quality` and the lighting switches
        if let Some(s) = stored {
            for (flag, value) in stored_switches(s) {
                if let Err(e) = tier.take(flag, value) {
                    eprintln!("settings: {flag} {} from the settings file not applied: {e:#}", value.unwrap_or(""));
                }
            }
            if s.performance.shadows == "off" {
                args.look.shadows = false; // "Sun shadows" off: `--no-shadows`
            }
        }
        let mut it = raw.iter();
        while let Some(s) = it.next() {
            if let Some(n) = tier.take(s, it.as_slice().first().map(String::as_str))? {
                it.by_ref().take(n).for_each(drop); // the value it took, if any
                continue;
            }
            let mut val = || it.next().cloned().ok_or_else(|| anyhow!("{s} needs a value"));
            match s.as_str() {
                "--ui" => {}
                "--ui-bench" => mode = Mode::Bench,
                "--capture-ui" => {
                    let v = val()?;
                    let list = if v == "all" { ["title", "loading", "pause", "settings", "hud", "hud-drive", "map"].iter().map(|s| s.to_string()).collect() } else { v.split(',').map(|s| s.trim().to_string()).collect() };
                    mode = Mode::Capture(list);
                }
                "--out" => args.out_dir = Some(val()?.into()),
                "--frames" => args.frames = val()?.parse()?,
                "--warmup" => args.warmup = val()?.parse()?,
                "--cache" => args.cache_dir = Some(val()?.into()),
                "--no-cache" => args.no_cache = true,
                "--no-models" => args.models = false,
                "--no-kit" => args.kit = false,
                "--no-shadows" => args.look.shadows = false,
                "--no-settings-apply" => {} // read by `settings_apply`
                "--ocean-life" | "--no-ocean-life" | "--ocean-reef" | "--no-ocean-reef"
                | "--dive-detail" | "--no-dive-detail" | "--coastal-habitat" | "--no-coastal-habitat" => {}
                "--water-optics" => args.look.water_optics = true,
                "--cove-detail" => args.cove_detail = true,
                "--cove-canopy" => { args.cove_detail = true; args.look.cove_canopy = true; }
                "--no-cove-canopy" => args.look.cove_canopy = false,
                "--cove-sediment" => { args.cove_detail = true; args.look.cove_sediment = true; }
                "--no-cove-sediment" => args.look.cove_sediment = false,
                "--cove-daylight" => { args.cove_detail = true; args.look.cove_daylight = true; }
                "--no-cove-daylight" => args.look.cove_daylight = false,
                "--cove-pile-contact" => args.cove_detail = true,
                "--no-cove-pile-contact" => {} // resolved with the bay preview settings
                "--black-point-intertidal" => { args.cove_detail = true; args.black_point_intertidal = true; }
                "--no-black-point-intertidal" => args.black_point_intertidal = false,
                "--black-point-rockweed" => { args.cove_detail = true; args.black_point_intertidal = true; args.black_point_rockweed = true; }
                "--no-black-point-rockweed" => args.black_point_rockweed = false,
                "--no-cove-detail" => args.cove_detail = false,
                "--floating-docks" => args.floating_docks = true,
                "--no-floating-docks" => args.floating_docks = false,
                "--underwater-habitats" => args.underwater_habitats = true,
                "--no-underwater-habitats" => args.underwater_habitats = false,
                "--no-water-optics" => args.look.water_optics = false,
                "--dive-exposure" => args.look.dive_exposure = true,
                "--no-dive-exposure" => args.look.dive_exposure = false,
                "--bridge-detail" | "--no-bridge-detail" => {} // read by `crate::bridge_detail`
                "--no-vsync" => args.vsync = false,
                "--no-props" => args.props = false, // props in the UI's street
                "--no-landmark-access" => args.landmark_access = false, // the landmark decks
                "--no-audio" | "--audio-null" => {} // read by audio::AudioRuntime::start
                "--no-photo-mode" | "--no-dof" => {} // read by photo.rs
                s if args.props2.parse_flag(s) => {}
                // read by main's `groundscale_flags`.
                "--ground-scale" | "--height-scale" | "--people-scale" | "--vehicle-scale" => {
                    val()?;
                }
                "--low-camera" | "--default-camera" => {}
                "--no-roof-colours" | "--no-roof-contrast" | "--no-roof-parapet" | "--no-roof-patches" | "--no-roof-plant" => {} // main's `roofs_flags`
                "--no-wall-paint" | "--no-ground-green" | "--no-ground-hills" | "--no-ground-contact" => {} // main's `roofs_flags`
                s if eo_landmarks::suspension::SWITCHES.contains(&s) => {} // eo_landmarks::suspension::features
                "--no-external" => {} // main's `external_flag`
                other => anyhow::bail!("unknown argument {other} with --ui (see native/eo-app/src/ui.rs)"),
            }
        }
        tier.apply(&mut args);
        crate::apply_cove_look(&mut args, raw);
        let apply = Self::settings_apply(&mode, raw);
        Ok(Flags { mode, args, raw: raw.to_vec(), apply })
    }
}

/// What a running renderer takes of `wanted` (the settings' look, expanded as
/// at start), given the look it started with (`start`). At once: the sun's
/// shadows (on and off, the map size, the slope bias), the sky's stages and
/// quality, SSR, contact shadows, the cascade blend, lamp and headlight
/// shadows (their atlas follows the options each frame), the lit fog (its
/// volume too), the smooth clock and the photo finish (its pass is made the
/// first time). What is placed or cooked at start keeps its start value
/// until a restart: the weathering's decals (`grime`), the street life's
/// placement, the shop windows' placement (and, outside the look, the facade
/// clutter, the stand-ins and the roads). The far shadows, the ambient
/// volume and the probe grid are attached once, at start, at its level:
/// they go off at once, and come back only at that level (the ground bounce
/// with the volume). (Stage P1) A lighting feature that was off at start has
/// its shader code compiled out of the pipelines (eo-render
/// `look::ShaderFeatures`): the cascade blend, lamp and headlight shadows,
/// lit fog and the smooth clock come on only after a restart; started on, they
/// go off and on again at once.
pub(crate) fn live_look(start: &eo_render::look::LookOptions, wanted: &eo_render::look::LookOptions) -> eo_render::look::LookOptions {
    let mut l = *wanted;
    l.grime = start.grime;
    l.street_life = start.street_life;
    l.sky.shop_light = start.sky.shop_light;
    let attached = |s: u8, w: u8| if w == 0 { 0 } else { s };
    l.lighting.far_shadows = attached(start.lighting.far_shadows, wanted.lighting.far_shadows);
    l.lighting.ambient_volume = attached(start.lighting.ambient_volume, wanted.lighting.ambient_volume);
    l.lighting.probe_grid = attached(start.lighting.probe_grid, wanted.lighting.probe_grid);
    l.lighting.ground_bounce = start.lighting.ground_bounce && l.lighting.ambient_volume > 0;
    // (stage P1) a feature off at start has its shader code compiled out: it comes on after a restart.
    eo_render::look::shader_features().limit(&l)
}

/// The app's defaults (`parse_args` in main.rs with no flags).
fn default_args() -> Args {
    Args {
        interior_lighting: true,
        house_finishes: false,
        house_patina: false,
        house_furniture: false,
        house_photo: false,
        house_photo_lod: false,
        house_blender_bedroom: false,
        house_mission_roof: false,
        house_dining_light: false,
        house_downstairs_light: false,
        house_table_light: false,
        house_oak_finish: false,
        house_oak_joinery: false,
        house_floor_reflections: false,
        house_fitted_dining: false,
        house_fitted_finishes: false,
        house_kitchen_light: false,
        house_window_linen: false,
        house_street_garden: false,
        house_front_daylight: false,
        house_wall_daylight: false,
        house_window_optics: false,
        house_joinery_light: false,
        house_porch_lamp: false,
        house_fired_stoop: false,
        house_site_camera: false,
        house_dining_meter: false,
        house_glazing: true,
        beaver_house: false,
        bench: false,
        start_underwater: false,
        dive_site: None,
        underwater_habitats: false,
        cove_detail: false,
        floating_docks: true,
        black_point_intertidal: false,
        black_point_rockweed: false,
        cove_pile_contact: false,
        surface_start: false,
        quality: Default::default(),
        frames: 720,
        warmup: 60,
        width: W,
        height: H,
        msaa: 1,
        indirect_validation: true,
        no_indirect_count: false,
        no_count_emulation: false,
        bounded_draws: false,
        vsync: true,
        out_dir: None,
        label: None,
        capture: false,
        far: eo_render::camera::GAME_FAR_M,
        multiply: 1,
        check_budget: false,
        budget_ms: None,
        models: true,
        terrain_lod: true,
        cache_dir: None,
        no_cache: false,
        stream_threads: 6,
        stream_radius: f32::INFINITY,
        segments: None,
        kit: true,
        kit_threads: 6,
        kit_split_materials: false,
        upload_budget: 4 << 20,
        kit_vertices: 16_000_000,
        kit_indices: 26_000_000,
        kit_objects: 32_768,
        kit_radii: eo_render::kit::KitRadii::default(),
        drive_frames: 3600,
        streets: true,
        // noon, default shadow quality.
        time_of_day: eo_render::look::DEFAULT_HOUR,
        look: Default::default(),
        agents: true, // agents
        agent_density: 1,
        rocketbox: true, // people
        test_character: None,
        props: true, // props
        props2: Default::default(),
        perf: Default::default(), // occlusion / hitch
        landmarks: true,          // landmarks
        landmarks2: true,
        structures: None, // `--structures` (quality::Switches)
        facade_clutter: 0, // `--facade-clutter` (quality::Switches)
        player_contact: false, // set by net.rs (`--connect`)
        landmark_access: true,
        car_detail: 0, // `--car-detail` (quality::Switches)
    }
}

pub fn run(flags: Result<Flags>, process_start: Instant) -> Result<()> {
    let flags = flags?;
    match flags.mode.clone() {
        Mode::Window => window::run(&flags, process_start),
        Mode::Capture(list) => capture(&flags.args, &list, process_start),
        Mode::Bench => bench(&flags.args, process_start),
    }
}

fn out_dir(args: &Args) -> PathBuf {
    args.out_dir.clone().unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench-out"))
}

/// Direction of the Folsom Street segment nearest the spawn, oriented east
/// (the bench's `street_walk` heading).
fn folsom_heading(world: &World) -> Vec3 {
    let s = glam::Vec2::from(crate::bench::spawn());
    let mut best = (f32::MAX, glam::Vec2::new(1.0, -1.0));
    for r in world.city.roads.iter().filter(|r| r.name == "Folsom Street") {
        for w in r.points.windows(2) {
            let (a, b) = (glam::Vec2::from(w[0]), glam::Vec2::from(w[1]));
            let ab = b - a;
            let t = ((s - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let d = (a + ab * t).distance(s);
            if d < best.0 {
                best = (d, ab);
            }
        }
    }
    let mut dir = best.1.normalize_or(glam::Vec2::new(1.0, -1.0).normalize());
    if dir.x < 0.0 {
        dir = -dir;
    }
    Vec3::new(dir.x, 0.0, dir.y)
}

fn yaw_of(dir: Vec3) -> f32 {
    dir.x.atan2(-dir.z)
}

/// The HUD the captures and the bench show: on foot at the spawn, pistol
/// drawn, a cash change and the objective on screen (both are transient in
/// the JS: shown when they change).
fn demo_hud(player: [f32; 2], heading: f32) -> HudState {
    HudState {
        player,
        heading,
        health: 1.0,
        armour: 0.0,
        cash: 1250,
        wanted: 0,
        hour: 9.0,
        mode: HudMode::OnFoot,
        weapon: Some(Weapon { kind: WeaponKind::Pistol, magazine: 12, reserve: 48 }),
        objective: Some(("Explore freely".into(), "San Francisco".into())),
        prompt: None,
        sky: eo_ui::theme::hud::SKY,
    }
}

struct Offscreen {
    color: wgpu::Texture,
    /// sRGB view for the 3D renderer.
    srgb: wgpu::TextureView,
    /// Unorm view of the same texture for the UI: blending happens on the
    /// stored sRGB values, as a browser composites CSS.
    unorm: wgpu::TextureView,
}

fn offscreen(ctx: &GpuContext) -> Offscreen {
    let color = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui offscreen"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
    });
    let srgb = color.create_view(&Default::default());
    let unorm = color.create_view(&wgpu::TextureViewDescriptor { format: Some(wgpu::TextureFormat::Rgba8Unorm), ..Default::default() });
    Offscreen { color, srgb, unorm }
}

fn clear(enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
    clear_ts(enc, view, None);
}

fn clear_ts(enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, ts: Option<(&wgpu::QuerySet, u32)>) {
    let _ = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("ui clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: ts.map(|(q, i)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: None }),
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

fn save_png(ctx: &GpuContext, tex: &wgpu::Texture, path: &std::path::Path) -> Result<()> {
    let (w, h) = (tex.width(), tex.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui capture"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    ctx.queue.submit([enc.finish()]);
    let bytes = read_mapped(ctx, &buf)?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        rgba.extend_from_slice(&bytes[y * row as usize..y * row as usize + w as usize * 4]);
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut e = png::Encoder::new(file, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&rgba)?;
    Ok(())
}

/// Stream around `cam` until everything it wants is resident (or `timeout_s`).
fn settle(ctx: &GpuContext, renderer: &Renderer, world: &World, models: &mut ModelRuntime, cam: &Camera, timeout_s: f64) -> Result<()> {
    let t = Instant::now();
    models.set_camera(cam.eye);
    loop {
        models.pump(ctx, renderer, world, cam.eye)?;
        renderer.update(&ctx.queue, cam);
        ctx.queue.submit([]);
        let _ = ctx.device.poll(wgpu::PollType::Poll);
        if models.settled() || t.elapsed().as_secs_f64() > timeout_s {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

/// The 3D side for the HUD paths: the world, its renderer and streaming.
struct Street {
    world: World,
    renderer: Renderer,
    targets: Targets,
    models: ModelRuntime,
    start: Vec3,
    heading: Vec3,
    /// the street props (no parked cars: the UI paths run no agents).
    props: crate::props::PropsRuntime,
}

impl Street {
    fn new(ctx: &GpuContext, args: &Args, process_start: Instant) -> Result<Street> {
        let mut world = load_world(args)?;
        let (renderer, bases) = make_renderer(ctx, &world, args, wgpu::TextureFormat::Rgba8UnormSrgb, false)?;
        let targets = Targets::new(&ctx.device, W, H, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
        let heading = folsom_heading(&world);
        let start = Vec3::new(crate::bench::spawn()[0], 0.0, crate::bench::spawn()[1]);
        let models = ModelRuntime::start(&mut world, args, bases, start, process_start);
        let props = crate::props::PropsRuntime::start(&world, args);
        Ok(Street { world, renderer, targets, models, start, heading, props })
    }

    /// the props around `eye` resident (and their uploads written).
    fn settle_props(&mut self, ctx: &GpuContext, eye: Vec3, timeout_s: f64) -> Result<()> {
        let t = Instant::now();
        while !self.props.settled() && t.elapsed().as_secs_f64() < timeout_s {
            self.props.pump(ctx, &self.renderer, &mut self.models.uploads, eye, None, 0.0);
            self.models.pump(ctx, &self.renderer, &self.world, eye)?;
            ctx.queue.submit([]);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok(())
    }
    fn camera(&self, along_m: f32, turn: f32, far: f32) -> Camera {
        let p = self.start + self.heading * along_m;
        let g = self.world.elevation.ground(p.x as f64, p.z as f64) as f32;
        Camera { far, ..Camera::new(Vec3::new(p.x, g + EYE_M, p.z), yaw_of(self.heading) + turn, 0.0, W as f32 / H as f32) }
    }
}

fn gpu() -> Result<GpuContext> {
    let opts = GpuOptions::default();
    GpuContext::new(GpuContext::create_instance(&opts), None, opts)
}

fn make_ui(ctx: &GpuContext, city: &eo_data::City, land: Option<&eo_data::LandCover>, repo_root: &std::path::Path, settings: Option<PathBuf>) -> Ui {
    let t = Instant::now();
    let ui = Ui::new(&ctx.device, &ctx.queue, Some(repo_root), settings, Some(eo_ui::MapInputs { city, land, backdrop_centre: crate::bench::spawn() }));
    eprintln!("ui: ready in {:.0} ms (map mesh {:.0} ms)", t.elapsed().as_secs_f64() * 1e3, ui.map_mesh_ms().unwrap_or(0.0));
    ui
}

/// Run `n` frames of the UI at `t0 + i * dt` (map images render, motion
/// settles), then draw one more over `view`.
fn ui_frames(ctx: &GpuContext, ui: &mut Ui, t0: f64, n: usize) {
    for i in 0..n {
        ui.set_time(t0 + i as f64 / 60.0);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        ui.prepare(&ctx.device, &ctx.queue, &mut enc, W, H, &Input::default());
        ctx.queue.submit([enc.finish()]);
    }
}

fn capture(args: &Args, list: &[String], process_start: Instant) -> Result<()> {
    let ctx = gpu()?;
    let dir = out_dir(args);
    std::fs::create_dir_all(&dir)?;
    let off = offscreen(&ctx);
    let want_hud = list.iter().any(|s| s.starts_with("hud") || s.starts_with("photo"));
    // The world (for the street under the HUD) or just the city data (menus).
    let mut street = if want_hud { Some(Street::new(&ctx, args, process_start)?) } else { None };
    let paths = eo_data::DataPaths::discover()?;
    let city_own;
    let city: &eo_data::City = match &street {
        Some(s) => &s.world.city,
        None => {
            city_own = eo_data::City::load(&paths)?;
            &city_own
        }
    };
    let land = eo_data::LandCover::load(&paths).ok();
    // Captures never touch the player's settings file.
    let mut ui = make_ui(&ctx, city, land.as_ref(), &paths.repo_root, None);
    let heading = street.as_ref().map(|s| yaw_of(s.heading)).unwrap_or(1.2);
    ui.hud = demo_hud(crate::bench::spawn(), heading);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    for name in list {
        let t0 = 100.0;
        ui.set_time(t0);
        ui.settings = eo_ui::settings::Settings::default(); // every screen at the defaults (settings-hyper sets its own)
        match name.as_str() {
            "title" => {
                ui.set_screen(Screen::Title);
                ui.set_title_view(TitleView::Main);
                ui.set_screen_age(3.2);
            }
            "loading" => {
                ui.set_screen(Screen::Title);
                ui.set_screen(Screen::Loading);
                ui.loading = eo_ui::Loading { progress: 0.46, phase: "Building the city".into(), detail: "Streaming buildings around the start.".into() };
                ui.set_screen_age(4.0);
            }
            "settings" => {
                ui.set_screen(Screen::Title);
                ui.set_title_view(TitleView::Settings);
                // Scrolled (the panel scrolls, as in the JS) so the People row is in view.
                ui.set_scroll(300.0);
                ui.set_screen_age(3.2);
            }
            "settings-hyper" | "settings-world" => {
                // The panel's top with Hyper real chosen and the roads with tunnels
                // (the presets, the Lighting and detail row and its note), or
                // scrolled to the World block at the defaults.
                if name == "settings-hyper" {
                    ui.settings.performance.apply_preset("hyper");
                    ui.settings.world.structures = "tunnels".into();
                }
                ui.set_screen(Screen::Title);
                ui.set_title_view(TitleView::Settings);
                ui.set_scroll(if name == "settings-hyper" { SETTINGS_HYPER_SCROLL } else { SETTINGS_WORLD_SCROLL });
                ui.set_screen_age(3.2);
            }
            "pause-settings" => {
                // The pause sheet scrolled to its graphics block.
                ui.set_screen(Screen::Pause);
                ui.set_pause_tab(PauseTab::Settings);
                ui.set_scroll(PAUSE_SETTINGS_SCROLL);
                ui.set_screen_age(1.0);
            }
            "pause" => {
                ui.set_screen(Screen::Pause);
                ui.set_pause_tab(PauseTab::Map);
                ui.set_pause_map_view(None);
                ui.set_screen_age(1.0);
            }
            "map" => {
                ui.set_screen(Screen::Pause);
                ui.set_pause_tab(PauseTab::Map);
                ui.set_pause_map_view(Some((crate::bench::spawn()[0], crate::bench::spawn()[1], eo_ui::theme::pause::LOCATE_SCALE)));
                ui.set_screen_age(1.0);
            }
            "hud" | "hud-drive" => {
                ui.set_screen(Screen::Hud);
                ui.hud.mode = if name == "hud-drive" { HudMode::Drive { speed_kph: 42.0 } } else { HudMode::OnFoot };
                ui.set_screen_age(0.0);
            }
            "photo" | "photo-look" => {
                ui.set_screen(Screen::Hud);
                ui.hud.mode = HudMode::OnFoot;
                ui.set_screen_age(0.0);
            }
            other => anyhow::bail!("--capture-ui: unknown screen {other} (title, loading, pause, settings, hud, hud-drive, map, photo, photo-look, all; settings-hyper, settings-world, pause-settings)"),
        }
        let age_t0 = t0;
        ui_frames(&ctx, &mut ui, age_t0, 3);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        if name.starts_with("hud") || name.starts_with("photo") {
            let s = street.as_mut().unwrap();
            let mut cam = s.camera(0.0, 0.0, args.far);
            // photo mode: a 50 mm lens looking down the street at f/2.8, focused 12 m out; the
            // Camera page open on focus peaking (on: the 12 m plane's edges marked), or the Look
            // page on monochrome's contrast filter.
            let photo = (name.starts_with("photo") && crate::photo::enabled()).then(|| {
                let mut m = crate::photo::PhotoMode::new(false, args.time_of_day);
                m.enter(&cam, cam.eye.as_dvec3() + glam::DVec3::new(0.0, -0.27, 0.0));
                m.focal_mm = 50.0;
                m.focus = crate::photo::Focus::Manual;
                m.focus_m = 12.0;
                m.step(&crate::photo::Feed { held: &Default::default(), mouse: [0.0, 0.0], dt: 10.0, aspect: W as f64 / H as f64, anchor: m.anchor, cw: None });
                if name == "photo-look" {
                    m.grade = eo_render::photo_mode::Grade::Monochrome;
                    m.grain = m.grade.look().grain;
                    m.set_tab(1);
                    m.select_row(3);
                } else {
                    m.peaking = true;
                    m.select_row(7);
                }
                m
            });
            if let Some(m) = &photo {
                cam = m.camera(W as f64 / H as f64, args.far);
                s.renderer.photo_mode.set(Some(eo_render::photo_mode::PhotoParams { accumulate: eo_render::photo_mode::SHOT_FRAMES, animate_grain: false, ..m.params() }));
                ui.photo = Some(m.panel(W as f64 / H as f64));
            }
            settle(&ctx, &s.renderer, &s.world, &mut s.models, &cam, 600.0)?;
            s.settle_props(&ctx, cam.eye, 600.0)?;
            for _ in 0..if photo.is_some() { 64 } else { 2 } {
                s.renderer.update(&ctx.queue, &cam);
                let mut e = ctx.device.create_command_encoder(&Default::default());
                s.renderer.encode(&mut e, &off.srgb, None, &s.targets.depth, None);
                ctx.queue.submit([e.finish()]);
            }
            s.renderer.photo_mode.set(None);
            // Show the HUD a moment after it changed (chips and the objective are up).
            ui.set_time(age_t0 + 0.6);
            ui.prepare(&ctx.device, &ctx.queue, &mut enc, W, H, &Input::default());
        } else {
            clear(&mut enc, &off.unorm);
            ui.set_time(age_t0 + 3.0 / 60.0);
            ui.prepare(&ctx.device, &ctx.queue, &mut enc, W, H, &Input::default());
        }
        ui.draw(&ctx.device, &mut enc, &off.unorm, format, None);
        ctx.queue.submit([enc.finish()]);
        ui.photo = None;
        let path = dir.join(format!("capture-ui-{name}.png"));
        save_png(&ctx, &off.color, &path)?;
        println!("wrote {}", path.display());
    }
    for (k, f) in ui.fonts() {
        println!("font {k}: {} -> {} {:?}", f.css_family, f.file.display(), f.variations);
    }
    Ok(())
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((p / 100.0 * (s.len() - 1) as f64).round() as usize).min(s.len() - 1)]
}

struct Sample {
    prepare_ms: Vec<f64>,
    encode_ms: Vec<f64>,
    gpu_ui_ms: Vec<f64>,
    frame_ms: Vec<f64>,
}

fn verdict(v: f64, budget: f64) -> &'static str {
    if v <= budget { "PASS" } else { "FAIL" }
}

/// Timestamped frames: optional 3D street, then the UI pass.
#[allow(clippy::too_many_arguments)]
fn measure(
    ctx: &GpuContext,
    ui: &mut Ui,
    off: &Offscreen,
    mut street: Option<&mut Street>,
    frames: usize,
    warmup: usize,
    far: f32,
    step: &mut dyn FnMut(&mut Ui, usize, &mut Input),
) -> Result<(Sample, eo_ui::UiStats, eo_ui::UiStats)> {
    let per = QUERIES_PER_FRAME + 2;
    // One query set holds at most 4096 queries.
    let frames = frames.min(4096 / per as usize - warmup);
    let total = frames + warmup;
    let qs = ctx.timestamps.then(|| ctx.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("ui ts"), ty: wgpu::QueryType::Timestamp, count: per * total as u32 }));
    let mut s = Sample { prepare_ms: vec![], encode_ms: vec![], gpu_ui_ms: vec![], frame_ms: vec![] };
    let mut cpu3d = vec![];
    let mut subs: Vec<wgpu::SubmissionIndex> = vec![];
    let mut stats_after_warmup = ui.stats;
    for i in 0..total {
        if i == warmup {
            stats_after_warmup = ui.stats;
        }
        if i >= 2 {
            ctx.device.poll(wgpu::PollType::Wait { submission_index: Some(subs[i - 2].clone()), timeout: None }).map_err(|e| anyhow!("{e}"))?;
        }
        let mut input = Input::default();
        step(ui, i, &mut input);
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        let t0 = Instant::now();
        if let Some(st) = street.as_deref_mut() {
            let along = 1.4 / 60.0 * i as f32;
            let turn = std::f32::consts::TAU * i as f32 / total as f32;
            let cam = st.camera(along, turn, far);
            st.props.pump(ctx, &st.renderer, &mut st.models.uploads, cam.eye, None, 1.0 / 60.0);
            let _ = st.models.uploads.pump(&st.renderer, &ctx.queue);
            st.renderer.update(&ctx.queue, &cam);
            st.renderer.encode(&mut enc, &off.srgb, None, &st.targets.depth, qs.as_ref().map(|q| (q, per * i as u32)));
        } else {
            clear_ts(&mut enc, &off.unorm, qs.as_ref().map(|q| (q, per * i as u32)));
        }
        let t1 = Instant::now();
        ui.prepare(&ctx.device, &ctx.queue, &mut enc, W, H, &input);
        let t2 = Instant::now();
        ui.draw(&ctx.device, &mut enc, &off.unorm, wgpu::TextureFormat::Rgba8Unorm, qs.as_ref().map(|q| (q, per * i as u32 + QUERIES_PER_FRAME)));
        let t3 = Instant::now();
        subs.push(ctx.queue.submit([enc.finish()]));
        if i >= warmup {
            cpu3d.push((t1 - t0).as_secs_f64() * 1e3);
            s.prepare_ms.push((t2 - t1).as_secs_f64() * 1e3);
            s.encode_ms.push((t3 - t2).as_secs_f64() * 1e3);
        }
    }
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("{e}"))?;
    let stats_end = ui.stats;
    if let Some(q) = &qs {
        let n = per * total as u32;
        let resolve = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n as u64 * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.resolve_query_set(q, 0..n, &resolve, 0);
        enc.copy_buffer_to_buffer(&resolve, 0, &read, 0, n as u64 * 8);
        ctx.queue.submit([enc.finish()]);
        let bytes = read_mapped(ctx, &read)?;
        let ts: Vec<u64> = bytes.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
        let period = ctx.queue.get_timestamp_period() as f64;
        let ms = |a: u64, b: u64| b.wrapping_sub(a) as f64 * period / 1e6;
        for (k, i) in (warmup..total).enumerate() {
            let b = (per as usize) * i;
            // From the end of the 3D draw pass (or the start of the clear) to
            // the end of the UI pass: the UI pass and any map image renders.
            let ui_end = ts[b + QUERIES_PER_FRAME as usize + 1];
            // the renderer's frame ends with its last query (post end), not query 3.
            let end3d = b + eo_render::look::Q_POST as usize + 1; // post is the last pass executed, not the last slot
            let ui_gpu = if street.is_some() { ms(ts[end3d], ui_end) } else { ms(ts[b], ui_end) };
            s.gpu_ui_ms.push(ui_gpu);
            let gpu3d = if street.is_some() { ms(ts[b], ts[end3d]) } else { 0.0 };
            s.frame_ms.push(cpu3d[k] + s.prepare_ms[k] + s.encode_ms[k] + gpu3d + ui_gpu);
        }
    }
    Ok((s, stats_after_warmup, stats_end))
}

fn report(name: &str, s: &Sample, a: eo_ui::UiStats, b: eo_ui::UiStats, budget: f64, frame: bool) -> (String, bool, serde_json::Value) {
    let ui_total: Vec<f64> = (0..s.prepare_ms.len()).map(|k| s.prepare_ms[k] + s.encode_ms[k] + s.gpu_ui_ms.get(k).copied().unwrap_or(0.0)).collect();
    let med = pct(&ui_total, 50.0);
    let p95 = pct(&ui_total, 95.0);
    let pass = med <= budget;
    let frames = (b.frames - a.frames).max(1);
    let line = format!(
        "[ui {name}] UI cost (CPU prepare + CPU encode + GPU pass) median {med:.3} ms [{}] p95 {p95:.3} max {:.3} (budget {budget} ms)\n    split median: CPU prepare {:.3}, CPU encode {:.3}, GPU UI pass {:.3}; rebuilds {} / {} frames, glyph runs shaped {}, glyphs rasterised {}, map renders {}; {} vertices, {} draw calls{}",
        verdict(med, budget),
        pct(&ui_total, 100.0),
        pct(&s.prepare_ms, 50.0),
        pct(&s.encode_ms, 50.0),
        pct(&s.gpu_ui_ms, 50.0),
        b.rebuilds - a.rebuilds,
        frames,
        b.shaped_runs - a.shaped_runs,
        b.rasterised_glyphs - a.rasterised_glyphs,
        b.map_renders - a.map_renders,
        b.vertices,
        b.draw_calls,
        if frame { format!("\n    whole frame with the HUD (3D CPU+GPU + UI CPU+GPU, summed): median {:.3} ms, p95 {:.3} ms", pct(&s.frame_ms, 50.0), pct(&s.frame_ms, 95.0)) } else { String::new() }
    );
    let json = serde_json::json!({
        "name": name, "budget_ms": budget, "verdict": verdict(med, budget),
        "ui_total_ms": { "median": med, "p95": p95, "max": pct(&ui_total, 100.0) },
        "cpu_prepare_ms_median": pct(&s.prepare_ms, 50.0), "cpu_encode_ms_median": pct(&s.encode_ms, 50.0), "gpu_ui_pass_ms_median": pct(&s.gpu_ui_ms, 50.0),
        "rebuilds": b.rebuilds - a.rebuilds, "frames": frames, "shaped_runs": b.shaped_runs - a.shaped_runs, "map_renders": b.map_renders - a.map_renders,
        "vertices": b.vertices, "draw_calls": b.draw_calls,
        "frame_with_hud_ms": if frame { serde_json::json!({ "median": pct(&s.frame_ms, 50.0), "p95": pct(&s.frame_ms, 95.0) }) } else { serde_json::Value::Null },
    });
    (line, pass, json)
}

fn bench(args: &Args, process_start: Instant) -> Result<()> {
    let ctx = gpu()?;
    if !ctx.timestamps {
        eprintln!("warning: no timestamp queries on this adapter; GPU UI cost cannot be measured");
    }
    let off = offscreen(&ctx);
    let mut street = Street::new(&ctx, args, process_start)?;
    let paths = eo_data::DataPaths::discover()?;
    let land = eo_data::LandCover::load(&paths).ok();
    let mut ui = make_ui(&ctx, &street.world.city, land.as_ref(), &paths.repo_root, None);
    let heading = yaw_of(street.heading);
    ui.hud = demo_hud(crate::bench::spawn(), heading);
    ui.warm(&ctx.device, wgpu::TextureFormat::Rgba8Unorm);
    let cam = street.camera(0.0, 0.0, args.far);
    settle(&ctx, &street.renderer, &street.world, &mut street.models, &cam, 600.0)?;
    street.settle_props(&ctx, cam.eye, 600.0)?;
    let (frames, warmup) = (args.frames, args.warmup);
    let mut lines = vec![];
    let mut results = vec![];
    let mut all_pass = true;
    let start_xz = [street.start.x, street.start.z];
    let dir = street.heading;
    let total = (frames + warmup) as f32;

    // 1. HUD over the street walk: the player walks and turns a full circle,
    //    so the radar, its north pip and the street plate change every frame.
    ui.set_screen(Screen::Hud);
    let clock = Instant::now();
    let mut step = |ui: &mut Ui, i: usize, _in: &mut Input| {
        let along = 1.4 / 60.0 * i as f32;
        ui.hud.player = [start_xz[0] + dir.x * along, start_xz[1] + dir.z * along];
        ui.hud.heading = heading + std::f32::consts::TAU * i as f32 / total;
        ui.set_time(clock.elapsed().as_secs_f64());
    };
    let (s, a, b) = measure(&ctx, &mut ui, &off, Some(&mut street), frames, warmup, args.far, &mut step)?;
    let (l, p, j) = report("hud_street_walk", &s, a, b, HUD_BUDGET_MS, true);
    lines.push(l);
    all_pass &= p;
    results.push(j);

    // 2. HUD with nothing changing (player standing, no chips moving).
    let still_t = clock.elapsed().as_secs_f64() + 30.0;
    let mut step = |ui: &mut Ui, _i: usize, _in: &mut Input| {
        ui.set_time(still_t);
    };
    let (s, a, b) = measure(&ctx, &mut ui, &off, Some(&mut street), frames, warmup, args.far, &mut step)?;
    let (l, p, j) = report("hud_unchanged", &s, a, b, HUD_BUDGET_MS, true);
    lines.push(l);
    all_pass &= p;
    results.push(j);

    // 3. Full-screen menus (UI only: they cover the frame).
    let menus: [(&str, Screen, Option<TitleView>, Option<PauseTab>, bool); 5] = [
        ("title_animating", Screen::Title, Some(TitleView::Main), None, false),
        ("title_settings", Screen::Title, Some(TitleView::Settings), None, false),
        ("pause_map", Screen::Pause, None, Some(PauseTab::Map), false),
        ("pause_settings", Screen::Pause, None, Some(PauseTab::Settings), false),
        ("pause_map_panning", Screen::Pause, None, Some(PauseTab::Map), true),
    ];
    for (name, screen, tv, pt, pan) in menus {
        // The menu clock starts where this segment's frames start, so the
        // screen's opening motion is over after its own duration.
        ui.set_time(clock.elapsed().as_secs_f64());
        ui.set_screen(screen);
        if let Some(v) = tv {
            ui.set_title_view(v);
        }
        if let Some(t) = pt {
            ui.set_pause_tab(t);
        }
        let base = clock.elapsed().as_secs_f64();
        let mut step = |ui: &mut Ui, i: usize, input: &mut Input| {
            ui.set_time(base + i as f64 / 60.0);
            if pan {
                // A steady drag across the pane: the map image re-renders every frame.
                input.held = true;
                input.drag = [2.0, 1.0];
                input.mouse = Some([600.0, 500.0]);
            }
        };
        let (s, a, b) = measure(&ctx, &mut ui, &off, None, frames, warmup, args.far, &mut step)?;
        let (l, p, j) = report(name, &s, a, b, MENU_BUDGET_MS, false);
        lines.push(l);
        // Panning is reported against the menu budget but is a different case
        // (a map redraw per frame); it does not gate the PASS line.
        if !pan {
            all_pass &= p;
        }
        results.push(j);
        ui.set_screen(Screen::Hud);
    }
    for l in &lines {
        println!("{l}");
    }
    let summary = if all_pass { "PASS" } else { "FAIL" };
    println!("ui budget: {summary} (HUD median <= {HUD_BUDGET_MS} ms; full-screen menus median <= {MENU_BUDGET_MS} ms; panning reported, not gated)");
    let dir = out_dir(args);
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let path = dir.join(format!("bench-ui-{stamp}.json"));
    let out = serde_json::json!({
        "adapter": ctx.info.name, "width": W, "height": H,
        "definition": "UI cost = CPU (Ui::prepare: input, rebuild when changed, uploads, map image renders recorded) + CPU (Ui::draw: pass encode) + GPU (UI render pass timestamps). Map image render passes are recorded in prepare but their GPU time is outside the UI pass timestamps; the panning row shows that case's CPU side.",
        "segments": results, "verdict": summary,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&out)?)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// `--ui`: a window with the title, boot progress, the fly-through with the
/// HUD and the pause menu. Not run by the bench, the captures or the tests.
mod window {
    use super::*;
    use std::sync::Arc;
    use winit::application::ApplicationHandler;
    use winit::dpi::PhysicalSize;
    use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::keyboard::{KeyCode, PhysicalKey};
    use winit::window::{CursorGrabMode, Window, WindowId};

    struct Gfx {
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        ctx: GpuContext,
        renderer: Renderer,
        targets: Targets,
        models: ModelRuntime,
        ui: Ui,
        props: crate::props::PropsRuntime,
        audio: crate::audio::AudioRuntime, // the menus' sounds and the city under the HUD
    }

    struct App {
        world: World,
        args: Args,
        /// The command line and whether the stored settings apply (`Flags`), to
        /// expand a change in the menu as the start did (`apply_settings`).
        raw: Vec<String>,
        settings_apply: bool,
        gfx: Option<Gfx>,
        cam: Camera,
        keys: std::collections::HashSet<KeyCode>,
        input: Input,
        mouse_down: bool,
        last: Instant,
        clock: Instant,
        process_start: Instant,
        loading_since: Option<Instant>,
    }

    pub fn run(flags: &Flags, process_start: Instant) -> Result<()> {
        let args = &flags.args;
        if flags.apply {
            let s = args.structures.as_ref().map(|s| if s.tunnels { "on, with tunnels" } else { "on" }).unwrap_or("off");
            eprintln!("settings: quality {}, sun shadows {}, grade-separated roads {s} (the settings file under the line's switches; --no-settings-apply ignores it)", args.quality.name(), if args.look.shadows { "on" } else { "off" });
        }
        let world = load_world(args)?;
        let spawn = crate::bench::spawn();
        let ground = world.elevation.ground(spawn[0] as f64, spawn[1] as f64) as f32;
        let cam = Camera::new(Vec3::new(spawn[0], ground + EYE_M, spawn[1]), 0.8, 0.0, args.width as f32 / args.height as f32);
        let event_loop = EventLoop::new()?;
        let mut app = App {
            world,
            args: args.clone(),
            raw: flags.raw.clone(),
            settings_apply: flags.apply,
            gfx: None,
            cam,
            keys: Default::default(),
            input: Input::default(),
            mouse_down: false,
            last: Instant::now(),
            clock: Instant::now(),
            process_start,
            loading_since: None,
        };
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    /// A change in the settings screen on the running renderer: the record
    /// expanded under the line as at start (`Flags::parse_with`), then what a
    /// running renderer takes of it (`live_look`); the rest waits for a
    /// restart, as the rows' notes say.
    fn apply_settings(raw: &[String], start: &Args, s: &eo_ui::settings::Settings, renderer: &mut Renderer, device: &wgpu::Device) {
        let wanted = match Flags::parse_with(raw, Some(s)) {
            Ok(f) => f.args,
            Err(e) => {
                eprintln!("settings: not applied: {e:#}");
                return;
            }
        };
        let look = live_look(&start.look, &wanted.look);
        // `LookOptions` has no PartialEq; its Debug form names every field and value.
        if format!("{look:?}") == format!("{:?}", renderer.look.options) {
            return;
        }
        renderer.look.set_options(device, look);
        eprintln!(
            "settings: quality {}, sun shadows {} on the running renderer (placed and cooked detail, far shadows, ambient light, street reflections, lamp shadows and lit fog coming on when the game started without them, and the roads take effect after restart)",
            wanted.quality.name(),
            if look.shadows { "on" } else { "off" }
        );
    }

    impl App {
        fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
            let attrs = Window::default_attributes().with_title("Earth Online").with_inner_size(PhysicalSize::new(W, H));
            let window = Arc::new(el.create_window(attrs)?);
            let opts = GpuOptions::default();
            let instance = GpuContext::create_instance(&opts);
            let surface = instance.create_surface(window.clone())?;
            let ctx = GpuContext::new(instance, Some(&surface), opts)?;
            let size = window.inner_size();
            let caps = surface.get_capabilities(&ctx.adapter);
            let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
            let mut config = surface.get_default_config(&ctx.adapter, size.width.max(1), size.height.max(1)).ok_or_else(|| anyhow!("surface unsupported"))?;
            config.format = format;
            // The UI blends on the stored sRGB values through a non-sRGB view when the surface allows one.
            let plain = format.remove_srgb_suffix();
            if plain != format {
                config.view_formats = vec![plain];
            }
            config.present_mode = if self.args.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
            surface.configure(&ctx.device, &config);
            let (renderer, base) = make_renderer(&ctx, &self.world, &self.args, format, false)?;
            let targets = Targets::new(&ctx.device, config.width, config.height, format, 1);
            let models = ModelRuntime::start(&mut self.world, &self.args, base, self.cam.eye, self.process_start);
            let land = eo_data::LandCover::load(&self.world.paths).ok();
            let ui = Ui::new(
                &ctx.device,
                &ctx.queue,
                Some(&self.world.paths.repo_root),
                eo_ui::settings::default_path(),
                Some(eo_ui::MapInputs { city: &self.world.city, land: land.as_ref(), backdrop_centre: crate::bench::spawn() }),
            );
            let props = crate::props::PropsRuntime::start(&self.world, &self.args);
            let mut audio = crate::audio::AudioRuntime::start(&self.world, eo_audio::Output::Device); // audio
            audio.set_volume(&ui.settings);
            self.gfx = Some(Gfx { window, surface, config, ctx, renderer, targets, models, ui, props, audio });
            Ok(())
        }

        fn grab(&self, on: bool) {
            if let Some(g) = &self.gfx {
                if on {
                    let _ = g.window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Confined));
                } else {
                    let _ = g.window.set_cursor_grab(CursorGrabMode::None);
                }
                g.window.set_cursor_visible(!on);
            }
        }

        fn redraw(&mut self, el: &ActiveEventLoop) {
            let now = Instant::now();
            let dt = (now - self.last).as_secs_f32().min(0.1);
            self.last = now;
            let Some(g) = self.gfx.as_mut() else { return };
            let screen = g.ui.screen;
            if screen == Screen::Hud {
                let k = |c| self.keys.contains(&c);
                let mut speed = 6.0;
                if k(KeyCode::ShiftLeft) {
                    speed *= 4.0;
                }
                let (f, r) = (self.cam.forward(), self.cam.right());
                let mut m = Vec3::ZERO;
                if k(KeyCode::KeyW) {
                    m += f;
                }
                if k(KeyCode::KeyS) {
                    m -= f;
                }
                if k(KeyCode::KeyD) {
                    m += r;
                }
                if k(KeyCode::KeyA) {
                    m -= r;
                }
                m.y = 0.0;
                self.cam.eye += m.normalize_or_zero() * speed * dt;
                let g0 = self.world.elevation.ground(self.cam.eye.x as f64, self.cam.eye.z as f64) as f32;
                self.cam.eye.y = g0 + EYE_M;
            }
            g.ui.hud.player = [self.cam.eye.x, self.cam.eye.z];
            g.ui.hud.heading = self.cam.yaw;
            if let Some(since) = self.loading_since {
                let total = self.world.authored.buildings.len().max(1);
                let p = g.models.stats.resident as f32 / total as f32;
                g.ui.loading.progress = 0.1 + 0.9 * p.min(1.0);
                g.ui.loading.phase = "Building the city".into();
                g.ui.loading.detail = format!("{} of {} hand-built buildings resident.", g.models.stats.resident, total);
                if g.models.settled() || since.elapsed().as_secs_f32() > 20.0 {
                    self.loading_since = None;
                    g.ui.set_screen(Screen::Hud);
                }
            }
            let frame = match g.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                _ => {
                    g.surface.configure(&g.ctx.device, &g.config);
                    return;
                }
            };
            let view = frame.texture.create_view(&Default::default());
            let plain_format = g.config.format.remove_srgb_suffix();
            let ui_view = if g.config.view_formats.contains(&plain_format) {
                frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(plain_format), ..Default::default() })
            } else {
                frame.texture.create_view(&Default::default())
            };
            let ui_format = if g.config.view_formats.contains(&plain_format) { plain_format } else { g.config.format };
            let mut enc = g.ctx.device.create_command_encoder(&Default::default());
            if matches!(g.ui.screen, Screen::Hud | Screen::Loading) {
                g.models.set_camera(self.cam.eye);
                let _ = g.models.pump(&g.ctx, &g.renderer, &self.world, self.cam.eye);
                g.props.pump(&g.ctx, &g.renderer, &mut g.models.uploads, self.cam.eye, None, dt as f64);
                g.renderer.update(&g.ctx.queue, &self.cam);
                g.renderer.encode(&mut enc, &view, None, &g.targets.depth, None);
            }
            g.ui.set_time(self.clock.elapsed().as_secs_f64());
            let input = std::mem::take(&mut self.input);
            let event = g.ui.prepare(&g.ctx.device, &g.ctx.queue, &mut enc, g.config.width, g.config.height, &input);
            g.ui.draw(&g.ctx.device, &mut enc, &ui_view, ui_format, None);
            if event == Some(Event::SettingsChanged) {
                g.audio.set_volume(&g.ui.settings); // audio
                if self.settings_apply {
                    apply_settings(&self.raw, &self.args, &g.ui.settings, &mut g.renderer, &g.ctx.device);
                }
            }
            let cues = g.ui.take_cues(); // audio
            g.audio.frame(crate::audio::FrameIn::interface(&self.cam, dt as f64, &g.renderer, &self.world.elevation, g.ui.screen, cues)); // audio
            g.ctx.queue.submit([enc.finish()]);
            g.window.pre_present_notify();
            g.ctx.queue.present(frame);
            self.input.mouse = input.mouse;
            match event {
                Some(Event::StartGame { .. }) => self.loading_since = Some(Instant::now()),
                Some(Event::Exit) => el.exit(),
                Some(Event::Resume) => {
                    g.ui.set_screen(Screen::Hud);
                    g.ui.release_pause_map();
                    self.grab(true);
                }
                Some(Event::QuitToTitle) => {
                    g.ui.set_screen(Screen::Title);
                    g.ui.release_pause_map();
                }
                _ => {}
            }
        }
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            if self.gfx.is_none() {
                if let Err(e) = self.init(el) {
                    eprintln!("init failed: {e:#}");
                    el.exit();
                }
            }
        }

        fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => el.exit(),
                WindowEvent::Resized(s) => {
                    if let Some(g) = self.gfx.as_mut() {
                        if s.width > 0 && s.height > 0 {
                            g.config.width = s.width;
                            g.config.height = s.height;
                            g.surface.configure(&g.ctx.device, &g.config);
                            g.targets = Targets::new(&g.ctx.device, s.width, s.height, g.config.format, 1);
                            self.cam.aspect = s.width as f32 / s.height as f32;
                        }
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    let p = [position.x as f32, position.y as f32];
                    if self.mouse_down {
                        if let Some(old) = self.input.mouse {
                            self.input.drag[0] += p[0] - old[0];
                            self.input.drag[1] += p[1] - old[1];
                        }
                        self.input.held = true;
                    }
                    self.input.mouse = Some(p);
                }
                WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                    self.mouse_down = state == ElementState::Pressed;
                    if self.mouse_down {
                        self.input.click = true;
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    self.input.wheel += match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
                    };
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    let PhysicalKey::Code(code) = event.physical_key else { return };
                    if event.state == ElementState::Released {
                        self.keys.remove(&code);
                        return;
                    }
                    self.keys.insert(code);
                    let screen = self.gfx.as_ref().map(|g| g.ui.screen);
                    if code == KeyCode::Escape && screen == Some(Screen::Hud) {
                        if let Some(g) = self.gfx.as_mut() {
                            g.ui.set_screen(Screen::Pause);
                        }
                        self.grab(false);
                        return;
                    }
                    let shift = self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
                    self.input.keys |= match code {
                        KeyCode::ArrowUp => eo_ui::key::UP,
                        KeyCode::ArrowDown => eo_ui::key::DOWN,
                        KeyCode::ArrowLeft => eo_ui::key::LEFT,
                        KeyCode::ArrowRight => eo_ui::key::RIGHT,
                        KeyCode::Enter | KeyCode::Space => eo_ui::key::ENTER,
                        KeyCode::Escape => eo_ui::key::ESCAPE,
                        KeyCode::Tab if shift => eo_ui::key::SHIFT_TAB,
                        KeyCode::Tab => eo_ui::key::TAB,
                        _ => 0,
                    };
                }
                WindowEvent::RedrawRequested => self.redraw(el),
                _ => {}
            }
        }

        fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
            if let DeviceEvent::MouseMotion { delta } = event {
                if self.gfx.as_ref().is_some_and(|g| g.ui.screen == Screen::Hud) {
                    let s = self.gfx.as_ref().map(|g| g.ui.settings.input.clone()).unwrap_or_default();
                    // MOUSE_LOOK_BASE (rad/px) times the player's sensitivity.
                    self.cam.yaw += delta.0 as f32 * 0.00235 * s.look_x as f32;
                    let inv = if s.invert_y { -1.0 } else { 1.0 };
                    self.cam.pitch = (self.cam.pitch - delta.1 as f32 * 0.002 * s.look_y as f32 * inv).clamp(-1.4, 1.4);
                }
            }
        }

        fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::Quality;
    use eo_render::look::LookOptions;
    use eo_structures::StructureOptions;
    use eo_ui::settings::Settings;

    fn line(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn args(v: &[&str], stored: Option<&Settings>) -> Args {
        Flags::parse_with(&line(v), stored).unwrap_or_else(|e| panic!("{v:?}: {e:#}")).args
    }

    /// `Args` has no PartialEq; its Debug form names every field and value.
    fn same(a: &Args, b: &Args) -> bool {
        format!("{a:?}") == format!("{b:?}")
    }

    fn dbg(l: &LookOptions) -> String {
        format!("{l:?}")
    }

    /// A stored record: the default's, with these three rows.
    fn stored(tier: &str, shadows: &str, roads: &str) -> Settings {
        let mut s = Settings::default();
        s.performance.lighting = tier.into();
        s.performance.shadows = shadows.into();
        s.world.structures = roads.into();
        s
    }

    #[test]
    fn a_fresh_install_changes_nothing() {
        let d = Settings::default();
        for v in [&["--ui"][..], &["--ui", "--no-kit"], &["--ui", "--quality", "high"]] {
            assert!(same(&args(v, Some(&d)), &args(v, None)), "{v:?}");
        }
        assert_eq!(args(&["--ui"], Some(&d)).quality, Quality::Medium);
    }

    #[test]
    fn the_stored_tier_expands_as_quality_does() {
        for q in Quality::ALL {
            let a = args(&["--ui"], Some(&stored(q.name(), "on", "off")));
            assert_eq!(a.quality, q);
            assert!(same(&a, &args(&["--ui", "--quality", q.name()], None)), "{q:?}");
        }
        // each preset: its tier, and Low's "Sun shadows" off as `--no-shadows`.
        for p in eo_ui::settings::presets() {
            let mut s = Settings::default();
            s.performance.apply_preset(p.key);
            let mut want = vec!["--ui", "--quality", p.fields.lighting.as_str()];
            if p.fields.shadows == "off" {
                want.push("--no-shadows");
            }
            assert!(same(&args(&["--ui"], Some(&s)), &args(&want, None)), "{}", p.key);
        }
        let off = args(&["--ui"], Some(&stored("medium", "off", "off")));
        assert!(!off.look.shadows);
        assert!(same(&off, &args(&["--ui", "--no-shadows"], None)));
        assert!(args(&["--ui"], Some(&stored("medium", "on", "off"))).look.shadows);
    }

    #[test]
    fn a_switch_on_the_line_wins_over_the_stored_settings() {
        let hyper = stored("hyper", "on", "tunnels");
        // the line's `--quality`, wherever it stands.
        assert_eq!(args(&["--ui", "--quality", "low"], Some(&hyper)).quality, Quality::Low);
        assert!(same(&args(&["--quality", "medium", "--ui"], Some(&stored("hyper", "on", "off"))), &args(&["--ui"], None)));
        // a feature's own switch over the stored tier; the rest of the tier stays.
        for v in [&["--ui", "--no-grime", "--far-shadows", "1"][..], &["--no-grime", "--ui", "--far-shadows", "1"]] {
            let a = args(v, Some(&hyper));
            assert_eq!((a.quality, a.look.grime, a.look.lighting.far_shadows, a.look.lighting.lit_fog, a.facade_clutter), (Quality::Hyper, 0, 1, 2, 2), "{v:?}");
        }
        assert_eq!(args(&["--ui", "--no-facade-clutter"], Some(&hyper)).facade_clutter, 0);
        // the roads: the line's last word.
        assert_eq!(args(&["--ui", "--no-structures"], Some(&hyper)).structures, None);
        assert_eq!(args(&["--ui", "--no-tunnels"], Some(&hyper)).structures, Some(StructureOptions { tunnels: false }));
        assert_eq!(args(&["--ui", "--structures", "--tunnels"], Some(&stored("medium", "on", "off"))).structures, Some(StructureOptions { tunnels: true }));
    }

    #[test]
    fn the_roads_row_is_the_world_switches_and_nothing_else() {
        for (roads, want) in [("off", None), ("on", Some(false)), ("tunnels", Some(true))] {
            let a = args(&["--ui"], Some(&stored("medium", "on", roads)));
            assert_eq!(a.structures.as_ref().map(|s| s.tunnels), want, "{roads}");
            let mut b = a.clone();
            b.structures = None;
            assert!(same(&b, &args(&["--ui"], None)), "{roads}: nothing else moves");
        }
        // no tier sets the roads.
        for q in Quality::ALL {
            assert_eq!(args(&["--ui"], Some(&stored(q.name(), "on", "off"))).structures, None);
        }
    }

    #[test]
    fn only_the_window_reads_the_settings_file() {
        let apply = |v: &[&str]| Flags::parse_with(&line(v), None).unwrap().apply;
        assert!(apply(&["--ui"]));
        assert!(!apply(&["--ui", "--no-settings-apply"]));
        assert!(!apply(&["--capture-ui", "settings"]));
        assert!(!apply(&["--ui", "--capture-ui", "all"]));
        assert!(!apply(&["--ui-bench"]));
    }

    #[test]
    fn a_change_in_the_menu_applies_live_where_the_feature_allows() {
        let look = |q: &str| args(&["--ui", "--quality", q], None).look;
        // the same options: nothing moves.
        for q in Quality::ALL {
            assert_eq!(dbg(&live_look(&look(q.name()), &look(q.name()))), dbg(&look(q.name())), "{q:?}");
        }
        let (m, hi, h) = (look("medium"), look("high"), look("hyper"));
        // started at medium, Hyper real chosen: the live features take hyper's values ...
        let l = live_look(&m, &h);
        assert_eq!((l.shadow_map, l.sky.quality, l.shadow_slope_bias, l.photo_finish), (h.shadow_map, h.sky.quality, true, 2));
        let g = l.lighting;
        assert_eq!((g.cascade_blend, g.light_shadows, g.light_shadow_moving, g.headlight_shadows, g.lit_fog, g.smooth_time), (true, 3, true, 8, 2, true));
        // ... what is placed at start waits (medium's own: its grime, since medium is the street look), and what is attached at start stays off.
        assert_eq!((l.grime, l.street_life, l.sky.shop_light), (m.grime, 0, 0));
        assert_eq!(m.grime, 1);
        assert_eq!((g.far_shadows, g.ambient_volume, g.probe_grid, g.ground_bounce), (0, 0, 0, false));
        // started at hyper, medium chosen: medium's frame but for what is placed at start; the attached go off.
        let l = live_look(&h, &m);
        assert_eq!((l.grime, l.street_life, l.sky.shop_light), (2, 2, 2));
        let mut rest = l;
        (rest.grime, rest.street_life, rest.sky.shop_light) = (m.grime, m.street_life, m.sky.shop_light);
        assert_eq!(dbg(&rest), dbg(&m));
        // started at high, Hyper real chosen: the attached keep the level they were attached at
        // (the probe grid is hyper's alone, stage T2: not attached at high, it stays off).
        let g = live_look(&hi, &h).lighting;
        assert_eq!((g.far_shadows, g.ambient_volume, g.probe_grid, g.ground_bounce), (1, 1, 0, true));
        let g = live_look(&look("high"), &look("high")).lighting;
        assert_eq!(g.probe_grid, 0);
        // "Sun shadows" off and on again at once.
        let off = args(&["--ui"], Some(&stored("high", "off", "off"))).look;
        assert!(!live_look(&hi, &off).shadows);
        assert!(live_look(&off, &hi).shadows);
    }
}
