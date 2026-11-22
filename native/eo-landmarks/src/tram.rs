//! The streetcar tracks of the Embarcadero / Ferry Building corridor: the
//! historic F Market & Wharves line's and the E line's rails along The
//! Embarcadero's median and down Market Street, drawn flush in the paving.
//! Nothing else in the native build draws a rail.
//!
//! Where: the `railWays` of public/data/sf/sf-city.json.gz (OpenStreetMap
//! `railway=tram|light_rail`, ODbL; see sf-city.NOTICE.txt), each way's
//! segments with an end within `CORRIDOR_M` of the Ferry Building's tower, on the scaled
//! ground plan (`groundscale`: the street plan they run in is scaled; the
//! gauge and the rail sections stay real). An OSM tram way is one track.
//!
//! What (cited):
//! * Track gauge 1435 mm, standard gauge, which the Muni streetcar and light
//!   rail network runs on (SFMTA; the UIC / AREMA standard gauge).
//! * The rails are girder groove rails set in the street (EN 14811:2006+A1:2009
//!   profile 59R2 / Ri59N, the common street-running tram rail): a running
//!   head 56 mm wide beside a 42 mm flangeway groove on its gauge side.
//! * Gauge is measured between the running heads' inner (gauge) faces.
//! The rails stand `lift` over the drawn paving (the street's own junction
//! paint lift, one step more), so they never fight its markings.

use crate::geometry::{Frozen, MeshArrays, Rgb};
use anyhow::Result;

/// Standard gauge (1435 mm) between the running heads' gauge faces, m.
pub const GAUGE_M: f64 = 1.435;
/// EN 14811 59R2 (Ri59N): the running head's width and the groove beside it, m.
pub const HEAD_M: f64 = 0.056;
pub const GROOVE_M: f64 = 0.042;
/// Track within this of the Ferry Building's tower (surveyed metres, before the plan scale).
pub const CORRIDOR_M: f64 = 1500.0;
/// The longest piece a rail is drawn in (it follows the paving's height between pieces), m.
pub const PIECE_M: f64 = 2.5;

/// The polished running head (iron's specular colour, linear; a metal's
/// albedo is its F0: Fe about 0.56, 0.57, 0.58) and the dirt-dark groove.
pub const HEAD_LINEAR: Rgb = [0.56, 0.57, 0.58];
pub const GROOVE_LINEAR: Rgb = [0.02, 0.019, 0.018];

/// The rail's finish (`features::rail_finish`; off: the whole head is bare
/// steel and the groove near black, as before). A street rail's head is
/// polished only where the wheels run: the worn band, about 20-30 mm wide on a
/// tread of 50-60 mm (the wheel-rail contact patch of a tram wheel is roughly
/// 10-15 mm long and a little wider; the band is the spread of its lateral
/// position, INFERRED from that). The rest of the head, its gauge corner and
/// the groove's keeper lip carry mill scale and rust (iron oxides: a dark
/// brown-grey dielectric, albedo about 0.1, rough), and the flangeway fills
/// with packed grit and brake dust (a dark matte, a little over the paving's
/// shade). Widths, m: the field shoulder and gauge corner either side of the
/// band, the keeper lip on the far side of the groove (INFERRED).
pub const FIELD_SHOULDER_M: f64 = 0.014;
pub const GAUGE_CORNER_M: f64 = 0.012;
pub const KEEPER_LIP_M: f64 = 0.012;
/// The worn band's steel (F0) and roughness: bright steel, not a mirror (the
/// wheels burnish it but grit scores it and a road film of dust and oil
/// covers part of it at sub-millimetre scale, so it reads grey-silver with a
/// blurred sheen, not the sky): roughness 0.4 and metalness 0.65 (the bare
/// share of the band), INFERRED.
pub const BAND_LINEAR: Rgb = [0.52, 0.52, 0.52];
pub const BAND_ROUGHNESS: f64 = 0.4;
pub const BAND_METALNESS: f64 = 0.65;
/// Mill scale and rust (Fe3O4 / Fe2O3 mixes over dust read brown-grey,
/// albedo about 0.15), linear, and its roughness.
pub const OXIDE_LINEAR: Rgb = [0.17, 0.135, 0.105];
pub const OXIDE_ROUGHNESS: f64 = 0.75;
/// The grit packed in the flangeway (road dust and brake fines, darker than the paving), linear.
pub const GRIT_LINEAR: Rgb = [0.075, 0.068, 0.06];

/// The rail's polish (`features::rail_polish`; off: the worn band whole at
/// `BAND_ROUGHNESS` / `BAND_METALNESS`, as before). The wheels burnish the
/// middle of the running band to bare steel, `POLISH_M` wide (the contact
/// patch's own width, INFERRED from the 10-15 mm patch of a tram wheel): a
/// metal (metalness 1) whose colour is iron's F0 (`HEAD_LINEAR`), smooth
/// enough at roughness 0.2 to carry a thin highlight of the sun, the sky or
/// a street light along the track, the line a street rail reads by
/// (INFERRED: rolled steel polished by traffic but scored by grit, between a
/// mirror at 0.05 and the band's 0.4); the band's edges either side stay as
/// the second pass drew them. Where
/// the rail meets the paving a joint of bituminous sealant 6 mm wide (the
/// rail-embedment joints of grooved street track are poured 5-10 mm wide,
/// INFERRED) outlines the rail on both sides: a black dielectric (bitumen,
/// albedo about 0.04), rough.
pub const POLISH_M: f64 = 0.014;
pub const POLISH_ROUGHNESS: f64 = 0.2;
pub const POLISH_METALNESS: f64 = 1.0;
pub const SEALANT_M: f64 = 0.006;
pub const SEALANT_LINEAR: Rgb = [0.032, 0.031, 0.03];

/// The OSM tram and light-rail ways' runs within the corridor, on the scaled plan.
pub fn load_corridor_ways(repo_root: &std::path::Path, centre_surveyed: [f64; 2]) -> Result<Vec<Vec<[f64; 2]>>> {
    use std::io::Read;
    #[derive(serde::Deserialize)]
    struct Way {
        #[serde(default)]
        railway: String,
        #[serde(default)]
        points: Vec<f64>,
    }
    #[derive(serde::Deserialize)]
    struct Rails {
        #[serde(rename = "railWays", default)]
        rail_ways: Vec<Way>,
    }
    let bytes = std::fs::read(repo_root.join("public/data/sf/sf-city.json.gz"))?;
    let mut s = String::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut s)?;
    let r: Rails = serde_json::from_str(&s)?;
    let k = eo_data::groundscale::scale();
    let mut out = Vec::new();
    for w in r.rail_ways {
        if !(w.railway == "tram" || w.railway == "light_rail") || w.points.len() < 4 {
            continue;
        }
        let pts: Vec<[f64; 2]> = w.points.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
        // the way's runs of segments with an end inside the corridor (a line
        // way runs on for kilometres: only the corridor's part is drawn).
        let inside = |p: &[f64; 2]| (p[0] - centre_surveyed[0]).hypot(p[1] - centre_surveyed[1]) <= CORRIDOR_M;
        let mut run: Vec<[f64; 2]> = Vec::new();
        for seg in pts.windows(2) {
            if inside(&seg[0]) || inside(&seg[1]) {
                if run.is_empty() {
                    run.push(seg[0]);
                }
                run.push(seg[1]);
            } else if run.len() >= 2 {
                out.push(std::mem::take(&mut run));
            } else {
                run.clear();
            }
        }
        if run.len() >= 2 {
            out.push(run);
        }
    }
    for line in &mut out {
        for p in line.iter_mut() {
            *p = [p[0] * k, p[1] * k];
        }
    }
    Ok(out)
}

/// A polyline resampled to pieces of at most `step`, with a unit normal per point (left of travel).
fn resample(line: &[[f64; 2]], step: f64) -> Vec<([f64; 2], [f64; 2])> {
    let mut pts: Vec<[f64; 2]> = Vec::new();
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        if l < 1e-6 {
            continue;
        }
        let n = (l / step).ceil().max(1.0) as usize;
        for i in 0..n {
            let t = i as f64 / n as f64;
            pts.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    if let Some(l) = line.last() {
        pts.push(*l);
    }
    let m = pts.len();
    (0..m)
        .filter_map(|i| {
            let (p, q) = (pts[i.saturating_sub(1)], pts[(i + 1).min(m - 1)]);
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let l = dx.hypot(dz);
            (l > 1e-9).then(|| (pts[i], [-dz / l, dx / l]))
        })
        .collect()
}

pub struct Tracks {
    /// The running heads (polished steel; with the finish, the worn band only).
    pub heads: Frozen,
    /// The flangeway grooves.
    pub grooves: Frozen,
    /// With the finish: the oxidised field shoulders, gauge corners and keeper lips (else empty).
    pub sides: Frozen,
    /// With the polish: the burnished strip down the middle of each band (else empty).
    pub polish: Frozen,
    /// The burnished strips and the sealant joints are drawn (`features::rail_polish`).
    pub polished: bool,
    pub tracks: usize,
    pub metres: f64,
}

/// Both rails of every way: a running head and its groove on the gauge side,
/// strips over the paving `surface_at` (x, z) gives, `lift` over it; with the
/// finish (`features::rail_finish`) the head's worn band, its oxidised edges
/// and the keeper lip, and a grit-filled groove.
pub fn build(ways: &[Vec<[f64; 2]>], surface_at: &dyn Fn(f64, f64) -> f64, lift: f64) -> Tracks {
    let finish = crate::features::rail_finish();
    build_polished(ways, surface_at, lift, finish, finish && crate::features::rail_polish())
}

pub fn build_with(ways: &[Vec<[f64; 2]>], surface_at: &dyn Fn(f64, f64) -> f64, lift: f64, finish: bool) -> Tracks {
    build_polished(ways, surface_at, lift, finish, false)
}

/// `build_with`, and with `polish` (the finish's second pass) the sealant joints either side of each rail.
pub fn build_polished(ways: &[Vec<[f64; 2]>], surface_at: &dyn Fn(f64, f64) -> f64, lift: f64, finish: bool, polish: bool) -> Tracks {
    let mut heads = MeshArrays::new("tram:heads");
    let mut grooves = MeshArrays::new("tram:grooves");
    let mut sides = MeshArrays::new("tram:sides");
    let mut polished = MeshArrays::new("tram:polish");
    let mut metres = 0.0;
    let half = GAUGE_M / 2.0;
    // lateral strips (from, to) off the centreline: each rail's head outside
    // the gauge face, its groove inside it.
    let (head_strips, groove_strips, side_strips): (Vec<(f64, f64)>, Vec<(f64, f64)>, Vec<(f64, f64)>) = if finish {
        let (f, g, k) = (FIELD_SHOULDER_M, GAUGE_CORNER_M, KEEPER_LIP_M);
        (
            vec![(-half - HEAD_M + f, -half - g), (half + g, half + HEAD_M - f)],
            vec![(-half, -half + GROOVE_M), (half - GROOVE_M, half)],
            vec![(-half - HEAD_M, -half - HEAD_M + f), (-half - g, -half), (-half + GROOVE_M, -half + GROOVE_M + k), (half - GROOVE_M - k, half - GROOVE_M), (half, half + g), (half + HEAD_M - f, half + HEAD_M)],
        )
    } else {
        (vec![(-half - HEAD_M, -half), (half, half + HEAD_M)], vec![(-half, -half + GROOVE_M), (half - GROOVE_M, half)], vec![])
    };
    let (head_col, groove_col) = if finish {
        (BAND_LINEAR, GRIT_LINEAR)
    } else {
        (HEAD_LINEAR, GROOVE_LINEAR)
    };
    // the sealant joints: outside each rail's field face and past its keeper lip.
    let seal_strips: Vec<(f64, f64)> = if polish {
        let (s, k) = (SEALANT_M, KEEPER_LIP_M);
        vec![(-half - HEAD_M - s, -half - HEAD_M), (-half + GROOVE_M + k, -half + GROOVE_M + k + s), (half - GROOVE_M - k - s, half - GROOVE_M - k), (half + HEAD_M, half + HEAD_M + s)]
    } else {
        vec![]
    };
    // with the polish each band splits round its burnished middle strip.
    let (head_strips, polish_strips): (Vec<(f64, f64)>, Vec<(f64, f64)>) = if polish {
        let mut band = Vec::new();
        let mut mid = Vec::new();
        for (a, b) in head_strips {
            let c = (a + b) / 2.0;
            band.extend([(a, c - POLISH_M / 2.0), (c + POLISH_M / 2.0, b)]);
            mid.push((c - POLISH_M / 2.0, c + POLISH_M / 2.0));
        }
        (band, mid)
    } else {
        (head_strips, vec![])
    };
    // each mesh's strips with their colours (the grooves' mesh carries the sealant joints too).
    let tagged = |v: &[(f64, f64)], c: Rgb| v.iter().map(|&(a, b)| (a, b, c)).collect::<Vec<_>>();
    let head_strips = tagged(&head_strips, head_col);
    let polish_strips = tagged(&polish_strips, HEAD_LINEAR);
    let mut groove_strips = tagged(&groove_strips, groove_col);
    groove_strips.extend(tagged(&seal_strips, SEALANT_LINEAR));
    let side_strips = tagged(&side_strips, OXIDE_LINEAR);
    for line in ways {
        let pts = resample(line, PIECE_M);
        for w in line.windows(2) {
            metres += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        }
        for pair in pts.windows(2) {
            let ((p0, n0), (p1, n1)) = (pair[0], pair[1]);
            let at = |p: [f64; 2], n: [f64; 2], o: f64| {
                let (x, z) = (p[0] + n[0] * o, p[1] + n[1] * o);
                [x, surface_at(x, z) + lift, z]
            };
            for (strips, m) in [(&head_strips, &mut heads), (&polish_strips, &mut polished), (&groove_strips, &mut grooves), (&side_strips, &mut sides)] {
                for &(o0, o1, col) in strips.iter() {
                    let (o0, o1) = (&o0, &o1);
                    let q = [at(p0, n0, *o0), at(p1, n1, *o0), at(p1, n1, *o1), at(p0, n0, *o1)];
                    // facing up whichever way the line runs
                    let e1 = [q[1][0] - q[0][0], q[1][2] - q[0][2]];
                    let e2 = [q[3][0] - q[0][0], q[3][2] - q[0][2]];
                    let up = e1[1] * e2[0] - e1[0] * e2[1] > 0.0;
                    if up { m.quad_n(q[0], q[1], q[2], q[3], col, [0.0, 1.0, 0.0]) } else { m.quad_n(q[3], q[2], q[1], q[0], col, [0.0, 1.0, 0.0]) }
                }
            }
        }
    }
    Tracks { heads: heads.freeze(), grooves: grooves.freeze(), sides: sides.freeze(), polish: polished.freeze(), polished: polish, tracks: ways.len(), metres }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_track_has_both_rails_at_gauge_and_faces_up() {
        let t = build_with(&[vec![[0.0, 0.0], [10.0, 0.0]]], &|_, _| 1.0, 0.02, false);
        assert_eq!(t.tracks, 1);
        assert!((t.metres - 10.0).abs() < 1e-9);
        // 4 pieces of 2.5 m, two heads each, two triangles a quad.
        assert_eq!(t.heads.triangles(), 4 * 2 * 2);
        let zs: Vec<f32> = t.heads.positions.chunks_exact(3).map(|p| p[2]).collect();
        let (lo, hi) = (zs.iter().cloned().fold(f32::MAX, f32::min), zs.iter().cloned().fold(f32::MIN, f32::max));
        assert!(((hi - lo) as f64 - (GAUGE_M + 2.0 * HEAD_M)).abs() < 1e-5);
        assert!(t.heads.normals.chunks_exact(3).all(|n| n[1] == 1.0));
        assert!(t.heads.positions.chunks_exact(3).all(|p| (p[1] - 1.02).abs() < 1e-6));
        // the winding faces up
        for tri in t.heads.indices.chunks_exact(3) {
            let p = |i: u32| [t.heads.positions[3 * i as usize] as f64, t.heads.positions[3 * i as usize + 2] as f64];
            let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
            let cross_y = (b[1] - a[1]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[1] - a[1]);
            assert!(cross_y > 0.0, "a rail triangle faces down");
        }
    }

    #[test]
    fn the_finish_splits_the_head_without_changing_the_rail() {
        let line = [vec![[0.0, 0.0], [10.0, 0.0]]];
        let (a, b) = (build_with(&line, &|_, _| 1.0, 0.02, false), build_with(&line, &|_, _| 1.0, 0.02, true));
        assert!(a.sides.is_empty() && !b.sides.is_empty());
        // the same outer edges: the head and keeper strips span the bare head and more, nothing past it.
        let span = |m: &[&Frozen]| {
            let zs: Vec<f32> = m.iter().flat_map(|f| f.positions.chunks_exact(3).map(|p| p[2])).collect();
            (zs.iter().cloned().fold(f32::MAX, f32::min), zs.iter().cloned().fold(f32::MIN, f32::max))
        };
        assert_eq!(span(&[&a.heads]), span(&[&b.heads, &b.sides]));
        // the bands sit inside the heads, the field shoulders outside them.
        let w = |f: &Frozen| f.positions.chunks_exact(3).map(|p| p[2]).fold(f32::MIN, f32::max) - f.positions.chunks_exact(3).map(|p| p[2]).fold(f32::MAX, f32::min);
        assert!(w(&b.heads) < w(&a.heads));
        assert!(((w(&a.heads) - w(&b.heads)) as f64 - 2.0 * FIELD_SHOULDER_M).abs() < 1e-5, "the band loses the field shoulders on the outside");
        assert!(b.sides.normals.chunks_exact(3).all(|n| n[1] == 1.0));
    }

    #[test]
    fn the_polish_adds_sealant_joints_outside_the_rail_only() {
        let line = [vec![[0.0, 0.0], [10.0, 0.0]]];
        let (a, b) = (build_polished(&line, &|_, _| 1.0, 0.02, true, false), build_polished(&line, &|_, _| 1.0, 0.02, true, true));
        assert!(b.polished && !a.polished && a.polish.is_empty());
        // the same sides; four joints more in the grooves' mesh, per piece.
        assert_eq!(a.sides.positions, b.sides.positions);
        assert_eq!(b.grooves.triangles(), a.grooves.triangles() * 3);
        // the band split round its burnished strip: the same span, a strip of POLISH_M each rail.
        let span = |m: &[&Frozen]| {
            let zs: Vec<f32> = m.iter().flat_map(|f| f.positions.chunks_exact(3).map(|p| p[2])).collect();
            (zs.iter().cloned().fold(f32::MAX, f32::min), zs.iter().cloned().fold(f32::MIN, f32::max))
        };
        assert_eq!(span(&[&a.heads]), span(&[&b.heads]));
        assert_eq!(b.polish.triangles() * 2, b.heads.triangles());
        let width: f64 = b.polish.positions.chunks_exact(3).map(|p| p[2] as f64).fold(f64::MIN, f64::max) - b.polish.positions.chunks_exact(3).map(|p| p[2] as f64).filter(|z| *z > 0.0).fold(f64::MAX, f64::min);
        assert!((width - POLISH_M).abs() < 1e-5, "strip {width}");
        // the joints reach SEALANT_M past the head's field face and no further.
        let z = |f: &Frozen| f.positions.chunks_exact(3).map(|p| p[2]).fold(f32::MIN, f32::max);
        assert!(((z(&b.grooves) - z(&a.heads).max(z(&a.sides))) as f64 - SEALANT_M).abs() < 1e-5);
        // the burnished strip is iron's F0.
        assert!(b.polish.colors.chunks_exact(3).all(|c| (c[0] as f64 - HEAD_LINEAR[0]).abs() < 1e-6));
    }
}
