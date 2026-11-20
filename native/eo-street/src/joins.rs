//! Where the grade-separated roads (eo-structures' built chains) meet the
//! paved network: plain data a host hands in through
//! `StreetOptions::structures`. This crate does not depend on eo-structures;
//! eo-structures computes the joins from a street world built without them
//! (`eo_structures::StructureWorld::street_joins`), and the host builds the
//! street world again with them.
//!
//! With joins (and only then) the plan:
//!
//! * gives a structure end that lands where two or more paved runs end (a
//!   street passing through, or a junction built without it) a leg in the
//!   junction there: an approach from a run that is not drawn
//!   (`Entry::structure_leg`), so the junction has a mouth for the structure:
//!   no kerb or footway across it, the corners joined to it, no crosswalk or
//!   limit line painted on it (the structure's own deck is there);
//! * closes the open end of a stub (one paved run ending at a structure end,
//!   no junction) with a cap face over its kerbs, footways, frontage and
//!   banks beyond what the structure's own end face closes
//!   (`Entry::structure_ends`, `StructureEnd::closed`);
//! * raises a stub's datum toward its end on a ramp profile where the
//!   structure asks a higher end (`StructureEnd::rise`: an approach
//!   embankment, its sides the street's own banks), over the stub and the
//!   runs it continues into end to end (`Entry::structure_ramps`), only
//!   where the ramp is back at the street datum before any junction;
//!   likewise lowers it under a cone rising from a tunnel portal
//!   (`StructureEnd::cut`), so the street meets the portal down in its cut,
//!   the ground carpet held under it there (`StreetWorld::terrain_cap`).
//!
//! Without joins (`None`, the default) nothing here runs and the street is
//! drawn exactly as before.

use crate::plan::P2;

/// The joins a host hands in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructureJoins {
    pub ends: Vec<StructureEnd>,
}

/// One structure end on the paved network.
#[derive(Clone, Debug, PartialEq)]
pub struct StructureEnd {
    /// The structure's end point (plan, ground-scaled, as the street plan).
    pub at: P2,
    /// Its centreline from `at` outward (`at` first), long enough for a
    /// junction mouth (a few tens of metres; the whole structure if shorter).
    pub line: Vec<P2>,
    /// Its roadway's half width at the end.
    pub half: f64,
    /// Half the width its own end face closes across a stub's end (the
    /// roadway and its barriers): a stub's cap covers what lies beyond.
    pub closed: f64,
    /// The separated street segment (`StreetCity::segments` index) of the
    /// structure's end piece: the leg's class, lanes and direction.
    pub segment: usize,
    /// The height (game frame) the paved approach should reach at `at`, and
    /// the grade it may climb at (the structure's own class limit, read from
    /// the standards table by the host); None keeps the street datum.
    pub rise: Option<Rise>,
    /// The other way: the stub's datum held under a cone rising from `to` at
    /// `at` at `grade` (a tunnel's approach cut: the street's last metres
    /// follow the cut down to the portal rather than climb the bare-earth
    /// grid's hillside over it); None keeps the street datum.
    pub cut: Option<Rise>,
}

/// An approach embankment's ramp: the datum climbs at `grade` to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rise {
    pub to: f64,
    pub grade: f64,
}

impl StructureJoins {
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// A digest of everything here (64-bit FNV-1a over the values' bits),
    /// for cache keys that depend on the street surface (the kit tiles' floors
    /// and skirts stand on it).
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        eat(self.ends.len() as u64);
        for e in &self.ends {
            for v in [e.at[0], e.at[1], e.half, e.closed] {
                eat(v.to_bits());
            }
            eat(e.segment as u64);
            eat(e.line.len() as u64);
            for p in &e.line {
                eat(p[0].to_bits());
                eat(p[1].to_bits());
            }
            for r in [e.rise, e.cut] {
                match r {
                    Some(r) => {
                        eat(1);
                        eat(r.to.to_bits());
                        eat(r.grade.to_bits());
                    }
                    None => eat(0),
                }
            }
        }
        h
    }
}

/// The datum a run with a cut at its end draws at distance `d` from that
/// end, under its own datum `datum`: the cone's height there or the datum,
/// whichever is lower.
pub fn cut_datum(datum: f64, cut: &Rise, d: f64) -> f64 {
    datum.min(cut.to + cut.grade * d.max(0.0))
}

/// The datum a run with a rise at its end draws at distance `d` from that
/// end, over its own datum `datum`: the ramp's height there or the datum,
/// whichever is higher.
pub fn ramp_datum(datum: f64, rise: &Rise, d: f64) -> f64 {
    datum.max(rise.to - rise.grade * d.max(0.0))
}
