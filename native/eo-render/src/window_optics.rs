//! Static geometry and baked surface radiance for reflected rays from the furnished house.
use anyhow::{Result, ensure};

const HEADER: usize = 7;
const LEAF: u32 = 1 << 31;
const MAGIC: u32 = u32::from_le_bytes(*b"BWO1");

#[derive(Debug)]
pub struct WindowOptics(Vec<u8>);

impl WindowOptics {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        ensure!(
            bytes.len() >= 16 * HEADER && bytes.len() % 16 == 0,
            "invalid optical scene length"
        );
        let records = bytes.len() / 16;
        let word = |i: usize, k: usize| {
            let offset = 16 * i + 4 * k;
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
        };
        let vertices = word(0, 1) as usize;
        let sky = word(0, 2) as usize;
        let count = word(0, 3) as usize;
        let triangles = word(6, 1);
        ensure!(
            word(0, 0) == MAGIC
                && word(6, 0) == HEADER as u32
                && triangles > 0
                && HEADER < vertices
                && vertices < sky
                && sky < records
                && count.checked_mul(2).and_then(|n| n.checked_add(vertices)) == Some(sky)
                && word(1, 3) == 160
                && word(2, 3) == 80
                && word(6, 2) == 5
                && (word(6, 3) & !LEAF) == 64000
                && records - sky == 16000,
            "incompatible optical scene"
        );
        for k in 0..3 {
            let step = f32::from_bits(word(2, k));
            ensure!(
                step.is_finite() && step > 0. && f32::from_bits(word(1, k)).is_finite(),
                "invalid optical bounds"
            );
        }
        for r in 3..6 {
            ensure!(
                (0..4).all(|k| f32::from_bits(word(r, k)).is_finite()),
                "invalid optical basis"
            );
        }
        // Every traversal advances to its child or its escape address. Check
        // the complete tree before a shader is allowed to read those addresses.
        fn subtree(
            word: &impl Fn(usize, usize) -> u32,
            at: usize,
            limit: usize,
            depth: u32,
            vertices: usize,
            triangles: &mut u32,
        ) -> Result<usize> {
            ensure!(
                at < limit && depth < 64,
                "invalid optical tree depth or child"
            );
            let link = word(at, 3);
            let end = (link & !LEAF) as usize;
            ensure!(
                at + 1 < end && end <= limit,
                "optical tree escape outside parent"
            );
            for k in 0..3 {
                let axis = word(at, k);
                ensure!((axis & 65535) <= (axis >> 16), "inverted optical bound");
            }
            if link & LEAF != 0 {
                ensure!(end - at - 1 <= 8, "invalid optical leaf size");
                for i in at + 1..end {
                    ensure!(
                        (0..3).all(|k| (word(i, k) as usize) < vertices),
                        "optical triangle outside vertex table"
                    );
                }
                *triangles += (end - at - 1) as u32;
            } else {
                let right = subtree(word, at + 1, end, depth + 1, vertices, triangles)?;
                ensure!(
                    subtree(word, right, end, depth + 1, vertices, triangles)? == end,
                    "invalid optical siblings"
                );
            }
            Ok(end)
        }
        let mut found = 0;
        ensure!(
            subtree(&word, HEADER, vertices, 0, count, &mut found)? == vertices
                && found == triangles,
            "incomplete optical tree"
        );
        for i in (vertices..sky).step_by(2) {
            ensure!(
                (0..3).all(|k| f32::from_bits(word(i, k)).is_finite()),
                "invalid optical vertex"
            );
        }
        Ok(Self(bytes))
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

/// The four measured solar states include a night field. Keep that field
/// through the night, interpolating only through the dawn and dusk intervals.
pub(crate) fn light_sample(hour: f32) -> [f32; 3] {
    let h = hour.rem_euclid(24.);
    if h < 6. || h >= 20. {
        [3., 3., 0.]
    } else if h < 8.5 {
        [3., 0., (h - 6.) / 2.5]
    } else if h < 12. {
        [0., 1., (h - 8.5) / 3.5]
    } else if h < 17. {
        [1., 2., (h - 12.) / 5.]
    } else {
        [2., 3., (h - 17.) / 3.]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_escape_and_vertex_addresses_before_upload() {
        let first_vertex = 9;
        let first_sky = first_vertex + 6;
        let mut rows = vec![[0u32; 4]; first_sky + 16000];
        rows[0] = [MAGIC, first_vertex as u32, first_sky as u32, 3];
        rows[1][3] = 160;
        rows[2] = [1f32.to_bits(), 1f32.to_bits(), 1f32.to_bits(), 80];
        rows[6] = [7, 1, 5, 64000];
        rows[7] = [65535u32 << 16, 65535u32 << 16, 65535u32 << 16, LEAF | 9];
        rows[8] = [0, 1, 2, 0];
        let check = |records: &[[u32; 4]]| {
            WindowOptics::from_bytes(
                records
                    .iter()
                    .flatten()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            )
        };
        assert!(check(&rows).is_ok());
        rows[7][3] = LEAF | 7;
        assert!(check(&rows).is_err());
        rows[7][3] = 9;
        assert!(check(&rows).is_err());
        rows[7][3] = LEAF | 9;
        rows[8][0] = 3;
        assert!(check(&rows).is_err());
        assert_eq!(light_sample(0.), light_sample(22.));
        assert_eq!(light_sample(12.), [1., 2., 0.]);
        assert_eq!(light_sample(17.), [2., 3., 0.]);
    }
}
