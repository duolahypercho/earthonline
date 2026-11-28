//! The classified colour transfer of the aerial macro map: each 2 m texel's
//! dehazed photo colour is read as a soft mix of six ground classes (from
//! its NDVI, lightness and chroma), and redrawn as the same mix of the
//! renderer's own class colours, keeping the photo's within-class pattern
//! (lightness, and a share of its hue). The photo's true colour does not
//! match the eye's on these surfaces (dry grass reads pinkish-brown, a lawn
//! as bright as dry grass); its classes do.

use serde::{Deserialize, Serialize};

pub const LAWN: usize = 0;
pub const DRY: usize = 1;
pub const SOIL: usize = 2;
pub const PAVING: usize = 3;
pub const SCRUB: usize = 4;
pub const CANOPY: usize = 5;
pub const N: usize = 6;
pub const NAMES: [&str; N] = ["lawn", "dry", "soil", "paving", "scrub", "canopy"];

/// The rules' thresholds (each pair the edges of a smoothstep) and the
/// transfer's constants.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Vigorous vegetation (lawn, scrub, canopy): smoothstep of NDVI.
    pub veg_ndvi: [f32; 2],
    /// Bare of live vegetation (dry grass, soil, paving): 1 - smoothstep of NDVI.
    pub bare_ndvi: [f32; 2],
    /// Lawn against woody (scrub, canopy): smoothstep of the photo's luminance.
    pub lawn_lum: [f32; 2],
    /// In a wild mapped green (park, grassland, scrub, wood), lawn fades to
    /// woody with slope (1 - smoothstep): irrigated lawns are not on wild
    /// slopes, sunlit scrub is.
    pub wild_lawn_slope: [f32; 2],
    /// Canopy against scrub within woody: 1 - smoothstep of luminance (canopy darker).
    pub canopy_lum: [f32; 2],
    /// Inside a mapped wood or forest, woody is at least this much canopy.
    pub wood_canopy: f32,
    /// Dry grass is warm: smoothstep of min(r/g, g/b).
    pub warm: [f32; 2],
    /// Dry grass is not very dark: smoothstep of luminance.
    pub dry_lum: [f32; 2],
    /// Soil by colour: smoothstep of r/g. In this imagery cured grass is the
    /// reddest open ground (r/g median 1.25, trails 1.12), so only an extreme
    /// red counts; soil comes mostly from the mapped trails and bare ground.
    pub soil_red: [f32; 2],
    /// On a mapped trail or bare ground, bare texels are at least this much soil.
    pub trail_soil: f32,
    /// Paving is neutral: 1 - smoothstep of max / min channel.
    pub neutral: [f32; 2],
    /// Inside a mapped green away from any way, neutrality counts this much
    /// (greyish cured grass on a park's slope is not paving).
    pub green_neutral: f32,
    /// Paving's weight floor: what nothing else claims (deep shadow, water) keeps its own colour.
    pub floor: f32,
    /// pattern = (L / class median L)^exp, clamped.
    pub pattern_exp: f32,
    pub pattern_clamp: [f32; 2],
    /// The share of the photo's chroma deviation from its class's median kept.
    pub chroma_share: f32,
    /// Paving: the share of its own chroma kept (1 - the desaturation).
    pub paving_chroma: f32,
    /// Soil on slopes past this (a smoothstep) takes the eroded face colour.
    pub eroded_slope: [f32; 2],
}

impl Default for Params {
    fn default() -> Params {
        Params {
            veg_ndvi: [0.12, 0.25],
            bare_ndvi: [0.05, 0.15],
            lawn_lum: [0.05, 0.09],
            wild_lawn_slope: [0.2, 0.35],
            canopy_lum: [0.025, 0.045],
            wood_canopy: 0.7,
            warm: [0.95, 1.03],
            dry_lum: [0.03, 0.06],
            soil_red: [1.65, 1.85],
            trail_soil: 0.7,
            neutral: [1.12, 1.3],
            green_neutral: 0.35,
            floor: 0.02,
            pattern_exp: 0.7,
            pattern_clamp: [0.55, 1.6],
            chroma_share: 0.15,
            paving_chroma: 0.15,
            eroded_slope: [0.4, 0.6],
        }
    }
}

/// What the classes are read from, per texel.
#[derive(Clone, Copy, Debug)]
pub struct Features {
    /// Dehazed photo colour, linear.
    pub c: [f32; 3],
    /// NDVI (NaN: no colour-infrared data; read as neither vigorous nor bare).
    pub ndvi: f32,
    /// Inside a mapped wood or forest.
    pub wood: bool,
    /// On a mapped trail or bare ground.
    pub trail: bool,
    /// Inside a mapped green (park, grassland, scrub, wood, golf course, lawn), away from any way.
    pub green: bool,
    /// Inside a wild mapped green (park, grassland, scrub, wood).
    pub wild: bool,
    pub slope: f32,
}

pub fn smoothstep(e: [f32; 2], x: f32) -> f32 {
    let t = ((x - e[0]) / (e[1] - e[0])).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn luminance(c: [f32; 3]) -> f32 {
    crate::macro_map::luminance(c)
}

/// The class weights of a texel, summing to 1.
pub fn weights(f: &Features, p: &Params) -> [f32; N] {
    let [r, g, b] = f.c.map(|v| v.max(1e-5));
    let l = luminance(f.c);
    let (veg, bare) = if f.ndvi.is_nan() { (0.5, 0.5) } else { (smoothstep(p.veg_ndvi, f.ndvi), 1.0 - smoothstep(p.bare_ndvi, f.ndvi)) };
    let lit = smoothstep(p.lawn_lum, l) * if f.wild { 1.0 - smoothstep(p.wild_lawn_slope, f.slope) } else { 1.0 };
    let woody = veg * (1.0 - lit);
    let canopy_share = (1.0 - smoothstep(p.canopy_lum, l)).max(if f.wood { p.wood_canopy } else { 0.0 });
    let neutral = (1.0 - smoothstep(p.neutral, r.max(g).max(b) / r.min(g).min(b))) * if f.green { p.green_neutral } else { 1.0 };
    let red = smoothstep(p.soil_red, r / g).max(if f.trail { p.trail_soil } else { 0.0 });
    let warm = smoothstep(p.warm, (r / g).min(g / b));
    let mut w = [0f32; N];
    w[LAWN] = veg * lit;
    w[SCRUB] = woody * (1.0 - canopy_share);
    w[CANOPY] = woody * canopy_share;
    w[SOIL] = bare * (1.0 - neutral) * red;
    w[DRY] = bare * (1.0 - neutral) * (1.0 - red) * warm * smoothstep(p.dry_lum, l);
    w[PAVING] = bare * neutral + p.floor;
    let s: f32 = w.iter().sum();
    w.map(|v| v / s)
}

/// The class with the largest weight.
pub fn winner(w: &[f32; N]) -> usize {
    (0..N).max_by(|&a, &b| w[a].total_cmp(&w[b])).unwrap()
}

/// Each class's reference in the photo: the median luminance of the texels
/// it wins, and their median chromaticity (colour over luminance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClassRefs {
    pub median_lum: [f32; N],
    pub chroma: [[f32; 3]; N],
}

/// The class colours: lawn, dry, soil (surface, eroded face), paving (unused:
/// its own colour), scrub, canopy, all linear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub lawn: [f32; 3],
    pub dry: [f32; 3],
    pub soil: [f32; 3],
    pub soil_eroded: [f32; 3],
    pub scrub: [f32; 3],
    pub canopy: [f32; 3],
}

impl Palette {
    /// Class `k`'s colour on a slope.
    pub fn colour(&self, k: usize, slope: f32, p: &Params) -> [f32; 3] {
        match k {
            LAWN => self.lawn,
            DRY => self.dry,
            SOIL => {
                let t = smoothstep(p.eroded_slope, slope);
                [0, 1, 2].map(|i| self.soil[i] * (1.0 - t) + self.soil_eroded[i] * t)
            }
            SCRUB => self.scrub,
            CANOPY => self.canopy,
            _ => [0.0; 3],
        }
    }
}

/// The texel redrawn: sum over the classes of weight x the class colour x
/// the photo's pattern ((L / class median L)^exp, clamped), each class colour
/// carrying `chroma_share` of the photo's chromaticity off its class median
/// (at the class colour's luminance); paving is the photo's own colour,
/// desaturated toward grey at its luminance, times `paving_gain`.
pub fn transfer(f: &Features, w: &[f32; N], pal: &Palette, refs: &ClassRefs, p: &Params, paving_gain: f32) -> [f32; 3] {
    let l = luminance(f.c).max(1e-5);
    let q = f.c.map(|v| v / l);
    let mut out = [0f32; 3];
    for k in 0..N {
        if w[k] <= 0.0 {
            continue;
        }
        let o = if k == PAVING {
            f.c.map(|v| paving_gain * (l + p.paving_chroma * (v - l)))
        } else {
            let pat = (l / refs.median_lum[k].max(1e-5)).powf(p.pattern_exp).clamp(p.pattern_clamp[0], p.pattern_clamp[1]);
            let base = pal.colour(k, f.slope, p);
            let lb = luminance(base);
            [0, 1, 2].map(|i| pat * (base[i] + p.chroma_share * lb * (q[i] - refs.chroma[k][i])))
        };
        for i in 0..3 {
            out[i] += w[k] * o[i];
        }
    }
    out.map(|v| v.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feat(c: [f32; 3], ndvi: f32) -> Features {
        Features { c, ndvi, wood: false, trail: false, green: false, wild: false, slope: 0.2 }
    }

    #[test]
    fn classes_from_ndvi_lightness_and_chroma() {
        let p = Params::default();
        let cases = [
            (feat([0.105, 0.139, 0.041], 0.4), LAWN),  // a lawn: vigorous, light
            (feat([0.040, 0.050, 0.020], 0.4), SCRUB), // vigorous, darker
            (feat([0.015, 0.018, 0.004], 0.4), CANOPY), // vigorous, darkest
            (feat([0.158, 0.123, 0.088], 0.0), DRY),   // cured grass: bare, warm
            (feat([0.300, 0.160, 0.100], 0.0), SOIL),  // bare, extremely red
            (feat([0.200, 0.130, 0.090], 0.0), DRY),   // bare, as red as cured grass is
            (feat([0.236, 0.242, 0.265], 0.0), PAVING), // bare, neutral
            (feat([0.010, 0.030, 0.060], -0.3), PAVING), // water: nothing else claims it
        ];
        for (f, want) in cases {
            let w = weights(&f, &p);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
            assert_eq!(NAMES[winner(&w)], NAMES[want], "{f:?}: {w:?}");
        }
        // A mapped wood makes woody canopy; a mapped trail makes bare ground soil.
        let mut f = feat([0.040, 0.050, 0.020], 0.4);
        f.wood = true;
        assert_eq!(winner(&weights(&f, &p)), CANOPY);
        let mut f = feat([0.158, 0.123, 0.088], 0.0);
        f.trail = true;
        assert_eq!(winner(&weights(&f, &p)), SOIL);
        // Greyish cured grass: paving in the street, dry grass in a park.
        let mut f = feat([0.150, 0.140, 0.132], 0.0);
        assert_eq!(winner(&weights(&f, &p)), PAVING);
        f.green = true;
        assert_eq!(winner(&weights(&f, &p)), DRY);
        // Sunlit green on a wild slope is scrub, on a lawn's flat ground lawn.
        let mut f = feat([0.105, 0.139, 0.041], 0.4);
        f.wild = true;
        f.slope = 0.5;
        assert_eq!(winner(&weights(&f, &p)), SCRUB);
        f.slope = 0.05;
        assert_eq!(winner(&weights(&f, &p)), LAWN);
    }

    #[test]
    fn transfer_keeps_pattern_and_some_hue() {
        let p = Params::default();
        let pal = Palette { lawn: [0.05, 0.09, 0.03], dry: [0.26, 0.24, 0.14], soil: [0.3, 0.17, 0.1], soil_eroded: [0.2, 0.1, 0.06], scrub: [0.045, 0.058, 0.025], canopy: [0.021, 0.037, 0.016] };
        let dry = [0.158, 0.123, 0.088];
        let l = luminance(dry);
        let refs = ClassRefs { median_lum: [0.12, l, 0.15, 0.24, 0.05, 0.03], chroma: [[1.0; 3], dry.map(|v| v / l), [1.0; 3], [1.0; 3], [1.0; 3], [1.0; 3]] };
        let only = |k: usize| {
            let mut w = [0f32; N];
            w[k] = 1.0;
            w
        };
        // The class median itself becomes the class colour exactly.
        let o = transfer(&feat(dry, 0.0), &only(DRY), &pal, &refs, &p, 1.0);
        assert!((0..3).all(|i| (o[i] - pal.dry[i]).abs() < 1e-5), "{o:?}");
        // Half as light again: brighter by 1.5^0.7; a texel 3x as light hits the clamp.
        let o2 = transfer(&feat(dry.map(|v| v * 1.5), 0.0), &only(DRY), &pal, &refs, &p, 1.0);
        assert!((luminance(o2) / luminance(pal.dry) - 1.5f32.powf(0.7)).abs() < 1e-3);
        let o3 = transfer(&feat(dry.map(|v| v * 3.0), 0.0), &only(DRY), &pal, &refs, &p, 1.0);
        assert!((luminance(o3) / luminance(pal.dry) - 1.6).abs() < 1e-3);
        // A redder texel than the median keeps 15 % of the difference in hue.
        let red = [dry[0] * 1.2, dry[1], dry[2]];
        let o4 = transfer(&feat(red, 0.0), &only(DRY), &pal, &refs, &p, 1.0);
        assert!(o4[0] / o4[1] > pal.dry[0] / pal.dry[1] && o4[0] / o4[1] < red[0] / red[1]);
        // Paving: its own luminance (times the gain), 85 % toward grey.
        let c = [0.2, 0.25, 0.3];
        let o5 = transfer(&feat(c, 0.0), &only(PAVING), &pal, &refs, &p, 1.1);
        let lc = luminance(c);
        assert!((luminance(o5) - 1.1 * lc).abs() < 1e-5);
        assert!(((o5[2] - o5[0]) - 1.1 * 0.15 * (c[2] - c[0])).abs() < 1e-5);
        // Soil on a steep face takes the eroded colour.
        assert_eq!(pal.colour(SOIL, 0.9, &p), pal.soil_eroded);
        assert_eq!(pal.colour(SOIL, 0.1, &p), pal.soil);
    }
}
