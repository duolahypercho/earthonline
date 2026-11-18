# Earth Online

Earth Online is an open-world city you can walk, drive and fly through,
built from open data. It starts with San Francisco: every street, building
footprint, hill and shoreline in the city comes from public sources such as
OpenStreetMap, DataSF and the U.S. Geological Survey, and the long-term aim
is to grow the same way to other places.

It is a desktop program written in Rust, with a GPU-driven renderer on
[wgpu](https://wgpu.rs). The city has procedurally planned buildings and
several hundred hand-built ones, streets laid out to the California street
standards, terrain and the bay, sky and weather, simulated traffic and
pedestrians, a walking and driving player, sound, and an online server.

## Status

Early and in active development. Many systems are first versions, content
is missing, and things change quickly. It is not a finished game. The
project keeps a strict visual-quality gate, and its recorded verdict is
currently REJECT; see [Docs/VISUAL_QUALITY_GATE.md](Docs/VISUAL_QUALITY_GATE.md).

## Screenshots

<!-- PLACEHOLDER: screenshots to be added. -->

*Screenshots will be added here.*

## Quick start

You need a stable Rust toolchain (Rust 1.85 or newer, from
[rustup](https://rustup.rs)), a C/C++ compiler, a GPU that supports
Vulkan, DirectX 12 or Metal, and Python 3 for the one-time asset
download. The project is developed and tested on Windows; Linux and macOS
are untested.

```sh
git clone <repository-url> earth-online
cd earth-online
python3 scripts/setup-assets.py            # once: the optional assets into external/ (about 2 GB)
cd native
cargo run --release -p eo-app              # fly through the city
cargo run --release -p eo-app -- --play    # walk and drive
cargo run --release -p eo-app -- --ui      # with the title screen and HUD
```

The first start takes a while: models, building tiles and textures are
cooked into `native/cache/` (several gigabytes), and later starts read that
cache.

Everything needed to run is in the repository. Street people, sound, the
scanned CC0 surfaces, official sign art and the far-shore relief come from
open sources and are too large for Git (about 2 GB):
`python3 scripts/setup-assets.py` downloads them once into `external/` at
the repository root, a git-ignored folder, checking every file against the
manifest in the repository that pins it, and the game reads them from
there. It needs Python 3 and nothing else; running it again fetches only
what is missing. Never commit anything under `external/`. Without these
files the game still runs, and those features are off. An environment
variable per set (`EO_PEOPLE_ROOT`, `EO_PEOPLE_MOTIONS_ROOT` and
`EO_PEOPLE_STYLES_ROOT` for people, `EO_AUDIO_ROOT`, `EO_CC0_ROOT`,
`EO_SIGN_ART_ROOT`, `EO_DEM_ROOT`) points the game at another copy. See
[native/README.md](native/README.md#optional-assets) for what each set adds
and where it comes from, and for every command-line switch, environment
variable and test.

The source binaries of the asset pipelines (Blender scenes and similar)
are in a separate optional store that the game never reads. If you work on
those pipelines, restore what you need with Node.js:

```sh
npm run assets:pull <path-substring>
```

See [Docs/ASSET-STORE.md](Docs/ASSET-STORE.md).

## Repository layout

| Path | What |
| --- | --- |
| `native/` | The Cargo workspace: the game, its crates and tests ([native/README.md](native/README.md)) |
| `public/data/sf/` | The San Francisco data the game loads, each file with a `NOTICE` |
| `public/assets/` | Shipped models and textures, each with a provenance file |
| `external/` | The optional open assets, downloaded by `scripts/setup-assets.py`; git-ignored, never committed |
| `artifacts/` | Provenance and the lock file of the off-repository asset store |
| `scripts/` | The optional-asset setup (`setup-assets.py`), asset-store, Blender building and character tooling, and the street-standards table |
| `Docs/` | Data pipeline, street standards, asset store and quality-gate documents |

## Data sources and attribution

The city is built from open data. The main sources, with the attribution
they require:

- **OpenStreetMap**: © OpenStreetMap contributors, licensed under the
  [Open Database License (ODbL) 1.0](https://opendatacommons.org/licenses/odbl/1-0/).
  Streets, building outlines, parks, car parks, piers and place names.
- **DataSF**, City and County of San Francisco:
  [PDDL 1.0](https://opendatacommons.org/licenses/pddl/1-0/). Building
  footprints and heights, the shoreline, storm drains, business locations
  and the assessor roll.
- **SF Planning**, San Francisco Land Use: CC0 1.0.
- **U.S. Geological Survey**, 3D Elevation Program: public domain. Ground
  elevation in the city and, optionally, the land around the bay.
- **NOAA NCEI CUDEM**: public domain. The seabed. Not for navigation.
- **ESA WorldCover 2021**: © ESA WorldCover project 2021 / Contains modified
  Copernicus Sentinel data (2021) processed by ESA WorldCover consortium,
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Land cover.
- **USDA NAIP** aerial imagery: public domain. The roof-colour and
  vegetation tables are measured from it; no imagery is stored here.
- **California street standards**: California MUTCD, California Vehicle
  Code, Caltrans Highway Design Manual and Standard Plans, and San
  Francisco local standards, cited per value in
  `scripts/world-data/sources/california-street-standards.json`.

Every data file in `public/data/sf/` has a `*.NOTICE.txt` beside it with
its exact source, retrieval and digest. Third-party models, textures, fonts,
sounds, motion data and Rust crates are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). No data provider endorses
Earth Online.

## Licence

The source code is MIT licensed; see [LICENSE](LICENSE).

Third-party work keeps its own terms, recorded in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Two points matter before
you redistribute:

- **The map data is not MIT.** The OpenStreetMap-derived files in
  `public/data/sf/` are licensed under the ODbL 1.0, which is share-alike
  for derived databases: if you publish a modified version of that data,
  you must publish it under the ODbL too. It does not apply to your code.
- **Assets carry mixed licences** (CC0, MIT, CC BY and others), and a few
  are marked unconfirmed in the notices. Check an asset's provenance file
  before you reuse it on its own.

## Contributing

Contributions are welcome. Before you start:

- Read [AGENTS.md](AGENTS.md): the repository layout, how to build and test,
  the commit message shape (`type(scope): summary`), and the rules every
  change follows.
- Keep each change focused on one subsystem, and run the tests of the crates
  you touch (`cargo test --release -p <crate>`).
- New street, sidewalk, marking, signal, sign or parking values come from
  the California standards table or survey data, never a literal; see
  [Docs/CALIFORNIA-STREET-STANDARDS.md](Docs/CALIFORNIA-STREET-STANDARDS.md).
- New data or assets need a recorded source, licence and digest; see
  [Docs/CITY_DATA_PIPELINE.md](Docs/CITY_DATA_PIPELINE.md) and
  [Docs/ASSET-STORE.md](Docs/ASSET-STORE.md).
- Visual changes come with matched before/after captures and a switch to
  turn them off. Do not describe a change as a quality improvement from
  tests alone.
- Keep the whole frame under 10 ms.
