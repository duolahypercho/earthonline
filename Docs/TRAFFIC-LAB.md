# Traffic Lab — realism roadmap

The objective is to validate the traffic rules before spending time on the
city's visual assets. A top-down replay makes failures attributable to a
particular vehicle, lane, movement and simulation time.

## Run the diagnostic

From `native/`:

```sh
cargo run --release -p eo-sim --example traffic_lab -- bench-out/traffic-lab.html
```

Open the resulting local HTML file in a browser. It is an offline diagnostic
report, not a browser build of the game: the Rust example records the same
`eo_sim::World::step` that the game uses, at 60 Hz, with a sample every 12
steps for traffic and every step for the pedestrian probes. The viewer only displays recorded data. There is no city extract,
GPU, model, texture, player, physics world, audio, or network load. The
small standards registry is read through `Standards`, `SignalTiming`, and
`StreetDims`; `NativeSource` generates the lanes, movement plans, footways,
and controller plans. Fixture coordinates are synthetic, not surveyed roads.
Each template uses a fixed seed, no distance-tier stepping, and a finite
network whose terminal lanes recycle cars.

| Template | Things to inspect | Initial acceptance checks |
|---|---|---|
| Following | Nose direction, vehicle dimensions, spacing, acceleration and braking | No overlaps; no off-lane poses; responsive queues; no unexplained speed jumps |
| Signal junction | Timing, stop bars, turn arcs, conflicting claims, pedestrians | No uncommitted red entries or starts outside WALK; no conflicting occupied paths |
| Uncontrolled T | Priority, gaps, competing turns | No overlap or forced crossing; no starvation in a finite observation window |
| Short-block queues | Two controllers, short exit lanes, spillback | Keep the junction clear when discharge is blocked; record gridlock relief separately from normal flow |
| Person ahead | Controlled pedestrian observation already occupying the route | Sense, brake, hold until clear, then resume; no sampled body overlap |
| Unexpected person | New observation supplied while the car is driving | Same safety response; no artificial speed clamp or early restart in the controlled probe |
| Left turn | Opposing through vehicle, legal turn arc and intersection claims | Both clear without overlaps; the through movement precedes the conflicting left turn in this fixture |
| Right turn | Approach, turn indicator, corner path and destination lane | Complete the intended legal movement without overlap or off-lane pose |
| Right turn + cyclist | Stationary scripted bicycle observation on the exit lane | Detect, yield and resume after clearance; this is a proxy, not bicycle dynamics or bike-lane priority |

Play, pause, scrub or step by one recorded frame. Toggle lane arrows, vehicle IDs,
pedestrian paths and the selected route. Click a car to inspect its stable
entity ID, actual speed, lane, next movement, signal and claim state. Body
length and width are the simulation's own presented dimensions. The small
colored lines at controlled approaches are diagnostic overlays, not a
street-marking design. Overlaps and off-lane counts are sampled; red entries
and WALK violations are cumulative engine counters. Do not interpret a
clean sample or zero counter as proof of realistic behavior on every map.
Use **Jump to first issue** to select the vehicle involved in the first
sampled pedestrian/body overlap. Terminal attempts count retry calls, not
successful relocations; compare them with the separate respawn counter.

The seed-1 baseline is already useful: the following corridor reaches
57,635 terminal attempts without a successful relocation in 90 seconds.
The other three templates record pedestrian/body overlap samples, with the
first at 68.4 s (vehicle 7), 71.4 s (vehicle 3) and 34.6 s (vehicle 6),
respectively. These are diagnostics, not approved gate results. Prioritize
the boundary-flow and pedestrian-yield fixes before increasing population.

## Pedestrian safety: detection and unexpected entry

The two controlled probes capture every fixed step (60 Hz). They inject a
scripted pedestrian observation into `World::step`; the vehicle still uses
the real fleet sensing, acceleration, braking, hold and movement code. They
do not simulate a physical jump, attention, occlusion, injury or a crash.
The inspector distinguishes **person present** from **person sensed**; these
are not equivalent. The brake lamp reports actual fleet state, not an
inferred reason. The above-brake-limit counter detects speed reductions
exceeding the fleet's own configured deceleration on a single step.

The controlled acceptance test checks detection, braking, a stationary
hold while the person remains, safe resumption after clearance, and no
body overlap or surprising deceleration clamp at any recorded step.
Both probes pass the seed-1 test. The ambient intersection overlap failures
above remain open; these easy-to-isolate probes are not a general safety
certification.

### Realism design

1. **Perception:** Track people across steps, including motion toward the
   roadway and temporary occlusion. Predict possible path conflict instead
   of relying only on the present forward corridor. Distinguish stationary
   people, ordinary crossings, and unexpected movement.
2. **Response:** Release the accelerator, brake progressively for anticipated
   crossings, and request emergency braking for unexpected hazards. The
   achievable response must depend on the vehicle's physical state and road
   grip, with a bounded driver response model calibrated to sourced data.
3. **Safe hold:** No creeping or timeout-based release toward a person still
   occupying the swept route. Resume only after the person and their likely
   path are clear. A horn or warning is secondary to braking; never take an
   unverified evasive route into other road users.
4. **Physics and recovery:** Do not silently teleport, freeze in mid-air or
   claim an impossible instant stop. If physics reports contact, stop the
   vehicle safely, retain the event and let the emergency-response system
   handle it. Do not model injuries in the traffic diagnostic.
5. **Regression:** Exercise slow crossings, turning conflicts, occlusion,
   stopped pedestrians, queues behind yielding vehicles, different weather
   and vehicle classes. Use controlled safe fixtures and verify the whole
   run, not just a screenshot; keep independent deterministic event logs.

The current fleet uses a geometric forward-corridor sensor, occupied crossing
bands, speed caps and an emergency displacement clamp. Its sensing is not a
human perception model; the timeout-based creep path and displacement clamp
need separate review before claiming physically realistic emergency behavior.

## Turning and cycling

Templates 07–09 pin legal driver intent, then record the real production
corner trajectories, signals, admission claims and exit lanes at 60 Hz.
At seed 1 all three traverse the selected corner and reach the exit, with no
sampled body overlaps or off-lane poses. The opposing through car in the
left-turn fixture is held while the left-turn vehicle completes first; this
is a failed priority-order expectation, not a successful right-of-way test.
Use **Jump to first issue** to investigate it. The cyclist observation case
detects the proxy, stops and resumes after it is cleared. The proxy is
stationary and uses the pedestrian sensing channel; it is not a simulated
cyclist or a test of a moving bicycle's legal priority.

The next milestone is a single shared traffic simulator for motor vehicles,
motorcycles, bicycles and pedestrians. A native bicycle actor needs its own
dimensions, acceleration, braking, steering, lane membership and route
intent. Reuse sourced bicycle-lane and crossing geometry, not a car lane
painted green or an arbitrary sidewalk route. Model separated bike lanes,
shared streets and crossing approaches; include motorcycle-specific body
and motion separately. Test following and passing, bike-lane continuity,
right-turn cyclist yielding, left-turn opposing through traffic (including
cyclists), protected signal phases, and visibility at intersections. The
turning planner must honor priority and safe path occupancy before claiming
the junction; a clear body-overlap counter alone is insufficient.

## Current rule inventory and gaps

| System | Existing implementation | What to improve or verify |
|---|---|---|
| Lane graph and turn geometry | `eo-sim/network.rs`, `geom.rs`, `native.rs`: directed lanes, legal links, geometric turns, conflict matrix | Compare native lane roles and movement restrictions against the sourced map; add deterministic merge and one-way fixtures |
| Vehicle dynamics | `vehicles.rs`, `consts.rs`: leader spacing, acceleration/deceleration, turn caps, cross corridor gaps | Measure headway, jerk, braking onset, and queue discharge distributions; add controlled two-car fixtures |
| Traffic signals | `signals.rs`, `standards.rs`: standards-derived phase timing and stop offset | Verify approach-specific legal movements, protected permissive turns, red-to-green startup and short-block coordination |
| Stop signs and priority | `vehicles.rs` supports `ControlKind::Stop`; `import_js.rs` can import a stop contract | `native.rs` currently constructs signal control, not native stop signs. Add sourced stop control and verify arrival order, ties, full stops, priority and crossing yield before enabling map-wide |
| Pedestrians | `pedestrians.rs` crossing states, WALK windows and vehicle gaps; `vehicles.rs` yielding | Expose crossing bands, pedestrian indications, clearance and yield reason; verify turn conflicts and unsignalized crossings |
| Junction admission | `vehicles.rs` claim tracking and geometric holds; exit occupancy checks | Log why a car was denied admission, conflict occupancy, and every gridlock recycle; make controlled blocked-exit fixtures |
| Routing and population | Weighted legal links, terminal/near-focus recycling and detail tiers in `region.rs` | Add origin/destination routes, time-of-day demand, queue conservation, and realistic spawn/despawn at unseen boundaries |
| Lane changes, parked obstacles | No free lane-changing planner or parked/wreck blocker bypass in the headless fleet | Add safe gap acceptance and lane-change trajectories; integrate drivable obstructions and curbside loading through sourced rules |

## Implementation order

1. **Observe first.** Keep replay from the real simulation as the baseline.
   Add per-step event logging: brake reason (leader, red, stop, turning,
   person, occupied exit), entry and exit times, IDs, lane changes, vehicle
   gaps, claim owner and exact rule failure. Record seed and fixture name.
2. **Isolate basic behavior.** Build explicit controlled initial states for
   two cars following, stationary leader, queue discharge, left/right turn,
   four-way stop, major/minor priority, pedestrian yield and blocked exit.
   Add assertions for full-run events and numeric traces; do not alter the
   simulator solely to make a replay look smoother.
3. **Complete the sourced road rules.** Import stop controls and lane-use
   restrictions from verified map data. Verify movement authorization,
   arrival order and yielding, including legal turn-on-red exceptions and
   red arrows. Source every road-rule value through `CaStandards`; no new
   ad hoc traffic timings or dimensions.
4. **Improve driver behavior.** Evaluate smooth speed transitions, distinct
   but bounded driver response, following headway, merge and lane-change
   gap acceptance, negotiation and safe perception of pedestrians. Compare
   against measured, licensed traffic observations; avoid tuning to one seed.
5. **Build credible demand.** Add route intent and boundary flows, then
   scenarios for peak jams, transit, curbside activity, and player-created
   obstacles. Validate throughput, queue length, stopped time, travel time,
   vehicle conservation and no invisible mid-road teleporting.
6. **Integrate with the city.** Reuse the same event diagnostics on a small
   sourced neighborhood before enabling further world integration. Keep
   deterministic tests, fixed-step time, and the existing performance budget.

## Gates

- Synthetic and sourced fixtures: run all steps, not just sampled frames;
  zero illegal red entries, starts outside WALK, overlapping vehicles, and
  unapproved lane/movement violations. Treat recovery as an explicit event.
- Measure rather than guess: headway, acceleration, queue discharge, travel
  time, lane change success and throughput against sourced observations.
- Re-run fixed seeds and vary seeds/demand. Inspect failures in the replay
  before moving on. Separate normal terminal recycling from gridlock relief.
- For production simulation changes use the repository's verification matrix,
  including dependent crates, frame performance and the visual gate where
  applicable. This offline report does not change the game's visual output
  and does not change the recorded visual gate's REJECT verdict.
