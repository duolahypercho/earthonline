//! Screen layouts, ported rule by rule from the JS stylesheets at the
//! viewport's CSS-pixel size. Each section names the selectors it follows.

use crate::draw::{Dir, ImageId, Painter, Rect};
use crate::map::{MapView, StreetIndex};
use crate::settings::{self, Settings};
use crate::text::{Stack, TextStyle};
use crate::theme::{self, Bezier, Color, Len, Viewport, hud, pause, perf, rgba, title};
use crate::{Action, Hit, HudMemory, HudMode, HudState, Loading, NameTag, PauseTab, Screen, TitleView, WeaponKind};
use std::fmt::Write as _;

pub(crate) struct MapRefs<'a> {
    pub hud_image: ImageId,
    pub pause_image: ImageId,
    pub title_image: ImageId,
    pub streets: &'a StreetIndex,
    pub pause_view: Option<MapView>,
    pub bounds: [f32; 4],
}

pub(crate) struct Ctx<'a> {
    pub vp: Viewport,
    /// Seconds since the screen opened.
    pub t: f32,
    pub now: f64,
    pub screen: Screen,
    pub title_view: TitleView,
    pub pause_tab: PauseTab,
    pub settings: &'a Settings,
    pub loading: &'a Loading,
    pub hud: &'a HudState,
    pub mem: &'a mut HudMemory,
    pub hits: &'a mut Vec<Hit>,
    pub focus: Option<usize>,
    pub hover_t: &'a [f32; 16],
    pub scroll: f32,
    pub ready: bool,
    pub reduced_motion: bool,
    pub map: Option<MapRefs<'a>>,
    pub pause_view: Option<(f32, f32, f32)>,
    /// Set while anything on screen is still moving.
    pub animating: bool,
    /// Where the pause map's image goes (read back by the Ui).
    pub pause_pane: Rect,
    pub fmt: &'a mut String,
    /// The other players' name tags, and whether the HUD's plates are drawn.
    pub tags: &'a [NameTag],
    pub avoid: &'a [[f32; 4]],
    pub chrome: bool,
}

impl Ctx<'_> {
    fn len(&self, l: Len) -> f32 {
        l.px(self.vp, theme::REM_PX)
    }
    /// Record a hit region; returns the focus index of a focusable one.
    fn hit(&mut self, rect: Rect, action: Action, focusable: bool) -> Option<usize> {
        let idx = if focusable { Some(self.hits.iter().filter(|h| h.focusable).count()) } else { None };
        self.hits.push(Hit { rect, action, focusable });
        idx
    }
    /// Eased hover/focus amount of a focusable item.
    fn hover(&self, idx: Option<usize>) -> f32 {
        idx.and_then(|i| self.hover_t.get(i).copied()).map(|v| title::MENU_EASE.at(v)).unwrap_or(0.0)
    }
    fn focused(&self, idx: Option<usize>) -> bool {
        idx.is_some() && idx == self.focus
    }
    /// Progress of a CSS animation with a delay (0..1), eased; flags motion.
    fn anim(&mut self, delay_ms: f32, dur_ms: f32, ease: Bezier) -> f32 {
        if self.reduced_motion {
            return 1.0;
        }
        let x = (self.t * 1000.0 - delay_ms) / dur_ms;
        if x < 1.0 {
            self.animating = true;
        }
        ease.at(x.clamp(0.0, 1.0))
    }
}

/// The radar plate's CSS size (`--hud-map-w`, `--hud-map-h`).
pub(crate) fn hud_map_size(vp: Viewport) -> (f32, f32) {
    let w = hud::MAP_W.px(vp, theme::REM_PX);
    (w, w * hud::MAP_H_RATIO)
}

pub(crate) fn build(p: &mut Painter, c: &mut Ctx) {
    match c.screen {
        Screen::Title | Screen::Loading => title_screen(p, c),
        Screen::Pause => pause_screen(p, c),
        Screen::Hud => {
            name_tags(p, c);
            if c.chrome {
                hud_screen(p, c);
            }
        }
    }
}

/// A name tag's plate: the HUD's surface ink at 70 % (legible over a bright
/// sky or a white facade), a 1 px light hairline round it, the name in the
/// HUD's ink and the second line a little dimmer.
const TAG_PLATE: Color = rgba(6, 10, 16, 0.70);
const TAG_EDGE: Color = rgba(237, 245, 251, 0.34);
const TAG_LEADER: Color = rgba(237, 245, 251, 0.78);
const TAG_DETAIL: Color = rgba(237, 245, 251, 0.86);

/// One tag's plate as laid out.
struct TagBox {
    rect: Rect,
    name_size: f32,
    detail_size: f32,
}

fn radius_of(b: &TagBox) -> f32 {
    (b.name_size * 0.62).min(b.rect.h * 0.5)
}

fn tag_box(p: &mut Painter, t: &NameTag) -> TagBox {
    let fs = t.size.clamp(7.0, 40.0);
    let ds = t.detail_size.clamp(7.0, 40.0);
    let ts = st(Stack::HudSans, 600, fs).tracking(0.02);
    let mut w = p.text_width(&t.name, ts);
    if let Some(d) = &t.detail {
        w = w.max(p.text_width(d, st(Stack::HudSans, 500, ds).tracking(0.03)));
    }
    let (padx, pady) = (fs * 0.6, fs * 0.28);
    let h = fs * 1.2 + if t.detail.is_some() { ds * 1.18 } else { 0.0 } + 2.0 * pady;
    TagBox { rect: Rect::new(t.x - w * 0.5 - padx, 0.0, w + 2.0 * padx, h), name_size: fs, detail_size: ds }
}

/// Name tags over the other players. Laid out nearest first: each plate
/// sits over its head point with a small pointer down to it; one that would
/// cover a plate already placed or someone else's face (`Ctx::avoid`) tries
/// the places beside and above (up to three plates up), nearest first, and
/// draws a 1 px leader line down to its head; with no place clear of the
/// faces it takes one clear of the other plates; with none of those either
/// it is left out. Drawn farthest first, so the nearest is on top.
fn name_tags(p: &mut Painter, c: &mut Ctx) {
    let mut order: Vec<usize> = (0..c.tags.len()).filter(|&i| c.tags[i].alpha > 0.01 && c.tags[i].x.is_finite() && c.tags[i].y.is_finite()).collect();
    order.sort_by(|&a, &b| c.tags[a].depth.total_cmp(&c.tags[b].depth));
    let mut placed: Vec<(usize, TagBox, bool)> = vec![];
    let gap = 2.0;
    let over = |a: &Rect, b: [f32; 4]| a.x < b[2] + gap && b[0] < a.x + a.w + gap && a.y < b[3] + gap && b[1] < a.y + a.h + gap;
    for i in order {
        let t = &c.tags[i];
        let mut b = tag_box(p, t);
        let point = b.name_size * 0.34;
        let home = Rect::new(b.rect.x, t.y - point - b.rect.h, b.rect.w, b.rect.h);
        let (w, h) = (b.rect.w, b.rect.h + gap);
        let mut cands = vec![(0.0, 0.0)];
        for k in 0..4 {
            let up = -(k as f32) * h;
            for dx in [0.0, 0.62 * w + gap, -(0.62 * w + gap)] {
                if k > 0 || dx != 0.0 {
                    cands.push((dx, up));
                }
            }
        }
        cands.sort_by(|a, b| (a.0.hypot(a.1)).total_cmp(&b.0.hypot(b.1)));
        let free_of_tags = |r: &Rect| !placed.iter().any(|(_, q, _)| over(r, [q.rect.x, q.rect.y, q.rect.right(), q.rect.bottom()]));
        let free_of_faces = |r: &Rect| !c.avoid.iter().any(|f| *f != t.head && over(r, *f));
        let at = |d: &(f32, f32)| home.translate(d.0, d.1);
        let Some(d) = cands.iter().find(|d| free_of_tags(&at(d)) && free_of_faces(&at(d))).or_else(|| cands.iter().find(|d| free_of_tags(&at(d)))) else { continue };
        b.rect = at(d);
        placed.push((i, b, d.0 != 0.0 || d.1 != 0.0));
    }
    for (i, b, moved) in placed.iter().rev() {
        let t = &c.tags[*i];
        let r = b.rect;
        p.opacity = t.alpha.min(1.0);
        let point = b.name_size * 0.34;
        if *moved {
            // The leader: from the plate's lower edge (above the head where it can) down to the point over the head.
            let lx = t.x.clamp(r.x + radius_of(b), r.right() - radius_of(b));
            p.polyline(&[[lx, r.bottom()], [t.x, t.y - 1.0]], 1.0, false, TAG_LEADER);
            p.ellipse(t.x, t.y, 1.6, 1.6, 0.0, TAG_LEADER);
        } else {
            p.polygon(&[[t.x - point, r.bottom() - 0.5], [t.x + point, r.bottom() - 0.5], [t.x, r.bottom() + point]], TAG_PLATE);
        }
        let radius = radius_of(b);
        p.rrect(r, radius, TAG_PLATE);
        p.rrect_stroke(r, radius, 1.0, TAG_EDGE);
        let pady = b.name_size * 0.28;
        let ts = st(Stack::HudSans, 600, b.name_size).tracking(0.02);
        let nw = p.text_width(&t.name, ts);
        let base = p.text.baseline(ts, r.y + pady, 1.2);
        let cx = r.x + r.w * 0.5;
        p.text(cx - nw * 0.5, base, &t.name, ts, hud::INK);
        if let Some(d) = &t.detail {
            let ds = st(Stack::HudSans, 500, b.detail_size).tracking(0.03);
            let dw = p.text_width(d, ds);
            let base = p.text.baseline(ds, r.y + pady + b.name_size * 1.2, 1.18);
            p.text(cx - dw * 0.5, base, d, ds, TAG_DETAIL);
        }
        p.opacity = 1.0;
    }
}

fn st(stack: Stack, weight: u16, size: f32) -> TextStyle {
    TextStyle::new(stack, weight, size)
}

/// Mix two colours (straight sRGB) by `k` toward `b`.
fn lerp_c(a: Color, b: Color, k: f32) -> Color {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: a.a + (b.a - a.a) * k }
}

/// Wrap `text` into lines no wider than `w`, drawing each; returns the height.
fn paragraph(p: &mut Painter, x: f32, top: f32, w: f32, text: &str, style: TextStyle, lh: f32, color: Color) -> f32 {
    let line_h = style.size * lh;
    let space = p.text_width(" ", style);
    let mut y = top;
    let mut line_start = 0usize;
    let mut line_w = 0.0f32;
    let mut last_end = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut lines = 0;
    let flush = |p: &mut Painter, s: usize, e: usize, y: f32| {
        let base = p.text.baseline(style, y, lh);
        p.text(x, base, &text[s..e], style, color);
    };
    while i <= bytes.len() {
        let end = text[i..].find(' ').map(|k| i + k).unwrap_or(bytes.len());
        let word = &text[i..end];
        let ww = p.text_width(word, style);
        let add = if line_w > 0.0 { space + ww } else { ww };
        if line_w > 0.0 && line_w + add > w {
            flush(p, line_start, last_end, y);
            lines += 1;
            y += line_h;
            line_start = i;
            line_w = ww;
        } else {
            line_w += add;
        }
        last_end = end;
        i = end + 1;
    }
    if line_w > 0.0 {
        flush(p, line_start, last_end, y);
        lines += 1;
    }
    lines as f32 * line_h
}

// ---------------------------------------------------------------------------
// Title (src/ui/title-menu.css, title-menu.js)
// ---------------------------------------------------------------------------

/// `animation: ... infinite alternate` progress with an ease-in-out curve.
fn alternate(t: f32, period_s: f32, reverse: bool) -> f32 {
    let x = t / period_s;
    let k = x.floor() as i64;
    let mut f = x - x.floor();
    if (k % 2 == 1) != reverse {
        f = 1.0 - f;
    }
    theme::EASE_IN_OUT.at(f)
}

fn title_screen(p: &mut Painter, c: &mut Ctx) {
    let (w, h) = (c.vp.w, c.vp.h);
    let t = c.t;
    let motion_t = if c.settings.background_motion && !c.reduced_motion { t } else { 0.0 };
    if c.settings.background_motion {
        c.animating = true;
    }
    let vmax = w.max(h) / 100.0;
    // .eo-menu: night ground.
    p.rect(Rect::new(0.0, 0.0, w, h), title::NIGHT);

    // .eo-menu__plate: the city map on a tilted, slowly turning table.
    let title_image = c.map.as_ref().map(|m| m.title_image).unwrap_or(0);
    if title_image != 0 {
        let pw = c.len(title::PLATE_W);
        let ph = pw * 10.0 / 16.0;
        let k = alternate(motion_t, title::PLATE_DRIFT_S, false);
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        let (from, to) = (title::PLATE_DRIFT_FROM, title::PLATE_DRIFT_TO);
        let (tx, ty, rx, rz) = (lerp(from[0], to[0]) * pw, lerp(from[1], to[1]) * ph, lerp(from[2], to[2]).to_radians(), lerp(from[3], to[3]).to_radians());
        let d = title::PLATE_PERSPECTIVE_PX;
        let origin = [w * 0.5, h * 0.24];
        let centre = [w * 0.5, h * 0.5];
        let project = |lx: f32, ly: f32| -> ([f32; 2], f32) {
            let (x1, y1) = (lx * rz.cos() - ly * rz.sin(), lx * rz.sin() + ly * rz.cos());
            let (x2, y2, z2) = (x1, y1 * rx.cos(), y1 * rx.sin());
            let (x3, y3, z3) = (centre[0] + x2 + tx, centre[1] + y2 + ty, z2);
            let s = d / (d - z3);
            ([origin[0] + (x3 - origin[0]) * s, origin[1] + (y3 - origin[1]) * s], s)
        };
        let corners = [(-pw / 2.0, -ph / 2.0), (pw / 2.0, -ph / 2.0), (pw / 2.0, ph / 2.0), (-pw / 2.0, ph / 2.0)];
        let mut pos = [[0.0f32; 2]; 4];
        let mut q = [0.0f32; 4];
        for (i, (lx, ly)) in corners.iter().enumerate() {
            let (pt, s) = project(*lx, *ly);
            pos[i] = pt;
            q[i] = s;
        }
        let fade = c.anim(0.0, title::PLATE_FADE_MS, theme::EASE);
        p.image_projective(pos, [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], q, title_image, fade);
        // .eo-menu__here: "you are here" at the backdrop's centre (the spawn).
        let (here, s) = project(0.0, 0.0);
        let dot = 0.55 * vmax * s.min(1.6);
        p.radial(here[0], here[1], dot * 4.0, dot * 4.0, &[(0.0, rgba(226, 173, 97, 0.55 * fade)), (0.45, rgba(226, 173, 97, 0.2 * fade)), (1.0, rgba(226, 173, 97, 0.0))]);
        p.ellipse(here[0], here[1], dot, dot, 0.0, title::ACCENT.alpha(fade));
        let ring_k = if c.settings.background_motion { ((t * 1000.0) % 2600.0) / 2600.0 } else { 0.0 };
        let ring_e = Bezier([0.2, 0.6, 0.3, 1.0]).at(ring_k);
        let rr = dot * (1.0 + 5.5 * ring_e);
        p.ellipse(here[0], here[1], rr, rr, 0.14 * vmax * (1.0 + 5.5 * ring_e), title::ACCENT.alpha(0.9 * (1.0 - ring_e) * fade));
    }

    // .eo-menu__glow--warm / --cool.
    let gs = c.len(title::GLOW_SIZE) * 0.5;
    let glow_in = c.anim(200.0, 2200.0, theme::EASE);
    for (warm, period, cx, cy) in [(true, title::GLOW_DRIFT_WARM_S, w - 17.0 * vmax, h + 7.0 * vmax), (false, title::GLOW_DRIFT_COOL_S, 21.0 * vmax, -7.0 * vmax)] {
        let k = alternate(motion_t, period, !warm);
        let (dx, dy, sc) = (-3.0 + 7.0 * k, 2.0 - 5.0 * k, 1.0 + 0.14 * k);
        let cols = if warm { title::GLOW_WARM } else { title::GLOW_COOL };
        let mid = if warm { 0.52 } else { 0.55 };
        let r = gs * sc;
        p.radial(cx + dx * vmax, cy + dy * vmax, r, r, &[(0.0, cols[0].alpha(glow_in)), (mid, cols[1].alpha(glow_in)), (1.0, cols[1].with_a(0.0))]);
    }

    // .eo-menu__shade: two gradients, the first listed on top.
    let sh = rgba(5, 8, 12, 1.0);
    p.linear(Rect::new(0.0, 0.0, w, h), Dir::ToRight, &[(0.0, sh.with_a(0.94)), (0.26, sh.with_a(0.7)), (0.58, sh.with_a(0.12)), (1.0, sh.with_a(0.3))]);
    p.linear(
        Rect::new(0.0, 0.0, w, h),
        Dir::ToBottom,
        &[(0.0, title::NIGHT), (0.14, sh.with_a(0.88)), (0.44, sh.with_a(0.24)), (0.62, sh.with_a(0.2)), (0.88, sh.with_a(0.8)), (1.0, title::NIGHT)],
    );

    // .eo-menu__bars i.
    let bar = c.len(title::BAR_H);
    p.rect(Rect::new(0.0, 0.0, w, bar), theme::BLACK);
    p.rect(Rect::new(0.0, h - bar, w, bar), theme::BLACK);

    // .eo-menu__content padding.
    let pad_x = c.len(Len::Clamp(theme::rem(1.5), theme::vw(5.5), theme::rem(6.0)));
    let pad_top = bar + c.len(Len::Clamp(theme::rem(1.5), theme::vh(6.0), theme::rem(4.5)));
    let pad_bottom = bar + c.len(Len::Clamp(theme::rem(1.0), theme::vh(3.2), theme::rem(2.25)));
    let x0 = pad_x;
    let content_w = w - 2.0 * pad_x;

    // .eo-menu__brand: eyebrow, wordmark, rule.
    let eb = st(Stack::TitleDisplay, 600, c.len(title::EYEBROW_SIZE)).tracking(title::EYEBROW_TRACKING).upper();
    let rise = c.anim(520.0, 900.0, title::MENU_EASE);
    let mut y = pad_top;
    let base = p.text.baseline(eb, y, 1.0);
    p.dy = (1.0 - rise) * 12.0;
    p.opacity = rise;
    p.text(x0, base, "San Francisco", eb, title::ACCENT);
    p.dy = 0.0;
    p.opacity = 1.0;
    y += eb.size + c.len(Len::Clamp(theme::rem(0.5), theme::vh(1.4), theme::rem(1.0)));
    let wm_size = c.len(title::WORDMARK_SIZE);
    let mut letter = 0;
    for (line, word) in ["Earth", "Online"].iter().enumerate() {
        let style = st(Stack::TitleDisplay, if line == 0 { 700 } else { 300 }, wm_size).stretch(title::WORDMARK_STRETCH as u16).upper();
        let base = p.text.baseline(style, y, 0.9);
        let mut x = x0 - 0.04 * wm_size;
        let mut buf = [0u8; 4];
        for ch in word.chars() {
            let s: &str = ch.encode_utf8(&mut buf);
            let k = c.anim(title::LETTER_DELAY_MS + letter as f32 * title::LETTER_STAGGER_MS, title::LETTER_MS, title::MENU_EASE);
            p.opacity = k;
            p.dy = (1.0 - k) * 0.34 * wm_size;
            // text-shadow: 0 0.06em 0.5em rgba(3, 6, 10, 0.6) - drawn as a soft offset copy.
            let adv = p.text(x, base + 0.06 * wm_size, s, style, rgba(3, 6, 10, 0.18));
            p.text(x, base, s, style, theme::hex(0xf6f9fb));
            x += adv + 0.105 * wm_size;
            letter += 1;
        }
        y += 0.9 * wm_size;
    }
    p.opacity = 1.0;
    p.dy = 0.0;
    y += c.len(Len::Clamp(theme::rem(0.9), theme::vh(2.2), theme::rem(1.6)));
    let rule_k = c.anim(760.0, 1000.0, title::MENU_EASE);
    p.opacity = rule_k;
    p.rect(Rect::new(x0, y, c.len(title::RULE_W) * rule_k, 2.0), title::ACCENT);
    p.opacity = 1.0;
    let brand_bottom = y + 2.0;

    // Grid rows: brand, spacer (>= 1.5rem), rail, loading stage.
    let starting = c.screen == Screen::Loading;
    let content_bottom = h - pad_bottom;
    let stage_h = if starting { loading_stage_height(p, c) } else { 0.0 };
    let stage_margin = if starting { c.len(Len::Clamp(theme::rem(1.25), theme::vh(4.0), theme::rem(2.75))) } else { 0.0 };
    let rail_w = (title::RAIL_MAX.px(c.vp, 16.0)).min(content_w);
    let rail_h = match c.title_view {
        TitleView::Main => menu_list_height(p, c),
        TitleView::Settings => c.len(Len::V(theme::L(62.0, theme::Unit::Dvh))),
        TitleView::Exit => 0.0,
    };
    let rail_h = if c.title_view == TitleView::Exit { exit_panel(p, c, x0, 0.0, rail_w, false) } else { rail_h };
    let min_top = brand_bottom + 24.0;
    let rail_top = (content_bottom - stage_h - stage_margin - rail_h).max(min_top);

    // .eo-menu__rail (hidden while starting).
    let rail_k = if starting { 1.0 - c.anim(0.0, 320.0, theme::EASE) } else { 1.0 };
    if rail_k > 0.0 {
        let slide = if starting { (1.0 - rail_k) * -24.0 } else { 0.0 };
        p.dx = slide;
        p.opacity = rail_k;
        match c.title_view {
            TitleView::Main => menu_list(p, c, x0, rail_top, !starting),
            TitleView::Settings => {
                let panel_h = rail_h;
                title_settings_panel(p, c, x0, rail_top, rail_w, panel_h)
            }
            TitleView::Exit => {
                exit_panel(p, c, x0, rail_top, rail_w, true);
            }
        }
        p.dx = 0.0;
        p.opacity = 1.0;
    }

    if starting {
        loading_stage(p, c, x0, content_bottom - stage_h, content_w, stage_h);
    }

    // .eo-menu__credit (+ the Rocketbox avatars' credit).
    let credit = st(Stack::TitleSans, 400, 0.6 * 16.0).tracking(0.06);
    let cw = p.text_width(TITLE_CREDIT, credit);
    let ck = c.anim(0.0, 1600.0, theme::EASE);
    let cy = h - bar * 0.5;
    let base = p.text.baseline(credit, cy - credit.size * 0.5, 1.0);
    p.opacity = ck;
    p.text(w - pad_x - cw, base, TITLE_CREDIT, credit, rgba(225, 235, 243, 0.34));
    p.opacity = 1.0;
}

/// The title's credit line (.eo-menu__credit), and (people) the credit the
/// Rocketbox avatars' MIT licence travels with (their clips
/// too, and the 100STYLE walks' CC BY 4.0 attribution), and (audio) the
/// recorded sounds' (every CC BY clip is listed in native/assets/AUDIO-CREDITS.md).
pub const TITLE_CREDIT: &str = "Map data \u{a9} OpenStreetMap contributors \u{b7} DataSF \u{b7} Avatars and motion: Microsoft Rocketbox Avatar Library \u{a9} 2020 Microsoft, MIT \u{b7} Walks: The 100STYLE Dataset - Ian Mason, CC BY 4.0 \u{b7} Sounds: Freesound contributors via FSD50K, CC0 / CC BY (AUDIO-CREDITS.md)";

const MENU_ITEMS: [(&str, &str, Action); 4] = [
    ("Start Game", "Enter San Francisco", Action::Start),
    ("Ghost View", "Free camera \u{b7} WASD move \u{b7} O to return", Action::Observe),
    ("Settings", "Performance and controls", Action::TitleSettings),
    ("Exit", "Leave Earth Online", Action::TitleExit),
];

fn item_style(c: &Ctx, primary: bool) -> TextStyle {
    if primary {
        st(Stack::TitleDisplay, 700, c.len(title::PRIMARY_SIZE)).tracking(title::PRIMARY_TRACKING).upper()
    } else {
        st(Stack::TitleDisplay, 600, c.len(title::ACTION_SIZE)).tracking(title::ACTION_TRACKING).upper()
    }
}

fn item_height(c: &Ctx, primary: bool) -> f32 {
    let s = item_style(c, primary).size;
    if primary { 0.35 * 16.0 + s * 1.15 + 0.6 * 16.0 } else { 16.0 + s * 1.15 }
}

fn menu_list_height(_p: &mut Painter, c: &mut Ctx) -> f32 {
    let gap = c.len(Len::Clamp(theme::rem(0.1), theme::vh(0.5), theme::rem(0.4)));
    (0..4).map(|i| item_height(c, i == 0)).sum::<f32>() + 3.0 * gap
}

/// `.eo-menu__list` with `.eo-menu__action` items.
fn menu_list(p: &mut Painter, c: &mut Ctx, x: f32, top: f32, interactive: bool) {
    let gap = c.len(Len::Clamp(theme::rem(0.1), theme::vh(0.5), theme::rem(0.4)));
    let cap = st(Stack::TitleSans, 600, 0.7 * 16.0).tracking(0.08);
    let mut y = top;
    for (i, (label, caption, action)) in MENU_ITEMS.iter().enumerate() {
        let primary = i == 0;
        let style = item_style(c, primary);
        let ih = item_height(c, primary);
        let pad_top = if primary { 0.35 * 16.0 } else { 8.0 };
        let enabled = c.ready || !matches!(action, Action::Start | Action::Observe);
        let tw = p.text_width(label, style);
        let cw = p.text_width(caption, cap);
        let bw = 1.6 * 16.0 + tw + 1.1 * 16.0 + cw + 1.25 * 16.0;
        let idx = if interactive && enabled { c.hit(Rect::new(x, y, bw, ih), *action, true) } else { None };
        let hv = if enabled { c.hover(idx) } else { 0.0 };
        let entry = c.anim(title::ITEM_DELAY_MS + i as f32 * title::ITEM_STAGGER_MS, title::ITEM_MS, title::MENU_EASE);
        let o = p.opacity;
        p.opacity = o * entry;
        let base_dx = p.dx;
        p.dx = base_dx + (1.0 - entry) * -16.0 + hv * 0.45 * 16.0;
        let base = p.text.baseline(style, y + pad_top, 1.15);
        let col = if !enabled {
            title::FAINT
        } else if primary {
            lerp_c(title::INK, theme::WHITE, hv)
        } else {
            lerp_c(title::MUTED, title::INK, hv)
        };
        // ::before marker.
        if hv > 0.0 {
            p.rect(Rect::new(x, y + ih * 0.5 - 1.0, 0.95 * 16.0 * hv, 2.0), title::ACCENT.alpha(hv));
        }
        // text-shadow: 0 2px 16px rgba(1, 4, 8, 0.9), approximated by a faint offset copy.
        p.text(x + 1.6 * 16.0, base + 2.0, label, style, rgba(1, 4, 8, 0.35));
        p.text(x + 1.6 * 16.0, base, label, style, col);
        if hv > 0.0 {
            let cx = x + 1.6 * 16.0 + tw + 1.1 * 16.0 - (1.0 - hv) * 8.0;
            p.text(cx, base, caption, cap, title::FAINT.alpha(hv));
        }
        p.dx = base_dx;
        p.opacity = o;
        y += ih + gap;
    }
}

/// `.eo-menu__view[data-menu-view="exit"]`.
fn exit_panel(p: &mut Painter, c: &mut Ctx, x: f32, top: f32, w: f32, draw: bool) -> f32 {
    let pad = c.len(title::SETTINGS_PANEL_PAD);
    let head = st(Stack::TitleDisplay, 600, c.len(title::HEADING_SIZE)).tracking(title::HEADING_TRACKING).upper();
    let copy = st(Stack::TitleSans, 400, 0.92 * 16.0);
    let primary = item_style(c, true);
    let back = st(Stack::TitleDisplay, 700, 0.72 * 16.0).tracking(0.16).upper();
    let copy_h = 2.0 * copy.size * 1.55;
    let h = pad + head.size + 16.0 + 2.0 + 1.6 * 16.0 + copy_h + 1.5 * 16.0 + item_height(c, true) + 1.4 * 16.0 + back.size * 1.2 + 19.2 + 1.0 + pad;
    if !draw {
        return h;
    }
    p.rect(Rect::new(x, top, w, h), title::SURFACE_STRONG);
    p.border(Rect::new(x, top, w, h), [1.0; 4], rgba(225, 235, 243, 0.18));
    let ix = x + pad;
    let mut y = top + pad;
    let base = p.text.baseline(head, y, 1.0);
    p.text(ix, base, "Exit Earth Online?", head, title::INK);
    y += head.size + 16.0;
    p.rect(Rect::new(ix, y, 3.5 * 16.0, 2.0), title::ACCENT);
    y += 2.0 + 1.6 * 16.0;
    paragraph(p, ix, y, 28.0 * copy.size * 0.5, "Close the game window to leave Earth Online.", copy, 1.55, title::MUTED);
    y += copy_h + 1.5 * 16.0;
    let ih = item_height(c, true);
    let tw = p.text_width("Close Game", primary);
    let idx = c.hit(Rect::new(ix, y, 1.6 * 16.0 + tw + 20.0, ih), Action::Close, true);
    let hv = c.hover(idx);
    let b = p.text.baseline(primary, y + 0.35 * 16.0, 1.15);
    if hv > 0.0 {
        p.rect(Rect::new(ix, y + ih * 0.5 - 1.0, 0.95 * 16.0 * hv, 2.0), title::ACCENT.alpha(hv));
    }
    p.text(ix + 1.6 * 16.0 + hv * 7.2, b, "Close Game", primary, lerp_c(title::INK, theme::WHITE, hv));
    y += ih + 1.4 * 16.0;
    back_button(p, c, ix, y, Stack::TitleDisplay);
    h
}

/// `.eo-menu__back`.
fn back_button(p: &mut Painter, c: &mut Ctx, x: f32, y: f32, stack: Stack) -> f32 {
    let back = st(stack, 700, 0.72 * 16.0).tracking(0.16).upper();
    let tw = p.text_width("Back", back);
    let bh = 0.6 * 16.0 * 2.0 + back.size * 1.2;
    let idx = c.hit(Rect::new(x, y, tw, bh), Action::Back, true);
    let hv = c.hover(idx);
    let b = p.text.baseline(back, y + 0.6 * 16.0, 1.2);
    p.text(x, b, "Back", back, lerp_c(title::MUTED, title::INK, hv));
    p.rect(Rect::new(x, y + bh, tw, 1.0), title::LINE);
    bh + 1.0
}

/// `.eo-menu__view[data-menu-view="settings"]`: heading, then the shared
/// graphics block and the title's own rows, scrolled inside 62dvh.
fn title_settings_panel(p: &mut Painter, c: &mut Ctx, x: f32, top: f32, w: f32, h: f32) {
    let pad = c.len(title::SETTINGS_PANEL_PAD);
    p.rect(Rect::new(x, top, w, h), title::SURFACE_STRONG);
    p.border(Rect::new(x, top, w, h), [1.0; 4], rgba(225, 235, 243, 0.18));
    let screen = [c.vp.w, c.vp.h];
    p.set_scissor(Some(Rect::new(x + 1.0, top + 1.0, w - 2.0, h - 2.0)), screen);
    let ix = x + pad;
    let iw = w - 2.0 * pad;
    let mut y = top + pad - c.scroll;
    let head = st(Stack::TitleDisplay, 600, c.len(title::HEADING_SIZE)).tracking(title::HEADING_TRACKING).upper();
    let base = p.text.baseline(head, y, 1.0);
    p.text(ix, base, "Settings", head, title::INK);
    y += head.size + 16.0;
    p.rect(Rect::new(ix, y, 3.5 * 16.0, 2.0), title::ACCENT);
    y += 2.0 + 1.6 * 16.0;
    y += graphics_block(p, c, ix, y, iw, Stack::TitleSans);
    // Background motion.
    let on = c.settings.background_motion;
    y += title_setting_row(p, c, ix, y, iw, "Background motion", if on { "On" } else { "Off" }, Action::Motion);
    for (key, label) in settings::INPUT_SLIDERS {
        let v = c.settings.input.value(key);
        y += title_slider(p, c, ix, y, iw, label, v, settings::InputSettings::range(key), Action::Slider(key));
    }
    let inv = c.settings.input.invert_y;
    y += title_setting_row(p, c, ix, y, iw, "Invert look Y", if inv { "On" } else { "Off" }, Action::InvertY);
    y += 1.4 * 16.0;
    back_button(p, c, ix, y, Stack::TitleDisplay);
    p.set_scissor(None, screen);
}

/// `.eo-menu__setting`.
fn title_setting_row(p: &mut Painter, c: &mut Ctx, x: f32, y: f32, w: f32, label: &str, value: &str, action: Action) -> f32 {
    let ls = st(Stack::TitleSans, 400, 0.9 * 16.0).tracking(0.025);
    let vs = st(Stack::TitleDisplay, 400, 0.76 * 16.0).tracking(0.13).upper();
    let rh = (0.85 * 16.0 * 2.0 + ls.size * 1.33).max(3.3 * 16.0);
    let idx = c.hit(Rect::new(x, y, w, rh), action, true);
    let hv = c.hover(idx);
    let b = p.text.baseline(ls, y + (rh - ls.size * 1.33) * 0.5, 1.33);
    p.text(x, b, label, ls, lerp_c(title::MUTED, title::INK, hv));
    let vw = p.text_width(value, vs);
    p.text(x + w - vw, b, value, vs, title::ACCENT);
    p.rect(Rect::new(x, y + rh - 1.0, w, 1.0), rgba(225, 235, 243, 0.14));
    rh
}

/// A native `<input type=range>` with `accent-color`, as Chrome draws it.
fn range_input(p: &mut Painter, x: f32, y: f32, w: f32, frac: f32, accent: Color, focused: bool) -> f32 {
    let h = 16.0;
    let ty = y + h * 0.5 - 2.0;
    let fx = x + 8.0 + (w - 16.0) * frac.clamp(0.0, 1.0);
    p.rrect(Rect::new(x, ty, w, 4.0), 2.0, rgba(239, 239, 239, 1.0));
    p.rrect(Rect::new(x, ty, fx - x, 4.0), 2.0, accent);
    p.ellipse(fx, y + h * 0.5, 8.0, 8.0, 0.0, accent);
    if focused {
        p.border(Rect::new(x - 4.0, y - 4.0, w + 8.0, h + 8.0), [1.0; 4], accent);
    }
    h
}

/// `.eo-menu__slider`.
fn title_slider(p: &mut Painter, c: &mut Ctx, x: f32, y: f32, w: f32, label: &str, v: f64, r: settings::Range, action: Action) -> f32 {
    let ls = st(Stack::TitleSans, 400, 0.86 * 16.0).tracking(0.06).upper();
    let pad = 0.7 * 16.0;
    let mut yy = y + pad;
    let b = p.text.baseline(ls, yy, 1.33);
    p.text(x, b, label, ls, title::INK);
    c.fmt.clear();
    let _ = write!(c.fmt, "{v:.2}\u{d7}");
    let out = std::mem::take(c.fmt);
    let ow = p.text_width(&out, ls);
    p.text(x + w - ow, b, &out, ls, title::ACCENT);
    *c.fmt = out;
    yy += ls.size * 1.33 + 0.45 * 16.0;
    let idx = c.hit(Rect::new(x, yy, w, 16.0), action, true);
    let f = ((v - r.min) / (r.max - r.min)) as f32;
    yy += range_input(p, x, yy, w, f, title::ACCENT, c.focused(idx));
    yy += pad;
    p.rect(Rect::new(x, yy, w, 1.0), title::LINE);
    yy + 1.0 - y
}

/// `installPerformanceSettings` markup with `performance-settings.css`:
/// the Graphics block both menus share. Returns its height.
fn graphics_block(p: &mut Painter, c: &mut Ctx, x: f32, top: f32, w: f32, stack: Stack) -> f32 {
    let accent = theme::hex(0xe2ad61);
    let body = st(stack, 400, perf::BUTTON_SIZE);
    let small = st(stack, 400, perf::SMALL_SIZE);
    let lh_normal = 1.33;
    let mut y = top;
    let h3 = st(stack, 400, perf::H3_SIZE);
    let b = p.text.baseline(h3, y, lh_normal);
    p.text(x, b, "Graphics", h3, perf::INK);
    y += h3.size * lh_normal + 14.0;
    // Presets.
    let current = c.settings.performance.preset_name();
    let mut bx = x;
    let bh = 22.0 + body.size * lh_normal + 2.0;
    for preset in settings::presets() {
        let tw = p.text_width(preset.label, body);
        let r = Rect::new(bx, y, tw + 28.0 + 2.0, bh);
        let idx = c.hit(r, Action::Preset(preset.key), true);
        let pressed = current == preset.key;
        button_box(p, r, pressed || c.focused(idx), accent);
        let tb = p.text.baseline(body, y + 12.0, lh_normal);
        p.text(bx + 15.0, tb, preset.label, body, if pressed { accent } else { perf::INK });
        bx += r.w + 8.0;
    }
    y += bh + body.size;
    let desc = settings::presets().into_iter().find(|p| p.key == current).map(|p| p.description).unwrap_or(settings::CUSTOM_DESCRIPTION);
    y += paragraph(p, x, y, w, desc, body, 1.5, perf::COPY) + body.size;
    // Adaptive quality: the native build has no governor yet; the row stays, honestly labelled.
    y += 16.0;
    y += toggle_row(p, c, x, y, w, "Adaptive quality", "Off", false, Action::Adaptive, body, accent);
    y += 8.0;
    y += paragraph(p, x, y, w, "Not in the native build yet: your settings are applied as set.", small, 1.5, perf::COPY);
    // Render resolution.
    y += 16.0;
    let pct = (c.settings.performance.resolution_scale * 100.0).round() as i64;
    c.fmt.clear();
    let _ = write!(c.fmt, "{pct}%");
    let out = std::mem::take(c.fmt);
    let lb = p.text.baseline(body, y, lh_normal);
    p.text(x, lb, "Render resolution ", body, perf::INK);
    let ow = p.text_width(&out, body);
    p.text(x + w - ow, lb, &out, body, accent);
    *c.fmt = out;
    y += body.size * lh_normal + 12.0;
    let idx = c.hit(Rect::new(x, y, w, 16.0), Action::MapPane, false);
    let _ = idx;
    y += range_input(p, x, y, w, ((c.settings.performance.resolution_scale - 0.5) / 0.5) as f32, accent, false);
    y += 8.0;
    y += paragraph(p, x, y, w, "Lower resolution draws fewer pixels. The menus stay sharp.", small, 1.5, perf::COPY);
    for (key, label, _) in settings::GRAPHICS_OPTIONS {
        y += 16.0;
        let v = c.settings.performance.get(key);
        let pressed = v == "on" || v == "high" || v == "smaa" || v == "native";
        y += toggle_row(p, c, x, y, w, label, settings::option_label(v), pressed, Action::Option(key), body, accent);
        y += 8.0 - 8.0;
    }
    y += 8.0;
    y += paragraph(p, x, y, w, "High, Medium and Low change these together. Turn any one down on its own for a custom mix.", small, 1.5, perf::COPY);
    y += paragraph(
        p,
        x,
        y,
        w,
        "Resolution cap: the most pixels the game draws on a large display; above it the picture is stretched to fit. Anti-aliasing: SMAA, the lighter FXAA, or off; a change applies the next time the game starts.",
        small,
        1.5,
        perf::COPY,
    );
    y += 16.0;
    let fps = c.settings.performance.show_fps;
    y += toggle_row(p, c, x, y, w, "Show FPS", if fps { "On" } else { "Off" }, fps, Action::Fps, body, accent);
    y += 8.0;
    y += paragraph(p, x, y, w, "Saved on this device and applied in the city.", small, 1.5, perf::COPY);
    y += 22.0;
    p.rect(Rect::new(x, y, w, 1.0), perf::RULE);
    y += 1.0 + 28.0;
    y - top
}

fn button_box(p: &mut Painter, r: Rect, pressed: bool, accent: Color) {
    p.rect(r, perf::BUTTON_BG);
    p.border(r, [1.0; 4], if pressed { accent } else { perf::BUTTON_BORDER });
}

/// `.performance-settings__row.performance-settings__toggle`.
#[allow(clippy::too_many_arguments)]
fn toggle_row(p: &mut Painter, c: &mut Ctx, x: f32, y: f32, w: f32, label: &str, value: &str, pressed: bool, action: Action, body: TextStyle, accent: Color) -> f32 {
    let lh = 1.33;
    let h = 22.0 + body.size * lh + 2.0;
    let r = Rect::new(x, y, w, h);
    let idx = c.hit(r, action, true);
    button_box(p, r, pressed || c.focused(idx), accent);
    let b = p.text.baseline(body, y + 12.0, lh);
    let col = if pressed { accent } else { perf::INK };
    p.text(x + 15.0, b, label, body, col);
    let vs = TextStyle { weight: 500, ..body };
    let vw = p.text_width(value, vs);
    p.text(x + w - 15.0 - vw, b, value, vs, col);
    h
}

fn loading_stage_height(_p: &mut Painter, c: &Ctx) -> f32 {
    let phase = c.len(title::LOADING_PHASE_SIZE);
    phase * 1.2 + 0.4 * 16.0 + 0.8 * 16.0 * 1.4 + 0.9 * 16.0
}

/// `.eo-menu__loading-stage`: phase, detail, key hints and the progress line.
fn loading_stage(p: &mut Painter, c: &mut Ctx, x: f32, top: f32, w: f32, h: f32) {
    c.animating = true;
    let fade = c.anim(0.0, 900.0, theme::EASE);
    let o = p.opacity;
    p.opacity = fade;
    let phase = st(Stack::TitleDisplay, 600, c.len(title::LOADING_PHASE_SIZE)).tracking(title::LOADING_PHASE_TRACKING).upper();
    let b = p.text.baseline(phase, top, 1.2);
    p.text(x, b, &c.loading.phase, phase, title::ACCENT);
    let detail = st(Stack::TitleSans, 400, 0.8 * 16.0);
    let dy = top + phase.size * 1.2 + 0.4 * 16.0;
    let b = p.text.baseline(detail, dy, 1.4);
    p.text(x, b, &c.loading.detail, detail, title::MUTED);
    // Hints (`CONTROL_HINTS` with `DEFAULT_KEYBOARD_BINDINGS`), one at a time.
    const HINTS: [(&[&str], &str); 12] = [
        (&["W", "A", "S", "D"], "Move"),
        (&["Shift"], "Sprint"),
        (&["Space"], "Jump"),
        (&["F"], "Enter or leave a vehicle"),
        (&["E"], "Interact"),
        (&["Right mouse"], "Aim"),
        (&["Left mouse"], "Fire"),
        (&["Tab"], "Weapon wheel"),
        (&["P"], "Phone"),
        (&["J"], "Missions"),
        (&["O"], "Ghost view"),
        (&["Esc"], "Pause"),
    ];
    let k = ((c.t / 3.6).floor() as usize) % HINTS.len();
    let ph = ((c.t / 3.6).fract() * 3.6).min(3.6 - (c.t / 3.6).fract() * 3.6);
    let hint_a = (ph / 0.3).clamp(0.0, 1.0);
    let (keys, label) = HINTS[k];
    let kbd = st(Stack::TitleDisplay, 600, 0.72 * 16.0).tracking(0.04);
    let lab = st(Stack::TitleSans, 600, 0.74 * 16.0).tracking(0.08).upper();
    let lw = p.text_width(label, lab);
    let mut total = 0.45 * 16.0 + lw;
    for key in keys {
        total += (p.text_width(key, kbd) + 12.8 + 2.0).max(24.0) + 0.35 * 16.0;
    }
    let hint_bottom = top + h - 0.9 * 16.0;
    let kh = kbd.size + 6.4 + 2.0;
    let mut hx = x + w - total;
    p.opacity = fade * hint_a;
    for key in keys {
        let kw = (p.text_width(key, kbd) + 12.8 + 2.0).max(24.0);
        let r = Rect::new(hx, hint_bottom - kh, kw, kh);
        p.rrect(r, 3.0, rgba(5, 10, 17, 0.55));
        p.rrect_stroke(r, 3.0, 1.0, rgba(225, 235, 243, 0.3));
        let tw = p.text_width(key, kbd);
        let b = p.text.baseline(kbd, r.y + 3.2 + 1.0, 1.0);
        p.text(hx + (kw - tw) * 0.5, b, key, kbd, title::INK);
        hx += kw + 0.35 * 16.0;
    }
    let b = p.text.baseline(lab, hint_bottom - kh + (kh - lab.size * 1.2) * 0.5, 1.2);
    p.text(hx + 0.45 * 16.0 - 0.35 * 16.0, b, label, lab, title::MUTED);
    p.opacity = fade;
    // .eo-menu__track
    let ty = top + h - 2.0;
    p.rect(Rect::new(x, ty, w, 2.0), rgba(237, 245, 251, 0.14));
    p.rect(Rect::new(x, ty, w * c.loading.progress.clamp(0.0, 1.0), 2.0), title::ACCENT);
    let lk = Bezier([0.4, 0.0, 0.3, 1.0]).at(((c.t * 1000.0) % title::LINE_LIGHT_MS) / title::LINE_LIGHT_MS);
    let lw = 0.22 * w;
    let lx = x + lw * (-1.1 + 6.7 * lk);
    let light = rgba(255, 244, 226, 0.75);
    let seg = Rect::new(lx, ty, lw, 2.0);
    if seg.x < x + w && seg.right() > x {
        let screen = [c.vp.w, c.vp.h];
        p.set_scissor(Some(Rect::new(x, ty, w, 2.0)), screen);
        p.linear(seg, Dir::ToRight, &[(0.0, light.with_a(0.0)), (0.5, light), (1.0, light.with_a(0.0))]);
        p.set_scissor(None, screen);
    }
    p.opacity = o;
}

// ---------------------------------------------------------------------------
// Pause (src/ui/pause-menu.css, pause-menu.js, pause-map.js)
// ---------------------------------------------------------------------------

fn edges(vp: Viewport) -> (f32, f32) {
    // --hud-edge-x / --hud-edge-y: the 16:9 frame, inset 5%.
    let fw = vp.w.min(vp.h * 16.0 / 9.0);
    let fh = vp.h.min(vp.w * 9.0 / 16.0);
    ((vp.w - fw) / 2.0 + fw * 0.05, (vp.h - fh) / 2.0 + fh * 0.05)
}

fn pause_screen(p: &mut Painter, c: &mut Ctx) {
    let (w, h) = (c.vp.w, c.vp.h);
    let open = c.anim(0.0, pause::OPEN_MS, theme::EASE);
    p.opacity = open;
    p.rect(Rect::new(0.0, 0.0, w, h), pause::BACKGROUND);
    // .pause-menu__shade (bottom layer first).
    p.linear(Rect::new(0.0, 0.0, w, h), Dir::ToRight, &[(0.0, pause::SHADE_BASE[0]), (1.0, pause::SHADE_BASE[1])]);
    p.radial(0.08 * w, -0.12 * h, 1.1 * w * 0.62, 0.8 * h * 0.62, &[(0.0, pause::SHADE_COOL), (1.0, pause::SHADE_COOL.with_a(0.0))]);
    p.radial(0.82 * w, 1.08 * h, 1.2 * w * 0.6, 0.9 * h * 0.6, &[(0.0, pause::SHADE_WARM), (1.0, pause::SHADE_WARM.with_a(0.0))]);

    let (ex, ey) = edges(c.vp);
    let gap = c.len(pause::GAP);
    let x0 = ex;
    let cw = w - 2.0 * ex;
    let mut y = ey;
    // Head: "Paused" with its double bar, and the set-type brand.
    let head_h = 56.0f32;
    let ts = st(Stack::Pause, 500, c.len(pause::TITLE_SIZE)).tracking(pause::TITLE_TRACKING).upper();
    p.rect(Rect::new(x0, y + head_h * 0.5 - 8.5, 4.0, 17.0), pause::ACCENT);
    p.rect(Rect::new(x0 + 8.0, y + head_h * 0.5 - 8.5, 4.0, 17.0), pause::ACCENT);
    let b = p.text.baseline(ts, y + (head_h - ts.size * 1.15) * 0.5, 1.15);
    p.text(x0 + 12.0 + 16.0, b, "Paused", ts, pause::INK);
    let bs = st(Stack::PauseCondensed, 800, c.len(pause::BRAND_SIZE)).tracking(pause::BRAND_TRACKING).upper();
    let bw = p.text_width("Earth Online", bs);
    let brand_h = bs.size * (1.0 + 0.12) + 3.0;
    let by = y + (head_h - brand_h) * 0.5;
    let b = p.text.baseline(bs, by, 1.0);
    p.text(x0 + cw - bw, b + 2.0, "Earth Online", bs, pause::BRAND_SHADOW);
    p.text(x0 + cw - bw, b, "Earth Online", bs, pause::BRAND_INK);
    p.rect(Rect::new(x0 + cw - bw, by + bs.size * 1.12, bw, 3.0), pause::BRAND_RULE);
    y += head_h + gap;
    // Tabs.
    let tab = st(Stack::PauseCondensed, 600, c.len(pause::TAB_SIZE)).tracking(pause::TAB_TRACKING).upper();
    let tpx = c.len(pause::TAB_PAD_X);
    let tab_h = 10.0 + tab.size * 1.15 + 14.0 + 2.0;
    let tabs = [(PauseTab::Map, "Map"), (PauseTab::Journal, "Journal"), (PauseTab::Inventory, "Inventory"), (PauseTab::Garage, "Garage"), (PauseTab::Settings, "Settings")];
    let mut tx = x0;
    let tgap = c.len(pause::TABS_GAP);
    for (t, label) in tabs {
        let tw = p.text_width(label, tab) + 2.0 * tpx;
        let idx = c.hit(Rect::new(tx, y, tw, tab_h), Action::Tab(t), true);
        let current = c.pause_tab == t || (t == PauseTab::Map && c.pause_tab == PauseTab::Quit && false);
        let b = p.text.baseline(tab, y + 10.0, 1.15);
        let col = if current { pause::ACCENT } else if c.focused(idx) { pause::INK } else { pause::MUTED };
        p.text(tx + tpx, b, label, tab, col);
        if current {
            p.rect(Rect::new(tx, y + tab_h - 2.0, tw, 2.0), pause::ACCENT);
        }
        tx += tw + tgap;
    }
    p.rect(Rect::new(x0, y + tab_h, cw, 1.0), pause::LINE);
    y += tab_h + 1.0 + gap;
    // Footer hints.
    let hint = st(Stack::Pause, 400, 11.0);
    let foot_h = 1.0 + 14.0 + (hint.size * 1.15 + 8.0 + 2.0);
    let foot_y = h - ey - foot_h;
    // Body: pages and the action rail.
    let body = Rect::new(x0, y, cw, foot_y - gap - y);
    let rail_w = if c.pause_tab == PauseTab::Map { c.len(pause::RAIL_W) } else { (cw * 1.0 / 2.45).max(260.0) };
    let pages = Rect::new(body.x, body.y, body.w - rail_w - 12.0, body.h);
    p.rect(pages, pause::PAGES_BG);
    p.border(pages, [1.0; 4], pause::PAGES_BORDER);
    let inner = pages.inset(1.0, 1.0, 1.0, 1.0);
    match c.pause_tab {
        PauseTab::Map => pause_map(p, c, inner),
        PauseTab::Settings => pause_settings(p, c, inner),
        PauseTab::Quit => pause_quit(p, c, inner),
        PauseTab::Journal => pause_sheet_empty(p, c, inner, "Your story", "Journal", "Your next chapter is out there.", "Jobs and the story are not in the native build yet."),
        PauseTab::Inventory => pause_sheet_empty(p, c, inner, "What you carry", "Inventory", "Nothing to show yet.", "The economy is not in the native build yet."),
        PauseTab::Garage => pause_sheet_empty(p, c, inner, "Your vehicles", "Garage", "No vehicles yet.", "Vehicles are not in the native build yet."),
    }
    // Action rail tiles.
    let rail = Rect::new(pages.right() + 12.0, body.y, rail_w, body.h);
    let tiles = [("Resume game", "Return to the city", Action::Resume, true), ("Settings", "Audio, graphics and controls", Action::PauseSettings, false), ("Quit to title", "Leave Earth Online", Action::Quit, false)];
    let span = st(Stack::PauseCondensed, 650, c.len(pause::TILE_SIZE)).tracking(0.12).upper();
    let small = st(Stack::Pause, 400, 10.0).tracking(0.1).upper();
    let tile_h = 13.0 + span.size * 1.15 + 4.0 + small.size * 1.15 + 12.0 + 2.0;
    let mut ty = rail.y;
    for (label, sub, action, primary) in tiles {
        let r = Rect::new(rail.x, ty, rail.w, tile_h);
        let idx = c.hit(r, action, true);
        let hv = c.focused(idx) || c.hover(idx) > 0.5;
        let bg = if primary { pause::TILE_PRIMARY_BG } else { pause::TILE_BG };
        if primary {
            p.linear(r, Dir::ToRight, &[(0.0, bg[0]), (0.7, bg[1])]);
        } else {
            p.linear(r, Dir::ToRight, &[(0.0, bg[0]), (1.0, bg[1])]);
        }
        if hv {
            p.rect(r, rgba(255, 255, 255, 0.05));
        }
        p.border(r, [1.0, 1.0, 1.0, 0.0], pause::TILE_BORDER);
        p.rect(Rect::new(r.x, r.y, 3.0, r.h), if primary || hv { pause::ACCENT } else { pause::TILE_BORDER.with_a(0.0) });
        let sb = p.text.baseline(small, r.bottom() - 12.0 - small.size * 1.15, 1.15);
        let lb = p.text.baseline(span, r.bottom() - 12.0 - small.size * 1.15 - 4.0 - span.size * 1.15, 1.15);
        p.text(r.x + 3.0 + 16.0, lb, label, span, pause::INK);
        p.text(r.x + 3.0 + 16.0, sb, sub, small, pause::TILE_SMALL);
        if primary {
            let arrow = st(Stack::Pause, 400, 22.0);
            let aw = p.text_width("\u{2192}", arrow);
            let ab = p.text.baseline(arrow, r.y + (r.h - 22.0 * 1.15) * 0.5, 1.15);
            p.text(r.right() - 16.0 - aw, ab, "\u{2192}", arrow, pause::ACCENT);
        }
        ty += tile_h + 6.0;
    }
    // Footer.
    p.rect(Rect::new(x0, foot_y, cw, 1.0), pause::LINE);
    let mut fx = x0;
    let fy = foot_y + 1.0 + 14.0;
    let fgap = c.len(Len::Clamp(theme::px(12.0), theme::vw(2.0), theme::px(35.0)));
    let back_label = if matches!(c.pause_tab, PauseTab::Settings | PauseTab::Quit) { "Back" } else { "Resume" };
    let items: [(&str, &str, bool); 6] = [("Esc", back_label, true), ("Enter", "Select", true), ("\u{2191}\u{2193}", "Move", true), ("Click", "Waypoint", false), ("Drag", "Pan", false), ("Wheel", "Zoom", false)];
    for (k, label, always) in items {
        if !always && c.pause_tab != PauseTab::Map {
            continue;
        }
        let kw = (p.text_width(k, hint) + 14.0 + 2.0).max(20.0 + 2.0);
        let kh = hint.size * 1.15 + 8.0 + 2.0;
        p.border(Rect::new(fx, fy, kw, kh), [1.0; 4], pause::KBD_BORDER);
        let b = p.text.baseline(hint, fy + 5.0, 1.15);
        let tw = p.text_width(k, hint);
        p.text(fx + (kw - tw) * 0.5, b, k, hint, pause::INK);
        let lw = p.text(fx + kw + 8.0, b, label, hint, pause::MUTED);
        fx += kw + 8.0 + lw + fgap;
    }
    p.opacity = 1.0;
    if open < 1.0 {
        c.animating = true;
    }
}

fn pause_sheet_empty(p: &mut Painter, c: &mut Ctx, r: Rect, eyebrow: &str, head: &str, big: &str, small_text: &str) {
    let pad = c.len(pause::SHEET_PAD);
    let x = r.x + pad;
    let mut y = r.y + pad;
    let eb = st(Stack::Pause, 400, 11.0).tracking(0.17).upper();
    let b = p.text.baseline(eb, y, 1.15);
    p.text(x, b, eyebrow, eb, pause::ACCENT);
    y += 11.0 * 1.15 + 5.0;
    let h3 = st(Stack::Pause, 500, c.len(pause::SHEET_H3));
    let b = p.text.baseline(h3, y, 1.15);
    p.text(x, b, head, h3, pause::INK);
    y += h3.size * 1.15 + 26.0 + 0.08 * c.vp.h;
    let h4 = st(Stack::Pause, 500, 23.0);
    y += paragraph(p, x, y, 360.0, big, h4, 1.4, pause::INK) + 14.0;
    paragraph(p, x, y, 360.0, small_text, st(Stack::Pause, 400, 14.0), 1.6, pause::MUTED);
}

fn pause_quit(p: &mut Painter, c: &mut Ctx, r: Rect) {
    let pad = c.len(pause::SHEET_PAD);
    let x = r.x + pad;
    let mut y = r.y + pad;
    let eb = st(Stack::Pause, 400, 11.0).tracking(0.17).upper();
    let b = p.text.baseline(eb, y, 1.15);
    p.text(x, b, "Leave the city", eb, pause::ACCENT);
    y += 11.0 * 1.15 + 5.0;
    let h3 = st(Stack::Pause, 500, c.len(pause::SHEET_H3));
    let b = p.text.baseline(h3, y, 1.15);
    p.text(x, b, "Quit to title?", h3, pause::INK);
    y += h3.size * 1.15 + 26.0;
    y += paragraph(p, x, y, r.w - 2.0 * pad, "Leave the city and return to the title screen?", st(Stack::Pause, 400, 14.0), 1.6, pause::MUTED) + 14.0;
    let w = (r.w - 2.0 * pad).min(520.0);
    y += pause_item(p, c, x, y, w, "Quit to title", "", Action::QuitConfirm, true);
    pause_item(p, c, x, y, w, "Stay", "Esc", Action::Back, false);
}

/// `.pause-menu__item`.
fn pause_item(p: &mut Painter, c: &mut Ctx, x: f32, y: f32, w: f32, label: &str, small_text: &str, action: Action, danger: bool) -> f32 {
    let s = st(Stack::Pause, 400, 16.0);
    let h = 32.0 + s.size * 1.15 + 2.0;
    let r = Rect::new(x, y + 12.0, w, h);
    let idx = c.hit(r, action, true);
    p.rect(r, pause::ITEM_BG);
    let border = if c.focused(idx) || c.hover(idx) > 0.5 { pause::ACCENT } else if danger { theme::hex(0xcb826a) } else { pause::ITEM_BORDER };
    p.border(r, [1.0; 4], border);
    let b = p.text.baseline(s, r.y + 16.0, 1.15);
    p.text(r.x + 16.0, b, label, s, pause::INK);
    if !small_text.is_empty() {
        let sw = p.text_width(small_text, s);
        p.text(r.right() - 16.0 - sw, b, small_text, s, pause::MUTED);
    }
    h + 12.0
}

/// `.pause-menu__slider` (with `--mix` for the per-category sliders).
#[allow(clippy::too_many_arguments)]
fn pause_slider(p: &mut Painter, c: &mut Ctx, x: f32, y: f32, w: f32, label: &str, note: &str, readout: &str, frac: f32, action: Action, mix: bool) -> f32 {
    let ls = st(Stack::Pause, 400, 13.0);
    let (pad_y, gap, ix) = if mix { (14.0, 10.0, x + 18.0) } else { (20.0, 16.0, x) };
    let iw = w - (ix - x);
    let mut yy = y + pad_y;
    let b = p.text.baseline(ls, yy, 1.15);
    p.text(ix, b, label, ls, pause::INK);
    let rw = p.text_width(readout, ls);
    p.text(ix + iw - rw, b, readout, ls, pause::ACCENT);
    yy += ls.size * 1.15;
    if !note.is_empty() {
        let ns = st(Stack::Pause, 400, 11.0).tracking(0.01);
        let b = p.text.baseline(ns, yy + 3.0, 1.15);
        p.text(ix, b, note, ns, theme::hex(0x8ea3b4));
        yy += 3.0 + 11.0 * 1.15;
    }
    yy += gap;
    let idx = c.hit(Rect::new(ix, yy, iw, 16.0), action, true);
    yy += range_input(p, ix, yy, iw, frac, pause::ACCENT, c.focused(idx));
    yy += pad_y;
    if mix {
        p.rect(Rect::new(x, y, 2.0, yy - y), pause::SLIDER_RULE);
    } else {
        p.rect(Rect::new(x, yy, w, 1.0), pause::SLIDER_RULE);
    }
    yy + if mix { 0.0 } else { 1.0 } - y
}

/// The pause Settings sheet: audio (master and sound effects drive eo-audio; radio and dialogue have nothing to play yet), the shared graphics
/// block, controls.
fn pause_settings(p: &mut Painter, c: &mut Ctx, r: Rect) {
    let screen = [c.vp.w, c.vp.h];
    p.set_scissor(Some(r), screen);
    let pad = c.len(pause::SHEET_PAD);
    let x = r.x + pad;
    let w = r.w - 2.0 * pad;
    let mut y = r.y + pad - c.scroll;
    let eb = st(Stack::Pause, 400, 11.0).tracking(0.17).upper();
    let b = p.text.baseline(eb, y, 1.15);
    p.text(x, b, "Make it yours", eb, pause::ACCENT);
    y += 11.0 * 1.15;
    let h3 = st(Stack::Pause, 500, c.len(pause::SHEET_H3));
    let heading = |p: &mut Painter, y: &mut f32, text: &str| {
        *y += 5.0;
        let b = p.text.baseline(h3, *y, 1.15);
        p.text(x, b, text, h3, pause::INK);
        *y += h3.size * 1.15 + 26.0;
    };
    heading(p, &mut y, "Audio");
    let vol = c.settings.audio_volume;
    let readout = settings::format_audio_volume(vol);
    y += pause_slider(p, c, x, y, w, "Master volume", "", &readout, vol as f32, Action::Volume, false);
    y += paragraph(p, x, y + 14.0, w, "Scales every sound in the city. Remembered next time. (Native build: the city and the menus sound; no radio or voices yet.)", st(Stack::Pause, 400, 14.0), 1.6, pause::MUTED) + 28.0;
    for (key, label, note) in settings::AUDIO_MIX_CATEGORIES {
        let v = c.settings.audio_mix.get(key);
        let readout = settings::format_audio_volume(v);
        y += pause_slider(p, c, x, y, w, label, note, &readout, v as f32, Action::Mix(key), true);
    }
    y += 28.0;
    y += graphics_block(p, c, x, y, w, Stack::Pause);
    heading(p, &mut y, "Controls");
    for (key, label) in settings::INPUT_SLIDERS {
        let v = c.settings.input.value(key);
        let rg = settings::InputSettings::range(key);
        c.fmt.clear();
        let _ = write!(c.fmt, "{v:.2}\u{d7}");
        let out = std::mem::take(c.fmt);
        y += pause_slider(p, c, x, y, w, label, "", &out, ((v - rg.min) / (rg.max - rg.min)) as f32, Action::Slider(key), false);
        *c.fmt = out;
    }
    let inv = c.settings.input.invert_y;
    y += pause_item(p, c, x, y, w, "Invert look Y", if inv { "On" } else { "Off" }, Action::InvertY, false);
    pause_item(p, c, x, y, w, "Back", "Esc", Action::Back, false);
    p.set_scissor(None, screen);
}

/// The Map tab: the city map image, the location plate, tools, legend.
fn pause_map(p: &mut Painter, c: &mut Ctx, r: Rect) {
    let screen = [c.vp.w, c.vp.h];
    let legend_font = st(Stack::Pause, 400, 11.0);
    let legend_h = 9.0 * 2.0 + legend_font.size * 1.15 + 4.0 + 1.0;
    let pane = Rect::new(r.x, r.y, r.w, r.h - legend_h);
    c.pause_pane = pane;
    c.hit(pane, Action::MapPane, false);
    p.rect(pane, hud::MAP_GROUND);
    let (cx, cz, s) = c.pause_view.unwrap_or((c.hud.player[0], c.hud.player[1], pause::OPEN_SCALE));
    let image = c.map.as_ref().map(|m| (m.pause_image, m.pause_view));
    if let Some((img, Some(v))) = image {
        if img != 0 && v.w == pane.w.round() as u32 && v.h == pane.h.round() as u32 {
            p.image(Rect::new(pane.x, pane.y, v.w as f32, v.h as f32), img, [0.0, 0.0, 1.0, 1.0], 0.0, 1.0);
        }
    }
    p.set_scissor(Some(pane), screen);
    // The player arrow (MAP_PALETTE.player).
    let px = pane.x + pane.w * 0.5 + (c.hud.player[0] - cx) * s;
    let pz = pane.y + pane.h * 0.5 + (c.hud.player[1] - cz) * s;
    arrow(p, px, pz, c.hud.heading, 9.0, theme::map::PLAYER, rgba(10, 12, 16, 0.9));
    p.set_scissor(None, screen);
    // Location plate.
    let name = c.map.as_ref().and_then(|m| m.streets.nearest(c.hud.player[0], c.hud.player[1], 150.0)).map(|(n, _)| n).unwrap_or("Current location");
    let ls = st(Stack::Pause, 600, c.len(pause::LOCATION_SIZE)).tracking(0.12).upper();
    let ws = st(Stack::Pause, 400, 11.0).tracking(0.08);
    let nw = p.text_width(name, ls).max(p.text_width("San Francisco", ws));
    let plate = Rect::new(pane.x + 16.0, pane.y + 14.0, 2.0 + 14.0 + nw + 14.0, 10.0 + ls.size * 1.15 + 4.0 + 11.0 * 1.15 + 9.0);
    p.rect(plate, pause::LOCATION_BG);
    p.rect(Rect::new(plate.x, plate.y, 2.0, plate.h), pause::ACCENT);
    let b = p.text.baseline(ls, plate.y + 10.0, 1.15);
    p.text(plate.x + 16.0, b, name, ls, pause::ACCENT);
    let b = p.text.baseline(ws, plate.y + 10.0 + ls.size * 1.15 + 4.0, 1.15);
    p.text(plate.x + 16.0, b, "San Francisco", ws, pause::MUTED);
    // North and the tools (top right).
    let ns = st(Stack::Pause, 600, 11.0);
    let nw = p.text_width("N", ns);
    let north = Rect::new(pane.right() - 16.0 - nw - 14.0, pane.y + 14.0, nw + 14.0, 11.0 * 1.15 + 10.0);
    p.rect(north, pause::NOTE_BG);
    let b = p.text.baseline(ns, north.y + 5.0, 1.15);
    p.text(north.x + 7.0, b, "N", ns, pause::ACCENT);
    let tool = st(Stack::Pause, 500, 11.0);
    let zoom = st(Stack::Pause, 700, 18.0);
    let mut tx = pane.right() - 58.0;
    let tools: [(&str, Action, bool); 4] = [("Whole city", Action::MapFit, false), ("Locate player", Action::MapPlayer, false), ("+", Action::MapIn, true), ("\u{2212}", Action::MapOut, true)];
    let th = 8.0 * 2.0 + 11.0 * 1.15 + 2.0;
    for (label, action, is_zoom) in tools {
        let style = if is_zoom { zoom } else { tool };
        let tw = p.text_width(label, style);
        let bw = if is_zoom { (tw + 16.0 + 2.0).max(34.0) } else { tw + 20.0 + 2.0 };
        tx -= bw;
        let rr = Rect::new(tx, pane.y + 12.0, bw, th);
        let idx = c.hit(rr, action, true);
        p.rect(rr, pause::TOOL_BG);
        let disabled = (action == Action::MapIn && s >= pause::MAX_SCALE * 0.999) || (action == Action::MapOut && c.map.as_ref().is_some_and(|m| s <= fit_scale(pane, m.bounds) * 1.001));
        let border = if c.focused(idx) { pause::ACCENT } else if is_zoom { pause::LINE } else { pause::TOOL_BORDER };
        p.border(rr, [1.0; 4], if disabled { theme::hex(0x34495a) } else { border });
        let col = if disabled { theme::hex(0x8d99a4) } else if is_zoom { pause::ACCENT } else { pause::INK };
        let b = p.text.baseline(style, rr.y + (rr.h - style.size * 1.15) * 0.5, 1.15);
        p.text(rr.x + (bw - tw) * 0.5, b, label, style, col);
        tx -= 5.0;
    }
    // Legend strip.
    let lg = Rect::new(r.x, pane.bottom(), r.w, legend_h);
    p.rect(lg, pause::LEGEND_BG);
    p.rect(Rect::new(lg.x, lg.y, lg.w, 1.0), rgba(196, 216, 235, 0.16));
    let ly = lg.y + 1.0 + 9.0;
    let b = p.text.baseline(legend_font, ly, 1.15);
    let mut lx = lg.x + 16.0;
    p.ellipse(lx + 3.0, ly + legend_font.size * 0.6, 3.0, 3.0, 0.0, hud::INK);
    lx += 6.0 + 7.0;
    lx += p.text(lx, b, "You", legend_font, pause::MUTED) + 7.0 + 8.0;
    for (label, col) in [("Objective", hud::MAP_ROUTE), ("Waypoint", hud::WAYPOINT)] {
        let d = [[lx + 4.0, ly + 1.0], [lx + 8.0, ly + 5.0], [lx + 4.0, ly + 9.0], [lx, ly + 5.0]];
        p.polygon(&d, col);
        lx += 8.0 + 4.0;
        lx += p.text(lx, b, label, legend_font, pause::MUTED) + 7.0 + 8.0;
    }
    // Ruler: a round metric length near 90 px.
    let target = 90.0 / s;
    let power = 10f32.powf(target.log10().floor());
    let metres = [5.0, 2.0, 1.0].iter().map(|k| k * power).find(|m| *m <= target).unwrap_or(power);
    c.fmt.clear();
    if metres >= 1000.0 {
        let _ = write!(c.fmt, "{} km", metres / 1000.0);
    } else {
        let _ = write!(c.fmt, "{} m", metres.round());
    }
    let label = std::mem::take(c.fmt);
    let bar_w = metres * s;
    lx += 10.0;
    let lw = p.text_width(&label, legend_font);
    let rcx = lx + bar_w.max(lw) * 0.5;
    p.text(rcx - lw * 0.5, b - 4.0, &label, legend_font, pause::MUTED);
    let rb = Rect::new(rcx - bar_w * 0.5, b - 1.0, bar_w, 4.0);
    p.border(rb, [0.0, 1.0, 1.0, 1.0], pause::MUTED);
    *c.fmt = label;
}

fn fit_scale(pane: Rect, b: [f32; 4]) -> f32 {
    ((pane.w - 48.0).max(1.0) / (b[2] - b[0]).max(1.0)).min((pane.h - 48.0).max(1.0) / (b[3] - b[1]).max(1.0))
}

/// The player arrow: a chevron pointing along the heading.
fn arrow(p: &mut Painter, x: f32, y: f32, heading: f32, size: f32, fill: Color, edge: Color) {
    let (s, c) = heading.sin_cos();
    let rot = |dx: f32, dy: f32| [x + dx * c - dy * s, y + dx * s + dy * c];
    let shape = |k: f32| [rot(0.0, -size * k), rot(size * 0.72 * k, size * 0.8 * k), rot(0.0, size * 0.38 * k), rot(-size * 0.72 * k, size * 0.8 * k)];
    p.polygon(&shape(1.25), edge);
    p.polygon(&shape(1.0), fill);
}

// ---------------------------------------------------------------------------
// HUD (src/ui/player-hud.css, player-hud.js, hud-map.js, index.html markup)
// ---------------------------------------------------------------------------

const CHIP_HOLD_S: f64 = 2.6;
const OBJECTIVE_HOLD_S: f64 = 4.2;
const WEAPON_NAME_HOLD_S: f64 = 1.8;

/// Opacity of an element shown at `since` for `hold` seconds, with the
/// HUD's 180 ms enter/exit; flags motion while it changes.
fn held(c: &mut Ctx, since: f64, hold: f64) -> f32 {
    let age = (c.now - since) as f32;
    let m = hud::MOTION_MS / 1000.0;
    let hold = hold as f32;
    if age < 0.0 {
        return 0.0;
    }
    if age <= hold + m {
        c.animating = true;
    }
    if age < m {
        hud::EASE.at(age / m)
    } else if age < hold {
        1.0
    } else {
        1.0 - hud::EASE_OUT.at(((age - hold) / m).min(1.0))
    }
}

/// The HUD's plate (`.hud-plate`): border, radius, surface, soft shadow.
fn plate(p: &mut Painter, r: Rect, sky: Color) {
    // --hud-shadow: 0 8px 24px rgba(3, 5, 6, 0.28): approximated by three
    // widening translucent layers (no blur pass).
    for k in 1..=3 {
        let e = k as f32 * 4.0;
        p.rrect(Rect::new(r.x - e, r.y + 8.0 - e, r.w + 2.0 * e, r.h + 2.0 * e), hud::RADIUS + e, rgba(3, 5, 6, 0.05));
    }
    p.rrect(r, hud::RADIUS, hud::surface(sky));
    p.rrect_stroke(r, hud::RADIUS, 1.0, hud::LINE);
}

fn star(p: &mut Painter, cx: f32, cy: f32, outer: f32, inner: f32, color: Color) {
    let mut pts = p.take_points();
    for i in 0..10 {
        let r = if i % 2 == 0 { outer } else { inner };
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        pts.push([cx + a.cos() * r, cy + a.sin() * r]);
    }
    p.polygon(&pts, color);
    p.give_points(pts);
}

/// A weapon silhouette from `WEAPON_SHAPES` (straight-segment SVG paths) on
/// its 34 x 16 box, scaled into `r`.
fn weapon_glyph(p: &mut Painter, kind: WeaponKind, r: Rect, color: Color) {
    let d = match kind {
        WeaponKind::Pistol | WeaponKind::Fists => "M6 2.6h3.4v1.4H6ZM6 3.4h24.4v4.4H6ZM6 7.8h15.4v2H6ZM6 9.8h8.2l-3.1 5.4H3Z",
        WeaponKind::Smg => "M1 4.8h4.6v3.2H1ZM5 4h18.4v4.8H5ZM23.4 5.4h7.2v2.2h-7.2ZM11.2 8.8h4v6.2h-4ZM5 8.8h4.6l-1.7 4.4H3.4Z",
        WeaponKind::Rifle => "M2.6 3.4h4.2v1.4H2.6ZM1 4.6h7.4v3.8H1ZM8 4.6h16.2v4.2H8ZM24.2 5.6h8.6v2.2h-8.6ZM29.4 4h1.6v1.8h-1.6ZM13 8.6h4.6l1.4 5.8h-4.6ZM8 8.6h4.2l-1.5 4.6H6.6Z",
        WeaponKind::Shotgun => "M1 4.8h7.4v3.8H1ZM8 4.2h10.4v4.8H8ZM10 4.2h23v2.4H10ZM18 6.9h9.4v2.3H18ZM8.4 9h4.2l-1.7 4.4H6.6Z",
        WeaponKind::Thrown => "M15 2.2h4.2v2.6H15ZM19.2 2.4h6.4l-.7 1.7-5.7 1Z",
    };
    let (sx, sy) = (r.w / 34.0, r.h / 16.0);
    let mut pts = p.take_points();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let mut cmd = b'M';
    let bytes = d.as_bytes();
    let mut i = 0;
    let num = |i: &mut usize| -> f32 {
        while *i < bytes.len() && (bytes[*i] == b' ' || bytes[*i] == b',') {
            *i += 1;
        }
        let s = *i;
        if *i < bytes.len() && (bytes[*i] == b'-' || bytes[*i] == b'+') {
            *i += 1;
        }
        let mut dot = false;
        while *i < bytes.len() && (bytes[*i].is_ascii_digit() || (bytes[*i] == b'.' && !dot)) {
            dot |= bytes[*i] == b'.';
            *i += 1;
        }
        d[s..*i].parse().unwrap_or(0.0)
    };
    while i < bytes.len() {
        let ch = bytes[i];
        if ch.is_ascii_alphabetic() {
            cmd = ch;
            i += 1;
            if cmd == b'Z' || cmd == b'z' {
                if pts.len() >= 3 {
                    for q in pts.iter_mut() {
                        *q = [r.x + q[0] * sx, r.y + q[1] * sy];
                    }
                    p.polygon(&pts, color);
                }
                pts.clear();
            }
            continue;
        }
        if ch == b' ' || ch == b',' {
            i += 1;
            continue;
        }
        match cmd {
            b'M' | b'L' => {
                x = num(&mut i);
                y = num(&mut i);
            }
            b'l' => {
                x += num(&mut i);
                y += num(&mut i);
            }
            b'h' => x += num(&mut i),
            b'H' => x = num(&mut i),
            b'v' => y += num(&mut i),
            b'V' => y = num(&mut i),
            _ => {
                i += 1;
                continue;
            }
        }
        pts.push([x, y]);
        if cmd == b'M' {
            cmd = b'L';
        }
    }
    p.give_points(pts);
}

fn hud_screen(p: &mut Painter, c: &mut Ctx) {
    let vp = c.vp;
    let (ex, ey) = edges(vp);
    let hs = c.hud;
    let fs_label = c.len(hud::FS_LABEL);
    let fs_body = c.len(hud::FS_BODY);
    let fs_value = c.len(hud::FS_VALUE);
    let fs_prompt = c.len(hud::FS_PROMPT);
    let pad = c.len(hud::PAD);
    let gap = c.len(hud::GAP);

    // Presentation memory: what changed and when.
    let now = c.now;
    if c.mem.cash != Some(hs.cash) {
        c.mem.cash_from = c.mem.cash.unwrap_or(hs.cash);
        c.mem.cash = Some(hs.cash);
        c.mem.cash_changed_at = now;
    }
    let clock_key = (hs.hour * 4.0).floor() as i32;
    if c.mem.clock_key != Some(clock_key) {
        c.mem.clock_key = Some(clock_key);
        c.mem.clock_at = now;
    }
    if c.mem.objective != hs.objective {
        c.mem.objective = hs.objective.clone();
        c.mem.objective_at = now;
    }
    let wkind = hs.weapon.map(|w| w.kind);
    if c.mem.weapon != wkind {
        c.mem.weapon = wkind;
        c.mem.weapon_at = now;
    }
    // Street plate: looked up again only after the player moves 4 m.
    let moved = (hs.player[0] - c.mem.street_at[0]).abs() + (hs.player[1] - c.mem.street_at[1]).abs();
    if moved > 4.0 || c.mem.street.is_empty() {
        c.mem.street_at = hs.player;
        let name = c.map.as_ref().and_then(|m| m.streets.nearest(hs.player[0], hs.player[1], 30.0)).map(|(n, _)| n).unwrap_or("");
        if c.mem.street != name {
            c.mem.street.clear();
            c.mem.street.push_str(name);
        }
    }

    // ---- Top-left: objective ------------------------------------------
    if let Some((title_text, meta)) = &hs.objective {
        let k = held(c, c.mem.objective_at, OBJECTIVE_HOLD_S);
        if k > 0.0 {
            let ts = st(Stack::HudDisplay, 650, fs_value).tracking(0.06).upper();
            let ms = st(Stack::HudSans, 650, fs_label).tracking(0.14).upper();
            let tw = p.text_width(title_text, ts).max(p.text_width(meta, ms));
            let r = Rect::new(ex - (1.0 - k) * 8.0, ey, 2.0 + (pad + 4.0) * 2.0 + tw, pad * 2.0 + fs_value * 1.1 + 4.0 + fs_label * 1.15);
            p.opacity = k;
            plate(p, r, hs.sky);
            p.rect(Rect::new(r.x, r.y + 1.0, 2.0, r.h - 2.0), hud::ACCENT);
            let b = p.text.baseline(ts, r.y + pad, 1.1);
            p.text(r.x + 2.0 + pad + 4.0, b, title_text, ts, hud::INK);
            let b = p.text.baseline(ms, r.y + pad + fs_value * 1.1 + 4.0, 1.15);
            p.text(r.x + 2.0 + pad + 4.0, b, meta, ms, hud::INK_MUTED);
            p.opacity = 1.0;
        }
    }

    // ---- Top-right: wanted stars, cash, then the clock chip -----------
    let right = vp.w - ex;
    let mut ty = ey;
    let star_h = c.len(hud::STAR);
    if hs.wanted > 0 {
        let sw = star_h * 7.14;
        let x0 = right - sw;
        let unit = sw / 100.0;
        p.radial(x0 + sw * 0.5, ty + star_h * 0.5, sw * 0.62 * 0.5 + 10.0, star_h * 1.2 * 0.5 + 5.0, &[(0.0, rgba(4, 6, 8, 0.52)), (0.7, rgba(4, 6, 8, 0.3)), (1.0, rgba(4, 6, 8, 0.0))]);
        for i in 0..hud::WANTED_STARS {
            let cx = x0 + (8.0 + i as f32 * 21.0) * unit;
            let cy = ty + 7.0 * unit;
            star(p, cx, cy, 6.4 * unit + 0.6, 2.7 * unit + 0.3, rgba(242, 240, 232, 0.3));
            star(p, cx, cy, 6.4 * unit, 2.7 * unit, if i < hs.wanted { hud::INK } else { rgba(4, 6, 8, 0.72) });
        }
        ty += star_h + 4.0;
    }
    let cash_k = held(c, c.mem.cash_changed_at, CHIP_HOLD_S);
    if cash_k > 0.0 {
        // Count-up over CASH_TICK_MS (520 ms).
        let tick = (((now - c.mem.cash_changed_at) as f32) / 0.52).clamp(0.0, 1.0);
        if tick < 1.0 {
            c.animating = true;
        }
        let shown = c.mem.cash_from as f64 + (hs.cash - c.mem.cash_from) as f64 * tick as f64;
        let amount = shown.round() as i64;
        c.fmt.clear();
        if amount < 0 {
            c.fmt.push('-');
        }
        c.fmt.push('$');
        let digits = amount.unsigned_abs();
        let mut groups = [0u16; 7];
        let mut n = 0;
        let mut v = digits;
        loop {
            groups[n] = (v % 1000) as u16;
            n += 1;
            v /= 1000;
            if v == 0 {
                break;
            }
        }
        for g in (0..n).rev() {
            if g == n - 1 {
                let _ = write!(c.fmt, "{}", groups[g]);
            } else {
                let _ = write!(c.fmt, ",{:03}", groups[g]);
            }
        }
        let text = std::mem::take(c.fmt);
        let cs = st(Stack::HudDisplay, 700, fs_body * 1.25).tracking(0.02);
        let tw = p.text_width(&text, cs);
        p.opacity = cash_k;
        let b = p.text.baseline(cs, ty + (1.0 - cash_k) * 2.0, 1.0);
        p.text(right - tw + 1.0, b + 1.0, &text, cs, rgba(0, 0, 0, 0.45));
        p.text(right - tw, b, &text, cs, hud::INK);
        p.opacity = 1.0;
        *c.fmt = text;
        ty += cs.size + gap;
    }
    let clock_k = held(c, c.mem.clock_at, CHIP_HOLD_S);
    if clock_k > 0.0 {
        let rounded = (hs.hour * 4.0).floor() / 4.0;
        let whole = rounded.floor() as i32;
        let minutes = ((rounded - whole as f32) * 60.0).round() as i32;
        let suffix = if whole >= 12 { "PM" } else { "AM" };
        let display = (whole + 11) % 12 + 1;
        c.fmt.clear();
        let _ = write!(c.fmt, "{display}:{minutes:02} {suffix}");
        let clock = std::mem::take(c.fmt);
        let vs = st(Stack::HudDisplay, 650, fs_value).tracking(0.04);
        let ms = st(Stack::HudSans, 650, fs_label).tracking(0.14).upper();
        let mode = if matches!(hs.mode, HudMode::Drive { .. }) { "Driving" } else { "On foot" };
        let w1 = p.text_width(&clock, vs);
        let w2 = p.text_width(mode, ms);
        let cw = pad * 2.0 + w1 + 10.0 + w2;
        let chh = (pad - 2.0) * 2.0 + fs_value;
        let r = Rect::new(right - cw + (1.0 - clock_k) * 8.0, ty, cw, chh);
        p.opacity = clock_k;
        plate(p, r, hs.sky);
        let b = p.text.baseline(vs, r.y + pad - 2.0, 1.0);
        p.text(r.x + pad, b, &clock, vs, hud::INK);
        p.text(r.x + pad + w1 + 10.0, b, mode, ms, hud::INK_MUTED);
        p.opacity = 1.0;
        *c.fmt = clock;
    }

    // ---- Bottom-left cluster: street, radar, bars, dock ---------------
    let (mw, mh) = hud_map_size(vp);
    let bar_h = c.len(hud::BAR_H);
    let bottom = vp.h - ey;
    let weapon_shown = hs.weapon.is_some();
    let name_k = if weapon_shown { held(c, c.mem.weapon_at, WEAPON_NAME_HOLD_S) } else { 0.0 };
    let dock_h = (fs_body + 1.0 + fs_label * 1.15).max(18.0);
    let dock_y = bottom - dock_h;
    let bars_y = dock_y - 4.0 - bar_h;
    let map_y = bars_y - 4.0 - mh;
    let x0 = ex;
    // Street plate over the radar's top-left corner.
    if !c.mem.street.is_empty() {
        let ss = st(Stack::HudDisplay, 700, c.len(hud::STREET_SIZE)).tracking(0.14).upper();
        let sy = map_y - 4.0 - ss.size * 1.1;
        let b = p.text.baseline(ss, sy, 1.1);
        let name = std::mem::take(&mut c.mem.street);
        p.text(x0 + 1.0 + 1.0, b + 1.0, &name, ss, rgba(0, 0, 0, 0.7));
        p.text(x0 + 1.0, b, &name, ss, hud::INK);
        c.mem.street = name;
    }
    // Radar: rim, frame, the projected map, the player marker, north pip.
    let outer = Rect::new(x0, map_y, mw, mh);
    p.rrect(Rect::new(outer.x - 1.0, outer.y + 2.0, outer.w + 2.0, outer.h + 8.0), 9.0, rgba(0, 0, 0, 0.18));
    p.rrect(Rect::new(outer.x - 1.0, outer.y - 1.0, outer.w + 2.0, outer.h + 2.0), 6.0, hud::RADAR_RIM);
    p.rrect(outer, 5.0, hud::RADAR_FRAME);
    let inner = outer.inset(2.0, 2.0, 2.0, 2.0);
    p.rrect(inner, 3.0, hud::MAP_GROUND);
    if let Some(m) = c.map.as_ref() {
        p.radar(inner, 3.0, m.hud_image);
    }
    // The player marker sits where the camera puts the player (74% down).
    let f = hud::FOCAL_RATIO * mh;
    let pitch = hud::CAM_PITCH_DEG.to_radians();
    let below = (hud::CAM_HEIGHT_M / hud::CAM_BACK_M).atan() - pitch;
    let marker_y = outer.y + outer.h * 0.5 + f * below.tan();
    arrow(p, outer.x + outer.w * 0.5, marker_y, 0.0, 7.5, hud::INK, hud::MAP_OUTLINE);
    // North pip on the rim, in the direction of north on the plate.
    let north = -hs.heading;
    let (sn, cn) = north.sin_cos();
    let (hw, hh) = (outer.w * 0.5 - 3.0 - 6.5, outer.h * 0.5 - 3.0 - 6.5);
    let k = (hw / sn.abs().max(1e-4)).min(hh / cn.abs().max(1e-4));
    let (nx, ny) = (outer.x + outer.w * 0.5 + sn * k, outer.y + outer.h * 0.5 - cn * k);
    p.ellipse(nx, ny, 6.5, 6.5, 0.0, hud::MAP_OUTLINE.with_a(0.9));
    let ns = st(Stack::HudDisplay, 700, 8.5);
    let nw = p.text_width("N", ns);
    let b = p.text.baseline(ns, ny - 8.5 * 0.55, 1.1);
    p.text(nx - nw * 0.5, b, "N", ns, hud::MAP_COMPASS_INK);
    // Health (and armour) bars.
    let has_armour = hs.armour > 0.0;
    let bw = if has_armour { (mw - 3.0) * 0.5 } else { mw };
    for (i, (frac, col)) in [(hs.health, hud::HEALTH_BAR), (hs.armour, hud::ARMOUR_BAR)].into_iter().enumerate() {
        if i == 1 && !has_armour {
            continue;
        }
        let r = Rect::new(x0 + i as f32 * (bw + 3.0), bars_y, bw, bar_h);
        p.rrect(r, 2.0, hud::RADAR_FRAME);
        let in_r = r.inset(1.0, 1.0, 1.0, 1.0);
        p.rect(in_r, rgba(4, 6, 8, 0.82));
        let fill_x = in_r.x + bar_h * 1.25;
        let low = i == 0 && frac < 0.25;
        p.rect(Rect::new(fill_x, in_r.y, (in_r.right() - fill_x) * frac.clamp(0.0, 1.0), in_r.h), if low { hud::DANGER } else { col });
        // The glyph cut into the bar's left end: a cross or a shield.
        let g = bar_h - 2.0;
        let gx = in_r.x + 1.0;
        let gy = in_r.y + (in_r.h - g) * 0.5;
        let white = rgba(255, 255, 255, 0.92);
        if i == 0 {
            p.rect(Rect::new(gx + g * 0.36, gy, g * 0.28, g), white);
            p.rect(Rect::new(gx, gy + g * 0.36, g, g * 0.28), white);
        } else {
            let sh = [[gx + g * 0.5, gy], [gx + g * 0.94, gy + g * 0.16], [gx + g * 0.94, gy + g * 0.47], [gx + g * 0.5, gy + g], [gx + g * 0.06, gy + g * 0.47], [gx + g * 0.06, gy + g * 0.16]];
            p.polygon(&sh, white);
        }
    }
    // Dock: weapon (glyph + ammo), name for a moment after a switch; health readout when low.
    if let Some(wp) = hs.weapon {
        let melee = wp.kind == WeaponKind::Fists;
        let gr = Rect::new(x0 + 2.0, dock_y + (fs_body - 13.0) * 0.5, 28.0, 13.0);
        let empty = !melee && wp.magazine == 0;
        weapon_glyph(p, wp.kind, gr.translate(0.0, 1.0), rgba(0, 0, 0, 0.6));
        weapon_glyph(p, wp.kind, gr, if empty { hud::DANGER } else { hud::INK });
        let mut ax = gr.right() + 8.0;
        if !melee {
            let ms = st(Stack::HudDisplay, 700, fs_body).tracking(0.01);
            let rs = st(Stack::HudDisplay, 650, fs_label);
            c.fmt.clear();
            let _ = write!(c.fmt, "{}", wp.magazine);
            let mag = std::mem::take(c.fmt);
            let b = p.text.baseline(ms, dock_y, 1.0);
            ax += p.text(ax, b, &mag, ms, if empty { hud::DANGER } else { hud::INK }) + 5.0;
            *c.fmt = mag;
            c.fmt.clear();
            let _ = write!(c.fmt, "/ {}", wp.reserve);
            let res = std::mem::take(c.fmt);
            p.text(ax, b, &res, rs, hud::INK_MUTED);
            *c.fmt = res;
        }
        if name_k > 0.0 {
            let ns = st(Stack::HudSans, 700, fs_label).tracking(0.14).upper();
            let name = match wp.kind {
                WeaponKind::Fists => "Fists",
                WeaponKind::Pistol => "Pistol",
                WeaponKind::Smg => "SMG",
                WeaponKind::Rifle => "Rifle",
                WeaponKind::Shotgun => "Shotgun",
                WeaponKind::Thrown => "Grenade",
            };
            p.opacity = name_k;
            let b = p.text.baseline(ns, dock_y + fs_body + 1.0, 1.15);
            p.text(x0 + 2.0, b, name, ns, hud::INK);
            p.opacity = 1.0;
        }
    }
    if hs.health < hud::HEALTH_READOUT_AT {
        let rs = st(Stack::HudDisplay, 700, fs_label);
        c.fmt.clear();
        let _ = write!(c.fmt, "{}", (hs.health * 100.0).round() as i32);
        let v = std::mem::take(c.fmt);
        let low = hs.health < 0.25;
        let b = p.text.baseline(rs, dock_y, 1.0);
        let vx = x0 + mw * 0.55;
        let adv = p.text(vx, b, &v, rs, if low { hud::DANGER } else { hud::INK_MUTED });
        p.text(vx + adv + 5.0, b, if low { "LOW" } else { "HURT" }, st(Stack::HudDisplay, 700, fs_label).tracking(0.14), if low { hud::DANGER } else { hud::ACCENT });
        *c.fmt = v;
    }

    // ---- Bottom-right: the vehicle cluster's speed readout, while driving.
    // A reduced port of vehicle-hud.js: the speed numeral and its unit on
    // the two-stop scrim; dials, gear, fuel and telltales are not ported.
    if let HudMode::Drive { speed_kph } = hs.mode {
        let size = c.len(hud::FS_CLUSTER);
        let ns = st(Stack::HudDisplay, hud::WEIGHT_HERO as u16, size);
        let ls = st(Stack::HudSans, 700, fs_label).tracking(0.14);
        c.fmt.clear();
        let _ = write!(c.fmt, "{}", speed_kph.max(0.0).round() as i32);
        let v = std::mem::take(c.fmt);
        let vw = p.text_width(&v, ns);
        let lw = p.text_width("KM/H", ls);
        let right_x = vp.w - ex;
        let base_y = vp.h - ey;
        let cx = right_x - vw.max(lw) * 0.5;
        let cy = base_y - size * 0.6;
        let r = vw.max(lw) * 0.9 + 40.0;
        p.radial(cx, cy, r, r * 0.8, &[(0.0, hud::VEH_SCRIM), (0.55, hud::VEH_SCRIM_FADE), (1.0, hud::VEH_SCRIM.with_a(0.0))]);
        let b = base_y - fs_label * 1.4;
        p.text(right_x - lw, base_y, "KM/H", ls, hud::VEH_INK_LABEL);
        p.text(right_x - vw, b, &v, ns, hud::INK);
        *c.fmt = v;
    }

    // ---- Centre-bottom prompt -----------------------------------------
    if let Some((key, text)) = &hs.prompt {
        let ps = st(Stack::HudSans, 650, fs_prompt).tracking(0.06).upper();
        let ks = st(Stack::HudMono, 700, fs_label).tracking(0.04);
        let kw = (p.text_width(key, ks) + 0.9 * fs_label).max(1.7 * fs_label);
        let kh = 1.7 * fs_label;
        let tw = p.text_width(text, ps);
        let w = (pad + 6.0) * 2.0 + kw + 12.0 + tw;
        let h = (pad - 1.0) * 2.0 + kh.max(fs_prompt * 1.2);
        let r = Rect::new(vp.w * 0.5 - w * 0.5, vp.h - ey - 4.0 - h, w, h);
        plate(p, r, hs.sky);
        let kr = Rect::new(r.x + pad + 6.0, r.y + (h - kh) * 0.5, kw, kh);
        p.rrect(kr, 3.0, hud::ACCENT.fade(0.14));
        p.rrect_stroke(kr, 3.0, 1.0, hud::line_accent());
        let kb = p.text.baseline(ks, kr.y + (kh - fs_label) * 0.5, 1.0);
        let kt = p.text_width(key, ks);
        p.text(kr.x + (kw - kt) * 0.5, kb, key, ks, hud::ACCENT);
        let b = p.text.baseline(ps, r.y + (h - fs_prompt * 1.2) * 0.5, 1.2);
        p.text(kr.right() + 12.0, b, text, ps, hud::INK);
    }
}
