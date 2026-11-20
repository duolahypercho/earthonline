//! The grade-separated roads (eo-structures: freeways, ramps, road bridges,
//! tunnels) at load time and their attach to the renderer, behind
//! `--structures` (`--tunnels` for the bores and portals).
//!
//! `spawn` starts the build on its own thread as soon as the street world is
//! ready (it reads the street plan, the city's roads, the standards and the
//! landmark bridges, all already in hand). The build is stage A6's joined one
//! (`eo_structures::build_joined`): the structures solved on that street, the
//! street built again with their joins (junction legs, capped, raised and
//! lowered stubs), the structures solved and meshed on it. `Pending::streets`
//! waits for that joined street world, which `load_world` then hands to
//! everything built after it (street tiles, terrain carpet, props, player,
//! agents, plazas, shore), so they all stand on one street; eo-net's
//! `GameWorld::load_with` builds the same (`StreetGround::build_joined`), so a
//! dedicated server agrees. The same thread then converts every tile's far and
//! coarse levels for the GPU (`eo_render::structures::StructurePrepared`)
//! beside the rest of the load. `Pending::join` takes both at the end of
//! `load_world` into `World::structures` (which every consumer reads: the
//! renderer, the player's ground, the agents' feed, the props world, the
//! in-process server) and `World::structures_drawn` (the renderer's part:
//! groups, materials, the static levels, the arena it needs). Without the
//! switch, or without streets, nothing is spawned, the street world is the
//! one built first and every consumer holds None.

use anyhow::Result;
use eo_data::City;
use eo_data::standards::CaStandards;
use eo_render::structures::{Placement, StructurePrepared, StructuresArena};
use eo_render::{GpuContext, Renderer};
use eo_street::StreetWorld;
use eo_structures::{StructureOptions, StructureWorld};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::Instant;

/// Threads building and converting the tiles' meshes at load (on the build's own thread's pool).
const PREPARE_THREADS: usize = 4;
/// Worker threads cooking the near packs (below the render thread), as the street's.
pub const NEAR_THREADS: usize = 2;

/// What the build thread hands back.
pub struct Built {
    pub world: Arc<StructureWorld>,
    /// The renderer's part (None for an empty world: nothing is drawn).
    pub drawn: Option<Arc<StructurePrepared>>,
    build_ms: f64,
}

/// The joined build's street world (None when the structures asked for no
/// joins: the first one stands) and the structures over it, sent as soon as
/// they are built (before the meshes are converted).
type Joined = (Option<Arc<StreetWorld>>, Arc<StructureWorld>);

/// The build, running on its own thread.
pub struct Pending {
    job: Option<JoinHandle<Result<Built>>>,
    joined: Receiver<Joined>,
    /// What `streets` received (the world, for the load's users before the join).
    world: Option<Arc<StructureWorld>>,
    started: Instant,
}

/// Start building the structures over `streets` when `options` is Some
/// (`--structures`); None spawns nothing.
pub fn spawn(options: Option<StructureOptions>, streets: &Arc<StreetWorld>, city: &Arc<City>, standards: &CaStandards) -> Option<Pending> {
    let options = options?;
    let (streets, city, standards) = (streets.clone(), city.clone(), standards.clone());
    let (tx, joined) = std::sync::mpsc::channel();
    let job = std::thread::Builder::new()
        .name("eo-structures-build".into())
        .spawn(move || {
            eo_render::hitch::worker_started();
            let t = Instant::now();
            // stage A6: the street built again with the structures' joins, the structures on it.
            let j = eo_structures::build_joined(&streets, &city, &standards, eo_landmarks::LandmarksJs::get(), options)?;
            drop(streets);
            let world = Arc::new(j.structures);
            let build_ms = t.elapsed().as_secs_f64() * 1e3;
            let _ = tx.send((j.streets.map(Arc::new), world.clone()));
            let drawn = (!world.is_empty()).then(|| Arc::new(StructurePrepared::prepare(world.clone(), eo_render::street::StreetRadii::default(), PREPARE_THREADS)));
            Ok(Built { world, drawn, build_ms })
        })
        .expect("spawn the structures build");
    Some(Pending { job: Some(job), joined, world: None, started: Instant::now() })
}

impl Pending {
    /// Wait for the joined build's street world (stage A6): the street every
    /// consumer built after it stands on. `first` (the street world the build
    /// started from) when the structures asked for no joins. A failed build
    /// stops the load here.
    pub fn streets(&mut self, first: Arc<StreetWorld>) -> Result<Arc<StreetWorld>> {
        let t = Instant::now();
        let (streets, world) = match self.joined.recv() {
            Ok(j) => j,
            Err(_) => {
                // the thread ended before sending: its error (or panic).
                let job = self.job.take().expect("the structures build is joined once");
                return Err(match job.join() {
                    Ok(Err(e)) => e,
                    Ok(Ok(_)) => anyhow::anyhow!("the structures build sent no street world"),
                    Err(_) => anyhow::anyhow!("structures thread panicked"),
                });
            }
        };
        if let Some(j) = &world.census().joins {
            eprintln!("startup: structures' street joins ({:.0} ms waiting for them):\n{j}", t.elapsed().as_secs_f64() * 1e3);
        }
        self.world = Some(world);
        Ok(streets.unwrap_or(first))
    }

    /// The structures world `streets` received (None before it).
    pub fn world(&self) -> Option<&Arc<StructureWorld>> {
        self.world.as_ref()
    }

    /// Wait for the build (it ran beside the load). Returns the world, empty or not, and what the renderer draws of it.
    pub fn join(mut self) -> Result<Built> {
        let t = Instant::now();
        let job = self.job.take().ok_or_else(|| anyhow::anyhow!("the structures build already failed"))?;
        let b = job.join().map_err(|_| anyhow::anyhow!("structures thread panicked"))??;
        eprintln!(
            "startup: structures ({}{}) built in {:.0} ms ({:.0} ms waiting at the end of the load)",
            if b.world.options().tunnels { "with tunnels" } else { "no tunnels" },
            if b.world.is_empty() { ", empty" } else { "" },
            b.build_ms,
            t.elapsed().as_secs_f64() * 1e3
        );
        if let Some(p) = &b.drawn {
            let s = &p.set;
            let (sv, si) = p.static_bytes();
            eprintln!(
                "startup: structures drawn: {} tiles, {} concrete tints; triangles near {} / far {} / coarse {}; static far + coarse {:.1} MB vertices + {:.1} MB indices; near part {:?} (vertices, indices, objects); meshes built and converted in {:.0} ms ({:.0} ms since the build started)",
                s.tiles.len(),
                s.tints.rgb.len(),
                s.totals.triangles[0],
                s.totals.triangles[1],
                s.totals.triangles[2],
                sv as f64 / 1e6,
                si as f64 / 1e6,
                s.near_bound,
                s.prepare_ms,
                self.started.elapsed().as_secs_f64() * 1e3
            );
        }
        Ok(b)
    }
}

/// The grade-separated roads the world was built with (None without
/// `--structures` or without streets): what the online handshake claims.
pub fn options(world: &crate::World) -> Option<StructureOptions> {
    world.structures.as_ref().map(|s| s.options())
}

/// What the renderer draws: only for a built, non-empty world (otherwise no
/// arena, object, group, material or attach, and every other arena offset
/// is as without the switch).
pub fn drawn(world: &crate::World) -> Option<&Arc<StructurePrepared>> {
    world.structures_drawn.as_ref()
}

/// The arena after the street's, at (vertex, index, object) bases: the
/// static far and coarse levels and room for the near packs, sized from the
/// meshes (`StructurePrepared::arena_needs`); all zero (nothing reserved)
/// unless the world draws structures.
pub fn arena(world: &crate::World, vertex_base: u32, index_base: u32, object_base: u32) -> StructuresArena {
    let Some(p) = drawn(world) else { return StructuresArena::default() };
    let (v, i, o) = p.arena_needs();
    StructuresArena { vertex_base, vertex_capacity: v, index_base, index_capacity: i, object_base, object_capacity: o }
}

/// Where the structures' groups and materials go: the groups after every
/// other group (`group_base`), the concrete materials after the set's
/// (`concrete_material`), the deck on the street material (the vertex
/// colour if the street has none).
pub fn placement(world: &crate::World, group_base: u32, concrete_material: u32) -> Placement {
    let street_material = world.street.as_ref().and_then(|s| s.palette).map(|p| p.base).unwrap_or(eo_render::scene::MATERIAL_VERTEX_COLOUR);
    Placement { group_base, street_material, concrete_material }
}

/// Hand the static levels to the renderer (written into the arena's
/// start, which the vertex and index buffers already count). Returns the
/// GPU bytes allocated beyond the arenas (none).
pub fn attach(ctx: &GpuContext, r: &Renderer, world: &crate::World, placement: Placement, arena: StructuresArena) -> u64 {
    match drawn(world) {
        Some(p) => r.attach_structures(ctx, p, placement, arena),
        None => 0,
    }
}

/// The bench JSON's top-level `structures` object.
pub fn json(args: &crate::Args, world: &crate::World) -> serde_json::Value {
    let mut out = serde_json::json!({
        "on": args.structures.is_some(),
        "tunnels": args.structures.is_some_and(|s| s.tunnels),
        "built": world.structures.is_some(),
        "empty": world.structures.as_ref().is_none_or(|s| s.is_empty()),
    });
    if let Some(p) = drawn(world) {
        let s = &p.set;
        let (sv, si) = p.static_bytes();
        out["drawn"] = serde_json::json!({
            "tiles": s.tiles.len(),
            "concrete_tints": s.tints.rgb,
            "triangles": { "near": s.totals.triangles[0], "far": s.totals.triangles[1], "coarse": s.totals.triangles[2] },
            "vertices": { "near": s.totals.vertices[0], "far": s.totals.vertices[1], "coarse": s.totals.vertices[2] },
            "largest_tile": { "near": s.totals.max_tile[0], "far": s.totals.max_tile[1], "coarse": s.totals.max_tile[2] },
            "static_objects": p.statics.objects.len(),
            "static_vertex_bytes": sv,
            "static_index_bytes": si,
            "near_bound": { "vertices": s.near_bound.0, "indices": s.near_bound.1, "objects": s.near_bound.2 },
            "arena": p.arena_needs(),
            "prepare_ms": s.prepare_ms,
        });
    }
    // stage A8: the joins the street was built with, and the tunnels' bores (only when there are any).
    if let Some(w) = world.structures.as_ref() {
        if let Some(j) = &w.census().joins {
            out["joins"] = serde_json::json!({ "asked": j.asked, "dropped": j.dropped, "street_builds": j.passes, "legs": j.legs, "caps": j.caps, "raised": j.raised, "raised_max_m": j.raised_max_m, "rescued": j.rescued, "lost": j.lost });
        }
        if !w.bores().is_empty() {
            out["tunnels_drawn"] = serde_json::json!({ "bores": w.bores().len(), "terrain_cuts": w.terrain_cuts().len(), "portal_canopies": w.terrain_cuts().iter().filter(|c| c.canopy).count(), "lights": w.all_tunnel_lights().len() });
        }
    }
    out
}
