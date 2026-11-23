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
- [Optional assets](#optional-assets)
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

- `assets/` holds small tracked inputs: manifests of the [optional
  assets](#optional-assets), the sound credits, the roof-colour and ground
  tables, the fallback font, and the placement modules of the hand-built
  buildings.
- `tools/` holds the Node scripts that made the optional assets'
  provenance manifests from their sources. Inside the repository they
  download only into `external/<set>`; the manifests they rewrite are the
  tracked ones in `assets/` and `eo-props/data/`.
- `*/data/` folders hold reference answers and tables that the crates and
  their tests read. Some were exported from the earlier browser
  implementation of the game, whose exporters are no longer in this
  repository; treat them as fixed test fixtures.
- `bench-out/`, `cache/` and `target/` are generated per machine and
  ignored by Git.

## Requirements

- A stable Rust toolchain that supports edition 2024 and let chains (Rust
  1.88 or newer), installed with [rustup](https://rustup.rs).
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
- Python 3 (its standard library only) for `scripts/setup-assets.py`, the
  one-time download of the [optional assets](#optional-assets) into
  `external/` at the repository root (about 2 GB more disk).
- Node.js only if you use the scripts in `tools/` or the asset-store scripts
  at the repository root.

## Build and run

First-time setup, once, from the repository root: download the optional
assets (people, motions, scanned surfaces, sound, sign art, far-shore
elevation; about 2 GB) into the git-ignored `external/` folder. Run it again
after a pull that changes a manifest; files already present and verified
are skipped, and an interrupted run resumes.

```sh
python3 scripts/setup-assets.py
```

The game runs without them too, with those features off (see
[Optional assets](#optional-assets)). Run every other command from
`native/`.

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
| E | At a landmark's lift stop: ride up to its deck, or back down | |
| V | Place a new car ahead (a test helper) | |
| P | Photo mode (below) | Photo mode |
| Esc | Release the mouse; press again to quit | |

The title bar shows the frame rate, and in `--play` the mode, speed and the
player step time; at a lift stop it starts with the lift's prompt (in
`--connect` the prompt is the HUD's).

**Photo mode** (`--play` and `--connect`, P; `eo-app/src/photo.rs`). The
player's input is frozen (in `--play` the local world holds still too; in
`--connect` the server's simulation runs on). A free camera flies or orbits
within 30 m of the player, swept as a ball against the collision world: mouse
look, W A S D fly (orbit: W S nearer and farther), Q E down and up, Shift
faster, Ctrl slower, O orbit or fly. The lens runs 14 to 200 mm (35 mm
equivalent: the field of view across the 36 mm width; the wheel), Z X roll.
The panel has three pages (Tab: Camera, Look, Frame; Up and Down choose a
setting, Left and Right change it, H hides it; it fades back while the camera
moves and docks left or right). Camera: focus (on your eyes, at an AF point,
the centre or a rule-of-thirds crossing, F, or a distance; the depth of
field's near and far limits are shown, a reticle marks where the focus is
taken and G turns focus peaking on, never saved), aperture (f/1.4 to f/22,
or off) and the iris (9 rounded blades, 7 or 6 straight blades, an ideal
circle). Look: exposure compensation (plus or minus 3 EV), a look (Neutral;
Warm film, an 81A warming filter; Cool, an 82B cooling filter; Monochrome,
Tri-X 400 through a yellow, deep yellow, orange, red or green contrast
filter) at a strength, fade, vignetting and grain. Frame: a letterbox
(2.39:1, 1.85:1, 4:3, 1:1) whose opening can be moved up or down, the
rule-of-thirds guides, a local time of day (only you see it), the capture
size and the panel's side. Enter saves a PNG at the window's size or rendered
at twice it, with its settings beside it, into the photos folder
(`EO_PHOTOS_DIR`, else `photos` in the application-data folder). The HUD is
hidden while photo mode is on; P or Esc leaves. The game's own keys (E, R,
V, F and the rest) do nothing while it is on. The depth of field and finish
are `eo-render/src/photo_mode.rs`; post's display (the filmic display or the
camera look, `--blender-camera`, `--photo-finish`, `--look cinematic`) takes
its output in place of the frame, as before, and the frame's bloom is added
in photo mode's finish instead of in post.

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

### Packaged builds

`scripts/release/package.sh <label> [out-dir]` (after
`cargo build --release -p eo-app -p eo-server`) packs a playable copy into
`dist/EarthOnline-<version>-<label>.zip`: `eo-app`, `eo-server`, the
platform's launcher from `scripts/release/launchers/` (it starts `--play`),
[Docs/PLAYING.md](../Docs/PLAYING.md), and the files tracked at `HEAD` that
the game reads at run time: `public/`, `native/assets/`, each crate's
`data/`, the street-standards table and the licences. The folder keeps the
repository's layout. A packaged build finds its files as a checkout does:
the crates' tables, `native/assets/` and the model cache are under
`eo_data::native_dir()`, which is the `native/` folder the binary was built
in while that folder exists, and otherwise the `native/` folder beside the
data root found from the working directory or the executable. The cache is
`native/cache/` inside the game's folder unless `EO_NATIVE_CACHE` names
another. Optional off-repository assets are never packed.

[`.github/workflows/release.yml`](../.github/workflows/release.yml) builds
and packs Windows x64, macOS arm64 and Linux x64 for every pull request into
`main` that touches the game (the zips are the run's artifacts for a week),
and for a pushed `v*` tag publishes them as a GitHub Release:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The builds are not code-signed; PLAYING.md tells players how to open them.

Presence at landmarks (`eo_net::presence`): the server owns the hour, how
fast it runs and the weather, and every snapshot carries them (a client
eases toward the server's clock, so friends see the same sunset;
`--time-of-day`, `--day-rate` and `--weather` set a local server's).
Other players are drawn as Rocketbox people in their account's body,
moved from their replicated speed and state (walk; stop with the feet
closing together; step through a turn on the spot; emotes), on a smooth
path through the snapshots, and no street person is drawn in a player's
body. The emotes are edited at the bake (`eo_agents::pose_edit`): a
one-arm wave with the forearm swinging, a point with the index finger out
and the head turned along the arm, a selfie with a phone in the hand
(`eo_agents::rig`), and a crouch. The ground under each body darkens
where the body takes light from it (`eo_agents::rig::contacts`): each
shoe, each shin and thigh and a crouch's seat is a box, and a patch on
the ground around it takes away the share of the sky the box hides (its
exact view factor) and, a few centimetres from a sole, the sun it blocks,
out of the light the renderer puts there (eo-render `agents_draw.wgsl`
`ag_contact`, in the agents' glass pass; none in a tunnel's bore, where the
bore's own light draws the body). Name tags hang over each head as it is
drawn, fade with distance (opaque to 12 m, 0.35 at 25 m, gone at 40 m),
hide behind buildings and, from a GPU depth probe of the frame, behind
anything nearer than the head (a passer-by, a pole, another player), and
step aside or up with a leader line rather than cover another tag or a
face. In the window, 1 to 4 wave, point, crouch and take a selfie, 0
stops, and T fast-travels to the next landmark viewpoint
(`assets/landmark-viewpoints.json`; the server checks the request and
lands the player there half a second later).

```sh
./target/release/eo-server bots --connect 127.0.0.1:47800 --count 12 --landmarks   # bots that walk up to the viewpoints, emote, travel on
./target/release/eo-app --connect local --net-shots scenes.json --out DIR --width 1600 --height 900 [--pair-before DIR2]   # landmark captures (eo-app/src/net.rs); DIR2: the same instants with --no-remote-bodies --no-name-tags
```

## Command-line switches

`eo-app --help` prints the main bench and streaming options. The most useful
switches are below; each mode (`--play`, `--ui`, `--connect`) rejects a
switch it does not take.

### Modes

| Switch | What |
| --- | --- |
| (none) | Windowed fly-through |
| `--start-underwater` | Start the fly camera just beneath the Bay, looking up toward the skyline |
| `--beaver-house` / `--no-beaver-house` | Opt in to the furnished three-storey terrace at 21 Beaver Street (off by default). With `--play --beaver-house`, start outside the shut front door. Press E at the handle to open or close it from either side. Walk up the stairs to the bedroom and press R beside the bed to wake at 07:00. The top floor is a workroom. The [Blender builder](../scripts/blender-buildings/build_beaver_terrace.py) accepts `-- --blend PATH --parts DIR` to save the editable scene and individual house and furniture GLBs outside the checkout. |
| `--house-finishes` / `--no-house-finishes` | With `--beaver-house`, use the Southern California courtyard design at the same Beaver Street address. It has a private path with clay-tiled steps and a planted gravel bed, a deep arched entry, a rounded stair bay, clay-tile roofs, pale photographed floorboards and locally reflected kitchen lighting. The architectural reference is the [SurveyLA Mediterranean and Indigenous Revival survey](https://planning.lacity.gov/odocument/097f6db5-feee-43f5-a448-fd140763de90/MediterraneanandIndigenousRevivalArchitecture_1893-1948.pdf). Rebuild the [floor scan atlas](../scripts/blender-buildings/prepare_beaver_floor.py), run the Blender builder with `-- --house-finishes` and the desired scene/component output paths, then run [the Blender reflection baker](../scripts/blender-buildings/bake_beaver_reflections.py) against the exported house. |
| `--house-patina` / `--no-house-patina` | With `--beaver-house`, opt in to the furnished courtyard with scan-derived plaster, gravel, fired clay, painted timber and linen. Adds a trained bougainvillea, a wrought-iron stair rail, casement sashes, projecting sills and rainwater fittings. The front glass reflects day and night captures of the actual block. The upstairs linen quilt is settled against a mattress in Blender and exported at rest. Turning it off with `--house-finishes` keeps the previous courtyard. The [surface builder](../scripts/blender-buildings/prepare_beaver_surfaces.py) verifies its photographic inputs from an external cache. Pass `-- --house-patina` to the Blender house builder and kitchen reflection baker. The [glazing baker](../scripts/blender-buildings/bake_beaver_glazing.py) takes an external capture directory and `--capture` to regenerate the two street panoramas. |
| `--house-furniture` / `--no-house-furniture` | Opt in to the furnished 21 Beaver house with sewn sofa cushions, a wool carpet, a six-seat dining table, a sideboard, and a wood coffee table. Furniture and its collision use physical metres inside the surveyed footprint. The indoor camera follows the standing or crouching character’s eye height in play mode. Use `--no-house-eye-camera` to retain the chase camera. An `EO_CAPTURE_POSES` entry can include `"player": {"feet": [x, y, z], "yaw": 0}` to pose the loaded game avatar as a scale reference. Reuse the previous sitting room with `--house-patina --no-house-furniture`. The [material builder](../scripts/blender-buildings/prepare_beaver_furniture.py) verifies the rug photograph and timber scans. Pass `-- --house-furniture` to the house builder and kitchen reflection baker. Run [the furniture exporter](../scripts/blender-buildings/export_beaver_furniture_parts.py) against the saved Blender file to export the fifteen loose furniture and prop GLBs and the hinge-pivoted front door with embedded materials. The closed door measures 0.96 × 2.05 m on site and opens with E from the recessed landing or hall. |
| `--house-photo` / `--no-house-photo` | Load the furnished 21 Beaver reference house with interlocking tile, the surveyed frontage tree, a 285 mm stair going, garden beds, sewn bedding and waxed boards. The camera meters the glazing on the occupied floor. For comparison with a shifted architectural lens, an `EO_CAPTURE_POSES` camera may specify `projection_shift_ndc: [x, y]`. Zero centres the optical axis. The courtyard includes the source scene’s citrus and grasses. Run the [planting exporter](../scripts/blender-buildings/export_beaver_planting.py) in Blender with the finished editable scene open to rebuild its portable meshes. The terrace, kitchen reflection, surface and floor builders accept `--house-photo`. After building the terrace and kitchen reflections, run the [glazing baker](../scripts/blender-buildings/bake_beaver_glazing.py) with an external capture directory and `--capture --house-photo`, once for the street and again with `--rear`. The baker uses `--no-house-glazing` to exclude reflections of the probe being baked. |
| `--house-table-light` / `--no-house-table-light` | Opt in to the measured pendant on the dining table, fruit and crockery. Rebuild from the packed house with the [table light baker](../scripts/blender-buildings/bake_beaver_table.py). A separate receiver just above the wood resolves dish shadows. The place settings carry light measurements on both sides of their surfaces. |
| `--house-oak-finish` / `--no-house-oak-finish` | Use the packed dining boards’ grain relief, scanned roughness, thin wax and measured room reflections. [Blender bake](../scripts/blender-buildings/bake_beaver_oak.py). |
| `--house-oak-joinery` / `--no-house-oak-joinery` | Use the Blender cut-face material at both ends of each dining plank and the installed fixtures measured on its rounded edges. Combine with `--house-oak-finish` for the waxed tops and `--house-table-light` for the place settings. [Source bake](../scripts/blender-buildings/bake_beaver_table.py). |
| `--house-front-daylight` / `--no-house-front-daylight` | Light the fitted front garden with directional sky and reflected sunlight measured in the editable house. Six positions follow the sloping bed. Daytime fields follow the city clock and blend toward a separately measured overcast response. The game still calculates direct solar shadows. This option selects the street garden. Generate solar samples with `cargo run --release -p eo-look --example site_daylight`, run the [Blender survey](../scripts/blender-buildings/survey_beaver_front_daylight.py) on the packed garden house, then run the [daylight cook](../scripts/blender-buildings/cook_beaver_front_daylight.py). |
| `--house-fired-stoop` / `--no-house-fired-stoop` | Use the individually modeled terracotta landing and stair at 21 Beaver Street. Six clay courses follow the installed stair height. The two arch piers extend to the local grade. With the fitted porch lamp, the clay receives the [measured light on the steps](../scripts/blender-buildings/bake_beaver_stoop_light.py) under the current sky. [Blender build](../scripts/blender-buildings/build_beaver_stoop.py) · [Surface preparation](../scripts/blender-buildings/prepare_beaver_stoop.py) · [Runtime cooker](../scripts/blender-buildings/cook_beaver_stoop.py). Disabled by default. |
| `--house-porch-lamp` / `--no-house-porch-lamp` | Light the entrance from the fitted six-watt opal shade. The measured field includes the fixture hood, bracket and arch. Five surface charts carry diffuse light on the plaster. The swinging door casts a live shadow. With the site and Blender cameras, the approach uses the source exposure and retains the lamp color at night. Rebuild from the editable house with the [visibility survey](../scripts/blender-buildings/bake_beaver_porch_light.py), [surface bake](../scripts/blender-buildings/bake_beaver_porch_surfaces.py) and [atlas cooker](../scripts/blender-buildings/cook_beaver_porch_light.py). Time the approach with `--bench --segments house_porch`. |
| `--house-joinery-light` / `--no-house-joinery-light` | Use the measured diffuse light on the actual casement timber, putty, ironwork and painted entrance. The live material supplies the specular highlight. |
| `--house-window-optics` / `--no-house-window-optics` | Trace reflections from either side of the front and rear windows through the furnished Blender scene. This enables the photo house and uses its glazing. Measure the moving reflection with `--bench --segments house_windows`. |
| `--house-wall-daylight` / `--no-house-wall-daylight` | Opt into the Cycles diffuse light measured on the 21 Beaver street and yard walls, both returns, round stair bay and porch. The sun and its shadows follow the live city. Measure the approach with `--bench --segments house_frontage`. |
| `--house-site-camera` / `--no-house-site-camera` | With `--blender-camera`, use the packed scene’s daylight exposure near the house exterior. The approach and entrance blend into the city and room meters. |
| `--house-street-garden` / `--no-house-street-garden` | Fit the sloped street planter with separate agaves, deergrass, mineral mulch and weathered render. The native materials use the UV surfaces from the [editable Blender garden](../scripts/blender-buildings/build_beaver_street_garden.py). Measure the approach with `--bench --segments house_garden`. |
| `--house-window-linen` / `--no-house-window-linen` | Fit the editable five-pleat flax blind, woven lift tapes, cords and brass rings at the rear dining window. The cloth uses separate Cycles measurements for its indoor and outdoor faces under the current lamps, sky and sun. [Blender assembly](../scripts/blender-buildings/build_beaver_window_linen.py) · [Light bake and portable export](../scripts/blender-buildings/bake_beaver_window_linen.py) · [Native cooker](../scripts/blender-buildings/cook_beaver_window_linen.py). |
| `--house-kitchen-light` / `--no-house-kitchen-light` | Use measured room reflections on the fitted kitchen, light from the two hood spots on the backsplash, and the authored oak on the chopping board. With `--blender-camera`, the source lens blends in at the ground-floor entrance. Bake the [kitchen lighting](../scripts/blender-buildings/bake_beaver_kitchen_light.py) and [six board faces](../scripts/blender-buildings/bake_beaver_kitchen_board.py) in the packed fitted scene, both with `-- PORTABLE_DIR EXTERNAL_SURVEY_DIR`. Run the [kitchen cooker](../scripts/blender-buildings/cook_beaver_kitchen_light.py) with `EXTERNAL_SURVEY_DIR/kitchen-light-survey.npz PORTABLE_DIR/fitted-room.glb`. |
| `--house-fitted-finishes` / `--no-house-fitted-finishes` | Opt into the fitted dining room with the source walnut varnish, ceramic glaze, chair and table finishes. Full-sphere Cycles measurements supply local reflections. Dedicated diffuse charts preserve the lighting on the bowl, folded linen and individual boards. The negative switch restores the fitted room’s previous material response. Rebuild in the packed fitted scene using the [surface baker](../scripts/blender-buildings/bake_beaver_fitted_finishes.py) with `-- PORTABLE_DIR EXTERNAL_SURVEY_DIR`, then the [finish cooker](../scripts/blender-buildings/cook_beaver_fitted_finishes.py) with `SURVEY.npz PORTABLE_DIR/fitted-room.glb`. |
| `--house-fitted-dining` / `--no-house-fitted-dining` | Fit the Blender dining table, six chairs, kitchen, stair, and rear window at 21 Beaver. Uses their authored UVs and matching collision. For the calibrated daytime view, also enable the dining meter, Blender camera, downstairs light, dining light and floor reflections. From the fitted scene, run the [room baker](../scripts/blender-buildings/bake_beaver_bedroom.py) with `-- meter --scene "Dining · site daylight" --exterior-blend ORIGINAL.blend --output-dir LIGHTS`. Install the portable meshes and measured lighting using the [fitted room cooker](../scripts/blender-buildings/cook_beaver_fitted_room.py) with `PORTABLE --lighting LIGHTS`. Opt-in. |
| `--house-floor-reflections` / `--no-house-floor-reflections` | Reproduce the ground-storey oak strips and sample the furnished room's directional reflections under the current sky, sun and installed lights. [Source bake](../scripts/blender-buildings/bake_beaver_floor_gloss.py). |
| `--house-dining-meter` / `--no-house-dining-meter` | Meter daylight on the ground floor using its measured sky return and visible window area. It corrects the dome-estimate meter; with the scene meter on (the default) the frame's own reading sets the exposure, and `--no-scene-meter` selects this meter. With `--blender-camera`, the furnished source camera and the calibrated white card set the interior exposure ceiling. [Dining measurements](../public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/downstairs-daylight.provenance.json). |
| `--house-downstairs-light` / `--no-house-downstairs-light` | Opt in to the furnished ground-floor lighting survey. The ceiling, stairs and furniture share the Cycles window and fixture measurements. Rebuild with the [room baker](../scripts/blender-buildings/bake_beaver_bedroom.py) and the `downstairs` product. Combine with `--house-dining-light` to resolve chair shadows on the oak. |
| `--house-dining-light` / `--no-house-dining-light` | Opt in to the furnished house with Cycles lighting on the ground-floor boards. The 19 W pendant casts the shadows of the installed chairs. Window light follows the city sky and sun. Bake the two pages in the packed house with the [floor light baker](../scripts/blender-buildings/bake_beaver_dining.py), passing an external output directory, `--install` and `--reference-key R G B` from a clear-noon capture. Measure with `--bench --segments house_dining`. |
| `--house-mission-roof` / `--no-house-mission-roof` | Fit scanned, two-piece clay tiles to the gable and rounded stair bay at 21 Beaver. This opts into the photo house. Rebuild the mesh and material pages with the [Blender roof builder](../scripts/blender-buildings/build_beaver_mission_roof.py) on the staged house, passing an external output directory, the verified source-scan directory and `--runtime`. The roof timing path is `--bench --segments house_roof`. Disabled by default. |
| `--blender-camera` / `--no-blender-camera` | Display scene-linear light through the measured sRGB response of the furnished house’s Blender camera. Applies to the full view in play and captures, including water optics, in place of the filmic display and the camera look (the `--look cinematic` and `--photo-finish` passes keep their own). The existing exposure meter and night adaptation remain active. Its shader code is compiled out without it (`SF_BLENDER_CAMERA`). Calibrate with the [camera chart](../scripts/blender-buildings/calibrate_beaver_camera.py). Disabled by default. |
| `--house-blender-bedroom` / `--no-house-blender-bedroom` | Opt in to the Blender bedroom on the photo house. Its quilt uses the cutting pattern and baked wrinkle normals. The upstairs diffuse light uses six hemispheres per probe under a unit sky and four solar directions. The game keeps the sun, weather, fixtures and shadows live. Rebuild both pages with the [bedroom baker](../scripts/blender-buildings/bake_beaver_bedroom.py) on the editable scene, once with `-- quilt` and once with `-- light`. Measure the occupied room with `--bench --segments house_bedroom` and the photo house loaded. Disabled by default. |
| `--house-photo-lod` / `--no-house-photo-lod` | With the optional furnished house enabled, use a reduced mesh for material parts more than 120 m from the camera. Foliage and architectural detail retain their full geometry at the house. Disabled by default. |
| `--no-interior-lighting` | Disable the baked window and pendant light contribution in the Beaver Street house. |
| `--play` | The player instead of the free camera |
| `--test-character ID` / `--no-test-character` | Opt in to a demo avatar in `--play`, or restore the standard avatar. See [Demo characters](#demo-characters). |
| `--test-clip NAME`, `--list-test-clips` | Preview a named animation on the selected demo avatar, or print the roster and clip names. |
| `--car-door-animation` / `--no-car-door-animation` | In local play with a main demo avatar, use the paired entry and exit clips and the sedan's driver-door hinge when pressing F. |
| `--ui` | The interface shell: title, loading, HUD, pause menu |
| `--connect ADDR\|local` | Online client (with `--name`, `--spawn x,z,heading`) |
| `--bench` | Offscreen timing run over scripted camera paths; prints and writes JSON to `bench-out/` |
| `--bench --play` | Scripted walk and drive paths with the player |
| `--capture-ui SCREEN` | Writes a PNG of one screen: `title`, `loading`, `settings`, `pause`, `map`, `hud` or `all`; also (not in `all`) `settings-hyper` (the settings panel's presets with Hyper real chosen and the "Lighting and detail" row with its note), `settings-world` (its World block), `pause-settings` (the pause sheet's graphics rows), `photo` (the photo mode panel's Camera page over a frame through its pipeline, focus peaking on) and `photo-look` (its Look page, monochrome) |
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
| `--rt-gi` | Ray-traced one-bounce diffuse light (a prototype; no tier turns it on). Inline ray queries (wgpu's experimental `EXPERIMENTAL_RAY_QUERY`) against the traced scene (eo-render `rt_scene`): acceleration structures built from the renderer's own vertex and index arena: the terrain, the street's flat far strips and plazas, the grade-separated roads' static level and the landmarks in 256 m cells, each resident kit tile's rung (the near rung where it is resident, else the mid one), and the hand-built models as they arrive, within 420 m of the camera (no agents, props, trees, water or the street's near packs: kerbs and paint). The city's opaque pass writes a thin G-buffer beside its colour (`rt_composite`: the indirect diffuse it added and what multiplied it, the vertex normal; the colour itself is unchanged); then, at half resolution, one cosine-distributed ray a texel from that surface: a ray that leaves the scene takes the rig's sky in its direction, a hit its Lambert radiance (the material's albedo, the kit paint and layer mean, the street class; lit by the sun through a shadow ray, the rim, and the rig's sky x 0.6 for the bounce beyond). Accumulated over up to 24 frames with reprojection and filtered by an edge-aware a-trous filter stopped by the surface's plane and normal and by the history's variance; a full-screen composite in the same frame puts it in place of the hemisphere light, the environment's diffuse (by the traced open share), the ambient volume and its ground bounce, and the screen-space bounce, with half the screen-space occlusion kept; it fades back to the rig's light from 260 to 340 m. The rest of the city pass (far layer, trees, props, agents, far shore, sky, glass) draws after the composite in a pass of its own and keeps the rig's light. Metal only (the one backend tested): on any other device it is off and the log says so in one line. `--no-rt-gi` (the default) leaves the shaders, the layout, the device and the frame as before. Tunables for experiments: `EO_RT_GI_*` (eo-render `rt_gi::RtGiConfig::from_env`; `EO_RT_GI_DEBUG=1` the traced light alone, `2` the open share, `3` the bounce) and the traced scene's `EO_RT_RADIUS`, `EO_RT_DROP`, `EO_RT_BUILD`, `EO_RT_KIT_NEAR`. The bench JSON's `lighting_passes_ms` times `rt_gi` (trace, temporal, filter; the acceleration-structure builds are outside it), `rt_composite` and `city_rest`, and `rt` reports what was traced. Cost (M5 Ultra, high tier, 1920 x 1080, a busy machine: load 15-30 with other sessions' GPU work; the lower of two interleaved runs' GPU frame medians against the frame without it): street_walk +0.7 ms, kit_street +0.0, drive +0.8, city_overview +0.4 (nothing is traced past 340 m there). Of it the trace and filter take 0.5-0.65 ms, the composite 0.6-0.8, and splitting the city pass so the rest draws after the composite 0.3-0.6; the city's early opaque draw is unchanged within the noise (stage R1 read the result in every lit pipeline, which cost that draw 1.0-2.5 ms). The bench's settle builds the traced scene round the camera before a capture is drawn |
| `--rt-reflect` | Ray-traced reflections (a prototype; no tier turns it on; with or without `--rt-gi`, one G-buffer for both). From the same G-buffer (the reflection the rig added, its Fresnel and occlusion weight, the roughness, the vertex normal), at half resolution, one ray a texel drawn from the surface's GGX lobe (the mirror direction below roughness 0.06) through the same traced scene: a hit takes its Lambert light, a ray that leaves the scene the analytic sky the rig reflects, scaled by the surface's environment class. Accumulated over a number of frames that grows with the roughness (none on a mirror, up to 12) and filtered as far as the roughness reaches; the composite puts it in place of the sky, probe and screen-space reflection on the streets, wet or dry, the buildings and their glazing, terrain and structures up to roughness 0.6, fading back to the rig's from 260 to 340 m. Not yet on car paint, props, the bay or the glazing's clearcoat (drawn after the composite: the rig's reflection), and what is reflected is lit diffusely, without props, agents, trees, water or emissive light. Metal only; `--no-rt-reflect` (the default) leaves the frame as before. Tunables: `EO_RT_REFLECT_*` (eo-render `rt_reflect::RtReflectConfig::from_env`; `EO_RT_REFLECT_DEBUG=1` the traced reflection alone, `2` its radiance, `3` the rig's reflection it replaces); the bench JSON's `lighting_passes_ms.rt_reflect`. Cost (as `--rt-gi`'s): alone street_walk +0.6 ms, kit_street -0.1, drive +0.7, city_overview +0.5 (the trace and filter about 0.2 ms, its composite 0.25-0.3, the split city pass as above); with `--rt-gi` street_walk +1.75, kit_street +0.6, drive +1.4, city_overview +1.1. |
| `--photo-finish [1\|2]` | A photographic finish (eo-render `photo.rs`, `look_photo.wgsl`; eo-look `tone`, eo-sky `photo`; off by default, so the post pass and the sky are as before): the post pass becomes a camera on the resolved frame. Its response keeps the old chain's neutral curve (middle grey lands where it did, so the exposure means the same) on each colour's brightest channel, keeps the colour's hue, pales bright colours toward white by as much as a per-channel curve would (a lamp's or a sunlit wall's core goes to white instead of clipping to a pure hue) and turns their hue a third of a per-channel curve's way in the shoulder; the grade adds no saturation or vibrance and rolls its brightness off to white instead of clipping. The sensor adds noise in the scene's units (shot noise and a read floor, by the ISO a phone camera at f/1.8 and 1/30 s picks for the scene's EV: base ISO by day, invisible; a few hundred on a lit street at night, a fine grain in the shadows), a new pattern each frame, after the TAA. The lens adds a residual fall-off (0.85 at the corners), lateral colour growing to about a pixel at the corners, and a bloom with a lower, softer threshold, more of a small source's light and less in the wide levels (a glow round lamps and glints, no veil). The sky's tables and the aerial perspective take San Francisco's measured air (IMPROVE's 50 km visual range row, an aerosol of albedo 0.9 under the 400 m marine inversion): a paler horizon and a warm-grey haze over the far city. Level 2 adds chroma noise and a smoother lateral colour. The `high` tier sets 1 and `hyper` 2; `--no-photo-finish` restores the frame |
| `--facade-clutter [1\|2]` | The things San Francisco fronts carry (eo-kit `clutter.rs`, stage W3; off by default, so the kit meshes and their cache key are as before): fire escapes on older multi-unit fronts (Tenderloin, Chinatown, Nob Hill, SoMa, the Mission most; one per stack of windows: slatted landings at each floor above the ground floor with picket railings, 60-degree stairs between them through a hatch, a raised drop ladder, knee brackets, a gooseneck over the parapet where it reaches; painted steel, mostly black, rusted by the building's wear; the port's own fire escapes are drawn this way in their place), awnings on shop and food tenancies without a shade (shed, dome, box and open retractable forms in the common fabric colours and stripes, sun-faded, a few torn), sign faces on the fascia boards and projecting blade signs (lit boxes or neon tubes on a dark board) carrying a generic word of the shop's category (stage S1: `SIGN_PHRASES`, CAFE, LIQUOR, DELI, LAUNDRY ...; never a name, a brand or a mark) in a bold, condensed, serif or script face found among the system's fonts at start (drawn in memory, never written to disk; with none installed, W3's colour fields and shapes), beside an emblem or in a border, painted, back-lit (letters over a dim face, or the face lit through under the tone curve's clip) or in neon tubes with their glow, faded by the building's wear; stacked letters on tall blades; lit by night where the shop is lit (`draw.wgsl` `fc_lit`: with `--shop-light` where the shop's own light is on, dimmed to its security level, else by the shops' lit share), roll-down security shutters (some down), bars over older ground-floor windows, downspouts at the party walls with their conductor heads and straps, meters, gas meter sets, conduit and the standpipe connection at the base, and on flat roofs chimneys with clay pots, vent stacks and flue caps, satellite dishes aimed at the south-south-east sky and TV aerials. Nothing passes through another piece or the plan's own parts, and nothing stands out over the walk past `sidewalk-width-sf-neighborhood-residential` less `sidewalk-edge-zone-sf` (the alley widths on a service way; awnings and signs also under the code caps and the 8 ft clearance). Near level all of it, mid level a simplified version, far level none. Level 2 adds window air conditioners (rare), dishes on fronts, pipe scaffolding and boarded shops. The kit tiles cook into their own cache folder with it. The `high` tier sets 1 and `hyper` 2; `--no-facade-clutter` restores the meshes |
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
| `--photo-bench` | Every path through photo mode at the depth of field's heaviest (f/1.4 focused at 0.5 m); first the photo passes' own GPU time at each path's start with and without the depth of field, into `photo-cost-<label>.json` (and, with `EO_PHOTOS_DIR` set, one 2x capture) |
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
  `drizzle`, `rain`, `storm`), `what`, `photo`, `subject` (below), `person`
  (`{"feet": [x, y, z], "heading": degrees}`: the player's body standing
  there, the simulated people beside it hidden; `eo-landmarks`'
  `access_shots` example writes one such shot per landmark deck), and keys
  starting with `_` (comments).
- `photo` renders the shot through photo mode's pipeline: `focal_mm` (35 mm
  equivalent; it sets the field of view), `f_number` (none: everything
  sharp), `focus` (metres, `"look_at"` or `"subject"`), `grade` (`neutral`,
  `warm-film`, `cool`, `monochrome`), `strength` (0 to 2 of the grade),
  `filter` (monochrome's contrast filter: `none`, `yellow`, `deep-yellow`,
  the default, `orange`, `red`, `green`), `fade`, `ev`, `vignette` and
  `grain` (0 to 1), `letterbox` (an aspect), `frame_offset` (-1 to 1: the
  letterbox opening from the top to the bottom), `roll` (degrees), `blades`
  (0: a circle, else 3 to 16; default 9), `rounded` (default true), `cat_eye`
  (default true) and `peaking`. `subject` stands the player's
  body in the shot: `at` [x, z] game metres on the drawn street, `facing`
  (compass degrees; default toward the camera), `model` (an index into the
  people models), `walking`. With `--no-photo-mode` a shot keeps the lens's
  field of view and the subject and nothing else of photo mode.
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
| Underwater habitats | `--underwater-habitats` adds the Bay and Pacific beds. `--no-underwater-habitats` removes them |
| Aquatic Park Cove | `--cove-detail` enables the beach profile, cobbles, pier parapet, distant wooded slope, fair-weather clouds and cove optics. `--no-cove-detail` restores the previous scene. The Bay dive-site starts enable the preview. |
| Black Point canopy | `--cove-canopy` enables the cove preview and resolves bark, branches and leaf cover from eight views of each park tree species. `--no-cove-canopy` restores the previous distant crowns. |
| Submerged cove sediment | `--cove-sediment` enables the cove preview with rough submerged sand and silt. `--no-cove-sediment` restores the previous seabed material. |
| Municipal Pier pile contact | The cove preview fits each support to the drawn seabed. `--no-cove-pile-contact` restores the previous fixed pile ends, and `--cove-pile-contact` enables the preview. The park describes the pier’s [piled construction and protection against scour](https://www.nps.gov/safr/learn/historyculture/aquatic-park-pier.htm). |
| Black Point intertidal | `--dive-site black-point` starts beside barnacle-covered rock and small algae at the west end of Aquatic Park beach. `--black-point-intertidal` adds this habitat to the map. `--no-black-point-intertidal` hides its geometry. The location follows the [National Park Service account](https://www.nps.gov/safr/blogs/a-hidden-community-in-aquatic-park-cove.htm). Individual rocks and organisms are modeled. |
| Black Point rockweed | `--black-point-rockweed` models forked blades with raised midribs and thickened tips on the rocky habitat. `--no-black-point-rockweed` uses the original ribbons. Blade margins transmit more backlight than the thicker rib. The geometry follows the form described by [Stanford’s Hopkins Marine Station](https://seanet.stanford.edu/brown) and the habitat reported by a [central Bay shoreline survey](https://escholarship.org/uc/item/8jt3v8mk). Individual plants are illustrative. |
| Cove daylight | `--cove-daylight` uses one depth-dependent bed-light model on both sides of the waterline. `--no-cove-daylight` restores the prior surface and underwater lighting. The regional reference is the median of 102 downwelling-light profiles at USGS station 20, about 2.9 km from Aquatic Park. The broadband coefficient is measured, while the RGB balance and caustics are modeled. [Measurements and attribution](../THIRD_PARTY_NOTICES.md#san-francisco-bay-underwater-daylight). |
| Dive exposure | `--dive-exposure` compensates for the loss of downwelling light at the eye. `--no-dive-exposure` restores the original exposure |
| Bridge detail | `--bridge-detail` enables the Golden Gate detail preview (a darker paint tone, ribbed tower legs, framed portal struts, full arch pylons, lamp standards), each part drawn only where the bridges' own passes (Bridges, below) leave it as exported; left out, the preview is off. `--no-bridge-detail` is not its off form: it is the bridges' own detail switch (Bridges), so with both the preview's legs and portal struts stand in for that pass |
| Landmark decks | `--no-landmark-access` (the decks, their lifts and the tower bands and tower tops they take over; the collision world without them; the rooms' camera meter and the player's deck occlusion, whose shader terms compile out, `SF_LANDMARK_ACCESS`) |
| People | `--no-rocketbox`, `--no-rocketbox-motions` (either one: no people) |
| Demo player avatar | `--no-test-character` |
| Driver-door animation | `--no-car-door-animation` |
| Props | `--no-props2`, `--no-furniture`, `--no-eztree`, `--no-wires`, `--no-overhead`, `--no-lamp-sprites`, `--no-stand-ins`, `--no-surveyed-furniture`, `--no-tree-basins`, `--no-furniture-models` (a furniture kind whose GLB is not in the checkout draws as a 0.3 m box at its height, not its model drawn in code), `--no-street-props-v2` (the v1 street-prop files the JS lists, drawn as proxies or stand-ins where the checkout lacks them, as before the v2 models) |
| Bridges | `--no-bridges2` (all of these), `--no-bridge-scale`, `--no-bridge-paint`, `--no-bridge-detail`, `--no-bridge-wires`, `--no-bridge-night`; the second pass: `--no-bridges3` (all of its), `--no-bridge-air`, `--no-bridge-finish`, `--no-bridge-members`, `--no-bridge-floods`, `--no-bridge-curtain`; `--no-bridge-wire-sky` (the wire overlay reads its aerial perspective's sky colours per pixel, as before, instead of per vertex) |
| Harbours | `--no-floating-docks` (no floats along the harbours' pier lines, and so no berths or boats), `--no-boats` (the floats without the moored boats; `--no-props2` also leaves them out) |
| Buildings | `--no-facade-detail`, `--no-facade-age`, `--no-rooms`, `--no-room-frames`, `--no-room-dim`, `--no-room-lit-runs`, `--no-tower-night` (a far tower's lit floors as offices: ceiling glow and fixtures over the desks, tenants' own lamps, cleaners' floors, blinds), `--no-tower-crowns` (a tall tower's crown at night: a faint glow over its top storey and LED lines at the parapet), `--no-roof-colours`, `--no-roof-plant`, `--no-roof-parapet`, `--no-roof-patches`, `--no-roof-neutral`, `--no-wall-paint`, `--no-period-paint`, `--no-wall-chroma`, `--no-facade-typology`, `--no-room-blind-runs`, `--no-pane-transmittance`, `--no-far-facade`, `--no-far-roofs` |
| Landmarks | `--no-landmarks2`, `--no-towers2` (the five landmark buildings as the JS export's generators built them), `--no-skyline` (the named skyline towers back to the kit), `--no-towers3` (the landmark buildings' and skyline towers' third pass off: the lit band in place of the projector floodlights, the previous glazing, night windows and crown, base truss, Coit shaft, City Hall granite and Sutro lattice, and the groves), `--no-grove` (the tree canopy of Pioneer Park round Coit Tower and of Mount Sutro round Sutro Tower off), `--no-towers4` (the fourth pass off, the third's output: Salesforce's crown fins, sunshades and perforated panels with Day for Night's walkers, the facades' fin and reveal sun shadows, spandrel and pier finishes, 555 California's carnelian, the coated-aluminium spires and braces, City Hall's gold leaf and drum floods, and the groves' summit clearing, species forms, forest floor and Coit loop lamps) |
| Ground | `--no-ground-hills`, `--no-ground-green`, `--no-ground-contact`, `--no-ground-beach-fit` (the sand round a mapped beach spreads as the JS rule does: over lawns and paving mapped beside it and along a seawall's quay), `--no-terrain-lod` |
| Water | `--no-water` (the flat sea plane), `--no-water-instanced` (the pass needs multi-draw-indirect-count, as before; without it the flat sea plane), `--no-water-brdf` (the earlier Fresnel, roughness, sun lobe and reflection march), `--no-water-body` (the earlier body colour), `--no-water-foam-cover` (the earlier whitecap cover and foam sampling), `--no-water-glint` (the earlier hashed sparkle), `--no-water-turbid` (the earlier three-wavelength body), `--no-water-shore` (the earlier waterline, wall collar and the lift under piers), `--no-water-foam-lit` (the earlier whitecap shapes and foam light), `--no-water-ppr` (the reflection march alone), `--no-water-wind-texture` (no gust patches, tidal current or rotated chop; drawn from the `high` tier up, `--water-wind-texture` below it), `--no-water-grazing` (one mean Fresnel times the sky in one direction) |
| Waterfront corridor | `--no-ferry-building` (its record's plain box instead), `--no-tram-tracks` (no streetcar rails), `--no-quay-railing` (no guard on the Embarcadero quay wall), `--no-ferry-building-detail` (the Ferry Building's first pass: no ashlar course scale, rustication, deep reveals, modillions, granite plinth, weathering or tower balustrades), `--no-rail-finish` (one bare-steel rail head and a black groove), `--no-corridor-palms` (no Embarcadero palms), `--no-guard-detail` (the quay guard's first pass, on the JS crest: no kick plate, solid pickets), `--no-pier-sheds` (the Ferry Plaza's JS shed boxes), `--no-rail-polish` (the rails' worn band whole at roughness 0.4, no burnished strip or sealant joints), `--no-facade-relief` (the Ferry Building's second pass: one glazing pane an opening, the second pass's reveal tone and dirt under the cornices, no fanlight frames or voussoir tones), `--no-palm-detail` (the corridor palms' first pass: no tree well, root initials, scar lattice, senescent leaves or spear, and unsurveyed palms at the median size) |
| Scanned surfaces | `--no-cc0` (all), `--no-area-paint` (a lane-wide marking worn as coloured pavement, not as a line), `--no-cc0-street`, `--no-cc0-walls`, `--no-cc0-ground`, `--no-cc0-decals`, `--no-cc0-grime`, `--no-cc0-stains`, `--no-cc0-pom`, `--no-cc0-tiling` |
| Lighting and sky | `--no-shadows`, `--no-ao`, `--no-lamp-spread`, `--no-far-street-light` (past ~300 m the streets lit from a map of every lamp's pool), `--no-sky-vis`, `--no-contact-shadows`, `--no-atmosphere`, `--no-phys-sky`, `--no-ap-lut`, `--no-night-shaping`, `--no-fog`, `--no-clouds`, `--no-cloud-shadows`, `--no-cloud-depth-haze`, `--no-shafts`, `--no-moon-stars`, `--no-lights` |
| Lighting additions (opt-in) | `--no-cascade-blend`, `--no-far-shadows`, `--no-ambient-volume`, `--no-probe-grid`, `--no-light-shadows`, `--no-light-shadow-moving`, `--no-headlight-shadows`, `--no-lit-fog`, `--no-smooth-time`, `--no-ground-bounce`, `--no-shadow-slope-bias`, `--no-grime`, `--no-street-life`, `--no-photo-finish`, `--no-facade-clutter`, `--no-shop-light`, `--no-car-detail`, `--no-look` (each wins over `--quality`), `--no-rt-gi`, `--no-rt-reflect` (no tier turns `--rt-gi` or `--rt-reflect` on) |
| World additions (opt-in) | `--no-structures`, `--no-tunnels` (the last of a switch and its `--no-` form wins; `eo-server` takes them too) |
| Weather | `--no-rain`, `--no-wet`, `--no-wind` |
| Camera finish | `--no-taa`, `--no-bloom`, `--no-adaptation`, `--no-ev100`, `--no-filmic`, `--no-ssr` |
| Camera look and exposure | `--no-gg2-camera` (all three below), `--no-camera-look`, `--no-scene-meter`, `--no-lamp-photocell`, `--no-night-lift` (the darker night of 2026-11-17: unlit ground at 8/255, not 14) |
| Daylight and sky | `--no-gg2-daylight` (all of these), `--no-ground-albedo`, `--no-photo-date`, `--no-clear-sky-state`, `--no-cirrus`; reserved, nothing reads them yet: `--no-terminator-bias`, `--no-spectral-sky` |
| Golden Gate, fourth pass | `--no-bridges4` (all of these), `--no-bridge-shade2`, `--no-bridge-dielectric`, `--no-bridge-haunch`, `--no-bridge-bracing`, `--no-bridge-pylons`, `--no-bridge-arch2`, `--no-bridge-fender2`, `--no-bridge-crest`, `--no-bridge-collar` |
| Strait water | `--no-gg2-water` (all of these), `--no-water-exposure`, `--no-water-current`, `--no-water-lean` (drawn from the `high` tier up, `--water-lean` below it), `--no-water-ppr-blur`, `--no-beach-profiles`, `--no-water-streak-caps`, `--no-water-far-calm`, `--no-baker-surf`; reserved: `--no-swash-line` |
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
| Grass, third pass | `--no-grass3` (all of these: run 2's meadow), `--no-scrub-mounds`, `--no-scrub-bloom`, `--no-bare-ground`, `--no-meadow-breakup` |
| Flora (scrub species, grassland season, chert, serpentinite, flowers) | `--no-flora` (all of these: the third grass pass's output), `--no-scrub-species`, `--no-grass-ramp`, `--no-bluff-scrub`, `--no-chert-ground`, `--no-serpentinite`, `--no-flora-flowers` |
| Flora 2 (Battery Spencer knoll, Baker bluff rock and boulders) | `--no-flora2` (all of these: the flora pass's output), `--no-knoll-ground`, `--no-knoll-relief`, `--no-bluff-rock`, `--no-boulder-surf` |
| Surf and bathymetry | `--no-gg3-water` (all of these), `--no-bathymetry`, `--no-surf-break`, `--no-swash-foam`, `--no-obstacle-foam`, `--no-water-body2`; reserved: `--no-ppr-dash` |
| Golden Gate, fifth pass | `--no-gg3-bridge` or `--no-bridges5` (all of these), `--no-paint-brdf2`, `--no-paint-weathering`, `--no-concrete2`, `--no-leg-cells`, `--no-cable-detail` |
| Visibility and streaming | `--no-occlusion`, `--no-view-detail`, `--no-upload-ring`, `--no-submit-pacing`, `--no-indirect-count`, `--no-count-emulation`, `--no-bounded-draws` (wins over the opt-in `--bounded-draws`; see [What is drawn](#what-is-drawn)), `--no-city-classes` (the city's opaque pass as one pipeline and one list pair, as before the split), `--no-motion-bounds` (the count emulation's late opaque lists always floored by their early lists' bounds and the early lists at their reserve, as before the motion-aware bounds) |
| Interface and sound | `--no-hud`, `--no-audio`, `--audio-null` (mix to a null output), `--no-settings-apply` (`--ui`: the settings file does not reach the renderer or the world) |
| Optional assets | `--no-external` (read each set only from the folder its variable names, not from `external/`: the output before that folder was read) |
| Online (`--connect`) | `--no-remote-bodies` (other players drawn as before: first walk style and idle, linear path, no emotes or phone, street people may wear a player's body), `--no-name-tags` (no tags, no tag depth probe), `--no-player-contact` (no ground contact under their bodies, and its shading compiled out of the agents' shaders: `SF_PLAYER_CONTACT`), `--no-shared-sky` (the client's own hour and weather) |
| Photo mode | `--no-photo-mode` (none of it: P does nothing, shots draw no depth of field or finish), `--no-dof` (photo mode without its depth of field) |

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
variance (left out below the `high` tier unless `--water-lean`: Quality
tiers); `--no-water-ppr-blur` the slope-blurred projected reflection;
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
on Apple GPUs), the PPR and scene-meter passes, the bridges' wire overlay
(`wire_overlay`, drawn after the bloom; untimed, its time was counted in
the scene meter's share), and a budget verdict of "not run" when no
standard path ran.

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

The third grass pass (`--no-grass3`, eo-landmarks `vegetation::Grass3`; it acts
inside the second pass's meadow cover): `--no-scrub-mounds` leaves out the
lumpy coastal scrub mounds (coyote brush, sagebrush, bush lupine; leaf clumps
0.5-1.5 m tall) planted where the land cover and NAIP show scrub (including
the summer image's bright green) and on the Marin headland's seaward slopes;
`--no-scrub-bloom` leaves the lupine stands out of flower (coyote brush
there); `--no-bare-ground` gives the eroded ground (steep faces the NAIP shows
bare, red-brown soil, warm bare knoll tops) back to the meadow and the
terrain's bare class back to sward, instead of a red-brown soil mat with
sparse tufts; `--no-meadow-breakup` keeps the meadow's round type and cure
patches, the patch levels' fixed distances (a band across the view) and the
mats' full discs.

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
`--structures`, `--tunnels`, `--shop-light`, `--facade-clutter`,
`--look cinematic` (`SF_CINEMATIC`), the playable house (`SF_HOUSE`: its
terms, its varyings and its finishes' runtime branches), the sea bed's
habitats (`SF_SEABED`, with a seabed object in the world) and
`--cove-sediment` (`SF_COVE_SEDIMENT`). With none of them the frame costs
what it cost before they existed; with one on, its code is the code it ran
before, so its output is the same.

| Tier | Sets | Budget (ms) |
| --- | --- | ---: |
| `low` | `--shadow-map 2048`, `--no-ssr`, `--no-contact-shadows`, `--sky-quality low`, `--no-ao`, `--no-shafts`, `--no-bloom`, `--surveyed-furniture`, `--tree-basins` | 8 |
| `medium` (default) | the street look: `--grime 1`, `--stand-ins`, `--surveyed-furniture`, `--tree-basins` (made the default with the owner's approval on 2026-11-20, after the citywide street audit; `--no-grime --no-stand-ins --no-surveyed-furniture --no-tree-basins` gives the frame before it) | 10 |
| `high` | `--cascade-blend`, `--far-shadows 1`, `--ambient-volume 1` (with `--ground-bounce`), `--light-shadows 16`, `--headlight-shadows 2`, `--lit-fog 1`, `--smooth-time`, `--shadow-slope-bias`, `--grime 1`, `--street-life 1`, `--stand-ins`, `--photo-finish 1`, `--facade-clutter 1`, `--shop-light 1`, `--car-detail 1`, `--surveyed-furniture`, `--tree-basins`, `--look cinematic`, `--water-wind-texture`, `--water-lean` | 16.6 |
| `hyper` | `high`, then `--shadow-map 4096`, `--sky-quality high`, `--far-shadows 2`, `--ambient-volume 2`, `--probe-grid 2`, `--light-shadows 48`, `--light-shadow-moving`, `--headlight-shadows 8`, `--lit-fog 2`, `--grime 2`, `--street-life 2`, `--photo-finish 2`, `--facade-clutter 2`, `--shop-light 2`, `--car-detail 2`, `--surveyed-furniture`, `--tree-basins`, `--look cinematic` | 33.3 |

The sea's wind texture (`--water-wind-texture`: cat's-paw gust patches,
the tidal jet's rough water through the Golden Gate, the rotated re-read of
the chop cascade) and its lean (`--water-lean`: the anisotropic cascade fade
and sub-pixel slope variance) are drawn from `high` up; `low` and `medium`
leave them out of the water's shaders (each `--no-` form turns it off at any
tier, the positive form on at any tier). With the water's features compiled
in as constants, the two together cost the `medium` water pass about 0.5 ms
on `bay` (2.6 -> 3.1 ms of `water_and_glass` GPU time), 0.25 ms on
`golden_gate` and nothing measurable on `waterfront`.

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
| `EO_NATIVE_CACHE` | The cache folder (default `native/cache/`, inside the game's folder in a packaged build) |
| `EO_UI_SETTINGS` | Where the interface keeps its settings file (default: the user's application-data folder) |
| `EO_UI_FONT_DIR` | A font folder to use instead of the system font folder |
| `EO_TRACE_LOAD` | Print timings of the world load |
| `EO_PHOTOS_DIR` | Where photo mode saves its pictures (default: `photos` in the application-data folder the settings file is in) |
| `EO_CUMULUS` | `1` draws the fair-weather cumulus field under a clear sky (off by default; the overcast, fog and rain decks are drawn either way) |
| `EO_CAPTURE_HOLD_SKY` | `1` holds the sky's clock at `--world-time` for every pose of an `EO_CAPTURE_POSES` file, so the drifting clouds are the same in two runs (matched captures) |
| `EO_CAPTURE_STREETS` | `N` with `--bench --capture-props2`: the citywide street audit instead of the props cards: up to N street-level views spread over the whole map (a grid of cells over the street plan, the longest run of a class taken in turn in each: arterial, collector, residential), seen from the wider footway at eye height, written as `capture-<label>-street-NN-<class>.png` with the poses file, so a change to the street is judged across the city; deterministic (the same plan gives the same views) |
| `EO_GRIME_STREET_V1` | `1` (a diagnostic) keeps the street weathering `--grime` drew before the road's cracks and the markings' wear were redrawn (the polygon crack network, the flaked paint, the scan's full holes), for matched A/B captures on one build |

The [optional assets](#optional-assets) are read from `external/` at the
repository root; the variables below point a set at another folder.

### Optional asset folders

Each set is read from its folder under `external/` (the repository is found
as [Where the data comes from](#where-the-data-comes-from) describes) unless
its variable names another folder; an empty variable counts as unset. When
the folder does not hold a copy that matches the set's manifest, that
feature is turned off, the log says so in one line, and the game runs
without it. `--no-external` stops the game reading `external/`, so only the
variables count.

| Variable | Default folder | What it holds | Read by |
| --- | --- | --- | --- |
| `EO_PEOPLE_ROOT` | `external/people` | Rocketbox avatars | The game |
| `EO_PEOPLE_MOTIONS_ROOT` | `external/people-motions` | Rocketbox motion clips | The game |
| `EO_PEOPLE_STYLES_ROOT` | `external/people-styles` | 100STYLE walking styles | The game |
| `EO_TEST_CHARACTERS_ROOT` | none | The local test demo's `public` directory: the demo avatars and motion packs (`assets/test-characters-manifest.json`) | The game with `--test-character` |
| `EO_CC0_ROOT` | `external/cc0` | Scanned CC0 texture sets (Poly Haven, ambientCG) | The game, `tools/fetch-cc0.mjs` |
| `EO_AUDIO_ROOT` | `external/audio` | Sound clips | The game |
| `EO_SIGN_ART_ROOT` | `external/sign-art` | Official sign art and the sign font | The game, `tools/fetch-sign-art.mjs` |
| `EO_DEM_ROOT` | `external/dem` | Elevation tiles for the far shore | The game |
| `EO_SURVEY_ROOT` | none | The Golden Gate survey set: NAIP 0.6 m aerial tiles, USGS 3DEP 1 m DEMs, NOAA CUDEM topobathy, Poly Haven scanned rocks and plants (`assets/survey-manifest.json`) | The game (`eo_farshore::survey`, read by nothing yet), `tools/survey-manifest.mjs` |
| `EO_AERIAL_DIR` | none | Aerial imagery | Tools only (the roof survey and `tools/fetch-aerial.mjs`) |
| `EO_POLYHAVEN_STORE` | none | Poly Haven downloads | The Blender building scripts |

The street people need all three people sets (`people`, `people-motions`,
`people-styles`). With any of them missing, or with `--no-rocketbox` or
`--no-rocketbox-motions`, there are no people on the streets: there is no
other motion set to fall back on.

The fetch tools in `tools/` need either a folder argument (`--root <folder>`;
`fetch-aerial.mjs` takes it as its first argument) or their variable; they
have no default folder. Inside the repository the only folder they accept
is `external/<set>` (`external/aerial` for the imagery).

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

## Optional assets

Some content is too large to keep in Git (about 2 GB in all), or its source
asks that it be fetched from the source. It is never committed. One command,
run from the repository root, downloads all of it into `external/` there, a
git-ignored folder with one subfolder per set, and the game reads it from
that folder:

```sh
python3 scripts/setup-assets.py                    # every set
python3 scripts/setup-assets.py --only people,dem  # some sets
python3 scripts/setup-assets.py --dry-run          # what is missing; no request is made
```

Each set has a manifest in `native/assets/` (or `eo-props/data/`) that pins
every file by source, size and SHA-256. The script fetches exactly those
files and checks each one before it is renamed into place; a file already
present with the right digest is skipped and an interrupted download
resumes, so running it again is cheap. It uses only the Python 3 standard
library, sends requests one at a time only to the hosts the manifests name
(and the download servers those hosts redirect to), with a generic
User-Agent and nothing about the user, extracts each archive into its own
new temporary folder, and never executes anything it downloads. The game
checks every file again: a folder that does not match its manifest is
ignored as a whole, and the log says why. The program runs without any of
it and draws a simpler stand-in where it is missing.

| Content | Folder | Source | Manifest |
| --- | --- | --- | --- |
| Rocketbox people | `external/people` | The GLB repackaging of the Microsoft Rocketbox Avatar Library (MIT) | `assets/people-manifest.json` |
| Rocketbox motion clips | `external/people-motions` | The Rocketbox in-place clips in GLB (MIT) | `assets/rocketbox-motions-manifest.json` |
| 100STYLE walking styles | `external/people-styles` | The 100STYLE dataset in GLB (CC BY 4.0) | `assets/100style-manifest.json` |
| Scanned surface sets | `external/cc0` | Poly Haven and ambientCG (CC0) | `assets/cc0-manifest.json` |
| Sound clips | `external/audio` | FSD50K and SFX Kit clips (CC0 and CC BY); credits in `assets/AUDIO-CREDITS.md` | `assets/audio-manifest.json` |
| Sign art and sign font | `external/sign-art` | FHWA, Caltrans, Overpass | `eo-props/data/sign-art-manifest.json` |
| Far-shore elevation | `external/dem` | USGS 3DEP 1 arc-second tiles (public domain) | `assets/dem-manifest.json` |
| Aerial imagery (not part of the setup) | `EO_AERIAL_DIR` | `node native/tools/fetch-aerial.mjs <folder>` (USDA NAIP, public domain); only the roof survey tool reads it | written beside the imagery |

The Node scripts in `tools/` are how the manifests were made:
`fetch-cc0.mjs` and `fetch-sign-art.mjs` fetch their sets and rewrite
their manifests, and `write-people-manifest.mjs` and
`write-motion-manifests.mjs` regenerate the people and motion manifests
from a local copy. Inside the repository they download only into
`external/<set>` (the manifests they rewrite are tracked files in `assets/`
and `eo-props/data/`); they send nothing about the user in their requests
and never execute what they download.

Without all three people and motion sets there are no street people.
Without the sound clips the game is silent; without the scanned sets, sign
art or far-shore tiles, those surfaces, faces and the distant relief fall
back to simpler drawings or are left out. Each set's variable (see
[Environment variables](#environment-variables)) points it at another
folder, and `--no-external` turns the `external/` folders off.

Some street-prop models and two vehicle bodies are kept outside the
repository because they cannot be redistributed; without them the game
draws grey boxes for those props (procedural models of them with
`--stand-ins`) and other vehicles in traffic.

The source binaries of the asset pipelines (Blender scenes, vehicle
sources) are a separate, optional store; the game never reads
them. See [Docs/ASSET-STORE.md](../Docs/ASSET-STORE.md).

## Demo characters

From `native/`, with the test demo in the sibling `test` directory:

```sh
export EO_TEST_CHARACTERS_ROOT=../../test/ar_rifle_demo/public
./target/release/eo-app --list-test-clips
./target/release/eo-app --play --test-character tripo --car-door-animation
./target/release/eo-app --play --test-character civilian-m-01-player --test-clip car-enter
```

The five main demo avatars each have 110 clips. The motorcycle rider and
passenger use the demo's original rigs and its 15-clip rider pack. In local
play the avatars follow the existing controller. The main rigs use
directional walking and jogging, sprinting, crouching, falling, landing,
seated driving, steering and looking back. The motorcycle rigs use their
idle, walk and run on foot.

With `--car-door-animation`, press F beside the stopped sedan's driver
door to enter, and press F again when stopped to exit. The driver and door
share the authored timing. Controls return after the door closes. Leave
room to stand beside the driver door. Where the animated entry or exit
cannot start (away from the driver door, the car moving, on a lift ride),
F enters or leaves the car at once as without the switch. Photo mode holds
it as it holds the rest of play. Both switches are local play only:
`--connect` takes neither, and the network sends no avatar or door state.

`--test-clip` plays any clip from the chosen avatar's catalogue, including its
authored root motion. Motorcycle, helicopter, parachute and weapon-handling
poses are available through this preview.
For a fixed frame in an `EO_CAPTURE_POSES` capture, set `player.time` in
seconds along with `player.feet` and `player.yaw`.

Use `--play --bench --segments car_access --test-character tripo --car-door-animation`
to exercise both F presses in sequence. Add `--capture --out DIR` for
fixed-camera frames before entry, with the door open, and after exit.

The loader verifies byte counts and SHA-256 hashes from
[`assets/test-characters-manifest.json`](assets/test-characters-manifest.json).
Regenerate that manifest with
[`tools/write-test-character-manifest.mjs`](tools/write-test-character-manifest.mjs)
against the source demo's `public` directory.

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
multi-draw there into one CPU draw call per slot, so without the feature a
list is drawn to a bound the CPU knows when it encodes, its slots past this
frame's count zeroed to empty draws. The paths, in this precedence
(`GpuContext::draw_lists`, named in the bench JSON's `draw_lists`):

1. The indirect count, wherever the adapter has it (`--no-indirect-count`
   skips it on an adapter that has the feature).
2. The count emulation, Metal's default (`eo-render/src/counted.rs`): the
   lists are still compacted and each is drawn to a bound read back from
   its counts a few frames late: its recent peak plus half again and 256
   commands. After a camera cut, before the first read-back or when a count
   outran its bound, a conservative cull is submitted and waited for to
   measure the bound. A late opaque list is floored by its early list's
   bound (after a fast turn every visible object can be late), except in a
   frame where only a small rotation can change what is visible: the eye
   still, the same lens, nothing streamed or written that selects objects,
   the occlusion on in this frame and the last, a fresh read-back, and a
   turn under 2 degrees and no larger than one the read-backs saw; there it
   takes its own reserve. An early list is drawn to the smaller of its
   reserve and an exact bound: the early pass draws only what the last late
   pass found visible, so its count is at most the newest read-back's early
   and late counts plus the late bounds of the frames encoded since
   (`--no-motion-bounds` keeps the floor and the reserves, as before).
3. The bounded draws, opt-in with `--bounded-draws` on top of 2
   (`eo-render/src/bounds.rs`; `--no-bounded-draws` wins wherever it stands
   and gives 2): every cull also counts guards, the objects in a frustum
   widened by 20 degrees a side plus a 40 m sphere, and every object within
   the far distance plus a margin that follows the camera's speed; a list is
   drawn to the guard its newest read-back covers (a turn under 10 degrees,
   or a move within the margin), never waited for, and at its capacity with
   no usable read-back or for 60 frames after an overflow. It bounds the
   city's lists, the sun cascades, the far layer and the distant trees, and
   the scene probe's faces from a count pass per capture. The far sun shadow
   map's lists and, with `--probe-grid`, the probe's keep 2's bounds. On
   matched bench runs (Apple M-series) it drew 2 to 3.4 times the
   emulation's city commands on the street paths and cost up to 0.9 ms more
   CPU a frame there; it was faster only on `city_overview`, so it stays
   opt-in. The sea's patches are one instanced draw in every path.
4. `--no-count-emulation` keeps the uncompacted fallback: one slot per
   object, issued in conservative blocks of 64 slots (`--no-draw-ranges`:
   every slot), with the far layer's own fixed-size lists and no occlusion
   or probe.

With compacted lists (1 and 2) the city's opaque pass is split by class
(eo-render `CityClasses`): the cull sorts each opaque object by its
material into the street surface, a landmark's surface kinds or the rest,
each with its own early and late list, and each list is drawn with a
pipeline that compiles only its class's surface code. One fragment shader
holding every surface cost every pixel the whole shader on Apple's GPU;
each object takes the same code in its class's pipeline, so the frame is
the same. `--no-city-classes` keeps one pipeline and one list pair; the
glass, the scene probe, the bounded draws and the fixed-slot lists keep
it too.

**Hand-built buildings.** Several hundred buildings are modelled in Blender
from DataSF footprints, LiDAR heights and the assessor roll
(`scripts/blender-buildings/`). Their exteriors ship as hash-verified GLBs
under `public/assets/buildings/`, placed by the modules in
`assets/blender-buildings/`.

**Kit buildings.** Every other building, about 170,000, is planned by
`eo-kit` from its footprint, height, district and use: an archetype, a
street front, storeys and bays, shop fronts, roof plant, materials. Plans
are meshed per 128 m tile at several levels, cooked on worker threads and
cached. Each building draws its own window module (bay pitch, window
width, sill and head), and a tower its own facade system (unitised or
stick curtain wall, ribbon, punched or finned precast, masonry piers) with
its glass coating and stone tone, known towers their recorded one
(`eo-kit` `typology`), glazed on every face off a lot line; Queen Anne
rows carry their front gable, a "Victorian" front taller than a Victorian
house is drawn as an apartment house, and the timber rows take the city's
pastels with white trim (the Postcard Row a pastel each). Wall bodies are
held to a muted chroma, with a Chinatown front's accent on its trim.
Facades carry a detail atlas, weathering and age, window rooms behind the
glass (an office's blinds by tenant run, seen through the pane's own
visible transmittance), and roofs coloured from the measured aerial table
with the imagery's colour cast taken off. With
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
cell covers the buildings that are far in this frame. Where the far layer is
not drawn, a building past its own levels is its footprint box, drawn by the
city shader with its archetype's facade (`eo-kit` `far_facade`, `eo-render`
`far_box`): storeys, window bays, glass, frames, spandrels, shop fronts,
cornices and crowns, the kit's paint, and the measured roof colour, with
the coping round the roof's edge, the roof plant (packaged units, stair
bulkheads, elevator overruns, a tower's penthouse) as boxes, a curtain
wall's panes a little out of plane, and the lot rhythm of a footprint over
several lots. `--no-far-facade` draws the plain boxes again and compiles the
facade's shader code out (`SF_FAR_FACADE`).

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
`PIERS_OVER_WATER`). `--no-water` draws a flat sea plane. The shading
features and the strait's parts in force (the `--no-water-*` switches,
`--no-gg2-water`, `--no-gg3-water` and their members) are compiled into the
sea's and the surf strip's shaders as constants (eo-render
`water::common_source`), so the code of a feature that is off, and the
earlier path a feature that is on replaces, is not in the pipeline at all.

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

To record the cove descent, run the following from `native/`. The game writes
1,800 consecutive PNGs at 60 frames per second into a new directory. The
camera, water and sky advance together. The recording uses the live rendered
frame, including streaming and temporal effects. Run performance checks in a
separate invocation because recording reads every frame back from the GPU.

```sh
EO_RECORD_DIVE=bench-out/cove-film ./target/release/eo-app --bench --dive-site bay --segments cove_descent --width 2560 --height 1440 --time-of-day 11 --weather clear --weather-seed 7
```

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
cover (`--no-bridge-curtain`). The wires' aerial perspective takes its sky
colours (the dome's and the atmosphere tables' toward the member) at each
segment's ends and its distance terms per pixel (`--no-bridge-wire-sky`
reads the colours per pixel, as before: the overlay then costs about
1.8 ms of GPU time on the golden_gate path instead of about 0.3).
Eight landmarks can be entered (`eo-landmarks/src/access.rs`, every height
cited there): Coit Tower's gallery in its glazed arcade, Salesforce Tower's
glazed top floor, the Transamerica Pyramid's 48th-floor room under the
spire, the Golden Gate Bridge's east sidewalk along the span and the south
tower's top, the Bay Bridge's W2 tower top, Sutro Tower's upper platform and
City Hall's lantern gallery. Each deck is drawn (floor, railings or glazing,
parapets, lamps) and is in the collision world with the same geometry, and a
lift at the ground entrance takes the player up and down (E, a short fade).
The enclosed decks' interiors carry a baked sky visibility through their
openings (`access_sky.rs`), the camera meter opens up inside them
(`eo-render/src/deck_rooms.rs`; on the dome-estimate meter,
`--no-scene-meter`: the scene meter reads the room in the drawn frame),
and on a deck the player's body occludes the floor's indirect light
(`landmarks_deck.wgsl`). With the decks the Golden Gate leaves its east
sidewalk railing and its south tower's saddle housings, and the west span
its W2 tower's, to the decks, which stand on the bridges as drawn (their
scaled heights and paint); the decks' rope sockets give way to the
bridges' own.
The ride is part of the player's fixed step and state, so a server runs it
from the same input (protocol 4 carries the tap) and the client agrees bit
for bit. The Ferry Building's clock-tower balcony is not built
(`ferry_building.rs` draws the tower, not yet a deck on it); Twin Peaks'
summit is open ground the player already walks.

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

- **Optional assets**: tests that read people, motions, scanned sets, sound,
  sign art or the DEM use `external/` (or the set's variable) and are
  skipped, saying so, when the set is not there; each crate checks that its
  folder defaults to `external/<set>`.
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
  `eo-server` `structures`), and a replay lands where the first run did
  however slowly the collision tiles cook (every authoritative step has the
  tiles it reaches resident first: `CollisionWorld::ensure_step_tiles`);
  names, emotes, the sky clock and fast travel reach every client.
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

Six tests fail today. They are open defects, not flaky tests; do not
loosen them to make them pass. The last four came with the Golden Gate
branch and fail on it alone too.

| Test | What fails |
| --- | --- |
| `eo-street` `surface::street_surface_is_closed_and_on_its_section` | Two walkable surfaces fight in one place at more probes than the test allows |
| `eo-street` `surface::verge_banks_stay_off_the_paving` | Some verge banks reach onto the paving |
| `eo-look` `atmosphere::cie_sky_tests::the_sky_near_the_horizon_within_the_cie_clear_skies` | Near the horizon the physical sky is brighter than the CIE clear-sky range (up to x1.5 at 5 degrees elevation) |
| `eo-look` `atmosphere::cie_sky_tests::the_sky_gradient_away_from_the_horizon_within_the_cie_clear_skies` | The gradient away from the horizon leaves the CIE range at some sun positions (up to x1.12) |
| `eo-look` `tone::camera::tests::the_virtual_chart_renders_neutral_greys_and_keeps_hues` | The Aces2 camera style keeps 0.80 of the saturated patches' chroma, under the 0.85 the test allows |
| `eo-render` `landmarks::landmark_lights_join_the_sky_gather` | 244 bridge deck lamps on the x1.25 ground plan, under the 250 the test expects: the Golden Gate's cable detail places them at the published 150 ft pitch on the plan |

The `eo-server` network test
`loss_reorder_and_duplicates_lose_no_input_and_keep_prediction_exact` is
flaky: it passes alone and fails about one run in three when other network
tests load the machine at the same time (a few millimetres of prediction
drift; a joining world may insert its collision tiles in a timing-dependent
order).

## Known issues

- The six failing tests and the flaky network test above.
- The sign-art manifest records the first ZIP of the FHWA Standard Highway
  Signs 2024 Release 3 as the source of six signs (S1-1, W8-1, W11-2,
  W16-7PL, W16-9P, W17-1), but the published first ZIP does not hold them;
  its second ZIP (`..._Vector_Graphics_2-of-2.zip`) does, with the recorded
  SHA-256. So `setup-assets.py` cannot fetch them from the recorded source,
  the sign-art set is incomplete, and the sign atlases keep their drawn
  faces. `tools/fetch-sign-art.mjs` has the same table; correcting it and
  regenerating the manifest with it fixes both.
- Some content and behaviour of the earlier browser game is not ported yet:
  general building interiors, mantling and swimming, the car's full handling
  model, gamepad input in the window, and more. General building interiors
  are not drawn (the furnished house at 21 Beaver Street is the exception);
  their models were removed from the repository and remain in its Git
  history.
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
