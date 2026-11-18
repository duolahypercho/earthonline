//! The interface's design tokens as data: colour, type, spacing, radii and
//! timings, ported from the JS game's stylesheets and modules.
//!
//! Sources (the earlier browser version):
//!
//! * `src/ui/title-menu.css`   - the title menu (`--eo-menu-*`, rule sizes)
//! * `src/ui/hud-tokens.css`   - the HUD, pause map surfaces (`--hud-*`)
//! * `src/ui/player-hud.css`   - HUD layout rules
//! * `src/ui/pause-menu.css`   - the pause shell (`--pause-*`, rule sizes)
//! * `src/ui/performance-settings.css` - the graphics settings block
//! * `src/ui/city-map-render.js` - `MAP_PALETTE`, the pause map's palette
//! * `src/ui/hud-map.js`, `src/ui/pause-map.js`, `src/ui/title-menu.js` -
//!   the map camera and zoom constants
//!
//! Lengths keep the CSS expression (`clamp(...)`, `max(...)`, `vw`, `rem`),
//! resolved against the viewport at layout time exactly as the browser does
//! (1 rem = 16 px, 1 CSS px = 1 device px at a device pixel ratio of 1).

/// An sRGB colour with straight alpha, as CSS writes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r, g, b, a: 1.0 }
}
pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color { r, g, b, a }
}
pub const fn hex(v: u32) -> Color {
    Color { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8, a: 1.0 }
}
/// `#rrggbbaa`.
pub const fn hexa(v: u32) -> Color {
    Color { r: (v >> 24) as u8, g: (v >> 16) as u8, b: (v >> 8) as u8, a: (v & 0xff) as f32 / 255.0 }
}

pub const TRANSPARENT: Color = rgba(0, 0, 0, 0.0);
pub const BLACK: Color = rgb(0, 0, 0);
pub const WHITE: Color = rgb(255, 255, 255);

impl Color {
    pub fn alpha(self, k: f32) -> Color {
        Color { a: self.a * k, ..self }
    }
    pub fn with_a(self, a: f32) -> Color {
        Color { a, ..self }
    }
    /// `color-mix(in srgb, self p, other)`: straight sRGB channels and alpha
    /// mixed by weight, as the browser does for opaque inputs.
    pub fn mix(self, p: f32, other: Color) -> Color {
        let m = |a: u8, b: u8| (a as f32 * p + b as f32 * (1.0 - p)).round().clamp(0.0, 255.0) as u8;
        Color { r: m(self.r, other.r), g: m(self.g, other.g), b: m(self.b, other.b), a: self.a * p + other.a * (1.0 - p) }
    }
    /// `color-mix(in srgb, self p, transparent)`: the colour, alpha scaled.
    pub fn fade(self, p: f32) -> Color {
        self.alpha(p)
    }
    /// Premultiplied RGBA8 packed little-endian (r in the low byte), with an
    /// extra opacity factor (a CSS `opacity` on the element).
    pub fn premul(self, opacity: f32) -> u32 {
        let a = (self.a * opacity).clamp(0.0, 1.0);
        let c = |v: u8| ((v as f32 * a).round() as u32).min(255);
        c(self.r) | (c(self.g) << 8) | (c(self.b) << 16) | (((a * 255.0).round() as u32) << 24)
    }
    pub fn to_f32(self) -> [f32; 4] {
        [self.r as f32 / 255.0, self.g as f32 / 255.0, self.b as f32 / 255.0, self.a]
    }
}

/// A CSS length unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    Px,
    Rem,
    Em,
    Vw,
    Vh,
    Vmax,
    Dvh,
}

/// One CSS dimension, `12px`, `0.72vw`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct L(pub f32, pub Unit);

/// A CSS length expression as written in the source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Len {
    V(L),
    Max(L, L),
    Min(L, L),
    Clamp(L, L, L),
    /// `clamp(a, min(b, c), d)`.
    ClampMin(L, L, L, L),
}

/// The window, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub w: f32,
    pub h: f32,
}

pub const REM_PX: f32 = 16.0;

impl L {
    pub fn px(self, vp: Viewport, em: f32) -> f32 {
        let v = self.0;
        match self.1 {
            Unit::Px => v,
            Unit::Rem => v * REM_PX,
            Unit::Em => v * em,
            Unit::Vw => v * vp.w / 100.0,
            Unit::Vh | Unit::Dvh => v * vp.h / 100.0,
            Unit::Vmax => v * vp.w.max(vp.h) / 100.0,
        }
    }
    pub fn css(self) -> String {
        let u = match self.1 {
            Unit::Px => "px",
            Unit::Rem => "rem",
            Unit::Em => "em",
            Unit::Vw => "vw",
            Unit::Vh => "vh",
            Unit::Vmax => "vmax",
            Unit::Dvh => "dvh",
        };
        format!("{}{u}", self.0)
    }
}

impl Len {
    pub fn px(self, vp: Viewport, em: f32) -> f32 {
        match self {
            Len::V(a) => a.px(vp, em),
            Len::Max(a, b) => a.px(vp, em).max(b.px(vp, em)),
            Len::Min(a, b) => a.px(vp, em).min(b.px(vp, em)),
            Len::Clamp(lo, v, hi) => v.px(vp, em).min(hi.px(vp, em)).max(lo.px(vp, em)),
            Len::ClampMin(lo, a, b, hi) => a.px(vp, em).min(b.px(vp, em)).min(hi.px(vp, em)).max(lo.px(vp, em)),
        }
    }
    /// The CSS source form (the token test compares it with the stylesheet).
    pub fn css(self) -> String {
        match self {
            Len::V(a) => a.css(),
            Len::Max(a, b) => format!("max({}, {})", a.css(), b.css()),
            Len::Min(a, b) => format!("min({}, {})", a.css(), b.css()),
            Len::Clamp(a, b, c) => format!("clamp({}, {}, {})", a.css(), b.css(), c.css()),
            Len::ClampMin(a, b, c, d) => format!("clamp({}, min({}, {}), {})", a.css(), b.css(), c.css(), d.css()),
        }
    }
}

pub const fn px(v: f32) -> L {
    L(v, Unit::Px)
}
pub const fn rem(v: f32) -> L {
    L(v, Unit::Rem)
}
pub const fn em(v: f32) -> L {
    L(v, Unit::Em)
}
pub const fn vw(v: f32) -> L {
    L(v, Unit::Vw)
}
pub const fn vh(v: f32) -> L {
    L(v, Unit::Vh)
}
pub const fn vmax(v: f32) -> L {
    L(v, Unit::Vmax)
}

/// `cubic-bezier(x1, y1, x2, y2)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bezier(pub [f32; 4]);

pub const EASE: Bezier = Bezier([0.25, 0.1, 0.25, 1.0]);
pub const EASE_IN_OUT: Bezier = Bezier([0.42, 0.0, 0.58, 1.0]);
pub const LINEAR: Bezier = Bezier([0.0, 0.0, 1.0, 1.0]);

impl Bezier {
    /// The eased progress at time fraction `t` (0..1), solved as the browser does.
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let [x1, y1, x2, y2] = self.0;
        let bez = |a: f32, b: f32, s: f32| 3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s;
        // Newton on x(s) = t, then bisection as a fallback.
        let mut s = t;
        for _ in 0..8 {
            let x = bez(x1, x2, s) - t;
            let dx = 3.0 * x1 * (1.0 - s) * (1.0 - s) + 6.0 * (x2 - x1) * s * (1.0 - s) + 3.0 * (1.0 - x2) * s * s;
            if x.abs() < 1e-5 {
                return bez(y1, y2, s);
            }
            if dx.abs() < 1e-6 {
                break;
            }
            s = (s - x / dx).clamp(0.0, 1.0);
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            s = 0.5 * (lo + hi);
            if bez(x1, x2, s) < t {
                lo = s;
            } else {
                hi = s;
            }
        }
        bez(y1, y2, s)
    }
}

/// A CSS font stack, first family first.
pub type FontStack = &'static [&'static str];

/// Title menu, `src/ui/title-menu.css` `:root` and rules.
pub mod title {
    use super::*;
    pub const INK: Color = hex(0xedf5fb);
    pub const MUTED: Color = rgba(225, 235, 243, 0.68);
    pub const FAINT: Color = rgba(225, 235, 243, 0.42);
    pub const ACCENT: Color = hex(0xe2ad61);
    pub const LINE: Color = rgba(226, 173, 97, 0.42);
    pub const NIGHT: Color = hex(0x05080c);
    pub const SURFACE: Color = rgba(5, 10, 17, 0.78);
    pub const SURFACE_STRONG: Color = rgba(5, 10, 17, 0.92);
    pub const DISPLAY: FontStack =
        &["Bahnschrift", "DIN Alternate", "Avenir Next Condensed", "Roboto Condensed", "Arial Narrow", "Helvetica Neue", "sans-serif"];
    pub const SANS: FontStack = &["Segoe UI Variable Text", "Segoe UI", "Avenir Next", "Helvetica Neue", "sans-serif"];
    pub const MENU_EASE: Bezier = Bezier([0.16, 1.0, 0.3, 1.0]);

    // Backdrop (`.eo-menu__plate`, `.eo-menu__glow*`, `.eo-menu__bars i`).
    pub const PLATE_W: Len = Len::V(vmax(176.0));
    pub const PLATE_PERSPECTIVE_PX: f32 = 1400.0;
    pub const PLATE_FADE_MS: f32 = 2400.0;
    pub const PLATE_DRIFT_S: f32 = 120.0;
    /// `@keyframes eo-plate-drift`: translate (fractions of the plate box), rotateX, rotateZ (degrees).
    pub const PLATE_DRIFT_FROM: [f32; 4] = [-0.03, 0.07, 54.0, -19.0];
    pub const PLATE_DRIFT_TO: [f32; 4] = [0.03, 0.03, 51.0, -11.0];
    pub const GLOW_SIZE: Len = Len::V(vmax(110.0));
    pub const GLOW_WARM: [Color; 2] = [rgba(226, 150, 82, 0.26), rgba(226, 150, 82, 0.08)];
    pub const GLOW_COOL: [Color; 2] = [rgba(76, 128, 178, 0.22), rgba(76, 128, 178, 0.06)];
    pub const GLOW_DRIFT_WARM_S: f32 = 46.0;
    pub const GLOW_DRIFT_COOL_S: f32 = 58.0;
    pub const BAR_H: Len = Len::Clamp(rem(1.25), vh(5.5), rem(3.75));

    // Type scale.
    pub const EYEBROW_SIZE: Len = Len::Clamp(rem(0.7), vw(0.95), rem(0.95));
    pub const EYEBROW_TRACKING: f32 = 0.42;
    pub const WORDMARK_SIZE: Len = Len::Clamp(rem(3.0), vw(8.6), rem(9.4));
    pub const WORDMARK_STRETCH: f32 = 84.0;
    pub const RULE_W: Len = Len::Clamp(rem(4.0), vw(9.0), rem(9.0));
    pub const ACTION_SIZE: Len = Len::Clamp(rem(1.02), vw(1.42), rem(1.42));
    pub const ACTION_TRACKING: f32 = 0.2;
    pub const PRIMARY_SIZE: Len = Len::Clamp(rem(1.7), vw(2.9), rem(3.0));
    pub const PRIMARY_TRACKING: f32 = 0.1;
    pub const HEADING_SIZE: Len = Len::Clamp(rem(1.45), vw(2.0), rem(2.0));
    pub const HEADING_TRACKING: f32 = 0.14;
    pub const LOADING_PHASE_SIZE: Len = Len::Clamp(rem(0.72), vw(0.9), rem(0.88));
    pub const LOADING_PHASE_TRACKING: f32 = 0.3;
    /// `.eo-menu__rail { width: min(100%, 34rem) }`.
    pub const RAIL_MAX: L = rem(34.0);
    pub const SETTINGS_PANEL_PAD: Len = Len::Clamp(rem(1.5), vw(3.0), rem(2.4));

    // Motion (`animation` / `transition` durations and delays, ms).
    pub const LETTER_MS: f32 = 1100.0;
    pub const LETTER_DELAY_MS: f32 = 140.0;
    pub const LETTER_STAGGER_MS: f32 = 62.0;
    pub const ITEM_MS: f32 = 900.0;
    pub const ITEM_DELAY_MS: f32 = 1080.0;
    pub const ITEM_STAGGER_MS: f32 = 80.0;
    pub const HOVER_MS: f32 = 420.0;
    pub const LINE_LIGHT_MS: f32 = 2600.0;
    pub const LEAVE_MS: f32 = 900.0;
}

/// HUD tokens, `src/ui/hud-tokens.css` `:root`.
pub mod hud {
    use super::*;
    pub const FONT_DISPLAY: FontStack = &["Avenir Next Condensed", "Arial Narrow", "Helvetica Neue", "sans-serif"];
    pub const FONT_SANS: FontStack = &["Avenir Next", "Helvetica Neue", "sans-serif"];
    pub const FONT_MONO: FontStack = &["ui-monospace", "SFMono-Regular", "Consolas", "monospace"];

    pub const FS_LABEL: Len = Len::Max(px(12.0), vw(0.72));
    pub const FS_BODY: Len = Len::Max(px(13.0), vw(0.86));
    pub const FS_PROMPT: Len = Len::Max(px(14.0), vw(0.95));
    pub const FS_VALUE: Len = Len::Max(px(18.0), vw(1.35));
    pub const FS_NUMERAL: Len = Len::Max(px(30.0), vw(2.3));
    pub const TRACKING: Len = Len::V(em(0.14));

    pub const INK: Color = hex(0xf2f0e8);
    pub const INK_MUTED: Color = rgba(242, 240, 232, 0.76);
    pub const INK_FAINT: Color = rgba(242, 240, 232, 0.42);
    pub const ACCENT: Color = hex(0xf2bb52);
    pub const ACCENT_INK: Color = hex(0x1a1408);
    pub const DANGER: Color = hex(0xff8f75);
    pub const OK: Color = hex(0x8fe0b0);
    pub const ARMOUR: Color = hex(0x8fc9ff);
    /// Noon sky (`--hud-sky-r/g/b`), the runtime writes the live one.
    pub const SKY: Color = rgb(120, 160, 200);
    pub const PLATE_BASE: Color = rgb(8, 11, 14);
    pub const LINE: Color = rgba(242, 240, 232, 0.16);
    pub const RADIUS: f32 = 4.0;
    pub const BLUR_PX: f32 = 10.0;
    pub const MOTION_MS: f32 = 180.0;
    pub const MOTION_FAST_MS: f32 = 120.0;
    pub const EASE: Bezier = Bezier([0.16, 1.0, 0.3, 1.0]);
    pub const EASE_OUT: Bezier = Bezier([0.2, 0.8, 0.2, 1.0]);
    pub const COLUMN_W: Len = Len::Min(px(320.0), vw(30.0));
    pub const GAP: Len = Len::Clamp(px(8.0), vw(0.7), px(14.0));
    pub const PAD: Len = Len::Clamp(px(8.0), vw(0.6), px(12.0));

    pub const MAP_W: Len = Len::ClampMin(px(160.0), vw(17.0), vh(26.0), px(330.0));
    /// `--hud-map-h: calc(var(--hud-map-w) * 0.66)`.
    pub const MAP_H_RATIO: f32 = 0.66;
    pub const STAR: Len = Len::Clamp(px(20.0), vw(1.2), px(32.0));
    pub const BAR_H: Len = Len::Clamp(px(7.0), vw(0.56), px(12.0));
    pub const RADAR_FRAME: Color = rgba(4, 6, 8, 0.94);
    pub const RADAR_RIM: Color = rgba(242, 240, 232, 0.22);
    pub const HEALTH_BAR: Color = hex(0x5ccf72);
    pub const ARMOUR_BAR: Color = hex(0x4aa8ff);
    pub const MAP_OPACITY: f32 = 0.93;
    pub const MAP_MASSING: f32 = 0.0;

    pub const MAP_GROUND: Color = rgb(15, 17, 20);
    pub const MAP_ROAD: Color = rgb(98, 104, 114);
    pub const MAP_ROAD_MINOR: Color = rgb(74, 79, 88);
    pub const MAP_SIDEWALK: Color = rgb(38, 41, 47);
    pub const MAP_PARK: Color = rgb(36, 50, 43);
    pub const MAP_WATER: Color = rgb(28, 42, 58);
    pub const MAP_EDGE: Color = rgba(0, 0, 0, 0.28);
    pub const MAP_OUTLINE: Color = rgb(14, 16, 19);
    pub const MAP_ROOF: Color = rgb(120, 125, 133);
    pub const MAP_ROUTE: Color = hex(0xf2bb52);
    pub const WAYPOINT: Color = hex(0xc97bff);
    pub const POLICE_RED: Color = hex(0xff4d4d);
    pub const POLICE_BLUE: Color = hex(0x4d8dff);
    pub const MAP_COMPASS_INK: Color = rgba(242, 240, 232, 0.8);

    /// `--hud-plate`: the plate base with 8% of the sky mixed in.
    pub fn plate(sky: Color) -> Color {
        PLATE_BASE.mix(0.92, sky)
    }
    /// `--hud-surface`: the plate at 78%.
    pub fn surface(sky: Color) -> Color {
        plate(sky).fade(0.78)
    }
    /// `--hud-line-accent`.
    pub fn line_accent() -> Color {
        ACCENT.fade(0.72)
    }
    /// `--hud-shadow: 0 8px 24px rgba(3, 5, 6, 0.28)`.
    pub const SHADOW: (f32, f32, Color) = (8.0, 24.0, rgba(3, 5, 6, 0.28));

    // Vehicle instrument cluster (`--hud-veh-*`; layout in vehicle-hud.css).
    pub const VEH_SCRIM: Color = rgba(8, 11, 14, 0.9);
    pub const VEH_SCRIM_FADE: Color = rgba(8, 11, 14, 0.52);
    pub const VEH_INK_LABEL: Color = hex(0xd1d0ca);
    pub const WEIGHT_HERO: f32 = 250.0;
    pub const FS_CLUSTER: Len = Len::Max(px(62.0), vw(4.9));

    // Map camera (`src/ui/hud-map.js`).
    pub const CAM_PITCH_DEG: f32 = 45.0;
    pub const CAM_HEIGHT_M: f32 = 120.0;
    pub const CAM_BACK_M: f32 = 81.0;
    pub const FOCAL_RATIO: f32 = 1.245;

    // Layout rules (`src/ui/player-hud.css`).
    pub const STREET_SIZE: Len = Len::Clamp(px(11.0), vw(0.78), px(15.0));
    /// The vitals readout appears under this health fraction (`HEALTH_READOUT_AT`).
    pub const HEALTH_READOUT_AT: f32 = 0.55;
    pub const WANTED_STARS: u8 = 5;
}

/// Pause shell, `src/ui/pause-menu.css`.
pub mod pause {
    use super::*;
    pub const INK: Color = hex(0xedf5fb);
    pub const MUTED: Color = hex(0xacbbc8);
    pub const ACCENT: Color = hex(0xe2ad61);
    pub const LINE: Color = rgba(226, 173, 97, 0.3);
    pub const BACKGROUND: Color = hex(0x060b12);
    pub const FONT: FontStack = &["Avenir Next", "Helvetica Neue", "sans-serif"];
    pub const FONT_CONDENSED: FontStack = &["Avenir Next Condensed", "Arial Narrow", "sans-serif"];
    pub const OPEN_MS: f32 = 160.0;
    pub const GAP: Len = Len::Clamp(px(12.0), vh(1.7), px(24.0));
    pub const TITLE_SIZE: Len = Len::Clamp(px(13.0), vw(1.1), px(20.0));
    pub const TITLE_TRACKING: f32 = 0.25;
    pub const BRAND_SIZE: Len = Len::Clamp(px(26.0), vw(2.6), px(52.0));
    pub const BRAND_TRACKING: f32 = 0.32;
    pub const BRAND_INK: Color = hex(0xf1e3cf);
    pub const BRAND_RULE: Color = hex(0xc9673f);
    pub const BRAND_SHADOW: Color = hex(0x7a3a22);
    pub const TABS_GAP: Len = Len::Clamp(px(16.0), vw(4.0), px(70.0));
    pub const TAB_PAD_X: Len = Len::Clamp(px(4.0), vw(1.3), px(24.0));
    pub const TAB_SIZE: Len = Len::Clamp(px(12.0), vw(1.0), px(18.0));
    pub const TAB_TRACKING: f32 = 0.2;
    pub const PAGES_BORDER: Color = rgba(196, 216, 235, 0.23);
    pub const PAGES_BG: Color = rgba(5, 15, 24, 0.92);
    pub const RAIL_W: Len = Len::Clamp(px(190.0), vw(14.0), px(270.0));
    pub const LOCATION_BG: Color = hexa(0x0b151fe6);
    pub const LOCATION_SIZE: Len = Len::Clamp(px(14.0), vw(1.25), px(23.0));
    pub const TILE_SIZE: Len = Len::Clamp(px(15.0), vw(1.15), px(22.0));
    pub const TILE_BORDER: Color = rgba(196, 216, 235, 0.18);
    pub const TILE_BG: [Color; 2] = [rgba(14, 26, 37, 0.96), rgba(8, 16, 24, 0.9)];
    pub const TILE_PRIMARY_BG: [Color; 2] = [rgba(226, 173, 97, 0.2), rgba(8, 16, 24, 0.9)];
    pub const TILE_SMALL: Color = hex(0x9fb0bd);
    pub const SHEET_PAD: Len = Len::Clamp(px(20.0), vw(2.5), px(45.0));
    pub const SHEET_H3: Len = Len::Clamp(px(25.0), vw(2.5), px(42.0));
    pub const KBD_BORDER: Color = hex(0x65727c);
    pub const LEGEND_BG: Color = hex(0x0a131b);
    pub const TOOL_BG: Color = hex(0x091722);
    pub const TOOL_BORDER: Color = hex(0x526170);
    pub const NOTE_BG: Color = hexa(0x0f1114df);
    pub const ITEM_BG: Color = hex(0x152633);
    pub const ITEM_BORDER: Color = hex(0x435665);
    pub const SLIDER_RULE: Color = hex(0x293d4d);
    pub const SHADE_WARM: Color = rgba(226, 150, 82, 0.12);
    pub const SHADE_COOL: Color = rgba(76, 128, 178, 0.12);
    pub const SHADE_BASE: [Color; 2] = [rgba(3, 9, 16, 0.7), rgba(3, 9, 16, 0.76)];

    // Map zoom (`src/ui/pause-map.js`), CSS px per metre.
    pub const MAX_SCALE: f32 = 6.0;
    pub const OPEN_SCALE: f32 = 0.55;
    pub const LOCATE_SCALE: f32 = 1.25;
    pub const ZOOM_STEP: f32 = 1.6;
}

/// The graphics settings block, `src/ui/performance-settings.css`.
pub mod perf {
    use super::*;
    pub const INK: Color = hex(0xedf2f5);
    pub const RULE: Color = hex(0x435665);
    pub const BUTTON_BORDER: Color = hex(0x65727c);
    pub const BUTTON_BG: Color = hex(0x152633);
    pub const COPY: Color = hex(0xc1ccd4);
    pub const H3_SIZE: f32 = 20.0;
    pub const BUTTON_SIZE: f32 = 13.0;
    pub const SMALL_SIZE: f32 = 11.0;
    pub const FPS_BG: Color = hexa(0x09121bdd);
    pub const FPS_INK: Color = hex(0xeef4f6);
}

/// `MAP_PALETTE` and the road table, `src/ui/city-map-render.js` and
/// `src/ui/city-map-decode.js`: the pause map's cartography.
pub mod map {
    use super::*;
    pub const WATER_DEEP: Color = hex(0x0a1622);
    pub const WATER: Color = hex(0x0f2131);
    pub const SHORE_GLOW: Color = rgba(86, 128, 160, 0.20);
    pub const SHORE_LINE: Color = rgba(150, 178, 198, 0.30);
    pub const LAND: Color = hex(0x2a3038);
    pub const PARK: Color = hex(0x2c3b30);
    pub const WOOD: Color = hex(0x27352b);
    pub const SAND: Color = hex(0x4b4637);
    pub const BUILDING: Color = hex(0x363d47);
    pub const BUILDING_EDGE: Color = rgba(18, 22, 27, 0.55);
    pub const PIER: Color = hex(0x323941);
    pub const GRID: Color = rgba(160, 182, 204, 0.045);
    pub const ROADS: [Color; 6] = [hex(0x3f464f), hex(0x4c535d), hex(0x646b75), hex(0x7b828c), hex(0x959ba4), hex(0xb8bcc2)];
    pub const FREEWAY_CASING: Color = hex(0x1b2026);
    pub const ROUTE: Color = hex(0xc03cff);
    pub const OBJECTIVE: Color = hex(0xf2c230);
    pub const PLAYER: Color = hex(0xffffff);
    /// `CITY_ROAD_WIDTH_M`: path, alley, residential, collector, arterial, freeway.
    pub const ROAD_WIDTH_M: [f32; 6] = [2.5, 6.0, 11.0, 14.0, 18.0, 24.0];
    /// Pixel floor per class when zoomed out (`floorPx` in the renderer).
    pub const ROAD_FLOOR_PX: [f32; 6] = [0.6, 0.5, 0.7, 1.0, 1.5, 2.4];
    /// The title backdrop bitmap (`BACKDROP` in title-menu.js).
    pub const BACKDROP_W: u32 = 3200;
    pub const BACKDROP_H: u32 = 2000;
    pub const BACKDROP_METRES_WIDE: f32 = 3600.0;
}
