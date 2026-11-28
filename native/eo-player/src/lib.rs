//! Earth Online player, ported from the JS game: on-foot character
//! controller, walk and drive cameras, the player's car, input mapping, and
//! the collision world streamed per tile. No GPU dependency.
//!
//! Engine: Rapier (`rapier3d`), as the JS game uses Rapier for the same jobs.
//! See `world.rs` for the tile stream, `character.rs` for the walk,
//! `vehicle.rs` for the car, `camera.rs` for the rigs, `routes.rs` for the
//! scripted walk and drive the bench and tests run.

pub mod camera;
pub mod character;
pub mod harbour; // moored boats floating on the drawn sea
pub mod input;
pub mod lift;
pub mod momentum;
pub mod player;
pub mod routes;
pub mod street_ground;
pub mod traffic;
pub mod traffic_contact;
pub mod people_contact;
pub mod tuning;
pub mod vehicle;
pub mod weather;
pub mod world;

pub use player::{CameraView, Mode, Player, PlayerState, StepInput}; // + PlayerState
pub use weather::WeatherPhysics;
pub use world::{BuildingSet, CollisionWorld, DeckPart, Decks, ElevationGround, GroundProvider, LiftDef, PrismCut, SurfaceMesh, TileFeed, TileKey}; // + TileFeed, decks
