//! a small SVG reader for the FHWA Standard Highway Signs layouts
//! (Adobe Illustrator exports: `viewBox`, `<style>` classes with `fill` and
//! `fill-rule`, `<g>`, `<path>`, `<polygon>`, `<rect>` with `rx`/`ry` and
//! `transform`). Shapes are flattened into tiny-skia paths in viewBox units;
//! Illustrator's private `i:*` and `<metadata>` elements are skipped. Fills
//! only: the SHS layouts carry no strokes. Anything else this reader does not
//! know is an error, so a layout it would draw wrongly is never drawn.

use anyhow::{Result, anyhow, bail};
use std::collections::HashMap;
use tiny_skia::{FillRule, Path, PathBuilder, Rect, Transform};

/// One filled shape, in viewBox units.
#[derive(Clone, Debug)]
pub struct Shape {
    pub path: Path,
    /// sRGB, straight alpha.
    pub fill: [u8; 4],
    pub rule: FillRule,
    pub bounds: Rect,
}

/// A parsed layout.
#[derive(Clone, Debug)]
pub struct Svg {
    /// `[min_x, min_y, width, height]`.
    pub view: [f32; 4],
    pub shapes: Vec<Shape>,
}

#[derive(Clone, Default)]
struct Style {
    fill: Option<Option<[u8; 4]>>,
    rule: Option<FillRule>,
}

impl Style {
    fn over(&self, top: &Style) -> Style {
        Style { fill: top.fill.or(self.fill), rule: top.rule.or(self.rule) }
    }
}

fn parse_colour(v: &str) -> Result<Option<[u8; 4]>> {
    let v = v.trim();
    if v == "none" {
        return Ok(None);
    }
    let h = v.strip_prefix('#').ok_or_else(|| anyhow!("colour {v:?}"))?;
    let n = u32::from_str_radix(h, 16).map_err(|_| anyhow!("colour {v:?}"))?;
    Ok(Some(match h.len() {
        3 => [((n >> 8) & 15) as u8 * 17, ((n >> 4) & 15) as u8 * 17, (n & 15) as u8 * 17, 255],
        6 => [(n >> 16) as u8, (n >> 8) as u8, n as u8, 255],
        _ => bail!("colour {v:?}"),
    }))
}

fn apply_decl(s: &mut Style, key: &str, val: &str) -> Result<()> {
    match key.trim() {
        "fill" => s.fill = Some(parse_colour(val)?),
        "fill-rule" | "clip-rule" => {
            if key.trim() == "fill-rule" {
                s.rule = Some(if val.trim() == "evenodd" { FillRule::EvenOdd } else { FillRule::Winding });
            }
        }
        "" => {}
        // The SHS exports carry no strokes; refuse anything that would change how a fill draws.
        "stroke" if val.trim() == "none" => {}
        "isolation" | "mix-blend-mode" if val.trim() == "isolate" || val.trim() == "normal" => {}
        k => bail!("unsupported style property {k}: {val}"),
    }
    Ok(())
}

fn parse_decls(body: &str) -> Result<Style> {
    let mut s = Style::default();
    for d in body.split(';') {
        if let Some((k, v)) = d.split_once(':') {
            apply_decl(&mut s, k, v)?;
        }
    }
    Ok(s)
}

/// `.a, .b { decls }` rules of a `<style>` block.
fn parse_css(css: &str, classes: &mut HashMap<String, Style>) -> Result<()> {
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}').ok_or_else(|| anyhow!("css: unclosed rule"))? + open;
        let decl = parse_decls(&rest[open + 1..close])?;
        for sel in rest[..open].split(',') {
            let sel = sel.trim();
            let name = sel.strip_prefix('.').ok_or_else(|| anyhow!("css selector {sel:?}"))?;
            let e = classes.entry(name.to_string()).or_default();
            *e = e.over(&decl);
        }
        rest = &rest[close + 1..];
    }
    Ok(())
}

fn attrs(tag: &str) -> Result<Vec<(String, String)>> {
    let mut out = vec![];
    let b = tag.as_bytes();
    let mut i = 0;
    // skip the name
    while i < b.len() && !b[i].is_ascii_whitespace() {
        i += 1;
    }
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let k0 = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = tag[k0..i].to_string();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            out.push((key, String::new()));
            continue;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let q = *b.get(i).ok_or_else(|| anyhow!("attribute {key} without value"))?;
        if q != b'"' && q != b'\'' {
            bail!("attribute {key}: unquoted value");
        }
        let v0 = i + 1;
        let v1 = tag[v0..].find(q as char).ok_or_else(|| anyhow!("attribute {key}: unclosed"))? + v0;
        out.push((key, tag[v0..v1].to_string()));
        i = v1 + 1;
    }
    Ok(out)
}

fn numbers(s: &str) -> Result<Vec<f32>> {
    let mut lx = Lexer { b: s.as_bytes(), i: 0 };
    let mut v = vec![];
    while let Some(n) = lx.number()? {
        v.push(n);
    }
    Ok(v)
}

fn parse_transform(s: &str) -> Result<Transform> {
    let mut t = Transform::identity();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let open = rest.find('(').ok_or_else(|| anyhow!("transform {s:?}"))?;
        let close = rest.find(')').ok_or_else(|| anyhow!("transform {s:?}"))?;
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let a = numbers(&rest[open + 1..close])?;
        let m = match (name, a.len()) {
            ("translate", 1) => Transform::from_translate(a[0], 0.0),
            ("translate", 2) => Transform::from_translate(a[0], a[1]),
            ("scale", 1) => Transform::from_scale(a[0], a[0]),
            ("scale", 2) => Transform::from_scale(a[0], a[1]),
            ("rotate", 1) => Transform::from_rotate(a[0]),
            ("rotate", 3) => Transform::from_rotate_at(a[0], a[1], a[2]),
            ("matrix", 6) => Transform::from_row(a[0], a[1], a[2], a[3], a[4], a[5]),
            _ => bail!("transform {name}{a:?}"),
        };
        t = t.pre_concat(m);
        rest = rest[close + 1..].trim();
    }
    Ok(t)
}

struct Lexer<'a> {
    b: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn skip(&mut self) {
        while self.i < self.b.len() && (self.b[self.i].is_ascii_whitespace() || self.b[self.i] == b',') {
            self.i += 1;
        }
    }
    fn peek_cmd(&mut self) -> Option<u8> {
        self.skip();
        self.b.get(self.i).copied().filter(|c| c.is_ascii_alphabetic() && *c != b'e' && *c != b'E')
    }
    /// SVG number grammar: `-.5.5` is two numbers, `1e-3` one.
    fn number(&mut self) -> Result<Option<f32>> {
        self.skip();
        let s = self.i;
        let b = self.b;
        let mut i = self.i;
        if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
            i += 1;
        }
        let mut digits = false;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits = true;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
                digits = true;
            }
        }
        if !digits {
            return Ok(None);
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_digit() {
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        self.i = i;
        let txt = std::str::from_utf8(&b[s..i])?;
        Ok(Some(txt.parse::<f32>().map_err(|_| anyhow!("number {txt:?}"))?))
    }
    fn need(&mut self) -> Result<f32> {
        self.number()?.ok_or_else(|| anyhow!("path: number expected at byte {}", self.i))
    }
    fn flag(&mut self) -> Result<bool> {
        self.skip();
        match self.b.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => bail!("path: arc flag expected"),
        }
    }
}

/// The endpoint-parameterised elliptical arc as cubics (SVG 1.1 F.6).
#[allow(clippy::too_many_arguments)]
fn arc_to(pb: &mut PathBuilder, x1: f32, y1: f32, mut rx: f32, mut ry: f32, phi_deg: f32, large: bool, sweep: bool, x2: f32, y2: f32) {
    if (x1 - x2).abs() < 1e-6 && (y1 - y2).abs() < 1e-6 {
        return;
    }
    rx = rx.abs();
    ry = ry.abs();
    if rx < 1e-6 || ry < 1e-6 {
        pb.line_to(x2, y2);
        return;
    }
    let (x1, y1, x2, y2) = (x1 as f64, y1 as f64, x2 as f64, y2 as f64);
    let (mut rx, mut ry) = (rx as f64, ry as f64);
    let phi = (phi_deg as f64).to_radians();
    let (c, s) = (phi.cos(), phi.sin());
    let dx = (x1 - x2) / 2.0;
    let dy = (y1 - y2) / 2.0;
    let x1p = c * dx + s * dy;
    let y1p = -s * dx + c * dy;
    let lam = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut k = (num / den).max(0.0).sqrt();
    if large == sweep {
        k = -k;
    }
    let cxp = k * rx * y1p / ry;
    let cyp = -k * ry * x1p / rx;
    let cx = c * cxp - s * cyp + (x1 + x2) / 2.0;
    let cy = s * cxp + c * cyp + (y1 + y2) / 2.0;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let d = (ux * vx + uy * vy) / ((ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt());
        let a = d.clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 { -a } else { a }
    };
    let th1 = ang(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dth = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if !sweep && dth > 0.0 {
        dth -= std::f64::consts::TAU;
    } else if sweep && dth < 0.0 {
        dth += std::f64::consts::TAU;
    }
    let n = (dth.abs() / (std::f64::consts::PI / 2.0)).ceil().max(1.0) as usize;
    let step = dth / n as f64;
    let kk = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |t: f64| (cx + rx * c * t.cos() - ry * s * t.sin(), cy + rx * s * t.cos() + ry * c * t.sin());
    let dpt = |t: f64| (-rx * c * t.sin() - ry * s * t.cos(), -rx * s * t.sin() + ry * c * t.cos());
    let mut t = th1;
    for _ in 0..n {
        let t2 = t + step;
        let (p0, d0, p3, d3) = (pt(t), dpt(t), pt(t2), dpt(t2));
        pb.cubic_to((p0.0 + kk * d0.0) as f32, (p0.1 + kk * d0.1) as f32, (p3.0 - kk * d3.0) as f32, (p3.1 - kk * d3.1) as f32, p3.0 as f32, p3.1 as f32);
        t = t2;
    }
}

/// SVG path data to a tiny-skia path (absolute and relative M L H V C S Q T A Z).
pub fn parse_path(d: &str) -> Result<Option<Path>> {
    let mut lx = Lexer { b: d.as_bytes(), i: 0 };
    let mut pb = PathBuilder::new();
    let (mut cx, mut cy, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
    let mut last_c2: Option<(f32, f32)> = None; // reflected control for S
    let mut last_q: Option<(f32, f32)> = None; // for T
    let mut cmd = 0u8;
    loop {
        if let Some(c) = lx.peek_cmd() {
            cmd = c;
            lx.i += 1;
        } else if lx.i >= lx.b.len() {
            break;
        } else if cmd == 0 {
            bail!("path: data before a command");
        } else if cmd == b'M' {
            cmd = b'L';
        } else if cmd == b'm' {
            cmd = b'l';
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (lx.need()? + ox, lx.need()? + oy);
                pb.move_to(x, y);
                (cx, cy, sx, sy) = (x, y, x, y);
                last_c2 = None;
                last_q = None;
            }
            b'L' => {
                let (x, y) = (lx.need()? + ox, lx.need()? + oy);
                pb.line_to(x, y);
                (cx, cy) = (x, y);
                last_c2 = None;
                last_q = None;
            }
            b'H' => {
                let x = lx.need()? + ox;
                pb.line_to(x, cy);
                cx = x;
                last_c2 = None;
                last_q = None;
            }
            b'V' => {
                let y = lx.need()? + oy;
                pb.line_to(cx, y);
                cy = y;
                last_c2 = None;
                last_q = None;
            }
            b'C' => {
                let (x1, y1, x2, y2, x, y) = (lx.need()? + ox, lx.need()? + oy, lx.need()? + ox, lx.need()? + oy, lx.need()? + ox, lx.need()? + oy);
                pb.cubic_to(x1, y1, x2, y2, x, y);
                last_c2 = Some((x2, y2));
                last_q = None;
                (cx, cy) = (x, y);
            }
            b'S' => {
                let (x2, y2, x, y) = (lx.need()? + ox, lx.need()? + oy, lx.need()? + ox, lx.need()? + oy);
                let (x1, y1) = last_c2.map(|(px, py)| (2.0 * cx - px, 2.0 * cy - py)).unwrap_or((cx, cy));
                pb.cubic_to(x1, y1, x2, y2, x, y);
                last_c2 = Some((x2, y2));
                last_q = None;
                (cx, cy) = (x, y);
            }
            b'Q' => {
                let (x1, y1, x, y) = (lx.need()? + ox, lx.need()? + oy, lx.need()? + ox, lx.need()? + oy);
                pb.quad_to(x1, y1, x, y);
                last_q = Some((x1, y1));
                last_c2 = None;
                (cx, cy) = (x, y);
            }
            b'T' => {
                let (x, y) = (lx.need()? + ox, lx.need()? + oy);
                let (x1, y1) = last_q.map(|(px, py)| (2.0 * cx - px, 2.0 * cy - py)).unwrap_or((cx, cy));
                pb.quad_to(x1, y1, x, y);
                last_q = Some((x1, y1));
                last_c2 = None;
                (cx, cy) = (x, y);
            }
            b'A' => {
                let (rx, ry, rot) = (lx.need()?, lx.need()?, lx.need()?);
                let (large, sweep) = (lx.flag()?, lx.flag()?);
                let (x, y) = (lx.need()? + ox, lx.need()? + oy);
                arc_to(&mut pb, cx, cy, rx, ry, rot, large, sweep, x, y);
                (cx, cy) = (x, y);
                last_c2 = None;
                last_q = None;
            }
            b'Z' => {
                pb.close();
                (cx, cy) = (sx, sy);
                last_c2 = None;
                last_q = None;
                // Z takes no numbers: a number after it would be an implicit command, which SVG forbids.
                if lx.peek_cmd().is_none() && lx.i < lx.b.len() {
                    lx.skip();
                    if lx.i < lx.b.len() {
                        bail!("path: data after Z");
                    }
                }
            }
            c => bail!("path: command {}", c as char),
        }
    }
    Ok(pb.finish())
}

fn num_attr(a: &[(String, String)], k: &str) -> Result<f32> {
    match a.iter().find(|(n, _)| n == k) {
        Some((_, v)) => numbers(v)?.first().copied().ok_or_else(|| anyhow!("{k}={v:?}")),
        None => Ok(0.0),
    }
}

fn rect_path(x: f32, y: f32, w: f32, h: f32, mut rx: f32, mut ry: f32) -> Option<Path> {
    if rx <= 0.0 && ry > 0.0 {
        rx = ry;
    }
    if ry <= 0.0 && rx > 0.0 {
        ry = rx;
    }
    rx = rx.min(w / 2.0);
    ry = ry.min(h / 2.0);
    if rx <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?));
    }
    let k = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + rx, y);
    pb.line_to(x + w - rx, y);
    pb.cubic_to(x + w - rx + k * rx, y, x + w, y + ry - k * ry, x + w, y + ry);
    pb.line_to(x + w, y + h - ry);
    pb.cubic_to(x + w, y + h - ry + k * ry, x + w - rx + k * rx, y + h, x + w - rx, y + h);
    pb.line_to(x + rx, y + h);
    pb.cubic_to(x + rx - k * rx, y + h, x, y + h - ry + k * ry, x, y + h - ry);
    pb.line_to(x, y + ry);
    pb.cubic_to(x, y + ry - k * ry, x + rx - k * rx, y, x + rx, y);
    pb.close();
    pb.finish()
}

impl Svg {
    pub fn parse(text: &str) -> Result<Svg> {
        let mut classes: HashMap<String, Style> = HashMap::new();
        // Pass 1: the <style> blocks (Illustrator puts them in <defs> before use, but be safe).
        let mut rest = text;
        while let Some(i) = rest.find("<style") {
            let open_end = rest[i..].find('>').ok_or_else(|| anyhow!("style tag"))? + i;
            let close = rest[open_end..].find("</style>").ok_or_else(|| anyhow!("style close"))? + open_end;
            parse_css(&rest[open_end + 1..close], &mut classes)?;
            rest = &rest[close..];
        }
        let mut view = None;
        let mut shapes = vec![];
        // (transform, style) per open element.
        let root_style = Style { fill: Some(Some([0, 0, 0, 255])), rule: Some(FillRule::Winding) };
        let mut stack: Vec<(String, Transform, Style)> = vec![];
        let mut skip_depth = 0usize; // inside <metadata>, <i:*>, <style>, <title>
        let mut i = 0;
        let t = text;
        while let Some(lt) = t[i..].find('<') {
            let s = i + lt;
            if t[s..].starts_with("<!--") {
                i = t[s..].find("-->").ok_or_else(|| anyhow!("comment"))? + s + 3;
                continue;
            }
            if t[s..].starts_with("<![CDATA[") {
                i = t[s..].find("]]>").ok_or_else(|| anyhow!("cdata"))? + s + 3;
                continue;
            }
            let e = t[s..].find('>').ok_or_else(|| anyhow!("unclosed tag"))? + s;
            let tag = &t[s + 1..e];
            i = e + 1;
            if tag.starts_with('?') || tag.starts_with('!') {
                continue;
            }
            if let Some(name) = tag.strip_prefix('/') {
                let name = name.trim();
                if skip_depth > 0 {
                    skip_depth -= 1;
                    continue;
                }
                match stack.pop() {
                    Some((open, _, _)) if open == name => {}
                    other => bail!("mismatched </{name}> (open: {:?})", other.map(|o| o.0)),
                }
                continue;
            }
            let self_close = tag.ends_with('/');
            let name: String = tag.split(|c: char| c.is_ascii_whitespace() || c == '/').next().unwrap_or("").to_string();
            if skip_depth > 0 {
                if !self_close {
                    skip_depth += 1;
                }
                continue;
            }
            if name.contains(':') || matches!(name.as_str(), "metadata" | "style" | "title" | "desc") {
                if !self_close {
                    skip_depth = 1;
                }
                continue;
            }
            let a = attrs(tag)?;
            let get = |k: &str| a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
            let (parent_t, parent_s) = stack.last().map(|(_, t, s)| (*t, s.clone())).unwrap_or((Transform::identity(), root_style.clone()));
            let mut style = parent_s;
            if let Some(cl) = get("class") {
                for c in cl.split_whitespace() {
                    let cs = classes.get(c).ok_or_else(|| anyhow!("class {c} has no style"))?;
                    style = style.over(cs);
                }
            }
            if let Some(f) = get("fill") {
                style.fill = Some(parse_colour(f)?);
            }
            if let Some(r) = get("fill-rule") {
                style.rule = Some(if r == "evenodd" { FillRule::EvenOdd } else { FillRule::Winding });
            }
            if let Some(st) = get("style") {
                style = style.over(&parse_decls(st)?);
            }
            for (k, _) in &a {
                if matches!(k.as_str(), "stroke" | "stroke-width" | "opacity" | "fill-opacity" | "clip-path" | "mask" | "filter") {
                    bail!("<{name}> uses {k}, which this reader does not draw");
                }
            }
            let tr = match get("transform") {
                Some(v) => parent_t.pre_concat(parse_transform(v)?),
                None => parent_t,
            };
            let geom: Option<Path> = match name.as_str() {
                "svg" => {
                    let v = numbers(get("viewBox").ok_or_else(|| anyhow!("svg without viewBox"))?)?;
                    if v.len() != 4 {
                        bail!("viewBox {v:?}");
                    }
                    view = Some([v[0], v[1], v[2], v[3]]);
                    None
                }
                "g" | "defs" => None,
                "path" => parse_path(get("d").unwrap_or(""))?,
                "polygon" | "polyline" => {
                    let p = numbers(get("points").unwrap_or(""))?;
                    let mut pb = PathBuilder::new();
                    for (k, xy) in p.chunks_exact(2).enumerate() {
                        if k == 0 { pb.move_to(xy[0], xy[1]) } else { pb.line_to(xy[0], xy[1]) }
                    }
                    if name == "polygon" {
                        pb.close();
                    }
                    pb.finish()
                }
                "rect" => rect_path(num_attr(&a, "x")?, num_attr(&a, "y")?, num_attr(&a, "width")?, num_attr(&a, "height")?, num_attr(&a, "rx")?, num_attr(&a, "ry")?),
                "circle" | "ellipse" => {
                    let (cx, cy) = (num_attr(&a, "cx")?, num_attr(&a, "cy")?);
                    let (rx, ry) = if name == "circle" { let r = num_attr(&a, "r")?; (r, r) } else { (num_attr(&a, "rx")?, num_attr(&a, "ry")?) };
                    Rect::from_xywh(cx - rx, cy - ry, 2.0 * rx, 2.0 * ry).and_then(PathBuilder::from_oval)
                }
                other => bail!("unsupported element <{other}>"),
            };
            if let Some(p) = geom {
                if let Some(Some(fill)) = style.fill {
                    if let Some(p) = p.transform(tr) {
                        let bounds = p.bounds();
                        shapes.push(Shape { path: p, fill, rule: style.rule.unwrap_or(FillRule::Winding), bounds });
                    }
                }
            }
            if !self_close {
                stack.push((name, tr, style));
            }
        }
        if !stack.is_empty() {
            bail!("unclosed <{}>", stack.last().map(|s| s.0.as_str()).unwrap_or(""));
        }
        Ok(Svg { view: view.ok_or_else(|| anyhow!("no <svg> element"))?, shapes })
    }

    /// Draws the layout so that the viewBox (or `region` of it, viewBox units)
    /// fills `dest` (pixels: x, y, w, h; non-uniform scale allowed). With a
    /// region, only shapes whose bounds lie inside it are drawn.
    pub fn draw(&self, pix: &mut tiny_skia::Pixmap, dest: [f32; 4], region: Option<[f32; 4]>, base: Transform, recolour: &dyn Fn([u8; 4]) -> [u8; 4]) {
        let r = region.unwrap_or(self.view);
        let t = base.pre_concat(Transform::from_row(dest[2] / r[2], 0.0, 0.0, dest[3] / r[3], dest[0] - r[0] * dest[2] / r[2], dest[1] - r[1] * dest[3] / r[3]));
        for s in &self.shapes {
            if let Some(g) = region {
                let b = s.bounds;
                let tol = 1e-3 * g[2].max(g[3]);
                if b.left() < g[0] - tol || b.top() < g[1] - tol || b.right() > g[0] + g[2] + tol || b.bottom() > g[1] + g[3] + tol {
                    continue;
                }
            }
            let c = recolour(s.fill);
            let mut paint = tiny_skia::Paint::default();
            paint.anti_alias = true;
            paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
            pix.fill_path(&s.path, &paint, s.rule, t, None);
        }
    }

    /// Size in the layout's units.
    pub fn size(&self) -> [f32; 2] {
        [self.view[2], self.view[3]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_follow_the_svg_grammar() {
        assert_eq!(numbers("632.6-.1.5,2e1 -3").unwrap(), vec![632.6, -0.1, 0.5, 20.0, -3.0]);
    }

    #[test]
    fn parses_an_illustrator_layout() {
        let s = r##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50">
          <!-- c --><defs><style>.st0 { fill: #231f20; } .st1, .st2 { fill: #fff; fill-rule: evenodd; }</style></defs>
          <metadata><i:pgf id="x"><![CDATA[ junk <path/> ]]></i:pgf></metadata>
          <g><polygon class="st0" points="0 0 100 0 100 50 0 50"/>
          <rect class="st1" x="10" y="10" width="20" height="10" rx="2" ry="2" transform="translate(5 0)"/>
          <path class="st2" d="M50,10h10v10h-10Z m20,0a5 5 0 1 0 10 0a5 5 0 1 0 -10 0z"/></g></svg>"##;
        let svg = Svg::parse(s).unwrap();
        assert_eq!(svg.view, [0.0, 0.0, 100.0, 50.0]);
        assert_eq!(svg.shapes.len(), 3);
        assert_eq!(svg.shapes[0].fill, [0x23, 0x1f, 0x20, 255]);
        assert_eq!(svg.shapes[1].fill, [255, 255, 255, 255]);
        assert!((svg.shapes[1].bounds.left() - 15.0).abs() < 1e-3, "rect translated");
        assert_eq!(svg.shapes[2].rule, FillRule::EvenOdd);
        assert!((svg.shapes[2].bounds.right() - 80.0).abs() < 0.05, "arc reaches x 80: {:?}", svg.shapes[2].bounds);
    }

    #[test]
    fn refuses_what_it_cannot_draw() {
        assert!(Svg::parse(r#"<svg viewBox="0 0 1 1"><path d="M0 0L1 1" stroke="red"/></svg>"#).is_err());
        assert!(Svg::parse(r#"<svg viewBox="0 0 1 1"><image href="x.png"/></svg>"#).is_err());
    }
}
