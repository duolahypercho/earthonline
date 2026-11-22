//! Text: CSS font stacks resolved against the fonts the operating system
//! provides (as the browser does for the JS game, which ships no font
//! files), glyphs rasterised with `ab_glyph` into one R8 atlas, and shaped
//! runs cached by content so an unchanged label is never re-shaped.
//!
//! Font licences: the stacks name system fonts (Bahnschrift, Segoe UI,
//! Segoe UI Variable, Consolas, Arial on Windows). They are read from the
//! system font folder at run time and never copied or redistributed, the
//! same use the browser makes of them. When none of a stack is installed
//! the fallback is Kenney's `kenpixel.ttf` (CC0 1.0), copied from three.js
//! to `native/assets/fonts/` with its licence note beside it.

use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont, VariableFont, point};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::theme::{self, FontStack};

/// Which CSS stack a style draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stack {
    TitleDisplay,
    TitleSans,
    HudDisplay,
    HudSans,
    HudMono,
    Pause,
    PauseCondensed,
}

impl Stack {
    pub fn families(self) -> FontStack {
        match self {
            Stack::TitleDisplay => theme::title::DISPLAY,
            Stack::TitleSans => theme::title::SANS,
            Stack::HudDisplay => theme::hud::FONT_DISPLAY,
            Stack::HudSans => theme::hud::FONT_SANS,
            Stack::HudMono => theme::hud::FONT_MONO,
            Stack::Pause => theme::pause::FONT,
            Stack::PauseCondensed => theme::pause::FONT_CONDENSED,
        }
    }
}

/// A text style: the CSS `font-family`, `font-weight`, `font-stretch`,
/// `font-size`, `letter-spacing` (em) and `text-transform: uppercase`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub stack: Stack,
    pub weight: u16,
    pub stretch: u16,
    pub size: f32,
    pub tracking_em: f32,
    pub upper: bool,
}

impl TextStyle {
    pub const fn new(stack: Stack, weight: u16, size: f32) -> Self {
        Self { stack, weight, stretch: 100, size, tracking_em: 0.0, upper: false }
    }
    pub const fn tracking(mut self, em: f32) -> Self {
        self.tracking_em = em;
        self
    }
    pub const fn upper(mut self) -> Self {
        self.upper = true;
        self
    }
    pub const fn stretch(mut self, s: u16) -> Self {
        self.stretch = s;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FaceKey {
    stack: Stack,
    weight: u16,
    stretch: u16,
}

/// What a stack resolved to, for the report.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub css_family: String,
    pub file: PathBuf,
    pub variations: Vec<(String, f32)>,
}

struct Face {
    font: FontVec,
    resolved: Resolved,
}

/// A face's index in a font collection (.ttc), carried with the variations.
const COLLECTION_INDEX: &str = "\0ttc";

/// System font folders, in lookup order.
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
        v.push(PathBuf::from("/usr/share/fonts"));
        v.push(PathBuf::from("/System/Library/Fonts"));
        // macOS keeps Arial and the other single-face files here (the folder above holds collections).
        v.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
        v.push(PathBuf::from("/Library/Fonts"));
    }
    v
}

fn find(dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    for d in dirs {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// One CSS family name to a file and variation coordinates, the way the
/// browser's font matching picks a face for a weight (CSS Fonts 4 §5.2:
/// a weight above 500 takes the nearest heavier face first).
fn resolve_family(name: &str, weight: u16, stretch: u16, dirs: &[PathBuf]) -> Option<(PathBuf, Vec<(&'static str, f32)>)> {
    let w = weight as f32;
    let bold = weight >= 600;
    match name.to_ascii_lowercase().as_str() {
        "bahnschrift" => find(dirs, &["bahnschrift.ttf"]).map(|p| (p, vec![("wght", w.clamp(300.0, 700.0)), ("wdth", (stretch as f32).clamp(75.0, 100.0))])),
        "segoe ui variable text" => find(dirs, &["SegUIVar.ttf"]).map(|p| (p, vec![("wght", w.clamp(300.0, 700.0)), ("opsz", 10.5)])),
        "segoe ui" => {
            let file = match weight {
                0..=300 => "segoeuil.ttf",
                301..=399 => "segoeuisl.ttf",
                400..=599 => "segoeui.ttf",
                600..=649 => "seguisb.ttf",
                650..=849 => "segoeuib.ttf",
                _ => "seguibl.ttf",
            };
            find(dirs, &[file, "segoeui.ttf"]).map(|p| (p, vec![]))
        }
        // macOS ships the stacks' first families as collections (Avenir Next.ttc,
        // Avenir Next Condensed.ttc): the face for a weight is its index in the
        // collection (Regular 7, Medium 5, Demi Bold 2, Bold 0, Heavy 8, Ultra Light 10).
        "avenir next" | "avenir next condensed" => {
            let file = if name.eq_ignore_ascii_case("avenir next") { "Avenir Next.ttc" } else { "Avenir Next Condensed.ttc" };
            let index = match weight {
                0..=250 => 10.0,
                251..=450 => 7.0,
                451..=550 => 5.0,
                551..=650 => 2.0,
                651..=800 => 0.0,
                _ => 8.0,
            };
            find(dirs, &[file]).map(|p| (p, vec![(COLLECTION_INDEX, index)]))
        }
        "arial narrow" => find(dirs, if bold { &["ARIALNB.TTF", "arialnb.ttf", "Arial Narrow Bold.ttf"] } else { &["ARIALN.TTF", "arialn.ttf", "Arial Narrow.ttf"] }).map(|p| (p, vec![])),
        "consolas" | "ui-monospace" | "monospace" => find(dirs, if bold { &["consolab.ttf"] } else { &["consola.ttf"] }).map(|p| (p, vec![])),
        "sans-serif" => find(dirs, if bold { &["arialbd.ttf", "DejaVuSans-Bold.ttf", "Arial Bold.ttf"] } else { &["arial.ttf", "DejaVuSans.ttf", "Arial.ttf"] }).map(|p| (p, vec![])),
        _ => None,
    }
}

/// A shaped, positioned run of glyphs (relative to the pen origin on the baseline).
#[derive(Clone, Debug, Default)]
pub struct Run {
    text: String,
    style_bits: [u32; 6],
    generation: u32,
    pub glyphs: Vec<PlacedGlyph>,
    /// Advance width including the trailing letter-spacing, as CSS lays it out.
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PlacedGlyph {
    /// Bitmap top-left relative to the pen origin on the baseline, px.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Atlas texel rectangle.
    pub uv: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    face: u16,
    glyph: u16,
    size_q: u32,
    sub: u8,
}

pub const ATLAS_SIZE: u32 = 2048;
const SUBPIXEL: f32 = 4.0;

/// The glyph atlas: an R8 coverage image packed in shelves. The GPU copy is
/// refreshed from `dirty` rows only when new glyphs land.
pub struct Atlas {
    pub pixels: Vec<u8>,
    shelves: Vec<(u32, u32, u32)>, // y, height, next x
    next_y: u32,
    glyphs: HashMap<GlyphKey, Option<[u32; 4]>>,
    /// Rows written since the last upload (min y, max y exclusive).
    pub dirty: Option<(u32, u32)>,
    pub generation: u32,
}

impl Atlas {
    fn new() -> Self {
        Self { pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE) as usize], shelves: vec![], next_y: 0, glyphs: HashMap::new(), dirty: None, generation: 1 }
    }
    fn clear(&mut self) {
        self.pixels.fill(0);
        self.shelves.clear();
        self.next_y = 0;
        self.glyphs.clear();
        self.dirty = Some((0, ATLAS_SIZE));
        self.generation += 1;
    }
    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (w, h) = (w + 1, h + 1);
        for s in self.shelves.iter_mut() {
            if h <= s.1 && s.1 <= h + h / 3 + 2 && s.2 + w <= ATLAS_SIZE {
                let x = s.2;
                s.2 += w;
                return Some((x, s.0));
            }
        }
        if self.next_y + h > ATLAS_SIZE {
            return None;
        }
        let y = self.next_y;
        self.next_y += h;
        self.shelves.push((y, h, w));
        Some((0, y))
    }
    fn mark(&mut self, y0: u32, y1: u32) {
        self.dirty = Some(match self.dirty {
            Some((a, b)) => (a.min(y0), b.max(y1)),
            None => (y0, y1),
        });
    }
}

/// Fonts, the atlas and the run cache.
pub struct TextSystem {
    dirs: Vec<PathBuf>,
    fallback: Option<PathBuf>,
    faces: Vec<Face>,
    face_of: HashMap<FaceKey, u16>,
    file_bytes: HashMap<PathBuf, Vec<u8>>,
    pub atlas: Atlas,
    runs: HashMap<u64, Run>,
    /// Shaping work done (cache misses), for the "no re-shaping" check.
    pub shaped_runs: u64,
    pub rasterised_glyphs: u64,
    scratch: String,
}

fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl TextSystem {
    /// `repo_root` locates the CC0 fallback font only.
    pub fn new(repo_root: Option<&Path>) -> Self {
        let fallback = repo_root.map(|r| r.join("native/assets/fonts/kenpixel.ttf")).filter(|p| p.is_file());
        Self {
            dirs: font_dirs(),
            fallback,
            faces: vec![],
            face_of: HashMap::new(),
            file_bytes: HashMap::new(),
            atlas: Atlas::new(),
            runs: HashMap::new(),
            shaped_runs: 0,
            rasterised_glyphs: 0,
            scratch: String::new(),
        }
    }

    /// The faces resolved so far: (stack, weight, stretch) -> family, file, variations.
    pub fn resolved(&self) -> Vec<(String, Resolved)> {
        let mut v: Vec<(String, Resolved)> = self
            .face_of
            .iter()
            .map(|(k, &i)| (format!("{:?} {} {}%", k.stack, k.weight, k.stretch), self.faces[i as usize].resolved.clone()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    fn face(&mut self, stack: Stack, weight: u16, stretch: u16) -> u16 {
        let key = FaceKey { stack, weight, stretch };
        if let Some(&i) = self.face_of.get(&key) {
            return i;
        }
        let mut chosen: Option<(String, PathBuf, Vec<(&'static str, f32)>)> = None;
        for fam in stack.families() {
            if let Some((p, v)) = resolve_family(fam, weight, stretch, &self.dirs) {
                chosen = Some((fam.to_string(), p, v));
                break;
            }
        }
        if chosen.is_none() {
            if let Some(p) = &self.fallback {
                chosen = Some(("kenpixel (CC0 fallback)".into(), p.clone(), vec![]));
            }
        }
        let Some((family, path, vars)) = chosen else {
            panic!("no font found for {:?}: none of {:?} is installed and the CC0 fallback is missing", stack, stack.families());
        };
        let bytes = match self.file_bytes.get(&path) {
            Some(b) => b.clone(),
            None => {
                let b = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                self.file_bytes.insert(path.clone(), b.clone());
                b
            }
        };
        let index = vars.iter().find(|(t, _)| *t == COLLECTION_INDEX).map(|(_, i)| *i as u32).unwrap_or(0);
        let mut font = FontVec::try_from_vec_and_index(bytes, index).unwrap_or_else(|e| panic!("parse {} (face {index}): {e}", path.display()));
        let mut applied = vec![];
        for (tag, value) in vars.iter().filter(|(t, _)| *t != COLLECTION_INDEX) {
            let t: [u8; 4] = tag.as_bytes().try_into().unwrap();
            if font.set_variation(&t, *value) {
                applied.push((tag.to_string(), *value));
            }
        }
        let i = self.faces.len() as u16;
        self.faces.push(Face { font, resolved: Resolved { css_family: family, file: path, variations: applied } });
        self.face_of.insert(key, i);
        i
    }

    fn scale_of(font: &FontVec, size: f32) -> PxScale {
        let upem = font.units_per_em().unwrap_or(1000.0);
        PxScale::from(size * font.height_unscaled() / upem)
    }

    /// Line metrics for a style (ascent, descent as positive px).
    pub fn metrics(&mut self, style: TextStyle) -> (f32, f32) {
        let f = self.face(style.stack, style.weight, style.stretch);
        let font = &self.faces[f as usize].font;
        let s = font.as_scaled(Self::scale_of(font, style.size));
        (s.ascent(), -s.descent())
    }

    /// The baseline y for text whose CSS line box starts at `top` with
    /// `line_height` (a multiple of the font size), half-leading split.
    pub fn baseline(&mut self, style: TextStyle, top: f32, line_height: f32) -> f32 {
        let (a, d) = self.metrics(style);
        let lh = line_height * style.size;
        top + (lh - (a + d)) * 0.5 + a
    }

    fn key(text: &str, style: TextStyle) -> (u64, [u32; 6]) {
        let bits = [style.stack as u32, style.weight as u32, style.stretch as u32, style.size.to_bits(), style.tracking_em.to_bits(), style.upper as u32];
        let mut h = fnv(text.as_bytes(), 0xcbf29ce484222325);
        h = fnv(bytemuck::cast_slice(&bits), h);
        (h, bits)
    }

    /// The shaped run for `text` in `style`, shaped once and then served from
    /// the cache (no allocation on a hit).
    pub fn run(&mut self, text: &str, style: TextStyle) -> &Run {
        let (h, bits) = Self::key(text, style);
        let generation = self.atlas.generation;
        let hit = matches!(self.runs.get(&h), Some(r) if r.text == text && r.style_bits == bits && r.generation == generation);
        if !hit {
            let mut run = self.shape(text, style);
            if run.generation != self.atlas.generation {
                // The atlas filled and was cleared while shaping: shape again into the fresh one.
                run = self.shape(text, style);
            }
            run.text = text.to_string();
            run.style_bits = bits;
            self.runs.insert(h, run);
        }
        &self.runs[&h]
    }

    /// Width of `text` in `style` (shapes on a miss).
    pub fn width(&mut self, text: &str, style: TextStyle) -> f32 {
        self.run(text, style).width
    }

    fn shape(&mut self, text: &str, style: TextStyle) -> Run {
        self.shaped_runs += 1;
        let f = self.face(style.stack, style.weight, style.stretch);
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        if style.upper {
            for c in text.chars() {
                scratch.extend(c.to_uppercase());
            }
        } else {
            scratch.push_str(text);
        }
        let generation = self.atlas.generation;
        let (ascent, descent, ids) = {
            let font = &self.faces[f as usize].font;
            let scale = Self::scale_of(font, style.size);
            let s = font.as_scaled(scale);
            let ids: Vec<(GlyphId, f32, f32)> = {
                let mut out = Vec::with_capacity(scratch.len());
                let mut prev: Option<GlyphId> = None;
                for c in scratch.chars() {
                    let id = s.glyph_id(c);
                    let kern = prev.map(|p| s.kern(p, id)).unwrap_or(0.0);
                    out.push((id, kern, s.h_advance(id)));
                    prev = Some(id);
                }
                out
            };
            (s.ascent(), -s.descent(), ids)
        };
        let spacing = style.tracking_em * style.size;
        let mut pen = 0.0f32;
        let mut glyphs = Vec::with_capacity(ids.len());
        for (id, kern, adv) in ids {
            pen += kern;
            if let Some(g) = self.raster(f, id, style.size, pen) {
                glyphs.push(g);
            }
            pen += adv + spacing;
        }
        self.scratch = scratch;
        Run { text: String::new(), style_bits: [0; 6], generation, glyphs, width: pen, ascent, descent }
    }

    /// Rasterise (or find) one glyph at a pen position; the bitmap is placed
    /// relative to the pen origin, at a quarter-pixel horizontal phase.
    fn raster(&mut self, face: u16, id: GlyphId, size: f32, pen: f32) -> Option<PlacedGlyph> {
        let base = pen.floor();
        let sub = (((pen - base) * SUBPIXEL).round() as u8).min(SUBPIXEL as u8 - 1);
        let key = GlyphKey { face, glyph: id.0, size_q: (size * 8.0).round() as u32, sub };
        let font = &self.faces[face as usize].font;
        let scale = Self::scale_of(font, size);
        let glyph = id.with_scale_and_position(scale, point(sub as f32 / SUBPIXEL, 0.0));
        let outlined = font.outline_glyph(glyph);
        let bounds = outlined.as_ref().map(|o| o.px_bounds());
        let rect = match self.atlas.glyphs.get(&key) {
            Some(r) => *r,
            None => {
                let r = match (&outlined, bounds) {
                    (Some(o), Some(b)) => {
                        let (w, h) = (b.width().ceil() as u32, b.height().ceil() as u32);
                        if w == 0 || h == 0 {
                            None
                        } else {
                            let at = match self.atlas.alloc(w, h) {
                                Some(a) => a,
                                None => {
                                    self.atlas.clear();
                                    self.atlas.alloc(w, h)?
                                }
                            };
                            let (ax, ay) = at;
                            let pixels = &mut self.atlas.pixels;
                            o.draw(|x, y, c| {
                                if x < w && y < h {
                                    let i = ((ay + y) * ATLAS_SIZE + ax + x) as usize;
                                    pixels[i] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
                                }
                            });
                            self.atlas.mark(ay, ay + h);
                            self.rasterised_glyphs += 1;
                            Some([ax, ay, w, h])
                        }
                    }
                    _ => None,
                };
                self.atlas.glyphs.insert(key, r);
                r
            }
        }?;
        let b = bounds?;
        Some(PlacedGlyph {
            x: base + b.min.x,
            y: b.min.y,
            w: rect[2] as f32,
            h: rect[3] as f32,
            uv: [rect[0] as f32, rect[1] as f32, (rect[0] + rect[2]) as f32, (rect[1] + rect[3]) as f32],
        })
    }
}
