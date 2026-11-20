//! Shop sign lettering (stage S1, eo-app `--facade-clutter`): the generic
//! shop-category words the facade clutter puts on its sign faces (eo-kit
//! `clutter::letter_cells`: CAFE, LIQUOR, DELI, LAUNDRY ...; never a name, a
//! brand or a mark) rasterised into a coverage atlas the renderer samples on
//! the faces (eo-render `sign_letters`, draw.wgsl `sl_sign`). No GPU here.
//!
//! Fonts: four faces, a bold sans, a condensed sans, a serif and a script,
//! resolved from the system font folders at run time the way eo-ui and the
//! street sign atlas resolve theirs (`EO_UI_FONT_DIR` first). They are read,
//! never copied, and the atlas is built in memory only: nothing rendered here
//! is written into the repository or a shipped file. A missing face takes the
//! next one in its fallback order; with none installed `LetterFonts::system`
//! is None and the signs keep their emblems.
//!
//! The atlas: one layer per cell, `CELL_W` x `CELL_H` texels, two channels
//! (`Rg8Unorm`): r the letters' fill coverage, g a neon tube (the glyph
//! outline stroked; a script's own strokes). A cell's text box (its `aspect`,
//! width over height) is stretched over the whole layer, so a layer is the
//! text box whatever its aspect and the renderer samples it with the box's own
//! (u, v). A stacked cell (a blade sign's letters one under another) is stored
//! turned: layer u runs down the stack, layer v across it, right to left.
//! Letters stay inside a margin of every layer (the renderer's sampler
//! repeats), so nothing reaches a layer's edge.

use ab_glyph::{Font, FontVec, GlyphId, OutlineCurve};
use std::path::{Path, PathBuf};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Layer size (texels). 96 texels over a text box: a 0.4 m box seen from 5 m
/// fills about 90 screen pixels at 1080p, so a near sign is drawn at about
/// one texel a pixel.
pub const CELL_W: u32 = 512;
pub const CELL_H: u32 = 96;
/// Bumped when the drawing of a cell changes.
pub const VERSION: &str = "eo-props-shop-lettering-v1";

/// The face slots (`CellSpec::face`).
pub const FACE_BOLD: u8 = 0;
pub const FACE_CONDENSED: u8 = 1;
pub const FACE_SERIF: u8 = 2;
pub const FACE_SCRIPT: u8 = 3;

/// Letter cap height over the text box height (caps faces), and a script's
/// ink height over it. The box keeps the rest as margin for the tube's halo.
pub const CAP_SHARE: f32 = 0.60;
pub const SCRIPT_SHARE: f32 = 0.82;
/// Most of a box's width the text may take.
pub const WIDTH_SHARE: f32 = 0.90;
/// A neon tube's width over the cap height (a 12 mm tube on 300 mm letters
/// is 0.04; drawn a little wider so it holds at a distance).
pub const TUBE_SHARE: f32 = 0.065;

/// One cell to draw: its text (already in the case it is drawn in), face
/// slot, whether its letters are stacked, and its text box's aspect (width
/// over height; for a stacked cell, the stack's length over its width).
#[derive(Clone, Copy, Debug)]
pub struct CellSpec<'a> {
    pub text: &'a str,
    pub face: u8,
    pub stacked: bool,
    pub aspect: f32,
}

/// The four faces, each the first of its candidates installed.
pub struct LetterFonts {
    faces: [Option<(FontVec, PathBuf)>; 4],
}

/// Single-face files per slot, in preference order (macOS, Windows, Linux).
const CANDIDATES: [&[&str]; 4] = [
    &["Arial Bold.ttf", "arialbd.ttf", "ARIALBD.TTF", "LiberationSans-Bold.ttf", "DejaVuSans-Bold.ttf", "FreeSansBold.ttf", "Verdana Bold.ttf", "verdanab.ttf"],
    &["DIN Condensed Bold.ttf", "Impact.ttf", "impact.ttf", "Arial Narrow Bold.ttf", "ARIALNB.TTF", "arialnb.ttf", "LiberationSansNarrow-Bold.ttf", "DejaVuSansCondensed-Bold.ttf"],
    &["Georgia Bold.ttf", "georgiab.ttf", "Times New Roman Bold.ttf", "timesbd.ttf", "BigCaslon.ttf", "LiberationSerif-Bold.ttf", "DejaVuSerif-Bold.ttf", "FreeSerifBold.ttf"],
    &["Brush Script.ttf", "BRUSHSCI.TTF", "segoescb.ttf", "segoesc.ttf", "Apple Chancery.ttf", "Bradley Hand Bold.ttf"],
];
/// What a slot falls back to when its own face is missing.
const FALLBACK: [[u8; 4]; 4] = [[0, 1, 2, 3], [1, 0, 2, 3], [2, 0, 1, 3], [3, 2, 0, 1]];

/// System font folders, in lookup order (eo-ui's, with its `EO_UI_FONT_DIR`
/// override, and the folders the single-face files live in).
pub fn font_dirs() -> Vec<PathBuf> {
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
        for d in [
            "/System/Library/Fonts/Supplemental",
            "/System/Library/Fonts",
            "/Library/Fonts",
            "/usr/share/fonts/truetype/msttcorefonts",
            "/usr/share/fonts/truetype/liberation",
            "/usr/share/fonts/truetype/liberation2",
            "/usr/share/fonts/truetype/dejavu",
            "/usr/share/fonts/truetype/freefont",
            "/usr/share/fonts/truetype",
            "/usr/share/fonts",
        ] {
            v.push(PathBuf::from(d));
        }
    }
    v
}

impl LetterFonts {
    /// The faces in the system font folders; None when not one is installed.
    pub fn system() -> Option<Self> {
        Self::from_dirs(&font_dirs())
    }

    /// The faces in `dirs` (the first candidate found per slot).
    pub fn from_dirs(dirs: &[PathBuf]) -> Option<Self> {
        let mut faces: [Option<(FontVec, PathBuf)>; 4] = [None, None, None, None];
        for (slot, names) in CANDIDATES.iter().enumerate() {
            'found: for d in dirs {
                for n in names.iter() {
                    let p = d.join(n);
                    if !p.is_file() {
                        continue;
                    }
                    if let Some(f) = std::fs::read(&p).ok().and_then(|b| FontVec::try_from_vec(b).ok()) {
                        faces[slot] = Some((f, p));
                        break 'found;
                    }
                }
            }
        }
        faces.iter().any(|f| f.is_some()).then_some(Self { faces })
    }

    /// The file each slot resolved to (None: it takes a fallback).
    pub fn files(&self) -> [Option<&Path>; 4] {
        [0, 1, 2, 3].map(|k| self.faces[k].as_ref().map(|f| f.1.as_path()))
    }

    /// The face a slot draws with: its own, else the first of its fallbacks
    /// installed that has every character of `text`.
    fn face_for(&self, slot: u8, text: &str) -> Option<(&FontVec, u8)> {
        let order = FALLBACK[(slot as usize).min(3)];
        let has_all = |f: &FontVec| text.chars().all(|c| c == ' ' || f.glyph_id(c).0 != 0);
        order.iter().find_map(|&k| self.faces[k as usize].as_ref().filter(|f| has_all(&f.0)).map(|f| (&f.0, k)))
    }
}

/// A run of glyph outlines as a path (y down) at `scale` px per font unit,
/// the pen from `x0` on the baseline `y0`, `tracking` px between letters.
fn text_path(font: &FontVec, text: &str, scale: f32, tracking: f32, x0: f32, y0: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let mut pen = x0;
    let mut prev: Option<GlyphId> = None;
    for ch in text.chars() {
        let g = font.glyph_id(ch);
        if let Some(p) = prev {
            pen += font.kern_unscaled(p, g) * scale;
        }
        if let Some(outline) = font.outline(g) {
            let map = |p: ab_glyph::Point| (pen + p.x * scale, y0 - p.y * scale);
            let mut last: Option<(f32, f32)> = None;
            for curve in &outline.curves {
                let (a, rest): ((f32, f32), Vec<(f32, f32)>) = match *curve {
                    OutlineCurve::Line(p0, p1) => (map(p0), vec![map(p1)]),
                    OutlineCurve::Quad(p0, p1, p2) => (map(p0), vec![map(p1), map(p2)]),
                    OutlineCurve::Cubic(p0, p1, p2, p3) => (map(p0), vec![map(p1), map(p2), map(p3)]),
                };
                let joined = last.is_some_and(|l| (l.0 - a.0).abs() < 1e-3 && (l.1 - a.1).abs() < 1e-3);
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
            }
            if last.is_some() {
                pb.close();
            }
        }
        pen += font.h_advance_unscaled(g) * scale + tracking;
        prev = Some(g);
    }
    pb.finish()
}

/// Advance width of `text` in font units (with kerning), and the number of letters.
fn advance(font: &FontVec, text: &str) -> (f32, usize) {
    let mut w = 0.0;
    let mut prev: Option<GlyphId> = None;
    for ch in text.chars() {
        let g = font.glyph_id(ch);
        if let Some(p) = prev {
            w += font.kern_unscaled(p, g);
        }
        w += font.h_advance_unscaled(g);
        prev = Some(g);
    }
    (w, text.chars().count())
}

/// Ink bounds (x0, y0, x1, y1; font units, y up) of `text` set at scale 1.
fn ink_bounds(font: &FontVec, text: &str) -> Option<[f32; 4]> {
    let mut pen = 0.0;
    let mut prev: Option<GlyphId> = None;
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for ch in text.chars() {
        let g = font.glyph_id(ch);
        if let Some(p) = prev {
            pen += font.kern_unscaled(p, g);
        }
        if let Some(o) = font.outline(g) {
            // (ab_glyph's bounds are y down: min.y the glyph's top, max.y its bottom, in y-up units)
            b = [b[0].min(pen + o.bounds.min.x), b[1].min(o.bounds.max.y), b[2].max(pen + o.bounds.max.x), b[3].max(o.bounds.min.y)];
        }
        pen += font.h_advance_unscaled(g);
        prev = Some(g);
    }
    (b[0] < b[2] && b[1] < b[3]).then_some(b)
}

/// Cap height in font units (the top of 'H', else 0.7 em).
fn cap_height(font: &FontVec) -> f32 {
    let g = font.glyph_id('H');
    font.outline(g).map(|o| o.bounds.min.y).filter(|h| *h > 0.0).unwrap_or(0.7 * font.units_per_em().unwrap_or(1000.0))
}

fn fill(pix: &mut Pixmap, path: &tiny_skia::Path, ts: Transform) {
    let mut paint = Paint::default();
    paint.anti_alias = true;
    paint.set_color_rgba8(255, 255, 255, 255);
    pix.fill_path(path, &paint, FillRule::Winding, ts, None);
}

/// The glyph paths of a cell in its true box (`w` x `h` px, y down), and
/// the tube width (px): one path a letter group (the whole word, or one
/// path per stacked letter).
fn layout(font: &FontVec, slot: u8, cell: &CellSpec, w: f32, h: f32) -> (Vec<tiny_skia::Path>, f32) {
    let script = slot == FACE_SCRIPT;
    let cap = cap_height(font);
    if cell.stacked {
        // letters one under another, each centred across the column.
        let letters: Vec<char> = cell.text.chars().filter(|c| *c != ' ').collect();
        let n = letters.len().max(1) as f32;
        let pitch = h / n;
        let ch = (0.6 * w).min(0.74 * pitch);
        let s = ch / cap;
        let mut out = vec![];
        for (k, c) in letters.iter().enumerate() {
            let t = c.to_string();
            let (adv, _) = advance(font, &t);
            let lw = ink_bounds(font, &t).map(|b| b[2] - b[0]).unwrap_or(adv);
            let s = s.min(0.86 * w / lw.max(1.0));
            let x0 = (w - lw * s) / 2.0 - ink_bounds(font, &t).map(|b| b[0] * s).unwrap_or(0.0);
            let base = pitch * (k as f32 + 0.5) + ch / 2.0;
            if let Some(p) = text_path(font, &t, s, 0.0, x0, base) {
                out.push(p);
            }
        }
        return (out, TUBE_SHARE * ch);
    }
    if script {
        // a script by its ink: centred, as tall as the box allows.
        let Some(b) = ink_bounds(font, cell.text) else { return (vec![], 0.0) };
        let s = (SCRIPT_SHARE * h / (b[3] - b[1])).min(WIDTH_SHARE * w / (b[2] - b[0]));
        let x0 = (w - (b[2] - b[0]) * s) / 2.0 - b[0] * s;
        let y0 = (h - (b[3] - b[1]) * s) / 2.0 + b[3] * s;
        let tube = TUBE_SHARE * cap * s;
        return (text_path(font, cell.text, s, 0.0, x0, y0).into_iter().collect(), tube);
    }
    // capitals: cap height a share of the box, narrowed to fit, letter-spaced
    // a little when the word is short for its box (as sign writers space it).
    let ch = CAP_SHARE * h;
    let mut s = ch / cap;
    let (adv, n) = advance(font, cell.text);
    let mut tracking = 0.0;
    let wide = adv * s;
    if wide > WIDTH_SHARE * w {
        s *= WIDTH_SHARE * w / wide;
    } else if n > 1 && wide < 0.8 * w {
        tracking = ((0.84 * w - wide) / (n - 1) as f32).min(0.16 * ch);
    }
    let total = adv * s + tracking * (n.max(1) - 1) as f32;
    let ch = cap * s;
    let x0 = (w - total) / 2.0;
    let y0 = (h + ch) / 2.0;
    (text_path(font, cell.text, s, tracking, x0, y0).into_iter().collect(), TUBE_SHARE * ch)
}

/// One cell's layer: `CELL_W * CELL_H` texel pairs (fill, tube), rows top first.
/// None when no installed face has the cell's characters.
pub fn rasterise_cell(fonts: &LetterFonts, cell: &CellSpec) -> Option<Vec<u8>> {
    let (font, slot) = fonts.face_for(cell.face, cell.text)?;
    let script = slot == FACE_SCRIPT;
    // the true box (px): the layer height across a horizontal box, the
    // layer height across a stacked column.
    let aspect = cell.aspect.clamp(0.25, 40.0);
    let (w, h) = if cell.stacked { (CELL_H as f32, CELL_H as f32 * aspect) } else { (CELL_H as f32 * aspect, CELL_H as f32) };
    let (paths, tube) = layout(font, slot, cell, w, h);
    if paths.is_empty() {
        return None;
    }
    // true box -> layer: stretched along u; a stack turned (layer x down the stack, layer y across it).
    let ts = if cell.stacked { Transform::from_row(0.0, 1.0, CELL_W as f32 / h, 0.0, 0.0, 0.0) } else { Transform::from_row(CELL_W as f32 / w, 0.0, 0.0, 1.0, 0.0, 0.0) };
    let mut fill_pix = Pixmap::new(CELL_W, CELL_H)?;
    let mut tube_pix = Pixmap::new(CELL_W, CELL_H)?;
    let stroke = Stroke { width: tube.max(0.8), miter_limit: 4.0, line_cap: LineCap::Round, line_join: LineJoin::Round, dash: None };
    for p in &paths {
        fill(&mut fill_pix, p, ts);
        if script {
            fill(&mut tube_pix, p, ts);
        } else if let Some(outline) = p.stroke(&stroke, 1.0) {
            // stroked in the true box, then stretched: the tube keeps its width on the sign.
            fill(&mut tube_pix, &outline, ts);
        }
    }
    let mut out = Vec::with_capacity((CELL_W * CELL_H * 2) as usize);
    for (a, b) in fill_pix.pixels().iter().zip(tube_pix.pixels()) {
        out.push(a.alpha());
        out.push(b.alpha());
    }
    Some(out)
}

/// The atlas: `layers` layers of `CELL_W` x `CELL_H`, every mip level (each
/// level all layers in order, rows top first, two bytes a texel).
pub struct LetterAtlas {
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub levels: Vec<Vec<u8>>,
    /// Cells no installed face could draw (left empty).
    pub missing: Vec<usize>,
}

impl LetterAtlas {
    pub fn bytes(&self) -> u64 {
        self.levels.iter().map(|l| l.len() as u64).sum()
    }
    /// Level `m`'s size.
    pub fn level_size(&self, m: usize) -> (u32, u32) {
        ((self.width >> m).max(1), (self.height >> m).max(1))
    }
    /// One layer of level 0.
    pub fn layer(&self, k: usize) -> &[u8] {
        let n = (self.width * self.height * 2) as usize;
        &self.levels[0][k * n..(k + 1) * n]
    }
}

/// The mip levels below a level-0 stack of layers (2x2 box filter; an odd
/// last row or column is left out, a side of 1 stays 1).
pub fn mip_chain(level0: Vec<u8>, width: u32, height: u32, layers: u32) -> Vec<Vec<u8>> {
    let count = 32 - width.max(height).leading_zeros();
    let mut levels = vec![level0];
    let (mut w, mut h) = (width, height);
    for _ in 1..count {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let src = levels.last().unwrap();
        let mut dst = vec![0u8; (nw * nh * 2 * layers) as usize];
        for l in 0..layers as usize {
            let (so, d0) = (l * (w * h * 2) as usize, l * (nw * nh * 2) as usize);
            for y in 0..nh {
                for x in 0..nw {
                    let xs = [2 * x, (2 * x + 1).min(w - 1)];
                    let ys = [2 * y, (2 * y + 1).min(h - 1)];
                    for c in 0..2 {
                        let mut sum = 0u32;
                        for yy in ys {
                            for xx in xs {
                                sum += src[so + ((yy * w + xx) * 2 + c) as usize] as u32;
                            }
                        }
                        dst[d0 + ((y * nw + x) * 2 + c) as usize] = ((sum + 2) / 4) as u8;
                    }
                }
            }
        }
        levels.push(dst);
        (w, h) = (nw, nh);
    }
    levels
}

/// Draw every cell (layer k = cell k) and its mip chain. Deterministic for
/// the same fonts and cells.
pub fn build(fonts: &LetterFonts, cells: &[CellSpec]) -> LetterAtlas {
    let n = (CELL_W * CELL_H * 2) as usize;
    let layers = cells.len().max(1) as u32;
    let mut level0 = vec![0u8; n * layers as usize];
    let mut missing = vec![];
    for (k, c) in cells.iter().enumerate() {
        match rasterise_cell(fonts, c) {
            Some(px) => level0[k * n..(k + 1) * n].copy_from_slice(&px),
            None => missing.push(k),
        }
    }
    let levels = mip_chain(level0, CELL_W, CELL_H, layers);
    LetterAtlas { width: CELL_W, height: CELL_H, layers, levels, missing }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells() -> Vec<CellSpec<'static>> {
        vec![
            CellSpec { text: "LIQUOR", face: FACE_CONDENSED, stacked: false, aspect: 3.0 },
            CellSpec { text: "Bakery", face: FACE_SCRIPT, stacked: false, aspect: 2.6 },
            CellSpec { text: "DRY CLEANERS", face: FACE_BOLD, stacked: false, aspect: 7.0 },
            CellSpec { text: "BAR", face: FACE_SERIF, stacked: false, aspect: 2.0 },
            CellSpec { text: "DELI", face: FACE_CONDENSED, stacked: true, aspect: 3.1 },
        ]
    }

    #[test]
    fn mip_chain_halves_to_one_texel() {
        let levels = mip_chain(vec![255; (CELL_W * CELL_H * 2 * 3) as usize], CELL_W, CELL_H, 3);
        assert_eq!(levels.len(), 10, "512 x 96 has ten levels");
        let (mut w, mut h) = (CELL_W, CELL_H);
        for l in &levels {
            assert_eq!(l.len(), (w * h * 2 * 3) as usize);
            assert!(l.iter().all(|v| *v == 255), "a full layer stays full");
            (w, h) = ((w / 2).max(1), (h / 2).max(1));
        }
    }

    #[test]
    fn no_fonts_no_atlas() {
        assert!(LetterFonts::from_dirs(&[]).is_none());
        assert!(LetterFonts::from_dirs(&[PathBuf::from("/nonexistent-font-folder")]).is_none());
    }

    /// With the machine's fonts (skipped where none is installed): every cell
    /// drawn, inside its layer's margin, the same bytes twice; a stacked cell
    /// turned (its letters run along the layer).
    #[test]
    fn cells_are_drawn_inside_their_layer_and_the_same_each_time() {
        let Some(fonts) = LetterFonts::system() else {
            eprintln!("no system font: skipped");
            return;
        };
        let cells = cells();
        let a = build(&fonts, &cells);
        let b = build(&fonts, &cells);
        assert_eq!(a.levels, b.levels, "deterministic");
        assert!(a.missing.is_empty(), "{:?}", a.missing);
        assert_eq!(a.layers as usize, cells.len());
        for k in 0..cells.len() {
            let px = a.layer(k);
            let at = |x: u32, y: u32, c: usize| px[((y * CELL_W + x) * 2) as usize + c];
            let (mut x0, mut x1, mut y0, mut y1, mut ink) = (u32::MAX, 0, u32::MAX, 0, 0u64);
            for y in 0..CELL_H {
                for x in 0..CELL_W {
                    if at(x, y, 0) > 0 || at(x, y, 1) > 0 {
                        (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
                    }
                    ink += at(x, y, 0) as u64;
                }
            }
            assert!(ink > 255 * 800, "cell {k} ({}) is drawn", cells[k].text);
            assert!(x0 >= 2 && x1 + 2 < CELL_W && y0 >= 2 && y1 + 2 < CELL_H, "cell {k} ({}) inside its margin: {x0}..{x1} x {y0}..{y1}", cells[k].text);
            // a box's text spans most of its width, and so does a turned stack.
            assert!((x1 - x0) as f32 > 0.5 * CELL_W as f32, "cell {k} ({}) spans {x0}..{x1}", cells[k].text);
        }
    }
}
