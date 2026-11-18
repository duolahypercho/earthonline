//! The 2D layer: a draw list of textured, rounded, gradient-filled quads and
//! fans, built by a `Painter` and drawn by `UiGpu` in one render pass with
//! one pipeline (one draw call per image/scissor change; a HUD frame is one
//! or two).

use crate::text::{ATLAS_SIZE, TextStyle, TextSystem};
use crate::theme::Color;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

pub const MODE_FILL: u32 = 0;
pub const MODE_GLYPH: u32 = 1;
pub const MODE_IMAGE: u32 = 2;
pub const MODE_RADAR: u32 = 3;
pub const MODE_ELLIPSE: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub local: [f32; 2],
    pub half: [f32; 2],
    pub color: u32,
    pub radius: f32,
    pub q: f32,
    pub mode: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] < self.x + self.w && p[1] >= self.y && p[1] < self.y + self.h
    }
    pub fn inset(&self, l: f32, t: f32, r: f32, b: f32) -> Rect {
        Rect::new(self.x + l, self.y + t, (self.w - l - r).max(0.0), (self.h - t - b).max(0.0))
    }
    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

/// Images the list can reference; 0 is "none" (a 1x1 white texel).
pub type ImageId = u16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cmd {
    pub image: ImageId,
    pub scissor: Option<[u32; 4]>,
    pub start: u32,
    pub count: u32,
}

/// A gradient direction (the CSS `<angle>` cases the stylesheets use).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dir {
    /// `0deg`, `to top`
    ToTop,
    /// `90deg`, `to right`
    ToRight,
    /// `180deg`, `to bottom`
    ToBottom,
    /// `270deg`, `to left`
    ToLeft,
}

#[derive(Default)]
pub struct DrawList {
    pub verts: Vec<Vertex>,
    pub idx: Vec<u32>,
    pub cmds: Vec<Cmd>,
    image: ImageId,
    scissor: Option<[u32; 4]>,
}

impl DrawList {
    pub fn clear(&mut self) {
        self.verts.clear();
        self.idx.clear();
        self.cmds.clear();
        self.image = 0;
        self.scissor = None;
    }
    fn begin(&mut self, image: ImageId) {
        let start = self.idx.len() as u32;
        let same = matches!(self.cmds.last(), Some(c) if (c.image == image || image == 0 || c.image == 0) && c.scissor == self.scissor);
        if same {
            let c = self.cmds.last_mut().unwrap();
            if image != 0 {
                c.image = image;
            }
            return;
        }
        self.cmds.push(Cmd { image, scissor: self.scissor, start, count: 0 });
        let _ = self.image;
        self.image = image;
    }
    fn end(&mut self) {
        let n = self.idx.len() as u32;
        if let Some(c) = self.cmds.last_mut() {
            c.count = n - c.start;
        }
    }
    pub fn set_scissor(&mut self, s: Option<[u32; 4]>) {
        self.scissor = s;
    }
    fn quad(&mut self, image: ImageId, v: [Vertex; 4]) {
        self.begin(image);
        let b = self.verts.len() as u32;
        self.verts.extend_from_slice(&v);
        self.idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        self.end();
    }
    fn tris(&mut self, image: ImageId, v: &[Vertex], idx: impl Iterator<Item = u32>) {
        self.begin(image);
        let b = self.verts.len() as u32;
        self.verts.extend_from_slice(v);
        self.idx.extend(idx.map(|i| b + i));
        self.end();
    }
}

/// Reused buffers so a rebuild allocates nothing once warm.
#[derive(Default)]
pub struct Scratch {
    verts: Vec<Vertex>,
    tri: Vec<u32>,
    pts: Vec<(f32, Color)>,
    ring: Vec<[f32; 2]>,
    ear: Option<earcut::Earcut<f32>>,
}

/// Builds a draw list: CSS-like primitives at an opacity and offset.
pub struct Painter<'a> {
    pub list: &'a mut DrawList,
    pub text: &'a mut TextSystem,
    pub opacity: f32,
    pub dx: f32,
    pub dy: f32,
    scratch: &'a mut Scratch,
}

fn v(pos: [f32; 2], color: u32) -> Vertex {
    Vertex { pos, color, q: 1.0, ..Default::default() }
}

impl<'a> Painter<'a> {
    pub fn new(list: &'a mut DrawList, text: &'a mut TextSystem, scratch: &'a mut Scratch) -> Self {
        Self { list, text, opacity: 1.0, dx: 0.0, dy: 0.0, scratch }
    }

    fn c(&self, c: Color) -> u32 {
        c.premul(self.opacity)
    }

    /// A rounded rectangle (`border-radius`), antialiased by its SDF.
    pub fn rrect(&mut self, r: Rect, radius: f32, color: Color) {
        if color.a * self.opacity <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let col = self.c(color);
        let (x, y) = (r.x + self.dx, r.y + self.dy);
        let half = [r.w * 0.5, r.h * 0.5];
        let (cx, cy) = (x + half[0], y + half[1]);
        let e = 1.0;
        let mk = |px: f32, py: f32| Vertex { pos: [px, py], local: [px - cx, py - cy], half, color: col, radius: radius.min(half[0]).min(half[1]), q: 1.0, mode: MODE_FILL, uv: [0.0; 2] };
        self.list.quad(0, [mk(x - e, y - e), mk(x + r.w + e, y - e), mk(x + r.w + e, y + r.h + e), mk(x - e, y + r.h + e)]);
    }

    /// A pixel-snapped hard-edged rectangle.
    pub fn rect(&mut self, r: Rect, color: Color) {
        if color.a * self.opacity <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let col = self.c(color);
        let x0 = (r.x + self.dx).round();
        let y0 = (r.y + self.dy).round();
        let x1 = (r.x + r.w + self.dx).round().max(x0 + 1.0);
        let y1 = (r.y + r.h + self.dy).round().max(y0 + 1.0);
        self.list.quad(0, [v([x0, y0], col), v([x1, y0], col), v([x1, y1], col), v([x0, y1], col)]);
    }

    /// A 1-px-style border box: four edges of the given widths.
    pub fn border(&mut self, r: Rect, w: [f32; 4], color: Color) {
        let [t, rt, b, l] = w;
        if t > 0.0 {
            self.rect(Rect::new(r.x, r.y, r.w, t), color);
        }
        if b > 0.0 {
            self.rect(Rect::new(r.x, r.bottom() - b, r.w, b), color);
        }
        if l > 0.0 {
            self.rect(Rect::new(r.x, r.y + t, l, r.h - t - b), color);
        }
        if rt > 0.0 {
            self.rect(Rect::new(r.right() - rt, r.y + t, rt, r.h - t - b), color);
        }
    }

    /// A rounded outline (`border` with `border-radius`): ring of width `w`.
    pub fn rrect_stroke(&mut self, r: Rect, radius: f32, w: f32, color: Color) {
        // Drawn as the outer shape minus nothing: an SDF ring would need a mode;
        // use four straight edges plus corner arcs approximated by the outer
        // rounded rect clipped by an inner one drawn in the same colour band.
        let segs = 6;
        let col = self.c(color);
        let (x, y) = (r.x + self.dx, r.y + self.dy);
        let ro = radius;
        let corners = [(x + r.w - ro, y + ro, -90.0f32), (x + r.w - ro, y + r.h - ro, 0.0), (x + ro, y + r.h - ro, 90.0), (x + ro, y + ro, 180.0)];
        let verts = &mut self.scratch.verts;
        verts.clear();
        // Outer ring, then the inner one on the same corner centres (the
        // callers keep radius >= width; square boxes use `border`).
        for rad in [ro, (ro - w).max(0.0)] {
            for (cx, cy, a0) in corners {
                for k in 0..=segs {
                    let a = (a0 + 90.0 * k as f32 / segs as f32).to_radians();
                    verts.push(v([cx + rad * a.cos(), cy + rad * a.sin()], col));
                }
            }
        }
        let n = verts.len() as u32 / 2;
        let verts = std::mem::take(&mut self.scratch.verts);
        self.list.tris(0, &verts, (0..n).flat_map(move |i| {
            let j = (i + 1) % n;
            [i, j, n + j, i, n + j, n + i]
        }));
        self.scratch.verts = verts;
    }

    /// `linear-gradient(dir, stops...)` over a rectangle; stops are (0..1, colour).
    pub fn linear(&mut self, r: Rect, dir: Dir, stops: &[(f32, Color)]) {
        if stops.is_empty() || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let (x, y) = (r.x + self.dx, r.y + self.dy);
        // Position along the gradient line -> a rectangle slice.
        let slice = |t0: f32, t1: f32| -> ([f32; 2], [f32; 2], [f32; 2], [f32; 2]) {
            match dir {
                Dir::ToBottom => ([x, y + r.h * t0], [x + r.w, y + r.h * t0], [x + r.w, y + r.h * t1], [x, y + r.h * t1]),
                Dir::ToTop => ([x, y + r.h * (1.0 - t0)], [x + r.w, y + r.h * (1.0 - t0)], [x + r.w, y + r.h * (1.0 - t1)], [x, y + r.h * (1.0 - t1)]),
                Dir::ToRight => ([x + r.w * t0, y], [x + r.w * t0, y + r.h], [x + r.w * t1, y + r.h], [x + r.w * t1, y]),
                Dir::ToLeft => ([x + r.w * (1.0 - t0), y], [x + r.w * (1.0 - t0), y + r.h], [x + r.w * (1.0 - t1), y + r.h], [x + r.w * (1.0 - t1), y]),
            }
        };
        let mut pts = std::mem::take(&mut self.scratch.pts);
        pts.clear();
        if stops[0].0 > 0.0 {
            pts.push((0.0, stops[0].1));
        }
        pts.extend_from_slice(stops);
        if pts.last().unwrap().0 < 1.0 {
            pts.push((1.0, pts.last().unwrap().1));
        }
        for w in pts.windows(2) {
            let (t0, c0) = (w[0].0.clamp(0.0, 1.0), w[0].1);
            let (t1, c1) = (w[1].0.clamp(0.0, 1.0), w[1].1);
            if t1 <= t0 || (c0.a <= 0.0 && c1.a <= 0.0) {
                continue;
            }
            let (a, b, cc, d) = slice(t0, t1);
            let (k0, k1) = (self.c(c0), self.c(c1));
            self.list.quad(0, [v(a, k0), v(b, k0), v(cc, k1), v(d, k1)]);
        }
        self.scratch.pts = pts;
    }

    /// `radial-gradient(<rx> <ry> at cx cy, stops...)` as rings of a fan,
    /// clipped to nothing (the caller's box is the screen).
    pub fn radial(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, stops: &[(f32, Color)]) {
        const SEG: u32 = 72;
        let (cx, cy) = (cx + self.dx, cy + self.dy);
        let c0 = self.c(stops[0].1);
        let mut verts = std::mem::take(&mut self.scratch.verts);
        verts.clear();
        verts.push(v([cx, cy], c0));
        let mut rings = 0u32;
        for (t, c) in stops.iter() {
            if *t <= 0.0 {
                continue;
            }
            let col = self.c(*c);
            for k in 0..SEG {
                let a = std::f32::consts::TAU * k as f32 / SEG as f32;
                verts.push(v([cx + rx * t * a.cos(), cy + ry * t * a.sin()], col));
            }
            rings += 1;
        }
        let idx = (0..SEG).flat_map(|k| [0, 1 + k, 1 + (k + 1) % SEG]).chain((1..rings).flat_map(|ring| {
            let (a, b) = (1 + (ring - 1) * SEG, 1 + ring * SEG);
            (0..SEG).flat_map(move |k| {
                let k1 = (k + 1) % SEG;
                [a + k, a + k1, b + k1, a + k, b + k1, b + k]
            })
        }));
        self.list.tris(0, &verts, idx);
        self.scratch.verts = verts;
    }

    /// A filled ellipse (or a ring of width `ring`).
    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, ring: f32, color: Color) {
        if color.a * self.opacity <= 0.0 {
            return;
        }
        let col = self.c(color);
        let (cx, cy) = (cx + self.dx, cy + self.dy);
        let half = [rx, ry];
        let mk = |px: f32, py: f32| Vertex { pos: [px, py], local: [px - cx, py - cy], half, color: col, radius: ring, q: 1.0, mode: MODE_ELLIPSE, uv: [0.0; 2] };
        let (ex, ey) = (rx + 1.0, ry + 1.0);
        self.list.quad(0, [mk(cx - ex, cy - ey), mk(cx + ex, cy - ey), mk(cx + ex, cy + ey), mk(cx - ex, cy + ey)]);
    }

    /// A filled polygon (any simple ring), triangulated with earcut.
    pub fn polygon(&mut self, pts: &[[f32; 2]], color: Color) {
        if pts.len() < 3 || color.a * self.opacity <= 0.0 {
            return;
        }
        let col = self.c(color);
        let mut tri = std::mem::take(&mut self.scratch.tri);
        let ear = self.scratch.ear.get_or_insert_with(earcut::Earcut::<f32>::new);
        ear.earcut(pts.iter().copied(), &[] as &[u32], &mut tri);
        let mut verts = std::mem::take(&mut self.scratch.verts);
        verts.clear();
        for p in pts {
            verts.push(v([p[0] + self.dx, p[1] + self.dy], col));
        }
        self.list.tris(0, &verts, tri.iter().copied());
        self.scratch.verts = verts;
        self.scratch.tri = tri;
    }

    /// Scratch point buffer for building a polygon without allocating.
    pub fn take_points(&mut self) -> Vec<[f32; 2]> {
        let mut p = std::mem::take(&mut self.scratch.ring);
        p.clear();
        p
    }
    pub fn give_points(&mut self, p: Vec<[f32; 2]>) {
        self.scratch.ring = p;
    }

    /// A stroked polyline of width `w` (butt ends, mitre-less joins).
    pub fn polyline(&mut self, pts: &[[f32; 2]], w: f32, closed: bool, color: Color) {
        let n = pts.len();
        if n < 2 {
            return;
        }
        let segs = if closed { n } else { n - 1 };
        for i in 0..segs {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-4);
            let (nx, ny) = (-dy / l * w * 0.5, dx / l * w * 0.5);
            let (ex, ey) = (dx / l * w * 0.5, dy / l * w * 0.5);
            let q = [[a[0] + nx - ex, a[1] + ny - ey], [b[0] + nx + ex, b[1] + ny + ey], [b[0] - nx + ex, b[1] - ny + ey], [a[0] - nx - ex, a[1] - ny - ey]];
            self.polygon(&q, color);
        }
    }

    /// An image (`ImageId`) over a rectangle with texture coordinates `uv`
    /// (0..1), rounded corners and an opacity.
    pub fn image(&mut self, r: Rect, image: ImageId, uv: [f32; 4], radius: f32, alpha: f32) {
        let col = Color { r: 255, g: 255, b: 255, a: alpha }.premul(self.opacity);
        let (x, y) = (r.x + self.dx, r.y + self.dy);
        let half = [r.w * 0.5, r.h * 0.5];
        let (cx, cy) = (x + half[0], y + half[1]);
        let mk = |px: f32, py: f32, u: f32, vv: f32| Vertex { pos: [px, py], local: [px - cx, py - cy], half: if radius > 0.0 { half } else { [0.0; 2] }, color: col, radius, q: 1.0, mode: MODE_IMAGE, uv: [u, vv] };
        self.list.quad(image, [mk(x, y, uv[0], uv[1]), mk(x + r.w, y, uv[2], uv[1]), mk(x + r.w, y + r.h, uv[2], uv[3]), mk(x, y + r.h, uv[0], uv[3])]);
    }

    /// An image on a projected quad: four screen corners, their texture
    /// coordinates and their 1/w (perspective-correct across the quad).
    pub fn image_projective(&mut self, corners: [[f32; 2]; 4], uvs: [[f32; 2]; 4], inv_w: [f32; 4], image: ImageId, alpha: f32) {
        let col = Color { r: 255, g: 255, b: 255, a: alpha }.premul(self.opacity);
        let mut q = [Vertex::default(); 4];
        for i in 0..4 {
            q[i] = Vertex {
                pos: [corners[i][0] + self.dx, corners[i][1] + self.dy],
                uv: [uvs[i][0] * inv_w[i], uvs[i][1] * inv_w[i]],
                q: inv_w[i],
                color: col,
                mode: MODE_IMAGE,
                ..Default::default()
            };
        }
        self.list.quad(image, q);
    }

    /// The radar plate: every pixel back-projected to the ground (see ui.wgsl).
    pub fn radar(&mut self, r: Rect, radius: f32, image: ImageId) {
        let col = crate::theme::WHITE.premul(self.opacity);
        let (x, y) = (r.x + self.dx, r.y + self.dy);
        let half = [r.w * 0.5, r.h * 0.5];
        let (cx, cy) = (x + half[0], y + half[1]);
        let mk = |px: f32, py: f32| Vertex { pos: [px, py], local: [px - cx, py - cy], half, color: col, radius, q: 1.0, mode: MODE_RADAR, uv: [0.0; 2] };
        self.list.quad(image, [mk(x, y), mk(x + r.w, y), mk(x + r.w, y + r.h), mk(x, y + r.h)]);
    }

    /// Text with its pen at `(x, baseline)`; returns the advance width.
    pub fn text(&mut self, x: f32, baseline: f32, s: &str, style: TextStyle, color: Color) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let col = self.c(color);
        let ox = (x + self.dx).round();
        let oy = (baseline + self.dy).round();
        let inv = 1.0 / ATLAS_SIZE as f32;
        let _ = inv;
        let run = self.text.run(s, style);
        let width = run.width;
        if color.a * self.opacity <= 0.0 {
            return width;
        }
        self.list.begin(0);
        for gl in &run.glyphs {
            let (x0, y0) = (ox + gl.x, oy + gl.y);
            let (x1, y1) = (x0 + gl.w, y0 + gl.h);
            let b = self.list.verts.len() as u32;
            let [u0, v0, u1, v1] = gl.uv;
            let mk = |px: f32, py: f32, u: f32, vv: f32| Vertex { pos: [px, py], uv: [u, vv], color: col, q: 1.0, mode: MODE_GLYPH, ..Default::default() };
            self.list.verts.extend_from_slice(&[mk(x0, y0, u0, v0), mk(x1, y0, u1, v0), mk(x1, y1, u1, v1), mk(x0, y1, u0, v1)]);
            self.list.idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        self.list.end();
        width
    }

    pub fn text_width(&mut self, s: &str, style: TextStyle) -> f32 {
        self.text.width(s, style)
    }

    pub fn set_scissor(&mut self, r: Option<Rect>, screen: [f32; 2]) {
        self.list.set_scissor(r.map(|r| {
            let x0 = (r.x + self.dx).max(0.0).floor();
            let y0 = (r.y + self.dy).max(0.0).floor();
            let x1 = (r.right() + self.dx).min(screen[0]).ceil();
            let y1 = (r.bottom() + self.dy).min(screen[1]).ceil();
            [x0 as u32, y0 as u32, (x1 - x0).max(0.0) as u32, (y1 - y0).max(0.0) as u32]
        }));
    }
}

/// Per-frame uniforms (see `Globals` in ui.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct Globals {
    pub screen: [f32; 4],
    pub radar0: [f32; 4],
    pub radar1: [f32; 4],
    pub radar2: [f32; 4],
    pub radar3: [f32; 4],
}

struct Image {
    bind: wgpu::BindGroup,
}

/// GPU side of the 2D layer.
pub struct UiGpu {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    ubuf: wgpu::Buffer,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    vcap: u64,
    icap: u64,
    atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    images: Vec<Image>,
    _white: wgpu::Texture,
    pub cmds: Vec<Cmd>,
    pub globals: Globals,
    last_globals: Option<Globals>,
    pub bytes_uploaded: u64,
}

impl UiGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("eo-ui"), source: wgpu::ShaderSource::Wgsl(include_str!("ui.wgsl").into()) });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("eo-ui"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("eo-ui"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("eo-ui globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mk_buf = |label, size, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let vcap = 64 * 1024 * std::mem::size_of::<Vertex>() as u64;
        let icap = 96 * 1024 * 4;
        let vbuf = mk_buf("eo-ui vertices", vcap, wgpu::BufferUsages::VERTEX);
        let ibuf = mk_buf("eo-ui indices", icap, wgpu::BufferUsages::INDEX);
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eo-ui glyph atlas"),
            size: wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("eo-ui"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let white = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eo-ui white"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            white.as_image_copy(),
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let white_view = white.create_view(&Default::default());
        let mut s = Self {
            layout,
            pipeline_layout,
            shader,
            pipelines: HashMap::new(),
            ubuf,
            vbuf,
            ibuf,
            vcap,
            icap,
            atlas,
            atlas_view,
            sampler,
            images: vec![],
            _white: white,
            cmds: vec![],
            globals: Globals::default(),
            last_globals: None,
            bytes_uploaded: 0,
        };
        s.add_image(device, &white_view);
        s
    }

    /// Register a texture the draw list can reference; returns its id.
    pub fn add_image(&mut self, device: &wgpu::Device, view: &wgpu::TextureView) -> ImageId {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("eo-ui image"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.images.push(Image { bind });
        (self.images.len() - 1) as ImageId
    }

    /// Replace a registered image's texture (same id).
    pub fn set_image(&mut self, device: &wgpu::Device, id: ImageId, view: &wgpu::TextureView) {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("eo-ui image"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.images[id as usize] = Image { bind };
    }

    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("eo-ui"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2,
                            4 => Unorm8x4, 5 => Float32, 6 => Float32, 7 => Uint32
                        ],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// Create the pipeline for a target format ahead of the first frame.
    pub fn warm(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        let _ = self.pipeline(device, format);
    }

    /// Upload a draw list (only call when it changed) and new atlas rows.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, list: &DrawList, text: &mut TextSystem) {
        let vbytes = (list.verts.len() * std::mem::size_of::<Vertex>()) as u64;
        let ibytes = (list.idx.len() * 4) as u64;
        if vbytes > self.vcap {
            self.vcap = vbytes.next_power_of_two();
            self.vbuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("eo-ui vertices"), size: self.vcap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if ibytes > self.icap {
            self.icap = ibytes.next_power_of_two();
            self.ibuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("eo-ui indices"), size: self.icap, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        if vbytes > 0 {
            queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(&list.verts));
        }
        if ibytes > 0 {
            // Index data must be a multiple of 4 bytes, which u32 always is.
            queue.write_buffer(&self.ibuf, 0, bytemuck::cast_slice(&list.idx));
        }
        self.bytes_uploaded += vbytes + ibytes;
        self.cmds.clear();
        self.cmds.extend(list.cmds.iter().filter(|c| c.count > 0).copied());
        self.upload_atlas(queue, text);
    }

    pub fn upload_atlas(&mut self, queue: &wgpu::Queue, text: &mut TextSystem) {
        if let Some((y0, y1)) = text.atlas.dirty.take() {
            let rows = y1 - y0;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.atlas, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: y0, z: 0 }, aspect: wgpu::TextureAspect::All },
                &text.atlas.pixels[(y0 * ATLAS_SIZE) as usize..(y1 * ATLAS_SIZE) as usize],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(ATLAS_SIZE), rows_per_image: Some(rows) },
                wgpu::Extent3d { width: ATLAS_SIZE, height: rows, depth_or_array_layers: 1 },
            );
            self.bytes_uploaded += (rows * ATLAS_SIZE) as u64;
        }
    }

    /// Write the uniforms if they changed.
    pub fn write_globals(&mut self, queue: &wgpu::Queue) {
        if self.last_globals != Some(self.globals) {
            queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&self.globals));
            self.last_globals = Some(self.globals);
        }
    }

    /// Draw the uploaded list over `view` (loaded, not cleared).
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        clear: Option<wgpu::Color>,
        timestamps: Option<(&wgpu::QuerySet, u32)>,
    ) {
        self.pipeline(device, format);
        let pipeline = &self.pipelines[&format];
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("eo-ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: clear.map(wgpu::LoadOp::Clear).unwrap_or(wgpu::LoadOp::Load), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps.map(|(q, i)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if self.cmds.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        let size = [self.globals.screen[0] as u32, self.globals.screen[1] as u32];
        let mut bound: Option<ImageId> = None;
        let mut sc: Option<Option<[u32; 4]>> = None;
        for c in &self.cmds {
            if bound != Some(c.image) {
                pass.set_bind_group(0, &self.images[c.image as usize].bind, &[]);
                bound = Some(c.image);
            }
            if sc != Some(c.scissor) {
                let [x, y, w, h] = c.scissor.unwrap_or([0, 0, size[0], size[1]]);
                let (x, y) = (x.min(size[0]), y.min(size[1]));
                let (w, h) = (w.min(size[0] - x), h.min(size[1] - y));
                if w == 0 || h == 0 {
                    sc = Some(c.scissor);
                    continue;
                }
                pass.set_scissor_rect(x, y, w, h);
                sc = Some(c.scissor);
            }
            if let Some([_, _, w, h]) = c.scissor {
                if w == 0 || h == 0 {
                    continue;
                }
            }
            pass.draw_indexed(c.start..c.start + c.count, 0, 0..1);
        }
    }
}
