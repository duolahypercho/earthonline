//! Earth Online online foundation (no GPU).
//!
//!   `link`      UDP on `std::net`, plus a link conditioner (latency, jitter, loss, duplicates)
//!   `conn`      packet sequence, acks, round-trip time, the reliable ordered channel
//!   `proto`     framing and the handshake (padded requests, stateless cookie, session token)
//!   `auth`      the session token interface an account system plugs into
//!   `wire`      quantised inputs (resent until run) and the exact player state of a correction
//!   `snapshot`  entity states, delta compression against the acknowledged baseline, the budget, interpolation
//!   `interest`  64 m cells keyed by zone, per-class radii with hysteresis
//!   `game`      the shared player rules (spawn, the one authoritative step)
//!   `predict`   client prediction and reconciliation
//!   `client`    the client's network thread
//!   `pool`      the server's worker pool
//!
//! See native/README.md "Online" for the architecture, the sharding plan and
//! the measurements.

pub mod auth;
pub mod bits;
pub mod client;
pub mod conn;
pub mod game;
pub mod interest;
pub mod link;
pub mod pool;
pub mod predict;
pub mod proto;
pub mod snapshot;
pub mod wire;
