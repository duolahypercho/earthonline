//! Streamed content at run time: the kit texture maps, the Blender-built
//! buildings' models and (slice 3) the kit buildings' packs. Everything goes
//! through one budgeted upload queue (`eo_render::upload`): at most
//! `--upload-budget-mb` bytes are written to the GPU per frame, a large model
//! or pack is spread over several frames, and its object entries and LOD
//! readiness are written only once all of its data is on the GPU.

use crate::{Args, World, cache_dir};
use anyhow::Result;
use eo_assets::LoadSource;
use eo_render::authored::{ArenaBase, GpuLodGroup, Streamer};
use eo_render::kit::{KitArena, KitRuntime};
use eo_render::scene::{GpuObject, Vertex};
use eo_render::upload::{Data, Job, PumpStats, Target, UploadQueue, Write};
use eo_render::{GpuContext, Renderer, RendererInit, hitch, wgpu};
use glam::Vec3;
use std::time::Instant;

pub struct Bases {
    pub models: ArenaBase,
    pub kit: KitArena,
    pub street: eo_render::street::StreetArena,
    /// Some when the Blender models share an arena of these
    /// vertices and indices (view detail on), instead of a slot each.
    pub model_arena: Option<(u32, u32)>,
    /// the near packs' own part of the kit arena (view detail on).
    pub kit_near: Option<(u32, u32, u32)>,
    /// The far layer's GPU side is attached. It needs multi-draw-indirect-count
    /// (absent on Metal); without it the far layer's CPU side is not started
    /// either, since nothing would draw what it bakes and it would never settle.
    pub far_layer: bool,
    /// the grade-separated roads' part of the arenas, after the street's
    /// (all zero unless `--structures` built a non-empty world; structures.rs).
    pub structures: eo_render::structures::StructuresArena,
    /// where their groups and materials are (default unless drawn).
    pub structures_placement: eo_render::structures::Placement,
}

/// Slice 4 street arena (near packs): vertices, indices, objects.
pub const STREET_ARENA: (u32, u32, u32) = (1_500_000, 2_250_000, 1024);

/// The kit arena's sizes without `--kit-arena-*` (main.rs defaults).
const KIT_ARENA_DEFAULT: (u32, u32, u32) = (16_000_000, 26_000_000, 32_768);

/// with view detail on the arenas are sized from the peaks the
/// bench measured with it on (README "View-gated detail"), with a margin;
/// `--full-arenas` (or `--no-view-detail`) keeps today's sizes. The kit
/// arena is split: the mid packs (radial) and the near packs (in view) each
/// have a part of their own, so near packs can never keep a mid pack out.
/// A part that fills defers the pack (it shows the other rung or the box)
/// until space frees.
/// Measured with view detail on and today's arenas (2026-11-15, nine bench
/// paths incl. Castro, the drive and the elevated sweep, `--stream-all-paths`):
/// resident mid packs at most 8.30 M vertices / 12.90 M indices / 3,413
/// objects, near packs 2.50 M / 3.82 M / 394, the street 163 k / 235 k / 52.
/// Sized at about 1.33x (mid), 1.4x (near) and 2.5x (street).
pub const VD_KIT_MID: (u32, u32, u32) = (11_000_000, 17_000_000, 4_608);
pub const VD_KIT_NEAR: (u32, u32, u32) = (3_500_000, 5_400_000, 640);
pub const VD_STREET_ARENA: (u32, u32, u32) = (400_000, 600_000, 256);

/// the most model vertices and indices that can stand within
/// MODEL_DROP_M of any point (a 40 m grid over the models, each counted
/// within MODEL_DROP_M + the grid's half diagonal): the model arena never
/// fills, whatever the view.
pub fn model_arena_bound(set: &eo_render::authored::AuthoredSet) -> (u32, u32) {
    if set.buildings.is_empty() {
        return (0, 0);
    }
    let pts: Vec<(Vec3, f32, u32, u32)> = set
        .buildings
        .iter()
        .map(|b| {
            let g = &set.groups[b.group as usize];
            (Vec3::new(g.center[0], 0.0, g.center[2]), g.radius, b.slot.vertex_count, b.slot.index_count)
        })
        .collect();
    let step = 40.0f32;
    let reach = MODEL_DROP_M + step * std::f32::consts::FRAC_1_SQRT_2;
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &pts {
        lo = lo.min(p.0);
        hi = hi.max(p.0);
    }
    let (mut best_v, mut best_i) = (0u64, 0u64);
    let mut x = lo.x - MODEL_DROP_M;
    while x <= hi.x + MODEL_DROP_M {
        let mut z = lo.z - MODEL_DROP_M;
        while z <= hi.z + MODEL_DROP_M {
            let c = Vec3::new(x, 0.0, z);
            let (mut v, mut i) = (0u64, 0u64);
            for p in &pts {
                if (p.0 - c).length() - p.1 <= reach {
                    v += p.2 as u64;
                    i += p.3 as u64;
                }
            }
            best_v = best_v.max(v);
            best_i = best_i.max(i);
            z += step;
        }
        x += step;
    }
    (best_v.min(set.vertex_capacity as u64) as u32, best_i.min(set.index_capacity as u64) as u32)
}

/// Threads baking the ambient volume (`--ambient-volume`; below the render thread).
const AMBIENT_THREADS: usize = 4;
/// (`--look cinematic`, stage L2) the street lamp palette's cell (m): fine
/// enough that a lamp near a corner finds its own street, not the crossing one.
const PALETTE_CELL_M: f64 = 16.0;

pub fn make_renderer(ctx: &GpuContext, world: &World, args: &Args, format: wgpu::TextureFormat, debug_ids: bool) -> Result<(Renderer, Bases)> {
    let set = &world.authored;
    let mut objects = world.scene.objects.clone();
    let models = ArenaBase {
        vertex: world.scene.vertices.len() as u32,
        index: world.scene.indices.len() as u32,
        object: objects.len() as u32,
    };
    // the arenas' sizes (today's with --no-view-detail or --full-arenas).
    let sized = args.perf.view_detail.enabled && !args.perf.full_arenas;
    let model_arena = (sized && !set.buildings.is_empty()).then(|| model_arena_bound(set));
    let (model_v, model_i) = model_arena.unwrap_or((set.vertex_capacity, set.index_capacity));
    let kit_default = (args.kit_vertices, args.kit_indices, args.kit_objects) == KIT_ARENA_DEFAULT;
    let kit_near = (sized && kit_default).then_some(VD_KIT_NEAR);
    let (kit_v, kit_i, kit_o) = match kit_near {
        Some(n) => (VD_KIT_MID.0 + n.0, VD_KIT_MID.1 + n.1, VD_KIT_MID.2 + n.2),
        None => (args.kit_vertices, args.kit_indices, args.kit_objects),
    };
    let street_arena = if sized { VD_STREET_ARENA } else { STREET_ARENA };
    if sized {
        eprintln!(
            "viewdetail: arenas sized for view detail: models {model_v} vertices / {model_i} indices (every slot: {} / {}), kit {kit_v} / {kit_i} / {kit_o} (near part {:?}), street {:?}",
            set.vertex_capacity, set.index_capacity, kit_near, street_arena
        );
    }
    objects.extend(set.placeholder_objects());
    let mut groups: Vec<GpuLodGroup> = set.groups.clone();
    let mut kit = KitArena::default();
    if let Some(k) = &world.kit {
        assert_eq!(groups.len() as u32, k.group_base);
        groups.extend_from_slice(&k.groups);
        kit = KitArena {
            vertex_base: models.vertex + model_v,
            vertex_capacity: kit_v,
            index_base: models.index + model_i,
            index_capacity: kit_i,
            object_base: objects.len() as u32,
            object_capacity: kit_o,
        };
        objects.extend(std::iter::repeat_n(GpuObject::zeroed_kit(), kit_o as usize));
    }
    let mut street = eo_render::street::StreetArena::default();
    if let Some(s) = &world.street {
        assert_eq!(groups.len() as u32, s.group_base);
        groups.extend_from_slice(&s.groups);
        street = eo_render::street::StreetArena {
            vertex_base: models.vertex + model_v + kit.vertex_capacity,
            vertex_capacity: street_arena.0,
            index_base: models.index + model_i + kit.index_capacity,
            index_capacity: street_arena.1,
            object_base: objects.len() as u32,
            object_capacity: street_arena.2,
        };
        objects.extend(std::iter::repeat_n(GpuObject::zeroed_street(), street_arena.2 as usize));
    }
    // the grade-separated roads' arena after the street's: reserved only with
    // `--structures` and a non-empty world (else zero, and nothing above moves).
    let structures = crate::structures::arena(world, models.vertex + model_v + kit.vertex_capacity + street.vertex_capacity, models.index + model_i + kit.index_capacity + street.index_capacity, objects.len() as u32);
    objects.extend(std::iter::repeat_n(eo_render::structures::zeroed_object(), structures.object_capacity as usize));
    if let Some(l) = &world.landmarks {
        // one LOD group per landmark (the set's near/far switch), after the street's.
        assert_eq!(groups.len() as u32, l.group_base);
        groups.extend_from_slice(&l.groups);
    }
    if let Some(h) = &world.hlod {
        // the code-built authored buildings' groups (the far layer switches their box), last.
        assert_eq!(groups.len() as u32, h.extra_group_base);
        groups.extend_from_slice(&h.extra_groups);
    }
    // the grade-separated roads' groups after every other (no group above
    // moves), their concrete materials after the set's, their static objects
    // in the slots reserved above (only with `--structures` and a non-empty world).
    let materials_with_structures: Vec<eo_render::authored::GpuMaterial>;
    let mut materials: &[eo_render::authored::GpuMaterial] = &set.materials.gpu;
    let mut structures_placement = eo_render::structures::Placement::default();
    if let Some(p) = crate::structures::drawn(world) {
        structures_placement = crate::structures::placement(world, groups.len() as u32, set.materials.gpu.len() as u32);
        groups.extend(p.groups());
        materials_with_structures = set.materials.gpu.iter().copied().chain(p.materials()).collect();
        materials = &materials_with_structures;
        let first = structures.object_base as usize;
        objects.splice(first..first + structures.object_capacity as usize, p.objects(structures_placement, structures));
    }
    // the opt-in features' shader code: what is off at start is compiled out of every pipeline (stage P1;
    // `EO_SHADER_FEATURES=all`, a diagnostic: every feature's code compiled in, as before the stage).
    let features = eo_render::look::ShaderFeatures::of(&args.look, crate::structures::drawn(world).is_some(), args.structures.as_ref().is_some_and(|s| s.tunnels), args.facade_clutter > 0);
    let mut features = if std::env::var("EO_SHADER_FEATURES").is_ok_and(|v| v == "all") { eo_render::look::ShaderFeatures::ALL } else { features };
    // the facades track's switches (`--facade-glass`, `--far-facades`, `--facade-weather`, `--canyon-glass`; off: compiled out, the frame as before).
    features.facade_glass |= eo_render::facade::glass_from_args();
    features.far_facades |= eo_render::facade::far_from_args();
    features.facade_weather |= eo_render::facade::weather_from_args();
    features.canyon_glass |= eo_render::facade::canyon_from_args();
    for w in std::env::var("EO_SHADER_FEATURES").unwrap_or_default().split(',') {
        match w {
            "lx" => features.lx_bits = u32::MAX,
            "grime" => features.grime = true,
            "street_life" => features.street_life = true,
            "structures" => features.structures = true,
            "tunnels" => features.tunnels = true,
            "shop_light" => features.shop_light = true,
            "clutter" => features.facade_clutter = true,
            "cinematic" => features.cinematic = true, // (`--look cinematic`, stage L0)
            "facade_glass" => features.facade_glass = true,
            "far_facades" => features.far_facades = true,
            "facade_weather" => features.facade_weather = true,
            "canyon_glass" => features.canyon_glass = true,
            "street_pbr" => features.street_pbr = true, // (`--street-pbr`)
            "wet_surfaces" => features.wet_surfaces = true, // (`--wet-surfaces`)
            "screen_reflect" => features.screen_reflect = true, // (`--screen-reflect`)
            _ => {}
        }
    }
    features.street_pbr |= crate::street_pbr(); // the street's physically based surface (`--street-pbr`; no runtime gate, so only with its switch)
    eo_render::look::set_shader_features(features);
    let mut r = Renderer::from_init(
        ctx,
        &RendererInit {
            vertices: &world.scene.vertices,
            indices: &world.scene.indices,
            objects: &objects,
            extra_vertices: model_v + kit.vertex_capacity + street.vertex_capacity + structures.vertex_capacity, // model_v / model_i
            extra_indices: model_i + kit.index_capacity + street.index_capacity + structures.index_capacity,
            materials,
            groups: &groups,
            hlod_enter_m: set.hlod_enter_m,
            hlod_exit_m: set.hlod_exit_m,
            kit_layers: set.materials.layers.len() as u32
                + u32::from(set.materials.by_name(eo_render::seabed::rock_maps::NAME).is_some()),
            kit_res: eo_assets::kit::HIGH_RES as u32,
            color_format: format,
            sample_count: args.msaa,
            multiplier: args.multiply,
            debug_ids,
            blend_first_object: models.object,
        },
    )?;
    r.kit_radii = eo_render::kit::DEFAULT_KIT_RADII;
    r.set_occlusion(args.perf.occlusion); // occlusion
    // the far layer's GPU side and every group's cell.
    let mut far_layer = false;
    if let Some(h) = &world.hlod {
        if r.attach_hlod(ctx, &eo_render::hlod::HlodOptions::from_env()) > 0 {
            far_layer = true;
            r.write_hlod_map(&ctx.queue, &h.group_cell);
            let b = r.hlod.get().map(|g| g.bytes).unwrap_or_default();
            r.bytes.vertex += b[0];
            r.bytes.index += b[1];
            r.bytes.textures += b[2];
            r.bytes.draws += b[3];
        }
    }
    r.set_view_detail(args.perf.view_detail); // the best-quality layer by view (`--no-view-detail`)
    eo_render::hitch::set_worker_priority_lowered(args.perf.thread_priority); // workers started after this
    // the clock and the shadow quality.
    r.look.set_time_of_day(args.time_of_day);
    r.look.set_options(&ctx.device, args.look);
    // surface weathering (`--grime`): its street decals (kerb paint, building-line dirt) placed from the first tile on.
    eo_render::cc0_decals::set_weathering(args.look.grime > 0);
    // street life (`--street-life`): its level, and the land-use survey its decals place by (read beside the load).
    crate::street_life::start(args.look.street_life, world.city.clone(), world.paths.repo_root.clone(), world.structures.clone());
    // the street's scanned sets (written once, before the first frame).
    if let Some((maps, _)) = &world.street_maps {
        r.bytes.textures += r.upload_street_maps(ctx, maps);
    }
    // the switches, the Blender buildings' weathering records, the detail sheet.
    r.set_facade_flags(args.look.facade);
    {
        let mut recs = vec![[[0f32; 4]; 13]; set.groups.len()];
        for b in &set.buildings {
            if let Some(slot) = recs.get_mut(b.group as usize) {
                *slot = b.weather;
            }
        }
        r.facade.write_weather(&ctx.queue, &recs);
    }
    if let Some(d) = &world.facade_detail {
        r.bytes.textures += r.upload_facade_detail(ctx, d);
    }
    // the CC0 sets' arrays, tables and decal buffers, when the downloads are there (else the renderer's own materials).
    r.cc0.set_options(args.look.cc0);
    if args.look.cc0 != 0 {
        let t = std::time::Instant::now();
        match crate::cc0_runtime::availability(ctx) {
            Ok(()) => {
                let checked = t.elapsed().as_secs_f64() * 1e3;
                let b = r.attach_cc0(ctx);
                r.bytes.textures += b;
                eprintln!("cc0: downloads present ({checked:.0} ms to check), {:.1} MB of arrays, tables and decal buffers allocated in {:.0} ms", b as f64 / 1e6, t.elapsed().as_secs_f64() * 1e3 - checked);
            }
            Err(e) => crate::cc0_runtime::say_unavailable(&e),
        }
    }
    if let Some(l) = &world.landmarks {
        r.bytes.vertex += r.attach_landmark_wires(ctx, &l.wire_vertices, &l.wire_indices); // the wire members
    }
    // the sea over the terrain grid's extent (where the flat sea quad was).
    if let Some(data) = world.water.as_deref().filter(|_| args.look.water) {
        let g = &world.elevation;
        let extent = [g.meta.origin_x as f32, g.meta.origin_z as f32, g.max_x() as f32, g.max_z() as f32];
        r.attach_water(ctx, &eo_render::water::WaterInit { data, extent });
        if let Some(w) = r.water.get() {
            r.bytes.textures += w.bytes;
        }
        // the surf strip along the mapped beaches (eo-water surf_strip, eo-render surf.rs); drawn unless `EO_SURF=0`
        if eo_render::surf::enabled() {
            let t = std::time::Instant::now();
            let beaches = eo_water::load_beaches(&world.paths.repo_root).unwrap_or_default();
            let strips = eo_water::surf_strip::build_strips(&data.field, &beaches);
            let built = t.elapsed().as_secs_f64() * 1e3;
            r.bytes.vertex += r.attach_surf(ctx, &strips);
            if std::env::var("EO_TRACE_LOAD").is_ok() || std::env::var("EO_SURF_TRACE").is_ok() {
                for st in &strips {
                    let p = st.points[st.points.len() / 2];
                    eprintln!("surf: strip {:.0} m long at ({:.0}, {:.0}), {}", st.length(), p[0], p[1], if st.climate.pacific { "Pacific" } else { "bay" });
                }
                eprintln!("surf: {} strips built in {built:.0} ms, attached in {:.0} ms", strips.len(), t.elapsed().as_secs_f64() * 1e3 - built);
            }
        }
    }
    r.bytes.textures += crate::farshore::attach(ctx, &r, world); // the far terrain
    r.bytes.vertex += crate::structures::attach(ctx, &r, world, structures_placement, structures); // the grade-separated roads' static levels (built during the load; nothing without them)
    // the ambient volume (`--ambient-volume`): the sky's visibility baked round the camera and over the city (lighting).
    if args.look.lighting.ambient_volume > 0 {
        let lift = eo_assets::placement::PlacementConstants::load()?.construction_lift_m;
        let inputs = eo_render::ambient::AmbientInputs::city(world.city.clone(), lift, std::sync::Arc::new(world.elevation.clone()), world.structures.clone(), cache_dir(args), AMBIENT_THREADS);
        r.bytes.textures += r.ambient.attach(ctx, &r.look, inputs, [0.0, 0.0]);
    }
    // the far sun shadow maps (`--far-shadows`): a ring (two at level 2) past the cascades (lighting).
    if args.look.lighting.far_shadows > 0 {
        let b = r.attach_far_shadows(ctx, args.look.lighting.far_shadows);
        r.bytes.textures += b;
        eprintln!("far shadows: {} ring(s) of {} px, {:.1} MB of maps, lists and uniforms", r.far_shadow.rings(), eo_render::far_shadow::FAR_MAP_SIZE, b as f64 / 1e6);
    }
    // the probe grid (`--probe-grid`): reflection probes on the streets round the camera, in the single probe's place (lighting).
    if args.look.lighting.probe_grid > 0 {
        match &world.street {
            Some(s) => {
                let t = std::time::Instant::now();
                let anchors = eo_render::probe_grid::Anchors::from_streets(&s.world);
                let n = anchors.len();
                let b = r.attach_probe_grid(ctx, args.look.lighting.probe_grid, anchors);
                r.bytes.textures += b;
                let lv = r.probe_grid.level();
                eprintln!(
                    "probe grid: {} probes of {} px in {} slots from {n} street anchors ({:.0} ms), {:.1} MB of cubes, face target and table",
                    lv.map(|l| l.probes).unwrap_or(0),
                    lv.map(|l| l.face).unwrap_or(0),
                    lv.map(|l| l.slots).unwrap_or(0),
                    t.elapsed().as_secs_f64() * 1e3,
                    b as f64 / 1e6
                );
            }
            None => eprintln!("probe grid: no streets to anchor it on; the single probe stays"),
        }
    }
    // the street lamps the city is lit by at night. Until the props slice
    // delivers its per-lamp records, the JS fallback placement (eo-sky lights)
    // on the street contract's segments, standing on the drawn surface.
    if let Some(s) = &world.street {
        let t = std::time::Instant::now();
        let segs: Vec<eo_sky::lights::SegmentIn> = s
            .world
            .city
            .segments
            .iter()
            .map(|g| eo_sky::lights::SegmentIn {
                highway: g.highway.as_str(),
                width: g.width,
                sidewalk_left: g.sidewalk_left.or(g.sidewalk_w).unwrap_or(0.0),
                sidewalk_right: g.sidewalk_right.or(g.sidewalk_w).unwrap_or(0.0),
                points: &g.points,
            })
            .collect();
        // The walk level from the elevation grid and the road surface lift (the
        // drawn surface's own query is ~6 ms a lamp; a head 5-9 m up does not
        // need the kerb's 0.15 m).
        let lift = world.city.meta.street_design.as_ref().and_then(|d| d.road_surface_lift).unwrap_or(0.0);
        let mut lamps = eo_sky::lights::place_street_lamps(&segs, |x, z| world.elevation.ground(x, z) + lift); // mut
        // the landmarks' lights in the same gather, first so the cap
        // keeps them (none: the JS bridges and pier carry no lights).
        if let Some(l) = &world.landmarks {
            lamps.splice(0..0, eo_render::landmarks::sky_lamps(&l.lights));
        }
        let n = r.look.sky.set_lamps(&ctx.queue, &lamps);
        // (`--look cinematic`, stage L2) the street lamp palette (eo-sky
        // `cine_lamps`): each lamp street's colour and axis on a grid from the
        // elevation grid's corner, for the gather (near lamps) and here for the
        // far lamps; made only with the look compiled in.
        let mut far_lamps = None;
        if eo_render::look::shader_features().cinematic {
            let g = &world.elevation;
            let corner = [g.meta.origin_x, g.meta.origin_z];
            let span = (g.max_x() - corner[0]).max(g.max_z() - corner[1]);
            let cell = PALETTE_CELL_M.max((span / eo_render::sky::PALETTE_CELLS_MAX as f64).ceil());
            let cols = (((g.max_x() - corner[0]) / cell).ceil() as usize).min(eo_render::sky::PALETTE_CELLS_MAX);
            let rows = (((g.max_z() - corner[1]) / cell).ceil() as usize).min(eo_render::sky::PALETTE_CELLS_MAX);
            let streets: Vec<eo_sky::cine_lamps::StreetIn> = s
                .world
                .city
                .segments
                .iter()
                .map(|g| eo_sky::cine_lamps::StreetIn { name: if g.street_name.is_empty() { g.id.as_str() } else { g.street_name.as_str() }, highway: g.highway.as_str(), width: g.width, points: &g.points })
                .collect();
            let grid = eo_sky::cine_lamps::PaletteGrid::build(corner, cell, cols, rows, &streets);
            r.look.sky.set_lamp_palette(&ctx.queue, &grid);
            if r.look.sky.options().cinematic {
                far_lamps = Some(eo_sky::cine_lamps::recolour(&lamps, &grid));
            }
        }
        if world.hlod.is_some() {
            r.bytes.draws += r.attach_far_lights(ctx, far_lamps.as_deref().unwrap_or(&lamps)); // stage 3, the same lamps far away
        }
        // By the water: the cells whose centre is within WATERSIDE_M of the
        // shore (eo-water's shore field, + over water); a lamp in one, the
        // fallback's or the props slice's, is drawn in the sea too.
        const WATERSIDE_M: f64 = 40.0;
        const CELL_M: f64 = 32.0;
        if let Some(w) = world.water.as_deref() {
            let g = &world.elevation;
            let corner = [g.meta.origin_x, g.meta.origin_z];
            // the cell grows when the grid is wider than 512 cells of
            // 32 m (the ground plan x1.25 made it so, and the east of the grid,
            // the Embarcadero, was cut off: its lamps drew no column in the bay)
            let span = (g.max_x() - corner[0]).max(g.max_z() - corner[1]);
            let cell_m = CELL_M.max((span / eo_render::sky::WATER_CELLS_MAX as f64).ceil());
            let cols = (((g.max_x() - corner[0]) / cell_m).ceil() as usize).min(eo_render::sky::WATER_CELLS_MAX);
            let rows = (((g.max_z() - corner[1]) / cell_m).ceil() as usize).min(eo_render::sky::WATER_CELLS_MAX);
            let cells: Vec<bool> = (0..rows * cols)
                .map(|i| {
                    let (c, r) = ((i % cols) as f64 + 0.5, (i / cols) as f64 + 0.5);
                    w.field.signed_at(corner[0] + c * cell_m, corner[1] + r * cell_m) > -WATERSIDE_M
                })
                .collect();
            r.look.sky.set_waterside(&ctx.queue, corner, cell_m, cols, rows, &cells);
        }
        eprintln!(
            "sky: {n} street lamps placed (JS fallback placement, standing down where the props slice's lamps are resident; {} cells by the water) in {:.0} ms",
            r.look.sky.waterside_count(),
            t.elapsed().as_secs_f64() * 1e3
        );
    }
    // stage 4, the distant trees' pipeline and buffer; the list (gathered on
    // the load's own thread) is written in by `ModelRuntime::pump` when it is ready.
    if (world.hlod.is_some() || args.cove_detail) && world.hlod_trees.is_some() {
        r.bytes.draws += r.attach_far_trees(ctx);
    }
    Ok((r, Bases { models, kit, street, model_arena, kit_near, far_layer, structures, structures_placement })) // + model_arena, kit_near, far_layer, structures
}

#[derive(Clone, Debug, Default)]
pub struct StreamStats {
    pub resident: usize,
    pub from_cache: usize,
    pub cooked: usize,
    pub failed: Vec<(String, String)>,
    pub triangles_resident: u64,
    pub bytes_uploaded: u64,
    /// Sum of per-model load time on the worker threads.
    pub worker_ms: f64,
    pub kit_bake_ms: Option<f64>,
    /// Process start -> kit maps on the GPU / every model resident.
    pub kit_ready_at_ms: Option<f64>,
    pub all_resident_at_ms: Option<f64>,
    /// Longest single `pump` call (CPU on the render thread).
    pub max_pump_ms: f64,
    /// Most bytes written in one `pump` (all streamed content).
    pub max_pump_bytes: u64,
}

/// What one `pump` did (per frame).
#[derive(Clone, Copy, Debug, Default)]
pub struct PumpFrame {
    pub bytes: u64,
}

/// where one Blender building's full model stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelState {
    Absent,
    /// Asked of the streamer (pending or loading on a worker).
    Requested,
    /// Its upload job is queued.
    Queued,
    Resident,
    /// Refused or failed to load (its footprint stays; never asked again).
    Failed,
}

/// a full model is requested while its bounds are within
/// `MODEL_LOAD_M` of the camera and it is in the widened view (or was, within
/// the hold), and dropped past `MODEL_DROP_M` or once out of view past the hold.
/// The GPU's HLOD rule (320 m in, 380 m out, to the bounds) still decides
/// which level draws; a model not resident draws its footprint.
pub const MODEL_LOAD_M: f32 = 360.0;
pub const MODEL_DROP_M: f32 = 420.0;
/// every model whose bounds are within this of the camera is
/// wanted in every direction (no view test): a Blender building has no middle
/// level, so turning toward one this close must never show its footprint box.
pub const MODEL_RING_M: f32 = 150.0;
/// Resident models evicted per frame at most.
const MODEL_EVICTIONS_PER_FRAME: usize = 4;

/// per-model state and what it costs, with the view gate on or off.
#[derive(Clone, Debug, Default)]
pub struct ModelResidency {
    pub resident: usize,
    pub resident_vertex_bytes: u64,
    pub resident_index_bytes: u64,
    pub peak_bytes: u64,
    pub uploaded: usize,
    pub evicted: usize,
    pub cancelled: usize,
    pub dropped_unneeded: usize,
    pub out_of_view: usize,
    /// Vertices and indices resident, and the most at once.
    pub vertices: u64,
    pub indices: u64,
    pub peak_vertices: u64,
    pub peak_indices: u64,
    /// Models that arrived while the model arena was full (dropped; asked again once it frees).
    pub arena_full: usize,
}

pub struct ModelRuntime {
    /// every Blender building's model state (both modes), and
    /// whether the view gates them (else all load at start, as before).
    pub model_state: Vec<ModelState>,
    pub model_gated: bool,
    pub residency: ModelResidency,
    model_hold: eo_render::view_detail::ViewHold<usize>,
    model_radius: f32,
    /// with the model arena (view detail on), its vertex and index
    /// allocators, each model's ranges in it, and whether it filled (no new
    /// model is asked for until one is evicted).
    model_alloc: Option<(eo_render::kit::RangeAlloc, eo_render::kit::RangeAlloc)>,
    model_ranges: Vec<Option<(u32, u32)>>,
    model_wait_free: bool,
    streamer: Option<Streamer>,
    kit_rx: Option<std::sync::mpsc::Receiver<Result<(eo_render::KitUpload, f64), String>>>,
    /// (`--shop-light`) the shop windows' lights, placed from the kit planner's
    /// shop bays on a thread of their own once the kit city is there (handed
    /// to the sky on arrival; None without the option or once handed over),
    /// and (stage N2) the shops' fronts table for the rooms (empty: none).
    shop_rx: Option<std::sync::mpsc::Receiver<(Vec<eo_render::shop_lights::GpuShopLight>, Vec<[u32; 4]>)>>,
    kit_ready: bool,
    kit_queued: bool,
    pub uploads: UploadQueue,
    pub kit: Option<KitRuntime>,
    pub street: Option<eo_render::street::StreetRuntime>,
    /// the grade-separated roads' near packs (`--structures`).
    pub structures: Option<eo_render::structures::StructuresRuntime>,
    /// the far layer's streamer (cells and districts).
    pub hlod: Option<eo_render::hlod::HlodRuntime>,
    /// the distant trees' list is still to arrive or to be written (0 none, 1 waiting, 2 writing).
    hlod_trees: u8,
    bases: Bases,
    pub stats: StreamStats,
    started: Instant,
    models_in_queue: usize,
    /// the CC0 sets being cooked (or read from the cache) and uploaded.
    pub cc0: Option<crate::cc0_runtime::Cc0Runtime>,
    /// the kit map layers' mean colours (what the CC0 scans are scaled to).
    kit_means: Option<Vec<[f32; 3]>>,
    cc0_cache: Option<std::path::PathBuf>,
}

const TAG_TEXTURES: u64 = 1 << 60;
const TAG_MODEL: u64 = 1 << 61;
/// (`--shop-light`) The shop windows' lights are placed on this many worker
/// threads (the kit planner again for every retail ground floor: ~4 s on 8).
const SHOP_LIGHT_THREADS: usize = 4;

impl ModelRuntime {
    pub fn start(world: &mut World, args: &Args, bases: Bases, camera: Vec3, process_start: Instant) -> Self {
        let gated = args.perf.view_detail.enabled;
        let streamer = (!world.authored.buildings.is_empty()).then(|| {
            Streamer::start_with(
                world.authored.clone(),
                world.paths.repo_root.clone(),
                cache_dir(args),
                bases.models,
                camera,
                args.stream_radius,
                args.stream_threads,
                gated,
            )
        });
        // the planner's city goes to the kit's streamer and the far layer's
        // (only when the renderer attached the far layer's GPU side).
        // (and, `--shop-light`, to the shop windows' lights' placement)
        let want_hlod = world.hlod.is_some() && bases.far_layer;
        let want_shops = args.look.sky.shop_light > 0 && world.kit.is_some();
        let (kit_city_rx, hlod_city_rx, shop_city_rx) = match world.kit_city_rx.take() {
            Some(rx) if want_hlod || want_shops => {
                let (a, ar) = std::sync::mpsc::channel();
                let (b, br) = std::sync::mpsc::channel();
                let (c, cr) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    if let Ok(v) = rx.recv() {
                        let _ = a.send(v.clone());
                        if want_shops {
                            let _ = c.send(v.clone());
                        }
                        if want_hlod {
                            let _ = b.send(v);
                        }
                    }
                });
                (Some(ar), want_hlod.then_some(br), want_shops.then_some(cr))
            }
            rx => (rx, None, None),
        };
        let shop_rx = shop_city_rx.zip(world.kit.clone()).map(|(rx, set)| {
            let level = args.look.sky.shop_light;
            let (tx, out) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                eo_render::hitch::worker_started();
                let Ok(Ok((kc, _))) = rx.recv() else { return };
                let (lights, fronts, st) = eo_render::shop_lights::build_with_fronts(&kc, &set, level, SHOP_LIGHT_THREADS);
                eprintln!(
                    "shop lights: level {level}, {} lights in {} shop windows of {} shops in {} buildings ({} with a retail ground planned) in {:.0} ms; by kind {:?}; vacant {} ({} boarded, {} with a work light)",
                    st.lights, st.bays, st.shops, st.with_bays, st.planned, st.ms, st.by_kind, st.vacant, st.boarded, st.work_lit
                );
                // (stage N2) the rooms' fronts table; `EO_SHOP_INTERIORS=0` (diagnostic) keeps N1's rooms.
                let table = if std::env::var("EO_SHOP_INTERIORS").is_ok_and(|v| v == "0") { vec![] } else { eo_render::shop_lights::fronts_table(&fronts) };
                let _ = tx.send((lights, table));
            });
            out
        });
        let hlod = match (&world.hlod, &world.kit, hlod_city_rx) {
            (Some(set), Some(kit), Some(rx)) => {
                let opts = eo_render::hlod::HlodOptions::from_env();
                opts.on.then(|| {
                    eo_render::hlod::HlodRuntime::start(
                        eo_render::hlod::HlodInputs {
                            set: set.clone(),
                            kit: kit.clone(),
                            authored: world.authored.clone(),
                            city: world.city.clone(),
                            repo_root: world.paths.repo_root.clone(),
                            cache_dir: cache_dir(args),
                        },
                        rx,
                        opts,
                        camera,
                        process_start,
                    )
                })
            }
            _ => None,
        };
        let mut kit = match (&world.kit, kit_city_rx) {
            (Some(set), Some(rx)) => Some(KitRuntime::start(
                set.clone(),
                rx,
                cache_dir(args),
                bases.kit,
                args.kit_radii,
                args.kit_threads,
                camera,
                args.kit_split_materials,
                process_start,
            )),
            _ => None,
        };
        // the near packs' own part of the kit arena.
        if let (Some(k), Some(n)) = (kit.as_mut(), bases.kit_near) {
            k.split_near(n.0, n.1, n.2);
        }
        let street = world.street.as_ref().map(|s| eo_render::street::StreetRuntime::start(s.clone(), bases.street, Default::default(), 2, camera));
        // the grade-separated roads' arena (reserved only for a non-empty `--structures` world): the static levels, then the near packs.
        let sa = bases.structures;
        let structures = crate::structures::drawn(world).filter(|_| !sa.is_empty()).map(|p| {
            let near = p.near_arena(sa);
            eprintln!(
                "structures: arena of {} vertices, {} indices, {} objects reserved after the street's (vertex base {}, object base {}); near packs {} / {} / {} of it",
                sa.vertex_capacity, sa.index_capacity, sa.object_capacity, sa.vertex_base, sa.object_base, near.vertex_capacity, near.index_capacity, near.object_capacity
            );
            eo_render::structures::StructuresRuntime::start(p.set.clone(), bases.structures_placement, near, crate::structures::NEAR_THREADS, camera)
        });
        let model_gated = gated && streamer.is_some();
        Self {
            model_state: vec![ModelState::Absent; world.authored.buildings.len()],
            model_gated,
            residency: Default::default(),
            model_hold: Default::default(),
            model_radius: args.stream_radius,
            model_alloc: bases.model_arena.filter(|_| model_gated).map(|(v, i)| (eo_render::kit::RangeAlloc::new(v), eo_render::kit::RangeAlloc::new(i))),
            model_ranges: vec![None; world.authored.buildings.len()],
            model_wait_free: false,
            streamer,
            street,
            structures,
            hlod_trees: if world.hlod_trees.is_some() && (hlod.is_some() || args.cove_detail) { 1 } else { 0 }, // hlod
            hlod, // hlod
            kit_rx: world.kit_rx.take(),
            shop_rx,
            kit_ready: false,
            kit_queued: false,
            // The kit's group and eviction writes (bounded per frame) come out of the same budget.
            uploads: UploadQueue::new(args.upload_budget.saturating_sub(eo_render::kit::KIT_FRAME_RESERVE_BYTES)).with_ring(args.perf.upload_ring), // the staging ring switch
            kit,
            bases,
            stats: StreamStats::default(),
            started: process_start,
            models_in_queue: 0,
            cc0: None,
            kit_means: None,
            cc0_cache: cache_dir(args),
        }
    }

    pub fn set_camera(&self, eye: Vec3) {
        if let Some(s) = &self.streamer {
            s.set_camera(eye);
        }
    }

    /// One frame of streaming: queue what arrived (kit maps first; models and
    /// kit packs only once the maps are on the GPU, so nothing draws
    /// untextured), then write at most the budget.
    pub fn pump(&mut self, ctx: &GpuContext, renderer: &Renderer, world: &World, cam: Vec3) -> Result<PumpFrame> {
        let t = Instant::now();
        // (`--shop-light`) the shop windows' lights, once placed: to the sky's light gather.
        if let Some(rx) = &self.shop_rx {
            if let Ok((lights, fronts)) = rx.try_recv() {
                renderer.look.sky.set_shop_lights(&ctx.queue, &lights);
                renderer.look.set_shop_fronts(eo_render::shop_lights::fronts_buffer(&ctx.device, &ctx.queue, &fronts).as_ref()); // (stage N2) the shops' rooms
                self.shop_rx = None;
            }
        }
        if let Some(k) = &mut self.kit {
            k.begin_frame();
        }
        if let Some(s) = &mut self.street {
            s.begin_frame();
        }
        if let Some(s) = &mut self.structures {
            s.begin_frame();
        }
        if !self.kit_queued {
            match &self.kit_rx {
                None => {
                    self.kit_queued = true;
                    self.kit_ready = true;
                }
                Some(rx) => {
                    if let Ok(r) = rx.try_recv() {
                        let (maps, ms) = r.map_err(|e| anyhow::anyhow!("kit bake: {e}"))?;
                        self.stats.kit_bake_ms = Some(ms);
                        self.kit_means = Some(crate::cc0_runtime::kit_means(&maps)); // cc0
                        if let Some(h) = &mut self.hlod {
                            h.set_layer_means(eo_render::hlod_bake::layer_means(&maps)); // the far layer's material colours
                        }
                        let mut bulk = vec![];
                        for (layer, chain) in maps.layers.into_iter().enumerate() {
                            for (map, mips) in chain.into_iter().enumerate() {
                                for (mip, data) in mips.into_iter().enumerate() {
                                    bulk.push(Write { target: Target::Texture { map: map as u8, layer: layer as u32, mip: mip as u32 }, offset: 0, data: Data::Bytes(data) });
                                }
                            }
                        }
                        self.uploads.push(Job::new(TAG_TEXTURES, bulk, vec![]));
                        self.kit_queued = true;
                    }
                }
            }
        }
        let mut gate_bytes = 0;
        if self.kit_ready {
            let model_span = hitch::span(hitch::Phase::ModelPoll); // hitch
            if self.model_gated {
                gate_bytes = self.gate_models(ctx, renderer, world, cam);
            }
            if let Some(s) = &mut self.streamer {
                // Take models off the channel only as fast as the queue drains.
                if self.models_in_queue < 4 {
                    for mut up in s.poll_bytes(4 - self.models_in_queue, usize::MAX) {
                        // a model no longer wanted when it arrives is dropped.
                        if self.model_gated && self.model_state[up.building] != ModelState::Requested {
                            self.residency.dropped_unneeded += 1;
                            self.stats.worker_ms += up.load_ms;
                            eo_render::upload::bury(Box::new(up));
                            continue;
                        }
                        let b = &world.authored.buildings[up.building];
                        // with the model arena, ranges of it (else the model's own slot).
                        let (av, ai) = match self.model_alloc.as_mut() {
                            Some((va, ia)) => {
                                let got = va.alloc(b.slot.vertex_count).and_then(|v| match ia.alloc(b.slot.index_count) {
                                    Some(i) => Some((v, i)),
                                    None => {
                                        va.free(v, b.slot.vertex_count);
                                        None
                                    }
                                });
                                let Some(r) = got else {
                                    // Full: the footprint stays; asked for again once a model is evicted.
                                    self.residency.arena_full += 1;
                                    self.model_wait_free = true;
                                    self.model_state[up.building] = ModelState::Absent;
                                    eo_render::upload::bury(Box::new(up));
                                    continue;
                                };
                                self.model_ranges[up.building] = Some(r);
                                r
                            }
                            None => (b.slot.vertex_offset, b.slot.index_offset),
                        };
                        if (av, ai) != (b.slot.vertex_offset, b.slot.index_offset) {
                            let (m, from) = (self.bases.models, b.slot.index_offset);
                            for o in &mut up.objects {
                                o.base_vertex = (m.vertex + av) as i32;
                                o.first_index = o.first_index - (m.index + from) + m.index + ai;
                            }
                        }
                        self.model_state[up.building] = ModelState::Queued;
                        let base_v = self.bases.models.vertex + av;
                        let base_i = self.bases.models.index + ai;
                        let base_o = self.bases.models.object + b.slot.object_offset;
                        let vs = std::mem::size_of::<Vertex>() as u64;
                        let os = std::mem::size_of::<GpuObject>() as u64;
                        let job = Job::new(
                            TAG_MODEL | up.building as u64,
                            vec![
                                Write { target: Target::Vertex, offset: base_v as u64 * vs, data: Data::Vertices(std::mem::take(&mut up.vertices)) },
                                Write { target: Target::Index, offset: base_i as u64 * 4, data: Data::Words(std::mem::take(&mut up.indices)) },
                            ],
                            vec![
                                Write { target: Target::Object, offset: base_o as u64 * os, data: Data::Objects(up.objects.clone()) },
                                Write {
                                    target: Target::Group,
                                    offset: b.group as u64 * std::mem::size_of::<GpuLodGroup>() as u64 + 16,
                                    data: Data::Words(vec![1]),
                                },
                            ],
                        );
                        self.uploads.push(job);
                        self.models_in_queue += 1;
                        self.stats.triangles_resident += up.triangles;
                        self.stats.worker_ms += up.load_ms;
                        match up.source {
                            LoadSource::Cache => self.stats.from_cache += 1,
                            LoadSource::Cooked => self.stats.cooked += 1,
                        }
                    }
                }
                if self.stats.failed.len() != s.failed.len() {
                    self.stats.failed = s
                        .failed
                        .iter()
                        .map(|(i, e)| (world.authored.buildings[i.parse::<usize>().unwrap_or(0)].id.clone(), e.clone()))
                        .collect();
                    // a model that failed is never asked for again (its footprint stays).
                    for (i, _) in &s.failed {
                        if let Some(st) = i.parse::<usize>().ok().and_then(|i| self.model_state.get_mut(i)) {
                            *st = ModelState::Failed;
                        }
                    }
                }
            }
            drop(model_span); // hitch
            if let Some(k) = &mut self.kit {
                let _s = hitch::span(hitch::Phase::KitUpdate); // hitch
                k.update(renderer, &ctx.queue, &mut self.uploads, cam);
            }
        }
        if let Some(s) = &mut self.street {
            let _s = hitch::span(hitch::Phase::StreetUpdate); // hitch
            s.update(renderer, &ctx.queue, &mut self.uploads, cam);
        }
        if let Some(s) = &mut self.structures {
            let _s = hitch::span(hitch::Phase::StreetUpdate); // the grade-separated roads' near packs, in the street's span
            s.update(renderer, &ctx.queue, &mut self.uploads, cam);
        }
        if let Some(h) = &mut self.hlod {
            h.update(renderer, &ctx.queue, cam); // the far layer, its own upload budget
        }
        // the distant trees' list, once gathered, written in a slice a frame.
        if self.hlod_trees > 0 {
            match (renderer.hlod_trees.get(), &world.hlod_trees) {
                (Some(t), Some(rx)) => {
                    if self.hlod_trees == 1 {
                        match rx.try_recv() {
                            Ok(trees) => {
                                t.supply(trees);
                                self.hlod_trees = 2;
                            }
                            // The gathering thread is gone without a list: no distant trees.
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.hlod_trees = 0,
                            Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        }
                    }
                    if self.hlod_trees == 2 {
                        renderer.note_streamed(t.pump(&ctx.queue));
                        if t.idle() {
                            self.hlod_trees = 0;
                        }
                    }
                }
                _ => self.hlod_trees = 0,
            }
        }
        // cooked sets into the queue (two at a time), the tables once every layer is on the GPU.
        if self.cc0.is_none() && renderer.cc0.attached() {
            self.cc0 = Some(crate::cc0_runtime::Cc0Runtime::start(self.cc0_cache.clone(), self.started));
        }
        if let Some(c) = &mut self.cc0 {
            // Without a kit bake there is no kit layer to match (an empty list).
            let km: Option<&[[f32; 3]]> = if self.kit_rx.is_none() { Some(&[]) } else { self.kit_means.as_deref() };
            c.pump(renderer, &ctx.queue, &mut self.uploads, &world.authored.materials, km);
        }
        let up_span = hitch::span(hitch::Phase::Uploads); // hitch
        let (st, done): (PumpStats, Vec<u64>) = self.uploads.pump(renderer, &ctx.queue);
        drop(up_span); // hitch
        let _after_span = hitch::span(hitch::Phase::AfterPump); // hitch
        if let Some(c) = &mut self.cc0 {
            c.after_pump(&done); // cc0
        }
        for &tag in &done {
            if tag == TAG_TEXTURES {
                self.kit_ready = true;
                self.stats.kit_ready_at_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
            } else if tag & TAG_MODEL != 0 && tag & eo_render::kit::TAG_KIT == 0 && tag & eo_render::street::TAG_STREET == 0 {
                self.models_in_queue -= 1;
                self.stats.resident += 1;
                // resident (the job wrote its objects and the ready word).
                let i = (tag & 0xffff_ffff) as usize;
                if let Some(b) = world.authored.buildings.get(i) {
                    self.model_state[i] = ModelState::Resident;
                    let r = &mut self.residency;
                    r.resident += 1;
                    r.uploaded += 1;
                    r.resident_vertex_bytes += b.slot.vertex_count as u64 * std::mem::size_of::<Vertex>() as u64;
                    r.resident_index_bytes += b.slot.index_count as u64 * 4;
                    r.peak_bytes = r.peak_bytes.max(r.resident_vertex_bytes + r.resident_index_bytes);
                    r.vertices += b.slot.vertex_count as u64;
                    r.indices += b.slot.index_count as u64;
                    r.peak_vertices = r.peak_vertices.max(r.vertices);
                    r.peak_indices = r.peak_indices.max(r.indices);
                }
            }
        }
        let mut bytes = st.bytes + gate_bytes; // + eviction writes
        if let Some(k) = &mut self.kit {
            k.after_pump(renderer, &ctx.queue, &done);
            bytes += k.last_group_bytes;
        }
        if let Some(s) = &mut self.street {
            s.after_pump(renderer, &ctx.queue, &done);
            bytes += s.last_group_bytes;
        }
        if let Some(s) = &mut self.structures {
            s.after_pump(renderer, &ctx.queue, &done);
            bytes += s.last_group_bytes;
        }
        self.stats.bytes_uploaded += bytes;
        if self.kit_ready && self.stats.all_resident_at_ms.is_none() {
            // gated, "every model" is every model the view wants.
            let models_done = if self.model_gated { self.models_wanted_resident() } else { self.streamer.as_ref().map(|s| s.received >= s.total && self.models_in_queue == 0).unwrap_or(true) };
            if models_done {
                self.stats.all_resident_at_ms = Some(self.started.elapsed().as_secs_f64() * 1e3);
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1e3;
        if ms > 5.0 && std::env::var("EO_TRACE_PUMP").is_ok() {
            eprintln!("slow pump {ms:.2} ms: {bytes} B, {} writes, {} jobs done, kit ready {}, queued {}", st.writes, st.jobs_done, self.kit_ready, self.uploads.pending());
        }
        self.stats.max_pump_ms = self.stats.max_pump_ms.max(ms);
        self.stats.max_pump_bytes = self.stats.max_pump_bytes.max(bytes);
        Ok(PumpFrame { bytes })
    }

    /// Models and kit maps resident.
    pub fn done(&self) -> bool {
        if self.model_gated {
            return self.kit_ready && self.models_wanted_resident();
        }
        self.kit_ready && self.stats.all_resident_at_ms.is_some()
    }

    /// no model requested or queued, nothing with the streamer.
    fn models_wanted_resident(&self) -> bool {
        self.model_state.iter().all(|s| !matches!(s, ModelState::Requested | ModelState::Queued)) && self.streamer.as_ref().is_none_or(|s| !s.busy())
    }

    /// request, keep or drop each Blender building's full model:
    /// wanted within MODEL_LOAD_M (kept to MODEL_DROP_M) of its bounds while in
    /// the widened view or within the hold. A queued upload no longer wanted is
    /// cancelled; a resident model is evicted (its ready word cleared and its
    /// objects emptied, so the footprint draws). Returns the bytes written.
    fn gate_models(&mut self, ctx: &GpuContext, renderer: &Renderer, world: &World, cam: Vec3) -> u64 {
        let Some(s) = self.streamer.as_mut() else { return 0 };
        let cone = renderer.view_cone();
        s.set_view(cone);
        let set = &world.authored;
        let load = MODEL_LOAD_M.min(self.model_radius);
        let (mut gone_queued, mut evict) = (std::collections::HashSet::new(), vec![]);
        self.residency.out_of_view = 0;
        for (i, b) in set.buildings.iter().enumerate() {
            let st = self.model_state[i];
            if st == ModelState::Failed {
                continue;
            }
            let g = &set.groups[b.group as usize];
            let c = Vec3::from(g.center);
            let d = (c - cam).length() - g.radius;
            let mut want = d <= load || (st != ModelState::Absent && d <= MODEL_DROP_M.min(self.model_radius.max(load)));
            // Within the ring every direction is wanted; past it, the view decides.
            if want && d > MODEL_RING_M.min(load) {
                if let Some(cn) = cone.as_ref() {
                    if !self.model_hold.wanted(cn, i, c - g.radius, c + g.radius) {
                        want = false;
                        self.residency.out_of_view += 1;
                    }
                }
            }
            match (want, st) {
                // A full model arena asks for nothing new until a model is evicted.
                (true, ModelState::Absent) if !self.model_wait_free => {
                    s.request(i);
                    self.model_state[i] = ModelState::Requested;
                }
                (false, ModelState::Requested) => {
                    s.cancel(i);
                    self.model_state[i] = ModelState::Absent;
                }
                (false, ModelState::Queued) => {
                    gone_queued.insert(i);
                }
                (false, ModelState::Resident) if evict.len() < MODEL_EVICTIONS_PER_FRAME => evict.push(i),
                _ => {}
            }
        }
        s.flush();
        if let Some(cn) = cone.as_ref() {
            self.model_hold.prune(cn);
        }
        let is_model = |tag: u64| tag & TAG_MODEL != 0 && tag & (eo_render::kit::TAG_KIT | eo_render::street::TAG_STREET | TAG_TEXTURES) == 0;
        if !gone_queued.is_empty() {
            for tag in self.uploads.cancel(|tag| !(is_model(tag) && gone_queued.contains(&((tag & 0xffff_ffff) as usize)))) {
                let i = (tag & 0xffff_ffff) as usize;
                self.models_in_queue -= 1;
                self.model_state[i] = ModelState::Absent;
                self.residency.cancelled += 1;
                self.stats.triangles_resident -= set.buildings[i].counts.indices as u64 / 3;
                self.free_model_ranges(set, i);
            }
        }
        let mut bytes = 0;
        for i in evict {
            let b = &set.buildings[i];
            renderer.write(&ctx.queue, Target::Group, b.group as u64 * std::mem::size_of::<GpuLodGroup>() as u64 + 16, bytemuck::bytes_of(&0u32));
            let mut ph = GpuObject::new(eo_render::scene::ObjectKind::Model, [0.0; 3], [0.0; 3], 0, 0, 0);
            ph.lod_group = b.group;
            ph.lod_role = eo_render::scene::LOD_NEAR;
            let objs = vec![ph; b.slot.object_count as usize];
            renderer.write(&ctx.queue, Target::Object, (self.bases.models.object + b.slot.object_offset) as u64 * std::mem::size_of::<GpuObject>() as u64, bytemuck::cast_slice(&objs));
            bytes += 4 + (objs.len() * std::mem::size_of::<GpuObject>()) as u64;
            self.model_state[i] = ModelState::Absent;
            self.stats.resident -= 1;
            self.stats.triangles_resident -= b.counts.indices as u64 / 3;
            let r = &mut self.residency;
            r.resident -= 1;
            r.evicted += 1;
            r.resident_vertex_bytes -= b.slot.vertex_count as u64 * std::mem::size_of::<Vertex>() as u64;
            r.resident_index_bytes -= b.slot.index_count as u64 * 4;
            r.vertices -= b.slot.vertex_count as u64;
            r.indices -= b.slot.index_count as u64;
            self.free_model_ranges(set, i);
        }
        bytes
    }

    /// give a model's arena ranges back (the ranges are reused only
    /// by later uploads, which the queue orders after the writes that emptied
    /// its objects); a full arena may now take the next model.
    fn free_model_ranges(&mut self, set: &eo_render::authored::AuthoredSet, i: usize) {
        if let (Some((va, ia)), Some((v, ix))) = (self.model_alloc.as_mut(), self.model_ranges[i].take()) {
            let b = &set.buildings[i];
            va.free(v, b.slot.vertex_count);
            ia.free(ix, b.slot.index_count);
            self.model_wait_free = false;
        }
    }

    /// the model arena's capacity and peak use (vertices, indices), when it is one.
    pub fn model_arena_use(&self) -> Option<[(u32, u32); 2]> {
        self.model_alloc.as_ref().map(|(v, i)| [(v.capacity, v.peak), (i.capacity, i.peak)])
    }

    /// which part of `settled` is not true (for a settle that timed out).
    pub fn unsettled(&self) -> String {
        format!(
            "kit maps ready {}, models done {}, uploads pending {}, kit settled {}, street settled {}, structures settled {}, far settled {}, far trees state {} (0 done), cc0 settled {}, {}",
            self.kit_ready,
            self.done(),
            self.uploads.pending(),
            self.kit.as_ref().map(|k| k.settled()).unwrap_or(true),
            self.street.as_ref().map(|s| s.settled()).unwrap_or(true),
            self.structures.as_ref().is_none_or(|s| s.settled()),
            self.hlod.as_ref().is_none_or(|h| h.settled()),
            self.hlod_trees,
            self.cc0.as_ref().map(|c| c.settled()).unwrap_or(true),
            self.hlod.as_ref().map(|h| h.unsettled()).unwrap_or_default() // hlod
        )
    }

    /// Models, maps and every kit pack wanted around the camera resident.
    pub fn settled(&self) -> bool {
        self.done() && self.uploads.pending() == 0 && self.kit.as_ref().map(|k| k.settled()).unwrap_or(true) && self.street.as_ref().map(|s| s.settled()).unwrap_or(true)
            && self.structures.as_ref().is_none_or(|s| s.settled()) // the grade-separated roads' near packs
            && self.hlod.as_ref().is_none_or(|h| h.settled()) // the far cells around the camera and every district
            && self.hlod_trees == 0 // and the distant trees
            && self.cc0.as_ref().map(|c| c.settled()).unwrap_or(true) // cc0
            && self.shop_rx.is_none() // (`--shop-light`) the shop windows' lights placed
    }
}
