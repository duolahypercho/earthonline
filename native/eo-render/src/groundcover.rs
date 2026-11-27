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
//! the first `update` after it starts a worker thread that builds their GPU
//! state (the library's meshes, its texture arrays and their mips, the
//! pieces, the pipelines) on the look's device, and the hooks draw them
//! from the first frame after it is done: the build (about a second) never
//! runs on the render thread. A capture waits for it (`settled`). Without
//! the survey root, or with `--no-scan-rocks` (`--no-gg3-rock`, `--no-gg3`),
//! nothing is placed and nothing is attached.

mod rock;

use crate::camera::Camera;
use crate::look::LookGpu;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

pub use rock::{CAST_DISTANCE_M, MAX_DISTANCE_M, MIN_PX};

/// The ground cover's renderer state.
#[derive(Default)]
pub struct GroundCover {
    /// The scanned rock, once built (None: placed, but nothing to draw).
    rock: Arc<OnceLock<Option<rock::RockGpu>>>,
    /// Whether the worker that builds it has been started.
    started: AtomicBool,
    /// Whether the city pass writes the debug ids target (0 not told: read
    /// from the command line; 1 no; 2 yes), `set_debug_ids`.
    debug_ids: AtomicU8,
}

/// Whether this run's renderer writes the debug ids (the only eo-app path
/// that makes one is `--bench --occlusion-check`): the stand-in until the
/// renderer calls `set_debug_ids`.
fn debug_ids_from_args() -> bool {
    std::env::args().any(|a| a == "--occlusion-check")
}

/// Whether this run captures frames that must not depend on how fast the
/// worker is (`--shots`, `--occlusion-check`): the first update after the
/// placement then waits for the build. The stand-in until the captures'
/// settle reads `GroundCover::settled`.
fn waits_for_build() -> bool {
    std::env::args().any(|a| a == "--shots" || a == "--occlusion-check")
}

impl GroundCover {
    fn rock(&self) -> Option<&rock::RockGpu> {
        self.rock.get().and_then(|r| r.as_ref())
    }

    /// Tell the ground cover whether the renderer's city pass writes the
    /// debug ids target (its pipelines must match the pass). Call before
    /// the first `update`.
    pub fn set_debug_ids(&self, on: bool) {
        self.debug_ids.store(if on { 2 } else { 1 }, Ordering::Relaxed);
    }

    fn debug_ids(&self) -> bool {
        match self.debug_ids.load(Ordering::Relaxed) {
            0 => debug_ids_from_args(),
            v => v == 2,
        }
    }

    /// The scanned rock's GPU bytes (0 when none is attached).
    pub fn bytes(&self) -> u64 {
        self.rock().map(|r| r.bytes).unwrap_or(0)
    }

    /// Whether everything placed so far is attached: no placement yet, or
    /// its GPU state built (false while the worker runs).
    pub fn settled(&self) -> bool {
        // (`--no-hill-outcrops`) not while the hills' pieces wait for the drawn ground
        if eo_landmarks::rocks::scan::hills_pending() {
            return false;
        }
        !self.started.load(Ordering::Acquire) || self.rock.get().is_some()
    }

    /// Per frame, before the passes: the camera and the look (beside the props' `update`).
    pub fn update(&self, queue: &wgpu::Queue, cam: &Camera, look: &LookGpu, occlusion: bool) {
        if !self.started.load(Ordering::Acquire) {
            // start the build once the landmark build has placed the scans (polled: a cheap read until then)
            // (`--no-hill-outcrops`) not while the hills' pieces wait for the drawn ground (rocks/hills.rs `late_pass`)
            if let Some(placed) = eo_landmarks::rocks::scan::placed().filter(|_| !eo_landmarks::rocks::scan::hills_pending()) {
                self.started.store(true, Ordering::Release);
                let setup = rock::RockSetup::new(&look.device, queue, look, self.debug_ids());
                let slot = self.rock.clone();
                let h = std::thread::Builder::new()
                    .name("gc-rock-build".into())
                    .spawn(move || {
                        let gpu = eo_landmarks::rocks::scan::library().filter(|_| !placed.pieces.is_empty()).map(|lib| rock::RockGpu::new(&setup, &lib, &placed));
                        let _ = slot.set(gpu);
                    })
                    .ok();
                match h {
                    Some(h) if waits_for_build() => {
                        if h.join().is_err() {
                            let _ = self.rock.set(None);
                        }
                    }
                    Some(_) => {} // it sets the slot when done
                    None => {
                        let _ = self.rock.set(None);
                    }
                }
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
