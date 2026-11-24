# Earth Online: the native build

`native/` is a Cargo workspace that builds Earth Online as a desktop
program: a GPU-driven renderer on [wgpu](https://wgpu.rs) that draws the
whole of San Francisco from open data, with streets, buildings, terrain,
water, sky and weather, simulated traffic and people, a walking and driving
player, sound, an interface and an online server.

The project is early and in active development. Many systems are first
versions, some content is missing, and the program changes quickly. Nothing
in this document is a statement about how a frame looks: the project's
recorded visual-quality verdict is REJECT (see
[Docs/VISUAL_QUALITY_GATE.md](../Docs/VISUAL_QUALITY_GATE.md)), and only a
new blind review can change it.

- [Crates](#crates)
- [Requirements](#requirements)
- [Build and run](#build-and-run)
- [Command-line switches](#command-line-switches)
- [Environment variables](#environment-variables)
- [Where the data comes from](#where-the-data-comes-from)
- [Optional off-repository assets](#optional-off-repository-assets)
- [What is drawn](#what-is-drawn)
- [Tests](#tests)
- [Known issues](#known-issues)
- [Performance target](#performance-target)
- [Licences](#licences)

## Crates

| Crate | What it holds |
| --- | --- |
| `eo-data` | Loaders for the city files in `public/data/sf/` (streets, footprints, elevation, land cover, roof colours) and the California street standards registry. No GPU. |
| `eo-assets` | Hash-verified model loading (GLB), the binary model cache, the building kit's material registry and map bake, placement of the hand-built buildings, the scanned surface sets. |
| `eo-kit` | The procedural building planner: classification, facade layout, storeys and bays, shop fronts, roofs, and the mesh emitters for each level of detail. No GPU. |
| `eo-street` | The street surface: cross sections, junctions, corners, kerb ramps, crossings, lane paint, tile meshes, and the building floor and foundation rules. No GPU. |
| `eo-look` | The lighting model as pure functions: sun position, analytic sky, fog, the light rig, exposure, tone curve and shadow cascade fitting. No GPU. |
| `eo-sky` | Weather schedule, clouds, marine layer, rain and wetness, moon and stars, street-lamp photometry. No GPU. |
| `eo-water` | The shoreline field, the FFT ocean spectrum, wind, water optics, surf and the open-sea grid. No GPU. |
| `eo-landmarks` | Bridges, the quay wall, piers, seawalls, plazas, landmark towers and the open-space ground as plain meshes. No GPU. |
| `eo-farshore` | The land beyond the city out to the horizon, cooked from elevation tiles. No GPU. |
| `eo-structures` | Grade-separated roads (freeways, ramps, road bridges, tunnels): their vertical profiles, deck, support and portal meshes and collision triangles, behind `--structures`; the joins a host builds the street again with (`build_joined`: junction legs, capped, raised and lowered stubs), the tunnels' bores, lights and terrain cuts (`--tunnels`). It reads the street plan and never edits the street surface itself (eo-street draws the joins). No GPU. |
| `eo-props` | Street furniture, traffic and pedestrian signals, signs and sign atlases, street trees, parked cars, placed per tile by rule. No GPU. |
| `eo-sim` | The headless world simulation: traffic, signals, pedestrians, population and level of detail, tile-keyed and multi-threaded. No GPU. |
| `eo-agents` | Vehicle bodies and people cooked into one GPU-ready library (wheel rigs, lamps, skeletal motion, atlases), and the feed from the simulation to the renderer. No GPU. |
| `eo-player` | The on-foot controller, walk and drive cameras, the player's car, input mapping and the streamed collision world, on Rapier. No GPU. |
| `eo-render` | Scene build and the wgpu renderer: GPU culling and LOD, occlusion, streaming and uploads, and every render pass. |
| `eo-ui` | Title screen, menus, settings, map and HUD, drawn with wgpu over the 3D frame. |
| `eo-audio` | Sound bank, real-time 3D mixer, output devices and the city's sound director. |
| `eo-net` | Online foundation: UDP transport, handshake, snapshots, client prediction and interpolation. No GPU. |
| `eo-server` | The headless authoritative server binary, scripted bots and network benches. No GPU. |
| `eo-app` | The game binary: window, fly-through, `--play`, `--ui`, `--connect`, and the offscreen bench. |

Other folders:

- `assets/` holds small tracked inputs: manifests of the optional
  off-repository assets, the sound credits, the roof-colour and ground
  tables, the fallback font, and the placement modules of the hand-built
  buildings.
- `tools/` holds Node scripts that download optional assets into a folder
  outside the repository and write their provenance manifests.
- `*/data/` folders hold reference answers and tables that the crates and
  their tests read. Some were exported from the earlier browser
  implementation of the game, whose exporters are no longer in this
  repository; treat them as fixed test fixtures.
- `bench-out/`, `cache/` and `target/` are generated per machine and
  ignored by Git.

## Requirements

- A stable Rust toolchain that supports edition 2024 (Rust 1.85 or newer),
  installed with [rustup](https://rustup.rs).
- A GPU and driver that wgpu supports: Vulkan or DirectX 12 on Windows,
  Vulkan on Linux, Metal on macOS. The project is developed and tested on
  Windows. macOS (Metal) builds, runs the bench and runs the tests, but is
  not a reference platform; Linux builds are untested.
- A C/C++ compiler for the crates that build native code (`meshopt` and
  others). On Windows that is the MSVC build tools; on Linux, `gcc` or
  `clang`.
- On Linux, the ALSA development package (for example `libasound2-dev`) for
  the audio output.
- Disk space: the first run cooks models, building tiles and textures into
  `native/cache/`, which grows to several gigabytes.
- Node.js only if you use the scripts in `tools/` or the asset-store scripts
  at the repository root.

## Build and run

Run every command from `native/`.

```sh
cargo build --release
cargo run --release -p eo-app                  # windowed fly-through
cargo run --release -p eo-app -- --play        # walk and drive
cargo run --release -p eo-app -- --ui          # title screen, HUD and pause menu over the fly-through
```

The first start is slow: the city is read, models and building tiles are
cooked and cached, and shaders are built. Later starts read the cache.

The release profile keeps line-level debug information (`debug = 1`), and
dependencies are optimised in the dev profile too, so a debug build is
usable but a release build is the one to measure.

### Controls

Fly-through (default):

| Input | Action |
| --- | --- |
| Right mouse button | Toggle mouse look |
| W A S D | Move |
| Q / E | Down / up |
| Shift / Ctrl | Faster / slower |
| F | Print the camera position |
| Esc | Release the mouse; press again to quit |

`--play`:

| Input | On foot | Driving |
| --- | --- | --- |
| Click the window | Capture the mouse | Capture the mouse |
| W A S D | Move (camera relative) | Throttle, brake and reverse, steer |
| Shift | Run; held straight ahead, sprint | |
| C or Ctrl | Crouch | Look behind (C) |
| Space | Jump | Handbrake |
| F | Enter a car within reach | Get out |
| V | Place a new car ahead (a test helper) | |
| Esc | Release the mouse; press again to quit | |

The title bar shows the frame rate, and in `--play` the mode, speed and the
player step time.

### Online

```sh
./target/release/eo-server --bind 127.0.0.1:47800                      # the server
./target/release/eo-app --connect 127.0.0.1:47800 --name ada           # a client (--play controls)
./target/release/eo-app --connect 127.0.0.1:47800 --name bo --spawn 1549,-366,3.14
./target/release/eo-app --connect local                                # a server inside the client process
./target/release/eo-server bots --connect 127.0.0.1:47800 --count 16   # scripted bots
```

`--net-sim RTT,JITTER,LOSS` (round trip in ms, jitter in ms, loss in
percent) impairs a client's link in both directions. `eo-server bench`,
`eo-server bandwidth` and `eo-server predict` measure the server, the
bandwidth on a busy street and client prediction. Clients and bots must
use the server's `--ground-scale` and its `--structures` / `--tunnels`;
the handshake refuses one whose world differs and says why.

## Command-line switches

`eo-app --help` prints the main bench and streaming options. The most useful
switches are below; each mode (`--play`, `--ui`, `--connect`) rejects a
switch it does not take.

### Modes

| Switch | What |
| --- | --- |
| (none) | Windowed fly-through |
| `--start-underwater` | Start the fly camera just beneath the Bay, looking up toward the skyline |
| `--play` | The player instead of the free camera |
| `--ui` | The interface shell: title, loading, HUD, pause menu |
| `--connect ADDR\|local` | Online client (with `--name`, `--spawn x,z,heading`) |
| `--bench` | Offscreen timing run over scripted camera paths; prints and writes JSON to `bench-out/` |
| `--bench --play` | Scripted walk and drive paths with the player |
| `--capture-ui SCREEN` | Writes a PNG of one screen: `title`, `loading`, `settings`, `pause`, `map`, `hud` or `all`; also (not in `all`) `settings-hyper` (the settings panel's presets with Hyper real chosen and the "Lighting and detail" row with its note), `settings-world` (its World block) and `pause-settings` (the pause sheet's graphics rows) |
| `--ui-bench` | Interface cost over the street walk and each full-screen menu |
| `--audio-render all --out DIR` | Offline sound mixes; no GPU, no audio device |

### World and view

| Switch | What |
| --- | --- |
| `--time-of-day H` | Hour of the day, 0 to 24 (default 12) |
| `--weather KIND` | `clear`, `overcast`, `fog`, `drizzle`, `rain`, `storm`, `damp` (a street an hour after rain: the film and puddles, drying patches, no rain, a clearing deck), or `schedule` for the changing weather |
| `--weather-seed N`, `--world-time S` | Seed and clock of the weather schedule |
| `--moon-day D` | Day of the lunar cycle |
| `--sky-quality low\|medium\|high` | Cloud and atmosphere quality |
| `--cloud-depth-haze` | A low-deck cloud seen over the ground (below the horizon from inside or above the deck) takes the aerial perspective of its own depth and hides the haze behind it, instead of the haze of the whole path to the ground in front of it (a blue veil cut at the horizon). Not under a weather veil (fog, drizzle, rain, storm), where those frames stay as they were. Off by default until reviewed under the visual gate; `--no-cloud-depth-haze` restores the frame |
| `--agent-density N` | Multiply the traffic and pedestrian population (1 to 64) |
| `--width W --height H` | Frame size (default 1920 x 1080) |
| `--msaa 1\|4` | Multisampling |
| `--no-vsync` | Present without vertical sync |
| `--far M` | Camera far plane in metres (default 4200) |
| `--shadow-map N` | Sun shadow map size (default 3072) |
| `--quality low\|medium\|high\|hyper` | Quality tier (default `medium`, the street look: `--grime 1`, `--stand-ins`, `--surveyed-furniture`, `--tree-basins`); see [Quality tiers](#quality-tiers). A switch the tier sets, given anywhere on the line, wins over the tier |
| `--light-shadows [N]`, `--light-shadow-moving`, `--headlight-shadows [N]` | Shadows of the street lamps and headlights (eo-render `light_shadow.rs`, `light_shadow*.wgsl`; all off by default; a level left out is the `high` tier's). `--light-shadows N` (lamps, rounded up to a multiple of 16, at most 48): the N street lamps nearest the eye each keep a 512 px depth map, straight down from the luminaire over 72 deg from the nadir, drawn once (at most 2 new maps a frame, nearest first) from the city's objects at the level drawn, the props and the parked cars, and kept while the lamp stays selected; `--light-shadow-moving` draws the 4 nearest again every frame with every car and person; `--headlight-shadows N` (at most 8) gives the N nearest vehicles with their low beams lit a map each every frame, over both beams, from everything but the vehicle itself. One depth atlas (16 + 2: 2560 x 2048, 12.1 MB; 48 + 4 + 8: 4096 x 4096, 35.1 MB) at the look's group 1 binding 17, its table at 18; the lights' order and every light cluster's list become stable (nearest first, the same frame after frame and run after run); each lit surface's lamp light is multiplied by its occlusion and its map (four bilinear compares); the lit fog (`--lit-fog`) reads the maps too; `--light-shadows` also draws the street lamps in SF's 3000 K LED colour as a night-adapted eye sees it (eo-sky `lights::led_street_lamp_color`; 0xffc46a otherwise). Selected, culled and drawn on the GPU (no read-back), timed in the lighting set's `light_shadows` and `headlight_shadows` slots; nothing runs by day. `--no-light-shadows`, `--no-light-shadow-moving`, `--no-headlight-shadows` restore the frame |
| `--lit-fog [1\|2]` | Fog lit by the street lamps and headlamps (eo-render `lit_fog.rs`, `lit_fog.wgsl`; eo-sky `lit_fog`): a froxel volume on the light clusters' frustum (1: 160 x 90 columns x 64 log slices over the clusters' 0.5-300 m, 22.1 MB; 2: 240 x 135 x 96, 74.6 MB), filled between the sky's volume and its resolve with the volume's own fog at each froxel (the marine layer, the weather's veil, a continuous low deck, the haze, rain) and the light each clustered lamp sends toward the eye through the droplets' forward lobe (Henyey-Greenstein g 0.9, widening with the optical depth of the light's path; street luminaires with their tail out to the horizontal, so a lamp seen down the street is ringed by its glow), temporally blended at a low history weight with the history held near the new value (a moving beam leaves no trail), integrated front to back and added by the resolve at each pixel's depth; a light with a shadow map (`--light-shadows`, `--headlight-shadows`) lights only the fog it reaches, and the luminaires' own air light is not drawn where the volume draws it. Nothing by day or with no lamp lit (the passes skip, the slot is an empty span). Timed in the lighting set's `lit_fog` slot; the `high` tier sets 1 and `hyper` 2; `--no-lit-fog` restores the frame |
| `--cascade-blend` | The sun cascades hand over to the next over the last fifth of a cascade's slice (a smoothstep to the split) instead of switching at the split. The city, agents, landmark wires, CC0 decals and props take it; the far layer (past the cascades) has nothing to blend; the water and the shafts keep the switch (no visible change there) |
| `--far-shadows [1\|2]` | Sun shadows past the cascades' 160 m from a cached depth map round the camera: 1 one ring (2 km across round a street-level eye, growing with the eye's height to about 6.7 times it, up to the far plane: the far plane's 4.2 km from 300 m up), 2 a second ring out to the far plane (4096 px, Depth16, two layers a ring: 64 MB a ring). A ring is redrawn a quarter (one scissored tile) a frame, only after the key light has turned 0.05 degrees or the camera has moved an eighth of the ring from its fit; casters are each building's footprint prism or, while the far layer holds it, its cell's mesh, the terrain as drawn, the structures' coarse concrete and the landmarks' far items. Sampled in the cascades' fade (120-160 m) and past it by the city, agents, landmark wires, CC0, the far cells, far trees and lights, and the sea; each ring fades out with the ground distance from the eye. Props keep the cascades alone (Apple's shader compiler); the shafts end at 160 m as before |
| `--shadow-slope-bias` | The sun cascades' casters drawn with a slope-scaled depth bias (1.5 depth slopes a texel, at most 0.002 of the light range): surfaces the sun grazes (a viaduct's fascia, a wall nearly edge-on) no longer shadow themselves in stripes. The `high` and `hyper` tiers turn it on; `--no-shadow-slope-bias` |
| `--grime [1\|2]` | Surface weathering that reads at normal viewing distance (eo-render `weathering.rs`, `weathering.wgsl`; off by default, so the frame is as before): walks at an aged, soiled mid grey (LBNL weathered concrete 0.20-0.30, drawn at 0.225) with each scored slab its own pour and a few lighter replacement slabs, darker joints, a dense trodden-gum speckle that keeps its mean at any distance, blotchy stains and dirt along the kerb and (decals) the building line; kerbs grimy up the face with crumbled arrises and, at each corner on the vehicle approach side, the daylighting's red paint (`no-parking-crosswalk-daylighting`, 20 ft) faded and chipped; carriageways with a quilt of trench patches and repaved pieces in different greys with sealed saw cuts, cracks where the old concrete base's slabs meet under the asphalt (transverse a slab apart and longitudinal along the lanes, on a cell net stretched along the road; some sealed with a 2-3 in bead of binder and the squeegee's film, black fresh and dusty grey aged, the rest open hairlines with ravelled edges), more oil down the lane centres, darker gutter pans and worn thermoplastic markings (soiled, blackened in long tyre streaks along the line, worn through to the aggregate in fine specks, a few small chips; the scanned road-line mask's holes and marks kept at a third, since the scan repeats along the line); facades washed down below every sill line (some rust-stained), sooted under the roof edge and on old buildings' upper storeys, splashed at the foot, sun-faded on the walls the day's sun reaches most, each building its own age; and patterns that still read from a block away (50-200 m, where the sill streaks average out): soot and rust plumes down from the roof edge, damp patches with a tide line at the foot, and on painted fronts paint blocks repainted in a paint that does not quite match (most on the ground floor) or sun-faded; Blender-built buildings the plumes and damp, far buildings (the far layer) each their own tone; cars dusty on the lower body and grimy round the wheel arches, older ones' paint dull, each car its own; the kit buildings' windows (stage W5): a dirt film on the glass (haze that blurs and dulls the reflection, grime against the frame and in the bottom rail's tide band, run-off from the head, dried rain spots, more on old buildings and on their unwashed upper storeys, shop glass washed often and curtain walls on a contract, hand grease on shop doors; the lit room behind held back by it), old painted sashes and frames chalked and peeling to grey wood or an older coat, dirt on the trim's ledges and a dark drip edge under them. Albedo and roughness only (and the glass's metalness, clearcoat and opacity). Level 2 adds a second gum scatter and deepens the plumes and repaints. With `--structures` the freeway concrete weathers by each structure's era (eo-structures `Era`: the 1955-1971 viaducts and ramps heavily, the rebuilt Central Viaduct and Bay Bridge West Approach less, the Presidio Parkway hardly): soiling in water-stained blotches, run-off streaks down the fascia from the barrier top and on down the girder, down cap faces, walls and columns (some rust-coloured or white with efflorescence under joints and bearings), salts leaching from the soffit's joints, splash-back at column and wall feet, grime along the barrier toe; with `--tunnels` the bores' tile is sooted over the vault and in the grout, road-sprayed over the bottom metre, unevenly glazed with a few stained and replaced tiles (eo-render `structures.wgsl` `sx_weather`, `sx_tile_weather`). The kerb paint and building-line dirt are street decals and need the CC0 sets (`EO_CC0_ROOT`); the rest is procedural and needs nothing. The `medium` (default, since the owner's approval of 2026-11-20) and `high` tiers set 1 and `hyper` 2; `--no-grime` |
| `--street-life [1\|2]` | The small things that say people use a street (eo-render `street_life.rs`, `street_life.wgsl`; off by default, so the frame is as before), as street decals placed per tile by deterministic rules and drawn in the material of the surface they lie on: paper scraps, receipts, cups, wrappers, caps, napkins, cigarette butts and torn bits on the walks and against the kerb in the gutters, dead leaves under the surveyed street trees (DataSF street tree list), grit and sand drifts in the kerb corner and along the building line, cardboard and bags on neglected stretches; clusters metres long with a dark wet halo that still read at 30-80 m (drifts against the kerb and along the building line, a pile at each surveyed storm-drain inlet, a bin's spill at the stops and corners); heaps at the wall's foot (bags, a leaning sheet of cardboard, a stained mattress); on the ground floors at the building line (every kit material but glass) bold graffiti: fat-cap and marker tags (2.5-7 cm strokes, some two-tone) in a low and a high band, throw-ups 0.5-1.25 m tall with drop shadows, drips and overspray, on the hottest walls a field of old faded tags and multi-colour pieces with a 3D block, buffed-over patches and wheat-pasted posters; round the freeway columns' bases (`--structures`) the same, covered to 2-3 m. Densities by land use (the building-metadata packs' `use`, else the OSM tags: industrial and vacant fronts most, shops some, homes little, civic buildings none), foot traffic (the surveyed stops and shopfronts), corners, the street's class and the decks overhead; each block side its own care (some swept and buffed, some neglected). No letters, names or marks anywhere: tags are meandering strokes with curls and hooks on no alphabet, throw-ups and pieces merged blobs and random strokes, posters colour fields, bars and dots. On the kit buildings' glass (stage W5): stickers on shop doors and in a corner of other shop glass (opening-hours plates, card marks, an alarm firm's square: plain shapes and colour fields), tags scratched or acid-etched into ground-floor panes where the wall beside them carries tags (jagged tangles, no letters), paper, kraft paper, newspaper sheets, board or soaped-over glass inside vacant shops (dark at night), and potted plants on flats' sills seen in parallax behind the glass (more on older buildings; dark against the lit room at night). Level 2 is a step denser (more litter, tags, posters and heaps) and adds stickers, utility locate marks and pigeon droppings under the decks. Needs the CC0 decal grid (`EO_CC0_ROOT`). The `high` tier sets 1 and `hyper` 2; `--no-street-life` |
| `--stand-ins` | A street-prop kind whose GLB is absent from the checkout draws a procedural model of the San Francisco design at the GLB's size (eo-render `props_lib2.rs`, stage W2a) instead of a grey box; with every GLB present it changes nothing. On in the `medium` (default, since 2026-11-20), `high` and `hyper` tiers; `--no-stand-ins` |
| `--surveyed-furniture` | The street-furniture walk fills the gaps between surveyed items with what a San Francisco footway has (eo-props `furniture::surveyed_first`): a slot its hashed walk gives to a kind rare on SF footways (bollards, planters, barricades, scaffolding, newsracks) stays empty, and one it gives to a street-commerce kind (benches, bins, post boxes, bike racks, café sets, sandwich boards) stays empty unless the frontage there trades; the walk steps on as if the item stood there, with the same draw. Surveyed, observed and forced items are as before. On a citywide sample of 265 tiles: bollards 4,486 to 0, planters 1,588 to 0, newsracks 726 to 0, bins 1,717 to 179, benches 1,305 to 127, post boxes 1,045 to 131; trees, meters and signs unchanged; hydrants 1,508 to 1,250 (an emptied slot takes a placed item's step); parked cars 3,784 to 3,981 (fewer kerbside items in the way). Render-only (the props are not in the collision world or the server). On in every tier; `--no-surveyed-furniture` gives the JS walk as ported |
| `--tree-basins` | A street tree's basin under each tree and palm the furniture walk places on a footway (eo-props `basins.rs`, eo-render `props_lib.rs` `Model::TreeBasin`): a panel flush with the walk whose bed is drawn in the props shader (props_draw.wgsl `pr_basin`): the scanned CC0 `park_dirt` soil where the scanned sets are on (`EO_CC0_ROOT`), else drawn grain, clods and grit, under damp patches and the watered ground round the trunk, the root flare (five tapering roots, read from the trunk's place in the instance's `variant`), leaf litter piled against the cut edge, a cigarette end now and then, moss by the edge and the cut face's shade line; a fringe past the edge keeps only the crumbs and leaves spilled onto the walk (cutout pipeline); centred on the trunk and squared to the kerb, moved out to clear the curb top, laid at its highest corner's ground plus the street's marking lift and tilted with the street's grade along the kerb, so it is never buried on SF's slopes. Its size is the minimum basin of SF Public Works Director's Order 187,246 for the footway's surveyed width (`street-tree-basin-sf`: 3 x 4 ft from 7 ft 6 in, 4 x 4 ft from 12 ft, 5 x 5 ft from 13 ft; across the walk x along the kerb; a narrower footway's older planting the smallest). Trees off a footway (medians, parks) get none. On a citywide sample of 177 tiles: 1,644 basins for 1,643 trees and palms (3 x 4 ft 380, 4 x 4 ft 165, 5 x 5 ft 1,099). Three levels, drawn to 110 m, no shadow. Render-only. On in every tier; `--no-tree-basins` gives the trees standing on bare paving as before |
| `--probe-grid [1\|2]` | Reflection probes on the streets round the camera in place of the single cube at the viewer: 1 four probes of 128 px, 2 nine of 256 px (in an array with two spare slots: 7.1 / 49.5 MB with the face target and table). Anchors at every junction's centre and along every drawn street's centreline at most 32 m apart, 1.7 m over the street datum, none inside a building; the set is the probes nearest a point 24 m ahead of the eye, each captured once when it enters (one face a frame in all, the single probe's deferral rules) and again only when the clock's quarter-hour or the sky moves. A new probe fades in, a dropped one out, a recapture cross-fades from its old cube (0.6 s): no reflection changes in one frame. Every lit surface (city, glass with the canyon lift, agents, props, landmark wires, CC0, the sea) reads the two probes nearest it by distance (and the analytic sky past 250-600 m from any); the far layer reads the analytic sky. With it or `--far-shadows`, a camera jump starts a capture under way again (a cube's six faces under one camera's shadow fit). `--no-probe-grid` restores the frame |
| `--ambient-volume [1\|2]` | Large-scale sky visibility: narrow streets, courtyards and the ground under a deck get less of the sky's light than an open square, and what blocks the sky lights them instead (the rig's ground bounce), so nothing goes black. Baked on worker threads from the building prisms, the terrain and, with `--structures`, the decks, per 128 m tile round the camera (1: 4 m cells at least 430 m out; 2: also 2 m cells at least 170 m out; 24 layers to 80 m above the ground) and once over the whole city at 8 m, cached under `<cache>/ambient/`. Props and the far layer read the city-wide 8 m plane only (Apple's shader compiler does not take more in their shaders). Off by default; `--no-ambient-volume` restores the frame |
| `--ground-bounce` | With `--ambient-volume` (on with it unless `--no-ground-bounce`; nothing without it): light bounced up from the sunlit ground onto deck soffits, cap beam undersides, girder webs and walls facing a sunlit street. The volume's levels also bake the open ground's share of each cell's lower hemisphere (ground, roofs and deck tops with nothing over them; not the ground under a deck), weighted by how likely the ground round the cell is to be in the sun (its horizon of buildings and terrain: a narrow alley's floor bounces little); undersides read it shifted toward the sun by their deck's shadow. The shader takes the rig's ground lobe's sun part (it stands for a lower hemisphere of sunlit street) out of the lower hemisphere and puts the sunlit ground's exitance from the sun (albedo 0.20) x the cloud transmittance back over that share, so undersides read brighter towards an open edge and darker deep under (a lid over a closed yard keeps only the lobe's sky part); nothing changes facing up, with the sun down or in a cloud's shadow. Twice the volume's GPU memory. The city, the structures, agents and landmark wires take it; props and the far layer do not (Apple's shader compiler); past the volume's levels there is none |
| `--smooth-time` | The drawn hour follows the simulation's clock (eo-sim's fixed-step time, the server's with `--connect`; it starts at `--time-of-day` and stays there without a simulation, `--no-agents`), through keyframes of the look a clock minute apart blended each frame; the rig's quarter-hour rows through a monotone cubic; the moon takes the key over from the sun in a blend; vehicle lamps and the offices' morning share ramp instead of stepping; the exposure converges instead of snapping on a turn or a clock jump (a teleport still snaps it); the histories survive small weather steps. Off by default |
| `--photo-finish [1\|2]` | A photographic finish (eo-render `photo.rs`, `look_photo.wgsl`; eo-look `tone`, eo-sky `photo`; off by default, so the post pass and the sky are as before): the post pass becomes a camera on the resolved frame. Its response keeps the old chain's neutral curve (middle grey lands where it did, so the exposure means the same) on each colour's brightest channel, keeps the colour's hue, pales bright colours toward white by as much as a per-channel curve would (a lamp's or a sunlit wall's core goes to white instead of clipping to a pure hue) and turns their hue a third of a per-channel curve's way in the shoulder; the grade adds no saturation or vibrance and rolls its brightness off to white instead of clipping. The sensor adds noise in the scene's units (shot noise and a read floor, by the ISO a phone camera at f/1.8 and 1/30 s picks for the scene's EV: base ISO by day, invisible; a few hundred on a lit street at night, a fine grain in the shadows), a new pattern each frame, after the TAA. The lens adds a residual fall-off (0.85 at the corners), lateral colour growing to about a pixel at the corners, and a bloom with a lower, softer threshold, more of a small source's light and less in the wide levels (a glow round lamps and glints, no veil). The sky's tables and the aerial perspective take San Francisco's measured air (IMPROVE's 50 km visual range row, an aerosol of albedo 0.9 under the 400 m marine inversion): a paler horizon and a warm-grey haze over the far city. Level 2 adds chroma noise and a smoother lateral colour. The `high` tier sets 1 and `hyper` 2; `--no-photo-finish` restores the frame |
| `--facade-clutter [1\|2]` | The things San Francisco fronts carry (eo-kit `clutter.rs`, stage W3; off by default, so the kit meshes and their cache key are as before): fire escapes on older multi-unit fronts (Tenderloin, Chinatown, Nob Hill, SoMa, the Mission most; one per stack of windows: slatted landings at each floor above the ground floor with picket railings, 60-degree stairs between them through a hatch, a raised drop ladder, knee brackets, a gooseneck over the parapet where it reaches; painted steel, mostly black, rusted by the building's wear; the port's own fire escapes are drawn this way in their place), awnings on shop and food tenancies without a shade (shed, dome, box and open retractable forms in the common fabric colours and stripes, sun-faded, a few torn), sign faces on the fascia boards and projecting blade signs (lit boxes or neon tubes on a dark board) carrying a generic word of the shop's category (stage S1: `SIGN_PHRASES`, CAFE, LIQUOR, DELI, LAUNDRY ...; never a name, a brand or a mark) in a bold, condensed, serif or script face found among the system's fonts at start (drawn in memory, never written to disk; with none installed, W3's colour fields and shapes), beside an emblem or in a border, painted, back-lit (letters over a dim face, or the face lit through under the tone curve's clip) or in neon tubes with their glow, faded by the building's wear; stacked letters on tall blades; lit by night where the shop is lit (`draw.wgsl` `fc_lit`: with `--shop-light` where the shop's own light is on, dimmed to its security level, else by the shops' lit share), roll-down security shutters (some down), bars over older ground-floor windows, downspouts at the party walls with their conductor heads and straps, meters, gas meter sets, conduit and the standpipe connection at the base, and on flat roofs chimneys with clay pots, vent stacks and flue caps, satellite dishes aimed at the south-south-east sky and TV aerials. Nothing passes through another piece or the plan's own parts, and nothing stands out over the walk past `sidewalk-width-sf-neighborhood-residential` less `sidewalk-edge-zone-sf` (the alley widths on a service way; awnings and signs also under the code caps and the 8 ft clearance). Near level all of it, mid level a simplified version, far level none. Level 2 adds window air conditioners (rare), dishes on fronts, pipe scaffolding and boarded shops. The kit tiles cook into their own cache folder with it. The `high` tier sets 1 and `hyper` 2; `--no-facade-clutter` restores the meshes |
| `--facade-glass` | The kit glazing drawn as glass is (eo-render draw.wgsl, rooms.wgsl behind `SF_FACADE_GLASS` and the frame switch `facade::RUN_FACADE_GLASS`; off by default, so the frame is as before): each pane a dielectric with two reflecting faces (F0 0.04 each, Schlick: the light's base layer and its clearcoat layer), opaque over its room, which shows through what the faces do not reflect, tinted by the glass, in place of a 28 % blend whose reflection took the dark room's colour as a metal's tint (the black glass); each pane on its own normal (rooms.wgsl `rm_pane_normal`: set up to ~0.46 degrees out of the wall's plane about each axis, a sealed unit bowed ~0.4 degrees at its edges; a window cut into panes of about 1.25 x 1.7 m), so the sky, the street and the sun's glint break pane by pane; behind it an office floor runs back 6.5-11 m to its core under a white acoustic ceiling lit by the daylight off the floor, each office its own troffer and grid layout, 85 % with them on and read by day as lit diffusers against the daylight (not the white shards of the default's gain); 62 % of office windows with a blind or (35 % of those) a dark solar screen of 14 % open weave, each at its own height, a fifth all the way down; homes also with roller shades, lit from behind by night (brightest across from the room's lamp, its light leaking past the fabric's edges); by night each lit room its own lamp's colour temperature (blackbody, seen against a 4300 K white: homes mostly 2300-3400 K, a fifth cool LED at 3800-4500 K and a few 5000-6200 K tubes; each office floor's tenant its own, 2900-4800 K) and offices as varied in brightness as homes; glass at the packing's limit (ribbons and curtain walls over 15.9 m) takes rooms of two or three 1.5 m mullion modules with their mullions and, on a curtain wall, the building's own storeys (its sill rhythm, `fa_drivers`): vision glass from floor to ceiling and an opaque spandrel to the next floor, not 3.6 m cells of the world (the checkerboard). INFERRED shares and sizes (rooms.wgsl `FG_*`). `--no-facade-glass` |
| `--far-facades` | The far layer's roofs and walls (eo-render hlod_draw.wgsl behind `SF_FAR_FACADES` and the frame switch `facade::RUN_FAR_FACADES`, hlod.rs `draw_wgsl`; off by default): a district's flat roof carries its plant (at most one unit of 1-3.6 x 1-3.2 m and 0.8-2.4 m tall in each 7 m square, 14-62 % of the squares by building), a stair or lift penthouse (3-5.5 x 3.5-7 m, 2.4-3.6 m tall) in 12-55 % of its 22 m squares, hatches and skylights, and the shade units and penthouses cast down the roof, as long as the sun's height throws it (a ten-step march toward the sun over their tops, so a golden-hour unit shades many metres), on a membrane of re-roofed patches and ponding stains (a cell's roof the membrane only: its bake carries the kit's own plant); each far window its own room (a blind or a lit ceiling in 36 %); a wall the far layer draws as its plain prism (a building with no kit plan and no model here, as the Ferry Building) a dressed-stone front: a ground-floor arcade of round-headed openings, a belt course, windows in stone surrounds with sills in storeys and bays, a cornice with dentils under its coping, each opening a recess its reveal shades and the low sun reaches only where the ray clears it, soiled at the foot and sooted under the cornice (the wall's top and first corner from its quad's first vertex, its foot from the cell's lowest point). Everything filtered over the pixel's footprint and given way to its mean below a pixel. INFERRED proportions (hlod_draw.wgsl `FF_*`, `FB_*`). `--no-far-facades` |
| `--facade-weather` | The kit walls' weathering where it reads (eo-render kit.rs `weather_marks`, draw.wgsl, facade_age.wgsl `fw_apply` behind `SF_FACADE_WEATHER` and the frame switch `facade::RUN_FACADE_WEATHER`; off by default, so the kit packs, their cache key and the frame are as before): every wall piece in a window's column (the band under a window and the band over the window below, one piece on the mid rung) carries its place across the window and its depth under the sill above in its normal's spare bytes (the kit tiles cook into their own cache folder with it), so the rain's run-off hangs from each sill: dark streaks from most sill ends (45-95 % by the building's wear), widening as they fall and fading over 0.5-3 m, uneven down their length, drips off the sill's front edge, and the dirt settled in the sheltered strip under the sill; at every wall's foot the splash-back (a ragged band of mud specks), on render and masonry rising damp with a pale salt tide line, and the street's grime fading up the ground floor. In the rain the walls are wet where the driven rain reaches them (more up the wall and under the roof's edge, less on the sheltered ground floor) and in the run-off's paths under each sill: render and masonry darker and deeper in colour, still matte; paint less dark and glossy; a sheet of water down the streaks. Every detail gives way to its mean as it shrinks under a pixel. INFERRED sizes and strengths (facade_age.wgsl `FW_*`). `--no-facade-weather` |
| `--shop-light [1\|2]` | Lit shop windows that carry the night street (eo-render `shop_lights.rs`, sky_lights.wgsl `lights_shops`, rooms.wgsl `rm_shop_light`; eo-sky `shops`; off by default, so the frame is as before): behind every shop window the kit draws (the kit planner's shop bays, planned again on worker threads once the kit city is loaded, a few seconds) a light 0.3 m inside the glass at the window's middle height, aimed out and a little down with the window's lobe, in its trade's colour as a night-adapted eye sees it beside SF's 3000 K street lamps (bars ~2300 K, restaurants 2700 K, shops 3500 K, markets and laundromats 4300-5000 K), at its window's luminance (bars ~16, restaurants ~40, shops ~75, markets ~120 cd/m2) times its glazed area, lit by its trade's opening hours with a per-shop spread (most shops dark after 19:00-21:00, restaurants until 22:00-23:00, bars until 02:00, a few markets, pharmacies and laundromats round the clock, a share of fronts vacant, a share of closed shops keeping a dim display or security light), so a block is mixed. Through the frame's light list: clustered (after every street lamp and headlamp: a crowded cluster drops shops first, and the lamp shadows' stable order ranks them last), occluded with `--light-shadows` (no map of their own), lighting the fog with `--lit-fog`; the room behind a kit shop's glass reads its light (lit in its colour at its window's luminance, or dark when the shop is closed), giving way to the rooms' own rule at each room's own distance over the last fifth of the reach. Stage N2: that room is the shop's own, filled by its trade (eo-sky `shops::fit_of`; eo-render `shop_rooms.wgsl`, read through the fronts table at group 1 binding 19): gondola runs and wall shelving stacked with goods (markets, liquor, pharmacies, hardware, books, flowers), clothing racks and a display table, tables and chairs under pendants with a counter and a lit kitchen pass, a bar's counter, stools and back bar, salon chairs facing a mirror wall, washers and stacked dryers, a gallery's framed pieces, a clinic's reception, a cleaner's bagged rails, a gym's machines; a back wall in the trade's colour; ceiling strips, troffers, downlights or track spots, the light falling to the back and the corners; now and then a person; colour fields and plain shapes, nothing that reads. One room behind every pane of the shop (its door and windows); its lit surfaces where the night exposure shows them (about a tenth of the window's luminance), its fixtures blown out; by day the same room through the glass; a closed shop dim under its security light. One vacancy rule (eo-sky `shops::vacant`, per shop): a vacant shop's lights stay dark (a few keep a work light), its room is emptied and dark and `--street-life` papers its glass; a front `--facade-clutter 2` boards up is vacant and never lit. `EO_SHOP_INTERIORS=0` (a diagnostic) keeps N1's rooms and W5's own vacancy. 1: within 160 m of the eye, one light a window; 2: to the light clusters' 300 m, a light every 2 m of window. Nothing by day. The `high` tier sets 1 and `hyper` 2; `--no-shop-light` |
| `--car-detail [1\|2]` | The cars' detail (eo-agents `vehicles::far_body`, eo-render `agents::Tiers::with_car_detail`; off by default, so the frame is as before). Without it each traffic body has four levels from the browser build: the full body within 22 m for the 2 nearest cars, LOD 1 within 60 m for the next 10, LOD 2 to 160 m, then three stacked boxes (36 triangles) to 400 m. With it the far slot is the body's own LOD 2 simplified at load (each material and wheel part on its own, by meshopt's clustering, to 0.2 m: about a pixel where the slot starts; 1,080-1,800 of LOD 2's ~5,900 triangles over the shipped fleet), in the same mesh slots, so the people's mesh budget is unchanged; the library is cached under a key of its own. 1: the full body within 30 m for the 6 nearest cars, LOD 1 within 90 m for the next 24, LOD 2 to 220 m, the far body to 400 m; 2: 40 m (12), 120 m (40), 260 m, 400 m. Shadow casters follow the same distances (cascade 2 casts the far body). Levels still switch without a fade, and nothing is drawn past 400 m (the moving traffic lives within about 240 m of the camera). The bench JSON records the level and the tiers under `car_detail`. The `high` tier sets 1 and `hyper` 2; `--no-car-detail` |
| `--look default\|cinematic` | The look profile (the cinematic look plan, stage L0; eo-look `cinematic`, `LookOptions::look`, `SkyOptions::cinematic`, the shader constant `SF_CINEMATIC`): `default` is today's frame on its physical references; `cinematic` is a graded open-world look (punchier contrast, warm lights against cool shadows, a richer golden hour, mixed night lamp colours, wet reflections), built by stages L1-L7. Every value it takes that departs from a physical reference the default keeps is a named constant listed in `CINEMATIC_DEVIATIONS` (eo-look `cinematic.rs`) and in the bench JSON's `look_profile`. `high` and `hyper` turn it on; `--no-look` (the same as `--look default`) turns it off wherever it stands. At stage L0 nothing draws differently yet: the switch, the compile-out constant and the deviation table only |
| `--structures`, `--tunnels` | Grade-separated roads (`eo-structures`: freeways, ramps, road bridges) and, with `--structures`, their tunnels; off by default, no tier sets them. A world switch: it changes collision and traffic, so `eo-server` takes it too and the handshake refuses a client whose setting differs from the server's. With it the street is built again with the structures' joins (`eo_structures::build_joined`; eo-app and `eo-server` alike, so they stand on the same street): a structure end where a street passes through or meets a junction is a leg of that junction (no kerb across its mouth), a stub ending at a structure is capped, a short bridge's stub climbs an approach embankment to the height its clearance needs, and the Central Freeway's columns stand in 13th Street's overlapping parking lanes; the structures stand retaining walls where that street meets them (eo-structures `walls.rs`): a wall with a coping over each face the street draws round a leg's junction, a walled fill with a barrier where a stub is raised past what its bank carries, a concrete edge beam under a kerbless stub's edge over the ground; the kit tiles of that street cook into their own cache folder. The props keep clear of the decks (trees become a smaller species or go, signal masts become pedestals or go, sign posts move along the kerb, lamps, furniture, overhead poles and wires under a deck and parked cars on a column go) and the built freeway decks get barrier-mounted luminaires every 50 m on both edges. Drawn by the city pipeline (`eo-render` `structures.rs`): every 128 m tile is one LOD group of the street's kind, its far and coarse levels static and its near level streamed by distance like the street's; the deck is the street's asphalt, the concrete a light warm grey with form panels, board lines and stain streaks (the CC0 board-formed scan when present), casting sun shadows at the level shown. `--tunnels`: tunnels draw and collide: bores (roadway, raised walkways behind a kerb, walls and a vault lined in glossy small cream glazed tile in grout, a glowing linear luminaire along the crown, a centre wall between twin bores), approach cuts with retaining walls (with a coping; the stubs lowered into them walled too), portal headwalls with a coping and an arch round each bore's mouth (twin bores each their own, the centre wall kept); a building over an approach cut or a portal's canopy is left out (as a building a landmark claims); the terrain (`scene::build_scene_with_cuts`) and the collision heightfield leave out the ground the cuts go through and the portals' canopies; inside a bore neither the sun nor the open sky light anything (only the daylight a portal lets in, falling off over its first 12 m), its lamps every 8 m under the crown are lit round the clock (100 lux on the roadway by day, the approach road's level by night, 4000 K) with an even interreflection off the tile, by day brighter over the threshold zone in from each portal (CIE 88: about 6 times the interior over the first 30 m, falling to the interior about 80 m in), and the exposure adapts to the bore over its first 20 m. A car (or the player) inside a bore is lit as the bore's own surfaces are (eo-agents `feed::in_bore` marks it with `AGENT_IN_BORE` and its depth in from the portal; eo-render agents_draw.wgsl `agent_bore_fs`, pipelines of their own made only when the world has bores: the bore's lamps and headlamps, its interreflection, the threshold zone and the portal's daylight; no sun, sky, rim or environment; `EO_AGENTS_NO_BORE_LIGHT=1` lights them as before, a diagnostic). Without the switch nothing is reserved, attached or drawn and the frame does not change |
| `--cache DIR`, `--no-cache` | Cache folder, or cook everything fresh |

### Bench

| Switch | What |
| --- | --- |
| `--frames N`, `--warmup N` | Measured and warm-up frames per path (720 and 60) |
| `--segments a,b,...` | Which camera paths to run, for example `street_walk`, `elevated_sweep`, `city_overview`, `authored_street`, `kit_street`, `drive`, `turn_around`, `waterfront`, `bay`, `bay_dive`; with `--structures` and built decks also `freeway_drive` (1.5 m over a built Route 80 deck near the I-80 approach, 150 m) and `under_viaduct` (1.7 m on 5th Street under the I-80 approach, sweeping across the deck) |
| `--capture` | Also write PNGs of each path to `bench-out/` |
| `--structures-shots` | Instead of the timing, the grade-separated roads' capture card (`capture-structures-<label>-<name>.png` and a JSON list): the I-80 approach, the Central Freeway, Selby Street under I-280 and the Presidio Parkway, each from the street and from above, at 15:00 under the clear preset (weather seed 7); the poses are placed on the structures (built for the poses alone without `--structures`), so runs with and without them match; `EO_STRUCTURES_SHOTS=a,b` keeps some, `EO_STRUCTURES_SHOTS_HOUR=H` takes them at another hour, `EO_STRUCTURES_SHOTS_HDR=1` also writes each card's scene luminance (`.lum`: `EOLM`, width, height, f32 a pixel); `EO_STRUCTURES_SHOTS` naming them adds stage A8's cards: `join-leg-214`, `join-leg-215`, `join-leg-service` (a junction where a structure end is a leg, from the street and from above), `join-raised-306`, `join-raised-132`, `join-raised-203` (a raised stub's approach), `thirteenth` (the Central Freeway over 13th Street), `tunnel-macarthur-south`, `tunnel-macarthur-north`, `tunnel-broadway-west`, `tunnel-broadway-east` (`-out` 45 m out on the approach looking at the portal, `-in` 40 % into the bore looking back at it), placed on the street datum and on a tunnel world built for the poses, so runs with and without `--structures` / `--tunnels` match |
| `--light-shots` | Instead of the timing, the lighting reference cards (`capture-light-<label>-<name>.png` and a JSON list): an alley and a plaza at noon and at golden hour, the skyline and a hill's shadow at low sun, a parked row and a junction at night, under the I-80 viaduct at noon and at golden hour, a dusk strip of the kit street, and (`far-*`) the `city_overview` path's first frame and the `elevated_sweep` path's first and quarter frames at golden hour; `EO_LIGHT_SHOTS=a,b` keeps some, `EO_LIGHT_SHOTS_HOUR=H` draws every card at hour H; `EO_PROBE_STRIP=path[,frames[,m a frame[,turn]]]` instead walks a street (default `kit_street`, 120 frames of 0.5 m, looking 45 degrees across) one frame a step, reads every frame back and writes a CSV of the probes' finished captures and the largest change of a screen cell a frame, with the frames round each capture. Every capture card (`--capture`, the cards here) first waits for `--probe-grid`'s set to be captured; `EO_CAPTURE_SETTLE_PROBE=1` waits for the single probe's capture too (matched pairs). `EO_LIGHT_SHOTS_WEATHER=fog` (any `--weather` preset) draws every card under that weather instead of clear; `EO_LIGHT_SHOTS` naming `night-kit` adds `night-kit-street-0`, `-mid` and `-260` (the night_kit_street path's frames at night); `EO_LIGHT_SHOTS` naming `shops` adds `shops-castro-18th` (Castro Street from the south-east corner of 18th, looking up the block), `shops-castro-walk` (12 m up it on the east walk) and `shops-mission-24th` (Mission Street 12 m north of 24th on the east walk), at night; with `--shop-light` each card's JSON carries the frame's lights by kind and the clusters' fill (`shop_lights`); `EO_HEADLIGHT_STRIP=frames` (default 60) instead waits at the props card `junction`'s street for a car passing at over 5 m/s, stands 11 m beside its path and writes every frame of it crossing the view (`capture-light-<label>-headlights-fNN.png`, at night, fog unless `EO_LIGHT_SHOTS_WEATHER` says otherwise) with a JSON (`--lit-fog`'s cards; `EO_HEADLIGHT_STRIP_VIEW=behind` stands 9 m behind the car looking along its heading instead). With `--light-shadows` or `--headlight-shadows` each card's JSON carries the selection (`light_shadows`); `EO_LIGHT_SHADOW_ATLAS=1` writes the atlas as a grey PNG beside each card and `EO_LIGHT_SHADOW_MAPS_OFF=1` draws the maps but lets no light read them (a pair that shows the shadows alone). `EO_LIGHT_SHOTS=hero` draws the six key views of the cinematic look plan (`hero-castro-night`, `hero-mission-wet`, `hero-downtown-golden`, `hero-viaduct-golden`, `hero-embarcadero-blue`, `hero-aerial-golden`; bench `hero_views.rs`), each at its own hour, weather and sky seed with the sky's clock held at `--world-time` and the water's fixed while it settles, so two runs give the same frame; the viaduct card needs `--structures`; `EO_HERO_TUNE` tries other cameras |
| `--cc0-shots` | Instead of the timing, the CC0 scanned sets' matched cards (`capture-cc0-<label>-<name>.png`): the kit street at noon, golden hour and night, a sidewalk walk, one close wall per finish class, the kerb, the walk and a wheel path up close, plazas, seawalls, a beach and the city from above; `EO_CC0_SHOTS=a,b` keeps some, `EO_CC0_SHOTS_HOUR=H` draws every card at hour H (the `--grime` evidence is taken at 15) |
| `--clutter-shots` | Instead of the timing, the facade clutter's cards (`capture-clutter-<label>-<name>.png`): the kit and authored street paths at two frames, fronts the planner dresses at level 1 whatever the run draws (fire escapes in the Tenderloin, Nob Hill and Chinatown, shop fronts in the Mission, the Castro and Chinatown, a blade sign, an awning and the services up close, an escape from 110 m) and a roof from above, and (stage S1) lettered signs close up in the Castro and Chinatown and neon signs in shop windows in the Mission and the Castro, so a run with `--facade-clutter` and one without stand at the same places; `EO_CLUTTER_SHOTS=a,b` keeps some, `EO_CLUTTER_SHOTS_HOUR=H` draws every card at hour H (default 15) |
| `--dusk-strip` | Instead of the timing, the drawn hour moving at one camera (the kit street, sky in view): 40 frames a clock minute apart across sunset and 40 across sunrise (`capture-light-<label>-dusk-<nn>.png`, `-dawn-<nn>.png`), a CSV each of the mean luminance and its step from the frame before, and a JSON summary. With `--smooth-time` each frame is reached through the soft path, without it through one `--time-of-day`-style set a frame; `EO_DUSK_STRIP=dusk` or `dawn` runs one |
| `--out DIR`, `--label NAME` | Output folder and run label |
| `--check-budget` | Exit 1 if a path's median frame reaches the budget (10 ms; the `--quality` tier's when one is given) |
| `--check-max` | Exit 1 if any path's median, p95 or worst frame reaches the budget |
| `--worst-frames N` | How many of the worst frames to report |
| `--pace realtime\|60hz\|all-60hz\|none` | How the bench paces its frames |

### Kill switches

Every visual feature can be turned off from the command line, so a change
can be compared with and without it (A/B) and a fault can be isolated. A
selection:

| Area | Switches |
| --- | --- |
| Content | `--no-models`, `--no-kit`, `--no-streets`, `--no-props`, `--no-agents`, `--no-landmarks`, `--no-water`, `--no-farshore`, `--no-sky` |
| Water optics | `--water-optics` enables the preview, `--no-water-optics` restores the original water shading |
| Underwater habitats | `--underwater-habitats` adds the Bay and Pacific beds. `--no-underwater-habitats` removes them |
| Aquatic Park Cove | `--cove-detail` enables the beach profile, cobbles, pier parapet, distant wooded slope, fair-weather clouds and cove optics. `--no-cove-detail` restores the previous scene. The Bay dive-site starts enable the preview. |
| Black Point canopy | `--cove-canopy` enables the cove preview and resolves bark, branches and leaf cover from eight views of each park tree species. `--no-cove-canopy` restores the previous distant crowns. |
| Submerged cove sediment | `--cove-sediment` enables the cove preview with rough submerged sand and silt. `--no-cove-sediment` restores the previous seabed material. |
| Municipal Pier pile contact | The cove preview fits each support to the drawn seabed. `--no-cove-pile-contact` restores the previous fixed pile ends, and `--cove-pile-contact` enables the preview. The park describes the pier’s [piled construction and protection against scour](https://www.nps.gov/safr/learn/historyculture/aquatic-park-pier.htm). |
| Black Point intertidal | `--dive-site black-point` starts beside barnacle-covered rock and small algae at the west end of Aquatic Park beach. `--black-point-intertidal` adds this habitat to the map. `--no-black-point-intertidal` hides its geometry. The location follows the [National Park Service account](https://www.nps.gov/safr/blogs/a-hidden-community-in-aquatic-park-cove.htm). Individual rocks and organisms are modeled. |
| Black Point rockweed | `--black-point-rockweed` models forked blades with raised midribs and thickened tips on the rocky habitat. `--no-black-point-rockweed` uses the original ribbons. Blade margins transmit more backlight than the thicker rib. The geometry follows the form described by [Stanford’s Hopkins Marine Station](https://seanet.stanford.edu/brown) and the habitat reported by a [central Bay shoreline survey](https://escholarship.org/uc/item/8jt3v8mk). Individual plants are illustrative. |
| Dive exposure | `--dive-exposure` compensates for the loss of downwelling light at the eye. `--no-dive-exposure` restores the original exposure |
| Bridge detail | `--bridge-detail` enables the Golden Gate detail preview (a darker paint tone, ribbed tower legs, framed portal struts, full arch pylons, lamp standards), `--no-bridge-detail` keeps the bridge as exported |
| People | `--no-rocketbox`, `--no-rocketbox-motions` (either one: no people) |
| Props | `--no-props2`, `--no-furniture`, `--no-eztree`, `--no-wires`, `--no-overhead`, `--no-lamp-sprites`, `--no-stand-ins`, `--no-surveyed-furniture`, `--no-tree-basins`, `--no-street-props-v2` (the v1 street-prop files the JS lists, drawn as proxies or stand-ins where the checkout lacks them, as before the v2 models) |
| Harbours | `--no-floating-docks` (no floats along the harbours' pier lines, and so no berths or boats), `--no-boats` (the floats without the moored boats; `--no-props2` also leaves them out) |
| Buildings | `--no-facade-detail`, `--no-facade-age`, `--no-rooms`, `--no-room-frames`, `--no-room-dim`, `--no-roof-colours`, `--no-roof-plant`, `--no-roof-parapet`, `--no-roof-patches`, `--no-wall-paint`, `--no-facade-glass`, `--no-far-facades`, `--no-facade-weather` (opt-in, off by default) |
| Ground | `--no-ground-hills`, `--no-ground-green`, `--no-ground-contact`, `--no-ground-beach-fit` (the sand round a mapped beach spreads as the JS rule does: over lawns and paving mapped beside it and along a seawall's quay), `--no-terrain-lod` |
| Scanned surfaces | `--no-cc0` (all), `--no-cc0-street`, `--no-cc0-walls`, `--no-cc0-ground`, `--no-cc0-decals`, `--no-cc0-grime`, `--no-cc0-stains`, `--no-cc0-pom`, `--no-cc0-tiling` |
| Lighting and sky | `--no-shadows`, `--no-ao`, `--no-contact-shadows`, `--no-atmosphere`, `--no-fog`, `--no-clouds`, `--no-cloud-shadows`, `--no-cloud-depth-haze`, `--no-shafts`, `--no-moon-stars`, `--no-lights` |
| Lighting additions (opt-in) | `--no-cascade-blend`, `--no-far-shadows`, `--no-ambient-volume`, `--no-probe-grid`, `--no-light-shadows`, `--no-light-shadow-moving`, `--no-headlight-shadows`, `--no-lit-fog`, `--no-smooth-time`, `--no-ground-bounce`, `--no-shadow-slope-bias`, `--no-grime`, `--no-street-life`, `--no-photo-finish`, `--no-facade-clutter`, `--no-shop-light`, `--no-car-detail`, `--no-look` (each wins over `--quality`) |
| World additions (opt-in) | `--no-structures`, `--no-tunnels` (the last of a switch and its `--no-` form wins; `eo-server` takes them too) |
| Weather | `--no-rain`, `--no-wet`, `--no-wind` |
| Camera finish | `--no-taa`, `--no-bloom`, `--no-adaptation`, `--no-ssr` |
| Visibility and streaming | `--no-occlusion`, `--no-view-detail`, `--no-upload-ring`, `--no-submit-pacing`, `--no-indirect-count`, `--no-count-emulation` |
| Interface and sound | `--no-hud`, `--no-audio`, `--audio-null` (mix to a null output), `--no-settings-apply` (`--ui`: the settings file does not reach the renderer or the world) |

A new visual feature should come with its own switch, defaulting to on only
once it has been reviewed.

### Quality tiers

`--quality` expands, once every switch has been read, to the settings
below; a switch it sets that is also given on the line (`--shadow-map`,
`--no-ssr`, `--sky-quality`, a lighting switch or its `--no-` form) keeps
its own value. Each tier has its own frame budget for `--check-budget` and
`--check-max` at 1920 x 1080; `--budget-ms` overrides it. The bench JSON
records the tier and budget under `settings` and every lighting option under
`lighting`. Every lighting addition draws (none is plumbing only any more).
A feature that is off at start has its shader code compiled out of every
pipeline (eo-render `look::ShaderFeatures`, chosen from the switches before
the renderer is made): the lighting additions, `--grime`, `--street-life`,
`--structures`, `--tunnels`, `--shop-light`, `--facade-clutter`, `--look cinematic` (`SF_CINEMATIC`), `--facade-glass`, `--far-facades` and `--facade-weather`. With
none of them the frame costs what it cost before they existed; with one on,
its code is the code it ran before, so its output is the same.

| Tier | Sets | Budget (ms) |
| --- | --- | ---: |
| `low` | `--shadow-map 2048`, `--no-ssr`, `--no-contact-shadows`, `--sky-quality low`, `--no-ao`, `--no-shafts`, `--no-bloom`, `--surveyed-furniture`, `--tree-basins` | 8 |
| `medium` (default) | the street look: `--grime 1`, `--stand-ins`, `--surveyed-furniture`, `--tree-basins` (made the default with the owner's approval on 2026-11-20, after the citywide street audit; `--no-grime --no-stand-ins --no-surveyed-furniture --no-tree-basins` gives the frame before it) | 10 |
| `high` | `--cascade-blend`, `--far-shadows 1`, `--ambient-volume 1` (with `--ground-bounce`), `--light-shadows 16`, `--headlight-shadows 2`, `--lit-fog 1`, `--smooth-time`, `--shadow-slope-bias`, `--grime 1`, `--street-life 1`, `--stand-ins`, `--photo-finish 1`, `--facade-clutter 1`, `--shop-light 1`, `--car-detail 1`, `--surveyed-furniture`, `--tree-basins`, `--look cinematic` | 16.6 |
| `hyper` | `high`, then `--shadow-map 4096`, `--sky-quality high`, `--far-shadows 2`, `--ambient-volume 2`, `--probe-grid 2`, `--light-shadows 48`, `--light-shadow-moving`, `--headlight-shadows 8`, `--lit-fog 2`, `--grime 2`, `--street-life 2`, `--photo-finish 2`, `--facade-clutter 2`, `--shop-light 2`, `--car-detail 2`, `--surveyed-furniture`, `--tree-basins`, `--look cinematic` | 33.3 |

The tiers are tuned to their budgets on the bench paths (stage T2). With
every feature compiled in, what a tier costs is mostly the lit shader's
code, not the features' own passes: `--probe-grid` (every lit pixel reads
two probes) cost the old `high` 4.9 ms on `city_overview` and 1.6 ms on the
street paths, so the grid is `hyper`'s alone (a bare `--probe-grid` still
takes 1); `--no-ao`, `--no-shafts` and `--no-bloom` (which have no positive
form, so `low` always drops them) take 0.3-0.6 ms off the street paths.
`low` cannot reach 8 ms on `city_overview`: about 4.8 ms of that path is
the CPU's draw encoding (Metal has no indirect-count draw, so each
compacted slot is a CPU draw call; see [What is drawn](#what-is-drawn)) and 3.9 ms the
city's draw, which no tier switch changes without drawing less of the
city. That is left to the owner (draw less in `low`, or a higher `low`
budget); the budget stays 8 ms. Frame total medians, ms, from
`--bench --check-max --quality TIER` on an idle Apple M-series machine
(1920 x 1080, 720 frames a path; worst frames over the budget are named
in the bench JSON):

| Path | `low` | `medium` | `high` | `hyper` | `hyper` `--structures --tunnels` |
| --- | ---: | ---: | ---: | ---: | ---: |
| `street_walk` | 3.2 | 3.9 | 6.0 | 7.9 | 10.7 |
| `city_overview` | 10.7 | 10.9 | 15.1 | 20.0 | 22.5 |
| `kit_street` | 3.3 | 4.0 | 6.1 | 7.9 | 10.3 |
| `drive` | 3.2 | 3.8 | 6.0 | 7.7 | 10.2 |
| `bay` | 5.2 | 5.5 | 7.4 | 8.9 | 9.9 |
| `rain_street_walk` | 4.2 | 4.8 | 7.7 | 9.9 | 12.1 |
| `fog_elevated_sweep` | 5.3 | 5.7 | 8.3 | 10.8 | 11.4 |
| `night_kit_street` | 3.6 | 4.2 | 6.7 | 9.0 | 11.2 |
| `night_waterfront` | 4.5 | 4.9 | 6.7 | 8.2 | 8.5 |

The settings screen (`eo-ui` `settings.rs`) has a "Lighting and detail"
row with these tiers (Low, Medium, High, Hyper real) and a "Hyper real"
preset beside High, Medium and Low; each preset sets the tier of its name.
A separate World block holds "Grade-separated roads" (off, on, on with
tunnels: `--structures`, `--tunnels`), which no preset touches. In the
`--ui` window the stored record applies at start as if `--quality TIER`,
`--no-shadows` (when "Sun shadows" is off) and `--structures [--tunnels]`
were written first on the line, so any switch on the line wins. A change in
the menu reaches the running renderer for the sun's shadows and map size,
the sky (with its shafts and bloom), the surface occlusion and the photo
finish, turns lamp and headlight shadows, the lit fog
and the smooth clock on and off when the game started with them, and turns
the far shadows, ambient volume and probe grid off; facade clutter,
stand-ins, weathering decals, street life, shop windows, the attached
features and the compiled-out lighting coming on and the roads take effect
after restart,
as the rows' notes say. A fresh install stores Medium (the street look);
a file written before the row existed takes the tier of the preset its
rows match (else Medium). The captures and `--ui-bench` never read the file, and
`--no-settings-apply` keeps the window to the line alone.

### Scale of the ground plan

The street plan is drawn wider than the survey about the map origin, an
approved deviation from the California street standards recorded in
`eo-data/src/standards.rs`. `--ground-scale`, `--height-scale`,
`--people-scale` and `--vehicle-scale` override the factors for experiments,
and `--low-camera` / `--default-camera` choose the player camera height.
Experiments are not shipped defaults.

## Environment variables

| Variable | What |
| --- | --- |
| `EO_REPO_ROOT` | The repository root, if it cannot be found from the working directory |
| `EO_NATIVE_CACHE` | The cache folder (default `native/cache/`) |
| `EO_UI_SETTINGS` | Where the interface keeps its settings file (default: the user's application-data folder) |
| `EO_UI_FONT_DIR` | A font folder to use instead of the system font folder |
| `EO_TRACE_LOAD` | Print timings of the world load |
| `EO_CUMULUS` | `1` draws the fair-weather cumulus field under a clear sky (off by default; the overcast, fog and rain decks are drawn either way) |
| `EO_CAPTURE_HOLD_SKY` | `1` holds the sky's clock at `--world-time` for every pose of an `EO_CAPTURE_POSES` file, so the drifting clouds are the same in two runs (matched captures) |
| `EO_CAPTURE_STREETS` | `N` with `--bench --capture-props2`: the citywide street audit instead of the props cards: up to N street-level views spread over the whole map (a grid of cells over the street plan, the longest run of a class taken in turn in each: arterial, collector, residential), seen from the wider footway at eye height, written as `capture-<label>-street-NN-<class>.png` with the poses file, so a change to the street is judged across the city; deterministic (the same plan gives the same views) |
| `EO_GRIME_STREET_V1` | `1` (a diagnostic) keeps the street weathering `--grime` drew before the road's cracks and the markings' wear were redrawn (the polygon crack network, the flaked paint, the scan's full holes), for matched A/B captures on one build |

The locations of the optional off-repository assets are read from the
variables below.

### Off-repository folders

Each of these names a folder outside the repository. None has a default:
when one is unset, or its folder does not match its manifest, that feature
is turned off, the log says so in one line, and the game runs without it.

| Variable | Folder it names | Read by |
| --- | --- | --- |
| `EO_PEOPLE_ROOT` | Rocketbox avatars | The game |
| `EO_PEOPLE_MOTIONS_ROOT` | Rocketbox motion clips | The game |
| `EO_PEOPLE_STYLES_ROOT` | 100STYLE walking styles | The game |
| `EO_CC0_ROOT` | Scanned CC0 texture sets (Poly Haven, ambientCG) | The game, `tools/fetch-cc0.mjs` |
| `EO_AUDIO_ROOT` | Sound clips | The game |
| `EO_SIGN_ART_ROOT` | Official sign art and the sign font | The game, `tools/fetch-sign-art.mjs` |
| `EO_DEM_ROOT` | Elevation tiles for the far shore | The game |
| `EO_AERIAL_DIR` | Aerial imagery | Tools only (the roof survey and `tools/fetch-aerial.mjs`) |
| `EO_POLYHAVEN_STORE` | Poly Haven downloads | The Blender building scripts |

The street people need all three `EO_PEOPLE_*` variables. With any of them
missing, or with `--no-rocketbox` or `--no-rocketbox-motions`, there are no
people on the streets: there is no other motion set to fall back on.

The fetch tools in `tools/` need either a folder argument (`--root <folder>`;
`fetch-aerial.mjs` takes it as its first argument) or their
variable; they have no default folder.

## Where the data comes from

The program finds the repository by walking up from the working directory
(and from the executable's folder) to the first folder that holds
`public/data/sf/sf-city.json.gz`. Set `EO_REPO_ROOT` to override that.

It reads, all tracked in the repository:

| File | What | Source and licence |
| --- | --- | --- |
| `public/data/sf/sf-city.json.gz` | Streets, footprints, signals, shoreline and city boundary | OpenStreetMap (ODbL 1.0) and DataSF (PDDL 1.0); every source, URL and digest is in the file's `meta.sources` |
| `public/data/sf/sf-footprints.json.gz` | Citywide building footprints and heights | DataSF Building Footprints (PDDL 1.0) |
| `public/data/sf/sf-footprint-supplement.json` | Footprint records keyed to the hand-built buildings | DataSF footprints and assessor roll (PDDL 1.0) |
| `public/data/sf/sf-elevation.bin.gz` | Ground and seabed on an 8 m grid | USGS 3DEP and NOAA NCEI CUDEM (public domain) |
| `public/data/sf/sf-land-cover.bin.gz` | Land-cover classes for unmapped open ground | ESA WorldCover 2021 (CC BY 4.0) |
| `public/data/sf/sf-lots-parks.json.gz` | Car parks, green areas, inland water | OpenStreetMap (ODbL 1.0) |
| `public/data/sf/sf-map-citywide.json.gz` | Map cartography, piers, place names | OpenStreetMap (ODbL 1.0), DataSF (PDDL 1.0), USGS (public domain) |
| `public/data/sf/sf-storm-drains.json.gz` | Storm drains in the gutters | DataSF (PDDL 1.0) |
| `public/data/sf/building-metadata/` | Ground-floor shop categories | DataSF (PDDL 1.0), OpenStreetMap (ODbL 1.0), SF Planning (CC0 1.0); inputs in each `index.json` |
| `native/assets/roof-colours-v1.bin.gz`, `ground-green-v1.bin.gz` | Measured roof colours and vegetation shares | Derived from USDA NAIP aerial imagery (public domain) |
| `scripts/world-data/sources/california-street-standards.json` | The cited street-standards table | California manuals; see [Docs/CALIFORNIA-STREET-STANDARDS.md](../Docs/CALIFORNIA-STREET-STANDARDS.md) |
| `public/assets/` | Hand-built building exteriors, vehicles, a few street props, texture atlases | Each with a provenance file beside it |

Each data file has a `*.NOTICE.txt` beside it with its source, retrieval,
digest and required attribution. Models and textures under `public/assets/`
are checked against the SHA-256 digest in their provenance file before they
are used; a file that does not match is refused.

## Optional off-repository assets

Some content is too large to keep in Git, or its source asks that it be
fetched from the source. The program runs without any of it and draws a
simpler stand-in where it is missing. Each kind has a manifest in
`native/assets/` (or `eo-props/data/`) that pins every file by size and
SHA-256; a folder that does not match its manifest is ignored as a whole,
and the log says why.

| Content | How to get it | Manifest |
| --- | --- | --- |
| Rocketbox people | A copy of the GLB repackaging of the Microsoft Rocketbox Avatar Library (MIT) | `assets/people-manifest.json` |
| Rocketbox motion clips | A copy of the Rocketbox in-place clips in GLB (MIT) | `assets/rocketbox-motions-manifest.json` |
| 100STYLE walking styles | A copy of the 100STYLE dataset in GLB (CC BY 4.0) | `assets/100style-manifest.json` |
| Scanned surface sets | `node native/tools/fetch-cc0.mjs --root <folder>` (Poly Haven and ambientCG, CC0) | `assets/cc0-manifest.json` |
| Sign art and sign font | `node native/tools/fetch-sign-art.mjs --root <folder>` (FHWA, Caltrans, Overpass) | `eo-props/data/sign-art-manifest.json` |
| Sound clips | FSD50K and SFX Kit clips (CC0 and CC BY); credits in `assets/AUDIO-CREDITS.md` | `assets/audio-manifest.json` |
| Far-shore elevation | USGS 3DEP 1 arc-second tiles (public domain) | `assets/dem-manifest.json` |
| Aerial imagery | `node native/tools/fetch-aerial.mjs <folder>` (USDA NAIP, public domain); only the roof survey tool reads it | written beside the imagery |

The scripts in `tools/` refuse a target folder inside the repository, send
nothing about the user in their requests, and never execute what they
download. `write-people-manifest.mjs` and `write-motion-manifests.mjs`
regenerate the people and motion manifests from a local copy.

Without all three people and motion copies there are no street people.
Without the sound clips the game is silent; without the scanned sets, sign
art or far-shore tiles, those surfaces, faces and the distant relief fall
back to simpler drawings or are left out. The variables that name each
folder are listed under [Environment variables](#environment-variables).

Some street-prop models and two vehicle bodies are kept outside the
repository because they cannot be redistributed; without them the game
draws grey boxes for those props (procedural models of them with
`--stand-ins`) and other vehicles in traffic.

The source binaries of the asset pipelines (Blender scenes, vehicle
sources) are a separate, optional store; the game never reads
them. See [Docs/ASSET-STORE.md](../Docs/ASSET-STORE.md).

## What is drawn

This is a summary of the systems, not a description of how they look.

**Renderer.** One vertex and index arena, one object table, and GPU culling:
a compute pass chooses each object's level of detail and tests it against
the frustum and a hierarchical depth pyramid (occlusion), then the frame is
drawn with multi-draw indirect. Streamed content (models, building tiles,
street tiles) is uploaded through a budgeted staging ring, a few megabytes a
frame, so streaming does not stall the frame. The best-quality layer is
loaded for what the camera can see, with a lighter level around it.

The culled lists are compacted on the GPU and drawn with the count the cull
wrote (`MULTI_DRAW_INDIRECT_COUNT`). Metal has no such draw, and wgpu turns a
multi-draw there into one CPU draw call per slot, so without the feature
the lists are still compacted and drawn to a bound read back from their
counts a few frames late (`eo-render/src/counted.rs`): every list's recent
peak plus headroom, the slots past this frame's count zeroed, the whole list
after a camera cut or when a count outran its bound. `--no-count-emulation`
keeps the older uncompacted fallback (one draw per object slot, no
occlusion, no far layer, no water); `--no-indirect-count` selects the
emulation on an adapter that has the feature.

**Hand-built buildings.** Several hundred buildings are modelled in Blender
from DataSF footprints, LiDAR heights and the assessor roll
(`scripts/blender-buildings/`). Their exteriors ship as hash-verified GLBs
under `public/assets/buildings/`, placed by the modules in
`assets/blender-buildings/`.

**Kit buildings.** Every other building, about 170,000, is planned by
`eo-kit` from its footprint, height, district and use: an archetype, a
street front, storeys and bays, shop fronts, roof plant, materials. Plans
are meshed per 128 m tile at several levels, cooked on worker threads and
cached. Facades carry a detail atlas, weathering and age, window rooms
behind the glass, and roofs coloured from the measured aerial table. With
`--facade-clutter` (opt-in; the `high` and `hyper` tiers) they carry what
people live with: fire escapes on the older apartment fronts, awnings, signs
carrying generic words of the shop's trade in system fonts (lit by night,
`eo-render/src/sign_letters.rs`, `eo-props/src/shop_lettering.rs`) and
roll-down shutters on the shop fronts, bars over ground-floor windows, downspouts, meters and conduit at the
base, chimneys, vents, dishes and aerials on the roofs (`eo-kit/src/clutter.rs`).

**Far city.** Past the near bands, buildings are drawn from merged meshes
prepared ahead of time: 256 m neighbourhood cells with a facade atlas, then
2,048 m districts with flat facade colours, with distant lit windows and
distant trees. The level is chosen per building on the GPU, so one draw per
cell covers the buildings that are far in this frame.

**Terrain.** The 8 m elevation grid in 256 m tiles with a coarse far level,
tinted by land cover and the open-space ground (greens, sand, park trees).
Past the city, `eo-farshore` draws the surrounding land to the horizon from
the optional elevation tiles.

**Streets.** `eo-street` builds the street surface from the street data and
the California standards: carriageway, kerbs, gutters, sidewalks, corner
radii, kerb ramps, crossings and lane paint, per 128 m tile near the camera,
with flat and coarse levels further out. Every building's ground floor and
entrances are fitted to the drawn street. With `--grime` (opt-in; the `high`
and `hyper` tiers) the streets, kerbs, facades and cars weather as the city's
photographs show: soiled walks with gum and stains, faded red kerbs at the
corners, patched and crack-sealed carriageways, washed-down facades and dusty
cars, and per-building stain plumes, damp and repainted paint blocks that
read from a block away (`eo-render/src/weathering.rs`). With `--street-life`
(opt-in; the same tiers) they show who uses them: litter, butts, leaves and
grit on the walks and in the gutters, gathered in drifts and piles that read
down the block, heaps of bags and cardboard at the walls, bold graffiti
(tags, throw-ups, pieces), buffed patches and posters on the ground floors and
the freeway columns, by land use and foot traffic
(`eo-render/src/street_life.rs`).

**Water.** A camera-following grid displaced by a cascaded FFT ocean
spectrum, masked by a citywide shore field, with its own optics per body of
water, screen-space reflection, surf on the mapped beaches and a shore
collar. `--no-water` draws a flat sea plane.

Run `./target/release/eo-app --water-optics --start-underwater` for the
submerged fly camera. Hold Ctrl for fine movement, E to surface and Q to
descend. The preview requires single-sample rendering (`--msaa 1`).
It uses the displaced wave surface, refraction at the
waterline, depth-dependent scattering and absorption, and moving light on
shallow receivers. Visibility follows the Bay and Pacific water properties.
Above the surface the preview integrates unresolved sun glitter over the
pixel. The `bay_dive` bench path crosses the surface in both directions.

Start beside swaying eelgrass in Aquatic Park Cove with
`./target/release/eo-app --dive-site bay`, or above wave-rippled sand and
partly buried shells off Ocean Beach with `--dive-site pacific`. Both fly
camera starts enable water optics, underwater habitats and dive exposure.
Each feature has an independent off switch. Dive exposure compensates up
to four stops as the light reaching the camera decreases and partially
corrects the local colour cast. The `bay_grass` and `pacific_sand` bench
paths swim across the same beds.

For the view from the beach, use `--dive-site bay-surface`. Use
`--dive-site black-point` for the rocky habitat near the west end of the beach.
All three Bay starts load the same trees, sediment, intertidal rocks and water. Within the south beach, join mean water level to
the first valid depth in the shipped NOAA topobathymetric grid, then preserve
the measured offshore profile. The gaps between soundings are interpolated.
The nearshore cobbles and suspended sediment are illustrative, with estimated
optical properties. The `cove_descent` path crosses the waterline and travels
112 game metres offshore. Municipal Pier follows the city map polygon and
Black Point's far trees use the mapped woods and existing park-tree placement.
The pier deck is carried on piles above the water. Its concrete parapet is described in the [National Park Service cultural landscape report](https://www.nps.gov/safr/learn/management/upload/cultural_ldscape_rpt_opimized.pdf).

The placements illustrate regional habitats. Local shore distance and the
drawn terrain determine where each patch fits. The eelgrass and ripples
are original procedural geometry inspired by descriptions of [Bay eelgrass](https://www.epa.gov/sfbay-program/priorities-subtidal)
and [the Ocean Beach nearshore bed](https://www.spn.usace.army.mil/Portals/68/docs/P%20and%20Programs/Navigation/SF-17_EA_and_Draft_FONSI_05062025.pdf).

**Sky and weather.** Analytic sky and atmosphere, volumetric clouds and the
marine layer, a weather schedule (clear to storm) with rain, wet ground and
drying, sun shadows in cascades, moon and stars, and night lighting: street
lamps, vehicle lamps and lit windows. The camera finish is temporal
anti-aliasing, bloom, exposure adaptation, tone mapping and a grade.

**Landmarks.** The two bridges, the quay wall and piers, seawalls, plazas,
landmark towers and the bridges' lights are generated by `eo-landmarks`.

**Traffic and people.** `eo-sim` runs traffic on the street graph with
signals and junction rules, and pedestrians on the sidewalks and crossings,
at a fixed step and deterministic for a given seed, with population and
detail tiers around the camera. `eo-agents` draws the cars (with wheel rigs
and lamps) and the people as skinned, instanced meshes. With the optional
copies present, the people are Rocketbox avatars that stand, talk and phone
with Rocketbox clips and walk in 100STYLE styles.

**Player.** `eo-player` is a kinematic character controller and a raycast
vehicle on [Rapier](https://rapier.rs) with enhanced determinism, against a
collision world streamed per 128 m tile from the drawn street surface and
building prisms. The step is fixed at 1/60 s and reads no wall clock, so the
same inputs give the same body; the server runs the same step.

**Props.** Signal poles, mast arms and heads, pedestrian signals, signs
(official sign layouts when the sign art is present), street trees from the
DataSF tree list (ez-tree style trees), street furniture, tactile pads,
overhead wires and parked cars that follow the parking rules, placed per
tile by rule and drawn as instanced classes.

**Sound.** A real-time mixer (hundreds of sources, the most audible few
dozen mixed, with distance, pan, Doppler and air absorption) and a director
that plays footsteps by surface, traffic engines and road noise, rain,
wind, the sea, birds and the city's ambience from the game state.

**Interface.** Title screen and menu, boot progress, pause menu with map,
settings, and the HUD with radar, drawn in one pass over the frame with
cached text. Fonts are resolved from the system font folder, with the
bundled `assets/fonts/kenpixel.ttf` as the fallback.

**Online.** An authoritative server at 60 Hz with region cells and agent
hand-off, delta-compressed snapshots with interest management, client
prediction and reconciliation over UDP.

## Tests

```sh
cargo test --release --workspace         # everything; several tests need a GPU and the real data
cargo test --release -p eo-data          # one crate
cargo test --release -p eo-player
cargo test --release -p eo-net -p eo-server
```

What the tests cover, in short:

- **Data and assets** (`eo-data`, `eo-assets`): the real files load with the
  expected counts; every GLB matches its manifest digest, and a corrupted or
  truncated file is refused and never cached; cooked models and caches round
  trip exactly.
- **Planners and ports** (`eo-kit`, `eo-street`, `eo-look`, `eo-water`,
  `eo-sim`, `eo-player`): answers compared with the reference fixtures in
  each crate's `data/` folder; byte-identical output when planned again in
  another order and on other threads.
- **Renderer** (`eo-render`, `eo-app`): culling and occlusion agree with a
  CPU reference and never drop an object with a visible pixel; no two levels
  of a building are drawn together; each pixel resolves its own object's
  material, and a planted swap is caught; no building floats or buries an
  entrance; the GPU FFT matches a CPU sum.
- **Sky and light** (`eo-sky`): daylight, night lamps, moon and twilight
  against published illuminance tables and lighting recommendations.
- **Player and network** (`eo-player`, `eo-server`): kerbs climbed, walls
  stop the walker, a long walk and drive over the hills stay on the ground;
  server and client agree bit for bit (with `--structures` on the joined
  street, and with `--tunnels` on the tiles the bores' cuts hollow out:
  `eo-server` `structures`).
- **Grade-separated roads** (`eo-structures`, `eo-street`, `eo-player`,
  `eo-props`): clearance, grade and end heights, meshes and the clearance
  envelope (`eo-structures`); the joins' junction legs, caps and raised and
  lowered stubs (`eo-street` `structure_joins`, `eo-structures` `joins`); a
  car driven through the MacArthur Tunnel (`eo-player` `tunnels`); the bores'
  lights (`eo-props` `tunnels`); the bore's extent (`eo-structures`
  `a_tunnels_bore_is_found_inside_its_walls_from_its_floor_to_its_roof`);
  a car in a bore marked with its depth and lit by the bore, not the sun
  (`eo-agents` `bores`, `eo-render` `agents`).
- **Lit shop windows** (`eo-sky` `shops`, `eo-render` `shop_lights`,
  `shop_lights_gpu`): the trades' hours (a block mixed at 21:00, bars lit to
  02:00), colours and luminances; the kit planner's shop bays light the
  commercial streets from behind their glass, on the kit's floors, alike on
  any thread count; the gather takes open shops, closed ones by their
  security light, none by day or past the reach; a crowded cluster keeps
  every street lamp before a shop light; an open shop lights the walk in
  front of its window, not behind it. Stage N2 (`shop_lights` unit,
  `shop_fronts`, `shop_rooms`): one vacancy rule (a vacant shop never lit
  but by its work light, a front the facade clutter boards up vacant and
  dark, every shop window finding its own shop's front in the rooms' table,
  on the real city with the clutter at level 2); the rooms' table found
  alike on the GPU and the CPU; every trade's room at the night exposure a
  lit room with things in it (its middle at mid tones, real contrast,
  fixtures blown out, little of the pane white), each trade its own, the
  same twice.
- **Frame budget** (`eo-app` `frame_budget`): the real bench, under the
  10 ms budget at the median on the standard paths, and no frame over the
  upload budget. It needs a GPU and a quiet machine; a loaded machine can
  fail it.

### Known failing tests

Three tests fail today. They are open defects, not flaky tests; do not
loosen them to make them pass.

| Test | What fails |
| --- | --- |
| `eo-server` `determinism::a_client_rewound_to_a_server_state_replays_to_the_same_place` | A client rewound to a server state does not replay to exactly the same place (rewind determinism) |
| `eo-street` `surface::street_surface_is_closed_and_on_its_section` | Two walkable surfaces fight in one place at more probes than the test allows |
| `eo-street` `surface::verge_banks_stay_off_the_paving` | Some verge banks reach onto the paving |

## Known issues

- The three failing tests above.
- Some content and behaviour of the earlier browser game is not ported yet:
  mantling and swimming, the car's full handling model, gamepad input in the
  window, and more. Building interiors are not drawn; their models were
  removed from the repository and remain in its Git history.
- Linux builds are untested. On macOS (Metal) the standard paths run at
  about 11 to 16 ms a frame at the median on an M-series GPU, over the 10 ms
  budget, almost all of it GPU time; the count emulation (above) can miss
  objects for the two or three frames between a list's count outrunning its
  bound and the read-back that sees it (the bench prints how often).
- Several systems are first versions and change often; switches and
  defaults may change between commits.
- The recorded visual-quality verdict is REJECT.

## Performance target

The target is a whole frame under **10 ms** at 1920 x 1080 on a desktop
GPU: the render thread's CPU work and the GPU's work together, with the
simulation, animation and streaming on their own threads, each also
finishing inside the frame period. The budget is split roughly as follows:

| Line | Budget (ms) |
| --- | ---: |
| Static world culling (frustum, LOD, occlusion) | 0.2 |
| Static world draw (buildings, terrain, streets) | 2.0 |
| Cars and props | 0.6 |
| Skinned people | 0.8 |
| Sun shadows | 1.5 |
| Sky, atmosphere, water | 0.6 |
| Post-processing | 1.5 |
| Interface | 0.2 |
| Render thread: encode and submit | 0.5 |
| Render thread: uploads | 0.4 |
| Main thread on the critical path | 0.5 |
| Reserve | 1.0 |

Check it with:

```sh
./target/release/eo-app --bench --check-max     # every path's median, p95 and worst frame under 10 ms
./target/release/eo-app --bench --check-budget  # every path's median under 10 ms
```

Timings are only meaningful on an otherwise idle machine: no build, no other
GPU program. A change that costs frame time should say how much, per bench
path, from matched runs with and without it (use the kill switches).

## Licences

The code is MIT licensed (see [LICENSE](../LICENSE)). The data and assets
the program reads keep their own licences, and the Rust dependencies are
permissively licensed; both are listed in
[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md). OpenStreetMap-derived
data is ODbL 1.0, which is share-alike for derived databases.
