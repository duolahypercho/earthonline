//! Snapshots: what a client is told about the world, quantised, delta
//! compressed against the newest snapshot it confirmed, and fitted to a
//! bandwidth budget.
//!
//! ENTITIES. Everything replicated is an entity with a 32-bit net id whose
//! top two bits are its class (simulated car, walker, player, player's car)
//! and a quantised state (`QState`): position in 1/128 m (absolute, in the
//! zone's frame), yaw in 2 pi / 65536, speed in 1/64 m/s, a one-byte
//! animation phase, kind and flags as the simulation writes them. Physical
//! cars carry pitch and roll, and player cars also carry wheel steer. Cars
//! without a surface height and walkers are grounded by the client.
//!
//! DELTA. The server keeps, per client, what the client holds after each of
//! its last `HISTORY` snapshots. A snapshot names its baseline (the newest
//! snapshot the client acknowledged) and lists records sorted by id: NEW
//! (full state), UPDATE (changed fields as small signed differences) or
//! REMOVE. An entity of the baseline not mentioned is unchanged. Ids are
//! gap coded; numbers go in two-bit size classes (`bits::uvar`).
//!
//! BUDGET. Removals and players always go; the other records are taken by a
//! priority accumulator (nearer and faster first, starved ones rising) until
//! the snapshot's share of the client's bytes per second is used. A record
//! left out keeps the entity at its baseline state for the client and its
//! priority climbs, so under pressure far things update less often instead
//! of the snapshot growing.

use crate::bits::{BitReader, BitWriter, Truncated};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

pub const CLASS_CAR: u32 = 0;
pub const CLASS_WALKER: u32 = 1;
pub const CLASS_PLAYER: u32 = 2;
pub const CLASS_PLAYER_CAR: u32 = 3;

/// A fleet car's pose and tilt come from server physics, including its datum y.
pub const TRAFFIC_PHYSICAL: u16 = 0x2000;

#[inline]
pub fn net_id(class: u32, local: u32) -> u32 {
    (class << 30) | (local & 0x3FFF_FFFF)
}

#[inline]
pub fn class_of(id: u32) -> u32 {
    id >> 30
}

/// Metres per position unit.
pub const POS_UNIT: f64 = 1.0 / 128.0;
/// m/s per speed unit.
pub const SPEED_UNIT: f64 = 1.0 / 64.0;
/// The client's snapshots kept on the server (baselines older than this fall back to a full snapshot).
pub const HISTORY: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QState {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub yaw: u16,
    /// Physical cars: pitch (low 16 bits) and roll (high 16), 2 pi / 65536 each.
    pub tilt: u32,
    pub speed: i16,
    pub anim: u8,
    pub kind: u8,
    pub flags: u16,
    pub steer: i8,
}

pub fn q_pos(m: f64) -> i32 {
    (m / POS_UNIT).round() as i32
}
pub fn q_angle(a: f64) -> u16 {
    crate::wire::quantize_yaw(a)
}
pub fn q_speed(v: f64) -> i16 {
    (v / SPEED_UNIT).round().clamp(-32767.0, 32767.0) as i16
}

impl QState {
    pub fn pos(&self) -> [f64; 3] {
        [self.x as f64 * POS_UNIT, self.y as f64 * POS_UNIT, self.z as f64 * POS_UNIT]
    }
    pub fn yaw_rad(&self) -> f64 {
        crate::wire::yaw_of(self.yaw)
    }
    pub fn speed_mps(&self) -> f64 {
        self.speed as f64 * SPEED_UNIT
    }
    pub fn pitch_roll(&self) -> (f64, f64) {
        (crate::wire::yaw_of(self.tilt as u16), crate::wire::yaw_of((self.tilt >> 16) as u16))
    }
}

const F_XZ: u32 = 1;
const F_Y: u32 = 2;
const F_YAW: u32 = 4;
const F_TILT: u32 = 8;
const F_SPEED: u32 = 16;
const F_ANIM: u32 = 32;
const F_KIND: u32 = 64;
const F_FLAGS: u32 = 128;
const F_STEER: u32 = 256;
const FIELDS: u32 = 9;

const OP_UPDATE: u32 = 0;
const OP_NEW: u32 = 1;
const OP_REMOVE: u32 = 2;

fn diff_mask(a: &QState, b: &QState) -> u32 {
    let mut m = 0;
    if a.x != b.x || a.z != b.z {
        m |= F_XZ;
    }
    if a.y != b.y {
        m |= F_Y;
    }
    if a.yaw != b.yaw {
        m |= F_YAW;
    }
    if a.tilt != b.tilt {
        m |= F_TILT;
    }
    if a.speed != b.speed {
        m |= F_SPEED;
    }
    if a.anim != b.anim {
        m |= F_ANIM;
    }
    if a.kind != b.kind {
        m |= F_KIND;
    }
    if a.flags != b.flags {
        m |= F_FLAGS;
    }
    if a.steer != b.steer {
        m |= F_STEER;
    }
    m
}

#[inline]
fn svar_bits(v: i32) -> u32 {
    let u = ((v << 1) ^ (v >> 31)) as u32;
    2 + if u < 1 << crate::bits::CLASSES[0] {
        crate::bits::CLASSES[0]
    } else if u < 1 << crate::bits::CLASSES[1] {
        crate::bits::CLASSES[1]
    } else if u < 1 << crate::bits::CLASSES[2] {
        crate::bits::CLASSES[2]
    } else {
        32
    }
}

fn wrap16(a: u16, b: u16) -> i32 {
    a.wrapping_sub(b) as i16 as i32
}

fn write_update(w: &mut BitWriter, base: &QState, cur: &QState, mask: u32) {
    w.write(mask, FIELDS);
    if mask & F_XZ != 0 {
        w.svar(cur.x.wrapping_sub(base.x));
        w.svar(cur.z.wrapping_sub(base.z));
    }
    if mask & F_Y != 0 {
        w.svar(cur.y.wrapping_sub(base.y));
    }
    if mask & F_YAW != 0 {
        w.svar(wrap16(cur.yaw, base.yaw));
    }
    if mask & F_TILT != 0 {
        w.svar(wrap16(cur.tilt as u16, base.tilt as u16));
        w.svar(wrap16((cur.tilt >> 16) as u16, (base.tilt >> 16) as u16));
    }
    if mask & F_SPEED != 0 {
        w.svar(cur.speed as i32 - base.speed as i32);
    }
    if mask & F_ANIM != 0 {
        w.svar(cur.anim.wrapping_sub(base.anim) as i8 as i32);
    }
    if mask & F_KIND != 0 {
        w.write(cur.kind as u32, 8);
    }
    if mask & F_FLAGS != 0 {
        w.write(cur.flags as u32, 16);
    }
    if mask & F_STEER != 0 {
        w.svar(cur.steer as i32 - base.steer as i32);
    }
}

fn update_bits(base: &QState, cur: &QState, mask: u32) -> u32 {
    let mut b = FIELDS;
    if mask & F_XZ != 0 {
        b += svar_bits(cur.x.wrapping_sub(base.x)) + svar_bits(cur.z.wrapping_sub(base.z));
    }
    if mask & F_Y != 0 {
        b += svar_bits(cur.y.wrapping_sub(base.y));
    }
    if mask & F_YAW != 0 {
        b += svar_bits(wrap16(cur.yaw, base.yaw));
    }
    if mask & F_TILT != 0 {
        b += svar_bits(wrap16(cur.tilt as u16, base.tilt as u16)) + svar_bits(wrap16((cur.tilt >> 16) as u16, (base.tilt >> 16) as u16));
    }
    if mask & F_SPEED != 0 {
        b += svar_bits(cur.speed as i32 - base.speed as i32);
    }
    if mask & F_ANIM != 0 {
        b += svar_bits(cur.anim.wrapping_sub(base.anim) as i8 as i32);
    }
    if mask & F_KIND != 0 {
        b += 8;
    }
    if mask & F_FLAGS != 0 {
        b += 16;
    }
    if mask & F_STEER != 0 {
        b += svar_bits(cur.steer as i32 - base.steer as i32);
    }
    b
}

fn read_update(r: &mut BitReader, base: &QState) -> Result<QState, Truncated> {
    let mask = r.read(FIELDS)?;
    let mut s = *base;
    if mask & F_XZ != 0 {
        s.x = base.x.wrapping_add(r.svar()?);
        s.z = base.z.wrapping_add(r.svar()?);
    }
    if mask & F_Y != 0 {
        s.y = base.y.wrapping_add(r.svar()?);
    }
    if mask & F_YAW != 0 {
        s.yaw = base.yaw.wrapping_add(r.svar()? as u16);
    }
    if mask & F_TILT != 0 {
        let p = (base.tilt as u16).wrapping_add(r.svar()? as u16);
        let q = ((base.tilt >> 16) as u16).wrapping_add(r.svar()? as u16);
        s.tilt = p as u32 | (q as u32) << 16;
    }
    if mask & F_SPEED != 0 {
        s.speed = (base.speed as i32 + r.svar()?) as i16;
    }
    if mask & F_ANIM != 0 {
        s.anim = base.anim.wrapping_add(r.svar()? as u8);
    }
    if mask & F_KIND != 0 {
        s.kind = r.read(8)? as u8;
    }
    if mask & F_FLAGS != 0 {
        s.flags = r.read(16)? as u16;
    }
    if mask & F_STEER != 0 {
        s.steer = (base.steer as i32 + r.svar()?) as i8;
    }
    Ok(s)
}

fn write_full(w: &mut BitWriter, s: &QState) {
    w.svar(s.x);
    w.svar(s.z);
    w.svar(s.y);
    w.write(s.yaw as u32, 16);
    w.svar(s.speed as i32);
    w.write(s.anim as u32, 8);
    w.write(s.kind as u32, 8);
    w.bit(s.flags != 0);
    if s.flags != 0 {
        w.write(s.flags as u32, 16);
    }
    w.bit(s.tilt != 0 || s.steer != 0);
    if s.tilt != 0 || s.steer != 0 {
        w.write(s.tilt, 32);
        w.write(s.steer as u8 as u32, 8);
    }
}

fn full_bits(s: &QState) -> u32 {
    svar_bits(s.x) + svar_bits(s.z) + svar_bits(s.y) + 16 + svar_bits(s.speed as i32) + 16 + 1 + if s.flags != 0 { 16 } else { 0 } + 1 + if s.tilt != 0 || s.steer != 0 { 40 } else { 0 }
}

fn read_full(r: &mut BitReader) -> Result<QState, Truncated> {
    let x = r.svar()?;
    let z = r.svar()?;
    let y = r.svar()?;
    let yaw = r.read(16)? as u16;
    let speed = r.svar()? as i16;
    let anim = r.read(8)? as u8;
    let kind = r.read(8)? as u8;
    let flags = if r.bit()? { r.read(16)? as u16 } else { 0 };
    let (tilt, steer) = if r.bit()? { (r.read(32)?, r.read(8)? as u8 as i8) } else { (0, 0) };
    Ok(QState { x, y, z, yaw, tilt, speed, anim, kind, flags, steer })
}

/// The client's own player, reported in every snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Own {
    /// The newest input seq the server ran for this client.
    pub ack_input: u32,
    /// The server's body after that input (the client compares its prediction).
    pub check: crate::wire::Check,
}

/// A correction: the server's whole player state after input `seq`.
#[derive(Clone, Debug, PartialEq)]
pub struct Correction {
    pub weather: crate::weather::WeatherEpoch,
    pub id: u16,
    pub seq: u32,
    pub state: Vec<u8>,
}

/// One snapshot's header (everything but the records).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapHeader {
    pub seq: u32,
    pub baseline: u32,
    pub tick: u32,
    pub sim_time: f64,
    pub own: Own,
    pub correction: Option<Correction>,
    /// The client's own player entity (it draws its predicted body instead).
    pub you: u32,
    /// The newest `InputPacket::ping_ms` and how long the server held it (ms).
    pub echo_ms: u16,
    pub hold_ms: u16,
    /// The server's hour and weather at `tick` (`presence::SkyClock`).
    pub sky: crate::presence::SkyClock,
}

impl SnapHeader {
    fn write(&self, w: &mut BitWriter) {
        w.write(self.seq, 32);
        w.write(self.baseline, 32);
        w.write(self.tick, 32);
        w.f64(self.sim_time);
        w.write(self.echo_ms as u32, 16);
        w.write(self.hold_ms as u32, 16);
        w.write(self.own.ack_input, 32);
        for v in self.own.check.pos_mm {
            w.write(v as u32, 32);
        }
        w.bit(self.own.check.driving);
        w.write(self.you, 32);
        self.sky.write(w);
        match &self.correction {
            Some(c) => {
                w.bit(true);
                w.write(c.id as u32, 16);
                w.write(c.seq, 32);
                w.bytes(&c.weather.bytes());
                w.write(c.state.len() as u32, 16);
                w.bytes(&c.state);
            }
            None => w.bit(false),
        }
    }

    fn read(r: &mut BitReader) -> Result<Self, Truncated> {
        let seq = r.read(32)?;
        let baseline = r.read(32)?;
        let tick = r.read(32)?;
        let sim_time = r.f64()?;
        let echo_ms = r.read(16)? as u16;
        let hold_ms = r.read(16)? as u16;
        let ack_input = r.read(32)?;
        let pos_mm = [r.read(32)? as i32, r.read(32)? as i32, r.read(32)? as i32];
        let driving = r.bit()?;
        let you = r.read(32)?;
        let sky = crate::presence::SkyClock::read(r)?;
        let correction = if r.bit()? {
            let id = r.read(16)? as u16;
            let seq = r.read(32)?;
            let weather = crate::weather::WeatherEpoch::read(&r.bytes(crate::weather::WeatherEpoch::BYTES)?)?;
            let n = r.read(16)? as usize;
            Some(Correction { id, seq, weather, state: r.bytes(n)? })
        } else {
            None
        };
        Ok(SnapHeader { seq, baseline, tick, sim_time, own: Own { ack_input, check: crate::wire::Check { seq: ack_input, pos_mm, driving } }, correction, you, echo_ms, hold_ms, sky })
    }
}

/// Sorted (id, state) list: what a client holds after a snapshot.
pub type EntitySet = Arc<Vec<(u32, QState)>>;

/// What the server wants a client to hold now (interest-filtered, sorted by id).
#[derive(Clone, Copy, Debug)]
pub struct Desired {
    pub id: u32,
    pub state: QState,
    /// Priority weight this snapshot (players: very high).
    pub weight: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BuildStats {
    pub seq: u32,
    pub bytes: usize,
    pub records: usize,
    pub new: usize,
    pub updated: usize,
    pub removed: usize,
    /// Updates or new entities left for a later snapshot by the budget.
    pub deferred: usize,
    pub entities: usize,
    pub full: bool,
}

/// One client's replication state on the server.
pub struct Replicator {
    history: VecDeque<(u32, EntitySet)>,
    acked: u32,
    next_seq: u32,
    priority: HashMap<u32, f32>,
    scratch: Vec<(u32, QState)>,
}

impl Default for Replicator {
    fn default() -> Self {
        Self::new()
    }
}

/// Header and record count bits are always paid.
const HEADER_BITS: u32 = 32 * 3 + 64 + 32 + 32 + 96 + 1 + 32 + 1 + 16;

impl Replicator {
    pub fn new() -> Self {
        Self { history: VecDeque::new(), acked: 0, next_seq: 1, priority: HashMap::new(), scratch: vec![] }
    }

    /// The client says it decoded snapshot `seq`.
    pub fn ack(&mut self, seq: u32) {
        if seq > self.acked && seq < self.next_seq {
            self.acked = seq;
        }
    }

    pub fn acked(&self) -> u32 {
        self.acked
    }

    /// Does the client hold (or is it being sent) this entity?
    pub fn holds(&self, id: u32) -> bool {
        self.history.back().is_some_and(|h| h.1.binary_search_by_key(&id, |e| e.0).is_ok())
    }

    /// Forget everything (a reconnect: the new client holds nothing).
    pub fn reset(&mut self) {
        self.history.clear();
        self.acked = 0;
        self.priority.clear();
    }

    /// Encode the next snapshot. `desired` must be sorted by id; `budget_bytes`
    /// is this snapshot's share of the client's rate.
    pub fn build(&mut self, mut header: SnapHeader, desired: &[Desired], budget_bytes: usize) -> (Vec<u8>, BuildStats) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let baseline = self.history.iter().find(|h| h.0 == self.acked).map(|h| h.1.clone());
        header.seq = seq;
        header.baseline = if baseline.is_some() { self.acked } else { 0 };
        let empty: Vec<(u32, QState)> = vec![];
        let base: &[(u32, QState)] = baseline.as_deref().map(|v| v.as_slice()).unwrap_or(&empty);
        let mut st = BuildStats { seq, full: baseline.is_none(), ..Default::default() };
        // Candidates: (id, op, base index or MAX, desired index or MAX, bits, priority)
        struct Cand {
            id: u32,
            op: u32,
            b: usize,
            d: usize,
            bits: u32,
            prio: f32,
            must: bool,
        }
        let mut cands: Vec<Cand> = Vec::with_capacity(desired.len() + 8);
        let (mut i, mut j) = (0usize, 0usize);
        while i < base.len() || j < desired.len() {
            let bi = base.get(i).map(|e| e.0).unwrap_or(u32::MAX);
            let dj = desired.get(j).map(|e| e.id).unwrap_or(u32::MAX);
            if bi == dj {
                let mask = diff_mask(&base[i].1, &desired[j].state);
                if mask != 0 {
                    let acc = self.priority.get(&dj).copied().unwrap_or(0.0) + desired[j].weight;
                    cands.push(Cand { id: dj, op: OP_UPDATE, b: i, d: j, bits: 14 + update_bits(&base[i].1, &desired[j].state, mask), prio: acc, must: desired[j].weight >= 1000.0 });
                } else {
                    self.priority.remove(&dj);
                }
                i += 1;
                j += 1;
            } else if bi < dj {
                cands.push(Cand { id: bi, op: OP_REMOVE, b: i, d: usize::MAX, bits: 14, prio: f32::INFINITY, must: true });
                self.priority.remove(&bi);
                i += 1;
            } else {
                let acc = self.priority.get(&dj).copied().unwrap_or(0.0) + desired[j].weight;
                cands.push(Cand { id: dj, op: OP_NEW, b: usize::MAX, d: j, bits: 14 + full_bits(&desired[j].state), prio: acc, must: desired[j].weight >= 1000.0 });
                j += 1;
            }
        }
        let budget_bits = (budget_bytes as u32).saturating_mul(8).saturating_sub(HEADER_BITS + header.sky.bits() + header.correction.as_ref().map_or(0, |c| 64 + (crate::weather::WeatherEpoch::BYTES + c.state.len()) as u32 * 8));
        let mut order: Vec<usize> = (0..cands.len()).collect();
        order.sort_by(|&a, &b| (cands[b].must as u8).cmp(&(cands[a].must as u8)).then(cands[b].prio.total_cmp(&cands[a].prio)));
        let mut take = vec![false; cands.len()];
        let mut used = 0u32;
        for &k in &order {
            let c = &cands[k];
            if c.must || used + c.bits <= budget_bits {
                take[k] = true;
                used += c.bits;
            }
        }
        // Encode in id order and build what the client will hold.
        let mut w = BitWriter::with_capacity(budget_bytes + 64);
        header.write(&mut w);
        let n_records = take.iter().filter(|t| **t).count();
        w.write(n_records as u32, 16);
        self.scratch.clear();
        let mut prev_id: Option<u32> = None;
        let (mut bi, mut ci) = (0usize, 0usize);
        // Walk the baseline and the candidates (both sorted by id) together.
        loop {
            let b_id = base.get(bi).map(|e| e.0).unwrap_or(u32::MAX);
            let c_id = cands.get(ci).map(|c| c.id).unwrap_or(u32::MAX);
            if b_id == u32::MAX && c_id == u32::MAX {
                break;
            }
            if b_id < c_id {
                // Unchanged baseline entity.
                self.scratch.push(base[bi]);
                bi += 1;
                continue;
            }
            let c = &cands[ci];
            let taken = take[ci];
            if taken {
                let gap = match prev_id {
                    Some(p) => c.id - p - 1,
                    None => c.id,
                };
                if prev_id.is_none() {
                    w.write(gap, 32);
                } else {
                    w.uvar(gap);
                }
                prev_id = Some(c.id);
                w.write(c.op, 2);
                st.records += 1;
                match c.op {
                    OP_UPDATE => {
                        let cur = desired[c.d].state;
                        write_update(&mut w, &base[c.b].1, &cur, diff_mask(&base[c.b].1, &cur));
                        self.scratch.push((c.id, cur));
                        st.updated += 1;
                    }
                    OP_NEW => {
                        let cur = desired[c.d].state;
                        write_full(&mut w, &cur);
                        self.scratch.push((c.id, cur));
                        st.new += 1;
                    }
                    _ => st.removed += 1,
                }
                self.priority.remove(&c.id);
            } else {
                st.deferred += 1;
                self.priority.insert(c.id, c.prio);
                if c.op == OP_UPDATE {
                    self.scratch.push(base[c.b]);
                }
            }
            if b_id == c_id {
                bi += 1;
            }
            ci += 1;
        }
        // Entities of the baseline that were unchanged but also desired were
        // pushed above (b_id < c_id); the result is sorted by construction.
        let set: EntitySet = Arc::new(std::mem::take(&mut self.scratch));
        st.entities = set.len();
        self.history.push_back((seq, set));
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        let bytes = w.finish();
        st.bytes = bytes.len();
        (bytes, st)
    }
}

/// A decoded snapshot (client side).
#[derive(Clone, Debug)]
pub struct Decoded {
    pub header: SnapHeader,
    pub entities: EntitySet,
}

/// The client's decoded snapshots by sequence (baselines for the next ones).
#[derive(Default)]
pub struct Store {
    pub decoded: VecDeque<Arc<Decoded>>,
}

#[derive(Debug)]
pub enum DecodeError {
    Truncated,
    MissingBaseline(u32),
    Corrupt,
}

impl From<Truncated> for DecodeError {
    fn from(_: Truncated) -> Self {
        DecodeError::Truncated
    }
}

impl Store {
    pub fn newest_seq(&self) -> u32 {
        self.decoded.iter().map(|d| d.header.seq).max().unwrap_or(0)
    }

    pub fn decode(&mut self, data: &[u8]) -> Result<Arc<Decoded>, DecodeError> {
        let mut r = BitReader::new(data);
        let header = SnapHeader::read(&mut r)?;
        let empty: Vec<(u32, QState)> = vec![];
        let base_set = if header.baseline == 0 {
            None
        } else {
            Some(self.decoded.iter().find(|d| d.header.seq == header.baseline).ok_or(DecodeError::MissingBaseline(header.baseline))?.entities.clone())
        };
        let base: &[(u32, QState)] = base_set.as_deref().map(|v| v.as_slice()).unwrap_or(&empty);
        let n = r.read(16)? as usize;
        let mut out: Vec<(u32, QState)> = Vec::with_capacity(base.len() + n);
        let mut bi = 0usize;
        let mut prev: Option<u32> = None;
        for _ in 0..n {
            let id = match prev {
                None => r.read(32)?,
                Some(p) => p.checked_add(1).and_then(|x| x.checked_add(r.uvar().ok()?)).ok_or(DecodeError::Corrupt)?,
            };
            prev = Some(id);
            while bi < base.len() && base[bi].0 < id {
                out.push(base[bi]);
                bi += 1;
            }
            let in_base = bi < base.len() && base[bi].0 == id;
            match r.read(2)? {
                OP_UPDATE => {
                    if !in_base {
                        return Err(DecodeError::Corrupt);
                    }
                    out.push((id, read_update(&mut r, &base[bi].1)?));
                    bi += 1;
                }
                OP_NEW => {
                    if in_base {
                        bi += 1;
                    }
                    out.push((id, read_full(&mut r)?));
                }
                OP_REMOVE => {
                    if in_base {
                        bi += 1;
                    }
                }
                _ => return Err(DecodeError::Corrupt),
            }
        }
        out.extend_from_slice(&base[bi..]);
        let d = Arc::new(Decoded { header, entities: Arc::new(out) });
        self.decoded.push_back(d.clone());
        while self.decoded.len() > 2 * HISTORY {
            self.decoded.pop_front();
        }
        Ok(d)
    }
}

/// An entity as drawn this frame (interpolated).
#[derive(Clone, Copy, Debug, Default)]
pub struct Shown {
    pub id: u32,
    pub pos: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
    pub speed: f64,
    /// Animation phase, 0..1 of a cycle (walkers: the gait; cars: 16 m of travel).
    pub anim: f64,
    pub kind: u8,
    pub flags: u16,
    pub steer: f64,
}

fn lerp_angle(a: f64, b: f64, t: f64) -> f64 {
    let mut d = b - a;
    while d > std::f64::consts::PI {
        d -= std::f64::consts::TAU;
    }
    while d < -std::f64::consts::PI {
        d += std::f64::consts::TAU;
    }
    a + d * t
}

pub fn show(id: u32, s: &QState) -> Shown {
    let (pitch, roll) = s.pitch_roll();
    Shown { id, pos: s.pos(), yaw: s.yaw_rad(), pitch, roll, speed: s.speed_mps(), anim: s.anim as f64 / 256.0, kind: s.kind, flags: s.flags, steer: s.steer as f64 / 127.0 * 0.6 }
}

/// The world between two snapshots: `a` at tick `ta`, `b` at `tb`, at render
/// tick `t` (beyond `tb`: extrapolated along heading and speed, at most `max_ahead` ticks).
pub fn interpolate(a: &Decoded, b: &Decoded, t: f64, max_ahead: f64, out: &mut Vec<Shown>) {
    interpolate_smooth(None, a, b, None, t, max_ahead, out);
}

fn find(d: &Decoded, id: u32) -> Option<QState> {
    d.entities.binary_search_by_key(&id, |e| e.0).ok().map(|i| d.entities[i].1)
}

/// A cubic Hermite step from `p0` (tangent `m0`) to `p1` (tangent `m1`), tangents in units per span.
fn hermite(p0: f64, p1: f64, m0: f64, m1: f64, u: f64) -> f64 {
    let (u2, u3) = (u * u, u * u * u);
    (2.0 * u3 - 3.0 * u2 + 1.0) * p0 + (u3 - 2.0 * u2 + u) * m0 + (-2.0 * u3 + 3.0 * u2) * p1 + (u3 - u2) * m1
}

/// As `interpolate`, and the other players move on a C1 path: a cubic
/// Hermite through `a` and `b` whose tangents are the finite differences
/// over the snapshots either side (`pa` before `a`, `pb` after `b`; a
/// missing one is taken as a straight line), so a walker's velocity does not
/// step at every snapshot (20 a second) and its feet do not tick. Height
/// stays linear (a kerb step must not overshoot), the emote byte is taken
/// from the nearer snapshot, and an arrival by fast travel (`ARRIVED`) or a
/// jump of 40 m is drawn where it is.
pub fn interpolate_smooth(pa: Option<&Decoded>, a: &Decoded, b: &Decoded, pb: Option<&Decoded>, t: f64, max_ahead: f64, out: &mut Vec<Shown>) {
    out.clear();
    let (ta, tb) = (a.header.tick as f64, b.header.tick as f64);
    let span = (tb - ta).max(1e-6);
    let u = ((t - ta) / span).clamp(0.0, 1.0);
    let ahead = ((t - tb) / crate::proto::TICK_HZ as f64).clamp(0.0, max_ahead / crate::proto::TICK_HZ as f64);
    let ae = &a.entities;
    let mut ai = 0usize;
    for (id, sb) in b.entities.iter() {
        while ai < ae.len() && ae[ai].0 < *id {
            ai += 1;
        }
        let mut s = show(*id, sb);
        if ai < ae.len() && ae[ai].0 == *id && ahead == 0.0 {
            let sa = show(*id, &ae[ai].1);
            // The y of a vehicle without a surface reference is a placeholder.
            use eo_data::sim_source::TRAFFIC_SURFACE_Y;
            let surface_changed = class_of(*id) == CLASS_CAR
                && (sa.flags ^ sb.flags) & TRAFFIC_SURFACE_Y != 0;
            let surface_height = if sb.flags & TRAFFIC_SURFACE_Y != 0 { s.pos[1] } else { sa.pos[1] };
            let jump = (0..3).filter(|&k| k != 1 || !surface_changed)
                .map(|k| (s.pos[k] - sa.pos[k]).powi(2)).sum::<f64>().sqrt();
            if jump < 40.0 {
                for k in 0..3 {
                    s.pos[k] = sa.pos[k] + (s.pos[k] - sa.pos[k]) * u;
                }
                if surface_changed {
                    s.pos[1] = surface_height;
                }
                s.yaw = lerp_angle(sa.yaw, s.yaw, u);
                s.pitch = lerp_angle(sa.pitch, s.pitch, u);
                s.roll = lerp_angle(sa.roll, s.roll, u);
                s.speed = sa.speed + (s.speed - sa.speed) * u;
                let mut da = s.anim - sa.anim;
                if da < -0.5 {
                    da += 1.0;
                } else if da > 0.5 {
                    da -= 1.0;
                }
                s.anim = (sa.anim + da * u).rem_euclid(1.0);
                s.steer = sa.steer + (s.steer - sa.steer) * u;
                if u < 0.5 {
                    s.flags = sa.flags;
                    s.kind = sa.kind;
                }
                if class_of(*id) == CLASS_PLAYER {
                    // The emote byte is not a phase.
                    s.anim = if u < 0.5 { sa.anim } else { show(*id, sb).anim };
                    if sb.flags & crate::presence::flag::ARRIVED != 0 && sa.flags & crate::presence::flag::ARRIVED == 0 {
                        // Arrived by fast travel between the two: no glide across the city.
                        s.pos = show(*id, sb).pos;
                    } else {
                        let qa = sa.pos;
                        let qb = show(*id, sb).pos;
                        let span = tb - ta;
                        let near = |d: Option<&Decoded>, from: [f64; 3]| -> Option<([f64; 3], f64)> {
                            let d = d?;
                            let q = find(d, *id)?.pos();
                            let far = (q[0] - from[0]).hypot(q[2] - from[2]);
                            (far < 40.0).then_some((q, d.header.tick as f64))
                        };
                        // Tangents per span: (b - pa) / (tb - tpa) * span, (pb - a) / (tpb - ta) * span.
                        let m0 = match near(pa, qa) {
                            Some((q, tq)) if ta - tq > 0.5 => [(qb[0] - q[0]) / (tb - tq) * span, (qb[2] - q[2]) / (tb - tq) * span],
                            _ => [qb[0] - qa[0], qb[2] - qa[2]],
                        };
                        let m1 = match near(pb, qb) {
                            Some((q, tq)) if tq - tb > 0.5 => [(q[0] - qa[0]) / (tq - ta) * span, (q[2] - qa[2]) / (tq - ta) * span],
                            _ => [qb[0] - qa[0], qb[2] - qa[2]],
                        };
                        s.pos[0] = hermite(qa[0], qb[0], m0[0], m1[0], u);
                        s.pos[2] = hermite(qa[2], qb[2], m0[1], m1[1], u);
                    }
                }
            }
        } else if ahead > 0.0 && class_of(*id) != CLASS_PLAYER_CAR {
            s.pos[0] += s.yaw.sin() * s.speed * ahead;
            s.pos[2] += s.yaw.cos() * s.speed * ahead;
        }
        out.push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car(x: i32, z: i32, yaw: u16, speed: i16) -> QState {
        QState { x, z, yaw, speed, anim: (x & 255) as u8, ..Default::default() }
    }

    #[test]
    fn traffic_surface_height_interpolates_only_between_valid_references() {
        use eo_data::sim_source::TRAFFIC_SURFACE_Y;
        let snap = |tick, x, y, flags| Decoded {
            header: SnapHeader { tick, ..Default::default() },
            entities: vec![(net_id(CLASS_CAR, 1), QState {
                x: q_pos(x), y: q_pos(y), flags, ..Default::default()
            })].into(),
        };
        let a = snap(0, 0.0, 0.0, 0);
        let b = snap(2, 2.0, 60.0, TRAFFIC_SURFACE_Y);
        let c = snap(4, 4.0, 62.0, TRAFFIC_SURFACE_Y);
        let mut out = Vec::new();
        interpolate(&a, &b, 1.5, 0.0, &mut out);
        assert_eq!(out[0].pos, [1.5, 60.0, 0.0]);
        assert_eq!(out[0].flags & TRAFFIC_SURFACE_Y, TRAFFIC_SURFACE_Y);
        interpolate(&b, &c, 3.0, 0.0, &mut out);
        assert_eq!(out[0].pos, [3.0, 61.0, 0.0]);
    }

    #[test]
    fn delta_round_trip_with_loss_of_snapshots_and_removals() {
        let mut rep = Replicator::new();
        let mut store = Store::default();
        let mut world: Vec<(u32, QState)> = (0..200u32).map(|k| (net_id(if k % 2 == 0 { CLASS_CAR } else { CLASS_WALKER }, k * 3 + 1), car(k as i32 * 400, -(k as i32) * 300, (k * 300) as u16, 300))).collect();
        world.sort_by_key(|e| e.0);
        let mut total = 0usize;
        for step in 0..60u32 {
            for (k, e) in world.iter_mut().enumerate() {
                if k % 3 != 0 {
                    e.1.x += 64;
                    e.1.z -= 10;
                    e.1.anim = e.1.anim.wrapping_add(16);
                }
            }
            if step == 20 {
                world.retain(|e| e.0 % 7 != 0);
                world.push((net_id(CLASS_PLAYER, 5), QState { x: 9, y: 4000, z: 9, ..Default::default() }));
                world.sort_by_key(|e| e.0);
            }
            let desired: Vec<Desired> = world.iter().map(|e| Desired { id: e.0, state: e.1, weight: 1.0 }).collect();
            let (bytes, st) = rep.build(SnapHeader { tick: step, ..Default::default() }, &desired, 100_000);
            total += bytes.len();
            assert_eq!(st.deferred, 0);
            // Lose every 4th snapshot: the client acks nothing for it and the next deltas from an older baseline.
            if step % 4 == 3 {
                continue;
            }
            let d = store.decode(&bytes).expect("decode");
            assert_eq!(d.entities.as_slice(), world.as_slice(), "client holds exactly the server's set at step {step}");
            rep.ack(d.header.seq);
        }
        assert!(total < 60 * 200 * 8, "deltas are compact: {total} bytes for 60 snapshots of 200 entities");
    }

    #[test]
    fn the_budget_defers_updates_and_they_catch_up() {
        let mut rep = Replicator::new();
        let mut store = Store::default();
        let mut world: Vec<(u32, QState)> = (0..400u32).map(|k| (net_id(CLASS_CAR, k), car(k as i32 * 100, 0, 0, 0))).collect();
        for step in 0..40 {
            for e in world.iter_mut() {
                e.1.x += 300;
            }
            let desired: Vec<Desired> = world.iter().map(|e| Desired { id: e.0, state: e.1, weight: 1.0 + (e.0 % 10) as f32 }).collect();
            let (bytes, st) = rep.build(SnapHeader::default(), &desired, 1200);
            assert!(bytes.len() <= 1260, "snapshot {} bytes over the budget", bytes.len());
            let d = store.decode(&bytes).unwrap();
            rep.ack(d.header.seq);
            if step > 5 {
                assert!(st.deferred > 0);
            }
        }
        // Every entity reached the client and none is more than a few snapshots stale.
        let d = store.decoded.back().unwrap();
        assert_eq!(d.entities.len(), 400);
        let stale = d.entities.iter().zip(&world).map(|(a, b)| (b.1.x - a.1.x) / 300).max().unwrap();
        assert!(stale <= 12, "the most starved entity is {stale} snapshots behind");
    }
}
