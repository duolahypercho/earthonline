//! Moored boats (eo-app `--no-boats` leaves them out): the bodies
//! `scripts/assets/build-boats-v1.py` builds (`public/assets/boats`, each
//! file's digest in the provenance beside it) and which of them lies in each
//! berth of the harbours' floating docks (eo-landmarks `floating_docks`).
//!
//! A body is drawn at its true size (as the cars are), its origin on the
//! design waterline at the middle of the waterline, bow +Z, so a berth's boat
//! stands at the still sea level. The plan is wider than life
//! (`eo_data::groundscale`), so a body that fits the real berth fits the drawn one.

use crate::data::StreetProp;
use anyhow::{Context, Result, anyhow};
use eo_landmarks::floating_docks::{Berth, SIDE_TIE_GAP_M};
use std::path::Path;

/// The bodies' provenance (the digests every file is checked against).
pub const PROVENANCE: &str = "public/assets/boats/boats-v1.provenance.json";

/// The bodies in render order (`eo_render::props_lib2::BOAT_BASE + i`).
pub const BODIES: [&str; 8] = ["sailYacht-a", "sailYacht-b", "motorYacht-a", "motorYacht-b", "fishingBoat-a", "fishingBoat-b", "jetSki-a", "jetSki-b"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Sail,
    Motor,
    Fishing,
    Jet,
}

impl Kind {
    fn of(name: &str) -> Kind {
        match name.split('-').next().unwrap_or("") {
            "sailYacht" => Kind::Sail,
            "motorYacht" => Kind::Motor,
            "fishingBoat" => Kind::Fishing,
            _ => Kind::Jet,
        }
    }
}

/// One body: its three levels (lod0 full, lod1 reduced, lod2 silhouette) and the
/// dimensions the berths are fitted by (the provenance's `dimensionsM`, metres).
#[derive(Clone, Debug)]
pub struct Body {
    pub name: &'static str,
    pub kind: Kind,
    pub lods: [StreetProp; 3],
    /// Overall length (bow to stern, every fitting).
    pub loa: f64,
    /// The hull's beam with its rub rails (fenders, tyres and poles left out).
    pub beam: f64,
    /// Highest point above the waterline.
    pub height: f64,
    pub draft: f64,
}

/// The bodies in [`BODIES`] order, from the provenance (the files are not read here).
pub fn load_bodies(repo: &Path) -> Result<Vec<Body>> {
    let path = repo.join(PROVENANCE);
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let dir = Path::new(PROVENANCE).parent().unwrap_or(Path::new(""));
    BODIES
        .iter()
        .map(|&name| {
            let b = &v["bodies"][name];
            let lods = b["lods"].as_array().filter(|l| l.len() == 3).ok_or_else(|| anyhow!("{PROVENANCE}: {name} has no three lods"))?;
            let lod = |k: usize| -> Result<StreetProp> {
                let l = &lods[k];
                let file = l["file"].as_str().ok_or_else(|| anyhow!("{name} lod{k}: no file"))?;
                Ok(StreetProp {
                    path: dir.join(file).to_string_lossy().replace('\\', "/"),
                    sha256: l["sha256"].as_str().ok_or_else(|| anyhow!("{name} lod{k}: no sha256"))?.to_string(),
                    bytes: l["bytes"].as_u64().ok_or_else(|| anyhow!("{name} lod{k}: no bytes"))?,
                    triangles: l["triangles"].as_u64(),
                    dimensions_m: None,
                })
            };
            let d = &b["dimensionsM"];
            let num = |k: &str| d[k].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: {name} has no dimensionsM.{k}"));
            Ok(Body { name, kind: Kind::of(name), lods: [lod(0)?, lod(1)?, lod(2)?], loa: num("loaOverall")?, beam: num("beamHull")?, height: num("heightAboveWaterline")?, draft: num("draft")? })
        })
        .collect()
}

/// The share of berths with a boat in them. INFERRED (the city's harbours keep waiting lists).
pub const OCCUPANCY: f64 = 0.88;
/// The water a finger berth's boat keeps off the main walk at its bow. Metres. INFERRED.
pub const BOW_GAP_M: f64 = 0.6;
/// How much wider than its berth's room a hull may be (the fenders give). Metres. INFERRED.
pub const BEAM_SLACK_M: f64 = 0.25;
/// Hyde Street Harbor, the Port of San Francisco's commercial fishing harbour
/// at Fisherman's Wharf (lat, lon, and its reach in metres): its berths hold
/// the fishing fleet. Elsewhere the harbours are recreational marinas.
pub const FISHING_HARBOUR: (f64, f64, f64) = (37.8098, -122.4205, 350.0);

/// The mix of kinds a berth draws from (weights; only kinds that fit are drawn).
fn mix(fishing: bool) -> [(Kind, f64); 4] {
    if fishing {
        [(Kind::Fishing, 0.80), (Kind::Motor, 0.12), (Kind::Sail, 0.08), (Kind::Jet, 0.0)]
    } else {
        [(Kind::Sail, 0.50), (Kind::Motor, 0.36), (Kind::Fishing, 0.06), (Kind::Jet, 0.08)]
    }
}

/// A boat at its berth: the body (index into [`BODIES`]), the middle of its
/// waterline on the plan (metres, game frame) and its bow's heading (radians,
/// from +x toward +z, as `Berth::heading`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moored {
    pub body: usize,
    pub pos: [f64; 2],
    pub heading: f64,
}

fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A uniform 0..1 from the berth's place (decimetres) and a stream number.
fn roll(b: &Berth, stream: u64) -> f64 {
    let (x, z) = ((b.centre[0] * 10.0).round() as i64 as u64, (b.centre[1] * 10.0).round() as i64 as u64);
    (mix64(mix64(x ^ mix64(z)) ^ stream) >> 11) as f64 / (1u64 << 53) as f64
}

/// Does the body lie in the berth? A finger berth holds it bow in, short of
/// the walk; a side tie may run into the gap to the next one, less a metre.
fn fits(body: &Body, b: &Berth) -> bool {
    let room = if b.finger { b.length - BOW_GAP_M } else { b.length + SIDE_TIE_GAP_M - 1.0 };
    body.loa <= room && body.beam <= b.beam + BEAM_SLACK_M
}

/// Which boat lies in each berth (none in `1 - OCCUPANCY` of them, or where
/// no body fits): a kind from the harbour's mix among those that fit, then one
/// of its bodies, each by a roll on the berth's place (the same every run).
/// A finger berth's boat lies bow in, `BOW_GAP_M` off the walk; a side-tied
/// one at the middle of its berth.
pub fn moor(berths: &[Berth], bodies: &[Body]) -> Vec<Moored> {
    let f = eo_data::Frame::sf();
    let s = eo_data::groundscale::scale();
    let (lat, lon, reach) = FISHING_HARBOUR;
    let harbour = [(lon - f.center_lon) * f.m_per_deg_lon * s, -(lat - f.center_lat) * f.m_per_deg_lat * s];
    let mut out = Vec::new();
    for b in berths {
        if roll(b, 1) >= OCCUPANCY {
            continue;
        }
        let fishing = (b.centre[0] - harbour[0]).hypot(b.centre[1] - harbour[1]) <= reach * s;
        let kinds: Vec<(Kind, f64)> = mix(fishing).into_iter().filter(|&(k, w)| w > 0.0 && bodies.iter().any(|x| x.kind == k && fits(x, b))).collect();
        let total: f64 = kinds.iter().map(|k| k.1).sum();
        if total <= 0.0 {
            continue;
        }
        let mut r = roll(b, 2) * total;
        let kind = kinds.iter().find(|k| {
            r -= k.1;
            r < 0.0
        });
        let kind = kind.unwrap_or(&kinds[kinds.len() - 1]).0;
        let fit: Vec<usize> = (0..bodies.len()).filter(|&i| bodies[i].kind == kind && fits(&bodies[i], b)).collect();
        let body = fit[((roll(b, 3) * fit.len() as f64) as usize).min(fit.len() - 1)];
        let along = if b.finger { b.length / 2.0 - BOW_GAP_M - bodies[body].loa / 2.0 } else { 0.0 };
        let u = [b.heading.cos(), b.heading.sin()];
        out.push(Moored { body, pos: [b.centre[0] + u[0] * along, b.centre[1] + u[1] * along], heading: b.heading });
    }
    out
}
