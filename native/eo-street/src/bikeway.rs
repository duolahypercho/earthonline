//! (`--bikeways`, `StreetOptions::bikeways`) Each kerbside bike lane drawn as
//! the city's bike network says it is built (eo-data `bikeways`, DataSF
//! ygmz-vaxd, SFMTA): its class (II lane, IV separated), one way or two, its
//! barrier (a raised concrete island, safe-hit posts, or paint), whether it is
//! painted green, and the markings the California MUTCD asks of it:
//!
//!  * the green colored pavement inside the chromaticity box of 23 CFR 655
//!    (`caMetric('colored-pavement-green-chromaticity')`), only where the
//!    network records green paint;
//!  * the buffer bounded by solid white lines, with diagonal markings at the
//!    standard spacing where it is 2 ft or wider and no island or parking
//!    lane fills it (`caMetric('separated-bike-lane-buffer')`, Section 9E.07);
//!  * a raised island in the buffer where the barrier is concrete, at the
//!    kerb's height (`curbFaceHeight`);
//!  * tubular markers along the buffer where the barrier is posts,
//!    (`caMetric('tubular-marker')`, Section 3I.02: no further apart than N,
//!    here N / 2 as Section 3B.16 asks of markers along a channelizing line);
//!  * a broken yellow centre line in a two-way bikeway
//!    (`caMetric('shared-use-path-center-line')`, Section 9E.13);
//!  * the bicycle symbol with its arrow downstream of it after each
//!    intersection, for each direction the lane carries
//!    (`caMetric('bike-lane-symbol-size')`, Section 9E.07 para 11).

use anyhow::{Context, Result, anyhow};

/// The facility a run's kerbside bike lane is built as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BikeStyle {
    /// The network's segment (CNN).
    pub cnn: u32,
    /// 2 lane, 4 separated bikeway.
    pub class: u8,
    pub two_way: bool,
    pub green: bool,
    /// A raised concrete island fills the buffer.
    pub island: bool,
    /// Tubular markers stand in the buffer.
    pub posts: bool,
}

impl BikeStyle {
    /// From the network's record (eo-data `bikeways::Bikeway`).
    pub fn from_record(b: &eo_data::bikeways::Bikeway) -> Option<Self> {
        if !(b.class == 2 || b.class == 4) {
            return None;
        }
        let island = b.class == 4 && b.barrier & eo_data::bikeways::B_CONCRETE != 0;
        let posts = !island && b.barrier & eo_data::bikeways::B_POSTS != 0;
        Some(Self { cnn: b.cnn, class: b.class, two_way: b.two_way, green: b.green, island, posts })
    }
}

/// The dimensions the markings read from the standards table.
#[derive(Clone, Copy, Debug)]
pub struct BikeDims {
    pub symbol_len: f64,
    pub arrow_len: f64,
    pub symbol_gap: f64,
    pub line_width: f64,
    pub buffer_chevrons_from: f64,
    pub chevron_spacing: f64,
    pub centre_segment: f64,
    pub centre_gap: f64,
    pub post_height: f64,
    pub post_width: f64,
    /// The green's linear albedo (sRGB primaries).
    pub green: [f64; 3],
}

/// Green colored pavement's luminance factor as drawn: 1.5 x the weathered
/// asphalt the street draws (0.10-0.14 in eo-render, here 0.12): the median of
/// the green-to-asphalt luminance ratio measured in four SFMTA photographs of
/// SoMa bikeways (1.26-1.64). INFERRED (the CFR box bounds chromaticity only).
pub const GREEN_LUMINANCE: f64 = 0.18;

impl BikeDims {
    pub fn load(ca: &eo_data::CaStandards, o: &crate::StreetOptions) -> Result<Self> {
        let k = |id: &str, key: &str| ca.metric_key(id, key);
        let corners = ca.metric_value("colored-pavement-green-chromaticity")?.get("daytimeCorners").and_then(|v| v.as_array()).context("daytimeCorners")?;
        let poly: Vec<[f64; 2]> = corners.iter().filter_map(|c| Some([c.get(0)?.as_f64()?, c.get(1)?.as_f64()?])).collect();
        if poly.len() < 3 {
            return Err(anyhow!("green chromaticity box has {} corners", poly.len()));
        }
        Ok(Self {
            symbol_len: k("bike-lane-symbol-size", "bikeSymbolLength")?,
            arrow_len: k("bike-lane-symbol-size", "arrowLength")?,
            symbol_gap: k("bike-lane-symbol-size", "gapSymbolToArrow")?,
            line_width: (o.line_width_local[0] + o.line_width_local[1]) / 2.0,
            buffer_chevrons_from: k("separated-bike-lane-buffer", "chevronsRequiredFrom")?,
            chevron_spacing: k("separated-bike-lane-buffer", "chevronSpacingMin")?,
            centre_segment: k("shared-use-path-center-line", "segment")?,
            centre_gap: k("shared-use-path-center-line", "gap")?,
            post_height: k("tubular-marker", "minHeight")?,
            post_width: k("tubular-marker", "minWidth")?,
            green: green_in_box(&poly, GREEN_LUMINANCE),
        })
    }
}

fn inside(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
    let mut c = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// Linear sRGB of chromaticity (x, y) at luminance `y_lum`.
pub fn xy_to_linear_srgb(x: f64, y: f64, y_lum: f64) -> [f64; 3] {
    let (cx, cz) = (x * y_lum / y, (1.0 - x - y) * y_lum / y);
    [
        3.2404542 * cx - 1.5371385 * y_lum - 0.4985314 * cz,
        -0.9692660 * cx + 1.8760108 * y_lum + 0.0415560 * cz,
        0.0556434 * cx - 0.2040259 * y_lum + 1.0572252 * cz,
    ]
}

/// The colour inside the green box nearest its centroid that a display can
/// show (sRGB gamut: most of the box lies outside it), at luminance `y_lum`.
pub fn green_in_box(poly: &[[f64; 2]], y_lum: f64) -> [f64; 3] {
    let n = poly.len() as f64;
    let c = [poly.iter().map(|p| p[0]).sum::<f64>() / n, poly.iter().map(|p| p[1]).sum::<f64>() / n];
    let (x0, x1) = poly.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[0]), a.1.max(p[0])));
    let (y0, y1) = poly.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
    let mut best: Option<(f64, [f64; 3])> = None;
    let steps = 300;
    for i in 0..=steps {
        for j in 0..=steps {
            let p = [x0 + (x1 - x0) * i as f64 / steps as f64, y0 + (y1 - y0) * j as f64 / steps as f64];
            if !inside(p, poly) {
                continue;
            }
            let rgb = xy_to_linear_srgb(p[0], p[1], y_lum);
            if rgb.iter().any(|v| *v < 0.0 || *v > 1.0) {
                continue;
            }
            let d = (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, rgb));
            }
        }
    }
    best.map(|b| b.1).unwrap_or([0.0, y_lum, 0.0])
}

/// Linear albedo as the street mesh's colour bytes (the renderer reads them as albedo).
pub fn albedo_bytes(c: [f64; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// A marking shape as triangles in its own frame: (along, across) metres,
/// along the direction of travel from the shape's start, across to its right.
pub type Shape = Vec<[[f64; 2]; 3]>;

fn stroke(out: &mut Shape, a: [f64; 2], b: [f64; 2], w: f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = (dx * dx + dy * dy).sqrt().max(1e-9);
    let (nx, ny) = (-dy / l * w / 2.0, dx / l * w / 2.0);
    let (p0, p1, p2, p3) = ([a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]);
    out.push([p0, p1, p2]);
    out.push([p0, p2, p3]);
}

fn ring(out: &mut Shape, c: [f64; 2], r: f64, w: f64) {
    let n = 18;
    for k in 0..n {
        let (t0, t1) = (k as f64 / n as f64 * std::f64::consts::TAU, (k + 1) as f64 / n as f64 * std::f64::consts::TAU);
        let (ri, ro) = (r - w / 2.0, r + w / 2.0);
        let p = |rr: f64, t: f64| [c[0] + rr * t.cos(), c[1] + rr * t.sin()];
        out.push([p(ri, t0), p(ro, t0), p(ro, t1)]);
        out.push([p(ri, t0), p(ro, t1), p(ri, t1)]);
    }
}

/// The bicycle symbol (CA MUTCD Figure 9E-1), `len` long along the travel:
/// two wheels and the frame seen from the side, the bike's top toward the
/// lane's left. Proportions INFERRED from the figure's shape; its length is
/// the table's.
pub fn bicycle(len: f64) -> Shape {
    let mut s = vec![];
    let w = 0.045 * len;
    let r = 0.17 * len;
    let (rear, front) = ([0.2 * len, 0.0], [0.8 * len, 0.0]);
    ring(&mut s, rear, r, w);
    ring(&mut s, front, r, w);
    let crank = [0.47 * len, 0.0];
    let seat = [0.40 * len, -0.27 * len];
    let head = [0.70 * len, -0.29 * len];
    for (a, b) in [(rear, crank), (rear, seat), (seat, crank), (seat, head), (crank, head), (head, front)] {
        stroke(&mut s, a, b, w);
    }
    stroke(&mut s, [0.33 * len, -0.30 * len], [0.47 * len, -0.30 * len], w); // saddle
    stroke(&mut s, head, [0.66 * len, -0.37 * len], w); // stem
    stroke(&mut s, [0.60 * len, -0.37 * len], [0.74 * len, -0.37 * len], w); // bars
    s
}

/// A straight arrow `len` long pointing along the travel (CA MUTCD Figure
/// 9E-1's lane arrow). Shaft and head proportions INFERRED.
pub fn arrow(len: f64) -> Shape {
    let (shaft_w, head_w, head_l) = (0.1 * len, 0.36 * len, 0.38 * len);
    let b = len - head_l;
    vec![
        [[0.0, -shaft_w / 2.0], [b, -shaft_w / 2.0], [b, shaft_w / 2.0]],
        [[0.0, -shaft_w / 2.0], [b, shaft_w / 2.0], [0.0, shaft_w / 2.0]],
        [[b, -head_w / 2.0], [len, 0.0], [b, head_w / 2.0]],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_green_is_inside_the_cfr_box_and_displayable() {
        let poly = [[0.026, 0.399], [0.166, 0.364], [0.286, 0.446], [0.207, 0.771]];
        let g = green_in_box(&poly, GREEN_LUMINANCE);
        assert!(g.iter().all(|v| (0.0..=1.0).contains(v)), "{g:?}");
        let lum = 0.2126 * g[0] + 0.7152 * g[1] + 0.0722 * g[2];
        assert!((lum - GREEN_LUMINANCE).abs() < 0.002, "{lum}");
        assert!(g[1] > 2.0 * g[0] && g[1] > 2.0 * g[2], "green dominates: {g:?}");
    }

    #[test]
    fn shapes_stay_in_their_length() {
        for (sh, len) in [(bicycle(1.829), 1.829), (arrow(1.829), 1.829)] {
            for t in &sh {
                for p in t {
                    assert!(p[0] >= -0.2 * len && p[0] <= 1.2 * len && p[1].abs() <= 0.6 * len, "{p:?}");
                }
            }
        }
    }
}
