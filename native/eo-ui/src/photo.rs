//! The photo mode panel: a column at the right (or left) edge in the pause
//! menu's type and colours (`theme::pause`), its settings on three pages
//! (Camera, Look, Frame) under tabs like the pause menu's, the
//! rule-of-thirds guides over the picture, the focus reticle where the
//! focus is taken, and a line when a photo is saved. The host owns every
//! setting and formats the rows; the panel only draws them (it takes no
//! input: the host's keys move the selection and change the values). It
//! fades back while the camera moves (`alpha`).
//!
//! While a panel is set the HUD is not drawn (photo mode hides the HUD).

use crate::draw::{Dir, Painter, Rect};
use crate::text::{Stack, TextStyle};
use crate::theme::{self, Len, Viewport, pause, px, rgba, vw};

/// One row: a label, its value as text, and where the value sits in its range.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoRow {
    pub label: String,
    pub value: String,
    /// 0..1 along the row's range (a slider), None for a choice.
    pub frac: Option<f32>,
    /// A small line under the row (for example "Only you see this").
    pub note: Option<String>,
}

impl PhotoRow {
    pub fn new(label: &str, value: String, frac: Option<f32>) -> Self {
        Self { label: label.to_string(), value, frac, note: None }
    }
    pub fn note(mut self, n: &str) -> Self {
        self.note = Some(n.to_string());
        self
    }
}

/// Where the focus is taken, on the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Reticle {
    /// 0..1 across and down the screen.
    pub x: f32,
    pub y: f32,
    /// The lens has reached the distance (the brackets turn the accent colour).
    pub locked: bool,
    /// The distance, under the brackets.
    pub label: String,
}

/// What the panel shows this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoPanel {
    /// One line under the title (lens, aperture, focus).
    pub summary: String,
    /// The pages' names and the one shown.
    pub tabs: Vec<String>,
    pub tab: usize,
    /// The shown page's rows.
    pub rows: Vec<PhotoRow>,
    pub selected: usize,
    /// (key, what it does) along the foot.
    pub hints: Vec<(String, String)>,
    /// A transient line (a saved photo), shown at the bottom of the picture.
    pub toast: Option<String>,
    /// Draw the rule-of-thirds guides.
    pub thirds: bool,
    /// The picture's aspect when letterboxed (the guides fit it), and where its opening sits (-1 top .. 1 bottom).
    pub frame_aspect: Option<f32>,
    pub frame_offset: f32,
    /// The column itself (hidden: only the guides, the reticle and the toast).
    pub visible: bool,
    /// The column at the left edge (else the right).
    pub dock_left: bool,
    /// The column's opacity (it fades back while the camera moves).
    pub alpha: f32,
    pub reticle: Option<Reticle>,
}

impl Default for PhotoPanel {
    fn default() -> Self {
        Self {
            summary: String::new(),
            tabs: vec![],
            tab: 0,
            rows: vec![],
            selected: 0,
            hints: vec![],
            toast: None,
            thirds: false,
            frame_aspect: None,
            frame_offset: 0.0,
            visible: true,
            dock_left: false,
            alpha: 1.0,
            reticle: None,
        }
    }
}

/// The column's width (`--photo-panel-w`, a little wider than the pause rail).
pub const PANEL_W: Len = Len::Clamp(px(330.0), vw(21.0), px(420.0));
/// The column's backing: the pause pages' blue-black, nearly opaque (readable over sky and asphalt).
const BACKING: [(f32, theme::Color); 2] = [(0.0, rgba(5, 15, 24, 0.9)), (1.0, rgba(5, 15, 24, 0.94))];

/// The picture's rectangle: the screen, or the letterbox's opening.
pub fn picture(vp: Viewport, aspect: Option<f32>, offset: f32) -> Rect {
    let o = offset.clamp(-1.0, 1.0);
    match aspect {
        Some(a) if a > vp.w / vp.h => {
            let h = vp.w / a;
            Rect::new(0.0, (vp.h - h) * 0.5 * (1.0 + o), vp.w, h)
        }
        Some(a) => {
            let w = vp.h * a;
            Rect::new((vp.w - w) * 0.5 * (1.0 + o), 0.0, w, vp.h)
        }
        None => Rect::new(0.0, 0.0, vp.w, vp.h),
    }
}

fn st(stack: Stack, weight: u16, size: f32) -> TextStyle {
    TextStyle::new(stack, weight, size)
}

/// The AF brackets: four corners of a square, a dark under-stroke, the distance beneath.
fn reticle(p: &mut Painter, vp: Viewport, r: &Reticle) {
    let (cx, cy) = ((r.x * vp.w).round(), (r.y * vp.h).round());
    let s = (vp.h * 0.03).clamp(18.0, 40.0);
    let arm = (s * 0.42).round();
    let col = if r.locked { pause::ACCENT } else { rgba(255, 255, 255, 0.85) };
    let under = rgba(0, 0, 0, 0.35);
    for (dx, dy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let x = cx + dx * s;
        let y = cy + dy * s;
        for (c, o) in [(under, 1.0), (col, 0.0)] {
            // a horizontal and a vertical arm into the square from each corner.
            let hx = if dx < 0.0 { x } else { x - arm };
            let vy = if dy < 0.0 { y } else { y - arm };
            p.rect(Rect::new(hx + o, y - 1.0 + o, arm, 2.0), c);
            p.rect(Rect::new(x - 1.0 + o, vy + o, 2.0, arm), c);
        }
    }
    if !r.label.is_empty() {
        let ls = st(Stack::Pause, 500, 14.0).tracking(0.04);
        let w = p.text_width(&r.label, ls);
        let b = p.text.baseline(ls, cy + s + 6.0, 1.15);
        p.text(cx - w * 0.5 + 1.0, b + 1.0, &r.label, ls, under);
        p.text(cx - w * 0.5, b, &r.label, ls, col);
    }
}

pub(crate) fn draw(p: &mut Painter, vp: Viewport, panel: &PhotoPanel) {
    let pic = picture(vp, panel.frame_aspect, panel.frame_offset);
    let edge = (vp.h * 0.045).clamp(16.0, 40.0);
    let w = PANEL_W.px(vp, theme::REM_PX);
    let alpha = panel.alpha.clamp(0.0, 1.0);
    let open = panel.visible && alpha > 0.0;
    let px0 = if panel.dock_left { edge } else { vp.w - edge - w };
    if panel.thirds {
        // hairlines with a dark under-stroke, readable on sky and on asphalt;
        // they stop at the open panel.
        let line = rgba(255, 255, 255, 0.42);
        let under = rgba(0, 0, 0, 0.22);
        let (l, r) = match (open, panel.dock_left) {
            (true, false) => (pic.x, (px0 - 1.0).min(pic.right())),
            (true, true) => ((px0 + w + 1.0).max(pic.x), pic.right()),
            _ => (pic.x, pic.right()),
        };
        for k in [1.0f32, 2.0] {
            let x = (pic.x + pic.w * k / 3.0).round();
            let y = (pic.y + pic.h * k / 3.0).round();
            if x - 2.0 > l && x + 2.0 < r {
                p.rect(Rect::new(x + 1.0, pic.y, 1.0, pic.h), under);
                p.rect(Rect::new(x, pic.y, 1.0, pic.h), line);
            }
            p.rect(Rect::new(l, y + 1.0, r - l, 1.0), under);
            p.rect(Rect::new(l, y, r - l, 1.0), line);
        }
    }
    if let Some(r) = &panel.reticle {
        reticle(p, vp, r);
    }
    if let Some(t) = &panel.toast {
        let s = st(Stack::Pause, 500, 15.0).tracking(0.04);
        let tw = p.text_width(t, s);
        let r = Rect::new(pic.x + (pic.w - tw) * 0.5 - 18.0, pic.bottom() - 72.0, tw + 36.0, 15.0 * 1.15 + 20.0);
        p.rect(r, pause::NOTE_BG);
        p.rect(Rect::new(r.x, r.y, 3.0, r.h), pause::ACCENT);
        let b = p.text.baseline(s, r.y + 10.0, 1.15);
        p.text(r.x + 18.0, b, t, s, pause::INK);
    }
    if !open {
        return;
    }
    let keep = p.opacity;
    p.opacity = keep * alpha;
    // type at 1080p: labels 14, values 17, notes, keys and the summary 14-15 (readable from a sofa).
    let label = st(Stack::PauseCondensed, 600, 14.0).tracking(0.12).upper();
    let value = st(Stack::Pause, 500, 17.0);
    let note = st(Stack::Pause, 400, 14.0).tracking(0.01);
    let hint = st(Stack::Pause, 500, 14.0);
    let tab = st(Stack::PauseCondensed, 600, 14.0).tracking(pause::TAB_TRACKING).upper();
    let row_h = |row: &PhotoRow| 10.0 + value.size * 1.15 + 4.0 + if row.frac.is_some() { 9.0 } else { 0.0 } + if row.note.is_some() { 2.0 + note.size * 1.2 } else { 0.0 } + 9.0;
    let total: f32 = panel.rows.iter().map(row_h).sum();
    let pad = 20.0;
    let iw = w - 2.0 * pad;
    // the foot's keys, laid out first (their lines set its height).
    let kh = hint.size * 1.15 + 8.0;
    let mut lines = 1;
    {
        let mut hx = 0.0;
        for (k, what) in &panel.hints {
            let kw = (p.text_width(k, hint) + 12.0).max(22.0);
            let ww = p.text_width(what, hint);
            if hx + kw + 7.0 + ww > iw && hx > 0.0 {
                hx = 0.0;
                lines += 1;
            }
            hx += kw + 7.0 + ww + 14.0;
        }
    }
    let foot_h = 14.0 + lines as f32 * (kh + 7.0);
    let head_h = 30.0;
    let tabs_h = if panel.tabs.is_empty() { 0.0 } else { tab.size * 1.15 + 16.0 };
    let summary_h = if panel.summary.is_empty() { 0.0 } else { 15.0 * 1.15 + 10.0 };
    let head_total = head_h + 6.0 + summary_h + tabs_h + 1.0 + 6.0;
    let h = (2.0 * pad + head_total + total + foot_h + 8.0).min(vp.h - 2.0 * edge);
    let r = Rect::new(px0, edge, w, h);
    p.linear(r, if panel.dock_left { Dir::ToLeft } else { Dir::ToRight }, &BACKING);
    p.border(r, [1.0; 4], pause::PAGES_BORDER);
    let x = r.x + pad;
    let mut y = r.y + pad;
    // head: the double bar and "Photo mode", as the pause menu's "Paused".
    let ts = st(Stack::Pause, 500, 16.0).tracking(pause::TITLE_TRACKING).upper();
    p.rect(Rect::new(x, y + head_h * 0.5 - 8.0, 3.0, 16.0), pause::ACCENT);
    p.rect(Rect::new(x + 6.0, y + head_h * 0.5 - 8.0, 3.0, 16.0), pause::ACCENT);
    let b = p.text.baseline(ts, y + (head_h - ts.size * 1.15) * 0.5, 1.15);
    p.text(x + 21.0, b, "Photo mode", ts, pause::INK);
    y += head_h + 6.0;
    if !panel.summary.is_empty() {
        let ss = st(Stack::Pause, 500, 15.0).tracking(0.02);
        let b = p.text.baseline(ss, y, 1.15);
        p.text(x, b, &panel.summary, ss, pause::MUTED);
        y += summary_h;
    }
    // the pages: upper-case condensed names, the shown one in ink over an accent rule.
    if !panel.tabs.is_empty() {
        let mut tx = x;
        let b = p.text.baseline(tab, y + 2.0, 1.15);
        for (i, name) in panel.tabs.iter().enumerate() {
            let tw = p.text_width(name, tab);
            let on = i == panel.tab;
            p.text(tx, b, name, tab, if on { pause::INK } else { pause::MUTED });
            if on {
                p.rect(Rect::new(tx, y + tab.size * 1.15 + 6.0, tw, 2.0), pause::ACCENT);
            }
            tx += tw + 22.0;
        }
        y += tabs_h;
    }
    p.rect(Rect::new(x, y, iw, 1.0), pause::LINE);
    y += 7.0;
    // rows, scrolled so the selected one is in view.
    let rows_bottom = r.bottom() - pad - foot_h;
    let avail = rows_bottom - y;
    let mut scroll = 0.0;
    if total > avail {
        let before: f32 = panel.rows.iter().take(panel.selected).map(row_h).sum();
        let sel_h = panel.rows.get(panel.selected).map(row_h).unwrap_or(0.0);
        scroll = (before + sel_h * 0.5 - avail * 0.5).clamp(0.0, total - avail);
    }
    let screen = [vp.w, vp.h];
    p.set_scissor(Some(Rect::new(r.x, y, r.w, avail)), screen);
    let mut ry = y - scroll;
    let arrow = st(Stack::Pause, 500, 17.0);
    let (lw, rw) = (p.text_width("\u{2039}", arrow), p.text_width("\u{203a}", arrow));
    for (i, row) in panel.rows.iter().enumerate() {
        let h = row_h(row);
        let sel = i == panel.selected;
        let rr = Rect::new(r.x + 1.0, ry, r.w - 2.0, h);
        if sel {
            p.linear(rr, Dir::ToRight, &[(0.0, rgba(226, 173, 97, 0.2)), (1.0, rgba(226, 173, 97, 0.03))]);
            p.rect(Rect::new(rr.x, rr.y, 3.0, rr.h), pause::ACCENT);
        }
        let yy = ry + 10.0;
        // the label and the value on one line (the label's smaller caps on the value's baseline).
        let vb = p.text.baseline(value, yy, 1.15);
        p.text(x, vb, &row.label, label, if sel { pause::INK } else { pause::MUTED });
        // the value right-aligned between the arrows every row carries (the accent on the selected one).
        let vw_ = p.text_width(&row.value, value);
        let right = x + iw;
        let vx = right - rw - 8.0 - vw_;
        let ac = if sel { pause::ACCENT } else { rgba(172, 187, 200, 0.45) };
        p.text(vx - 8.0 - lw, vb, "\u{2039}", arrow, ac);
        p.text(right - rw, vb, "\u{203a}", arrow, ac);
        p.text(vx, vb, &row.value, value, if sel { pause::ACCENT } else { pause::INK });
        let mut ly = yy + value.size * 1.15 + 4.0;
        if let Some(f) = row.frac {
            let f = f.clamp(0.0, 1.0);
            p.rect(Rect::new(x, ly + 1.0, iw, 2.0), pause::SLIDER_RULE);
            p.rect(Rect::new(x, ly + 1.0, (iw * f).max(1.0), 2.0), if sel { pause::ACCENT } else { rgba(237, 245, 251, 0.55) });
            p.rect(Rect::new(x + iw * f - 1.0, ly - 2.0, 2.0, 8.0), if sel { pause::ACCENT } else { pause::INK });
            ly += 9.0;
        }
        if let Some(n) = &row.note {
            let b = p.text.baseline(note, ly + 2.0, 1.2);
            p.text(x, b, n, note, theme::hex(0x9fb0bd));
        }
        if !sel && i + 1 < panel.rows.len() && i + 1 != panel.selected {
            p.rect(Rect::new(x, ry + h - 1.0, iw, 1.0), rgba(196, 216, 235, 0.08));
        }
        ry += h;
    }
    p.set_scissor(None, screen);
    // foot: the keys.
    let fy = rows_bottom + 8.0;
    p.rect(Rect::new(x, fy, iw, 1.0), pause::LINE);
    let mut hx = x;
    let mut hy = fy + 12.0;
    for (k, what) in &panel.hints {
        let kw = (p.text_width(k, hint) + 12.0).max(22.0);
        let ww = p.text_width(what, hint);
        if hx + kw + 7.0 + ww > x + iw && hx > x {
            hx = x;
            hy += kh + 7.0;
        }
        p.border(Rect::new(hx, hy, kw, kh), [1.0; 4], pause::KBD_BORDER);
        let b = p.text.baseline(hint, hy + 4.0, 1.15);
        let tw = p.text_width(k, hint);
        p.text(hx + (kw - tw) * 0.5, b, k, hint, pause::INK);
        p.text(hx + kw + 7.0, b, what, hint, pause::MUTED);
        hx += kw + 7.0 + ww + 14.0;
    }
    p.opacity = keep;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_the_letterbox_opening() {
        let vp = Viewport { w: 1920.0, h: 1080.0 };
        assert_eq!(picture(vp, None, 0.0), Rect::new(0.0, 0.0, 1920.0, 1080.0));
        let scope = picture(vp, Some(2.39), 0.0);
        assert!((scope.h - 1920.0 / 2.39).abs() < 1e-3 && (scope.y - (1080.0 - scope.h) * 0.5).abs() < 1e-3);
        // moved down to the bottom edge, and up to the top.
        let low = picture(vp, Some(2.39), 1.0);
        assert!((low.bottom() - 1080.0).abs() < 1e-3, "{low:?}");
        assert!(picture(vp, Some(2.39), -1.0).y.abs() < 1e-3);
        let square = picture(vp, Some(1.0), 0.0);
        assert!((square.w - 1080.0).abs() < 1e-3 && (square.x - 420.0).abs() < 1e-3);
    }
}
