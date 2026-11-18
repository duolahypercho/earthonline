//! Packet framing and the connection handshake.
//!
//! Every datagram starts `MAGIC` (u32, protocol id and version) and a kind
//! byte. Handshake (client C, server S):
//!
//!   C -> S  Request    { client salt, protocol version, ground plan, session token, spawn hint, resume } padded to 1000 bytes
//!   S -> C  Challenge  { client salt, server salt, cookie }           (no state kept on S)
//!   C -> S  Response   { client salt, ground plan, server salt, cookie, token, spawn hint, resume } padded
//!   S -> C  Accept     { connection id, player id, resume key, rates, server tick, the player's state }
//!        or Deny       { client salt, reason }
//!
//! the ground plan is the client's street size (`ground_scale_ppm`):
//! a server whose own differs refuses the client (`DenyReason::World`), since
//! every position and the collision world would disagree (online play would
//! desync).
//!
//! The request and response are padded so a spoofed source address cannot
//! make the server send more bytes than it received (no amplification); the
//! cookie is a keyed hash of (address, salts) so the server allocates a
//! session only for an address that can receive. The session token is
//! opaque bytes for an `Authenticator` (`auth.rs`): today a development
//! token, later a signed token from the account service.
//!
//! Connected packets: `MAGIC, kind, connection id (u64 = client salt ^ server
//! salt)`, then the `conn::Header` (seq, ack, ack bits) and the reliable
//! section, then the payload: `Input` (client), `Snapshot` fragments
//! (server), `KeepAlive`, `Disconnect`.

use crate::bits::{ByteReader, PutLe, Truncated};

/// "EON" + protocol generation 1.
pub const MAGIC: u32 = 0x454F_4E01;
/// 2: the handshake carries the ground plan (groundscale).
/// 3: snapshots carry the server's sky clock; player entities their emote
/// and more state flags; emote and fast-travel commands, the player roster.
pub const PROTOCOL_VERSION: u16 = 3;

/// this process's ground-plan scale in parts per million (the
/// handshake's ground plan; 1,250,000 by default). hills: when the ground
/// heights do not follow the plan (`--height-scale` other than the plan
/// scale) the code also carries them: `0x8000_0000 | plan_e4 << 15 | heights_e4`
/// (units of 1e-4; never a plain ppm value), so a server refuses a client
/// whose ground heights differ.
pub fn ground_scale_ppm() -> u32 {
    let (s, h) = (eo_data::groundscale::scale(), eo_data::groundscale::height_scale());
    if h == s {
        (s * 1e6).round() as u32
    } else {
        0x8000_0000 | (((s * 1e4).round() as u32) << 15) | (h * 1e4).round() as u32
    }
}
/// Datagram payload limit (stays under a 1280-byte IPv6 minimum MTU with headers).
pub const MTU: usize = 1200;
/// The handshake requests are padded to this many bytes.
pub const REQUEST_PAD: usize = 1000;
/// Server simulation rate.
pub const TICK_HZ: u32 = 60;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Request = 1,
    Challenge = 2,
    Response = 3,
    Accept = 4,
    Deny = 5,
    Input = 6,
    Snapshot = 7,
    KeepAlive = 8,
    Disconnect = 9,
}

impl Kind {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Kind::Request,
            2 => Kind::Challenge,
            3 => Kind::Response,
            4 => Kind::Accept,
            5 => Kind::Deny,
            6 => Kind::Input,
            7 => Kind::Snapshot,
            8 => Kind::KeepAlive,
            9 => Kind::Disconnect,
            _ => return None,
        })
    }
}

pub fn begin(kind: Kind, out: &mut Vec<u8>) {
    out.put_u32(MAGIC);
    out.put_u8(kind as u8);
}

/// Magic and kind of a datagram.
pub fn peek(data: &[u8]) -> Option<(Kind, ByteReader<'_>)> {
    let mut r = ByteReader::new(data);
    if r.u32().ok()? != MAGIC {
        return None;
    }
    let k = Kind::from_u8(r.u8().ok()?)?;
    Some((k, r))
}

/// Where a new player asks to stand (development; a real server decides
/// from the account's saved place).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpawnHint {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    /// Spawn sitting in the car (bots that drive).
    pub driving: bool,
}

/// A reconnecting client's claim on its old player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Resume {
    pub player_id: u32,
    pub key: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hello {
    pub client_salt: u64,
    pub version: u16,
    /// the sender's ground-plan scale, parts per million (`ground_scale_ppm`).
    pub ground: u32,
    pub token: Vec<u8>,
    pub spawn: Option<SpawnHint>,
    pub resume: Option<Resume>,
    /// Response only.
    pub server_salt: u64,
    pub cookie: u64,
}

impl Hello {
    pub fn write(&self, kind: Kind, out: &mut Vec<u8>) {
        begin(kind, out);
        out.put_u64(self.client_salt);
        out.put_u16(self.version);
        out.put_u32(self.ground);
        if kind == Kind::Response {
            out.put_u64(self.server_salt);
            out.put_u64(self.cookie);
        }
        out.put_u16(self.token.len() as u16);
        out.extend_from_slice(&self.token);
        match self.spawn {
            Some(s) => {
                out.put_u8(1 | (s.driving as u8) << 1);
                out.put_f32(s.x);
                out.put_f32(s.z);
                out.put_f32(s.yaw);
            }
            None => out.put_u8(0),
        }
        match self.resume {
            Some(r) => {
                out.put_u8(1);
                out.put_u32(r.player_id);
                out.put_u64(r.key);
            }
            None => out.put_u8(0),
        }
        if out.len() < REQUEST_PAD {
            out.resize(REQUEST_PAD, 0);
        }
    }

    pub fn read(kind: Kind, r: &mut ByteReader) -> Result<Self, Truncated> {
        let client_salt = r.u64()?;
        let version = r.u16()?;
        let ground = r.u32()?;
        let (server_salt, cookie) = if kind == Kind::Response { (r.u64()?, r.u64()?) } else { (0, 0) };
        let n = r.u16()? as usize;
        let token = r.take(n.min(512))?.to_vec();
        let f = r.u8()?;
        let spawn = if f & 1 != 0 { Some(SpawnHint { x: r.f32()?, z: r.f32()?, yaw: r.f32()?, driving: f & 2 != 0 }) } else { None };
        let resume = if r.u8()? != 0 { Some(Resume { player_id: r.u32()?, key: r.u64()? }) } else { None };
        Ok(Hello { client_salt, version, ground, token, spawn, resume, server_salt, cookie })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Challenge {
    pub client_salt: u64,
    pub server_salt: u64,
    pub cookie: u64,
}

impl Challenge {
    pub fn write(&self, out: &mut Vec<u8>) {
        begin(Kind::Challenge, out);
        out.put_u64(self.client_salt);
        out.put_u64(self.server_salt);
        out.put_u64(self.cookie);
    }
    pub fn read(r: &mut ByteReader) -> Result<Self, Truncated> {
        Ok(Challenge { client_salt: r.u64()?, server_salt: r.u64()?, cookie: r.u64()? })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Accept {
    pub connection: u64,
    pub player_id: u32,
    pub resume_key: u64,
    pub tick_hz: u8,
    pub snapshot_hz: u8,
    pub server_tick: u32,
    /// The input sequence the player's state was taken after (the next input is this + 1).
    pub input_seq: u32,
    /// The player's whole state (`wire::write_player_state`) and where its cars were spawned.
    pub state: Vec<u8>,
    pub car_spawns: Vec<[f32; 3]>,
    pub resumed: bool,
}

impl Accept {
    pub fn write(&self, out: &mut Vec<u8>) {
        begin(Kind::Accept, out);
        out.put_u64(self.connection);
        out.put_u32(self.player_id);
        out.put_u64(self.resume_key);
        out.put_u8(self.tick_hz);
        out.put_u8(self.snapshot_hz);
        out.put_u32(self.server_tick);
        out.put_u32(self.input_seq);
        out.put_u8(self.resumed as u8);
        out.put_u8(self.car_spawns.len() as u8);
        for c in &self.car_spawns {
            out.put_f32(c[0]);
            out.put_f32(c[1]);
            out.put_f32(c[2]);
        }
        out.put_u16(self.state.len() as u16);
        out.extend_from_slice(&self.state);
    }
    pub fn read(r: &mut ByteReader) -> Result<Self, Truncated> {
        let connection = r.u64()?;
        let player_id = r.u32()?;
        let resume_key = r.u64()?;
        let tick_hz = r.u8()?;
        let snapshot_hz = r.u8()?;
        let server_tick = r.u32()?;
        let input_seq = r.u32()?;
        let resumed = r.u8()? != 0;
        let n = r.u8()? as usize;
        let mut car_spawns = vec![];
        for _ in 0..n {
            car_spawns.push([r.f32()?, r.f32()?, r.f32()?]);
        }
        let len = r.u16()? as usize;
        let state = r.take(len)?.to_vec();
        Ok(Accept { connection, player_id, resume_key, tick_hz, snapshot_hz, server_tick, input_seq, state, car_spawns, resumed })
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    Version = 1,
    Token = 2,
    Full = 3,
    Banned = 4,
    /// the client's ground plan is not the server's.
    World = 5,
}

pub fn write_deny(client_salt: u64, reason: DenyReason, out: &mut Vec<u8>) {
    begin(Kind::Deny, out);
    out.put_u64(client_salt);
    out.put_u8(reason as u8);
}

/// Connected packet prefix: magic, kind, connection id.
pub fn begin_connected(kind: Kind, connection: u64, out: &mut Vec<u8>) {
    begin(kind, out);
    out.put_u64(connection);
}

/// Reliable-channel messages (application level).
pub mod msg {
    /// Server to client: u32 player id, then the display name (UTF-8). Sent
    /// for every player already in when a client joins, and to everyone
    /// when one joins.
    pub const PLAYER_JOINED: u8 = 1;
    /// Server to client: u32 player id.
    pub const PLAYER_LEFT: u8 = 2;
    pub const NOTICE: u8 = 3;
    /// Client to server: u8 `presence::Emote` (0 ends one).
    pub const EMOTE: u8 = 4;
    /// Client to server: u16 index into `presence::viewpoints()`.
    pub const TRAVEL: u8 = 5;
    /// Server to client: u16 viewpoint, u8 `TravelResult`, then f32 x, z,
    /// yaw of the arrival and u32 the server tick it lands on.
    pub const TRAVEL_REPLY: u8 = 6;
}

/// The server's answer to a fast-travel request.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TravelResult {
    Accepted = 0,
    /// No such viewpoint.
    Unknown = 1,
    /// Not on foot (get out of the car first).
    Driving = 2,
    /// Too soon after the last one.
    Cooldown = 3,
    /// One already under way.
    Busy = 4,
}

impl TravelResult {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => TravelResult::Accepted,
            2 => TravelResult::Driving,
            3 => TravelResult::Cooldown,
            4 => TravelResult::Busy,
            _ => TravelResult::Unknown,
        }
    }
}

/// A decoded fast-travel reply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TravelReply {
    pub viewpoint: u16,
    pub result: TravelResult,
    pub arrival: [f32; 3],
    pub tick: u32,
}

impl TravelReply {
    pub fn write(&self) -> Vec<u8> {
        let mut m = vec![msg::TRAVEL_REPLY];
        m.put_u16(self.viewpoint);
        m.put_u8(self.result as u8);
        for v in self.arrival {
            m.put_f32(v);
        }
        m.put_u32(self.tick);
        m
    }

    /// From a reliable message (its first byte `msg::TRAVEL_REPLY`).
    pub fn read(m: &[u8]) -> Option<Self> {
        if m.first() != Some(&msg::TRAVEL_REPLY) {
            return None;
        }
        let mut r = ByteReader::new(&m[1..]);
        Some(TravelReply { viewpoint: r.u16().ok()?, result: TravelResult::from_u8(r.u8().ok()?), arrival: [r.f32().ok()?, r.f32().ok()?, r.f32().ok()?], tick: r.u32().ok()? })
    }
}

/// A roster message: (player id, name) for `PLAYER_JOINED`, (id, "") for `PLAYER_LEFT`.
pub fn read_roster(m: &[u8]) -> Option<(bool, u32, String)> {
    let (&k, rest) = m.split_first()?;
    if (k != msg::PLAYER_JOINED && k != msg::PLAYER_LEFT) || rest.len() < 4 {
        return None;
    }
    let id = u32::from_le_bytes(rest[..4].try_into().ok()?);
    Some((k == msg::PLAYER_JOINED, id, String::from_utf8_lossy(&rest[4..]).chars().take(32).collect()))
}

pub fn roster_joined(id: u32, name: &str) -> Vec<u8> {
    let mut m = vec![msg::PLAYER_JOINED];
    m.put_u32(id);
    m.extend_from_slice(name.as_bytes());
    m
}

pub fn roster_left(id: u32) -> Vec<u8> {
    let mut m = vec![msg::PLAYER_LEFT];
    m.put_u32(id);
    m
}
