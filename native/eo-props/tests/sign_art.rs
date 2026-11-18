//! the official sign art path of the sign atlases (sign_atlas::art).
//! The manifest check, the fallback to the drawn faces, the face table
//! (every inventory, printed and prop-sheet face maps to art or is marked
//! drawn), and, when `EO_SIGN_ART_ROOT` names a folder with the downloads,
//! the cooked faces themselves.

use eo_props::JsProps;
use eo_props::data::sha256_hex;
use eo_props::sign_atlas::art::{self, Manifest, SignArt};
use eo_props::sign_atlas::*;
use std::path::PathBuf;

fn fonts() -> Fonts {
    Fonts::system().expect("the JS sign font stack must resolve on this machine")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("eo-props-sign-art-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The art when `EO_SIGN_ART_ROOT` holds the downloads; None (the test says so) otherwise.
fn art_here() -> Option<SignArt> {
    let Some(root) = art::root() else {
        eprintln!("skipped: EO_SIGN_ART_ROOT is not set");
        return None;
    };
    match SignArt::load(&root) {
        Ok(a) => Some(a),
        Err(e) => {
            eprintln!("skipped: no verified sign art at {} ({e:#})", root.display());
            None
        }
    }
}

#[test]
fn manifest_check_accepts_the_recorded_bytes_and_refuses_any_other() {
    let root = tmp("verify");
    std::fs::create_dir_all(root.join("a")).unwrap();
    let body = b"<svg viewBox=\"0 0 1 1\"></svg>".to_vec();
    std::fs::write(root.join("a/x.svg"), &body).unwrap();
    let json = format!(r#"{{ "files": [{{ "id": "x", "path": "a/x.svg", "bytes": {}, "sha256": "{}" }}] }}"#, body.len(), sha256_hex(&body));
    let man = Manifest::parse(&json).unwrap();
    let got = art::read_verified(&root, &man).expect("the recorded bytes verify");
    assert_eq!(got["x"], body);
    // One byte different: refused, naming the digest.
    std::fs::write(root.join("a/x.svg"), b"<svg viewBox=\"0 0 1 2\"></svg>").unwrap();
    let e = art::read_verified(&root, &man).unwrap_err().to_string();
    assert!(e.contains("SHA-256"), "{e}");
    // Missing: refused.
    std::fs::remove_file(root.join("a/x.svg")).unwrap();
    assert!(art::read_verified(&root, &man).is_err());
    // No folder: refused.
    assert!(art::read_verified(&root.join("nope"), &man).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_tracked_manifest_records_every_file_with_its_provenance() {
    let m = Manifest::get();
    assert!(m.files.len() >= 20, "{} files", m.files.len());
    for f in &m.files {
        assert_eq!(f.sha256.len(), 64, "{}: sha256", f.id);
        assert!(f.bytes > 0, "{}: bytes", f.id);
        assert!(f.url.starts_with("https://mutcd.fhwa.dot.gov/") || f.url.starts_with("https://dot.ca.gov/") || f.url.starts_with("https://github.com/RedHatOfficial/Overpass/"), "{}: source {}", f.id, f.url);
        assert!(!f.licence.is_empty() && !f.retrieved.is_empty(), "{}: licence and retrieval date", f.id);
        assert!(!f.path.contains(".."), "{}: path stays under the root", f.id);
    }
}

#[test]
fn every_face_maps_to_official_art_or_is_marked_drawn() {
    let props = JsProps::load().unwrap();
    let m = Manifest::get();
    let ids: std::collections::HashSet<&str> = m.files.iter().map(|f| f.id.as_str()).collect();
    let check = |table: &str, face: &str, spec: &serde_json::Value| {
        fn refs<'a>(spec: &'a serde_json::Value, out: &mut Vec<(&'static str, &'a str)>) {
            if let Some(s) = spec["svg"].as_str() {
                out.push(("svg", s));
            }
            if let Some(l) = spec["layout"].as_str() {
                out.push(("layout", l));
            }
            for it in spec["stack"].as_array().into_iter().flatten() {
                refs(it, out);
            }
        }
        let mut r = vec![];
        refs(spec, &mut r);
        let drawn = spec["drawn"].as_str();
        assert!(!r.is_empty() ^ drawn.is_some(), "{table}.{face}: either art or a `drawn` reason, got {spec}");
        if let Some(why) = drawn {
            assert!(why.len() > 20, "{table}.{face}: say why it stays drawn");
        }
        for (kind, id) in r {
            match kind {
                "svg" => assert!(ids.contains(id), "{table}.{face}: {id} is not a manifest file"),
                _ => {
                    let lay = art::layout(id).unwrap_or_else(|e| panic!("{table}.{face}: {e}"));
                    let sheet = lay["sheet"].as_str().unwrap();
                    assert!(ids.contains(sheet), "{table}.{face}: layout {id} cites {sheet}, not a manifest file");
                    for e in lay["elements"].as_array().unwrap() {
                        assert!(e["from"].as_str().is_some_and(|s| !s.is_empty()), "{id}: element {e} cites no dimension");
                    }
                }
            }
        }
    };
    let mut official = 0;
    for face in &props.signs.faces {
        let spec = m.spec("faces", face).unwrap_or_else(|| panic!("inventory face {face} has no entry in the sign art manifest"));
        check("faces", face, spec);
        official += usize::from(spec["drawn"].is_null());
    }
    for face in PRINTED_SIGN_FACES {
        check("printed", face, m.spec("printed", face).unwrap_or_else(|| panic!("printed face {face} has no entry")));
    }
    for (cell, spec) in m.raw["propSheet"].as_object().unwrap() {
        assert!(PROP_SHEET_CELLS.iter().any(|c| c.0 == cell), "propSheet.{cell} is not a sheet cell");
        check("propSheet", cell, spec);
    }
    eprintln!("inventory faces: {official} of {} from official art, {} drawn", props.signs.faces.len(), props.signs.faces.len() - official);
    assert!(official >= 18, "official art for {official} faces");
}

#[test]
fn without_the_art_the_atlases_are_todays_drawings() {
    let props = JsProps::load().unwrap();
    let fonts = fonts();
    assert!(SignArt::load(&tmp("absent").join("no-such-folder")).is_err(), "a missing folder is no art");
    let drawn = cook_sign_atlases_with(&props, &fonts, &[], SIGN_FALLBACK_STREETS, None).unwrap();
    assert!(drawn.provenance.is_empty());
    assert_eq!(drawn.faces, sign_face_atlas(&props, &fonts).unwrap(), "the drawn face atlas, byte for byte");
    assert_eq!(drawn.printed, printed_sign_atlas(&fonts).unwrap());
    assert_eq!(drawn.prop_sheet, prop_sign_sheet(SIGN_FALLBACK_STREETS, &fonts).unwrap());
    // And the cache keys of the two paths differ.
    if let Some(a) = art_here() {
        assert_ne!(sign_atlas_cache_key_with(&props, &fonts, &[], SIGN_FALLBACK_STREETS, Some(&a)), sign_atlas_cache_key(&props, &fonts, &[], SIGN_FALLBACK_STREETS));
    }
}

#[test]
fn street_names_go_mixed_case() {
    assert_eq!(art::mixed_case("MISSION ST"), "Mission St");
    assert_eq!(art::mixed_case("5TH ST"), "5th St");
    assert_eq!(art::mixed_case("O'FARRELL ST"), "O'Farrell St");
    assert_eq!(art::mixed_case("ST MARY'S SQ"), "St Mary's Sq");
    assert_eq!(art::mixed_case("CESAR CHAVEZ ST"), "Cesar Chavez St");
}

#[test]
fn dominant_plate_sizes_are_the_most_common() {
    let plates = [0, 30, 30, 0, 0, 30, 30, 0, 0, 36, 36, 0, 1, 18, 6, 0];
    let d = art::dominant_plate_sizes(&plates, 3);
    assert_eq!(d, vec![Some([30.0, 30.0]), Some([18.0, 6.0]), None]);
    // Fitting a 12 x 18 sign into an 18 x 18 plate drawn in a 192 x 256 rect.
    let r = art::fit([12.0, 18.0], [18.0, 18.0], [0.0, 0.0, 192.0, 256.0]);
    assert!((r[2] - 128.0).abs() < 1e-9 && (r[3] - 256.0).abs() < 1e-9 && (r[0] - 32.0).abs() < 1e-9, "{r:?}");
}

fn hexrgb(s: &str) -> [u8; 3] {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap();
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn count_near(a: &Atlas, r: [u32; 4], c: [u8; 3], tol: i32) -> usize {
    let mut n = 0;
    for y in r[1]..r[3] {
        for x in r[0]..r[2] {
            let p = a.pixel(x, y);
            if p[3] > 200 && (0..3).all(|k| (p[k] as i32 - c[k] as i32).abs() <= tol) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn official_faces_cook_from_the_verified_art() {
    let Some(art) = art_here() else { return };
    let props = JsProps::load().unwrap();
    let fonts = fonts();
    let t = std::time::Instant::now();
    let a = cook_sign_atlases_with(&props, &fonts, &["MISSION ST".into(), "5TH ST".into(), "O'FARRELL ST".into()], ["MISSION ST", "5TH ST"], Some(&art)).unwrap();
    let cook_ms = t.elapsed().as_secs_f64() * 1e3;
    let drawn = cook_sign_atlases_with(&props, &fonts, &["MISSION ST".into(), "5TH ST".into(), "O'FARRELL ST".into()], ["MISSION ST", "5TH ST"], None).unwrap();
    // The layout is the JS's: same sizes, same tiles.
    for (x, y) in [(&a.faces, &drawn.faces), (&a.printed, &drawn.printed), (&a.prop_sheet, &drawn.prop_sheet), (&a.street_names, &drawn.street_names)] {
        assert_eq!((x.width, x.height, &x.tiles), (y.width, y.height, &y.tiles));
    }
    assert_eq!(a.provenance.iter().filter(|p| p.0.starts_with("faces/")).count(), props.signs.faces.len());
    let idx = |f: &str| props.signs.faces.iter().position(|x| x == f).unwrap();
    let cell = |f: &str| a.faces.tile_pixels(a.faces.tiles[idx(f)]);
    let pal = |n: &str| hexrgb(art::Manifest::get().raw["palette"][n]["hex"].as_str().unwrap());
    // R1-1: red field and white legend in the FHWA colours; octagon corners clear.
    assert!(count_near(&a.faces, cell("stop"), pal("red"), 6) > 20000, "STOP is FHWA red");
    assert!(count_near(&a.faces, cell("stop"), pal("white"), 6) > 3000, "STOP legend white");
    let [x0, y0, ..] = cell("stop");
    assert_eq!(a.faces.pixel(x0 + 2, y0 + 2)[3], 0, "octagon corner clear");
    // Warning and school colours; the school assembly fills its 30 x 42 frame.
    assert!(count_near(&a.faces, cell("warning"), pal("yellow"), 6) > 10000, "W17-1 yellow");
    assert!(count_near(&a.faces, cell("pedestrian"), pal("yellow"), 6) > 10000, "W11-2 yellow");
    // CA layouts: R30B red on white, R32C green, R99 blue.
    assert!(count_near(&a.faces, cell("streetCleaning"), pal("red"), 6) > 3000, "R30B red legend");
    assert!(count_near(&a.faces, cell("permitParking"), pal("green"), 6) > 3000, "R32C green");
    assert!(count_near(&a.faces, cell("disabledParking"), pal("blue"), 6) > 8000, "R99 blue");
    // R2-1: the sample numerals replaced, black ink stays.
    assert!(count_near(&a.faces, cell("speedLimit"), pal("black"), 30) > 4000, "R2-1 legend");
    // A drawn face keeps its JS colour (guide: green #0b6e3c).
    assert!(count_near(&a.faces, cell("guide"), hexrgb("#0b6e3c"), 12) > 5000, "guide stays drawn");
    // Street blades: the D3-1a green.
    let r = a.street_names.tile_pixels(a.street_names.tiles[0]);
    assert!(count_near(&a.street_names, r, pal("green"), 6) > 6000, "blade green");
    // Series calibration and its residual (the spread between lines of one series).
    for (s, text, f) in &art.calibration {
        eprintln!("series {s}: '{text}' factor {f:.3} (mean {:.3})", art.series[s]);
    }
    let b: Vec<f64> = art.calibration.iter().filter(|c| c.0 == "B").map(|c| c.2).collect();
    let spread = (b.iter().cloned().fold(f64::MIN, f64::max) - b.iter().cloned().fold(f64::MAX, f64::min)) / art.series["B"];
    // The residual is reported, not gated: it is Overpass's letter proportions against Series B's
    // (HOUR vs PARKING); the bound only catches a broken measurement.
    eprintln!("series B residual: the two calibration lines differ by {:.1} % of their mean", spread * 100.0);
    assert!(spread < 0.3, "Series B calibration lines within 30 % of each other ({:.1} %): a measurement is broken", spread * 100.0);
    for f in art.series.values() {
        assert!((0.3..1.2).contains(f), "series factor {f} outside any FHWA series' width over Overpass Bold");
    }
    eprintln!("official-art cook: {cook_ms:.0} ms, {:.1} MB RGBA", a.rgba_bytes() as f64 / 1e6);
    for (k, v) in &a.provenance {
        eprintln!("  {k}: {v}");
    }
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("bench-out");
    let written = a.write_pngs(&out, "sign-art-").unwrap();
    eprintln!("wrote {written:?}");
}

#[test]
fn the_cache_keeps_the_provenance() {
    let Some(art) = art_here() else { return };
    let props = JsProps::load().unwrap();
    let fonts = fonts();
    let dir = tmp("cache");
    let (a, o1) = load_or_cook_with(&dir, &props, &fonts, &[], SIGN_FALLBACK_STREETS, Some(&art)).unwrap();
    assert_eq!(o1, CacheOutcome::Cooked);
    let (b, o2) = load_or_cook_with(&dir, &props, &fonts, &[], SIGN_FALLBACK_STREETS, Some(&art)).unwrap();
    assert_eq!(o2, CacheOutcome::Hit);
    assert_eq!(a, b, "atlases and provenance read back");
    assert!(!b.provenance.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
