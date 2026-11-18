//! A baseline (SOF0, 8-bit, Huffman) JPEG decoder: enough for the scanned
//! street surface sets the JS game loads from `public/assets/` (Poly Haven
//! diffuse maps, 4:4:4 and 4:2:0). No crate in the workspace decodes JPEG,
//! and these are the only JPEGs the native renderer reads. Progressive,
//! arithmetic-coded and 12-bit files are refused with an error.
//!
//! Output is interleaved RGB8, with the JFIF YCbCr -> RGB transform and the
//! chroma upsampled by replication (the browser's decoder interpolates; the
//! difference is confined to chroma edges inside one 2x2 block).

use anyhow::{Result, anyhow, bail};

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone, Default)]
struct Huffman {
    /// (code length, code) -> symbol, as canonical tables: max code per length and value offsets.
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    values: Vec<u8>,
    /// Fast lookup on the next 9 bits: (length << 8) | symbol, 0 when longer.
    fast: Vec<u16>,
}

impl Huffman {
    fn new(counts: &[u8; 16], values: Vec<u8>) -> Self {
        let mut h = Huffman { values, ..Default::default() };
        let mut code = 0i32;
        let mut k = 0i32;
        for l in 1..=16 {
            let n = counts[l - 1] as i32;
            h.valptr[l] = k;
            h.mincode[l] = code;
            code += n;
            k += n;
            h.maxcode[l] = if n > 0 { code - 1 } else { -1 };
            code <<= 1;
        }
        h.maxcode[17] = i32::MAX;
        h.fast = vec![0u16; 512];
        let mut code = 0u32;
        let mut k = 0usize;
        for l in 1..=16u32 {
            for _ in 0..counts[l as usize - 1] {
                if l <= 9 {
                    let shift = 9 - l;
                    for f in 0..(1u32 << shift) {
                        h.fast[((code << shift) | f) as usize] = ((l as u16) << 8) | h.values[k] as u16;
                    }
                }
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        h
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    marker_hit: bool,
}

impl<'a> Bits<'a> {
    fn fill(&mut self) {
        while self.n <= 56 {
            let mut b = 0u8;
            if !self.marker_hit && self.pos < self.data.len() {
                b = self.data[self.pos];
                if b == 0xFF {
                    let nx = *self.data.get(self.pos + 1).unwrap_or(&0);
                    if nx == 0x00 {
                        self.pos += 2;
                    } else {
                        // A marker: feed zeros, leave pos at the marker.
                        self.marker_hit = true;
                        b = 0;
                    }
                } else {
                    self.pos += 1;
                }
            }
            self.acc |= (b as u64) << (56 - self.n);
            self.n += 8;
        }
    }
    fn peek(&mut self, k: u32) -> u32 {
        if self.n < k {
            self.fill();
        }
        (self.acc >> (64 - k)) as u32
    }
    fn skip(&mut self, k: u32) {
        self.acc <<= k;
        self.n -= k;
    }
    fn get(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        let v = self.peek(k);
        self.skip(k);
        v
    }
    fn decode(&mut self, h: &Huffman) -> Result<u8> {
        let p = self.peek(9) as usize;
        let f = h.fast[p];
        if f != 0 {
            self.skip((f >> 8) as u32);
            return Ok((f & 255) as u8);
        }
        let mut code = self.peek(16) as i32;
        for l in 10..=16 {
            let c = code >> (16 - l);
            if c <= h.maxcode[l] {
                self.skip(l as u32);
                let idx = h.valptr[l] + c - h.mincode[l];
                return h.values.get(idx as usize).copied().ok_or_else(|| anyhow!("jpeg: bad huffman index"));
            }
        }
        code = 0;
        let _ = code;
        bail!("jpeg: bad huffman code")
    }
    fn receive_extend(&mut self, s: u32) -> i32 {
        if s == 0 {
            return 0;
        }
        let v = self.get(s) as i32;
        if v < (1 << (s - 1)) { v - (1 << s) + 1 } else { v }
    }
    /// Discard the bit buffer and step over an RSTn marker.
    fn restart(&mut self) {
        self.acc = 0;
        self.n = 0;
        self.marker_hit = false;
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }
}

struct Comp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    td: usize,
    ta: usize,
    pred: i32,
    plane: Vec<u8>,
    stride: usize,
}

/// Separable 8x8 inverse DCT (float, straightforward; decode speed matters little here).
fn idct(block: &[i32; 64], q: &[u16; 64], out: &mut [u8], stride: usize) {
    static mut COS: [[f32; 8]; 8] = [[0.0; 8]; 8];
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        for x in 0..8 {
            for u in 0..8 {
                let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
                // SAFETY: written once inside call_once before any read.
                unsafe { COS[x][u] = cu * (((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI) / 16.0).cos() };
            }
        }
    });
    // SAFETY: initialised above, read-only afterwards.
    let cos = unsafe { &*std::ptr::addr_of!(COS) };
    let mut f = [0f32; 64];
    for k in 0..64 {
        f[k] = (block[k] * q[k] as i32) as f32;
    }
    let mut tmp = [0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut s = 0.0;
            for u in 0..8 {
                s += cos[x][u] * f[y * 8 + u];
            }
            tmp[y * 8 + x] = s / 2.0;
        }
    }
    for x in 0..8 {
        for y in 0..8 {
            let mut s = 0.0;
            for v in 0..8 {
                s += cos[y][v] * tmp[v * 8 + x];
            }
            out[y * stride + x] = (s / 2.0 + 128.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Decode a baseline JPEG to (width, height, RGB8).
pub fn decode_rgb(data: &[u8]) -> Result<(usize, usize, Vec<u8>)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        bail!("jpeg: no SOI");
    }
    let mut qt = [[0u16; 64]; 4];
    let mut dc: Vec<Option<Huffman>> = vec![None; 4];
    let mut ac: Vec<Option<Huffman>> = vec![None; 4];
    let mut comps: Vec<Comp> = vec![];
    let (mut w, mut h) = (0usize, 0usize);
    let mut restart_interval = 0usize;
    let mut pos = 2;
    loop {
        while pos < data.len() && data[pos] != 0xFF {
            pos += 1;
        }
        while pos < data.len() && data[pos] == 0xFF {
            pos += 1;
        }
        if pos >= data.len() {
            bail!("jpeg: truncated before SOS");
        }
        let m = data[pos];
        pos += 1;
        if m == 0xD9 {
            bail!("jpeg: EOI before SOS");
        }
        let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        let seg = &data[pos + 2..pos + len];
        match m {
            0xDB => {
                let mut i = 0;
                while i < seg.len() {
                    let (pq, tq) = (seg[i] >> 4, (seg[i] & 15) as usize);
                    i += 1;
                    for k in 0..64 {
                        let v = if pq == 0 { seg[i + k] as u16 } else { u16::from_be_bytes([seg[i + 2 * k], seg[i + 2 * k + 1]]) };
                        qt[tq][ZIGZAG[k]] = v;
                    }
                    i += if pq == 0 { 64 } else { 128 };
                }
            }
            0xC4 => {
                let mut i = 0;
                while i < seg.len() {
                    let (tc, th) = (seg[i] >> 4, (seg[i] & 15) as usize);
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&seg[i + 1..i + 17]);
                    let n: usize = counts.iter().map(|c| *c as usize).sum();
                    let values = seg[i + 17..i + 17 + n].to_vec();
                    let t = Huffman::new(&counts, values);
                    if tc == 0 { dc[th] = Some(t) } else { ac[th] = Some(t) }
                    i += 17 + n;
                }
            }
            0xDD => restart_interval = u16::from_be_bytes([seg[0], seg[1]]) as usize,
            0xC0 => {
                if seg[0] != 8 {
                    bail!("jpeg: {}-bit samples not supported", seg[0]);
                }
                h = u16::from_be_bytes([seg[1], seg[2]]) as usize;
                w = u16::from_be_bytes([seg[3], seg[4]]) as usize;
                let n = seg[5] as usize;
                for c in 0..n {
                    let b = &seg[6 + 3 * c..9 + 3 * c];
                    comps.push(Comp { id: b[0], h: (b[1] >> 4) as usize, v: (b[1] & 15) as usize, tq: b[2] as usize, td: 0, ta: 0, pred: 0, plane: vec![], stride: 0 });
                }
            }
            0xC1..=0xCF if m != 0xC4 && m != 0xC8 && m != 0xCC => bail!("jpeg: only baseline (SOF0) is supported, found SOF{}", m - 0xC0),
            0xDA => {
                let ns = seg[0] as usize;
                for k in 0..ns {
                    let (cid, t) = (seg[1 + 2 * k], seg[2 + 2 * k]);
                    let c = comps.iter_mut().find(|c| c.id == cid).ok_or_else(|| anyhow!("jpeg: scan names unknown component"))?;
                    c.td = (t >> 4) as usize;
                    c.ta = (t & 15) as usize;
                }
                if ns != comps.len() {
                    bail!("jpeg: non-interleaved scans not supported");
                }
                pos += len;
                break;
            }
            _ => {}
        }
        pos += len;
    }
    if w == 0 || comps.is_empty() {
        bail!("jpeg: no frame header");
    }
    let hmax = comps.iter().map(|c| c.h).max().unwrap();
    let vmax = comps.iter().map(|c| c.v).max().unwrap();
    let mcux = w.div_ceil(8 * hmax);
    let mcuy = h.div_ceil(8 * vmax);
    for c in comps.iter_mut() {
        c.stride = mcux * c.h * 8;
        c.plane = vec![0u8; c.stride * mcuy * c.v * 8];
    }
    let mut bits = Bits { data, pos, acc: 0, n: 0, marker_hit: false };
    let mut block = [0i32; 64];
    let mut mcu_count = 0usize;
    for my in 0..mcuy {
        for mx in 0..mcux {
            if restart_interval > 0 && mcu_count > 0 && mcu_count % restart_interval == 0 {
                bits.restart();
                for c in comps.iter_mut() {
                    c.pred = 0;
                }
            }
            mcu_count += 1;
            for c in comps.iter_mut() {
                let dct = dc[c.td].as_ref().ok_or_else(|| anyhow!("jpeg: missing DC table"))?;
                let act = ac[c.ta].as_ref().ok_or_else(|| anyhow!("jpeg: missing AC table"))?;
                for by in 0..c.v {
                    for bx in 0..c.h {
                        block.fill(0);
                        let t = bits.decode(dct)? as u32;
                        let diff = bits.receive_extend(t);
                        c.pred += diff;
                        block[0] = c.pred;
                        let mut k = 1;
                        while k < 64 {
                            let rs = bits.decode(act)?;
                            let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
                            if s == 0 {
                                if r == 15 {
                                    k += 16;
                                    continue;
                                }
                                break;
                            }
                            k += r;
                            if k > 63 {
                                bail!("jpeg: AC run past the block");
                            }
                            block[ZIGZAG[k]] = bits.receive_extend(s);
                            k += 1;
                        }
                        let x0 = (mx * c.h + bx) * 8;
                        let y0 = (my * c.v + by) * 8;
                        let stride = c.stride;
                        idct(&block, &qt[c.tq], &mut c.plane[y0 * stride + x0..], stride);
                    }
                }
            }
        }
    }
    let mut out = vec![0u8; w * h * 3];
    let gray = comps.len() == 1;
    for y in 0..h {
        for x in 0..w {
            let s = |c: &Comp| c.plane[(y * c.v / vmax) * c.stride + x * c.h / hmax] as f32;
            let o = (y * w + x) * 3;
            if gray {
                let v = s(&comps[0]) as u8;
                out[o..o + 3].copy_from_slice(&[v, v, v]);
            } else {
                let (yy, cb, cr) = (s(&comps[0]), s(&comps[1]) - 128.0, s(&comps[2]) - 128.0);
                out[o] = (yy + 1.402 * cr).round().clamp(0.0, 255.0) as u8;
                out[o + 1] = (yy - 0.344136 * cb - 0.714136 * cr).round().clamp(0.0, 255.0) as u8;
                out[o + 2] = (yy + 1.772 * cb).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok((w, h, out))
}
