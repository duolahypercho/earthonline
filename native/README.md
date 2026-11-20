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
| `eo-landmarks` | Bridges, the quay wall, piers, seawalls, plazas, landmark towers, the Ferry Building, the corridor's streetcar rails and quay railing, and the open-space ground as plain meshes. No GPU. |
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
| `--start-underwater` | Start the fly camera just beneath the Bay, looking up toward the skyline |
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
| `--segments a,b,...` | Which camera paths to run, for example `street_walk`, `elevated_sweep`, `city_overview`, `authored_street`, `kit_street`, `drive`, `turn_around`, `waterfront`, `bay`, `bay_dive` |
| `--capture` | Also write PNGs of each path to `bench-out/` |
| `--out DIR`, `--label NAME` | Output folder and run label |
| `--check-budget` | Exit 1 if a path's median frame reaches the 10 ms budget |
| `--check-max` | Exit 1 if any path's median, p95 or worst frame reaches the budget |
| `--worst-frames N` | How many of the worst frames to report |
| `--pace realtime\|60hz\|all-60hz\|none` | How the bench paces its frames |
| `--shots FILE.json --out DIR` | Offscreen stills instead of the timing: one PNG per shot of the file, and `DIR/<name>.json` beside it (below) |
| `--scene-cards --out DIR` | The eight scene cards of [the visual gate](../Docs/VISUAL_QUALITY_GATE.md) at the Ferry Building and The Embarcadero, 2560 x 1440 unless `--width`/`--height` are given (below) |
| `--clip FILE.json --out DIR` | Moving clips along camera paths: every frame drawn at 60 Hz of wall time, every `60 / fps`-th written as `DIR/<name>/NNNN.png` with `DIR/<name>.json`; each clip is `{"name", "seconds", "fps", "time_of_day", "fov", "keys": [{"t", "eye", "look_at"}]}` in game metres (`bench/clip.rs`) |
| `--shots-only a,b` | Only the shots or cards whose name contains `a` or `b` |
| `--settle-timeout S` | The most a shot waits for streaming to settle (600 s) |
| `--card8-seconds S`, `--card8-fps N`, `--card8-speed M` | Card 8's length (30 s), frames written per second (10) and speed (the player's run, 5.6 m/s) |

Neither needs `--bench`. Every world switch and kill switch applies, and
`--time-of-day` and `--weather` set the hour and weather of a shot that
does not give its own.

**Shots.** The file is a JSON array; each shot has a `name` (letters,
digits, `-`, `_`, `.`; it names the files) and one pose:

```json
[
  { "name": "transamerica_oblique",
    "geo": { "lat": 37.7952, "lon": -122.4028, "alt": 110, "range": 800, "heading": 30, "tilt": 75, "fov": 35 },
    "time_of_day": 13, "weather": "clear" },
  { "name": "embarcadero", "eye": [2830.9, 4.5, -2366.9], "yaw": 314, "pitch": -2, "fov": 52 },
  { "name": "salesforce", "eye": [2830.9, 4.5, -2366.9], "look_at": [2475.1, 160.0, -1627.9] }
]
```

- `geo` is the look-at pose of the common web-globe URL convention:
  (`lat`, `lon`, `alt`) is the target, `alt` in metres above mean sea level;
  the camera stands `range` metres from it; `heading` is the compass bearing
  the camera faces (0 north, 90 east); `tilt` is the angle from straight down
  (0 looks straight down, 90 at the horizon); `fov` is the vertical field of
  view in degrees. The camera is `target - range * forward` on the WGS84
  ellipsoid, `forward = (sin tilt sin heading, sin tilt cos heading, -cos tilt)`
  (east, north, up). The same keys at the top level of a shot are read as a
  `geo` pose too.
- `eye` with `yaw` and `pitch` (degrees; yaw 0 looks north, -z, and 90 east,
  +x; pitch up positive), or `eye` with `look_at`, are game metres (x east,
  y up, z south).
- Optional: `fov` (degrees, default 52, the game camera's), `far` (metres;
  default `--far` if given, else 20000 m so a view across the city is not
  cut short), `time_of_day`, `weather` (`clear`, `overcast`, `fog`,
  `drizzle`, `rain`, `storm`), `what`, and keys starting with `_` (comments).
  Any other key is an error.

Latitude, longitude and height go to game coordinates through
`eo_data::geo`: the city's projection (`sf-city.json` `meta`: centre and
metres per degree) times the ground-plan scale (below), and the height as
height above the surveyed ground kept over the drawn ground (the elevation
grid is NAVD88; mean sea level is game y -0.55; the drawn ground lifts
heights above mean sea level by the height scale, and buildings and
landmarks stand on it at their real size). Both the target and the camera
are converted and the game camera looks at the target, so the bearing is
kept and the tilt and distance change with the plan scale; the manifest
records both.

Before each PNG is written the hour and weather are set with the cloud and
water clocks held still, the traffic is rebuilt round the shot from the
simulation's seeded warm-up and stepped 2 s, streaming settles for that view
(the upload queue empty and every tile, model, far cell and prop the view
wants resident, up to `--settle-timeout`), and the temporal resolve is drawn
to convergence under the still camera (two frames one jitter cycle apart
under 0.5 RMS in 8-bit levels, or a difference that has stopped changing
over three comparisons: what is left changes every frame, such as the post
pass's display dither; 48 to 320 frames). `DIR/<name>.json` records
the input, the resolved eye and orientation in game coordinates (and the
eye's latitude, longitude and height), the commit the binary was built from
and the checkout's state, the full command line, the size, the hour, the
weather, every switch, the traffic, how the settle and the convergence went,
and notes (an eye inside a building or under the ground, a view against a
wall, a settle that timed out).

**Scene cards.** `--scene-cards` takes the gate's eight cards at the Ferry
Building / Embarcadero corridor, each placed from the data (the street
plan's runs and junctions by name, the shore field, the Ferry Building's
anchor): 1 Market Street's footway at noon; 2 the painted junction on The
Embarcadero where Market Street meets it, across the crosswalk; 3 California
Street among the towers at 18.5 h; 4 the Embarcadero promenade over the bay;
5 The Embarcadero by the Ferry Building in drizzle at 16 h; 6 card 1's
street at 21 h; 7 a third-person character standing, walking toward and
stopped at The Embarcadero's kerb (7a, 7b, 7c) with the on-foot camera rig;
8 the promenade south-east of the Ferry Building at the player's run,
rendered every frame at 60 Hz of wall time with streaming and traffic
running, written as `DIR/card8-traversal/0000.png`, ... and placed so a
128 m tile boundary is crossed halfway (the frames that cross it are in
`card8-traversal.json`). Each card's manifest is the shots manifest plus
the card's number, category and place, and `honoured: false` with the
reason when the run cannot give what the card needs (no people without the
optional people assets, so no pedestrians and no character; no traffic with
`--no-agents`).

```sh
EO_NATIVE_CACHE=... ./target/release/eo-app --shots views.json --out shots-out --width 1600 --height 900 --time-of-day 13 --weather clear
EO_NATIVE_CACHE=... ./target/release/eo-app --scene-cards --out cards-out
```

### Kill switches

Every visual feature can be turned off from the command line, so a change
can be compared with and without it (A/B) and a fault can be isolated. A
selection:

| Area | Switches |
| --- | --- |
| Content | `--no-models`, `--no-kit`, `--no-streets`, `--no-props`, `--no-agents`, `--no-landmarks`, `--no-water`, `--no-farshore`, `--no-sky` |
| Water optics | `--water-optics` enables the preview, `--no-water-optics` restores the original water shading |
| People | `--no-rocketbox`, `--no-rocketbox-motions` (either one: no people) |
| Props | `--no-props2`, `--no-furniture`, `--no-eztree`, `--no-wires`, `--no-overhead`, `--no-lamp-sprites`, `--no-furniture-models` (a furniture kind whose GLB is not in the checkout draws as a 0.3 m box at its height, not its model drawn in code) |
| Bridges | `--no-bridges2` (all of these), `--no-bridge-scale`, `--no-bridge-paint`, `--no-bridge-detail`, `--no-bridge-wires`, `--no-bridge-night`; the second pass: `--no-bridges3` (all of its), `--no-bridge-air`, `--no-bridge-finish`, `--no-bridge-members`, `--no-bridge-floods`, `--no-bridge-curtain` |
| Buildings | `--no-facade-detail`, `--no-facade-age`, `--no-rooms`, `--no-room-frames`, `--no-room-dim`, `--no-roof-colours`, `--no-roof-plant`, `--no-roof-parapet`, `--no-roof-patches`, `--no-wall-paint` |
| Ground | `--no-ground-hills`, `--no-ground-green`, `--no-ground-contact`, `--no-terrain-lod` |
| Water | `--no-water` (the flat sea plane), `--no-water-instanced` (the pass needs multi-draw-indirect-count, as before; without it the flat sea plane), `--no-water-brdf` (the earlier Fresnel, roughness, sun lobe and reflection march), `--no-water-body` (the earlier body colour), `--no-water-foam-cover` (the earlier whitecap cover and foam sampling), `--no-water-glint` (the earlier hashed sparkle), `--no-water-turbid` (the earlier three-wavelength body), `--no-water-shore` (the earlier waterline, wall collar and the lift under piers), `--no-water-foam-lit` (the earlier whitecap shapes and foam light), `--no-water-ppr` (the reflection march alone), `--no-water-wind-texture` (no gust patches, tidal current or rotated chop), `--no-water-grazing` (one mean Fresnel times the sky in one direction) |
| Waterfront corridor | `--no-ferry-building` (its record's plain box instead), `--no-tram-tracks` (no streetcar rails), `--no-quay-railing` (no guard on the Embarcadero quay wall), `--no-ferry-building-detail` (the Ferry Building's first pass: no ashlar course scale, rustication, deep reveals, modillions, granite plinth, weathering or tower balustrades), `--no-rail-finish` (one bare-steel rail head and a black groove), `--no-corridor-palms` (no Embarcadero palms), `--no-guard-detail` (the quay guard's first pass, on the JS crest: no kick plate, solid pickets), `--no-pier-sheds` (the Ferry Plaza's JS shed boxes), `--no-rail-polish` (the rails' worn band whole at roughness 0.4, no burnished strip or sealant joints), `--no-facade-relief` (the Ferry Building's second pass: one glazing pane an opening, the second pass's reveal tone and dirt under the cornices, no fanlight frames or voussoir tones), `--no-palm-detail` (the corridor palms' first pass: no tree well, root initials, scar lattice, senescent leaves or spear, and unsurveyed palms at the median size) |
| Scanned surfaces | `--no-cc0` (all), `--no-cc0-street`, `--no-cc0-walls`, `--no-cc0-ground`, `--no-cc0-decals`, `--no-cc0-grime`, `--no-cc0-stains`, `--no-cc0-pom`, `--no-cc0-tiling` |
| Lighting and sky | `--no-shadows`, `--no-ao`, `--no-sky-vis`, `--no-contact-shadows`, `--no-atmosphere`, `--no-phys-sky`, `--no-ap-lut`, `--no-night-shaping`, `--no-fog`, `--no-clouds`, `--no-cloud-shadows`, `--no-shafts`, `--no-moon-stars`, `--no-lights` |
| Weather | `--no-rain`, `--no-wet`, `--no-wind` |
| Camera finish | `--no-taa`, `--no-bloom`, `--no-adaptation`, `--no-ev100`, `--no-filmic`, `--no-ssr` |
| Camera look and exposure | `--no-gg2-camera` (all three below), `--no-camera-look`, `--no-scene-meter`, `--no-lamp-photocell` |
| Daylight and sky | `--no-gg2-daylight` (all of these), `--no-ground-albedo`, `--no-photo-date`, `--no-clear-sky-state`, `--no-cirrus`; reserved, nothing reads them yet: `--no-terminator-bias`, `--no-spectral-sky` |
| Golden Gate, fourth pass | `--no-bridges4` (all of these), `--no-bridge-shade2`, `--no-bridge-dielectric`, `--no-bridge-haunch`, `--no-bridge-bracing`, `--no-bridge-pylons`, `--no-bridge-arch2`, `--no-bridge-fender2`, `--no-bridge-crest`, `--no-bridge-collar` |
| Strait water | `--no-gg2-water` (all of these), `--no-water-exposure`, `--no-water-current`, `--no-water-lean`, `--no-water-ppr-blur`, `--no-beach-profiles`, `--no-water-streak-caps`, `--no-water-far-calm`, `--no-baker-surf`; reserved: `--no-swash-line` |
| Shore ground | `--no-gg2-ground` (all of these), `--no-beach-sand-limit`, `--no-sand-albedo`, `--no-rocks`, `--no-riprap`, `--no-pier-lines`, `--no-season-seam`; reserved: `--no-ground-steps` |
| Far layer | `--no-gg2-far` (all of these), `--no-far-fallback`, `--no-landmark-far-models`, `--no-far-tree-density`, `--no-presidio-rule`; reserved: `--no-far-paint` |
| Corridor traffic | `--no-corridor-traffic` (the whole layer), `--no-deck-walkers` |
| Coastal vegetation | `--no-gg2-vegetation` (all of these), `--no-coastal-trees`, `--no-ground-cover`, `--no-hill-rock` |
| Round 3, all tracks | `--no-gg3` turns every round-3 track's roll-up on (restores round 2's output) |
| Frame budget | `--no-gg3-perf` (all of these), `--no-draw-ranges` |
| Photo dates and sky | `--no-gg3-sky` (all of these), `--no-photo-season`, `--no-cirrus2`, `--no-sky-day-fade`, `--no-live-calendar` |
| Colour and camera | `--no-gg3-colour` (all of these), `--no-sun-spectrum2`, `--no-meter-highlights`; reserved: `--no-sky-calibration`, `--no-camera-style`, `--no-haze-visibility` |
| Terrain surface | `--no-gg3-ground` (all of these), `--no-facet-normal-fix`, `--no-terrain-detail`, `--no-terrain-materials`, `--no-sand-detail2`; reserved: `--no-bluff-soil`, `--no-bank-grade` |
| Scanned rock | `--no-gg3-rock` (all of these), `--no-scan-rocks`, `--no-cliff-dressing`, `--no-surf-rocks`, `--no-scan-riprap`, `--no-rock-geology` |
| Coastal vegetation, round 3 | `--no-gg3-vegetation` (all of these), `--no-naip-planting`, `--no-cliff-rock-cover`, `--no-far-species`, `--no-far-tree-light`, `--no-bloom-date`, `--no-grass-color-variety`; reserved: `--no-veg-scans` |
| Grass, second pass | `--no-grass2` (all of these: run 1's meadow), `--no-meadow-cover`, `--no-meadow-types`, `--no-blade-shading` |
| Surf and bathymetry | `--no-gg3-water` (all of these), `--no-bathymetry`, `--no-surf-break`, `--no-swash-foam`, `--no-obstacle-foam`, `--no-water-body2`; reserved: `--no-ppr-dash` |
| Golden Gate, fifth pass | `--no-gg3-bridge` or `--no-bridges5` (all of these), `--no-paint-brdf2`, `--no-paint-weathering`, `--no-concrete2`, `--no-leg-cells`, `--no-cable-detail` |
| Visibility and streaming | `--no-occlusion`, `--no-view-detail`, `--no-upload-ring`, `--no-submit-pacing`, `--no-indirect-count` |
| Interface and sound | `--no-hud`, `--no-audio`, `--audio-null` (mix to a null output) |

A new visual feature should come with its own switch, defaulting to on only
once it has been reviewed.

The lighting switches: `--no-phys-sky` keeps the browser game's light rig,
aerosol and dome anchor in place of the physical daylight (the sun, the sky
and the ground's light from the clear coastal atmosphere in `eo-look`
`atmosphere` and `phys`, the sky drawn at the sun's own scale, the city's
reflections read from the sky tables); `--no-ap-lut` keeps the haze toward
the horizon's colour in place of the aerial-perspective volume; `--no-filmic`
keeps the ACES fit with the post chain's shoulder, grade and finish in place
of the filmic curve, the daylight white balance and the chroma gain;
`--no-ev100` keeps the JS exposure curve in place of the incident meter's
EV100; `--no-sky-vis` keeps the occlusion pass's 4 m contact term in place of
the canyon-scale sky visibility and wall bounce; `--no-night-shaping` keeps
the rig's rim light on at night (without it an unlit wall at night is lit by
the night sky's fill and the practicals alone). All six together give the
previous output.

The camera switches. `--no-camera-look` keeps the filmic curve, its chroma
gain and the incident meter's +0.85 EV offset in place of the camera look.
The camera look is the daylight white balance, the ISO 12232 SOS placement
(the meter's grey at scene 0.18, sRGB 118 at 0.2215) and the ACES 2.0 output
transform for a 100-nit sRGB display, baked into a 65^3 table (eo-look
`tone::camera`); the submerged camera (`--water-optics`) finishes through
the same table. `--no-filmic` still keeps the older ACES-fit chain and turns
the camera look off with it. `--no-scene-meter` keeps the dome-estimate meter
in place of the reflected-light meter on the drawn frame: a
quarter-resolution, centre-weighted luminance histogram, K = 12.5, within 2
stops of the incident reading, holding the top 2 % under the shoulder.
`--no-lamp-photocell` keeps the sun-altitude lamp ramp in place of
street-lamp photocells (ANSI C136.10 levels, 1.5 +- 0.5 fc on, at most 1.5x
off) on the twilight illuminance. `--no-gg2-camera` turns all three off and
gives the previous output.

The daylight switches. A clear day's sky is cloudless, cirrus-only or fair
cumulus, drawn per game day of the world clock from San Francisco airport's
2014-2023 METAR shares for the month of the sun's date (the table is
`eo-look/data/ksfo-clear-day-sky.csv`, generated by `ksfo-clear-day-sky.py`
from the query it records; input through `EO_KSFO_METAR_CSV`,
digest-checked). The state fades with the weather's clear weight
(1 - smoothstep(0.1, 0.3, overcast)), with no jump. `--no-clear-sky-state`
keeps the fair-weather sky on every clear day; `--no-cirrus` keeps the 5.2 km
sheet in place of fibrous ice cloud at 9.5 km; `--no-ground-albedo` keeps the
city's ground albedo under the sky light everywhere in place of the land
cover's round the eye; `--no-photo-date` makes a shot ignore its
`day_of_year`. Shots can hold the sky (`sky`), the date (`day_of_year`) and
the game day (`game_day`). `--no-gg2-daylight` turns them all off.

The Golden Gate's fourth pass (`--no-bridges4`, read by eo-landmarks
`suspension`): `--no-bridge-shade2` keeps the JS's dark bake on the struts,
fins, bracing and floor beams and the baked occlusion in place of one paint
and the sky the deck and the opposite leg hide; `--no-bridge-dielectric`
adds the sea's light on top of the environment's ground share instead of
replacing it; `--no-bridge-haunch`, `--no-bridge-bracing`,
`--no-bridge-pylons`, `--no-bridge-arch2`, `--no-bridge-fender2` and
`--no-bridge-collar` leave out the portal struts' casings and haunches, the
bracing under the roadway, the pylons S1, S2, N1 and N2 with the tie-downs
and anchorage housings, the Fort Point arch's laced ribs, the south fender
at its published size, and the truss closed at the tower legs;
`--no-bridge-crest` keeps the roadway's sag in place of its surveyed crest at
midspan (the corridor traffic follows the deck either way).

The water switches (`--no-gg2-water`, read by eo-water `switches`):
`--no-water-exposure` leaves out the exposure field and the strait's
measured wind floor; `--no-water-current` the tidal current on the game's
clock; `--no-water-lean` the anisotropic cascade fade and sub-pixel slope
variance; `--no-water-ppr-blur` the slope-blurred projected reflection;
`--no-beach-profiles` each beach's own foreshore slope and turbid bottom;
`--no-water-streak-caps` the measured afternoon wind and streaked
whitecaps; `--no-water-far-calm` the far calm and current lines;
`--no-baker-surf` the open coast's surf.

The shore-ground switches (`--no-gg2-ground`, read by eo-landmarks
`features::shore_ground`): `--no-beach-sand-limit` lets the beach sand run
past the berm and the cliff toe; `--no-sand-albedo` keeps the earlier sand
tone in place of measured dry and wet sand, the tide-keyed damp band, wrack
and footprints; `--no-rocks` leaves out the boulders; `--no-riprap` the
quarried-block revetment and the Fort Point seawall's coursed granite;
`--no-pier-lines` the decks built from the citywide map's pier lines
(Torpedo Wharf, the Coast Guard walkway); `--no-season-seam` keeps the city
terrain and the far layer on their own dryness curves.

The far-layer switches (`--no-gg2-far`, eo-render `far_city`):
`--no-far-fallback` keeps the far layer off where the adapter lacks
multi-draw-indirect-count; `--no-landmark-far-models` lets Fort Point's
authored model leave residency at its old range; `--no-far-tree-density`
keeps the old far park tree sampling; `--no-presidio-rule` keeps the
Presidio's buildings in the generic palette instead of off-white walls and
red tile roofs.

The corridor traffic (eo-app `agents::corridor`): through traffic on US-101,
CA-1 and Marina Boulevard and walkers on the bridge's east walk, from a flow
layer on the world clock (eo-sim `corridor`) at the published counts, with
the driver handbook's three-second gap. `--no-corridor-traffic` leaves the
whole layer out; `--no-deck-walkers` the walkers alone.

The coastal vegetation (eo-landmarks `vegetation`): coastal tree stands,
ground cover and hill rock on the props tiles. `--no-coastal-trees` keeps the
park species in the mapped woods; `--no-ground-cover` leaves out the grass,
tufts, flowers and scrub; `--no-hill-rock` the rock outcrops.
`--no-gg2-vegetation` turns all three off.

The round-3 switches. `--no-gg3` turns every round-3 track off at once and
gives round 2's output; each track also has its own roll-up.

The frame budget (`--no-gg3-perf`, eo-render `pace`): `--no-draw-ranges`. On
an adapter without multi-draw-indirect-count, the camera's opaque and glass
lists and each refreshed shadow cascade keep one slot per object, and the
cull pass writes instance count 0 or 1 into each slot. By default only the
blocks of 64 slots that this frame's cull can fill are issued, in slot
order. The renderer keeps a CPU copy of the object table and the far layer's
residency to decide this; a block written in the last 8 frames is issued
whole. The frame is the same either way. With `--no-draw-ranges` every slot
is issued as before: about 640 k Metal draws a frame and 26 ms of
render-thread time on golden_gate, plus wgpu's indirect-validation pass
before the water pass. The bench also reports `fixed_slot_lists` (slots
issued per frame), `gpu_serial_ms` (each pass's end minus the previous
pass's end, which adds up to the frame; begin-to-end pass windows overlap
on Apple GPUs), the PPR and scene-meter passes, and a budget verdict of
"not run" when no standard path ran.

The photo-date sky (`--no-gg3-sky`, eo-look `season`): `--no-photo-season`
keeps round 2's season curve, read once at load, instead of the cited
grassland phenology the far layer reads every frame; `--no-cirrus2` keeps
round 2's one high deck and its streaks instead of the second cirrus deck,
which takes its METAR amount (FEW/SCT/BKN from KSFO, or a shot's
`sky_oktas`) and covers that share of the plane at every mip;
`--no-sky-day-fade` lets a clear day's sky flip at the game day boundary
instead of giving way to the next day's over the world clock's 21-24 h;
`--no-live-calendar` keeps the date on 22 September instead of advancing one
day each game day (the renderer's sky tells the calendar the world clock).

The colour and camera (`--no-gg3-colour`, eo-look `features`):
`--no-sun-spectrum2` draws the direct sun's colour from the atmosphere's
three channels instead of its spectrum; `--no-meter-highlights` restores the
scene meter's 2 % highlight hold and its centre weighting (by default the
meter averages the whole frame, ISO 2720's K = 12.5). `--no-sky-calibration`,
`--no-camera-style` and `--no-haze-visibility` are reserved and change
nothing. `EO_LOOK_DEBUG=10..17` (also read as `EO_AO_DEBUG`) draws one term
of a surface's scene radiance, log2-encoded over 2^-14..2^4: 10 all, 11 sun
diffuse, 12 sun specular, 13 sky diffuse, 14 sky specular, 15 bounce, 16
none (aerial in-scatter only), 17 unit white; decode with eo-look
`probe::decode_level`. (The colour track's ACES 1.3 option is not in this
build until its licence text is approved.)

The terrain surface (`--no-gg3-ground`, eo-landmarks `features::gg3_ground`;
the survey readers need `EO_SURVEY_ROOT`, and without it the ground draws as
round 2): `--no-facet-normal-fix` keeps round 2's two-sided flip of the
interpolated normal instead of orienting it by the triangle and bending it
toward the eye's horizon; `--no-terrain-detail` leaves out the 1-2 m relief
from the 1 m DEM (published through eo-data `detail_ground`, which the
plants and rocks sit on); `--no-terrain-materials` the ground materials on
the NAIP land cover; `--no-sand-detail2` keys the damp band, wrack and
footprints to round 2's constant slope instead of each beach's measured
foreshore. `--no-bluff-soil` and `--no-bank-grade` are reserved.

The scanned rock (`--no-gg3-rock`, eo-landmarks `rocks::scan`; the Poly Haven
scans under `EO_SURVEY_ROOT`, cooked under `EO_NATIVE_CACHE`): `--no-scan-rocks`
leaves out every scan (round 2's procedural rock); `--no-cliff-dressing` the
cliff band's rock faces; `--no-surf-rocks` the intertidal rock field from the
imagery and the bed; `--no-scan-riprap` the
revetment's armour as scanned stone; `--no-rock-geology` keeps round 2's
colours instead of tints from the mapped geologic units.

The coastal vegetation, round 3 (`--no-gg3-vegetation`, eo-landmarks
`vegetation::Gg3`): `--no-naip-planting` keeps round 2's placement instead of
scrub, trees and grass placed from the 0.6 m NAIP image; `--no-cliff-rock-cover`
lets plants stand on a cliff over a beach, mapped bare rock or a sand cliff;
`--no-far-species` keeps round 2's far tree forms instead of the near tier's
coastal species; `--no-far-tree-light` keeps round 2's far card light instead
of a leaf volume with transmission; `--no-bloom-date` leaves out the cited
bloom window and the flower heads' own colour; `--no-grass-color-variety`
keeps every grass bunch in the ground's tone (no meadow mosaic of green and
cured patches by aspect, slope and hollow, no grey-green or brown patches, no
lighter seed heads or flower-tinted tips, no olive to grey-green scrub).
`--no-veg-scans` is reserved.

The second grass pass (`--no-grass2`, eo-landmarks `vegetation::Grass2`):
`--no-meadow-cover` keeps the scattered bunches and tufts instead of a meadow
patch in every 1 m cell of grass, scrub and (inside the NAIP image) the bare
class, each a matted thatch and short-sward mat with blades over it, and keeps
the soil the terrain materials draw on the bare class; `--no-meadow-types`
makes every patch short sward (no wild oats, fescue tussocks, clover and
forbs, ice plant or thatch patches by slope, aspect, bluff edge and scrub, and
no low scrub mounds in the scrub's gaps); `--no-blade-shading` leaves out the
blades' leaf optics (transmission, keel and rolled-margin normals, clump
occlusion, waxy sheen).

The surf and bathymetry (`--no-gg3-water`, eo-water `switches`; the CUDEM
tiles under `EO_SURVEY_ROOT`): `--no-bathymetry` keeps the geometric exposure
instead of the measured sea bed and the swell refracted and shoaled over it;
`--no-surf-break` the surf breaking where the depth says; `--no-swash-foam`
the bay beaches' thin swash line; `--no-obstacle-foam` the white water at
rocks, the fender and walls; `--no-water-body2` keeps round 2's constant
grey in the reflected sky. `--no-ppr-dash` is reserved. `--no-gg2-water`
turns these off too.

The Golden Gate's fifth pass (`--no-gg3-bridge` or `--no-bridges5`, eo-landmarks
`suspension`): `--no-paint-brdf2` keeps the fourth pass's paint instead of
the District's named colour match as a dielectric film; `--no-paint-weathering`
the chalking, touch-up patches, rust bloom at rivet rows and dirt on ledges;
`--no-concrete2` the pylons', housings', piers' and fender's weathered
concrete with its pour lifts and zero-mean staining; `--no-leg-cells` the
tower legs' cellular section and the portal casings' own shadow;
`--no-cable-detail` the cable bands, four-rope suspender groups, spacing
clamps and the lamp standards at the published 150 ft pitch.

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
| `EO_SURVEY_ROOT` | The Golden Gate survey set: NAIP 0.6 m aerial tiles, USGS 3DEP 1 m DEMs, NOAA CUDEM topobathy, Poly Haven scanned rocks and plants (`assets/survey-manifest.json`) | The game (`eo_farshore::survey`, read by nothing yet), `tools/survey-manifest.mjs` |
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
collar. Its patches are one instanced indirect draw, so it draws on adapters
without multi-draw-indirect-count too. The slope variance a pixel does not
resolve goes to the BRDF (the Cox-Munk total for the wind kept at every
range), with the Fresnel averaged over the visible facets and a Beckmann sun
lobe; the body's colour is the water-leaving reflectance of the bay's and
the coast's optical properties under the city's own irradiance
(`eo-water` `optics`, integrated over the visible spectrum for the bay's
sediment); whitecaps cover what the Monahan and O'Muircheartaigh fit gives,
drawn as lit foam patches that dissolve as they age. The sun's glints are a
per-pixel count of the facets that mirror its disc, so the glitter path
tapers by itself; the sky reflection is integrated over the facets the eye
sees; the frame's own geometry is projected across the sea plane
(pixel-projected reflection) for decks, cables and the skyline. At the shore
the beach sheet fades into the sand, a seawall sends the chop back
(clapotis), and piers on piles stand over the sea (`eo-water` `field`
`PIERS_OVER_WATER`). `--no-water` draws a flat sea plane.

Run `./target/release/eo-app --water-optics --start-underwater` for the
submerged fly camera. Hold Ctrl for fine movement, E to surface and Q to
descend. The preview requires the compact water pass and single-sample
rendering (`--msaa 1`). It uses the displaced wave surface, refraction at the
waterline, depth-dependent scattering and absorption, and moving light on
shallow receivers. Visibility follows the Bay and Pacific water properties.
Above the surface the preview integrates unresolved sun glitter over the
pixel. The `bay_dive` bench path crosses the surface in both directions.

**Sky and weather.** Analytic sky and atmosphere, volumetric clouds and the
marine layer, a weather schedule (clear to storm) with rain, wet ground and
drying, sun shadows in cascades, moon and stars, and night lighting: street
lamps, vehicle lamps and lit windows. The camera finish is temporal
anti-aliasing, bloom, exposure adaptation, tone mapping and a grade.

**Landmarks.** The two bridges, the quay wall and piers, seawalls, plazas,
landmark towers and the bridges' lights are generated by `eo-landmarks`. The
bridges' heights and members follow the drawn terrain's height scale, so
each keeps its published proportions on the wider ground plan; they wear
their cited paint (International Orange, the west span's aluminium grey);
the Golden Gate towers carry their notched legs and ribbed portal struts and
its deck its painted steel underside; the cables and suspenders are drawn
after the atmosphere at every distance, anti-aliased at a screen-space floor
width; at night the Golden Gate towers are floodlit and the west span's
suspenders carry their LEDs (`eo_landmarks::suspension::BridgeFeatures`).
The second pass gives the wires the aerial perspective and a hairline's
mean half-section shading (`--no-bridge-air`); paints the coats as
dielectric films at a low sheen, lights the members' undersides with the
sea's reflected light, shadows what stands under a deck from the sun past
the shadow maps' reach and eases the baked occlusion (`--no-bridge-finish`);
adds the cable saddles' housings, cable bands and sockets, the Fort Point
arch's open web, the pylons' pilasters, the south pier's fender, the lamp
standards and the west span's floor system and laced tower bracing
(`--no-bridge-members`); lights the Golden Gate towers by their placed
floods above and below the roadway and plays the suspender LEDs in bands
(`--no-bridge-floods`); and veils a receding suspender curtain at its mean
cover (`--no-bridge-curtain`).

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
