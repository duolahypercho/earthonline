//! Packet framing and the connection handshake.
//!
//! Every datagram starts `MAGIC` (u32, protocol id and version) and a kind
//! byte. Handshake (client C, server S):
//!
//!   C -> S  Request    { client salt, protocol version, ground plan, session token, spawn hint, resume, structures } padded to 1000 bytes
//!   S -> C  Challenge  { client salt, server salt, cookie }           (no state kept on S)
//!   C -> S  Response   { client salt, ground plan, server salt, cookie, token, spawn hint, resume, structures } padded
//!   S -> C  Accept     { connection id, player id, resume key, rates, server tick, the player's state }
//!        or Deny       { client salt, reason }
//!
//! the ground plan is the client's street size (`ground_scale_ppm`):
//! a server whose own differs refuses the client (`DenyReason::World`), since
//! every position and the collision world would disagree (online play would
//! desync).
//!
//! The request and response also carry the client's grade-separated roads
//! (`structures_code`: `--structures`, `--tunnels`), compared beside the
//! ground plan: a server whose world has other ones refuses the client
//! (`DenyReason::Structures`, with the server's own code after the reason),
//! since decks, ramps and tunnels change the collision world and the
//! traffic. The byte is the last field before the padding, so a hello
//! written without it reads as 0, "none" (the setting every world had before).
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
/// 4: inputs carry 16 button bits (the lift's interact tap) and the player
/// state a landmark lift ride.
/// 5: predictions include the selected vehicle, its payload and moving support velocity.
/// The reliable-count byte marks a valid ack, including the first packet (seq 0).
/// 6: accept and correction carry the input sequence's driving weather timeline.
/// 7: explicit Drive/Reverse range in inputs and vehicle checkpoints.
/// 8: the coastal promenades and adjoining stairs are authoritative support.
/// 9: tile loads preserve simulation time and unoccupied cars share the weather.
/// 10: car interactions respect intervening floors and a jump resolves ceiling contacts.
/// 11: a standing rider follows the vehicle roof and carries its velocity into a fall.
pub const PROTOCOL_VERSION: u16 = 11;

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

/// A world's grade-separated roads as the handshake carries them: 0 none,
/// `STRUCTURES_ON` with `--structures`, plus `STRUCTURES_TUNNELS` with `--tunnels`.
pub fn structures_code(s: Option<eo_structures::StructureOptions>) -> u8 {
    match s {
        None => 0,
        Some(o) => STRUCTURES_ON | if o.tunnels { STRUCTURES_TUNNELS } else { 0 },
    }
}

/// `structures_code` bits.
pub const STRUCTURES_ON: u8 = 1;
pub const STRUCTURES_TUNNELS: u8 = 2;

/// A `structures_code` as the switches that give it, for messages.
pub fn structures_switches(code: u8) -> &'static str {
    match code {
        0 => "no --structures",
        STRUCTURES_ON => "--structures",
        c if c == STRUCTURES_ON | STRUCTURES_TUNNELS => "--structures --tunnels",
        _ => "an unknown structures setting",
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
    pub vehicle: eo_player::vehicle::VehicleConfig,
    pub client_salt: u64,
    pub version: u16,
    /// the sender's ground-plan scale, parts per million (`ground_scale_ppm`).
    pub ground: u32,
    /// the sender's grade-separated roads (`structures_code`); on the wire
    /// after `resume`, in what was padding (0 from a sender without it).
    pub structures: u8,
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
        out.put_u8(self.structures);
        crate::wire::put_vehicle_config(out, self.vehicle);
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
        let structures = r.u8()?;
        let vehicle = if r.remaining() == 0 { Default::default() } else { crate::wire::get_vehicle_config(r)? };
        Ok(Hello { vehicle, client_salt, version, ground, structures, token, spawn, resume, server_salt, cookie })
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
    pub weather: crate::weather::WeatherEpoch,
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
        out.extend_from_slice(&self.weather.bytes());
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
        let weather = crate::weather::WeatherEpoch::read(r.take(crate::weather::WeatherEpoch::BYTES)?)?;
        let resumed = r.u8()? != 0;
        let n = r.u8()? as usize;
        let mut car_spawns = vec![];
        for _ in 0..n {
            car_spawns.push([r.f32()?, r.f32()?, r.f32()?]);
        }
        let len = r.u16()? as usize;
        let state = r.take(len)?.to_vec();
        Ok(Accept { connection, player_id, resume_key, tick_hz, snapshot_hz, server_tick, input_seq, weather, state, car_spawns, resumed })
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
    /// the client's grade-separated roads are not the server's; the server's
    /// own `structures_code` follows the reason byte.
    Structures = 6,
}

pub fn write_deny(client_salt: u64, reason: DenyReason, out: &mut Vec<u8>) {
    begin(Kind::Deny, out);
    out.put_u64(client_salt);
    out.put_u8(reason as u8);
}

/// `Deny` for `DenyReason::Structures`, with the server's own `structures_code` after the reason.
pub fn write_deny_structures(client_salt: u64, own: u8, out: &mut Vec<u8>) {
    write_deny(client_salt, DenyReason::Structures, out);
    out.put_u8(own);
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
    /// One already under way (or a landmark lift ride).
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

#[cfg(test)]
mod tests {
    use super::*;
    use eo_structures::StructureOptions;

    fn hello(structures: u8) -> Hello {
        Hello {
            client_salt: 0x1234_5678_9abc_def0,
            vehicle: Default::default(),
            version: PROTOCOL_VERSION,
            ground: 1_250_000,
            structures,
            token: b"dev:test".to_vec(),
            spawn: Some(SpawnHint { x: 1.5, z: -2.5, yaw: 0.25, driving: true }),
            resume: Some(Resume { player_id: 7, key: 99 }),
            server_salt: 11,
            cookie: 22,
        }
    }

    #[test]
    fn structures_codes() {
        assert_eq!(structures_code(None), 0);
        assert_eq!(structures_code(Some(StructureOptions { tunnels: false })), STRUCTURES_ON);
        assert_eq!(structures_code(Some(StructureOptions { tunnels: true })), STRUCTURES_ON | STRUCTURES_TUNNELS);
        assert_eq!(structures_switches(0), "no --structures");
        assert_eq!(structures_switches(3), "--structures --tunnels");
    }

    #[test]
    fn hello_carries_structures_in_what_was_padding() {
        for kind in [Kind::Request, Kind::Response] {
            for code in [0, STRUCTURES_ON, STRUCTURES_ON | STRUCTURES_TUNNELS] {
                let h = hello(code);
                let mut out = vec![];
                h.write(kind, &mut out);
                assert_eq!(out.len(), REQUEST_PAD);
                let (k, mut r) = peek(&out).unwrap();
                assert_eq!(k, kind);
                let back = Hello::read(kind, &mut r).unwrap();
                let want = if kind == Kind::Response { h.clone() } else { Hello { server_salt: 0, cookie: 0, ..h.clone() } };
                assert_eq!(back, want);
            }
            // The setting is one byte where the padding began: the datagram with
            // it cleared is exactly the one without structures, which is the
            // datagram written before the field existed (resume, then zeros),
            // and reads as "none".
            let (mut on, mut off) = (vec![], vec![]);
            hello(STRUCTURES_ON | STRUCTURES_TUNNELS).write(kind, &mut on);
            hello(0).write(kind, &mut off);
            let at = on.iter().rposition(|&b| b != 0).unwrap();
            assert_eq!(on[at], STRUCTURES_ON | STRUCTURES_TUNNELS);
            assert!(off[at..].iter().all(|&b| b == 0));
            on[at] = 0;
            assert_eq!(on, off);
            let (_, mut r) = peek(&off).unwrap();
            assert_eq!(Hello::read(kind, &mut r).unwrap().structures, 0);
        }
    }

    #[test]
    fn a_structures_denial_names_the_servers_own() {
        let mut out = vec![];
        write_deny_structures(42, STRUCTURES_ON, &mut out);
        let (k, mut r) = peek(&out).unwrap();
        assert_eq!(k, Kind::Deny);
        assert_eq!((r.u64().unwrap(), r.u8().unwrap(), r.u8().unwrap()), (42, DenyReason::Structures as u8, STRUCTURES_ON));
    }
}
