//! the cooked sign atlases against the JS (the earlier browser version's exporter
//! writes data/sign-atlas-js.json.gz from the game's own modules, headless).
//! Layout (tiles, plans, rects, the data-built symbol texture) must equal the
//! JS; the canvas-drawn pixels cannot be exported headless, so they are held
//! to shape and colour: every face's cell is drawn, its dominant colour is the
//! JS colour, and its outline is the cell alpha.

use eo_props::JsProps;
use eo_props::sign_atlas::*;
use serde_json::Value;
use std::io::Read;

fn js() -> Value {
    let gz = include_bytes!("../data/sign-atlas-js.json.gz");
    let mut s = String::new();
    flate2::read::GzDecoder::new(&gz[..]).read_to_string(&mut s).unwrap();
    serde_json::from_str(&s).unwrap()
}

fn arr4(v: &Value) -> [f64; 4] {
    let a = v.as_array().unwrap();
    [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap(), a[3].as_f64().unwrap()]
}

fn close4(rust: [f32; 4], js: [f64; 4], what: &str) {
    for k in 0..4 {
        assert!((rust[k] as f64 - js[k]).abs() < 1e-6, "{what}: rust {rust:?} js {js:?}");
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().filter_map(|s| s.as_str().map(String::from)).collect()
}

fn fonts() -> Fonts {
    Fonts::system().expect("the JS sign font stack (Helvetica Neue, Helvetica, Arial, sans-serif) must resolve on this machine")
}

fn hexrgb(s: &str) -> [u8; 3] {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap();
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// The most common colour (5-bit quantised) among the opaque pixels of a pixel rect, and the opaque share.
fn dominant(a: &Atlas, [x0, y0, x1, y1]: [u32; 4]) -> ([u8; 3], f64) {
    let mut hist = std::collections::HashMap::<[u8; 3], (u32, [u64; 3])>::new();
    let (mut opaque, mut all) = (0u32, 0u32);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = a.pixel(x, y);
            all += 1;
            if p[3] < 200 {
                continue;
            }
            opaque += 1;
            let e = hist.entry([p[0] >> 3, p[1] >> 3, p[2] >> 3]).or_default();
            e.0 += 1;
            for k in 0..3 {
                e.1[k] += p[k] as u64;
            }
        }
    }
    let Some((_, (n, sum))) = hist.into_iter().max_by_key(|(_, (n, _))| *n) else { return ([0, 0, 0], 0.0) };
    ([(sum[0] / n as u64) as u8, (sum[1] / n as u64) as u8, (sum[2] / n as u64) as u8], opaque as f64 / all.max(1) as f64)
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|k| (a[k] as i32 - b[k] as i32).abs() <= tol)
}

fn count_near(a: &Atlas, [x0, y0, x1, y1]: [u32; 4], c: [u8; 3], tol: i32) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = a.pixel(x, y);
            if p[3] > 200 && near([p[0], p[1], p[2]], c, tol) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn face_tiles_equal_the_js() {
    let j = js();
    let props = JsProps::load().unwrap();
    let f = &j["faces"];
    let names = strings(&f["names"]);
    assert_eq!(names, props.signs.faces, "the export's face list is the inventory's");
    assert_eq!(f["atlas"]["cell"].as_u64().unwrap() as u32, SIGN_FACE_CELL);
    assert_eq!(f["atlas"]["columns"].as_u64().unwrap() as u32, SIGN_FACE_COLUMNS);
    assert_eq!(f["atlas"]["size"].as_u64().unwrap() as u32, SIGN_FACE_SIZE);
    assert_eq!(strings(&f["drawn"]), SIGN_FACE_NAMES.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    for (name, aspect) in f["aspect"].as_object().unwrap() {
        assert!((sign_face_aspect(name) - aspect.as_f64().unwrap()).abs() < 1e-12, "aspect {name}");
    }
    for (i, (face, tile)) in names.iter().zip(f["tiles"].as_array().unwrap()).enumerate() {
        close4(sign_face_tile(face, i), arr4(tile), &format!("signFaceTile({face}, {i})"));
    }
    // The live JS paints no plate-back / post-zinc finish cells; neither does the port.
    assert!(f["finishCells"].is_null(), "the JS reference now has SIGN_FINISH_CELLS: port the finish");
}

#[test]
fn export_is_from_the_current_js() {
    // The export records the SHA-256 of each JS source; warn (as JsProps does) when the
    // checkout at EO_REPO_ROOT has moved on since, so the export can be refreshed.
    let j = js();
    let Ok(root) = std::env::var("EO_REPO_ROOT") else { return };
    for (p, digest) in j["sources"].as_object().unwrap() {
        let now = std::fs::read(std::path::Path::new(&root).join(p)).map(|b| eo_props::data::sha256_hex(&b));
        if now.as_deref().ok() != digest.as_str() {
            eprintln!("warning: {p} changed since the sign atlases were exported");
        }
    }
}

#[test]
fn street_name_plans_and_legends_equal_the_js() {
    let j = js();
    let sn = &j["streetNames"];
    for plan in sn["plans"].as_array().unwrap() {
        let input = strings(&plan["input"]);
        let ours = plan_street_sign_atlas(&input);
        assert_eq!(ours.width as u64, plan["width"].as_u64().unwrap());
        assert_eq!(ours.height as u64, plan["height"].as_u64().unwrap(), "height for {} names", input.len());
        assert_eq!(ours.capacity as u64, plan["capacity"].as_u64().unwrap());
        assert_eq!(ours.requested, strings(&plan["requested"]), "requested order (localeCompare)");
        assert_eq!(ours.accepted, strings(&plan["accepted"]));
        assert_eq!(ours.overflow, strings(&plan["overflow"]));
        for (i, t) in plan["tiles"].as_array().unwrap().iter().enumerate() {
            close4(ours.tiles[i], arr4(t), &format!("tile of {}", ours.accepted[i]));
        }
    }
    for l in sn["legends"].as_array().unwrap() {
        let input = l["input"].as_str().unwrap();
        assert_eq!(normalize_street_legend(input).as_deref(), l["output"].as_str(), "normalizeStreetLegend({input:?})");
    }
    assert_eq!(strings(&sn["fallback"]), SIGN_FALLBACK_STREETS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    // signStreetLegends: count first, then name.
    let segs = [("Mission Street", None), ("Folsom Street", None), ("Mission Street", None), ("SF Rd 9", None), ("Folsom Street", None), ("Alpha Street", None)];
    assert_eq!(sign_street_legends(segs, None), ["FOLSOM ST".to_string(), "MISSION ST".to_string()]);
    assert_eq!(sign_street_legends([("Route 80", None)], None), ["MAIN ST".to_string(), "CROSS ST".to_string()]);
}

#[test]
fn prop_sheet_and_printed_layout_equal_the_js() {
    let j = js();
    let ps = &j["propSheet"];
    assert_eq!(ps["size"].as_u64().unwrap() as u32, PROP_SHEET_SIZE);
    assert!((ps["blank"]["u"].as_f64().unwrap() - PROP_SHEET_BLANK_UV[0] as f64).abs() < 1e-9);
    assert!((ps["blank"]["v"].as_f64().unwrap() - PROP_SHEET_BLANK_UV[1] as f64).abs() < 1e-9);
    let cells = ps["cells"].as_object().unwrap();
    assert_eq!(cells.len(), PROP_SHEET_CELLS.len());
    for (name, rect) in cells {
        close4(prop_sheet_cell_rect(name).unwrap_or_else(|| panic!("cell {name}")), arr4(rect), &format!("SIGN_ATLAS.cells.{name}"));
    }
    let pr = &j["printed"];
    assert_eq!(strings(&pr["faces"]), PRINTED_SIGN_FACES.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(pr["atlas"]["width"].as_u64().unwrap() as u32, PRINTED_ATLAS_WIDTH);
    assert_eq!(pr["atlas"]["height"].as_u64().unwrap() as u32, PRINTED_ATLAS_HEIGHT);
    assert_eq!(pr["atlas"]["cell"].as_u64().unwrap() as u32, PRINTED_ATLAS_CELL);
    let rects = pr["rects"].as_object().unwrap();
    assert_eq!(rects.len(), PRINTED_SIGN_FACES.len(), "every printed face is on a JS plate");
    for (face, r) in rects {
        let [u0, v0, u1, v1] = arr4(r);
        close4(printed_face_tile(face), [u0, v0, u1 - u0, v1 - v0], &format!("printedFaceRect({face})"));
    }
    assert!(pr["finishCells"].is_null(), "the JS reference now has PRINTED_FINISH_CELLS: port the finish");
    let white: Vec<f64> = serde_json::from_value(pr["whiteUv"].clone()).unwrap();
    assert_eq!(white, PRINTED_WHITE_UV.iter().map(|v| *v as f64).collect::<Vec<_>>(), "printed white point");
}

#[test]
fn pedestrian_symbol_texture_is_the_js_bytes_and_digit_tables_match() {
    let j = js();
    let p = &j["ped"];
    let t = ped_symbol_texture();
    assert_eq!((t.width as u64, t.height as u64), (p["symbol"]["width"].as_u64().unwrap(), p["symbol"]["height"].as_u64().unwrap()));
    let js_order = flip_rows(&t.rgba, t.width as usize * 4);
    assert_eq!(eo_props::data::sha256_hex(&js_order), p["symbol"]["sha256"].as_str().unwrap(), "createPedestrianSymbolTexture bytes");
    let segs: Vec<Vec<u8>> = serde_json::from_value(p["digitSegments"].clone()).unwrap();
    assert_eq!(segs, DIGIT_SEGMENTS.iter().map(|d| d.to_vec()).collect::<Vec<_>>());
    assert_eq!(p["seg"]["w"].as_f64().unwrap(), SEG_W);
    assert_eq!(p["seg"]["h"].as_f64().unwrap(), SEG_H);
    assert_eq!(p["seg"]["t"].as_f64().unwrap(), SEG_T);
    let dx: Vec<f64> = serde_json::from_value(p["digitX"].clone()).unwrap();
    assert_eq!(dx, DIGIT_X.to_vec());
    assert_eq!(p["symbolX"].as_f64().unwrap(), SYMBOL_X);
    // The digits: "8" lights all seven boxes, "1" only b and c.
    let d = ped_digit_atlas().unwrap();
    let lit = |digit: usize| {
        let r = d.tile_pixels(d.tiles[digit]);
        (r[1]..r[3]).flat_map(|y| (r[0]..r[2]).map(move |x| (x, y))).filter(|&(x, y)| d.pixel(x, y)[3] > 128).count()
    };
    assert!(lit(8) > lit(0) && lit(0) > lit(7) && lit(7) > lit(1) && lit(1) > 0, "segment counts order 8 > 0 > 7 > 1");
}

#[test]
fn face_atlas_reads_as_the_faces() {
    let props = JsProps::load().unwrap();
    let fonts = fonts();
    let a = sign_face_atlas(&props, &fonts).unwrap();
    assert_eq!((a.width, a.height), (SIGN_FACE_SIZE, SIGN_FACE_SIZE));
    assert_eq!(a.tiles.len(), props.signs.faces.len());
    let colours = |name: &str| -> [u8; 3] {
        let v: Value = props.value("signFaceColours")[name].clone();
        hexrgb(v.as_str().unwrap())
    };
    for (i, face) in props.signs.faces.iter().enumerate() {
        let r = a.tile_pixels(a.tiles[i]);
        let (dom, cover) = dominant(&a, r);
        assert!(cover > 0.3, "{face}: cell is drawn ({cover:.2} opaque)");
        // A plate's dominant colour is its flat colour (SIGN_FACE_COLOURS), except
        // the white-on-colour / colour-on-white faces where the legend or the
        // border takes over less than the field: these all hold.
        let want = colours(face);
        let tol = 20;
        let check = match face.as_str() {
            // The black ONE WAY plaque's dominant is its white arrow, by design.
            "oneWayRight" | "oneWayLeft" => near(dom, hexrgb("#f4f4f0"), tol) || near(dom, want, tol),
            _ => near(dom, want, tol),
        };
        assert!(check, "{face}: dominant {dom:?}, SIGN_FACE_COLOURS {want:?}");
    }
    let idx = |f: &str| props.signs.faces.iter().position(|x| x == f).unwrap();
    let alpha = |x: f64, y: f64| a.pixel(x as u32, y as u32)[3];
    // Octagon (STOP), diamonds (pedestrian, warning), pentagon (school): the
    // corners of the face rect are transparent, the middle opaque.
    for face in ["stop", "pedestrian", "warning", "school"] {
        let [x, y, w, h] = sign_face_rect(face, idx(face));
        assert_eq!(alpha(x + 3.0, y + 3.0), 0, "{face}: top-left corner clear");
        assert_eq!(alpha(x + w - 4.0, y + 3.0), 0, "{face}: top-right corner clear");
        assert_eq!(alpha(x + w / 2.0, y + h / 2.0), 255, "{face}: middle opaque");
        if face != "school" {
            assert_eq!(alpha(x + 3.0, y + h - 4.0), 0, "{face}: bottom-left corner clear");
            assert_eq!(alpha(x + w - 4.0, y + h - 4.0), 0, "{face}: bottom-right corner clear");
        } else {
            assert_eq!(alpha(x + 3.0, y + h - 4.0), 255, "school: square foot");
        }
    }
    // The octagon's edge midpoints are opaque (it is an octagon, not a circle).
    let [x, y, w, _] = sign_face_rect("stop", idx("stop"));
    assert_eq!(alpha(x + w / 2.0, y + 2.0), 255);
    assert_eq!(alpha(x + 2.0, y + w / 2.0), 255);
    // Rectangular plates fill their face rect to the corner.
    for face in ["streetCleaning", "allWay", "doNotEnter", "speedLimit"] {
        let [x, y, _, _] = sign_face_rect(face, idx(face));
        assert_eq!(alpha(x + 2.0, y + 2.0), 255, "{face}: square corner");
    }
    // STOP carries white legend on its red; school FYG carries black figures.
    let stop = a.tile_pixels(a.tiles[idx("stop")]);
    assert!(count_near(&a, stop, hexrgb("#f4f4f0"), 12) > 2000, "STOP legend + rim are white");
    let school = a.tile_pixels(a.tiles[idx("school")]);
    assert!(count_near(&a, school, hexrgb("#111111"), 12) > 1500, "school figures are black");
    // Cells past the last face stay clear (no finish cells in the live JS).
    let last = SIGN_FACE_SIZE - 1;
    assert_eq!(a.pixel(last - 10, last - 10)[3], 0, "unused cells are clear");
}

#[test]
fn street_names_printed_and_sheet_read_as_the_js() {
    let j = js();
    let fonts = fonts();
    let names = strings(&j["streetNames"]["plans"][0]["input"]);
    let (a, tiles) = street_name_atlas(&names, &fonts).unwrap();
    assert_eq!((a.width, a.height), (1024, 1024));
    let green = hexrgb("#1d4632");
    for (name, t) in &tiles {
        let r = a.tile_pixels(*t);
        let (dom, cover) = dominant(&a, r);
        assert!(cover > 0.99 && near(dom, green, 8), "{name}: green blade, got {dom:?}");
        let white = count_near(&a, r, [255, 255, 255], 30);
        assert!(white > 400, "{name}: white legend and rules ({white} px)");
    }
    // Past the accepted cells the sheet is white.
    assert_eq!(a.pixel(1000, 1000), [255, 255, 255, 255]);

    let p = printed_sign_atlas(&fonts).unwrap();
    assert_eq!((p.width, p.height), (PRINTED_ATLAS_WIDTH, PRINTED_ATLAS_HEIGHT));
    for (face, want) in [
        ("street-cleaning", "#f4f4ef"),
        ("pay-here", "#1f4fa3"),
        ("pay-by-plate", "#1f4fa3"),
        ("right-lane-must-turn-right", "#f4f4ef"),
        ("one-way-left", "#121416"),
        ("passenger-loading", "#f4f4ef"),
    ] {
        let r = p.tile_pixels(printed_face_tile(face));
        let (dom, cover) = dominant(&p, r);
        assert!(cover > 0.99 && near(dom, hexrgb(want), 12), "{face}: dominant {dom:?} want {want}");
    }
    assert!(count_near(&p, p.tile_pixels(printed_face_tile("street-cleaning")), hexrgb("#b3202a"), 16) > 3000, "street cleaning red legend");
    // The rows under the faces stay white, as does the white point.
    assert_eq!(p.pixel(128, 6 * PRINTED_ATLAS_CELL + 128), [255, 255, 255, 255]);
    assert_eq!(p.pixel(128, 7 * PRINTED_ATLAS_CELL + 128), [255, 255, 255, 255]);
    // The white point the unlettered parts sample stays white.
    let white_y = ((1.0 - PRINTED_WHITE_UV[1]) * PRINTED_ATLAS_HEIGHT as f32) as u32;
    assert_eq!(p.pixel((PRINTED_WHITE_UV[0] * PRINTED_ATLAS_WIDTH as f32) as u32, white_y), [255, 255, 255, 255]);

    let s = prop_sign_sheet(["MISSION ST", "5TH ST"], &fonts).unwrap();
    let cell = |n: &str| {
        let [u0, v0, u1, v1] = prop_sheet_cell_rect(n).unwrap();
        s.tile_pixels([u0, v0, u1 - u0, v1 - v0])
    };
    assert!(near(dominant(&s, cell("bladeA")).0, green, 8));
    assert!(near(dominant(&s, cell("transit")).0, hexrgb("#1c3f6b"), 8));
    assert!(near(dominant(&s, cell("countdown")).0, hexrgb("#141618"), 8));
    assert!(count_near(&s, cell("countdown"), hexrgb("#d8382a"), 16) > 1000, "the countdown face's red 12");
    assert!(count_near(&s, cell("countdown"), hexrgb("#d5541f"), 16) > 3000, "the countdown face's orange hand");
}

#[test]
// the drawn path explicitly (`load_or_cook` takes the official art when this machine has it;
// tests/sign_art.rs covers that path and its cache).
fn cache_round_trips_and_rejects_a_bad_payload() {
    let props = JsProps::load().unwrap();
    let fonts = fonts();
    let j = js();
    let names = strings(&j["streetNames"]["plans"][0]["input"]);
    let dir = std::env::temp_dir().join(format!("eo-props-sign-atlas-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (cooked, o1) = load_or_cook_with(&dir, &props, &fonts, &names, ["MISSION ST", "5TH ST"], None).unwrap();
    assert_eq!(o1, CacheOutcome::Cooked);
    let (read, o2) = load_or_cook_with(&dir, &props, &fonts, &names, ["MISSION ST", "5TH ST"], None).unwrap();
    assert_eq!(o2, CacheOutcome::Hit);
    assert_eq!(read, cooked, "the cache returns the cooked bytes");
    // Another input is another key.
    let (_, o3) = load_or_cook_with(&dir, &props, &fonts, &names[..3], ["MISSION ST", "5TH ST"], None).unwrap();
    assert_eq!(o3, CacheOutcome::Cooked);
    // A PNG replaced by another image of the same size fails its digest and is re-cooked.
    let key = sign_atlas_cache_key(&props, &fonts, &names, ["MISSION ST", "5TH ST"]);
    let png = dir.join("props").join(format!("sign-atlas-{}-ped-symbols.png", &key[..24]));
    assert!(png.is_file(), "{}", png.display());
    let mut bad = cooked.ped_symbols.clone();
    bad.rgba[3] ^= 0xff;
    bad.write_png(&png).unwrap();
    let (again, o4) = load_or_cook_with(&dir, &props, &fonts, &names, ["MISSION ST", "5TH ST"], None).unwrap();
    assert_eq!(o4, CacheOutcome::Cooked);
    assert_eq!(again, cooked);
    let _ = std::fs::remove_dir_all(&dir);

    // The cooked atlases for a look: native/bench-out/sign-atlas-*.png (git-ignored).
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("bench-out");
    let written = cooked.write_pngs(&out, "sign-atlas-").unwrap();
    eprintln!("wrote {written:?} (fonts {} / {})", fonts.regular_file.display(), fonts.bold_file.display());
}
