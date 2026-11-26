# Third-party notices

Earth Online's own code is FSL-1.1-ALv2 licensed (see [LICENSE](LICENSE)). This file
lists the third-party code, data and assets the project ships or reads, with
their licences and where the full terms and provenance are kept. It keeps
only what the records in this repository support; entries that could not be
confirmed from those records are marked **Unconfirmed**.

The repository has three kinds of third-party material:

1. **Shipped**: files tracked in this repository.
2. **Read when present**: optional content the program reads from the
   git-ignored `external/` folder at the repository root (or from a folder
   an environment variable names). The repository holds only a manifest
   that pins each file by size and SHA-256; the files themselves are
   downloaded from their source by `scripts/setup-assets.py`, are never
   committed, and are not redistributed here.
3. **Dependencies**: Rust crates fetched by Cargo at build time.

No source, agency or author listed here endorses Earth Online.

## Code

### Rust crates

The native build depends on crates from crates.io under permissive
licences. A summary by declared licence is generated at the end of this file
(see [Rust crate licences](#rust-crate-licences)). Two crates (`r-efi`)
offer LGPL-2.1-or-later only as one option of a choice that includes MIT;
the project uses them under MIT or Apache-2.0.

Some crates are licensed under the Apache License 2.0 alone (the
`Apache-2.0` row of that table, among them `rapier3d`, `parry3d`,
`nalgebra`, `winit` and `cpal`). A build of the game contains their object
code, so the licence text they require is kept in
[LICENSES/Apache-2.0.txt](LICENSES/Apache-2.0.txt).

### wgpu-hal (modified copy)

`native/vendor/wgpu-hal/` is a copy of the `wgpu-hal` 30.0.2 crate from the
wgpu project (<https://github.com/gfx-rs/wgpu>), MIT OR Apache-2.0, used in
place of the crates.io release. Earth Online changed its Metal backend so
that one shader module can be compiled in Metal's relaxed math mode instead
of fast math; the changed places are marked "Earth Online change", and
[native/vendor/wgpu-hal/EARTH-ONLINE.md](native/vendor/wgpu-hal/EARTH-ONLINE.md)
lists them with the source crate's SHA-256. Licence texts:
[LICENSE.MIT](native/vendor/wgpu-hal/LICENSE.MIT),
[LICENSE.APACHE](native/vendor/wgpu-hal/LICENSE.APACHE).

### ez-tree (port)

`native/eo-props/src/eztree/` is a Rust port of ez-tree's tree generator.
MIT License, Copyright (c) 2024 Daniel Greenheck. Full text:
[native/eo-props/src/eztree/LICENSE](native/eo-props/src/eztree/LICENSE).

### tidewater (port)

Parts of the water model in `native/eo-water/` are ported from tidewater.
MIT License, Copyright (c) 2026 DRG Software Solutions LLC. Full text:
[native/eo-water/LICENSE-tidewater.md](native/eo-water/LICENSE-tidewater.md).

### ACES 2.0 output transform (port)

`native/eo-look/src/tone/aces2.rs` is a Rust translation of the Academy
Color Encoding System 2.0 reference CTL (aces-aswf/aces-core and
aces-output), Copyright Contributors to the ACES Project, Apache License 2.0
([LICENSES/Apache-2.0.txt](LICENSES/Apache-2.0.txt)). The source commit,
file digests and the changes made are in
[native/eo-look/data/camera/ACES.NOTICE.txt](native/eo-look/data/camera/ACES.NOTICE.txt).

### Weather and tide data in the native crates

- **NOAA / National Weather Service ASOS METAR reports** of San Francisco
  International Airport (KSFO), 2014-2023, as archived by the Iowa
  Environmental Mesonet (Iowa State University): public domain (U.S.
  Government work). Summarised in
  `native/eo-look/data/ksfo-clear-day-sky.csv`; source, query and digests in
  its `.NOTICE.txt`.
- **NOAA National Ocean Service CO-OPS**, station 9414290 hourly wind and
  current station SFB1203 harmonic constants: public domain (U.S.
  Government work). Summarised in
  `native/eo-water/data/golden-gate-climate.json`; source and digests in its
  `.NOTICE.txt`.

## Fonts

- **kenpixel** (`native/assets/fonts/kenpixel.ttf`), from Kenney Fonts
  (https://www.kenney.nl/assets/kenney-fonts): CC0 1.0. The interface's
  fallback font. Source and digest:
  [native/assets/fonts/kenpixel.LICENSE.txt](native/assets/fonts/kenpixel.LICENSE.txt).
- **Overpass 3.0.5** (read when present): SIL Open Font License 1.1,
  Copyright 2015 Red Hat, Inc. (the font is dual OFL-1.1 / LGPL-2.1; used
  under OFL-1.1). The sign atlases rasterise its glyphs when the sign art
  folder is present; the font file is not copied into the repository.
  Source and digest are in `native/eo-props/data/sign-art-manifest.json`.

## City and map data (shipped)

Each data file under `public/data/sf/` has a `*.NOTICE.txt` beside it with
its source, retrieval, SHA-256 and required attribution. `sf-city.json.gz`
carries its sources under `meta.sources`.

- **OpenStreetMap**: © OpenStreetMap contributors, Open Database License
  (ODbL) 1.0, https://www.openstreetmap.org/copyright. Used in
  `sf-city.json.gz`, `sf-lots-parks.json.gz`, `sf-map-citywide.json.gz` and
  `building-metadata/`, and through the street grades of
  `native/assets/sf-decks-v1.json`. These are Derivative Databases of OpenStreetMap and
  are offered under the ODbL; publishing a modified version of them obliges
  you to publish it under the ODbL too. The ODbL does not apply to the code.
- **DataSF**, City and County of San Francisco: Open Data Commons Public
  Domain Dedication and License (PDDL) 1.0. Building Footprints
  (`ynuv-fyni`), SF Shoreline and Islands, Stormwater inlets, drains and
  catch basins (`jtgq-b7c5`, maintained by the SFPUC), Registered Business
  Locations (`g8m3-pdis`) and the Assessor Historical Secured Property Tax
  Rolls (`wv5m-vpq2`).
- **SF Planning, San Francisco Land Use** (`c5ge-t6pj`): CC0 1.0, as
  recorded in `public/data/sf/building-metadata/*/index.json`.
- **U.S. Geological Survey, 3D Elevation Program** (1 m DEM in
  `sf-elevation.bin.gz` and `native/assets/sf-decks-v1.json`; 1 arc-second DEM for the far shore, read when
  present): public domain (U.S. Government work). Credit: U.S. Geological
  Survey.
- **NOAA NCEI Continuously Updated Digital Elevation Model (CUDEM)**, 1/9
  arc-second topobathymetric tiles (seabed in `sf-elevation.bin.gz`): public
  domain (U.S. Government work). Cite as: Cooperative Institute for Research
  in Environmental Sciences (CIRES) at the University of Colorado, Boulder,
  2014, https://doi.org/10.25921/ds9v-ky35. Not for navigation; NOAA and
  NCEI give no warranty and do not endorse this product.
- **NOAA NCEI ETOPO 2022**, 15 arc-second global relief. The regional shelf and
  Bay subset supplies offshore soundings beyond the detailed city grid. US
  Government public-domain data. [Dataset](https://doi.org/10.25921/fd45-gt74)
  and [build record](public/data/sf/sf-shelf.NOTICE.txt).
- **U.S. Geological Survey geologic map databases** (`native/assets/sf-lithology-v1.json.gz`:
  the map units read as rock classes): Blake, M.C., Jr., Graymer, R.W., and
  Jones, D.L., 2000, MF-2337; Bonilla, M.G., 1998, Open-File Report 98-354;
  Graymer, R.W., 2000, MF-2342. Public domain (U.S. Government work). Credit:
  U.S. Geological Survey; no endorsement implied. Sources, digests and the
  unit table are in the [build record](native/assets/sf-lithology-v1.NOTICE.txt).
- **ESA WorldCover 10 m 2021 v200** (`sf-land-cover.bin.gz`): © ESA
  WorldCover project 2021 / Contains modified Copernicus Sentinel data
  (2021) processed by ESA WorldCover consortium. Zanaga, D. et al., 2022,
  https://doi.org/10.5281/zenodo.7254221. Creative Commons Attribution 4.0
  International (CC BY 4.0); attribution is required.
- **USDA NAIP orthoimagery**, through the USGS National Map image service:
  public domain. `native/assets/roof-colours-v1.bin.gz` and
  `native/assets/ground-green-v1.bin.gz` are measurements derived from it
  (roof colours and vegetation shares); no imagery is stored in the
  repository. Credit: U.S. Geological Survey.
- **California street-design standards**: values cited from the California
  MUTCD, the California Vehicle Code, the Caltrans Highway Design Manual and
  Standard Plans, and San Francisco local standards, as listed with
  document, edition, section and URL in
  `scripts/world-data/sources/california-street-standards.json`.

City transit, service and response markings shown in the simulation are
fictional.

## Models and textures (shipped)

Every file under `public/assets/` has a provenance file beside it with its
source, licence and SHA-256; the program refuses a file that does not match.

- **Poly Haven** texture sets and models: CC0 1.0. Per-set authors, pages,
  download URLs and digests are in `public/assets/polyhaven-*.provenance.json`,
  `public/assets/facade-atlas-v2.provenance.json` (the facade atlas; authors
  Rob Tuytel, Dimitrios Savva, Amal Kumar, Dario Barresi and Charlotte
  Baglioni) and `public/assets/facade-detail-v1.provenance.json` (repacked
  from shipped atlas cells).
- **ambientCG**: CC0 1.0. The bark texture of the vegetation atlas
  (Bark001). The CC0 legal text is in
  `public/assets/world-assets-cc0-v1.LICENSE.txt`.
- **Vegetation atlas** (`public/assets/vegetation/`): leaf, grass and flower
  textures and meshes from ez-tree by Daniel Greenheck (MIT), and the
  ambientCG bark (CC0). Terms:
  [public/assets/vegetation/ez-vegetation-v1.LICENSE.txt](public/assets/vegetation/ez-vegetation-v1.LICENSE.txt).
- **Hand-built buildings** (`public/assets/buildings/`): original geometry
  built in Blender by `scripts/blender-buildings/` from open data (DataSF
  footprints, LiDAR heights, the assessor roll). Per-building provenance
  beside each model.
- **Vehicles** (`public/assets/vehicles/fleet-v3/`): hand-built in Blender
  for this project; see `build-report.json` beside them.
- **Street props** (`public/assets/street-props/`): three models, all
  from Poly Haven under CC0 1.0: `cafeSet` (Outdoor Table Chair Set 01,
  James Ray Cock), `sandwichBoard` (Standing Chalkboard 01, ParzivalCG) and
  `utilityBox` (Utility Box 02, James Ray Cock). Source pages and digests are
  in `public/assets/street-props/street-props-v1.provenance.json`.

### Not distributed

These are kept locally, untracked and ignored by Git, and are not part of
the repository or any release. Without the optional prop and vehicle
models the game draws simple proxies for those objects.

The optional demo avatars and motions load from a local copy of
`test/ar_rifle_demo/public`. The four civilian bodies and the motorcycle
variants use the MakeHuman / MPFB source described above. The Tripo
avatar is user-supplied for local use. The animation packs are authored in
the demo projects. Source files and hashes are listed in
[`native/assets/test-characters-manifest.json`](native/assets/test-characters-manifest.json).
The demo's `characters/SOURCE.md` records the asset origins.

- 22 street-prop models made with a text-to-3D service, whose output terms
  are not recorded: `bench`, `bikeRack`, `bollard`, `busStopFlag`,
  `cobraArm`, `hydrant`, `lampBanner`, `lampCobra`, `lampHistoric`,
  `lantern`, `mailbox`, `newsBox`, `palm`, `palmFar`, `parkingMeter`,
  `payStation`, `signPole`, `signPost`, `signalHead`, `transitShelter`,
  `utilityPole` and `wasteBin`.
- The robo-cab and urban-utility vehicles, whose source model's origin is
  not recorded.

## Read when present (not in the repository)

These are downloaded from their source into `external/` by
`scripts/setup-assets.py` and never committed; the manifests named here pin
every file. The aerial imagery and the Golden Gate survey set are not part
of that setup: the imagery is fetched by its own tool, and the survey set is
read from the folder `EO_SURVEY_ROOT` names.

- **Microsoft Rocketbox Avatar Library**: MIT License, Copyright (c) 2020
  Microsoft, https://github.com/microsoft/Microsoft-Rocketbox. The avatars
  and the in-place motion clips, read from CodePawl's GLB repackaging on
  Hugging Face. Manifests: `native/assets/people-manifest.json` and
  `native/assets/rocketbox-motions-manifest.json`.
- **100STYLE dataset**, Ian Mason: Creative Commons Attribution 4.0
  International (CC BY 4.0). Walking styles for the street people, read
  from a GLB repackaging. Manifest: `native/assets/100style-manifest.json`.
- **Sounds**: clips from Freesound through the FSD50K dataset (Fonseca et
  al., 2020, https://zenodo.org/records/4060432; the dataset is CC BY 4.0,
  each clip keeps its own licence, and only CC0 1.0 and CC BY 3.0 clips are
  used), and the SFX Kit by Wyzowl (https://github.com/wyzowl/sfx-kit, CC0
  1.0) for interface sounds. Every CC BY clip is credited by uploader,
  Freesound ID and title in
  [native/assets/AUDIO-CREDITS.md](native/assets/AUDIO-CREDITS.md).
  Manifest: `native/assets/audio-manifest.json`.
- **Poly Haven and ambientCG scanned surface sets**: CC0 1.0. Per-file
  source, authors, URL and digest in `native/assets/cc0-manifest.json`.
- **FHWA Standard Highway Signs 2024** (MUTCD 11th Edition) vector layouts:
  public domain, a work of the U.S. Government (17 U.S.C. 105).
- **Caltrans California sign specification sheets**: used under the
  Caltrans conditions of use (https://dot.ca.gov/conditions-of-use), which
  the manifest records as a public-domain basis. A person reads them for
  dimensions; the program does not parse them. **Unconfirmed**: that
  reading of the conditions has not been independently reviewed.
- **Overpass** font: see [Fonts](#fonts).
- **USGS 3DEP 1 arc-second DEM** for the far shore: public domain.
  Manifest: `native/assets/dem-manifest.json`.
- **USDA NAIP aerial imagery** for the roof survey tool: public domain.
- **The Golden Gate survey set** (`EO_SURVEY_ROOT`; read by the
  `eo_farshore::survey` readers). Manifest, with path, bytes and SHA-256 per
  file and each group's attribution: `native/assets/survey-manifest.json`
  (written by `native/tools/survey-manifest.mjs`).
  - **U.S. Geological Survey, 3D Elevation Program**, 1 m DEM GeoTIFFs
    (projects CA_SanFrancisco_B23 and CA_NoCal_Wildfires_B5b_QL1_2018):
    public domain (U.S. Government work). Credit: U.S. Geological Survey,
    3D Elevation Program.
  - **NOAA NCEI CUDEM** 1/9 arc-second topobathymetric tiles: public domain
    (U.S. Government work). Cite as: Cooperative Institute for Research in
    Environmental Sciences (CIRES) at the University of Colorado, Boulder,
    2014, https://doi.org/10.25921/ds9v-ky35. Not for navigation; NOAA and
    NCEI give no warranty and do not endorse this product.
  - **USDA NAIP orthoimagery** (0.6 m RGB), through the USGS National Map
    image service: public domain (U.S. Government work,
    https://www.usa.gov/publicdomain/label/1.0/). Credit: Imagery: USDA NAIP,
    via the U.S. Geological Survey National Map.
  - **Poly Haven** scanned rocks and plants (glTF models and their 2k
    texture maps): CC0 1.0 (https://creativecommons.org/publicdomain/zero/1.0/);
    no attribution required, credited to Poly Haven (polyhaven.com).
  - The USGS lidar point clouds listed in the download were not fetched; the
    manifest records them as such.

The sign art and sign font are pinned, with source URL, licence basis and
digest per file, in `native/eo-props/data/sign-art-manifest.json`. No
trademark or brand logo is drawn on any sign face.

## Removed material

- The third-party motion-capture pack of the earlier build has been
  removed, because its terms do not allow redistribution. People animate
  only from the Rocketbox (MIT) and 100STYLE (CC BY 4.0) motions above;
  without them there are no street people.
- The NeuroMechFly body and the male CNS connectome tables were part of the
  browser build and are no longer in the repository.
- The earlier browser build and its JavaScript dependencies have been
  removed from the repository.

The optional 21 Beaver Street floor finish uses Dimitrios Savva’s *Wood Floor Worn* for Poly Haven, released under CC0 1.0. Eight board strips were cropped from the diffuse, OpenGL normal and roughness maps. The [material provenance](public/assets/buildings/beaver-terrace/finishes/floorboards.provenance.json) records the original files and their checksums. [Asset](https://polyhaven.com/a/wood_floor_worn) · [Licence](https://polyhaven.com/license)

The weathered 21 Beaver Street variant uses six CC0 photographic materials from Poly Haven. Amal Kumar photographed *Clay Roof Tiles 02*, Rob Tuytel created *Plaster Grey 04*, Dimitrios Savva created *White Stucco 02* and *Wood Floor Worn*, Charlotte Baglioni created *Gravel Floor 03*, and colormass and Rico Cilliers produced *Rough Linen*. The [surface manifests](public/assets/buildings/beaver-terrace/finishes/patina) list each input and its SHA-256. The optional fitted roof samples sixteen individual tile faces from Kumar’s scan for its [clay material](public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/mission-clay.provenance.json). [CC0 licence](https://polyhaven.com/license)

## Rust crate licences

<!-- BEGIN GENERATED: cargo-licences -->
Generated from `cargo metadata --format-version 1 --manifest-path native/Cargo.toml`: the `license` field of every
third-party package in `native/Cargo.lock` (341 packages, including platform-specific ones a given build may not compile),
grouped by that field exactly as each crate declares it. Regenerate it after a dependency change.

| Declared licence | Packages | Crates |
| --- | --: | --- |
| `MIT OR Apache-2.0` | 173 | ahash 0.8.12, allocator-api2 0.2.21, android-activity 0.6.1, android_system_properties 0.1.6, anyhow 1.0.104, arrayvec 0.7.8, as-raw-xcb-connection 1.0.1, ash 0.38.0+1.3.281, bitflags 2.13.2, block-buffer 0.12.1, bumpalo 3.20.3, cc 1.11.2, cfg-if 1.0.5, core-foundation 0.9.4, core-foundation-sys 0.8.7, core-graphics 0.23.2, core-graphics-types 0.1.3, cpufeatures 0.3.1, crc32fast 1.5.5, crossbeam-utils 0.8.23, crypto-common 0.2.2, dasp_frame 0.11.0, dasp_sample 0.11.0, digest 0.11.3, document-features 0.2.12, downcast-rs 2.0.2, earcut 0.4.12, either 1.19.0, ena 0.14.4, errno 0.3.15, fdeflate 0.3.7, find-msvc-tools 0.1.15, flate2 1.1.11, futures-core 0.3.34, futures-task 0.3.34, futures-util 0.3.34, getrandom 0.3.4, getrandom 0.4.3, glam 0.30.10, glam 0.31.1, glam 0.32.1, glam 0.33.12, glam 0.34.3, glamx 0.3.1, gltf 1.4.1, gltf-derive 1.4.1, gltf-json 1.4.1, gpu-allocator 0.28.0, half 2.7.1, hash32 0.3.1, hashbrown 0.16.1, hashbrown 0.17.1, heapless 0.8.0, hermit-abi 0.5.3, hybrid-array 0.4.15, image-webp 0.2.4, itoa 1.0.19, jni 0.22.4, jni-macros 0.22.4, jni-sys 0.3.1, jni-sys 0.4.1, jni-sys-macros 0.4.1, jobserver 0.1.35, jpeg-decoder 0.3.2, js-sys 0.3.107, lazy_static 1.5.1, libc 0.2.191, litrs 1.0.1, lock_api 0.4.14, log 0.4.35, memmap2 0.9.12, meshopt 0.7.0, naga 30.0.2, naga-types 30.0.2, ndk 0.9.0, ndk-context 0.1.1, ndk-sys 0.6.0+11769913, num-bigint 0.4.8, num-complex 0.4.6, num-derive 0.4.2, num-derive 0.5.1, num-integer 0.1.47, num-rational 0.4.2, num-traits 0.2.19, num_cpus 1.17.0, once_cell 1.21.4, parking_lot 0.12.5, parking_lot_core 0.9.12, percent-encoding 2.3.2, pkg-config 0.3.34, png 0.18.1, presser 0.3.1, proc-macro-crate 3.5.0, proc-macro2 1.0.107, profiling 1.0.18, profiling-procmacros 1.0.18, quote 1.0.47, range-alloc 0.1.5, raw-window-metal 1.1.0, renderdoc-sys 1.1.0, robust 1.2.0, rstar 0.13.1, rtrb 0.3.6, rustc_version 0.4.1, rustversion 1.0.23, scopeguard 1.2.0, semver 1.0.28, serde 1.0.229, serde_core 1.0.229, serde_derive 1.0.229, serde_json 1.0.154, sha2 0.11.1, shlex 2.0.1, simdutf8 0.1.5, smallvec 1.16.5, smol_str 0.2.2, spade 2.15.1, stable_deref_trait 1.2.1, static_assertions 1.1.0, syn 2.0.119, syn 3.0.10, thiserror 1.0.69, thiserror 2.0.21, thiserror-impl 1.0.69, thiserror-impl 2.0.21, toml_datetime 1.1.3+spec-1.1.0, toml_edit 0.25.19+spec-1.1.0, toml_parser 1.1.5+spec-1.1.0, ttf-parser 0.25.1, typenum 1.20.1, unicode-segmentation 1.13.3, unicode-width 0.2.3, wasm-bindgen 0.2.130, wasm-bindgen-futures 0.4.80, wasm-bindgen-macro 0.2.130, wasm-bindgen-macro-support 0.2.130, wasm-bindgen-shared 0.2.130, web-sys 0.3.107, web-time 1.1.0, weezl 0.1.12, wgpu 30.0.2, wgpu-core 30.0.2, wgpu-core-deps-apple 30.0.2, wgpu-core-deps-emscripten 30.0.2, wgpu-core-deps-windows-linux-android 30.0.2, wgpu-hal 30.0.2, wgpu-naga-bridge 30.0.2, wgpu-types 30.0.2, windows 0.62.2, windows-collections 0.3.2, windows-core 0.62.2, windows-future 0.3.2, windows-implement 0.60.2, windows-interface 0.59.3, windows-link 0.2.1, windows-numerics 0.3.1, windows-result 0.4.1, windows-strings 0.5.1, windows-sys 0.52.0, windows-sys 0.59.0, windows-sys 0.61.2, windows-targets 0.52.6, windows-threading 0.2.1, windows_aarch64_gnullvm 0.52.6, windows_aarch64_msvc 0.52.6, windows_i686_gnu 0.52.6, windows_i686_gnullvm 0.52.6, windows_i686_msvc 0.52.6, windows_x86_64_gnu 0.52.6, windows_x86_64_gnullvm 0.52.6, windows_x86_64_msvc 0.52.6, x11rb 0.13.2, x11rb-protocol 0.13.2 |
| `MIT` | 68 | alsa-sys 0.4.0, android-properties 0.2.2, block2 0.5.1, block2 0.6.2, bytes 1.12.1, calloop 0.13.0, calloop-wayland-source 0.3.0, cfg_aliases 0.2.2, combine 4.6.9, crunchy 0.2.4, dispatch 0.2.0, dlib 0.5.3, ebur128 0.1.11, float-cmp 0.10.0, inflections 1.1.1, ispc_rt 2.0.4, libm 0.2.16, libredox 0.1.25, objc-sys 0.3.5, objc2 0.5.3, objc2 0.6.5, objc2-app-kit 0.2.2, objc2-cloud-kit 0.2.2, objc2-contacts 0.2.2, objc2-core-data 0.2.2, objc2-core-image 0.2.2, objc2-core-location 0.2.2, objc2-encode 4.1.0, objc2-foundation 0.2.2, objc2-foundation 0.3.2, objc2-link-presentation 0.2.2, objc2-metal 0.2.2, objc2-quartz-core 0.2.2, objc2-symbols 0.2.2, objc2-ui-kit 0.2.2, objc2-uniform-type-identifiers 0.2.2, objc2-user-notifications 0.2.2, orbclient 0.3.55, ordered-float 5.5.1, quick-xml 0.41.0, redox_syscall 0.4.1, redox_syscall 0.5.18, redox_syscall 0.9.5, sctk-adwaita 0.10.1, simd-adler32 0.3.10, slab 0.4.12, smithay-client-toolkit 0.19.2, strict-num 0.1.1, texpresso 2.0.2, tiff 0.10.3, tokio 1.54.1, tracing 0.1.44, tracing-core 0.1.36, wayland-backend 0.3.17, wayland-client 0.31.15, wayland-csd-frame 0.3.0, wayland-cursor 0.31.14, wayland-protocols 0.32.13, wayland-protocols-plasma 0.3.12, wayland-protocols-wlr 0.3.12, wayland-scanner 0.31.11, wayland-sys 0.31.11, winnow 1.0.5, x11-dl 2.21.0, xcursor 0.3.11, xkbcommon-dl 0.4.2, xml-rs 0.8.29, zmij 1.0.23 |
| `Apache-2.0` | 18 | ab_glyph 0.2.32, ab_glyph_rasterizer 0.1.10, approx 0.5.1, codespan-reporting 0.13.1, cpal 0.18.2, gethostname 1.1.0, gl_generator 0.14.0, glutin_wgl_sys 0.6.1, hound 3.5.1, khronos_api 3.1.0, nalgebra 0.35.0, nalgebra-macros 0.3.0, owned_ttf_parser 0.25.1, parry3d 0.31.1, rapier3d 0.36.1, simba 0.10.2, spirv 0.4.0+sdk-1.4.341.0, winit 0.30.13 |
| `Apache-2.0 OR MIT` | 15 | atomic-waker 1.1.2, autocfg 1.5.1, bit-set 0.10.0, bit-vec 0.9.1, concurrent-queue 2.5.0, const-oid 0.10.3, equivalent 1.0.3, indexmap 2.14.2, pin-project 1.1.13, pin-project-internal 1.1.13, pin-project-lite 0.2.17, polling 3.11.0, portable-atomic 1.15.0, portable-atomic-util 0.2.8, simd_cesu8 1.2.0 |
| `MIT/Apache-2.0` | 14 | bitflags 1.3.2, coreaudio-rs 0.14.2, downcast-rs 1.2.1, foreign-types 0.5.0, foreign-types-macros 0.2.4, foreign-types-shared 0.3.1, intel_tex_2 0.5.0, khronos-egl 6.0.0, matrixmultiply 0.3.11, plain 0.2.3, quick-error 2.0.1, rawpointer 0.2.1, scoped-tls 1.0.1, version_check 0.9.5 |
| `Zlib OR Apache-2.0 OR MIT` | 14 | bytemuck 1.25.2, bytemuck_derive 1.12.1, dispatch2 0.3.1, objc2-audio-toolbox 0.3.2, objc2-avf-audio 0.3.2, objc2-core-audio 0.3.2, objc2-core-audio-types 0.3.2, objc2-core-foundation 0.3.2, objc2-core-graphics 0.3.2, objc2-io-surface 0.3.2, objc2-metal 0.3.2, objc2-quartz-core 0.3.2, safe_arch 1.2.0, wide 1.7.1 |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 6 | linux-raw-sys 0.12.1, linux-raw-sys 0.4.15, rustix 0.38.44, rustix 1.1.6, wasip2 1.0.4+wasi-0.2.12, wit-bindgen 0.57.1 |
| `Unlicense OR MIT` | 5 | byteorder 1.5.0, byteorder-lite 0.1.0, memchr 2.8.3, termcolor 1.4.1, winapi-util 0.1.11 |
| `MIT OR Apache-2.0 OR Zlib` | 4 | cursor-icon 1.2.0, glow 0.17.0, raw-window-handle 0.6.2, xkeysym 0.2.1 |
| `Apache-2.0/MIT` | 3 | alsa 0.11.0, pollster 1.0.1, rustc-hash 1.1.0 |
| `Zlib` | 3 | foldhash 0.2.0, slotmap 1.1.1, zlib-rs 0.6.8 |
| `BSD-2-Clause OR Apache-2.0 OR MIT` | 2 | zerocopy 0.8.63, zerocopy-derive 0.8.63 |
| `BSD-3-Clause` | 2 | tiny-skia 0.11.4, tiny-skia-path 0.11.4 |
| `BSD-3-Clause OR MIT OR Apache-2.0` | 2 | num_enum 0.7.6, num_enum_derive 0.7.6 |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 2 | r-efi 5.3.0, r-efi 6.0.0 |
| `MIT OR Zlib OR Apache-2.0` | 2 | miniz_oxide 0.8.9, miniz_oxide 0.9.1 |
| `Unlicense/MIT` | 2 | same-file 1.0.6, walkdir 2.5.0 |
| `(MIT OR Apache-2.0) AND Unicode-3.0` | 1 | unicode-ident 1.0.27 |
| `0BSD OR MIT OR Apache-2.0` | 1 | adler2 2.0.1 |
| `Apache-2.0 AND MIT` | 1 | dpi 0.1.2 |
| `BSD-2-Clause` | 1 | arrayref 0.3.9 |
| `BSD-2-Clause OR MIT OR Apache-2.0` | 1 | mach2 0.6.0 |
| `ISC` | 1 | libloading 0.8.9 |
<!-- END GENERATED: cargo-licences -->

The submerged reef uses reduced colour, normal and roughness maps from *Coast Sand 04* by Rob Tuytel, Dario Barresi ([source](https://polyhaven.com/a/coast_sand_04), [provenance](public/assets/shore/coast-sand-04.provenance.json)) and *Damp Beach Sand* by Dimitrios Savva ([source](https://polyhaven.com/a/damp_beach_sand), [provenance](public/assets/shore/damp-beach-sand.provenance.json)). Both scans are from Poly Haven under [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/). The reef rocks also use the credited *Seaside Rock* scan.

The Black Point shoreline uses *Seaside Rock* by Dimitrios Savva for Poly Haven under CC0 1.0. The game ships reduced colour, normal and roughness maps from this scan. [Source](https://polyhaven.com/a/seaside_rock) · [Licence](https://polyhaven.com/license) · [Provenance](public/assets/shore/seaside-rock.provenance.json)

The optional 21 Beaver sitting-room carpet uses the public-domain recto photograph of *Carpet*, attributed to Turkey, first half of the nineteenth century, Bequest of Joseph V. McMullan, 1973, accession 1974.149.36. The material builder rectifies and resamples the photograph. [The Metropolitan Museum of Art object record](https://www.metmuseum.org/art/collection/search/452588) · [The Met Open Access policy](https://www.metmuseum.org/about-the-met/policies-and-documents/open-access).

- The private frontage tree uses the City and County of San Francisco street-tree inventory (DataSF dataset `tkzw-k3nq`, PDDL 1.0). The original mesh follows the position, height and crown recorded in the existing city data. [Street Tree List](https://data.sfgov.org/d/tkzw-k3nq).

The optional private courtyard and paved apron at 21 Beaver use the CC0 scans [Park Dirt](https://polyhaven.com/a/park_dirt) by Christopher Melani, [Leafy Grass](https://polyhaven.com/a/leafy_grass) by Charlotte Baglioni and [Concrete Floor Worn 001](https://polyhaven.com/a/concrete_floor_worn_001) by Dimitrios Savva, Rico Cilliers. [Poly Haven licence](https://polyhaven.com/license).

Rob Tuytel photographed the frontage tree’s bark. Dimitrios Savva photographed the terracotta used on the porch and steps. The optional house uses these CC0 colour, normal and roughness maps. [Bark Brown 02](https://polyhaven.com/a/bark_brown_02) · [Terracotta Floor Tiles](https://polyhaven.com/a/terracotta_floor_tiles) · [Poly Haven licence](https://polyhaven.com/license).

The optional house courtyard follows parcel 3561049 from the City and County of San Francisco parcel survey, released under PDDL 1.0. [Parcels – Active and Retired](https://data.sf.gov/d/acdm-wktn) · [Source and licence](scripts/blender-buildings/sources/beaver-parcel.NOTICE.txt).

The Black Point shoreline uses *Seaside Rock* by Dimitrios Savva for Poly Haven under CC0 1.0. The game ships reduced colour, normal and roughness maps from this scan. [Source](https://polyhaven.com/a/seaside_rock) · [Licence](https://polyhaven.com/license) · [Provenance](public/assets/shore/seaside-rock.provenance.json)

### San Francisco Bay underwater daylight

The cove daylight preview uses a regional median from USGS station 20 (37.82° N, 122.393333° W). The 2016–2021 release includes 102 nonempty positive values of `Extinction_Coefficient` at that station, one per sampling date. Their median is 0.80 m⁻¹ and the middle half range from 0.60 to 0.90 m⁻¹. The measurements describe vertical attenuation of photosynthetically active radiation in the water column. They do not measure viewing distance or establish the conditions on a particular day at Aquatic Park.

Tara Schraga, Erica S. Nejad, Charles A. Martin and James E. Cloern, *USGS Measurements of Water Quality in San Francisco Bay (CA), 2016–2021*, version 4.0, March 2023. [USGS data release](https://doi.org/10.5066/F7D21WGF), [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/).

Source SHA-256 values:

- `2016_2021SanFranciscoBayWaterQualityData.csv` — `726906506c62e64b5a3e78a8c4e41100cf4e1f6f82c54a13e5adcf892639ddcf`
- `SFBay_TableofStationLocations_v4.csv` — `55f28f5b4f706ec0b0d78feafc56be1779e665fc7e9ebd5dfc0041c8edec7726`

## Procedural marine forms

The two rock outcrops, sponges, branching colony, kelp-column arrangement and
baitfish use mesh generators by Maksymilian Dendura, copyright 2026, licensed
under MIT. The source leaf shader supplies the kelp's taper, curl, width, twist,
edge ruffling and pigment. Earth Online bakes the geometry, adds a midrib and
adapts the animation and placement to the coastal habitat. Earth Online contributors
modeled the bull-kelp forms, eelgrass shoots, bedrock ledges and fractured boulders. Their biological
references are listed in the provenance record. The shipped model includes
[the original license](public/assets/ocean/reef-forms.LICENSE.txt) and a
[provenance record](public/assets/ocean/reef-forms.provenance.json).
[Upstream source at the pinned revision](https://github.com/Noniv/abyssal-garden/tree/a596354eff400d458bc2aeb9bdf5e0a9734b2bcf).
