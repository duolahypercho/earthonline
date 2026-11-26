//! (`--no-deck-ground`) Roof decks the bare-earth survey keeps as ground, brought
//! down to the streets' grade (Docs/HILLS-PLAN.md stage H4).
//!
//! The USGS bare-earth model takes some roofs for ground. Moscone Center
//! South's roof (Yerba Buena Gardens: the children's garden, the carousel, the
//! ice rink) stands 5-9 m over 4th Street and Folsom Street in it, and the 8 m
//! grid draws it as an earth mound whose banks run 25 m into the streets.
//! `native/assets/sf-decks-v1.json` (eo-hills `cook decks`) lists such decks
//! and, for each, the grid's cells over it and round it with their new
//! heights: the harmonic fill of the cells round them, so the block meets its
//! streets at their own grade. [`ElevationGrid::load`](crate::ElevationGrid::load)
//! writes them into the grid before anything reads it (the terrain, the
//! collision ground, the streets, the planting), so all of them agree.
//!
//! The table is refused whole (a warning, the grid as surveyed) if its grid's
//! version, frame or any cell's old height differs from the shipped grid's.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::sync::OnceLock;

/// The table under `native/assets/` (`crate::roof_colours::native_assets`).
pub const FILE: &str = "sf-decks-v1.json";
/// A cell's old height may differ from the grid's by this much (cm, rounding).
const OLD_TOLERANCE_CM: f64 = 1.0;

/// The switch.
pub fn switch_on() -> bool {
    !std::env::args().any(|a| a == "--no-deck-ground")
}

#[derive(Deserialize)]
struct Doc {
    meta: Meta,
    decks: Vec<DeckRec>,
}

#[derive(Deserialize)]
struct Meta {
    version: String,
    grid: GridRec,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GridRec {
    version: String,
    origin_x: f64,
    origin_z: f64,
    cell_size: f64,
    width: usize,
    height: usize,
}

#[derive(Deserialize)]
struct DeckRec {
    name: String,
    /// [i, j, old, new]: the cell (i east, j south), its surveyed and its new height (NAVD88 cm).
    cells: Vec<[i64; 4]>,
}

/// One deck: its name and its cells (row-major index into the grid, new NAVD88 cm).
#[derive(Clone, Debug)]
pub struct Deck {
    pub name: String,
    pub cells: Vec<(usize, usize, i64, i64)>,
}

/// The table as read (not yet checked against a grid).
#[derive(Clone, Debug)]
pub struct Table {
    /// FNV-1a 64 of the file's bytes (the ground caches' key, [`cache_tag`]).
    pub digest: u64,
    pub grid_version: String,
    pub origin: [f64; 2],
    pub cell_size: f64,
    pub width: usize,
    pub height: usize,
    pub decks: Vec<Deck>,
}

impl Table {
    pub fn parse(bytes: &[u8]) -> Result<Table> {
        let d: Doc = serde_json::from_slice(bytes).context("parsing the deck table")?;
        ensure!(d.meta.version == "sf-decks-v1", "deck table: version {}, expected sf-decks-v1", d.meta.version);
        let g = d.meta.grid;
        let mut decks = vec![];
        for r in d.decks {
            let mut cells = vec![];
            for c in r.cells {
                ensure!(c[0] >= 0 && c[1] >= 0 && (c[0] as usize) < g.width && (c[1] as usize) < g.height, "deck {}: cell ({}, {}) off the grid", r.name, c[0], c[1]);
                cells.push((c[0] as usize, c[1] as usize, c[2], c[3]));
            }
            decks.push(Deck { name: r.name, cells });
        }
        let digest = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
        Ok(Table { digest, grid_version: g.version, origin: [g.origin_x, g.origin_z], cell_size: g.cell_size, width: g.width, height: g.height, decks })
    }

    /// Every cell of every deck (i, j).
    pub fn cells(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.decks.iter().flat_map(|d| d.cells.iter().map(|c| (c.0, c.1)))
    }
}

/// The table, read once (None: switched off, missing or unreadable).
pub fn table() -> Option<&'static Table> {
    static T: OnceLock<Option<Table>> = OnceLock::new();
    T.get_or_init(|| {
        if !switch_on() {
            return None;
        }
        let p = crate::roof_colours::native_assets().join(FILE);
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("deck ground: {}: {e}; the ground as surveyed", p.display());
                return None;
            }
        };
        match Table::parse(&bytes) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("deck ground: {}: {e:#}; the ground as surveyed", p.display());
                None
            }
        }
    })
    .as_ref()
}

/// What the ground's caches add to their keys (`crate::groundscale::cache_tag`): the
/// table's name and digest while it is read; empty with `--no-deck-ground` or no
/// table, so the keys are as before.
pub fn cache_tag() -> String {
    table().map_or(String::new(), |t| format!("decks sf-decks-v1 {:016x}", t.digest))
}

/// Writes the table's heights into a grid as decoded (the surveyed frame,
/// before `ElevationGrid::load` scales it). Refuses the whole table (Err, the
/// grid untouched) when its grid is not this one.
pub fn apply(t: &Table, g: &mut crate::ElevationGrid) -> Result<usize> {
    let m = &g.meta;
    ensure!(t.grid_version == m.version, "deck table for grid {}, the grid is {}", t.grid_version, m.version);
    ensure!(
        (t.origin[0] - m.origin_x).abs() < 1e-6 && (t.origin[1] - m.origin_z).abs() < 1e-6 && (t.cell_size - m.cell_size).abs() < 1e-9 && t.width == m.width && t.height == m.height,
        "deck table's grid frame differs from the grid's"
    );
    let cm = |y: f32| (y as f64 - crate::NAVD88_TO_GAME_Y) * 100.0;
    for d in &t.decks {
        for &(i, j, old, _) in &d.cells {
            let y = g.lidar[j * m.width + i];
            ensure!(!y.is_nan() && (cm(y) - old as f64).abs() <= OLD_TOLERANCE_CM, "deck {}: cell ({i}, {j}) is {:.0} cm in the grid, {old} cm in the table", d.name, cm(y));
        }
    }
    let mut n = 0;
    for d in &t.decks {
        for &(i, j, _, new) in &d.cells {
            g.lidar[j * m.width + i] = (new as f64 / 100.0 + crate::NAVD88_TO_GAME_Y) as f32;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> (Table, crate::ElevationGrid) {
        let p = crate::roof_colours::native_assets().join(FILE);
        let t = Table::parse(&std::fs::read(&p).expect("the deck table")).expect("parses");
        let paths = crate::DataPaths::discover().expect("paths");
        let g = crate::ElevationGrid::decode(&crate::read_maybe_gzip(&paths.sf("sf-elevation.bin.gz")).unwrap()).unwrap();
        (t, g)
    }

    #[test]
    fn the_table_fits_the_shipped_grid_and_lowers_moscone_south() {
        let (t, mut g) = shipped();
        let frame = crate::roof_colours::PlanFrame::from_meta(&crate::City::load_at(&crate::DataPaths::discover().unwrap(), 1.0).unwrap().meta);
        // the children's garden on Moscone Center South's roof, between 4th, Howard, 3rd and Folsom
        let (x, z) = frame.to_plan(-122.40121, 37.78325);
        let before = g.sample_surveyed(x, z).unwrap();
        let n = apply(&t, &mut g).expect("the table fits the shipped grid");
        assert!(n > 100, "{n} cells");
        let after = g.sample_surveyed(x, z).unwrap();
        // the 4th Street sidewalk is NAVD88 6-8 m; the deck 13 m
        assert!(before - after > 4.0, "the deck lowered by {:.2} m only ({before:.2} to {after:.2})", before - after);
        let navd = after - crate::NAVD88_TO_GAME_Y;
        assert!((4.0..9.0).contains(&navd), "the block's ground at NAVD88 {navd:.2} m, not the streets' grade");
        // no cell raised
        for d in &t.decks {
            for c in &d.cells {
                assert!(c.3 <= c.2, "deck {}: cell ({}, {}) raised {} cm", d.name, c.0, c.1, c.3 - c.2);
            }
        }
    }

    #[test]
    fn a_table_for_another_grid_is_refused_whole() {
        let (mut t, mut g) = shipped();
        let before = g.lidar.clone();
        t.decks[0].cells[0].2 += 50;
        assert!(apply(&t, &mut g).is_err());
        assert!(g.lidar.iter().zip(&before).all(|(a, b)| a.to_bits() == b.to_bits()), "a refused table touched the grid");
        let (mut t2, mut g2) = shipped();
        t2.width += 1;
        assert!(apply(&t2, &mut g2).is_err());
    }
}
