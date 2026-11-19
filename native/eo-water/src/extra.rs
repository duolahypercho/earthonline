//! The water's extra field: one RGBA16F texture over a rectangle of the
//! water, beside the shore field and the exposure field, for what a later
//! pass measures over the sea (for example depth from a bathymetric survey,
//! or the surf zone). eo-render uploads it to the water, underwater, spray
//! and surf pipelines (group 0, `w_extra` and `w_extra_samp` in
//! water_common.wgsl) with its frame in the water uniform (`wv_extra`).
//! There is none yet: `field()` answers None, which uploads a 1 x 1 zero
//! texture with the identity frame, and no shader reads it.

/// An extra field: `width` x `height` texels of `cell_m` (game metres) from
/// `origin` (game x, z of texel (0, 0)'s outer corner), rows along +z, each
/// texel four IEEE 754 half floats (bits) R, G, B, A.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraField {
    pub origin: [f64; 2],
    pub cell_m: f64,
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[u16; 4]>,
}

impl ExtraField {
    /// The field's frame as the water uniform carries it: origin x, z and
    /// 1 / extent x, z (so a world point maps to its texture coordinate as
    /// (xz - origin) x inverse extent).
    pub fn frame(&self) -> [f64; 4] {
        [self.origin[0], self.origin[1], 1.0 / (self.width as f64 * self.cell_m), 1.0 / (self.height as f64 * self.cell_m)]
    }
}

/// The frame uploaded without a field: origin 0, inverse extent 1.
pub const IDENTITY_FRAME: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

/// The extra field in force: none yet.
pub fn field() -> Option<&'static ExtraField> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_yet_and_the_frame_inverts_the_extent() {
        assert!(field().is_none());
        let f = ExtraField { origin: [-100.0, 50.0], cell_m: 4.0, width: 25, height: 10, texels: vec![[0; 4]; 250] };
        assert_eq!(f.frame(), [-100.0, 50.0, 0.01, 0.025]);
        assert_eq!(IDENTITY_FRAME, [0.0, 0.0, 1.0, 1.0]);
    }
}
