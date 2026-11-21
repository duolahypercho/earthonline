//! The shop signs' words (stage S1, `--facade-clutter`): only generic
//! phrases of the list are ever drawn; a tenancy's words are the same on every
//! thread and in any order; every lettered vertex names a cell of the list and
//! codes the shader reads back; off, nothing is lettered.

use eo_kit::city::KitCity;
use eo_kit::clutter::{self, LetterCode, SIGN_PHRASES, STACKED_WORDS};
use eo_kit::pipeline::{Planned, RungMesh, build_rung_with, plan_building};
use std::collections::BTreeMap;
use std::sync::OnceLock;

fn city() -> &'static KitCity {
    static C: OnceLock<KitCity> = OnceLock::new();
    C.get_or_init(|| {
        let root = eo_kit::repo_root().expect("repo root");
        KitCity::load(&root).expect("city").1
    })
}

fn planned(i: usize) -> Option<Planned> {
    let kc = city();
    if kc.excluded.contains(&i) {
        return None;
    }
    plan_building(&kc.records[i], &kc.world, i as u32, 3.25)
}

fn letters_bytes(m: &RungMesh) -> Vec<u8> {
    let mut out = vec![];
    for (k, l) in m.letters.iter().enumerate() {
        out.extend_from_slice(&(k as u32).to_le_bytes());
        for b in l {
            out.extend_from_slice(b);
        }
    }
    out
}

/// Generic words only: capitals, spaces and '&', each word one a sign writer
/// would paint for a trade (no name, no brand, no mark), the cells in a byte.
#[test]
fn only_generic_phrases_and_their_cells() {
    // every word any phrase is made of: shop categories and service descriptions.
    const WORDS: &[&str] = &[
        "CAFE", "COFFEE", "ESPRESSO", "SHOP", "RESTAURANT", "DINER", "NOODLES", "NOODLE", "HOUSE", "DIM", "SUM", "TAQUERIA", "TACOS", "PIZZA", "PIZZERIA", "SUSHI", "BURGERS", "DELI",
        "MARKET", "BAR", "COCKTAILS", "LOUNGE", "TAVERN", "CLUB", "LIQUOR", "LIQUORS", "SPIRITS", "WINE", "BAKERY", "GROCERY", "PRODUCE", "BOOKS", "USED", "RECORDS", "VIDEO",
        "ANTIQUES", "VINTAGE", "GALLERY", "PRINTING", "COPIES", "ART", "SUPPLY", "GIFTS", "THRIFT", "CLEANERS", "DRY", "LAUNDRY", "COIN", "LAUNDROMAT", "PHARMACY", "CLINIC",
        "DENTAL", "OPTICAL", "SALON", "HAIR", "BEAUTY", "BARBER", "NAILS", "NAIL", "TATTOO", "STUDIO", "YOGA", "FITNESS", "GYM", "PHOTO", "TAILOR", "ALTERATIONS", "SHOES", "SHOE",
        "REPAIR", "FLOWERS", "FLORIST", "FURNITURE", "RUGS", "LIGHTING", "HARDWARE", "PAINT", "BIKES", "BIKE", "TAX", "SERVICE", "INSURANCE", "OPEN",
    ];
    for (p, faces) in SIGN_PHRASES {
        assert!(p.chars().all(|c| c.is_ascii_uppercase() || c == ' ' || c == '&'), "{p}");
        for w in p.split([' ', '&']).filter(|w| !w.is_empty()) {
            assert!(WORDS.contains(&w), "{p}: '{w}' is not a generic trade word");
        }
        assert!(faces[0] <= clutter::FACE_SCRIPT, "{p}");
    }
    for w in STACKED_WORDS {
        assert!(SIGN_PHRASES.iter().any(|(p, _)| p == w), "stacked {w} is a phrase");
    }
    let cells = clutter::letter_cells();
    assert!(cells.len() <= 256 && cells.len() > SIGN_PHRASES.len(), "{} cells", cells.len());
    for c in cells {
        assert!(SIGN_PHRASES.iter().any(|(p, _)| *p == c.phrase), "{} is a listed phrase", c.phrase);
        assert_eq!(c.text.to_uppercase(), c.phrase, "a cell draws its phrase, only its case changes");
        assert!(c.aspect >= 1.0 && c.aspect < 16.0, "{} {}", c.text, c.aspect);
    }
    // a tenancy's text: its trade word is looked up, a name never drawn.
    let fam = |t: Option<&str>, ty: &str| clutter::sign_family(t, ty).to_vec();
    assert!(fam(Some("LITTLE CUP|LIQUOR"), "grocery").contains(&"LIQUOR"));
    assert!(fam(Some("CORNER CAFE"), "books").contains(&"CAFE"));
    assert_eq!(fam(Some("NORTHLIGHT"), "bar"), fam(None, "bar"), "a name with no trade word: the shop type's phrases");
    for ty in ["books", "cleaners", "cafe", "studio", "grocery", "bar", "bakery", "other"] {
        for p in fam(None, ty) {
            assert!(SIGN_PHRASES.iter().any(|(q, _)| *q == p), "{ty}: {p} listed");
        }
    }
}

/// Off: no lettering. On: the same words on every thread, in any order.
#[test]
fn the_same_words_for_a_tenancy_every_time() {
    let kc = city();
    let picks: Vec<usize> = (0..kc.records.len()).step_by(211).filter(|i| !kc.excluded.contains(i)).collect();
    for &i in picks.iter().take(40) {
        let Some(p) = planned(i) else { continue };
        for rung in [0, 2] {
            assert!(build_rung_with(&p, rung, 0).letters.is_empty(), "#{i}: no lettering without the clutter");
        }
    }
    let mesh = |i: usize| -> Vec<u8> {
        let Some(p) = planned(i) else { return vec![] };
        let mut out = letters_bytes(&build_rung_with(&p, 0, 1));
        out.extend(letters_bytes(&build_rung_with(&p, 2, 1)));
        out
    };
    let first: Vec<Vec<u8>> = picks.iter().map(|&i| mesh(i)).collect();
    let second: Vec<(usize, Vec<u8>)> = std::thread::scope(|s| {
        let chunks: Vec<Vec<usize>> = picks.iter().rev().copied().collect::<Vec<_>>().chunks(picks.len().div_ceil(8)).map(|c| c.to_vec()).collect();
        let hs: Vec<_> = chunks.into_iter().map(|c| s.spawn(move || c.into_iter().map(|i| (i, mesh(i))).collect::<Vec<_>>())).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let differing = second.iter().filter(|(i, b)| &first[picks.iter().position(|p| p == i).unwrap()] != b).count();
    assert!(first.iter().filter(|b| !b.is_empty()).count() > 5, "some lettered signs in the sample");
    assert_eq!(differing, 0);
}

/// Every lettered vertex: a cell of the list, codes in range, on the panel
/// finish, one byte list a vertex; the sample carries every mode and words
/// in all four faces.
#[test]
fn every_lettered_vertex_names_a_cell_of_the_list() {
    let kc = city();
    let cells = clutter::letter_cells();
    let mut modes = [0usize; 4];
    let mut faces = [0usize; 4];
    let mut phrases: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut signs, mut lettered, mut neon) = (0u32, 0u32, 0u32);
    for i in (0..kc.records.len()).step_by(23) {
        let Some(p) = planned(i) else { continue };
        let c = clutter::plan(&p, 1).census;
        signs += c.sign_faces + c.blade_signs;
        lettered += c.lettered;
        neon += c.neon_letters;
        for rung in [0, 2] {
            let m = build_rung_with(&p, rung, 1);
            if m.letters.is_empty() {
                continue;
            }
            assert_eq!(m.letters.len(), m.groups.len(), "#{i}: a list a group");
            for (l, (name, _, sink)) in m.letters.iter().zip(&m.groups) {
                if l.is_empty() {
                    continue;
                }
                assert_eq!(name, "painted-white", "#{i}: lettered on the panel finish");
                assert_eq!(l.len(), sink.positions.len(), "#{i}: one code a vertex");
                for (b, n) in l.iter().zip(&sink.normals) {
                    let (code, _, _) = LetterCode::decode(*b).unwrap_or_else(|| panic!("#{i}: {b:?} decodes"));
                    assert!((code.cell as usize) < cells.len() && code.mode < 4 && code.query < 3 && code.ink < 6 && (code.field as usize) < clutter::SIGN_FIELD_COUNT && code.emblem < 8, "#{i}: {code:?}");
                    assert!(n[1].abs() < 1e-6 || code.mode == clutter::LETTER_PAINTED, "#{i}: a lettered face is vertical");
                    if rung == 0 && code.mode == clutter::LETTER_PAINTED && b[0] == 0 && b[1] == 0 {
                        continue; // a painted edge
                    }
                    if rung == 0 {
                        modes[code.mode as usize] += 1;
                        faces[cells[code.cell as usize].face as usize] += 1;
                        *phrases.entry(cells[code.cell as usize].phrase).or_default() += 1;
                    }
                }
            }
        }
    }
    eprintln!("signs {signs}, lettered {lettered}, neon {neon}; vertices by mode {modes:?}, by face {faces:?}; {} phrases: {phrases:?}", phrases.len());
    assert!(lettered as f64 > 0.8 * signs as f64, "most signs carry words: {lettered} of {signs}");
    assert!(neon > 0 && modes.iter().all(|m| *m > 0) && faces.iter().all(|f| *f > 0), "{modes:?} {faces:?}");
    assert!(phrases.len() > 20, "{} phrases", phrases.len());
}
