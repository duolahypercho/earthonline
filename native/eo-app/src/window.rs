//! Windowed fly-through. Not launched by the bench or the tests.
//!
//! Controls: right mouse button toggles mouse look (cursor captured);
//! W/A/S/D move, Q/E down/up, Shift x5 speed, Ctrl x0.2, Esc releases the
//! mouse or quits, F prints the camera position.

use crate::models::{ModelRuntime, make_renderer};
use crate::{Args, World, load_world};
use anyhow::Result;
use eo_render::camera::Camera;
use eo_render::{GpuContext, GpuOptions, Renderer, Targets, wgpu};
use glam::Vec3;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
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
    agents: crate::agents::AgentRuntime, // agents
    audio: crate::audio::AudioRuntime, // the device (`--no-audio`, `--audio-null`)
    pacer: eo_render::pace::WaitPacer, // the driver's submission spike, taken in the swapchain wait
}

struct App {
    world: World,
    args: Args,
    vsync: bool,
    msaa: u32,
    opts: GpuOptions,
    gfx: Option<Gfx>,
    cam: Camera,
    keys: HashSet<KeyCode>,
    looking: bool,
    last: Instant,
    fps_frames: u32,
    fps_t: Instant,
    process_start: Instant,
    first_frame_reported: bool,
}

pub fn run(args: &Args, process_start: Instant) -> Result<()> {
    if args.perf.thread_priority {
        eo_render::hitch::raise_render_thread(); // render thread above the streaming workers
    }
    let world = load_world(args)?;
    let aspect = args.width as f32 / args.height as f32;
    let cam = if let Some(habitat) = args.dive_site {
        let site = world.dive_sites.iter().find(|s| s.habitat == habitat)
            .ok_or_else(|| anyhow::anyhow!("the requested dive site requires water and a shallow, submerged bed"))?;
        if args.surface_start && site.beach.is_none() { anyhow::bail!("the cove beach waterline is unavailable"); }
        if args.surface_start { site.surface_camera(aspect, args.far) } else { site.camera(aspect, args.far) }
    } else if args.start_underwater {
        crate::bench::dive_camera(&world, aspect, args.far)
            .ok_or_else(|| anyhow::anyhow!("the underwater start requires an open-water point above the bay bed"))?
    } else {
        let spawn = crate::bench::spawn();
        let ground = world.elevation.ground(spawn[0] as f64, spawn[1] as f64) as f32;
        Camera::new(Vec3::new(spawn[0], ground + 1.7, spawn[1]), 0.8, 0.0, aspect)
    };
    let event_loop = EventLoop::new()?;
    let mut app = App {
        world,
        args: args.clone(),
        vsync: args.vsync,
        msaa: args.msaa,
        opts: GpuOptions {
            indirect_validation: args.indirect_validation,
            force_no_indirect_count: args.no_indirect_count,
            ..Default::default()
        },
        gfx: None,
        cam,
        keys: HashSet::new(),
        looking: false,
        last: Instant::now(),
        fps_frames: 0,
        fps_t: Instant::now(),
        process_start,
        first_frame_reported: false,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

impl App {
    fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
        let attrs = Window::default_attributes()
            .with_title("Earth Online native slice")
            .with_inner_size(PhysicalSize::new(1920u32, 1080u32));
        let window = Arc::new(el.create_window(attrs)?);
        let instance = GpuContext::create_instance(&self.opts);
        let surface = instance.create_surface(window.clone())?;
        let ctx = GpuContext::new(instance, Some(&surface), self.opts)?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&ctx.adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut config = surface
            .get_default_config(&ctx.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| anyhow::anyhow!("surface unsupported by adapter"))?;
        config.format = format;
        config.present_mode = if self.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        surface.configure(&ctx.device, &config);
        let (renderer, base): (Renderer, _) = make_renderer(&ctx, &self.world, &self.args, format, false)?;
        let targets = Targets::new(&ctx.device, config.width, config.height, format, self.msaa);
        println!(
            "adapter {} ({:?}); indirect count {}; {} static objects, {} triangles loaded; {} Blender buildings streaming",
            ctx.info.name,
            ctx.info.backend,
            renderer.compact,
            self.world.scene.stats.objects,
            self.world.scene.stats.triangles,
            self.world.authored.buildings.len()
        );
        let models = ModelRuntime::start(&mut self.world, &self.args, base, self.cam.eye, self.process_start);
        let agents = crate::agents::AgentRuntime::start(&self.world, &self.args, crate::agents::AgentRuntime::focus_of(self.cam.eye, self.cam.forward())); // agents
        crate::props::global_start(&self.world, &self.args); // props
        let pacer = eo_render::pace::WaitPacer::new(self.args.perf.submit_pacing);
        let audio = crate::audio::AudioRuntime::start(&self.world, eo_audio::Output::Device); // audio
        self.gfx = Some(Gfx { window, surface, config, ctx, renderer, targets, models, agents, audio, pacer });
        Ok(())
    }

    fn resize(&mut self, w: u32, h: u32) {
        let Some(g) = self.gfx.as_mut() else { return };
        if w == 0 || h == 0 {
            return;
        }
        g.config.width = w;
        g.config.height = h;
        g.surface.configure(&g.ctx.device, &g.config);
        g.targets = Targets::new(&g.ctx.device, w, h, g.config.format, self.msaa);
        self.cam.aspect = w as f32 / h as f32;
    }

    fn step_camera(&mut self, dt: f32) {
        let k = |c| self.keys.contains(&c);
        let mut speed = 12.0;
        if k(KeyCode::ShiftLeft) || k(KeyCode::ShiftRight) {
            speed *= 5.0;
        }
        if k(KeyCode::ControlLeft) {
            speed *= 0.2;
        }
        let f = self.cam.forward();
        let r = self.cam.right();
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
        if k(KeyCode::KeyE) {
            m += Vec3::Y;
        }
        if k(KeyCode::KeyQ) {
            m -= Vec3::Y;
        }
        self.cam.eye += m.normalize_or_zero() * speed * dt;
    }

    fn set_looking(&mut self, on: bool) {
        self.looking = on;
        if let Some(g) = &self.gfx {
            if on {
                let _ = g
                    .window
                    .set_cursor_grab(CursorGrabMode::Locked)
                    .or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Confined));
            } else {
                let _ = g.window.set_cursor_grab(CursorGrabMode::None);
            }
            g.window.set_cursor_visible(!on);
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.step_camera(dt);
        let Some(g) = self.gfx.as_mut() else { return };
        let (acquired, _) = g.pacer.around(&g.ctx.device, &g.ctx.queue, || g.surface.get_current_texture());
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                g.surface.configure(&g.ctx.device, &g.config);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        // Streamed models and kit packs: nearest to the camera first, at most
        // the upload budget per frame.
        g.models.set_camera(self.cam.eye);
        if let Err(e) = g.models.pump(&g.ctx, &g.renderer, &self.world, self.cam.eye) {
            eprintln!("model streaming: {e:#}");
        }
        crate::props::global_pump_live(&g.ctx, &g.renderer, &mut g.models.uploads, self.cam.eye, g.agents.sim_time, dt as f64); // props
        g.agents.parked = crate::props::global_parked(g.agents.lib.as_deref(), self.cam.eye); // props
        g.agents.pump(&g.ctx, &g.renderer, crate::agents::AgentRuntime::focus_of(self.cam.eye, self.cam.forward()), dt as f64); // agents
        g.audio.frame(crate::audio::FrameIn::camera(&self.cam, dt as f64, &mut g.agents, &g.renderer, &self.world.elevation)); // audio
        g.renderer.update(&g.ctx.queue, &self.cam);
        let mut enc = g.ctx.device.create_command_encoder(&Default::default());
        let (attach, resolve) = match &g.targets.msaa {
            Some(m) => (m, Some(&view)),
            None => (&view, None),
        };
        g.renderer.encode(&mut enc, attach, resolve, &g.targets.depth, None);
        eo_render::pace::submit(&g.ctx.queue, [enc.finish()]); // counted
        g.window.pre_present_notify();
        g.ctx.queue.present(frame);
        if !self.first_frame_reported {
            self.first_frame_reported = true;
            println!("process start -> first presented frame: {:.0} ms", self.process_start.elapsed().as_secs_f64() * 1e3);
        }
        self.fps_frames += 1;
        let el = self.fps_t.elapsed().as_secs_f32();
        if el >= 1.0 {
            g.window.set_title(&format!(
                "Earth Online native slice - {:.0} fps ({:.2} ms) - slice renderer, not the game",
                self.fps_frames as f32 / el,
                el * 1e3 / self.fps_frames as f32
            ));
            self.fps_frames = 0;
            self.fps_t = Instant::now();
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
            WindowEvent::Resized(s) => self.resize(s.width, s.height),
            WindowEvent::Focused(false) => {
                self.keys.clear();
                self.set_looking(false);
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                let on = !self.looking;
                self.set_looking(on);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            if code == KeyCode::Escape {
                                if self.looking {
                                    self.set_looking(false);
                                } else {
                                    el.exit();
                                }
                            }
                            if code == KeyCode::KeyF {
                                println!("camera eye {:?} yaw {:.3} pitch {:.3}", self.cam.eye, self.cam.yaw, self.cam.pitch);
                            }
                            self.keys.insert(code);
                        }
                        ElementState::Released => {
                            self.keys.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.looking {
                self.cam.yaw += delta.0 as f32 * 0.0025;
                self.cam.pitch = (self.cam.pitch - delta.1 as f32 * 0.0025).clamp(-1.5, 1.5);
            }
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }
}
