//! A reader for the packed people GLBs (`people` source): the glTF JSON and
//! binary chunk, buffer views compressed with `EXT_meshopt_compression`
//! (decoded with meshoptimizer's own decoder, which is safe for untrusted
//! input), accessors quantized per `KHR_mesh_quantization`, and images in
//! `EXT_texture_webp` (decoded by `image-webp`). Everything is bounds checked:
//! these files are downloaded data. No file outside the GLB is ever read.

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use std::borrow::Cow;

/// Largest vertex or index count, and image side, this reader accepts.
const MAX_COUNT: usize = 4_000_000;
const MAX_IMAGE: u32 = 4096;

pub struct Glb {
    pub json: Value,
    bin: Vec<u8>,
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| anyhow!("GLB truncated"))
}

fn uint(v: &Value, key: &str) -> Option<usize> {
    v.get(key).and_then(|x| x.as_u64()).map(|x| x as usize)
}

impl Glb {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if u32_at(bytes, 0)? != 0x4654_6c67 || u32_at(bytes, 4)? != 2 {
            bail!("not a glTF 2 binary");
        }
        let total = (u32_at(bytes, 8)? as usize).min(bytes.len());
        let jlen = u32_at(bytes, 12)? as usize;
        if u32_at(bytes, 16)? != 0x4e4f_534a || 20 + jlen > total {
            bail!("GLB without a JSON chunk");
        }
        let json: Value = serde_json::from_slice(&bytes[20..20 + jlen])?;
        let mut at = 20 + jlen;
        let mut bin = vec![];
        if at + 8 <= total {
            let blen = u32_at(bytes, at)? as usize;
            if u32_at(bytes, at + 4)? == 0x004e_4942 && at + 8 + blen <= total {
                bin = bytes[at + 8..at + 8 + blen].to_vec();
            }
            at += 8 + blen;
        }
        let _ = at;
        // Only the GLB's own binary chunk: a buffer with a URI is refused.
        if let Some(bufs) = json.get("buffers").and_then(|b| b.as_array()) {
            for (i, b) in bufs.iter().enumerate() {
                if b.get("uri").is_some() {
                    bail!("buffer {i} names an external URI");
                }
            }
        }
        Ok(Glb { json, bin })
    }

    fn arr(&self, key: &str) -> &[Value] {
        self.json.get(key).and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn nodes(&self) -> &[Value] {
        self.arr("nodes")
    }
    pub fn materials(&self) -> &[Value] {
        self.arr("materials")
    }
    pub fn meshes(&self) -> &[Value] {
        self.arr("meshes")
    }
    pub fn skins(&self) -> &[Value] {
        self.arr("skins")
    }

    /// A buffer view's bytes (meshopt-decoded when compressed).
    pub fn view(&self, i: usize) -> Result<Cow<'_, [u8]>> {
        let bv = self.arr("bufferViews").get(i).ok_or_else(|| anyhow!("bufferView {i} missing"))?;
        if let Some(m) = bv.get("extensions").and_then(|e| e.get("EXT_meshopt_compression")) {
            if uint(m, "buffer").unwrap_or(0) != 0 {
                bail!("meshopt data outside the binary chunk");
            }
            let off = uint(m, "byteOffset").unwrap_or(0);
            let len = uint(m, "byteLength").ok_or_else(|| anyhow!("meshopt view without byteLength"))?;
            let stride = uint(m, "byteStride").ok_or_else(|| anyhow!("meshopt view without byteStride"))?;
            let count = uint(m, "count").ok_or_else(|| anyhow!("meshopt view without count"))?;
            let src = self.bin.get(off..off + len).ok_or_else(|| anyhow!("meshopt view outside the binary chunk"))?;
            if count > MAX_COUNT || stride == 0 || stride > 256 {
                bail!("meshopt view of {count} x {stride} bytes refused");
            }
            let mut out = vec![0u8; count * stride];
            let mode = m.get("mode").and_then(|x| x.as_str()).unwrap_or("ATTRIBUTES");
            // SAFETY: `out` holds count * stride bytes, as each decoder writes; the
            // decoders validate their input (meshoptimizer: safe for untrusted data).
            let rc = unsafe {
                match mode {
                    "ATTRIBUTES" => {
                        if stride % 4 != 0 {
                            bail!("meshopt attribute stride {stride}");
                        }
                        meshopt::ffi::meshopt_decodeVertexBuffer(out.as_mut_ptr().cast(), count, stride, src.as_ptr(), src.len())
                    }
                    "TRIANGLES" | "INDICES" if stride == 2 || stride == 4 => {
                        if mode == "TRIANGLES" {
                            meshopt::ffi::meshopt_decodeIndexBuffer(out.as_mut_ptr().cast(), count, stride, src.as_ptr(), src.len())
                        } else {
                            meshopt::ffi::meshopt_decodeIndexSequence(out.as_mut_ptr().cast(), count, stride, src.as_ptr(), src.len())
                        }
                    }
                    other => bail!("meshopt mode {other} (stride {stride})"),
                }
            };
            if rc != 0 {
                bail!("meshopt decode failed ({rc})");
            }
            match m.get("filter").and_then(|x| x.as_str()).unwrap_or("NONE") {
                "NONE" => {}
                // SAFETY: in place over the decoded buffer with its own stride and count.
                "OCTAHEDRAL" if stride == 4 || stride == 8 => unsafe { meshopt::ffi::meshopt_decodeFilterOct(out.as_mut_ptr().cast(), count, stride) },
                "QUATERNION" if stride == 8 => unsafe { meshopt::ffi::meshopt_decodeFilterQuat(out.as_mut_ptr().cast(), count, stride) },
                "EXPONENTIAL" => unsafe { meshopt::ffi::meshopt_decodeFilterExp(out.as_mut_ptr().cast(), count, stride) },
                "COLOR" if stride == 4 || stride == 8 => unsafe { meshopt::ffi::meshopt_decodeFilterColor(out.as_mut_ptr().cast(), count, stride) },
                f => bail!("meshopt filter {f} with stride {stride}"),
            }
            return Ok(Cow::Owned(out));
        }
        if uint(bv, "buffer").unwrap_or(0) != 0 {
            bail!("bufferView {i} outside the binary chunk");
        }
        let off = uint(bv, "byteOffset").unwrap_or(0);
        let len = uint(bv, "byteLength").ok_or_else(|| anyhow!("bufferView {i} without byteLength"))?;
        self.bin.get(off..off + len).map(Cow::Borrowed).ok_or_else(|| anyhow!("bufferView {i} outside the binary chunk"))
    }

    fn view_stride(&self, i: usize) -> Option<usize> {
        let bv = self.arr("bufferViews").get(i)?;
        bv.get("extensions").and_then(|e| e.get("EXT_meshopt_compression")).and_then(|m| uint(m, "byteStride")).or_else(|| uint(bv, "byteStride"))
    }

    /// An accessor as floats, up to 4 components a row (normalized integers
    /// mapped as glTF defines; other integers as their value).
    pub fn floats(&self, acc: usize) -> Result<Vec<[f32; 4]>> {
        let a = self.arr("accessors").get(acc).ok_or_else(|| anyhow!("accessor {acc} missing"))?;
        if a.get("sparse").is_some() {
            bail!("sparse accessor {acc}");
        }
        let count = uint(a, "count").ok_or_else(|| anyhow!("accessor {acc} without count"))?;
        if count > MAX_COUNT {
            bail!("accessor {acc}: {count} elements");
        }
        let n = match a.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            t => bail!("accessor type {t}"),
        };
        let ct = uint(a, "componentType").unwrap_or(0);
        let size = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            c => bail!("component type {c}"),
        };
        let norm = a.get("normalized").and_then(|x| x.as_bool()).unwrap_or(false);
        let Some(bv) = uint(a, "bufferView") else { return Ok(vec![[0.0; 4]; count]) };
        let data = self.view(bv)?;
        let stride = self.view_stride(bv).unwrap_or(n * size);
        let off = uint(a, "byteOffset").unwrap_or(0);
        if count > 0 && off + (count - 1) * stride + n * size > data.len() {
            bail!("accessor {acc} runs past its view");
        }
        let read = |at: usize| -> f32 {
            let b = &data[at..at + size];
            match (ct, norm) {
                (5120, true) => (b[0] as i8 as f32 / 127.0).max(-1.0),
                (5120, false) => b[0] as i8 as f32,
                (5121, true) => b[0] as f32 / 255.0,
                (5121, false) => b[0] as f32,
                (5122, true) => (i16::from_le_bytes([b[0], b[1]]) as f32 / 32767.0).max(-1.0),
                (5122, false) => i16::from_le_bytes([b[0], b[1]]) as f32,
                (5123, true) => u16::from_le_bytes([b[0], b[1]]) as f32 / 65535.0,
                (5123, false) => u16::from_le_bytes([b[0], b[1]]) as f32,
                (5125, _) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            }
        };
        if n == 16 {
            bail!("use mat4s() for accessor {acc}");
        }
        Ok((0..count)
            .map(|k| {
                let mut v = [0.0; 4];
                for (c, slot) in v.iter_mut().enumerate().take(n) {
                    *slot = read(off + k * stride + c * size);
                }
                v
            })
            .collect())
    }

    /// An integer accessor (indices, joints), up to 4 components a row.
    pub fn uints(&self, acc: usize) -> Result<Vec<[u32; 4]>> {
        let a = self.arr("accessors").get(acc).ok_or_else(|| anyhow!("accessor {acc} missing"))?;
        if a.get("normalized").and_then(|x| x.as_bool()).unwrap_or(false) {
            bail!("integer accessor {acc} is normalized");
        }
        Ok(self.floats(acc)?.into_iter().map(|v| v.map(|c| c as u32)).collect())
    }

    /// A MAT4 float accessor (inverse bind matrices), column-major.
    pub fn mat4s(&self, acc: usize) -> Result<Vec<[f32; 16]>> {
        let a = self.arr("accessors").get(acc).ok_or_else(|| anyhow!("accessor {acc} missing"))?;
        if a.get("type").and_then(|t| t.as_str()) != Some("MAT4") || uint(a, "componentType") != Some(5126) {
            bail!("accessor {acc} is not a float MAT4");
        }
        let count = uint(a, "count").unwrap_or(0);
        let bv = uint(a, "bufferView").ok_or_else(|| anyhow!("MAT4 accessor without a view"))?;
        let data = self.view(bv)?;
        let stride = self.view_stride(bv).unwrap_or(64);
        let off = uint(a, "byteOffset").unwrap_or(0);
        if count > 4096 || (count > 0 && off + (count - 1) * stride + 64 > data.len()) {
            bail!("MAT4 accessor {acc} runs past its view");
        }
        Ok((0..count)
            .map(|k| {
                let mut m = [0f32; 16];
                for (c, slot) in m.iter_mut().enumerate() {
                    let at = off + k * stride + c * 4;
                    *slot = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                }
                m
            })
            .collect())
    }

    pub(crate) fn texture_bytes(&self, texture: usize) -> Result<(&str, Cow<'_, [u8]>)> {
        let t = self.arr("textures").get(texture).ok_or_else(|| anyhow!("texture {texture} missing"))?;
        let src = t.get("extensions").and_then(|e| e.get("EXT_texture_webp")).and_then(|w| uint(w, "source")).or_else(|| uint(t, "source")).ok_or_else(|| anyhow!("texture {texture} without a source"))?;
        let im = self.arr("images").get(src).ok_or_else(|| anyhow!("image {src} missing"))?;
        if im.get("uri").is_some() {
            bail!("image {src} names an external URI");
        }
        let bv = uint(im, "bufferView").ok_or_else(|| anyhow!("image {src} without a buffer view"))?;
        Ok((im.get("mimeType").and_then(|m| m.as_str()).unwrap_or(""), self.view(bv)?))
    }

    /// The image a texture samples (`EXT_texture_webp` source, else `source`), as RGBA8.
    pub fn texture_rgba(&self, texture: usize) -> Result<(u32, u32, Vec<u8>)> {
        let (mime, bytes) = self.texture_bytes(texture)?;
        match mime {
            "image/webp" => decode_webp(&bytes),
            m => bail!("texture {texture}: {m} is not decoded here"),
        }
    }
}

fn decode_webp(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut dec = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| anyhow!("webp: {e}"))?;
    let (w, h) = dec.dimensions();
    if w == 0 || h == 0 || w > MAX_IMAGE || h > MAX_IMAGE || dec.is_animated() {
        bail!("webp {w}x{h} refused");
    }
    dec.set_memory_limit(64 << 20);
    let n = dec.output_buffer_size().ok_or_else(|| anyhow!("webp too large"))?;
    let mut buf = vec![0u8; n];
    dec.read_image(&mut buf).map_err(|e| anyhow!("webp: {e}"))?;
    let px = (w * h) as usize;
    let rgba = if dec.has_alpha() { buf } else { buf[..px * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect() };
    Ok((w, h, rgba))
}
