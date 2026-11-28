//! (`--no-aerial-ground`) The open ground's colour from the aerial survey
//! (Docs/HILLS-PLAN.md stage H2): the hill builder's aerial macro map
//! (eo-hills `macro_map`, `<cache>/hills/aerial-macro-v1.bin`: USDA NAIP
//! 0.3 m imagery averaged to 2 m texels on the surveyed plan, the survey's
//! haze taken out, scaled to the measured lawn luminance, its shadows lifted,
//! the footprints left out) drawn as the terrain's base colour where its
//! texels hold open ground (draw.wgsl `hm_ground`), in place of the per-vertex
//! tones guessed from the 16 m green share and WorldCover (ground2.rs). The
//! scans, the turf and the bare ground then work from that colour as they did
//! from the tone; the hills' cover mosaic (`--ground-cover`, `nt_cover`) steps
//! back where the survey's colour already carries the real one.
//!
//! The terrain at 8 m keeps its vertex colour where the map has no data (the
//! sea, the far land, under footprints), at beach sand, at the Golden Gate's
//! surveyed materials and at the bare-ground marks (their vertex alpha). With
//! the switch off, the map missing or unreadable, or no BC sampling on the
//! adapter, a 1 x 1 stand-in is bound and the shader's `HM_ON` is false: the
//! frame is as before.

use std::sync::OnceLock;

/// The switch.
pub fn switch_on() -> bool {
    !std::env::args().any(|a| a == "--no-aerial-ground")
}

/// The map's header, read once (None: off, missing or unreadable).
fn header() -> Option<&'static (eo_hills::macro_map::Header, u64)> {
    static H: OnceLock<Option<(eo_hills::macro_map::Header, u64)>> = OnceLock::new();
    H.get_or_init(|| {
        if !switch_on() {
            return None;
        }
        let p = eo_hills::macro_map::file_path(&eo_hills::cache_dir());
        if !p.is_file() {
            eprintln!("aerial ground: no map at {} (eo-hills cook macro); the ground as before", p.display());
            return None;
        }
        match eo_hills::macro_map::read_header(&p) {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("aerial ground: {e:#}; the ground as before");
                None
            }
        }
    })
    .as_ref()
}

/// The map's GPU texture (or the 1 x 1 stand-in) and whether the shader uses it.
pub struct AerialGround {
    pub view: wgpu::TextureView,
    _tex: wgpu::Texture,
    pub bytes: u64,
    pub on: bool,
}

impl AerialGround {
    pub fn new(d: &wgpu::Device, q: &wgpu::Queue) -> Self {
        let bc = d.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        if let Some((h, offset)) = header().filter(|_| bc) {
            match Self::upload(d, q, h, *offset) {
                Ok(a) => return a,
                Err(e) => eprintln!("aerial ground: {e:#}; the ground as before"),
            }
        } else if header().is_some() {
            eprintln!("aerial ground: the adapter samples no BC; the ground as before");
        }
        let tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("aerial-ground-off"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        q.write_texture(tex.as_image_copy(), &[0, 0, 0, 0], wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) }, wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
        let view = tex.create_view(&Default::default());
        AerialGround { view, _tex: tex, bytes: 0, on: false }
    }

    fn upload(d: &wgpu::Device, q: &wgpu::Queue, h: &eo_hills::macro_map::Header, offset: u64) -> anyhow::Result<Self> {
        let p = eo_hills::macro_map::file_path(&eo_hills::cache_dir());
        let all = std::fs::read(&p)?;
        let data = all.get(offset as usize..).ok_or_else(|| anyhow::anyhow!("{}: shorter than its header", p.display()))?;
        let tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("aerial-ground"),
            size: wgpu::Extent3d { width: h.width, height: h.height, depth_or_array_layers: 1 },
            mip_level_count: h.levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bc7RgbaUnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut bytes = 0;
        for (mip, l) in h.levels.iter().enumerate() {
            let blocks = (l.width.div_ceil(4), l.height.div_ceil(4));
            let level = data.get(l.offset as usize..(l.offset + l.length) as usize).ok_or_else(|| anyhow::anyhow!("{}: level {mip} past the end", p.display()))?;
            q.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: mip as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                level,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(blocks.0 * 16), rows_per_image: Some(blocks.1) },
                wgpu::Extent3d { width: blocks.0 * 4, height: blocks.1 * 4, depth_or_array_layers: 1 },
            );
            bytes += l.length;
        }
        eprintln!(
            "aerial ground: {} x {} texels of {} m ({} levels, {:.1} MB), K {:.3}, NAIP {} tiles",
            h.width,
            h.height,
            h.texel_m,
            h.levels.len(),
            bytes as f64 / 1e6,
            h.k,
            h.inputs.naip_tiles_used
        );
        let view = tex.create_view(&Default::default());
        Ok(AerialGround { view, _tex: tex, bytes, on: true })
    }
}

/// draw.wgsl's constants: whether the map is drawn, and its box in game
/// metres (the surveyed plan times the ground plan's scale, as the elevation
/// grid's frame is scaled: eo-data `ElevationGrid::load`).
pub fn wgsl() -> String {
    let gs = eo_data::groundscale::scale();
    let (on, o, s) = match header() {
        Some((h, _)) if switch_on() => (true, [h.origin_x * gs, h.origin_z * gs], [h.width as f64 * h.texel_m * gs, h.height as f64 * h.texel_m * gs]),
        _ => (false, [0.0, 0.0], [1.0, 1.0]),
    };
    format!(
        "\n// generated by aerial_ground.rs (`--no-aerial-ground`)\nconst HM_ON: bool = {on};\nconst HM_ORIGIN: vec2<f32> = vec2<f32>({:?}, {:?});\nconst HM_SIZE: vec2<f32> = vec2<f32>({:?}, {:?});\n",
        o[0] as f32, o[1] as f32, s[0] as f32, s[1] as f32
    )
}
