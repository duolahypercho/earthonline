//! Ground vegetation and hill rock drawn near the camera (grass, coastal
//! scrub, wildflowers, rock outcrops). Empty for now: the renderer holds one
//! `GroundCover` (`Renderer::groundcover`) and calls its hooks where the
//! street props are updated, culled and drawn; with nothing attached every
//! hook does nothing and records no GPU command. The renderer builds it
//! with `Default::default()` and reaches it through `&self` only, so what
//! is attached later goes behind interior mutability (a `OnceLock`, as the
//! props' `Renderer::props`).

use crate::camera::Camera;
use crate::look::LookGpu;

/// The ground cover's renderer state; nothing attached yet.
#[derive(Default)]
pub struct GroundCover {}

impl GroundCover {
    /// Per frame, before the passes: the camera and the look (beside the props' `update`).
    pub fn update(&self, _queue: &wgpu::Queue, _cam: &Camera, _look: &LookGpu, _occlusion: bool) {}

    /// Before the shadow and city passes: cascade cull and casters (beside the props' `encode_pre`).
    pub fn encode_pre(&self, _enc: &mut wgpu::CommandEncoder, _look: &LookGpu) {}

    /// Inside the late cull pass, against this frame's depth pyramid (beside the props' `cull_camera`).
    pub fn cull_camera(&self, _pass: &mut wgpu::ComputePass<'_>, _hzb_bg: &wgpu::BindGroup) {}

    /// In the city pass after the street props. Returns true when it bound
    /// buffers of its own, so the caller rebinds the city's.
    pub fn draw(&self, _pass: &mut wgpu::RenderPass<'_>, _look: &LookGpu) -> bool {
        false
    }
}
