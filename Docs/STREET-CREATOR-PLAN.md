# Street creator plan

Status: proposal, 2026-11-26. Nothing here is built yet. Decisions that need
the owner are listed under [Decisions needed](#decisions-needed).

Build one tool of our own that turns map data into finished streets anywhere
on Earth. It covers everything from one property line to the other:
carriageways, junctions, kerbs, gutters, sidewalks, crossings, driveways,
alleys, parking lots, plazas, medians, grass verges, tree wells, trees,
hedges, fences, walls, guardrails, highways, ramps, bridges, tunnels and
underpasses. San Francisco is the first city and the reference. Every part is
built so that a second city with different rules is new data, not new code.

The tool has two jobs, and the first comes before the second.

1. **A correct layout.** Every street, junction and sidewalk is in the right
   place at the right width and height. It stays correct however many streets
   meet, at whatever angle, and wherever a street widens, narrows, splits,
   goes over or goes under.
2. **A photoreal surface.** Materials, wear, markings and roadside life are
   placed *from* the layout. A wheel path is polished because the tool knows
   where wheels run. A kerb is grimy because the tool knows where the gutter
   is. Without the layout, the look can only guess.

## Where we start

The facts below come from a read of the current code on 2026-11-26.

| Area | Today | What it means for a worldwide tool |
| --- | --- | --- |
| Street builder | [eo-street](../native/eo-street/src) (8,600 lines) is a port of the old browser builder. Each street is a ribbon. Junctions are drawn by trimming the ribbons back and filleting their corners. A straight chord is the fallback where a fillet fails (`plan.rs:1220`, `plan.rs:1936`). A later pass removes overlapping ribbon spans by priority (`plan.rs:1525`), backed by a paving rank in `paving.rs`. | It works on the San Francisco grid. Overlaps are cleaned up after they happen instead of being prevented. |
| Input | One San Francisco export, `street-city.json.gz`. It holds 96,654 segments: 50,628 footways, 14,262 residential and 12,743 service ways. Every segment has the id `sf-street-<osm way>`. | It is not a pipeline from map data to streets. |
| Cross-section | The geometry uses one width per segment plus a sidewalk width per side. Lanes, parking, bike lanes, buffers and medians exist only as paint and as traffic data (`paint.rs`, `plan.rs:268`). Right-of-way is empty on every segment, so frontage falls back to a 6 m reach. | No real median, planting strip, verge or property line is drawn. |
| Junctions | A junction needs three or more ends within 0.6 m. Nearby nodes are not clustered, so a divided road produces several small junctions. Roundabouts are not read. A width change between two segments has no taper; the ribbons abut at their own widths. A dead end just stops. | These are exactly the hard cases a worldwide tool must handle. |
| Heights | The road datum is terrain plus a 0.45 m lift per station. Kerb, crown and cross-fall come from the standards. Tile seams line up because both sides recompute the same values; their vertices are not welded. Retaining walls over the bank limit are not ported (`mesh.rs:376`). | This needs one height rule for the whole layout, applied after the 2D layout is final. |
| Raised and underground roads | [eo-structures](../native/eo-structures/src) (12,800 lines) builds freeways, ramps, road bridges, tunnels and portals from the street plan. It is off by default (`--structures`, `--tunnels`). | The structure is good. The tool should feed it a better cross-section and keep its solver. |
| Driveways, alleys, lots | Driveways get kerb cuts only, and only with pavement repairs on (`driveway.rs`). Car parks exist only as OSM outlines in `sf-lots-parks.json.gz`, used to keep trees off them (`eo-landmarks/src/grove.rs`). No lot paving, stalls, aisles or islands are drawn. | This needs to be built. |
| Roadside | [eo-props](../native/eo-props/src) places furniture, survey street trees, basins, signals, signs, parking and overhead wires. It works out its own furnishing band from the plan (`lib.rs:600`). The surveys are compiled-in exports whose generators are gone. Fences, hedges and yard ground exist only in an unwired `yards.rs`. There are no guardrails at street level and no grass verges ("verge" in the code means a concrete bank). | The roadside needs zones from the layout, not a band each consumer works out for itself. |
| Rendering | The street materials are strong: scanned asphalt and concrete, CC0 sets, decals, grime, wheel-path and oil decals, and the cited `--street-pbr` stack. The GPU vertex has no UVs, lane ids or wear masks, so the shader infers lanes from kerb distance (`street_pbr.wgsl:171`). | The look is held back by missing layout data, not by missing shaders. |
| Regions | There is one standards table, California's. `drivingSideSign` is parsed but never read, and right-hand traffic is hard-coded in the limit lines, the decals and the simulator. The world frame is a fixed San Francisco origin (`eo-data/src/lib.rs:842`). | Nothing yet supports a second region. |
| Tools | There is no street inspector or debug overlay. A top-down view needs a `--shots` pose with tilt 0. | We need our own tools to see and check what we build. |

The visual gate's recorded verdict is **REJECT**
([VISUAL_QUALITY_GATE.md](VISUAL_QUALITY_GATE.md)). Nothing in this plan
changes that without a new blind review.

## What we keep, adapt and replace

This is not a rebuild of the streets. Most of today's work stays. What gets
replaced is the 2D layout core. It is the one part that does not extend to the
hard cases: each case it could not handle got a cleanup pass afterwards, and
the cleanup passes exist only because the ribbons overlap. Once junctions are
built as one area and sidewalks come from blocks, nothing overlaps and the
cleanup passes are no longer needed.

| Part | Where | Plan | What changes |
| --- | --- | --- | --- |
| Surface shading: scanned and CC0 materials, decals, grime, street life, `--street-pbr` and its wear | `eo-render`: `street.rs`, `street_pbr.rs`, `cc0_decals.rs`, `weathering.rs`, `street_life.rs`, `draw.wgsl` | **Keep** | It gets real inputs from S9: lane id, distance along the street (UV), wheel-path coordinate, wear seed. Decals are placed from the layout's lanes instead of the kerb-distance guess. |
| Height rules: crown, cross-fall, gutter pan, kerb height, ramp depression, building floors, roadbed fit, driveway grading | `eo-street`: `options.rs`, `mesh.rs` ramps, `grounding.rs`, `roadbed.rs`, `driveway.rs` | **Keep** | They are applied in S8 to the new layout. The not-ported retaining wall and yard floors are added. |
| Crossing evidence | `eo-street/src/evidence.rs` | **Keep** | It becomes an input to S7. |
| Standards registry and deviations | `eo-data/src/standards.rs`, the California table | **Keep** | It is extended to more than one jurisdiction (decision D1). |
| Surface tests | `eo-street/tests/surface.rs`, `tests/structure_joins.rs` | **Keep** | They run against both builders and become part of the checks. They are never loosened. |
| Grade-separated roads: profile solver, bents, decks, tunnels, portals, walls, joins | `eo-structures` | **Keep, adapt the input** | It reads the deck and bore cross-section from S2 instead of one width. |
| Roadside placement: furniture, survey trees, basins, signals, signs, parking, overhead wires | `eo-props` | **Keep, adapt the input** | `PropsWorld::band` (`lib.rs:600`) reads the layout's zones instead of working out its own band. |
| Lane paint and markings | `eo-street`: `paint.rs`, `mesh/marking.rs` | **Adapt** | Lanes come from the cross-section. Colour, pattern, crossing style and driving side come from the rulebook. |
| Surface mesh and tiles | `eo-street`: `mesh.rs`, `lib.rs` (`StreetWorld`) | **Adapt** | The same surfaces and layers are emitted from strips and junction areas, with welded seams and the new attributes. Player collision keeps reading the same layers. |
| Traffic sections | `plan.rs:268` (`traffic_sections`) | **Adapt, then replace** | An adapter serves today's sections from the layout. The lane graph from S9 replaces them in a later, separately reviewed change. |
| Layout core: prepare, run merging, node collection, junction trims, corner fillets and chords, corner paths | `eo-street/src/plan.rs` (2,211 lines) | **Replace** | Replaced by S1 to S4 in `eo-streetscape`. |
| Cleanup passes: carriageway yield, pavement overlap, paving rank, arterial corners | `plan.rs:1525`, `pavement_overlap.rs`, `paving.rs`, `arterial_corner.rs` (716 lines plus the yield pass) | **Retire** | They stay only with the old builder, as its fallback. |
| Input export | `eo-street/data/street-city.json.gz`, `city.rs` | **Replace** | Replaced by S0, which reads an OSM extract plus surveys. The old export is kept for the fallback and for comparison. |

How the change lands:

- **Side by side.** Today's builder stays the default and the fallback. The
  new layout runs behind `--streetscape` on the same build, so every
  comparison is the same scene with one switch flipped.
- **Measured first.** M0 records where today's builder actually fails in San
  Francisco. If those places are few, the old junction code keeps San
  Francisco's ground-level streets longer. The new layout then proves itself
  first on the hard set and the second region.
- **One consumer at a time.** Props, traffic, the player, structures and the
  renderer each move from today's `Plan` to the layout in their own reviewed
  change. Until then they read it through the M3 adapter.
- **Retirement only on evidence.** The old builder is removed only after
  matched captures at every hard-set place and the visual gate (decision D6).

## What stays the same everywhere and what changes

Streets around the world are built from the same parts: lanes, a kerb or
edge, a sidewalk or verge, crossings, signals and junctions. One builder can
make all of them. The differences are values, styles and props. Those go in a
**rulebook** (numbers and rules) and a **kit** (materials and props) for each
region.

| Changes by region | Examples |
| --- | --- |
| Driving side | Right (US, most of Europe); left (UK, Japan, India, Australia). Mirrors lane order, turn paths, stop lines, signal faces and parking. |
| Lane and kerb values | Lane widths by class, kerb height and profile, corner radii, design vehicle |
| Markings | Centre line colour (yellow in the Americas, white in Europe), line widths and dash patterns, crosswalk style (ladder, zebra, two lines), give-way and stop lines, words versus symbols |
| Signals and signs | Mast arm versus near-side pole, horizontal versus vertical heads, sign system and shapes |
| Sidewalks | Concrete slabs, pavers, setts, asphalt, tactile paving colour and layout, or no sidewalk at all |
| Roadside | Planting-strip habits, tree species and spacing, fence and wall types, guardrail types |
| Parking | Stall sizes, angles, aisle widths, accessible stall rules, kerbside parking rules |
| Street types | Shared streets, old-town lanes without kerbs, rural roads with ditches |

The builder never assumes a region. It reads the region's rulebook for every
value. If a value is missing, it falls back to the parent rulebook (city, then
state, then country), and the coverage report says so. An unverified value
still returns an error, as `CaStandards` does today. It never becomes a silent
default.

## How the tool is built

### One shared layout model

The centre of the tool is a single **street layout** for each tile: plain
data, with no GPU code. Every consumer reads it: the surface mesh, props,
structures, traffic, the player and the renderer. Today each consumer works out
its own version of the street (props work out a band, the shader guesses lanes,
the simulator gets its own sections). That ends.

The proposed owner is a new workspace crate, **`eo-streetscape`** (decision
D2). Ownership after the change:

| Crate | Owns |
| --- | --- |
| `eo-streetscape` (new) | Network cleanup, cross-sections, junction shapes, blocks and sidewalk zones, driveways, lots, roadside zones, the lane graph, stable ids, overrides, checks, the layout report |
| `eo-data` | Source loaders, the rulebook registry (today's `standards.rs`, extended to more than one jurisdiction), deviations |
| `eo-street` | The street surface mesh built from the layout: carriageway, kerb, walk, ramps, paint, LODs, collision surface, building floors |
| `eo-structures` | Decks, supports, tunnels, portals and walls built from the layout's grade-separated pieces; its height solver stays |
| `eo-props` | Objects in the layout's zones: trees, furniture, signals, signs, fences, hedges, guardrails, parked cars |
| `eo-render` | Drawing, with the layout's surface ids, lane ids, UVs and wear inputs in the vertex |
| `eo-app` | Switches, benches, captures, the report command |

### The stages

Each stage is a pure, deterministic function from the stage before it. Each
can be tested and drawn on its own, and gives every piece a stable id derived
from the source ids. A hand fix is an override keyed by those ids, applied
after its stage (see [Overrides](#overrides)).

| Stage | Input | Output | The hard cases it must handle |
| --- | --- | --- | --- |
| **S0 Ingest** | OSM regional extract; local surveys (DataSF and the like); the terrain | Source records with provenance, in a local frame per tile on a global grid (f64 world coordinates) | Different projections; data seams between surveys; missing tags |
| **S1 Network** | S0 ways and nodes | A clean graph: street pieces between junction clusters, and a level (ground, bridge, tunnel, layer) on every piece | Divided roads merged into one corridor with a median. Nearby nodes merged into one junction (offset T, short links, slip lanes). Roundabouts and mini-roundabouts. Mapped sidewalks used as evidence about the street they run beside, not drawn as separate ribbons. Dead ends and turning circles. Ways that cross without a node. |
| **S2 Cross-section** | S1 pieces, tags, surveys, rulebook | An ordered list of strips from centre to property line, for each piece and each side, with change points along it | Median (painted, raised, planted, barrier, rail). Travel, turn, bus, bike, parking and shoulder lanes. Gutter. Kerb type (barrier, mountable, flush, none). Sidewalk zones: edge, furnishing or planting strip (grass, tree wells, paving), throughway, frontage. Boundary: fence, wall, hedge, yard, building. A lane added or dropped, parking ending, or a bike lane starting all become **tapers** whose lengths come from the rulebook. |
| **S3 Junctions** | S2 at each cluster | One area per junction, built from where the kerb and edge lines of its legs meet; corner radius from the rulebook and design vehicle | Any number of legs (3 to 8 and more). Very acute and very obtuse angles. Legs of different widths. Roundabouts with central and splitter islands. Channelised turn islands. Merges, diverges and gores. Kerb ramps and bulb-outs at corners. One-sided sidewalks. |
| **S4 Blocks and sidewalks** | The outer edges of all paved areas | City blocks, as the areas the roads enclose. Sidewalk zones run along each block's whole edge, unbroken around corners. | No gap and no overlap, by construction. Blocks with no sidewalk. Plazas and shared streets (flush, no kerb). Stairs streets and paths through blocks. |
| **S5 Access and off-street** | S4, service ways, car-park outlines, parcels, buildings | Driveways (kerb cut, apron, wings, the drive itself), alleys, parking lots, loading areas, fuel forecourts | See [Driveways and parking lots](#driveways-and-parking-lots) |
| **S6 Roadside** | S2 zones, S4 blocks, tree survey, OSM barriers, land cover, lidar where we have it | What stands or grows in each zone and on the boundary: grass, groundcover, tree wells and trees, hedges, fences, walls, guardrails, bollards, planters, yards | See [Roadside: grass, trees, fences and walls](#roadside-grass-trees-fences-and-walls) |
| **S7 Control and markings** | S2, S3, rulebook | Lane lines, centre and edge lines, stop and give-way lines, crossings, arrows from turn-lane tags, words, bike symbols, stall lines, hatching; signal and sign positions by junction control and region style | Left-hand mirroring. Crossing style by region and by tag. Markings through tapers and gores. |
| **S8 Heights** | Everything above, plus the terrain | A height for every point: the long profile of each corridor under its grade limit, crown and cross-fall, a junction surface that meets every approach's crown, kerb and walk heights, ramps, then earthwork (cut, fill, banks, retaining walls where a drop exceeds the limit) | San Francisco's steep hills. Junctions on a slope. Two-level street meets. Bridges and tunnels hand their pieces to eo-structures, whose solver sets their profiles. |
| **S9 Mesh and semantics** | S8 | Per-tile meshes and collision, with these per-vertex attributes: surface kind, material id, lane id, distance across the street, distance along it (a real UV), wheel-path coordinate, age and wear seed. LODs, and welded tile seams. A lane graph with turn paths through junctions, for traffic. | Tile edges, LOD changes, building floors |
| **S10 Wear and look** | S9 attributes, rulebook, kit | Wear inputs placed from the layout: wheel-path polish and ruts, braking zones before stop lines, turning scuffs in junctions, oil drips at stalls and lane centres, paint worn by traffic, utility patches along lanes, manholes and grates (survey first, rule second), kerb grime, staining by doors, bins and trees, root heave beside trees, puddles in low spots | It has to look aged and matte at shot distance, not new |

The old builder stays as the fallback behind a switch until the new one has
passed matched captures and the visual gate (decision D6).

### Driveways and parking lots

- **Driveways.** These come from OSM `service=driveway`, garage doors on
  building facades where we have them, parcel frontages, and a rule fallback.
  Each driveway gets a kerb cut with wings (or a flush crossing where the
  rulebook says so), an apron across the furnishing zone, and a drive surface
  up to the garage or lot. It follows the grade with the
  limits in the rulebook. A drive's two wheel tracks can sit at different
  heights on a hill; today's `driveway.rs` already handles that, and the rule
  carries over.
- **Alleys and service drives.** These are kept as their own street class with
  their own cross-section: often no kerb, a central gutter, and a concrete or
  patched-asphalt surface.
- **Parking lots.** These come from `amenity=parking` areas (surface lots),
  `service=parking_aisle` ways and the rulebook.
  - Aisles are laid along the mapped aisles. Where no aisles are mapped, they
    are fitted to the lot's long axis.
  - Stalls are generated along each aisle at the rulebook's size and angle:
    90°, 60° or 45°, or parallel.
  - Each lot gets end islands with kerbs and trees, accessible stalls with
    their markings and access aisle, wheel stops, light poles, entry and exit
    driveways, a drainage slope to inlets, and stall-line paint with wear.
- **Garages.** Multi-storey and underground garages (`parking=multi-storey`,
  `parking=underground`) get their entry ramps and portals from S5 and
  eo-structures. Their interiors are out of scope for the first pass.
- **Parked cars** in lots and at the kerb are placed by eo-props from the
  stalls and parking lanes.

### Roadside: grass, trees, fences and walls

The roadside is first-class work, not polish. It is in the cross-section from
the first milestone, as strips, before any object is placed.

- **Planting strip and grass verge.** These are strips in S2 between the kerb
  and the throughway, or between the carriageway and a ditch on rural roads.
  Each has its width, its ground (grass, groundcover, mulch, dirt, gravel) and
  its edge (flush, raised kerb, low wall). Data comes from OSM (`sidewalk`,
  `landuse=grass` beside streets), surveys, land-cover and aerial
  classification (we already have WorldCover and the NAIP-measured green
  share), and the rulebook's habits for the street type.
- **Trees.** The survey comes first: today's DataSF list of about 139,000 rows.
  Then OSM `natural=tree` and `natural=tree_row`. The rulebook's species,
  spacing and well size are the fallback in planting and furnishing zones,
  kept clear of driveways, crossings, hydrants, signals, sight lines and
  overhead wires. A tree well, basin or grate is placed with every tree.
- **Hedges, fences, walls and gates.** These sit on the property line or the
  back of the walk: OSM `barrier=hedge|fence|wall|retaining_wall|gate`,
  parcels, lidar edges, and a rule fallback by land use and region. Fences are
  typed: chain-link, picket, iron railing, wood board, wire. Walls are typed
  too: garden, retaining and sound walls. All of them follow the grade, with
  stepped tops where a real wall would step.
- **Front yards.** Lawn, planting, gravel, paving, steps and stoops between the
  sidewalk and the building. They reuse the unwired classes in
  `eo-props/src/yards.rs`, and the yard and front-walk floor rules that
  `grounding.rs` notes as not ported.
- **Guardrails and barriers.** W-beam, thrie-beam, concrete safety-shape and
  cable barriers, end terminals and crash cushions, placed where the
  rulebook's warrants say: drop-offs, bridge approaches, medians, gores.
  Bollards and planters go where the layout marks a pedestrian area against
  traffic.
- **Ditches and banks.** Rural and hillside edges get a ditch or a bank in
  S8, not a kerb.

### Highways, bridges, tunnels and underground roads

- **Highways.** They are corridors like any other street, with a highway
  cross-section: lanes, shoulders, rumble strips, the median and its barrier,
  sound walls, and the right-of-way fence. Interchanges are junction clusters
  of a special kind. Their legs meet as merges and diverges with gores and
  tapers, not as an at-grade crossing. Diamond, cloverleaf, partial and
  stacked forms all reduce to merges, diverges, ramps and grade separations.
- **Bridges and elevated roads.** S1 marks each piece's level. S2 gives the
  deck its real cross-section (lanes, sidewalks, parapets, barriers).
  eo-structures keeps solving the profile and the supports, and builds the
  deck, bents and abutments from that section. Its joins hand back the
  heights where a ramp lands on a street.
- **Tunnels, underpasses and cut-and-cover.** eo-structures' bores and
  portals take their roadway and walkways from the same cross-section. The
  approach cut and its retaining walls come from S8.
- **Landmark bridges** keep their authored decks; their approaches join the
  layout like any other piece.
- **Turning structures on by default** is a milestone of its own (M6), with
  its own bench cost and captures.

### The region rulebook and kit

- **Rulebooks** live in `scripts/world-data/sources/<jurisdiction>-street-standards.json`,
  using the same entry shape as the California table: id, value, unit, metric
  value, condition, document, edition, section, URL, applicability and
  confidence.
  - A jurisdiction names its parent (city, state, country), so a city file
    holds only its local rules.
  - New keys cover what the California table does not: driving side, marking
    colours, crossing style, signal placement style, sign system, stall
    geometry, tree spacing and well size, fence and wall habits.
  - The California table stays as it is and stays binding for San Francisco.
- **Kits** are material and prop families for each region:
  - asphalt types and ages, slab concrete, pavers, setts and cobbles, brick,
    gravel, dirt;
  - kerb stone, markings (thermoplastic, paint);
  - signal heads and poles, signs, fences, walls, hedges, guardrails, lights.
  - Each follows the asset rules in [AGENTS.md](../AGENTS.md#data-and-provenance)
    and [ASSET-STORE.md](ASSET-STORE.md): a recorded source, licence and digest
    for each, props hand-built in Blender, and CC0 scans for surfaces.

### Overrides

Map data is sometimes wrong. An override file for each region holds hand
fixes keyed by stable ids, for example a lane count, a junction's corner
radius, a missing driveway, or a fence that isn't there. Every override
records who made it, why, and the evidence. An override that departs from the
rulebook is a deviation, under today's rule.

## Our own tools

| Tool | What it does | When |
| --- | --- | --- |
| **The hard set** | A fixed list of real places, each with its source extract, its expected properties and its `--shots` poses. San Francisco first: Market Street's diagonal junctions, Castro / Market / 17th, Van Ness and Lombard (US-101), the Embarcadero, the Octavia Boulevard freeway ramps, Portola's curves, the crooked block of Lombard, a stair street, Bernal Heights hill streets, the Great Highway. Then the world: a star junction with ten or more legs, a left-hand roundabout, narrow streets with no sidewalk, a city with separated bike paths, a suburban cul-de-sac with driveways and a big parking lot, a motorway interchange, a mountain switchback, an old-town square with cobbles. | M0 |
| **Layout report** | An offline top-down picture for each place or tile (SVG in an HTML page, written to a folder given on the command line, never tracked): strips coloured by kind, ids, tapers, junction areas, flagged problems, and a diff between two builds. It runs from an example binary, so it is not a second window. | M0 |
| **Automatic checks** | Geometry rules run on the hard set and on whole cities: no two surfaces of different kinds overlapping; no holes in paved areas; kerb lines closed and unbroken; every lane connected through each junction; sidewalks continuous around blocks; a ramp at both ends of every marked crossing; no step in height except at a kerb; grades under the limit; nothing floating above or buried under the terrain; welded tile seams; the same ids and the same output on every rebuild. They extend today's `eo-street/tests/surface.rs` and never loosen it. | From M1 |
| **Captures** | Matched `--shots` runs for every hard-set place, before and after, in play's look (cinematic, cine camera, screen GI, High), plus traversal clips for the gate | From M3 |
| **Coverage report** | For each region: what came from a survey, what from a map tag, what from a rulebook inference, and what from an override | From M1 |
| **Inspect mode** | Later: an in-game mode in eo-app's existing loop to fly to any junction, show the layout over the scene, and write an override. It is not a second window or main loop. | After M3 |
| **Cook** | An offline cook of layout and mesh tiles for streaming, timed per tile, with the size of each tile recorded | M9 |

## Milestones

| Milestone | Deliverable | Ready to move on when |
| --- | --- | --- |
| **M0 Baseline** | The hard set (about 30 San Francisco places and 12 world places), the layout report, and today's builder run on the San Francisco places. Each place gets captures and a failure list. | Every place has a repeatable report, poses and a written list of what is wrong |
| **M1 Network and cross-sections (2D)** | `eo-streetscape` with S0–S2 for San Francisco: clusters, divided roads, roundabouts, profiles with tapers, verges and planting strips as strips. The report and the checks, behind the `--streetscape` switch. | The checks hold on the hard set; the citywide report shows no unexplained piece |
| **M2 Junctions, blocks and sidewalks (2D)** | S3 and S4: junction areas for any number of legs, blocks, unbroken sidewalk zones, dead-end caps and turning circles | Zero overlaps and holes on the hard set and citywide; a person reviews the report for every hard-set junction |
| **M3 Heights and mesh in the game** | S8 and S9 for ground-level streets, drawn by eo-street from the layout. Collision, LODs, welded seams. An adapter keeps props, traffic and the player working while they move over. | Matched captures against the old builder at every hard-set place; bench cost per path; today's surface tests pass |
| **M4 Driveways, alleys, lots and plazas** | S5 | Every mapped lot and driveway in the hard set is built and checked; captures of three lots |
| **M5 Roadside** | S6, with eo-props reading the zones: grass, trees and wells, hedges, fences, walls, guardrails, yards | Captures at residential, commercial and park-edge places; checks keep objects out of lanes, ramps and sight lines |
| **M6 Highways, bridges and tunnels** | eo-structures built from the layout's cross-section; interchanges as merge and diverge clusters; `--structures` and `--tunnels` made the default if their cost fits | The structure tests pass; captures at the US-101 and I-80 approaches and a tunnel; bench cost per path, including the `hyper` tier with structures |
| **M7 Markings, control and look** | S7 and S10: region-driven markings, signals and signs; wear placed from the layout; lane ids and UVs in the shader | Matched captures in play's look, then a new blind review under the gate (its verdict stays REJECT until then) |
| **M8 Second and third region** | One left-hand city and one European right-hand city: rulebook, kit and hard-set places. This proves the tool is worldwide. | Their hard-set places pass the checks and are reviewed in captures; no region-specific branch in code |
| **M9 The planet** | The cook and streaming, with size and time budgets per tile, and a coverage report for each region cooked | Cook time and tile size within budget; streaming adds no frame over budget |

M1 and M2 are 2D only on purpose. Most of what makes streets "feel weird"
today (floating streets, dug-down ground, sidewalks on walls) starts as a
wrong 2D layout or a height rule fighting it. A correct 2D layout first makes
the height and look work tractable.

## Budgets

- **Frame.** The whole frame stays under 10 ms at 1080p. Streets draw inside
  the static-draw line (2.0 ms) and roadside objects inside cars and props
  (0.6 ms) ([native/README.md](../native/README.md#performance-target)).
  - Every milestone that reaches the screen states its cost per bench path,
    from matched runs with its switch on and off on the same build, on an
    idle machine.
  - It also A/Bs the cost of the default path with the switch off, since
    pixel-equal defaults have hidden regressions before.
- **Cook.** The layout is built offline or on worker threads, never on the
  render thread. M1 sets a time-per-tile target from measurement, and every
  later milestone reports against it.
- **Kill switches.** `--streetscape` while it is opt-in, and
  `--no-streetscape` once it is the default. Each new visible feature inside
  it gets its own `--no-<feature>` switch that restores the previous output.

## Data and licences

- **Street network.** It comes from OpenStreetMap regional extracts, the only
  worldwide road map. The OSM-derived files stay a separate ODbL artifact, as
  [CITY_DATA_PIPELINE.md](CITY_DATA_PIPELINE.md) already says. Shipping
  cooked tiles in a commercial game needs a licence review (decision D3).
- **Local surveys** (sidewalk widths, trees, signals, signs, kerb ramps,
  inlets) are registered with their source, licence and digest, and win over
  map tags and rules where they exist.
- **Aerial imagery and land cover** (NAIP in the US, WorldCover worldwide) are
  used only to classify ground and edges, never shown. Google Maps, Google
  Earth and Street View are never used.
- **Reference.** osm2streets is an open-source Rust project that solves some of
  the same OSM problems. We read its approach and its test cases where its
  licence allows; we build our own tool and do not depend on it.

## Decisions needed

| # | Decision | Why it needs the owner |
| --- | --- | --- |
| D1 | Change the street-standards rule from "California manuals only" to "the jurisdiction's own standards", with California binding for San Francisco | It is a binding project rule ([AGENTS.md](../AGENTS.md#california-street-standards)) |
| D2 | Add the new workspace crate `eo-streetscape`. Decide whether to use a third-party polygon library (boolean operations and offsets) or write our own. | A new dependency crate needs approval ([AGENTS.md](../AGENTS.md#stop-and-ask)). Robust polygon operations are hard to get right, so a well-tested library is the recommendation. |
| D3 | How OSM's ODbL applies to cooked street tiles shipped in a commercial game | Licence and business risk; it needs someone who knows the licence |
| D4 | An offline layout report first, and an inspect mode inside the existing loop later | It rules out a second window or main loop, which needs approval |
| D5 | Which left-hand city and which European city are regions two and three | It sets the data, rulebook and kit work for M8 |
| D6 | When the old builder retires | It stays as the fallback until the new one passes matched captures and the gate |

## Risks

- **Map quality.** Widths, lanes and sidewalks are missing on many streets
  worldwide, so inference carries much of the work. The coverage report makes
  that visible. Overrides fix the places that matter.
- **Hills.** Junction surfaces on San Francisco's grades are the hardest
  height problem. The hard set includes the steepest junctions from M0.
- **Consumers.** Props, traffic, the player, structures and the renderer all
  read today's `Plan`. M3's adapter keeps them working; each one then moves to
  the layout in its own reviewed change.
- **Robust geometry.** Near-parallel lines, tiny edges and floating-point
  error break polygon code. The checks run citywide from M1, not only on the
  hard set.
- **Scale.** The planet has billions of map nodes. The layout is built per
  tile, from that tile and its neighbours' borders only, so cooking can run in
  parallel and resume.

## The next deliverable

**M0: the hard set, the layout report, and today's builder measured on it.**
That shows, with pictures and numbers, every place the current streets go
wrong. It becomes the test the new tool must pass.
