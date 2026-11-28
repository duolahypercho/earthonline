//! The SFMTA sign plates as Blender models (eo-app `--no-sign-models` keeps
//! the plates built in code): `scripts/assets/build-street-signs-v1.py`
//! models each inventory face ([`crate::Model::SignPlate`]'s `variant`) from
//! the official sign art (the sign art manifest's FHWA layouts and Caltrans
//! sheets): an aluminium blank cut to the art's outline, the weathered
//! sheeting, bolts, saddle brackets and post bands, at three levels of
//! detail. `public/assets/street-signs/<face>-v1*.glb`; every file's digest is
//! in the provenance beside them and checked when the renderer reads it.
//!
//! Frame (the provenance's note): glTF Y up, the face toward +Z, the post
//! axis at x = z = 0, the face's plate frame (`frameM`: the inventory's most
//! common plate size for the face) from y = 0 to `frameM[1]`, the art fitted
//! into it as the game's sign atlas fits it. The renderer scales the frame to
//! each plate's own inventory size, as it scales the plate built in code.
//!
//! Faces the manifest draws by hand (no published layout: tow-away, the
//! regulatory and guide catch-alls, angle parking) have no model and keep the
//! plate built in code.

use crate::data::StreetProp;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::path::Path;

/// The provenance (the digests every file is checked against).
pub const PROVENANCE: &str = "public/assets/street-signs/street-signs-v1.provenance.json";

/// The wear variants a face is built in, in the provenance's order: the plate as first
/// modelled, one rusted (rust bled from its steel hardware, edges corroded), one battered
/// (creased, knocked crooked, peeled, defaced). A face may carry fewer: the first ones.
pub const WEAR: [&str; 3] = ["aged", "rusty", "battered"];

/// One wear variant of a face: its three levels (the plates with their hardware, the
/// plates, the plates' simplified outlines; variants after the first share the first's far level).
#[derive(Clone, Debug)]
pub struct SignVariant {
    pub name: String,
    pub lods: [StreetProp; 3],
}

/// One inventory face's model.
#[derive(Clone, Debug)]
pub struct SignModel {
    pub face: String,
    /// Index into the inventory's faces (`JsProps::signs.faces`): the SignPlate `variant`.
    pub face_index: u16,
    /// The plates with their hardware, the plates, the plates' simplified outlines (the first variant's).
    pub lods: [StreetProp; 3],
    /// Every wear variant, `WEAR` order, the first `lods`.
    pub variants: Vec<SignVariant>,
    /// The plate frame (width, height, metres) the model is built in.
    pub frame_m: [f64; 2],
}

pub struct SignModels {
    pub faces: Vec<SignModel>,
    /// The sign post the brackets are built round (`signPostRadiusM`).
    pub post_radius_m: f64,
}

/// Read the provenance (the files are not read here). `inventory_faces`:
/// `JsProps::signs.faces`, against which every face's index is checked.
pub fn load(repo: &Path, inventory_faces: &[String]) -> Result<SignModels> {
    let path = repo.join(PROVENANCE);
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let v: Value = serde_json::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let post_radius_m = v["postRadiusM"].as_f64().ok_or_else(|| anyhow!("{PROVENANCE}: no postRadiusM"))?;
    let faces_v = v["faces"].as_object().ok_or_else(|| anyhow!("{PROVENANCE}: no faces"))?;
    let mut faces = vec![];
    for (face, f) in faces_v {
        let index = f["faceIndex"].as_u64().ok_or_else(|| anyhow!("{PROVENANCE}: {face} has no faceIndex"))? as usize;
        if inventory_faces.get(index).map(String::as_str) != Some(face.as_str()) {
            bail!("{PROVENANCE}: {face} is face {index}, the inventory has {:?} there", inventory_faces.get(index));
        }
        let lods = three_lods(face, &f["lods"])?;
        let mut variants = vec![SignVariant { name: WEAR[0].to_string(), lods: lods.clone() }];
        for (k, v) in f["variants"].as_array().into_iter().flatten().enumerate() {
            let name = v["name"].as_str().unwrap_or_default();
            if WEAR.get(k + 1) != Some(&name) {
                bail!("{PROVENANCE}: {face} variant {k} is {name:?}, expected {:?}", WEAR.get(k + 1));
            }
            variants.push(SignVariant { name: name.to_string(), lods: three_lods(face, &v["lods"])? });
        }
        let fm = f["frameM"].as_array().filter(|a| a.len() == 2).ok_or_else(|| anyhow!("{PROVENANCE}: {face} has no frameM"))?;
        let frame_m = [fm[0].as_f64().unwrap_or(0.0), fm[1].as_f64().unwrap_or(0.0)];
        if !(frame_m[0] > 0.0 && frame_m[1] > 0.0) {
            bail!("{PROVENANCE}: {face} frameM {frame_m:?}");
        }
        faces.push(SignModel { face: face.clone(), face_index: index as u16, lods, variants, frame_m });
    }
    faces.sort_by_key(|f| f.face_index);
    Ok(SignModels { faces, post_radius_m })
}

fn three_lods(face: &str, v: &Value) -> Result<[StreetProp; 3]> {
    let list = v.as_array().filter(|l| l.len() == 3).ok_or_else(|| anyhow!("{PROVENANCE}: {face} needs three lods"))?;
    let mut lods = vec![];
    for l in list {
        let s = |k: &str| l[k].as_str().map(str::to_string).ok_or_else(|| anyhow!("{PROVENANCE}: {face} lod has no {k}"));
        lods.push(StreetProp {
            path: s("path")?,
            sha256: s("sha256")?,
            bytes: l["bytes"].as_u64().ok_or_else(|| anyhow!("{PROVENANCE}: {face} lod has no bytes"))?,
            triangles: l["triangles"].as_u64(),
            dimensions_m: None,
        });
    }
    lods.try_into().map_err(|_| anyhow!("{PROVENANCE}: {face} lods"))
}

/// The wear variant of the plate at `pos` (its foot; x and z): one in 100 by a hash of
/// the 0.5 m cell it stands in (so the plates of one pole wear alike), aged for most,
/// rusty for 25 in 100, battered for 15. `count` variants are drawn; past them, the first.
pub fn wear_of(pos: [f64; 3], count: usize) -> usize {
    let x = (pos[0] / 0.5).floor() as i64 as u64;
    let z = (pos[2] / 0.5).floor() as i64 as u64;
    let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ z.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    let v = match h % 100 {
        0..60 => 0,
        60..85 => 1,
        _ => 2,
    };
    if v < count { v } else { 0 }
}
