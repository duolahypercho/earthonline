//! Every official sign face the game draws (the sign art manifest's `faces`,
//! `printed` and `propSheet` specs that name an FHWA layout or a Caltrans
//! sheet), rendered alone at `--px` on its long side, with its size in inches,
//! plus street-name blades: the print layer of the Blender sign build
//! (scripts/assets/build-street-signs-v1.py), so the 3D plates carry exactly
//! the faces the game's atlases do. An inventory face the manifest marks
//! `drawn` (no published layout) is the face the game draws itself
//! (`sign_atlas::drawn_face`) filling its plate frame; other drawn faces are left out.
//!
//!   cargo run --release -p eo-props --example sign_faces_export -- --out DIR [--px 2048]
//!
//! DIR gets `<table>-<face>.png` (sRGB, straight alpha: clear outside the
//! sign) and `faces.json` ([{table, face, png, size_in: [w, h], spec}], an
//! inventory face also with `face_index` and `frame_in`, the plate frame the
//! game draws it in: the inventory's most common plate size).

use eo_props::sign_atlas::art::{self, Manifest, SignArt};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let out = std::path::PathBuf::from(arg("--out").ok_or_else(|| anyhow::anyhow!("--out DIR"))?);
    let px: u32 = arg("--px").and_then(|v| v.parse().ok()).unwrap_or(2048);
    let root = art::root().ok_or_else(|| anyhow::anyhow!("no sign art: {}", eo_data::external::SIGN_ART.looked_in()))?;
    let art = SignArt::load(&root)?;
    std::fs::create_dir_all(&out)?;
    let m = Manifest::get();
    // The plate frame each inventory face is drawn in (`dominant_plate_sizes`, as the atlas cells
    // and the game's SignPlate instances use it): the art fitted into it, uniformly, centred.
    let js = eo_props::JsProps::load()?;
    let frames = art::dominant_plate_sizes(&js.signs.plates, js.signs.faces.len());
    let frame_of = |face: &str| js.signs.faces.iter().position(|f| f == face).and_then(|i| frames[i]).map(|f| (js.signs.faces.iter().position(|g| g == face).unwrap(), f));
    let mut rows = vec![];
    for table in ["faces", "printed", "propSheet"] {
        for (face, spec) in m.raw[table].as_object().into_iter().flatten() {
            if spec["drawn"].is_string() {
                // no published layout: the face the game draws itself (`sign_atlas::drawn_face`),
                // filling the face's plate frame, for an inventory face (its frame known).
                match (table, frame_of(face)) {
                    ("faces", Some((index, f))) => {
                        let (w, h) = (px, (px as f64 * f[1] / f[0]).round() as u32);
                        let Some(a) = eo_props::sign_atlas::drawn_face(face, w, h, &art.fonts)? else {
                            println!("{table}/{face}: drawn, but the DRAW table has no drawing: left out");
                            continue;
                        };
                        let name = format!("{table}-{face}.png");
                        a.write_png(&out.join(&name))?;
                        println!("{table}/{face}: drawn by the game, {:.0} x {:.0} in frame", f[0], f[1]);
                        rows.push(serde_json::json!({ "table": table, "face": face, "png": name, "size_in": f, "spec": spec, "face_index": index, "frame_in": f, "drawn": true }));
                    }
                    _ => println!("{table}/{face}: drawn, no published layout: left out"),
                }
                continue;
            }
            let size = art.spec_size(spec)?;
            let png = art::render_spec_png(&art, spec, px)?;
            let name = format!("{table}-{face}.png");
            std::fs::write(out.join(&name), png)?;
            println!("{table}/{face}: {:.0} x {:.0} in", size[0], size[1]);
            let mut row = serde_json::json!({ "table": table, "face": face, "png": name, "size_in": size, "spec": spec });
            if table == "faces" {
                if let Some((index, f)) = frame_of(face) {
                    row["face_index"] = index.into();
                    row["frame_in"] = serde_json::json!(f);
                }
            }
            rows.push(row);
        }
    }
    // Street-name blades (D3-1a) at the game's blade frame, 0.9 x 0.16 m.
    for legend in ["Folsom St", "Mission St", "5th St", "Valencia St"] {
        let frame = [0.9, 0.16];
        let (w, h) = (px, (px as f64 * frame[1] / frame[0]).round() as u32);
        let mut pix = tiny_skia::Pixmap::new(w, h).ok_or_else(|| anyhow::anyhow!("pixmap"))?;
        art.street_blade(&mut pix, &legend.to_uppercase(), [0.0, 0.0, w as f64, h as f64], frame)?;
        let rgba: Vec<u8> = pix
            .pixels()
            .iter()
            .flat_map(|c| {
                let c = c.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        let name = format!("blade-{}.png", legend.to_lowercase().replace(' ', "-"));
        eo_props::sign_atlas::Atlas { width: w, height: h, rgba, tiles: vec![], srgb: true }.write_png(&out.join(&name))?;
        let size_in = [frame[0] / 0.0254, frame[1] / 0.0254];
        println!("blade {legend}: {:.1} x {:.1} in", size_in[0], size_in[1]);
        rows.push(serde_json::json!({ "table": "blade", "face": legend, "png": name, "size_in": size_in }));
    }
    std::fs::write(out.join("faces.json"), serde_json::to_string_pretty(&rows)?)?;
    println!("{} faces into {}", rows.len(), out.display());
    Ok(())
}
