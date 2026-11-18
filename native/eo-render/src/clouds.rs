//! the cloud noise volumes, the blue-noise tile and the cloud-shadow map.
//!
//! The sky slice's volume pass marches the low cloud deck with the density and
//! light of `sky_clouds.wgsl` (appended to its module). This module owns what
//! that needs beyond the coverage sheet:
//!
//! * **Noise** (`sky_cloud_noise.wgsl`, baked once on the GPU before the first
//!   frame's city pass): a 128^3 RGBA8 base-shape volume (Perlin-Worley and
//!   three Worley fBm octaves, 3.2 km tile), a 32^3 RGBA8 Worley detail volume
//!   (267 m tile), both with box-filtered mips, and a 128^2 curl field (1.6 km
//!   tile). Everything tiles; nothing is stored art.
//! * **Blue noise** (64^2, void-and-cluster, Ulichney 1993, made on the CPU at
//!   start-up in ~30 ms): the march's start offset per pixel, moved along the
//!   golden ratio per trace, so neighbouring pixels never share an offset and a
//!   pixel's own traces cover the step evenly.
//! * **The cloud-shadow map** (384^2 RGBA16F, 83 m texels, a 32 km window
//!   about the eye, wrapped so a texel stays on its world point): the deck's
//!   transmittance along the key through the same shape the march draws, made
//!   each frame before the city pass. The city, the water, the surf, the far
//!   terrain (`sky_cloud_transmittance`) and the volume's fog and shafts
//!   (`vol_cloud_t`) read it; it takes the look's group-1 binding 7, where the
//!   sheet was bound for the old sheet-sampled shadow.
//!
//! Portable: compute and sampled 3D textures only; rgba8unorm and rgba16float
//! storage (core WebGPU), no atomics.

use std::sync::atomic::{AtomicBool, Ordering};

pub const BASE_RES: u32 = 128;
pub const BASE_MIPS: u32 = 4;
pub const DETAIL_RES: u32 = 32;
pub const DETAIL_MIPS: u32 = 3;
pub const CURL_RES: u32 = 128;
pub const BLUE_RES: usize = 64;
/// sky_common.wgsl `SKY_CSHADOW_RES`.
pub const SHADOW_RES: u32 = 384;

/// The baked noise volumes and their one-time bake (GPU).
pub struct CloudNoise {
    pub base: wgpu::Texture,
    pub detail: wgpu::Texture,
    pub curl: wgpu::Texture,
    /// (pipeline, bind group, workgroups): the one-time bake, in order.
    bake: Vec<(wgpu::ComputePipeline, wgpu::BindGroup, [u32; 3])>,
}

pub struct CloudGpu {
    noise: CloudNoise,
    pub(crate) base_view: wgpu::TextureView,
    pub(crate) detail_view: wgpu::TextureView,
    pub(crate) curl_view: wgpu::TextureView,
    pub(crate) blue_view: wgpu::TextureView,
    /// The cloud-shadow map (the look's group-1 binding 7, the volume's 18).
    pub(crate) shadow_view: wgpu::TextureView,
    blue: wgpu::Texture,
    blue_ready: AtomicBool,
    baked: AtomicBool,
    shadow_pipe: wgpu::ComputePipeline,
    shadow_bg: wgpu::BindGroup,
    pub bytes: u64,
}

fn tex3(d: &wgpu::Device, label: &str, res: u32, mips: u32) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: res },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn tex2(d: &wgpu::Device, label: &str, res: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// The inputs of the cloud-shadow pass (bindings of the volume module it shares).
pub struct ShadowInputs<'a> {
    /// The volume module (sky_volume.wgsl + sky_clouds.wgsl), entry `cl_shadow_main`.
    pub module: &'a wgpu::ShaderModule,
    pub look_buf: &'a wgpu::Buffer,
    pub sky_uniform: &'a wgpu::Buffer,
    pub sheet: &'a wgpu::TextureView,
    pub repeat: &'a wgpu::Sampler,
    /// The atmosphere's sky-view table and a clamped linear sampler: the pass also makes
    /// the clouds' sky light once a frame.
    pub sky_view: &'a wgpu::TextureView,
    pub linear: &'a wgpu::Sampler,
}

impl CloudNoise {
    pub fn new(d: &wgpu::Device) -> Self {
        let base = tex3(d, "clouds-base-noise", BASE_RES, BASE_MIPS);
        let detail = tex3(d, "clouds-detail-noise", DETAIL_RES, DETAIL_MIPS);
        let su = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC;
        let curl = tex2(d, "clouds-curl", CURL_RES, wgpu::TextureFormat::Rgba8Unorm, su);
        let mip = |t: &wgpu::Texture, m: u32| t.create_view(&wgpu::TextureViewDescriptor { base_mip_level: m, mip_level_count: Some(1), ..Default::default() });
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let st3 = wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba8Unorm, view_dimension: wgpu::TextureViewDimension::D3 };
        let st2 = |format| wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 };
        let t3 = |filterable| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false };
        let layout = |label: &str, bgl: &wgpu::BindGroupLayout| d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
        let compute = |label: &str, l: &wgpu::PipelineLayout, m: &wgpu::ShaderModule, e: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(l), module: m, entry_point: Some(e), compilation_options: Default::default(), cache: None })
        };

        // The bake.
        let nm = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sky_cloud_noise.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("sky_cloud_noise.wgsl").into()) });
        let gen_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("clouds-gen-bgl"), entries: &[entry(0, st3)] });
        let down_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("clouds-down-bgl"), entries: &[entry(0, st3), entry(1, t3(false))] });
        let curl_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("clouds-curl-bgl"), entries: &[entry(2, st2(wgpu::TextureFormat::Rgba8Unorm))] });
        let gen_l = layout("clouds-gen-layout", &gen_bgl);
        let base_pipe = compute("clouds-base", &gen_l, &nm, "nz_base_main");
        let detail_pipe = compute("clouds-detail", &gen_l, &nm, "nz_detail_main");
        let down_pipe = compute("clouds-down", &layout("clouds-down-layout", &down_bgl), &nm, "nz_down_main");
        let curl_pipe = compute("clouds-curl", &layout("clouds-curl-layout", &curl_bgl), &nm, "nz_curl_main");
        let bg = |l: &wgpu::BindGroupLayout, e: &[(u32, &wgpu::TextureView)]| {
            let entries: Vec<wgpu::BindGroupEntry> = e.iter().map(|(b, v)| wgpu::BindGroupEntry { binding: *b, resource: wgpu::BindingResource::TextureView(v) }).collect();
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("clouds-bake-bg"), layout: l, entries: &entries })
        };
        let wg = |r: u32| [r.div_ceil(4); 3];
        let mut bake = vec![
            (base_pipe, bg(&gen_bgl, &[(0, &mip(&base, 0))]), wg(BASE_RES)),
            (detail_pipe, bg(&gen_bgl, &[(0, &mip(&detail, 0))]), wg(DETAIL_RES)),
        ];
        for (t, res, mips) in [(&base, BASE_RES, BASE_MIPS), (&detail, DETAIL_RES, DETAIL_MIPS)] {
            for m in 1..mips {
                bake.push((down_pipe.clone(), bg(&down_bgl, &[(0, &mip(t, m)), (1, &mip(t, m - 1))]), wg(res >> m)));
            }
        }
        bake.push((curl_pipe, bg(&curl_bgl, &[(2, &curl.create_view(&Default::default()))]), [CURL_RES.div_ceil(8), CURL_RES.div_ceil(8), 1]));
        Self { base, detail, curl, bake }
    }

    /// The bake's dispatches (once, before anything samples the noise).
    pub fn encode(&self, pass: &mut wgpu::ComputePass) {
        for (p, bg, w) in &self.bake {
            pass.set_pipeline(p);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(w[0], w[1], w[2]);
        }
    }
}

impl CloudGpu {
    pub fn new(d: &wgpu::Device, inp: ShadowInputs) -> Self {
        let noise = CloudNoise::new(d);
        let su = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING;
        let blue = tex2(d, "clouds-blue-noise", BLUE_RES as u32, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let shadow = tex2(d, "clouds-shadow-map", SHADOW_RES, wgpu::TextureFormat::Rgba16Float, su);
        let cs = wgpu::ShaderStages::COMPUTE;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: cs, ty, count: None };
        let st2 = |format| wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 };
        let t3 = |filterable| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false };
        let t2 = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let layout = |label: &str, bgl: &wgpu::BindGroupLayout| d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
        let compute = |label: &str, l: &wgpu::PipelineLayout, m: &wgpu::ShaderModule, e: &str| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(l), module: m, entry_point: Some(e), compilation_options: Default::default(), cache: None })
        };

        // The cloud-shadow map: the volume module's `cl_shadow_main` and the bindings it uses.
        let samp = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let shadow_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clouds-shadow-bgl"),
            entries: &[entry(0, ubuf), entry(1, ubuf), entry(3, t2), entry(5, samp), entry(12, samp), entry(14, t2), entry(15, t3(true)), entry(17, t2), entry(21, st2(wgpu::TextureFormat::Rgba16Float))],
        });
        let shadow_pipe = compute("clouds-shadow", &layout("clouds-shadow-layout", &shadow_bgl), inp.module, "cl_shadow_main");
        let base_view = noise.base.create_view(&Default::default());
        let curl_view = noise.curl.create_view(&Default::default());
        let shadow_view = shadow.create_view(&Default::default());
        let shadow_bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clouds-shadow-bg"),
            layout: &shadow_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: inp.look_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: inp.sky_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(inp.sheet) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(inp.repeat) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(inp.linear) },
                wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(inp.sky_view) },
                wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&base_view) },
                wgpu::BindGroupEntry { binding: 17, resource: wgpu::BindingResource::TextureView(&curl_view) },
                wgpu::BindGroupEntry { binding: 21, resource: wgpu::BindingResource::TextureView(&shadow_view) },
            ],
        });
        let bytes = (BASE_RES as u64).pow(3) * 4 * 8 / 7 + (DETAIL_RES as u64).pow(3) * 4 * 8 / 7 + (CURL_RES as u64).pow(2) * 4 + (BLUE_RES as u64).pow(2) * 4 + (SHADOW_RES as u64).pow(2) * 8;
        Self {
            detail_view: noise.detail.create_view(&Default::default()),
            noise,
            base_view,
            curl_view,
            blue_view: blue.create_view(&Default::default()),
            shadow_view,
            blue,
            blue_ready: AtomicBool::new(false),
            baked: AtomicBool::new(false),
            shadow_pipe,
            shadow_bg,
            bytes,
        }
    }

    /// The blue-noise tile, once (with the frame's first uniform upload).
    pub(crate) fn upload_once(&self, queue: &wgpu::Queue) {
        if self.blue_ready.swap(true, Ordering::Relaxed) {
            return;
        }
        let n = BLUE_RES;
        let r = blue_noise(n, 0xb10e_5eed);
        // R the tile; G, B, A the same tile shifted (spare decorrelated channels).
        let at = |x: usize, y: usize| r[(y % n) * n + (x % n)];
        let mut px = vec![0u8; n * n * 4];
        for y in 0..n {
            for x in 0..n {
                let o = (y * n + x) * 4;
                px[o] = at(x, y);
                px[o + 1] = at(x + n / 2, y + n / 3);
                px[o + 2] = at(x + n / 5, y + n / 2);
                px[o + 3] = at(x + 3 * n / 4, y + n / 7);
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.blue, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n as u32 * 4), rows_per_image: Some(n as u32) },
            wgpu::Extent3d { width: n as u32, height: n as u32, depth_or_array_layers: 1 },
        );
    }

    /// Before the city pass: the one-time noise bake, then this frame's cloud-shadow map.
    /// `ts`: the bench's query set and this pass's first slot (sky.rs `Q_CLOUD_SHADOW`).
    pub(crate) fn encode(&self, enc: &mut wgpu::CommandEncoder, ts: Option<(&wgpu::QuerySet, u32)>) {
        let tw = ts.map(|(q, b)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(b), end_of_pass_write_index: Some(b + 1) });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("clouds-shadow"), timestamp_writes: tw });
        if !self.baked.swap(true, Ordering::Relaxed) {
            self.noise.encode(&mut pass);
        }
        pass.set_pipeline(&self.shadow_pipe);
        pass.set_bind_group(0, &self.shadow_bg, &[]);
        pass.dispatch_workgroups(SHADOW_RES.div_ceil(8), SHADOW_RES.div_ceil(8), 1);
    }
}

/// A tiling blue-noise threshold map, `n` x `n`, by void and cluster (Ulichney 1993):
/// every value 0..255 equally often, each rank placed in the largest void of the ones
/// before it, so no two near pixels take near values.
pub fn blue_noise(n: usize, seed: u32) -> Vec<u8> {
    let size = n * n;
    let sigma = 1.5f32;
    let mut kern = vec![0f32; size];
    for dy in 0..n {
        for dx in 0..n {
            let (x, y) = (dx.min(n - dx) as f32, dy.min(n - dy) as f32);
            kern[dy * n + dx] = (-(x * x + y * y) / (2.0 * sigma * sigma)).exp();
        }
    }
    let splat = |e: &mut [f32], q: usize, s: f32| {
        let (qx, qy) = (q % n, q / n);
        for y in 0..n {
            let ky = (y + n - qy) % n;
            for x in 0..n {
                e[y * n + x] += s * kern[ky * n + (x + n - qx) % n];
            }
        }
    };
    let argmax = |e: &[f32], on: &[bool], want: bool| -> usize {
        let mut best = usize::MAX;
        for i in 0..e.len() {
            if on[i] == want && (best == usize::MAX || e[i] > e[best]) {
                best = i;
            }
        }
        best
    };
    let argmin = |e: &[f32], on: &[bool], want: bool| -> usize {
        let mut best = usize::MAX;
        for i in 0..e.len() {
            if on[i] == want && (best == usize::MAX || e[i] < e[best]) {
                best = i;
            }
        }
        best
    };
    // An initial pattern: a tenth of the pixels, then relaxed (the tightest cluster
    // moved to the largest void until they coincide).
    let mut on = vec![false; size];
    let mut e = vec![0f32; size];
    let ones = size / 10;
    let mut h = seed.max(1);
    let mut placed = 0;
    while placed < ones {
        h ^= h << 13;
        h ^= h >> 17;
        h ^= h << 5;
        let i = h as usize % size;
        if !on[i] {
            on[i] = true;
            splat(&mut e, i, 1.0);
            placed += 1;
        }
    }
    for _ in 0..size * 2 {
        let c = argmax(&e, &on, true);
        on[c] = false;
        splat(&mut e, c, -1.0);
        let v = argmin(&e, &on, false);
        on[v] = true;
        splat(&mut e, v, 1.0);
        if v == c {
            break;
        }
    }
    let mut rank = vec![0usize; size];
    // Ranks below the initial pattern: its tightest clusters removed in turn.
    let (mut on1, mut e1) = (on.clone(), e.clone());
    for r in (0..ones).rev() {
        let c = argmax(&e1, &on1, true);
        on1[c] = false;
        splat(&mut e1, c, -1.0);
        rank[c] = r;
    }
    // Ranks above it: the largest void filled in turn.
    for r in ones..size {
        let v = argmin(&e, &on, false);
        on[v] = true;
        splat(&mut e, v, 1.0);
        rank[v] = r;
    }
    rank.iter().map(|&r| ((r * 256) / size) as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blue_noise_is_a_permutation_and_has_no_clumps() {
        let n = 32;
        let b = blue_noise(n, 7);
        // every threshold equally often
        let mut hist = [0usize; 256];
        for v in &b {
            hist[*v as usize] += 1;
        }
        let per = n * n / 256;
        assert!(hist.iter().all(|c| *c == per), "uneven histogram");
        // blue: a pixel and its 4 neighbours differ more than random values would
        // (mean |difference| of uniform values is 1/3 of the range; blue noise ~0.4+)
        let mut d = 0.0;
        for y in 0..n {
            for x in 0..n {
                let v = b[y * n + x] as f64;
                d += (v - b[y * n + (x + 1) % n] as f64).abs() + (v - b[((y + 1) % n) * n + x] as f64).abs();
            }
        }
        let mean = d / (2 * n * n) as f64 / 255.0;
        assert!(mean > 0.36, "neighbours too alike for blue noise: {mean}");
        // low values spread out: the lowest 10 % never touch each other
        let low: Vec<usize> = (0..n * n).filter(|i| b[*i] < 26).collect();
        for &i in &low {
            for &j in &low {
                if i < j {
                    let (dx, dy) = ((i % n).abs_diff(j % n), (i / n).abs_diff(j / n));
                    let (dx, dy) = (dx.min(n - dx), dy.min(n - dy));
                    assert!(dx + dy > 1, "low ranks adjacent at {i} and {j}");
                }
            }
        }
    }
}
