//! Bit packing: the snapshot and input codecs write fields at their own
//! widths, and small integers (deltas, gaps) in a two-bit size class.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncated;

impl std::fmt::Display for Truncated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bit stream ended early")
    }
}
impl std::error::Error for Truncated {}

/// Size classes of `uvar`: 2 prefix bits, then 3, 7, 12 or 32 value bits.
pub const CLASSES: [u32; 4] = [3, 7, 12, 32];

#[derive(Default, Clone)]
pub struct BitWriter {
    buf: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(bytes: usize) -> Self {
        Self { buf: Vec::with_capacity(bytes), acc: 0, n: 0 }
    }

    /// Write the low `bits` bits of `v` (bits <= 32).
    #[inline]
    pub fn write(&mut self, v: u32, bits: u32) {
        debug_assert!(bits <= 32);
        if bits == 0 {
            return;
        }
        let v = (v as u64) & ((1u64 << bits) - 1);
        self.acc |= v << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.buf.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    #[inline]
    pub fn bit(&mut self, b: bool) {
        self.write(b as u32, 1);
    }

    pub fn u64(&mut self, v: u64) {
        self.write(v as u32, 32);
        self.write((v >> 32) as u32, 32);
    }

    pub fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }

    pub fn f32(&mut self, v: f32) {
        self.write(v.to_bits(), 32);
    }

    /// An unsigned value in the smallest of `CLASSES` that holds it.
    #[inline]
    pub fn uvar(&mut self, v: u32) {
        let c = if v < 1 << CLASSES[0] {
            0
        } else if v < 1 << CLASSES[1] {
            1
        } else if v < 1 << CLASSES[2] {
            2
        } else {
            3
        };
        self.write(c, 2);
        self.write(v, CLASSES[c as usize]);
    }

    /// A signed value, zigzagged into `uvar`.
    #[inline]
    pub fn svar(&mut self, v: i32) {
        self.uvar(((v << 1) ^ (v >> 31)) as u32);
    }

    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.write(x as u32, 8);
        }
    }

    pub fn bits(&self) -> usize {
        self.buf.len() * 8 + self.n as usize
    }

    pub fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.buf.push(self.acc as u8);
        }
        self.buf
    }
}

pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[inline]
    pub fn read(&mut self, bits: u32) -> Result<u32, Truncated> {
        if bits == 0 {
            return Ok(0);
        }
        if self.pos + bits as usize > self.data.len() * 8 {
            return Err(Truncated);
        }
        let mut v: u64 = 0;
        let mut got = 0u32;
        while got < bits {
            let byte = self.data[self.pos / 8] as u64;
            let off = (self.pos % 8) as u32;
            let take = (8 - off).min(bits - got);
            v |= ((byte >> off) & ((1 << take) - 1)) << got;
            got += take;
            self.pos += take as usize;
        }
        Ok(v as u32)
    }

    #[inline]
    pub fn bit(&mut self) -> Result<bool, Truncated> {
        Ok(self.read(1)? != 0)
    }

    pub fn u64(&mut self) -> Result<u64, Truncated> {
        let lo = self.read(32)? as u64;
        let hi = self.read(32)? as u64;
        Ok(lo | (hi << 32))
    }

    pub fn f64(&mut self) -> Result<f64, Truncated> {
        Ok(f64::from_bits(self.u64()?))
    }

    pub fn f32(&mut self) -> Result<f32, Truncated> {
        Ok(f32::from_bits(self.read(32)?))
    }

    #[inline]
    pub fn uvar(&mut self) -> Result<u32, Truncated> {
        let c = self.read(2)?;
        self.read(CLASSES[c as usize])
    }

    #[inline]
    pub fn svar(&mut self) -> Result<i32, Truncated> {
        let u = self.uvar()?;
        Ok(((u >> 1) as i32) ^ -((u & 1) as i32))
    }

    pub fn bytes(&mut self, n: usize) -> Result<Vec<u8>, Truncated> {
        (0..n).map(|_| self.read(8).map(|b| b as u8)).collect()
    }

    pub fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }
}

/// Little-endian byte cursor for the packet framing (headers are byte aligned).
pub struct ByteReader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> ByteReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Truncated> {
        if self.pos + n > self.data.len() {
            return Err(Truncated);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, Truncated> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, Truncated> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, Truncated> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, Truncated> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Result<f32, Truncated> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn f64(&mut self) -> Result<f64, Truncated> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.data[self.pos..];
        self.pos = self.data.len();
        s
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

pub trait PutLe {
    fn put_u8(&mut self, v: u8);
    fn put_u16(&mut self, v: u16);
    fn put_u32(&mut self, v: u32);
    fn put_u64(&mut self, v: u64);
    fn put_f32(&mut self, v: f32);
    fn put_f64(&mut self, v: f64);
}

impl PutLe for Vec<u8> {
    fn put_u8(&mut self, v: u8) {
        self.push(v);
    }
    fn put_u16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_u32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_u64(&mut self, v: u64) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_f32(&mut self, v: f32) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_f64(&mut self, v: f64) {
        self.extend_from_slice(&v.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut w = BitWriter::new();
        let vals: Vec<i32> = vec![0, 1, -1, 3, -4, 63, -64, 2047, -2048, 1 << 20, -(1 << 30), i32::MAX, i32::MIN + 1];
        for &v in &vals {
            w.svar(v);
            w.write(5, 3);
            w.bit(v < 0);
        }
        w.f64(std::f64::consts::PI);
        w.u64(u64::MAX - 7);
        let b = w.finish();
        let mut r = BitReader::new(&b);
        for &v in &vals {
            assert_eq!(r.svar().unwrap(), v);
            assert_eq!(r.read(3).unwrap(), 5);
            assert_eq!(r.bit().unwrap(), v < 0);
        }
        assert_eq!(r.f64().unwrap(), std::f64::consts::PI);
        assert_eq!(r.u64().unwrap(), u64::MAX - 7);
        assert!(r.read(9).is_err());
    }
}
