# project-sedan-v1

Generic mid-size sedan built procedurally in Blender 5.1 (headless, no GUI needed).
No third-party vehicle was referenced; the shape is an original loft driven by
the station tables in `sedan_lib.py`.

## Files

- `project-sedan-v1.glb` - LOD0, ~165k triangles, 31 materials, 3.6 MB.
- `project-sedan-v1-lod1.glb` - LOD1, ~62k triangles, 1.4 MB (lofts decimated to 30%, interior, lamp internals and brake parts dropped).
- `project-sedan-v1.blend` - editable scene (Sedan collection + review camera/lights).
- `sedan_lib.py`, `build_sedan.py`, `headless_build.py` - rebuild from scratch:
  `Blender -b -P headless_build.py -- <out_dir> <tag> export` (renders review shots, exports LOD0/LOD1 and the .blend, ~50 s).
- `render_closeups.py` - alignment review: 14 close-ups of every junction from the saved .blend (`renders/close/`).
- `renders/` - EEVEE review shots: closed (`front34`, `side_close`, `cabin`, `inside`), everything open
  (`open_*`), a roof-off cutaway of the cabin (`open_cutaway*`), dusk shots with the lamps lit (`dusk_*`), and `close/`.

## How the body is built (alignment notes)

- **One envelope surface.** `sedan_lib.envelope_loft` samples, per station, the outer envelope of the
  body section and the glasshouse section (ray-cast about the section centre, soft-max blend with a
  ~12 mm fillet). The old two-loft build left the glasshouse overhanging a round shoulder by up to
  18 cm, which read as a dark belt band. The body's top exponent varies along the car
  (`body_ntop`: soft nose, square shoulders through the cabin, moderately square tail) and the body
  top sits ~6 cm above the glasshouse base so the glasshouse corner is buried in the shoulder.
- **Crease column.** The shoulder/glasshouse crossing sits on fixed vertex columns that drift at most
  0.5 degrees per ring along the doors and are frozen elsewhere, so no strip ever shears. The
  glasshouse tapers in plan at the cowl and deck so only its roof skin emerges there.
- **Cabin.** Hollowed with a glasshouse loft inset 2.5 cm (pillars and roof thickness) plus a box
  down to the floor; the headliner sits 1 cm inside the roof skin.
- **Panels.** Hood, trunk, doors and fuel door are cut out of the pristine envelope first
  (`extract_panels`) and then get the same arch and cabin-void cuts as the body. Each panel is inset
  3 mm on its free edges (`PANEL_GAP`), so every shut line is a real gap showing the painted jamb.
  Window openings are the glass outline inset 12 mm, solidified and subtracted, so the pane always laps
  the frame by 12 mm and the gaskets sit on skin.
- **Booleans.** EXACT solver (keeps quads). Its inside/outside classification can flip with vertex
  order on these lofts, so `boolean_apply` restores and retries with self-intersection handling on an
  empty result, and `_cut_piece` validates every panel (face count, skin volume) the same way.
  MANIFOLD is avoided (triangulates into shading fans). No bevel or weighted-normal modifiers: both
  smeared cut edges into spikes; smooth-by-angle shading from `new_obj` is enough.
- Fake shut-line, bonnet-line and boot-line strips are gone; the bumper cover seams remain as strips.

## Conventions (match `src/vehicles/vehicle-geometry.js`)

- Y-up glTF, forward is **+Z** (nose at z = +2.36), ground plane at y = 0.
- Origin on the ground under the wheelbase centre.
- Length 4.72 m body (4.88 m with plates/exhaust), width 1.84 m body (2.08 m with mirrors), height 1.465 m.
- Wheelbase 2.85 m, track 1.56 m, wheel radius 0.335 m, tyre width 0.235 m.
- Every moving part is its own node with the origin on its hinge or slide axis; closed / rest = identity.
  Axes below are glTF axes (Blender Z becomes glTF Y, Blender Y becomes glTF -Z).

## Articulated nodes

Exterior

- `Hood` (hinge at the cowl, rotate about X, negative opens), `Trunk` (hinge at the deck front edge, rotate about X, positive opens).
- `Door_FL/FR/RL/RR` (hinge at the front edge, rotate about Y; left doors open with positive, right with negative).
  Each door carries its frame, gaskets, handle, window, door card, armrest and pull; front doors also carry the mirror.
- `Window_FL/FR/RL/RR` (child of its door, origin at the bottom edge): translate along -Y to roll down, up to ~0.40 m.
  The runtime should clip the pane below the belt line while lowered; the door skin is a solid slab, not a hollow.
- `Mirror_L/R` (child of the front door, pivot at the arm base, rotate about Y; left folds with negative, right with positive).
  Holds `Mirror_*_Arm`, `Mirror_*_Head`, `Mirror_*_Glass` and the `Indicator_M*` repeater.
- `Handle_FL/FR/RL/RR` (child of its door, hinge at the front end, rotate about Y; left flips with positive, right with negative).
- `FuelDoor` (right rear quarter above the arch, hinge at the front edge, rotate about Y, negative opens; `FuelWell` and `FuelCap` sit behind it).
- `Wiper_L/R` (pivot at the cowl end, rotate about local -Y to sweep).
- `Wheel_FL/FR/RL/RR` steer pivots (rotate about Y) each containing `Wheel_*_Spin` (rotate about X) with tyre, rim, disc and caliper.

Interior

- `Seat_FL/FR/Rear` slide pivots at the floor (translate along -Z to move rearward); each holds the base, insert and rail
  plus `SeatBack_FL/FR/Rear` (recline pivot at the base/back junction, rotate about X, negative reclines) with the back, insert, headrests and posts.
- `SteeringWheel` (pivot at the hub on the column axis; rotate about local Z) with rim, hub and spokes; `SteeringColumn` is fixed.
- `GearLever` (pivot at the console, rotate about X), `PedalBrake` / `PedalThrottle` (hang from the top edge, rotate about X, negative presses).
- `GloveboxLid` (hinge at the bottom edge, rotate about X, negative drops it; `GloveboxCavity` is the tray behind it, cut through both dash blocks).
- `ConsoleLid` (hinge at the rear edge, rotate about X, negative lifts it; `ConsoleBin` is the tray below).
- `SunVisor_L/R` (hinge at the front edge under the headliner, rotate about X, negative flips down).
- `DomeLight`, `Dials`, `CentreScreen` are fixed but carry switchable emissive materials.

Lamps (grouped, switched by material rather than moved)

- `Headlamp_L/R` empties hold `*_Housing` (45 mm behind the lens elements), `*_Cover` (VehicleHeadlamp, alpha),
  `*_LowBeam` + dish (VehicleLowBeam), `*_HighBeam` + dish (VehicleHighBeam), `*_DRL0..6` (VehicleDRL) and `Indicator_F*_2..6` (VehicleIndicator).
- `Taillamp_L/R` empties hold `*_Housing`, `*_Cover` (VehicleTaillamp, alpha), `*_Bar0..6` (chrome), `*_Tail0..6` (VehicleTailLight),
  `BrakeLamp_*` x3 (VehicleBrakeLamp), `Indicator_R*` x2 (VehicleIndicator), `ReverseLamp_*` x2 (VehicleReverseLamp).
- `FogLamp_L/R` (VehicleFogLamp) flush in the front valance corners, `HighMountStop` (VehicleBrakeLamp) on the rear shelf,
  `Indicator_ML/MR` repeaters on the mirror heads, `DomeLight` (VehicleDomeLight) in the headliner.
- The runtime switches a function by raising the emissive intensity of that material name; all default to a dim "off" glow.

Other material groups: `VehiclePaint` (tint per instance), `VehicleGlass` (alpha 0.28), `VehicleTrimBlack`, `VehicleTrimGloss`,
`VehicleChrome`, `VehicleAlloy`, `VehicleAlloyDark`, `VehicleTyre`, `VehicleTread`, `VehicleSteel`, `VehicleCaliper`,
`VehicleInterior`, `VehicleSeat`, `VehicleSeatInsert`, `VehicleCarpet`, `VehicleDash`, `VehicleHeadliner`, `VehicleScreen`, `VehicleDial`, `VehiclePlate`.

## Checks

- `node scripts/qa/probe-vehicle-glb-articulation.mjs <glb>` rotates or slides all 40 moving nodes in three.js, checks the
  lamp groups carry meshes and that every light material is present; fails if any part is missing or does not move.
- `node scripts/qa/probe-vehicle-glb-load.mjs <glb>` parses the file with the project's GLTFLoader and prints mesh/material/triangle/bbox stats.
- `build()` asserts the body still has its roof after all cuts (a boolean failure once removed it silently).
- The runtime loader hash-verifies both GLBs (`src/vehicles/hero-sedan-glb.js`, `src/render/passes/vehicle-hero-overlay.js`);
  update those SHA-256 constants after every export, the export is not byte-deterministic.

## Known limits

- Flat colours only, no UV texturing; plates are blank.
- Interior is blocked out from primitives; no stitching, gauges, switchgear or trim detail yet. Seat belts, inner door latches
  and the trunk floor/lining are not modelled.
- Door skins are solid slabs (~9 cm) rather than hollow shells, so a rolled-down window needs runtime clipping below the belt.
- No LOD2 yet; LOD1 is a straight decimate, not a hand-retopo.
- The runtime only drives wheels so far; door, window, mirror, lamp and interior states are exposed by node/material name but not yet animated by the game.

## Dressed scene (Blender MCP pass, 2026-09-02)

`project-sedan-v1-dressed.blend` / `project-sedan-v1-dressed.glb` are the procedural build plus ~300 hand-placed
parts added live through the Blender MCP; review shots in `renders/dressed/`. Not wired into the runtime (the
hero overlay still loads the lighter `project-sedan-v1.glb`); procedural node materials do not survive glTF export.

- Engine bay: radiator, fan shroud, front crossmember, strut towers with top mounts, battery, fuse box, air box and
  intake duct, coolant/washer/brake reservoirs, brake booster, cam-cover ribs, intake manifold and runners,
  alternator, pulleys, serpentine belt, hoses, wiring loom, hood insulation (offset copy of the hood underside),
  hood hinges, gas struts, latch.
- Trunk: floor, spare-wheel cover, side trims, goose-neck hinges, gas struts, lid liner, latch/striker, seat-back panel, sill.
- Jambs: body-side and door-side weatherstrips, two hinges per door, strikers, latches, check straps, sill scuff plates,
  inner sills, A/B/C-pillar trims.
- Wheel wells and underbody: arch liners, dampers with coil springs, lower arms, hub carriers, brake hoses, tie rods,
  subframes, fuel tank, exhaust with cat and muffler, heat shield, driveshaft, differential, half-shafts, oil pan, gearbox.
- Exterior: glass surrounds, drip rails, neutral ring emblems, plate frames/lamps/screws, tow-hook covers, reflectors,
  mud flaps, mirror gaskets, washer nozzles, antenna gasket, fuel-cap ring, lug nuts and valve stems.
- Interior: seat belts with retractors/buckles, inner handles and bezels, switch panels, speakers, lock pins, door
  pockets, stalks, horn pad, instrument hood, start button, floor mats, gear gaiter, footrest, mirror stem, dome lens
  ring, grab handles, glovebox latch, cupholders, vent slats.
- Materials: flake paint with clearcoat, transmissive glass, tyre and tread bump, leather grain, fuzzy carpet and
  headliner, brushed alloy, cast-iron grain, asphalt ground. World is a 2k Poly Haven sky HDRI, AgX view transform,
  EEVEE with raytracing.
