//! What a build made of the city: counts of chains built and held (by
//! reason), kilometres by class, the least clearance and the steepest grade
//! achieved. Deterministic: the same inputs give the same census.

use std::collections::BTreeMap;
use std::fmt;

use crate::chains::Gathered;
use crate::{BentRule, Chain, ChainKind, RoadClass, Status};

/// One extreme value and where it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extreme {
    pub value: f64,
    /// The limit it is measured against (the clearance asked, the grade
    /// allowed; 0 where there is none).
    pub limit: f64,
    pub chain: u32,
    pub what: String,
}

/// Kilometres of one class: built (solved), held, landmark decks, tunnels
/// solved but off (`--tunnels`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Km {
    pub built: f64,
    pub held: f64,
    pub landmark: f64,
    pub tunnel_off: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Census {
    /// Separated vehicle segments read from the street plan.
    pub segments: usize,
    /// Of those, with no road of their way id in the city.
    pub unjoined: usize,
    /// Of those, with no usable points or width.
    pub degenerate: usize,
    /// Of those, carried (wholly or in part) by a landmark deck.
    pub landmark_segments: usize,
    pub chains: usize,
    /// Chains built by the solve (landmark decks not counted).
    pub built: usize,
    pub landmark: usize,
    pub held: usize,
    pub tunnels_off: usize,
    /// Held chains by reason.
    pub held_by_reason: BTreeMap<String, usize>,
    /// Kilometres by class name.
    pub km: BTreeMap<String, Km>,
    /// [built, held] chains by kind.
    pub kinds: BTreeMap<String, [usize; 2]>,
    /// Chain ends by what they meet (every chain).
    pub ends: BTreeMap<String, usize>,
    /// Bents on built decks, their spans (ends and bents), longest span.
    pub bents: usize,
    pub spans: usize,
    pub mean_span_m: f64,
    pub longest_span_m: f64,
    /// Crossings under built decks, and those under the clearance asked.
    pub crossings: usize,
    pub clearance_short: usize,
    /// The least clearance achieved over a crossing of a built deck.
    pub min_clearance: Option<Extreme>,
    /// The steepest step of a built chain, and steps over their limit.
    pub steepest: Option<Extreme>,
    pub grade_over: usize,
    /// The highest built deck above the ground under its centreline.
    pub highest_above_ground: Option<Extreme>,
    /// Same-level crossings with no shared node (each holds its deck).
    pub unknown_crossings: usize,
    /// Bents on built decks by the rule that placed them.
    pub bents_by_rule: BTreeMap<String, usize>,
    /// Columns on built decks by where they stand.
    pub columns_by_site: BTreeMap<String, usize>,
    /// Columns in a street's parking lane, furniture zone or median, by
    /// street name (`<name> (<run id>)`), for A6's parking and props.
    pub street_columns: BTreeMap<String, usize>,
    /// Bents with no column: the superstructure sits on its approach fill there.
    pub fill_bents: usize,
    /// Bents shared by two parallel decks (stage A5): lines whose columns and
    /// cap carry both (counted once, on the deck that owns the columns).
    pub shared_bents: usize,
    /// Deck chains whose bents the A1b rules left a span too long for and
    /// stage A5's relaxed rule (junctions by the drawn street, barrier
    /// medians, shared bents) placed instead.
    pub relaxed_bent_chains: usize,
    /// Crossings not asked because the piece under the deck is the same
    /// route running along it (route.rs).
    pub same_route_ignored: usize,
    /// Drawn chains with a piece whose section came from the fallback rule
    /// (no lane layout in the street data).
    pub section_fallbacks: usize,
    /// Stage A6: what the street joins did (only from `build_joined` with joins).
    pub joins: Option<crate::joins::JoinStats>,
    /// Stage A9: retaining walls where the joined street meets the structures
    /// (0 without the joins).
    pub walls: usize,
    /// Outer solve passes (each holds what the last could not meet) and
    /// rounds of the last pass.
    pub passes: usize,
    pub rounds: usize,
}

fn label(c: &Chain) -> String {
    let name = if c.name().is_empty() { "unnamed" } else { c.name() };
    format!("{name} (chain {}, {} level {}, ways {})", c.id, c.class.name(), c.level, c.ways().join(","))
}

impl Census {
    pub(crate) fn of(chains: &[Chain], g: &Gathered, unknown: usize, passes: usize, rounds: usize) -> Self {
        let mut c = Census {
            segments: g.segments,
            unjoined: g.unjoined,
            degenerate: g.degenerate,
            landmark_segments: g.landmark,
            chains: chains.len(),
            unknown_crossings: unknown,
            passes,
            rounds,
            ..Default::default()
        };
        for class in RoadClass::ALL {
            c.km.insert(class.name().to_string(), Km::default());
        }
        for rule in BentRule::ALL {
            c.bents_by_rule.insert(rule.name().to_string(), 0);
        }
        let mut span_sum = 0.0;
        for ch in chains {
            let km = c.km.get_mut(ch.class.name()).unwrap();
            let len = ch.length() / 1000.0;
            let kind = match ch.kind {
                ChainKind::Deck => "deck",
                ChainKind::Below => "below",
                ChainKind::Tunnel => "tunnel",
                ChainKind::Landmark => "landmark",
            };
            for e in &ch.ends {
                *c.ends.entry(e.kind.name().to_string()).or_default() += 1;
            }
            match &ch.status {
                Status::Built if ch.kind == ChainKind::Landmark => {
                    c.landmark += 1;
                    km.landmark += len;
                }
                Status::Built => {
                    c.built += 1;
                    km.built += len;
                    c.kinds.entry(kind.to_string()).or_default()[0] += 1;
                }
                Status::Held(h) => {
                    c.held += 1;
                    km.held += len;
                    c.kinds.entry(kind.to_string()).or_default()[1] += 1;
                    *c.held_by_reason.entry(h.code().to_string()).or_default() += 1;
                }
                Status::TunnelOff => {
                    c.tunnels_off += 1;
                    km.tunnel_off += len;
                }
            }
            if ch.status != Status::Built || ch.kind == ChainKind::Landmark || ch.heights.len() != ch.stations.len() {
                continue;
            }
            if ch.kind == ChainKind::Deck {
                for b in &ch.supports {
                    *c.bents_by_rule.entry(b.rule.name().to_string()).or_default() += 1;
                    if b.columns.is_empty() && b.twin.is_none_or(|t| !t.carried) {
                        c.fill_bents += 1;
                    }
                    if b.twin.is_some_and(|t| !t.carried) {
                        c.shared_bents += 1;
                    }
                    for col in &b.columns {
                        *c.columns_by_site.entry(col.site.name().to_string()).or_default() += 1;
                        if let Some(st) = &col.street {
                            let name = if st.name.is_empty() { st.id.clone() } else { format!("{} ({})", st.name, st.id) };
                            *c.street_columns.entry(name).or_default() += 1;
                        }
                    }
                }
                c.bents += ch.bents.len();
                let mut edges = vec![0.0];
                edges.extend(&ch.bents);
                edges.push(ch.length());
                for w in edges.windows(2) {
                    c.spans += 1;
                    span_sum += w[1] - w[0];
                    c.longest_span_m = c.longest_span_m.max(w[1] - w[0]);
                }
                for x in &ch.crossings {
                    // a crossing of a lower chain that was not built clears nothing.
                    if !x.achieved.is_finite() {
                        continue;
                    }
                    if x.chain.is_some_and(|d| chains[d as usize].status != Status::Built) {
                        continue;
                    }
                    c.crossings += 1;
                    if x.achieved < x.clearance - 1e-6 {
                        c.clearance_short += 1;
                    }
                    if c.min_clearance.as_ref().is_none_or(|m| x.achieved < m.value) {
                        c.min_clearance = Some(Extreme { value: x.achieved, limit: x.clearance, chain: ch.id, what: format!("{} over {}", label(ch), x.over) });
                    }
                }
            }
            for i in 0..ch.stations.len() - 1 {
                let ds = ch.stations[i + 1] - ch.stations[i];
                if ds <= 0.0 {
                    continue;
                }
                let g = (ch.heights[i + 1] - ch.heights[i]).abs() / ds;
                let limit = ch.grade_limits.get(i).copied().unwrap_or(f64::NAN);
                if g > limit + 1e-3 {
                    c.grade_over += 1;
                }
                if c.steepest.as_ref().is_none_or(|m| g > m.value) {
                    c.steepest = Some(Extreme { value: g, limit, chain: ch.id, what: label(ch) });
                }
            }
        }
        if c.spans > 0 {
            c.mean_span_m = span_sum / c.spans as f64;
        }
        c
    }

    /// Record the highest deck above its ground (needs the ground, which the
    /// chain does not keep).
    pub(crate) fn note_height(&mut self, value: f64, chain: &Chain) {
        if self.highest_above_ground.as_ref().is_none_or(|m| value > m.value) {
            self.highest_above_ground = Some(Extreme { value, limit: 0.0, chain: chain.id, what: label(chain) });
        }
    }
}

impl fmt::Display for Census {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "segments: {} separated vehicle segments ({} not joined to a road, {} degenerate, {} on a landmark deck)", self.segments, self.unjoined, self.degenerate, self.landmark_segments)?;
        writeln!(f, "chains: {} ({} built, {} landmark decks, {} held, {} tunnels off)", self.chains, self.built, self.landmark, self.held, self.tunnels_off)?;
        for (k, [b, h]) in &self.kinds {
            writeln!(f, "  {k}: {b} built, {h} held")?;
        }
        writeln!(f, "held by reason:")?;
        for (k, v) in &self.held_by_reason {
            writeln!(f, "  {k}: {v}")?;
        }
        writeln!(f, "km by class (built / held / landmark / tunnel off):")?;
        let mut tot = Km::default();
        for (k, v) in &self.km {
            writeln!(f, "  {k:14} {:7.2} {:7.2} {:7.2} {:7.2}", v.built, v.held, v.landmark, v.tunnel_off)?;
            tot.built += v.built;
            tot.held += v.held;
            tot.landmark += v.landmark;
            tot.tunnel_off += v.tunnel_off;
        }
        writeln!(f, "  {:14} {:7.2} {:7.2} {:7.2} {:7.2}", "all", tot.built, tot.held, tot.landmark, tot.tunnel_off)?;
        let ends: Vec<String> = self.ends.iter().map(|(k, v)| format!("{k} {v}")).collect();
        writeln!(f, "chain ends: {}", ends.join(", "))?;
        writeln!(f, "bents: {} on built decks; {} spans, mean {:.1} m, longest {:.1} m", self.bents, self.spans, self.mean_span_m, self.longest_span_m)?;
        let rules: Vec<String> = self.bents_by_rule.iter().map(|(k, v)| format!("{k} {v}")).collect();
        writeln!(f, "bents by rule: {}; {} on approach fill (no column)", rules.join(", "), self.fill_bents)?;
        writeln!(f, "bents placed by the relaxed rule on {} chains; {} shared by twin decks", self.relaxed_bent_chains, self.shared_bents)?;
        let sites: Vec<String> = self.columns_by_site.iter().map(|(k, v)| format!("{k} {v}")).collect();
        writeln!(f, "columns by site: {}", sites.join(", "))?;
        if !self.street_columns.is_empty() {
            let total: usize = self.street_columns.values().sum();
            writeln!(f, "columns standing in a street ({total}, by street):")?;
            for (k, v) in &self.street_columns {
                writeln!(f, "  {v:3}  {k}")?;
            }
        }
        writeln!(f, "crossings under built decks: {} ({} under the clearance asked); {} same-route overlaps not crossed", self.crossings, self.clearance_short, self.same_route_ignored)?;
        if let Some(m) = &self.min_clearance {
            writeln!(f, "minimum clearance: {:.3} m (asked {:.3} m) at {}", m.value, m.limit, m.what)?;
        }
        if let Some(m) = &self.steepest {
            writeln!(f, "steepest grade: {:.4} (limit {:.4}) on {}; {} steps over their limit", m.value, m.limit, m.what, self.grade_over)?;
        }
        if let Some(m) = &self.highest_above_ground {
            writeln!(f, "highest deck above its ground: {:.1} m on {}", m.value, m.what)?;
        }
        writeln!(f, "unknown same-level crossings: {}", self.unknown_crossings)?;
        writeln!(f, "drawn chains with a fallback section: {}", self.section_fallbacks)?;
        write!(f, "solve: {} passes, {} rounds in the last", self.passes, self.rounds)?;
        if let Some(j) = &self.joins {
            write!(f, "\n{j}")?;
        }
        if self.walls > 0 {
            write!(f, "\nretaining walls where the joined street meets the structures: {}", self.walls)?;
        }
        Ok(())
    }
}
