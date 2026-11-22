# Contributor and agent guide

These rules apply to every change in this repository, by a person or by a
coding agent: code, data, assets, docs and commit text. [CLAUDE.md](CLAUDE.md)
is a short summary that points here.

## Repository at a glance

Earth Online is a Rust desktop program that draws San Francisco from open
data. The browser build has been removed; the only runtime is the Cargo
workspace in `native/`.

| Path | What | Notes |
| --- | --- | --- |
| `native/` | Cargo workspace: the game and its crates | [native/README.md](native/README.md) lists every crate, switch and test |
| `native/assets/` | Small tracked inputs: manifests of the optional assets, sound credits, measured tables, the fallback font, building placement modules | Regenerate manifests with the scripts in `native/tools/` |
| `native/tools/` | Node scripts that fetch optional assets and write their manifests | Inside the repository the only target is `external/<set>` |
| `external/` | The optional assets (people, motions, scanned surfaces, sound, sign art, far-shore terrain), one folder per set, filled by `python3 scripts/setup-assets.py` | Git-ignored; never commit or stage its contents, or a link to it |
| `scripts/setup-assets.py` | One-command download of the optional assets into `external/`, every file checked against its manifest | Python 3 standard library only; reads the manifests, never edits them |
| `public/data/sf/` | The city data the game loads | Each file has a `*.NOTICE.txt` |
| `public/assets/` | Shipped models and textures | Each has a provenance file; hash-verified at load |
| `artifacts/` | Provenance and `ASSETS.lock.json` of the off-repository asset store | No binaries ([Docs/ASSET-STORE.md](Docs/ASSET-STORE.md)) |
| `scripts/assets/`, `scripts/verify/` | Asset-store and vehicle tooling, the asset-lock check | Run through `npm run` |
| `scripts/blender-buildings/` | The hand-built building pipeline | Blender, headless |
| `scripts/world-data/sources/` | The cited California street-standards table | [Docs/CALIFORNIA-STREET-STANDARDS.md](Docs/CALIFORNIA-STREET-STANDARDS.md) |
| `Docs/` | Data pipeline, street standards, asset store, visual gate | |

### Crates and what each owns

Assign each change to one owning crate. A change that spans several is an
integration change: say so in the description and keep each crate's part
reviewable on its own.

| Crate | Owns | Does not own |
| --- | --- | --- |
| `eo-data` | City file loaders, the street-standards registry and approved deviations | Rendering, simulation |
| `eo-assets` | GLB verification and cooking, the model cache, kit materials, building placement, scanned surfaces | Runtime policy |
| `eo-kit` | The procedural building planner and its meshes | GPU code |
| `eo-street` | The street surface, junctions, kerbs, ramps, crossings, paint, building floors | GPU code, traffic behaviour |
| `eo-look`, `eo-sky` | Lighting model, sky, weather and night photometry as pure functions | GPU passes |
| `eo-water` | Shore field, ocean spectrum, water optics, surf | GPU passes |
| `eo-landmarks`, `eo-farshore` | Bridges, waterfront, landmark towers, open-space ground; the land to the horizon | GPU passes |
| `eo-structures` | Grade-separated roads (freeways, ramps, road bridges, tunnels): vertical profiles, deck, support and portal meshes, collision triangles | GPU passes, the street surface |
| `eo-props` | Street furniture, signals, signs, trees, parked cars | GPU passes |
| `eo-sim` | Traffic, signals, pedestrians, population; one fixed-step deterministic clock | Rendering, street geometry |
| `eo-agents` | Vehicle and people libraries, motion, the simulation-to-renderer feed | Simulation truth |
| `eo-player` | Character controller, vehicle, cameras, input, collision world | Map generation |
| `eo-render` | The wgpu renderer: scene, culling, streaming, uploads, every pass | Source coordinates, simulation truth |
| `eo-ui` | Title, menus, settings, map, HUD | Simulation state (it reads diagnostics and issues commands) |
| `eo-audio` | Sound bank, mixer, outputs, the city's sound director | Simulation state |
| `eo-net`, `eo-server` | Transport, snapshots, prediction; the authoritative server | Map truth, rendering |
| `eo-app` | The binary: composition, modes, command-line switches, benches | Crate internals |

## Build and test

All Cargo commands run from `native/`. See
[native/README.md](native/README.md) for requirements, switches and
environment variables.

```sh
cargo build --release
cargo test --release -p <crate>          # the crates you touched
cargo test --release --workspace         # everything (several tests need a GPU)
./target/release/eo-app --bench --check-max
```

At the repository root (Node.js):

```sh
npm run verify:asset-lock                # nothing binary tracked under artifacts/, nothing drifted
```

Six tests fail today and are recorded as known issues in
[native/README.md](native/README.md#known-failing-tests): the `eo-street`
surface tests for fighting surfaces and verge banks, eo-look's two CIE
clear-sky tests and its virtual-chart hue test, and eo-render's bridge
deck-lamp count. Do not loosen them; a fix makes them pass.

### Verification matrix

Run every row that applies, plus any test for a contract the patch can
affect, plus `git diff --check`. A command needed by several rows runs once
for the same code and inputs.

| Change | Required checks |
| --- | --- |
| Any Rust code | `cargo build --release`; `cargo test --release -p <crate>` for each crate touched and each crate that depends on it |
| A loader or data file in `public/data/sf/` | `cargo test --release -p eo-data`, and the tests of the crates that read it |
| Buildings, kit, facades | `cargo test --release -p eo-kit -p eo-assets -p eo-render` |
| Streets, kerbs, crossings, paint, building floors | `cargo test --release -p eo-street -p eo-data -p eo-render`; the street-standards rule below |
| Signals, signs, props, parking | `cargo test --release -p eo-props`; the street-standards rule below |
| Traffic or pedestrians | `cargo test --release -p eo-sim -p eo-agents` |
| Player, collision, vehicle | `cargo test --release -p eo-player` |
| Network | `cargo test --release -p eo-net -p eo-server` |
| Sound | `cargo test --release -p eo-audio` |
| Renderer, materials, a pass, streaming | `cargo test --release -p eo-render -p eo-app`; `eo-app --bench --check-max`; two matched captures (below) for anything visible |
| Anything that can cost frame time | `eo-app --bench --check-max` on an idle machine, and the cost per bench path from matched runs with and without the change |
| A quality tier, or a feature switch a tier sets | `cargo test --release -p eo-app -p eo-ui`; `eo-app --bench --check-max --quality <tier>` for each tier that changes, against that tier's budget (native/README.md, Quality tiers), and for `hyper` also with `--structures --tunnels`; the default (`medium`) must not change |
| Menus, settings, HUD | `cargo test --release -p eo-ui -p eo-app`; `eo-app --capture-ui <screen>` before and after, looked at; `eo-app --ui-bench` (menus 1.0 ms) |
| Anything under `artifacts/` or `public/assets/` | `npm run verify:asset-lock`; the crate tests that load the asset |
| Docs or rules only | Links and paths reviewed (`git ls-files`), the Markdown diff read, `git diff --check` |

A result of "N passed / 0 failed" means the tests ran and their checks held.
It is not evidence about how a frame looks.

## Commits

- One subsystem per commit. The message follows the history's shape,
  `type(scope): summary`, for example `feat(render): ...`,
  `fix(street): ...`, `docs(native): ...`, `perf(kit): ...`.
- `git diff --check` is clean. Keep each file's existing line endings.
- Stage only the files your change owns. Never stage local scratch (`tmp/`,
  `.qa*`, `*.local.md`, `benchmark-local/`), build output
  (`native/target/`, `native/cache/`, `native/bench-out/`) or any binary
  under `artifacts/`.
- Push to the current branch. Never force-push and never rewrite shared
  history.

## Data and provenance

- Every data file and every shipped asset traces to a recorded source with
  its licence, attribution and SHA-256. Data files carry a `*.NOTICE.txt`;
  assets carry a provenance file beside them. Follow
  [Docs/CITY_DATA_PIPELINE.md](Docs/CITY_DATA_PIPELINE.md) for new sources.
- Provenance manifests stay accurate. Regenerate them with the asset's build
  or manifest script; never hand-edit a digest. Shipped models are
  hash-verified at load, so a stale digest makes the program refuse the
  file.
- Keep required licences, attribution and provenance visible where they are
  required, and update [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) when
  third-party material is added or removed. Do not copy third-party assets
  whose terms forbid redistribution; do not hide an asset's origin.
- Do not use Google Maps, Google Earth, Street View or their imagery as a
  data source.

### The asset store

- Pipeline source binaries (Blender scenes, vehicle sources, texture
  plates) live in an off-repository object store. Only
  `artifacts/ASSETS.lock.json` and the JSON and Markdown provenance beside
  the assets are tracked. `npm run assets:pull <substring>` restores what a
  task needs; a clone with nothing pulled is a valid state. See
  [Docs/ASSET-STORE.md](Docs/ASSET-STORE.md).
- Never stage a `.glb`, `.blend`, `.fbx` or texture plate under
  `artifacts/`. `npm run verify:asset-lock` rejects tracked source binaries
  and drifted files.
- Optional runtime content too large for Git (people, motions, scanned
  surfaces, sound, sign art, far-shore terrain) is pinned by a manifest in
  `native/assets/` or `native/eo-props/data/` and downloaded from its
  source by `python3 scripts/setup-assets.py` into the git-ignored
  `external/<set>/` at the repository root. Never commit anything under
  `external/`. The game finds that folder from the repository root
  (`eo_data::external`); a set's environment variable names another folder
  instead. Never hard-code an absolute path.
- Vehicles are hand-built in Blender. Never use a text- or image-to-3D
  generator for a vehicle body, not even as a shape guide.

## California street standards

Every street, sidewalk, kerb, curb ramp, marking, crossing, signal, sign,
curb colour, parking and traffic-rule value follows the California manuals
(CA MUTCD, California Vehicle Code, Caltrans Highway Design Manual and
Standard Plans, SF local standards) or the surveyed real street.

- The cited values are in
  `scripts/world-data/sources/california-street-standards.json`. Rust code
  reads them through `eo_data::standards::CaStandards` (`metric`,
  `metric_key`, `metric_range`) in `native/eo-data/src/standards.rs`. An
  entry with no verified value returns an error; it can never become a
  silent default.
- A literal street dimension or timing in code is a defect.
- A departure from the standard and the survey is a deviation. It is
  recorded in `CALIFORNIA_STREET_DEVIATIONS` in
  `native/eo-data/src/standards.rs` with the standard it departs from, the
  game value, the reason and its status, and is read only through
  `deviation()`, which refuses anything not approved by the owner.
- Changing a table entry needs a primary source (document, edition,
  section or page, URL).

Details: [Docs/CALIFORNIA-STREET-STANDARDS.md](Docs/CALIFORNIA-STREET-STANDARDS.md).

## Quality claims

- Do not claim a visual improvement from functional or integrity checks
  alone.
- Do not claim source correctness from a screenshot.
- Do not weaken a failing gate or test to make a candidate pass. Fix the
  behaviour, or record the rejection.
- Automated image statistics (edge density, histograms, perceptual hashes)
  are regression signals only. They cannot approve a quality bar.
- Before/after evidence means two matched captures: same camera, same time
  of day and weather, same switches and settings, same build profile, a
  fresh start on each side. One capture is a screenshot, not evidence.
- The visual gate's recorded verdict is **REJECT** for the AAA-level claim.
  Carry that fact in completion reports. Only a new blind review round
  following [Docs/VISUAL_QUALITY_GATE.md](Docs/VISUAL_QUALITY_GATE.md) may
  change it.
- Shared text uses neutral, original standards. Never name a commercial
  game, studio or publisher in code, docs, assets, commits or issues, and
  never commit third-party screenshots or video frames.

## Performance: the 10 ms frame

The whole frame (render-thread CPU and GPU) stays under 10 ms at 1920 x 1080;
work on other threads must also finish inside the frame period.
`eo-app --bench --check-max` must keep every bench path's median, p95 and
worst frame under the budget. A change that costs frame time says how much,
per path, from matched runs with and without it on an idle machine. The
budget split is in [native/README.md](native/README.md#performance-target).

## Kill switches

Every visual feature has a command-line switch that turns it off
(`--no-<feature>`, listed in
[native/README.md](native/README.md#kill-switches)), so it can be measured
and compared with and without it, and disabled if it misbehaves.

- A new visual feature adds its own switch, and the switch restores the
  previous output.
- A new feature becomes the default only after matched captures and, where
  it changes the look, review under the visual gate.
- Timing and visual comparisons are made by toggling the switch on the same
  build.

## Stop and ask

These need the owner's explicit approval; finish the permitted work, then
name the action, the rule and the decision needed:

- Weakening, skipping, deleting or loosening a failing test, gate or
  threshold.
- A street, sidewalk, marking, signal, sign, parking or traffic-rule value
  that departs from the standards table and the survey, or a change to a
  table entry without a primary source.
- Adding a dependency crate, or changing `.gitignore` rules that guard
  local or reference material.
- Modifying a shipped model or data file outside its deterministic build or
  manifest script.
- Adding a second renderer, window, main loop or simulation clock.
- Force-pushing, rewriting history, or staging someone else's changes.
- A change whose only justification is that a screenshot looks better.

## Completion report

Every non-trivial handoff uses:

```text
Assumption:
Changed:
Verified:
Remaining risk:
```

`Changed` lists files, not intentions. `Verified` names the exact commands
run and the paths of any captures or reports they produced. `Remaining risk`
names what was not run and why, including the visual gate's REJECT verdict
when the change is visible.
