//! the browser game's sign legends, cooked on the CPU into RGBA
//! atlases the renderer uploads (no GPU here). A port of the JS canvas
//! drawing, call for call:
//!
//! * [`sign_face_atlas`]: `buildSignFaceAtlas` (src/render/street-sign-faces.js),
//!   one 256 px cell per SFMTA inventory face in index order, the `DRAW` table,
//!   outline = cell alpha. Tiles = `signFaceTile(face, index)`.
//! * [`street_name_atlas`]: the green corner-pole name blades,
//!   `planStreetSignAtlas` + `buildStreetSignAtlas` (passes/street-furniture.js),
//!   with [`normalize_street_legend`] and [`sign_street_legends`].
//! * [`prop_sign_sheet`]: `buildSignAtlas`, the shared 1024 px prop sign sheet
//!   (`SIGN_ATLAS`: two legacy blades, bus stop, news box, regulatory plate,
//!   block number, pedestrian face, meter, menu board, work zone, the static
//!   countdown face, bin decal, utility label, front page).
//! * [`printed_sign_atlas`]: `printedSignAtlasTexture` (passes/street-overhead.js),
//!   the kerb sign picket's printed faces (`PRINTED_SIGN_FACES`,
//!   `drawPrintedSignFace`).
//!
//! The reference is the live checkout's JS (`EO_REPO_ROOT`), which has no
//! plate-back / post-zinc finish cells: neither atlas paints them.
//! * [`ped_symbol_texture`]: `createPedestrianSymbolTexture`
//!   (passes/traffic-pedestrian-heads.js), byte for byte (it is built from data).
//! * [`ped_digit_atlas`]: the live pedestrian heads draw their countdown as
//!   seven-segment quads (`SEGMENT_BOXES`, `DIGIT_SEGMENTS`, `DIGIT_X`), not
//!   glyphs; this rasterises those same boxes into ten cells, and the tables
//!   are exported ([`DIGIT_SEGMENTS`], [`SEGMENT_BOXES`], [`DIGIT_X`]) for a
//!   renderer that draws the quads the way the JS does.
//!
//! Conventions of every [`Atlas`]: `rgba` is sRGB-encoded (except where
//! `srgb` is false), straight (not premultiplied) alpha, rows top first (the
//! canvas's row 0 first). A tile is `[u0, v0, du, dv]` in the JS / three.js
//! convention: u to the right, **v up from the bottom row** (a CanvasTexture
//! with flipY). For a wgpu texture uploaded rows-top-first, the texel row of
//! v is `(1 - v) * height`: sample at `(u0 + s * du, 1 - (v0 + t * dv))` for
//! the quad's own (s, t) with t up.
//!
//! Text: the JS stack `"Helvetica Neue", Helvetica, Arial, sans-serif` is
//! resolved against the operating system's font folders at run time (as
//! eo-ui does; no font file is copied into the repo). Weight 600 and 700
//! take the bold face (CSS font matching above 500 goes heavier first: Arial
//! Bold on Windows), 500 and below the regular. Glyph outlines from
//! `ab_glyph`, filled with `tiny-skia` as the canvas fills paths; kerning
//! from the font's `kern` table; `textBaseline = 'middle'` as Chromium places
//! it ((ascent - descent) / 2 above the baseline). Pixels are not the
//! browser's pixels (different rasteriser, no hinting, possibly a different
//! font file): the layout, colours and shapes are the JS's.
//!
//! No California dimension is drawn here: the faces are the JS's artwork.

use ab_glyph::{Font, FontVec, OutlineCurve};
use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

use crate::JsProps;
use crate::data::sha256_hex;

// the official artwork (FHWA SHS 2024, Caltrans CA codes) and Overpass, verified from outside the repo.
#[path = "sign_art.rs"]
pub mod art;
#[path = "sign_svg.rs"]
pub mod svg;

/// Bumped when the cooked output of this module changes for the same inputs
/// (the cache key also hashes this file's bytes). signs: v2 adds the official
/// artwork path and the provenance list; the drawn faces are unchanged.
/// v3: the official-art path draws the guide plate as MUNI BUS STOP again.
pub const SIGN_ATLAS_VERSION: &str = "eo-props-sign-atlas-v3";

// ---------------------------------------------------------------------------
// the atlas
// ---------------------------------------------------------------------------

/// A cooked atlas. See the module docs for the colour, row and tile conventions.
#[derive(Clone, Debug, PartialEq)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, rows top first, straight alpha.
    pub rgba: Vec<u8>,
    /// `[u0, v0, du, dv]`, v up from the bottom row (three.js convention).
    pub tiles: Vec<[f32; 4]>,
    /// True when `rgba` is sRGB-encoded colour (upload as `*UnormSrgb`);
    /// false for data (the pedestrian symbol mask, `NoColorSpace` in the JS).
    pub srgb: bool,
}

impl Atlas {
    /// The texel at (x, y), y down from the top row.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * self.width + x) * 4) as usize;
        [self.rgba[o], self.rgba[o + 1], self.rgba[o + 2], self.rgba[o + 3]]
    }

    /// A tile's pixel rectangle `[x0, y0, x1, y1]` (y down, exclusive end).
    pub fn tile_pixels(&self, tile: [f32; 4]) -> [u32; 4] {
        let (w, h) = (self.width as f32, self.height as f32);
        let x0 = (tile[0] * w).floor().max(0.0) as u32;
        let x1 = ((tile[0] + tile[2]) * w).ceil().min(w) as u32;
        let y0 = ((1.0 - tile[1] - tile[3]) * h).floor().max(0.0) as u32;
        let y1 = ((1.0 - tile[1]) * h).ceil().min(h) as u32;
        [x0, y0, x1, y1]
    }

    /// Writes the atlas as an 8-bit RGBA PNG (tmp file + rename).
    pub fn write_png(&self, path: &Path) -> Result<()> {
        let bytes = encode_png(self.width, self.height, &self.rgba, self.srgb)?;
        write_atomic(path, &bytes)
    }
}

fn encode_png(width: u32, height: u32, rgba: &[u8], srgb: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        if srgb {
            enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        }
        let mut w = enc.write_header()?;
        w.write_image_data(rgba)?;
        w.finish()?;
    }
    Ok(out)
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = dec.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| anyhow!("png too large"))?];
    let info = reader.next_frame(&mut buf)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        bail!("png is not 8-bit RGBA");
    }
    buf.truncate(info.buffer_size());
    Ok((info.width, info.height, buf))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename to {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fonts
// ---------------------------------------------------------------------------

/// The JS sign stack, `"Helvetica Neue", Helvetica, Arial, sans-serif`,
/// resolved to a regular and a bold face from the system font folders.
pub struct Fonts {
    regular: FontVec,
    bold: FontVec,
    pub regular_file: PathBuf,
    pub bold_file: PathBuf,
    /// SHA-256 of the two files' bytes (part of the cache key).
    pub digest: String,
}

/// System font folders, in lookup order (eo-ui's `font_dirs`, with its
/// `EO_UI_FONT_DIR` override).
fn font_dirs() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Ok(d) = std::env::var("EO_UI_FONT_DIR") {
        v.push(PathBuf::from(d));
    }
    if cfg!(windows) {
        let win = std::env::var("WINDIR").or_else(|_| std::env::var("SystemRoot")).unwrap_or_else(|_| "C:\\Windows".into());
        v.push(PathBuf::from(win).join("Fonts"));
        if let Ok(l) = std::env::var("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts"));
        }
    } else {
        v.push(PathBuf::from("/usr/share/fonts/truetype"));
        v.push(PathBuf::from("/usr/share/fonts/truetype/msttcorefonts"));
        v.push(PathBuf::from("/usr/share/fonts/truetype/liberation"));
        v.push(PathBuf::from("/usr/share/fonts/truetype/dejavu"));
        v.push(PathBuf::from("/usr/share/fonts"));
        v.push(PathBuf::from("/Library/Fonts"));
        v.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
    }
    v
}

/// The stack's families in order, each as (regular file names, bold file names).
/// Only single-face .ttf/.otf files are tried (no .ttc collections).
const SANS_STACK: &[(&str, &[&str], &[&str])] = &[
    ("Helvetica Neue", &["HelveticaNeue.ttf", "HelveticaNeue.otf", "HelveticaNeue-Regular.ttf"], &["HelveticaNeue-Bold.ttf", "HelveticaNeue-Bold.otf", "HelveticaNeueBold.ttf"]),
    ("Helvetica", &["Helvetica.ttf", "Helvetica.otf"], &["Helvetica-Bold.ttf", "Helvetica-Bold.otf", "HelveticaBold.ttf"]),
    ("Arial", &["arial.ttf", "Arial.ttf", "ARIAL.TTF"], &["arialbd.ttf", "Arial Bold.ttf", "ARIALBD.TTF"]),
    ("sans-serif", &["LiberationSans-Regular.ttf", "DejaVuSans.ttf"], &["LiberationSans-Bold.ttf", "DejaVuSans-Bold.ttf"]),
];

fn find(dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

impl Fonts {
    /// The first family of the stack with both a regular and a bold face installed.
    pub fn system() -> Result<Self> {
        let dirs = font_dirs();
        for (family, regular, bold) in SANS_STACK {
            if let (Some(r), Some(b)) = (find(&dirs, regular), find(&dirs, bold)) {
                return Self::from_files(&r, &b).with_context(|| format!("font family {family}"));
            }
        }
        bail!("none of \"Helvetica Neue\", Helvetica, Arial, sans-serif is installed (searched {dirs:?}; set EO_UI_FONT_DIR)")
    }

    pub fn from_files(regular: &Path, bold: &Path) -> Result<Self> {
        let rb = std::fs::read(regular).with_context(|| format!("read {}", regular.display()))?;
        let bb = std::fs::read(bold).with_context(|| format!("read {}", bold.display()))?;
        Self::from_bytes(rb, bb, regular.into(), bold.into())
    }

    /// from bytes already read (and verified) by the caller.
    pub fn from_bytes(rb: Vec<u8>, bb: Vec<u8>, regular: PathBuf, bold: PathBuf) -> Result<Self> {
        let mut both = rb.clone();
        both.extend_from_slice(&bb);
        let digest = sha256_hex(&both);
        let regular_font = FontVec::try_from_vec(rb).map_err(|e| anyhow!("{}: {e}", regular.display()))?;
        let bold_font = FontVec::try_from_vec(bb).map_err(|e| anyhow!("{}: {e}", bold.display()))?;
        Ok(Self { regular: regular_font, bold: bold_font, regular_file: regular.into(), bold_file: bold.into(), digest })
    }

    fn face(&self, bold: bool) -> &FontVec {
        if bold { &self.bold } else { &self.regular }
    }
}

/// A CSS weight keyword or number to the bold face (CSS Fonts 4 §5.2 against a
/// regular + bold family: 600 and above go to the bold).
fn is_bold(weight: &str) -> bool {
    match weight {
        "bold" | "bolder" => true,
        w => w.parse::<u32>().map(|n| n >= 600).unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// a small 2D canvas over tiny-skia (the subset the JS drawing calls)
// ---------------------------------------------------------------------------

type Rgba = [f32; 4];

fn hex(s: &str) -> Rgba {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).expect("colour");
    [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0, 1.0]
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Center,
}

#[derive(Clone)]
struct State {
    fill: Rgba,
    stroke: Rgba,
    line_width: f64,
    cap: LineCap,
    alpha: f64,
    transform: Transform,
    bold: bool,
    px: f64,
    align: Align,
}

#[derive(Clone, Copy)]
enum Cmd {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

struct Canvas<'a> {
    pix: Pixmap,
    st: State,
    stack: Vec<State>,
    path: Vec<Cmd>,
    cur: Option<(f64, f64)>,
    start: Option<(f64, f64)>,
    fonts: &'a Fonts,
}

const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

impl<'a> Canvas<'a> {
    fn new(width: u32, height: u32, fonts: &'a Fonts) -> Result<Self> {
        let pix = Pixmap::new(width, height).ok_or_else(|| anyhow!("pixmap {width}x{height}"))?;
        let st = State {
            fill: [0.0, 0.0, 0.0, 1.0],
            stroke: [0.0, 0.0, 0.0, 1.0],
            line_width: 1.0,
            cap: LineCap::Butt,
            alpha: 1.0,
            transform: Transform::identity(),
            bold: false,
            px: 10.0,
            align: Align::Left,
        };
        Ok(Self { pix, st, stack: vec![], path: vec![], cur: None, start: None, fonts })
    }

    fn save(&mut self) {
        self.stack.push(self.st.clone());
    }
    fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.st = s;
        }
    }
    fn translate(&mut self, x: f64, y: f64) {
        self.st.transform = self.st.transform.pre_translate(x as f32, y as f32);
    }
    fn scale(&mut self, x: f64, y: f64) {
        self.st.transform = self.st.transform.pre_scale(x as f32, y as f32);
    }
    fn fill_style(&mut self, c: &str) {
        self.st.fill = hex(c);
    }
    fn stroke_style(&mut self, c: &str) {
        self.st.stroke = hex(c);
    }
    fn font(&mut self, weight: &str, px: f64) {
        self.st.bold = is_bold(weight);
        self.st.px = px;
    }

    fn paint(&self, c: Rgba) -> Paint<'static> {
        let mut p = Paint::default();
        p.anti_alias = true;
        let a = (c[3] as f64 * self.st.alpha).clamp(0.0, 1.0) as f32;
        p.set_color(Color::from_rgba(c[0], c[1], c[2], a).unwrap_or(Color::BLACK));
        p
    }

    fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
        let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
        if !(w > 0.0 && h > 0.0) {
            return;
        }
        let Some(r) = Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) else { return };
        let paint = self.paint(self.st.fill);
        let ts = self.st.transform;
        if ts.is_identity() {
            self.pix.fill_rect(r, &paint, ts, None);
        } else {
            let path = PathBuilder::from_rect(r);
            self.pix.fill_path(&path, &paint, FillRule::Winding, ts, None);
        }
    }

    fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.begin_path();
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close_path();
        self.stroke();
        self.begin_path();
    }

    fn begin_path(&mut self) {
        self.path.clear();
        self.cur = None;
        self.start = None;
    }
    fn move_to(&mut self, x: f64, y: f64) {
        self.path.push(Cmd::Move(x, y));
        self.cur = Some((x, y));
        self.start = Some((x, y));
    }
    fn line_to(&mut self, x: f64, y: f64) {
        if self.cur.is_none() {
            self.move_to(x, y);
            return;
        }
        self.path.push(Cmd::Line(x, y));
        self.cur = Some((x, y));
    }
    fn close_path(&mut self) {
        if self.cur.is_some() {
            self.path.push(Cmd::Close);
            self.cur = self.start;
        }
    }

    /// Elliptical arc segments (rotation 0) from angle `a0` sweeping `sweep` (signed, radians, y down).
    fn arc_sweep(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, sweep: f64) {
        let start = (cx + rx * a0.cos(), cy + ry * a0.sin());
        if self.cur.is_some() {
            self.line_to(start.0, start.1);
        } else {
            self.move_to(start.0, start.1);
        }
        if sweep == 0.0 {
            return;
        }
        let n = (sweep.abs() / (PI / 2.0)).ceil().max(1.0) as usize;
        let step = sweep / n as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let mut a = a0;
        for _ in 0..n {
            let b = a + step;
            let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
            let p1 = (cx + rx * (ca - k * sa), cy + ry * (sa + k * ca));
            let p2 = (cx + rx * (cb + k * sb), cy + ry * (sb - k * cb));
            let p3 = (cx + rx * cb, cy + ry * sb);
            self.path.push(Cmd::Cubic(p1.0, p1.1, p2.0, p2.1, p3.0, p3.1));
            self.cur = Some(p3);
            a = b;
        }
    }

    /// `ctx.arc(cx, cy, r, a0, a1)` (clockwise on screen, anticlockwise = false).
    fn arc(&mut self, cx: f64, cy: f64, r: f64, a0: f64, a1: f64) {
        self.ellipse(cx, cy, r, r, a0, a1);
    }

    /// `ctx.ellipse(cx, cy, rx, ry, 0, a0, a1)`.
    fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64) {
        let sweep = if a1 - a0 >= TAU {
            TAU
        } else {
            let s = (a1 - a0) % TAU;
            if s < 0.0 { s + TAU } else { s }
        };
        self.arc_sweep(cx, cy, rx, ry, a0, sweep);
    }

    /// `ctx.arcTo(x1, y1, x2, y2, r)`.
    fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, r: f64) {
        let Some((x0, y0)) = self.cur else {
            self.move_to(x1, y1);
            return;
        };
        let (v1x, v1y) = (x0 - x1, y0 - y1);
        let (v2x, v2y) = (x2 - x1, y2 - y1);
        let (l1, l2) = (v1x.hypot(v1y), v2x.hypot(v2y));
        let cross = v1x * v2y - v1y * v2x;
        if l1 < 1e-12 || l2 < 1e-12 || r <= 0.0 || cross.abs() < 1e-12 * l1 * l2 {
            self.line_to(x1, y1);
            return;
        }
        let (u1x, u1y, u2x, u2y) = (v1x / l1, v1y / l1, v2x / l2, v2y / l2);
        let theta = (u1x * u2x + u1y * u2y).clamp(-1.0, 1.0).acos();
        let d = r / (theta / 2.0).tan();
        let t1 = (x1 + u1x * d, y1 + u1y * d);
        let t2 = (x1 + u2x * d, y1 + u2y * d);
        let (bx, by) = (u1x + u2x, u1y + u2y);
        let bl = bx.hypot(by);
        let dist = r / (theta / 2.0).sin();
        let c = (x1 + bx / bl * dist, y1 + by / bl * dist);
        let a = (t1.1 - c.1).atan2(t1.0 - c.0);
        let b = (t2.1 - c.1).atan2(t2.0 - c.0);
        let mut sweep = b - a;
        while sweep > PI {
            sweep -= TAU;
        }
        while sweep <= -PI {
            sweep += TAU;
        }
        self.line_to(t1.0, t1.1);
        self.arc_sweep(c.0, c.1, r, r, a, sweep);
    }

    fn build(&self) -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        for c in &self.path {
            match *c {
                Cmd::Move(x, y) => pb.move_to(x as f32, y as f32),
                Cmd::Line(x, y) => pb.line_to(x as f32, y as f32),
                Cmd::Cubic(a, b, c, d, e, f) => pb.cubic_to(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32),
                Cmd::Close => pb.close(),
            }
        }
        pb.finish()
    }

    fn fill(&mut self) {
        if let Some(path) = self.build() {
            let paint = self.paint(self.st.fill);
            self.pix.fill_path(&path, &paint, FillRule::Winding, self.st.transform, None);
        }
    }

    fn stroke(&mut self) {
        if let Some(path) = self.build() {
            let paint = self.paint(self.st.stroke);
            let stroke = Stroke {
                width: self.st.line_width as f32,
                miter_limit: 10.0,
                line_cap: self.st.cap,
                line_join: LineJoin::Miter,
                dash: None,
            };
            self.pix.stroke_path(&path, &paint, &stroke, self.st.transform, None);
        }
    }

    /// `polygon(c, points)` of street-sign-faces.js: a closed path.
    fn polygon(&mut self, pts: &[(f64, f64)]) {
        self.begin_path();
        for (i, &(x, y)) in pts.iter().enumerate() {
            if i == 0 { self.move_to(x, y) } else { self.line_to(x, y) }
        }
        self.close_path();
    }

    // --- text -------------------------------------------------------------

    /// `measureText(text).width` at the current font.
    fn measure(&self, text: &str) -> f64 {
        let font = self.fonts.face(self.st.bold);
        let s = self.st.px / font.units_per_em().unwrap_or(1000.0) as f64;
        let mut w = 0.0;
        let mut prev = None;
        for ch in text.chars() {
            let g = font.glyph_id(ch);
            if let Some(p) = prev {
                w += font.kern_unscaled(p, g) as f64 * s;
            }
            w += font.h_advance_unscaled(g) as f64 * s;
            prev = Some(g);
        }
        w
    }

    /// `fillText(text, x, y)` with `textBaseline = 'middle'` and the current align.
    fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        let font = self.fonts.face(self.st.bold);
        let s = self.st.px / font.units_per_em().unwrap_or(1000.0) as f64;
        let width = self.measure(text);
        let mut pen = match self.st.align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
        };
        let ascent = font.ascent_unscaled() as f64;
        let descent = -(font.descent_unscaled() as f64).abs();
        let baseline = y + (ascent + descent) / 2.0 * s;
        let mut pb = PathBuilder::new();
        let mut any = false;
        let mut prev = None;
        for ch in text.chars() {
            let g = font.glyph_id(ch);
            if let Some(p) = prev {
                pen += font.kern_unscaled(p, g) as f64 * s;
            }
            if let Some(outline) = font.outline(g) {
                let map = |p: ab_glyph::Point| ((pen + p.x as f64 * s) as f32, (baseline - p.y as f64 * s) as f32);
                let mut last: Option<(f32, f32)> = None;
                for curve in &outline.curves {
                    let (a, rest): ((f32, f32), Vec<(f32, f32)>) = match *curve {
                        OutlineCurve::Line(p0, p1) => (map(p0), vec![map(p1)]),
                        OutlineCurve::Quad(p0, p1, p2) => (map(p0), vec![map(p1), map(p2)]),
                        OutlineCurve::Cubic(p0, p1, p2, p3) => (map(p0), vec![map(p1), map(p2), map(p3)]),
                    };
                    let joined = last.is_some_and(|l| (l.0 - a.0).abs() < 1e-4 && (l.1 - a.1).abs() < 1e-4);
                    if !joined {
                        if last.is_some() {
                            pb.close();
                        }
                        pb.move_to(a.0, a.1);
                    }
                    match rest.len() {
                        1 => pb.line_to(rest[0].0, rest[0].1),
                        2 => pb.quad_to(rest[0].0, rest[0].1, rest[1].0, rest[1].1),
                        _ => pb.cubic_to(rest[0].0, rest[0].1, rest[1].0, rest[1].1, rest[2].0, rest[2].1),
                    }
                    last = rest.last().copied();
                    any = true;
                }
                if last.is_some() {
                    pb.close();
                }
            }
            pen += font.h_advance_unscaled(g) as f64 * s;
            prev = Some(g);
        }
        if !any {
            return;
        }
        if let Some(path) = pb.finish() {
            let paint = self.paint(self.st.fill);
            self.pix.fill_path(&path, &paint, FillRule::Winding, self.st.transform, None);
        }
    }

    fn finish(self, tiles: Vec<[f32; 4]>) -> Atlas {
        let (width, height) = (self.pix.width(), self.pix.height());
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for p in self.pix.pixels() {
            let c = p.demultiply();
            rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        Atlas { width, height, rgba, tiles, srgb: true }
    }
}

/// JS `Math.round` (half up).
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

// ---------------------------------------------------------------------------
// sign faces (src/render/street-sign-faces.js)
// ---------------------------------------------------------------------------

/// `SIGN_FACE_ATLAS`.
pub const SIGN_FACE_CELL: u32 = 256;
pub const SIGN_FACE_COLUMNS: u32 = 8;
pub const SIGN_FACE_SIZE: u32 = 2048;

const RED: &str = "#c4122f";
const GREEN: &str = "#0b6e3c";
const BLACK: &str = "#111111";
const WHITE: &str = "#f4f4f0";
const YELLOW: &str = "#f3c316";
const FYG: &str = "#c4df1d";
const BLUE: &str = "#1c4f9c";

/// `SIGN_FACE_ASPECT` (width / height); 1 for a face it does not list.
pub fn sign_face_aspect(face: &str) -> f64 {
    match face {
        "stop" => 1.0,
        "allWay" => 3.0,
        "streetCleaning" => 18.0 / 21.0,
        "permitParking" | "timeParking" | "noParking" | "towAway" => 18.0 / 24.0,
        "oneWayRight" | "oneWayLeft" => 3.0,
        "speedLimit" => 24.0 / 30.0,
        "noLeftTurn" | "noRightTurn" | "school" | "schoolAhead" | "pedestrian" | "warning" | "doNotEnter" => 1.0,
        "regulatory" | "guide" | "parkingAngle" | "noDoubleParking" => 18.0 / 24.0,
        "bikeGuide" => 24.0 / 18.0,
        "transitLane" => 24.0 / 30.0,
        "disabledParking" => 12.0 / 18.0,
        _ => 1.0,
    }
}

/// The faces the JS `DRAW` table draws (`SIGN_FACE_NAMES`), in its order.
pub const SIGN_FACE_NAMES: [&str; 24] = [
    "stop", "allWay", "streetCleaning", "permitParking", "timeParking", "noParking", "towAway", "oneWayRight", "oneWayLeft",
    "speedLimit", "noLeftTurn", "noRightTurn", "school", "schoolAhead", "pedestrian", "warning", "regulatory", "doNotEnter",
    "guide", "bikeGuide", "transitLane", "parkingAngle", "disabledParking", "noDoubleParking",
];

/// `faceRect(face, index)`: the face's drawn rectangle in its cell, pixels (y down).
pub fn sign_face_rect(face: &str, index: usize) -> [f64; 4] {
    let cell = SIGN_FACE_CELL as f64;
    let cols = SIGN_FACE_COLUMNS as usize;
    let aspect = sign_face_aspect(face);
    let w = if aspect >= 1.0 { cell } else { cell * aspect };
    let h = if aspect >= 1.0 { cell / aspect } else { cell };
    [(index % cols) as f64 * cell + (cell - w) / 2.0, (index / cols) as f64 * cell + (cell - h) / 2.0, w, h]
}

/// `signFaceTile(face, index)`: `[u0, v0, du, dv]`, v up, one texel inside the face.
pub fn sign_face_tile(face: &str, index: usize) -> [f32; 4] {
    let size = SIGN_FACE_SIZE as f64;
    let [x, y, w, h] = sign_face_rect(face, index);
    let inset = 1.0;
    [((x + inset) / size) as f32, (1.0 - (y + h - inset) / size) as f32, ((w - 2.0 * inset) / size) as f32, ((h - 2.0 * inset) / size) as f32]
}

/// `lines(c, rows, x, y, w, h)`: rows of centred text, each fitted by 2 px steps.
fn lines(c: &mut Canvas, rows: &[(&str, &str, &str, f64)], x: f64, y: f64, w: f64, h: f64) {
    let total: f64 = rows.iter().map(|r| r.3).sum();
    let mut cursor = y;
    for &(text, colour, weight, rel) in rows {
        let row_h = (h * rel) / total;
        let mut px = row_h * 0.78;
        c.font(weight, px);
        while c.measure(text) > w && px > 8.0 {
            px -= 2.0;
            c.font(weight, px);
        }
        c.fill_style(colour);
        c.st.align = Align::Center;
        c.fill_text(text, x + w / 2.0, cursor + row_h / 2.0);
        cursor += row_h;
    }
}

fn octagon(x: f64, y: f64, s: f64) -> Vec<(f64, f64)> {
    let k = s * 0.2929;
    vec![(x + k, y), (x + s - k, y), (x + s, y + k), (x + s, y + s - k), (x + s - k, y + s), (x + k, y + s), (x, y + s - k), (x, y + k)]
}

fn arrow(c: &mut Canvas, x: f64, y: f64, w: f64, h: f64, right: bool, colour: &str) {
    let shaft_h = h * 0.36;
    let head = w * 0.3;
    let cy = y + h / 2.0;
    let pts = if right {
        vec![(x, cy - shaft_h / 2.0), (x + w - head, cy - shaft_h / 2.0), (x + w - head, y), (x + w, cy), (x + w - head, y + h), (x + w - head, cy + shaft_h / 2.0), (x, cy + shaft_h / 2.0)]
    } else {
        vec![(x + w, cy - shaft_h / 2.0), (x + head, cy - shaft_h / 2.0), (x + head, y), (x, cy), (x + head, y + h), (x + head, cy + shaft_h / 2.0), (x + w, cy + shaft_h / 2.0)]
    };
    c.fill_style(colour);
    c.polygon(&pts);
    c.fill();
}

fn prohibition(c: &mut Canvas, cx: f64, cy: f64, r: f64) {
    c.stroke_style(RED);
    c.st.line_width = r * 0.16;
    c.begin_path();
    c.arc(cx, cy, r, 0.0, TAU);
    c.stroke();
    c.begin_path();
    c.move_to(cx - r * 0.7, cy - r * 0.7);
    c.line_to(cx + r * 0.7, cy + r * 0.7);
    c.stroke();
}

fn figure(c: &mut Canvas, cx: f64, cy: f64, s: f64, colour: &str) {
    c.fill_style(colour);
    c.begin_path();
    c.arc(cx, cy - s * 0.42, s * 0.11, 0.0, TAU);
    c.fill();
    c.stroke_style(colour);
    c.st.line_width = s * 0.12;
    c.st.cap = LineCap::Round;
    c.begin_path();
    c.move_to(cx, cy - s * 0.28);
    c.line_to(cx, cy + s * 0.1);
    c.move_to(cx, cy + s * 0.1);
    c.line_to(cx - s * 0.18, cy + s * 0.45);
    c.move_to(cx, cy + s * 0.1);
    c.line_to(cx + s * 0.18, cy + s * 0.45);
    c.move_to(cx - s * 0.22, cy - s * 0.12);
    c.line_to(cx + s * 0.22, cy - s * 0.02);
    c.stroke();
}

/// `rect(c, x, y, w, h, fill, border, inset)`.
fn face_rect(c: &mut Canvas, x: f64, y: f64, w: f64, h: f64, fill: &str, border: &str, inset: f64) {
    let b = w.min(h) * inset;
    c.fill_style(border);
    c.fill_rect(x, y, w, h);
    c.fill_style(fill);
    c.fill_rect(x + b, y + b, w - 2.0 * b, h - 2.0 * b);
}

fn text_plate(c: &mut Canvas, x: f64, y: f64, w: f64, h: f64, fill: &str, border: &str, rows: &[(&str, &str, &str, f64)]) {
    face_rect(c, x, y, w, h, fill, border, 0.04);
    lines(c, rows, x + w * 0.08, y + h * 0.06, w * 0.84, h * 0.88);
}

fn diamond(c: &mut Canvas, x: f64, y: f64, s: f64, fill: &str) {
    c.fill_style(BLACK);
    c.polygon(&[(x + s / 2.0, y), (x + s, y + s / 2.0), (x + s / 2.0, y + s), (x, y + s / 2.0)]);
    c.fill();
    c.fill_style(fill);
    c.polygon(&[(x + s / 2.0, y + s * 0.04), (x + s * 0.96, y + s / 2.0), (x + s / 2.0, y + s * 0.96), (x + s * 0.04, y + s / 2.0)]);
    c.fill();
}

fn one_way(c: &mut Canvas, x: f64, y: f64, w: f64, h: f64, right: bool) {
    face_rect(c, x, y, w, h, BLACK, WHITE, 0.06);
    arrow(c, x + w * 0.04, y + h * 0.16, w * 0.92, h * 0.68, right, WHITE);
    lines(c, &[("ONE WAY", BLACK, "700", 1.0)], x + w * if right { 0.08 } else { 0.3 }, y + h * 0.38, w * 0.56, h * 0.24);
}

fn no_turn(c: &mut Canvas, x: f64, y: f64, w: f64, right: bool) {
    let s = w;
    face_rect(c, x, y, s, s, WHITE, BLACK, 0.03);
    c.stroke_style(BLACK);
    c.st.line_width = s * 0.08;
    let k = if right { 1.0 } else { -1.0 };
    c.begin_path();
    c.move_to(x + s * (0.5 - 0.08 * k), y + s * 0.8);
    c.line_to(x + s * (0.5 - 0.08 * k), y + s * 0.45);
    c.line_to(x + s * (0.5 + 0.15 * k), y + s * 0.45);
    c.stroke();
    arrow(c, x + s * if right { 0.6 } else { 0.2 }, y + s * 0.35, s * 0.2, s * 0.2, right, BLACK);
    prohibition(c, x + s / 2.0, y + s / 2.0, s * 0.36);
}

fn draw_school(c: &mut Canvas, x: f64, y: f64, w: f64) {
    let s = w;
    c.fill_style(BLACK);
    c.polygon(&[(x + s / 2.0, y), (x + s, y + s * 0.42), (x + s, y + s), (x, y + s), (x, y + s * 0.42)]);
    c.fill();
    c.fill_style(FYG);
    c.polygon(&[(x + s / 2.0, y + s * 0.05), (x + s * 0.96, y + s * 0.44), (x + s * 0.96, y + s * 0.96), (x + s * 0.04, y + s * 0.96), (x + s * 0.04, y + s * 0.44)]);
    c.fill();
    figure(c, x + s * 0.38, y + s * 0.62, s * 0.42, BLACK);
    figure(c, x + s * 0.62, y + s * 0.66, s * 0.36, BLACK);
}

/// The `DRAW` table. False when the face has no drawing (its cell stays clear).
fn draw_face(c: &mut Canvas, face: &str, x: f64, y: f64, w: f64, h: f64) -> bool {
    match face {
        "stop" => {
            c.fill_style(WHITE);
            c.polygon(&octagon(x, y, w));
            c.fill();
            c.fill_style(RED);
            c.polygon(&octagon(x + w * 0.03, y + w * 0.03, w * 0.94));
            c.fill();
            lines(c, &[("STOP", WHITE, "700", 1.0)], x + w * 0.08, y + w * 0.3, w * 0.84, w * 0.4);
        }
        "allWay" => {
            face_rect(c, x, y, w, h, RED, WHITE, 0.08);
            lines(c, &[("ALL WAY", WHITE, "700", 1.0)], x + w * 0.06, y + h * 0.1, w * 0.88, h * 0.8);
        }
        "streetCleaning" => text_plate(c, x, y, w, h, WHITE, RED, &[
            ("NO", RED, "700", 1.0), ("PARKING", RED, "700", 1.1), ("TUES 8AM-10AM", BLACK, "700", 0.8),
            ("1ST & 3RD OF MONTH", BLACK, "600", 0.6), ("STREET", RED, "700", 0.8), ("CLEANING", RED, "700", 0.8),
        ]),
        "permitParking" => text_plate(c, x, y, w, h, WHITE, GREEN, &[
            ("2 HOUR", GREEN, "700", 1.1), ("PARKING", GREEN, "700", 1.0), ("8AM - 6PM", GREEN, "700", 0.8),
            ("MON THRU FRI", GREEN, "700", 0.7), ("VEHICLES WITH", GREEN, "600", 0.55), ("AREA PERMITS", GREEN, "600", 0.55), ("EXEMPT", GREEN, "700", 0.6),
        ]),
        "timeParking" => text_plate(c, x, y, w, h, WHITE, GREEN, &[
            ("1 HOUR", GREEN, "700", 1.2), ("PARKING", GREEN, "700", 1.0), ("7AM - 6PM", GREEN, "700", 0.8), ("EXCEPT SUNDAY", GREEN, "600", 0.7),
        ]),
        "noParking" => {
            face_rect(c, x, y, w, h, WHITE, RED, 0.04);
            lines(c, &[("P", BLACK, "700", 1.0)], x + w * 0.25, y + h * 0.08, w * 0.5, w * 0.5);
            prohibition(c, x + w / 2.0, y + h * 0.08 + w * 0.25, w * 0.24);
            lines(c, &[("ANY", RED, "700", 1.0), ("TIME", RED, "700", 1.0)], x + w * 0.1, y + h * 0.58, w * 0.8, h * 0.34);
        }
        "towAway" => text_plate(c, x, y, w, h, WHITE, RED, &[
            ("TOW-AWAY", RED, "700", 1.0), ("NO", RED, "700", 1.0), ("STOPPING", RED, "700", 1.0), ("4PM - 6PM", BLACK, "700", 0.8),
            ("EXCEPT SAT & SUN", BLACK, "600", 0.6),
        ]),
        "oneWayRight" => one_way(c, x, y, w, h, true),
        "oneWayLeft" => one_way(c, x, y, w, h, false),
        "speedLimit" => text_plate(c, x, y, w, h, WHITE, BLACK, &[("SPEED", BLACK, "700", 1.0), ("LIMIT", BLACK, "700", 1.0), ("25", BLACK, "700", 2.4)]),
        "noLeftTurn" => no_turn(c, x, y, w, false),
        "noRightTurn" => no_turn(c, x, y, w, true),
        "school" => draw_school(c, x, y, w),
        "schoolAhead" => {
            draw_school(c, x, y, w);
            lines(c, &[("AHEAD", BLACK, "700", 1.0)], x + w * 0.15, y + w * 0.84, w * 0.7, w * 0.1);
        }
        "pedestrian" => {
            diamond(c, x, y, w, FYG);
            figure(c, x + w / 2.0, y + w * 0.52, w * 0.5, BLACK);
        }
        "warning" => {
            diamond(c, x, y, w, YELLOW);
            c.fill_style(BLACK);
            c.begin_path();
            c.ellipse(x + w / 2.0, y + w * 0.56, w * 0.22, w * 0.09, PI, 0.0);
            c.fill();
            c.fill_rect(x + w * 0.22, y + w * 0.56, w * 0.56, w * 0.04);
        }
        "regulatory" => text_plate(c, x, y, w, h, WHITE, BLACK, &[
            ("NO", BLACK, "700", 1.0), ("STOPPING", BLACK, "700", 1.0), ("EXCEPT", BLACK, "600", 0.7), ("COMMERCIAL", BLACK, "600", 0.7), ("VEHICLES", BLACK, "600", 0.7),
        ]),
        "doNotEnter" => {
            let s = w;
            face_rect(c, x, y, s, s, WHITE, WHITE, 0.0);
            c.fill_style(RED);
            c.begin_path();
            c.arc(x + s / 2.0, y + s / 2.0, s * 0.46, 0.0, TAU);
            c.fill();
            c.fill_style(WHITE);
            c.fill_rect(x + s * 0.16, y + s * 0.45, s * 0.68, s * 0.1);
            lines(c, &[("DO NOT", WHITE, "700", 1.0)], x + s * 0.2, y + s * 0.24, s * 0.6, s * 0.16);
            lines(c, &[("ENTER", WHITE, "700", 1.0)], x + s * 0.2, y + s * 0.6, s * 0.6, s * 0.16);
        }
        "guide" => text_plate(c, x, y, w, h, GREEN, WHITE, &[("MUNI", WHITE, "700", 1.0), ("BUS", WHITE, "700", 1.0), ("STOP", WHITE, "700", 1.0)]),
        "bikeGuide" => {
            face_rect(c, x, y, w, h, GREEN, WHITE, 0.04);
            c.stroke_style(WHITE);
            c.st.line_width = h * 0.05;
            for wx in [0.32, 0.68] {
                c.begin_path();
                c.arc(x + w * wx, y + h * 0.42, h * 0.17, 0.0, TAU);
                c.stroke();
            }
            c.begin_path();
            c.move_to(x + w * 0.32, y + h * 0.42);
            c.line_to(x + w * 0.47, y + h * 0.22);
            c.line_to(x + w * 0.68, y + h * 0.42);
            c.move_to(x + w * 0.47, y + h * 0.22);
            c.line_to(x + w * 0.5, y + h * 0.42);
            c.stroke();
            lines(c, &[("BIKE ROUTE", WHITE, "700", 1.0)], x + w * 0.08, y + h * 0.68, w * 0.84, h * 0.24);
        }
        "transitLane" => text_plate(c, x, y, w, h, WHITE, BLACK, &[("RIGHT LANE", BLACK, "700", 1.0), ("BUSES", BLACK, "700", 1.0), ("TAXIS", BLACK, "700", 1.0), ("ONLY", BLACK, "700", 1.0)]),
        "parkingAngle" => text_plate(c, x, y, w, h, WHITE, BLACK, &[("PARK AT", BLACK, "700", 1.0), ("90\u{b0}", BLACK, "700", 1.4), ("ANGLE", BLACK, "700", 1.0)]),
        "disabledParking" => {
            face_rect(c, x, y, w, h, BLUE, WHITE, 0.04);
            c.stroke_style(WHITE);
            c.st.line_width = w * 0.06;
            c.begin_path();
            c.arc(x + w * 0.48, y + h * 0.6, w * 0.24, 0.3, PI * 1.7);
            c.stroke();
            figure(c, x + w * 0.52, y + h * 0.42, w * 0.5, WHITE);
        }
        "noDoubleParking" => text_plate(c, x, y, w, h, WHITE, RED, &[("NO", RED, "700", 1.0), ("DOUBLE", RED, "700", 1.0), ("PARKING", RED, "700", 1.0), ("ANY TIME", BLACK, "700", 0.8)]),
        _ => return false,
    }
    true
}

/// `buildSignFaceAtlas(faces)`: 2048 px, clear, one 256 px cell per face in
/// index order (a face the `DRAW` table does not know stays clear).
/// `tiles[i]` is `signFaceTile(faces[i], i)`.
pub fn sign_face_atlas_for(faces: &[String], fonts: &Fonts) -> Result<Atlas> {
    let size = SIGN_FACE_SIZE;
    let mut c = Canvas::new(size, size, fonts)?;
    for (i, face) in faces.iter().enumerate() {
        let [x, y, w, h] = sign_face_rect(face, i);
        c.save();
        draw_face(&mut c, face, x, y, w, h);
        c.restore();
    }
    let tiles = faces.iter().enumerate().map(|(i, f)| sign_face_tile(f, i)).collect();
    Ok(c.finish(tiles))
}

/// The face atlas for the SFMTA inventory's faces (`SF_STREET_SIGNS.faces`,
/// `js.signs.faces`): the plate `variant` indexes `tiles`.
pub fn sign_face_atlas(js: &JsProps, fonts: &Fonts) -> Result<Atlas> {
    sign_face_atlas_for(&js.signs.faces, fonts)
}

// ---------------------------------------------------------------------------
// street-name blades (src/render/passes/street-furniture.js)
// ---------------------------------------------------------------------------

/// `STREET_SIGN_ATLAS`.
pub const STREET_SIGN_ATLAS_WIDTH: u32 = 1024;
pub const STREET_SIGN_ATLAS_MIN_HEIGHT: u32 = 1024;
pub const STREET_SIGN_ATLAS_MAX_HEIGHT: u32 = 4096;
pub const STREET_SIGN_CELL_WIDTH: u32 = 256;
pub const STREET_SIGN_CELL_HEIGHT: u32 = 64;
pub const STREET_SIGN_COLUMNS: u32 = 4;
/// `createStreetSignBladeGeometry`: the blade box (x along the street, y up,
/// z thickness), metres. three's BoxGeometry maps the whole tile onto each face.
pub const STREET_SIGN_BLADE_SIZE_M: [f64; 3] = [0.9, 0.16, 0.012];
/// `SIGN_FALLBACK_STREETS`.
pub const SIGN_FALLBACK_STREETS: [&str; 2] = ["MAIN ST", "CROSS ST"];

/// `planStreetSignAtlas(names)`.
#[derive(Clone, Debug, PartialEq)]
pub struct StreetSignPlan {
    pub width: u32,
    pub height: u32,
    pub capacity: usize,
    /// Distinct non-empty names, sorted as the JS `localeCompare` sorts them.
    pub requested: Vec<String>,
    pub accepted: Vec<String>,
    pub overflow: Vec<String>,
    /// `[u, v, du, dv]` per accepted name, in `accepted` order.
    pub tiles: Vec<[f32; 4]>,
}

impl StreetSignPlan {
    pub fn tile(&self, name: &str) -> Option<[f32; 4]> {
        self.accepted.iter().position(|n| n == name).map(|i| self.tiles[i])
    }
}

fn next_power_of_two(v: u32) -> u32 {
    let mut n = 1;
    while n < v.max(1) {
        n *= 2;
    }
    n
}

pub fn plan_street_sign_atlas(names: &[String]) -> StreetSignPlan {
    let mut requested: Vec<String> = Vec::new();
    for n in names {
        if !n.is_empty() && !requested.contains(n) {
            requested.push(n.clone());
        }
    }
    requested.sort_by(|a, b| locale_compare(a, b));
    let cols = STREET_SIGN_COLUMNS as usize;
    let wanted_rows = requested.len().div_ceil(cols).max(1) as u32;
    let height = STREET_SIGN_ATLAS_MAX_HEIGHT.min(next_power_of_two(STREET_SIGN_ATLAS_MIN_HEIGHT.max(wanted_rows * STREET_SIGN_CELL_HEIGHT)));
    let capacity = cols * (height / STREET_SIGN_CELL_HEIGHT) as usize;
    let accepted: Vec<String> = requested.iter().take(capacity).cloned().collect();
    let overflow: Vec<String> = requested.iter().skip(capacity).cloned().collect();
    let (w, cw, ch, hh) = (STREET_SIGN_ATLAS_WIDTH as f64, STREET_SIGN_CELL_WIDTH as f64, STREET_SIGN_CELL_HEIGHT as f64, height as f64);
    let inset = 1.5;
    let tiles = (0..accepted.len())
        .map(|i| {
            let x = (i % cols) as f64 * cw;
            let y = (i / cols) as f64 * ch;
            [((x + inset) / w) as f32, (1.0 - (y + ch - inset) / hh) as f32, ((cw - inset * 2.0) / w) as f32, ((ch - inset * 2.0) / hh) as f32]
        })
        .collect();
    StreetSignPlan { width: STREET_SIGN_ATLAS_WIDTH, height, capacity, requested, accepted, overflow, tiles }
}

/// `fitText(c, text, cx, cy, boxW, weight, family, startPx)`: shrink by 4 px to fit.
fn fit_text(c: &mut Canvas, text: &str, cx: f64, cy: f64, box_w: f64, weight: &str, start_px: f64) {
    let mut px = start_px;
    for _ in 0..40 {
        c.font(weight, px);
        if c.measure(text) <= box_w || px <= 12.0 {
            break;
        }
        px -= 4.0;
    }
    c.st.align = Align::Center;
    c.fill_text(text, cx, cy);
}

/// `buildStreetSignAtlas(plan)` for `planStreetSignAtlas(names)`: white sheet,
/// one 256 x 64 green blade per accepted name with its two white rules and the
/// name in bold. Returns the atlas (tiles in `accepted` order) and the tile per
/// name; names past the 4096 px capacity are left out (the plan's `overflow`).
pub fn street_name_atlas(names: &[String], fonts: &Fonts) -> Result<(Atlas, HashMap<String, [f32; 4]>)> {
    let plan = plan_street_sign_atlas(names);
    let atlas = street_name_atlas_for_plan(&plan, fonts)?;
    let map = plan.accepted.iter().cloned().zip(plan.tiles.iter().copied()).collect();
    Ok((atlas, map))
}

pub fn street_name_atlas_for_plan(plan: &StreetSignPlan, fonts: &Fonts) -> Result<Atlas> {
    let mut c = Canvas::new(plan.width, plan.height, fonts)?;
    c.fill_style("#ffffff");
    c.fill_rect(0.0, 0.0, plan.width as f64, plan.height as f64);
    let (cw, ch) = (STREET_SIGN_CELL_WIDTH as f64, STREET_SIGN_CELL_HEIGHT as f64);
    for (i, name) in plan.accepted.iter().enumerate() {
        let x = (i % STREET_SIGN_COLUMNS as usize) as f64 * cw;
        let y = (i / STREET_SIGN_COLUMNS as usize) as f64 * ch;
        c.fill_style("#1d4632");
        c.fill_rect(x, y, cw, ch);
        c.fill_style("#ffffff");
        c.fill_rect(x + 5.0, y + 5.0, cw - 10.0, 2.0);
        c.fill_rect(x + 5.0, y + ch - 7.0, cw - 10.0, 2.0);
        fit_text(&mut c, name, x + cw / 2.0, y + ch / 2.0, cw - 24.0, "700", 28.0);
    }
    Ok(c.finish(plan.tiles.clone()))
}

fn is_js_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// `s.replace(/\bWORD\b/, rep)`: the first whole-word occurrence only.
fn replace_word_once(s: &str, word: &str, rep: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let w: Vec<char> = word.chars().collect();
    for i in 0..chars.len() {
        if i + w.len() > chars.len() || chars[i..i + w.len()] != w[..] {
            continue;
        }
        let before = i == 0 || !is_js_word(chars[i - 1]);
        let after = i + w.len() == chars.len() || !is_js_word(chars[i + w.len()]);
        if before && after {
            let mut out: String = chars[..i].iter().collect();
            out.push_str(rep);
            out.extend(&chars[i + w.len()..]);
            return out;
        }
    }
    s.to_string()
}

/// Matches `^word\s+\d+` (case-insensitive word) and returns the rest after the digits.
fn word_space_digits<'s>(s: &'s str, word: &str) -> Option<&'s str> {
    if s.len() < word.len() || !s.is_char_boundary(word.len()) || !s[..word.len()].eq_ignore_ascii_case(word) {
        return None;
    }
    let rest = &s[word.len()..];
    let t = rest.trim_start_matches(is_js_space);
    if t.len() == rest.len() {
        return None;
    }
    let d = t.trim_start_matches(|c: char| c.is_ascii_digit());
    if d.len() == t.len() {
        return None;
    }
    Some(d)
}

/// `normalizeStreetLegend(value)`: the blade text, or None for a placeholder
/// (`SF Rd 1234567`, `Route 80`) or a name too long for one blade (> 22).
pub fn normalize_street_legend(value: &str) -> Option<String> {
    let raw = value.trim_matches(is_js_space);
    if raw.is_empty() {
        return None;
    }
    // `^SF\s+Rd\s+\d+$`
    if raw.is_char_boundary(2) && raw.len() >= 2 && raw[..2].eq_ignore_ascii_case("SF") {
        let rest = &raw[2..];
        let after = rest.trim_start_matches(is_js_space);
        if after.len() < rest.len() && word_space_digits(after, "Rd") == Some("") {
            return None;
        }
    }
    // `^Route\s+\d+`
    if word_space_digits(raw, "Route").is_some() {
        return None;
    }
    if raw.encode_utf16().count() > 22 {
        return None;
    }
    let mut s = raw.to_uppercase();
    for (word, rep) in [("STREET", "ST"), ("AVENUE", "AVE"), ("BOULEVARD", "BLVD"), ("DRIVE", "DR"), ("ROAD", "RD"), ("PLACE", "PL")] {
        s = replace_word_once(&s, word, rep);
    }
    Some(s)
}

/// `signStreetLegends(city, focus)`: the two most frequent legible names among
/// `segments` (name, first point), within 900 m of `focus` when given; ties by
/// name. Falls back to `SIGN_FALLBACK_STREETS`.
pub fn sign_street_legends<'s>(segments: impl IntoIterator<Item = (&'s str, Option<[f64; 2]>)>, focus: Option<[f64; 2]>) -> [String; 2] {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (name, at) in segments {
        let Some(name) = normalize_street_legend(name) else { continue };
        if let (Some(f), Some(p)) = (focus, at) {
            if (p[0] - f[0]).hypot(p[1] - f[1]) > 900.0 {
                continue;
            }
        }
        *counts.entry(name).or_default() += 1;
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| locale_compare(&a.0, &b.0)));
    let first = ranked.first().map(|r| r.0.clone()).unwrap_or_else(|| SIGN_FALLBACK_STREETS[0].into());
    let second = ranked.get(1).map(|r| r.0.clone()).unwrap_or_else(|| SIGN_FALLBACK_STREETS[1].into());
    [first, second]
}

/// The primary collation weight of `c` in the CLDR root order (what the JS
/// `localeCompare` uses): whitespace < punctuation < symbols < currency <
/// digits < letters, case-insensitively; None for an ignorable control.
/// Characters outside ASCII sort after the letters by code point (an
/// approximation: accented letters do not interleave with their base letter).
fn primary_weight(c: char) -> Option<u32> {
    const SPACES: &str = "\t\n\u{b}\u{c}\r ";
    const PUNCT: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    if let Some(i) = SPACES.find(c) {
        return Some(1 + i as u32);
    }
    if let Some(i) = PUNCT.find(c) {
        return Some(20 + i as u32);
    }
    if c.is_ascii_digit() {
        return Some(100 + (c as u32 - '0' as u32));
    }
    if c.is_ascii_alphabetic() {
        return Some(200 + (c.to_ascii_lowercase() as u32 - 'a' as u32));
    }
    if c.is_control() {
        return None;
    }
    Some(1000 + c.to_lowercase().next().unwrap_or(c) as u32)
}

/// `a.localeCompare(b)` (ICU root collation) for the street legends: primary
/// weights first, then case (lower before upper), left to right.
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| -> (Vec<u32>, Vec<u8>) {
        let mut p = vec![];
        let mut t = vec![];
        for c in s.chars() {
            if let Some(w) = primary_weight(c) {
                p.push(w);
                t.push(u8::from(c.is_uppercase()));
            }
        }
        (p, t)
    };
    let (pa, ta) = key(a);
    let (pb, tb) = key(b);
    pa.cmp(&pb).then_with(|| ta.cmp(&tb))
}

// ---------------------------------------------------------------------------
// the shared prop sign sheet (`buildSignAtlas`, `SIGN_ATLAS`)
// ---------------------------------------------------------------------------

pub const PROP_SHEET_SIZE: u32 = 1024;
/// `SIGN_ATLAS.cells` in order, with their pixel rects `[x0, y0, x1, y1]` (y down).
pub const PROP_SHEET_CELLS: [(&str, [u32; 4]); 14] = [
    ("bladeA", [0, 0, 1024, 128]),
    ("bladeB", [0, 128, 1024, 256]),
    ("transit", [0, 256, 256, 512]),
    ("masthead", [256, 256, 512, 512]),
    ("regulatory", [512, 256, 768, 512]),
    ("blockNumber", [768, 256, 1024, 512]),
    ("pedSignal", [0, 512, 256, 768]),
    ("meterFace", [256, 512, 512, 768]),
    ("chalkBoard", [512, 512, 768, 768]),
    ("workZone", [768, 512, 1024, 768]),
    ("countdown", [0, 768, 256, 1024]),
    ("binDecal", [256, 768, 512, 1024]),
    ("utilityLabel", [512, 768, 768, 1024]),
    ("frontPage", [768, 768, 1024, 1024]),
];
/// `SIGN_ATLAS.blank`: the single uv point every non-sign prop samples (u, v).
pub const PROP_SHEET_BLANK_UV: [f32; 2] = [0.5, 0.125];

/// `atlasRect(x0, y0, x1, y1)` as `[u0, v0, u1, v1]` (the JS cell form; 1.5 texel inset).
pub fn prop_sheet_cell_rect(name: &str) -> Option<[f32; 4]> {
    let &(_, [x0, y0, x1, y1]) = PROP_SHEET_CELLS.iter().find(|c| c.0 == name)?;
    let size = PROP_SHEET_SIZE as f64;
    let inset = 1.5 / size;
    Some([
        (x0 as f64 / size + inset) as f32,
        (1.0 - y1 as f64 / size + inset) as f32,
        (x1 as f64 / size - inset) as f32,
        (1.0 - y0 as f64 / size - inset) as f32,
    ])
}

/// `buildSignAtlas(legends)`: the shared 1024 px prop sheet. `tiles` follow
/// the 14 named `PROP_SHEET_CELLS` as `[u0, v0, du, dv]`.
pub fn prop_sign_sheet(legends: [&str; 2], fonts: &Fonts) -> Result<Atlas> {
    let size = PROP_SHEET_SIZE as f64;
    let mut c = Canvas::new(PROP_SHEET_SIZE, PROP_SHEET_SIZE, fonts)?;
    c.fill_style("#ffffff");
    c.fill_rect(0.0, 0.0, size, size);
    let blade = |c: &mut Canvas, y: f64, text: &str| {
        c.fill_style("#1d4632");
        c.fill_rect(0.0, y, size, 128.0);
        c.fill_style("#ffffff");
        c.fill_rect(10.0, y + 10.0, size - 20.0, 3.0);
        c.fill_rect(10.0, y + 115.0, size - 20.0, 3.0);
        fit_text(c, text, size / 2.0, y + 64.0, size - 90.0, "700", 78.0);
    };
    let l0 = if legends[0].is_empty() { SIGN_FALLBACK_STREETS[0] } else { legends[0] };
    let l1 = if legends[1].is_empty() { SIGN_FALLBACK_STREETS[1] } else { legends[1] };
    blade(&mut c, 0.0, l0);
    blade(&mut c, 128.0, l1);

    // Transit stop flag.
    c.fill_style("#1c3f6b");
    c.fill_rect(0.0, 256.0, 256.0, 256.0);
    c.fill_style("#ffffff");
    c.begin_path();
    c.arc(128.0, 348.0, 58.0, 0.0, TAU);
    c.fill();
    c.fill_style("#1c3f6b");
    fit_text(&mut c, "27", 128.0, 350.0, 92.0, "700", 66.0);
    c.fill_style("#ffffff");
    fit_text(&mut c, "BUS", 128.0, 434.0, 200.0, "700", 54.0);
    fit_text(&mut c, "STOP", 128.0, 480.0, 200.0, "700", 54.0);

    // News-box display.
    c.fill_style("#f2efe6");
    c.fill_rect(256.0, 256.0, 256.0, 256.0);
    c.fill_style("#20242a");
    c.fill_rect(256.0, 258.0, 256.0, 5.0);
    c.fill_rect(272.0, 300.0, 224.0, 5.0);
    c.fill_rect(272.0, 272.0, 150.0, 20.0);
    c.fill_rect(256.0, 505.0, 256.0, 5.0);
    c.fill_style("#3a3f45");
    c.fill_rect(272.0, 316.0, 224.0, 62.0);
    c.fill_style("#9aa0a6");
    c.fill_rect(282.0, 326.0, 204.0, 42.0);
    c.fill_style("#6d7076");
    for i in 0..8 {
        c.fill_rect(272.0, 392.0 + i as f64 * 13.0, 224.0 - (i % 3) as f64 * 44.0, 6.0);
    }

    // Regulatory plate.
    c.fill_style("#f4f2ec");
    c.fill_rect(512.0, 256.0, 256.0, 256.0);
    c.fill_style("#a8342a");
    c.fill_rect(512.0, 256.0, 256.0, 52.0);
    c.fill_style("#f4f2ec");
    fit_text(&mut c, "NO", 640.0, 284.0, 200.0, "700", 40.0);
    c.fill_style("#20242a");
    fit_text(&mut c, "PARKING", 640.0, 352.0, 216.0, "700", 50.0);
    fit_text(&mut c, "ANY TIME", 640.0, 408.0, 216.0, "600", 36.0);
    c.fill_rect(564.0, 444.0, 152.0, 6.0);
    fit_text(&mut c, "7 AM - 6 PM", 640.0, 478.0, 200.0, "500", 30.0);

    // Block-number plate.
    c.fill_style("#2f5f45");
    c.fill_rect(768.0, 256.0, 256.0, 256.0);
    c.fill_style("#ffffff");
    c.fill_rect(780.0, 268.0, 232.0, 4.0);
    c.fill_rect(780.0, 496.0, 232.0, 4.0);
    fit_text(&mut c, "1200", 896.0, 370.0, 210.0, "700", 92.0);
    fit_text(&mut c, "BLOCK", 896.0, 448.0, 190.0, "600", 40.0);

    // Pedestrian signal face.
    c.fill_style("#141618");
    c.fill_rect(0.0, 512.0, 256.0, 256.0);
    c.fill_style("#e8e4d8");
    c.begin_path();
    c.arc(128.0, 576.0, 20.0, 0.0, TAU);
    c.fill();
    c.fill_rect(118.0, 600.0, 22.0, 62.0);
    for quad in [
        [(118.0, 604.0), (88.0, 646.0), (100.0, 654.0), (128.0, 616.0)],
        [(140.0, 604.0), (172.0, 640.0), (162.0, 650.0), (130.0, 618.0)],
        [(118.0, 656.0), (100.0, 726.0), (116.0, 730.0), (130.0, 668.0)],
        [(134.0, 656.0), (156.0, 722.0), (142.0, 730.0), (124.0, 670.0)],
    ] {
        c.polygon(&quad);
        c.fill();
    }

    // Meter / pay-station display.
    c.fill_style("#2a2e31");
    c.fill_rect(256.0, 512.0, 256.0, 256.0);
    c.fill_style("#12281c");
    c.fill_rect(276.0, 546.0, 216.0, 96.0);
    c.fill_style("#7ee08a");
    fit_text(&mut c, "42", 346.0, 594.0, 110.0, "700", 76.0);
    fit_text(&mut c, "MIN", 450.0, 598.0, 76.0, "600", 38.0);
    c.fill_style("#c8ccc9");
    fit_text(&mut c, "PAY HERE", 384.0, 690.0, 210.0, "700", 40.0);
    c.fill_style("#6d7076");
    c.fill_rect(276.0, 718.0, 216.0, 5.0);

    // Chalk menu board.
    c.fill_style("#1f201d");
    c.fill_rect(512.0, 512.0, 256.0, 256.0);
    c.fill_style("#e8e2d2");
    fit_text(&mut c, "TODAY", 640.0, 548.0, 200.0, "700", 40.0);
    c.fill_rect(548.0, 572.0, 184.0, 3.0);
    c.fill_style("#d9c67a");
    fit_text(&mut c, "SOUP  6", 640.0, 610.0, 210.0, "600", 34.0);
    fit_text(&mut c, "TOAST  5", 640.0, 652.0, 210.0, "600", 34.0);
    fit_text(&mut c, "COFFEE  3", 640.0, 694.0, 210.0, "600", 34.0);
    c.fill_style("#e8e2d2");
    c.fill_rect(548.0, 724.0, 184.0, 3.0);

    // Work-zone board.
    c.fill_style("#d3641f");
    c.fill_rect(768.0, 512.0, 256.0, 256.0);
    c.fill_style("#1a1a1a");
    fit_text(&mut c, "ROAD", 896.0, 566.0, 220.0, "700", 62.0);
    fit_text(&mut c, "WORK", 896.0, 634.0, 220.0, "700", 62.0);
    fit_text(&mut c, "AHEAD", 896.0, 700.0, 220.0, "700", 52.0);
    for i in 0..6 {
        let x = 768.0 + i as f64 * 44.0;
        c.polygon(&[(x, 768.0), (x + 22.0, 768.0), (x + 44.0, 740.0), (x + 22.0, 740.0)]);
        c.fill();
    }

    // Countdown pedestrian face: an orange hand over red digits ("12").
    c.fill_style("#141618");
    c.fill_rect(0.0, 768.0, 256.0, 256.0);
    c.fill_style("#d5541f");
    c.polygon(&[(96.0, 800.0), (160.0, 800.0), (176.0, 860.0), (160.0, 900.0), (96.0, 900.0), (80.0, 860.0)]);
    c.fill();
    c.fill_style("#141618");
    for i in 0..4 {
        c.fill_rect(104.0 + i as f64 * 14.0, 800.0, 4.0, 40.0);
    }
    c.fill_style("#d8382a");
    fit_text(&mut c, "12", 128.0, 972.0, 160.0, "700", 96.0);

    // Litter-bin decal.
    c.fill_style("#37473d");
    c.fill_rect(256.0, 768.0, 256.0, 256.0);
    c.fill_style("#e6e2d6");
    c.begin_path();
    c.arc(384.0, 840.0, 34.0, 0.0, TAU);
    c.fill();
    c.fill_style("#37473d");
    c.begin_path();
    c.arc(384.0, 840.0, 24.0, 0.0, TAU);
    c.fill();
    c.fill_style("#e6e2d6");
    c.fill_rect(376.0, 852.0, 16.0, 30.0);
    fit_text(&mut c, "KEEP IT", 384.0, 928.0, 200.0, "700", 40.0);
    fit_text(&mut c, "CLEAN", 384.0, 976.0, 200.0, "700", 40.0);

    // Utility cabinet label.
    c.fill_style("#9aa09c");
    c.fill_rect(512.0, 768.0, 256.0, 256.0);
    c.fill_style("#c9b23a");
    c.fill_rect(548.0, 800.0, 184.0, 72.0);
    c.fill_style("#1a1a1a");
    fit_text(&mut c, "DANGER", 640.0, 822.0, 170.0, "700", 34.0);
    fit_text(&mut c, "HIGH VOLTAGE", 640.0, 856.0, 172.0, "600", 22.0);
    c.fill_style("#3b3f42");
    fit_text(&mut c, "TC-2417", 640.0, 940.0, 200.0, "700", 44.0);
    c.fill_rect(560.0, 976.0, 160.0, 4.0);

    // Newspaper front page.
    c.fill_style("#efeadc");
    c.fill_rect(768.0, 768.0, 256.0, 256.0);
    c.fill_style("#1d2024");
    c.fill_rect(784.0, 784.0, 224.0, 34.0);
    c.fill_rect(784.0, 828.0, 224.0, 3.0);
    c.fill_style("#2f3338");
    c.fill_rect(784.0, 840.0, 224.0, 52.0);
    c.fill_style("#7e858c");
    c.fill_rect(794.0, 850.0, 204.0, 32.0);
    c.fill_style("#a13a2c");
    c.fill_rect(784.0, 900.0, 100.0, 60.0);
    c.fill_style("#6d7076");
    for i in 0..5 {
        c.fill_rect(896.0, 902.0 + i as f64 * 12.0, 112.0 - (i % 2) as f64 * 30.0, 6.0);
    }
    for i in 0..4 {
        c.fill_rect(784.0, 970.0 + i as f64 * 12.0, 224.0 - (i % 3) as f64 * 40.0, 6.0);
    }

    let tiles = PROP_SHEET_CELLS
        .iter()
        .filter_map(|c| prop_sheet_cell_rect(c.0))
        .map(|[u0, v0, u1, v1]| [u0, v0, u1 - u0, v1 - v0])
        .collect();
    Ok(c.finish(tiles))
}

// ---------------------------------------------------------------------------
// the kerb picket's printed faces (src/render/passes/street-overhead.js)
// ---------------------------------------------------------------------------

/// `PRINTED_SIGN_FACES`, row k of the printed atlas.
pub const PRINTED_SIGN_FACES: [&str; 6] = ["street-cleaning", "pay-here", "pay-by-plate", "right-lane-must-turn-right", "one-way-left", "passenger-loading"];
/// `PRINTED_ATLAS`.
pub const PRINTED_ATLAS_WIDTH: u32 = 256;
pub const PRINTED_ATLAS_HEIGHT: u32 = 2048;
pub const PRINTED_ATLAS_CELL: u32 = 256;
/// `printedWhitePoint()`: where the unlettered parts sample (u, v).
pub const PRINTED_WHITE_UV: [f32; 2] = [0.5, 16.0 / 2048.0];

/// `printedFaceRect(face)` as `[u0, v0, du, dv]` (an unknown face takes row 0, as the JS).
pub fn printed_face_tile(face: &str) -> [f32; 4] {
    let row = PRINTED_SIGN_FACES.iter().position(|f| *f == face).unwrap_or(0) as f64;
    let (h, cell) = (PRINTED_ATLAS_HEIGHT as f64, PRINTED_ATLAS_CELL as f64);
    let v1 = 1.0 - (row * cell) / h;
    let v0 = 1.0 - ((row + 1.0) * cell) / h;
    [0.0, v0 as f32, 1.0, (v1 - v0) as f32]
}

/// `drawPrintedSignFace(ctx, face, x0, y0, S)`.
fn draw_printed_face(c: &mut Canvas, face: &str, x0: f64, y0: f64, s: f64) {
    const PRED: &str = "#b3202a";
    const PBLUE: &str = "#1f4fa3";
    const PWHITE: &str = "#f4f4ef";
    let k = s / 256.0;
    let text = |c: &mut Canvas, t: &str, x: f64, y: f64, size: f64, colour: &str, center: bool| {
        c.fill_style(colour);
        c.font("bold", js_round(size));
        c.st.align = if center { Align::Center } else { Align::Left };
        c.fill_text(t, x0 + x, y0 + y);
    };
    let rrect = |c: &mut Canvas, x: f64, y: f64, w: f64, h: f64, r: f64, fill: Option<&str>, stroke: Option<&str>, lw: f64| {
        c.begin_path();
        c.move_to(x0 + x + r, y0 + y);
        c.arc_to(x0 + x + w, y0 + y, x0 + x + w, y0 + y + h, r);
        c.arc_to(x0 + x + w, y0 + y + h, x0 + x, y0 + y + h, r);
        c.arc_to(x0 + x, y0 + y + h, x0 + x, y0 + y, r);
        c.arc_to(x0 + x, y0 + y, x0 + x + w, y0 + y, r);
        c.close_path();
        if let Some(f) = fill {
            c.fill_style(f);
            c.fill();
        }
        if let Some(st) = stroke {
            c.stroke_style(st);
            c.st.line_width = lw;
            c.stroke();
        }
    };
    match face {
        "street-cleaning" => {
            rrect(c, 0.0, 0.0, s, s, 10.0 * k, Some(PWHITE), None, 0.0);
            rrect(c, 6.0 * k, 6.0 * k, s - 12.0 * k, s - 12.0 * k, 8.0 * k, None, Some(PRED), 5.0 * k);
            rrect(c, 18.0 * k, 20.0 * k, 62.0 * k, 50.0 * k, 3.0 * k, Some(PRED), None, 0.0);
            text(c, "NO", 49.0 * k, 46.0 * k, 40.0 * k, PWHITE, true);
            text(c, "PARKING", 88.0 * k, 46.0 * k, 36.0 * k, PRED, false);
            text(c, "12:01", 22.0 * k, 104.0 * k, 40.0 * k, PRED, false);
            text(c, "A.M.", 122.0 * k, 110.0 * k, 18.0 * k, PRED, false);
            text(c, "TO", 166.0 * k, 104.0 * k, 26.0 * k, PRED, false);
            text(c, "6", 206.0 * k, 104.0 * k, 40.0 * k, PRED, false);
            text(c, "A.M.", 222.0 * k, 110.0 * k, 18.0 * k, PRED, false);
            text(c, "MON WED FRI", s / 2.0, 156.0 * k, 38.0 * k, PRED, true);
            text(c, "STREET CLEANING", s / 2.0, 206.0 * k, 22.0 * k, PRED, true);
        }
        "pay-here" => {
            rrect(c, 0.0, 0.0, s, s, 12.0 * k, Some(PBLUE), None, 0.0);
            rrect(c, 6.0 * k, 6.0 * k, s - 12.0 * k, s - 12.0 * k, 10.0 * k, None, Some(PWHITE), 4.0 * k);
            text(c, "Pay here", s / 2.0, 46.0 * k, 48.0 * k, PWHITE, true);
            rrect(c, 26.0 * k, 96.0 * k, 108.0 * k, 66.0 * k, 6.0 * k, Some(PWHITE), None, 0.0);
            c.fill_style("#20242a");
            c.fill_rect(x0 + 26.0 * k, y0 + 108.0 * k, 108.0 * k, 12.0 * k);
            rrect(c, 36.0 * k, 178.0 * k, 90.0 * k, 44.0 * k, 5.0 * k, Some(PWHITE), None, 0.0);
            text(c, "$", 81.0 * k, 201.0 * k, 34.0 * k, PBLUE, true);
            rrect(c, 160.0 * k, 92.0 * k, 70.0 * k, 132.0 * k, 12.0 * k, Some("#15181d"), Some(PWHITE), 3.0 * k);
            rrect(c, 168.0 * k, 108.0 * k, 54.0 * k, 96.0 * k, 4.0 * k, Some("#5a6fd6"), None, 0.0);
        }
        "pay-by-plate" => {
            rrect(c, 0.0, 0.0, s, s, 12.0 * k, Some(PBLUE), None, 0.0);
            rrect(c, 6.0 * k, 6.0 * k, s - 12.0 * k, s - 12.0 * k, 10.0 * k, None, Some(PWHITE), 4.0 * k);
            text(c, "Pay by", 22.0 * k, 44.0 * k, 38.0 * k, PWHITE, false);
            text(c, "License", 22.0 * k, 88.0 * k, 38.0 * k, PWHITE, false);
            text(c, "Plate", 22.0 * k, 132.0 * k, 38.0 * k, PWHITE, false);
            rrect(c, 186.0 * k, 110.0 * k, 44.0 * k, 30.0 * k, 5.0 * k, Some("#15181d"), Some(PWHITE), 2.0 * k);
            rrect(c, 22.0 * k, 168.0 * k, 212.0 * k, 56.0 * k, 6.0 * k, Some(PWHITE), Some("#15181d"), 3.0 * k);
            text(c, "41808004", 128.0 * k, 197.0 * k, 40.0 * k, "#15181d", true);
        }
        "right-lane-must-turn-right" => {
            let ink = "#15181d";
            rrect(c, 0.0, 0.0, s, s, 8.0 * k, Some(PWHITE), None, 0.0);
            rrect(c, 7.0 * k, 7.0 * k, s - 14.0 * k, s - 14.0 * k, 6.0 * k, None, Some(ink), 6.0 * k);
            text(c, "RIGHT LANE", s / 2.0, 62.0 * k, 40.0 * k, ink, true);
            text(c, "MUST", s / 2.0, 112.0 * k, 44.0 * k, ink, true);
            text(c, "TURN", s / 2.0, 160.0 * k, 44.0 * k, ink, true);
            text(c, "RIGHT", s / 2.0, 208.0 * k, 44.0 * k, ink, true);
        }
        "passenger-loading" => {
            c.save();
            c.translate(x0, y0);
            c.scale(1.0, 2.0 / 3.0);
            let h = s * 1.5;
            let ink = "#15181d";
            c.fill_style(PWHITE);
            c.fill_rect(0.0, 0.0, s, h);
            c.stroke_style(PRED);
            c.st.line_width = 8.0 * k;
            c.stroke_rect(6.0 * k, 6.0 * k, s - 12.0 * k, h - 12.0 * k);
            c.fill_style(PRED);
            c.fill_rect(6.0 * k, 6.0 * k, s - 12.0 * k, 60.0 * k);
            c.fill_style(PWHITE);
            c.fill_rect(30.0 * k, 30.0 * k, 70.0 * k, 22.0 * k);
            c.fill_rect(60.0 * k, 18.0 * k, 26.0 * k, 14.0 * k);
            c.fill_rect(150.0 * k, 28.0 * k, 76.0 * k, 22.0 * k);
            c.fill_rect(166.0 * k, 18.0 * k, 40.0 * k, 12.0 * k);
            c.begin_path();
            c.move_to(100.0 * k, 36.0 * k);
            c.line_to(150.0 * k, 26.0 * k);
            c.st.line_width = 5.0 * k;
            c.stroke_style(PWHITE);
            c.stroke();
            let t = |c: &mut Canvas, st: &str, y: f64, size: f64| {
                c.fill_style(ink);
                c.font("bold", js_round(size));
                c.st.align = Align::Center;
                c.fill_text(st, s / 2.0, y);
            };
            t(c, "PASSENGER", 100.0 * k, 34.0 * k);
            t(c, "LOADING ONLY", 138.0 * k, 30.0 * k);
            t(c, "AT ALL", 200.0 * k, 48.0 * k);
            t(c, "TIMES", 252.0 * k, 48.0 * k);
            c.stroke_style(ink);
            c.st.line_width = 6.0 * k;
            c.begin_path();
            c.move_to(44.0 * k, 312.0 * k);
            c.line_to(212.0 * k, 312.0 * k);
            c.stroke();
            c.fill_style(ink);
            c.polygon(&[(222.0 * k, 312.0 * k), (200.0 * k, 300.0 * k), (200.0 * k, 324.0 * k)]);
            c.fill();
            c.polygon(&[(34.0 * k, 312.0 * k), (56.0 * k, 300.0 * k), (56.0 * k, 324.0 * k)]);
            c.fill();
            c.restore();
        }
        "one-way-left" => {
            c.save();
            c.translate(x0, y0);
            c.scale(1.0 / 3.0, 1.0);
            let w = s * 3.0;
            c.fill_style("#121416");
            c.fill_rect(0.0, 0.0, w, s);
            c.stroke_style(PWHITE);
            c.st.line_width = 8.0 * k;
            c.stroke_rect(10.0 * k, 10.0 * k, w - 20.0 * k, s - 20.0 * k);
            c.fill_style(PWHITE);
            let (y0a, y1a) = (70.0 * k, 186.0 * k);
            let xx = |v: f64| w - v;
            c.polygon(&[
                (xx(40.0 * k), y0a),
                (xx(560.0 * k), y0a),
                (xx(560.0 * k), 36.0 * k),
                (xx(728.0 * k), s / 2.0),
                (xx(560.0 * k), s - 36.0 * k),
                (xx(560.0 * k), y1a),
                (xx(40.0 * k), y1a),
            ]);
            c.fill();
            c.fill_style("#121416");
            c.font("bold", js_round(96.0 * k));
            c.st.align = Align::Center;
            c.fill_text("ONE WAY", w - 300.0 * k, s / 2.0 + 4.0 * k);
            c.restore();
        }
        _ => {}
    }
}

/// `printedSignAtlasTexture()`: 256 x 2048, white, one 256 px row per
/// `PRINTED_SIGN_FACES` face; the rows under them stay white (the plate backs
/// and straps sample [`PRINTED_WHITE_UV`]). `tiles[k]` =
/// `printedFaceRect(PRINTED_SIGN_FACES[k])`.
pub fn printed_sign_atlas(fonts: &Fonts) -> Result<Atlas> {
    let (w, h, cell) = (PRINTED_ATLAS_WIDTH, PRINTED_ATLAS_HEIGHT, PRINTED_ATLAS_CELL as f64);
    let mut c = Canvas::new(w, h, fonts)?;
    c.fill_style("#ffffff");
    c.fill_rect(0.0, 0.0, w as f64, h as f64);
    for (row, face) in PRINTED_SIGN_FACES.iter().enumerate() {
        c.save();
        draw_printed_face(&mut c, face, 0.0, row as f64 * cell, cell);
        c.restore();
    }
    let tiles: Vec<[f32; 4]> = PRINTED_SIGN_FACES.iter().map(|f| printed_face_tile(f)).collect();
    Ok(c.finish(tiles))
}

// ---------------------------------------------------------------------------
// pedestrian heads (src/render/passes/traffic-pedestrian-heads.js)
// ---------------------------------------------------------------------------

/// `createPedestrianSymbolTexture()`: 256 x 128, the hand (left half) and the
/// walking person (right half) as signed-distance alpha with the LED stipple
/// in the colour. Data (`srgb` false), byte for byte the JS buffer (rows
/// flipped to top first). Tiles: hand, person (`symbolQuad(cell)`).
pub fn ped_symbol_texture() -> Atlas {
    let (w, s) = (256usize, 128usize);
    let mut data = vec![0u8; w * s * 4];
    let capsule = |px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64, r: f64| {
        let dx = bx - ax;
        let dy = by - ay;
        let dd = dx * dx + dy * dy;
        let t = (((px - ax) * dx + (py - ay) * dy) / if dd != 0.0 { dd } else { 1.0 }).clamp(0.0, 1.0);
        (px - ax - dx * t).hypot(py - ay - dy * t) - r
    };
    let hand = |x: f64, y: f64| {
        [
            capsule(x, y, 0.5, 0.2, 0.5, 0.46, 0.22),
            capsule(x, y, 0.31, 0.45, 0.31, 0.78, 0.07),
            capsule(x, y, 0.44, 0.48, 0.44, 0.9, 0.072),
            capsule(x, y, 0.58, 0.48, 0.58, 0.88, 0.072),
            capsule(x, y, 0.71, 0.44, 0.71, 0.76, 0.066),
            capsule(x, y, 0.3, 0.24, 0.14, 0.44, 0.075),
            capsule(x, y, 0.5, 0.04, 0.5, 0.2, 0.16),
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min)
    };
    let person = |x: f64, y: f64| {
        [
            (x - 0.53).hypot(y - 0.86) - 0.095,
            capsule(x, y, 0.51, 0.71, 0.46, 0.45, 0.105),
            capsule(x, y, 0.47, 0.45, 0.63, 0.25, 0.08),
            capsule(x, y, 0.63, 0.25, 0.68, 0.05, 0.07),
            capsule(x, y, 0.46, 0.45, 0.33, 0.26, 0.08),
            capsule(x, y, 0.33, 0.26, 0.22, 0.07, 0.07),
            capsule(x, y, 0.51, 0.67, 0.68, 0.55, 0.065),
            capsule(x, y, 0.68, 0.55, 0.74, 0.41, 0.058),
            capsule(x, y, 0.5, 0.67, 0.35, 0.53, 0.065),
            capsule(x, y, 0.35, 0.53, 0.31, 0.39, 0.058),
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min)
    };
    for py in 0..s {
        for px in 0..w {
            let cell = if px < s { 0 } else { 1 };
            let x = ((px % s) as f64 + 0.5) / s as f64;
            let y = (py as f64 + 0.5) / s as f64;
            let d = if cell == 0 { hand(x, y) } else { person(x, y) };
            let dot = if px % 4 == 1 && py % 4 == 1 { 1.0 } else { 0.86 };
            let a = js_round(255.0 * (0.5 - d * s as f64).clamp(0.0, 1.0)) as u8;
            let v = js_round(255.0 * dot) as u8;
            let o = (py * w + px) * 4;
            data[o..o + 4].copy_from_slice(&[v, v, v, a]);
        }
    }
    Atlas { width: w as u32, height: s as u32, rgba: flip_rows(&data, w * 4), tiles: vec![[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 1.0]], srgb: false }
}

/// The JS buffer order (row 0 = bottom) of an atlas stored top first, or back.
pub fn flip_rows(data: &[u8], stride: usize) -> Vec<u8> {
    data.chunks(stride).rev().flatten().copied().collect()
}

/// `DIGIT_SEGMENTS`: segments a..g lit per digit 0-9.
pub const DIGIT_SEGMENTS: [[u8; 7]; 10] = [
    [1, 1, 1, 1, 1, 1, 0], [0, 1, 1, 0, 0, 0, 0], [1, 1, 0, 1, 1, 0, 1], [1, 1, 1, 1, 0, 0, 1],
    [0, 1, 1, 0, 0, 1, 1], [1, 0, 1, 1, 0, 1, 1], [1, 0, 1, 1, 1, 1, 1], [1, 1, 1, 0, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1], [1, 1, 1, 1, 0, 1, 1],
];
/// `SEG_W`, `SEG_H`, `SEG_T` (metres on the head face).
pub const SEG_W: f64 = 0.062;
pub const SEG_H: f64 = 0.078;
pub const SEG_T: f64 = 0.016;
/// `SEGMENT_BOXES`: `[cx, cy, sx, sy]` per segment a..g, digit-local metres, y up.
pub const SEGMENT_BOXES: [[f64; 4]; 7] = [
    [0.0, SEG_H, SEG_W, SEG_T],
    [SEG_W / 2.0, SEG_H / 2.0, SEG_T, SEG_H],
    [SEG_W / 2.0, -SEG_H / 2.0, SEG_T, SEG_H],
    [0.0, -SEG_H, SEG_W, SEG_T],
    [-SEG_W / 2.0, -SEG_H / 2.0, SEG_T, SEG_H],
    [-SEG_W / 2.0, SEG_H / 2.0, SEG_T, SEG_H],
    [0.0, 0.0, SEG_W, SEG_T],
];
/// `DIGIT_X`: tens and ones digit centres on the face (x, metres); `SYMBOL_X` the symbol's.
pub const DIGIT_X: [f64; 2] = [0.062, 0.162];
pub const SYMBOL_X: f64 = -0.1;

/// One digit cell of [`ped_digit_atlas`]: pixels and the metres it spans.
pub const DIGIT_CELL_PX: [u32; 2] = [64, 128];
/// The digit-local metres a cell spans (`[width, height]`, centred on the
/// digit): the segment boxes' extent plus half a segment's thickness all round.
pub fn ped_digit_cell_m() -> [f64; 2] {
    let h = 2.0 * (SEG_H + SEG_T);
    [h * DIGIT_CELL_PX[0] as f64 / DIGIT_CELL_PX[1] as f64, h]
}

/// The countdown digits 0-9 as the JS draws them (`SEGMENT_BOXES` lit by
/// `DIGIT_SEGMENTS`), white on clear, one 64 x 128 cell each in a 1024 x 128
/// strip; `tiles[d]` spans [`ped_digit_cell_m`] centred on the digit. The JS
/// lights quads additively in Portland orange (the instance colour); this is
/// the coverage (overlapping segment corners are not doubled).
pub fn ped_digit_atlas() -> Result<Atlas> {
    let (cw, ch) = (DIGIT_CELL_PX[0], DIGIT_CELL_PX[1]);
    let width = 1024;
    let mut pix = Pixmap::new(width, ch).ok_or_else(|| anyhow!("pixmap"))?;
    let px_per_m = ch as f64 / ped_digit_cell_m()[1];
    let mut paint = Paint::default();
    paint.anti_alias = true;
    paint.set_color(Color::WHITE);
    let mut tiles = vec![];
    for (d, pattern) in DIGIT_SEGMENTS.iter().enumerate() {
        let (ox, oy) = (d as f64 * cw as f64 + cw as f64 / 2.0, ch as f64 / 2.0);
        let mut pb = PathBuilder::new();
        for (s, box_) in SEGMENT_BOXES.iter().enumerate() {
            if pattern[s] == 0 {
                continue;
            }
            let [cx, cy, sx, sy] = *box_;
            if let Some(r) = Rect::from_xywh(
                (ox + (cx - sx / 2.0) * px_per_m) as f32,
                (oy - (cy + sy / 2.0) * px_per_m) as f32,
                (sx * px_per_m) as f32,
                (sy * px_per_m) as f32,
            ) {
                pb.push_rect(r);
            }
        }
        if let Some(path) = pb.finish() {
            pix.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        let (w, h) = (width as f64, ch as f64);
        tiles.push([(d as f64 * cw as f64 / w) as f32, 0.0, (cw as f64 / w) as f32, (ch as f64 / h) as f32]);
    }
    let mut rgba = Vec::with_capacity((width * ch * 4) as usize);
    for p in pix.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(Atlas { width, height: ch, rgba, tiles, srgb: true })
}

// ---------------------------------------------------------------------------
// everything at once, and the cache
// ---------------------------------------------------------------------------

/// Every sign atlas the props draw.
#[derive(Clone, Debug, PartialEq)]
pub struct SignAtlases {
    /// [`sign_face_atlas`]: tiles by inventory face index.
    pub faces: Atlas,
    /// [`street_name_atlas`]: tiles in `street_name_plan.accepted` order.
    pub street_names: Atlas,
    pub street_name_plan: StreetSignPlan,
    /// [`prop_sign_sheet`].
    pub prop_sheet: Atlas,
    /// [`printed_sign_atlas`].
    pub printed: Atlas,
    /// [`ped_symbol_texture`].
    pub ped_symbols: Atlas,
    /// [`ped_digit_atlas`].
    pub ped_digits: Atlas,
    /// what each face was drawn from, `(table/face, source)`: an official
    /// sign code with its sheet, or `drawn: <reason>` (the JS drawing kept).
    /// Empty when the official art was not available (every face drawn).
    pub provenance: Vec<(String, String)>,
}

impl SignAtlases {
    fn named(&self) -> [(&'static str, &Atlas); 6] {
        [
            ("faces", &self.faces),
            ("street-names", &self.street_names),
            ("prop-sheet", &self.prop_sheet),
            ("printed", &self.printed),
            ("ped-symbols", &self.ped_symbols),
            ("ped-digits", &self.ped_digits),
        ]
    }

    /// Writes each atlas as `<dir>/<prefix><name>.png`.
    pub fn write_pngs(&self, dir: &Path, prefix: &str) -> Result<Vec<PathBuf>> {
        let mut out = vec![];
        for (name, a) in self.named() {
            let p = dir.join(format!("{prefix}{name}.png"));
            a.write_png(&p)?;
            out.push(p);
        }
        Ok(out)
    }

    /// bytes of RGBA held by the atlases (CPU side, before upload).
    pub fn rgba_bytes(&self) -> usize {
        self.named().iter().map(|(_, a)| a.rgba.len()).sum()
    }
}

/// Cooks every atlas: the inventory faces of `js`, the blades for `street_names`
/// (raw legends; normalise them with [`normalize_street_legend`] first), and
/// the prop sheet's two legacy blades from `sheet_legends`. The drawn faces
/// (the browser game's artwork); see [`cook_sign_atlases_with`] for the official art.
pub fn cook_sign_atlases(js: &JsProps, fonts: &Fonts, street_names: &[String], sheet_legends: [&str; 2]) -> Result<SignAtlases> {
    cook_sign_atlases_with(js, fonts, street_names, sheet_legends, None)
}

/// [`cook_sign_atlases`] with the official artwork where `art` is given:
/// each face the manifest maps to a sign code is drawn from that code's art,
/// each face it marks `drawn` keeps the JS drawing with Overpass for its text,
/// the blades are CA MUTCD D3-1 blades. Without `art`, today's drawings.
pub fn cook_sign_atlases_with(js: &JsProps, fonts: &Fonts, street_names: &[String], sheet_legends: [&str; 2], art: Option<&art::SignArt>) -> Result<SignAtlases> {
    let plan = plan_street_sign_atlas(street_names);
    let Some(art) = art else {
        return Ok(SignAtlases {
            faces: sign_face_atlas(js, fonts)?,
            street_names: street_name_atlas_for_plan(&plan, fonts)?,
            street_name_plan: plan,
            prop_sheet: prop_sign_sheet(sheet_legends, fonts)?,
            printed: printed_sign_atlas(fonts)?,
            ped_symbols: ped_symbol_texture(),
            ped_digits: ped_digit_atlas()?,
            provenance: vec![],
        });
    };
    let mut provenance = vec![];
    let faces = official_face_atlas(js, art, &mut provenance)?;
    let printed = official_printed_atlas(art, &mut provenance)?;
    let prop_sheet = official_prop_sheet(sheet_legends, art, &mut provenance)?;
    let street_names = official_street_name_atlas(&plan, art)?;
    provenance.push(("streetNames/*".into(), source_of(&serde_json::json!({ "svg": art::Manifest::get().raw["streetName"]["svg"] }))));
    Ok(SignAtlases { faces, street_names, street_name_plan: plan, prop_sheet, printed, ped_symbols: ped_symbol_texture(), ped_digits: ped_digit_atlas()?, provenance })
}

/// The provenance text of an art spec: the codes it is drawn from.
fn source_of(spec: &serde_json::Value) -> String {
    let man = art::Manifest::get();
    let file = |id: &str| man.raw["files"].as_array().and_then(|a| a.iter().find(|f| f["id"] == id)).map(|f| f["source"].as_str().unwrap_or_default().to_string()).unwrap_or_default();
    if let Some(r) = spec["drawn"].as_str() {
        return format!("drawn: {r}");
    }
    if let Some(id) = spec["svg"].as_str() {
        return format!("{id} ({})", file(id));
    }
    if let Some(items) = spec["stack"].as_array() {
        return items.iter().map(source_of).collect::<Vec<_>>().join(" over ");
    }
    if let Some(l) = spec["layout"].as_str() {
        let sheet = art::layout(l).ok().and_then(|v| v["sheet"].as_str()).unwrap_or_default();
        return format!("{l} composed from {}", file(sheet));
    }
    format!("unknown spec {spec}")
}

fn spec_of<'a>(table: &str, face: &str) -> Result<&'a serde_json::Value> {
    art::Manifest::get().spec(table, face).ok_or_else(|| anyhow!("sign art manifest: {table}.{face} has no entry (map it to art or mark it drawn)"))
}

/// One face of the `DRAW` table (a face with no published art: tow-away, the regulatory
/// and guide catch-alls, angle parking) alone on a `w` x `h` px canvas, the face filling it as
/// it fills its plate in the game (the cell's face rectangle is mapped onto the whole plate),
/// with `fonts` (the official art's Overpass, as `official_face_atlas` draws it). For the Blender
/// sign build (eo-props example `sign_faces_export`). None when the table does not draw the face.
pub fn drawn_face(face: &str, w: u32, h: u32, fonts: &Fonts) -> Result<Option<Atlas>> {
    let mut c = Canvas::new(w, h, fonts)?;
    c.save();
    let drawn = draw_face(&mut c, face, 0.0, 0.0, w as f64, h as f64);
    c.restore();
    Ok(drawn.then(|| c.finish(vec![])))
}

/// the face atlas from the official art (the JS cell layout, tiles unchanged).
fn official_face_atlas(js: &JsProps, art: &art::SignArt, provenance: &mut Vec<(String, String)>) -> Result<Atlas> {
    let size = SIGN_FACE_SIZE;
    let mut c = Canvas::new(size, size, &art.fonts)?;
    let frames = art::dominant_plate_sizes(&js.signs.plates, js.signs.faces.len());
    for (i, face) in js.signs.faces.iter().enumerate() {
        let spec = spec_of("faces", face)?;
        let [x, y, w, h] = sign_face_rect(face, i);
        if spec["drawn"].is_string() {
            c.save();
            // The guide plate keeps the JS legend, MUNI BUS STOP: San Francisco's
            // public transit agency names its own stops (user, 2026-11-15). The
            // word only; no agency logo is drawn on any face.
            draw_face(&mut c, face, x, y, w, h);
            c.restore();
        } else {
            let a = art.spec_size(spec)?;
            let dest = art::fit(a, frames[i].unwrap_or(a), [x, y, w, h]);
            art.draw_spec(&mut c.pix, spec, dest).with_context(|| format!("face {face}"))?;
        }
        provenance.push((format!("faces/{face}"), source_of(spec)));
    }
    let tiles = js.signs.faces.iter().enumerate().map(|(i, f)| sign_face_tile(f, i)).collect();
    Ok(c.finish(tiles))
}

/// the printed picket faces from the official art; each face fitted to
/// its plate in the overhead export (`signVariants`), on the white sheet.
fn official_printed_atlas(art: &art::SignArt, provenance: &mut Vec<(String, String)>) -> Result<Atlas> {
    let (w, h, cell) = (PRINTED_ATLAS_WIDTH, PRINTED_ATLAS_HEIGHT, PRINTED_ATLAS_CELL as f64);
    let oh = crate::overhead::OverheadJs::load()?;
    let plate = |face: &str| oh.sign_variants.iter().flat_map(|v| v.plates.iter()).find(|p| p.face.as_deref() == Some(face)).map(|p| [p.w, p.h]);
    let mut c = Canvas::new(w, h, &art.fonts)?;
    c.fill_style("#ffffff");
    c.fill_rect(0.0, 0.0, w as f64, h as f64);
    for (row, face) in PRINTED_SIGN_FACES.iter().enumerate() {
        let spec = spec_of("printed", face)?;
        let y0 = row as f64 * cell;
        if spec["drawn"].is_string() {
            c.save();
            draw_printed_face(&mut c, face, 0.0, y0, cell);
            c.restore();
        } else {
            let a = art.spec_size(spec)?;
            let dest = art::fit(a, plate(face).unwrap_or(a), [0.0, y0, cell, cell]);
            art.draw_spec(&mut c.pix, spec, dest).with_context(|| format!("printed {face}"))?;
        }
        provenance.push((format!("printed/{face}"), source_of(spec)));
    }
    let tiles: Vec<[f32; 4]> = PRINTED_SIGN_FACES.iter().map(|f| printed_face_tile(f)).collect();
    Ok(c.finish(tiles))
}

/// the prop sheet with Overpass for its text, the regulatory plate and
/// the work-zone board from the official art, the two blades as D3-1 blades.
fn official_prop_sheet(legends: [&str; 2], art: &art::SignArt, provenance: &mut Vec<(String, String)>) -> Result<Atlas> {
    let base = prop_sign_sheet(legends, &art.fonts)?;
    let mut pix = Pixmap::new(base.width, base.height).ok_or_else(|| anyhow!("pixmap"))?;
    for (d, s) in pix.pixels_mut().iter_mut().zip(base.rgba.chunks_exact(4)) {
        *d = tiny_skia::ColorU8::from_rgba(s[0], s[1], s[2], s[3]).premultiply();
    }
    let white = Paint { shader: tiny_skia::Shader::SolidColor(Color::WHITE), ..Paint::default() };
    for &(name, [x0, y0, x1, y1]) in PROP_SHEET_CELLS.iter() {
        let rect = [x0 as f64, y0 as f64, (x1 - x0) as f64, (y1 - y0) as f64];
        let spec = art::Manifest::get().spec("propSheet", name);
        let legend = match name {
            "bladeA" => Some(if legends[0].is_empty() { SIGN_FALLBACK_STREETS[0] } else { legends[0] }),
            "bladeB" => Some(if legends[1].is_empty() { SIGN_FALLBACK_STREETS[1] } else { legends[1] }),
            _ => None,
        };
        if spec.is_none() && legend.is_none() {
            continue;
        }
        if let Some(r) = Rect::from_xywh(rect[0] as f32, rect[1] as f32, rect[2] as f32, rect[3] as f32) {
            pix.fill_rect(r, &white, Transform::identity(), None);
        }
        if let Some(l) = legend {
            art.street_blade(&mut pix, l, rect, [rect[2], rect[3]])?;
        } else if let Some(spec) = spec {
            if !spec["drawn"].is_string() {
                let a = art.spec_size(spec)?;
                art.draw_spec(&mut pix, spec, art::fit(a, [rect[2], rect[3]], rect)).with_context(|| format!("prop sheet {name}"))?;
            }
            provenance.push((format!("propSheet/{name}"), source_of(spec)));
        }
    }
    let mut rgba = Vec::with_capacity(base.rgba.len());
    for p in pix.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(Atlas { width: base.width, height: base.height, rgba, tiles: base.tiles, srgb: true })
}

/// the street-name sheet with D3-1 blades (the plan's cells and tiles unchanged).
fn official_street_name_atlas(plan: &StreetSignPlan, art: &art::SignArt) -> Result<Atlas> {
    let mut c = Canvas::new(plan.width, plan.height, &art.fonts)?;
    c.fill_style("#ffffff");
    c.fill_rect(0.0, 0.0, plan.width as f64, plan.height as f64);
    let (cw, ch) = (STREET_SIGN_CELL_WIDTH as f64, STREET_SIGN_CELL_HEIGHT as f64);
    // Each cell covers the whole blade box face (`createStreetSignBladeGeometry`).
    let frame = [STREET_SIGN_BLADE_SIZE_M[0], STREET_SIGN_BLADE_SIZE_M[1]];
    for (i, name) in plan.accepted.iter().enumerate() {
        let x = (i % STREET_SIGN_COLUMNS as usize) as f64 * cw;
        let y = (i / STREET_SIGN_COLUMNS as usize) as f64 * ch;
        art.street_blade(&mut c.pix, name, [x, y, cw, ch], frame)?;
    }
    Ok(c.finish(plan.tiles.clone()))
}

/// The cache key: this module's version and source bytes, the fonts and every input.
pub fn sign_atlas_cache_key(js: &JsProps, fonts: &Fonts, street_names: &[String], sheet_legends: [&str; 2]) -> String {
    sign_atlas_cache_key_with(js, fonts, street_names, sheet_legends, None)
}

/// [`sign_atlas_cache_key`] with the art's digest (manifest and file
/// digests) and the art modules' sources when the official art is used.
pub fn sign_atlas_cache_key_with(js: &JsProps, fonts: &Fonts, street_names: &[String], sheet_legends: [&str; 2], art: Option<&art::SignArt>) -> String {
    let plan = plan_street_sign_atlas(street_names);
    let mut k = Vec::new();
    k.extend_from_slice(SIGN_ATLAS_VERSION.as_bytes());
    k.push(0);
    k.extend_from_slice(include_bytes!("sign_atlas.rs"));
    k.push(0);
    k.extend_from_slice(fonts.digest.as_bytes());
    for list in [&js.signs.faces, &plan.accepted] {
        k.push(1);
        for s in list.iter() {
            k.extend_from_slice(s.as_bytes());
            k.push(0);
        }
    }
    k.push(2);
    for s in sheet_legends {
        k.extend_from_slice(s.as_bytes());
        k.push(0);
    }
    if let Some(a) = art {
        k.push(3);
        k.extend_from_slice(a.digest.as_bytes());
        k.extend_from_slice(include_bytes!("sign_art.rs"));
        k.extend_from_slice(include_bytes!("sign_svg.rs"));
        // The inventory's plate sizes choose each face's frame.
        k.extend_from_slice(&js.signs.plates.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    }
    sha256_hex(&k)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ManifestAtlas {
    width: u32,
    height: u32,
    srgb: bool,
    tiles: Vec<[f32; 4]>,
    rgba_sha256: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    version: String,
    key: String,
    atlases: Vec<(String, ManifestAtlas)>,
    street_name_plan: (u32, u32, usize, Vec<String>, Vec<String>, Vec<String>),
    #[serde(default)]
    provenance: Vec<(String, String)>,
}

fn cache_paths(cache_dir: &Path, key: &str) -> (PathBuf, PathBuf) {
    let dir = cache_dir.join("props");
    let stem = format!("sign-atlas-{}", &key[..24]);
    (dir.join(format!("{stem}.json")), dir.join(stem))
}

fn read_cached(cache_dir: &Path, key: &str) -> Result<SignAtlases> {
    let (manifest_path, stem) = cache_paths(cache_dir, key);
    let m: Manifest = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    if m.version != SIGN_ATLAS_VERSION || m.key != key {
        bail!("stale manifest");
    }
    let mut got: HashMap<String, Atlas> = HashMap::new();
    for (name, ma) in m.atlases {
        let png = std::fs::read(format!("{}-{name}.png", stem.display()))?;
        let (w, h, rgba) = decode_png(&png)?;
        if w != ma.width || h != ma.height || sha256_hex(&rgba) != ma.rgba_sha256 {
            bail!("{name}: payload digest mismatch");
        }
        got.insert(name, Atlas { width: w, height: h, rgba, tiles: ma.tiles, srgb: ma.srgb });
    }
    let mut take = |n: &str| got.remove(n).ok_or_else(|| anyhow!("{n} missing from the cache"));
    let (width, height, capacity, requested, accepted, overflow) = m.street_name_plan;
    let street_names = take("street-names")?;
    let street_name_plan = StreetSignPlan { width, height, capacity, requested, accepted, overflow, tiles: street_names.tiles.clone() };
    Ok(SignAtlases {
        faces: take("faces")?,
        street_names,
        street_name_plan,
        prop_sheet: take("prop-sheet")?,
        printed: take("printed")?,
        ped_symbols: take("ped-symbols")?,
        ped_digits: take("ped-digits")?,
        provenance: m.provenance,
    })
}

fn write_cached(cache_dir: &Path, key: &str, a: &SignAtlases) -> Result<()> {
    let (manifest_path, stem) = cache_paths(cache_dir, key);
    let mut atlases = vec![];
    for (name, at) in a.named() {
        at.write_png(Path::new(&format!("{}-{name}.png", stem.display())))?;
        atlases.push((name.to_string(), ManifestAtlas { width: at.width, height: at.height, srgb: at.srgb, tiles: at.tiles.clone(), rgba_sha256: sha256_hex(&at.rgba) }));
    }
    let p = &a.street_name_plan;
    let m = Manifest {
        version: SIGN_ATLAS_VERSION.into(),
        key: key.into(),
        atlases,
        street_name_plan: (p.width, p.height, p.capacity, p.requested.clone(), p.accepted.clone(), p.overflow.clone()),
        provenance: a.provenance.clone(),
    };
    // The manifest goes last: its presence says every PNG is complete.
    write_atomic(&manifest_path, &serde_json::to_vec(&m)?)
}

/// Whether [`load_or_cook`] read the cache or cooked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheOutcome {
    Hit,
    Cooked,
}

/// The atlases from `<cache_dir>/props/sign-atlas-<key>*` when present and
/// their payload digests hold, else cooked and written there (tmp + rename;
/// a write failure is reported to stderr and the cooked atlases returned).
/// with the official art from [`art::root`] when it is there and every
/// file verifies ([`art::SignArt::load_default`]), else the drawn faces.
pub fn load_or_cook(cache_dir: &Path, js: &JsProps, fonts: &Fonts, street_names: &[String], sheet_legends: [&str; 2]) -> Result<(SignAtlases, CacheOutcome)> {
    let t = std::time::Instant::now();
    let art = art::SignArt::load_default();
    let verify_ms = t.elapsed().as_secs_f64() * 1e3;
    let r = load_or_cook_with(cache_dir, js, fonts, street_names, sheet_legends, art.as_ref())?;
    eprintln!(
        "eo-props: sign atlases {} in {:.0} ms ({}; art verified in {verify_ms:.0} ms; {:.1} MB RGBA)",
        if r.1 == CacheOutcome::Hit { "read from the cache" } else { "cooked" },
        t.elapsed().as_secs_f64() * 1e3,
        art.as_ref().map_or_else(|| "drawn faces".to_string(), |a| format!("official sign art + Overpass from {}", a.root.display())),
        r.0.rgba_bytes() as f64 / 1e6
    );
    Ok(r)
}

/// [`load_or_cook`] with the art given (None: the drawn faces).
pub fn load_or_cook_with(cache_dir: &Path, js: &JsProps, fonts: &Fonts, street_names: &[String], sheet_legends: [&str; 2], art: Option<&art::SignArt>) -> Result<(SignAtlases, CacheOutcome)> {
    let key = sign_atlas_cache_key_with(js, fonts, street_names, sheet_legends, art);
    if let Ok(a) = read_cached(cache_dir, &key) {
        return Ok((a, CacheOutcome::Hit));
    }
    let a = cook_sign_atlases_with(js, fonts, street_names, sheet_legends, art)?;
    if let Err(e) = write_cached(cache_dir, &key, &a) {
        eprintln!("eo-props: sign atlas cache not written: {e:#}");
    }
    Ok((a, CacheOutcome::Cooked))
}

// ---------------------------------------------------------------------------
// the mast arms' Street Name signs (eo-props `signals::Fitting::NameSign`, eo-app `--no-arm-name-signs`)
// ---------------------------------------------------------------------------

/// Cell size (px) of a mast arm's Street Name sign in the sign face atlas, and the columns a row holds.
pub const ARM_SIGN_CELL: [u32; 2] = [192, 32];
pub const ARM_SIGN_COLUMNS: u32 = 10;

/// The mast arms' Street Name signs drawn into the free rows of the sign face atlas (under its
/// inventory faces): per accepted legend its real size, its cell's tile and its mean colour.
pub struct ArmNameSigns {
    /// The legends drawn, in face order (past the inventory's faces); the rest did not fit.
    pub legends: Vec<String>,
    /// Per legend, the sign's real size (width, height, m).
    pub sizes: Vec<[f64; 2]>,
    /// Per legend, its tile `[u0, v0 up, du, dv]` in the face atlas (`SIGN_FACE_SIZE` square).
    pub tiles: Vec<[f32; 4]>,
    /// Per legend, the sign's mean colour (sRGB, 0..1): the face colour where the atlas is not drawn.
    pub colours: Vec<[f32; 4]>,
    /// The cells' pixels: `SIGN_FACE_SIZE` wide, rows from `y0` (top first), sRGB, straight alpha.
    pub rgba: Vec<u8>,
    pub y0: u32,
    pub height: u32,
}

/// Draw each legend as a CA MUTCD D3-1 Street Name sign (`art::SignArt::street_blade`: the FHWA
/// D3-1a layout's green, border and corners, mixed case in Overpass Bold) at its real size for
/// capitals `cap` m tall (`street-name-sign-overhead-letter-height`), into `ARM_SIGN_CELL` cells
/// from the first free row under `face_count` inventory faces; then weathered (the sheeting
/// faded and chalky, grime run down from the top edge, speckle): aged and matte. Legends past
/// the free rows are left out (listed in neither `legends` nor `sizes`).
pub fn arm_name_signs(legends: &[String], art: &art::SignArt, face_count: usize, cap: f64) -> Result<ArmNameSigns> {
    let size = SIGN_FACE_SIZE;
    let rows_used = (face_count as u32).div_ceil(SIGN_FACE_COLUMNS) * SIGN_FACE_CELL;
    let y0 = rows_used + 8;
    let [cw, ch] = ARM_SIGN_CELL;
    let rows = (size.saturating_sub(y0)) / ch;
    let capacity = (rows * ARM_SIGN_COLUMNS) as usize;
    let height = rows * ch;
    if height == 0 {
        bail!("no free rows in the sign face atlas under {face_count} faces");
    }
    let mut pix = Pixmap::new(size, height).ok_or_else(|| anyhow!("pixmap {size} x {height}"))?;
    let (mut out_l, mut sizes, mut tiles) = (vec![], vec![], vec![]);
    for (i, legend) in legends.iter().take(capacity).enumerate() {
        let (cx, cy) = ((i as u32 % ARM_SIGN_COLUMNS) * cw, (i as u32 / ARM_SIGN_COLUMNS) * ch);
        let frame = art.blade_size(legend, cap)?;
        // a pixel of gutter round each cell (the mips)
        let dest = [cx as f64 + 1.0, cy as f64 + 1.0, cw as f64 - 2.0, ch as f64 - 2.0];
        art.street_blade(&mut pix, legend, dest, frame)?;
        out_l.push(legend.clone());
        sizes.push(frame);
        let inset = 1.5;
        let (fx, fy) = (cx as f64 + inset, (y0 + cy) as f64 + inset);
        let (fw, fh) = (cw as f64 - 2.0 * inset, ch as f64 - 2.0 * inset);
        let s = size as f64;
        tiles.push([(fx / s) as f32, (1.0 - (fy + fh) / s) as f32, (fw / s) as f32, (fh / s) as f32]);
    }
    let mut rgba = Vec::with_capacity((size * height * 4) as usize);
    for p in pix.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    // weathering: per sign a fade, then grime run down from the top edge and speckle (hashes of the pixel)
    let hash = |x: u32, y: u32, k: u32| -> f32 {
        let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ k.wrapping_mul(0xcb1a_b31f);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        (h & 0xffff) as f32 / 65535.0
    };
    let mut colours = vec![];
    for i in 0..out_l.len() as u32 {
        let (cx, cy) = ((i % ARM_SIGN_COLUMNS) * cw, (i / ARM_SIGN_COLUMNS) * ch);
        let fade = 0.08 + 0.14 * hash(i, 7, 1);
        let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
        for y in cy..cy + ch {
            let ty = (y - cy) as f32 / ch as f32;
            for x in cx..cx + cw {
                let o = ((y * size + x) * 4) as usize;
                if rgba[o + 3] == 0 {
                    continue;
                }
                // a run of grime down from the top at this column, broken
                let col = hash(x / 3, 0, i + 11);
                let run = if col > 0.72 { (1.0 - ty / (0.3 + 0.7 * col)).clamp(0.0, 1.0) * 0.22 } else { 0.0 };
                let speck = if hash(x, y, i + 3) > 0.985 { 0.25 } else { 0.0 };
                let grain = 0.94 + 0.08 * hash(x, y, i + 5);
                for c in 0..3 {
                    let v = rgba[o + c] as f32 / 255.0;
                    // sun fade toward a chalky grey, grime and speckle darken
                    let faded = v * (1.0 - fade) + 0.62 * fade;
                    let d = faded * grain * (1.0 - run - speck).max(0.0);
                    let warm = [1.0, 0.985, 0.95][c];
                    let e = (d * warm).clamp(0.0, 1.0);
                    rgba[o + c] = (e * 255.0).round() as u8;
                    sum[c] += e;
                }
                n += 1.0;
            }
        }
        let m = if n > 0.0 { [sum[0] / n, sum[1] / n, sum[2] / n] } else { [0.2, 0.35, 0.25] };
        colours.push([m[0], m[1], m[2], 1.0]);
    }
    Ok(ArmNameSigns { legends: out_l, sizes, tiles, colours, rgba, y0, height })
}
