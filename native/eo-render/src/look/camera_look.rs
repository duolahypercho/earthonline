//! The camera look on the GPU: its switches and the 3D table the post pass
//! taps (eo-look `tone::camera`, baked once, uploaded on the first update).
//!
//! Switches (read from the command line, as `HlodOptions::from_env` does):
//! `--no-camera-look` keeps the filmic transform and its exposure offset,
//! `--no-scene-meter` keeps the dome-estimate meter, `--no-lamp-photocell`
//! keeps the sun-altitude lamp ramp; `--no-gg2-camera` turns all three off
//! (the Golden Gate pass's second iteration, this track's roll-up).

use eo_look::tone::camera as cam;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraSwitches {
    pub look: bool,
    pub meter: bool,
    pub photocell: bool,
}

impl CameraSwitches {
    pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Self {
        let mut s = Self { look: true, meter: true, photocell: true };
        for a in args {
            match a.as_str() {
                "--no-gg2-camera" => s = Self { look: false, meter: false, photocell: false },
                "--no-camera-look" => s.look = false,
                "--no-scene-meter" => s.meter = false,
                "--no-lamp-photocell" => s.photocell = false,
                _ => {}
            }
        }
        s
    }

    pub fn from_env() -> Self {
        Self::from_args(std::env::args())
    }
}

pub struct CameraLut {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    texture: wgpu::Texture,
    uploaded: AtomicBool,
    pub bytes: u64,
}

impl CameraLut {
    pub fn new(device: &wgpu::Device) -> Self {
        let n = cam::LUT_SIZE as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("camera-look-lut"),
            size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("camera-look-lut"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bytes = (n as u64).pow(3) * 8;
        Self { view, sampler, texture, uploaded: AtomicBool::new(false), bytes }
    }

    /// Bake (once per process, ~10 ms on the threads available) and upload, the first time only.
    pub fn upload(&self, queue: &wgpu::Queue) {
        if self.uploaded.swap(true, Ordering::AcqRel) {
            return;
        }
        let n = cam::LUT_SIZE as u32;
        let half = |v: f32| eo_farshore::half::f16::from_f32(v).to_bits();
        let one = half(1.0);
        let texels: Vec<[u16; 4]> = cam::lut().iter().map(|t| [half(t[0]), half(t[1]), half(t[2]), one]).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n * 8), rows_per_image: Some(n) },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
        );
    }
}

/// The table's shaper constants for the shader (look_camera.wgsl `cam_shaper`):
/// (log2 of the shaper's middle, low EV, 1 / span EV, texels).
pub fn shaper_uniform() -> [f32; 4] {
    [
        0.18f32.log2(),
        cam::SHAPER_LO_EV as f32,
        (1.0 / (cam::SHAPER_HI_EV - cam::SHAPER_LO_EV)) as f32,
        cam::LUT_SIZE as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roll_up_turns_every_switch_off() {
        let s = |a: &[&str]| CameraSwitches::from_args(a.iter().map(|s| s.to_string()));
        assert_eq!(s(&[]), CameraSwitches { look: true, meter: true, photocell: true });
        assert_eq!(s(&["--no-gg2-camera"]), CameraSwitches { look: false, meter: false, photocell: false });
        assert_eq!(s(&["--no-scene-meter"]), CameraSwitches { look: true, meter: false, photocell: true });
        assert_eq!(s(&["--no-camera-look", "--no-lamp-photocell"]), CameraSwitches { look: false, meter: true, photocell: false });
    }
}
