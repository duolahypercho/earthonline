//! Fog lit by the local lights (`LightingOptions::lit_fog`: 1 coarse, 2
//! fine): a froxel volume filled between the sky's volume and its resolve
//! (the `mid` hook of `SkyGpu::encode_after`), composited by the resolve
//! (sky_resolve.wgsl `rs_lit_fog`, bindings 10 and 11).
//!
//! The volume is aligned with the light clusters' view frustum: the screen
//! split into `dims[0]` x `dims[1]` columns and the clusters' 0.5-300 m of
//! view depth into `dims[2]` log-spaced slices (`slice_depth`), so each
//! froxel lies in one light cluster and reads its list. Two compute passes
//! in one timed pass (`lighting::Q_LIT_FOG`), lit_fog.wgsl:
//!
//! 1. `lf_inject`: per froxel, the fog's extinction from the volume's own fog
//!    (the marine layer, the weather's veil, a continuous low deck, the haze,
//!    rain: `vol_main`'s densities at the froxel), the light each clustered
//!    lamp and headlamp sends toward the eye from it through the droplets'
//!    forward lobe (eo-sky `lit_fog`: Henyey-Greenstein g 0.9, Narasimhan and
//!    Nayar 2003, its spread with the optical depth), at a depth jittered
//!    through the slice; blended with the last frame's froxel at the same
//!    world point with a weight of `NEW_WEIGHT` for the new frame (the
//!    volume's is 0.12), more as the eye moves and the point's direction to it
//!    turns (`TURN_HALF_RAD`), and the history held within `CLAMP` of the new
//!    value, so a moving headlamp's beam does not smear.
//! 2. `lf_integrate`: per column, front to back, the in-scatter and the
//!    transmittance to each slice's far face.
//!
//! The sun is not here: the volume carries it (its shafts, the marine
//! layer's key). The resolve adds the integrated light at each pixel's view
//! depth on top of the volume's result, which already applies this fog's
//! extinction to the scene. Transparent glass is drawn before the volume, so
//! the lit fog is added in front of it as the volume is.
//!
//! Light shadows (lighting, `--light-shadows`, `--headlight-shadows`;
//! light_shadow.rs): `lf_light_visibility` in lit_fog.wgsl multiplies a light
//! with a shadow map (its record's spare word, the tile + 1) by its
//! visibility from the froxel's point, read from the look's light shadow atlas
//! and table (group 1 bindings 17 and 18, bound here at 14 and 15 with the
//! look's comparison sampler at 16; their stand-ins while the option is off,
//! when no record names a tile). A lamp behind a building lights no fog in
//! front of it; shafts of lit fog stand between a tree's shadows.
//!
//! By day (no local light lit), with the sky chain off, or with the option
//! off, nothing is drawn and the slot is an empty span; the resolve reads its
//! stand-ins and adds nothing. Resources are made on first use at a level.

use crate::lighting::{self, Ts};
use crate::look::LookGpu;
use std::sync::Mutex;

/// Froxels (columns across, rows down, slices) at levels 1 (coarse) and 2 (fine).
pub const LEVELS: [[u32; 3]; 2] = [[160, 90, 64], [240, 135, 96]];
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The new frame's weight in the temporal blend (the volume's is 0.12): a
/// headlamp's beam moving across the froxels lags by (1 - w) / w ~ 2 frames.
pub const NEW_WEIGHT: f32 = 0.35;
/// The history is held within this factor of the new frame's light (a froxel
/// a beam has left keeps nothing of it; the depth jitter's spread averages).
pub const CLAMP: f32 = 2.0;
/// The turn (rad) of a froxel's direction to the eye since the last frame at
/// which the new frame takes half the blend: the droplets' forward lobe is
/// about (1 - g) = 0.1 rad wide and falls by tens of percent over a few
/// hundredths of it, so a history kept over frames of a moving eye leaves the
/// glow leaning toward where the eye was (measured: an eye walking past a lamp
/// at 6 m/s, the glow's side toward its old place 25 % too bright). INFERRED,
/// tuned with the test `a_moving_camera_keeps_the_glow_where_it_stands`.
pub const TURN_HALF_RAD: f32 = 0.0025;
/// The history's floor over a dark froxel (radiance units): below the
/// display's least step at night exposure.
pub const CLAMP_FLOOR: f32 = 1e-6;
/// The nearest a light is held to (m): a luminaire's head is not a point
/// (the sprites' lens is 0.16 m across; sky_fx.wgsl `FX_LENS_M`).
pub const MIN_DIST_M: f32 = 0.25;
/// Rain's cells at their mean: sky_volume.wgsl (E)'s 0.1 + 2.1 cell^2 over
/// the sheet's cells (cell^2 ~ 0.24 on average).
pub const RAIN_MEAN: f32 = 0.6;
/// sky_clouds.wgsl's `CL_DENSITY` and `CL_BASE_TILE` (the deck's density
/// scale and its base noise's tile, m), and sky_lights.wgsl's
/// `LT_JS_SOURCE_LIFT_MH`; checked equal by a unit test.
const CL_DENSITY: f32 = 2.6;
const CL_BASE_TILE: f32 = 1600.0;
const JS_SOURCE_LIFT_MH: f32 = 0.57;
/// The state word before the first integration: no frame matches it.
const STATE_UNSET: u32 = 0xffff_fff0;

/// Froxels at a level (1 or 2), None for 0.
pub fn dims(level: u8) -> Option<[u32; 3]> {
    match level {
        0 => None,
        1 => Some(LEVELS[0]),
        _ => Some(LEVELS[1]),
    }
}

/// GPU bytes at a level: two froxel volumes (this frame's and the history)
/// and the integrated one, Rgba16Float, and the state word.
pub fn bytes_for(level: u8) -> u64 {
    dims(level).map(|d| d[0] as u64 * d[1] as u64 * d[2] as u64 * 8 * 3 + 16).unwrap_or(0)
}

/// The view depth (m) of slice coordinate `s` of `slices`: log-spaced from
/// the clusters' near to their far (lit_fog.wgsl `lf_depth`).
pub fn slice_depth(s: f32, slices: u32) -> f32 {
    let (n, f) = (crate::sky::CLUSTER_NEAR_M, crate::sky::CLUSTER_FAR_M);
    n * (f / n).powf(s / slices as f32)
}

/// The slice coordinate (0..1 of the volume) of a view depth (the resolve's lookup).
pub fn depth_slice(vz: f32) -> f32 {
    let (n, f) = (crate::sky::CLUSTER_NEAR_M, crate::sky::CLUSTER_FAR_M);
    ((vz.max(n) / n).ln() / (f / n).ln()).clamp(0.0, 1.0)
}

/// The constants lit_fog.wgsl is compiled with.
pub fn wgsl() -> String {
    format!(
        "{}\n// generated by lit_fog.rs\nconst LF_NEW_WEIGHT: f32 = {:.6};\nconst LF_CLAMP: f32 = {:.6};\nconst LF_CLAMP_FLOOR: f32 = {:e};\nconst LF_TURN_HALF_RAD: f32 = {:.6};\nconst LF_MIN_DIST_M: f32 = {:.6};\nconst LF_RAIN_MEAN: f32 = {:.6};\nconst LF_CL_DENSITY: f32 = {:.6};\nconst LF_CL_BASE_TILE: f32 = {:.6};\nconst LF_JS_SOURCE_LIFT_MH: f32 = {:.6};\nconst LF_CLUSTER_CAP: u32 = {}u;\n",
        eo_sky::lit_fog::wgsl(),
        NEW_WEIGHT,
        CLAMP,
        CLAMP_FLOOR,
        TURN_HALF_RAD,
        MIN_DIST_M,
        RAIN_MEAN,
        CL_DENSITY,
        CL_BASE_TILE,
        JS_SOURCE_LIFT_MH,
        crate::sky::CLUSTER_CAP
    )
}

/// The passes' WGSL (look_common, the constants, lit_fog.wgsl, and the
/// light shadows' lookup with the names it reads on this group).
pub fn shader_source() -> String {
    format!(
        "{}\n{}\n{}\n// (lighting) the light shadow atlas, its table and the comparison sampler (light_shadow_use.wgsl)\n@group(0) @binding(14) var lx_light_shadow_atlas: texture_depth_2d_array;\n@group(0) @binding(15) var<storage, read> lx_light_shadow_table: array<vec4<f32>>;\n@group(0) @binding(16) var lk_shadow_cmp: sampler_comparison;\n{}",
        crate::look::look_common_source(),
        wgsl(),
        include_str!("lit_fog.wgsl"),
        crate::light_shadow::use_wgsl(true)
    )
}

/// What the lit fog did, for the logs and the bench.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LitFogStats {
    /// The level the volume is made at (0: none).
    pub level: u8,
    pub dims: [u32; 3],
    pub bytes: u64,
    /// Frames the passes ran, and frames they were skipped with the option on
    /// (no local light lit, or the sky chain off).
    pub frames: u64,
    pub skipped: u64,
}

struct Fog {
    level: u8,
    dims: [u32; 3],
    inject_pipe: wgpu::ComputePipeline,
    integrate_pipe: wgpu::ComputePipeline,
    /// [p]: inject writes froxels p and reads p ^ 1 (the history); integrate reads p.
    inject_bg: [wgpu::BindGroup; 2],
    integrate_bg: [wgpu::BindGroup; 2],
    integ_view: wgpu::TextureView,
    state: wgpu::Buffer,
    parity: usize,
    integ: wgpu::Texture,
    _textures: Vec<wgpu::Texture>,
    /// (lighting) what the inject groups were made with: the froxels, the
    /// layout, and the light shadow atlas and table bound (made again when the
    /// look binds others).
    froxel_views: [wgpu::TextureView; 2],
    inject_bgl: wgpu::BindGroupLayout,
    shadows: (wgpu::TextureView, wgpu::Buffer),
}

impl Fog {
    fn new(device: &wgpu::Device, look: &LookGpu, level: u8) -> Self {
        let d = device;
        let dims = dims(level).expect("a level");
        let sky = &look.sky;
        let tex3 = |label: &str, copy: bool| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: dims[0], height: dims[1], depth_or_array_layers: dims[2] },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | if copy { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() },
                view_formats: &[],
            })
        };
        let froxels = [tex3("lit-fog-froxels-0", false), tex3("lit-fog-froxels-1", false)];
        // (copied out by `integrated_bytes`, tests and diagnostics)
        let integ = tex3("lit-fog-integrated", true);
        let fv: [wgpu::TextureView; 2] = std::array::from_fn(|i| froxels[i].create_view(&Default::default()));
        let integ_view = integ.create_view(&Default::default());
        let state = d.create_buffer(&wgpu::BufferDescriptor { label: Some("lit-fog-state"), size: 16, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: true });
        if let Ok(mut m) = state.slice(..).get_mapped_range_mut() {
            m.copy_from_slice(bytemuck::cast_slice(&[STATE_UNSET, 0, 0, 0]));
        }
        state.unmap();
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let sbuf = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
        let tex = |dim, filterable| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: dim, multisampled: false };
        let samp = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let store = wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: FORMAT, view_dimension: wgpu::TextureViewDimension::D3 };
        let (d2, d3) = (wgpu::TextureViewDimension::D2, wgpu::TextureViewDimension::D3);
        let inject_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lit-fog-inject-bgl"),
            entries: &[
                entry(0, ubuf),
                entry(1, ubuf),
                entry(2, sbuf(true)),
                entry(3, sbuf(true)),
                entry(4, tex(d2, true)),
                entry(5, tex(d3, true)),
                entry(6, tex(d3, true)),
                entry(7, samp),
                entry(8, tex(d3, true)),
                entry(9, samp),
                entry(10, store),
                entry(11, sbuf(false)),
                // (lighting) the light shadow atlas, its table, the comparison sampler
                entry(14, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
                entry(15, sbuf(true)),
                entry(16, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
            ],
        });
        let integrate_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lit-fog-integrate-bgl"),
            entries: &[entry(1, ubuf), entry(11, sbuf(false)), entry(12, tex(d3, false)), entry(13, store)],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("lit_fog.wgsl"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let pipe = |label: &str, bgl: &wgpu::BindGroupLayout, entry_point: &str| {
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(&layout), module: &module, entry_point: Some(entry_point), compilation_options: Default::default(), cache: None })
        };
        let inject_pipe = pipe("lit-fog-inject", &inject_bgl, "lf_inject");
        let integrate_pipe = pipe("lit-fog-integrate", &integrate_bgl, "lf_integrate");
        let res = |binding, r| wgpu::BindGroupEntry { binding, resource: r };
        let lb = look.lighting_bindings();
        let shadows = (lb.light_shadow_atlas.clone(), lb.light_shadow_table.clone());
        let inject_bg = inject_groups(d, look, &inject_bgl, &fv, &state, &shadows);
        let integrate_bg = std::array::from_fn(|p: usize| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lit-fog-integrate-bg"),
                layout: &integrate_bgl,
                entries: &[
                    res(1, sky.uniform.as_entire_binding()),
                    res(11, state.as_entire_binding()),
                    res(12, wgpu::BindingResource::TextureView(&fv[p])),
                    res(13, wgpu::BindingResource::TextureView(&integ_view)),
                ],
            })
        });
        crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 4);
        let [a, b] = froxels;
        Self { level, dims, inject_pipe, integrate_pipe, inject_bg, integrate_bg, integ_view, state, parity: 0, integ, _textures: vec![a, b], froxel_views: fv, inject_bgl, shadows }
    }
}

/// The inject pass's groups by parity: [p] writes froxels p and reads p ^ 1
/// (the history); `shadows` the light shadow atlas and table the look binds.
fn inject_groups(d: &wgpu::Device, look: &LookGpu, layout: &wgpu::BindGroupLayout, fv: &[wgpu::TextureView; 2], state: &wgpu::Buffer, shadows: &(wgpu::TextureView, wgpu::Buffer)) -> [wgpu::BindGroup; 2] {
    let sky = &look.sky;
    let res = |binding, r| wgpu::BindGroupEntry { binding, resource: r };
    std::array::from_fn(|p: usize| {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-fog-inject-bg"),
            layout,
            entries: &[
                res(0, look.look_buffer().as_entire_binding()),
                res(1, sky.uniform.as_entire_binding()),
                res(2, sky.frame_lights.as_entire_binding()),
                res(3, sky.clusters.as_entire_binding()),
                res(4, wgpu::BindingResource::TextureView(&sky.sheet_view)),
                res(5, wgpu::BindingResource::TextureView(&sky.noise_view)),
                res(6, wgpu::BindingResource::TextureView(&sky.clouds.base_view)),
                res(7, wgpu::BindingResource::Sampler(&sky.rep)),
                res(8, wgpu::BindingResource::TextureView(&fv[p ^ 1])),
                res(9, wgpu::BindingResource::Sampler(sky.linear_sampler())),
                res(10, wgpu::BindingResource::TextureView(&fv[p])),
                res(11, state.as_entire_binding()),
                res(14, wgpu::BindingResource::TextureView(&shadows.0)),
                res(15, shadows.1.as_entire_binding()),
                res(16, wgpu::BindingResource::Sampler(&look.cmp_sampler)),
            ],
        })
    })
}

pub struct LitFogGpu {
    device: wgpu::Device,
    fog: Mutex<Option<Fog>>,
    stats: Mutex<LitFogStats>,
}

impl LitFogGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        Self { device: device.clone(), fog: Mutex::new(None), stats: Mutex::new(LitFogStats::default()) }
    }

    /// GPU bytes held (0 until the option is first used at night).
    pub fn bytes(&self) -> u64 {
        self.stats.lock().unwrap().bytes
    }

    pub fn stats(&self) -> LitFogStats {
        *self.stats.lock().unwrap()
    }

    /// The integrated volume as last drawn (Rgba16Float texels, x fastest, then
    /// y, then the slices), None before it exists. Waits for the GPU (tests and
    /// diagnostics only).
    pub fn integrated_bytes(&self, ctx: &crate::GpuContext) -> Option<Vec<u8>> {
        let g = self.fog.lock().unwrap();
        let f = g.as_ref()?;
        let [w, h, n] = f.dims;
        let row = (w * 8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: Some("lit-fog-read"), size: row as u64 * h as u64 * n as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            f.integ.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: n },
        );
        ctx.queue.submit([enc.finish()]);
        let raw = crate::read_mapped(ctx, &buf).ok()?;
        Some(raw.chunks(row as usize).flat_map(|r| r[..(w * 8) as usize].to_vec()).collect())
    }

    /// Between the sky-volume and sky-resolve passes (called through
    /// `LookGpu::encode_sky_after_with`; see `SkyGpu::encode_after` for what
    /// it must not call). `chain`: whether the volume and the resolve run
    /// this frame (false: the sky chain is off or the depth is not usable,
    /// nothing would composite it). Timed in `lighting::Q_LIT_FOG`.
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, look: &LookGpu, _depth: &wgpu::TextureView, chain: bool, ts: Ts<'_>) {
        let level = look.options.lighting.lit_fog;
        let sky = &look.sky;
        if level == 0 {
            // the option turned off: the volume is let go and the resolve takes its stand-ins again.
            if self.fog.lock().unwrap().take().is_some() {
                sky.set_lit_fog_volume(None);
                *self.stats.lock().unwrap() = LitFogStats::default();
            }
            lighting::empty_span(enc, ts, lighting::Q_LIT_FOG, "lit fog (off)");
            return;
        }
        if !chain || !sky.local_lights_on() {
            // nothing to scatter (by day) or nothing to composite it into: the resolve's state check
            // fails this frame, and the next frame that runs starts without history.
            self.stats.lock().unwrap().skipped += 1;
            lighting::empty_span(enc, ts, lighting::Q_LIT_FOG, "lit fog (no lights)");
            return;
        }
        let mut g = self.fog.lock().unwrap();
        if g.as_ref().is_none_or(|f| f.level != level.min(2)) {
            let f = Fog::new(&self.device, look, level.min(2));
            sky.set_lit_fog_volume(Some((&f.integ_view, &f.state)));
            let bytes = bytes_for(f.level);
            eprintln!("lit fog: level {}, {} x {} x {} froxels over {:.1}-{:.0} m, {:.1} MB", f.level, f.dims[0], f.dims[1], f.dims[2], crate::sky::CLUSTER_NEAR_M, crate::sky::CLUSTER_FAR_M, bytes as f64 / 1e6);
            let mut s = self.stats.lock().unwrap();
            (s.level, s.dims, s.bytes) = (f.level, f.dims, bytes);
            *g = Some(f);
        }
        let f = g.as_mut().unwrap();
        // (lighting) the light shadow atlas the look binds now (made, replaced or let go since).
        let lb = look.lighting_bindings();
        if f.shadows.0 != lb.light_shadow_atlas || f.shadows.1 != lb.light_shadow_table {
            f.shadows = (lb.light_shadow_atlas.clone(), lb.light_shadow_table.clone());
            f.inject_bg = inject_groups(&self.device, look, &f.inject_bgl, &f.froxel_views, &f.state, &f.shadows);
            crate::hitch::mark(crate::hitch::Mark::BindGroupCreated, 2);
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("lit fog"), timestamp_writes: lighting::compute_writes(ts, lighting::Q_LIT_FOG) });
            pass.set_pipeline(&f.inject_pipe);
            pass.set_bind_group(0, &f.inject_bg[f.parity], &[]);
            pass.dispatch_workgroups(f.dims[0].div_ceil(8), f.dims[1].div_ceil(8), f.dims[2]);
            pass.set_pipeline(&f.integrate_pipe);
            pass.set_bind_group(0, &f.integrate_bg[f.parity], &[]);
            pass.dispatch_workgroups(f.dims[0].div_ceil(8), f.dims[1].div_ceil(8), 1);
        }
        f.parity ^= 1;
        self.stats.lock().unwrap().frames += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    #[test]
    fn the_levels_and_their_bytes() {
        assert_eq!(dims(0), None);
        assert_eq!(dims(1), Some([160, 90, 64]));
        assert_eq!(dims(2), Some([240, 135, 96]));
        assert_eq!(bytes_for(0), 0);
        // three Rgba16Float volumes and the state word: 22.1 MB coarse, 74.6 MB fine.
        assert_eq!(bytes_for(1), 160 * 90 * 64 * 24 + 16);
        assert_eq!(bytes_for(2), 240 * 135 * 96 * 24 + 16);
        // the froxel columns split the light clusters' 16 x 9 tiles evenly.
        for l in LEVELS {
            assert_eq!(l[0] % crate::sky::CLUSTERS[0], 0);
            assert_eq!(l[1] % crate::sky::CLUSTERS[1], 0);
        }
    }

    #[test]
    fn the_slices_span_the_clusters_depth() {
        for l in LEVELS {
            let n = l[2];
            assert!((slice_depth(0.0, n) - crate::sky::CLUSTER_NEAR_M).abs() < 1e-5);
            assert!((slice_depth(n as f32, n) - crate::sky::CLUSTER_FAR_M).abs() < 1e-2);
            // log-spaced: every slice the same ratio deep (10 % at the coarse level).
            let r = slice_depth(1.0, n) / slice_depth(0.0, n);
            for k in 1..n {
                let rk = slice_depth(k as f32 + 1.0, n) / slice_depth(k as f32, n);
                assert!((rk - r).abs() < 1e-4, "slice {k}: {rk} vs {r}");
            }
            // the resolve's lookup inverts it.
            for k in 0..=n {
                let z = slice_depth(k as f32, n);
                assert!((depth_slice(z) - k as f32 / n as f32).abs() < 1e-5);
            }
        }
        assert!((slice_depth(1.0, 64) / slice_depth(0.0, 64) - 1.105).abs() < 0.002);
    }

    #[test]
    fn the_copied_constants_match_their_sources() {
        let clouds = include_str!("sky_clouds.wgsl");
        assert!(clouds.contains(&format!("const CL_DENSITY: f32 = {CL_DENSITY:?};")), "sky_clouds.wgsl CL_DENSITY moved");
        assert!(clouds.contains(&format!("const CL_BASE_TILE: f32 = {CL_BASE_TILE:?};")), "sky_clouds.wgsl CL_BASE_TILE moved");
        assert!(include_str!("sky_lights.wgsl").contains(&format!("const LT_JS_SOURCE_LIFT_MH: f32 = {JS_SOURCE_LIFT_MH:?};")));
        assert!(include_str!("sky_fx.wgsl").contains(&format!("const FX_LENS_M: f32 = {:?};", 0.16f32)));
    }

    #[test]
    fn the_passes_parse_and_validate() {
        let src = shader_source();
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("lit_fog.wgsl: {}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("lit_fog.wgsl validates");
        for e in ["lf_inject", "lf_integrate"] {
            assert!(module.entry_points.iter().any(|p| p.name == e), "{e}");
        }
    }
}
