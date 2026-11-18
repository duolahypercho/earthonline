//! every art spec of the sign art manifest rendered alone, and a few
//! D3-1 blades, into `native/bench-out/sign-art-preview/` (git-ignored), for a
//! look at the official faces without the game.
//!
//!   cargo run --release -p eo-props --example sign_art_preview

use eo_props::sign_atlas::art::{self, Manifest, SignArt};

fn main() -> anyhow::Result<()> {
    let t = std::time::Instant::now();
    let root = art::root().ok_or_else(|| anyhow::anyhow!("EO_SIGN_ART_ROOT is not set"))?;
    let art = SignArt::load(&root)?;
    println!("sign art verified and parsed in {:.0} ms from {}", t.elapsed().as_secs_f64() * 1e3, art.root.display());
    for (s, text, f) in &art.calibration {
        println!("series {s}: '{text}' factor {f:.3}");
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("bench-out").join("sign-art-preview");
    std::fs::create_dir_all(&out)?;
    let m = Manifest::get();
    for table in ["faces", "printed", "propSheet"] {
        for (face, spec) in m.raw[table].as_object().into_iter().flatten() {
            if spec["drawn"].is_string() {
                println!("{table}/{face}: drawn ({})", spec["drawn"].as_str().unwrap_or_default());
                continue;
            }
            let png = art::render_spec_png(&art, spec, 512)?;
            let p = out.join(format!("{table}-{face}.png"));
            std::fs::write(&p, png)?;
            println!("{table}/{face}: {}", p.display());
        }
    }
    // Blades at the game's blade size (0.9 x 0.16 m), 1024 px wide.
    let (w, h) = (1024u32, (1024.0 * 0.16 / 0.9) as u32);
    let names = ["MISSION ST", "5TH ST", "O'FARRELL ST", "CESAR CHAVEZ ST", "SOUTH VAN NESS AVE"];
    let mut pix = tiny_skia::Pixmap::new(w, h * names.len() as u32).unwrap();
    for (i, n) in names.iter().enumerate() {
        art.street_blade(&mut pix, n, [0.0, (i as u32 * h) as f64, w as f64, h as f64], [0.9, 0.16])?;
    }
    let p = out.join("blades.png");
    let rgba: Vec<u8> = pix.pixels().iter().flat_map(|c| {
        let c = c.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }).collect();
    eo_props::sign_atlas::Atlas { width: pix.width(), height: pix.height(), rgba, tiles: vec![], srgb: true }.write_png(&p)?;
    // The street-name atlas cells (256 x 64, as the game cooks them) for the same names, drawn and official.
    let js = eo_props::JsProps::load()?;
    let fonts = eo_props::sign_atlas::Fonts::system()?;
    let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    let legends = eo_props::sign_atlas::SIGN_FALLBACK_STREETS;
    for (tag, a) in [("before", None), ("after", Some(&art))] {
        let s = eo_props::sign_atlas::cook_sign_atlases_with(&js, &fonts, &names, legends, a)?.street_names;
        let rows = 2 * eo_props::sign_atlas::STREET_SIGN_CELL_HEIGHT;
        let crop = eo_props::sign_atlas::Atlas { width: s.width, height: rows, rgba: s.rgba[..(s.width * rows * 4) as usize].to_vec(), tiles: vec![], srgb: true };
        let p = out.join(format!("street-name-cells-{tag}.png"));
        crop.write_png(&p)?;
        println!("street-name cells ({tag}): {}", p.display());
    }
    println!("blades: {}", p.display());
    Ok(())
}
