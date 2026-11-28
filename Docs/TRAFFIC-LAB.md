# Traffic Lab — realism roadmap

The objective is to validate the traffic rules before spending time on the
city's visual assets. A top-down replay makes failures attributable to a
particular vehicle, lane, movement and simulation time.

## Run the diagnostic

From `native/`:

```sh
cargo run --release -p eo-sim --example traffic_lab -- bench-out/traffic-lab.html
cargo run --release -p eo-sim --example traffic_lab -- bench-out/soma.html --map soma
cargo run --release -p eo-sim --example traffic_lab -- bench-out/golden-gate.html --map golden-gate
```

Open the resulting local HTML file in a browser. The Rust example records
the game's `eo_sim::World::step` at 60 Hz and displays a sample every 12
steps for traffic and every step for controlled probes. The viewer plays
back those records without a GPU, vehicle models, audio, or a network
connection. The synthetic fixtures exercise a finite network with the
legacy fixture recycler. The map replays use the shipped San Francisco
extract, mapped signals, road layer tags, and the live population rules.
They hold the map window fixed and disable distance-tier stepping.
The Golden Gate replay shows grade separation from map tags. The game
with `--structures` reads its levels and heights from the built bridge
and ramp geometry.

Use `--traffic-bend-braking` to preview how the fleet plans its approach
speed for bends inside a lane and for curves after a junction. The speed
uses the path angle and the vehicle footprint used for heading and
clearance. The switch is available in the lab, the game, and the server.
Use `--no-traffic-bend-braking` to restore the existing speed policy.
The replay records when the car slows and when its brake lamps engage.
For a native capture from a replay frame, copy the vehicle's `braking`
value into the matching `vehicles` entry in `EO_CAPTURE_POSES`.

To replay the paved lane cross-sections enabled by the game's
`--traffic-street-plan` preview, export the street plan and pass it to the
map replay. Both commands run headlessly.

```sh
cargo run --release -p eo-agents --example traffic_streets -- bench-out/traffic-streets.json
cargo run --release -p eo-sim --example traffic_lab -- bench-out/soma-painted.html --map soma --street-plan bench-out/traffic-streets.json
cargo run --release -p eo-sim --example traffic_lab -- bench-out/folsom-fourth.html --map folsom-fourth --street-plan bench-out/traffic-streets.json
```

Use the same ground scale for both commands.

For the built bridge, freeway and tunnel configuration, export the
joined street plan and the structure data together. This records the
height of each road along its own surface, including where two roads
share a level label. The street export also carries the drawn junction
surface, which provides the height along the lanes and turning paths in
that junction. Both the map replay and the city sweep accept the
structure data. Omit `--tunnels`
at export to match a game using `--structures` alone.

```sh
cargo run --release -p eo-agents --example traffic_streets -- bench-out/traffic-streets-joined.json --structure-levels bench-out/traffic-levels.json --tunnels
cargo run --release -p eo-sim --example traffic_lab -- bench-out/traffic-deck.html --map tile:9,-6 --street-plan bench-out/traffic-streets-joined.json --structure-levels bench-out/traffic-levels.json --seed 7
```

For a built-plan replay, vehicle clearance uses the road surface
profiles when both vehicles have one. The same reference gives the
local and online vehicles their grounding at deck handoffs. Older plan
exports and plain level tables still load.

The simulator matches paved pieces by their source way and heading. A
consistent cross-section gives the count, widths and offsets of the paved
through lanes. The mapped direction of travel, connecting nodes and
signal membership still govern routing. A segment that spans a change in
cross-section keeps its existing profile. Freeway, ramp, bridge and
tunnel lanes use their existing structure path.
A one-way loop that the map extract has collapsed into a line out and
back is left out of the simulator's lane and sidewalk graph. A loop
with a distinct return path keeps its normal connections.
The preview excludes paved single shared lanes from autonomous routing
while preserving their footways. The paved lane role takes precedence
when the road extract lists two lanes or the registered shape needs
more than one lateral offset. Those roads need alternating traffic
before the fleet can use them.
Pedestrians on the paved plan stay on the same corner when changing
footways. They use the crossing state to reach the opposite curb.
On a divided road, the middle sidewalk can end where the opposing
carriageway bends into it. The paved preview trims the walking line
against that carriageway at grade. It keeps the usable sidewalk on each
side of the cut and the crossing links that still reach the original
junctions.

A fresh street-plan export also carries gaps in the drawn sidewalks.
The walking graph follows those gaps wherever the street planner leaves
room for another carriageway. It also trims parallel sidewalk fragments
that encroach on a neighboring traffic lane at the same level. At a
shared junction, a lane that meets the sidewalk at an angle can cut
any overlapping part of the approach, including before its final
polyline piece. Where an unconnected approach ends within the sidewalk
corridor, the drawn lane trims only the part of the walk it occupies.
Surviving kerb connections and transverse through crossings keep their
crossing rules.
Use `--no-drawn-walk-gaps` to compare with the older walking graph on
the same export and seed.

The paved preview also restores junctions where the simplified road
extract skipped a shared point. The street plan identifies which ways
enter each drawn junction. Each matching approach keeps its own
centerline and joins the other approaches at a shared routing node. The
split stays at the same place as tiles stream in and out. Bridges, tunnels
and other grade-separated ways keep their surveyed connections. Where the
built deck meets a drawn junction, its traffic follows the connected
surfaces through the seam.

The drawn street plan also enables source-matched stop signs. The replay
shows these as white bars. Hover over a bar to see its OpenStreetMap node
ID and whether the source tags it as an all-way stop. **Complete stops**
counts vehicles recorded at zero speed at their hold point. Use
`--no-mapped-stops` for a comparison with the same street plan and seed.
[Stop data provenance](../native/eo-data/data/sf-traffic-stops.NOTICE.txt)

Use `--map stop:65297322` to inspect Beach Street and Cervantes Boulevard,
or `--map stop:11348247284` to inspect the signed approach at Bryant and 21st.
These close views start with metre-scale vehicle bodies and pedestrian
footprints. Toggle **Vehicle dots** for easier tracking across a wide area.

Around bends the simulator checks the path swept by each vehicle's body,
including the rear of a long vehicle in the adjacent lane. A junction
reservation also accounts for a vehicle returning through the same
opening on a loop.

The white and yellow strokes show the street's longitudinal paint. Cyan
guides show the simulator's lane centers. A corner follows its own turn
path through the gap in the paint. Colored signal bars show the
simulator's stop positions. The Folsom and 4th view starts close enough
to compare a chassis footprint with both dividers.

With the paved plan, a car can signal and move left to pass a slower
moving leader across a broken white divider between straight through lanes.
The driver checks front and rear gaps, people, and room before the
junction or the end of the divider. The inspector shows lane changes and
horn warnings. A horn accompanies braking for an immediate collision
hazard, including sudden pedestrian entry. Normal pedestrian yielding
and stopped traffic queues stay quiet. Use `--no-traffic-passing` for
a comparison with the same street plan.
[Lane changes and passing](https://www.dmv.ca.gov/portal/handbook/california-driver-handbook/navigating-the-roads/)
· [Horn guidance](https://qr.dmv.ca.gov/portal/handbook/california-driver-handbook/introduction-to-driving/)

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

Play, pause, scrub or step by one recorded frame. Use vehicle dots for an
overview or switch to scaled chassis footprints. Toggle lane arrows, vehicle
IDs, pedestrian paths and the selected route. Gold lanes run above grade
and purple lanes below it. Click a car to inspect its stable
entity ID, actual speed, lane, next movement, signal and claim state. Body
length and width are the simulation's own presented dimensions. The small
colored lines at controlled approaches are diagnostic overlays, not a
street-marking design. Overlaps and off-lane counts are sampled; red entries
and WALK violations are cumulative engine counters. Do not interpret a
clean sample or zero counter as proof of realistic behavior on every map.
Use **Jump to first issue** to find the earliest recorded problem. Position
jumps are checked on every fixed step and count same-ID moves over 1 m.
Terminal attempts include unsuccessful retries. The live recycler gives
off-camera replacements a new ID.
Native lanes that meet at the same point and road height carry the
remaining travel in that step directly onto the receiving lane after
the driver claims the junction movement.

## Check wheel placement against the street mesh

Write a map replay with a `.json` extension and the measured vehicle
catalogue. The headless street exporter can check those wheel positions
against the native ground mesh and report the first non-carriageway
sample on each lane. Use the same structure options for the street
export and the wheel audit.

```sh
cargo run --release -p eo-sim --example traffic_lab -- bench-out/victoria.json --map tile:-20,28 --street-plan bench-out/traffic-streets.json --vehicle-scale-report bench-out/traffic-scale.json --drive-model-footprints --seed 7
cargo run --release -p eo-agents --example traffic_streets -- bench-out/victoria-wheels.json --wheel-audit bench-out/victoria.json
```

Add `--structures` and, where used in the export, `--tunnels` when
auditing joined streets. For ground traffic, the audit classifies the
highest street triangle at each wheel. For bridge and tunnel traffic and
their transitions, it checks the drawn deck, bore floor and adjoining
street at the replay's route height, including triangles assigned to
neighboring tiles. It reports which surface supports each wheel, using
half the measured wheelbase as the allowable height difference, and
counts missing route heights separately.
It distinguishes empty space from paving at the wrong elevation and
records the two heights when it finds an elevation mismatch.

The structure export also carries the painted Golden Gate lane center
lines. The replay uses those center lines and their deck elevations while
keeping the mapped links to the approaches.

The wheel coordinates are planar projections from the replay's position
and heading. The report includes sampled vehicle roof heights as ratios
to the loaded playable character.

Add `--body-probes` to check the corners, edge midpoints, and center of
each measured chassis footprint. The report gives probe counts by
surface, the number of poses with a point off the carriageway, and the
first offending point overall and by lane. Use these sampled footprint
checks with the wheel results when testing a turn or junction.

An experimental lane layout can supply a measured `VehicleEnvelope`
for each lane whose turns have been checked against the pavement. The
simulation compares vehicle length and width before admitting a new
vehicle or choosing a link into that lane. Include the approaches that
commit a vehicle to the restricted turn so that it can choose a suitable
route at the preceding junction.

Use the playable character's measured standing height for vehicle scale
comparisons. In matched native captures, place the avatar's feet on the
walkable surface and the vehicle on the road. Keep the camera, avatar
model and model scale fixed, and record each vehicle's roof height in
meters alongside its ratio to the avatar.

The original seed-1 following fixture counted 57,635 terminal retries
in 90 seconds. Retries now use the existing terminal interval even if
there is no eligible landing. The pedestrian overlap counters remain
active in the ambient fixtures.

## Preview surveyed junction pavement

For offline comparisons, set `EO_SURVEYED_JUNCTION` to a local JSON file.
Use the same file in the native app and the headless street exporter.
Set `groundScale` to the city's active horizontal scale. Provide
`positionsXZ`, `outerBoundaryXZ`, and `islandBoundaryXZ` as arrays of
world-space `[x, z]` pairs. Both boundaries close implicitly. Provide
`carriagewayTriangles` as counterclockwise triples of zero-based indices
into `positionsXZ`. The triangles should fill the outer boundary and
leave the traffic island open.

The preview fits surveyed pavement to the native terrain and approach
heights. Run the wheel audit with `--no-surveyed-junction` to compare
against the original street mesh. Keep local survey inputs with their
source and licensing records.

An optional `trafficRoutes` object in the same file connects a surveyed
circuit when the paved traffic plan and measured chassis are enabled.
It gives the `collapsedWay` source ID, `vehicleEnvelopeM` as maximum
centered length and width, `circulation` as the directed lane lines in
order, and `approaches` for existing native lanes. Each approach has its
`lane` key, its original downstream `tile` as `[x, z]` tile indices,
an `inbound` boolean, a `line`, and an optional mapped `stopId` for
an inbound approach. Each line carries world-space `points` in travel
order. The exporter samples their heights from the same carriageway.
The circuit retains its mapped direction and uses the source speed
and class lane width. Approach endpoints away from the circuit stay
on their original native nodes. The circuit and its affected approach
tiles stream as a group.

Use `--traffic-street-plan` and `EO_TRAFFIC_MODEL_FOOTPRINTS` in the
native app or server to activate a compatible route preview. In Traffic
Lab, export the same pavement with `traffic_streets` and then use
`--street-plan`, `--vehicle-scale-report` and `--drive-model-footprints`.
The pavement-only preview keeps the original traffic routing.

## Sweep the shipped road map

The city audit runs a 90-second fixed-focus window at the center of each
tile indexed by the vehicle-road source. It uses the same fleet, clock,
population settings and diagnostics as the 2D city replay. It saves a
JSON summary per window, with no rendering or frame recording.

```sh
cargo run --release -p eo-sim --example traffic_lab -- bench-out/traffic-city.json --audit-city --street-plan bench-out/traffic-streets.json --seed 7
cargo run --release -p eo-sim --example traffic_lab -- bench-out/traffic-tile.html --map tile:7,-2 --street-plan bench-out/traffic-streets.json --seed 7
```

Run the example from `native/`. To reproduce an issue, use the report's
`map` and `seed` with the same street plan, passing and mapped stop settings.
`mappedStopRecords` counts distinct source nodes bound to lanes across the
audit. Each window lists its bound node IDs, affected lanes and complete stops.
`personSamples` and `crossingPersonSamples` count pedestrian observations
before comparing how often a vehicle body overlaps a person.
`crossingsStarted` and `crossingsCompleted` count whole trips, so a long
wait in a crossing can be distinguished from crossing throughput.
`firstIssueStep` is the first diagnostic frame with an overlap or
violation. Divide it by 60 for replay time in seconds. The audit counts
same-ID horizontal jumps and jumps in a vehicle's known road-reference
height separately. Both checks use the 1 m threshold at every fixed step.
Repeated overlaps
count once per sampled frame. A window with zero `maxCars` provides no
driving coverage. `framesOffLaneCenter` also counts planned lane
changes, so it is a separate observation from the first issue.
This sweep exercises local routing around each focus. A drive through
streaming tile boundaries and deck heights needs its own check.

On a native lane bend, the vehicle centre follows the lane and its
heading turns across the length of its body. Opposing traffic on the
same bend checks clearance along that lane instead of projecting a
straight ray through the opposing lane. Local traffic starts on lanes
with a reachable continuation, so a disconnected service road cannot
fill with cars at startup.

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
At seed 1 all three traverse the selected corner and reach the exit. The
left-turn fixture checks that approaching through traffic clears first.
The junction admission check uses movement conflicts, approach distance,
the applicable signal aspect, and whether a nearby vehicle occupies the
turn's swept footprint. The sweep uses the vehicle's dimensions and
continues onto the receiving lane. When two reservations overlap, the
waiting movement can go after the other vehicle passes the last part
of its path that overlaps. The body turns continuously as it transfers
from the link onto the receiving lane. See the
[California Driver's Handbook, section 7](https://www.dmv.ca.gov/portal/handbook/california-driver-handbook/laws-and-rules-of-the-road/).
The cyclist observation case
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
| Stop signs and priority | `native/stops.rs` binds sourced all-way and approach tags to the drawn lane graph. `vehicles/stops.rs` records full stops, resolves arrival order and yields to crossing traffic and occupied crossings. | Verify coverage and moving traffic at sourced junctions. Compare tagged stop positions against field evidence. |
| Pedestrians | `pedestrians.rs` crossing states, WALK windows and vehicle gaps; `vehicles.rs` yielding | Expose crossing bands, pedestrian indications, clearance and yield reason; verify turn conflicts and unsignalized crossings |
| Junction admission | `vehicles.rs` claim tracking and geometric holds; exit occupancy checks | Log why a car was denied admission, conflict occupancy, and every gridlock recycle; make controlled blocked-exit fixtures |
| Routing and population | Weighted legal links, stable live trip IDs, off-camera replacements and streamed route retirement | Add origin/destination routes, demand at network boundaries, and queue conservation |
| Lane changes, parked obstacles | No free lane-changing planner or parked/wreck blocker bypass in the headless fleet | Add safe gap acceptance and lane-change trajectories; integrate drivable obstructions and curbside loading through sourced rules |
| Player contact | In local play, nearby traffic has kinematic chassis in Rapier. The shared lane fleet senses the player vehicle as an obstacle on its level | Model two-way impact response for traffic vehicles, then integrate authoritative online contact |

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
- Check vehicle proportions against the loaded playable avatar. Record the
  cooked vehicle dimensions and the avatar's standing height at default
  instance scales, then capture both on the same road surface at comparable
  camera depth. Check the roof line, wheel contact, and scale across nearby
  detail levels. Compare the contact body with the visible vehicle footprint
  so a close pass against the avatar agrees with the picture. Repeat the
  comparison when changing fleet assets, avatar scale, or vehicle presentation.
  Use the same avatar when checking roadside props and bridge details against
  their sourced dimensions.

  From `native/`, run
  `cargo run --release -p eo-agents --example cook_agents -- --scale-report`
  for a headless JSON report. It measures the loaded playable avatar and
  the indexed vehicle meshes at each detail level, using the game's instance
  scales. The dimensions are in metres and in multiples of the avatar's standing
  mesh height. For each vehicle, `chassis_contact` records the width and length
  used by local traffic collisions. The signed edge differences under each detail
  level show where the drawn mesh and the contact footprint diverge along either
  axis. A null chassis means the mesh supplied no contact bounds.
  `simulation_footprints` records the class footprints used by the traffic
  controller and the weighted catalogue list for each class. Add
  `--car-detail` for the detailed far mesh or
  `--test-character <id>` for the same local avatar selection used in play.

  Pass the report to a map replay with `--vehicle-scale-report <scale.json>`.
  The dashed footprint follows the near model the game selects for each ID.
  Select a vehicle for its length in metres and in standing player heights.
  Its side view aligns the measured chassis and wheels with the loaded avatar's
  standing height on the same road datum. It labels roof height in metres and
  standing player heights. The selected avatar's model and scale come from the
  supplied report.
  The replay checks same-surface overlaps over every simulation step and
  counts the steps where measured bodies overlap past the standard diagnostic
  threshold while the class footprints miss the pair. The comparison uses the
  near model's top-down bounds at the recorded road pose. The same flag adds
  those measurements to each road window in `--audit-city`.

  ```sh
  cargo run --release -p eo-agents --example cook_agents -- --scale-report > bench-out/traffic-scale.json
  cargo run --release -p eo-sim --example traffic_lab -- bench-out/presidio-scale.html --map tile:-16,-14 --street-plan bench-out/traffic-streets-joined.json --structure-levels bench-out/traffic-levels.json --vehicle-scale-report bench-out/traffic-scale.json
  ```

  Add `--drive-model-footprints` to test the controller with those dimensions.
  It selects the model before each admission and checks the incoming model
  before recycling an off-camera vehicle. The controller uses a rectangle
  centred on the vehicle datum that contains the measured near mesh footprint.
  Use `EO_TRAFFIC_MODEL_FOOTPRINTS=/path/to/traffic-scale.json` to try the
  same controller in local play or on the server. Generate the report from
  the asset set and vehicle scale that the game uses. Pass
  `--no-traffic-model-footprints` to disable the preview for a matched run.
  With `--traffic-contact`, the server also uses the report's `physical_chassis`
  data when a traffic car enters physics. The cooked bounds, wheel positions,
  wheel radius and selected model determine the contact body. Reports carrying
  only XZ bounds use the class chassis for server contacts.
  The `great-highway-clearance` map holds the lead bus and repeats the same
  approaching bus and lane poses for both controller settings. It uses the
  joined street plan and requires a scale report.
- For production simulation changes use the repository's verification matrix,
  including dependent crates, frame performance and the visual gate where
  applicable. This offline report does not change the game's visual output
  and does not change the recorded visual gate's REJECT verdict.
