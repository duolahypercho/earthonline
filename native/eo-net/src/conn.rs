//! One connection's packet layer: sequence numbers, acks, round-trip time,
//! and a reliable ordered message channel, on top of unreliable datagrams.
//!
//! Every connected packet carries `seq` (u16, wrapping), `ack` (the newest
//! sequence received from the peer) and `ack_bits` (the 32 before it). A sent
//! packet is acked when any later packet from the peer names it, so one lost
//! ack costs nothing. Round-trip time is the smoothed time from a packet's
//! send to its first ack (RFC 6298 style, alpha 1/8, beta 1/4).
//!
//! Reliable messages (u16 ids) ride in any connected packet. A message is
//! resent, in a later packet, until a packet holding it is acked; the
//! receiver delivers in id order, holding out-of-order ones back, and drops
//! duplicates. Snapshots and inputs do not use this channel: they are
//! unreliable by design and acknowledged by the application (a snapshot is
//! the baseline once the client says it decoded it; inputs are resent until
//! the server says it ran them).

use crate::bits::{ByteReader, PutLe, Truncated};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

#[inline]
pub fn seq_newer(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Header {
    pub seq: u16,
    pub ack: u16,
    pub ack_bits: u32,
}

impl Header {
    pub const BYTES: usize = 8;
    pub fn write(&self, out: &mut Vec<u8>) {
        out.put_u16(self.seq);
        out.put_u16(self.ack);
        out.put_u32(self.ack_bits);
    }
    pub fn read(r: &mut ByteReader) -> Result<Self, Truncated> {
        Ok(Header { seq: r.u16()?, ack: r.u16()?, ack_bits: r.u32()? })
    }
}

struct Sent {
    seq: u16,
    at: Instant,
    acked: bool,
    messages: Vec<u16>,
}

struct Outgoing {
    id: u16,
    data: Vec<u8>,
    last_sent: Option<Instant>,
}

/// Connection counters for the HUD and the reports.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConnStats {
    pub rtt_ms: f64,
    pub rtt_var_ms: f64,
    pub packets_sent: u64,
    pub packets_acked: u64,
    pub packets_lost: u64,
    pub packets_received: u64,
    pub duplicates: u64,
    pub out_of_order: u64,
    pub reliable_resends: u64,
}

pub struct Connection {
    next_seq: u16,
    /// Newest received sequence and the 32 before it.
    recv_seq: Option<u16>,
    recv_bits: u32,
    sent: VecDeque<Sent>,
    out: VecDeque<Outgoing>,
    next_msg_id: u16,
    /// Next id to deliver, and the ones held for order.
    deliver_id: u16,
    held: BTreeMap<u16, Vec<u8>>,
    pub stats: ConnStats,
    rtt_init: bool,
    pub last_recv: Instant,
    pub last_send: Instant,
}

/// The most reliable-message bytes one packet carries.
pub const RELIABLE_BUDGET: usize = 400;

impl Default for Connection {
    fn default() -> Self {
        Self::new()
    }
}

impl Connection {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            next_seq: 0,
            recv_seq: None,
            recv_bits: 0,
            sent: VecDeque::new(),
            out: VecDeque::new(),
            next_msg_id: 0,
            deliver_id: 0,
            held: BTreeMap::new(),
            stats: ConnStats { rtt_ms: 100.0, rtt_var_ms: 50.0, ..Default::default() },
            rtt_init: false,
            last_recv: now,
            last_send: now,
        }
    }

    pub fn rtt_ms(&self) -> f64 {
        self.stats.rtt_ms
    }

    /// Queue a reliable message.
    pub fn send_reliable(&mut self, data: Vec<u8>) {
        let id = self.next_msg_id;
        self.next_msg_id = self.next_msg_id.wrapping_add(1);
        self.out.push_back(Outgoing { id, data, last_sent: None });
    }

    pub fn reliable_pending(&self) -> usize {
        self.out.len()
    }

    /// Start a packet: header and the reliable messages due (unacked and not
    /// sent within a resend interval). The caller appends its payload.
    pub fn begin_packet(&mut self, out: &mut Vec<u8>) -> u16 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        Header { seq, ack: self.recv_seq.unwrap_or(0), ack_bits: if self.recv_seq.is_some() { self.recv_bits } else { 0 } }.write(out);
        let now = Instant::now();
        let resend = Duration::from_secs_f64((self.stats.rtt_ms * 1.25 + 2.0 * self.stats.rtt_var_ms).clamp(30.0, 1000.0) / 1e3);
        let mut ids = vec![];
        let count_at = out.len();
        out.put_u8(0);
        let mut bytes = 0;
        for m in self.out.iter_mut() {
            if ids.len() >= 255 || bytes + m.data.len() > RELIABLE_BUDGET {
                break;
            }
            if m.last_sent.is_some_and(|t| now - t < resend) {
                continue;
            }
            if m.last_sent.is_some() {
                self.stats.reliable_resends += 1;
            }
            m.last_sent = Some(now);
            out.put_u16(m.id);
            out.put_u16(m.data.len() as u16);
            out.extend_from_slice(&m.data);
            bytes += m.data.len() + 4;
            ids.push(m.id);
        }
        out[count_at] = ids.len() as u8;
        self.sent.push_back(Sent { seq, at: now, acked: false, messages: ids });
        while self.sent.len() > 256 {
            let s = self.sent.pop_front().unwrap();
            if !s.acked {
                self.stats.packets_lost += 1;
            }
        }
        self.stats.packets_sent += 1;
        self.last_send = now;
        seq
    }

    /// Read a connected packet's header and reliable section; returns the
    /// in-order reliable messages now deliverable, or None for a duplicate
    /// or stale packet (whose payload must be ignored).
    pub fn receive(&mut self, r: &mut ByteReader) -> Result<Option<(Header, Vec<Vec<u8>>)>, Truncated> {
        let h = Header::read(r)?;
        let n = r.u8()? as usize;
        let mut msgs = Vec::with_capacity(n);
        for _ in 0..n {
            let id = r.u16()?;
            let len = r.u16()? as usize;
            msgs.push((id, r.take(len)?.to_vec()));
        }
        // Receive window.
        match self.recv_seq {
            None => {
                self.recv_seq = Some(h.seq);
                self.recv_bits = 0;
            }
            Some(last) => {
                if seq_newer(h.seq, last) {
                    let shift = h.seq.wrapping_sub(last) as u32;
                    self.recv_bits = match shift {
                        33.. => 0,
                        32 => 1 << 31,
                        _ => (self.recv_bits << shift) | (1 << (shift - 1)),
                    };
                    self.recv_seq = Some(h.seq);
                } else {
                    let back = last.wrapping_sub(h.seq) as u32;
                    if back == 0 || back > 32 || self.recv_bits & (1 << (back - 1)) != 0 {
                        self.stats.duplicates += 1;
                        return Ok(None);
                    }
                    self.recv_bits |= 1 << (back - 1);
                    self.stats.out_of_order += 1;
                }
            }
        }
        self.stats.packets_received += 1;
        self.last_recv = Instant::now();
        // Acks of our packets.
        self.process_ack(h.ack, h.ack_bits);
        // Reliable delivery in order.
        let mut ready = vec![];
        for (id, data) in msgs {
            let ahead = id.wrapping_sub(self.deliver_id);
            if ahead >= 0x8000 {
                continue; // already delivered
            }
            self.held.entry(id).or_insert(data);
        }
        while let Some(d) = self.held.remove(&self.deliver_id) {
            ready.push(d);
            self.deliver_id = self.deliver_id.wrapping_add(1);
        }
        Ok(Some((h, ready)))
    }

    fn process_ack(&mut self, ack: u16, bits: u32) {
        let now = Instant::now();
        let mut acked_msgs: Vec<u16> = vec![];
        for s in self.sent.iter_mut() {
            if s.acked {
                continue;
            }
            let hit = if s.seq == ack {
                true
            } else if seq_newer(ack, s.seq) {
                let back = ack.wrapping_sub(s.seq) as u32;
                back <= 32 && bits & (1 << (back - 1)) != 0
            } else {
                false
            };
            if hit {
                s.acked = true;
                self.stats.packets_acked += 1;
                acked_msgs.extend_from_slice(&s.messages);
                let sample = (now - s.at).as_secs_f64() * 1e3;
                if !self.rtt_init {
                    self.stats.rtt_ms = sample;
                    self.stats.rtt_var_ms = sample / 2.0;
                    self.rtt_init = true;
                } else {
                    self.stats.rtt_var_ms = 0.75 * self.stats.rtt_var_ms + 0.25 * (self.stats.rtt_ms - sample).abs();
                    self.stats.rtt_ms = 0.875 * self.stats.rtt_ms + 0.125 * sample;
                }
            }
        }
        if !acked_msgs.is_empty() {
            self.out.retain(|m| !acked_msgs.contains(&m.id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acks_and_ordered_reliable_delivery_survive_loss_and_reorder() {
        let mut a = Connection::new();
        let mut b = Connection::new();
        for i in 0..50u16 {
            a.send_reliable(i.to_le_bytes().to_vec());
        }
        let mut got = vec![];
        let mut dice = crate::link::Dice::new(7);
        let mut in_flight: Vec<(f64, Vec<u8>, bool)> = vec![];
        for round in 0..400 {
            let mut p = vec![];
            a.begin_packet(&mut p);
            in_flight.push((round as f64 + dice.next_f64() * 4.0, p, true));
            let mut q = vec![];
            b.begin_packet(&mut q);
            in_flight.push((round as f64 + dice.next_f64() * 4.0, q, false));
            in_flight.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
            let due: Vec<_> = in_flight.iter().filter(|x| x.0 <= round as f64).cloned().collect();
            in_flight.retain(|x| x.0 > round as f64);
            for (_, data, to_b) in due {
                if dice.next_f64() < 0.2 {
                    continue; // 20 % loss
                }
                let mut r = ByteReader::new(&data);
                if to_b {
                    if let Ok(Some((_, msgs))) = b.receive(&mut r) {
                        got.extend(msgs.into_iter().map(|m| u16::from_le_bytes([m[0], m[1]])));
                    }
                } else {
                    let _ = a.receive(&mut r);
                }
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        assert_eq!(got, (0..50).collect::<Vec<u16>>(), "every reliable message once, in order");
        assert_eq!(a.reliable_pending(), 0, "all acked");
        assert!(b.stats.out_of_order > 0, "the test reordered packets");
    }
}
