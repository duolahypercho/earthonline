//! Ground vegetation and hill rock drawn near the camera (grass, coastal
//! scrub, wildflowers, rock outcrops). The renderer holds one `GroundCover`
//! (`Renderer::groundcover`) and calls its hooks where the street props are
//! updated, culled and drawn (and in each shadow cascade's depth pass); with
//! nothing attached every hook does nothing and records no GPU command. The
//! renderer builds it with `Default::default()` and reaches it through
//! `&self` only, so what is attached goes behind interior mutability.
//!
//! Attached so far: the scanned rock (`rock`): once the landmark build has
//! placed the survey set's rock scans (eo-landmarks `rocks::scan::placed`),
//! the first `update` after it uploads them (the library's meshes, its
//! texture arrays, the pieces) on the look's device and from then on culls
//! and draws them. Without the survey root, or with `--no-scan-rocks`
//! (`--no-gg3-rock`, `--no-gg3`), nothing is placed and nothing is attached.

mod rock;

use crate::camera::Camera;
use crate::look::LookGpu;
use std::sync::OnceLock;

pub use rock::{CAST_DISTANCE_M, LOD_PX, MAX_DISTANCE_M, MIN_PX};

/// The ground cover's renderer state.
#[derive(Default)]
pub struct GroundCover {
    /// The scanned rock, once placed (None: placed, but nothing to draw).
    rock: OnceLock<Option<rock::RockGpu>>,
}

impl GroundCover {
    fn rock(&self) -> Option<&rock::RockGpu> {
        self.rock.get().and_then(|r| r.as_ref())
    }

    /// The scanned rock's GPU bytes (0 when none is attached).
    pub fn bytes(&self) -> u64 {
        self.rock().map(|r| r.bytes).unwrap_or(0)
    }

    /// Per frame, before the passes: the camera and the look (beside the props' `update`).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, look: &LookGpu, occlusion: bool) {
        if self.rock.get().is_none() {
            // attach once the landmark build has placed the scans (polled: a cheap read until then)
            if let Some(placed) = eo_landmarks::rocks::scan::placed() {
                let gpu = eo_landmarks::rocks::scan::library().filter(|_| !placed.pieces.is_empty()).map(|lib| rock::RockGpu::new(&look.device, queue, look, &lib, &placed));
                let _ = self.rock.set(gpu);
            }
        }
        if let Some(r) = self.rock() {
            r.update(queue, cam, occlusion);
            r.report_stats(&look.device, queue);
        }
    }

    /// Before the shadow and city passes: cascade cull and casters (beside the props' `encode_pre`).
    pub fn encode_pre(&self, enc: &mut wgpu::CommandEncoder, _look: &LookGpu) {
        if let Some(r) = self.rock() {
            r.encode_pre(enc);
        }
    }

    /// Inside cascade `cascade`'s shadow-depth pass, after the city's casters
    /// (`LookGpu::encode_shadows_with`), beside the props' caster passes: the
    /// near LODs' shadow casters (the lists the previous frame's `encode_pre` made).
    pub fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>, cascade: usize, _look: &LookGpu) {
        if let Some(r) = self.rock() {
            r.draw_shadow(pass, cascade);
        }
    }

    /// Inside the late cull pass, against this frame's depth pyramid (beside the props' `cull_camera`).
    pub fn cull_camera(&self, pass: &mut wgpu::ComputePass<'_>, hzb_bg: &wgpu::BindGroup) {
        if let Some(r) = self.rock() {
            r.cull_camera(pass, hzb_bg);
        }
    }

    /// In the city pass after the street props. Returns true when it bound
    /// buffers of its own, so the caller rebinds the city's.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, look: &LookGpu) -> bool {
        match self.rock() {
            Some(r) => {
                r.draw(pass, look);
                true
            }
            None => false,
        }
    }
}
