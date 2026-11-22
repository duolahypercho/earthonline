//! Earth Online's interface, drawn with wgpu over the 3D frame: the title
//! screen and its menu, boot progress, the pause menu with its map, the
//! settings screens, and the in-game HUD with its radar.
//!
//! Ported from the JS game (`src/ui/title-menu.*`, `pause-menu.*`,
//! `performance-settings.*`, `player-hud.*`, `hud-tokens.css`, `hud-map.js`,
//! `pause-map.js`, `city-map-render.js`). The design tokens are data in
//! [`theme`], copied from the earlier browser version's stylesheets.
//!
//! The crate needs only a device, a queue and a target view. Input arrives
//! as plain data ([`Input`]); the host decides what [`Event`]s mean.
//!
//! Cost model: a draw list is rebuilt only when something on screen changes
//! (input, state, a running transition); text runs are shaped once and
//! cached; map images are re-rendered only when their view changes. An
//! unchanged HUD frame writes nothing and records one render pass with one
//! or two draw calls.

pub mod draw;
pub mod map;
mod screens;
pub mod settings;
pub mod text;
pub mod theme;

use draw::{DrawList, ImageId, Painter, Rect, Scratch, UiGpu};
use map::{MapGpu, MapMesh, MapTarget, MapView, StreetIndex, Style};
use settings::Settings;
use std::path::{Path, PathBuf};
use text::TextSystem;
use theme::Viewport;

/// Keys the interface reacts to (bit flags in [`Input::keys`]).
pub mod key {
    pub const UP: u32 = 1;
    pub const DOWN: u32 = 2;
    pub const LEFT: u32 = 4;
    pub const RIGHT: u32 = 8;
    pub const ENTER: u32 = 16;
    pub const ESCAPE: u32 = 32;
    pub const TAB: u32 = 64;
    pub const SHIFT_TAB: u32 = 128;
}

/// what the interface did this frame, for its sounds (bits in [`Ui::cues`];
/// the same bits as `eo_audio::city::cue`).
pub mod cue {
    pub const HOVER: u32 = 1;
    pub const MOVE: u32 = 2;
    pub const CONFIRM: u32 = 4;
    pub const BACK: u32 = 8;
    pub const TOGGLE: u32 = 16;
    pub const START: u32 = 32;
    pub const OPEN: u32 = 64;
}

/// One frame of input, as plain data (the host maps its window events,
/// and later a gamepad, onto this).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// Pointer position in window pixels, if the pointer is over the window.
    pub mouse: Option<[f32; 2]>,
    /// The primary button went down this frame.
    pub click: bool,
    /// The primary button is held (for dragging the pause map).
    pub held: bool,
    /// Pointer movement while held, pixels.
    pub drag: [f32; 2],
    /// Wheel steps this frame (positive = away from the user = zoom in).
    pub wheel: f32,
    /// Keys pressed this frame (`key::*`).
    pub keys: u32,
}

/// What the host should do after a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Start Game (or Ghost View): show boot progress and load the city.
    StartGame { observe: bool },
    /// Close Game from the title's Exit view.
    Exit,
    /// Resume from the pause menu.
    Resume,
    /// Quit to title from the pause menu.
    QuitToTitle,
    /// A setting changed (already saved to the settings file).
    SettingsChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Title,
    /// The title with Start pressed: the boot progress line.
    Loading,
    Hud,
    Pause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleView {
    Main,
    Settings,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseTab {
    Map,
    Journal,
    Inventory,
    Garage,
    Settings,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HudMode {
    OnFoot,
    /// Driving, with the vehicle's speed in km/h (the JS cluster's unit).
    Drive { speed_kph: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponKind {
    Fists,
    Pistol,
    Smg,
    Rifle,
    Shotgun,
    Thrown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weapon {
    pub kind: WeaponKind,
    pub magazine: u32,
    pub reserve: u32,
}

/// The HUD's input: plain numbers read from the runtime once per frame
/// (`update(snapshot)` in src/ui/player-hud.js).
#[derive(Clone, Debug, PartialEq)]
pub struct HudState {
    /// Player position, city metres (x east, z south).
    pub player: [f32; 2],
    /// Heading: 0 looks north (-z), positive turns east.
    pub heading: f32,
    pub health: f32,
    pub armour: f32,
    pub cash: i64,
    pub wanted: u8,
    /// Hour of day, 0..24.
    pub hour: f32,
    pub mode: HudMode,
    pub weapon: Option<Weapon>,
    pub objective: Option<(String, String)>,
    pub prompt: Option<(String, String)>,
    /// `--hud-sky-r/g/b`.
    pub sky: theme::Color,
}

impl Default for HudState {
    fn default() -> Self {
        Self {
            player: [0.0, 0.0],
            heading: 0.0,
            health: 1.0,
            armour: 0.0,
            cash: 0,
            wanted: 0,
            hour: 9.0,
            mode: HudMode::OnFoot,
            weapon: None,
            objective: Some(("Explore freely".into(), "San Francisco".into())),
            prompt: None,
            sky: theme::hud::SKY,
        }
    }
}

/// A label over another player (online). The host decides where it hangs
/// (`x`, `y`: the point over the head it points at, window pixels), how far
/// away the head is (`depth`, metres: the nearer tag keeps its place and is
/// drawn on top), how opaque (distance fade, occlusion) and how large it is;
/// the HUD lays the tags out: a tag that would cover a nearer one moves up
/// out of its way and keeps a 1 px leader line down to its head, and one
/// with no room left is not drawn. Drawn under the HUD's plates.
#[derive(Clone, Debug, PartialEq)]
pub struct NameTag {
    pub x: f32,
    pub y: f32,
    pub depth: f32,
    pub name: String,
    /// A second line inside the plate (what the player is doing: "waving"), or none.
    pub detail: Option<String>,
    pub alpha: f32,
    /// The name's text size, window pixels; the second line is `detail_size`.
    pub size: f32,
    pub detail_size: f32,
    /// Its own player's head on screen (x0, y0, x1, y1): the one face in
    /// `Ui::tag_avoid` its plate may cover.
    pub head: [f32; 4],
}

/// Boot progress (`setLoadingStatus` in title-menu.js).
#[derive(Clone, Debug, PartialEq)]
pub struct Loading {
    pub progress: f32,
    pub phase: String,
    pub detail: String,
}

impl Default for Loading {
    fn default() -> Self {
        Self { progress: 0.02, phase: "Preparing graphics".into(), detail: "Reserving the graphics device.".into() }
    }
}

/// What the city data gives the map (loaded by the host from `eo-data`).
pub struct MapInputs<'a> {
    pub city: &'a eo_data::City,
    pub land: Option<&'a eo_data::LandCover>,
    /// The title backdrop's centre (the spawn), city metres.
    pub backdrop_centre: [f32; 2],
}

/// Hit-test regions recorded while building (reused, not reallocated).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Action {
    Start,
    Observe,
    TitleSettings,
    TitleExit,
    Back,
    Close,
    Motion,
    InvertY,
    Slider(&'static str),
    Preset(&'static str),
    Option(&'static str),
    /// A world row (`settings::WORLD_OPTIONS`).
    World(&'static str),
    Fps,
    Adaptive,
    Volume,
    Mix(&'static str),
    Resume,
    PauseSettings,
    Quit,
    QuitConfirm,
    Tab(PauseTab),
    MapIn,
    MapOut,
    MapPlayer,
    MapFit,
    MapPane,
}

pub(crate) struct Hit {
    pub rect: Rect,
    pub action: Action,
    pub focusable: bool,
}

/// Counters for the "no work when nothing changed" checks and the bench.
#[derive(Clone, Copy, Debug, Default)]
pub struct UiStats {
    pub frames: u64,
    pub rebuilds: u64,
    pub uploads: u64,
    pub map_renders: u64,
    pub shaped_runs: u64,
    pub rasterised_glyphs: u64,
    pub vertices: usize,
    pub draw_calls: usize,
}

pub(crate) struct MapState {
    gpu: MapGpu,
    streets: StreetIndex,
    hud: MapTarget,
    hud_image: ImageId,
    pause: Option<MapTarget>,
    pause_image: ImageId,
    title: Option<MapTarget>,
    title_image: ImageId,
    backdrop_centre: [f32; 2],
    mesh_ms: f64,
}

/// Presentation memory for the HUD (the JS keeps the same).
#[derive(Clone, Debug, Default)]
pub(crate) struct HudMemory {
    pub cash: Option<i64>,
    pub cash_from: i64,
    pub cash_changed_at: f64,
    pub clock_key: Option<i32>,
    pub clock_at: f64,
    pub objective: Option<(String, String)>,
    pub objective_at: f64,
    pub weapon: Option<WeaponKind>,
    pub weapon_at: f64,
    pub street: String,
    pub street_at: [f32; 2],
}

pub(crate) const HUD_MAP_SIZE: u32 = 2048;
pub(crate) const HUD_MAP_S: f32 = 2.0;
/// Focusable items with a hover/focus transition, enough for every screen (the
/// pause screen on its Settings tab holds 33 with the native rows: 5 tabs, 25
/// in the sheet, 3 tiles); an item past the slots would show no hover.
pub(crate) const HOVER_SLOTS: usize = 48;
pub(crate) const HUD_MAP_RECENTRE_M: f32 = 200.0;

pub struct Ui {
    pub(crate) text: TextSystem,
    list: DrawList,
    scratch: Scratch,
    gpu: UiGpu,
    pub(crate) map: Option<MapState>,
    pub screen: Screen,
    pub title_view: TitleView,
    pub pause_tab: PauseTab,
    pub settings: Settings,
    settings_path: Option<PathBuf>,
    pub loading: Loading,
    pub hud: HudState,
    /// Name tags over the other players (the HUD screen draws them).
    pub tags: Vec<NameTag>,
    last_tags: Vec<NameTag>,
    /// Faces on screen (x0, y0, x1, y1: the other players' and the street
    /// people's heads) a tag's plate keeps off, moving aside or up with a
    /// leader line (a plate never sits on someone else's face).
    pub tag_avoid: Vec<[f32; 4]>,
    last_avoid: Vec<[f32; 4]>,
    /// The HUD's plates (radar, objective, clock); false leaves only the
    /// name tags (captures of a scene, a photo).
    pub hud_chrome: bool,
    pub(crate) hud_mem: HudMemory,
    pub(crate) hits: Vec<Hit>,
    pub(crate) focus: Option<usize>,
    pub(crate) hover: Option<usize>,
    /// Per focusable item: hover/focus transition progress 0..1.
    pub(crate) hover_t: [f32; HOVER_SLOTS],
    pub(crate) screen_t0: f64,
    pub(crate) now: f64,
    last_now: f64,
    pub(crate) vp: Viewport,
    pub(crate) scroll: f32,
    pub(crate) pause_view: Option<(f32, f32, f32)>,
    pub(crate) pause_pane: Rect,
    /// The world is built and Start may be pressed.
    pub ready: bool,
    /// Respect `prefers-reduced-motion` (no entry animations).
    pub reduced_motion: bool,
    dirty: bool,
    last_hud: Option<HudState>,
    pub(crate) animating: bool,
    format: Option<wgpu::TextureFormat>,
    pub stats: UiStats,
    /// Formatting buffer reused for numbers and labels.
    pub(crate) fmt: String,
    /// the cues since the host last took them (`take_cues`).
    pub cues: u32,
}

impl Ui {
    /// `repo_root` locates the fallback font; `settings_path` is where
    /// settings persist (None: in memory only, as with blocked storage).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, repo_root: Option<&Path>, settings_path: Option<PathBuf>, map: Option<MapInputs<'_>>) -> Ui {
        let settings = settings_path.as_deref().map(Settings::load).unwrap_or_default();
        let mut gpu = UiGpu::new(device, queue);
        let map = map.map(|m| {
            let mesh = MapMesh::build(m.city);
            let mesh_ms = mesh.build_ms;
            let mg = MapGpu::new(device, queue, &mesh, m.land);
            let hud = mg.target(device, Style::Hud, HUD_MAP_SIZE, HUD_MAP_SIZE, false);
            let hud_image = gpu.add_image(device, &hud.view_tex);
            MapState {
                gpu: mg,
                streets: StreetIndex::build(m.city),
                hud,
                hud_image,
                pause: None,
                pause_image: 0,
                title: None,
                title_image: 0,
                backdrop_centre: m.backdrop_centre,
                mesh_ms,
            }
        });
        Ui {
            text: TextSystem::new(repo_root),
            list: DrawList::default(),
            scratch: Scratch::default(),
            gpu,
            map,
            screen: Screen::Title,
            title_view: TitleView::Main,
            pause_tab: PauseTab::Map,
            settings,
            settings_path,
            loading: Loading::default(),
            hud: HudState::default(),
            tags: vec![],
            last_tags: vec![],
            tag_avoid: vec![],
            last_avoid: vec![],
            hud_chrome: true,
            hud_mem: HudMemory::default(),
            hits: Vec::with_capacity(64),
            focus: Some(0),
            hover: None,
            hover_t: [0.0; HOVER_SLOTS],
            screen_t0: 0.0,
            now: 0.0,
            last_now: 0.0,
            vp: Viewport { w: 1920.0, h: 1080.0 },
            scroll: 0.0,
            pause_view: None,
            pause_pane: Rect::new(0.0, 0.0, 0.0, 0.0),
            ready: true,
            reduced_motion: false,
            dirty: true,
            last_hud: None,
            animating: true,
            format: None,
            stats: UiStats::default(),
            fmt: String::with_capacity(64),
            cues: 0, // audio
        }
    }

    /// Milliseconds spent building the map mesh at start-up.
    pub fn map_mesh_ms(&self) -> Option<f64> {
        self.map.as_ref().map(|m| m.mesh_ms)
    }

    /// Fonts the stacks resolved to so far.
    pub fn fonts(&self) -> Vec<(String, text::Resolved)> {
        self.text.resolved()
    }

    /// Switch screens (resets focus and the screen's motion clock).
    /// the interface's cues since the last call (`cue::*` bits).
    pub fn take_cues(&mut self) -> u32 {
        std::mem::take(&mut self.cues)
    }

    pub fn set_screen(&mut self, s: Screen) {
        if self.screen != s {
            // Loading keeps the title's clock: it is the same scene.
            if !(self.screen == Screen::Title && s == Screen::Loading) {
                self.screen_t0 = self.now;
            }
            self.screen = s;
            self.focus = Some(0);
            self.scroll = 0.0;
            self.hover_t = [0.0; HOVER_SLOTS];
            if s == Screen::Pause {
                self.pause_tab = PauseTab::Map;
                self.pause_view = None;
            }
            self.dirty = true;
        }
    }

    pub fn set_title_view(&mut self, v: TitleView) {
        self.title_view = v;
        self.focus = Some(0);
        self.scroll = 0.0;
        self.dirty = true;
    }

    pub fn set_pause_tab(&mut self, t: PauseTab) {
        self.pause_tab = t;
        self.focus = None;
        self.scroll = 0.0;
        self.dirty = true;
    }

    /// The motion clock, seconds (the host passes its frame time).
    pub fn set_time(&mut self, now: f64) {
        self.now = now;
    }

    /// Reset the screen's motion clock to `now - t` (captures pick a moment).
    pub fn set_screen_age(&mut self, t: f64) {
        self.screen_t0 = self.now - t;
        self.dirty = true;
    }

    /// Scroll a scrolled panel (settings) to `px` from its top.
    pub fn set_scroll(&mut self, px: f32) {
        self.scroll = px.max(0.0);
        self.dirty = true;
    }

    /// Centre and scale of the pause map (px per metre); None = the opening view.
    pub fn set_pause_map_view(&mut self, v: Option<(f32, f32, f32)>) {
        self.pause_view = v;
        self.dirty = true;
    }

    pub fn save_settings(&self) {
        if let Some(p) = &self.settings_path {
            if let Err(e) = self.settings.save(p) {
                eprintln!("settings: could not write {}: {e}", p.display());
            }
        }
    }

    /// Handle input, rebuild the draw list if anything on screen changed,
    /// upload it, and render any map image whose view changed. Map renders
    /// are recorded into `enc` (submit it before or with the UI pass).
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, width: u32, height: u32, input: &Input) -> Option<Event> {
        self.stats.frames += 1;
        let vp = Viewport { w: width as f32, h: height as f32 };
        if vp != self.vp {
            self.vp = vp;
            self.dirty = true;
        }
        let dt = (self.now - self.last_now).clamp(0.0, 0.1) as f32;
        self.last_now = self.now;
        let event = if *input != Input::default() { self.handle_input(input) } else { None };
        if *input != Input::default() {
            self.dirty = true;
        }
        // Hover/focus transitions.
        let mut moving = false;
        for i in 0..self.hover_t.len() {
            let target = if Some(i) == self.hover || Some(i) == self.focus { 1.0 } else { 0.0 };
            let cur = self.hover_t[i];
            if cur != target {
                let step = dt / (theme::title::HOVER_MS / 1000.0);
                self.hover_t[i] = if target > cur { (cur + step).min(1.0) } else { (cur - step).max(0.0) };
                moving = true;
            }
        }
        if self.screen == Screen::Hud && (self.last_hud.as_ref() != Some(&self.hud) || self.tags != self.last_tags || self.tag_avoid != self.last_avoid) {
            self.dirty = true;
        }
        self.update_maps(device, queue, enc);
        let rebuild = self.dirty || self.animating || moving;
        if rebuild {
            self.build();
            self.gpu.upload(device, queue, &self.list, &mut self.text);
            self.stats.rebuilds += 1;
            self.stats.uploads += 1;
            self.dirty = false;
            if self.screen == Screen::Hud {
                self.last_hud = Some(self.hud.clone());
                self.last_tags.clone_from(&self.tags);
                self.last_avoid.clone_from(&self.tag_avoid);
            }
        }
        self.write_globals(queue);
        self.stats.shaped_runs = self.text.shaped_runs;
        self.stats.rasterised_glyphs = self.text.rasterised_glyphs;
        self.stats.vertices = self.list.verts.len();
        self.stats.draw_calls = self.gpu.cmds.len();
        event
    }

    /// Record the UI pass over `view` (the 3D frame already in it).
    pub fn draw(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, format: wgpu::TextureFormat, timestamps: Option<(&wgpu::QuerySet, u32)>) {
        if self.format != Some(format) {
            self.format = Some(format);
        }
        self.gpu.globals.screen[2] = if format.is_srgb() { 1.0 } else { 0.0 };
        self.gpu.encode(device, enc, view, format, None, timestamps);
    }

    /// Build the pipeline for `format` now rather than on the first frame.
    pub fn warm(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        self.gpu.warm(device, format);
    }

    fn write_globals(&mut self, queue: &wgpu::Queue) {
        let g = &mut self.gpu.globals;
        g.screen[0] = self.vp.w;
        g.screen[1] = self.vp.h;
        if let Some(m) = &self.map {
            let h = &self.hud;
            let (s, c) = h.heading.sin_cos();
            let pitch = theme::hud::CAM_PITCH_DEG.to_radians();
            let plate_h = screens::hud_map_size(self.vp).1;
            g.radar0 = [h.player[0], h.player[1], s, -c];
            g.radar1 = [theme::hud::FOCAL_RATIO * plate_h, theme::hud::CAM_HEIGHT_M, theme::hud::CAM_BACK_M, 1.0];
            if let Some(v) = m.hud.current {
                let span = v.w as f32 / v.s;
                g.radar2 = [pitch.sin(), pitch.cos(), v.cx - span * 0.5, v.cz - span * 0.5];
                g.radar3 = [1.0 / span, 1.0 / span, 0.0, 0.0];
            }
        }
        self.gpu.write_globals(queue);
    }

    fn update_maps(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder) {
        let Some(m) = self.map.as_mut() else { return };
        let mut rendered = 0;
        match self.screen {
            Screen::Hud => {
                let p = self.hud.player;
                let need = match m.hud.current {
                    Some(v) => (v.cx - p[0]).abs() > HUD_MAP_RECENTRE_M || (v.cz - p[1]).abs() > HUD_MAP_RECENTRE_M,
                    None => true,
                };
                if need {
                    let v = MapView { cx: p[0].round(), cz: p[1].round(), s: HUD_MAP_S, w: HUD_MAP_SIZE, h: HUD_MAP_SIZE };
                    if m.gpu.render(queue, enc, &mut m.hud, v) {
                        rendered += 1;
                    }
                    self.dirty = true;
                }
            }
            Screen::Title | Screen::Loading => {
                if m.title.is_none() {
                    let t = m.gpu.target(device, Style::Pause, theme::map::BACKDROP_W, theme::map::BACKDROP_H, false);
                    if m.title_image == 0 {
                        m.title_image = self.gpu.add_image(device, &t.view_tex);
                    } else {
                        self.gpu.set_image(device, m.title_image, &t.view_tex);
                    }
                    m.title = Some(t);
                }
                let t = m.title.as_mut().unwrap();
                let c = m.backdrop_centre;
                let v = MapView { cx: c[0], cz: c[1], s: theme::map::BACKDROP_W as f32 / theme::map::BACKDROP_METRES_WIDE, w: theme::map::BACKDROP_W, h: theme::map::BACKDROP_H };
                if m.gpu.render(queue, enc, t, v) {
                    rendered += 1;
                }
            }
            Screen::Pause => {
                let pane = self.pause_pane;
                if pane.w >= 1.0 && pane.h >= 1.0 && self.pause_tab == PauseTab::Map {
                    let (w, h) = (pane.w.round() as u32, pane.h.round() as u32);
                    let fits = m.pause.as_ref().is_some_and(|t| t.texture.width() == w && t.texture.height() == h);
                    if !fits {
                        let t = m.gpu.target(device, Style::Pause, w, h, true);
                        if m.pause_image == 0 {
                            m.pause_image = self.gpu.add_image(device, &t.view_tex);
                        } else {
                            self.gpu.set_image(device, m.pause_image, &t.view_tex);
                        }
                        m.pause = Some(t);
                    }
                    let (cx, cz, s) = self.pause_view.unwrap_or((self.hud.player[0], self.hud.player[1], theme::pause::OPEN_SCALE));
                    let v = MapView { cx, cz, s, w, h };
                    if m.gpu.render(queue, enc, m.pause.as_mut().unwrap(), v) {
                        rendered += 1;
                        self.dirty = true;
                    }
                }
            }
        }
        self.stats.map_renders += rendered;
    }

    /// Drop the pause map's textures (call when the pause menu closes).
    pub fn release_pause_map(&mut self) {
        if let Some(m) = self.map.as_mut() {
            m.pause = None;
        }
    }

    fn build(&mut self) {
        self.list.clear();
        self.hits.clear();
        let mut p = Painter::new(&mut self.list, &mut self.text, &mut self.scratch);
        let mut ctx = screens::Ctx {
            vp: self.vp,
            t: (self.now - self.screen_t0) as f32,
            now: self.now,
            screen: self.screen,
            title_view: self.title_view,
            pause_tab: self.pause_tab,
            settings: &self.settings,
            loading: &self.loading,
            hud: &self.hud,
            mem: &mut self.hud_mem,
            hits: &mut self.hits,
            focus: self.focus,
            hover_t: &self.hover_t,
            scroll: self.scroll,
            ready: self.ready,
            reduced_motion: self.reduced_motion,
            map: self.map.as_ref().map(|m| screens::MapRefs {
                hud_image: m.hud_image,
                pause_image: m.pause_image,
                title_image: m.title_image,
                streets: &m.streets,
                pause_view: m.pause.as_ref().and_then(|t| t.current),
                bounds: m.gpu.bounds,
            }),
            pause_view: self.pause_view,
            animating: false,
            pause_pane: Rect::new(0.0, 0.0, 0.0, 0.0),
            fmt: &mut self.fmt,
            tags: &self.tags,
            avoid: &self.tag_avoid,
            chrome: self.hud_chrome,
        };
        screens::build(&mut p, &mut ctx);
        self.animating = ctx.animating;
        self.pause_pane = ctx.pause_pane;
    }

    fn focusables(&self) -> usize {
        self.hits.iter().filter(|h| h.focusable).count()
    }

    fn focused_action(&self) -> Option<(Action, Rect)> {
        let f = self.focus?;
        self.hits.iter().filter(|h| h.focusable).nth(f).map(|h| (h.action, h.rect))
    }

    fn handle_input(&mut self, input: &Input) -> Option<Event> {
        if self.screen == Screen::Hud || self.screen == Screen::Loading {
            return None;
        }
        // Hover follows the pointer onto focusable items.
        if let Some(m) = input.mouse {
            let mut k = 0;
            let mut over = None;
            for h in &self.hits {
                if h.focusable {
                    if h.rect.contains(m) {
                        over = Some(k);
                    }
                    k += 1;
                }
            }
            if over.is_some() && over != self.hover {
                self.cues |= cue::HOVER; // audio
            }
            self.hover = over;
        }
        let n = self.focusables();
        let k = input.keys;
        if k & (key::DOWN | key::TAB) != 0 && n > 0 {
            if let Some((Action::Slider(_) | Action::Volume | Action::Mix(_), _)) = self.focused_action() {
                if k & key::TAB == 0 {
                    // Ranges own their arrows in the title; down still moves.
                }
            }
            self.focus = Some(self.focus.map(|f| (f + 1) % n).unwrap_or(0));
            self.cues |= cue::MOVE; // audio
        }
        if k & (key::UP | key::SHIFT_TAB) != 0 && n > 0 {
            self.focus = Some(self.focus.map(|f| (f + n - 1) % n).unwrap_or(n - 1));
            self.cues |= cue::MOVE; // audio
        }
        if k & (key::LEFT | key::RIGHT) != 0 {
            let dir = if k & key::RIGHT != 0 { 1.0 } else { -1.0 };
            match self.focused_action() {
                Some((Action::Slider(key), _)) => {
                    self.cues |= cue::TOGGLE; // audio
                    self.settings.input.step(key, dir);
                    self.save_settings();
                    return Some(Event::SettingsChanged);
                }
                Some((Action::Volume, _)) => {
                    self.cues |= cue::TOGGLE; // audio
                    self.settings.audio_volume = (self.settings.audio_volume + dir * settings::AUDIO_VOLUME_STEP).clamp(0.0, 1.0);
                    self.settings.audio_volume = (self.settings.audio_volume * 100.0).round() / 100.0;
                    self.save_settings();
                    return Some(Event::SettingsChanged);
                }
                _ => {
                    // In the title's main view left/right move like up/down.
                    if n > 0 && matches!(self.screen, Screen::Title) {
                        self.focus = Some(self.focus.map(|f| (f + if dir > 0.0 { 1 } else { n - 1 }) % n).unwrap_or(0));
                    }
                }
            }
        }
        if k & key::ESCAPE != 0 {
            self.cues |= cue::BACK; // audio
            match self.screen {
                Screen::Title if self.title_view != TitleView::Main => self.set_title_view(TitleView::Main),
                Screen::Pause if self.pause_tab == PauseTab::Settings || self.pause_tab == PauseTab::Quit => self.set_pause_tab(PauseTab::Map),
                Screen::Pause => return Some(Event::Resume),
                _ => {}
            }
            return None;
        }
        if input.wheel != 0.0 {
            if self.screen == Screen::Pause && self.pause_tab == PauseTab::Map && input.mouse.is_some_and(|m| self.pause_pane.contains(m)) {
                self.zoom_pause(if input.wheel > 0.0 { theme::pause::ZOOM_STEP } else { 1.0 / theme::pause::ZOOM_STEP });
            } else {
                self.scroll = (self.scroll - input.wheel * 48.0).max(0.0);
            }
        }
        if input.held && (input.drag[0] != 0.0 || input.drag[1] != 0.0) && self.screen == Screen::Pause && self.pause_tab == PauseTab::Map {
            let (cx, cz, s) = self.pause_view_or_default();
            self.pause_view = Some((cx - input.drag[0] / s, cz - input.drag[1] / s, s));
        }
        let mut act: Option<(Action, Rect)> = None;
        if k & key::ENTER != 0 {
            act = self.focused_action();
        }
        if input.click {
            if let Some(m) = input.mouse {
                act = self.hits.iter().rev().find(|h| h.rect.contains(m)).map(|h| (h.action, h.rect));
            }
        }
        let (action, rect) = act?;
        // the activation's sound.
        self.cues |= match action {
            Action::Start | Action::Observe if self.ready => cue::START,
            Action::Start | Action::Observe | Action::MapPane => 0,
            Action::Back => cue::BACK,
            Action::TitleSettings | Action::TitleExit | Action::PauseSettings | Action::Quit | Action::Tab(_) => cue::OPEN,
            Action::Motion | Action::InvertY | Action::Slider(_) | Action::Preset(_) | Action::Option(_) | Action::World(_) | Action::Fps | Action::Adaptive | Action::Volume | Action::Mix(_) => cue::TOGGLE,
            _ => cue::CONFIRM,
        };
        self.activate(action, rect, input.mouse)
    }

    fn pause_view_or_default(&self) -> (f32, f32, f32) {
        self.pause_view.unwrap_or((self.hud.player[0], self.hud.player[1], theme::pause::OPEN_SCALE))
    }

    fn zoom_pause(&mut self, k: f32) {
        let (cx, cz, s) = self.pause_view_or_default();
        let min = self.pause_fit_scale();
        self.pause_view = Some((cx, cz, (s * k).clamp(min, theme::pause::MAX_SCALE)));
    }

    fn pause_fit_scale(&self) -> f32 {
        let b = self.map.as_ref().map(|m| m.gpu.bounds).unwrap_or([0.0, 0.0, 1.0, 1.0]);
        let pane = self.pause_pane;
        ((pane.w - 48.0).max(1.0) / (b[2] - b[0]).max(1.0)).min((pane.h - 48.0).max(1.0) / (b[3] - b[1]).max(1.0))
    }

    fn activate(&mut self, a: Action, rect: Rect, mouse: Option<[f32; 2]>) -> Option<Event> {
        self.dirty = true;
        match a {
            Action::Start if self.ready => {
                self.set_screen(Screen::Loading);
                return Some(Event::StartGame { observe: false });
            }
            Action::Observe if self.ready => {
                self.set_screen(Screen::Loading);
                return Some(Event::StartGame { observe: true });
            }
            Action::Start | Action::Observe => {}
            Action::TitleSettings => self.set_title_view(TitleView::Settings),
            Action::TitleExit => self.set_title_view(TitleView::Exit),
            Action::Back => match self.screen {
                Screen::Pause => self.set_pause_tab(PauseTab::Map),
                _ => self.set_title_view(TitleView::Main),
            },
            Action::Close => return Some(Event::Exit),
            Action::Motion => self.settings.background_motion = !self.settings.background_motion,
            Action::InvertY => {
                self.settings.input.invert_y = !self.settings.input.invert_y;
                self.save_settings();
                return Some(Event::SettingsChanged);
            }
            Action::Slider(key) => {
                if let Some(m) = mouse {
                    let r = settings::InputSettings::range(key);
                    let f = ((m[0] - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0) as f64;
                    let v = r.min + f * (r.max - r.min);
                    let v = ((v - r.min) / r.step).round() * r.step + r.min;
                    let v = (v * 1e6).round() / 1e6;
                    match key {
                        "lookX" => self.settings.input.look_x = v,
                        "lookY" => self.settings.input.look_y = v,
                        _ => self.settings.input.aim_multiplier = v,
                    }
                    self.save_settings();
                    return Some(Event::SettingsChanged);
                }
            }
            Action::Volume => {
                if let Some(m) = mouse {
                    let f = ((m[0] - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0) as f64;
                    self.settings.audio_volume = ((f / settings::AUDIO_VOLUME_STEP).round() * settings::AUDIO_VOLUME_STEP * 100.0).round() / 100.0;
                    self.save_settings();
                    return Some(Event::SettingsChanged);
                }
            }
            Action::Mix(key) => {
                if let Some(m) = mouse {
                    let f = ((m[0] - rect.x) / rect.w.max(1.0)).clamp(0.0, 1.0) as f64;
                    let v = ((f / settings::AUDIO_VOLUME_STEP).round() * settings::AUDIO_VOLUME_STEP * 100.0).round() / 100.0;
                    match key {
                        "sfx" => self.settings.audio_mix.sfx = v,
                        "music" => self.settings.audio_mix.music = v,
                        _ => self.settings.audio_mix.dialogue = v,
                    }
                    self.save_settings();
                    return Some(Event::SettingsChanged);
                }
            }
            Action::Preset(k) => {
                self.settings.performance.apply_preset(k);
                self.save_settings();
                return Some(Event::SettingsChanged);
            }
            Action::Option(k) => {
                self.settings.performance.cycle(k);
                self.save_settings();
                return Some(Event::SettingsChanged);
            }
            Action::World(k) => {
                self.settings.world.cycle(k);
                self.save_settings();
                return Some(Event::SettingsChanged);
            }
            Action::Fps => {
                self.settings.performance.show_fps = !self.settings.performance.show_fps;
                self.save_settings();
                return Some(Event::SettingsChanged);
            }
            Action::Adaptive => {}
            Action::Resume => return Some(Event::Resume),
            Action::PauseSettings => self.set_pause_tab(PauseTab::Settings),
            Action::Quit => self.set_pause_tab(PauseTab::Quit),
            Action::QuitConfirm => return Some(Event::QuitToTitle),
            Action::Tab(t) => self.set_pause_tab(t),
            Action::MapIn => self.zoom_pause(theme::pause::ZOOM_STEP),
            Action::MapOut => self.zoom_pause(1.0 / theme::pause::ZOOM_STEP),
            Action::MapPlayer => self.pause_view = Some((self.hud.player[0], self.hud.player[1], theme::pause::LOCATE_SCALE)),
            Action::MapFit => {
                let b = self.map.as_ref().map(|m| m.gpu.bounds).unwrap_or([0.0, 0.0, 1.0, 1.0]);
                self.pause_view = Some(((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5, self.pause_fit_scale()));
            }
            Action::MapPane => {}
        }
        None
    }
}
