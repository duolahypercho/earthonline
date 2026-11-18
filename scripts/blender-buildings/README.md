# Blender buildings

One source building at a time, from open data to the running map, built in
headless Blender: the exterior shell, and (in `full` scope) every storey
furnished.

## Scope: exterior only until the city is built

`BUILDING_SCOPE` in [`scope.mjs`](scope.mjs) is the one switch, and it is
`'exterior'` (user decision, 2026-10-09: build every exterior first, then turn
interiors back on). To turn interiors back on for every future build, change
that one constant to `'full'`. For a single run, `--scope=full|exterior` on
`build.mjs` or the environment variable `EO_BUILDING_SCOPE` overrides it (flag
over variable over constant). `build.mjs` always passes the resolved scope to
`build_building.py --scope`; a direct Blender run without it builds `full`.

An exterior-scope build:

- builds the same shell, facade, trim, storefront, bays, roof and shadow proxy
  as a full build, and skips `build_interior()` entirely: no slabs, stair
  core, partitions, room programmes, furniture (box or CC0 library), lights,
  soft furnishings, coarse boxes or `fine-s<k>.glb` storeys. It writes
  `exterior.glb`, `physics.json` and `report.json` only;
- skips the interior triangle budget, the per-storey fine budget and the
  interior containment gate (there is nothing to measure);
- closes every door: an opaque panel in each door reveal
  (`door-closures`, 0.2 m behind the outer face) and no openings on the
  physics walls. `physics.json` is the closed shell plus the grade floor; no
  upper slabs, stair flights, guards or furniture solids;
- glazes every pane opaque: each see-through kit glass (the kit's own
  `physical.transparent`, read by `seeThroughKitGlass()`) becomes
  `EXTERIOR_SCOPE_GLASS` (`glass-shell`, the hollow-shell pane the archetype
  buildings use), keeping any `@rrggbb` tint, and `glassBacking` planes are
  not drawn. Windows, transoms, door lights and shopfronts never show the
  empty shell. (`glassBacking` alone was not enough: it only reaches
  punched / ribbon / high / bay / grid windows, never shopfront, transom or
  door glass);
- installs with `interior: false` and `scope: 'exterior'` in the generated
  module and provenance, a pass id ending `-ext`, and a manifest entry that
  keeps `interiorModule: null` and records `interior: false`. The runtime
  never streams an interior for it (no sentinel, no LOD, no request, no
  warning), keeps the footprint shadow proxy as its shadow caster, and
  publishes no in-place interior contract, so entry falls back to the
  building's generic portal (`src/citygen/interiors.js`).

Signature buildings (user decision, 2026-10-10: interiors only for some good
or signature buildings): [`signature-interiors.json`](signature-interiors.json)
is the short allowlist `scope.mjs` reads. An id under `approved` builds `full`
while the city default stays `exterior`; `proposed` is the shortlist waiting
for approval and changes nothing. Precedence: `--scope` flag, then
`EO_BUILDING_SCOPE`, then the allowlist, then `BUILDING_SCOPE`. A signature
build is an ordinary full build: no `scope` key, no `-ext` pass suffix.

At runtime a full build's see-through panes draw as the opaque shell pane
unless its rooms are live (`src/world/buildings/authored/live-interiors.js`:
the few nearest buildings, budgeted by `liveInteriorBudget()` in
`src/render/device-tier.js`); `root.userData.interiorLive` says which.

Installed buildings are whatever they were built as; the switch only decides
the next build. `build_house.py` (style.json `"builder": "house"`) honours
`--scope` the same way, through the same exterior-scope helpers.

```
make-spec.mjs      live record (loadSfData) -> artifacts/buildings/<id>/spec.json
                   footprint in the building's own frame, edge roles
                   (front / flank / rear / party), LiDAR height, Assessor storeys
style.json         artifacts/buildings/<id>/style.json - per-building overrides of
                   DEFAULT_STYLE in build_building.py (created once, edited by hand)
build_building.py  Blender: shell with real openings, glazing, trim, storefront,
                   floor slabs, stair core, partitions, furniture, lights
                   -> exterior.glb, interior.glb, physics.json, report.json
                   (exterior scope: exterior.glb, physics.json, report.json)
scope.mjs          BUILDING_SCOPE, the exterior-only / full switch (see below)
build.mjs          runs the three above, gates the result (budgets, physics
                   envelope), installs public/assets/buildings/<id>/ and the
                   generated module src/world/buildings/authored/blender/generated/<slug>.js
```

```bash
npm run buildings:blender -- sf-building-243356623                   # build + install (BUILDING_SCOPE)
npm run buildings:blender -- sf-building-243356623 --scope=full      # this run with interiors
npm run buildings:blender -- sf-building-243356623 --render /tmp/b7  # + Eevee stills (outside the repo)
npm run verify:blender-buildings                                     # every installed building
```

In the game, `?blenderBuildings=0` draws the procedural building instead on the
same build, for matched before/after captures.

## Sources

Geometry, height and storey count come from open survey data only: DataSF
building footprints, DataSF LiDAR, and the Assessor roll (via the civic
sidecar). Street-level reference is consulted by eye to set `style.json`; it is
never traced, saved, or read into any file here.

## Contract

- **Frame.** Authored in the footprint frame of `authoredFootprintFrame`
  (`src/world/buildings/authored/physics.js`): +X along the front edge, +Z out
  of it, y up. Blender is Z-up, so a frame point (x, y, z) is written as
  Blender (x, -z, y); the glTF +Y-up export hands back frame coordinates. The
  runtime refuses a GLB whose frame drifted more than 5 cm from the live
  record (rebuild it).
- **Materials.** Exterior materials use authored-kit names
  (`kit/materials.js`); the runtime swaps in the kit's world-anchored node
  materials, so the GLB carries no textures. Interior materials are `int-*`
  plain PBR.
- **Envelope.** Nothing past the world-axis footprint AABB + 0.45 m except
  parts named awning/canopy/sign/marquee/blade (2.5 m). Projections on a
  rotated building cost `p * (|cos| + |sin|)` of the heading; the builder caps
  cornices accordingly.
- **Budgets.** 60k exterior / 40k interior triangles, instances counted
  (the interior budget applies to full-scope builds only).
  Furniture is exported as linked duplicates and collapsed into InstancedMesh
  at load, so a furnished storey costs a draw per furniture part.
- **Streaming.** The exterior loads with the building; the interior (every
  storey) loads the first time the camera comes within 60 m and draws inside
  90 m. An exterior-only building (`interior: false`) streams nothing more.
- **Physics.** `physics.json` is the input of `compileAuthoredBuildingPhysics`:
  exterior walls with door openings (glass blocks), upper slabs as walkable
  solids around the stair opening, stair flights and landings, partitions with
  door gaps, and furniture solids.
- **Style positions.** Fractions in `style.json` are measured from the
  viewer's left, standing in the street facing that face. The builder derives
  which end of an edge that is from the ring's winding and the spec's
  `frameMirrored` (true while the world runs z = south and the authoring frame
  is replayed under `reflection.js`).
- **Street faces.** `streetEdges: { "<edge index>": { ...overrides } }` dresses
  more faces than the front (corners, jogged frontages); `edgeRoles` corrects a
  mis-detected role. Facade systems (all off by default): `pilasters`,
  `spandrels`, `stripes`, `coping`, `frameGrid` (exposed concrete frame with
  glass / balcony / screen-panel bays), storefront `garage | glazed-garage |
  blank | gate` infills, `transomPaneM`, and steel-sash `sashRows/sashCols`.
  The option list is at "facade systems" in `build_building.py`.
- **Storeys.** When the roll's storey count does not fit under the LiDAR
  height at 2.6 m a storey, the height wins and the count drops (a counted
  mezzanine); the generated module carries the built count and `levels`.
- **Fine furniture tier.** Furniture exports one `fine-s<k>.glb` per storey;
  the runtime shows only the storey the camera stands on inside the building,
  while the coarse boxes in `interior.glb` keep rooms reading furnished from
  the street. Each fine storey carries the interior budget on its own. Real
  CC0 pieces and interior textures are opt-in (`EO_FURNITURE_LIB`,
  `EO_FURNITURE_TEXTURES`): every GLB embeds its own texture copies.
- **Determinism.** Same spec + style + script gives the same bytes. The pass
  id (`<slug>-blender-<script digest>-<style digest>`, plus `-ext` for an
  exterior-only build) changes whenever either
  does; editing `build_building.py` means rebuilding every installed building.
