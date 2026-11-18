//! The photo mode panel: a slim column at the right edge in the pause
//! menu's type and colours (`theme::pause`), the rule-of-thirds guides over
//! the picture, and a line when a photo is saved. The host owns every
//! setting and formats the rows; the panel only draws them (it takes no
//! input: the host's keys move the selection and change the values).
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

/// What the panel shows this frame.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PhotoPanel {
    /// One line under the title (lens, aperture, focus).
    pub summary: String,
    pub rows: Vec<PhotoRow>,
    pub selected: usize,
    /// (key, what it does) along the foot.
    pub hints: Vec<(String, String)>,
    /// A transient line (a saved photo), shown at the bottom of the picture.
    pub toast: Option<String>,
    /// Draw the rule-of-thirds guides.
    pub thirds: bool,
    /// The picture's aspect when letterboxed (the guides fit it).
    pub frame_aspect: Option<f32>,
    /// The column itself (hidden: only the guides and the toast).
    pub visible: bool,
}

/// The column's width (`--photo-panel-w`, sized like the pause rail).
pub const PANEL_W: Len = Len::Clamp(px(280.0), vw(19.0), px(360.0));

/// The picture's rectangle: the screen, or the letterbox's opening.
pub fn picture(vp: Viewport, aspect: Option<f32>) -> Rect {
    match aspect {
        Some(a) if a > vp.w / vp.h => {
            let h = vp.w / a;
            Rect::new(0.0, (vp.h - h) * 0.5, vp.w, h)
        }
        Some(a) => {
            let w = vp.h * a;
            Rect::new((vp.w - w) * 0.5, 0.0, w, vp.h)
        }
        None => Rect::new(0.0, 0.0, vp.w, vp.h),
    }
}

fn st(stack: Stack, weight: u16, size: f32) -> TextStyle {
    TextStyle::new(stack, weight, size)
}

pub(crate) fn draw(p: &mut Painter, vp: Viewport, panel: &PhotoPanel) {
    let pic = picture(vp, panel.frame_aspect);
    let edge = (vp.h * 0.045).clamp(16.0, 40.0);
    let w = PANEL_W.px(vp, theme::REM_PX);
    if panel.thirds {
        // hairlines with a dark under-stroke, readable on sky and on asphalt;
        // they stop at the open panel.
        let line = rgba(255, 255, 255, 0.42);
        let under = rgba(0, 0, 0, 0.22);
        let stop = if panel.visible { (vp.w - edge - w - 1.0).min(pic.right()) } else { pic.right() };
        for k in [1.0f32, 2.0] {
            let x = (pic.x + pic.w * k / 3.0).round();
            let y = (pic.y + pic.h * k / 3.0).round();
            if x + 2.0 < stop {
                p.rect(Rect::new(x + 1.0, pic.y, 1.0, pic.h), under);
                p.rect(Rect::new(x, pic.y, 1.0, pic.h), line);
            }
            p.rect(Rect::new(pic.x, y + 1.0, stop - pic.x, 1.0), under);
            p.rect(Rect::new(pic.x, y, stop - pic.x, 1.0), line);
        }
    }
    if let Some(t) = &panel.toast {
        let s = st(Stack::Pause, 500, 13.0).tracking(0.04);
        let tw = p.text_width(t, s);
        let r = Rect::new(pic.x + (pic.w - tw) * 0.5 - 16.0, pic.bottom() - 64.0, tw + 32.0, 13.0 * 1.15 + 18.0);
        p.rect(r, pause::NOTE_BG);
        p.rect(Rect::new(r.x, r.y, 3.0, r.h), pause::ACCENT);
        let b = p.text.baseline(s, r.y + 9.0, 1.15);
        p.text(r.x + 16.0, b, t, s, pause::INK);
    }
    if !panel.visible {
        return;
    }
    let label = st(Stack::PauseCondensed, 600, 12.0).tracking(0.14).upper();
    let value = st(Stack::Pause, 500, 14.0);
    let note = st(Stack::Pause, 400, 10.5).tracking(0.02);
    let hint = st(Stack::Pause, 400, 10.5);
    let row_h = |row: &PhotoRow| 10.0 + label.size * 1.15 + 6.0 + if row.frac.is_some() { 8.0 } else { 0.0 } + if row.note.is_some() { 3.0 + note.size * 1.15 } else { 0.0 } + 10.0;
    let total: f32 = panel.rows.iter().map(row_h).sum();
    let pad = 18.0;
    // the column fits its rows (scrolling them only when the screen is too short).
    let head_total = 28.0 + 6.0 + if panel.summary.is_empty() { 0.0 } else { 12.0 * 1.15 + 10.0 } + 1.0 + 9.0;
    let foot_h = 14.0 + 2.0 * (hint.size * 1.15 + 10.0);
    let h = (2.0 * pad + head_total + total + foot_h + 8.0).min(vp.h - 2.0 * edge);
    let r = Rect::new(vp.w - edge - w, edge, w, h);
    p.linear(r, Dir::ToRight, &[(0.0, rgba(5, 15, 24, 0.84)), (1.0, rgba(5, 15, 24, 0.9))]);
    p.border(r, [1.0; 4], pause::PAGES_BORDER);
    let x = r.x + pad;
    let iw = r.w - 2.0 * pad;
    let mut y = r.y + pad;
    // head: the double bar and "Photo mode", as the pause menu's "Paused".
    let ts = st(Stack::Pause, 500, 14.0).tracking(pause::TITLE_TRACKING).upper();
    let head_h = 28.0;
    p.rect(Rect::new(x, y + head_h * 0.5 - 7.5, 3.0, 15.0), pause::ACCENT);
    p.rect(Rect::new(x + 6.0, y + head_h * 0.5 - 7.5, 3.0, 15.0), pause::ACCENT);
    let b = p.text.baseline(ts, y + (head_h - ts.size * 1.15) * 0.5, 1.15);
    p.text(x + 20.0, b, "Photo mode", ts, pause::INK);
    y += head_h + 6.0;
    if !panel.summary.is_empty() {
        let ss = st(Stack::Pause, 400, 12.0).tracking(0.03);
        let b = p.text.baseline(ss, y, 1.15);
        p.text(x, b, &panel.summary, ss, pause::MUTED);
        y += ss.size * 1.15 + 10.0;
    }
    p.rect(Rect::new(x, y, iw, 1.0), pause::LINE);
    y += 9.0;
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
    for (i, row) in panel.rows.iter().enumerate() {
        let h = row_h(row);
        let sel = i == panel.selected;
        let rr = Rect::new(r.x + 1.0, ry, r.w - 2.0, h);
        if sel {
            p.linear(rr, Dir::ToRight, &[(0.0, rgba(226, 173, 97, 0.18)), (1.0, rgba(226, 173, 97, 0.02))]);
            p.rect(Rect::new(rr.x, rr.y, 3.0, rr.h), pause::ACCENT);
        }
        let mut yy = ry + 10.0;
        let b = p.text.baseline(label, yy, 1.15);
        p.text(x, b, &row.label, label, if sel { pause::INK } else { pause::MUTED });
        // the value right-aligned, with the arrows of a choice when selected.
        let vb = p.text.baseline(value, yy - 1.0, 1.15);
        let vw_ = p.text_width(&row.value, value);
        let vx = x + iw - vw_ - if sel { 14.0 } else { 0.0 };
        if sel {
            let ar = st(Stack::Pause, 400, 13.0);
            let lw = p.text_width("\u{2039}", ar);
            p.text(vx - 8.0 - lw, vb, "\u{2039}", ar, pause::ACCENT);
            p.text(x + iw - 8.0, vb, "\u{203a}", ar, pause::ACCENT);
        }
        p.text(vx, vb, &row.value, value, if sel { pause::ACCENT } else { pause::INK });
        yy += label.size * 1.15 + 6.0;
        if let Some(f) = row.frac {
            let f = f.clamp(0.0, 1.0);
            p.rect(Rect::new(x, yy, iw, 2.0), pause::SLIDER_RULE);
            p.rect(Rect::new(x, yy, (iw * f).max(1.0), 2.0), if sel { pause::ACCENT } else { rgba(237, 245, 251, 0.55) });
            p.rect(Rect::new(x + iw * f - 1.0, yy - 3.0, 2.0, 8.0), if sel { pause::ACCENT } else { pause::INK });
            yy += 8.0;
        }
        if let Some(n) = &row.note {
            let b = p.text.baseline(note, yy + 3.0, 1.15);
            p.text(x, b, n, note, theme::hex(0x8ea3b4));
        }
        if !sel && i + 1 < panel.rows.len() && i + 1 != panel.selected {
            p.rect(Rect::new(x, ry + h - 1.0, iw, 1.0), rgba(196, 216, 235, 0.07));
        }
        ry += h;
    }
    p.set_scissor(None, screen);
    // foot: the keys, two lines.
    let fy = rows_bottom + 8.0;
    p.rect(Rect::new(x, fy, iw, 1.0), pause::LINE);
    let mut hx = x;
    let mut hy = fy + 10.0;
    let kh = hint.size * 1.15 + 6.0;
    for (k, what) in &panel.hints {
        let kw = (p.text_width(k, hint) + 10.0).max(18.0);
        let ww = p.text_width(what, hint);
        if hx + kw + 6.0 + ww > x + iw && hx > x {
            hx = x;
            hy += kh + 6.0;
        }
        p.border(Rect::new(hx, hy, kw, kh), [1.0; 4], pause::KBD_BORDER);
        let b = p.text.baseline(hint, hy + 3.0, 1.15);
        let tw = p.text_width(k, hint);
        p.text(hx + (kw - tw) * 0.5, b, k, hint, pause::INK);
        p.text(hx + kw + 6.0, b, what, hint, pause::MUTED);
        hx += kw + 6.0 + ww + 12.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_the_letterbox_opening() {
        let vp = Viewport { w: 1920.0, h: 1080.0 };
        assert_eq!(picture(vp, None), Rect::new(0.0, 0.0, 1920.0, 1080.0));
        let scope = picture(vp, Some(2.39));
        assert!((scope.h - 1920.0 / 2.39).abs() < 1e-3 && (scope.y - (1080.0 - scope.h) * 0.5).abs() < 1e-3);
        let square = picture(vp, Some(1.0));
        assert!((square.w - 1080.0).abs() < 1e-3 && (square.x - 420.0).abs() < 1e-3);
    }
}
