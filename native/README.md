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
  Windows; Linux and macOS builds are untested.
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
bandwidth on a busy street and client prediction.

## Command-line switches

`eo-app --help` prints the main bench and streaming options. The most useful
switches are below; each mode (`--play`, `--ui`, `--connect`) rejects a
switch it does not take.

### Modes

| Switch | What |
| --- | --- |
| (none) | Windowed fly-through |
| `--play` | The player instead of the free camera |
| `--ui` | The interface shell: title, loading, HUD, pause menu |
| `--connect ADDR\|local` | Online client (with `--name`, `--spawn x,z,heading`) |
| `--bench` | Offscreen timing run over scripted camera paths; prints and writes JSON to `bench-out/` |
| `--bench --play` | Scripted walk and drive paths with the player |
| `--capture-ui SCREEN` | Writes a PNG of one screen: `title`, `loading`, `settings`, `pause`, `map`, `hud` or `all` |
| `--ui-bench` | Interface cost over the street walk and each full-screen menu |
| `--audio-render all --out DIR` | Offline sound mixes; no GPU, no audio device |

### World and view

| Switch | What |
| --- | --- |
| `--time-of-day H` | Hour of the day, 0 to 24 (default 12) |
| `--weather KIND` | `clear`, `overcast`, `fog`, `drizzle`, `rain`, `storm`, or `schedule` for the changing weather |
| `--weather-seed N`, `--world-time S` | Seed and clock of the weather schedule |
| `--moon-day D` | Day of the lunar cycle |
| `--sky-quality low\|medium\|high` | Cloud and atmosphere quality |
| `--agent-density N` | Multiply the traffic and pedestrian population (1 to 64) |
| `--width W --height H` | Frame size (default 1920 x 1080) |
| `--msaa 1\|4` | Multisampling |
| `--no-vsync` | Present without vertical sync |
| `--far M` | Camera far plane in metres (default 4200) |
| `--shadow-map N` | Sun shadow map size (default 3072) |
| `--cache DIR`, `--no-cache` | Cache folder, or cook everything fresh |

### Bench

| Switch | What |
| --- | --- |
| `--frames N`, `--warmup N` | Measured and warm-up frames per path (720 and 60) |
| `--segments a,b,...` | Which camera paths to run, for example `street_walk`, `elevated_sweep`, `city_overview`, `authored_street`, `kit_street`, `drive`, `turn_around`, `waterfront`, `bay` |
| `--capture` | Also write PNGs of each path to `bench-out/` |
| `--out DIR`, `--label NAME` | Output folder and run label |
| `--check-budget` | Exit 1 if a path's median frame reaches the 10 ms budget |
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
| People | `--no-rocketbox`, `--no-rocketbox-motions` (either one: no people) |
| Props | `--no-props2`, `--no-furniture`, `--no-eztree`, `--no-wires`, `--no-overhead`, `--no-lamp-sprites` |
| Buildings | `--no-facade-detail`, `--no-facade-age`, `--no-rooms`, `--no-room-frames`, `--no-room-dim`, `--no-roof-colours`, `--no-roof-plant`, `--no-roof-parapet`, `--no-roof-patches`, `--no-wall-paint` |
| Ground | `--no-ground-hills`, `--no-ground-green`, `--no-ground-contact`, `--no-terrain-lod` |
| Scanned surfaces | `--no-cc0` (all), `--no-cc0-street`, `--no-cc0-walls`, `--no-cc0-ground`, `--no-cc0-decals`, `--no-cc0-grime`, `--no-cc0-stains`, `--no-cc0-pom`, `--no-cc0-tiling` |
| Lighting and sky | `--no-shadows`, `--no-ao`, `--no-contact-shadows`, `--no-atmosphere`, `--no-fog`, `--no-clouds`, `--no-cloud-shadows`, `--no-shafts`, `--no-moon-stars`, `--no-lights` |
| Weather | `--no-rain`, `--no-wet`, `--no-wind` |
| Camera finish | `--no-taa`, `--no-bloom`, `--no-adaptation`, `--no-ssr` |
| Visibility and streaming | `--no-occlusion`, `--no-view-detail`, `--no-upload-ring`, `--no-submit-pacing`, `--no-indirect-count` |
| Interface and sound | `--no-hud`, `--no-audio`, `--audio-null` (mix to a null output) |

A new visual feature should come with its own switch, defaulting to on only
once it has been reviewed.

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
draws simple stand-ins for those props and other vehicles in traffic.

The source binaries of the asset pipelines (Blender scenes, vehicle and
character sources) are a separate, optional store; the game never reads
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
behind the glass, and roofs coloured from the measured aerial table.

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
entrances are fitted to the drawn street.

**Water.** A camera-following grid displaced by a cascaded FFT ocean
spectrum, masked by a citywide shore field, with its own optics per body of
water, screen-space reflection, surf on the mapped beaches and a shore
collar. `--no-water` draws a flat sea plane.

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
  server and client agree bit for bit.
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
  interiors, mantling and swimming, the car's full handling model, gamepad
  input in the window, and more.
- Linux and macOS builds are untested.
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
