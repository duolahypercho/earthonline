//! The photographic finish's post pass (`--photo-finish 1|2`, stage W4;
//! `LookOptions::photo_finish`): a camera instead of the JS chain's display
//! transform. Its own pipeline (`look_photo.wgsl`), made the first time the
//! option is on; with it off the post pass is the old pipeline, its shader text
//! and bind groups untouched, so the frame is the old one bit for bit.
//!
//! In order, on the resolved frame (after TAA, so nothing here is smeared or
//! fed back by the history):
//!
//! 1. the lens: lateral colour (red spread outward, blue inward, by
//!    `tone::LENS_CA_CORNER_PX_1080` at the corner, growing as the square of
//!    the radius), the bloom (sky.rs builds it with the photo finish's
//!    threshold and weights, `tone::PHOTO_BLOOM_*`), the residual fall-off
//!    (`tone::vignette_gain`);
//! 2. the eye's mesopic fade at night (as the old chain);
//! 3. the sensor: noise in the scene's units, shot noise and a read floor
//!    scaled by the ISO the camera picks for the scene's EV (eo-sky
//!    `photo::scene_ev100`, eo-look `tone::camera_iso`, `tone::noise_terms`),
//!    a fresh pattern each frame, a little over a pixel across;
//! 4. the camera response (`tone::camera_response`): the old chain's neutral
//!    curve on the brightest channel, the hue kept, bright colours whitened;
//! 5. sRGB, the grade (`tone::PHOTO_GRADE_SATURATION`, the look's contrast,
//!    its brightness with `tone::soft_white`), the look's finish
//!    (`tone::PHOTO_VIBRANCE`) and the display dither (as the old chain).
//!
//! Level 2 (hyper) samples the lateral colour three times a side (a smoother
//! fringe, as a lens spreads each colour over its band) and adds the chroma
//! share of the noise (`tone::GRAIN_CHROMA_SHARE`).
//!
//! The submerged camera (`--water-optics`, water.rs `draw_submerged`) keeps the
//! old display transform. Cost: one full-screen pass in place of the old one,
//! three to seven more texture reads and about a hundred ALU operations a
//! pixel more; no new pass, no new target.

use crate::camera::Camera;
use bytemuck::{Pod, Zeroable};
use eo_look::tone;

/// The pass's uniform (look_photo.wgsl `Photo`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default, PartialEq)]
pub struct PhotoUniform {
    /// x the level (1, 2), y the noise's frame seed, z the lateral colour (pixels
    /// of red-to-blue separation at the corner, this frame's size), w the
    /// vignette's residual share.
    pub lens: [f32; 4],
    /// x, y the tangents of the half field of view across and down; z, w unused.
    pub view: [f32; 4],
    /// x the shot noise's variance per unit scene value, y the read floor's
    /// variance, z the chroma share (level 2), w the ISO (for the record).
    pub grain: [f32; 4],
}

/// Frames the noise's seed runs through before repeating.
pub const SEED_PERIOD: u32 = 4096;

/// The pass's uniform for a frame: the level, the frame counter, the output
/// size, the camera, and the scene's light (the sun's altitude, the street
/// lamps' share on 0..1, the weather's dome brightness: eo-sky
/// `photo::scene_ev100`).
pub fn params(level: u8, frame: u32, out: (u32, u32), cam: &Camera, sun_alt_deg: f64, lamps_on: f64, dome_brightness: f64) -> PhotoUniform {
    let ev = eo_sky::photo::scene_ev100(sun_alt_deg, lamps_on, dome_brightness);
    let iso = tone::camera_iso(ev);
    let (shot, read) = tone::noise_terms(iso);
    let ty = (0.5 * (cam.fov_y_deg as f64).to_radians()).tan();
    let aspect = if out.1 > 0 { out.0 as f64 / out.1 as f64 } else { cam.aspect as f64 };
    PhotoUniform {
        lens: [level as f32, (frame % SEED_PERIOD) as f32, (tone::LENS_CA_CORNER_PX_1080 * out.1 as f64 / 1080.0) as f32, tone::LENS_VIGNETTE_RESIDUAL as f32],
        view: [(ty * aspect) as f32, ty as f32, 0.0, 0.0],
        grain: [shot as f32, read as f32, if level >= 2 { tone::GRAIN_CHROMA_SHARE as f32 } else { 0.0 }, iso as f32],
    }
}

/// eo-look's constants the shader takes (the curve, the grade, the crosstalk).
pub fn wgsl() -> String {
    format!(
        "const PH_NEUTRAL_MAX: f32 = {:.1};\nconst PH_CROSSTALK: f32 = {:?};\nconst PH_HUE_SKEW: f32 = {:?};\nconst PH_SATURATION: f32 = {:?};\nconst PH_VIBRANCE: f32 = {:?};\nconst PH_SOFT_KNEE: f32 = {:?};\n",
        tone::NEUTRAL_MAX_SCALED,
        tone::SENSOR_CROSSTALK as f32,
        tone::CAMERA_HUE_SKEW as f32,
        tone::PHOTO_GRADE_SATURATION as f32,
        tone::PHOTO_VIBRANCE as f32,
        tone::PHOTO_SOFT_WHITE_KNEE as f32,
    )
}

/// The pass's whole shader: the look's common source (the `Look` and `Sky`
/// structs), the old display transform's helpers (`to_srgb`, `from_srgb`,
/// `hash`), the constants, and look_photo.wgsl.
pub fn shader_source() -> String {
    format!("{}\n{}\n{}\n{}", crate::look::look_common_source(), crate::look::DISPLAY_SHADER, wgsl(), include_str!("look_photo.wgsl"))
}

/// The pipeline, its layout and its uniform.
pub struct PhotoGpu {
    pub(crate) pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
}

impl PhotoGpu {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("look_photo.wgsl"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let fs = wgpu::ShaderStages::FRAGMENT;
        let ubuf = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let tex = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let e = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: fs, ty, count: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo-bgl"),
            // the look, the frame (resolved or HDR), the sky, the bloom, a linear sampler, the pass's own uniform.
            entries: &[e(0, ubuf), e(1, tex), e(2, ubuf), e(3, tex), e(4, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)), e(5, ubuf)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("photo-layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post-photo"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("photo_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("photo_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: output_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo-uniform"),
            size: std::mem::size_of::<PhotoUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { pipeline, bgl, uniform }
    }

    /// One bind group per post source (the HDR scene, the resolved history 0, 1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_groups(&self, device: &wgpu::Device, look: &wgpu::Buffer, srcs: [&wgpu::TextureView; 3], sky: &wgpu::Buffer, bloom: &wgpu::TextureView, sampler: &wgpu::Sampler) -> [wgpu::BindGroup; 3] {
        srcs.map(|src| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("photo-bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: look.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: sky.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(bloom) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) },
                    wgpu::BindGroupEntry { binding: 5, resource: self.uniform.as_entire_binding() },
                ],
            })
        })
    }

    pub(crate) fn write(&self, queue: &wgpu::Queue, u: &PhotoUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(u));
    }

    /// Bytes of its uniform.
    pub fn bytes(&self) -> u64 {
        self.uniform.size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    #[test]
    fn the_shader_validates_and_its_uniform_matches() {
        let src = shader_source();
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("look_photo.wgsl: {}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("look_photo.wgsl validates");
        let (_, ty) = module.types.iter().find(|(_, t)| t.name.as_deref() == Some("Photo")).expect("Photo struct");
        let naga::TypeInner::Struct { span, .. } = &ty.inner else { panic!("Photo is a struct") };
        assert_eq!(*span as usize, std::mem::size_of::<PhotoUniform>());
        for entry in ["photo_vs", "photo_fs"] {
            assert!(module.entry_points.iter().any(|e| e.name == entry), "{entry}");
        }
    }

    #[test]
    fn the_frame_parameters_follow_the_light_and_the_frame() {
        let cam = Camera::new(glam::Vec3::ZERO, 0.0, 0.0, 16.0 / 9.0);
        let day = params(1, 7, (1920, 1080), &cam, 50.0, 0.0, 1.0);
        let night = params(1, 7, (1920, 1080), &cam, -30.0, 1.0, 1.0);
        // base ISO by day, a few hundred on a lit street at night; more noise at night.
        assert_eq!(day.grain[3], tone::CAMERA_ISO_BASE as f32);
        assert!(night.grain[3] > 300.0 && night.grain[3] < 800.0, "{night:?}");
        assert!(night.grain[0] > 5.0 * day.grain[0] && night.grain[1] > 50.0 * day.grain[1]);
        // the lateral colour scales with the frame's height; the field is the camera's.
        assert!((day.lens[2] - tone::LENS_CA_CORNER_PX_1080 as f32).abs() < 1e-6);
        let small = params(1, 7, (320, 180), &cam, 50.0, 0.0, 1.0);
        assert!((small.lens[2] - tone::LENS_CA_CORNER_PX_1080 as f32 / 6.0).abs() < 1e-6);
        let ty = (26f32).to_radians().tan();
        assert!((day.view[1] - ty).abs() < 1e-5 && (day.view[0] - ty * 16.0 / 9.0).abs() < 1e-4);
        // the chroma noise only at level 2; the seed wraps.
        assert_eq!(day.grain[2], 0.0);
        assert!(params(2, 7, (1920, 1080), &cam, 50.0, 0.0, 1.0).grain[2] > 0.0);
        assert_eq!(params(1, SEED_PERIOD + 3, (1920, 1080), &cam, 50.0, 0.0, 1.0).lens[1], 3.0);
    }
}
