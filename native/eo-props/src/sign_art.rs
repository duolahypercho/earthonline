//! authoritative sign artwork and an open sign font for the sign
//! atlases, read from a folder Git does not track, and verified.
//!
//! * **Provenance**: `eo-props/data/sign-art-manifest.json` (written by
//!   `native/tools/fetch-sign-art.mjs`) lists every downloaded file with its
//!   source URL (and the ZIP entry it came from), its licence or
//!   public-domain basis, the retrieval date, its size and SHA-256. Files are
//!   read from `<repo>/external/sign-art` (git-ignored, filled by
//!   `scripts/setup-assets.py`), or the folder `EO_SIGN_ART_ROOT` names
//!   instead. Every file
//!   is read and its SHA-256 compared with the manifest before anything is
//!   drawn; one missing or different file and [`SignArt::load`] fails, the
//!   caller keeps today's drawings (the browser game's canvas port in
//!   `sign_atlas`), and the log says why.
//! * **Sources**: FHWA Standard Highway Signs 2024 (MUTCD 11th Edition)
//!   vector layouts (public domain, 17 U.S.C. 105) are drawn as they are
//!   ([`super::svg`]). California-coded signs (Caltrans sign specification
//!   sheets, public domain per the Caltrans conditions of use) exist only as
//!   PDF drawings with dimension marks over the face, so they are composed
//!   here from the sheet's own dimension table (transcribed into the
//!   manifest's `layouts`, each with the sheet it came from) in the Overpass
//!   font (SIL OFL-1.1). California codes win where the CA MUTCD has one.
//! * **Text**: Overpass is not a metric copy of the FHWA alphabets. A legend
//!   line is set at the sheet's letter height (cap height); its width is the
//!   sheet's width where the sheet gives one, else Overpass's width times the
//!   series factor measured from the sheets' own width dimensions
//!   (`seriesCalibration`); the residual is reported by the tests.
//! * **Frames**: the atlas cells keep the JS layout (cell size and aspect). A
//!   face's art is fitted (uniform scale, centred) into the inventory's most
//!   common plate size for that face, and that frame is mapped onto the cell,
//!   so on a plate of that size the sign keeps its shape; the cell outside the
//!   sign is clear (alpha 0) and the plate's alpha test cuts it.
//!
//! No dimension of a California street standard is written here: sign sizes
//! come from the surveyed inventory and the sign art itself, letter heights
//! and spacings from the cited sheets.

use super::svg::Svg;
use super::{Fonts, encode_png};
use crate::data::sha256_hex;
use ab_glyph::{Font, FontVec, OutlineCurve};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Rect, Transform};

/// The repository's provenance manifest (small JSON, tracked).
pub const MANIFEST_JSON: &str = include_str!("../data/sign-art-manifest.json");

/// One downloaded file.
#[derive(Clone, Debug)]
pub struct ArtFile {
    pub id: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: String,
    pub licence: String,
    pub retrieved: String,
}

pub struct Manifest {
    pub files: Vec<ArtFile>,
    pub raw: Value,
}

impl Manifest {
    pub fn get() -> &'static Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Self::parse(MANIFEST_JSON).expect("eo-props/data/sign-art-manifest.json"))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let raw: Value = serde_json::from_str(text)?;
        let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
        let files = raw["files"]
            .as_array()
            .ok_or_else(|| anyhow!("sign art manifest: no files"))?
            .iter()
            .map(|f| ArtFile {
                id: s(&f["id"]),
                path: s(&f["path"]),
                bytes: f["bytes"].as_u64().unwrap_or(0),
                sha256: s(&f["sha256"]),
                url: s(&f["url"]),
                licence: s(&f["licence"]),
                retrieved: s(&f["retrieved"]),
            })
            .collect();
        Ok(Self { files, raw })
    }

    pub fn file(&self, id: &str) -> Option<&ArtFile> {
        self.files.iter().find(|f| f.id == id)
    }

    /// The art spec of a face (`faces`, `printed` or `propSheet` table).
    pub fn spec(&self, table: &str, face: &str) -> Option<&Value> {
        self.raw[table].get(face)
    }
}

/// The art's folder this run: the one `EO_SIGN_ART_ROOT` names, else
/// `<repo>/external/sign-art`; None when neither is known (the drawn faces stay).
pub fn root() -> Option<PathBuf> {
    eo_data::external::SIGN_ART.root()
}

/// The manifest's files, read and checked against their SHA-256 (all or nothing).
pub fn read_verified(root: &Path, man: &Manifest) -> Result<HashMap<String, Vec<u8>>> {
    if !root.is_dir() {
        bail!("{} is not there", root.display());
    }
    let mut out = HashMap::new();
    for f in &man.files {
        let p = root.join(&f.path);
        let bytes = std::fs::read(&p).with_context(|| format!("{} missing", p.display()))?;
        let got = sha256_hex(&bytes);
        if got != f.sha256 {
            bail!("{} has SHA-256 {got}, the manifest records {}", p.display(), f.sha256);
        }
        out.insert(f.id.clone(), bytes);
    }
    Ok(out)
}

/// The verified art: parsed layouts, the Overpass faces and the calibrations.
pub struct SignArt {
    pub root: PathBuf,
    svgs: HashMap<String, Svg>,
    /// Overpass Regular + Bold (the JS layouts' text where a face keeps its drawing).
    pub fonts: Fonts,
    bold: FontVec,
    semibold: FontVec,
    palette: HashMap<String, [u8; 4]>,
    /// Width of an FHWA series line over Overpass Bold's at the same cap height.
    pub series: HashMap<String, f64>,
    /// The calibration lines behind `series`: (series, text, factor).
    pub calibration: Vec<(String, String, f64)>,
    /// SHA-256 over the manifest and every file digest (part of the atlas cache key).
    pub digest: String,
}

fn colour(s: &str) -> Result<[u8; 4]> {
    let h = s.trim_start_matches('#');
    let n = u32::from_str_radix(h, 16).map_err(|_| anyhow!("colour {s:?}"))?;
    Ok(match h.len() {
        3 => [((n >> 8) & 15) as u8 * 17, ((n >> 4) & 15) as u8 * 17, (n & 15) as u8 * 17, 255],
        6 => [(n >> 16) as u8, (n >> 8) as u8, n as u8, 255],
        _ => bail!("colour {s:?}"),
    })
}

impl SignArt {
    /// Every manifest file verified, every layout parsed, the fonts loaded.
    pub fn load(root: &Path) -> Result<Self> {
        let man = Manifest::get();
        let files = read_verified(root, man)?;
        let mut svgs = HashMap::new();
        for f in &man.files {
            if f.path.ends_with(".svg") {
                let text = std::str::from_utf8(&files[&f.id]).with_context(|| f.path.clone())?;
                svgs.insert(f.id.clone(), Svg::parse(text).with_context(|| format!("parsing {}", f.path))?);
            }
        }
        let font = |role: &str| -> Result<FontVec> {
            let id = man.raw["fonts"][role].as_str().ok_or_else(|| anyhow!("manifest: fonts.{role}"))?;
            FontVec::try_from_vec(files.get(id).ok_or_else(|| anyhow!("font file {id}"))?.clone()).map_err(|e| anyhow!("{id}: {e}"))
        };
        let reg_id = man.raw["fonts"]["regular"].as_str().unwrap_or_default();
        let bold_id = man.raw["fonts"]["bold"].as_str().unwrap_or_default();
        let fonts = Fonts::from_bytes(files[reg_id].clone(), files[bold_id].clone(), root.join(&man.file(reg_id).map(|f| f.path.clone()).unwrap_or_default()), root.join(&man.file(bold_id).map(|f| f.path.clone()).unwrap_or_default()))?;
        let mut palette = HashMap::new();
        for (k, v) in man.raw["palette"].as_object().ok_or_else(|| anyhow!("manifest: palette"))? {
            palette.insert(k.clone(), colour(v["hex"].as_str().unwrap_or_default())?);
        }
        let mut d = Vec::new();
        d.extend_from_slice(MANIFEST_JSON.as_bytes());
        for f in &man.files {
            d.extend_from_slice(f.sha256.as_bytes());
        }
        let mut art = SignArt { root: root.to_path_buf(), svgs, fonts, bold: font("bold")?, semibold: font("semibold")?, palette, series: HashMap::new(), calibration: vec![], digest: sha256_hex(&d) };
        art.calibrate(&man.raw["seriesCalibration"])?;
        Ok(art)
    }

    /// [`SignArt::load`] from [`root`]; None (and one line on stderr) when the
    /// folder is missing or any file fails its check: the drawings stay.
    pub fn load_default() -> Option<Self> {
        let Some(r) = root() else {
            eprintln!("eo-props: official sign art not used (no sign art folder: {}); the sign atlases keep the drawn faces", eo_data::external::SIGN_ART.looked_in());
            return None;
        };
        match Self::load(&r) {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("eo-props: official sign art not used ({e:#}); the sign atlases keep the drawn faces (fetch it with `python3 scripts/setup-assets.py --only sign-art`, or point EO_SIGN_ART_ROOT at a copy)");
                None
            }
        }
    }

    pub fn svg(&self, id: &str) -> Result<&Svg> {
        self.svgs.get(id).ok_or_else(|| anyhow!("{id} is not an SVG of the manifest"))
    }

    pub fn colour(&self, name: &str) -> Result<[u8; 4]> {
        self.palette.get(name).copied().ok_or_else(|| anyhow!("palette has no {name:?}"))
    }

    fn face(&self, weight: &str) -> &FontVec {
        if weight == "semibold" { &self.semibold } else { &self.bold }
    }

    /// The series factors: each calibration line is a legend line whose width
    /// the sheet dimensions; the factor is that width over Overpass's width at
    /// the sheet's letter height; several lines of one series are averaged.
    fn calibrate(&mut self, cal: &Value) -> Result<()> {
        let mut sums: HashMap<String, (f64, f64)> = HashMap::new();
        for c in cal.as_array().ok_or_else(|| anyhow!("manifest: seriesCalibration"))? {
            let series = c["series"].as_str().unwrap_or_default().to_string();
            let text = c["text"].as_str().unwrap_or_default();
            let cap = c["capIn"].as_f64().ok_or_else(|| anyhow!("calibration capIn"))?;
            let width = c["widthIn"].as_f64().ok_or_else(|| anyhow!("calibration widthIn"))?;
            // The sheets dimension a legend line by its ink extent.
            let w = ink_width(self.face(c["weight"].as_str().unwrap_or("bold")), text, cap);
            let f = width / w;
            self.calibration.push((series.clone(), text.to_string(), f));
            let e = sums.entry(series).or_default();
            e.0 += f;
            e.1 += 1.0;
        }
        for (k, (s, n)) in sums {
            self.series.insert(k, s / n);
        }
        // A series no sheet dimensions takes the factor named in `seriesFallback`.
        if let Some(fb) = cal_fallback() {
            for (k, v) in fb {
                if !self.series.contains_key(&k) {
                    let f = *self.series.get(&v).ok_or_else(|| anyhow!("seriesFallback {k} -> {v}: no calibration"))?;
                    self.series.insert(k, f);
                }
            }
        }
        Ok(())
    }

    pub fn series_factor(&self, s: &str) -> Result<f64> {
        self.series.get(s).copied().ok_or_else(|| anyhow!("no width factor for series {s}"))
    }

    /// Size of an art spec in inches (frame of a stack: widest x summed heights).
    pub fn spec_size(&self, spec: &Value) -> Result<[f64; 2]> {
        if let Some(id) = spec["svg"].as_str() {
            return Ok(self.svg_size_in(id, &spec["sizeIn"]));
        }
        if let Some(items) = spec["stack"].as_array() {
            let mut w: f64 = 0.0;
            let mut h = 0.0;
            for it in items {
                let s = self.spec_size(it)?;
                w = w.max(s[0]);
                h += s[1];
            }
            return Ok([w, h]);
        }
        if let Some(l) = spec["layout"].as_str() {
            let lay = layout(l)?;
            return Ok([num(&lay["plate"]["w"])?, num(&lay["plate"]["h"])?]);
        }
        bail!("art spec {spec}")
    }

    fn svg_size_in(&self, id: &str, override_in: &Value) -> [f64; 2] {
        if let (Some(w), Some(h)) = (override_in[0].as_f64(), override_in[1].as_f64()) {
            return [w, h];
        }
        let m = Manifest::get();
        let raw = m.raw["files"].as_array().and_then(|a| a.iter().find(|f| f["id"] == id));
        match raw.and_then(|f| Some([f["sizeIn"][0].as_f64()?, f["sizeIn"][1].as_f64()?])) {
            Some(s) => s,
            None => {
                let v = self.svgs.get(id).map(|s| s.size()).unwrap_or([1.0, 1.0]);
                [v[0] as f64, v[1] as f64]
            }
        }
    }

    /// Draws an art spec so that its `size` (inches) fills `dest` (pixels).
    pub fn draw_spec(&self, pix: &mut Pixmap, spec: &Value, dest: [f64; 4]) -> Result<()> {
        if let Some(id) = spec["svg"].as_str() {
            let svg = self.svg(id)?;
            let hide: Vec<[f32; 4]> = spec["replace"].as_array().map(|r| r.iter().filter_map(|x| region(&x["region"])).collect()).unwrap_or_default();
            if hide.is_empty() {
                svg.draw(pix, f32x4(dest), None, Transform::identity(), &|c| c);
            } else {
                // Shapes inside a replaced region are left out; the replacement legend is set in their place.
                let mut keep = svg.clone();
                let mut removed: Vec<Vec<Rect>> = vec![vec![]; hide.len()];
                keep.shapes.retain(|s| {
                    for (k, g) in hide.iter().enumerate() {
                        let b = s.bounds;
                        if b.left() >= g[0] && b.top() >= g[1] && b.right() <= g[0] + g[2] && b.bottom() <= g[1] + g[3] {
                            removed[k].push(b);
                            return false;
                        }
                    }
                    true
                });
                keep.draw(pix, f32x4(dest), None, Transform::identity(), &|c| c);
                let (sx, sy) = (dest[2] / svg.view[2] as f64, dest[3] / svg.view[3] as f64);
                for (k, r) in spec["replace"].as_array().into_iter().flatten().enumerate() {
                    let rm = &removed[k];
                    if rm.is_empty() {
                        bail!("{id}: replace region {} holds no shape", r["region"]);
                    }
                    // The replaced glyphs' own extent gives the letter height and the centre line.
                    let top = rm.iter().map(|b| b.top()).fold(f32::MAX, f32::min) as f64;
                    let bottom = rm.iter().map(|b| b.bottom()).fold(f32::MIN, f32::max) as f64;
                    let left = rm.iter().map(|b| b.left()).fold(f32::MAX, f32::min) as f64;
                    let right = rm.iter().map(|b| b.right()).fold(f32::MIN, f32::max) as f64;
                    let ink = self.colour(r["colour"].as_str().unwrap_or("black"))?;
                    let cap = (bottom - top) * sy;
                    let cx = dest[0] + (left + right) / 2.0 * sx;
                    let text = r["text"].as_str().unwrap_or_default();
                    // `fitWidth`: the new legend takes the replaced glyphs' ink width too.
                    let (factor, width) = if r["fitWidth"].as_bool() == Some(true) {
                        (1.0, Some((right - left) * sx))
                    } else {
                        (self.series_factor(r["series"].as_str().unwrap_or("C"))? * sx / sy, None)
                    };
                    self.text(pix, text, r["weight"].as_str().unwrap_or("bold"), cap, factor, width, cx, dest[1] + top * sy, ink, None);
                }
            }
            return Ok(());
        }
        if let Some(items) = spec["stack"].as_array() {
            let size = self.spec_size(spec)?;
            let (kx, ky) = (dest[2] / size[0], dest[3] / size[1]);
            let mut y = dest[1];
            for it in items {
                let s = self.spec_size(it)?;
                let d = [dest[0] + (size[0] - s[0]) / 2.0 * kx, y, s[0] * kx, s[1] * ky];
                self.draw_spec(pix, it, d)?;
                y += s[1] * ky;
            }
            return Ok(());
        }
        if let Some(l) = spec["layout"].as_str() {
            return self.compose(pix, l, dest);
        }
        bail!("art spec {spec}")
    }

    /// Sets a legend line: `cap` pixels tall from `top`, its ink centred on
    /// `cx`, horizontally scaled by `sx` (the series factor and any frame
    /// stretch), or fitted to `width` (ink extent) when given, then shrunk to
    /// `max_w`.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&self, pix: &mut Pixmap, text: &str, weight: &str, cap: f64, sx: f64, width: Option<f64>, cx: f64, top: f64, ink: [u8; 4], max_w: Option<f64>) {
        let font = self.face(weight);
        let Some(path) = glyph_path(font, text, cap / cap_height_em(font)) else { return };
        self.fill_line(pix, &[(path, 0.0)], sx, width, max_w, cx, top + cap, ink);
    }

    /// Fills glyph runs (path, baseline offset y) sharing one horizontal fit;
    /// the union of their ink is centred on `cx`.
    #[allow(clippy::too_many_arguments)]
    fn fill_line(&self, pix: &mut Pixmap, runs: &[(tiny_skia::Path, f64)], sx: f64, width: Option<f64>, max_w: Option<f64>, cx: f64, baseline: f64, ink: [u8; 4]) {
        let (l, r) = runs.iter().fold((f32::MAX, f32::MIN), |(l, r), (p, _)| (l.min(p.bounds().left()), r.max(p.bounds().right())));
        let natural = (r - l).max(1e-6) as f64;
        let mut k = width.map(|w| w / natural).unwrap_or(sx);
        if let Some(m) = max_w {
            if natural * k > m {
                k = m / natural;
            }
        }
        let tx = cx - k * (l + r) as f64 / 2.0;
        let mut paint = Paint::default();
        paint.anti_alias = true;
        paint.set_color_rgba8(ink[0], ink[1], ink[2], ink[3]);
        for (p, dy) in runs {
            let t = Transform::from_row(k as f32, 0.0, 0.0, 1.0, tx as f32, (baseline + dy) as f32);
            pix.fill_path(p, &paint, FillRule::Winding, t, None);
        }
    }

    /// A line of runs of different letter heights (`[text, series, cap, align]`;
    /// align `base` on the common baseline, `top` level with the previous
    /// run's top, `mid` on the previous run's middle), advances end to end.
    #[allow(clippy::too_many_arguments)]
    fn runs(&self, pix: &mut Pixmap, runs: &Value, top: f64, ky: f64, sx_frame: f64, width: Option<f64>, max_w: Option<f64>, cx: f64, ink: [u8; 4]) -> Result<()> {
        let font = self.face("bold");
        let list = runs.as_array().ok_or_else(|| anyhow!("runs"))?;
        let base_cap = list.iter().filter(|r| r[3] == "base").filter_map(|r| r[2].as_f64()).fold(0.0, f64::max) * ky;
        let baseline = top + base_cap;
        let mut out = vec![];
        let mut pen = 0.0;
        let (mut prev_top, mut prev_mid) = (top, top + base_cap / 2.0);
        let mut factor = 0.0;
        for r in list {
            let text = r[0].as_str().unwrap_or_default();
            let cap = r[2].as_f64().ok_or_else(|| anyhow!("run cap"))? * ky;
            factor = self.series_factor(r[1].as_str().unwrap_or("C"))?;
            let em = cap / cap_height_em(font);
            let dy = match r[3].as_str().unwrap_or("base") {
                "top" => prev_top + cap - baseline,
                "mid" => prev_mid + cap / 2.0 - baseline,
                _ => 0.0,
            };
            if let Some(p) = glyph_path(font, text, em).and_then(|p| p.transform(Transform::from_translate(pen as f32, 0.0))) {
                out.push((p, dy));
            }
            pen += text_width(font, text, cap);
            prev_top = baseline + dy - cap;
            prev_mid = baseline + dy - cap / 2.0;
        }
        self.fill_line(pix, &out, factor * sx_frame, width, max_w, cx, baseline, ink);
        Ok(())
    }

    /// A California layout composed from its sheet's dimension table.
    pub fn compose(&self, pix: &mut Pixmap, name: &str, dest: [f64; 4]) -> Result<()> {
        let lay = layout(name)?;
        let plate = &lay["plate"];
        let (pw, ph) = (num(&plate["w"])?, num(&plate["h"])?);
        let (kx, ky) = (dest[2] / pw, dest[3] / ph);
        let px = |x: f64| dest[0] + x * kx;
        let py = |y: f64| dest[1] + y * ky;
        let paint_of = |c: [u8; 4]| {
            let mut p = Paint::default();
            p.anti_alias = true;
            p.set_color_rgba8(c[0], c[1], c[2], c[3]);
            p
        };
        let rr = |x0: f64, y0: f64, x1: f64, y1: f64, r: f64| round_rect(px(x0) as f32, py(y0) as f32, px(x1) as f32, py(y1) as f32, (r * kx) as f32, (r * ky) as f32);
        let radius = num(&plate["r"])?;
        let bg = self.colour(plate["fill"].as_str().unwrap_or("white"))?;
        if let Some(p) = rr(0.0, 0.0, pw, ph, radius) {
            pix.fill_path(&p, &paint_of(bg), FillRule::Winding, Transform::identity(), None);
        }
        // The clip of a box drawn into the border's corner: the plate inside the border's outer edge.
        let mut inner_clip: Option<Mask> = None;
        for e in lay["elements"].as_array().ok_or_else(|| anyhow!("{name}: elements"))? {
            let ink = |k: &str| self.colour(e[k].as_str().unwrap_or("black"));
            match e["t"].as_str().unwrap_or_default() {
                "border" => {
                    let (i, w) = (num(&e["inset"])?, num(&e["width"])?);
                    let mut pb = PathBuilder::new();
                    if let Some(o) = rr(i, i, pw - i, ph - i, radius - i) {
                        pb.push_path(&o);
                    }
                    if let Some(n) = rr(i + w, i + w, pw - i - w, ph - i - w, (radius - i - w).max(0.0)) {
                        pb.push_path(&n);
                    }
                    if let Some(p) = pb.finish() {
                        pix.fill_path(&p, &paint_of(ink("colour")?), FillRule::EvenOdd, Transform::identity(), None);
                    }
                    if inner_clip.is_none() {
                        let mut m = Mask::new(pix.width(), pix.height()).ok_or_else(|| anyhow!("mask"))?;
                        if let Some(o) = rr(i, i, pw - i, ph - i, radius - i) {
                            m.fill_path(&o, FillRule::Winding, true, Transform::identity());
                        }
                        inner_clip = Some(m);
                    }
                }
                "field" => {
                    // A coloured field inside a white rim (R99: blue inside a 0.5 in rim).
                    let i = num(&e["inset"])?;
                    if let Some(p) = rr(i, i, pw - i, ph - i, (radius - i).max(0.0)) {
                        pix.fill_path(&p, &paint_of(ink("colour")?), FillRule::Winding, Transform::identity(), None);
                    }
                }
                "box" => {
                    let r = Rect::from_ltrb(px(num(&e["x0"])?) as f32, py(num(&e["y0"])?) as f32, px(num(&e["x1"])?) as f32, py(num(&e["y1"])?) as f32).ok_or_else(|| anyhow!("{name}: box"))?;
                    pix.fill_path(&PathBuilder::from_rect(r), &paint_of(ink("colour")?), FillRule::Winding, Transform::identity(), inner_clip.as_ref());
                }
                "text" => {
                    let series = e["series"].as_str().unwrap_or("C");
                    let cap = num(&e["cap"])? * ky;
                    let sx = self.series_factor(series)? * kx / ky;
                    let width = e["w"].as_f64().map(|w| w * kx);
                    let max_w = e["maxW"].as_f64().map(|w| w * kx);
                    let cx = match e["cx"].as_f64() {
                        Some(c) => px(c),
                        None => px(pw / 2.0),
                    };
                    let text = e["s"].as_str().unwrap_or_default();
                    let weight = e["weight"].as_str().unwrap_or("bold");
                    let cx = match e["left"].as_f64() {
                        // Ink starting at `left`: centre = left + the fitted ink width / 2.
                        Some(l) => {
                            let natural = ink_width(self.face(weight), text, cap);
                            let mut w = width.unwrap_or(natural * sx);
                            if let Some(m) = max_w {
                                w = w.min(m);
                            }
                            px(l) + w / 2.0
                        }
                        None => cx,
                    };
                    self.text(pix, text, weight, cap, sx, width, cx, py(num(&e["top"])?), ink("colour")?, max_w);
                }
                "runs" => {
                    let sx_frame = kx / ky;
                    let width = e["w"].as_f64().map(|w| w * kx);
                    let max_w = e["maxW"].as_f64().map(|w| w * kx);
                    let cx = e["cx"].as_f64().map(px).unwrap_or(px(pw / 2.0));
                    self.runs(pix, &e["runs"], py(num(&e["top"])?), ky, sx_frame, width, max_w, cx, ink("colour")?)?;
                }
                "ring" => {
                    let (cx, cy, ro, ri) = (num(&e["cx"])?, num(&e["cy"])?, num(&e["ro"])?, num(&e["ri"])?);
                    let mut pb = PathBuilder::new();
                    for r in [ro, ri] {
                        if let Some(rect) = Rect::from_ltrb(px(cx - r) as f32, py(cy - r) as f32, px(cx + r) as f32, py(cy + r) as f32) {
                            pb.push_oval(rect);
                        }
                    }
                    if let Some(p) = pb.finish() {
                        pix.fill_path(&p, &paint_of(ink("colour")?), FillRule::EvenOdd, Transform::identity(), None);
                    }
                }
                "slash" => {
                    // A bar of `width` across the ring's inner circle at `angleDeg` (from +x, y down).
                    let (cx, cy, r, w, a) = (num(&e["cx"])?, num(&e["cy"])?, num(&e["r"])?, num(&e["width"])?, num(&e["angleDeg"])?.to_radians());
                    let (ux, uy, nx, ny) = (a.cos(), a.sin(), -a.sin(), a.cos());
                    let mut pb = PathBuilder::new();
                    let pts = [(-r, -w / 2.0), (r, -w / 2.0), (r, w / 2.0), (-r, w / 2.0)];
                    for (k, (s, t)) in pts.iter().enumerate() {
                        let (x, y) = (px(cx + ux * s + nx * t), py(cy + uy * s + ny * t));
                        if k == 0 { pb.move_to(x as f32, y as f32) } else { pb.line_to(x as f32, y as f32) }
                    }
                    pb.close();
                    let mut m = Mask::new(pix.width(), pix.height()).ok_or_else(|| anyhow!("mask"))?;
                    if let Some(rect) = Rect::from_ltrb(px(cx - r) as f32, py(cy - r) as f32, px(cx + r) as f32, py(cy + r) as f32) {
                        if let Some(o) = PathBuilder::from_oval(rect) {
                            m.fill_path(&o, FillRule::Winding, true, Transform::identity());
                        }
                    }
                    if let Some(p) = pb.finish() {
                        pix.fill_path(&p, &paint_of(ink("colour")?), FillRule::Winding, Transform::identity(), Some(&m));
                    }
                }
                "svgpart" => {
                    let svg = self.svg(e["file"].as_str().unwrap_or_default())?;
                    let reg = region(&e["region"]).ok_or_else(|| anyhow!("{name}: svgpart region"))?;
                    let (x, y, w, h) = (num(&e["x"])?, num(&e["y"])?, num(&e["w"])?, num(&e["h"])?);
                    let to = self.colour(e["colour"].as_str().unwrap_or("white"))?;
                    svg.draw(pix, [px(x) as f32, py(y) as f32, (w * kx) as f32, (h * ky) as f32], Some(reg), Transform::identity(), &|_| to);
                }
                "arrow" => {
                    // A double-headed horizontal arrow: shaft `shaft` thick, heads `headL` long and `headW` wide.
                    let (x0, x1, cy) = (num(&e["x0"])?, num(&e["x1"])?, num(&e["cy"])?);
                    let (st, hl, hw) = (num(&e["shaft"])?, num(&e["headL"])?, num(&e["headW"])?);
                    let mut pb = PathBuilder::new();
                    let pts = [(x0, cy), (x0 + hl, cy - hw / 2.0), (x0 + hl, cy - st / 2.0), (x1 - hl, cy - st / 2.0), (x1 - hl, cy - hw / 2.0), (x1, cy), (x1 - hl, cy + hw / 2.0), (x1 - hl, cy + st / 2.0), (x0 + hl, cy + st / 2.0), (x0 + hl, cy + hw / 2.0)];
                    for (k, (x, y)) in pts.iter().enumerate() {
                        if k == 0 { pb.move_to(px(*x) as f32, py(*y) as f32) } else { pb.line_to(px(*x) as f32, py(*y) as f32) }
                    }
                    pb.close();
                    if let Some(p) = pb.finish() {
                        pix.fill_path(&p, &paint_of(ink("colour")?), FillRule::Winding, Transform::identity(), None);
                    }
                }
                other => bail!("{name}: element type {other:?}"),
            }
        }
        Ok(())
    }

    /// A CA MUTCD D3-1 street name blade in `dest`, `frame` its real size
    /// (width, height in any unit): the green, border and corner radii are
    /// the FHWA D3-1a layout's (`streetName.svg`), as is the cap height over
    /// the blade height; the legend is mixed case (CA MUTCD 2D.45) in
    /// Overpass Bold at the D3-1a legend's measured series factor, shrunk to
    /// fit inside the border.
    pub fn street_blade(&self, pix: &mut Pixmap, legend: &str, dest: [f64; 4], frame: [f64; 2]) -> Result<()> {
        let b = self.blade_geometry()?;
        let (kx, ky) = (dest[2] / frame[0], dest[3] / frame[1]);
        let h = frame[1];
        let paint_of = |c: [u8; 4]| {
            let mut p = Paint::default();
            p.anti_alias = true;
            p.set_color_rgba8(c[0], c[1], c[2], c[3]);
            p
        };
        let rr = |i: f64, r: f64| round_rect((dest[0] + i * h * kx) as f32, (dest[1] + i * h * ky) as f32, (dest[0] + dest[2] - i * h * kx) as f32, (dest[1] + dest[3] - i * h * ky) as f32, (r * h * kx) as f32, (r * h * ky) as f32);
        if let Some(p) = rr(0.0, b.outer_r) {
            pix.fill_path(&p, &paint_of(b.border), FillRule::Winding, Transform::identity(), None);
        }
        if let Some(p) = rr(b.inset, b.inner_r) {
            pix.fill_path(&p, &paint_of(b.green), FillRule::Winding, Transform::identity(), None);
        }
        let text = mixed_case(legend);
        let cap = b.cap * h * ky;
        let max_w = dest[2] - 2.0 * b.margin * h * kx;
        // Capitals centred on the blade, as on the D3-1a layout.
        let top = dest[1] + (dest[3] - cap) / 2.0 + b.cap_offset * h * ky;
        self.text(pix, &text, "bold", cap, b.factor * kx / ky, None, dest[0] + dest[2] / 2.0, top, b.border, Some(max_w));
        Ok(())
    }

    /// The real size (width, height, m) of a D3-1 Street Name sign with this legend whose capitals
    /// are `cap` tall: the height from the D3-1a layout's cap height share, the width the legend's
    /// (mixed case, Overpass Bold at the layout's series factor) and the layout's margin each side.
    pub fn blade_size(&self, legend: &str, cap: f64) -> Result<[f64; 2]> {
        let b = self.blade_geometry()?;
        let h = cap / b.cap;
        let w = ink_width(&self.bold, &mixed_case(legend), cap) * b.factor + 2.0 * b.margin * h;
        Ok([w.max(2.0 * h), h])
    }

    fn blade_geometry(&self) -> Result<Blade> {
        let m = Manifest::get();
        let sn = &m.raw["streetName"];
        let svg = self.svg(sn["svg"].as_str().ok_or_else(|| anyhow!("streetName.svg"))?)?;
        let h = svg.view[3] as f64;
        let rects: Vec<&super::svg::Shape> = svg.shapes.iter().collect();
        // The blade (largest shape) and the field inside its border (largest green shape).
        let green_hex = colour(m.raw["palette"]["green"]["hex"].as_str().unwrap_or_default())?;
        let field = rects.iter().filter(|s| s.fill == green_hex).max_by(|a, b| (a.bounds.width() * a.bounds.height()).total_cmp(&(b.bounds.width() * b.bounds.height()))).ok_or_else(|| anyhow!("D3-1a: no green field"))?;
        let inset = field.bounds.top() as f64 / h;
        let lr = region(&sn["legendRegion"]).ok_or_else(|| anyhow!("streetName.legendRegion"))?;
        let legend: Vec<Rect> = svg.shapes.iter().filter(|s| s.fill != green_hex && s.bounds.left() >= lr[0] && s.bounds.right() <= lr[0] + lr[2] && s.bounds.top() >= lr[1] && s.bounds.bottom() <= lr[1] + lr[3]).map(|s| s.bounds).collect();
        if legend.is_empty() {
            bail!("D3-1a: no legend shapes in {lr:?}");
        }
        // Cap height: the tallest glyph without a descender: top of the highest to the common baseline.
        let mut bottoms: Vec<f32> = legend.iter().map(|b| b.bottom()).collect();
        bottoms.sort_by(f32::total_cmp);
        let baseline = bottoms[bottoms.len() / 2] as f64;
        let top = legend.iter().map(|b| b.top()).fold(f32::MAX, f32::min) as f64;
        let cap = (baseline - top) / h;
        // The series factor: the calibration glyphs' ink width over Overpass Bold's at the same cap height.
        let cr = region(&sn["legendCalibrationRegion"]).ok_or_else(|| anyhow!("streetName.legendCalibrationRegion"))?;
        let cal: Vec<Rect> = legend.iter().copied().filter(|b| b.left() >= cr[0] && b.right() <= cr[0] + cr[2] && b.top() >= cr[1] && b.bottom() <= cr[1] + cr[3]).collect();
        if cal.is_empty() {
            bail!("D3-1a: no glyph in the calibration region {cr:?}");
        }
        let left = cal.iter().map(|b| b.left()).fold(f32::MAX, f32::min) as f64;
        let right = cal.iter().map(|b| b.right()).fold(f32::MIN, f32::max) as f64;
        let sample = sn["legendText"].as_str().ok_or_else(|| anyhow!("streetName.legendText"))?;
        let factor = (right - left) / h / ink_width(&self.bold, sample, cap);
        let outer_r = sn["outerRadius"].as_f64().ok_or_else(|| anyhow!("streetName.outerRadius"))? / h;
        let inner_r = sn["innerRadius"].as_f64().ok_or_else(|| anyhow!("streetName.innerRadius"))? / h;
        // Where the layout puts the capitals relative to the blade's centre line.
        let cap_offset = ((top + baseline) / 2.0 - h / 2.0) / h;
        Ok(Blade { green: field.fill, border: self.colour("white")?, inset, outer_r, inner_r, cap, factor, margin: inset * 2.0, cap_offset })
    }
}

struct Blade {
    green: [u8; 4],
    border: [u8; 4],
    /// Shares of the blade height.
    inset: f64,
    outer_r: f64,
    inner_r: f64,
    cap: f64,
    margin: f64,
    cap_offset: f64,
    /// The D3-1a legend's width over Overpass Bold's at its cap height.
    factor: f64,
}

/// Street names on a blade are mixed case (CA MUTCD 2D.45): initial capitals,
/// ordinals and suffix abbreviations in lower case ("5TH ST" -> "5th St").
pub fn mixed_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    let mut word_len = 0usize;
    for ch in s.chars() {
        if ch.is_alphabetic() {
            if start {
                out.extend(ch.to_uppercase());
            } else {
                out.extend(ch.to_lowercase());
            }
            start = false;
            word_len += 1;
        } else {
            out.push(ch);
            // A letter after a digit continues the word (5th); after an apostrophe
            // it starts one only behind a one-letter prefix (O'Farrell, not Mary'S).
            start = match ch {
                ' ' | '-' | '/' | '.' => true,
                '\'' | '\u{2019}' => word_len == 1,
                _ => false,
            };
            if ch.is_ascii_digit() {
                word_len += 1;
            } else {
                word_len = 0;
            }
        }
    }
    out
}

fn num(v: &Value) -> Result<f64> {
    v.as_f64().ok_or_else(|| anyhow!("number expected, got {v}"))
}

fn f32x4(d: [f64; 4]) -> [f32; 4] {
    [d[0] as f32, d[1] as f32, d[2] as f32, d[3] as f32]
}

fn region(v: &Value) -> Option<[f32; 4]> {
    Some([v[0].as_f64()? as f32, v[1].as_f64()? as f32, v[2].as_f64()? as f32, v[3].as_f64()? as f32])
}

fn cal_fallback() -> Option<Vec<(String, String)>> {
    Manifest::get().raw["seriesFallback"].as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
}

/// A layout of the manifest.
pub fn layout(name: &str) -> Result<&'static Value> {
    let v = &Manifest::get().raw["layouts"][name];
    if v.is_null() {
        bail!("no layout {name:?} in the manifest");
    }
    Ok(v)
}

pub(super) fn round_rect(x0: f32, y0: f32, x1: f32, y1: f32, rx: f32, ry: f32) -> Option<tiny_skia::Path> {
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let rx = rx.clamp(0.0, w / 2.0);
    let ry = ry.clamp(0.0, h / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_ltrb(x0, y0, x1, y1)?));
    }
    let k = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + rx, y0);
    pb.line_to(x1 - rx, y0);
    pb.cubic_to(x1 - rx + k * rx, y0, x1, y0 + ry - k * ry, x1, y0 + ry);
    pb.line_to(x1, y1 - ry);
    pb.cubic_to(x1, y1 - ry + k * ry, x1 - rx + k * rx, y1, x1 - rx, y1);
    pb.line_to(x0 + rx, y1);
    pb.cubic_to(x0 + rx - k * rx, y1, x0, y1 - ry + k * ry, x0, y1 - ry);
    pb.line_to(x0, y0 + ry);
    pb.cubic_to(x0, y0 + ry - k * ry, x0 + rx - k * rx, y0, x0 + rx, y0);
    pb.close();
    pb.finish()
}

/// Cap height (the 'H' outline) as a share of the em.
pub fn cap_height_em(font: &FontVec) -> f64 {
    let upem = font.units_per_em().unwrap_or(1000.0) as f64;
    let g = font.glyph_id('H');
    // ab_glyph bounds are y down (min.y the top): the height is the absolute difference.
    font.outline(g).map(|o| (o.bounds.max.y - o.bounds.min.y).abs() as f64 / upem).unwrap_or(0.7)
}

/// Advance width (with kerning) of `text` at cap height `cap`.
pub fn text_width(font: &FontVec, text: &str, cap: f64) -> f64 {
    let em = cap / cap_height_em(font);
    let s = em / font.units_per_em().unwrap_or(1000.0) as f64;
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

/// Ink extent (outline bounds, left to right) of `text` at cap height `cap`.
pub fn ink_width(font: &FontVec, text: &str, cap: f64) -> f64 {
    glyph_path(font, text, cap / cap_height_em(font)).map(|p| p.bounds().width() as f64).unwrap_or(0.0)
}

/// The outlines of `text` at `em` pixels, pen from x 0, baseline at y 0 (y down).
pub fn glyph_path(font: &FontVec, text: &str, em: f64) -> Option<tiny_skia::Path> {
    let s = em / font.units_per_em().unwrap_or(1000.0) as f64;
    let mut pb = PathBuilder::new();
    let mut pen = 0.0;
    let mut prev = None;
    for ch in text.chars() {
        let g = font.glyph_id(ch);
        if let Some(p) = prev {
            pen += font.kern_unscaled(p, g) as f64 * s;
        }
        if let Some(outline) = font.outline(g) {
            let map = |p: ab_glyph::Point| ((pen + p.x as f64 * s) as f32, (-(p.y as f64) * s) as f32);
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
            }
            if last.is_some() {
                pb.close();
            }
        }
        pen += font.h_advance_unscaled(g) as f64 * s;
        prev = Some(g);
    }
    pb.finish()
}

/// The inventory's most common plate size (inches) per face index
/// (`SF_STREET_SIGNS.plates`: faceIndex, widthIn, heightIn, yaw).
pub fn dominant_plate_sizes(plates: &[i32], faces: usize) -> Vec<Option<[f64; 2]>> {
    let mut counts: Vec<HashMap<(i32, i32), usize>> = vec![HashMap::new(); faces];
    for p in plates.chunks_exact(4) {
        if let Some(c) = counts.get_mut(p[0] as usize) {
            if p[1] > 0 && p[2] > 0 {
                *c.entry((p[1], p[2])).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .map(|c| c.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|((w, h), _)| [w as f64, h as f64]))
        .collect()
}

/// Where `art` (size in inches) lands in a cell rect `rect` (pixels) whose
/// plate is `frame` (any unit): fitted into the frame, then the frame mapped
/// onto the rect.
pub fn fit(art: [f64; 2], frame: [f64; 2], rect: [f64; 4]) -> [f64; 4] {
    let s = (frame[0] / art[0]).min(frame[1] / art[1]);
    let (aw, ah) = (art[0] * s, art[1] * s);
    let (kx, ky) = (rect[2] / frame[0], rect[3] / frame[1]);
    [rect[0] + (frame[0] - aw) / 2.0 * kx, rect[1] + (frame[1] - ah) / 2.0 * ky, aw * kx, ah * ky]
}

/// One art spec rendered alone (`size_px` on its long side), for previews and tests.
pub fn render_spec_png(art: &SignArt, spec: &Value, size_px: u32) -> Result<Vec<u8>> {
    let s = art.spec_size(spec)?;
    let k = size_px as f64 / s[0].max(s[1]);
    let (w, h) = ((s[0] * k).round() as u32, (s[1] * k).round() as u32);
    let mut pix = Pixmap::new(w.max(1), h.max(1)).ok_or_else(|| anyhow!("pixmap"))?;
    art.draw_spec(&mut pix, spec, [0.0, 0.0, w as f64, h as f64])?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for p in pix.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    encode_png(w, h, &rgba, true)
}
